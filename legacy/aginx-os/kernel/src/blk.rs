//! VirtIO-Block device driver
//!
//! Hand-written VirtIO 1.0 PCI transport, following the same pattern as net.rs.
//! Uses a single virtqueue (queue 0) with 3-descriptor chains for I/O.

use core::alloc::Layout;
use core::sync::atomic::{AtomicU16, AtomicU32, AtomicU64, Ordering};

const ECAM_BASE: usize = 0x3F00_0000;

#[cfg(not(feature = "board-redfin"))]
use crate::uart;
#[cfg(feature = "board-redfin")]
use crate::qup_uart as uart;

use crate::platform::UART;

const SECTOR_SIZE: usize = 512;
const QUEUE_SIZE: u16 = 64;

// VirtIO status bits
const VIRTIO_STATUS_ACKNOWLEDGE: u8 = 1;
const VIRTIO_STATUS_DRIVER: u8 = 2;
const VIRTIO_STATUS_DRIVER_OK: u8 = 4;
const VIRTIO_STATUS_FEATURES_OK: u8 = 8;

const DESC_F_NEXT: u16 = 1;
const DESC_F_WRITE: u16 = 2;

// VirtIO PCI capability types
const VIRTIO_PCI_CAP_VENDOR: u8 = 0x09;
const VIRTIO_PCI_CAP_COMMON_CFG: u8 = 1;
const VIRTIO_PCI_CAP_NOTIFY_CFG: u8 = 2;
const VIRTIO_PCI_CAP_DEVICE_CFG: u8 = 4;

// Block request types
const BLK_T_IN: u32 = 0;   // Read
const BLK_T_OUT: u32 = 1;  // Write
#[allow(dead_code)]
const BLK_T_FLUSH: u32 = 4;

// Block response status
const BLK_S_OK: u8 = 0;

/// VirtIO Descriptor
#[repr(C, align(16))]
struct Descriptor {
    addr: AtomicU64,
    len: AtomicU32,
    flags: AtomicU16,
    next: AtomicU16,
}

impl Descriptor {
    fn set(&self, addr: u64, len: u32, flags: u16, next: u16) {
        self.addr.store(addr, Ordering::SeqCst);
        self.len.store(len, Ordering::SeqCst);
        self.flags.store(flags, Ordering::SeqCst);
        self.next.store(next, Ordering::SeqCst);
    }
}

/// Available Ring
#[repr(C)]
struct AvailableRing {
    flags: AtomicU16,
    idx: AtomicU16,
}

/// Used Ring
#[repr(C)]
struct UsedRing {
    flags: AtomicU16,
    idx: AtomicU16,
}

/// Used Element
#[allow(dead_code)]
#[repr(C)]
struct UsedElement {
    id: AtomicU32,
    len: AtomicU32,
}

/// Common Configuration
#[repr(C)]
struct CommonCfg {
    pub device_feature_select: u32,
    pub device_feature: u32,
    pub driver_feature_select: u32,
    pub driver_feature: u32,
    pub config_msix_vector: u16,
    pub num_queues: u16,
    pub device_status: u8,
    pub config_generation: u8,
    pub queue_select: u16,
    pub queue_size: u16,
    pub queue_msix_vector: u16,
    pub queue_enable: u16,
    pub queue_notify_off: u16,
    pub queue_desc: u64,
    pub queue_driver: u64,
    pub queue_device: u64,
}

/// Block request header
#[allow(dead_code)]
#[repr(C)]
struct BlkReq {
    type_: u32,
    reserved: u32,
    sector: u64,
}

/// Block response
#[repr(C)]
struct BlkResp {
    status: u8,
}

/// Block device state
#[allow(dead_code)]
struct BlkState {
    common: *mut CommonCfg,
    notify_base: *mut u8,
    notify_multiplier: u32,
    #[allow(dead_code)]
    device_cfg: usize,
    desc_table: *mut Descriptor,
    avail_ring: *mut AvailableRing,
    used_ring: *mut UsedRing,
    queue_size: u16,
    last_used_idx: u16,
    capacity_sectors: u64,
    pci_bus: u8,
    pci_dev: u8,
    pci_func: u8,
}

static mut BLK_STATE: Option<BlkState> = None;

/// Static DMA buffers for block requests (avoid dynamic alloc/free issues)
#[repr(C, align(4096))]
struct BlkDmaBufs {
    req: BlkReq,
    resp: BlkResp,
}

static mut BLK_DMA_BUFS: BlkDmaBufs = BlkDmaBufs {
    req: BlkReq { type_: 0, reserved: 0, sector: 0 },
    resp: BlkResp { status: 0 },
};

// PCI ECAM access (same as net.rs)
fn config_read32(bus: u8, dev: u8, func: u8, offset: u8) -> u32 {
    let addr = ECAM_BASE | ((bus as usize) << 20) | ((dev as usize) << 15) | ((func as usize) << 12) | ((offset as usize) & 0xFC);
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

fn config_write32(bus: u8, dev: u8, func: u8, offset: u8, val: u32) {
    let addr = ECAM_BASE | ((bus as usize) << 20) | ((dev as usize) << 15) | ((func as usize) << 12) | ((offset as usize) & 0xFC);
    unsafe { core::ptr::write_volatile(addr as *mut u32, val) }
}

/// Allocate DMA memory (identity-mapped)
fn dma_alloc(size: usize) -> Option<usize> {
    let page_size = 4096;
    let aligned_size = ((size + page_size - 1) / page_size) * page_size;
    let layout = Layout::from_size_align(aligned_size, page_size).ok()?;
    unsafe {
        let ptr = alloc::alloc::alloc_zeroed(layout);
        if ptr.is_null() { None } else { Some(ptr as usize) }
    }
}

#[allow(dead_code)]
fn dma_free(addr: usize, size: usize) {
    let page_size = 4096;
    let aligned_size = ((size + page_size - 1) / page_size) * page_size;
    let layout = Layout::from_size_align(aligned_size, page_size).unwrap();
    unsafe { alloc::alloc::dealloc(addr as *mut u8, layout); }
}

/// Capability info
struct VirtioCaps {
    common: Option<usize>,
    notify: Option<(usize, u32)>,
    device: Option<usize>,
}

/// Scan VirtIO capabilities (same logic as net.rs)
fn scan_virtio_caps(bus: u8, dev: u8, func: u8) -> VirtioCaps {
    let mut caps = VirtioCaps { common: None, notify: None, device: None };
    let cap_ptr = config_read32(bus, dev, func, 0x34) as u8;
    if cap_ptr == 0 { return caps; }
    let mut ptr = cap_ptr;
    let mut iterations = 0;
    while ptr != 0 && iterations < 48 {
        iterations += 1;
        let cap_hdr = config_read32(bus, dev, func, ptr);
        let cap_id = cap_hdr as u8;
        let next_ptr = ((cap_hdr >> 8) & 0xFF) as u8;
        let cap_len = ((cap_hdr >> 16) & 0xFF) as u8;
        if cap_id == VIRTIO_PCI_CAP_VENDOR && cap_len >= 16 {
            let cfg_type = (cap_hdr >> 24) as u8;
            let bar = config_read32(bus, dev, func, ptr + 4) as u8;
            let offset = config_read32(bus, dev, func, ptr + 8);
            let _length = config_read32(bus, dev, func, ptr + 12);
            let notify_mult = config_read32(bus, dev, func, ptr + 16);
            let bar_val = config_read32(bus, dev, func, 0x10 + bar as u8 * 4);
            let bar_base = (bar_val & 0xFFFF_FFF0) as usize;
            let addr = bar_base + offset as usize;
            match cfg_type {
                VIRTIO_PCI_CAP_COMMON_CFG => caps.common = Some(addr),
                VIRTIO_PCI_CAP_NOTIFY_CFG => caps.notify = Some((addr, notify_mult)),
                VIRTIO_PCI_CAP_DEVICE_CFG => caps.device = Some(addr),
                _ => {}
            }
        }
        if ptr == next_ptr { break; }
        ptr = next_ptr;
    }
    caps
}

/// Notify the device for a queue
unsafe fn notify_queue(state: &BlkState, queue_idx: u16) {
    // Ensure all DMA writes (descriptors, available ring) are visible to device
    core::arch::asm!("dsb sy", "isb");
    let common = state.common;
    core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).queue_select), queue_idx);
    let notify_off = core::ptr::read_volatile(core::ptr::addr_of!((*common).queue_notify_off));
    let offset = notify_off as usize * state.notify_multiplier as usize;
    let notify_addr = state.notify_base.add(offset);
    core::ptr::write_volatile(notify_addr as *mut u16, queue_idx);
    // Ensure notification write completes before polling
    core::arch::asm!("dsb sy", "isb");
}

/// Initialize the block device driver
pub fn init(uart_base: usize) {
    let info = match crate::pci::get_virtio_blk() {
        Some(i) => i,
        None => {
            uart::puts(uart_base, "[SKIP] Block: no virtio-blk device\r\n");
            return;
        }
    };

    // Allocate BARs (start at 0x1100_0000 to avoid conflict with net)
    let mut next_bar_addr: usize = 0x1100_0000;
    for bar_idx in 0..6u8 {
        let offset = 0x10 + bar_idx * 4;
        let bar = config_read32(info.bus, info.dev, info.func, offset);
        if bar == 0 { continue; }
        if bar & 0x1 != 0 { continue; }
        let bar_type = (bar >> 1) & 0x3;
        config_write32(info.bus, info.dev, info.func, offset, 0xFFFF_FFFF);
        let size_val = config_read32(info.bus, info.dev, info.func, offset);
        let size = if size_val == 0 { 0 } else { !((size_val & 0xFFFF_FFF0) as usize) + 1 };
        if size == 0 {
            config_write32(info.bus, info.dev, info.func, offset, bar);
            continue;
        }
        let aligned_addr = (next_bar_addr + size - 1) & !(size - 1);
        config_write32(info.bus, info.dev, info.func, offset, (aligned_addr as u32) | (bar & 0xF));
        if bar_type == 0x2 {
            config_write32(info.bus, info.dev, info.func, offset + 4, (aligned_addr >> 32) as u32);
            break;
        }
        next_bar_addr = aligned_addr + size;
    }

    // Enable device
    let cmd = config_read32(info.bus, info.dev, info.func, 0x04);
    config_write32(info.bus, info.dev, info.func, 0x04, cmd | 0x07);

    // Scan capabilities
    let caps = scan_virtio_caps(info.bus, info.dev, info.func);
    let common_addr = match caps.common {
        Some(a) => a,
        None => { uart::puts(uart_base, "[FAIL] Block: no common config\r\n"); return; }
    };
    let device_addr = match caps.device {
        Some(a) => a,
        None => { uart::puts(uart_base, "[FAIL] Block: no device config\r\n"); return; }
    };
    let (notify_addr, notify_multiplier) = match caps.notify {
        Some((a, m)) => (a, m),
        None => { uart::puts(uart_base, "[FAIL] Block: no notify config\r\n"); return; }
    };

    let common = common_addr as *mut CommonCfg;

    // Reset and initialize — all writes via write_volatile for MMIO ordering
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).device_status), 0);
    }
    for _ in 0..10000 { core::hint::spin_loop(); }
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).device_status), VIRTIO_STATUS_ACKNOWLEDGE);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).device_status), VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER);
    }

    // Feature negotiation
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).device_feature_select), 0);
    }
    let _features_low = unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*common).device_feature)) };
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).device_feature_select), 1);
    }
    let features_high = unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*common).device_feature)) };
    // Negotiate: only VIRTIO_F_VERSION_1 (bit 32)
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).driver_feature_select), 0);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).driver_feature), 0);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).driver_feature_select), 1);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).driver_feature), if (features_high & 1) != 0 { 1 } else { 0 });
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).device_status), VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_FEATURES_OK);
    }

    let status_check = unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*common).device_status)) };
    if (status_check & VIRTIO_STATUS_FEATURES_OK) == 0 {
        uart::puts(uart_base, "[FAIL] Block: feature negotiation failed\r\n");
        return;
    }

    // Read capacity from device config (two u32: capacity_low, capacity_high)
    let capacity_low = unsafe { core::ptr::read_volatile(device_addr as *const u32) };
    let capacity_high = unsafe { core::ptr::read_volatile((device_addr + 4) as *const u32) };
    let capacity_sectors = (capacity_low as u64) | ((capacity_high as u64) << 32);

    // Setup queue 0 — match net.rs pattern: use device's native queue_size
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).queue_select), 0);
    }
    let queue_size = unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*common).queue_size)) };
    if queue_size == 0 {
        uart::puts(uart_base, "[FAIL] Block: queue size is 0\r\n");
        return;
    }
    // Use device's native queue_size (like net.rs), don't write it back
    let actual_size = queue_size.min(QUEUE_SIZE as u16) as usize;

    let desc_size = core::mem::size_of::<Descriptor>() * actual_size;
    let avail_size = 6 + 2 * actual_size;
    let used_size = 6 + 8 * actual_size;

    let desc_addr = match dma_alloc((desc_size + 4095) / 4096 * 4096) {
        Some(a) => a,
        None => { uart::puts(uart_base, "[FAIL] Block: desc alloc\r\n"); return; }
    };
    let avail_addr = match dma_alloc((avail_size + 4095) / 4096 * 4096) {
        Some(a) => a,
        None => { uart::puts(uart_base, "[FAIL] Block: avail alloc\r\n"); return; }
    };
    let used_addr = match dma_alloc((used_size + 4095) / 4096 * 4096) {
        Some(a) => a,
        None => { uart::puts(uart_base, "[FAIL] Block: used alloc\r\n"); return; }
    };

    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).queue_desc), desc_addr as u64);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).queue_driver), avail_addr as u64);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).queue_device), used_addr as u64);
        // Write queue_size back — required by VirtIO 1.0 spec; QEMU zeros num on reset
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).queue_size), actual_size as u16);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).queue_enable), 1);
        // Memory barrier before DRIVER_OK
        core::arch::asm!("dsb sy", "isb");
        // Set DRIVER_OK
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).device_status), VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_FEATURES_OK | VIRTIO_STATUS_DRIVER_OK);
    }

    let state = BlkState {
        common: common as *mut CommonCfg,
        notify_base: notify_addr as *mut u8,
        notify_multiplier,
        device_cfg: device_addr,
        desc_table: desc_addr as *mut Descriptor,
        avail_ring: avail_addr as *mut AvailableRing,
        used_ring: used_addr as *mut UsedRing,
        queue_size: actual_size as u16,
        last_used_idx: 0,
        capacity_sectors,
        pci_bus: info.bus,
        pci_dev: info.dev,
        pci_func: info.func,
    };

    unsafe { BLK_STATE = Some(state); }

    uart::puts(uart_base, "[OK] Block initialized\r\n");
}

/// Get device capacity in sectors
pub fn capacity() -> u64 {
    unsafe { BLK_STATE.as_ref().map(|s| s.capacity_sectors).unwrap_or(0) }
}

/// Submit a block request and wait for completion
/// Returns true on success
fn blk_request(req_type: u32, sector: u64, data_buf: *mut u8, data_len: u32) -> bool {
    unsafe {
        let state = match BLK_STATE.as_mut() {
            Some(s) => s,
            None => return false,
        };

        let dma = &raw mut BLK_DMA_BUFS;
        let req_addr = core::ptr::addr_of_mut!((*dma).req) as usize;
        let resp_addr = core::ptr::addr_of_mut!((*dma).resp) as usize;

        // Fill request header
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*dma).req.type_), req_type);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*dma).req.reserved), 0);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*dma).req.sector), sector);

        // Build 3-descriptor chain: req -> data -> resp
        let desc0 = &*state.desc_table.add(0);
        desc0.set(req_addr as u64, core::mem::size_of::<BlkReq>() as u32, DESC_F_NEXT, 1);

        // desc[1]: data (device-readable for write, device-writable for read)
        let data_flags: u16 = if req_type == BLK_T_IN { DESC_F_WRITE | DESC_F_NEXT } else { DESC_F_NEXT };
        let desc1 = &*state.desc_table.add(1);
        desc1.set(data_buf as u64, data_len, data_flags, 2);

        // desc[2]: BlkResp (device-writable)
        let desc2 = &*state.desc_table.add(2);
        desc2.set(resp_addr as u64, core::mem::size_of::<BlkResp>() as u32, DESC_F_WRITE, 0);

        // Memory barrier: ensure descriptor writes are visible before available ring update
        core::arch::asm!("dsb sy", "isb");

        // Submit to available ring (head = 0, 3 descriptors)
        let avail_idx = (*state.avail_ring).idx.load(Ordering::SeqCst);
        let ring_ptr = (state.avail_ring as *mut u8).add(4) as *mut u16;
        core::ptr::write_volatile(ring_ptr.add((avail_idx % state.queue_size) as usize), 0);
        core::arch::asm!("dsb sy", "isb");
        (*state.avail_ring).idx.store(avail_idx.wrapping_add(1), Ordering::SeqCst);

        // Notify device
        notify_queue(state, 0);

        // Wait for completion (poll used ring)
        let mut completed = false;
        for _ in 0..5_000_000 {
            let used_idx = (*state.used_ring).idx.load(Ordering::SeqCst);
            if used_idx != state.last_used_idx {
                core::arch::asm!("dsb sy", "isb");
                let status = core::ptr::read_volatile(core::ptr::addr_of!((*dma).resp.status));
                state.last_used_idx = state.last_used_idx.wrapping_add(1);
                completed = status == BLK_S_OK;
                break;
            }
            core::hint::spin_loop();
        }

        completed
    }
}

/// Read a single sector (512 bytes)
pub fn read_block(sector: u64, buf: &mut [u8; 512]) -> bool {
    blk_request(BLK_T_IN, sector, buf.as_mut_ptr(), SECTOR_SIZE as u32)
}

/// Write a single sector (512 bytes)
pub fn write_block(sector: u64, buf: &[u8; 512]) -> bool {
    blk_request(BLK_T_OUT, sector, buf.as_ptr() as *mut u8, SECTOR_SIZE as u32)
}

/// Flush device cache
#[allow(dead_code)]
pub fn flush() -> bool {
    // Use a dummy buffer for flush (no data transferred)
    let mut dummy = [0u8; 512];
    blk_request(BLK_T_FLUSH, 0, dummy.as_mut_ptr(), 0)
}

/// Hexdump a sector for debugging
pub fn hexdump_sector(sector: u64) {
    let mut buf = [0u8; 512];
    if !read_block(sector, &mut buf) {
        uart::puts(UART, "[FAIL] Read failed\r\n");
        return;
    }

    for row in 0..32 {
        let offset = row * 16;
        crate::print_hex(UART, offset as u32);
        uart::puts(UART, ": ");
        for col in 0..16 {
            crate::print_hex_byte(UART, buf[offset + col]);
            uart::putc(UART, b' ');
        }
        // ASCII representation
        for col in 0..16 {
            let b = buf[offset + col];
            uart::putc(UART, if b >= 0x20 && b < 0x7F { b } else { b'.' });
        }
        uart::puts(UART, "\r\n");
    }
}
