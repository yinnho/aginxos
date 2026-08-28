//! Pixel 5 (redfin) Hardware Addresses
//!
//! Qualcomm Snapdragon 765G (SM7350)

/// UART base address (QUP UART at 0x888000)
pub const UART0: usize = 0x0088_0000;

/// GIC-500 base (GICv3)
pub const GICD_BASE: usize = 0x0170_0000;
pub const GICC_BASE: usize = 0x0170_8000;
pub const GICH_BASE: usize = 0x0170_A000;
pub const GICV_BASE: usize = 0x0170_C000;

/// Memory
pub const RAM_START: usize = 0x8000_0000;
pub const RAM_SIZE: usize = 0x2000_0000; // 8GB, but we use 512MB for kernel

/// Timer frequency (19.2 MHz)
pub const TIMER_FREQ: u32 = 19_200_000;
