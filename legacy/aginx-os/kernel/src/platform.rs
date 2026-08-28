//! Platform-specific constants
//!
//! Centralizes hardware addresses that vary between QEMU virt and Pixel 5 (redfin).

// UART base address
#[cfg(feature = "board-redfin")]
pub const UART: usize = 0x0088_8000; // Pixel 5 QUP UART
#[cfg(not(feature = "board-redfin"))]
pub const UART: usize = 0x0900_0000; // QEMU virt PL011
