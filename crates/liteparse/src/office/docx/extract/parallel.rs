//! The page loop spread over render threads. Layout is done before, on the
//! calling thread; the threads only turn laid-out pages into records: text
//! items, image records and screenshots (rendered and encoded in the thread).
//!
//! Scene work uses single-threaded caches (`RefCell` in the document shell and
//! the font system), so each thread has its own document shell and font system
//! fork; the zip package, the decoded-picture store and the layer cache are
//! shared. Threads claim pages from one counter, at most a window ahead of the
//! oldest page not yet written, and the calling thread writes records strictly
//! in page order: the output is byte-identical whatever the thread count.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Condvar, Mutex};
use std::time::Instant;

use crate::office::pptx::config::Config;
use crate::office::pptx::document::Document;
use crate::office::pptx::error::DocError;
use crate::office::pptx::fonts::FontSystem;
use crate::office::pptx::render::ImageCache;

use super::{PageWriter, Ready, Shot, finish_record, page_record, panic_message};
use crate::office::docx::paginate::OutPage;

/// Render threads when neither the config (`workers`) nor `DOCX_READ_WORKERS` sets a count.
pub const DEFAULT_WORKERS: usize = 4;

/// Stack of each render thread, as large as the main thread's.
const STACK_BYTES: usize = 8 * 1024 * 1024;

/// Render threads for `pages` pages: the config, then the environment, then
/// the default capped by the cores available; never more than the pages.
pub fn worker_count(configured: Option<u32>, pages: usize) -> usize {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let n = configured
        .map(|w| w as usize)
        .or_else(|| {
            std::env::var("DOCX_READ_WORKERS")
                .ok()
                .and_then(|v| v.trim().parse().ok())
        })
        .unwrap_or_else(|| DEFAULT_WORKERS.min(cores));
    n.clamp(1, pages.max(1))
}

/// Everything the page loop needs from `run_read`.
pub struct Job<'a> {
    pub pkg: &'a std::sync::Arc<crate::office::pptx::package::Package>,
    /// Document shell size (EMU), as the calling thread's.
    pub size: (i64, i64),
    pub fonts: &'a FontSystem,
    pub images: &'a ImageCache<'a>,
    pub layers: &'a crate::office::pptx::layers::LayerCache,
    pub cfg: &'a Config,
    pub pages: &'a [OutPage],
    pub bookmarks: &'a super::bookmarks::Index,
    pub equations: &'a std::collections::HashSet<String>,
    pub selected: &'a [u32],
    pub started: Instant,
}

fn lock(m: &Mutex<Progress>) -> std::sync::MutexGuard<'_, Progress> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

struct Progress {
    next: usize,
    written: usize,
}

/// Renders the selected pages on `workers` threads and writes them in order.
/// `Ok(false)` when the document time budget ran out.
pub fn run(job: &Job, workers: usize, out: &mut PageWriter) -> Result<bool, String> {
    job.fonts.share_files();
    let forks: Vec<FontSystem> = (0..workers).map(|_| job.fonts.fork()).collect();
    let window = workers * 2;
    let progress = Mutex::new(Progress {
        next: 0,
        written: 0,
    });
    let changed = Condvar::new();
    let stop = AtomicBool::new(false);
    let started = job.started;
    let equations = job.equations;
    let budget = job.cfg.render.document_budget_ms;
    let over_budget = move || started.elapsed().as_millis() as u64 > budget;
    // What the threads share (the font system and image cache stay here).
    let store = job.images.store().clone();
    let bookmarks = job.bookmarks;
    let (pkg, size, layers, cfg, pages, selected) = (
        job.pkg,
        job.size,
        job.layers,
        job.cfg,
        job.pages,
        job.selected,
    );
    let (tx, rx) = mpsc::channel::<(usize, Result<Ready, String>)>();

    std::thread::scope(|scope| -> Result<bool, String> {
        let mut handles = Vec::new();
        for fonts in forks {
            let tx = tx.clone();
            let (progress, changed, stop, store) = (&progress, &changed, &stop, &store);
            let h = std::thread::Builder::new()
                .stack_size(STACK_BYTES)
                .spawn_scoped(scope, move || {
                    let doc = Document::from_package(pkg.clone(), size.0, size.1);
                    let images =
                        ImageCache::with_store(store.clone(), cfg.limits.max_image_pixels, &fonts);
                    loop {
                        let pos = {
                            let mut g = lock(progress);
                            while g.next < selected.len()
                                && g.next >= g.written + window
                                && !stop.load(Ordering::Relaxed)
                            {
                                g = changed.wait(g).unwrap_or_else(|p| p.into_inner());
                            }
                            if g.next >= selected.len() || stop.load(Ordering::Relaxed) {
                                break;
                            }
                            g.next += 1;
                            g.next - 1
                        };
                        if over_budget() {
                            stop.store(true, Ordering::Relaxed);
                            changed.notify_all();
                            break;
                        }
                        let page = selected[pos];
                        let p = &pages[page as usize];
                        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            page_record(
                                &doc,
                                &fonts,
                                &images,
                                layers,
                                cfg,
                                page,
                                (p, bookmarks, equations),
                            )
                        }))
                        .map_err(|e| DocError::Broken(format!("panic: {}", panic_message(&e))))
                        .map(|(record, pending)| {
                            let shot = match pending {
                                None => Shot::None,
                                Some(s) => Shot::Done(std::panic::catch_unwind(
                                    std::panic::AssertUnwindSafe(move || s.finish()),
                                )),
                            };
                            (record, shot)
                        });
                        let ready = finish_record((page, (p.info.w, p.info.h), built), None);
                        if tx.send((pos, ready)).is_err() {
                            break;
                        }
                    }
                    fonts.substituted.take()
                })
                .map_err(|e| e.to_string())?;
            handles.push(h);
        }
        drop(tx);
        let mut ready: BTreeMap<usize, Result<Ready, String>> = BTreeMap::new();
        let mut next = 0;
        let mut failed = None;
        for (pos, q) in rx.iter() {
            ready.insert(pos, q);
            while let Some(q) = ready.remove(&next) {
                if failed.is_none()
                    && let Err(e) = q.and_then(|r| out.write(r))
                {
                    failed = Some(e);
                    stop.store(true, Ordering::Relaxed);
                }
                next += 1;
                lock(&progress).written = next;
                changed.notify_all();
            }
        }
        for h in handles {
            match h.join() {
                Ok(subs) => job.fonts.substituted.borrow_mut().extend(subs),
                Err(e) => {
                    failed.get_or_insert(format!("render thread: {}", panic_message(&e)));
                }
            }
        }
        if let Some(e) = failed {
            return Err(e);
        }
        Ok(next == job.selected.len())
    })
}
