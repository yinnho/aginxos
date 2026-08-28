//! Physical page frame allocator (bitmap-based)
//!
//! Manages 4KB physical page frames using a bitmap.
//! For 512MB RAM: 131072 frames, 16KB bitmap.

const PAGE_SIZE: usize = 4096;

#[cfg(not(feature = "board-redfin"))]
const RAM_START: usize = 0x4000_0000;
#[cfg(feature = "board-redfin")]
const RAM_START: usize = 0x8000_0000;

#[cfg(not(feature = "board-redfin"))]
const RAM_SIZE: usize = 0x2000_0000; // 512MB
#[cfg(feature = "board-redfin")]
const RAM_SIZE: usize = 0x800_0000; // 128MB (keeps BITMAP small: 4KB)
const TOTAL_FRAMES: usize = RAM_SIZE / PAGE_SIZE; // 131072
const BITMAP_SIZE: usize = (TOTAL_FRAMES + 63) / 64; // 2048 u64s

static mut BITMAP: [u64; BITMAP_SIZE] = [0u64; BITMAP_SIZE];

/// Initialize the frame allocator.
///
/// `kernel_end` is the first physical address after the kernel + page tables.
///
/// # Safety
/// Must be called once before any alloc/free.
pub unsafe fn init(kernel_end: usize) {
    // Zero bitmap (all frames free)
    for i in 0..BITMAP_SIZE {
        BITMAP[i] = 0;
    }

    // Mark frames used by kernel + page tables as allocated
    let first_free_frame = page_align_up(kernel_end);
    let first_free_idx = (first_free_frame - RAM_START) / PAGE_SIZE;

    for i in 0..first_free_idx {
        set_bit(i);
    }

    // Mark stack region as used
    #[cfg(not(feature = "board-redfin"))]
    let stack_top = 0x4800_0000usize;
    #[cfg(feature = "board-redfin")]
    let stack_top = 0x8800_0000usize; // linker-redfin.ld: __stack_top
    let stack_bottom = stack_top - 0x1_0000; // 64KB stack
    let stack_start_idx = (stack_bottom - RAM_START) / PAGE_SIZE;
    let stack_end_idx = (stack_top - RAM_START) / PAGE_SIZE;
    for i in stack_start_idx..stack_end_idx {
        set_bit(i);
    }
}

/// Allocate a single 4KB physical frame.
/// Returns the physical address of the frame, or None if out of memory.
#[allow(dead_code)]
pub fn alloc_frame() -> Option<usize> {
    for i in 0..BITMAP_SIZE {
        if unsafe { BITMAP[i] } != !0 {
            // Find first zero bit in this u64
            let bits = unsafe { BITMAP[i] };
            for bit in 0..64 {
                let idx = i * 64 + bit;
                if idx >= TOTAL_FRAMES {
                    return None;
                }
                if bits & (1u64 << bit) == 0 {
                    unsafe { BITMAP[i] |= 1u64 << bit };
                    return Some(RAM_START + idx * PAGE_SIZE);
                }
            }
        }
    }
    None
}

/// Free a previously allocated physical frame.
#[allow(dead_code)]
pub fn free_frame(addr: usize) {
    if addr < RAM_START || addr >= RAM_START + RAM_SIZE {
        return;
    }
    let idx = (addr - RAM_START) / PAGE_SIZE;
    let word = idx / 64;
    let bit = idx % 64;
    unsafe { BITMAP[word] &= !(1u64 << bit) };
}

/// Get number of free frames
pub fn free_count() -> usize {
    let mut free = 0;
    for i in 0..BITMAP_SIZE {
        let bits = unsafe { BITMAP[i] };
        // Count set bits manually to avoid calling __popcountdi2
        let mut b = bits;
        let mut ones: usize = 0;
        while b != 0 {
            ones += 1;
            b &= b - 1; // clear lowest set bit
        }
        free += 64 - ones;
    }
    // Don't count beyond TOTAL_FRAMES
    let total_bits = BITMAP_SIZE * 64;
    if total_bits > TOTAL_FRAMES {
        let extra = total_bits - TOTAL_FRAMES;
        free = free.saturating_sub(extra);
    }
    free
}

/// Test: write a single value to BITMAP. Minimal write test.
pub fn test_write() {
    unsafe { BITMAP[0] = 1 };
}

/// Zero the entire BITMAP
pub fn zero_bitmap() {
    for i in 0..BITMAP_SIZE {
        unsafe { BITMAP[i] = 0 };
    }
}

/// Mark frames [start, end) as allocated
pub fn mark_range(start: usize, end: usize) {
    for i in start..end {
        if i < TOTAL_FRAMES {
            set_bit(i);
        }
    }
}

fn set_bit(idx: usize) {
    let word = idx / 64;
    let bit = idx % 64;
    unsafe { BITMAP[word] |= 1u64 << bit };
}

fn page_align_up(addr: usize) -> usize {
    (addr + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}
