//! Minimal DTB/FDT parser and DPU register access for Pixel 5 framebuffer

const FDT_MAGIC: u32 = 0xD00D_FEED;
const FDT_BEGIN_NODE: u32 = 1;
const FDT_END_NODE: u32 = 2;
const FDT_PROP: u32 = 3;
const FDT_NOP: u32 = 4;
const FDT_END: u32 = 9;

fn be32(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

fn be64(d: &[u8], o: usize) -> u64 {
    u64::from_be_bytes([d[o], d[o+1], d[o+2], d[o+3], d[o+4], d[o+5], d[o+6], d[o+7]])
}

fn align4(n: usize) -> usize { (n + 3) & !3 }

fn cstr(d: &[u8], o: usize) -> &str {
    let end = d[o..].iter().position(|&b| b == 0).unwrap_or(0);
    core::str::from_utf8(&d[o..o + end]).unwrap_or("")
}

fn read32(a: usize) -> u32 {
    unsafe { core::ptr::read_volatile(a as *const u32) }
}

fn write32(a: usize, v: u32) {
    unsafe { core::ptr::write_volatile(a as *mut u32, v) }
}

/// Read DPU SSPP registers to find current framebuffer address.
/// SM7250 DPU: MDSS@0xAE00000, MDP@0xAE01000
/// SSPP base offsets relative to MDP: 0x4000 or 0x5000
pub fn find_fb_from_dpu() -> Option<u64> {
    // Try both possible MDP bases
    let mdp_bases: &[usize] = &[0xAE01000, 0xAE00000];

    for &mdp in mdp_bases {
        // Read MDP revision to verify DPU is accessible
        let rev = read32(mdp);
        if rev == 0 || rev == 0xFFFFFFFF {
            continue;
        }

        // Try SSPP offsets: 0x4000, 0x5000, 0x6000 (VIG0, VIG1)
        let sspp_offsets: &[usize] = &[0x4000, 0x5000, 0x6000, 0x7000];

        for &sspp_off in sspp_offsets {
            let sspp = mdp + sspp_off;
            // SSPP_SRC_SIZE at +0x00 should have valid dimensions
            let src_size = read32(sspp);
            let src_w = (src_size & 0xFFFF) as usize;
            let src_h = ((src_size >> 16) & 0xFFFF) as usize;

            // Check for reasonable dimensions (1080x2340 or similar)
            if src_w >= 720 && src_w <= 2560 && src_h >= 1280 && src_h <= 3200 {
                // Found active pipe! Read SRC0_ADDR
                // Try multiple offsets for SRC0_ADDR
                for &addr_off in &[0x14usize, 0x1C, 0x28, 0x30] {
                    let val = read32(sspp + addr_off);
                    // Valid framebuffer address in RAM range
                    if val >= 0x80000000 && val < 0xC0000000 {
                        return Some(val as u64);
                    }
                    // Also try upper 32 bits
                    let val_hi = read32(sspp + addr_off + 4);
                    let full = ((val_hi as u64) << 32) | (val as u64);
                    if full >= 0x80000000 && full < 0x300000000 {
                        return Some(full);
                    }
                }
            }

            // Even if dimensions are 0, try reading SRC0_ADDR anyway
            for &addr_off in &[0x14usize, 0x1C, 0x28] {
                let val = read32(sspp + addr_off);
                if val >= 0x80000000 && val < 0xC0000000 {
                    // Verify by reading stride too
                    for &stride_off in &[0x18usize, 0x20, 0x24] {
                        let stride = read32(sspp + stride_off);
                        // Stride should be width * bpp (4320 for 1080*4)
                        if stride == 4320 || stride == 4352 || stride == 4096 || stride == 8192 {
                            return Some(val as u64);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Force DPU CTL flush to make our framebuffer writes visible
pub fn dpu_flush() {
    let mdp_bases: &[usize] = &[0xAE01000, 0xAE00000];
    for &mdp in mdp_bases {
        // CTL base at MDP + 0x2000, each CTL is 0x1E0 bytes
        for i in 0..5usize {
            let ctl = mdp + 0x2000 + i * 0x200;
            // CTL_FLUSH at offset 0x18
            // Bit 0 = flush MDP top
            // Write 0x1 to trigger flush
            write32(ctl + 0x18, 0x1);
        }
    }
    // Memory barrier
    unsafe { core::arch::asm!("dsb sy"); }
}

/// Scan RAM for the splash framebuffer by looking for non-zero pixel data
/// at the expected center-of-logo offset
pub fn scan_for_fb() -> Option<u64> {
    // 1080x2340 @ 32bpp, stride = 4320
    // Logo center is around row 1100-1200
    let stride: usize = 4320;
    let check_offsets: &[usize] = &[
        1100 * stride + 400 * 4,  // row 1100, col 400
        1150 * stride + 500 * 4,
        1200 * stride + 540 * 4,
    ];

    // Scan in 256KB steps from 0x80000000 to 0x100000000
    let mut base = 0x80000000u64;
    while base < 0x100000000 {
        let mut found = true;
        for &off in check_offsets {
            let addr = base as usize + off;
            // Read 4 bytes - should be non-zero if this is the logo area
            let val = read32(addr);
            if val == 0 {
                found = false;
                break;
            }
        }
        // Also check that the top-left is black (zeros)
        if found {
            let tl = read32(base as usize + 100 * stride);
            // And that some pixels in the logo area are non-zero
            if tl == 0 {
                return Some(base);
            }
        }
        base += 0x40000; // 256KB steps
    }
    None
}

/// Find interrupt-controller node in DTB and extract GICD/GICC base addresses.
/// Returns (gicd_base, gicc_base) from the first `reg` property in an
/// `interrupt-controller` compatible node.
pub fn find_gic(dtb: *const u8) -> Option<(usize, usize)> {
    let hdr = unsafe { core::slice::from_raw_parts(dtb, 40) };
    if be32(hdr, 0) != FDT_MAGIC {
        return None;
    }
    let total_size = be32(hdr, 4) as usize;
    let dtb_len = total_size.min(0x200000);
    let dtb = unsafe { core::slice::from_raw_parts(dtb, dtb_len) };

    let struct_off = be32(dtb, 8) as usize;
    let strings_off = be32(dtb, 12) as usize;
    let struct_size = be32(dtb, 36) as usize;
    let struct_end = struct_off + struct_size;

    if struct_off >= dtb.len() || struct_end > dtb.len() || strings_off >= dtb.len() {
        return None;
    }

    let s = &dtb[struct_off..struct_end];
    let strings = &dtb[strings_off..];

    let mut pos = 0;
    let mut in_intc = false;
    let mut intc_compatible = false;
    // Track #address-cells and #size-cells from the parent (/soc) node
    let mut addr_cells: usize = 2; // default for root
    let mut size_cells: usize = 1;
    // Stack of address-cells/size-cells per depth level
    let mut ac_stack: [usize; 8] = [2; 8];
    let mut sc_stack: [usize; 8] = [1; 8];
    let mut depth: usize = 0;

    while pos + 4 <= s.len() {
        let tok = be32(s, pos);
        pos += 4;

        match tok {
            FDT_BEGIN_NODE => {
                let nul = s[pos..].iter().position(|&b| b == 0).unwrap_or(0);
                let name = cstr(s, pos);
                pos = align4(pos + nul + 1);

                // Push current cells to stack for children
                if depth < 7 {
                    ac_stack[depth + 1] = ac_stack[depth];
                    sc_stack[depth + 1] = sc_stack[depth];
                }
                depth += 1;
                addr_cells = ac_stack[depth];
                size_cells = sc_stack[depth];

                in_intc = false;
                intc_compatible = false;
            }
            FDT_END_NODE => {
                if depth > 0 { depth -= 1; }
                addr_cells = ac_stack[depth];
                size_cells = sc_stack[depth];
                in_intc = false;
            }
            FDT_PROP => {
                if pos + 8 > s.len() { break; }
                let len = be32(s, pos) as usize;
                let nameoff = be32(s, pos + 4) as usize;
                pos += 8;
                let pname = if nameoff < strings.len() { cstr(strings, nameoff) } else { "" };

                // Update cells values for current node (used by children)
                if pname == "#address-cells" && len == 4 && pos + 4 <= s.len() {
                    let v = be32(s, pos) as usize;
                    ac_stack[depth] = v;
                    addr_cells = v;
                }
                if pname == "#size-cells" && len == 4 && pos + 4 <= s.len() {
                    let v = be32(s, pos) as usize;
                    sc_stack[depth] = v;
                    size_cells = v;
                }

                // Check for interrupt-controller compatible
                if pname == "compatible" {
                    // Check if value contains "arm,gic-400" or "arm,gic-v3" or "qcom,msm-qgic2"
                    if pos + len <= s.len() {
                        let compat = core::str::from_utf8(&s[pos..pos + len]).unwrap_or("");
                        if compat.contains("gic") || compat.contains("qgic") {
                            intc_compatible = true;
                            in_intc = true;
                        }
                    }
                }

                // Also match by node name containing "interrupt-controller"
                // (checked via FDT_BEGIN_NODE name above isn't reliable; use compatible)

                // If we're in a GIC node, read reg property
                if (in_intc || intc_compatible) && pname == "reg" {
                    // reg = (addr_cells * 4) + (size_cells * 4) per region
                    let entry_size = (addr_cells + size_cells) * 4;
                    if len >= entry_size && pos + entry_size <= s.len() {
                        // First entry = GICD
                        let gicd = if addr_cells >= 2 {
                            be64(s, pos) as usize
                        } else {
                            be32(s, pos) as usize
                        };
                        // Second entry = GICC (if present)
                        let gicc = if len >= entry_size * 2 && pos + entry_size * 2 <= s.len() {
                            let off = entry_size;
                            if addr_cells >= 2 {
                                be64(s, pos + off) as usize
                            } else {
                                be32(s, pos + off) as usize
                            }
                        } else {
                            gicd + 0x10000 // common offset for GICC from GICD
                        };
                        return Some((gicd, gicc));
                    }
                }

                pos = align4(pos + len);
            }
            FDT_NOP => {}
            FDT_END | _ => break,
        }
    }
    None
}

/// Find a node by compatible string and return its reg property.
/// Returns array of (addr, size) pairs (max 4 entries).
pub fn find_node_reg(dtb: *const u8, compat_match: &str) -> Option<[(usize, usize); 4]> {
    let hdr = unsafe { core::slice::from_raw_parts(dtb, 40) };
    if be32(hdr, 0) != FDT_MAGIC {
        return None;
    }
    let total_size = be32(hdr, 4) as usize;
    let dtb_len = total_size.min(0x200000);
    let dtb = unsafe { core::slice::from_raw_parts(dtb, dtb_len) };

    let struct_off = be32(dtb, 8) as usize;
    let strings_off = be32(dtb, 12) as usize;
    let struct_size = be32(dtb, 36) as usize;
    let struct_end = struct_off + struct_size;

    if struct_off >= dtb.len() || struct_end > dtb.len() || strings_off >= dtb.len() {
        return None;
    }

    let s = &dtb[struct_off..struct_end];
    let strings = &dtb[strings_off..];

    let mut pos = 0;
    let mut depth: usize = 0;
    let mut ac_stack: [usize; 8] = [2; 8];
    let mut sc_stack: [usize; 8] = [1; 8];
    let mut node_compat = false;
    let mut node_reg: [(usize, usize); 4] = [(0, 0); 4];
    let mut node_reg_count: usize = 0;

    while pos + 4 <= s.len() {
        let tok = be32(s, pos);
        pos += 4;

        match tok {
            FDT_BEGIN_NODE => {
                let nul = s[pos..].iter().position(|&b| b == 0).unwrap_or(0);
                pos = align4(pos + nul + 1);

                if depth < 7 {
                    ac_stack[depth + 1] = ac_stack[depth];
                    sc_stack[depth + 1] = sc_stack[depth];
                }
                depth += 1;

                node_compat = false;
                node_reg_count = 0;
            }
            FDT_END_NODE => {
                if node_compat && node_reg_count > 0 {
                    return Some(node_reg);
                }
                if depth > 0 { depth -= 1; }
                node_compat = false;
            }
            FDT_PROP => {
                if pos + 8 > s.len() { break; }
                let len = be32(s, pos) as usize;
                let nameoff = be32(s, pos + 4) as usize;
                pos += 8;
                let pname = if nameoff < strings.len() { cstr(strings, nameoff) } else { "" };

                let addr_cells = ac_stack[depth];
                let size_cells = sc_stack[depth];

                if pname == "#address-cells" && len == 4 && pos + 4 <= s.len() {
                    ac_stack[depth] = be32(s, pos) as usize;
                }
                if pname == "#size-cells" && len == 4 && pos + 4 <= s.len() {
                    sc_stack[depth] = be32(s, pos) as usize;
                }

                if pname == "compatible" && pos + len <= s.len() {
                    let compat = core::str::from_utf8(&s[pos..pos + len]).unwrap_or("");
                    if compat.contains(compat_match) {
                        node_compat = true;
                    }
                }

                if node_compat && pname == "reg" && node_reg_count < 4 {
                    let entry_size = (addr_cells + size_cells) * 4;
                    let entries = len / entry_size;
                    for i in 0..entries.min(4 - node_reg_count) {
                        let base = pos + i * entry_size;
                        if base + entry_size > s.len() { break; }
                        let addr = if addr_cells >= 2 { be64(s, base) as usize }
                                   else { be32(s, base) as usize };
                        let size = if size_cells >= 2 { be64(s, base + addr_cells * 4) as usize }
                                   else { be32(s, base + addr_cells * 4) as usize };
                        node_reg[node_reg_count] = (addr, size);
                        node_reg_count += 1;
                    }
                }

                pos = align4(pos + len);
            }
            FDT_NOP => {}
            FDT_END | _ => break,
        }
    }
    None
}

/// Find framebuffer region from DTB reserved-memory.
pub fn find_fb(dtb: *const u8, _max_len: usize) -> Option<(u64, u64)> {
    let hdr = unsafe { core::slice::from_raw_parts(dtb, 40) };
    if be32(hdr, 0) != FDT_MAGIC {
        return None;
    }
    let total_size = be32(hdr, 4) as usize;
    let dtb_len = total_size.min(0x200000);
    let dtb = unsafe { core::slice::from_raw_parts(dtb, dtb_len) };

    let struct_off = be32(dtb, 8) as usize;
    let strings_off = be32(dtb, 12) as usize;
    let struct_size = be32(dtb, 36) as usize;
    let struct_end = struct_off + struct_size;

    if struct_off >= dtb.len() || struct_end > dtb.len() || strings_off >= dtb.len() {
        return None;
    }

    let s = &dtb[struct_off..struct_end];
    let strings = &dtb[strings_off..];

    let mut pos = 0;
    let mut depth = 0u32;
    let mut in_rsv = false;
    let mut addr_cells: usize = 2;
    let mut size_cells: usize = 2;
    let mut candidate: Option<(u64, u64)> = None;

    while pos + 4 <= s.len() {
        let tok = be32(s, pos);
        pos += 4;

        match tok {
            FDT_BEGIN_NODE => {
                let nul = s[pos..].iter().position(|&b| b == 0).unwrap_or(0);
                let name = cstr(s, pos);
                pos = align4(pos + nul + 1);
                depth += 1;
                if name.contains("reserved-memory") {
                    in_rsv = true;
                }
            }
            FDT_END_NODE => {
                depth -= 1;
                if depth < 2 { in_rsv = false; }
            }
            FDT_PROP => {
                if pos + 8 > s.len() { break; }
                let len = be32(s, pos) as usize;
                let nameoff = be32(s, pos + 4) as usize;
                pos += 8;
                let pname = if nameoff < strings.len() { cstr(strings, nameoff) } else { "" };

                if pname == "#address-cells" && len == 4 && pos + 4 <= s.len() {
                    addr_cells = be32(s, pos) as usize;
                }
                if pname == "#size-cells" && len == 4 && pos + 4 <= s.len() {
                    size_cells = be32(s, pos) as usize;
                }

                if in_rsv && depth >= 3 && pname == "reg" {
                    let total = (addr_cells + size_cells) * 4;
                    if len >= total && pos + total <= s.len() {
                        let addr = if addr_cells >= 2 { be64(s, pos) } else { be32(s, pos) as u64 };
                        let sz = if size_cells >= 2 { be64(s, pos + addr_cells * 4) } else { be32(s, pos + addr_cells * 4) as u64 };
                        if sz >= 0x100_000 && sz <= 0x400_0000 {
                            candidate = Some((addr, sz));
                        }
                    }
                }
                pos = align4(pos + len);
            }
            FDT_NOP => {}
            FDT_END | _ => break,
        }
    }
    candidate
}
