//! A dedicated single-threaded process tests the real application startup policy.
//! A libtest test function would already run on a worker thread.

#[cfg(target_env = "gnu")]
#[path = "../examples/support/process_memory.rs"]
mod process_memory;

/// glibc mallinfo2 fields, in malloc.h order; counters expose no private addresses.
#[cfg(target_env = "gnu")]
#[repr(C)]
struct AllocatorInfo {
    arena: usize,
    free_chunks: usize,
    fast_chunks: usize,
    mapped_chunks: usize,
    mapped_bytes: usize,
    unused: usize,
    fast_free_bytes: usize,
    allocated_bytes: usize,
    free_bytes: usize,
    releasable_top_bytes: usize,
}

#[cfg(target_env = "gnu")]
unsafe extern "C" {
    /// Returns allocator counters by value without retaining caller state.
    fn mallinfo2() -> AllocatorInfo;
}

/// Repeated large frees must not raise the threshold and retain later allocations.
#[cfg(target_env = "gnu")]
fn main() {
    // SAFETY: harness=false gives this test its own main with no worker threads.
    unsafe { process_memory::configure() }.expect("startup policy rejected");

    // Without a fixed threshold, freeing the large block trains glibc to keep
    // subsequent 1 MiB allocations in its heap. Check reclamation on every cycle.
    for _ in 0..10 {
        let large = vec![1_u8; 4 * 1024 * 1024];
        std::hint::black_box(&large);
        drop(large);

        // SAFETY: this test links GNU libc and matches its by-value C layout.
        let before = unsafe { mallinfo2() }.mapped_bytes;
        let small = vec![2_u8; 1024 * 1024];
        std::hint::black_box(&small);
        // SAFETY: same glibc ABI as above; no pointers are exchanged.
        let during = unsafe { mallinfo2() }.mapped_bytes;
        assert!(
            during >= before + 1024 * 1024,
            "large allocation retained in heap"
        );
        drop(small);
        // SAFETY: same glibc ABI as above; no pointers are exchanged.
        let after = unsafe { mallinfo2() }.mapped_bytes;
        assert_eq!(after, before, "freed allocation still mapped");
    }

    println!("PASS: application allocator policy releases repeated large allocations");
}

/// Other Linux libc implementations do not use the GNU-specific startup policy.
#[cfg(not(target_env = "gnu"))]
fn main() {
    println!("SKIP: GNU allocator policy does not apply");
}
