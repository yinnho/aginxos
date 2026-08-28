//! QUPv3 UART driver for Pixel 5 (redfin)
//!
//! Qualcomm Snapdragon 765G QUP UART at 0x0888000 (ttyMSM0)
//! QUPv3 GENI SE interface - TX/RX via FIFO at 0x700/0x780

// === QUPv3 Wrapper Register Offsets ===

const QUP_SW_reset:        usize = 0x034;
const QUP_CONFIG:          usize = 0x044;
const QUP_STATE:          usize = 0x048;
const QUP_IO_MODES:       usize = 0x050;
const QUP_ERROR_FLAGS_EN: usize = 0x058;
const QUP_OPERATIONAL:    usize = 0x064;
const QUP_SERIAL_CLK:     usize = 0x10C;  // QUPv3 has clock at 0x10C

// === QUPv3 GENI SE Register Offsets ===

// TX data register: SE_GENI_TX_FIFOn at offset 0x700
const SE_GENI_TX_FIFOn: usize = 0x700;
// RX data register: SE_GENI_RX_FIFOn at offset 0x780
const SE_GENI_RX_FIFOn: usize = 0x780;
// TX FIFO status: SE_GENI_TX_FIFO_STATUS at 0x800
const SE_GENI_TX_FIFO_STATUS: usize = 0x800;
// RX FIFO status: SE_GENI_RX_FIFO_STATUS at 0x804
const SE_GENI_RX_FIFO_STATUS: usize = 0x804;

// State register bits
const TX_FIFO_WC_MASK: u32 = 0x7F;  // word count in TX FIFO status
const RX_FIFO_WC_MASK: u32 = 0x7F;  // word count in RX FIFO status

fn read_reg(addr: usize) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

fn write_reg(addr: usize, val: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, val) }
}

/// Initialize QUP UART with step-by-step status.
/// Returns Ok(()) on success, Err(step) on failure at given step.
pub fn init_debug(base: usize, con: &mut dyn DebugOutput) -> Result<(), usize> {
    // Step 1: Check QUP hardware version
    let hw_version = read_reg(base + 0x4);
    con.debug_hex("hw_ver", hw_version);
    if hw_version == 0 || hw_version == 0xFFFFFFFF {
        con.debug_str("QUP not accessible\r\n");
        return Err(1);
    }

    // Step 2: Reset the QUP core
    write_reg(base + QUP_SW_reset, 0x1);
    let mut timeout = 100_000u32;
    while timeout > 0 && (read_reg(base + QUP_SW_reset) & 0x1) != 0 {
        timeout -= 1;
    }
    if timeout == 0 {
        con.debug_str("reset timeout\r\n");
        return Err(2);
    }

    // Step 3: Read QUP state
    let state = read_reg(base + QUP_STATE);
    con.debug_hex("state", state);

    // Step 4: Configure to UART mode (protocol code 0x1 into mini-core)
    // QUPv3 GENI SE: write protocol to GENI_FW_REVISION_RO or SE_GENI_CLK_SEL
    // Try QUP_CONFIG first
    let cfg = read_reg(base + QUP_CONFIG);
    con.debug_hex("cfg_old", cfg);
    write_reg(base + QUP_CONFIG, 0x1);

    // Step 5: Set state to run
    write_reg(base + QUP_STATE, 0x1);
    let state2 = read_reg(base + QUP_STATE);
    con.debug_hex("state2", state2);

    // Step 6: Try writing a test byte directly to TX FIFO
    let tx_status = read_reg(base + SE_GENI_TX_FIFO_STATUS);
    con.debug_hex("tx_st", tx_status);

    con.debug_str("UART init OK\r\n");
    Ok(())
}

/// Trait for debug output (implemented by framebuffer console)
pub trait DebugOutput {
    fn debug_str(&mut self, s: &str);
    fn debug_hex(&mut self, label: &str, val: u32);
}

/// Initialize QUP UART (assumes bootloader already configured clocks and pinctrl)
pub fn init(base: usize) {
    // Check QUP hardware version (offset 0x4)
    let hw_version = read_reg(base + 0x4);
    if hw_version == 0 || hw_version == 0xFFFFFFFF {
        return;  // QUP not accessible — skip UART
    }

    // 1. Reset the QUP core
    write_reg(base + QUP_SW_reset, 0x1);
    let mut timeout = 100_000;
    while timeout > 0 && (read_reg(base + QUP_SW_reset) & 0x1) != 0 {
        timeout -= 1;
    }

    // 2. Configure to UART mode (protocol code 1)
    write_reg(base + QUP_CONFIG, 0x1);

    // 3. Set I/O modes: NOT_PACKED, 8-bit word, NO_DELAY bypass
    write_reg(base + QUP_IO_MODES, (1 << 11) | (1 << 10));

    // 4. Clock divider for 115200 baud from 19.2MHz
    write_reg(base + QUP_SERIAL_CLK, 10);

    // 5. Set state to reset (0) to initialize state machine
    write_reg(base + QUP_STATE, 0x0);

    // 6. Enable TX and RX
    write_reg(base + QUP_OPERATIONAL, (1 << 11) | (1 << 10));

    // 7. Set state to run
    write_reg(base + QUP_STATE, 0x1);
}

/// Put a character (blocking with timeout) — writes to SE_GENI_TX_FIFOn at 0x700
pub fn putc(base: usize, c: u8) {
    // Wait for space in TX FIFO (with timeout to avoid infinite hang)
    let mut timeout = 100000;
    loop {
        let status = read_reg(base + SE_GENI_TX_FIFO_STATUS);
        if (status & TX_FIFO_WC_MASK) < 0x40 {
            break;
        }
        timeout -= 1;
        if timeout == 0 {
            return;  // Timeout — UART not responding
        }
        core::hint::spin_loop();
    }
    write_reg(base + SE_GENI_TX_FIFOn, c as u32);
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

/// Get a character (blocking)
pub fn getc(base: usize) -> u8 {
    loop {
        let status = read_reg(base + SE_GENI_RX_FIFO_STATUS);
        if status & RX_FIFO_WC_MASK != 0 {
            break;
        }
        core::hint::spin_loop();
    }
    read_reg(base + SE_GENI_RX_FIFOn) as u8
}

/// Get a character (non-blocking), returns 0 if no data available
pub fn getc_nb(base: usize) -> u8 {
    let status = read_reg(base + SE_GENI_RX_FIFO_STATUS);
    if status & RX_FIFO_WC_MASK != 0 {
        read_reg(base + SE_GENI_RX_FIFOn) as u8
    } else {
        0
    }
}

/// Check if RX data is available
pub fn has_data(base: usize) -> bool {
    let status = read_reg(base + SE_GENI_RX_FIFO_STATUS);
    (status & RX_FIFO_WC_MASK) != 0
}

/// Write a byte slice to UART
pub fn write_bytes(base: usize, bytes: &[u8]) {
    for &c in bytes {
        putc(base, c);
    }
}
