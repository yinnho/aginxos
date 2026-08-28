//! VirtIO-Net driver (simplified)
//!
//! Based on Redox OS virtio-core approach

use core::alloc::Layout;
use core::sync::atomic::{AtomicU16, AtomicU32, AtomicU64, Ordering};

#[allow(dead_code)]
const ECAM_BASE: usize = 0x3F00_0000;

// Conditional UART imports
#[cfg(not(feature = "board-redfin"))]
use crate::uart;
#[cfg(feature = "board-redfin")]
use crate::qup_uart as uart;

use crate::platform::UART;

const VIRTIO_NET_HDR_SIZE: usize = 12;
const MAX_PACKET_SIZE: usize = 1514;
const QUEUE_SIZE: u16 = 256;
const TX_BUF_SIZE: usize = VIRTIO_NET_HDR_SIZE + MAX_PACKET_SIZE; // 1526

// VirtIO constants
#[allow(dead_code)]
const VIRTIO_STATUS_ACKNOWLEDGE: u8 = 1;
#[allow(dead_code)]
const VIRTIO_STATUS_DRIVER: u8 = 2;
#[allow(dead_code)]
const VIRTIO_STATUS_DRIVER_OK: u8 = 4;
#[allow(dead_code)]
const VIRTIO_STATUS_FEATURES_OK: u8 = 8;
#[allow(dead_code)]
const VIRTIO_NET_F_MAC: u32 = 5;

const DESC_F_WRITE: u16 = 2;
#[allow(dead_code)]
const VIRTIO_PCI_CAP_VENDOR: u8 = 0x09;
#[allow(dead_code)]
const VIRTIO_PCI_CAP_COMMON_CFG: u8 = 1;
#[allow(dead_code)]
const VIRTIO_PCI_CAP_NOTIFY_CFG: u8 = 2;
#[allow(dead_code)]
const VIRTIO_PCI_CAP_DEVICE_CFG: u8 = 4;

/// Virtqueue Descriptor
#[repr(C, align(16))]
pub struct Descriptor {
    addr: AtomicU64,
    len: AtomicU32,
    flags: AtomicU16,
    next: AtomicU16,
}

impl Descriptor {
    fn set(&self, addr: u64, len: u32, flags: u16, next: u16) {
        self.addr.store(addr, Ordering::Relaxed);
        self.len.store(len, Ordering::Relaxed);
        self.flags.store(flags, Ordering::Relaxed);
        self.next.store(next, Ordering::Relaxed);
    }
}

/// Available Ring
#[repr(C)]
pub struct AvailableRing {
    flags: AtomicU16,
    idx: AtomicU16,
}

/// Used Ring
#[repr(C)]
pub struct UsedRing {
    flags: AtomicU16,
    idx: AtomicU16,
}

/// Used Element
#[repr(C)]
pub struct UsedElement {
    id: AtomicU32,
    len: AtomicU32,
}

/// Common Configuration (VirtIO 1.0 spec section 4.1.4.3.1)
#[repr(C)]
pub struct CommonCfg {
    pub device_feature_select: u32,  // 0x00
    pub device_feature: u32,         // 0x04
    pub driver_feature_select: u32,  // 0x08
    pub driver_feature: u32,         // 0x0c
    pub config_msix_vector: u16,     // 0x10
    pub num_queues: u16,             // 0x12
    pub device_status: u8,           // 0x14
    pub config_generation: u8,       // 0x15
    pub queue_select: u16,           // 0x16
    pub queue_size: u16,             // 0x18
    pub queue_msix_vector: u16,      // 0x1a
    pub queue_enable: u16,           // 0x1c
    pub queue_notify_off: u16,       // 0x1e
    pub queue_desc: u64,             // 0x20 (natural alignment adds padding before this)
    pub queue_driver: u64,           // 0x28
    pub queue_device: u64,           // 0x30
}

/// VirtQueue state
#[repr(C)]
struct VirtQueue {
    desc_table: *mut Descriptor,
    avail_ring: *mut AvailableRing,
    used_ring: *mut UsedRing,
    queue_size: u16,
    free_head: u16,
    last_used_idx: u16,
    /// Cached notify offset (notify_off * notify_multiplier) — avoids runtime queue_select write
    notify_offset: usize,
}

/// Network state
#[repr(C)]
struct NetState {
    mac: [u8; 6],
    common: *mut CommonCfg,
    notify_base: *mut u8,
    notify_multiplier: u32,
    rx_queue: *mut VirtQueue,
    tx_queue: *mut VirtQueue,
    #[allow(dead_code)]
    rx_buffers: [*mut u8; QUEUE_SIZE as usize],
    /// TX buffer addresses for DMA free on completion
    tx_buffers: [usize; QUEUE_SIZE as usize],
    /// Number of TX descriptors currently in flight
    tx_inflight: u16,
    /// PCI device location for ECAM-based notify workaround
    pci_bus: u8,
    pci_dev: u8,
    pci_func: u8,
    /// PCI_CFG capability offset in config space (type 5)
    pci_cfg_cap_offset: u8,
    /// Notify BAR index and offset within BAR
    notify_bar: u8,
    notify_bar_offset: u32,
}

static mut NET_STATE: *mut NetState = core::ptr::null_mut();

/// PCI ECAM access
#[allow(dead_code)]
fn config_read32(bus: u8, dev: u8, func: u8, offset: u8) -> u32 {
    let addr = ECAM_BASE | ((bus as usize) << 20) | ((dev as usize) << 15) | ((func as usize) << 12) | ((offset as usize) & 0xFC);
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

#[allow(dead_code)]
fn config_write32(bus: u8, dev: u8, func: u8, offset: u8, val: u32) {
    let addr = ECAM_BASE | ((bus as usize) << 20) | ((dev as usize) << 15) | ((func as usize) << 12) | ((offset as usize) & 0xFC);
    unsafe { core::ptr::write_volatile(addr as *mut u32, val) }
}

/// Write a byte to PCI config space via ECAM
fn config_write8(bus: u8, dev: u8, func: u8, offset: u8, val: u8) {
    let addr = ECAM_BASE | ((bus as usize) << 20) | ((dev as usize) << 15) | ((func as usize) << 12) | (offset as usize);
    unsafe { core::ptr::write_volatile(addr as *mut u8, val) }
}

/// Write a 16-bit value to PCI config space via ECAM
fn config_write16(bus: u8, dev: u8, func: u8, offset: u8, val: u16) {
    let addr = ECAM_BASE | ((bus as usize) << 20) | ((dev as usize) << 15) | ((func as usize) << 12) | (offset as usize & 0xFE);
    unsafe { core::ptr::write_volatile(addr as *mut u16, val) }
}

/// Write a 32-bit value to PCI config space via ECAM with u32 offset (for extended config space)
fn ecam_write32(bus: u8, dev: u8, func: u8, offset: u32, val: u32) {
    let addr = ECAM_BASE | ((bus as usize) << 20) | ((dev as usize) << 15) | ((func as usize) << 12) | (offset as usize & 0xFC);
    unsafe { core::ptr::write_volatile(addr as *mut u32, val) }
}

/// Write a 16-bit value to PCI config space via ECAM with u32 offset
fn ecam_write16(bus: u8, dev: u8, func: u8, offset: u32, val: u16) {
    let addr = ECAM_BASE | ((bus as usize) << 20) | ((dev as usize) << 15) | ((func as usize) << 12) | (offset as usize & 0xFFE);
    unsafe { core::ptr::write_volatile(addr as *mut u16, val) }
}

/// Write a byte to PCI config space via ECAM with u32 offset
fn ecam_write8(bus: u8, dev: u8, func: u8, offset: u32, val: u8) {
    let addr = ECAM_BASE | ((bus as usize) << 20) | ((dev as usize) << 15) | ((func as usize) << 12) | (offset as usize);
    unsafe { core::ptr::write_volatile(addr as *mut u8, val) }
}
/// DMA region bump allocator — uses heap (Normal WB memory)
static mut DMA_NEXT: usize = 0;
const DMA_END: usize = 0;

/// Allocate page-aligned DMA memory from heap
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
fn read_bar_mem(bus: u8, dev: u8, func: u8, bar_idx: u8) -> Option<usize> {
    let offset = 0x10 + bar_idx * 4;
    let val = config_read32(bus, dev, func, offset);
    if val & 0x1 != 0 { return None; }
    let base = (val & 0xFFFF_FFF0) as usize;
    let bar_type = (val >> 1) & 0x3;
    if bar_type == 0x2 {
        let high = config_read32(bus, dev, func, offset + 4);
        Some(base | ((high as usize) << 32))
    } else {
        Some(base)
    }
}
/// Allocate a zeroed VirtQueue on the heap using volatile byte writes
fn alloc_virt_queue() -> Option<*mut VirtQueue> {
    let layout = core::alloc::Layout::from_size_align(core::mem::size_of::<VirtQueue>(), 8).ok()?;
    unsafe {
        let ptr = alloc::alloc::alloc_zeroed(layout) as *mut VirtQueue;
        if ptr.is_null() { None } else { Some(ptr) }
    }
}

/// Setup virtqueue using volatile field writes to avoid QEMU ARM64 codegen hang
/// Fills in a heap-allocated VirtQueue struct using volatile writes per field
fn setup_queue_volatile(q: *mut VirtQueue, common: *mut CommonCfg, queue_idx: u16) -> bool {
    unsafe {
        // Step 1: Select the queue
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).queue_select), queue_idx);
        let queue_size = core::ptr::read_volatile(core::ptr::addr_of!((*common).queue_size));
        if queue_size == 0 { return false; }

        let desc_size = core::mem::size_of::<Descriptor>() * queue_size as usize;
        let avail_size = 6 + 2 * queue_size as usize;
        let used_size = 6 + 8 * queue_size as usize;

        let desc_addr = match dma_alloc((desc_size + 4095) / 4096 * 4096) {
            Some(a) => a,
            None => return false,
        };
        let avail_addr = match dma_alloc((avail_size + 4095) / 4096 * 4096) {
            Some(a) => a,
            None => return false,
        };
        let used_addr = match dma_alloc((used_size + 4095) / 4096 * 4096) {
            Some(a) => a,
            None => return false,
        };

        // Step 2: Write physical addresses (VirtIO 1.0 spec order)
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).queue_desc), desc_addr as u64);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).queue_driver), avail_addr as u64);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).queue_device), used_addr as u64);
        // Write queue_size back (required by VirtIO 1.0 — QEMU may zero it on reset)
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).queue_size), queue_size);

        // Step 3: Enable the queue
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common).queue_enable), 1);
        // Barrier to ensure device sees all writes
        core::arch::asm!("dsb sy", "isb");

        // Memory barrier
        core::arch::asm!("dsb sy");

        // Fill VirtQueue fields using volatile writes via addr_of_mut!
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*q).desc_table), desc_addr as *mut Descriptor);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*q).avail_ring), avail_addr as *mut AvailableRing);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*q).used_ring), used_addr as *mut UsedRing);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*q).queue_size), queue_size);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*q).free_head), 0u16);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*q).last_used_idx), 0u16);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*q).notify_offset), 0usize);

        // Zero descriptor chain: each desc.next = i+1
        for i in 0..queue_size as usize {
            let desc = desc_addr as *mut Descriptor;
            let d = &*desc.add(i);
            d.addr.store(0, Ordering::Relaxed);
            d.len.store(0, Ordering::Relaxed);
            d.flags.store(0, Ordering::Relaxed);
            d.next.store((i + 1) as u16, Ordering::Relaxed);
        }
        // Mark last descriptor
        let last_desc = (desc_addr as *const Descriptor).add(queue_size as usize - 1);
        (*last_desc).next.store(queue_size as u16, Ordering::Relaxed);

        true
    }
}
/// Add RX buffer to queue
unsafe fn add_rx_buffer(queue: &mut VirtQueue, buf: *mut u8, len: usize) {
    let desc_idx = queue.free_head;
    let desc = &*queue.desc_table.add(desc_idx as usize);
    // Follow the chain to get next free descriptor
    let next_free = desc.next.load(Ordering::Relaxed);
    desc.set(buf as u64, len as u32, DESC_F_WRITE, 0);
    let avail_idx = (*queue.avail_ring).idx.load(Ordering::Relaxed);
    let ring_ptr = (queue.avail_ring as *mut u8).add(4) as *mut u16;
    core::ptr::write_volatile(ring_ptr.add((avail_idx % queue.queue_size) as usize), desc_idx);
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::Release);
    (*queue.avail_ring).idx.store(avail_idx.wrapping_add(1), Ordering::Relaxed);
    queue.free_head = next_free;
}
/// Notify device for a specific queue via direct BAR MMIO write
/// Falls back to ECAM config space write if BAR MMIO fails (TCG routing bug)
/// Detects MMIO mode (pci_bus == 0xFF sentinel) and uses MMIO notify instead
unsafe fn notify_queue(state: &NetState, queue_idx: u16) {
    // MMIO mode: use virtio-mmio notify register
    if state.pci_bus == 0xFF {
        notify_queue_mmio(state.notify_base as usize, queue_idx);
        return;
    }

    let queue = if queue_idx == 0 { &*state.rx_queue } else { &*state.tx_queue };
    // DSB to ensure all data writes are visible before notify
    core::arch::asm!("dsb sy");

    // Method 1: Direct BAR MMIO write
    let addr = state.notify_base as usize + queue.notify_offset;
    core::ptr::write_volatile(addr as *mut u16, queue_idx);

    // Method 2: Also write via ECAM PCI config space (PCI_CFG_NOTIFY workaround)
    // This bypasses the TCG BAR MMIO routing bug
    if state.pci_cfg_cap_offset != 0 {
        let bus = state.pci_bus;
        let dev = state.pci_dev;
        let func = state.pci_func;
        let cfg_off = state.pci_cfg_cap_offset as u32;
        let bar = state.notify_bar;
        let bar_offset = state.notify_bar_offset;
        let noff = (queue.notify_offset / core::mem::size_of::<u16>()) as u32;

        // Write cap.bar, cap.padding, cap.offset, cap.length at pci_cfg_cap
        // pci_cfg_data starts at cfg_off + 4
        ecam_write8(bus, dev, func, cfg_off, bar);                  // cap.bar
        ecam_write8(bus, dev, func, cfg_off + 1, 0);                // cap.padding[0]
        ecam_write8(bus, dev, func, cfg_off + 2, 0);                // cap.padding[1]
        ecam_write8(bus, dev, func, cfg_off + 3, 0);                // cap.padding[2]
        ecam_write32(bus, dev, func, cfg_off + 4, bar_offset + noff * 2); // cap.offset
        ecam_write32(bus, dev, func, cfg_off + 8, 2);               // cap.length = 2

        // Write the notify value to pci_cfg_data (at cfg_off + 12)
        ecam_write16(bus, dev, func, cfg_off + 12, queue_idx);
    }
}

/// Clean data cache to Point of Coherency for a memory range
unsafe fn cache_clean_range(start: usize, len: usize) {
    const CACHE_LINE_SIZE: usize = 64; // ARM64 typical cache line
    let end = start + len;
    let mut addr = start & !(CACHE_LINE_SIZE - 1);
    while addr < end {
        core::arch::asm!("dc cvac, {}", in(reg) addr);
        addr += CACHE_LINE_SIZE;
    }
}
/// Capability info
#[allow(dead_code)]
struct VirtioCaps {
    common: Option<usize>,
    notify: Option<(usize, u32, u8, u32)>, // (addr, multiplier, bar_index, bar_offset)
    device: Option<usize>,
    pci_cfg: Option<u8>, // PCI_CFG cap offset in config space (type 5)
}
/// Scan VirtIO capabilities
#[allow(dead_code)]
fn scan_virtio_caps(bus: u8, dev: u8, func: u8) -> VirtioCaps {
    let mut caps = VirtioCaps { common: None, notify: None, device: None, pci_cfg: None };
    // Get capabilities pointer from PCI status register (offset 0x34)
    let cap_ptr = config_read32(bus, dev, func, 0x34) as u8;
    if cap_ptr == 0 { return caps; }
    let mut ptr = cap_ptr;
    let mut iterations = 0;
    while ptr != 0 && iterations < 48 {  // Max 48 capabilities to prevent infinite loop
        iterations += 1;
        // Read capability header dword
        let cap_hdr = config_read32(bus, dev, func, ptr);
        let cap_id = cap_hdr as u8;
        let next_ptr = ((cap_hdr >> 8) & 0xFF) as u8;
        let cap_len = ((cap_hdr >> 16) & 0xFF) as u8;
        if cap_id == VIRTIO_PCI_CAP_VENDOR && cap_len >= 16 {
            // Read VirtIO capability fields
            // cfg_type is at offset 3 in the capability (4th byte of header)
            let cfg_type = (cap_hdr >> 24) as u8;
            let bar = config_read32(bus, dev, func, ptr + 4) as u8;
            let offset = config_read32(bus, dev, func, ptr + 8);
            let _length = config_read32(bus, dev, func, ptr + 12);
            // Only read notify_mult for NOTIFY_CFG to avoid reading pci_cfg_data
            let notify_mult = if cfg_type == VIRTIO_PCI_CAP_NOTIFY_CFG {
                config_read32(bus, dev, func, ptr + 16)
            } else {
                0
            };
            match cfg_type {
                5 => {
                    // VIRTIO_PCI_CAP_PCI_CFG — save config space offset for ECAM-based notify
                    caps.pci_cfg = Some(ptr);
                }
                _ => {
                    if let Some(bar_base) = read_bar_mem(bus, dev, func, bar) {
                        let addr = bar_base + offset as usize;
                        match cfg_type {
                            VIRTIO_PCI_CAP_COMMON_CFG => caps.common = Some(addr),
                            VIRTIO_PCI_CAP_NOTIFY_CFG => caps.notify = Some((addr, notify_mult, bar, offset)),
                            VIRTIO_PCI_CAP_DEVICE_CFG => caps.device = Some(addr),
                            _ => {}
                        }
                    }
                }
            }
        }
        if ptr == next_ptr { break; }  // Prevent infinite loop
        ptr = next_ptr;
    }
    caps
}

// ─── VirtIO-MMIO Transport ────────────────────────────────────────────────────
//
// Simple MMIO register interface (no PCI BAR programming needed).
// QEMU virt machine places virtio-mmio devices at 0x0A000000+.

const VIRTIO_MMIO_MAGIC: u32 = 0x7472_6976;
const VIRTIO_MMIO_VERSION_MODERN: u32 = 2;
const VIRTIO_MMIO_DEVICE_NET: u32 = 1; // network device

// MMIO register offsets (from Linux virtio_mmio.h)
const MMIO_MAGIC_VALUE: usize = 0x000;
const MMIO_VERSION: usize = 0x004;
const MMIO_DEVICE_ID: usize = 0x008;
#[allow(dead_code)]
const MMIO_VENDOR_ID: usize = 0x00c;
const MMIO_DEVICE_FEATURES: usize = 0x010;
const MMIO_DEVICE_FEATURES_SEL: usize = 0x014;
const MMIO_DRIVER_FEATURES: usize = 0x020;
const MMIO_DRIVER_FEATURES_SEL: usize = 0x024;
#[allow(dead_code)]
const MMIO_GUEST_PAGE_SIZE: usize = 0x028;
const MMIO_QUEUE_SEL: usize = 0x030;
const MMIO_QUEUE_NUM_MAX: usize = 0x034;
const MMIO_QUEUE_NUM: usize = 0x038;
const MMIO_QUEUE_ALIGN: usize = 0x03c;
const MMIO_QUEUE_PFN: usize = 0x040;       // Legacy (v1)
const MMIO_QUEUE_READY: usize = 0x044;      // Modern (v2)
const MMIO_QUEUE_NOTIFY: usize = 0x050;
#[allow(dead_code)]
const MMIO_INTERRUPT_STATUS: usize = 0x060;
#[allow(dead_code)]
const MMIO_INTERRUPT_ACK: usize = 0x064;
const MMIO_STATUS: usize = 0x070;
#[allow(dead_code)]
const MMIO_CONFIG_GENERATION: usize = 0x0fc;
const MMIO_CONFIG: usize = 0x100;
// Version 2 additional registers
const MMIO_QUEUE_DESC_LOW: usize = 0x080;
const MMIO_QUEUE_DESC_HIGH: usize = 0x084;
const MMIO_QUEUE_AVAIL_LOW: usize = 0x090;
const MMIO_QUEUE_AVAIL_HIGH: usize = 0x094;
const MMIO_QUEUE_USED_LOW: usize = 0x0a0;
const MMIO_QUEUE_USED_HIGH: usize = 0x0a4;

const MMIO_BASE: usize = 0x0A00_0000;
const MMIO_STRIDE: usize = 0x200;
const MMIO_MAX_SLOTS: usize = 32;

/// MMIO volatile read
unsafe fn mmio_read32(base: usize, offset: usize) -> u32 {
    core::ptr::read_volatile((base + offset) as *const u32)
}

/// MMIO volatile write
unsafe fn mmio_write32(base: usize, offset: usize, val: u32) {
    core::ptr::write_volatile((base + offset) as *mut u32, val);
}

/// MMIO volatile write 16-bit
unsafe fn mmio_write16(base: usize, offset: usize, val: u16) {
    core::ptr::write_volatile((base + offset) as *mut u16, val);
}

/// Probe for a virtio-mmio network device
unsafe fn probe_mmio_net() -> Option<usize> {
    for slot in 0..MMIO_MAX_SLOTS {
        let base = MMIO_BASE + slot * MMIO_STRIDE;
        let magic = mmio_read32(base, MMIO_MAGIC_VALUE);
        if magic != VIRTIO_MMIO_MAGIC { continue; }
        let device_id = mmio_read32(base, MMIO_DEVICE_ID);
        if device_id != VIRTIO_MMIO_DEVICE_NET { continue; }
        return Some(base);
    }
    None
}

/// Setup a virtqueue via MMIO registers
unsafe fn setup_queue_mmio(base: usize, queue_idx: u16, version: u32) -> Option<*mut VirtQueue> {
    // Select queue
    mmio_write32(base, MMIO_QUEUE_SEL, queue_idx as u32);
    let queue_size = mmio_read32(base, MMIO_QUEUE_NUM_MAX) as u16;
    if queue_size == 0 { return None; }
    let actual_size = if queue_size < QUEUE_SIZE { queue_size } else { QUEUE_SIZE };

    // Allocate descriptor table, available ring, used ring
    let desc_size = core::mem::size_of::<Descriptor>() * actual_size as usize;
    let avail_size = 6 + 2 * actual_size as usize;
    let used_size = 6 + 8 * actual_size as usize;

    let mut desc_addr = dma_alloc((desc_size + 4095) / 4096 * 4096)?;
    let mut avail_addr = dma_alloc((avail_size + 4095) / 4096 * 4096)?;
    let mut used_addr = dma_alloc((used_size + 4095) / 4096 * 4096)?;

    // Write queue size
    mmio_write32(base, MMIO_QUEUE_NUM, actual_size as u32);

    if version == 2 {
        // Modern (v2): write split queue addresses
        mmio_write32(base, MMIO_QUEUE_DESC_LOW, desc_addr as u32);
        mmio_write32(base, MMIO_QUEUE_DESC_HIGH, (desc_addr as u64 >> 32) as u32);
        mmio_write32(base, MMIO_QUEUE_AVAIL_LOW, avail_addr as u32);
        mmio_write32(base, MMIO_QUEUE_AVAIL_HIGH, (avail_addr as u64 >> 32) as u32);
        mmio_write32(base, MMIO_QUEUE_USED_LOW, used_addr as u32);
        mmio_write32(base, MMIO_QUEUE_USED_HIGH, (used_addr as u64 >> 32) as u32);
        // Enable queue
        mmio_write32(base, MMIO_QUEUE_READY, 1);
    } else {
        // Legacy (v1): contiguously allocate desc+avail+used, write PFN
        // Layout: desc | avail | pad | used (used must be page-aligned)
        let total = desc_size + avail_size + 4096 + used_size;
        let combined = dma_alloc((total + 4095) / 4096 * 4096)?;
        core::ptr::copy_nonoverlapping(desc_addr as *const u8, combined as *mut u8, desc_size);
        let avail_off = desc_size;
        core::ptr::copy_nonoverlapping(avail_addr as *const u8, (combined + avail_off) as *mut u8, avail_size);
        let used_off = (avail_off + avail_size + 4095) & !4095;
        core::ptr::copy_nonoverlapping(used_addr as *const u8, (combined + used_off) as *mut u8, used_size);
        desc_addr = combined;
        avail_addr = combined + avail_off;
        used_addr = combined + used_off;
        mmio_write32(base, MMIO_QUEUE_ALIGN, 4096);
        mmio_write32(base, MMIO_QUEUE_PFN, (combined as u32) / 4096);
    }

    // Allocate and init VirtQueue struct
    let q = alloc_virt_queue()?;
    core::ptr::write_volatile(core::ptr::addr_of_mut!((*q).desc_table), desc_addr as *mut Descriptor);
    core::ptr::write_volatile(core::ptr::addr_of_mut!((*q).avail_ring), avail_addr as *mut AvailableRing);
    core::ptr::write_volatile(core::ptr::addr_of_mut!((*q).used_ring), used_addr as *mut UsedRing);
    core::ptr::write_volatile(core::ptr::addr_of_mut!((*q).queue_size), actual_size);
    core::ptr::write_volatile(core::ptr::addr_of_mut!((*q).free_head), 0u16);
    core::ptr::write_volatile(core::ptr::addr_of_mut!((*q).last_used_idx), 0u16);
    core::ptr::write_volatile(core::ptr::addr_of_mut!((*q).notify_offset), 0usize); // MMIO uses queue_idx directly

    // Init free list — Descriptor fields are atomics, use store()
    for i in 0..actual_size as usize {
        let desc = desc_addr as *mut Descriptor;
        let d = &*desc.add(i);
        d.addr.store(0, Ordering::Relaxed);
        d.len.store(0, Ordering::Relaxed);
        d.flags.store(0, Ordering::Relaxed);
        d.next.store((i + 1) as u16, Ordering::Relaxed);
    }
    let last = &*(desc_addr as *const Descriptor).add(actual_size as usize - 1);
    last.next.store(actual_size as u16, Ordering::Relaxed);

    Some(q)
}

/// Notify a virtqueue via MMIO
unsafe fn notify_queue_mmio(base: usize, queue_idx: u16) {
    core::arch::asm!("dsb sy");
    mmio_write32(base, MMIO_QUEUE_NOTIFY, queue_idx as u32);
}

/// Initialize virtio-net via MMIO transport
unsafe fn init_mmio(uart_base: usize) -> bool {
    let mmio_base = match probe_mmio_net() {
        Some(b) => b,
        None => return false,
    };

    uart::puts(uart_base, "  MMIO net @0x");
    crate::print_hex(uart_base, mmio_base as u32);
    uart::puts(uart_base, "\r\n");

    // Read version
    let version = mmio_read32(mmio_base, MMIO_VERSION);
    uart::puts(uart_base, "  MMIO version=");
    crate::print_hex(uart_base, version);
    uart::puts(uart_base, "\r\n");

    // Reset device
    mmio_write32(mmio_base, MMIO_STATUS, 0);
    for _ in 0..10000 { core::hint::spin_loop(); }

    // Acknowledge
    mmio_write32(mmio_base, MMIO_STATUS, VIRTIO_STATUS_ACKNOWLEDGE as u32);
    mmio_write32(mmio_base, MMIO_STATUS, (VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER) as u32);

    // Feature negotiation
    mmio_write32(mmio_base, MMIO_DEVICE_FEATURES_SEL, 0);
    let features_low = mmio_read32(mmio_base, MMIO_DEVICE_FEATURES);
    let has_mac = (features_low & (1 << VIRTIO_NET_F_MAC)) != 0;
    mmio_write32(mmio_base, MMIO_DRIVER_FEATURES_SEL, 0);
    mmio_write32(mmio_base, MMIO_DRIVER_FEATURES, if has_mac { 1 << VIRTIO_NET_F_MAC } else { 0 });

    if version == 2 {
        // Modern: also negotiate high feature bits and check FEATURES_OK
        mmio_write32(mmio_base, MMIO_DEVICE_FEATURES_SEL, 1);
        let features_high = mmio_read32(mmio_base, MMIO_DEVICE_FEATURES);
        mmio_write32(mmio_base, MMIO_DRIVER_FEATURES_SEL, 1);
        mmio_write32(mmio_base, MMIO_DRIVER_FEATURES, if (features_high & 1) != 0 { 1 } else { 0 });
        mmio_write32(mmio_base, MMIO_STATUS,
            (VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_FEATURES_OK) as u32);
        let status = mmio_read32(mmio_base, MMIO_STATUS) as u8;
        if (status & VIRTIO_STATUS_FEATURES_OK) == 0 {
            uart::puts(uart_base, "[FAIL] Net MMIO: feature negotiation failed\r\n");
            return false;
        }
    }
    // Legacy: no FEATURES_OK bit, just set ACK|DRIVER

    // For legacy (v1): set guest page size before queue setup
    if version == 1 {
        mmio_write32(mmio_base, MMIO_GUEST_PAGE_SIZE, 4096);
    }

    // Read MAC from MMIO config area
    let mac: [u8; 6] = [
        core::ptr::read_volatile((mmio_base + MMIO_CONFIG) as *const u8),
        core::ptr::read_volatile((mmio_base + MMIO_CONFIG + 1) as *const u8),
        core::ptr::read_volatile((mmio_base + MMIO_CONFIG + 2) as *const u8),
        core::ptr::read_volatile((mmio_base + MMIO_CONFIG + 3) as *const u8),
        core::ptr::read_volatile((mmio_base + MMIO_CONFIG + 4) as *const u8),
        core::ptr::read_volatile((mmio_base + MMIO_CONFIG + 5) as *const u8),
    ];

    uart::puts(uart_base, "  MAC=");
    for (i, b) in mac.iter().enumerate() {
        if i > 0 { uart::putc(uart_base, b':'); }
        crate::print_hex_byte(uart_base, *b);
    }
    uart::puts(uart_base, "\r\n");

    // Setup RX queue (queue 0)
    let rx_queue = match setup_queue_mmio(mmio_base, 0, version) {
        Some(q) => q,
        None => { uart::puts(uart_base, "[FAIL] MMIO RX queue\r\n"); return false; }
    };

    // Setup TX queue (queue 1)
    let tx_queue = match setup_queue_mmio(mmio_base, 1, version) {
        Some(q) => q,
        None => { uart::puts(uart_base, "[FAIL] MMIO TX queue\r\n"); return false; }
    };

    // Allocate RX buffers
    let num_bufs = 128u16;
    let pkt_size = VIRTIO_NET_HDR_SIZE + MAX_PACKET_SIZE;
    let mut buf_count = 0u32;
    for i in 0..num_bufs as usize {
        let buf_layout = Layout::from_size_align(pkt_size, 8).unwrap();
        let buf_ptr = alloc::alloc::alloc_zeroed(buf_layout);
        if buf_ptr.is_null() { break; }
        let b = buf_ptr as usize;
        add_rx_buffer(&mut *rx_queue, b as *mut u8, pkt_size);
        buf_count += 1;
    }

    // Set DRIVER_OK
    mmio_write32(mmio_base, MMIO_STATUS,
        (VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_FEATURES_OK | VIRTIO_STATUS_DRIVER_OK) as u32);

    // Allocate NetState
    let ns_layout = Layout::from_size_align(core::mem::size_of::<NetState>(), 8).unwrap();
    let ns_ptr = alloc::alloc::alloc_zeroed(ns_layout) as *mut NetState;
    if ns_ptr.is_null() {
        uart::puts(uart_base, "[FAIL] MMIO alloc NetState\r\n");
        return false;
    }

    // Write NetState fields
    let mac_ptr = core::ptr::addr_of_mut!((*ns_ptr).mac) as *mut u8;
    for i in 0..6 { core::ptr::write_volatile(mac_ptr.add(i), mac[i]); }
    core::ptr::write_volatile(core::ptr::addr_of_mut!((*ns_ptr).common), core::ptr::null_mut()); // No CommonCfg for MMIO
    core::ptr::write_volatile(core::ptr::addr_of_mut!((*ns_ptr).notify_base), mmio_base as *mut u8); // Store MMIO base
    core::ptr::write_volatile(core::ptr::addr_of_mut!((*ns_ptr).notify_multiplier), 0);
    core::ptr::write_volatile(core::ptr::addr_of_mut!((*ns_ptr).rx_queue), rx_queue);
    core::ptr::write_volatile(core::ptr::addr_of_mut!((*ns_ptr).tx_queue), tx_queue);
    core::ptr::write_volatile(core::ptr::addr_of_mut!((*ns_ptr).pci_bus), 0xFF); // Sentinel: MMIO mode
    core::ptr::write_volatile(core::ptr::addr_of_mut!((*ns_ptr).pci_dev), 0);
    core::ptr::write_volatile(core::ptr::addr_of_mut!((*ns_ptr).pci_func), 0);
    core::ptr::write_volatile(core::ptr::addr_of_mut!((*ns_ptr).pci_cfg_cap_offset), 0);
    core::ptr::write_volatile(core::ptr::addr_of_mut!((*ns_ptr).notify_bar), 0);
    core::ptr::write_volatile(core::ptr::addr_of_mut!((*ns_ptr).notify_bar_offset), 0);

    // Store MMIO base in notify_bar field for MMIO mode detection
    // We use pci_bus == 0xFF as sentinel for MMIO mode

    // Init rx_buffers array
    // (Not needed for MMIO — the buffers are tracked in the descriptor table)

    // Kick RX queue
    notify_queue_mmio(mmio_base, 0);

    NET_STATE = ns_ptr;

    uart::puts(uart_base, "\r\n[OK] Net MMIO: ");
    crate::print_hex(uart_base, buf_count);
    uart::puts(uart_base, " RX buffers\r\n");

    true
}

/// Initialize network driver
#[allow(dead_code)]
pub fn init(uart: usize) {
    uart::puts(uart, "[..] Net\r\n");

    // Try VirtIO-MMIO first (works around QEMU TCG PCI BAR routing bug)
    unsafe {
        if init_mmio(uart) {
            return;
        }
    }

    let info = match crate::pci::get_virtio_net() {
        Some(i) => i,
        None => {
            uart::puts(uart, "[SKIP] Net: no virtio-net device\r\n");
            return;
        }
    };

    // Debug: print BAR values before any programming
    for bar_idx in 0..6u8 {
        let offset = 0x10 + bar_idx * 4;
        let bar = config_read32(info.bus, info.dev, info.func, offset);
        if bar != 0 {
            uart::puts(uart, "  BAR");
            uart::putc(uart, b'0' + bar_idx);
            uart::puts(uart, "=0x");
            crate::print_hex(uart, bar);
            uart::puts(uart, "\r\n");
        }
    }

    // Program BARs using proper PCI sizing to get correct sizes
    let mut next_bar_addr: usize = 0x1000_0000;
    let mut bar_idx: u8 = 0;
    while bar_idx < 6 {
        let off = 0x10 + bar_idx * 4;
        let orig = config_read32(info.bus, info.dev, info.func, off);
        if orig & 0x1 != 0 {
            // I/O space BAR — skip
            bar_idx += 1;
            continue;
        }
        let bar_type = (orig >> 1) & 0x3;
        let is_64bit = bar_type == 0x2;
        let orig_hi = if is_64bit {
            config_read32(info.bus, info.dev, info.func, off + 4)
        } else {
            0
        };

        // Write all 1s to size the BAR
        config_write32(info.bus, info.dev, info.func, off, 0xFFFF_FFF0);
        if is_64bit {
            config_write32(info.bus, info.dev, info.func, off + 4, 0xFFFF_FFFF);
        }
        let mask = config_read32(info.bus, info.dev, info.func, off);

        // Restore original
        config_write32(info.bus, info.dev, info.func, off, orig);
        if is_64bit {
            config_write32(info.bus, info.dev, info.func, off + 4, orig_hi);
        }

        if mask == 0 || mask == 0xFFFF_FFF0 {
            // No BAR or write-only
            bar_idx += if is_64bit { 2 } else { 1 };
            continue;
        }

        // Compute size from mask: size = ~(mask & 0xFFFF_FFF0) + 1
        let size = (!(mask & 0xFFFF_FFF0)).wrapping_add(1);
        if size == 0 {
            bar_idx += if is_64bit { 2 } else { 1 };
            continue;
        }

        let aligned_addr = (next_bar_addr + size as usize - 1) & !(size as usize - 1);
        config_write32(info.bus, info.dev, info.func, off, (aligned_addr as u32) | (orig & 0xF));
        if is_64bit {
            config_write32(info.bus, info.dev, info.func, off + 4, (aligned_addr >> 32) as u32);
            next_bar_addr = aligned_addr + size as usize;
        } else {
            next_bar_addr = aligned_addr + size as usize;
        }

        uart::puts(uart, "  BAR");
        uart::putc(uart, b'0' + bar_idx);
        uart::puts(uart, " size=0x");
        crate::print_hex(uart, size);
        uart::puts(uart, " addr=0x");
        crate::print_hex(uart, aligned_addr as u32);
        uart::puts(uart, "\r\n");

        bar_idx += if is_64bit { 2 } else { 1 };
    }

    // Enable device (Bus Master + Memory Space + I/O Space)
    let cmd = config_read32(info.bus, info.dev, info.func, 0x04);
    config_write32(info.bus, info.dev, info.func, 0x04, cmd | 0x07);

    // Scan capabilities
    let caps = scan_virtio_caps(info.bus, info.dev, info.func);
    let common_addr = match caps.common {
        Some(a) => a,
        None => { uart::puts(uart, "[FAIL] Net: no common config\r\n"); return; }
    };
    let device_addr = match caps.device {
        Some(a) => a,
        None => { uart::puts(uart, "[FAIL] Net: no device config\r\n"); return; }
    };
    let (notify_addr, notify_multiplier, notify_bar, notify_bar_offset) = match caps.notify {
        Some((a, m, b, o)) => (a, m, b, o),
        None => { uart::puts(uart, "[FAIL] Net: no notify config\r\n"); return; }
    };
    let pci_cfg_cap = match caps.pci_cfg {
        Some(o) => o,
        None => 0,
    };

    let common_ptr = common_addr as *mut CommonCfg;

    // Reset and initialize device using volatile MMIO writes
    unsafe { core::ptr::write_volatile(core::ptr::addr_of_mut!((*common_ptr).device_status), 0); }
    for _ in 0..10000 { core::hint::spin_loop(); }
    unsafe { core::ptr::write_volatile(core::ptr::addr_of_mut!((*common_ptr).device_status), VIRTIO_STATUS_ACKNOWLEDGE); }
    unsafe { core::ptr::write_volatile(core::ptr::addr_of_mut!((*common_ptr).device_status), VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER); }

    // Feature negotiation
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common_ptr).device_feature_select), 0);
    }
    let features_low = unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*common_ptr).device_feature)) };
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common_ptr).device_feature_select), 1);
    }
    let features_high = unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*common_ptr).device_feature)) };
    let has_mac = (features_low & (1 << VIRTIO_NET_F_MAC)) != 0;
    // Negotiate low features: MAC only
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common_ptr).driver_feature_select), 0);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common_ptr).driver_feature), if has_mac { 1 << VIRTIO_NET_F_MAC } else { 0 });
        // Negotiate high features: VIRTIO_F_VERSION_1 (bit 32 = bit 0 of high word)
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common_ptr).driver_feature_select), 1);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common_ptr).driver_feature), if (features_high & 1) != 0 { 1 } else { 0 });
    }
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common_ptr).device_status),
            VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_FEATURES_OK);
        core::arch::asm!("dsb sy");
    }

    let status_val = unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*common_ptr).device_status)) };
    if (status_val & VIRTIO_STATUS_FEATURES_OK) == 0 {
        uart::puts(uart, "[FAIL] Net: feature negotiation failed\r\n");
        return;
    }

    // Read MAC address
    let mac: [u8; 6] = unsafe {
        [
            core::ptr::read_volatile(device_addr as *const u8),
            core::ptr::read_volatile((device_addr + 1) as *const u8),
            core::ptr::read_volatile((device_addr + 2) as *const u8),
            core::ptr::read_volatile((device_addr + 3) as *const u8),
            core::ptr::read_volatile((device_addr + 4) as *const u8),
            core::ptr::read_volatile((device_addr + 5) as *const u8),
        ]
    };

    uart::puts(uart, "  MAC=");
    for (i, b) in mac.iter().enumerate() {
        if i > 0 { uart::putc(uart, b':'); }
        crate::print_hex_byte(uart, *b);
    }
    uart::puts(uart, "\r\n");

    // Setup queues — allocate queue structs on heap, fill via volatile
    let rx_queue = match alloc_virt_queue() {
        Some(q) => q,
        None => { uart::puts(uart, "[FAIL] alloc rx_queue\r\n"); return; }
    };
    if !setup_queue_volatile(rx_queue, common_ptr, 0) {
        uart::puts(uart, "[FAIL] Net: RX queue setup\r\n"); return;
    }
    // Cache RX queue notify offset (avoids runtime queue_select write)
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common_ptr).queue_select), 0u16);
        let noff = core::ptr::read_volatile(core::ptr::addr_of!((*common_ptr).queue_notify_off));
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*rx_queue).notify_offset), noff as usize * notify_multiplier as usize);
    }
    let tx_queue = match alloc_virt_queue() {
        Some(q) => q,
        None => { uart::puts(uart, "[FAIL] alloc tx_queue\r\n"); return; }
    };
    if !setup_queue_volatile(tx_queue, common_ptr, 1) {
        uart::puts(uart, "[FAIL] Net: TX queue setup\r\n"); return;
    }
    // Cache TX queue notify offset (avoids runtime queue_select write)
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common_ptr).queue_select), 1u16);
        let noff = core::ptr::read_volatile(core::ptr::addr_of!((*common_ptr).queue_notify_off));
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*tx_queue).notify_offset), noff as usize * notify_multiplier as usize);
    }

    // Allocate RX buffers
    let rx_buffers_layout = core::alloc::Layout::from_size_align(
        core::mem::size_of::<*mut u8>() * QUEUE_SIZE as usize, 8).unwrap();
    let rx_buffers = unsafe { alloc::alloc::alloc_zeroed(rx_buffers_layout) as *mut *mut u8 };
    let mut buf_count = 0u32;
    let num_bufs = 128u16;
    let pkt_size = VIRTIO_NET_HDR_SIZE + MAX_PACKET_SIZE;
    for i in 0..num_bufs as usize {
        let buf_layout = Layout::from_size_align(pkt_size, 8).unwrap();
        let buf_ptr = unsafe { alloc::alloc::alloc_zeroed(buf_layout) };
        if buf_ptr.is_null() { break; }
        let b = buf_ptr as usize;
        unsafe {
            core::ptr::write_volatile(rx_buffers.add(i), b as *mut u8);
            add_rx_buffer(&mut *rx_queue, b as *mut u8, pkt_size);
        }
        buf_count += 1;
    }
    // Set DRIVER_OK (volatile MMIO write)
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common_ptr).device_status),
            VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_FEATURES_OK | VIRTIO_STATUS_DRIVER_OK);
        core::arch::asm!("dsb sy");
    }

    // Save state using heap-allocated NetState with volatile writes
    let ns_layout = core::alloc::Layout::from_size_align(core::mem::size_of::<NetState>(), 8).unwrap();
    let ns_ptr = unsafe { alloc::alloc::alloc_zeroed(ns_layout) as *mut NetState };
    if ns_ptr.is_null() {
        uart::puts(uart, "[FAIL] alloc NetState\r\n"); return;
    }
    unsafe {
        // Write fields using addr_of_mut! for correct offsets
        let mac_ptr = core::ptr::addr_of_mut!((*ns_ptr).mac) as *mut u8;
        for i in 0..6 { core::ptr::write_volatile(mac_ptr.add(i), mac[i]); }

        let common_ptr_field = core::ptr::addr_of_mut!((*ns_ptr).common);
        core::ptr::write_volatile(common_ptr_field, common_ptr);

        let notify_base_field = core::ptr::addr_of_mut!((*ns_ptr).notify_base);
        core::ptr::write_volatile(notify_base_field, notify_addr as *mut u8);

        let mult_field = core::ptr::addr_of_mut!((*ns_ptr).notify_multiplier);
        core::ptr::write_volatile(mult_field, notify_multiplier);

        let rxq_field = core::ptr::addr_of_mut!((*ns_ptr).rx_queue);
        core::ptr::write_volatile(rxq_field, rx_queue);

        let txq_field = core::ptr::addr_of_mut!((*ns_ptr).tx_queue);
        core::ptr::write_volatile(txq_field, tx_queue);

        // Store PCI info for ECAM-based notify workaround
        let pci_bus_field = core::ptr::addr_of_mut!((*ns_ptr).pci_bus);
        core::ptr::write_volatile(pci_bus_field, info.bus);
        let pci_dev_field = core::ptr::addr_of_mut!((*ns_ptr).pci_dev);
        core::ptr::write_volatile(pci_dev_field, info.dev);
        let pci_func_field = core::ptr::addr_of_mut!((*ns_ptr).pci_func);
        core::ptr::write_volatile(pci_func_field, info.func);
        let pci_cfg_field = core::ptr::addr_of_mut!((*ns_ptr).pci_cfg_cap_offset);
        core::ptr::write_volatile(pci_cfg_field, pci_cfg_cap);
        let notify_bar_field = core::ptr::addr_of_mut!((*ns_ptr).notify_bar);
        core::ptr::write_volatile(notify_bar_field, notify_bar);
        let notify_bar_off_field = core::ptr::addr_of_mut!((*ns_ptr).notify_bar_offset);
        core::ptr::write_volatile(notify_bar_off_field, notify_bar_offset);

        // Copy rx_buffers from heap-allocated array using volatile writes
        let rxb_field = core::ptr::addr_of_mut!((*ns_ptr).rx_buffers) as *mut *mut u8;
        for i in 0..QUEUE_SIZE as usize {
            core::ptr::write_volatile(rxb_field.add(i), core::ptr::read_volatile(rx_buffers.add(i)));
        }

        notify_queue(&*ns_ptr, 0);
        NET_STATE = ns_ptr;
    }

    uart::puts(uart, "\r\n[OK] Net: ");
    crate::print_hex(uart, buf_count);
    uart::puts(uart, " RX buffers\r\n");
}
pub fn get_mac() -> Option<[u8; 6]> {
    unsafe { if NET_STATE.is_null() { None } else { Some((*NET_STATE).mac) } }
}

/// Check if virtio-net is initialized
pub fn is_initialized() -> bool {
    unsafe { !NET_STATE.is_null() }
}

/// Kick RX queue (re-notify device about available RX buffers)
pub fn kick_rx() {
    unsafe {
        if NET_STATE.is_null() { return; }
        notify_queue(&*NET_STATE, 0);
    }
}

/// Dump TX queue state for debugging
pub fn dump_tx_debug() {
    unsafe {
        if NET_STATE.is_null() { uart::puts(UART, "Net: not init\r\n"); return; }
        let state = &*NET_STATE;
        let tx = &*state.tx_queue;
        let avail_idx = (*tx.avail_ring).idx.load(Ordering::Relaxed);
        let used_idx = (*tx.used_ring).idx.load(Ordering::Relaxed);
        uart::puts(UART, "TX desc=0x");
        crate::print_hex(UART, tx.desc_table as u32);
        uart::puts(UART, " avail=0x");
        crate::print_hex(UART, avail_idx as u32);
        uart::puts(UART, " used=0x");
        crate::print_hex(UART, used_idx as u32);
        uart::puts(UART, " free=0x");
        crate::print_hex(UART, tx.free_head as u32);
        uart::puts(UART, " inflight=0x");
        crate::print_hex(UART, state.tx_inflight as u32);
        uart::puts(UART, "\r\n");
    }
}

/// Transmit a packet
/// Returns true if packet was queued successfully
#[inline(never)]
pub fn transmit(data: &[u8]) -> bool {
    unsafe {
        if NET_STATE.is_null() { return false; }
        let state = &mut *NET_STATE;

        if data.len() > MAX_PACKET_SIZE {
            return false;
        }

        // Reclaim completed TX buffers from used ring
        let tx_queue = &mut *state.tx_queue;
        let used_idx = (*tx_queue.used_ring).idx.load(Ordering::Relaxed);
        while tx_queue.last_used_idx != used_idx {
            let used_ptr = (tx_queue.used_ring as *const u8).add(4) as *const UsedElement;
            let used = &*used_ptr.add((tx_queue.last_used_idx % tx_queue.queue_size) as usize);
            let desc_idx = used.id.load(Ordering::Relaxed) as u16;
            // Free the DMA buffer for this completed descriptor
            let buf_addr = state.tx_buffers[desc_idx as usize];
            if buf_addr != 0 {
                let layout = Layout::from_size_align(TX_BUF_SIZE, 8).unwrap();
                alloc::alloc::dealloc(buf_addr as *mut u8, layout);
                state.tx_buffers[desc_idx as usize] = 0;
            }
            // Return descriptor to free list
            let desc = &*tx_queue.desc_table.add(desc_idx as usize);
            desc.set(0, 0, 0, tx_queue.free_head as u16);
            tx_queue.free_head = desc_idx;
            tx_queue.last_used_idx = tx_queue.last_used_idx.wrapping_add(1);
            if state.tx_inflight > 0 { state.tx_inflight -= 1; }
        }

        // Check if queue has space
        if state.tx_inflight >= tx_queue.queue_size {
            return false; // Queue full
        }

        // Get a free descriptor
        let desc_idx = tx_queue.free_head;
        let desc = &*tx_queue.desc_table.add(desc_idx as usize);
        let next_free = desc.next.load(Ordering::Relaxed);

        // Allocate a TX buffer (always max size so alloc/dealloc layouts match)
        let buf = match dma_alloc(TX_BUF_SIZE) {
            Some(b) => b,
            None => { uart::puts(UART, "[TX] alloc fail\r\n"); return false; }
        };

        // Zero the virtio-net header
        for i in 0..VIRTIO_NET_HDR_SIZE {
            core::ptr::write_volatile((buf + i) as *mut u8, 0);
        }
        // Copy packet data after header — volatile reads from source
        let src_ptr = data.as_ptr();
        for i in 0..data.len() {
            let b = core::ptr::read_volatile(src_ptr.add(i));
            core::ptr::write_volatile((buf + VIRTIO_NET_HDR_SIZE + i) as *mut u8, b);
        }

        // Track buffer address for later freeing
        state.tx_buffers[desc_idx as usize] = buf;

        // Set up descriptor (len = actual data, not allocated buffer size)
        let desc_len = (VIRTIO_NET_HDR_SIZE + data.len()) as u32;
        desc.set(buf as u64, desc_len, 0, 0);

        // Advance free_head via the chain
        tx_queue.free_head = next_free;

        // Add to available ring
        let avail_idx = (*tx_queue.avail_ring).idx.load(Ordering::Relaxed);
        let ring_ptr = (tx_queue.avail_ring as *mut u8).add(4) as *mut u16;
        core::ptr::write_volatile(ring_ptr.add((avail_idx % tx_queue.queue_size) as usize), desc_idx);
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::Release);
        (*tx_queue.avail_ring).idx.store(avail_idx.wrapping_add(1), Ordering::Relaxed);

        state.tx_inflight += 1;

        // Notify device
        let _ = tx_queue;
        notify_queue(state, 1);
        true
    }
}

/// Receive and process one packet
/// Returns Some((buffer, len, desc_idx)) if a packet is available, None otherwise
/// desc_idx must be passed to release_rx_buffer to correctly return the buffer
#[inline(never)]
pub fn receive() -> Option<(*mut u8, usize, u16)> {
    unsafe {
        if NET_STATE.is_null() { return None; }
        let state = &mut *NET_STATE;
        let rx_queue = &mut *state.rx_queue;
        let last_idx = rx_queue.last_used_idx;
        let used_idx = (*rx_queue.used_ring).idx.load(Ordering::Relaxed);

        if last_idx == used_idx {
            return None; // No packets
        }

        // Get the used element
        let used_ptr = (rx_queue.used_ring as *const u8).add(4) as *const UsedElement;
        let used = &*used_ptr.add((last_idx % rx_queue.queue_size) as usize);
        let desc_idx = used.id.load(Ordering::Relaxed) as u16;
        let total_len = used.len.load(Ordering::Relaxed) as usize;

        // Get the descriptor and buffer
        let desc = &*rx_queue.desc_table.add(desc_idx as usize);
        let buf_addr = desc.addr.load(Ordering::Relaxed) as *mut u8;

        // Skip virtio-net header, return pointer to actual packet data
        let packet_len = if total_len > VIRTIO_NET_HDR_SIZE {
            total_len - VIRTIO_NET_HDR_SIZE
        } else {
            0
        };

        rx_queue.last_used_idx = rx_queue.last_used_idx.wrapping_add(1);

        if packet_len == 0 {
            // Re-add the buffer immediately
            put_rx_desc(rx_queue, desc_idx, buf_addr);
            notify_queue(state, 0);
            return None;
        }

        // Return pointer to packet data (after header) and descriptor index
        Some((buf_addr.add(VIRTIO_NET_HDR_SIZE), packet_len, desc_idx))
    }
}

/// Put an RX descriptor back into the free list and available ring
unsafe fn put_rx_desc(rx_queue: &mut VirtQueue, desc_idx: u16, buf_addr: *mut u8) {
    let desc = &*rx_queue.desc_table.add(desc_idx as usize);

    // Set up the descriptor for the next receive (this sets next=0)
    desc.set(buf_addr as u64, (VIRTIO_NET_HDR_SIZE + MAX_PACKET_SIZE) as u32, DESC_F_WRITE, 0);

    // Link this descriptor into the free list (AFTER set, which overwrites next)
    desc.next.store(rx_queue.free_head, Ordering::Relaxed);
    rx_queue.free_head = desc_idx;

    // Add to available ring
    let avail_idx = (*rx_queue.avail_ring).idx.load(Ordering::Relaxed);
    let ring_ptr = (rx_queue.avail_ring as *mut u8).add(4) as *mut u16;
    core::ptr::write_volatile(ring_ptr.add((avail_idx % rx_queue.queue_size) as usize), desc_idx);
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::Release);
    (*rx_queue.avail_ring).idx.store(avail_idx.wrapping_add(1), Ordering::Relaxed);
}

/// Release an RX buffer back to the queue after processing
/// desc_idx must match the descriptor index returned by receive()
pub fn release_rx_buffer(buf: *mut u8, desc_idx: u16) {
    unsafe {
        if NET_STATE.is_null() { return; }
        let state = &mut *NET_STATE;

        // Get the original buffer address (before the header offset)
        let original_buf = buf.sub(VIRTIO_NET_HDR_SIZE);

        put_rx_desc(&mut *state.rx_queue, desc_idx, original_buf);
        // Notify device so it can reuse the buffer
        notify_queue(&*state, 0);
    }
}

/// Flush pending RX buffer notifications to device
pub fn flush_rx() {
    unsafe {
        if NET_STATE.is_null() { return; }
        notify_queue(&*NET_STATE, 0);
    }
}

/// Poll for packets and print info (for debugging)
pub fn poll() -> usize {
    let mut count: usize = 0;
    while let Some((buf, len, desc_idx)) = receive() {
        count += 1;
        // For now just print packet info
        if count <= 3 {
            let uart = UART;
            uart::puts(uart, "  RX pkt ");
            crate::print_dec_u32(uart, count as u32);
            uart::puts(uart, ": len=");
            crate::print_dec_u32(uart, len as u32);
            uart::puts(uart, "\r\n");
        }
        release_rx_buffer(buf, desc_idx);
    }
    count
}

/// Minimal RX queue debug (avoids print_dec_u32 which triggers TCG hang)
pub fn dump_rx_debug() {
    unsafe {
        if NET_STATE.is_null() { uart::puts(UART, "Net: not init\r\n"); return; }
        let state = &*NET_STATE;
        let rx = &*state.rx_queue;
        let avail_idx = (*rx.avail_ring).idx.load(Ordering::Relaxed);
        let used_idx = (*rx.used_ring).idx.load(Ordering::Relaxed);

        uart::puts(UART, "RX desc=0x");
        crate::print_hex(UART, rx.desc_table as u32);
        uart::puts(UART, " avail_idx=0x");
        crate::print_hex(UART, avail_idx as u32);
        uart::puts(UART, " used_idx=0x");
        crate::print_hex(UART, used_idx as u32);
        uart::puts(UART, " free=0x");
        crate::print_hex(UART, rx.free_head as u32);
        uart::puts(UART, "\r\n");
        // Check first descriptor
        let d0 = &*rx.desc_table;
        let d0_addr = d0.addr.load(Ordering::Relaxed);
        let d0_len = d0.len.load(Ordering::Relaxed);
        uart::puts(UART, "desc[0]: addr=0x");
        crate::print_hex(UART, d0_addr as u32);
        uart::puts(UART, " len=0x");
        crate::print_hex(UART, d0_len);
        uart::puts(UART, "\r\n");

        // Read back what the device sees for queue 0 (RX)
        let common_ptr = state.common as *mut CommonCfg;
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*common_ptr).queue_select), 0u16);
        let dev_desc = core::ptr::read_volatile(core::ptr::addr_of!((*common_ptr).queue_desc));
        let dev_avail = core::ptr::read_volatile(core::ptr::addr_of!((*common_ptr).queue_driver));
        let dev_used = core::ptr::read_volatile(core::ptr::addr_of!((*common_ptr).queue_device));
        let dev_size = core::ptr::read_volatile(core::ptr::addr_of!((*common_ptr).queue_size));
        let dev_enable = core::ptr::read_volatile(core::ptr::addr_of!((*common_ptr).queue_enable));
        uart::puts(UART, "dev Q0: desc=0x");
        crate::print_hex(UART, dev_desc as u32);
        uart::puts(UART, " avail=0x");
        crate::print_hex(UART, dev_avail as u32);
        uart::puts(UART, " used=0x");
        crate::print_hex(UART, dev_used as u32);
        uart::puts(UART, " size=0x");
        crate::print_hex(UART, dev_size as u32);
        uart::puts(UART, " en=0x");
        crate::print_hex(UART, dev_enable as u32);
        uart::puts(UART, "\r\n");

        // Read device status
        let dev_status = core::ptr::read_volatile(core::ptr::addr_of!((*common_ptr).device_status));
        uart::puts(UART, "dev_status=0x");
        crate::print_hex_byte(UART, dev_status);
        uart::puts(UART, "\r\n");

        // Check available ring entries
        let avail_ring_ptr = rx.avail_ring as *const u8;
        let flags = core::ptr::read_volatile(avail_ring_ptr as *const u16);
        let ring_idx = core::ptr::read_volatile(avail_ring_ptr.add(2) as *const u16);
        uart::puts(UART, "avail: flags=0x");
        crate::print_hex(UART, flags as u32);
        uart::puts(UART, " idx=0x");
        crate::print_hex(UART, ring_idx as u32);
        uart::puts(UART, " [0]=0x");
        let e0 = core::ptr::read_volatile((avail_ring_ptr.add(4) as *const u16));
        crate::print_hex(UART, e0 as u32);
        uart::puts(UART, " [1]=0x");
        let e1 = core::ptr::read_volatile((avail_ring_ptr.add(4) as *const u16).add(1));
        crate::print_hex(UART, e1 as u32);
        uart::puts(UART, "\r\n");
    }
}

/// Dump virtio-net driver state for debugging
pub fn dump_state() {
    unsafe {
        if NET_STATE.is_null() { uart::puts(UART, "Net not initialized\r\n"); return; }
        let state = &*NET_STATE;
        let u = UART;

        uart::puts(u, "=== VirtIO-Net State ===\r\n");

        // MAC
        uart::puts(u, "MAC: ");
        for (i, b) in state.mac.iter().enumerate() {
            if i > 0 { uart::putc(u, b':'); }
            crate::print_hex_byte(u, *b);
        }
        uart::puts(u, "\r\n");

        // Common config address
        uart::puts(u, "CommonCfg: 0x");
        crate::print_hex(u, state.common as u32);
        uart::puts(u, "\r\n");

        // Notification config
        uart::puts(u, "Notify: base=0x");
        crate::print_hex(u, state.notify_base as u32);
        uart::puts(u, " multiplier=");
        crate::print_dec_u32(u, state.notify_multiplier);
        uart::puts(u, "\r\n");

        // Read device status
        let common = &*state.common;
        uart::puts(u, "Device status: 0x");
        crate::print_hex_byte(u, common.device_status);
        uart::puts(u, "\r\n");

        // RX queue (queue 0)
        let rx = &*state.rx_queue;
        let rx_avail_idx = (*rx.avail_ring).idx.load(Ordering::Relaxed);
        let rx_used_idx = (*rx.used_ring).idx.load(Ordering::Relaxed);
        uart::puts(u, "RX Queue:\r\n");
        uart::puts(u, "  desc=0x");
        crate::print_hex(u, rx.desc_table as u32);
        uart::puts(u, " avail=0x");
        crate::print_hex(u, rx.avail_ring as u32);
        uart::puts(u, " used=0x");
        crate::print_hex(u, rx.used_ring as u32);
        uart::puts(u, "\r\n  size=");
        crate::print_dec_u32(u, rx.queue_size as u32);
        uart::puts(u, " free_head=");
        crate::print_dec_u32(u, rx.free_head as u32);
        uart::puts(u, " last_used=");
        crate::print_dec_u32(u, rx.last_used_idx as u32);
        uart::puts(u, "\r\n  avail_idx=");
        crate::print_dec_u32(u, rx_avail_idx as u32);
        uart::puts(u, " used_idx=");
        crate::print_dec_u32(u, rx_used_idx as u32);
        uart::puts(u, "\r\n");

        // Check first few RX descriptor addresses
        uart::puts(u, "  RX desc[0]: addr=0x");
        let d0 = &*rx.desc_table.add(0);
        crate::print_hex(u, d0.addr.load(Ordering::Relaxed) as u32);
        uart::puts(u, " len=");
        crate::print_dec_u32(u, d0.len.load(Ordering::Relaxed));
        uart::puts(u, " flags=0x");
        crate::print_hex_byte(u, d0.flags.load(Ordering::Relaxed) as u8);
        uart::puts(u, "\r\n");

        // TX queue (queue 1)
        let tx = &*state.tx_queue;
        let tx_avail_idx = (*tx.avail_ring).idx.load(Ordering::Relaxed);
        let tx_used_idx = (*tx.used_ring).idx.load(Ordering::Relaxed);
        uart::puts(u, "TX Queue:\r\n");
        uart::puts(u, "  desc=0x");
        crate::print_hex(u, tx.desc_table as u32);
        uart::puts(u, " avail=0x");
        crate::print_hex(u, tx.avail_ring as u32);
        uart::puts(u, " used=0x");
        crate::print_hex(u, tx.used_ring as u32);
        uart::puts(u, "\r\n  size=");
        crate::print_dec_u32(u, tx.queue_size as u32);
        uart::puts(u, " free_head=");
        crate::print_dec_u32(u, tx.free_head as u32);
        uart::puts(u, " last_used=");
        crate::print_dec_u32(u, tx.last_used_idx as u32);
        uart::puts(u, "\r\n  avail_idx=");
        crate::print_dec_u32(u, tx_avail_idx as u32);
        uart::puts(u, " used_idx=");
        crate::print_dec_u32(u, tx_used_idx as u32);
        uart::puts(u, "\r\n");

        // Read queue config from device for both queues
        let common_ptr = state.common as *mut CommonCfg;
        for q in 0..2u16 {
            core::ptr::write_volatile(core::ptr::addr_of_mut!((*common_ptr).queue_select), q);
            let qs = core::ptr::read_volatile(core::ptr::addr_of!((*common_ptr).queue_size));
            let qe = core::ptr::read_volatile(core::ptr::addr_of!((*common_ptr).queue_enable));
            let qno = core::ptr::read_volatile(core::ptr::addr_of!((*common_ptr).queue_notify_off));
            let qd = core::ptr::read_volatile(core::ptr::addr_of!((*common_ptr).queue_desc));
            let qa = core::ptr::read_volatile(core::ptr::addr_of!((*common_ptr).queue_driver));
            let qu = core::ptr::read_volatile(core::ptr::addr_of!((*common_ptr).queue_device));
            uart::puts(u, "  Q");
            crate::print_dec_u32(u, q as u32);
            uart::puts(u, ": size=");
            crate::print_dec_u32(u, qs as u32);
            uart::puts(u, " enable=");
            crate::print_dec_u32(u, qe as u32);
            uart::puts(u, " notify_off=");
            crate::print_dec_u32(u, qno as u32);
            uart::puts(u, "\r\n    desc=0x");
            crate::print_hex(u, qd as u32);
            uart::puts(u, " avail=0x");
            crate::print_hex(u, qa as u32);
            uart::puts(u, " used=0x");
            crate::print_hex(u, qu as u32);
            uart::puts(u, "\r\n");
        }
    }
}
