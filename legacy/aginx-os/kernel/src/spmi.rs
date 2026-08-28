//! SPMI PMIC Arbiter v5 driver for SM7250 / SC7180 (Pixel 5)
//!
//! Register regions (from sc7180.dtsi):
//!   core:   0x0C440000 (0x1100)  — core registers + APID map
//!   chnls:  0x0C600000 (32MB)    — TX (write) channels
//!   obsrvr: 0x0E600000 (1MB)     — observer / RX (read) channels
//!   intr:   0x0E700000 (640KB)   — interrupt controller
//!   cnfg:   0x0C40A000 (152KB)   — configuration
//!   qcom,ee = <0>

const SPMI_BASE: usize = 0x0C44_0000;  // core
const OBS_BASE: usize = 0x0E60_0000;   // observer (RX read channels)
const CHNLS_BASE: usize = 0x0C60_0000; // TX (write) channels
const APID_MAP_BASE: usize = 0x900;    // v5: core + 0x900

// Channel register offsets (within 0x80-byte channel)
const CH_CMD:    usize = 0x00;
const CH_STATUS: usize = 0x08;
const CH_WDATA0: usize = 0x10;
const CH_RDATA0: usize = 0x18;

// Status bits
const STATUS_DONE:    u32 = 1 << 0;
const STATUS_FAILURE: u32 = 1 << 1;
const STATUS_DENIED:  u32 = 1 << 2;

// SPMI opcodes (v5 arbiter internal opcodes, NOT SPMI wire protocol)
const OP_EXT_READ: u32 = 13;
const OP_EXT_WRITE: u32 = 2;    // EXT_WRITE = 2 (not 14!)

fn read_reg(off: usize) -> u32 {
    unsafe { core::ptr::read_volatile((SPMI_BASE + off) as *const u32) }
}

/// Check if SPMI arbiter is present (version register valid)
pub fn is_present() -> bool {
    let ver = read_reg(0x0000);
    ver != 0 && ver != 0xFFFF_FFFF
}

/// Get version register (offset 0x00)
pub fn get_version() -> u32 { read_reg(0x0000) }

/// Read arbitrary SPMI core register for diagnostics
pub fn read_diag(off: usize) -> u32 { read_reg(off) }

/// Get APID count from config register (offset 0x04, bits [10:0])
pub fn get_apid_count() -> u32 { read_reg(0x04) & 0x7FF }

/// Get channel count (offset 0x88)
pub fn get_chcnt() -> u32 { read_reg(0x88) }

/// Write PPID to an APID mapping table entry.
/// Returns true if the write was verified by read-back.
pub fn apid_map_write(apid: u32, ppid: u16) -> bool {
    let val = (ppid as u32) << 8;
    unsafe {
        core::ptr::write_volatile((SPMI_BASE + APID_MAP_BASE + apid as usize * 4) as *mut u32, val);
        core::arch::asm!("dsb sy");
        let rb = core::ptr::read_volatile((SPMI_BASE + APID_MAP_BASE + apid as usize * 4) as *const u32);
        ((rb >> 8) & 0xFFF) as u16 == ppid
    }
}

/// Read APID mapping entry (core+0x900 + apid*4)
/// Returns raw value; PPID = (val >> 8) & 0xFFF
pub fn apid_map(apid: u32) -> u32 {
    read_reg(APID_MAP_BASE + apid as usize * 4)
}

/// Find APID for a given PPID (SID<<8 | PID)
pub fn find_apid(target_ppid: u16) -> Option<u32> {
    let count = get_apid_count();
    for apid in 0..count {
        let entry = apid_map(apid);
        let ppid = ((entry >> 8) & 0xFFF) as u16;
        if ppid == target_ppid {
            return Some(apid);
        }
    }
    None
}

/// Issue observer read command for a given APID
/// Returns (cmd_readback, status, rdata)
/// addr: 16-bit register address within the peripheral
pub fn obs_cmd_read(apid: u32, addr: u16, len: u8) -> (u32, u32, u32) {
    let ee = 0usize; // HLOS EE (qcom,ee = <0>)
    let channel = OBS_BASE + 0x10000 * ee + 0x80 * apid as usize;
    let bc = if len > 8 { 7u32 } else { (len - 1) as u32 };
    let cmd = (OP_EXT_READ << 27) | ((addr as u32) << 4) | (bc & 0x7);

    unsafe {
        core::ptr::write_volatile((channel + CH_CMD) as *mut u32, cmd);
        core::arch::asm!("dsb sy");
        let cmd_rb = core::ptr::read_volatile((channel + CH_CMD) as *const u32);
        let mut status = 0u32;
        for _ in 0..200_000 {
            status = core::ptr::read_volatile((channel + CH_STATUS) as *const u32);
            if status & STATUS_DONE != 0 { break; }
            core::hint::spin_loop();
        }
        let rdata = core::ptr::read_volatile((channel + CH_RDATA0) as *const u32);
        (cmd_rb, status, rdata)
    }
}

/// Issue TX write command for a given APID
/// TX channel offset: CHNLS_BASE + 0x10000 * apid (v5, no EE multiplexing for writes)
/// Returns (cmd_readback, status)
pub fn chn_cmd_write(apid: u32, addr: u16, data: &[u8]) -> (u32, u32) {
    let channel = CHNLS_BASE + 0x80 * apid as usize; // v5: same 0x80 stride as observer
    let bc = if data.len() > 8 { 7u32 } else { (data.len() - 1) as u32 };
    // v5 cmd format: (opcode << 27) | ((addr & 0xFF) << 4) | (bc & 0x7)
    // Address is low 8 bits only (register offset within peripheral)
    let cmd = (OP_EXT_WRITE << 27) | (((addr & 0xFF) as u32) << 4) | (bc & 0x7);

    unsafe {
        // Pack write data (little-endian byte order)
        let mut wdata = 0u32;
        for (i, &b) in data.iter().enumerate() {
            wdata |= (b as u32) << (i * 8);
        }
        core::ptr::write_volatile((channel + CH_WDATA0) as *mut u32, wdata);
        core::ptr::write_volatile((channel + CH_CMD) as *mut u32, cmd);
        core::arch::asm!("dsb sy");

        let mut status = 0u32;
        for _ in 0..200_000 {
            status = core::ptr::read_volatile((channel + CH_STATUS) as *const u32);
            if status & STATUS_DONE != 0 { break; }
            core::hint::spin_loop();
        }
        let cmd_rb = core::ptr::read_volatile((channel + CH_CMD) as *const u32);
        (cmd_rb, status)
    }
}

/// Enable an LDO regulator with HPM mode
/// ldo_ppid: PPID = (SID << 8) | PID for the LDO peripheral
/// Returns (found, write_ok)
pub fn ldo_enable_hpm(ldo_ppid: u16) -> (bool, bool) {
    let apid = match find_apid(ldo_ppid) {
        Some(a) => a,
        None => return (false, false),
    };
    // Set HPM mode first (offset 0x45)
    let (_, mstat) = chn_cmd_write(apid, 0x45, &[0x80]);
    // Enable (offset 0x46)
    let (_, estat) = chn_cmd_write(apid, 0x46, &[0x80]);
    let ok = (estat & STATUS_DONE != 0) && (estat & (STATUS_FAILURE | STATUS_DENIED) == 0);
    (true, ok)
}

/// Read a single byte from a peripheral via observer channel
pub fn obs_read_byte(apid: u32, addr: u16) -> u8 {
    let (_, _, rdata) = obs_cmd_read(apid, addr, 1);
    (rdata & 0xFF) as u8
}

/// Read from observer channel directly (raw offset from OBS_BASE)
pub fn obs_read(off: usize) -> u32 {
    unsafe { core::ptr::read_volatile((OBS_BASE + off) as *const u32) }
}

/// Issue observer WRITE command for a given APID (bypass TX channel)
/// Uses the observer channel with OP_EXT_WRITE opcode instead of TX channel.
/// Returns (status, cmd_readback)
pub fn obs_cmd_write(apid: u32, addr: u16, data: &[u8]) -> (u32, u32) {
    let ee = 0usize;
    let channel = OBS_BASE + 0x10000 * ee + 0x80 * apid as usize;
    let bc = if data.len() > 8 { 7u32 } else { (data.len() - 1) as u32 };
    let cmd = (OP_EXT_WRITE << 27) | ((addr as u32) << 4) | (bc & 0x7);

    unsafe {
        // Pack write data (little-endian)
        let mut wdata = 0u32;
        for (i, &b) in data.iter().enumerate() {
            wdata |= (b as u32) << (i * 8);
        }
        core::ptr::write_volatile((channel + CH_WDATA0) as *mut u32, wdata);
        core::ptr::write_volatile((channel + CH_CMD) as *mut u32, cmd);
        core::arch::asm!("dsb sy");

        let mut status = 0u32;
        for _ in 0..200_000 {
            status = core::ptr::read_volatile((channel + CH_STATUS) as *const u32);
            if status & STATUS_DONE != 0 { break; }
            core::hint::spin_loop();
        }
        let cmd_rb = core::ptr::read_volatile((channel + CH_CMD) as *const u32);
        (status, cmd_rb)
    }
}

/// Get observer base for display
pub const fn obs_base() -> usize { OBS_BASE }

/// Dump first N non-zero APID table entries to framebuffer console
pub fn dump_apid_table(con: &mut crate::fb::Console) {
    let count = get_apid_count();
    con.puts("APID table (first 32 non-zero):\r\n");
    let mut shown = 0u32;
    for apid in 0..count {
        let entry = apid_map(apid);
        if entry != 0 {
            let ppid = ((entry >> 8) & 0xFFF) as u16;
            let sid = (ppid >> 8) & 0xF;
            let pid = ppid & 0xFF;
            con.puts(" [");
            crate::print_dec_u32(crate::platform::UART, apid);
            con.puts("] ppid=0x");
            crate::print_hex(crate::platform::UART, ppid as u32);
            con.puts(" sid=");
            crate::print_dec_u32(crate::platform::UART, sid as u32);
            con.puts(" pid=0x");
            crate::print_hex(crate::platform::UART, pid as u32);
            con.puts(" raw=0x");
            crate::print_hex(crate::platform::UART, entry);
            con.puts("\r\n");
            con.flush();
            shown += 1;
            if shown >= 32 { break; }
        }
    }
    if shown == 0 {
        con.puts("  (all zero — APID map at wrong offset?)\r\n");
    }
}

/// Scan observer channels, find LDO peripherals, and enable them.
/// LDO type = 0x1A. For each found LDO, set HPM mode and enable.
/// Only prints results (found + enable status) to framebuffer.
/// Returns number of LDOs successfully enabled.
/// Dump first N peripheral types to console for diagnostics
pub fn dump_types(con: &mut crate::fb::Console, max: u32) {
    let count = if get_apid_count() > max { max } else { get_apid_count() };
    con.puts("Types:"); con.flush();
    for apid in 0..count {
        let (_, status, rdata) = obs_cmd_read(apid, 0x04, 1);
        if (status & STATUS_DONE) == 0 || (status & (STATUS_FAILURE | STATUS_DENIED)) != 0 {
            continue;
        }
        let typ = (rdata & 0xFF) as u8;
        if typ == 0 { continue; }
        con.puts(" A"); fb_put_dec(con, apid);
        con.puts("=0x"); con.put_hex32(typ as u32);
        con.flush();
    }
    con.puts("\r\n"); con.flush();
}

pub fn scan_and_enable_ldos(con: &mut crate::fb::Console) -> u32 {
    // Scan fixed range 0..256 regardless of reported APID count
    // (get_apid_count() may be unreliable on some platforms)
    let max = 256u32;
    con.puts("Scan 256 APIDs\r\n"); con.flush();
    let mut enabled = 0u32;
    for apid in 0..max {
        let (_, status, rdata) = obs_cmd_read(apid, 0x04, 1);
        if (status & STATUS_DONE) == 0 || (status & (STATUS_FAILURE | STATUS_DENIED)) != 0 || rdata == 0 {
            continue;
        }
        let typ = (rdata & 0xFF) as u8;
        con.puts("A="); fb_put_dec(con, apid);
        con.puts(" T=0x"); con.put_hex32(typ as u32);
        con.puts("\r\n"); con.flush();
        if typ != 0x1A { continue; } // Not an LDO
        // Found an LDO — try to enable it
        let ok = ldo_enable_by_apid(apid);
        con.puts("LDO"); fb_put_dec(con, apid);
        if ok { con.puts(" OK\r\n"); } else { con.puts(" DEN\r\n"); }
        con.flush();
        if ok { enabled += 1; }
    }
    con.puts("LDOs=");
    fb_put_dec(con, enabled);
    con.puts("\r\n"); con.flush();
    enabled
}

/// Print decimal u32 to framebuffer console
fn fb_put_dec(con: &mut crate::fb::Console, mut n: u32) {
    if n == 0 { con.puts("0"); return; }
    let mut buf: [u8; 10] = [0; 10];
    let mut i = 0usize;
    while n > 0 { buf[i] = b'0' + (n % 10) as u8; n /= 10; i += 1; }
    while i > 0 { i -= 1; con.puts(core::str::from_utf8(&[buf[i]]).unwrap_or("?")); }
}

/// Enable LDO by APID directly (bypasses PPID lookup).
/// Set HPM mode then enable.
pub fn ldo_enable_by_apid(apid: u32) -> bool {
    // Use fixed TX channel address (0x80 stride, same as observer)
    let (_, mstat) = chn_cmd_write(apid, 0x45, &[0x80]);
    let (_, estat) = chn_cmd_write(apid, 0x46, &[0x80]);
    (estat & STATUS_DONE != 0) && (estat & (STATUS_FAILURE | STATUS_DENIED) == 0)
}

/// Program one PM6350 LDO: set voltage, HPM mode, enable.
/// ppid: (SID<<8)|PID_upper_byte
/// sel: voltage selector (0-based, range depends on LDO type)
/// Returns (found, ok)
pub fn program_ldo(con: &mut crate::fb::Console, ppid: u16, sel: u8, name: &str) -> (bool, bool) {
    let apid = match find_apid(ppid) {
        Some(a) => a,
        None => {
            con.puts(" "); con.puts(name); con.puts(":noAPID");
            con.flush();
            return (false, false);
        }
    };
    // Read current state
    let typ = obs_read_byte(apid, 0x04);
    let sub = obs_read_byte(apid, 0x05);
    let cur_lo = obs_read_byte(apid, 0x40);
    let cur_hi = obs_read_byte(apid, 0x41);
    let cur_en = obs_read_byte(apid, 0x46);
    let cur_val = ((cur_hi as u32) << 8) | cur_lo as u32;
    con.puts(" "); con.puts(name);
    con.puts(":A"); fb_put_dec(con, apid);
    con.puts(" T"); con.put_hex8(typ);
    con.puts("S"); con.put_hex8(sub);
    con.puts(" V="); fb_put_dec(con, cur_val);
    con.puts(" E="); con.put_hex8(cur_en);
    con.flush();

    // Set voltage (2-byte write: selector as 16-bit LE)
    let (st_v, _) = obs_cmd_write(apid, 0x40, &[sel, 0]);
    // HPM mode
    let _ = obs_cmd_write(apid, 0x45, &[0x80]);
    // Enable
    let (st_e, _) = obs_cmd_write(apid, 0x46, &[0x80]);

    // Read back
    let rb_lo = obs_read_byte(apid, 0x40);
    let rb_hi = obs_read_byte(apid, 0x41);
    let rb_val = ((rb_hi as u32) << 8) | rb_lo as u32;
    let rb_en = obs_read_byte(apid, 0x46);
    let v_ok = (st_v & STATUS_DONE != 0) && (st_v & (STATUS_FAILURE | STATUS_DENIED) == 0);
    let e_ok = (st_e & STATUS_DONE != 0) && (st_e & (STATUS_FAILURE | STATUS_DENIED) == 0);
    con.puts(" ->V"); fb_put_dec(con, rb_val);
    con.puts(" E"); con.put_hex8(rb_en);
    con.puts(if v_ok && e_ok { "ok" } else { "X" });
    con.flush();
    (true, v_ok && e_ok)
}
