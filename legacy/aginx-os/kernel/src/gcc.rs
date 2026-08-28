//! GCC (Global Clock Controller) driver for SM7250 (Pixel 5)
//!
//! Uses SC7180-compatible register offsets (0x17000 range) for QUPv3 clocks.
//! Vote register: 0x52008 (NOT 0x52000, which causes bus hang).

const GCC_BASE: usize = 0x0010_0000;

// QUP Wrapper 0 register offsets (SC7180-compatible)
const QUP_WRAP0_BCR: usize = 0x17000;
const QUP_WRAP0_M_AHB_CBCR: usize = 0x17004;
const QUP_WRAP0_S_AHB_CBCR: usize = 0x17008;
const QUP_WRAP0_S2_CBCR: usize = 0x1726C;
const QUP_WRAP0_S2_CMD_RCGR: usize = 0x17270;
const QUP_WRAP0_S2_CFG_RCGR: usize = 0x17274;
const QUP_WRAP0_VOTE: usize = 0x52008;

// Vote bits
const VOTE_M_AHB: u32 = 1 << 6;
const VOTE_S_AHB: u32 = 1 << 7;
const VOTE_S2_CLK: u32 = 1 << 12;

// CBCR bits
const CBCR_CLK_OFF: u32 = 1 << 31;
const CBCR_CLK_ENABLE: u32 = 1 << 0;

// RCG source: 0 = CXO (19.2 MHz)
const RCG_SRC_CXO: u32 = 0;
const RCG_CMD_UPDATE: u32 = 1 << 0;

fn read_reg(off: usize) -> u32 {
    unsafe { core::ptr::read_volatile((GCC_BASE + off) as *const u32) }
}

fn write_reg(off: usize, val: u32) {
    unsafe { core::ptr::write_volatile((GCC_BASE + off) as *mut u32, val) }
}

fn vote_set(vote_reg: usize, mask: u32) {
    let val = read_reg(vote_reg);
    write_reg(vote_reg, val | mask);
}

fn poll_until_clear(off: usize, mask: u32) -> bool {
    let mut timeout = 100_000u32;
    while timeout > 0 {
        if (read_reg(off) & mask) == 0 {
            return true;
        }
        timeout -= 1;
        core::hint::spin_loop();
    }
    false
}

#[derive(Debug, Clone, Copy)]
pub enum ClockStep {
    Success,
    MAhbPoll { val: u32 },
    SAhbWarn { val: u32 },  // S_AHB didn't clear but we continued
    RcgUpdate { val: u32 },
    S2Poll { val: u32 },
}

/// Enable QUP Wrapper 0 SE2 clocks (for UART at 0x0888000)
pub fn enable_qup_uart_debug() -> ClockStep {
    // Reset QUP Wrapper 0
    write_reg(QUP_WRAP0_BCR, 1);
    for _ in 0..100 { core::hint::spin_loop(); }
    write_reg(QUP_WRAP0_BCR, 0);
    for _ in 0..100 { core::hint::spin_loop(); }

    // M_AHB
    vote_set(QUP_WRAP0_VOTE, VOTE_M_AHB);
    if !poll_until_clear(QUP_WRAP0_M_AHB_CBCR, CBCR_CLK_OFF) {
        return ClockStep::MAhbPoll { val: read_reg(QUP_WRAP0_M_AHB_CBCR) };
    }

    // S_AHB — try to enable, but don't abort if CLK_OFF stays set
    let mut sah_b_warn = false;
    let sah_b_val;
    vote_set(QUP_WRAP0_VOTE, VOTE_S_AHB);
    write_reg(QUP_WRAP0_S_AHB_CBCR, read_reg(QUP_WRAP0_S_AHB_CBCR) | CBCR_CLK_ENABLE);
    if !poll_until_clear(QUP_WRAP0_S_AHB_CBCR, CBCR_CLK_OFF) {
        sah_b_val = read_reg(QUP_WRAP0_S_AHB_CBCR);
        sah_b_warn = true;
    } else {
        sah_b_val = 0;
    }

    // SE2 RCG — configure source = CXO, then update
    write_reg(QUP_WRAP0_S2_CFG_RCGR, RCG_SRC_CXO);
    write_reg(QUP_WRAP0_S2_CMD_RCGR, RCG_CMD_UPDATE);
    if !poll_until_clear(QUP_WRAP0_S2_CMD_RCGR, RCG_CMD_UPDATE) {
        return ClockStep::RcgUpdate { val: read_reg(QUP_WRAP0_S2_CMD_RCGR) };
    }

    // SE2 CBCR
    vote_set(QUP_WRAP0_VOTE, VOTE_S2_CLK);
    write_reg(QUP_WRAP0_S2_CBCR, read_reg(QUP_WRAP0_S2_CBCR) | CBCR_CLK_ENABLE);
    if !poll_until_clear(QUP_WRAP0_S2_CBCR, CBCR_CLK_OFF) {
        return ClockStep::S2Poll { val: read_reg(QUP_WRAP0_S2_CBCR) };
    }

    if sah_b_warn {
        ClockStep::SAhbWarn { val: sah_b_val }
    } else {
        ClockStep::Success
    }
}

pub fn enable_qup_uart() -> bool {
    matches!(enable_qup_uart_debug(), ClockStep::Success)
}

// ── USB30 clocks (SM7250) ──────────────────────────────────────────

// USB30 PRIM GDSCR — power domain control
const USB30_PRIM_GDSCR: usize = 0x0F004;
// USB30 PRIM BCR — block reset
const USB30_PRIM_BCR: usize = 0x0F000;
// QUSB2PHY PRIM BCR — PHY reset
const QUSB2PHY_PRIM_BCR: usize = 0x26000;
// QUSB2PHY PRIM ref clock — PHY reference clock (needed for register writes + PLL)
const QUSB2PHY_PRIM_CLK: usize = 0x26004;
// AHB2PHY bridge — needed for PHY register access
const USB_PHY_CFG_AHB2PHY_BCR: usize = 0x6A000;
const USB_PHY_CFG_AHB2PHY_CLK: usize = 0x6A004;

// CBCR branch clocks for USB30
const USB30_PRIM_MASTER_CLK: usize = 0x0F010;
const USB30_PRIM_SLEEP_CLK: usize = 0x0F014;
const USB30_PRIM_MOCK_UTMI_CLK: usize = 0x0F018;
const USB30_PRIM_PHY_AUX_CLK: usize = 0x0F050;
const USB30_PRIM_PHY_COM_AUX_CLK: usize = 0x0F054;
const USB30_PRIM_CFG_NOC_AXI_CLK: usize = 0x0502C;
const USB30_PRIM_AGGR_NOC_AXI_CLK: usize = 0x8201C;
const USB30_PRIM_CLKREF_CLK: usize = 0x8C010;

/// AHB2PHY only: clock + BCR. Does not touch DWC3 GDSCR or QUSB2 PHY BCR.
pub fn ahb2phy_bringup() -> bool {
    write_reg(USB_PHY_CFG_AHB2PHY_CLK, (read_reg(USB_PHY_CFG_AHB2PHY_CLK) & !0x2) | CBCR_CLK_ENABLE);
    let _ = poll_until_clear(USB_PHY_CFG_AHB2PHY_CLK, CBCR_CLK_OFF);
    write_reg(USB_PHY_CFG_AHB2PHY_BCR, 1);
    for _ in 0..10_000 {
        core::hint::spin_loop();
    }
    write_reg(USB_PHY_CFG_AHB2PHY_BCR, 0);
    for _ in 0..10_000 {
        core::hint::spin_loop();
    }
    write_reg(USB_PHY_CFG_AHB2PHY_CLK, (read_reg(USB_PHY_CFG_AHB2PHY_CLK) & !0x2) | CBCR_CLK_ENABLE);
    poll_until_clear(USB_PHY_CFG_AHB2PHY_CLK, CBCR_CLK_OFF)
}

/// Read-only USB30 power/clock snapshot (no writes — safe after FB bring-up).
pub fn usb30_gdscr() -> u32 {
    read_reg(USB30_PRIM_GDSCR)
}
pub fn usb30_master_cbcr() -> u32 {
    read_reg(USB30_PRIM_MASTER_CLK)
}
pub fn usb30_bcr() -> u32 {
    read_reg(USB30_PRIM_BCR)
}

// GDSCR bits
const GDSCR_PWR_ON: u32 = 1 << 31;

/// Enable USB30 PRIM clocks, power domain, and resets.
/// Order verified on hardware: GDSC → branch clocks → AHB2PHY BCR reset → AHB2PHY clock.
/// Returns (step, register_value) on failure, None on success.
/// step: 0=GDSCR, 1..7=branch_clock[i-1], 8=AHB2PHY_BCR, 9=AHB2PHY_CLK, 10=QUSB2PHY_BCR
pub fn enable_usb30_clocks_debug() -> Option<(usize, u32)> {
    // 1. Power on USB30 GDSC
    write_reg(USB30_PRIM_GDSCR, 0x0);
    let mut ok = false;
    for _ in 0..100_000 {
        if read_reg(USB30_PRIM_GDSCR) & GDSCR_PWR_ON != 0 {
            ok = true;
            break;
        }
        core::hint::spin_loop();
    }
    if !ok {
        return Some((0, read_reg(USB30_PRIM_GDSCR)));
    }

    // 2. Enable branch clocks FIRST (before AHB2PHY — they power the interconnect)
    let clocks: &[usize] = &[
        USB30_PRIM_CFG_NOC_AXI_CLK,
        USB30_PRIM_MASTER_CLK,
        USB30_PRIM_MOCK_UTMI_CLK,
        USB30_PRIM_SLEEP_CLK,
        USB30_PRIM_PHY_AUX_CLK,
        USB30_PRIM_PHY_COM_AUX_CLK,
        USB30_PRIM_AGGR_NOC_AXI_CLK,
        USB30_PRIM_CLKREF_CLK,
    ];
    for (i, &clk) in clocks.iter().enumerate() {
        write_reg(clk, read_reg(clk) | CBCR_CLK_ENABLE);
        if !poll_until_clear(clk, CBCR_CLK_OFF) {
            return Some((1 + i, read_reg(clk)));
        }
    }

    // 3. AHB2PHY BCR full reset cycle (MUST be after GDSC + branch clocks!)
    write_reg(USB_PHY_CFG_AHB2PHY_BCR, 1);
    for _ in 0..10_000 { core::hint::spin_loop(); }
    write_reg(USB_PHY_CFG_AHB2PHY_BCR, 0);
    for _ in 0..10_000 { core::hint::spin_loop(); }

    // 4. AHB2PHY clock enable — now the bridge is alive, clock should turn on
    write_reg(USB_PHY_CFG_AHB2PHY_CLK, (read_reg(USB_PHY_CFG_AHB2PHY_CLK) & !0x2) | CBCR_CLK_ENABLE);
    if !poll_until_clear(USB_PHY_CFG_AHB2PHY_CLK, CBCR_CLK_OFF) {
        return Some((9, read_reg(USB_PHY_CFG_AHB2PHY_CLK)));
    }

    // 4b. QUSB2PHY ref clock — PHY needs this for register writes + PLL
    write_reg(QUSB2PHY_PRIM_CLK, read_reg(QUSB2PHY_PRIM_CLK) | CBCR_CLK_ENABLE);
    if !poll_until_clear(QUSB2PHY_PRIM_CLK, CBCR_CLK_OFF) {
        return Some((11, read_reg(QUSB2PHY_PRIM_CLK)));
    }

    // 5. QUSB2PHY BCR reset — PHY auto-initializes after deassert
    write_reg(QUSB2PHY_PRIM_BCR, 1);
    for _ in 0..1000 { core::hint::spin_loop(); }
    write_reg(QUSB2PHY_PRIM_BCR, 0);
    if !poll_until_clear(QUSB2PHY_PRIM_BCR, 0x1) {
        return Some((10, read_reg(QUSB2PHY_PRIM_BCR)));
    }

    None
}

pub fn enable_usb30_clocks() -> bool {
    enable_usb30_clocks_debug().is_none()
}

/// Minimal clock enable: GDSC + branch clocks + AHB2PHY clock.
/// Skips AHB2PHY BCR reset to avoid killing PHY state.
pub fn enable_usb30_clocks_minimal() -> bool {
    // 1. Power on USB30 GDSC
    write_reg(USB30_PRIM_GDSCR, 0x0);
    let mut ok = false;
    for _ in 0..100_000 {
        if read_reg(USB30_PRIM_GDSCR) & GDSCR_PWR_ON != 0 {
            ok = true;
            break;
        }
        core::hint::spin_loop();
    }
    if !ok { return false; }

    // 2. Branch clocks
    let clocks: &[usize] = &[
        USB30_PRIM_CFG_NOC_AXI_CLK,
        USB30_PRIM_MASTER_CLK,
        USB30_PRIM_MOCK_UTMI_CLK,
        USB30_PRIM_SLEEP_CLK,
        USB30_PRIM_PHY_AUX_CLK,
        USB30_PRIM_PHY_COM_AUX_CLK,
        USB30_PRIM_AGGR_NOC_AXI_CLK,
        USB30_PRIM_CLKREF_CLK,
    ];
    for &clk in clocks {
        write_reg(clk, read_reg(clk) | CBCR_CLK_ENABLE);
        if !poll_until_clear(clk, CBCR_CLK_OFF) { return false; }
    }

    // 3. AHB2PHY clock (no BCR reset!)
    write_reg(USB_PHY_CFG_AHB2PHY_CLK, (read_reg(USB_PHY_CFG_AHB2PHY_CLK) & !0x2) | CBCR_CLK_ENABLE);
    if !poll_until_clear(USB_PHY_CFG_AHB2PHY_CLK, CBCR_CLK_OFF) { return false; }

    // 4. QUSB2PHY ref clock
    write_reg(QUSB2PHY_PRIM_CLK, read_reg(QUSB2PHY_PRIM_CLK) | CBCR_CLK_ENABLE);
    if !poll_until_clear(QUSB2PHY_PRIM_CLK, CBCR_CLK_OFF) { return false; }

    true
}
/// Preserves ABL's PHY state — no QUSB2PHY BCR reset.
pub fn enable_usb30_clocks_no_phy_reset() -> bool {
    // 1. Power on USB30 GDSC
    write_reg(USB30_PRIM_GDSCR, 0x0);
    let mut ok = false;
    for _ in 0..100_000 {
        if read_reg(USB30_PRIM_GDSCR) & GDSCR_PWR_ON != 0 {
            ok = true;
            break;
        }
        core::hint::spin_loop();
    }
    if !ok { return false; }

    // 2. Branch clocks first
    let clocks: &[usize] = &[
        USB30_PRIM_CFG_NOC_AXI_CLK,
        USB30_PRIM_MASTER_CLK,
        USB30_PRIM_MOCK_UTMI_CLK,
        USB30_PRIM_SLEEP_CLK,
        USB30_PRIM_PHY_AUX_CLK,
        USB30_PRIM_PHY_COM_AUX_CLK,
        USB30_PRIM_AGGR_NOC_AXI_CLK,
        USB30_PRIM_CLKREF_CLK,
    ];
    for &clk in clocks {
        write_reg(clk, read_reg(clk) | CBCR_CLK_ENABLE);
        if !poll_until_clear(clk, CBCR_CLK_OFF) { return false; }
    }

    // 3. AHB2PHY BCR full reset (after GDSC + branch clocks)
    write_reg(USB_PHY_CFG_AHB2PHY_BCR, 1);
    for _ in 0..10_000 { core::hint::spin_loop(); }
    write_reg(USB_PHY_CFG_AHB2PHY_BCR, 0);
    for _ in 0..10_000 { core::hint::spin_loop(); }

    // 4. AHB2PHY clock enable
    write_reg(USB_PHY_CFG_AHB2PHY_CLK, (read_reg(USB_PHY_CFG_AHB2PHY_CLK) & !0x2) | CBCR_CLK_ENABLE);
    if !poll_until_clear(USB_PHY_CFG_AHB2PHY_CLK, CBCR_CLK_OFF) { return false; }

    // 4b. QUSB2PHY ref clock — needed for PHY register writes + PLL
    write_reg(QUSB2PHY_PRIM_CLK, read_reg(QUSB2PHY_PRIM_CLK) | CBCR_CLK_ENABLE);
    if !poll_until_clear(QUSB2PHY_PRIM_CLK, CBCR_CLK_OFF) { return false; }

    // 5. SKIP QUSB2PHY BCR reset — keep ABL PHY state!
    true
}

/// PHY ref clock + block reset. Call after VRM rails are on.
pub fn qusb2phy_clk_reset() -> bool {
    write_reg(QUSB2PHY_PRIM_CLK, read_reg(QUSB2PHY_PRIM_CLK) | CBCR_CLK_ENABLE);
    let _ = poll_until_clear(QUSB2PHY_PRIM_CLK, CBCR_CLK_OFF);
    write_reg(QUSB2PHY_PRIM_BCR, 1);
    for _ in 0..10_000 {
        core::hint::spin_loop();
    }
    write_reg(QUSB2PHY_PRIM_BCR, 0);
    poll_until_clear(QUSB2PHY_PRIM_BCR, 0x1)
}

/// Reset QUSB2PHY after regulators are enabled.
/// PHY needs this to properly initialize with power applied.
pub fn reset_qusb2phy() -> bool {
    write_reg(QUSB2PHY_PRIM_BCR, 1);
    for _ in 0..10_000 { core::hint::spin_loop(); }
    write_reg(QUSB2PHY_PRIM_BCR, 0);
    poll_until_clear(QUSB2PHY_PRIM_BCR, 0x1)
}
