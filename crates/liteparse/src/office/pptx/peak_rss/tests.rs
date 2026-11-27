use super::*;

#[test]
fn reads_vm_hwm_in_bytes() {
    let status =
        "Name:\tliteparse\nVmPeak:\t  900000 kB\nVmHWM:\t   51200 kB\nVmRSS:\t   40000 kB\n";
    assert_eq!(vm_hwm_bytes(status), Some(51200 * 1024));
}

#[test]
fn missing_vm_hwm_is_none() {
    assert_eq!(vm_hwm_bytes("Name:\tx\nVmRSS:\t 1 kB\n"), None);
}

#[cfg(target_os = "linux")]
#[test]
fn reads_this_process_on_linux() {
    assert!(own_peak_rss_bytes().is_some_and(|bytes| bytes > 0));
}
