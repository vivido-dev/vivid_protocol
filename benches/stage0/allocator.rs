use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static TRACKING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);

pub struct CountingAllocator;

// SAFETY: every operation delegates to the process System allocator with the original pointer and
// layout. The additional atomics only observe successful allocation requests.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: delegated with the caller-provided layout.
        let pointer = unsafe { System.alloc(layout) };
        record(pointer, layout.size());
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: delegated with the caller-provided layout.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        record(pointer, layout.size());
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: delegated with the original pointer and layout.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: delegated with the original pointer, layout, and requested size.
        let new_pointer = unsafe { System.realloc(pointer, layout, new_size) };
        record(new_pointer, new_size);
        new_pointer
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AllocationSnapshot {
    pub allocations: u64,
    pub allocated_bytes: u64,
}

pub fn measure<T>(operation: impl FnOnce() -> T) -> (T, AllocationSnapshot) {
    TRACKING.store(false, Ordering::SeqCst);
    ALLOCATIONS.store(0, Ordering::SeqCst);
    ALLOCATED_BYTES.store(0, Ordering::SeqCst);
    TRACKING.store(true, Ordering::SeqCst);
    let result = operation();
    TRACKING.store(false, Ordering::SeqCst);
    (
        result,
        AllocationSnapshot {
            allocations: ALLOCATIONS.load(Ordering::SeqCst),
            allocated_bytes: ALLOCATED_BYTES.load(Ordering::SeqCst),
        },
    )
}

fn record(pointer: *mut u8, bytes: usize) {
    if !pointer.is_null() && TRACKING.load(Ordering::Relaxed) {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        ALLOCATED_BYTES.fetch_add(u64::try_from(bytes).unwrap_or(u64::MAX), Ordering::Relaxed);
    }
}
