//! Heap allocation counter for benchmarks (`--features alloc-count`).
//!
//! The binary installs [`Counting`] as its global allocator when the feature
//! is on; [`allocations`] then reports how many allocations (and reallocations)
//! the process has made. Without the feature it reports nothing and costs
//! nothing. Used by `scenarios/opcua-bench` to put a number on allocations per
//! delivered value.

#[cfg(feature = "alloc-count")]
mod imp {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicU64, Ordering};

    pub(super) static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

    /// The system allocator, counting calls.
    pub struct Counting;

    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            unsafe { System.alloc(layout) }
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            unsafe { System.alloc_zeroed(layout) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { System.dealloc(ptr, layout) }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }
}

#[cfg(feature = "alloc-count")]
pub use imp::Counting;

/// Allocations made so far, when the counting allocator is installed.
pub fn allocations() -> Option<u64> {
    #[cfg(feature = "alloc-count")]
    {
        Some(imp::ALLOCATIONS.load(std::sync::atomic::Ordering::Relaxed))
    }
    #[cfg(not(feature = "alloc-count"))]
    {
        None
    }
}
