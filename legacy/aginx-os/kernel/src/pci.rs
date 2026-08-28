//! PCI bus enumeration via ECAM (Enhanced Configuration Access Mechanism)
//!
//! QEMU virt machine with `highmem-ecam=off` places ECAM at 0x3F000000.
//! Each function gets a 4KB config space page:
//!   addr = ECAM_BASE + (bus << 20) | (dev << 15) | (func << 12) | offset

// Conditional UART imports
#[cfg(not(feature = "board-redfin"))]
use crate::uart;
#[cfg(feature = "board-redfin")]
use crate::qup_uart as uart;

const ECAM_BASE: usize = 0x3F00_0000;

/// PCI config space vendor/device IDs
const VIRTIO_VENDOR: u16 = 0x1AF4;
const VIRTIO_NET_TRANSITIONAL: u16 = 0x1000;  // Legacy virtio-net
const VIRTIO_NET_MODERN: u16 = 0x1041;      // Modern virtio-net
const VIRTIO_BLK_TRANSITIONAL: u16 = 0x1001;  // Legacy virtio-blk
const VIRTIO_BLK_MODERN: u16 = 0x1042;      // Modern virtio-blk

// QEMU xHCI controller
const XHCI_VENDOR: u16 = 0x1B36;  // Red Hat
const XHCI_DEVICE: u16 = 0x000D;  // QEMU XHCI

/// Discovered PCI device info
#[allow(dead_code)]
pub struct PciDeviceInfo {
    pub bus: u8,
    pub dev: u8,
    pub func: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub irq_line: u8,
}

/// The virtio-net device found during enumeration
static mut VIRTIO_NET: Option<PciDeviceInfo> = None;

/// The virtio-blk device found during enumeration
static mut VIRTIO_BLK: Option<PciDeviceInfo> = None;

/// The xHCI USB controller found during enumeration
static mut XHCI: Option<PciDeviceInfo> = None;

pub fn config_read32(bus: u8, dev: u8, func: u8, offset: u8) -> u32 {
    let addr = ECAM_BASE
        | ((bus as usize) << 20)
        | ((dev as usize) << 15)
        | ((func as usize) << 12)
        | ((offset as usize) & 0xFC);
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

fn config_read16(bus: u8, dev: u8, func: u8, offset: u8) -> u16 {
    let val = config_read32(bus, dev, func, offset & 0xFC);
    ((val >> ((offset as u32 & 0x2) * 8)) & 0xFFFF) as u16
}

fn config_read8(bus: u8, dev: u8, func: u8, offset: u8) -> u8 {
    let val = config_read32(bus, dev, func, offset & 0xFC);
    ((val >> ((offset as u32 & 0x3) * 8)) & 0xFF) as u8
}

pub fn config_write32(bus: u8, dev: u8, func: u8, offset: u8, val: u32) {
    let addr = ECAM_BASE
        | ((bus as usize) << 20)
        | ((dev as usize) << 15)
        | ((func as usize) << 12)
        | ((offset as usize) & 0xFC);
    unsafe { core::ptr::write_volatile(addr as *mut u32, val) }
}

/// Enable a PCI device: set Bus Master, Memory Space, I/O Space bits
fn enable_device(info: &PciDeviceInfo) {
    // Read current command register (offset 0x04)
    let cmd = config_read16(info.bus, info.dev, info.func, 0x04);
    // Set Bus Master (bit 2), Memory Space (bit 1), I/O Space (bit 0)
    let new_cmd = cmd | 0x07;
    config_write32(info.bus, info.dev, info.func, 0x04, new_cmd as u32);
}

/// Read a BAR (Base Address Register) value
pub fn read_bar(info: &PciDeviceInfo, bar_index: u8) -> u64 {
    let offset = 0x10 + bar_index * 4;
    let val = config_read32(info.bus, info.dev, info.func, offset);

    if val & 0x1 != 0 {
        // I/O space BAR
        (val & 0xFFFFFFFC) as u64
    } else {
        // Memory space BAR
        let bar_type = (val >> 1) & 0x3;
        let base = (val & 0xFFFFFFF0) as u64;
        if bar_type == 0x2 {
            // 64-bit BAR: read next BAR for high bits
            let high = config_read32(info.bus, info.dev, info.func, offset + 4);
            base | ((high as u64) << 32)
        } else {
            base
        }
    }
}

/// Enumerate PCI bus and find devices
pub fn init(uart: usize) {
    let mut found_count = 0;

    for dev in 0..32u8 {
        let vdid = config_read32(0, dev, 0, 0x00);
        let vendor = vdid as u16;
        let device_id = (vdid >> 16) as u16;

        if vendor == 0xFFFF {
            continue;
        }

        let class_code = config_read8(0, dev, 0, 0x09);
        let subclass = config_read8(0, dev, 0, 0x0A);
        let irq_line = config_read8(0, dev, 0, 0x3C);
        let header_type = config_read8(0, dev, 0, 0x0E);

        found_count += 1;
        uart::puts(uart, "  PCI 00:");
        crate::print_hex(uart, (dev as u32) << 16);
        uart::puts(uart, ".0 vendor=0x");
        crate::print_hex(uart, vendor as u32);
        uart::puts(uart, " device=0x");
        crate::print_hex(uart, device_id as u32);
        uart::puts(uart, " class=0x");
        crate::print_hex(uart, class_code as u32);
        uart::puts(uart, "\r\n");

        // Check for virtio-net (both transitional and modern)
        if vendor == VIRTIO_VENDOR
            && (device_id == VIRTIO_NET_TRANSITIONAL || device_id == VIRTIO_NET_MODERN)
        {
            let info = PciDeviceInfo {
                bus: 0,
                dev,
                func: 0,
                vendor_id: vendor,
                device_id: device_id,
                class_code,
                subclass,
                irq_line,
            };
            enable_device(&info);
            unsafe { VIRTIO_NET = Some(info) };
            uart::puts(uart, "  -> virtio-net found!\r\n");
        }

        // Check for virtio-blk (both transitional and modern)
        if vendor == VIRTIO_VENDOR
            && (device_id == VIRTIO_BLK_TRANSITIONAL || device_id == VIRTIO_BLK_MODERN)
        {
            let info = PciDeviceInfo {
                bus: 0,
                dev,
                func: 0,
                vendor_id: vendor,
                device_id: device_id,
                class_code,
                subclass,
                irq_line,
            };
            enable_device(&info);
            unsafe { VIRTIO_BLK = Some(info) };
            uart::puts(uart, "  -> virtio-blk found!\r\n");
        }

        // Check for xHCI USB controller
        if vendor == XHCI_VENDOR && device_id == XHCI_DEVICE {
            let info = PciDeviceInfo {
                bus: 0,
                dev,
                func: 0,
                vendor_id: vendor,
                device_id: device_id,
                class_code,
                subclass,
                irq_line,
            };
            enable_device(&info);
            unsafe { XHCI = Some(info) };
            uart::puts(uart, "  -> xHCI USB found!\r\n");
        }

        // Check multi-function bit
        if header_type & 0x80 != 0 {
            for func in 1..8u8 {
                let vdid_f = config_read32(0, dev, func, 0x00);
                if vdid_f as u16 == 0xFFFF {
                    continue;
                }
                uart::puts(uart, "  PCI 00:");
                crate::print_hex(uart, ((dev as u32) << 16) | (func as u32) << 8);
                uart::puts(uart, " vendor=0x");
                crate::print_hex(uart, (vdid_f & 0xFFFF) as u32);
                uart::puts(uart, "\r\n");
            }
        }
    }

    uart::puts(uart, "[OK] PCI: ");
    uart::putc(uart, b'0' + found_count as u8);
    uart::puts(uart, " devices\r\n");
}

/// Get the discovered virtio-net device info
pub fn get_virtio_net() -> Option<&'static PciDeviceInfo> {
    unsafe { VIRTIO_NET.as_ref() }
}

/// Get the discovered virtio-blk device info
pub fn get_virtio_blk() -> Option<&'static PciDeviceInfo> {
    unsafe { VIRTIO_BLK.as_ref() }
}

/// Get the discovered xHCI USB controller info
pub fn get_xhci() -> Option<&'static PciDeviceInfo> {
    unsafe { XHCI.as_ref() }
}
