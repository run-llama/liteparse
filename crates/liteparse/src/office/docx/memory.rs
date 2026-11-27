//! Process memory reading for the index's `peakRssBytes`.

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
