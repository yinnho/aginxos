//! GIC (Generic Interrupt Controller) driver
//!
//! Supports both GICv2 and GICv3:
//! - GICv2: fully memory-mapped (GICD + GICC)
//! - GICv3: GICD MMIO + ICC_*_EL1 system registers for CPU interface
//!
//! QEMU virt defaults to GICv3; Pixel 5 (redfin) uses GICv2-compatible mode.

/// GICD base — distributor
#[cfg(not(feature = "board-redfin"))]
const GICD_BASE: usize = 0x0800_0000;  // QEMU virt
/// GICC base — CPU interface (GICv2 only; GICv3 uses system registers)
#[cfg(not(feature = "board-redfin"))]
const GICC_BASE: usize = 0x0801_0000;

/// Physical timer PPI interrupt ID
pub const TIMER_INTID: u32 = 30;

/// Cached GIC version (set once during init)
static mut IS_GICV2: bool = false;
/// Cached base addresses (set by init or init_with_base)
static mut GICD: usize = 0;
static mut GICC: usize = 0;

fn read32(addr: usize) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

fn write32(addr: usize, val: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, val) }
}

fn is_gicv2() -> bool {
    unsafe { IS_GICV2 }
}

/// Write ICC_PMR_EL1 (GICv3 priority mask) — S3_0_C4_C6_0
#[inline]
fn write_icc_pmr(val: u64) {
    unsafe { core::arch::asm!("msr S3_0_C4_C6_0, {}", in(reg) val) };
}

/// Write ICC_IGRPEN1_EL1 (GICv3 group 1 interrupt enable) — S3_0_C12_C12_7
#[inline]
fn write_icc_igrpen1(val: u64) {
    unsafe { core::arch::asm!("msr S3_0_C12_C12_7, {}", in(reg) val) };
}

/// Read ICC_IAR_EL1 (GICv3 interrupt acknowledge) — S3_0_C12_C8_0
#[inline]
fn read_icc_iar() -> u64 {
    let val: u64;
    unsafe { core::arch::asm!("mrs {}, S3_0_C12_C8_0", out(reg) val) };
    val
}

/// Write ICC_EOIR_EL1 (GICv3 end of interrupt) — S3_0_C12_C8_1
#[inline]
fn write_icc_eoir(val: u64) {
    unsafe { core::arch::asm!("msr S3_0_C12_C8_1, {}", in(reg) val) };
}

/// Initialize GIC with explicit base addresses (from DTB)
pub fn init_with_base(gicd_base: usize, gicc_base: usize, _uart: usize) {
    unsafe {
        GICD = gicd_base;
        GICC = gicc_base;
    }

    // Detect GIC version: ARE bit in GICD_CTLR
    let ctlr = read32(gicd_base);
    unsafe { IS_GICV2 = (ctlr & 0x2) == 0; }

    // Distributor: common setup
    // GICD_CTLR: enable group 0 and group 1
    write32(gicd_base + 0x000, 0x1 | 0x2);

    // GICD_ICENABLER[0]: disable all interrupts first
    write32(gicd_base + 0x180, 0xFFFF_FFFF);
    write32(gicd_base + 0x184, 0xFFFF_FFFF);

    // GICD_ICFGR[0]: configure all as level-sensitive
    write32(gicd_base + 0x0C0, 0x0000_0000);

    // GICD_IPRIORITYR[0-7]: set priority for first 32 interrupts
    for i in 0..8 {
        write32(gicd_base + 0x400 + i * 4, 0xA0A0_A0A0);
    }

    // GICD_ISENABLER[0]: enable PPI 30 (timer)
    write32(gicd_base + 0x100, 1u32 << TIMER_INTID);

    // GICD_ITARGETSR[0]: route PPI to CPU0 (GICv2 only, but harmless on v3)
    write32(gicd_base + 0x800, 0x0101_0101);
    write32(gicd_base + 0x804, 0x0101_0101);
    write32(gicd_base + 0x808, 0x0101_0101);
    write32(gicd_base + 0x80C, 0x0101_0101);

    // GICD_CTLR: enable forwarding
    write32(gicd_base + 0x000, 0x1 | 0x2);

    if is_gicv2() {
        // GICv2 CPU Interface via MMIO
        write32(gicc_base + 0x0000, 0x1 | 0x2); // GICC_CTLR
        write32(gicc_base + 0x0004, 0xFF);       // GICC_PMR
        write32(gicc_base + 0x0008, 0x7);        // GICC_BPR
    } else {
        // GICv3 CPU Interface via system registers
        write_icc_pmr(0xFF);         // Accept all priorities
        write_icc_igrpen1(0x1);      // Enable Group 1 interrupts
        unsafe { core::arch::asm!("isb") };
    }
}

/// Initialize GIC with default base addresses (QEMU)
#[cfg(not(feature = "board-redfin"))]
pub fn init(_uart: usize) {
    init_with_base(GICD_BASE, GICC_BASE, _uart);
}

/// Initialize GIC — on redfin, must use init_with_base with DTB-derived address
#[cfg(feature = "board-redfin")]
pub fn init(_uart: usize) {
    // On redfin, this should not be called directly.
    // Use init_with_base() with addresses from DTB.
}

/// Acknowledge interrupt
pub fn acknowledge() -> u32 {
    unsafe {
        if IS_GICV2 {
            read32(GICC + 0x000C) & 0x3FF
        } else {
            read_icc_iar() as u32 & 0x3FF
        }
    }
}

/// End of interrupt
pub fn end(intid: u32) {
    unsafe {
        if IS_GICV2 {
            write32(GICC + 0x0010, intid);
        } else {
            write_icc_eoir(intid as u64);
        }
    }
}
