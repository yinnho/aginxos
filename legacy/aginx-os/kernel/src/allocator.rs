//! Kernel heap allocator — simple single-threaded bump allocator

use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;

struct BumpAllocator {
    next: Cell<usize>,
    end: Cell<usize>,
}

unsafe impl Sync for BumpAllocator {}
unsafe impl Send for BumpAllocator {}

unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let next = self.next.get();
        let align = layout.align();
        let alloc_start = (next + align - 1) & !(align - 1);
        let alloc_end = alloc_start + layout.size();
        if alloc_end > self.end.get() {
            return core::ptr::null_mut();
        }
        self.next.set(alloc_end);
        alloc_start as *mut u8
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // Bump allocator does not free
    }
}

#[global_allocator]
static ALLOCATOR: BumpAllocator = BumpAllocator {
    next: Cell::new(0),
    end: Cell::new(0),
};

/// Initialize the heap allocator.
pub fn init(heap_start: usize, heap_size: usize) {
    ALLOCATOR.next.set(heap_start);
    ALLOCATOR.end.set(heap_start + heap_size);
}
