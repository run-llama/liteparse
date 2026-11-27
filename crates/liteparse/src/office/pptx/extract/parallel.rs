//! The page loop spread over several render threads.
//!
//! Scene building uses single-threaded structures (`Rc` XML trees, `RefCell`
//! caches in the document and the font system), so each thread opens its own
//! view of the document and gets its own copy of the font system. The heavy
//! shared state is thread-safe and shared: the zip package, the decoded-picture
//! store and the master/layout layer cache.
//!
//! Threads claim pages in order from one counter, build and render them,
//! encode the screenshot themselves and send the record to the calling thread,
//! which writes records strictly in page order. Every cache only memoizes pure
//! functions of the deck, so the output is byte-identical to the sequential
//! loop whatever the thread count or timing.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Instant;

use super::{PageWriter, Queued, Shot, page_record, panic_message};
use crate::office::pptx::config::Config;
use crate::office::pptx::document::{Document, DocumentSeed};
use crate::office::pptx::error::DocError;
use crate::office::pptx::fonts::FontSystem;
use crate::office::pptx::layers::LayerCache;
use crate::office::pptx::render::{ImageCache, ImageStore};

/// Render threads when neither the config (`workers`) nor `PPTX_READ_WORKERS`
/// sets a count. See [`resolve_workers`] for why it is small.
pub const DEFAULT_WORKERS: usize = 4;

/// Stack of each render thread, as large as the main thread's: nested groups
/// and SmartArt recurse deeply, and the sequential loop runs on the main thread.
const STACK_BYTES: usize = 8 * 1024 * 1024;

/// Render threads for a document of `pages` pages.
pub fn worker_count(configured: Option<u32>, pages: usize) -> usize {
    let env = std::env::var("PPTX_READ_WORKERS").ok();
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    resolve_workers(configured, env.as_deref(), cores, pages)
}

/// The config's `workers` wins, then the environment, then the default capped
/// by the cores this process may use (`available_parallelism` follows cgroup
/// CPU quotas). The default stays small because a worker pod can run several
/// documents at once and each thread adds one slide's working set to memory.
/// Never more threads than pages; 0 or 1 means the sequential loop.
pub fn resolve_workers(
    configured: Option<u32>,
    env: Option<&str>,
    cores: usize,
    pages: usize,
) -> usize {
    let n = configured
        .map(|w| w as usize)
        .or_else(|| env.and_then(|v| v.trim().parse().ok()))
        .unwrap_or_else(|| DEFAULT_WORKERS.min(cores));
    n.clamp(1, pages.max(1))
}

/// Estimated memory of the pages in the threads' hands at once (see
/// `page_weights` in `extract.rs`): a deck of 10 MB slides or of slides with
/// their own large pictures builds them about one at a time, as the sequential
/// loop does, while ordinary slides never wait.
pub const IN_FLIGHT_BYTES: u64 = 192 * 1024 * 1024;

/// Pages a thread may claim ahead of the oldest page not yet written. Built
/// records wait in memory until written (their screenshots are already on
/// disk), so this bounds them when one slide is much slower than the rest.
fn window(workers: usize) -> usize {
    workers * 2
}

/// Everything the page loop needs from `run_read`.
pub struct Job<'a> {
    pub doc: &'a Document,
    /// Font system the threads' copies are made from; their substitutions are
    /// merged back into it.
    pub fonts: &'a FontSystem,
    pub images: &'a Arc<ImageStore>,
    pub layers: &'a LayerCache,
    pub cfg: &'a Config,
    /// (page, slide index) in output order.
    pub pages: &'a [(u32, usize)],
    /// Parts each page reaches, when pictures leave the store after their last use.
    pub reach: Option<Vec<HashSet<String>>>,
    /// Estimated memory each page needs (see [`IN_FLIGHT_BYTES`]).
    pub weights: Vec<u64>,
    pub started: Instant,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Which pages are claimed and written, shared by the threads.
struct Progress {
    /// Next position to claim.
    next: usize,
    /// Positions below this one are written.
    written: usize,
    /// Sum of the weights of pages claimed and not yet built.
    in_flight: u64,
}

/// What the render threads share (the document and font system stay on the
/// calling thread: each thread has its own).
struct Shared<'a> {
    images: &'a Arc<ImageStore>,
    layers: &'a LayerCache,
    cfg: &'a Config,
    pages: &'a [(u32, usize)],
    weights: &'a [u64],
    started: Instant,
    window: usize,
    progress: Mutex<Progress>,
    changed: Condvar,
    /// Set on a write error, a dead thread or the time budget: claim nothing more.
    stop: AtomicBool,
    timed_out: AtomicBool,
    releases: Option<Releases>,
}

impl Shared<'_> {
    /// The next position to build, waiting while it is a full window ahead of
    /// the writer or while its XML does not fit next to the pages being built
    /// (a page always fits when none is). `None` once every page is claimed or
    /// the loop stops.
    fn claim(&self) -> Option<usize> {
        let mut p = lock(&self.progress);
        loop {
            if self.stop.load(Ordering::SeqCst) || p.next >= self.pages.len() {
                return None;
            }
            let weight = self.weights.get(p.next).copied().unwrap_or(0);
            let fits = p.in_flight == 0 || p.in_flight + weight <= IN_FLIGHT_BYTES;
            if p.next < p.written + self.window && fits {
                p.next += 1;
                p.in_flight += weight;
                return Some(p.next - 1);
            }
            p = self
                .changed
                .wait(p)
                .unwrap_or_else(|poison| poison.into_inner());
        }
    }

    /// Page `pos` is built and its trees dropped: its weight is free again.
    fn built(&self, pos: usize) {
        let mut p = lock(&self.progress);
        p.in_flight = p
            .in_flight
            .saturating_sub(self.weights.get(pos).copied().unwrap_or(0));
        self.changed.notify_all();
    }

    fn halt(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let _guard = lock(&self.progress);
        self.changed.notify_all();
    }
}

/// Stops the other threads if its owner unwinds, so none waits forever for a
/// page that will never be written.
struct HaltOnUnwind<'s, 'a>(&'s Shared<'a>);

impl Drop for HaltOnUnwind<'_, '_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.halt();
        }
    }
}

/// Pictures leave the shared store once every page that reaches them is built:
/// the parallel form of releasing after the last use.
pub(super) struct Releases {
    reach: Vec<HashSet<String>>,
    /// Pages still to build per part, and the parts no page needs any more.
    state: Mutex<(HashMap<String, usize>, HashSet<String>)>,
}

impl Releases {
    pub(super) fn new(reach: Vec<HashSet<String>>) -> Releases {
        let mut left: HashMap<String, usize> = HashMap::new();
        for parts in &reach {
            for part in parts {
                *left.entry(part.clone()).or_default() += 1;
            }
        }
        Releases {
            reach,
            state: Mutex::new((left, HashSet::new())),
        }
    }

    /// Records that page `pos` is built and drops the pictures now unneeded.
    pub(super) fn page_built(&self, pos: usize, store: &ImageStore) {
        let mut state = lock(&self.state);
        let (left, done) = &mut *state;
        for part in &self.reach[pos] {
            if let Some(n) = left.get_mut(part) {
                *n -= 1;
                if *n == 0 {
                    done.insert(part.clone());
                }
            }
        }
        store.release(|part| done.contains(part));
    }
}

/// Builds and writes every page with `workers` threads. `Ok(false)` when the
/// document's time budget ran out.
pub(super) fn write_pages(
    job: Job<'_>,
    workers: usize,
    out: &mut PageWriter,
) -> Result<bool, String> {
    // Map every font file once so that the threads' copies share the mappings.
    job.fonts.share_files();
    let forks: Vec<FontSystem> = (0..workers).map(|_| job.fonts.fork()).collect();
    let releases = job.reach.clone().map(Releases::new);
    let shared = Shared {
        images: job.images,
        layers: job.layers,
        cfg: job.cfg,
        pages: job.pages,
        weights: &job.weights,
        started: job.started,
        window: window(workers),
        progress: Mutex::new(Progress {
            next: 0,
            written: 0,
            in_flight: 0,
        }),
        changed: Condvar::new(),
        stop: AtomicBool::new(false),
        timed_out: AtomicBool::new(false),
        releases,
    };
    let (tx, rx) = mpsc::channel::<(usize, Queued)>();
    let mut written = 0usize;
    let mut result = Ok(());
    std::thread::scope(|scope| -> Result<(), String> {
        let _halt = HaltOnUnwind(&shared);
        let mut handles = Vec::with_capacity(workers);
        for (n, fonts) in forks.into_iter().enumerate() {
            let (tx, seed, shared) = (tx.clone(), job.doc.seed(), &shared);
            let spawned = std::thread::Builder::new()
                .name(format!("pptx-render-{n}"))
                .stack_size(STACK_BYTES)
                .spawn_scoped(scope, move || render_thread(shared, seed, fonts, tx));
            match spawned {
                Ok(h) => handles.push(h),
                Err(e) => {
                    // Threads already started must not wait for a writer that never runs.
                    shared.halt();
                    return Err(format!("cannot start a render thread: {e}"));
                }
            }
        }
        drop(tx);
        // Built pages arrive in any order; write them in page order.
        let mut pending: BTreeMap<usize, Queued> = BTreeMap::new();
        for (pos, queued) in rx {
            pending.insert(pos, queued);
            while let Some(q) = pending.remove(&written) {
                if result.is_ok() {
                    result = out.commit(q, None);
                }
                written += 1;
            }
            if result.is_err() {
                shared.halt();
            }
            lock(&shared.progress).written = written;
            shared.changed.notify_all();
        }
        for h in handles {
            match h.join() {
                Ok(subs) => job.fonts.substituted.borrow_mut().extend(subs),
                Err(p) if result.is_ok() => {
                    result = Err(format!("render thread failed: {}", panic_message(&p)));
                }
                Err(_) => {}
            }
        }
        Ok(())
    })?;
    result?;
    if shared.timed_out.load(Ordering::SeqCst) {
        return Ok(false);
    }
    if written < job.pages.len() {
        return Err(format!(
            "render threads stopped after {written} of {} pages",
            job.pages.len()
        ));
    }
    Ok(true)
}

/// One render thread: claims pages until none are left. Returns the font
/// substitutions it made, for the document's warnings.
fn render_thread(
    shared: &Shared<'_>,
    seed: DocumentSeed,
    fonts: FontSystem,
    tx: mpsc::Sender<(usize, Queued)>,
) -> BTreeSet<(String, String)> {
    let _halt = HaltOnUnwind(shared);
    let cfg: &Config = shared.cfg;
    let Ok(doc) = seed.open() else {
        return BTreeSet::new();
    };
    let images = ImageCache::with_store(shared.images.clone(), cfg.limits.max_image_pixels, &fonts);
    while let Some(pos) = shared.claim() {
        if shared.started.elapsed().as_millis() as u64 > cfg.render.document_budget_ms {
            shared.timed_out.store(true, Ordering::SeqCst);
            shared.halt();
            break;
        }
        let (page, slide_idx) = shared.pages[pos];
        // A panic while building one slide becomes a page error, not a failed document.
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            page_record(&doc, &fonts, &images, shared.layers, cfg, page, slide_idx)
        }))
        .unwrap_or_else(|p| Err(DocError::Broken(format!("panic: {}", panic_message(&p)))));
        doc.release_slide_parts();
        shared.built(pos);
        if let Some(r) = &shared.releases {
            r.page_built(pos, shared.images);
        }
        let built = built.map(|(record, pending)| {
            let shot = match pending {
                None => Shot::None,
                Some(p) => Shot::Done(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    move || p.finish(),
                ))),
            };
            (record, shot)
        });
        if tx.send((pos, (page, slide_idx, built))).is_err() {
            break;
        }
    }
    fonts.substituted.take()
}

#[cfg(test)]
mod tests;
