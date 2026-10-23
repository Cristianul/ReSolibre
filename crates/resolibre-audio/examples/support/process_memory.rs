//! Application-owned GNU allocator policy; never called implicitly by LocalPlayer.

unsafe extern "C" {
    /// Sets one glibc allocator parameter and returns nonzero on success.
    fn mallopt(parameter: i32, value: i32) -> i32;
}

/// Keeps large freed allocations returnable instead of raising glibc's threshold.
/// This affects the entire process for its remaining lifetime, without imposing
/// a hard memory cap. Call once at application startup, before creating threads.
///
/// # Safety
/// The caller must own process initialization: glibc documents mallopt as
/// MT-Unsafe. No other thread may be running during this call.
pub unsafe fn configure() -> Result<(), &'static str> {
    // GNU malloc.h: M_MMAP_THRESHOLD = -3. A fixed 128 KiB threshold disables
    // dynamic growth; retain default arena/thread behavior and audio buffers.
    // SAFETY: the caller guarantees exclusive, single-threaded startup.
    let result = unsafe { mallopt(-3, 128 * 1024) };
    if result == 0 {
        return Err("process memory configuration failed");
    }

    Ok(())
}
