//! RPMh TCS driver for SM7250 (Pixel 5)
//!
//! APPS_RSC DRV2 = 0x18220000 (DTB: qcom,drv-id = <0x02>)
//! DTB: qcom,tcs-offset = <0xD00>, TCS stride = 0x2A0
//!
//! Fire-and-forget: set AMC_MODE=1, fixed delay, then clear.
//! AMC_MODE doesn't auto-clear on this hardware but commands execute.

const DRV2: usize = 0x1822_0000;
const TCS_OFFSET: usize = 0xD00;
const TCS_STRIDE: usize = 0x2A0;

// Linux rpmh-rsc: v2.7 vs v3.0 (picked from RSC_DRV_ID major).
#[derive(Clone, Copy)]
struct TcsLayout {
    irq_enable: usize,
    control: usize,
    cmd_enable: usize,
    msgid: usize,
    addr: usize,
    data: usize,
    cmd_status: usize,
    cmd_stride: usize,
}

const LAY_V27: TcsLayout = TcsLayout {
    irq_enable: 0x00,
    control: 0x14,
    cmd_enable: 0x1C,
    msgid: 0x30,
    addr: 0x34,
    data: 0x38,
    cmd_status: 0x3C,
    cmd_stride: 20,
};
const LAY_V30: TcsLayout = TcsLayout {
    irq_enable: 0x00,
    control: 0x24,
    cmd_enable: 0x2C,
    msgid: 0x34,
    addr: 0x38,
    data: 0x3C,
    cmd_status: 0x40,
    cmd_stride: 24,
};

const AMC_ENABLE: u32 = 1 << 16;
const AMC_TRIGGER: u32 = 1 << 24;
const CMD_MSGID_WRITE: u32 = 1 << 16;
const CMD_MSGID_LEN: u32 = 8;
const CMD_MSGID_RESP: u32 = 1 << 8;
const CMD_STATUS_COMPL: u32 = 1 << 16;

fn read32(addr: usize) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

fn write32(addr: usize, val: u32) {
    unsafe {
        core::ptr::write_volatile(addr as *mut u32, val);
        core::ptr::read_volatile(addr as *const u32);
    }
}

fn tcs_base(tcs_id: usize) -> usize {
    DRV2 + TCS_OFFSET + tcs_id * TCS_STRIDE
}

pub fn dump_tcs(con: &mut crate::fb::Console) {
    con.puts("[rpmh] D2:");
    for off in (0..0x40).step_by(4) {
        let v = read32(DRV2 + off);
        if v != 0 {
            con.put_hex8(off as u8);
            con.puts("="); con.put_hex32(v);
            con.puts(" ");
        }
    }
    con.puts("\r\n"); con.flush();
}

fn layout() -> TcsLayout {
    let major = (read32(DRV2) >> 16) & 0xff;
    if major == 3 {
        LAY_V30
    } else {
        LAY_V27
    }
}

fn write_cmd(tcs: usize, lay: TcsLayout, slot: usize, msgid: u32, addr: u32, data: u32) {
    let o = slot * lay.cmd_stride;
    write32(tcs + lay.msgid + o, msgid);
    write32(tcs + lay.addr + o, addr);
    write32(tcs + lay.data + o, data);
}

fn fire_triple(tcs_id: usize, vrm: u32, mv: u32) -> u32 {
    let lay = layout();
    let tcs = tcs_base(tcs_id);
    // Enable IRQ for active TCS 0/1 (Linux probe does this).
    write32(DRV2 + TCS_OFFSET + lay.irq_enable, 0x3);

    let mut ctl = read32(tcs + lay.control);
    write32(tcs + lay.control, ctl & !AMC_TRIGGER);
    ctl = read32(tcs + lay.control);
    write32(tcs + lay.control, ctl & !AMC_ENABLE);
    write32(tcs + lay.cmd_enable, 0);

    let mid = CMD_MSGID_LEN | CMD_MSGID_WRITE;
    let mid_w = mid | CMD_MSGID_RESP;
    write_cmd(tcs, lay, 0, mid, vrm, mv);
    write_cmd(tcs, lay, 1, mid, vrm | 8, 3);
    write_cmd(tcs, lay, 2, mid_w, vrm | 4, 1);
    write32(tcs + lay.cmd_enable, 0x7);

    write32(tcs + lay.control, AMC_ENABLE);
    for _ in 0..10_000 {
        core::hint::spin_loop();
    }
    write32(tcs + lay.control, AMC_ENABLE | AMC_TRIGGER);

    let mut st = 0;
    let st_off = tcs + lay.cmd_status + 2 * lay.cmd_stride;
    for _ in 0..2_000_000 {
        st = read32(st_off);
        if st & CMD_STATUS_COMPL != 0 {
            break;
        }
        core::hint::spin_loop();
    }
    st
}

/// Enable a VRM regulator. `voltage` is millivolts (Linux VRM units).
pub fn vrm_enable_full(vrm_addr: u32, voltage_mv: u32, con: &mut crate::fb::Console) -> bool {
    if vrm_addr == 0 {
        return false;
    }
    con.puts("[rpmh] mV=");
    con.put_hex32(voltage_mv);
    con.puts(" ");
    con.flush();
    let mut st = fire_triple(0, vrm_addr, voltage_mv);
    if st & CMD_STATUS_COMPL == 0 {
        st = fire_triple(1, vrm_addr, voltage_mv);
    }
    unsafe { LAST_ST = st; }
    con.puts("st=");
    con.put_hex32(st);
    con.puts("\r\n");
    con.flush();
    st & CMD_STATUS_COMPL != 0
}

static mut LAST_ST: u32 = 0;
pub fn last_st() -> u32 {
    unsafe { LAST_ST }
}

pub fn rsc_id() -> u32 {
    read32(DRV2)
}
