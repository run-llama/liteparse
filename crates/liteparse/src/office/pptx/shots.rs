//! Screenshot encoding on a helper thread.
//!
//! Rendering needs the document and its single-threaded caches, but JPEG/PNG
//! encoding and solid-rectangle detection only need the finished pixmap. One
//! helper thread runs `PendingShot::finish` for page N while the main thread
//! builds and renders page N+1. Jobs are finished in submission order, and the
//! hand-off is a rendezvous, so at most two rendered pages are alive at once.

use std::sync::mpsc::{Receiver, SyncSender, channel, sync_channel};
use std::thread::{Scope, ScopedJoinHandle};

use serde_json::Value;

use crate::office::pptx::output::PendingShot;

/// `finish`'s result, or the panic payload if it panicked.
pub type Finished = std::thread::Result<Option<(Value, String)>>;

pub struct ShotWorker<'scope> {
    jobs: Option<SyncSender<PendingShot>>,
    results: Receiver<Finished>,
    handle: Option<ScopedJoinHandle<'scope, ()>>,
}

impl<'scope> ShotWorker<'scope> {
    pub fn spawn<'env>(scope: &'scope Scope<'scope, 'env>) -> ShotWorker<'scope> {
        let (jobs, job_rx) = sync_channel::<PendingShot>(0);
        let (result_tx, results) = channel();
        let handle = scope.spawn(move || {
            for job in job_rx {
                let done =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || job.finish()));
                if result_tx.send(done).is_err() {
                    break;
                }
            }
        });
        ShotWorker {
            jobs: Some(jobs),
            results,
            handle: Some(handle),
        }
    }

    /// Queues a page; blocks while the helper is still busy with the previous one.
    /// Hands the page back if the helper is gone (the caller then finishes inline).
    pub fn submit(&self, shot: PendingShot) -> Result<(), Box<PendingShot>> {
        match &self.jobs {
            Some(tx) => tx.send(shot).map_err(|e| Box::new(e.0)),
            None => Err(Box::new(shot)),
        }
    }

    /// The result of the oldest submitted page not yet collected.
    pub fn next(&self) -> Option<Finished> {
        self.results.recv().ok()
    }
}

impl Drop for ShotWorker<'_> {
    fn drop(&mut self) {
        // Closing the job channel ends the helper's loop.
        self.jobs.take();
        if let Some(h) = self.handle.take() {
            drop(h.join());
        }
    }
}
