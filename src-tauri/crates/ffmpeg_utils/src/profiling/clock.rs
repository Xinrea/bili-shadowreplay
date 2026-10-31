//! Optional diagnostic clocks. These never run while the collector is disabled.

use std::time::Duration;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn clock_gettime_nsec_np(clock: u32) -> u64;
    fn pthread_self() -> *mut std::ffi::c_void;
    fn pthread_get_qos_class_np(
        thread: *mut std::ffi::c_void,
        class: *mut u32,
        relative: *mut i32,
    ) -> i32;
}

pub(super) fn thread_cpu() -> Option<Duration> {
    cpu(16) // Darwin CLOCK_THREAD_CPUTIME_ID, available since macOS 10.12.
}
pub(super) fn process_cpu() -> Option<Duration> {
    cpu(12) // Darwin CLOCK_PROCESS_CPUTIME_ID.
}
fn cpu(clock: u32) -> Option<Duration> {
    #[cfg(target_os = "macos")]
    {
        let value = unsafe { clock_gettime_nsec_np(clock) };
        (value != 0).then(|| Duration::from_nanos(value))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = clock;
        None
    }
}

pub(super) fn requested_qos() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let mut relative = 0;
        let mut class = 0;
        let status = unsafe { pthread_get_qos_class_np(pthread_self(), &mut class, &mut relative) };
        // This API reports requested QoS, not effective overrides or core placement.
        (status == 0).then(|| format!("class={class}, relative_priority={relative}"))
    }
    #[cfg(not(target_os = "macos"))]
    None
}
