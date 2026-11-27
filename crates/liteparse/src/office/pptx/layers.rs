//! Reuse of the rendered master/layout layers across slides.
//!
//! A scene draws the background, then master and layout shapes, then the
//! slide's own shapes. Slides sharing a layout start with the same items, so
//! the pixmap after that shared prefix is the same too. The cache keeps that
//! pixmap and the next slide resumes drawing from it. Rendering is
//! deterministic, so the result is byte-identical to drawing every item.
//!
//! Items are compared by a 128-bit hash of their `Debug` form (floats print in
//! round-trip form, so equal text means equal values). A snapshot is only
//! taken where no clip is pushed, so resuming needs no mask state.

use std::fmt::Write as _;
use std::sync::{Mutex, MutexGuard};

use tiny_skia::Pixmap;
use xxhash_rust::xxh3::Xxh3;

use crate::office::pptx::scene::{Item, Paint, Scene};

/// Snapshots kept at once, and their total size cap.
const MAX_ENTRIES: usize = 4;
const MAX_BYTES: usize = 48 * 1024 * 1024;

/// Render target identity: pixel size and the slide region, bit for bit.
pub type TargetKey = (u32, u32, [u64; 4]);

struct Entry {
    target: TargetKey,
    /// Hashes of the items drawn into `pm`, in order.
    hashes: Vec<u128>,
    pm: Pixmap,
}

/// Shared by the render threads of a document. Which slide planned last only
/// steers what gets cached; any hit is the exact pixmap, so the output does not
/// depend on the order threads reach the cache.
#[derive(Default)]
pub struct LayerCache {
    /// Most recently used last.
    entries: Mutex<Vec<Entry>>,
    /// Prefix hashes of the previous slide planned.
    prev: Mutex<Vec<u128>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// How to render one scene: start from `base` (or blank) at item `start`, and
/// hand the pixmap back to the cache before drawing item `snapshot_at`.
pub struct Plan {
    pub start: usize,
    pub base: Option<Pixmap>,
    pub snapshot_at: Option<usize>,
    target: TargetKey,
    hashes: Vec<u128>,
    /// Entry made obsolete by a shorter shared snapshot, by its item hashes
    /// (another thread may have moved it by the time this plan stores).
    replaces: Option<Vec<u128>>,
}

struct HashWriter(Xxh3);

impl std::fmt::Write for HashWriter {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.update(s.as_bytes());
        Ok(())
    }
}

fn item_hash(item: &Item) -> u128 {
    let mut w = HashWriter(Xxh3::new());
    let _ = write!(w, "{item:?}");
    w.0.digest128()
}

/// A picture: drawn by sampling every covered pixel, the costliest item to redraw.
fn is_picture(item: &Item) -> bool {
    match item {
        Item::Image(_) => true,
        Item::Path(p) => matches!(p.fill, Some(Paint::Image { .. })),
        _ => false,
    }
}

/// Worth a snapshot of its own when a slide extends a cached prefix with it.
fn is_costly(item: &Item) -> bool {
    is_picture(item) || matches!(item, Item::Shadow(_) | Item::PushSoftEdge(_))
}

/// The leading items a snapshot may cover: those inherited from the master and
/// layout, then the slide's own pictures (decks often open every slide with the
/// same one), up to its first other shape. Returns their hashes, for each
/// position `0..=len` whether the clip stack is empty there, and how many lead
/// before the slide's first own item.
fn prefix(scene: &Scene) -> (Vec<u128>, Vec<bool>, usize) {
    let mut hashes = Vec::new();
    let mut safe = vec![true];
    let mut depth = 0usize;
    let mut inherited = None;
    for item in &scene.items {
        let shape = match item {
            Item::Path(p) | Item::Multiply(p) => p.shape,
            Item::Image(i) => i.shape,
            Item::Text(t) => t.shape,
            _ => None,
        };
        let own = shape.is_some_and(|s| scene.shapes.get(s).is_none_or(|sh| !sh.inherited));
        if own && !is_picture(item) {
            break;
        }
        if own && inherited.is_none() {
            inherited = Some(hashes.len());
        }
        match item {
            Item::PushClip(_) | Item::PushSoftEdge(_) => depth += 1,
            Item::PopClip | Item::PopSoftEdge => depth = depth.saturating_sub(1),
            _ => {}
        }
        hashes.push(item_hash(item));
        safe.push(depth == 0);
    }
    let inherited = inherited.unwrap_or(hashes.len());
    (hashes, safe, inherited)
}

fn common(a: &[u128], b: &[u128]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

impl LayerCache {
    pub fn plan(&self, scene: &Scene, target: TargetKey) -> Plan {
        let (hashes, safe, inherited) = prefix(scene);
        // Items past the inherited ones are snapshot only once the previous slide
        // drew them too: a one-off picture would just evict useful entries.
        let repeated = common(
            &std::mem::replace(&mut *lock(&self.prev), hashes.clone()),
            &hashes,
        );
        let last_safe = |upto: usize| safe[..=upto.min(hashes.len())].iter().rposition(|s| *s);
        let mut entries = lock(&self.entries);
        let mut hit: Option<usize> = None;
        let mut shared: Option<(usize, usize)> = None;
        for (i, e) in entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.target == target)
        {
            let c = common(&e.hashes, &hashes);
            if c == e.hashes.len() {
                if hit.is_none_or(|h| entries[h].hashes.len() < c) {
                    hit = Some(i);
                }
            } else if c > 0 && safe[c] && shared.is_none_or(|(_, best)| best < c) {
                shared = Some((i, c));
            }
        }
        let start = hit.map_or(0, |h| entries[h].hashes.len());
        // Snapshot where this slide stops matching an older one (the part they
        // share), else at the end of its inherited (or repeated) prefix. A hit
        // shorter than the repeated part is extended when the rest is costly.
        let (snapshot_at, replaces) = match shared {
            Some((i, c)) if c > start => (Some(c), Some(i)),
            _ if hit.is_none() => (last_safe(inherited.max(repeated)).filter(|p| *p > 0), None),
            _ => (
                last_safe(repeated)
                    .filter(|p| *p > start && scene.items[start..*p].iter().any(is_costly)),
                None,
            ),
        };
        let base = hit.map(|h| entries[h].pm.clone());
        let replaces = replaces.map(|i| entries[i].hashes.clone());
        if let Some(h) = hit {
            // Most recently used goes last.
            let e = entries.remove(h);
            entries.push(e);
        }
        drop(entries);
        Plan {
            start,
            base,
            snapshot_at,
            target,
            hashes,
            replaces,
        }
    }

    /// Keeps `pm`, the pixmap after drawing `plan.snapshot_at` items.
    pub fn store(&self, plan: &Plan, pm: &Pixmap) {
        let Some(at) = plan.snapshot_at else {
            return;
        };
        if pm.data().len() > MAX_BYTES / 2 {
            return;
        }
        let mut entries = lock(&self.entries);
        if let Some(old) = &plan.replaces {
            entries.retain(|e| e.target != plan.target || e.hashes != *old);
        }
        entries.push(Entry {
            target: plan.target,
            hashes: plan.hashes[..at].to_vec(),
            pm: pm.clone(),
        });
        let bytes = |es: &Vec<Entry>| es.iter().map(|e| e.pm.data().len()).sum::<usize>();
        while entries.len() > 1 && (entries.len() > MAX_ENTRIES || bytes(&entries) > MAX_BYTES) {
            entries.remove(0);
        }
    }
}

#[cfg(test)]
mod tests;
