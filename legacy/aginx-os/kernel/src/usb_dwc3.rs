//! DWC3 USB gadget driver with CDC ACM serial console
//!
//! DWC3 (DesignWare Core) USB controller on SM7250 at PA 0x0a600000
//! Implements USB CDC ACM class for serial console over USB-C
//!
//! Register map from Linux kernel drivers/usb/dwc3/core.h

// ── DWC3 Register Offsets ──────────────────────────────────────────

/// DWC3 core base address
const DWC3_BASE: usize = 0x0a60_0000;
/// Qualcomm DWC3 wrapper
const QSCRATCH_BASE: usize = 0x0a6f_8800;

// Global registers
const GCTL: usize       = 0xc110;
const GSNPSID: usize    = 0xc120;
const GUSB2PHYCFG: usize = 0xc200;
const GUSB2PHYACC0: usize = 0xc280; // USB2 PHY register access through DWC3
const GTXFIFOSIZ: usize = 0xc300;
const GRXFIFOSIZ: usize = 0xc380;
const GEVNTADRLO: usize = 0xc400;
const GEVNTADRH: usize  = 0xc404;
const GEVNTSIZ: usize   = 0xc408;
const GEVNTCOUNT: usize = 0xc40c;

// Device registers
const DCFG: usize      = 0xc700;
const DCTL: usize      = 0xc704;
const DEVTEN: usize    = 0xc708;
const DSTS: usize      = 0xc70c;
const DGCMDPAR: usize  = 0xc710;
const DGCMD: usize     = 0xc714;
const DALEPENA: usize  = 0xc720;

// Endpoint command registers: base + 0xc800 + ep*0x10
fn dep_base(ep: usize) -> usize { 0xc800 + ep * 0x10 }
// Note: DWC3 spec PAR0=+0x00, PAR1=+0x04, PAR2=+0x08 (same as Linux)
fn dep_cmdpar0(ep: usize) -> usize { dep_base(ep) + 0x00 }
fn dep_cmdpar1(ep: usize) -> usize { dep_base(ep) + 0x04 }
fn dep_cmdpar2(ep: usize) -> usize { dep_base(ep) + 0x08 }
fn dep_cmd_reg(ep: usize) -> usize { dep_base(ep) + 0x0c }

// ── Bit Definitions ────────────────────────────────────────────────

// GCTL
const GCTL_PRTCAPDIR_MASK: u32 = 0x3 << 12;
const GCTL_PRTCAP_DEVICE: u32 = 0x2 << 12;
const GCTL_CORESOFTRESET: u32 = 1 << 11;
const GCTL_DSBLCLKGTNG: u32 = 1 << 0;

// DCTL (Device Control Register)
// NOTE: DWC3 does NOT have a "SOFTDISCONNECT" or "SFTCONN" bit!
// Connect/disconnect is done solely via RUN_STOP (bit 31).
// Bit 7 is part of ULSTCHNGREQ field (bits [8:5]), NOT a disconnect control.
// Bit 16 is CSS (Cold Save Status), read-only.
const DCTL_RUN_STOP: u32 = 1 << 31;  // 0=Stop/Disconnect, 1=Run/Connect
const DCTL_CSFTRST: u32  = 1 << 30;  // Core Soft Reset (device-side, self-clearing)
const DCTL_LSFTRST: u32  = 1 << 29;  // Logical Soft Reset (pre-v1.87a)

// DCFG
const DCFG_SPEED_MASK: u32 = 0x7;
const DCFG_HIGHSPEED: u32  = 0x0;
const DCFG_FULLSPEED: u32  = 0x1;
const DCFG_DEVADDR_SHIFT: u32 = 3;
const DCFG_DEVADDR_MASK: u32 = 0x7f << 3;

// DEVTEN
const DEVTEN_DISCONN: u32    = 1 << 0;
const DEVTEN_USBRST: u32     = 1 << 1;
const DEVTEN_CONNECTDONE: u32 = 1 << 2;
const DEVTEN_ULSTCHNG: u32   = 1 << 3;

// DEPCMD commands (DWC3 spec: 0=set_stall, 1=reserved, 2=get_state, 3=set_ep_config, 4=set_xfer_resource)
const DEPCMD_DEPSETEPCONFIG: u32 = 0x03;
const DEPCMD_DEPSETTRANSF: u32   = 0x04;
const DEPCMD_DEPSTARTTRANSFER: u32 = 0x06;
const DEPCMD_DEPENDTRANSFER: u32   = 0x08;
const DEPCMD_DEPSTARTCFG: u32      = 0x09;
const DEPCMD_CMDACT: u32 = 1 << 10;
const DEPCMD_CMDIOC: u32 = 1 << 8;

// DSTS
const DSTS_USBLNKST_MASK: u32 = 0xf << 22;
const DSTS_USBLNKST_U3: u32 = 0x3 << 22; // suspended

// ── DWC3 Physical Endpoint mapping ─────────────────────────────────
// DWC3 uses even physical EPs for OUT, odd for IN
// EP0: phys 0 (OUT) + 1 (IN) — control
// EP1: phys 2 (OUT, bulk data) + 3 (IN, interrupt notify)
// EP2: phys 4 (OUT, bulk data) + 5 (IN, bulk data)
// Actually for CDC ACM:
// EP0 = control (phys 0/1)
// EP1 IN = interrupt notification (phys 3)
// EP2 OUT = bulk data from host (phys 2)
// EP3 IN = bulk data to host (phys 5)

const PHY_EP0_OUT: usize = 0;
const PHY_EP0_IN: usize  = 1;
const PHY_EP1_IN: usize  = 3;  // Interrupt IN (notification)
const PHY_EP2_OUT: usize = 2;  // Bulk OUT (host→device)
const PHY_EP3_IN: usize  = 5;  // Bulk IN (device→host)

// USB endpoint addresses
const USB_EP0: u8     = 0x00;
const USB_EP1_IN: u8  = 0x81;
const USB_EP2_OUT: u8 = 0x02;
const USB_EP3_IN: u8  = 0x83;

// ── USB Descriptors ────────────────────────────────────────────────

/// Device Descriptor (18 bytes)
const DEVICE_DESCRIPTOR: [u8; 18] = [
    0x12,       // bLength
    0x01,       // bDescriptorType (Device)
    0x00, 0x02, // bcdUSB (2.00)
    0xEF,       // bDeviceClass (Misc)
    0x02,       // bDeviceSubClass (Common)
    0x01,       // bDeviceProtocol (IAD)
    0x40,       // bMaxPacketSize0 (64)
    0x25, 0x05, // idVendor (0x0525 = Linux Foundation)
    0xA7, 0xA4, // idProduct (0xA4A7 = CDC ACM)
    0x01, 0x00, // bcdDevice
    0x01,       // iManufacturer
    0x02,       // iProduct
    0x03,       // iSerialNumber
    0x01,       // bNumConfigurations
];

/// Configuration + interface + endpoint descriptors (75 bytes total)
const CONFIG_DESCRIPTOR: [u8; 75] = [
    // Configuration Descriptor (9 bytes)
    0x09,       // bLength
    0x02,       // bDescriptorType (Config)
    0x4B, 0x00, // wTotalLength (75)
    0x02,       // bNumInterfaces
    0x01,       // bConfigurationValue
    0x00,       // iConfiguration
    0x80,       // bmAttributes (bus powered)
    0x32,       // bMaxPower (100mA)

    // Interface Association Descriptor (8 bytes)
    0x08,       // bLength
    0x0B,       // bDescriptorType (IAD)
    0x00,       // bFirstInterface
    0x02,       // bInterfaceCount
    0x02,       // bFunctionClass (CDC)
    0x02,       // bFunctionSubClass (ACM)
    0x01,       // bFunctionProtocol
    0x00,       // iFunction

    // Communication Interface Descriptor (9 bytes)
    0x09,       // bLength
    0x04,       // bDescriptorType (Interface)
    0x00,       // bInterfaceNumber
    0x00,       // bAlternateSetting
    0x01,       // bNumEndpoints (1: interrupt IN)
    0x02,       // bInterfaceClass (CDC)
    0x02,       // bInterfaceSubClass (ACM)
    0x01,       // bInterfaceProtocol (V.250)
    0x00,       // iInterface

    // CDC Header Functional Descriptor (5 bytes)
    0x05, 0x24, 0x00, 0x10, 0x01,
    // CDC ACM Functional Descriptor (4 bytes)
    0x04, 0x24, 0x02, 0x02,
    // CDC Union Functional Descriptor (5 bytes)
    0x05, 0x24, 0x06, 0x00, 0x01,
    // CDC Call Management Functional Descriptor (5 bytes)
    0x05, 0x24, 0x01, 0x00, 0x01,

    // Interrupt IN Endpoint Descriptor (7 bytes) — EP1 IN
    0x07,       // bLength
    0x05,       // bDescriptorType (Endpoint)
    USB_EP1_IN, // bEndpointAddress
    0x03,       // bmAttributes (Interrupt)
    0x40, 0x00, // wMaxPacketSize (64)
    0x0A,       // bInterval (10)

    // Data Interface Descriptor (9 bytes)
    0x09,       // bLength
    0x04,       // bDescriptorType (Interface)
    0x01,       // bInterfaceNumber
    0x00,       // bAlternateSetting
    0x02,       // bNumEndpoints (2: bulk IN + OUT)
    0x0A,       // bInterfaceClass (CDC Data)
    0x00,       // bInterfaceSubClass
    0x00,       // bInterfaceProtocol
    0x00,       // iInterface

    // Bulk OUT Endpoint Descriptor (7 bytes) — EP2 OUT
    0x07,       // bLength
    0x05,       // bDescriptorType (Endpoint)
    USB_EP2_OUT,// bEndpointAddress
    0x02,       // bmAttributes (Bulk)
    0x00, 0x02, // wMaxPacketSize (512)
    0x00,       // bInterval

    // Bulk IN Endpoint Descriptor (7 bytes) — EP3 IN
    0x07,       // bLength
    0x05,       // bDescriptorType (Endpoint)
    USB_EP3_IN, // bEndpointAddress
    0x02,       // bmAttributes (Bulk)
    0x00, 0x02, // wMaxPacketSize (512)
    0x00,       // bInterval
];

/// String descriptor 0 — Language ID
const STRING0: [u8; 4] = [0x04, 0x03, 0x09, 0x04]; // US English
/// String descriptor 1 — Manufacturer
const STRING1: &[u8] = b"aginx\0";
/// String descriptor 2 — Product
const STRING2: &[u8] = b"aginx Serial\0";
/// String descriptor 3 — Serial
const STRING3: &[u8] = b"0123456789\0";

// ── TRB (Transfer Request Block) ───────────────────────────────────
// Each TRB is 16 bytes (4 x u32)
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct Trb {
    bp: u32,    // buffer pointer low
    bp_hi: u32, // buffer pointer high (for 64-bit)
    len: u32,   // length + burst size
    ctrl: u32,  // control flags
}

const TRB_CTRL_HWO: u32    = 1 << 0;  // Hardware Owned
const TRB_CTRL_LST: u32    = 1 << 1;  // Last TRB
const TRB_CTRL_CHN: u32    = 1 << 2;  // Chain
const TRB_CTRL_CSP: u32    = 1 << 3;  // Continue on Short
const TRB_CTRL_ISP: u32    = 1 << 5;  // Interrupt on Short Packet
const TRB_CTRL_IOC: u32    = 1 << 6;  // Interrupt on Completion
const TRB_CTRL_TRBTYPE_NORMAL: u32 = 1 << 10; // type = 1 (Normal), bits [15:10]
const TRB_CTRL_TRBTYPE_CONTROL_SETUP: u32 = 2 << 10;
const TRB_CTRL_TRBTYPE_CONTROL_STATUS2: u32 = 3 << 10;
const TRB_CTRL_TRBTYPE_CONTROL_DATA: u32 = 4 << 10;

// ── Internal State ─────────────────────────────────────────────────

/// 4KB-aligned event buffer (DWC3 requires alignment = buffer size)
#[repr(C, align(4096))]
struct EventBuf([u8; 4096]);
static mut EVENT_BUF: EventBuf = EventBuf([0; 4096]);

/// Public access to event buffer physical address
pub static mut EVENT_BUF_ADDR: u32 = 0;

/// ABL's event buffer address — DWC3 DMA target we can't change.
/// Set during init from GEVNTADRLO read-back.
static mut ABL_EVBUF: u32 = 0;

/// TRB rings — 16 TRBs each
static mut EP0_OUT_TRBS: [Trb; 16] = [Trb { bp: 0, bp_hi: 0, len: 0, ctrl: 0 }; 16];
static mut EP2_OUT_TRBS: [Trb; 16] = [Trb { bp: 0, bp_hi: 0, len: 0, ctrl: 0 }; 16];
static mut EP3_IN_TRBS: [Trb; 16]  = [Trb { bp: 0, bp_hi: 0, len: 0, ctrl: 0 }; 16];

/// Control transfer buffer (for SETUP data and EP0 data)
static mut CTRL_BUF: [u8; 1024] = [0; 1024];

/// Bulk data buffers
static mut BULK_OUT_BUF: [u8; 512] = [0; 512];
static mut BULK_IN_BUF: [u8; 512] = [0; 512];

/// Current device address
static mut DEV_ADDR: u8 = 0;
/// Configuration value (0 = unconfigured, 1 = configured)
static mut CONFIGURED: bool = false;
/// TX buffer pending
static mut TX_PENDING: bool = false;
static mut TX_LEN: usize = 0;

// ── Register Access ────────────────────────────────────────────────

fn read32(off: usize) -> u32 {
    unsafe { core::ptr::read_volatile((DWC3_BASE + off) as *const u32) }
}

fn write32(off: usize, val: u32) {
    unsafe { core::ptr::write_volatile((DWC3_BASE + off) as *mut u32, val) }
}

/// Public read of any DWC3 register offset
pub fn read_reg(off: usize) -> u32 { read32(off) }

/// Public write to any DWC3 register offset
pub fn write_reg(off: usize, val: u32) { write32(off, val) }

/// Set up event buffer for DWC3 DMA — call before enabling events
pub fn setup_event_buf() -> bool {
    let evnt_addr = unsafe { EVENT_BUF.0.as_ptr() as u32 };
    if evnt_addr == 0 { return false; }
    unsafe { EVENT_BUF_ADDR = evnt_addr; }
    write32(GEVNTSIZ, 0);
    write32(GEVNTADRLO, evnt_addr);
    write32(GEVNTADRH, 0);
    write32(GEVNTSIZ, 4096);
    write32(GEVNTCOUNT, 0);
    true
}

/// Get raw pointer to event buffer (for cache invalidation)
pub fn get_event_buf_ptr() -> *mut u8 {
    unsafe { EVENT_BUF.0.as_mut_ptr() }
}

/// Read 32-bit word from event buffer at byte offset
pub fn read_event_buf(off: usize) -> u32 {
    unsafe { core::ptr::read_volatile(EVENT_BUF.0.as_ptr().add(off) as *const u32) }
}

/// Wait for DEPCMD to complete
fn wait_issue_dep_cmd(ep: usize) {
    for _ in 0..100_000 {
        if read32(dep_cmd_reg(ep)) & DEPCMD_CMDACT == 0 {
            return;
        }
    }
}

/// Issue endpoint command with 3 parameters: p0→PAR0, p1→PAR1, p2→PAR2
/// No CMDIOC — we poll CMDACT instead of using events
fn issue_dep_cmd(ep: usize, cmd: u32, p0: u32, p1: u32, p2: u32) {
    write32(dep_cmdpar0(ep), p0);
    write32(dep_cmdpar1(ep), p1);
    write32(dep_cmdpar2(ep), p2);
    write32(dep_cmd_reg(ep), cmd | DEPCMD_CMDACT);
    wait_issue_dep_cmd(ep);
}

// ── QUSB2 v2 USB HS PHY (SC7180 / Pixel 5 — from Linux phy-qcom-qusb2.c) ──
// Pixel 5 uses QUSB2 v2 PHY (compatible = "qcom,qusb2-v2-phy"), NOT SNPS Femto!
// Register base = 0x088E3000, span = 0x400 bytes

// SM7250 (Pixel 5) QUSB2 v2 PHY base — 0x088E_3000 (Linux DT, confirmed
// non-zero register reads after GCC clocks enabled; 0x088E_0000 = zeros).
static mut HSPHY_BASE: usize = 0x088E_3000;

// ── SMC-based PHY register access (through TrustZone EL3) ──
// PHY registers at 0x088E3000 are TZ-write-protected from EL1.
// Use Qualcomm SCM IO read/write SMC calls to access them through EL3.

/// SMC call for IO write: function ID 0xC2000502
fn scm_io_write32(addr: u64, val: u32) -> u64 {
    let ret: u64;
    unsafe {
        let fnid: u64 = 0xC2000502;
        core::arch::asm!(
            "mov x1, #2",
            "mov x6, xzr",
            "smc #0",
            "mov {ret}, x0",
            in("x0") fnid,
            in("x2") addr,
            in("x3") val as u64,
            ret = out(reg) ret,
            out("x1") _,
            out("x4") _,
            out("x5") _,
            out("x6") _,
        );
    }
    ret
}

/// SMC call for IO read: function ID 0xC2000501
fn scm_io_read32(addr: u64) -> (u64, u64) {
    let status: u64;
    let value: u64;
    unsafe {
        let fnid: u64 = 0xC2000501;
        core::arch::asm!(
            "mov x1, #1",
            "mov x6, xzr",
            "smc #0",
            "mov {status}, x0",
            "mov {value}, x1",
            in("x0") fnid,
            in("x2") addr,
            status = out(reg) status,
            value = out(reg) value,
            out("x3") _,
            out("x4") _,
            out("x5") _,
            out("x6") _,
        );
    }
    (status, value)
}

pub fn phy_read32(off: usize) -> u32 {
    unsafe { core::ptr::read_volatile((HSPHY_BASE + off) as *const u32) }
}

fn phy_write32(off: usize, val: u32) {
    unsafe {
        core::ptr::write_volatile((HSPHY_BASE + off) as *mut u32, val);
        core::ptr::read_volatile((HSPHY_BASE + off) as *const u32);
    }
}

pub fn phy_read8(off: usize) -> u8 {
    unsafe { core::ptr::read_volatile((HSPHY_BASE + off) as *const u8) }
}

pub fn phy_write8(off: usize, val: u8) {
    unsafe {
        core::ptr::write_volatile((HSPHY_BASE + off) as *mut u8, val);
        core::ptr::read_volatile((HSPHY_BASE + off) as *const u8);
    }
}

fn phy_smc_write32(off: usize, val: u32) -> u64 {
    let base = unsafe { HSPHY_BASE };
    scm_io_write32((base + off) as u64, val)
}

fn femto_spin() {
    for _ in 0..100_000 {
        core::hint::spin_loop();
    }
}

/// SNPS Femto HS PHY (DT: qcom,usb-hsphy-snps-femto @ 0x088E3000).
/// VBUS detect + leave SIDDQ. Uses SCM so EL1 TZ traps do not apply.
/// Returns (scm_status, COMMON0).
pub fn femto_hs_on() -> (u32, u32) {
    let (st0, c0) = phy_smc_read32(0x54);
    // CFG0: override
    let (_, cfg) = phy_smc_read32(0x94);
    let _ = phy_smc_write32(0x94, cfg | (1 << 1));
    // UTMI_CTRL5: POR=1
    let (_, u5) = phy_smc_read32(0x50);
    let _ = phy_smc_write32(0x50, u5 | (1 << 1));
    femto_spin();
    // COMMON1: VBUSVLDEXTSEL
    let (_, c1) = phy_smc_read32(0x58);
    let _ = phy_smc_write32(0x58, c1 | (1 << 4) | (1 << 5));
    // CTRL1: VBUSVLDEXT
    let (_, t1) = phy_smc_read32(0x60);
    let _ = phy_smc_write32(0x60, t1 | 1);
    // COMMON2: VREGBYPASS
    let (_, c2) = phy_smc_read32(0x5C);
    let _ = phy_smc_write32(0x5C, c2 | 1);
    // CTRL2: SUSPEND_N_SEL + SUSPEND_N
    let (_, t2) = phy_smc_read32(0x64);
    let _ = phy_smc_write32(0x64, t2 | (1 << 3) | (1 << 2));
    // UTMI_CTRL0: SLEEPM
    let (_, u0) = phy_smc_read32(0x3C);
    let _ = phy_smc_write32(0x3C, u0 | 1);
    // COMMON0: SIDDQ=0
    let (_, cmn) = phy_smc_read32(0x54);
    let _ = phy_smc_write32(0x54, cmn & !(1 << 2));
    // POR=0
    let (_, u5b) = phy_smc_read32(0x50);
    let _ = phy_smc_write32(0x50, u5b & !(1 << 1));
    femto_spin();
    // drop SUSPEND_N_SEL
    let (_, t2b) = phy_smc_read32(0x64);
    let _ = phy_smc_write32(0x64, t2b & !(1 << 3));
    let _ = phy_smc_write32(0x94, cfg & !(1 << 1));
    let (st1, c0b) = phy_smc_read32(0x54);
    let _ = (st0, c0, st1);
    (st1 as u32, c0b)
}

fn femto_or(off: usize, bits: u32) {
    phy_write32(off, phy_read32(off) | bits);
}

fn femto_andnot(off: usize, bits: u32) {
    phy_write32(off, phy_read32(off) & !bits);
}

pub fn peek32(pa: usize) -> u32 {
    unsafe { core::ptr::read_volatile(pa as *const u32) }
}

pub fn poke32(pa: usize, val: u32) {
    unsafe {
        core::ptr::write_volatile(pa as *mut u32, val);
        core::ptr::read_volatile(pa as *const u32);
    }
}

/// Same Femto sequence via direct MMIO (AHB2PHY must already be up).
/// SCM is denied (0xfffffffe) on this TZ. Returns COMMON0 after init.
pub fn femto_hs_on_mmio() -> u32 {
    let _before = phy_read32(0x54);
    femto_or(0x94, 1 << 1); // CFG0 override
    femto_or(0x50, 1 << 1); // POR
    femto_spin();
    femto_or(0x58, (1 << 4) | (1 << 5)); // VBUSVLDEXTSEL + PLLBTUNE
    femto_or(0x60, 1); // VBUSVLDEXT
    femto_or(0x5C, 1); // VREGBYPASS
    femto_or(0x64, (1 << 3) | (1 << 2)); // SUSPEND_N_SEL + SUSPEND_N
    femto_or(0x3C, 1); // SLEEPM
    femto_andnot(0x54, 1 << 2); // SIDDQ=0
    femto_andnot(0x50, 1 << 1); // POR=0
    femto_spin();
    femto_andnot(0x64, 1 << 3); // drop SUSPEND_N_SEL
    femto_andnot(0x94, 1 << 1);
    phy_read32(0x54)
}

/// Write PHY register through SMC (bypasses TZ write-protection)
fn phy_smc_write8(off: usize, val: u8) -> u64 {
    let base = unsafe { HSPHY_BASE };
    let addr = (base + off) as u64;
    // For simplicity, just write the byte value as a 32-bit word
    // QUSB2 PHY 8-bit registers are typically at byte-aligned offsets
    scm_io_write32(addr, val as u32)
}

/// Read PHY register through SMC
fn phy_smc_read32(off: usize) -> (u64, u32) {
    let base = unsafe { HSPHY_BASE };
    let addr = (base + off) as u64;
    let (status, val) = scm_io_read32(addr);
    (status, val as u32)
}

/// Read any 32-bit register via SMC IO at an arbitrary physical address.
/// Useful for diagnostic reads of DWC3 or PHY registers bypassing cache/MMU.
pub fn phy_smc_read32_at(addr: usize) -> (u64, u32) {
    let (status, val) = scm_io_read32(addr as u64);
    (status, val as u32)
}

/// Write any 32-bit register via SMC IO at an arbitrary physical address.
pub fn smc_write32_at(addr: usize, val: u32) -> u64 {
    scm_io_write32(addr as u64, val)
}

/// Write PHY register via SMC (public wrapper for TZ-protected SNPS Femto PHY)
pub fn smc_write_phy(off: usize, val: u8) -> u64 {
    phy_smc_write8(off, val)
}

/// Read PHY register via SMC (public wrapper)
pub fn smc_read_phy(off: usize) -> (u64, u32) {
    phy_smc_read32(off)
}

/// Test SMC IO write to PHY register
pub fn phy_smc_test() -> (u64, u32, u64, u32) {
    // (read_status, read_val, write_status, post_read_val)
    let (rs, rv) = phy_smc_read32(0x240); // PORT_TUNE1
    let ws = phy_smc_write8(0x240, 0xA5);
    let (rs2, rv2) = phy_smc_read32(0x240);
    // Restore
    phy_smc_write8(0x240, rv as u8);
    (rs, rv, ws, rv2)
}

/// Read PHY diagnostic registers: PLL_STATUS (0x1A0) and PORT_POWERDOWN (0x210)
pub fn phy_debug_regs() -> (u32, u32) {
    (phy_read32(0x1A0), phy_read32(0x210))
}

/// Minimal PHY init based on ABL's observed behavior.
/// ABL only writes offset 0x04 = 0x0E. Full Linux init table may not apply.
fn init_qusb2_v2_phy(con: &mut crate::fb::Console) -> bool {
    const GCC_BASE: usize = 0x0010_0000;
    const QUSB2PHY_PRIM_BCR: usize = GCC_BASE + 0x26000;

    // 1. Enable USB30 clocks (skip AHB2PHY BCR to preserve PHY accessibility)
    if !crate::gcc::enable_usb30_clocks_minimal() {
        con.puts("[11phy] clocks fail\r\n"); con.flush();
        return false;
    }
    con.puts("[11phy] clocks ok\r\n"); con.flush();

    // 2. Full QUSB2PHY BCR reset cycle to put PHY in known state
    unsafe { core::ptr::write_volatile(QUSB2PHY_PRIM_BCR as *mut u32, 1); }
    for _ in 0..100_000 { core::hint::spin_loop(); }
    unsafe { core::ptr::write_volatile(QUSB2PHY_PRIM_BCR as *mut u32, 0); }
    for _ in 0..100_000 { core::hint::spin_loop(); }
    let bcr = unsafe { core::ptr::read_volatile(QUSB2PHY_PRIM_BCR as *const u32) };
    con.puts("[11phy] bcr="); con.put_hex32(bcr);
    con.puts("\r\n"); con.flush();

    // 3. ABL-style minimal init: only TUNE1 (offset 0x04) = 0x0E
    phy_write8(0x04, 0x0E);
    let tune1 = phy_read8(0x04);
    con.puts("[11phy] tune1="); con.put_hex32(tune1 as u32);
    con.puts("\r\n"); con.flush();

    // 4. Disable PHY, write full qusb2 v2 init table, then enable
    phy_write8(0x210, 0x23);
    for _ in 0..50_000 { core::hint::spin_loop(); }

    // Full qusb2 v2 init sequence (Linux phy-qcom-qusb2.c)
    const INIT_TBL: &[(usize, u8)] = &[
        (0x04, 0x03),   // PLL_ANALOG_CONTROLS_TWO
        (0x2c, 0x80),   // PLL_CMODE
        (0xb4, 0x19),   // PLL_DIGITAL_TIMERS_TWO
        (0x184, 0x0a),  // PLL_LOCK_DELAY
        (0x18c, 0x7c),  // PLL_CLOCK_INVERTERS
        (0x194, 0x40),  // PLL_BIAS_CONTROL_1
        (0x198, 0x20),  // PLL_BIAS_CONTROL_2
        (0x214, 0x21),  // PWR_CTRL2
        (0x220, 0x00),  // IMP_CTRL1
        (0x224, 0x58),  // IMP_CTRL2
        (0x23c, 0x00),  // CHG_CTRL2
        (0x240, 0x30),  // PORT_TUNE1
        (0x244, 0x29),  // PORT_TUNE2
        (0x248, 0xca),  // PORT_TUNE3
        (0x24c, 0x04),  // PORT_TUNE4
        (0x250, 0x03),  // PORT_TUNE5
    ];
    for &(off, val) in INIT_TBL {
        phy_write8(off, val);
    }
    con.puts("[11phy] init tbl done\r\n"); con.flush();

    // Enable: clear only POWER_DOWN, keep CLAMP_N_EN | FREEZIO_N
    phy_write8(0x210, 0x22);
    for _ in 0..100_000 { core::hint::spin_loop(); }

    let pwr = phy_read8(0x210);
    con.puts("[11phy] pwr="); con.put_hex32(pwr as u32);
    con.puts("\r\n"); con.flush();

    // 5. Wait for PLL lock (bit 0 of PLL_STATUS at 0x1A0)
    let mut locked = false;
    for i in 0..500_000 {
        if phy_read8(0x1A0) & 1 != 0 {
            locked = true;
            break;
        }
        if i % 100_000 == 0 {
            con.puts("[11phy] waiting pll..\r\n"); con.flush();
        }
        core::hint::spin_loop();
    }

    let pll_status = phy_read32(0x1A0);
    con.puts("[11phy] pll="); con.put_hex32(pll_status);
    con.puts(" locked=");
    con.puts(if locked { "1" } else { "0" });
    con.puts("\r\n"); con.flush();

    locked
}

/// Read DWC3 wrapper HS_PHY_CTRL register (QSCRATCH+0x10)
pub fn read_hs_phy_ctrl() -> u32 {
    unsafe { core::ptr::read_volatile((QSCRATCH_BASE + 0x10) as *const u32) }
}

/// Set bits in DWC3 wrapper HS_PHY_CTRL register
pub fn set_hs_phy_ctrl(bits: u32) {
    let base = QSCRATCH_BASE + 0x10;
    unsafe {
        let val = core::ptr::read_volatile(base as *const u32);
        core::ptr::write_volatile(base as *mut u32, val | bits);
    }
}

/// Read multiple PHY registers for bus diagnostics (8-bit reads!)
pub fn phy_bus_diag() -> (u32, u32, u32, u32) {
    (
        phy_read8(0x1A0) as u32,  // PLL_STATUS
        phy_read8(0x210) as u32,  // PORT_POWERDOWN
        phy_read8(0x240) as u32,  // PORT_TUNE1
        phy_read8(0x244) as u32,  // PORT_TUNE2
    )
}

/// Write-read test using 8-bit access: write to a PHY register, read back.
pub fn phy_test_write_read() -> u32 {
    let saved = phy_read8(0x240); // PORT_TUNE1
    phy_write8(0x240, 0xA5); // write test pattern
    unsafe { core::arch::asm!("dsb sy"); }
    let val = phy_read8(0x240);
    phy_write8(0x240, saved); // restore
    val as u32
}

/// Read PHY register through DWC3 GUSB2PHYACC (bypasses direct MMIO)
/// Returns (ready, data) — ready=true if access completed
pub fn phyacc_read(reg_addr: u8) -> (bool, u16) {
    // NEWREGREQ=bit5, REGWR=bit6(write=1), REGRDY=bit12
    // REGADDR=bits[15:8], REGDATA=bits[31:16]
    let cmd: u32 = (1 << 5)       // NEWREGREQ
                 | (reg_addr as u32) << 8;  // REGADDR (read: REGWR=0)
    write32(GUSB2PHYACC0, cmd);
    for _ in 0..10_000 {
        let val = read32(GUSB2PHYACC0);
        if val & (1 << 12) != 0 { // REGRDY
            return (true, (val >> 16) as u16);
        }
        core::hint::spin_loop();
    }
    (false, 0)
}

/// Write PHY register through DWC3 GUSB2PHYACC (bypasses direct MMIO)
pub fn phyacc_write(reg_addr: u8, data: u8) -> bool {
    let cmd: u32 = (1 << 5)       // NEWREGREQ
                 | (1 << 6)       // REGWR=1 (write)
                 | (reg_addr as u32) << 8
                 | (data as u32) << 16;
    write32(GUSB2PHYACC0, cmd);
    for _ in 0..10_000 {
        let val = read32(GUSB2PHYACC0);
        if val & (1 << 12) != 0 { // REGRDY
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

/// Diagnostic: PHY pre-init register state
static mut PHY_PRE_DIAG: [u32; 4] = [0; 4];
/// Diagnostic: PHY post-init register state
static mut PHY_POST_DIAG: [u32; 4] = [0; 4];

pub fn get_phy_pre_diag() -> [u32; 4] { unsafe { PHY_PRE_DIAG } }
pub fn get_phy_post_diag() -> [u32; 4] { unsafe { PHY_POST_DIAG } }

/// Diagnostic: read key registers at BOTH candidate PHY addresses.
/// No writes — purely diagnostic to avoid crashes.
fn diag_both_phy_addrs(con: &mut crate::fb::Console) {
    let candidates: [(usize, &str); 2] = [
        (0x088E_3000, "3k"),  // Linux DT address
        (0x088E_0000, "0k"),  // fallback
    ];
    for &(addr, label) in &candidates {
        // Read-only: PLL_STATUS, PORT_POWERDOWN, TUNE1
        let pll = unsafe { core::ptr::read_volatile((addr + 0x1A0) as *const u32) };
        let pwr = unsafe { core::ptr::read_volatile((addr + 0x210) as *const u32) };
        let t1  = unsafe { core::ptr::read_volatile((addr + 0x240) as *const u32) };
        con.puts("[phy] "); con.puts(label);
        con.puts(" pll=0x"); con.put_hex32(pll);
        con.puts(" pwr=0x"); con.put_hex32(pwr);
        con.puts(" t1=0x"); con.put_hex32(t1);
        con.puts("\r\n"); con.flush();
    }
}

/// Initialize QUSB2 v2 USB HS PHY for SM7250 (Pixel 5)
/// Uses 8-bit writes matching Linux qusb2_phy_init driver.
pub fn init_hsphy(con: &mut crate::fb::Console) -> bool {
    // Diagnostic: read both candidate PHY addresses (read-only, no crash risk)
    diag_both_phy_addrs(con);

    // Read pre-init state via the current HSPHY_BASE (8-bit reads)
    let pll_before = phy_read8(0x1A0);
    let pwr_before = phy_read8(0x210);
    let tune1_before = phy_read8(0x04);

    unsafe {
        PHY_PRE_DIAG[0] = pll_before as u32;
        PHY_PRE_DIAG[1] = pwr_before as u32;
        PHY_PRE_DIAG[2] = tune1_before as u32;
        PHY_PRE_DIAG[3] = phy_read8(0xA8) as u32;
    }

    // If PLL already locked, use as-is
    if pll_before & 0x01 != 0 {
        con.puts("[phy] PLL already locked!\r\n"); con.flush();
        unsafe {
            PHY_POST_DIAG[0] = pll_before as u32;
            PHY_POST_DIAG[1] = pwr_before as u32;
            PHY_POST_DIAG[2] = 0;
            PHY_POST_DIAG[3] = 0;
        }
        return true;
    }

    con.puts("[phy] PLL=0, init..\r\n"); con.flush();

    // QUSB2 v2 PHY init using 8-bit writes (matches Linux qusb2_phy_init)
    // Step 0: QUSB2PHY BCR reset to put PHY in known state
    const GCC_BASE: usize = 0x0010_0000;
    const QUSB2PHY_PRIM_BCR: usize = GCC_BASE + 0x26000;
    unsafe { core::ptr::write_volatile(QUSB2PHY_PRIM_BCR as *mut u32, 1); }
    for _ in 0..100_000 { core::hint::spin_loop(); }
    unsafe { core::ptr::write_volatile(QUSB2PHY_PRIM_BCR as *mut u32, 0); }
    for _ in 0..100_000 { core::hint::spin_loop(); }
    con.puts("[phy] bcr ok\r\n"); con.flush();

    // Step 1: Power down PHY during configuration
    phy_write8(0x210, 0x23);
    for _ in 0..50_000 { core::hint::spin_loop(); }

    // Step 2: Write init table with 8-bit accesses
    phy_write8(0x04,  0x03);  // PLL_ANALOG_CONTROLS_TWO
    phy_write8(0x18C, 0x7C);  // PLL_CLOCK_INVERTERS
    phy_write8(0x2C,  0x80);  // PLL_CMODE
    phy_write8(0x184, 0x0A);  // PLL_LOCK_DELAY
    phy_write8(0xB4,  0x19);  // PLL_DIGITAL_TIMERS_TWO
    phy_write8(0x194, 0x40);  // PLL_BIAS_CONTROL_1
    phy_write8(0x198, 0x20);  // PLL_BIAS_CONTROL_2
    phy_write8(0x214, 0x21);  // PWR_CTRL2
    phy_write8(0x220, 0x00);  // IMP_CTRL1
    phy_write8(0x224, 0x58);  // IMP_CTRL2
    phy_write8(0x240, 0x30);  // PORT_TUNE1
    phy_write8(0x244, 0x29);  // PORT_TUNE2
    phy_write8(0x248, 0xCA);  // PORT_TUNE3
    phy_write8(0x24C, 0x04);  // PORT_TUNE4
    phy_write8(0x250, 0x03);  // PORT_TUNE5
    phy_write8(0x23C, 0x00);  // CHG_CTRL2

    // Step 3: Release PLL override
    phy_write8(0xA8, 0x00);  // PLL_CORE_INPUT_OVERRIDE = 0

    // Step 4: Power up PHY (clear POWER_DOWN, keep CLAMP_N_EN + FREEZIO_N)
    phy_write8(0x210, 0x22);

    // Step 5: Wait for PLL lock (bit 0 = CORE_READY_STATUS)
    con.puts("[phy] wait PLL..\r\n"); con.flush();
    let mut pll_ready = false;
    for i in 0..5_000_000 {
        if phy_read8(0x1A0) & 0x01 != 0 {
            pll_ready = true;
            con.puts("[phy] PLL lock!\r\n"); con.flush();
            break;
        }
        if i > 0 && i % 1_000_000 == 0 {
            con.puts("[phy] still wait..\r\n"); con.flush();
        }
        core::hint::spin_loop();
    }

    unsafe {
        PHY_POST_DIAG[0] = phy_read8(0x1A0) as u32;
        PHY_POST_DIAG[1] = phy_read8(0x210) as u32;
        PHY_POST_DIAG[2] = 1; // did init
        PHY_POST_DIAG[3] = if pll_ready { 1 } else { 0 };
    }

    con.puts("[phy] post: pll=0x"); con.put_hex32(unsafe { PHY_POST_DIAG[0] });
    con.puts(" ready="); con.puts(if pll_ready { "1" } else { "0" });
    con.puts("\r\n"); con.flush();

    pll_ready
}

/// SMC IO-aware PHY init: uses TrustZone SMC calls for PHY register writes
/// to bypass EL1 TZ write-protection. Falls back to direct MMIO if SMC fails.
pub fn init_hsphy_smc(con: &mut crate::fb::Console) -> bool {
    let base = unsafe { HSPHY_BASE };

    // Check if SMC IO is available
    let smc_ok = smc_is_call_avail(0x02000502) == 0; // 0 = success/available

    // Read pre-init state
    let pll_before = if smc_ok {
        let (_, val) = phy_smc_read32(0x1A0);
        val as u8
    } else {
        phy_read8(0x1A0)
    };

    con.puts("[smc_phy] smc="); con.puts(if smc_ok { "ok" } else { "no" });
    con.puts(" pll="); con.put_hex32(pll_before as u32);
    con.puts("\r\n"); con.flush();

    if pll_before & 1 != 0 {
        con.puts("[smc_phy] PLL already locked\r\n"); con.flush();
        return true;
    }

    // Helper: write PHY register, using SMC if available
    let wr = |off: usize, val: u8| {
        if smc_ok {
            phy_smc_write8(off, val);
        } else {
            phy_write8(off, val);
        }
    };

    // QUSB2PHY BCR reset
    const GCC_BASE: usize = 0x0010_0000;
    const QUSB2PHY_PRIM_BCR: usize = GCC_BASE + 0x26000;
    unsafe { core::ptr::write_volatile(QUSB2PHY_PRIM_BCR as *mut u32, 1); }
    for _ in 0..100_000 { core::hint::spin_loop(); }
    unsafe { core::ptr::write_volatile(QUSB2PHY_PRIM_BCR as *mut u32, 0); }
    for _ in 0..100_000 { core::hint::spin_loop(); }
    con.puts("[smc_phy] bcr ok\r\n"); con.flush();

    // Power down PHY during configuration
    wr(0x210, 0x23);
    for _ in 0..50_000 { core::hint::spin_loop(); }

    // Write init table (qusb2 v2 sequence)
    wr(0x04,  0x03);  // PLL_ANALOG_CONTROLS_TWO
    wr(0x18C, 0x7C);  // PLL_CLOCK_INVERTERS
    wr(0x2C,  0x80);  // PLL_CMODE
    wr(0x184, 0x0A);  // PLL_LOCK_DELAY
    wr(0xB4,  0x19);  // PLL_DIGITAL_TIMERS_TWO
    wr(0x194, 0x40);  // PLL_BIAS_CONTROL_1
    wr(0x198, 0x20);  // PLL_BIAS_CONTROL_2
    wr(0x214, 0x21);  // PWR_CTRL2
    wr(0x220, 0x00);  // IMP_CTRL1
    wr(0x224, 0x58);  // IMP_CTRL2
    wr(0x240, 0x30);  // PORT_TUNE1
    wr(0x244, 0x29);  // PORT_TUNE2
    wr(0x248, 0xCA);  // PORT_TUNE3
    wr(0x24C, 0x04);  // PORT_TUNE4
    wr(0x250, 0x03);  // PORT_TUNE5
    wr(0x23C, 0x00);  // CHG_CTRL2
    con.puts("[smc_phy] tbl done\r\n"); con.flush();

    // Release PLL override
    wr(0xA8, 0x00);

    // Power up PHY
    wr(0x210, 0x22);

    // Read back key registers via SMC to verify writes took effect
    if smc_ok {
        let (_, pwr) = phy_smc_read32(0x210);
        let (_, tune1) = phy_smc_read32(0x240);
        con.puts("[smc_phy] rdback pwr="); con.put_hex32(pwr);
        con.puts(" t1="); con.put_hex32(tune1);
        con.puts("\r\n"); con.flush();
    }

    // Wait for PLL lock
    con.puts("[smc_phy] wait PLL..\r\n"); con.flush();
    let mut locked = false;
    for i in 0..5_000_000 {
        let pll = if smc_ok {
            let (_, val) = phy_smc_read32(0x1A0);
            val as u8
        } else {
            phy_read8(0x1A0)
        };
        if pll & 0x01 != 0 {
            locked = true;
            con.puts("[smc_phy] PLL LOCKED!\r\n"); con.flush();
            break;
        }
        if i > 0 && i % 1_000_000 == 0 {
            con.puts("[smc_phy] still wait..\r\n"); con.flush();
        }
        core::hint::spin_loop();
    }

    if !locked {
        let pll_val = if smc_ok { let (_, v) = phy_smc_read32(0x1A0); v } else { phy_read32(0x1A0) };
        con.puts("[smc_phy] PLL FAIL="); con.put_hex32(pll_val);
        con.puts("\r\n"); con.flush();
    }

    locked
}

// ── QMP USB SS PHY (0x088E9000) ──
// QMP (Qualcomm Multi-Protocol) PHY for SuperSpeed USB
// Minimal init based on Linux phy-qcom-qmp-usb.c for SM7250

const QMP_PHY_BASE: usize = 0x088E_9000;

fn qmp_read(off: usize) -> u32 {
    unsafe { core::ptr::read_volatile((QMP_PHY_BASE + off) as *const u32) }
}

fn qmp_write(off: usize, val: u32) {
    unsafe {
        core::ptr::write_volatile((QMP_PHY_BASE + off) as *mut u32, val);
        core::ptr::read_volatile((QMP_PHY_BASE + off) as *const u32);
    }
}

fn qmp_write8(off: usize, val: u8) {
    unsafe {
        core::ptr::write_volatile((QMP_PHY_BASE + off) as *mut u8, val);
        core::ptr::read_volatile((QMP_PHY_BASE + off) as *const u8);
    }
}

fn init_qmp_phy(con: &mut crate::fb::Console) -> bool {
    const GCC_BASE: usize = 0x0010_0000;
    const USB30_PRIM_SS_PHY_BCR: usize = GCC_BASE + 0x0F000;
    // Actually the SSPHY BCR is at a different offset for SM7250
    // GCC_USB30_PRIM_SS_PHY_BCR = GCC + 0x0F00 (reset for SS PHY)

    // Read initial QMP PHY state
    let serdes = qmp_read(0x000);
    let status1 = qmp_read(0x100);
    let status2 = qmp_read(0x1C0);
    con.puts("[qmp] serdes="); con.put_hex32(serdes);
    con.puts(" s1="); con.put_hex32(status1);
    con.puts(" s2="); con.put_hex32(status2);
    con.puts("\r\n"); con.flush();

    // If QMP PHY is already active (from ABL), skip init
    if status1 != 0 || status2 != 0 {
        con.puts("[qmp] already active?\r\n"); con.flush();
        return true;
    }

    // GCC SS PHY BCR reset
    unsafe {
        core::ptr::write_volatile(USB30_PRIM_SS_PHY_BCR as *mut u32, 1);
    }
    for _ in 0..100_000 { core::hint::spin_loop(); }
    unsafe {
        core::ptr::write_volatile(USB30_PRIM_SS_PHY_BCR as *mut u32, 0);
    }
    for _ in 0..100_000 { core::hint::spin_loop(); }
    con.puts("[qmp] bcr done\r\n"); con.flush();

    // Read post-reset state
    let serdes2 = qmp_read(0x000);
    con.puts("[qmp] post-bcr serdes="); con.put_hex32(serdes2);
    con.puts("\r\n"); con.flush();

    // Try to read/write QMP PHY registers to verify access
    // Write a test pattern to a safe register
    qmp_write8(0x08, 0x01);
    let val = qmp_read(0x08);
    con.puts("[qmp] test w/r="); con.put_hex32(val);
    con.puts("\r\n"); con.flush();

    // Minimal QMP PHY power-on sequence
    // 1. Deassert PHY reset (power control register)
    //    PCFG (offset 0x00): clear bit 0 = release reset
    qmp_write(0x000, qmp_read(0x000) & !1);
    for _ in 0..50_000 { core::hint::spin_loop(); }

    // 2. Enable PIPE clock (PCFG + 0x04)
    qmp_write(0x004, qmp_read(0x004) | (1 << 4)); // PIPE_CLK_EN
    for _ in 0..50_000 { core::hint::spin_loop(); }

    // 3. Check if PHY comes alive
    let pcfg = qmp_read(0x000);
    let pstat = qmp_read(0x100);
    con.puts("[qmp] pcfg="); con.put_hex32(pcfg);
    con.puts(" pstat="); con.put_hex32(pstat);
    con.puts("\r\n"); con.flush();

    // For now, return true if registers are accessible
    // Full QMP init needs the complete register table from Linux
    pcfg != 0 || pstat != 0 || val != 0
}

fn full_phy_init() -> bool {
    const GCC_BASE: usize = 0x0010_0000;
    const QUSB2PHY_BCR: usize = 0x26000;

    // BCR reset with proper timing
    unsafe { core::ptr::write_volatile((GCC_BASE + QUSB2PHY_BCR) as *mut u32, 1); }
    for _ in 0..30_000 { core::hint::spin_loop(); }
    unsafe { core::ptr::write_volatile((GCC_BASE + QUSB2PHY_BCR) as *mut u32, 0); }
    for _ in 0..30_000 { core::hint::spin_loop(); }

    // Hold PHY in powered-down state
    phy_write32(0x210, phy_read32(0x210) | 0x23);

    // Write init table (qusb2_v2_init_tbl)
    phy_write32(0x04,  0x03);
    phy_write32(0x18C, 0x7C);
    phy_write32(0x2C,  0x80);
    phy_write32(0x184, 0x0A);
    phy_write32(0xB4,  0x19);
    phy_write32(0x194, 0x40);
    phy_write32(0x198, 0x20);
    phy_write32(0x214, 0x21);
    phy_write32(0x220, 0x00);
    phy_write32(0x224, 0x58);
    phy_write32(0x240, 0x30);
    phy_write32(0x244, 0x29);
    phy_write32(0x248, 0xCA);
    phy_write32(0x24C, 0x04);
    phy_write32(0x250, 0x03);
    phy_write32(0x23C, 0x00);

    // Release power-down
    phy_write32(0x210, phy_read32(0x210) & !0x01);

    // Wait for PLL
    for _ in 0..300_000 { core::hint::spin_loop(); }

    let mut ready = false;
    for _ in 0..100_000 {
        if phy_read32(0x1A0) & 0x01 != 0 {
            ready = true;
            break;
        }
        core::hint::spin_loop();
    }

    unsafe {
        PHY_POST_DIAG[0] = phy_read32(0x1A0);
        PHY_POST_DIAG[1] = phy_read32(0x210);
        PHY_POST_DIAG[2] = 1; // flag: did full init
        PHY_POST_DIAG[3] = if ready { 1 } else { 0 };
    }

    ready
}

// ── Platform Init ──────────────────────────────────────────────────

/// Initialize Qualcomm platform infrastructure (clocks, PHY) before DWC3 core
fn init_platform(con: &mut crate::fb::Console) -> bool {
    // 1. GCC USB30 clocks (minimal: no AHB2PHY BCR reset, preserves PHY access)
    con.puts("[11a] GCC clks..\r\n"); con.flush();
    if !crate::gcc::enable_usb30_clocks_minimal() {
        con.puts("[11a] GCC FAIL\r\n"); con.flush();
        return false;
    }
    con.puts("[11a] GCC ok\r\n"); con.flush();
    // 2. QUSB2 PHY (probes correct address, does full init)
    con.puts("[11b] PHY..\r\n"); con.flush();
    if !init_hsphy(con) {
        con.puts("[11b] PHY FAIL\r\n"); con.flush();
        return false;
    }
    con.puts("[11b] PHY ok\r\n"); con.flush();
    true
}

// ── Cold init v2: full DWC3 init with correct register sequence ──

/// Full DWC3 initialization from scratch following the Linux dwc3_core_init sequence.
/// Call AFTER gcc::enable_usb30_clocks_debug() which sets up clocks and PHY BCR reset.
///
/// Sequence (matches Linux dwc3_core_init + dwc3_gadget_start):
/// 1. Clear SUSPHY (prevent PHY suspend from blocking commands)
/// 2. Core soft reset (GCTL.CORESOFTRESET + PHYSOFTRST)
/// 3. Set device mode (GCTL.PRTCAPDIR)
/// 4. Setup event buffer
/// 5. Set DCFG for HS speed
/// 6. Enable device events (DEVTEN)
/// 7. Configure EP0 (DEPCMD SETEPCONFIG + STARTCFG)
/// 8. Set RUN_STOP to connect
///
/// Warm init: take over DWC3 from ABL without resetting core or PHY.
/// ABL configured USB for fastboot — PHY registers are write-protected from EL1,
/// so we must preserve ABL's PHY configuration.
pub fn init_abl_takeover(con: &mut crate::fb::Console) -> bool {
    // Verify DWC3 is alive
    let snpsid = read32(GSNPSID);
    if snpsid == 0 || snpsid == 0xFFFF_FFFF {
        con.puts("[warm] no DWC3\r\n"); con.flush();
        return false;
    }
    con.puts("[warm] snps="); con.put_hex32(snpsid);
    con.puts("\r\n"); con.flush();

    // Read current state (ABL left DWC3 configured for fastboot)
    let dctl0 = read32(DCTL);
    let gctl0 = read32(GCTL);
    let usb2cfg = read32(GUSB2PHYCFG);
    let dsts0 = read32(DSTS);
    con.puts("[warm] dctl="); con.put_hex32(dctl0);
    con.puts(" gctl="); con.put_hex32(gctl0);
    con.puts(" usb2="); con.put_hex32(usb2cfg);
    con.puts(" dsts="); con.put_hex32(dsts0);
    con.puts("\r\n"); con.flush();

    // Disconnect: clear RUN_STOP preserving all other DCTL bits
    write32(DCTL, read32(DCTL) & !DCTL_RUN_STOP);
    for _ in 0..5_000_000 { core::hint::spin_loop(); } // 50ms disconnect

    // Clear SUSPHY
    write32(GUSB2PHYCFG, read32(GUSB2PHYCFG) & !(1u32 << 6));

    // Set device mode
    let gctl = read32(GCTL);
    write32(GCTL, (gctl & !GCTL_PRTCAPDIR_MASK) | GCTL_PRTCAP_DEVICE);

    // QSCRATCH VBUS override
    unsafe {
        let hs = core::ptr::read_volatile((QSCRATCH_BASE + 0x10) as *const u32);
        core::ptr::write_volatile((QSCRATCH_BASE + 0x10) as *mut u32,
            hs | (1u32 << 5) | (1u32 << 6) | (1u32 << 20) | (1u32 << 28));
        core::ptr::read_volatile((QSCRATCH_BASE + 0x10) as *const u32);
    }

    // Set up event buffer
    let evnt_addr = unsafe { EVENT_BUF.0.as_ptr() as u32 };
    write32(GEVNTSIZ, 0);
    write32(GEVNTADRLO, evnt_addr);
    write32(GEVNTADRH, 0);
    write32(GEVNTSIZ, 4096);
    write32(GEVNTCOUNT, 0);

    // Set DCFG for High Speed
    write32(DCFG, DCFG_HIGHSPEED);

    // Enable device events
    write32(DEVTEN, DEVTEN_DISCONN | DEVTEN_USBRST | DEVTEN_CONNECTDONE | DEVTEN_ULSTCHNG);

    // Configure EP0
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSETEPCONFIG, 64u32 << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSETEPCONFIG, 64u32 << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSTARTCFG, 0, 0, 0);

    write32(DALEPENA, (1 << PHY_EP0_OUT) | (1 << PHY_EP0_IN));
    queue_ep0_out_trb();

    // Reconnect: set RUN_STOP preserving all other DCTL bits
    write32(DCTL, read32(DCTL) | DCTL_RUN_STOP);
    for _ in 0..10_000_000 { core::hint::spin_loop(); } // 100ms for host detection

    let dctl_f = read32(DCTL);
    let dsts_f = read32(DSTS);
    let evc = read32(GEVNTCOUNT);
    con.puts("[warm] dctl="); con.put_hex32(dctl_f);
    con.puts(" dsts="); con.put_hex32(dsts_f);
    con.puts(" evc="); con.put_hex32(evc);
    con.puts("\r\n"); con.flush();

    true
}

/// Warm init with Full-Speed mode (12 Mbps). FS uses simple D+/D- signaling
/// that may work even if PHY HS PLL isn't locked.
pub fn init_abl_takeover_fs(con: &mut crate::fb::Console) -> bool {
    let snpsid = read32(GSNPSID);
    if snpsid == 0 || snpsid == 0xFFFF_FFFF {
        con.puts("[wfs] no DWC3\r\n"); con.flush();
        return false;
    }

    // Disconnect
    write32(DCTL, read32(DCTL) & !DCTL_RUN_STOP);
    for _ in 0..5_000_000 { core::hint::spin_loop(); }

    // Clear SUSPHY
    write32(GUSB2PHYCFG, read32(GUSB2PHYCFG) & !(1u32 << 6));

    // Set device mode
    let gctl = read32(GCTL);
    write32(GCTL, (gctl & !GCTL_PRTCAPDIR_MASK) | GCTL_PRTCAP_DEVICE);

    // QSCRATCH VBUS override
    unsafe {
        let hs = core::ptr::read_volatile((QSCRATCH_BASE + 0x10) as *const u32);
        core::ptr::write_volatile((QSCRATCH_BASE + 0x10) as *mut u32,
            hs | (1u32 << 5) | (1u32 << 6) | (1u32 << 20) | (1u32 << 28));
        core::ptr::read_volatile((QSCRATCH_BASE + 0x10) as *const u32);
    }

    // Event buffer
    let evnt_addr = unsafe { EVENT_BUF.0.as_ptr() as u32 };
    write32(GEVNTSIZ, 0);
    write32(GEVNTADRLO, evnt_addr);
    write32(GEVNTADRH, 0);
    write32(GEVNTSIZ, 4096);
    write32(GEVNTCOUNT, 0);

    // KEY: Set Full-Speed instead of High-Speed
    write32(DCFG, DCFG_FULLSPEED);

    // Enable device events
    write32(DEVTEN, DEVTEN_DISCONN | DEVTEN_USBRST | DEVTEN_CONNECTDONE | DEVTEN_ULSTCHNG);

    // Configure EP0
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSETEPCONFIG, 64u32 << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSETEPCONFIG, 64u32 << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSTARTCFG, 0, 0, 0);
    write32(DALEPENA, (1 << PHY_EP0_OUT) | (1 << PHY_EP0_IN));
    queue_ep0_out_trb();

    // Connect
    write32(DCTL, read32(DCTL) | DCTL_RUN_STOP);
    for _ in 0..10_000_000 { core::hint::spin_loop(); }

    let dctl_f = read32(DCTL);
    let dsts_f = read32(DSTS);
    let evc = read32(GEVNTCOUNT);
    con.puts("[wfs] dctl="); con.put_hex32(dctl_f);
    con.puts(" dsts="); con.put_hex32(dsts_f);
    con.puts(" evc="); con.put_hex32(evc);
    con.puts("\r\n"); con.flush();

    true
}

/// Warm init using SMC IO for all DWC3 register writes.
/// Does NOT reset the DWC3 core — takes over from ABL's configuration.
/// Uses TrustZone SMC calls to bypass potential cache/MMU issues with direct MMIO.
pub fn init_warm_smc(con: &mut crate::fb::Console) -> bool {
    // Verify DWC3 is alive via SMC IO
    let (st, snpsid) = phy_smc_read32_at(DWC3_BASE + GSNPSID);
    con.puts("[wsmc] snps="); con.put_hex32(snpsid);
    con.puts(" st="); con.put_hex32(st as u32);
    con.puts("\r\n"); con.flush();
    if snpsid == 0 || snpsid == 0xFFFF_FFFF {
        con.puts("[wsmc] no DWC3\r\n"); con.flush();
        return false;
    }

    // Read current DWC3 state via SMC
    let (_, dctl0) = phy_smc_read32_at(DWC3_BASE + DCTL);
    let (_, gctl0) = phy_smc_read32_at(DWC3_BASE + GCTL);
    let (_, usb2cfg) = phy_smc_read32_at(DWC3_BASE + GUSB2PHYCFG);
    let (_, dsts0) = phy_smc_read32_at(DWC3_BASE + DSTS);
    con.puts("[wsmc] dctl="); con.put_hex32(dctl0);
    con.puts(" gctl="); con.put_hex32(gctl0);
    con.puts(" usb2="); con.put_hex32(usb2cfg);
    con.puts(" dsts="); con.put_hex32(dsts0);
    con.puts("\r\n"); con.flush();

    // Check if RUN_STOP is already set (ABL left it connected)
    let was_connected = dctl0 & DCTL_RUN_STOP != 0;
    con.puts("[wsmc] connected="); con.puts(if was_connected { "1" } else { "0" });
    con.puts("\r\n"); con.flush();

    // Disconnect: clear RUN_STOP via SMC
    if was_connected {
        let new_dctl = dctl0 & !DCTL_RUN_STOP;
        smc_write32_at(DWC3_BASE + DCTL, new_dctl);
        for _ in 0..5_000_000 { core::hint::spin_loop(); } // 50ms disconnect
    }

    // Clear SUSPHY via SMC
    let usb2_new = usb2cfg & !(1u32 << 6);
    smc_write32_at(DWC3_BASE + GUSB2PHYCFG, usb2_new);

    // Set device mode via SMC
    let gctl_new = (gctl0 & !GCTL_PRTCAPDIR_MASK) | GCTL_PRTCAP_DEVICE;
    smc_write32_at(DWC3_BASE + GCTL, gctl_new);

    // QSCRATCH VBUS override (direct MMIO — this isn't TZ-protected)
    unsafe {
        let hs = core::ptr::read_volatile((QSCRATCH_BASE + 0x10) as *const u32);
        core::ptr::write_volatile((QSCRATCH_BASE + 0x10) as *mut u32,
            hs | (1u32 << 5) | (1u32 << 6) | (1u32 << 20) | (1u32 << 28));
        core::ptr::read_volatile((QSCRATCH_BASE + 0x10) as *const u32);
    }

    // Event buffer (direct MMIO — static buffer in our BSS)
    let evnt_addr = unsafe { EVENT_BUF.0.as_ptr() as u32 };
    write32(GEVNTSIZ, 0);
    write32(GEVNTADRLO, evnt_addr);
    write32(GEVNTADRH, 0);
    write32(GEVNTSIZ, 4096);
    write32(GEVNTCOUNT, 0);
    unsafe { ABL_EVBUF = evnt_addr; }

    // DCFG: High Speed via SMC
    smc_write32_at(DWC3_BASE + DCFG, DCFG_HIGHSPEED);

    // Enable device events via SMC
    smc_write32_at(DWC3_BASE + DEVTEN, DEVTEN_DISCONN | DEVTEN_USBRST | DEVTEN_CONNECTDONE | DEVTEN_ULSTCHNG);

    // Configure EP0 (direct MMIO — DEPCMD registers should work)
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSETEPCONFIG, 64u32 << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSETEPCONFIG, 64u32 << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSTARTCFG, 0, 0, 0);
    write32(DALEPENA, (1 << PHY_EP0_OUT) | (1 << PHY_EP0_IN));
    queue_ep0_out_trb();

    // Verify EP0 was configured: read back DALEPENA
    let (_, dalepena) = phy_smc_read32_at(DWC3_BASE + DALEPENA);
    con.puts("[wsmc] dalepena="); con.put_hex32(dalepena);
    con.puts("\r\n"); con.flush();

    // Reconnect: set RUN_STOP via SMC
    let (_, dctl_now) = phy_smc_read32_at(DWC3_BASE + DCTL);
    smc_write32_at(DWC3_BASE + DCTL, dctl_now | DCTL_RUN_STOP);
    for _ in 0..10_000_000 { core::hint::spin_loop(); } // 100ms for host detection

    // Final diagnostics via SMC
    let (_, dctl_f) = phy_smc_read32_at(DWC3_BASE + DCTL);
    let (_, dsts_f) = phy_smc_read32_at(DWC3_BASE + DSTS);
    let evc = read32(GEVNTCOUNT);
    con.puts("[wsmc] dctl="); con.put_hex32(dctl_f);
    con.puts(" dsts="); con.put_hex32(dsts_f);
    con.puts(" evc="); con.put_hex32(evc);
    con.puts("\r\n"); con.flush();

    unsafe {
        DEV_ADDR = 0;
        CONFIGURED = false;
        TX_PENDING = false;
    }

    true
}

pub fn init_cold_v2(con: &mut crate::fb::Console) -> bool {
    // Verify DWC3 is alive
    let snpsid = read32(GSNPSID);
    if snpsid == 0 || snpsid == 0xFFFF_FFFF {
        con.puts("[v2] no DWC3\r\n"); con.flush();
        return false;
    }
    con.puts("[v2] snps="); con.put_hex32(snpsid);
    con.puts("\r\n"); con.flush();

    // Read initial state
    let dctl0 = read32(DCTL);
    let gctl0 = read32(GCTL);
    let usb2cfg = read32(GUSB2PHYCFG);
    con.puts("[v2] dctl="); con.put_hex32(dctl0);
    con.puts(" gctl="); con.put_hex32(gctl0);
    con.puts(" usb2="); con.put_hex32(usb2cfg);
    con.puts("\r\n"); con.flush();

    // Step 1: Clear SUSPHY to prevent PHY suspend from blocking commands/events
    // Linux: "We must clear SUSPHY to ensure connection events are generated"
    let usb2cfg = read32(GUSB2PHYCFG);
    if usb2cfg & (1 << 6) != 0 {
        write32(GUSB2PHYCFG, usb2cfg & !(1u32 << 6));
        con.puts("[v2] SUSPHY cleared\r\n"); con.flush();
    }

    // Step 2: Core soft reset (Linux dwc3_core_soft_reset sequence)
    // 2a: Assert USB2 PHY soft reset
    write32(GUSB2PHYCFG, read32(GUSB2PHYCFG) | (1u32 << 0)); // PHYSOFTRST

    // 2b: Assert core soft reset
    write32(GCTL, read32(GCTL) | GCTL_CORESOFTRESET);
    for _ in 0..100_000 { core::hint::spin_loop(); }

    // 2c: De-assert core soft reset (must be done manually per DWC3 spec)
    write32(GCTL, read32(GCTL) & !GCTL_CORESOFTRESET);
    for _ in 0..100_000 { core::hint::spin_loop(); }

    // 2d: De-assert USB2 PHY soft reset
    write32(GUSB2PHYCFG, read32(GUSB2PHYCFG) & !(1u32 << 0)); // clear PHYSOFTRST
    for _ in 0..2_000_000 { core::hint::spin_loop(); } // 1ms for PHY settle

    // 2e: Re-clear SUSPHY after core reset (GUSB2PHYCFG is reset by core reset)
    let usb2cfg2 = read32(GUSB2PHYCFG);
    write32(GUSB2PHYCFG, usb2cfg2 & !(1u32 << 6)); // clear SUSPHY
    con.puts("[v2] usb2_post="); con.put_hex32(usb2cfg2);
    con.puts("\r\n"); con.flush();

    let gctl1 = read32(GCTL);
    con.puts("[v2] post-reset gctl="); con.put_hex32(gctl1);
    con.puts("\r\n"); con.flush();

    // Step 2f: QSCRATCH VBUS override — full set from Qualcomm downstream kernel
    // Bits: 5=VBUSVLDEXTSEL, 6=VBUSVLDEXT, 20=UTMI_OTG_VBUS_VALID, 28=SW_SESSVLD_SEL
    let hs_phy_before = unsafe { core::ptr::read_volatile((QSCRATCH_BASE + 0x10) as *const u32) };
    unsafe {
        core::ptr::write_volatile((QSCRATCH_BASE + 0x10) as *mut u32,
            hs_phy_before | (1u32 << 5) | (1u32 << 6) | (1u32 << 20) | (1u32 << 28));
        core::ptr::read_volatile((QSCRATCH_BASE + 0x10) as *const u32); // flush
    }
    let hs_phy_after = unsafe { core::ptr::read_volatile((QSCRATCH_BASE + 0x10) as *const u32) };
    con.puts("[v2] hsphy "); con.put_hex32(hs_phy_before);
    con.puts("->"); con.put_hex32(hs_phy_after);
    con.puts("\r\n"); con.flush();

    // Step 2g: Try PHY Power-On Reset through QSCRATCH (bit 9 = PHY_POR)
    // Assert then de-assert to trigger PHY auto-initialization
    {
        let hs = unsafe { core::ptr::read_volatile((QSCRATCH_BASE + 0x10) as *const u32) };
        // Assert PHY_POR
        unsafe { core::ptr::write_volatile((QSCRATCH_BASE + 0x10) as *mut u32, hs | (1u32 << 9)); }
        for _ in 0..100_000 { core::hint::spin_loop(); } // ~1ms
        // De-assert PHY_POR
        unsafe { core::ptr::write_volatile((QSCRATCH_BASE + 0x10) as *mut u32, hs & !(1u32 << 9)); }
        for _ in 0..1_000_000 { core::hint::spin_loop(); } // ~10ms for PHY settle

        let pll = phy_read8(0x1A0);
        let pwr = phy_read8(0x210);
        con.puts("[v2] POR pll="); con.put_hex32(pll as u32);
        con.puts(" pwr="); con.put_hex32(pwr as u32);
        con.puts("\r\n"); con.flush();
    }

    // Step 3: Set device mode (GCTL.PRTCAPDIR = 2 = device)
    let gctl2 = read32(GCTL);
    write32(GCTL, (gctl2 & !GCTL_PRTCAPDIR_MASK) | GCTL_PRTCAP_DEVICE);
    con.puts("[v2] gctl_dev="); con.put_hex32(read32(GCTL));
    con.puts("\r\n"); con.flush();

    // Step 4: Set up event buffer
    let evnt_addr = unsafe { EVENT_BUF.0.as_ptr() as u32 };
    write32(GEVNTSIZ, 0); // disable first
    write32(GEVNTADRLO, evnt_addr);
    write32(GEVNTADRH, 0);
    write32(GEVNTSIZ, 4096);
    write32(GEVNTCOUNT, 0);
    con.puts("[v2] evbuf="); con.put_hex32(evnt_addr);
    con.puts("\r\n"); con.flush();

    // Step 5: Set DCFG for High Speed
    // HS (speed=0) lets DWC3 negotiate speed with host
    write32(DCFG, DCFG_HIGHSPEED);

    // Step 6: Enable device events
    write32(DEVTEN, DEVTEN_DISCONN | DEVTEN_USBRST | DEVTEN_CONNECTDONE | DEVTEN_ULSTCHNG);

    // Step 7: Configure EP0
    // EP0 max packet size = 64 (HS default, will update after connect if SS)
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSETEPCONFIG, 64u32 << 3, 0, 0);
    let dep0 = read32(dep_cmd_reg(PHY_EP0_OUT));
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSETEPCONFIG, 64u32 << 3, 0, 0);
    let dep1 = read32(dep_cmd_reg(PHY_EP0_IN));
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSTARTCFG, 0, 0, 0);
    let dep2 = read32(dep_cmd_reg(PHY_EP0_OUT));
    con.puts("[v2] dep0="); con.put_hex32(dep0);
    con.puts(" dep1="); con.put_hex32(dep1);
    con.puts(" dep2="); con.put_hex32(dep2);
    con.puts("\r\n"); con.flush();

    // Enable EP0
    write32(DALEPENA, (1 << PHY_EP0_OUT) | (1 << PHY_EP0_IN));

    // Queue EP0 OUT TRB
    queue_ep0_out_trb();

    // Step 8: Connect! Set RUN_STOP (bit 31) — this is the ONLY connect mechanism in DWC3
    write32(DCTL, DCTL_RUN_STOP);
    for _ in 0..1_000_000 { core::hint::spin_loop(); } // ~10ms for host detection

    let dctl_f = read32(DCTL);
    let dsts_f = read32(DSTS);
    con.puts("[v2] done dctl="); con.put_hex32(dctl_f);
    con.puts(" dsts="); con.put_hex32(dsts_f);
    con.puts("\r\n"); con.flush();

    true
}

/// SMC IS_CALL_AVAIL probe: check if a specific SCM call is supported by TZ firmware.
/// Returns x0 result: 1 = available, 0 = not available, other = error.
pub fn smc_is_call_avail(fnid_to_check: u32) -> u64 {
    let ret: u64;
    unsafe {
        let fnid: u64 = 0xC2000601; // IS_CALL_AVAIL: ARM_64 FAST SIP
        core::arch::asm!(
            "mov x6, xzr",
            "smc #0",
            "mov {ret}, x0",
            in("x0") fnid,
            in("x1") 1u64,                    // arginfo: 1 argument
            in("x2") fnid_to_check as u64,    // the call to probe
            ret = out(reg) ret,
            out("x3") _, out("x4") _, out("x5") _, out("x6") _,
        );
    }
    ret
}

// ── Cold init: full DWC3 initialization from scratch (like Linux dwc3_core_init) ──

/// Full cold init: GCC clocks → PHY init → DWC3 core reset → configure → start.
/// Follows the Linux dwc3_core_init() sequence exactly.
pub fn init_cold(con: &mut crate::fb::Console) -> bool {
    // Step 1: Verify DWC3 is present
    let snpsid = read32(GSNPSID);
    con.puts("[cold] SNPSID="); con.put_hex32(snpsid);
    con.puts("\r\n"); con.flush();
    if snpsid == 0 || snpsid == 0xFFFF_FFFF {
        con.puts("[cold] no DWC3\r\n"); con.flush();
        return false;
    }

    // Step 2: GCC USB30 clocks (GDSC + branch clocks + AHB2PHY BCR, no QUSB2PHY BCR)
    con.puts("[cold] clocks..\r\n"); con.flush();
    if !crate::gcc::enable_usb30_clocks_no_phy_reset() {
        con.puts("[cold] clk fail\r\n"); con.flush();
        return false;
    }
    con.puts("[cold] clocks ok\r\n"); con.flush();

    // Step 3: DWC3 core soft reset (Linux dwc3_core_soft_reset sequence)
    con.puts("[cold] core reset..\r\n"); con.flush();

    // 3a. Assert core soft reset FIRST
    let gctl = read32(GCTL);
    write32(GCTL, gctl | GCTL_CORESOFTRESET);

    // 3b. Assert USB2 PHY soft reset while core is in reset
    write32(GUSB2PHYCFG, read32(GUSB2PHYCFG) | (1 << 0)); // PHYSOFTRST

    // 3c. Wait 100ms (Linux uses mdelay(100))
    for _ in 0..10_000_000 { core::hint::spin_loop(); }

    // 3d. Release USB2 PHY soft reset
    write32(GUSB2PHYCFG, read32(GUSB2PHYCFG) & !(1 << 0));

    // 3e. Set device mode before releasing core reset
    write32(GCTL, (read32(GCTL) & !GCTL_PRTCAPDIR_MASK) | GCTL_PRTCAP_DEVICE);

    // 3f. Release core soft reset
    write32(GCTL, read32(GCTL) & !GCTL_CORESOFTRESET);

    // 3g. Wait for core to stabilize
    for _ in 0..2_000_000 { core::hint::spin_loop(); }

    // Verify core is alive
    let snpsid2 = read32(GSNPSID);
    if snpsid2 == 0 || snpsid2 == 0xFFFF_FFFF {
        con.puts("[cold] core dead after reset\r\n"); con.flush();
        return false;
    }
    con.puts("[cold] core ok\r\n"); con.flush();

    // Step 4: Configure USB2 PHY interface
    // USBTRDTIM=9 for HS (8-bit UTMI), ENBLSLPM=1, SUSPHY=0
    write32(GUSB2PHYCFG, (9u32 << 10) | (1 << 8));

    // Step 4b: Try QMP SS PHY init (at 0x088E9000)
    con.puts("[cold] QMP PHY..\r\n"); con.flush();
    let qmp_ok = init_qmp_phy(con);

    // Step 5: QUSB2 PHY — check PLL, only init if not locked
    con.puts("[cold] QUSB2 PHY..\r\n"); con.flush();
    let pll_before = phy_read8(0x1A0);
    let pwr_before = phy_read8(0x210);
    con.puts("[cold] pre pll="); con.put_hex32(pll_before as u32);
    con.puts(" pwr="); con.put_hex32(pwr_before as u32);
    con.puts("\r\n"); con.flush();
    let mut hs_ok = false;
    if pll_before & 1 != 0 {
        con.puts("[cold] PLL already locked, skip PHY init\r\n"); con.flush();
        hs_ok = true;
    } else {
        con.puts("[cold] PLL not locked, init..\r\n"); con.flush();
        hs_ok = init_hsphy(con);
        if !hs_ok {
            con.puts("[cold] PHY PLL NOT locked\r\n"); con.flush();
        } else {
            con.puts("[cold] PHY ok\r\n"); con.flush();
        }
    }

    // Step 6: Set FIFO sizes (CRITICAL — after core reset, TX FIFO is unallocated!)
    write32(GRXFIFOSIZ, (0u32 << 16) | 256);   // RX: start=0, depth=256 DWORDs
    write32(GTXFIFOSIZ, (256u32 << 16) | 64);  // TX0: start=256, depth=64 DWORDs
    con.puts("[cold] FIFO ok\r\n"); con.flush();

    // Step 7: QSCRATCH VBUS override
    let hs_phy = qscratch_read(0x10);
    qscratch_write(0x10, hs_phy | (1 << 20) | (1 << 28));

    // Step 8: Set up event buffer
    let evnt_addr = unsafe { EVENT_BUF.0.as_ptr() as u32 };
    write32(GEVNTADRLO, evnt_addr);
    write32(GEVNTADRH, 0);
    write32(GEVNTSIZ, 4096);
    write32(GEVNTCOUNT, 0);
    let rb = read32(GEVNTADRLO);
    unsafe { ABL_EVBUF = rb; }
    con.puts("[cold] evbuf="); con.put_hex32(evnt_addr);
    con.puts(" rb="); con.put_hex32(rb);
    con.puts("\r\n"); con.flush();

    // Step 9: Set DCFG speed — SS if QMP ok, HS if QUSB2 ok, FS fallback
    let speed = if qmp_ok { 4u32 } else if hs_ok { 0u32 } else { 1u32 };
    let ep0_mps = if speed == 4 { 512u32 } else { 64u32 };
    write32(DCFG, speed);
    con.puts("[cold] DCFG speed="); con.put_hex32(speed);
    con.puts(" mps="); con.put_hex32(ep0_mps);
    con.puts("\r\n"); con.flush();

    // Step 10: Enable device events
    write32(DEVTEN, 0x0F);

    // Step 11: Configure EP0 (no DEPSETTRANSF for EP0 — implicit resources)
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSETEPCONFIG,
        ep0_mps << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSETEPCONFIG,
        ep0_mps << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSTARTCFG, 0, 0, 0);
    write32(DALEPENA, (1 << PHY_EP0_OUT) | (1 << PHY_EP0_IN));
    queue_ep0_out_trb();
    con.puts("[cold] EP0 ok\r\n"); con.flush();

    // Step 12: Start device (set RUN_STOP, clear SOFTDISCONNECT)
    con.puts("[cold] start..\r\n"); con.flush();
    let dctl = read32(DCTL);
    write32(DCTL, dctl | DCTL_RUN_STOP);

    // Step 13: Wait for host enumeration (~1 second)
    for _ in 0..10_000_000 { core::hint::spin_loop(); }

    // Step 14: Diagnostics
    let dctl_f = read32(DCTL);
    let dsts_f = read32(DSTS);
    let evcnt_f = read32(GEVNTCOUNT);
    let gctl_f = read32(GCTL);
    let phycfg = read32(GUSB2PHYCFG);
    let buf0 = unsafe { core::ptr::read_volatile(EVENT_BUF.0.as_ptr() as *const u32) };
    let pll = phy_read8(0x1A0);
    let pwr = phy_read8(0x210);
    con.puts("[cold] dctl="); con.put_hex32(dctl_f);
    con.puts(" dsts="); con.put_hex32(dsts_f);
    con.puts(" evc="); con.put_hex32(evcnt_f);
    con.puts(" pll="); con.put_hex32(pll as u32);
    con.puts(" pwr="); con.put_hex32(pwr as u32);
    con.puts("\r\n"); con.flush();
    con.puts("[cold] gctl="); con.put_hex32(gctl_f);
    con.puts(" phycfg="); con.put_hex32(phycfg);
    con.puts(" buf0="); con.put_hex32(buf0);
    con.puts("\r\n"); con.flush();

    con.puts("[cold] done\r\n"); con.flush();

    unsafe {
        DEV_ADDR = 0;
        CONFIGURED = false;
        TX_PENDING = false;
    }

    true
}

/// Cold init with SMC IO: GCC clocks → SMC PHY init → DWC3 core init.
/// Uses TrustZone SMC calls for PHY register writes to bypass EL1 TZ protection.
/// This is the recommended init path for Pixel 5 where PHY registers are TZ-write-protected.
pub fn init_cold_smc(con: &mut crate::fb::Console) -> bool {
    // Step 1: Verify DWC3 is present
    let snpsid = read32(GSNPSID);
    con.puts("[cold_smc] SNPSID="); con.put_hex32(snpsid);
    con.puts("\r\n"); con.flush();
    if snpsid == 0 || snpsid == 0xFFFF_FFFF {
        con.puts("[cold_smc] no DWC3\r\n"); con.flush();
        return false;
    }

    // Step 2: GCC USB30 clocks (full init with BCR resets)
    con.puts("[cold_smc] clocks..\r\n"); con.flush();
    match crate::gcc::enable_usb30_clocks_debug() {
        None => con.puts("[cold_smc] clk ok\r\n"),
        Some((step, val)) => {
            con.puts("[cold_smc] clk FAIL s=");
            crate::print_dec_u32(0, step as u32);
            con.puts(" v="); con.put_hex32(val);
            con.puts("\r\n");
            return false;
        }
    }
    con.flush();

    // Step 3: DWC3 core soft reset (before PHY init)
    con.puts("[cold_smc] core reset..\r\n"); con.flush();
    let gctl = read32(GCTL);
    write32(GCTL, gctl | GCTL_CORESOFTRESET);
    write32(GUSB2PHYCFG, read32(GUSB2PHYCFG) | (1 << 0)); // PHYSOFTRST
    for _ in 0..2_000_000 { core::hint::spin_loop(); }
    write32(GUSB2PHYCFG, read32(GUSB2PHYCFG) & !(1 << 0));
    write32(GCTL, (read32(GCTL) & !GCTL_PRTCAPDIR_MASK) | GCTL_PRTCAP_DEVICE);
    write32(GCTL, read32(GCTL) & !GCTL_CORESOFTRESET);
    for _ in 0..2_000_000 { core::hint::spin_loop(); }

    let snpsid2 = read32(GSNPSID);
    if snpsid2 == 0 || snpsid2 == 0xFFFF_FFFF {
        con.puts("[cold_smc] core dead\r\n"); con.flush();
        return false;
    }
    con.puts("[cold_smc] core ok\r\n"); con.flush();

    // Step 4: Configure USB2 PHY interface
    write32(GUSB2PHYCFG, (9u32 << 10) | (1 << 8)); // USBTRDTIM=9, ENBLSLPM

    // Step 5: SMC PHY init (bypasses TZ write protection)
    con.puts("[cold_smc] SMC PHY..\r\n"); con.flush();
    let phy_ok = init_hsphy_smc(con);
    if !phy_ok {
        con.puts("[cold_smc] PHY PLL NOT locked, continuing..\r\n"); con.flush();
        // Don't return false — try anyway, DWC3 might still work
    }

    // Step 6: Set FIFO sizes
    write32(GRXFIFOSIZ, (0u32 << 16) | 256);
    write32(GTXFIFOSIZ, (256u32 << 16) | 64);

    // Step 7: QSCRATCH VBUS override
    let hs_phy = qscratch_read(0x10);
    qscratch_write(0x10, hs_phy | (1 << 20) | (1 << 28));

    // Step 8: Event buffer
    let evnt_addr = unsafe { EVENT_BUF.0.as_ptr() as u32 };
    write32(GEVNTADRLO, evnt_addr);
    write32(GEVNTADRH, 0);
    write32(GEVNTSIZ, 4096);
    write32(GEVNTCOUNT, 0);
    unsafe { ABL_EVBUF = read32(GEVNTADRLO); }
    con.puts("[cold_smc] evbuf="); con.put_hex32(evnt_addr);
    con.puts("\r\n"); con.flush();

    // Step 9: DCFG speed (HS if PHY ok, FS fallback)
    let speed = if phy_ok { 0u32 } else { 1u32 }; // 0=HS, 1=FS
    write32(DCFG, speed);

    // Step 10: Enable device events
    write32(DEVTEN, 0x0F);

    // Step 11: Configure EP0
    let mps = if speed == 0 { 64u32 } else { 64u32 };
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSETEPCONFIG, mps << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSETEPCONFIG, mps << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSTARTCFG, 0, 0, 0);
    write32(DALEPENA, (1 << PHY_EP0_OUT) | (1 << PHY_EP0_IN));
    queue_ep0_out_trb();

    // Step 12: Start device (connect)
    con.puts("[cold_smc] start..\r\n"); con.flush();
    write32(DCTL, read32(DCTL) | DCTL_RUN_STOP);

    // Step 13: Wait for host
    for _ in 0..10_000_000 { core::hint::spin_loop(); }

    // Step 14: Diagnostics
    let dctl_f = read32(DCTL);
    let dsts_f = read32(DSTS);
    let evc = read32(GEVNTCOUNT);
    con.puts("[cold_smc] dctl="); con.put_hex32(dctl_f);
    con.puts(" dsts="); con.put_hex32(dsts_f);
    con.puts(" evc="); con.put_hex32(evc);
    con.puts("\r\n"); con.flush();

    unsafe {
        DEV_ADDR = 0;
        CONFIGURED = false;
        TX_PENDING = false;
    }

    true
}

// ── Warm SS init: reuse ABL's PHY/clock state, only reconfigure DWC3 device ──

/// Warm init: DON'T touch clocks, PHY, or DCTL.
/// ABL left DWC3 running in SS mode (CONNECTSPD=4) with QMP PHY.
/// Just set up event buffer + EP0 while device stays connected.
pub fn init_warm_ss(con: &mut crate::fb::Console) -> bool {
    // ── Step 1: Verify DWC3 present ──
    let snpsid = read32(GSNPSID);
    if snpsid == 0 || snpsid == 0xFFFF_FFFF {
        con.puts("[usb] no DWC3\r\n"); con.flush();
        return false;
    }
    con.puts("[usb] snps="); con.put_hex32(snpsid);
    con.puts("\r\n"); con.flush();

    // ── Step 2: GCC USB30 clocks (GDSC + branch clocks + AHB2PHY) ──
    con.puts("[usb] clocks..\r\n"); con.flush();
    match crate::gcc::enable_usb30_clocks_debug() {
        None => con.puts("[usb] clk ok\r\n"),
        Some((step, val)) => {
            con.puts("[usb] clk FAIL s=");
            crate::print_dec_u32(0, step as u32);
            con.puts(" v="); con.put_hex32(val);
            con.puts("\r\n");
        }
    }
    con.flush();

    // ── Step 3: QUSB2 PHY init ──
    con.puts("[usb] phy..\r\n"); con.flush();
    let pll_ok = init_hsphy(con);
    con.puts("[usb] phy "); con.puts(if pll_ok { "ok" } else { "noPLL" });
    con.puts("\r\n"); con.flush();

    // ── Step 4: DWC3 core soft reset ──
    con.puts("[usb] reset..\r\n"); con.flush();
    let gctl = read32(GCTL);
    write32(GCTL, gctl | GCTL_CORESOFTRESET);
    write32(GUSB2PHYCFG, read32(GUSB2PHYCFG) | (1 << 0)); // PHYSOFTRST
    for _ in 0..10_000_000 { core::hint::spin_loop(); }
    write32(GUSB2PHYCFG, read32(GUSB2PHYCFG) & !(1 << 0));
    write32(GCTL, (read32(GCTL) & !GCTL_PRTCAPDIR_MASK) | GCTL_PRTCAP_DEVICE);
    write32(GCTL, read32(GCTL) & !GCTL_CORESOFTRESET);
    for _ in 0..2_000_000 { core::hint::spin_loop(); }
    let snpsid2 = read32(GSNPSID);
    con.puts("[usb] post-reset snps=");
    con.put_hex32(snpsid2); con.puts("\r\n"); con.flush();

    // ── Step 5: Configure PHY interface (HS, 8-bit UTMI) ──
    write32(GUSB2PHYCFG, (9u32 << 10) | (1 << 8));

    // ── Step 6: FIFO sizes (after reset TX FIFO is unallocated) ──
    write32(GRXFIFOSIZ, (0u32 << 16) | 256);   // RX: start=0, depth=256
    write32(GTXFIFOSIZ, (256u32 << 16) | 64);  // TX0: start=256, depth=64

    // ── Step 7: QSCRATCH VBUS override ──
    let hs_phy = qscratch_read(0x10);
    qscratch_write(0x10, hs_phy | (1 << 20) | (1 << 28));

    // ── Step 8: Event buffer ──
    let evnt_addr = unsafe { EVENT_BUF.0.as_ptr() as u32 };
    write32(GEVNTADRLO, evnt_addr);
    write32(GEVNTADRH, 0);
    write32(GEVNTSIZ, 4096);
    write32(GEVNTCOUNT, 0);
    let rb = read32(GEVNTADRLO);
    unsafe { ABL_EVBUF = rb; }

    // ── Step 9: DCFG = Full Speed ──
    write32(DCFG, DCFG_FULLSPEED);

    // ── Step 10: Enable device events ──
    write32(DEVTEN, 0x0F);

    // ── Step 11: Configure EP0 ──
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSETEPCONFIG,
        512u32 << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSETTRANSF, 1, 0, 0);
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSETEPCONFIG,
        512u32 << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSETTRANSF, 1, 0, 0);
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSTARTCFG, 0, 0, 0);
    write32(DALEPENA, (1 << PHY_EP0_OUT) | (1 << PHY_EP0_IN));
    queue_ep0_out_trb();

    // ── Step 12: Start device ──
    con.puts("[usb] start..\r\n"); con.flush();
    write32(DCTL, read32(DCTL) | DCTL_RUN_STOP);

    // Wait for host to enumerate
    for _ in 0..10_000_000 { core::hint::spin_loop(); }

    // ── Step 13: Diagnostics ──
    let dctl_f = read32(DCTL);
    let dsts_f = read32(DSTS);
    let evc_f = read32(GEVNTCOUNT);
    let gctl_f = read32(GCTL);
    let phycfg = read32(GUSB2PHYCFG);
    let pll_f = phy_read8(0x1A0);
    let buf0 = unsafe { core::ptr::read_volatile(EVENT_BUF.0.as_ptr() as *const u32) };
    con.puts("[usb] dctl="); con.put_hex32(dctl_f);
    con.puts(" dsts="); con.put_hex32(dsts_f);
    con.puts(" evc="); con.put_hex32(evc_f);
    con.puts("\r\n"); con.flush();
    con.puts("[usb] gctl="); con.put_hex32(gctl_f);
    con.puts(" phy="); con.put_hex32(phycfg);
    con.puts(" pll="); con.put_hex32(pll_f as u32);
    con.puts("\r\n"); con.flush();
    con.puts("[usb] buf0="); con.put_hex32(buf0);
    con.puts("\r\n"); con.flush();

    unsafe {
        DEV_ADDR = 0;
        CONFIGURED = false;
        TX_PENDING = false;
    }

    con.puts("[usb] done\r\n"); con.flush();
    true
}

// ── Initialization ─────────────────────────────────────────────────

/// Minimal USB init: configure QSCRATCH wrapper + start DWC3 device
/// No BCR resets, no PHY changes, no DWC3 core reset.
/// Based on Linux dwc3-qcom.c initialization sequence.
pub fn init_minimal() -> bool {
    // 1. Verify DWC3 is present
    let snpsid = read32(GSNPSID);
    if snpsid == 0 || snpsid == 0xFFFF_FFFF {
        return false;
    }

    // 2. QSCRATCH: VBUS override for device mode (skip GENERAL_CFG for now)
    // HS_PHY_CTRL: set UTMI_OTG_VBUS_VALID (BIT(20)) + SW_SESSVLD_SEL (BIT(28))
    let hs_phy = qscratch_read(0x10);
    qscratch_write(0x10, hs_phy | (1 << 20) | (1 << 28));

    // 3. Set up event buffer
    let evnt_addr = unsafe { EVENT_BUF.0.as_ptr() as u32 };
    write32(GEVNTADRLO, evnt_addr);
    write32(GEVNTADRH, 0);
    write32(GEVNTSIZ, 4096);
    write32(GEVNTCOUNT, 0);

    // 4. Enable device events
    write32(DEVTEN, DEVTEN_DISCONN | DEVTEN_USBRST | DEVTEN_CONNECTDONE | DEVTEN_ULSTCHNG);

    // 5. Configure EP0
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSETEPCONFIG,
        512u32 << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSETTRANSF, 1, 0, 0);
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSETEPCONFIG,
        512u32 << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSETTRANSF, 1, 0, 0);
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSTARTCFG, 0, 0, 0);

    // 6. Enable EP0
    write32(DALEPENA, (1 << PHY_EP0_OUT) | (1 << PHY_EP0_IN));

    // 7. Queue EP0 OUT TRB
    queue_ep0_out_trb();

    // 8. Set DCFG speed to HS (ABL left it as SS=4)
    let dcfg = read32(DCFG);
    write32(DCFG, (dcfg & !0x7) | 0x0);

    // 9. Start device: set RUN_STOP, clear SOFTDISCONNECT
    let dctl = read32(DCTL);
    write32(DCTL, dctl | DCTL_RUN_STOP);

    unsafe {
        DEV_ADDR = 0;
        CONFIGURED = false;
        TX_PENDING = false;
    }

    true
}

/// Warm connect: enable clocks (no PHY reset), then just DWC3 connect.
/// Preserves ABL's PHY state. If ABL left PHY working, this should just work.
pub fn init_warm_connect(con: &mut crate::fb::Console) -> bool {
    // 1. Verify DWC3 is present
    let snpsid = read32(GSNPSID);
    if snpsid == 0 || snpsid == 0xFFFF_FFFF {
        return false;
    }

    // Diagnostic: print current PHY/DWC3 state WITHOUT changing anything
    let phycfg = read32(GUSB2PHYCFG);
    let dctl0 = read32(DCTL);
    let dsts0 = read32(DSTS);
    let gctl0 = read32(GCTL);
    let pwr = phy_read8(0x210);
    let pll = phy_read8(0x1A0);
    let tune1 = phy_read8(0x04);
    con.puts("[11diag] snpsid="); con.put_hex32(snpsid);
    con.puts(" phycfg="); con.put_hex32(phycfg);
    con.puts(" dctl="); con.put_hex32(dctl0);
    con.puts(" dsts="); con.put_hex32(dsts0);
    con.puts("\r\n"); con.flush();
    con.puts("[11diag] gctl="); con.put_hex32(gctl0);
    con.puts(" pwr="); con.put_hex32(pwr as u32);
    con.puts(" pll="); con.put_hex32(pll as u32);
    con.puts(" tune1="); con.put_hex32(tune1 as u32);
    con.puts("\r\n"); con.flush();

    // 2. QSCRATCH: VBUS override (keep ABL's other bits)
    let hs_phy = qscratch_read(0x10);
    qscratch_write(0x10, hs_phy | (1 << 20) | (1 << 28));

    // 3. DWC3 core soft reset (WITHOUT PHY reset) to clear ABL's stale state
    let gctl = read32(GCTL);
    write32(GCTL, gctl | GCTL_CORESOFTRESET);
    for _ in 0..10_000 { core::hint::spin_loop(); }
    write32(GCTL, read32(GCTL) & !GCTL_CORESOFTRESET);
    for _ in 0..10_000 { core::hint::spin_loop(); }

    // 4. Set device mode
    set_device_mode();

    // 5. Set up event buffer
    let evnt_addr = unsafe { EVENT_BUF.0.as_ptr() as u32 };
    write32(GEVNTADRLO, evnt_addr);
    write32(GEVNTADRH, 0);
    write32(GEVNTSIZ, 4096);
    write32(GEVNTCOUNT, 0);

    // 6. Enable device events
    write32(DEVTEN, DEVTEN_DISCONN | DEVTEN_USBRST | DEVTEN_CONNECTDONE | DEVTEN_ULSTCHNG);

    // 7. Configure EP0
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSETEPCONFIG,
        512u32 << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSETTRANSF, 1, 0, 0);
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSETEPCONFIG,
        512u32 << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSETTRANSF, 1, 0, 0);
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSTARTCFG, 0, 0, 0);
    write32(DALEPENA, (1 << PHY_EP0_OUT) | (1 << PHY_EP0_IN));
    queue_ep0_out_trb();

    // 8. Set DCFG to FS
    let dcfg = read32(DCFG);
    write32(DCFG, (dcfg & !0x7) | 0x1);

    // 9. Soft disconnect → wait → reconnect to force host re-enumeration
    let dctl = read32(DCTL);
    write32(DCTL, dctl & !DCTL_RUN_STOP); // disconnect: clear RUN_STOP
    for _ in 0..5_000_000 { core::hint::spin_loop(); }  // ~500ms disconnect
    write32(DCTL, read32(DCTL) | DCTL_RUN_STOP);

    unsafe {
        DEV_ADDR = 0;
        CONFIGURED = false;
        TX_PENDING = false;
    }

    true
}

/// Full USB init: GCC clocks + PHY reinit + DWC3 config.
/// Call AFTER enabling PMIC LDO regulators (vdda-phy, vdda-pll).
pub fn init_wakeup(con: &mut crate::fb::Console) -> bool {
    // 1. Verify DWC3 is present
    let snpsid = read32(GSNPSID);
    if snpsid == 0 || snpsid == 0xFFFF_FFFF {
        return false;
    }

    con.puts("[11] USB init..\r\n"); con.flush();
    con.puts("[11] SNPSID=0x"); con.put_hex32(snpsid);
    con.puts("\r\n"); con.flush();

    // 2. GCC USB30 clocks + QUSB2PHY BCR reset
    if !init_platform(con) {
        con.puts("[11] platform FAIL\r\n"); con.flush();
        return false;
    }

    // 3. Clear SUSPHY in DWC3
    let phycfg = read32(GUSB2PHYCFG);
    write32(GUSB2PHYCFG, phycfg & !(1u32 << 30));
    for _ in 0..500_000 { core::hint::spin_loop(); }

    con.puts("[11c] DWC3 cfg..\r\n"); con.flush();

    // 4. QSCRATCH: VBUS override
    let hs_phy = qscratch_read(0x10);
    qscratch_write(0x10, hs_phy | (1 << 20) | (1 << 28));

    // 5. Set up event buffer
    let evnt_addr = unsafe { EVENT_BUF.0.as_ptr() as u32 };
    write32(GEVNTADRLO, evnt_addr);
    write32(GEVNTADRH, 0);
    write32(GEVNTSIZ, 4096);
    write32(GEVNTCOUNT, 0);

    // 6. Enable device events
    write32(DEVTEN, DEVTEN_DISCONN | DEVTEN_USBRST | DEVTEN_CONNECTDONE | DEVTEN_ULSTCHNG);

    // 7. Configure EP0
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSETEPCONFIG,
        512u32 << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSETTRANSF, 1, 0, 0);
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSETEPCONFIG,
        512u32 << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSETTRANSF, 1, 0, 0);
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSTARTCFG, 0, 0, 0);
    write32(DALEPENA, (1 << PHY_EP0_OUT) | (1 << PHY_EP0_IN));
    queue_ep0_out_trb();

    con.puts("[11c] EP0 ok\r\n"); con.flush();

    // 8. Set DCFG to FS
    let dcfg = read32(DCFG);
    write32(DCFG, (dcfg & !0x7) | 0x1);

    // 9. Start device
    let dctl = read32(DCTL);
    write32(DCTL, dctl | DCTL_RUN_STOP);

    con.puts("[11d] RUN_STOP\r\n"); con.flush();

    unsafe {
        DEV_ADDR = 0;
        CONFIGURED = false;
        TX_PENDING = false;
    }

    true
}

/// Read QSCRATCH GENERAL_CFG register (offset 0x08)
pub fn read_general_cfg() -> u32 {
    qscratch_read(0x08)
}

/// Read QSCRATCH register
fn qscratch_read(off: usize) -> u32 {
    unsafe { core::ptr::read_volatile((QSCRATCH_BASE + off) as *const u32) }
}

/// Write QSCRATCH register with readback flush
fn qscratch_write(off: usize, val: u32) {
    unsafe {
        core::ptr::write_volatile((QSCRATCH_BASE + off) as *mut u32, val);
        core::ptr::read_volatile((QSCRATCH_BASE + off) as *const u32); // flush
    }
}

/// Warm init: keep ABL DWC3 state, change to HS + UTMI clock.
/// 1. Change DCFG to HS (stop DWC3 from using PIPE/SS)
/// 2. Stop device
/// 3. QSCRATCH UTMI clock selection
/// 4. Re-configure and start
pub fn init_warm() -> bool {
    let snpsid = read32(GSNPSID);
    if snpsid == 0 || snpsid == 0xFFFF_FFFF { return false; }

    // 1. Switch DCFG to HS first — tells DWC3 to not use SuperSpeed/PIPE
    let dcfg = read32(DCFG);
    write32(DCFG, (dcfg & !DCFG_SPEED_MASK) | DCFG_HIGHSPEED);

    // 2. Stop device (clear RUN_STOP, set SOFTDISCONNECT)
    let dctl = read32(DCTL);
    write32(DCTL, dctl & !DCTL_RUN_STOP); // disconnect: clear RUN_STOP
    // Wait for device to fully stop
    for _ in 0..500_000 { core::hint::spin_loop(); }

    // 3. QSCRATCH GENERAL_CFG: UTMI clock selection
    let gen0 = qscratch_read(0x08);
    qscratch_write(0x08, gen0 | (1 << 8)); // PIPE_UTMI_CLK_DIS
    let gen1 = qscratch_read(0x08);
    qscratch_write(0x08, gen1 | (1 << 0)); // PIPE_UTMI_CLK_SEL
    let gen2 = qscratch_read(0x08);
    qscratch_write(0x08, gen2 | (1 << 3)); // PIPE3_PHYSTATUS_SW
    let gen3 = qscratch_read(0x08);
    qscratch_write(0x08, gen3 & !(1 << 8)); // clear PIPE_UTMI_CLK_DIS

    // 4. HS_PHY_CTRL VBUS override
    let hs_phy = qscratch_read(0x10);
    qscratch_write(0x10, hs_phy | (1 << 20) | (1 << 28));

    // 5. Set up event buffer
    let evnt_addr = unsafe { EVENT_BUF.0.as_ptr() as u32 };
    write32(GEVNTADRLO, evnt_addr);
    write32(GEVNTADRH, 0);
    write32(GEVNTSIZ, 4096);
    write32(GEVNTCOUNT, 0);

    // 6. Enable device events
    write32(DEVTEN, DEVTEN_DISCONN | DEVTEN_USBRST | DEVTEN_CONNECTDONE | DEVTEN_ULSTCHNG);

    // 7. Configure EP0
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSETEPCONFIG,
        512u32 << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSETTRANSF, 1, 0, 0);
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSETEPCONFIG,
        512u32 << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSETTRANSF, 1, 0, 0);
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSTARTCFG, 0, 0, 0);
    write32(DALEPENA, (1 << PHY_EP0_OUT) | (1 << PHY_EP0_IN));
    queue_ep0_out_trb();

    // 8. Start device: set RUN_STOP, clear SOFTDISCONNECT
    let dctl2 = read32(DCTL);
    write32(DCTL, dctl2 | DCTL_RUN_STOP);

    unsafe {
        DEV_ADDR = 0;
        CONFIGURED = false;
        TX_PENDING = false;
    }

    true
}

// ── Step-by-step init (core reset → QSCRATCH → device config) ──────

/// Perform DWC3 core soft reset with PHY reset (Linux dwc3_core_init sequence).
/// MUST be called first — puts DWC3 core in idle state.
pub fn dwc3_core_reset() -> bool {
    let snpsid = read32(GSNPSID);
    if snpsid == 0 || snpsid == 0xFFFF_FFFF { return false; }

    // 1. Put USB2 PHY in soft reset before core reset
    write32(GUSB2PHYCFG, read32(GUSB2PHYCFG) | (1 << 0)); // PHYSOFTRST

    // 2. Assert core soft reset
    let gctl = read32(GCTL);
    write32(GCTL, gctl | GCTL_CORESOFTRESET);
    for _ in 0..100_000 { core::hint::spin_loop(); }
    // Manually clear reset (DWC3 spec requires software to clear this bit)
    write32(GCTL, read32(GCTL) & !GCTL_CORESOFTRESET);
    for _ in 0..100_000 { core::hint::spin_loop(); }

    // 3. Clear USB2 PHY soft reset after core reset
    write32(GUSB2PHYCFG, read32(GUSB2PHYCFG) & !(1 << 0)); // clear PHYSOFTRST
    // Wait 1ms for PHY to come out of reset
    for _ in 0..2_000_000 { core::hint::spin_loop(); }

    // Verify core is alive
    let snpsid2 = read32(GSNPSID);
    snpsid2 != 0 && snpsid2 != 0xFFFF_FFFF
}

/// Configure QSCRATCH wrapper for UTMI/HS operation.
/// Returns final GENERAL_CFG value for diagnostics.
pub fn qscratch_configure_utmi() -> u32 {
    // Skip GENERAL_CFG for now — test without it
    qscratch_read(0x08)
}

/// Pulse QSCRATCH PHY POR and leave COMMONONN + VBUS override set.
pub fn qscratch_phy_por() -> u32 {
    let hs = qscratch_read(0x10);
    qscratch_write(0x10, hs | (1 << 9));
    for _ in 0..200_000 {
        core::hint::spin_loop();
    }
    let hs2 = qscratch_read(0x10);
    qscratch_write(
        0x10,
        (hs2 & !(1 << 9)) | (1 << 11) | (1 << 20) | (1 << 28),
    );
    qscratch_read(0x10)
}

/// Synopsys GUSB2PHYACC (DWC3 spec bits), not the custom layout.
pub fn phyacc_snps_read(addr: u8) -> (bool, u32) {
    write32(0xc280, (1 << 25) | ((addr as u32) << 16));
    for _ in 0..100_000 {
        let v = read32(0xc280);
        if v & (1 << 23) == 0 {
            return (true, v);
        }
        core::hint::spin_loop();
    }
    (false, read32(0xc280))
}

pub fn phyacc_snps_write(addr: u8, data: u8) -> bool {
    write32(
        0xc280,
        (1 << 25) | (1 << 22) | ((addr as u32) << 16) | (data as u32),
    );
    for _ in 0..100_000 {
        if read32(0xc280) & (1 << 23) == 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

/// Set VBUS override in QSCRATCH HS_PHY_CTRL.
/// Returns read-back value for diagnostics.
pub fn qscratch_vbus_override() -> u32 {
    let ss = qscratch_read(0x30);
    qscratch_write(0x30, ss | (1 << 24)); // SS_PHY_CTRL: LANE0_PWR_PRESENT
    let hs_phy = qscratch_read(0x10);
    // bits 5/6 = VBUSVLDEXTSEL/VBUSVLDEXT, 20 = UTMI_OTG_VBUS_VALID, 28 = SW_SESSVLD_SEL
    qscratch_write(0x10, hs_phy | (1 << 5) | (1 << 6) | (1 << 20) | (1 << 28));
    qscratch_read(0x10)
}

fn qscratch_spin() {
    for _ in 0..100_000 {
        core::hint::spin_loop();
    }
}

/// Force DWC3 to use UTMI/HS instead of PIPE/SS (QSCRATCH GENERAL_CFG).
pub fn qscratch_force_utmi() -> u32 {
    let g0 = qscratch_read(0x08);
    qscratch_write(0x08, g0 | (1 << 8)); // PIPE_UTMI_CLK_DIS
    qscratch_spin();
    let g1 = qscratch_read(0x08);
    qscratch_write(0x08, g1 | (1 << 0) | (1 << 3)); // PIPE_UTMI_CLK_SEL | PIPE3_PHYSTATUS_SW
    qscratch_spin();
    let g2 = qscratch_read(0x08);
    qscratch_write(0x08, g2 & !(1 << 8)); // clear PIPE_UTMI_CLK_DIS
    qscratch_read(0x08)
}

pub fn qscratch_hs_phy() -> u32 {
    qscratch_read(0x10)
}

/// Clear GUSB2PHYCFG SUSPHY + ENBLSLPM (Pixel DT quirks).
pub fn clear_susphy() -> u32 {
    let r = read32(GUSB2PHYCFG);
    write32(GUSB2PHYCFG, r & !((1 << 6) | (1 << 8)));
    read32(GUSB2PHYCFG)
}

/// DCFG = High Speed + NUMP=16. Do not leave NUMP=0.
pub fn set_dcfg_hs_nump() {
    write32(DCFG, 16u32 << 17);
}

/// Keep USB3 PIPE PHY suspended so the core stays on UTMI/HS.
pub fn suspend_usb3_pipe() {
    const GUSB3PIPECTL: usize = 0xc2c0;
    let p = read32(GUSB3PIPECTL);
    write32(GUSB3PIPECTL, p | (1 << 17));
}

/// Configure and start DWC3 device mode.
/// Call AFTER dwc3_core_reset() + qscratch_configure_utmi() + qscratch_vbus_override().
/// Individual sub-steps are also exposed as pub functions for diagnostics.
pub fn init_device() -> bool {
    set_device_mode();
    set_usb2phycfg();
    setup_event_buffer();
    set_dcfg_hs();
    configure_ep0();
    start_device();
    true
}

/// Sub-step: Set GCTL PRTCAPDIR to device mode
pub fn set_device_mode() {
    let gctl = read32(GCTL);
    write32(GCTL, (gctl & !GCTL_PRTCAPDIR_MASK) | GCTL_PRTCAP_DEVICE);
}

/// Sub-step: Configure USB2 PHY interface
/// USBTRDTIM=9 for HS, ENBLSLPM=1, SUSPHY=1 (Linux does this)
pub fn set_usb2phycfg() {
    write32(GUSB2PHYCFG, (9u32 << 10) | (1 << 8) | (1 << 30));
}

/// Sub-step: Set TX/RX FIFO sizes (critical after core reset!)
/// RX FIFO: 256 DWORDs (1KB) starting at 0
/// TX FIFO 0 (EP0 IN): 64 DWORDs (256B) starting at 256
pub fn set_fifo_sizes() {
    write32(GRXFIFOSIZ, (0u32 << 16) | 256);   // RX: start=0, depth=256
    write32(GTXFIFOSIZ, (256u32 << 16) | 64);  // TX0: start=256, depth=64
}

/// Sub-step: Set up event buffer
pub fn setup_event_buffer() {
    let evnt_addr = unsafe { EVENT_BUF.0.as_ptr() as u32 };
    unsafe { EVENT_BUF_ADDR = evnt_addr; }
    write32(GEVNTSIZ, 0); // disable first
    write32(GEVNTADRLO, evnt_addr);
    write32(GEVNTADRH, 0);
    write32(GEVNTSIZ, 4096);
    write32(GEVNTCOUNT, 0);
}

/// Public endpoint command (for main.rs to call with correct order)
pub fn issue_dep_cmd_public(ep: usize, cmd: u32, p0: u32, p1: u32, p2: u32) {
    issue_dep_cmd(ep, cmd, p0, p1, p2);
}

/// Sub-step: Set DCFG to High Speed
pub fn set_dcfg_hs() {
    write32(DCFG, DCFG_HIGHSPEED);
}

/// Sub-step: Configure EP0 and enable it
pub fn configure_ep0() {
    write32(DEVTEN, DEVTEN_DISCONN | DEVTEN_USBRST | DEVTEN_CONNECTDONE | DEVTEN_ULSTCHNG);

    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSETEPCONFIG,
        512u32 << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSETTRANSF, 1, 0, 0);
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSETEPCONFIG,
        512u32 << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSETTRANSF, 1, 0, 0);
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSTARTCFG, 0, 0, 0);

    write32(DALEPENA, (1 << PHY_EP0_OUT) | (1 << PHY_EP0_IN));
    queue_ep0_out_trb();
}

// Fine-grained EP0 sub-steps for crash isolation
pub fn ep0_devten() {
    write32(DEVTEN, DEVTEN_DISCONN | DEVTEN_USBRST | DEVTEN_CONNECTDONE | DEVTEN_ULSTCHNG);
}

pub fn ep0_out_setcfg() {
    // PAR0: bits[14:3]=max_packet(64), bits[2:1]=ep_type(0=control)
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSETEPCONFIG,
        64u32 << 3, 0, 0);
}

pub fn ep0_out_setxf() {
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSETTRANSF, 1, 0, 0);
}

pub fn ep0_in_setcfg() {
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSETEPCONFIG,
        64u32 << 3, 0, 0);
}

pub fn ep0_in_setxf() {
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSETTRANSF, 1, 0, 0);
}

pub fn ep0_startcfg() {
    // PAR0=0: first resource config index
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSTARTCFG, 0, 0, 0);
}

pub fn ep0_enable() {
    write32(DALEPENA, (1 << PHY_EP0_OUT) | (1 << PHY_EP0_IN));
}

pub fn ep0_queue_trb() {
    queue_ep0_out_trb();
}

/// Sub-step: Start device (RUN_STOP + clear SOFTDISCONNECT)
pub fn start_device() {
    let dctl = read32(DCTL);
    write32(DCTL, dctl | DCTL_RUN_STOP);

    unsafe {
        DEV_ADDR = 0;
        CONFIGURED = false;
        TX_PENDING = false;
    }
}

/// Check if DWC3 core is present by reading GSNPSID
pub fn is_present() -> bool {
    let id = read32(GSNPSID);
    // DWC3 SNPSID: high byte = revision, next 3 bytes = "NC3" or similar
    (id & 0xFFFF_0000) == 0x5533_0000 || // "U3" magic
    (id >> 12) != 0  // non-zero means something is there
}

/// Read GSNPSID for diagnostics
pub fn get_snpsid() -> u32 {
    read32(GSNPSID)
}

/// Read DWC3 GUSB2PHYCFG register for diagnostics
pub fn get_usb2phycfg() -> u32 { read32(GUSB2PHYCFG) }

/// Read DWC3 DSTS (device status) register for diagnostics
pub fn get_dsts() -> u32 { read32(DSTS) }

/// Read DCTL register for diagnostics
pub fn get_dctl() -> u32 { read32(DCTL) }

/// Assert D+ pull-up (gadget connect). Does not read DCTL first.
pub fn gadget_run_stop() {
    write32(DCTL, DCTL_RUN_STOP);
}

/// Write DCTL register
pub fn write_dctl(val: u32) { write32(DCTL, val) }

/// Read GCTL for diagnostics
pub fn get_gctl() -> u32 { read32(GCTL) }

/// Write GCTL (used to switch PRTCAP host/device without core reset).
pub fn set_gctl(val: u32) { write32(GCTL, val) }

/// xHCI USBCMD / USBSTS (valid after PRTCAP=HOST).
pub fn xhci_usbcmd() -> u32 { read32(0x20) }
pub fn xhci_usbsts() -> u32 { read32(0x24) }

/// Capability length (byte 0 of xHCI spec). Operational regs start here.
pub fn xhci_caplength() -> u8 {
    (read32(0x00) & 0xff) as u8
}
pub fn xhci_hcsparams1() -> u32 {
    read32(0x04)
}
pub fn xhci_read(off: usize) -> u32 {
    read32(off)
}
pub fn xhci_write(off: usize, val: u32) {
    write32(off, val)
}
/// PORTSC for 1-based port index.
pub fn xhci_portsc(op_base: usize, port: usize) -> u32 {
    read32(op_base + 0x400 + (port - 1) * 0x10)
}

/// Read DCFG for diagnostics
pub fn get_dcfg() -> u32 { read32(DCFG) }

/// Read DALEPENA for diagnostics
pub fn get_dalepena() -> u32 { read32(DALEPENA) }

/// Full DWC3 initialization with core reset
pub fn init_v2() -> bool {
    // 1. Verify DWC3 is present
    let snpsid = read32(GSNPSID);
    if snpsid == 0 || snpsid == 0xFFFF_FFFF {
        return false;
    }

    // Read pre-reset state for diagnostics
    let gctl_before = read32(GCTL);
    let dctl_before = read32(DCTL);
    let wrap_10_before = unsafe { core::ptr::read_volatile((QSCRATCH_BASE + 0x10) as *const u32) };
    let pll_at_dwc3 = phy_read32(0x1A0);

    // 2. DWC3 Core Soft Reset — must do this to clear ABL state
    let gctl = read32(GCTL);
    // Set CORESOFTRESET (bit 11)
    write32(GCTL, gctl | GCTL_CORESOFTRESET);
    // Wait for reset to take effect
    for _ in 0..100_000 { core::hint::spin_loop(); }
    // MANUALLY clear reset (DWC3 spec: software must clear this bit)
    write32(GCTL, read32(GCTL) & !GCTL_CORESOFTRESET);
    // Wait for core to come out of reset
    for _ in 0..100_000 { core::hint::spin_loop(); }

    // Verify reset worked — read GSNPSID again
    let snpsid2 = read32(GSNPSID);
    if snpsid2 == 0 || snpsid2 == 0xFFFF_FFFF {
        // Core died after reset — store diagnostics
        unsafe {
            DCTL_DIAG = [read32(DCTL), read32(DSTS), wrap_10_before, 0xDEAD0001];
            DCTL_DIAG2[0] = pll_at_dwc3;
            DCTL_DIAG2[1] = read32(GCTL);
        }
        return false;
    }

    // 3. Set device mode: GCTL PRTCAPDIR = device (bits [13:12] = 10)
    let gctl_after = read32(GCTL);
    write32(GCTL, (gctl_after & !GCTL_PRTCAPDIR_MASK) | GCTL_PRTCAP_DEVICE);

    // 4. Configure USB2 PHY interface (GUSB2PHYCFG)
    // USBTRDTIM = 9 (bits [13:10]) for HS with 8-bit UTMI
    // Keep ENBLSLPM (bit 8) like ABL had
    write32(GUSB2PHYCFG, (9u32 << 10) | (1 << 8));

    // 5. Set up event buffer
    let evnt_addr = unsafe { EVENT_BUF.0.as_ptr() as u32 };
    write32(GEVNTADRLO, evnt_addr);
    write32(GEVNTADRH, 0);
    write32(GEVNTSIZ, 4096);
    write32(GEVNTCOUNT, 0);

    // 6. Set DCFG: Full Speed (test — if host detects FS, PHY path works)
    write32(DCFG, DCFG_FULLSPEED);

    // 7. Enable device events
    write32(DEVTEN, DEVTEN_DISCONN | DEVTEN_USBRST | DEVTEN_CONNECTDONE | DEVTEN_ULSTCHNG);

    // 8. Configure EP0
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSETEPCONFIG,
        512u32 << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSETTRANSF, 1, 0, 0);
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSETEPCONFIG,
        512u32 << 3, 0, 0);
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSETTRANSF, 1, 0, 0);
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSTARTCFG, 0, 0, 0);

    // 9. Enable EP0
    write32(DALEPENA, (1 << PHY_EP0_OUT) | (1 << PHY_EP0_IN));

    // 10. Queue EP0 OUT TRB
    queue_ep0_out_trb();

    // 11. Start device: set RUN_STOP (bit 31), clear SOFTDISCONNECT (bit 7)
    write32(DCTL, DCTL_RUN_STOP); // Fresh from reset, just set RUN_STOP
    let dctl_final = read32(DCTL);
    let dsts_final = read32(DSTS);

    // Store diagnostics
    unsafe {
        DCTL_DIAG = [dctl_final, dsts_final, wrap_10_before, read32(DCFG)];
        DCTL_DIAG2[0] = pll_at_dwc3;
        DCTL_DIAG2[1] = read32(GUSB2PHYCFG);
    }

    unsafe {
        DEV_ADDR = 0;
        CONFIGURED = false;
        TX_PENDING = false;
    }

    true
}

/// Diagnostic: DCTL before/after write test
static mut DCTL_DIAG: [u32; 4] = [0; 4];
static mut DCTL_DIAG2: [u32; 4] = [0; 4];

/// Get DCTL diagnostic values
pub fn get_dctl_diag() -> ([u32; 4], [u32; 4]) {
    unsafe { (DCTL_DIAG, DCTL_DIAG2) }
}

/// Queue a TRB for EP0 OUT to receive control data
fn queue_ep0_out_trb() {
    let buf_addr = unsafe { CTRL_BUF.as_ptr() as u32 };
    unsafe {
        EP0_OUT_TRBS[0].bp = buf_addr;
        EP0_OUT_TRBS[0].bp_hi = 0;
        EP0_OUT_TRBS[0].len = 512; // max bytes
        EP0_OUT_TRBS[0].ctrl = TRB_CTRL_HWO | TRB_CTRL_LST | TRB_CTRL_CSP | TRB_CTRL_ISP | TRB_CTRL_IOC | TRB_CTRL_TRBTYPE_CONTROL_SETUP;
    }
    let trb_ptr = unsafe { &EP0_OUT_TRBS[0] as *const Trb as u32 };
    // STARTTRANSFER: PAR0=upper32(TRB addr), PAR1=lower32(TRB addr)
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSTARTTRANSFER, 0, trb_ptr, 0);
}

// ── Event Handling ─────────────────────────────────────────────────

/// Event types
const EVENT_DEVICE: u32 = 0;
const EVENT_EP_OUT: u32 = 1;
const EVENT_EP_IN: u32 = 2;

/// Device event types
const DEV_EVENT_DISCONN: u32 = 0;
const DEV_EVENT_RESET: u32   = 1;
const DEV_EVENT_CONNECT_DONE: u32 = 2;
const DEV_EVENT_LINK_CHANGE: u32 = 3;

/// USB event counter for diagnostics
static mut USB_EVENT_COUNT: u32 = 0;
static mut USB_SETUP_COUNT: u32 = 0;

pub fn get_event_count() -> u32 { unsafe { USB_EVENT_COUNT } }
pub fn get_setup_count() -> u32 { unsafe { USB_SETUP_COUNT } }
pub fn reset_diag_counts() {
    unsafe {
        USB_EVENT_COUNT = 0;
        USB_SETUP_COUNT = 0;
    }
}

/// Poll events and handle them
pub fn poll() {
    let count = read32(GEVNTCOUNT) & 0xFFFF; // lower 16 bits = byte count
    if count == 0 {
        return;
    }

    unsafe { USB_EVENT_COUNT += 1; }

    let count_words = count as usize / 4;
    let max_words = count_words.min(64);

    // Read events from ABL's DMA buffer (GEVNTADRLO), not our EVENT_BUF.
    // We can't change GEVNTADRLO, so DWC3 DMA writes to ABL's address.
    // ABL's page table still maps this memory.
    let evbuf_ptr = unsafe {
        let addr = ABL_EVBUF as usize;
        if addr == 0 {
            // Fallback: use our buffer (shouldn't happen after init)
            EVENT_BUF.0.as_ptr() as usize
        } else {
            // Invalidate cache lines for the event data range
            for off in (0..max_words * 4).step_by(64) {
                core::arch::asm!("dc ivac, {}", in(reg) addr + off);
            }
            core::arch::asm!("dsb ish");
            core::arch::asm!("isb");
            addr
        }
    };
    let events = unsafe { core::slice::from_raw_parts(evbuf_ptr as *const u32, max_words) };

    for i in (0..events.len()).step_by(4) {
        if i + 3 >= events.len() { break; }
        let evt0 = events[i];
        let _evt1 = events[i + 1];
        let _evt2 = events[i + 2];
        let _evt3 = events[i + 3];

        let evt_type = (evt0 >> 30) & 0x3;

        match evt_type {
            EVENT_DEVICE => {
                let dev_evt = (evt0 >> 8) & 0xF;
                match dev_evt {
                    DEV_EVENT_RESET => handle_reset(),
                    DEV_EVENT_CONNECT_DONE => handle_connect_done(),
                    DEV_EVENT_DISCONN => handle_disconnect(),
                    _ => {}
                }
            }
            EVENT_EP_OUT | EVENT_EP_IN => {
                let ep_num = (evt0 >> 16) & 0x1F;
                let status = (evt0 >> 12) & 0xF;
                let actual_len = events[i + 2] & 0xFFFFFF;
                handle_ep_event(ep_num as usize, status as usize, actual_len as usize);
            }
            _ => {}
        }
    }

    // Clear event count
    write32(GEVNTCOUNT, count);
}

fn handle_reset() {
    unsafe {
        DEV_ADDR = 0;
        CONFIGURED = false;
        TX_PENDING = false;
    }
    // Reset device address
    let dcfg = read32(DCFG) & !DCFG_DEVADDR_MASK;
    write32(DCFG, dcfg);
    // Re-queue EP0 OUT TRB
    queue_ep0_out_trb();
}

fn handle_connect_done() {
    // Connection established — we're ready for enumeration
    // EP0 OUT TRB should already be queued
}

fn handle_disconnect() {
    unsafe {
        DEV_ADDR = 0;
        CONFIGURED = false;
        TX_PENDING = false;
    }
}

fn handle_ep_event(ep_num: usize, _status: usize, _actual_len: usize) {
    match ep_num {
        0 => {
            // EP0 OUT — SETUP packet received
            handle_setup();
        }
        PHY_EP2_OUT => {
            // Bulk OUT — data from host
            // Re-queue TRB
            queue_bulk_out_trb();
        }
        PHY_EP3_IN => {
            // Bulk IN — transfer complete
            unsafe { TX_PENDING = false; }
        }
        _ => {}
    }
}

// ── USB SETUP Handling ─────────────────────────────────────────────

#[repr(C)]
struct SetupPacket {
    bmRequestType: u8,
    bRequest: u8,
    wValue: u16,
    wIndex: u16,
    wLength: u16,
}

fn handle_setup() {
    unsafe { USB_SETUP_COUNT += 1; }
    let setup = unsafe { &*(CTRL_BUF.as_ptr() as *const SetupPacket) };

    let req_type = setup.bmRequestType;
    let request = setup.bRequest;
    let value = setup.wValue;
    let index = setup.wIndex;
    let length = setup.wLength;

    let direction = req_type & 0x80 != 0; // true = device-to-host

    match (req_type & 0x60, request) {
        // Standard requests
        (0x00, 0x05) => {
            // SET_ADDRESS
            let addr = (value & 0x7F) as u8;
            let dcfg = read32(DCFG) & !DCFG_DEVADDR_MASK;
            write32(DCFG, dcfg | ((addr as u32) << DCFG_DEVADDR_SHIFT));
            unsafe { DEV_ADDR = addr; }
            send_ep0_status();
        }
        (0x00, 0x09) => {
            // SET_CONFIGURATION
            unsafe { CONFIGURED = value == 1; }
            if value == 1 {
                configure_endpoints();
            }
            send_ep0_status();
        }
        (0x80, 0x00) => {
            // GET_STATUS
            let status: [u8; 2] = [0x01, 0x00]; // self-powered
            send_ep0_data(&status, length as usize);
        }
        (0x80, 0x06) => {
            // GET_DESCRIPTOR
            let desc_type = (value >> 8) as u8;
            let _desc_index = (value & 0xFF) as u8;
            match desc_type {
                0x01 => send_ep0_data(&DEVICE_DESCRIPTOR, length as usize),
                0x02 => send_ep0_data(&CONFIG_DESCRIPTOR, length as usize),
                0x03 => send_ep0_string(index as usize, length as usize),
                _ => send_ep0_stall(),
            }
        }
        // CDC ACM requests
        (0x20, 0x20) => {
            // SET_LINE_CODING — accept and ACK
            send_ep0_status();
        }
        (0x20, 0x22) => {
            // SET_CONTROL_LINE_STATE — accept and ACK
            send_ep0_status();
        }
        (0x21, 0x22) => {
            // SET_CONTROL_LINE_STATE (class request)
            send_ep0_status();
        }
        (0xA1, 0x21) => {
            // GET_LINE_CODING — return default 115200 8N1
            let line_coding: [u8; 7] = [
                0x00, 0xC2, 0x01, 0x00, // dwDTERate = 115200
                0x00,                   // bCharFormat = 1 stop bit
                0x00,                   // bParityType = none
                0x08,                   // bDataBits = 8
            ];
            send_ep0_data(&line_coding, length as usize);
        }
        _ => {
            send_ep0_stall();
        }
    }
}

fn send_ep0_data(data: &[u8], requested_len: usize) {
    let len = data.len().min(requested_len);
    // Copy data to control buffer
    let buf = unsafe { &mut CTRL_BUF };
    buf[..len].copy_from_slice(&data[..len]);

    let buf_addr = unsafe { buf.as_ptr() as u32 };
    unsafe {
        EP0_OUT_TRBS[0].bp = buf_addr;
        EP0_OUT_TRBS[0].bp_hi = 0;
        EP0_OUT_TRBS[0].len = len as u32;
        EP0_OUT_TRBS[0].ctrl = TRB_CTRL_HWO | TRB_CTRL_LST | TRB_CTRL_IOC | TRB_CTRL_TRBTYPE_CONTROL_DATA;
    }
    let trb_ptr = unsafe { &EP0_OUT_TRBS[0] as *const Trb as u32 };
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSTARTTRANSFER, 0, trb_ptr, 0);
}

fn send_ep0_status() {
    // Status stage: zero-length IN packet
    let buf_addr = unsafe { CTRL_BUF.as_ptr() as u32 };
    unsafe {
        EP0_OUT_TRBS[0].bp = buf_addr;
        EP0_OUT_TRBS[0].bp_hi = 0;
        EP0_OUT_TRBS[0].len = 0;
        EP0_OUT_TRBS[0].ctrl = TRB_CTRL_HWO | TRB_CTRL_LST | TRB_CTRL_IOC | TRB_CTRL_TRBTYPE_CONTROL_STATUS2;
    }
    let trb_ptr = unsafe { &EP0_OUT_TRBS[0] as *const Trb as u32 };
    issue_dep_cmd(PHY_EP0_IN, DEPCMD_DEPSTARTTRANSFER, 0, trb_ptr, 0);
}

fn send_ep0_stall() {
    // Stall EP0 — write DGCMD to stall
    write32(DGCMDPAR, 0); // EP0
    write32(DGCMD, 0x05 | (1 << 10)); // SET_ENDPOINT_STALL + CMDACT
    // Also stall IN direction
    write32(DGCMDPAR, 1); // EP0 IN
    write32(DGCMD, 0x05 | (1 << 10));
}

fn send_ep0_string(index: usize, requested_len: usize) {
    let s = match index {
        0 => make_string_descriptor(&STRING0[..4], true),
        1 => make_string_descriptor(STRING1, false),
        2 => make_string_descriptor(STRING2, false),
        3 => make_string_descriptor(STRING3, false),
        _ => {
            send_ep0_stall();
            return;
        }
    };
    send_ep0_data(&s, requested_len);
}

/// Convert ASCII string to USB string descriptor
fn make_string_descriptor(s: &[u8], raw: bool) -> [u8; 64] {
    let mut desc = [0u8; 64];
    if raw {
        // Already a descriptor (string 0)
        desc[0] = s.len() as u8;
        desc[1] = 0x03;
        desc[..s.len()].copy_from_slice(s);
        desc[0] = s.len() as u8;
        return desc;
    }
    // ASCII to UTF-16LE
    let num_chars = s.len() - 1; // exclude NUL
    let total_len = 2 + num_chars * 2;
    desc[0] = total_len as u8;
    desc[1] = 0x03;
    for i in 0..num_chars {
        desc[2 + i * 2] = s[i];
        desc[3 + i * 2] = 0;
    }
    desc
}

// ── Endpoint Configuration ─────────────────────────────────────────

fn configure_endpoints() {
    // Start new configuration (resource index 2 for config 1)
    issue_dep_cmd(PHY_EP0_OUT, DEPCMD_DEPSTARTCFG, 2, 0, 0);

    // EP1 IN (phys 3): Interrupt IN for CDC notification
    // PAR0: bits[14:3]=max_packet(64), bits[2:1]=ep_type(3=interrupt)
    issue_dep_cmd(PHY_EP1_IN, DEPCMD_DEPSETEPCONFIG,
        (64u32 << 3) | (3u32 << 1), 0, 0);
    issue_dep_cmd(PHY_EP1_IN, DEPCMD_DEPSETTRANSF, 1, 0, 0);

    // EP2 OUT (phys 2): Bulk OUT for data from host
    issue_dep_cmd(PHY_EP2_OUT, DEPCMD_DEPSETEPCONFIG,
        (512u32 << 3) | (2u32 << 1), 0, 0);
    issue_dep_cmd(PHY_EP2_OUT, DEPCMD_DEPSETTRANSF, 1, 0, 0);

    // EP3 IN (phys 5): Bulk IN for data to host
    issue_dep_cmd(PHY_EP3_IN, DEPCMD_DEPSETEPCONFIG,
        (512u32 << 3) | (2u32 << 1), 0, 0);
    issue_dep_cmd(PHY_EP3_IN, DEPCMD_DEPSETTRANSF, 1, 0, 0);

    // Enable all endpoints
    write32(DALEPENA,
        (1 << PHY_EP0_OUT) | (1 << PHY_EP0_IN) |
        (1 << PHY_EP1_IN) |
        (1 << PHY_EP2_OUT) |
        (1 << PHY_EP3_IN));

    // Queue bulk OUT TRB to receive data
    queue_bulk_out_trb();
}

fn queue_bulk_out_trb() {
    let buf_addr = unsafe { BULK_OUT_BUF.as_ptr() as u32 };
    unsafe {
        EP2_OUT_TRBS[0].bp = buf_addr;
        EP2_OUT_TRBS[0].bp_hi = 0;
        EP2_OUT_TRBS[0].len = 512;
        EP2_OUT_TRBS[0].ctrl = TRB_CTRL_HWO | TRB_CTRL_LST | TRB_CTRL_ISP | TRB_CTRL_IOC | TRB_CTRL_TRBTYPE_NORMAL;
    }
    let trb_ptr = unsafe { &EP2_OUT_TRBS[0] as *const Trb as u32 };
    issue_dep_cmd(PHY_EP2_OUT, DEPCMD_DEPSTARTTRANSFER, 0, trb_ptr, 0);
}

// ── Data Transfer API ──────────────────────────────────────────────

/// Send data to host via bulk IN (EP3 IN)
/// Returns false if a transfer is already pending
pub fn send(data: &[u8]) -> bool {
    if unsafe { TX_PENDING } {
        return false;
    }
    if !unsafe { CONFIGURED } {
        return false;
    }

    let len = data.len().min(512);
    let buf = unsafe { &mut BULK_IN_BUF };
    buf[..len].copy_from_slice(&data[..len]);

    let buf_addr = buf.as_ptr() as u32;
    unsafe {
        EP3_IN_TRBS[0].bp = buf_addr;
        EP3_IN_TRBS[0].bp_hi = 0;
        EP3_IN_TRBS[0].len = len as u32;
        EP3_IN_TRBS[0].ctrl = TRB_CTRL_HWO | TRB_CTRL_LST | TRB_CTRL_IOC | TRB_CTRL_TRBTYPE_NORMAL;
        TX_PENDING = true;
        TX_LEN = len;
    }

    let trb_ptr = unsafe { &EP3_IN_TRBS[0] as *const Trb as u32 };
    issue_dep_cmd(PHY_EP3_IN, DEPCMD_DEPSTARTTRANSFER, 0, trb_ptr, 0);
    true
}

/// Check if TX is complete (ready to send more)
pub fn tx_ready() -> bool {
    !unsafe { TX_PENDING } && unsafe { CONFIGURED }
}

/// Read received data from host (bulk OUT)
/// Returns number of bytes read, 0 if none available
pub fn recv(_buf: &mut [u8]) -> usize {
    // TODO: implement with event-based tracking
    0
}

/// Check if USB serial is connected and configured
pub fn is_serial_ready() -> bool {
    unsafe { CONFIGURED }
}
