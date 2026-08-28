//! CMD-DB (Command Database) parser for SM7250 (Pixel 5)
//!
//! CMD-DB is a firmware-populated database in DRAM containing RPMh resource addresses.
//! Populated by XBL/ABL at boot, used to find VRM addresses for PMIC regulator control.
//!
//! Base address: 0x80860000 (128KB, from sm6350.dtsi)
//! Previous experiments used WRONG address 0x00C80000 (SC7180), which hung.

const CMD_DB_BASE: usize = 0x8086_0000;

const MAGIC: [u8; 4] = [0xdb, 0x30, 0x03, 0x0c];
const MAX_RSC_HDR: usize = 32;
/// Mainline: version+magic+rsc_hdr[8]+csum+res = 144
const DATA_8: usize = 144;
/// Wide (32 headers): 528
const DATA_32: usize = 528;

fn read8(off: usize) -> u8 {
    unsafe { core::ptr::read_volatile((CMD_DB_BASE + off) as *const u8) }
}

fn read16(off: usize) -> u16 {
    unsafe { core::ptr::read_volatile((CMD_DB_BASE + off) as *const u16) }
}

fn read32(off: usize) -> u32 {
    unsafe { core::ptr::read_volatile((CMD_DB_BASE + off) as *const u32) }
}

/// Check if CMD-DB is present and valid
pub fn is_present() -> bool {
    read8(4) == MAGIC[0]
        && read8(5) == MAGIC[1]
        && read8(6) == MAGIC[2]
        && read8(7) == MAGIC[3]
}

/// Read CMD-DB version
pub fn version() -> u32 {
    read32(0)
}

/// Read first 16 bytes of CMD-DB for diagnostics
pub fn probe_header() -> [u8; 16] {
    let mut buf = [0u8; 16];
    for i in 0..16 {
        buf[i] = read8(i);
    }
    buf
}

fn name_eq(entry: usize, id: &[u8]) -> bool {
    let cmp = id.len().min(8);
    for k in 0..cmp {
        if read8(entry + k) != id[k] {
            return false;
        }
    }
    cmp == 8 || read8(entry + cmp) == 0
}

fn read_addr_at(data_start: usize, n_hdr: usize, id: &[u8]) -> u32 {
    for i in 0..n_hdr {
        let base = 8 + i * 16;
        let cnt = read16(base + 6) as usize;
        if cnt == 0 || cnt > 256 {
            continue;
        }
        let header_offset = read16(base + 2) as usize;
        for j in 0..cnt {
            let e = data_start + header_offset + j * 24;
            if e + 20 >= 0x8000 {
                break;
            }
            if name_eq(e, id) {
                let addr = read32(e + 16);
                if addr != 0 && addr != 0xffff_ffff {
                    return addr;
                }
            }
        }
    }
    0
}

/// Prefer mainline 8-header table (data@144), then 32-header (data@528).
pub fn find_vrm(id: &[u8]) -> u32 {
    let a = read_addr_at(DATA_8, 8, id);
    if a != 0 {
        return a;
    }
    read_addr_at(DATA_32, 32, id)
}

/// Print rsc headers + ldo names from the 8-header (144) table.
pub fn dump_ldo_scan(con: &mut crate::fb::Console) {
    con.puts("[hdr]");
    for i in 0..8 {
        let base = 8 + i * 16;
        let slv = read16(base);
        let cnt = read16(base + 6);
        if slv == 0 && cnt == 0 {
            continue;
        }
        con.puts(" ");
        con.put_hex8(i as u8);
        con.puts(":");
        con.put_hex16(slv);
        con.puts("/");
        con.put_hex16(cnt);
    }
    con.puts("\r\n[vrm]\r\n");
    con.flush();
    let mut shown = 0u32;
    for i in 0..8 {
        let base = 8 + i * 16;
        let slv = read16(base);
        let cnt = read16(base + 6) as usize;
        // slv 4 = VRM (LDOs). Skip BCM/ARC so ldoa* actually shows.
        if slv != 4 || cnt == 0 || cnt > 256 {
            continue;
        }
        let hoff = read16(base + 2) as usize;
        for j in 0..cnt {
            let e = DATA_8 + hoff + j * 24;
            if e + 8 >= 0x8000 {
                break;
            }
            let c0 = read8(e);
            if c0 != b'l' {
                continue;
            }
            for k in 0..8 {
                let c = read8(e + k);
                if c == 0 {
                    break;
                }
                if c >= 0x20 && c < 0x7f {
                    con.putc(c);
                }
            }
            con.puts("=");
            con.put_hex32(read32(e + 16));
            con.puts(" ");
            shown += 1;
            if shown % 3 == 0 {
                con.puts("\r\n");
            }
            if shown >= 21 {
                con.puts("\r\n");
                con.flush();
                return;
            }
        }
    }
    con.puts("\r\n");
    con.flush();
}

/// Find VRM address for a given resource ID string (e.g., b"ldoa2")
pub fn read_addr(id: &[u8]) -> u32 {
    find_vrm(id)
}

/// Dump ALL entry names and addresses (not just ldo*)
pub fn dump_all_names(con: &mut crate::fb::Console) {
    if !is_present() {
        con.puts("[cmddb] BAD MAGIC\r\n");
        con.flush();
        return;
    }

    let mut total = 0u32;
    for i in 0..MAX_RSC_HDR {
        let base = 8 + i * 16;
        let cnt = read16(base + 6);
        if cnt == 0 {
            continue;
        }
        let header_offset = read16(base + 2) as usize;

        for j in 0..cnt {
            let entry_abs = DATA_8 + header_offset + j as usize * 24;
            let addr = read32(entry_abs + 16);

            // Print: "name=addr "
            for k in 0..8 {
                let c = read8(entry_abs + k);
                if c == 0 {
                    break;
                }
                if c >= 0x20 && c < 0x7f {
                    con.puts(core::str::from_utf8(&[c]).unwrap_or("."));
                } else {
                    con.puts(".");
                }
            }
            con.puts("=");
            con.put_hex32(addr);
            con.puts(" ");
            con.flush();
            total += 1;
            // Line break every 6 entries
            if total % 6 == 0 {
                con.puts("\r\n");
                con.flush();
            }
        }
    }
    con.puts("\r\n[cmddb] total=");
    con.put_hex32(total);
    con.puts("\r\n");
    con.flush();
}

/// Dump only ldo* entries (compact, one line)
pub fn dump_vrm_names(con: &mut crate::fb::Console) {
    if !is_present() {
        con.puts("[cmddb] BAD MAGIC\r\n");
        con.flush();
        return;
    }

    let mut total = 0u32;
    for i in 0..MAX_RSC_HDR {
        let base = 8 + i * 16;
        let cnt = read16(base + 6);
        if cnt == 0 {
            continue;
        }
        let header_offset = read16(base + 2) as usize;

        for j in 0..cnt {
            let entry_abs = DATA_8 + header_offset + j as usize * 24;
            let id0 = read8(entry_abs);
            if id0 != b'l' {
                continue;
            }
            let addr = read32(entry_abs + 16);

            for k in 0..8 {
                let c = read8(entry_abs + k);
                if c == 0 {
                    break;
                }
                con.puts(core::str::from_utf8(&[c]).unwrap_or("."));
            }
            con.puts("=");
            con.put_hex32(addr);
            con.puts(" ");
            con.flush();
            total += 1;
        }
    }
    con.puts("\r\n[cmddb] ldo=");
    con.put_hex32(total);
    con.puts("\r\n");
    con.flush();
}

/// Dump CMD-DB RSC header summary (compact)
pub fn dump_headers(con: &mut crate::fb::Console) {
    if !is_present() {
        con.puts("[cmddb] BAD MAGIC\r\n");
        con.flush();
        return;
    }
    for i in 0..MAX_RSC_HDR {
        let base = 8 + i * 16;
        let slv_id = read16(base);
        let cnt = read16(base + 6);
        if cnt == 0 {
            continue;
        }
        con.put_hex8(i as u8);
        con.puts(":s");
        con.put_hex16(slv_id);
        con.puts("c");
        con.put_hex16(cnt);
        con.puts(" ");
        con.flush();
    }
    con.puts("\r\n");
    con.flush();
}
