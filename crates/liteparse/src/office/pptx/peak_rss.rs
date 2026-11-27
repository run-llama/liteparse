/// Peak resident set size of this process in bytes: `VmHWM` from
/// `/proc/self/status`. `None` where `/proc` is absent (macOS).
///
/// VmHWM belongs to the address space, which exec replaces, so a child spawned by a
/// large process reads only its own peak. Linux `getrusage` `ru_maxrss` survives
/// fork+exec and reads max(the spawner's RSS at the spawn, the child's own peak).
pub fn own_peak_rss_bytes() -> Option<u64> {
    vm_hwm_bytes(&std::fs::read_to_string("/proc/self/status").ok()?)
}

fn vm_hwm_bytes(status: &str) -> Option<u64> {
    let value = status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))?;
    let kb: u64 = value.trim().strip_suffix(" kB")?.trim().parse().ok()?;
    Some(kb * 1024)
}

#[cfg(test)]
mod tests;
