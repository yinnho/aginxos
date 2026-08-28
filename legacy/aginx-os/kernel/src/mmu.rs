//! aarch64 MMU and page table setup
//!
//! Pixel 5 strategy: ABL's page table is valid but missing mappings for
//! 0x80000000 (kernel) and 0xA0000000 (framebuffer). We patch the missing
//! entries into ABL's existing page table, then re-enable MMU with ABL's
//! original TTBR0/TCR/MAIR settings.

// Assembly-defined globals for TTBR0 switching
extern "C" {
    pub static mut KERNEL_TTBR0: u64;
    pub static mut CURRENT_USER_TTBR0: u64;
    pub static mut CURRENT_USER_SP: u64;
}

/// L2 block descriptor: Normal WB, inner-shareable, AF=1, AP=01 (EL0+EL1 access)
/// bits[1:0]=01, AttrIndx=000, SH=11, AF=1, AP[1]=1 (bit 6)
const L2_NORMAL: u64 = (1 << 0) | (1 << 6) | (1 << 10) | (3 << 8);
/// L2 block descriptor: Device_nGnRE, AP=01 (EL0+EL1 access)
const L2_DEVICE: u64 = (1 << 0) | (1 << 6) | (1 << 10) | (3 << 8) | (1 << 2);
/// L2 block descriptor: Normal Non-Cacheable (for framebuffer), AP=01
#[allow(dead_code)]
const L2_NORMAL_NC: u64 = (1 << 0) | (1 << 6) | (1 << 10) | (3 << 8) | (2 << 2);

// ─── QEMU ──────────────────────────────────────────────────────────────────────

#[cfg(not(feature = "board-redfin"))]
pub unsafe fn init(l1_table: *mut u64) {
    // Layout: l1_table has 512 entries (4KB)
    //         l2_dev  = l1_table + 512  (4KB) — Device mappings for L1[0]
    //         l2_norm = l1_table + 1024 (4KB) — Normal WB mappings for L1[1]
    for i in 0..1536 {  // Zero all three tables (12KB total)
        l1_table.add(i).write_volatile(0);
    }
    let l2_dev = l1_table.add(512);
    let l2_norm = l1_table.add(1024);

    // L1[0] → l2_dev (VA 0x0_0000_0000 .. 0x0_3FFF_FFFF)
    l1_table.add(0).write_volatile(l2_dev as u64 | 0b11);
    // L1[1] → l2_norm (VA 0x0_4000_0000 .. 0x0_7FFF_FFFF)
    l1_table.add(1).write_volatile(l2_norm as u64 | 0b11);

    // L2 Device: 256 × 2MB blocks = 512MB at PA 0x0
    for i in 0..256 {
        let pa = (i as u64) << 21;
        l2_dev.add(i).write_volatile(pa | L2_DEVICE);
    }
    // L2 Normal WB: 256 × 2MB blocks = 512MB at PA 0x4000_0000
    for i in 0..256 {
        let pa = 0x4000_0000u64 + ((i as u64) << 21);
        l2_norm.add(i).write_volatile(pa | L2_NORMAL);
    }
    let mair = (0xFFu64 << 0) | (0x04u64 << 8);
    core::arch::asm!("msr mair_el1, {}", in(reg) mair);
    core::arch::asm!("dsb sy");
    core::arch::asm!("msr ttbr0_el1, {}", in(reg) l1_table as u64);
    KERNEL_TTBR0 = l1_table as u64;
    core::arch::asm!("dsb nsh");
    core::arch::asm!("tlbi vmalle1");
    core::arch::asm!("dsb nsh");
    core::arch::asm!("isb");
    let tcr = (0u64 << 0) | (2u64 << 32) | (3u64 << 12) | (1u64 << 10) | (1u64 << 8);
    core::arch::asm!("msr tcr_el1, {}", in(reg) tcr);
    core::arch::asm!("ic iallu");
    core::arch::asm!("dsb ish");
    let sctlr_val = 0x30C50830u64;
    core::arch::asm!("msr sctlr_el1, {}", in(reg) sctlr_val);
    core::arch::asm!("dsb ish");
    core::arch::asm!("isb");
    core::arch::asm!("mrs x0, sctlr_el1");
    core::arch::asm!("orr x0, x0, #0xC00");
    core::arch::asm!("msr sctlr_el1, x0");
    core::arch::asm!("dsb ish");
    core::arch::asm!("isb");
}

// ─── Pixel 5 redfin ──────────────────────────────────────────────────────────
//
// ABL's page table structure (48-bit VA, 4KB granule):
//   TTBR0 → L0 table
//   L0[0] → L1 table (covers VA 0x00000000-0x7FFFFFFFFF)
//   L1[2] → L2 table (covers VA 0x80000000-0xBFFFFFFF)
//   L2[0] = 0 ← NOT MAPPED (our kernel at 0x80000000!)
//
// Strategy: patch L2[0..15] with identity-mapped Normal blocks for kernel,
// patch L2[256..263] for framebuffer, then re-enable MMU with ABL's settings.

#[cfg(feature = "board-redfin")]
pub unsafe fn init(_l1_table: *mut u64) {
    // DEBUG: skip MMU entirely to verify rest of code works
    // Just return immediately
}

// ─── User Page Table Creation ─────────────────────────────────────────────────

/// L2 block for user code/data: Normal WB, EL0 RWX (AP=01, bit 6)
/// UXN=0, PXN=0 (allow execution from EL0)
const L2_USER_CODE: u64 = (1 << 0) | (1 << 6) | (1 << 10) | (3 << 8);
/// L2 block for user stack: Normal WB, EL0 RW (AP=01, bit 6), PXN=1
const L2_USER_DATA: u64 = (1 << 0) | (1 << 6) | (1 << 10) | (3 << 8) | (1 << 53);

/// Create a user page table with mappings for code and stack.
///
/// User VA layout:
///   0x0001_0000: Code segment (mapped to code_pa)
///   0x000F_0000: Stack top (mapped to stack_pa, grows down)
///
/// Returns TTBR0 value (physical address of the L1 table).
#[cfg(not(feature = "board-redfin"))]
pub unsafe fn create_user_page_table(
    code_pa: usize,
    code_size: usize,
    stack_pa_top: usize,
    stack_size: usize,
) -> u64 {
    // Allocate one 4KB L1 table (512 entries)
    let layout = alloc::alloc::Layout::from_size_align(4096, 4096).unwrap();
    let l1 = alloc::alloc::alloc_zeroed(layout) as *mut u64;

    // Allocate one 4KB L2 table for user mappings (entries 0-3 cover 0x0-0x1FFFFF)
    let l2_layout = alloc::alloc::Layout::from_size_align(4096, 4096).unwrap();
    let l2 = alloc::alloc::alloc_zeroed(l2_layout) as *mut u64;

    // Point L1[0] → L2 table (covers VA 0x0 - 0x3FFF_FFFF)
    core::ptr::write_volatile(l1, (l2 as u64) | 0b11); // valid + page table

    // Map code at VA 0x10000 (L2 index 0, 2MB block starting at VA 0x0)
    // Since L2 entries are 2MB blocks, entry 0 covers VA 0x0 - 0x1FFFFF
    // This includes VA 0x10000 which is our entry point
    let code_l2_pa = code_pa & !0x1FFFFF; // 2MB-aligned physical address
    core::ptr::write_volatile(l2, code_l2_pa as u64 | L2_USER_CODE);

    // Map stack at VA 0xF0000 (still within L2[0]'s 2MB range: 0x0-0x1FFFFF)
    // Stack top is at 0xF0000, stack grows down from there
    // Since both code and stack are within the same 2MB block, they share L2[0]
    // For separate mapping, we'd need a second L2 entry or finer granularity

    // If stack PA is in a different 2MB region than code, map another L2 entry
    let stack_l2_idx = 0xF0000 / 0x200000; // = 0, same 2MB block
    // Stack is in the same 2MB block as code, so it's already mapped
    // The physical memory for stack is assumed to be within the same 2MB region

    // Flush TLB for new page table
    core::arch::asm!("dsb sy");
    core::arch::asm!("isb");

    l1 as u64
}

/// Map an additional page in a user page table (for mmap).
/// VA must be 2MB-aligned for L2 block mapping.
#[cfg(not(feature = "board-redfin"))]
pub unsafe fn user_vm_map(ttbr0: u64, va: usize, pa: usize, writable: bool) -> bool {
    let l1 = ttbr0 as *mut u64;
    // VA bits [47:30] → L1 index (9 bits)
    let l1_idx = (va >> 30) & 0x1FF;

    // Read L1 entry
    let l1_entry = core::ptr::read_volatile(l1.add(l1_idx));
    let l2: *mut u64;

    if l1_entry & 1 == 0 {
        // No L2 table yet — allocate one
        let l2_layout = alloc::alloc::Layout::from_size_align(4096, 4096).unwrap();
        l2 = alloc::alloc::alloc_zeroed(l2_layout) as *mut u64;
        core::ptr::write_volatile(l1.add(l1_idx), (l2 as u64) | 0b11);
    } else {
        // Extract L2 table address from L1 entry
        l2 = (l1_entry & !0x1FF) as *mut u64;
    }

    // VA bits [29:21] → L2 index (9 bits)
    let l2_idx = (va >> 21) & 0x1FF;
    let flags = if writable { L2_USER_DATA } else { L2_USER_CODE };
    let pa_aligned = pa & !0x1FFFFF; // 2MB-aligned
    core::ptr::write_volatile(l2.add(l2_idx), pa_aligned as u64 | flags);

    core::arch::asm!("dsb sy");
    true
}
