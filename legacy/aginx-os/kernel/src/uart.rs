//! UART driver for QEMU virt machine
//!
//! Uses PL011 UART at 0x09000000

/// Initialize UART
pub fn init(base: usize) {
    // Disable UART
    write_cr(base, 0);

    // Set baud rate (ignore for QEMU)
    // Write to IBRD and FBRD if needed

    // Enable FIFO
    write_lcrh(base, LCRH_FEN);

    // Enable TX/RX
    write_cr(base, CR_UARTEN | CR_TXE | CR_RXE);
}

/// Put a character
pub fn putc(base: usize, c: u8) {
    // Wait until TX FIFO has space
    while read_fr(base) & FR_TXFF != 0 {
        core::hint::spin_loop();
    }

    // Write character
    write_dr(base, c);
}

/// Put a string
pub fn puts(base: usize, s: &str) {
    for c in s.bytes() {
        if c == b'\n' {
            putc(base, b'\r');
        }
        putc(base, c);
    }
}

/// Write raw bytes
#[allow(dead_code)]
pub fn write_bytes(base: usize, bytes: &[u8]) {
    for &c in bytes {
        putc(base, c);
    }
}

/// Get a character (blocking)
pub fn has_data(base: usize) -> bool {
    read_fr(base) & FR_RXFE == 0
}

pub fn getc(base: usize) -> u8 {
    // Wait until RX FIFO has data
    while read_fr(base) & FR_RXFE != 0 {
        core::hint::spin_loop();
    }

    read_dr(base)
}

/// Non-blocking getc — returns 0 if no data available
pub fn getc_nb(base: usize) -> u8 {
    if read_fr(base) & FR_RXFE != 0 {
        return 0;
    }
    read_dr(base)
}

// --- Registers ---

const DR: usize = 0x00;   // Data Register
const FR: usize = 0x18;   // Flag Register
const CR: usize = 0x30;   // Control Register
const LCRH: usize = 0x2C; // Line Control Register

// Flag Register bits
const FR_TXFF: u32 = 1 << 5; // TX FIFO full
const FR_RXFE: u32 = 1 << 4; // RX FIFO empty

// Control Register bits
const CR_UARTEN: u32 = 1 << 0; // UART enable
const CR_TXE: u32 = 1 << 8;    // TX enable
const CR_RXE: u32 = 1 << 9;    // RX enable

// Line Control Register bits
const LCRH_FEN: u32 = 1 << 4; // FIFO enable

// --- Register access ---

fn read_reg(base: usize, offset: usize) -> u32 {
    unsafe {
        core::ptr::read_volatile((base + offset) as *const u32)
    }
}

fn write_reg(base: usize, offset: usize, val: u32) {
    unsafe {
        core::ptr::write_volatile((base + offset) as *mut u32, val);
    }
}

fn read_fr(base: usize) -> u32 {
    read_reg(base, FR)
}

fn read_dr(base: usize) -> u8 {
    read_reg(base, DR) as u8
}

fn write_dr(base: usize, c: u8) {
    write_reg(base, DR, c as u32);
}

fn write_cr(base: usize, val: u32) {
    write_reg(base, CR, val);
}

fn write_lcrh(base: usize, val: u32) {
    write_reg(base, LCRH, val);
}
