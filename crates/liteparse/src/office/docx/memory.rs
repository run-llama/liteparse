//! Process memory cap. A watchdog thread compares the process's peak resident
//! memory with `limits.maxRssBytes` and exits with [`EXIT_MEMORY_LIMIT`] once it
//! is over, so one document cannot take the memory of the worker pod that runs
//! it. The worker maps that exit code to its `memory_limit` fallback reason.

use std::time::Duration;

/// Exit code of a run stopped by the memory cap (1 and 2 are taken by `main`).
pub const EXIT_MEMORY_LIMIT: i32 = 3;

/// How often the watchdog samples. Allocation between two samples is the
/// overshoot past the cap, so the cap leaves room below the pod limit for it.
const POLL: Duration = Duration::from_millis(50);

/// Starts the watchdog; `limit_bytes == 0` means no cap.
#[cfg(unix)]
pub fn start_guard(limit_bytes: u64) {
    if limit_bytes == 0 {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("rss-guard".into())
        .stack_size(64 * 1024)
        .spawn(move || loop {
            if let Some(peak) = peak_rss_bytes()
                && peak > limit_bytes {
                    eprintln!(
                        "docx-read: MEMORY_LIMIT_EXCEEDED peak_rss_bytes={peak} limit_bytes={limit_bytes}"
                    );
                    // `_exit` skips atexit handlers and stdio flushing, which other
                    // threads may be inside of; stderr is unbuffered.
                    // SAFETY: `_exit` only terminates the process.
                    unsafe { libc::_exit(EXIT_MEMORY_LIMIT) };
                }
            std::thread::sleep(POLL);
        });
    if let Err(err) = spawned {
        eprintln!("docx-read: memory guard not started: {err}");
    }
}

#[cfg(not(unix))]
pub fn start_guard(_limit_bytes: u64) {}

/// Peak resident set size of this process. On Linux `getrusage` reports the
/// larger of the recorded high-water mark and the current RSS, so it is live.
#[cfg(unix)]
pub fn peak_rss_bytes() -> Option<u64> {
    // SAFETY: `rusage` is plain C data, so all-zero bytes are a valid value.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: getrusage writes into the zeroed struct we own.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &raw mut usage) } != 0 {
        return None;
    }
    let max = usage.ru_maxrss as u64;
    Some(if cfg!(target_os = "macos") {
        max
    } else {
        max * 1024
    })
}

#[cfg(not(unix))]
pub fn peak_rss_bytes() -> Option<u64> {
    None
}
