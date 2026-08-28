//! xHCI USB Host Controller Driver
//!
//! Implements a minimal xHCI driver for USB CDC-ECM ethernet support.
//! Reference: Redox OS xhcid + xHCI spec 1.2

#![allow(dead_code)]

use core::alloc::Layout;

#[cfg(not(feature = "board-redfin"))]
use crate::uart;
#[cfg(feature = "board-redfin")]
use crate::qup_uart as uart;

use crate::platform::UART;

// DMA allocation (same pattern as net.rs/blk.rs)
fn dma_alloc(size: usize) -> Option<usize> {
    let page_size = 4096;
    let aligned_size = ((size + page_size - 1) / page_size) * page_size;
    let layout = Layout::from_size_align(aligned_size, page_size).ok()?;
    unsafe {
        let ptr = alloc::alloc::alloc_zeroed(layout);
        if ptr.is_null() { None } else { Some(ptr as usize) }
    }
}

// === xHCI Register Offsets ===
// Capability registers (at MMIO base)
const CAP_LEN: usize = 0x00;        // u8, offset to operational regs
const CAP_HCIVERSION: usize = 0x02; // u16
const CAP_HCSPARAMS1: usize = 0x04; // u32: MaxSlots[7:0], MaxIntrs[18:8], MaxPorts[31:24]
const CAP_HCSPARAMS2: usize = 0x08; // u32: ERSTMax[7:4], MaxScratchPadBufs
const CAP_HCCPARAMS1: usize = 0x10; // u32: AC64[0], CSZ[2], xECP[31:16]
const CAP_DBOFF: usize = 0x14;      // u32: doorbell offset
const CAP_RTSOFF: usize = 0x18;     // u32: runtime register space offset

// Operational register offsets (from op_base = base + cap_len)
const OP_USBCMD: usize = 0x00;
const OP_USBSTS: usize = 0x04;
const OP_PAGESIZE: usize = 0x08;
const OP_CRCR_LO: usize = 0x18;
const OP_CRCR_HI: usize = 0x1C;
const OP_DCBAAP_LO: usize = 0x30;
const OP_DCBAAP_HI: usize = 0x34;
const OP_CONFIG: usize = 0x38;

// Port register offsets (from op_base + 0x400)
const PORT_SC: usize = 0x00;    // Port Status and Control
const PORT_PMSC: usize = 0x04;  // Port Power Management
const PORT_LI: usize = 0x08;    // Port Link Info

// Runtime register offsets (from run_base = base + rts_off)
const RUN_MFINDEX: usize = 0x00;
// Interrupter 0 at run_base + 0x20
const INT_IMAN: usize = 0x00;
const INT_IMOD: usize = 0x04;
const INT_ERSTSZ: usize = 0x08;
const INT_ERSTBA_LO: usize = 0x10;
const INT_ERSTBA_HI: usize = 0x14;
const INT_ERDP_LO: usize = 0x18;
const INT_ERDP_HI: usize = 0x1C;

// USBCMD bits
const USBCMD_RS: u32 = 1 << 0;     // Run/Stop
const USBCMD_HCRST: u32 = 1 << 1;  // Host Controller Reset
const USBCMD_INTE: u32 = 1 << 2;   // Interrupter Enable

// USBSTS bits
const USBSTS_HCH: u32 = 1 << 0;    // Halted
const USBSTS_CNR: u32 = 1 << 11;   // Controller Not Ready

// PORTSC bits
const PORTSC_CCS: u32 = 1 << 0;    // Current Connect Status
const PORTSC_PED: u32 = 1 << 1;    // Port Enabled/Disabled
const PORTSC_PR: u32 = 1 << 4;     // Port Reset
const PORTSC_PP: u32 = 1 << 9;     // Port Power
const PORTSC_SPEED_MASK: u32 = 0x3C0; // Speed [9:10] actually bits 10-13
const PORTSC_SPEED_SHIFT: u32 = 10;
const PORTSC_CSC: u32 = 1 << 17;   // Connect Status Change
const PORTSC_PRC: u32 = 1 << 21;   // Port Reset Change
const PORTSC_WRC: u32 = 1 << 19;   // Warm Port Reset Change

// TRB types (bits [15:10] of a TRB)
const TRB_TYPE_ENABLE_SLOT: u32 = 9;
const TRB_TYPE_ADDRESS_DEV: u32 = 11;
const TRB_TYPE_CONFIGURE_EP: u32 = 12;
const TRB_TYPE_EVAL_CTX: u32 = 13;
const TRB_TYPE_NOOP: u32 = 23;
const TRB_TYPE_LINK: u32 = 32;
const TRB_TYPE_NORMAL: u32 = 1;
const TRB_TYPE_SETUP_STAGE: u32 = 2;
const TRB_TYPE_DATA_STAGE: u32 = 3;
const TRB_TYPE_STATUS_STAGE: u32 = 4;

// TRB completion codes
const TRB_CC_SUCCESS: u8 = 1;
const TRB_CC_INVALID: u8 = 0;
const TRB_CC_SHORT_PACKET: u8 = 13;

// TRB event types
const TRB_EV_TRANSFER: u32 = 32;
const TRB_EV_CMD_COMPLETE: u32 = 33;
const TRB_EV_PORT_CHANGE: u32 = 34;

// Endpoint directions
const EP_OUT: u8 = 0;
const EP_IN: u8 = 1;

/// TRB (Transfer Request Block) - 16 bytes
#[repr(C, align(16))]
pub(crate) struct Trb {
    pub(crate) param: u64,   // Parameter (data pointer or other)
    pub(crate) status: u32,  // Status (length, etc.)
    pub(crate) control: u32, // Control (TRB type, flags, etc.)
}

/// Event Ring Segment Table Entry
#[repr(C)]
struct ErstEntry {
    addr_lo: u32,
    addr_hi: u32,
    size: u32,
    _reserved: u32,
}

/// Input Context for Address Device / Configure Endpoint
/// Layout: Input Control (8 bytes) + Slot Context (32 bytes) + Endpoint Contexts (32 bytes each)
#[allow(dead_code)]
#[repr(C, align(64))]
struct InputContext {
    drop_flags: u32,    // Context entries to drop
    add_flags: u32,     // Context entries to add
    _rsvd: [u32; 6],    // Reserved
    // Slot context follows at offset 0x20
    slot: [u32; 8],
    // Endpoint 0 context at offset 0x40
    ep0: [u32; 8],
    // More endpoint contexts follow...
}

/// xHCI controller state
#[allow(dead_code)]
pub(crate) struct XhciState {
    pub(crate) base: usize,          // MMIO base address
    pub(crate) op_base: usize,       // Operational register base
    pub(crate) run_base: usize,      // Runtime register base
    pub(crate) db_base: usize,       // Doorbell register base
    pub(crate) port_base: usize,     // Port register base
    pub(crate) max_slots: u8,
    pub(crate) max_ports: u8,
    pub(crate) page_size: u32,
    pub(crate) context_size: usize,  // 32 or 64 bytes

    // DMA buffers
    pub(crate) cmd_ring: usize,      // Command ring (page of TRBs)
    pub(crate) cmd_ring_size: usize, // Number of TRBs
    pub(crate) cmd_ring_idx: usize,
    pub(crate) cmd_cycle: bool,

    pub(crate) event_ring: usize,    // Event ring
    pub(crate) event_ring_size: usize,
    pub(crate) event_ring_idx: usize,
    pub(crate) event_cycle: bool,
    pub(crate) erst: usize,          // Event Ring Segment Table

    pub(crate) dcbaa: usize,         // Device Context Base Address Array
    pub(crate) scratchpad_buf: usize,// Scratchpad buffer array (if needed)

    // USB device state (single device for CDC-ECM)
    pub(crate) device_slot: u8,      // Allocated slot ID (0 = none)
    pub(crate) device_port: u8,      // Port number
    pub(crate) device_addr: u8,      // USB address
    pub(crate) ep0_ring: usize,      // EP0 transfer ring
    pub(crate) ep0_idx: usize,
    pub(crate) ep0_cycle: bool,
    pub(crate) ctrl_buf: usize,      // DMA buffer for control transfers
    pub(crate) bulk_in_ring: usize,  // Transfer ring for bulk IN
    pub(crate) bulk_out_ring: usize, // Transfer ring for bulk OUT
    pub(crate) bulk_in_idx: usize,
    pub(crate) bulk_out_idx: usize,
    pub(crate) bulk_in_cycle: bool,
    pub(crate) bulk_out_cycle: bool,
    pub(crate) bulk_in_dbl: u32,     // Doorbell target for bulk IN
    pub(crate) bulk_out_dbl: u32,    // Doorbell target for bulk OUT
    pub(crate) max_packet: u16,      // Bulk endpoint max packet size

    // Device contexts (DMA)
    pub(crate) input_ctx: usize,     // Input context buffer
    pub(crate) device_ctx: usize,    // Device context buffer (stored in DCBAA)
}

static mut XHCI_STATE: Option<XhciState> = None;

/// Invalidate CPU data cache for a range (needed before reading DMA buffers)
/// On ARMv8, DMA writes go to main memory but CPU may read stale cached data
pub(crate) unsafe fn cache_invalidate(addr: usize, len: usize) {
    const CACHE_LINE: usize = 64;
    let start = addr & !(CACHE_LINE - 1);
    let end = (addr + len + CACHE_LINE - 1) & !(CACHE_LINE - 1);
    let mut a = start;
    while a < end {
        core::arch::asm!("dc ivac, {}", in(reg) a);
        a += CACHE_LINE;
    }
    core::arch::asm!("dsb sy", "isb");
}

/// Clean CPU data cache for a range (needed before controller reads DMA buffers)
pub(crate) unsafe fn cache_clean(addr: usize, len: usize) {
    const CACHE_LINE: usize = 64;
    let start = addr & !(CACHE_LINE - 1);
    let end = (addr + len + CACHE_LINE - 1) & !(CACHE_LINE - 1);
    let mut a = start;
    while a < end {
        core::arch::asm!("dc cvac, {}", in(reg) a);
        a += CACHE_LINE;
    }
    core::arch::asm!("dsb sy", "isb");
}

/// Clean and invalidate cache for a range
#[allow(dead_code)]
unsafe fn cache_clean_invalidate(addr: usize, len: usize) {
    const CACHE_LINE: usize = 64;
    let start = addr & !(CACHE_LINE - 1);
    let end = (addr + len + CACHE_LINE - 1) & !(CACHE_LINE - 1);
    let mut a = start;
    while a < end {
        core::arch::asm!("dc civac, {}", in(reg) a);
        a += CACHE_LINE;
    }
    core::arch::asm!("dsb sy", "isb");
}

unsafe fn mmio_read32(base: usize, offset: usize) -> u32 {
    core::ptr::read_volatile((base + offset) as *const u32)
}

unsafe fn mmio_write32(base: usize, offset: usize, val: u32) {
    core::ptr::write_volatile((base + offset) as *mut u32, val)
}

unsafe fn mmio_read64(base: usize, offset: usize) -> u64 {
    let lo = mmio_read32(base, offset) as u64;
    let hi = mmio_read32(base, offset + 4) as u64;
    (hi << 32) | lo
}

unsafe fn mmio_write64(base: usize, offset: usize, val: u64) {
    mmio_write32(base, offset, val as u32);
    mmio_write32(base, offset + 4, (val >> 32) as u32);
}

/// Wait for a condition with timeout
fn wait_for<F: Fn() -> bool>(f: F, max_iters: usize) -> bool {
    for _ in 0..max_iters {
        if f() { return true; }
        core::hint::spin_loop();
    }
    false
}

/// Build a TRB control field from type, toggle cycle, and slot
#[allow(dead_code)]
fn trb_control(trb_type: u32, cycle: bool, slot: u8) -> u32 {
    let cycle_bit = if cycle { 1 } else { 0 };
    (trb_type << 10) | (cycle_bit) | ((slot as u32) << 24)
}

/// Initialize xHCI controller
pub fn init(uart_base: usize) {
    let info = match crate::pci::get_xhci() {
        Some(i) => i,
        None => {
            uart::puts(uart_base, "[SKIP] xHCI: no USB controller\r\n");
            return;
        }
    };

    uart::puts(uart_base, "[..] xHCI: probing\r\n");

    // Read BAR for MMIO base (supports 32-bit and 64-bit BARs)
    let bar_val = {
        let offset = 0x10;
        let bar = crate::pci::config_read32(info.bus, info.dev, info.func, offset as u8);
        let is_64bit = (bar & 0x6) == 0x4; // memory, 64-bit

        // Get size by writing all 1s
        crate::pci::config_write32(info.bus, info.dev, info.func, offset as u8, 0xFFFF_FFFF);
        let size_val = crate::pci::config_read32(info.bus, info.dev, info.func, offset as u8);
        let size_lo = if size_val == 0 { 0 } else { !((size_val & 0xFFFF_FFF0) as usize) + 1 };
        let mut size = size_lo;

        if is_64bit {
            let offset_hi = offset + 4;
            crate::pci::config_write32(info.bus, info.dev, info.func, offset_hi as u8, 0xFFFF_FFFF);
            let size_hi_val = crate::pci::config_read32(info.bus, info.dev, info.func, offset_hi as u8);
            let size_hi = if size_hi_val == 0 { 0 } else { !((size_hi_val) as usize) + 1 };
            size = ((size_hi as u64) << 32 | size_lo as u64) as usize;
            let _ = size_hi;
        }

        // Allocate BAR at fixed MMIO address
        let addr = 0x1200_0000usize;
        crate::pci::config_write32(info.bus, info.dev, info.func, offset as u8, (addr as u32) | (bar & 0xF));
        if is_64bit {
            crate::pci::config_write32(info.bus, info.dev, info.func, (offset + 4) as u8, (addr >> 32) as u32);
        }
        let _ = size;
        addr
    };

    let base = bar_val;
    unsafe {
        // Read capability registers
        let cap_len = mmio_read32(base, CAP_LEN) as u8;
        let hci_ver = (mmio_read32(base, CAP_LEN) >> 16) as u16;
        let hcs_params1 = mmio_read32(base, CAP_HCSPARAMS1);
        let hcc_params1 = mmio_read32(base, CAP_HCCPARAMS1);
        let db_off = mmio_read32(base, CAP_DBOFF) & 0xFFFF_FFFC;
        let rts_off = mmio_read32(base, CAP_RTSOFF) & 0xFFFF_FFE0;

        let max_slots = (hcs_params1 & 0xFF) as u8;
        let max_ports = ((hcs_params1 >> 24) & 0xFF) as u8;
        let context_size = if (hcc_params1 & 0x04) != 0 { 64 } else { 32 };
        let ac64 = (hcc_params1 & 0x01) != 0;
        uart::puts(uart_base, "  HCC1=0x");
        crate::print_hex(uart_base, hcc_params1);
        uart::puts(uart_base, " AC64=");
        uart::putc(uart_base, if ac64 { b'Y' } else { b'N' });
        uart::puts(uart_base, " pagesz=");
        uart::puts(uart_base, "\r\n");

        let op_base = base + cap_len as usize;
        let run_base = base + rts_off as usize;
        let db_base = base + db_off as usize;
        let port_base = op_base + 0x400;

        uart::puts(uart_base, "  xHCI v");
        crate::print_dec_u32(uart_base, (hci_ver >> 8) as u32);
        uart::putc(uart_base, b'.');
        crate::print_dec_u32(uart_base, (hci_ver & 0xFF) as u32);
        uart::puts(uart_base, " slots=");
        crate::print_dec_u32(uart_base, max_slots as u32);
        uart::puts(uart_base, " ports=");
        crate::print_dec_u32(uart_base, max_ports as u32);
        uart::puts(uart_base, "\r\n");

        // 1. Halt controller
        mmio_write32(op_base, OP_USBCMD, mmio_read32(op_base, OP_USBCMD) & !USBCMD_RS);
        if !wait_for(|| mmio_read32(op_base, OP_USBSTS) & USBSTS_HCH != 0, 100000) {
            uart::puts(uart_base, "[FAIL] xHCI: halt timeout\r\n");
            return;
        }

        // 2. Reset controller
        mmio_write32(op_base, OP_USBCMD, USBCMD_HCRST);
        if !wait_for(|| mmio_read32(op_base, OP_USBCMD) & USBCMD_HCRST == 0, 100000) {
            uart::puts(uart_base, "[FAIL] xHCI: reset timeout\r\n");
            return;
        }
        if !wait_for(|| mmio_read32(op_base, OP_USBSTS) & USBSTS_CNR == 0, 100000) {
            uart::puts(uart_base, "[FAIL] xHCI: not ready after reset\r\n");
            return;
        }

        // 3. Read page size (encoded as 2^(n+12))
        let page_reg = mmio_read32(op_base, OP_PAGESIZE);
        let page_size = if page_reg == 0 { 4096 } else { 1 << ((page_reg.trailing_zeros() + 12) as usize) };

        // 4. Allocate command ring (one page = 256 TRBs)
        let cmd_ring = match dma_alloc(4096) {
            Some(a) => a,
            None => { uart::puts(uart_base, "[FAIL] xHCI: cmd ring alloc\r\n"); return; }
        };

        // 5. Allocate event ring (one page = 256 TRBs) + ERST
        let event_ring = match dma_alloc(4096) {
            Some(a) => a,
            None => { uart::puts(uart_base, "[FAIL] xHCI: evt ring alloc\r\n"); return; }
        };
        let erst = match dma_alloc(4096) {
            Some(a) => a,
            None => { uart::puts(uart_base, "[FAIL] xHCI: erst alloc\r\n"); return; }
        };
        // Write ERST entry 0
        let erst_ptr = erst as *mut ErstEntry;
        core::ptr::write_volatile(&mut (*erst_ptr).addr_lo, event_ring as u32);
        core::ptr::write_volatile(&mut (*erst_ptr).addr_hi, (event_ring >> 32) as u32);
        core::ptr::write_volatile(&mut (*erst_ptr).size, 256); // 256 TRBs
        core::ptr::write_volatile(&mut (*erst_ptr)._reserved, 0);

        // 6. Allocate DCBAA (256 * 8 bytes = 2KB)
        let dcbaa = match dma_alloc(4096) {
            Some(a) => a,
            None => { uart::puts(uart_base, "[FAIL] xHCI: dcbaa alloc\r\n"); return; }
        };

        // 7. Allocate input context and device context
        let input_ctx = match dma_alloc(4096) {
            Some(a) => a,
            None => { uart::puts(uart_base, "[FAIL] xHCI: input_ctx alloc\r\n"); return; }
        };
        let device_ctx = match dma_alloc(4096) {
            Some(a) => a,
            None => { uart::puts(uart_base, "[FAIL] xHCI: device_ctx alloc\r\n"); return; }
        };

        // 8. Allocate transfer rings for bulk endpoints
        let bulk_in_ring = match dma_alloc(4096) {
            Some(a) => a,
            None => { uart::puts(uart_base, "[FAIL] xHCI: bulk_in ring alloc\r\n"); return; }
        };
        let bulk_out_ring = match dma_alloc(4096) {
            Some(a) => a,
            None => { uart::puts(uart_base, "[FAIL] xHCI: bulk_out ring alloc\r\n"); return; }
        };

        // 8b. Allocate EP0 transfer ring and control buffer
        let ep0_ring = match dma_alloc(4096) {
            Some(a) => a,
            None => { uart::puts(uart_base, "[FAIL] xHCI: ep0 ring alloc\r\n"); return; }
        };
        let ctrl_buf = match dma_alloc(4096) {
            Some(a) => a,
            None => { uart::puts(uart_base, "[FAIL] xHCI: ctrl_buf alloc\r\n"); return; }
        };
        // Initialize transfer rings with Link TRBs
        init_transfer_ring(bulk_in_ring, 256, true);
        init_transfer_ring(bulk_out_ring, 256, true);
        init_transfer_ring(ep0_ring, 256, true);

        // 8c. Init command ring with Link TRB at last entry
        init_transfer_ring(cmd_ring, 256, true);
        cache_clean(cmd_ring, 4096);

        // 9. Write CRCR (Command Ring Control Register)
        // Set RCS (Ring Cycle State) = 1, and address
        let crcr = (cmd_ring as u64) | 1; // cycle bit = 1
        mmio_write64(op_base, OP_CRCR_LO, crcr);

        // 10. Configure interrupter 0
        let int_base = run_base + 0x20; // Interrupter 0 at run_base + 0x20
        mmio_write32(int_base, INT_ERSTSZ, 1); // 1 segment
        mmio_write64(int_base, INT_ERSTBA_LO, erst as u64);
        mmio_write64(int_base, INT_ERDP_LO, event_ring as u64 | (1 << 3)); // clear EHB
        // Enable interrupter
        mmio_write32(int_base, INT_IMAN, 0x02); // IE bit = 1

        // 11. Set CONFIG register (max device slots enabled)
        let max_slots_val = max_ports.min(max_slots) as u32;
        mmio_write32(op_base, OP_CONFIG, max_slots_val);

        // 12. Write DCBAAP
        mmio_write64(op_base, OP_DCBAAP_LO, dcbaa as u64);

        // 13. Start controller
        mmio_write32(op_base, OP_USBCMD, USBCMD_RS | USBCMD_INTE);
        if !wait_for(|| mmio_read32(op_base, OP_USBSTS) & USBSTS_HCH == 0, 100000) {
            uart::puts(uart_base, "[FAIL] xHCI: start timeout\r\n");
            return;
        }

        // 14. Wait for ports to settle
        for _ in 0..500000 { core::hint::spin_loop(); }

        // Save state
        XHCI_STATE = Some(XhciState {
            base, op_base, run_base, db_base, port_base,
            max_slots, max_ports, page_size, context_size,
            cmd_ring, cmd_ring_size: 256, cmd_ring_idx: 0, cmd_cycle: true,
            event_ring, event_ring_size: 256, event_ring_idx: 0, event_cycle: true,
            erst, dcbaa, scratchpad_buf: 0,
            device_slot: 0, device_port: 0, device_addr: 0,
            ep0_ring, ep0_idx: 0, ep0_cycle: true, ctrl_buf,
            bulk_in_ring, bulk_out_ring,
            bulk_in_idx: 0, bulk_out_idx: 0,
            bulk_in_cycle: true, bulk_out_cycle: true,
            bulk_in_dbl: 0, bulk_out_dbl: 0,
            max_packet: 64,
            input_ctx, device_ctx,
        });

        uart::puts(uart_base, "[OK] xHCI initialized\r\n");
    }
}

/// Ring doorbell for slot/endpoint
/// DB Target [7:0] = endpoint ID (DCI), Stream ID [31:16] = 0 for non-stream
pub(crate) unsafe fn ring_doorbell(slot: u8, endpoint: u8, db_base: usize) {
    // Ensure all previous writes (TRBs, contexts) are visible before doorbell
    core::arch::asm!("dsb sy", "isb");
    let val = (endpoint as u32) & 0xFF;
    mmio_write32(db_base, (slot as usize) * 4, val);
    // Ensure doorbell write completes
    core::arch::asm!("dsb sy", "isb");
}

/// Poll event ring for a command completion
/// Returns (completion_code, slot_id, trb_pointer)
unsafe fn poll_command_complete(state: &mut XhciState, uart_base: usize) -> Option<(u8, u8, u64)> {
    for _ in 0..500000 {
        // Invalidate cache before reading DMA-written event ring entries
        cache_invalidate(state.event_ring + state.event_ring_idx * 16, 64);

        let idx = state.event_ring_idx;
        let trb_ptr = (state.event_ring as *const Trb).add(idx);
        let trb = &*trb_ptr;

        // Check cycle bit
        let trb_cycle = (trb.control & 1) != 0;
        if trb_cycle != state.event_cycle {
            core::hint::spin_loop();
            continue; // Not our cycle
        }

        let trb_type = (trb.control >> 10) & 0x3F;
        let completion_code = (trb.status >> 24) as u8;
        let slot_id = ((trb.control >> 24) & 0xFF) as u8;

        // Advance event ring index
        state.event_ring_idx = idx + 1;
        if state.event_ring_idx >= state.event_ring_size {
            state.event_ring_idx = 0;
            state.event_cycle = !state.event_cycle;
        }

        // Update ERDP
        let int_base = state.run_base + 0x20;
        let deque_ptr = (state.event_ring + (idx + 1) * 16) as u64;
        mmio_write64(int_base, INT_ERDP_LO, deque_ptr | (1 << 3));

        if trb_type == TRB_EV_CMD_COMPLETE {
            return Some((completion_code, slot_id, trb.param));
        }
        if trb_type == TRB_EV_PORT_CHANGE {
            // Port status change event - skip, keep polling
            continue;
        }
        if trb_type == TRB_EV_TRANSFER {
            // Transfer event - skip for now
            continue;
        }

        core::hint::spin_loop();
    }

    // Debug: show event ring state on timeout
    cache_invalidate(state.event_ring, 4096);
    uart::puts(uart_base, "  evt timeout: idx=");
    crate::print_dec_u32(uart_base, state.event_ring_idx as u32);
    let dbg_trb = (state.event_ring as *const Trb).add(state.event_ring_idx);
    uart::puts(uart_base, " ctl=0x");
    crate::print_hex(uart_base, core::ptr::read_volatile(&(*dbg_trb).control));
    uart::puts(uart_base, " sts=0x");
    crate::print_hex(uart_base, core::ptr::read_volatile(&(*dbg_trb).status));
    uart::puts(uart_base, "\r\n");
    // Show USBSTS
    let usbsts = mmio_read32(state.op_base, OP_USBSTS);
    uart::puts(uart_base, "  USBSTS=0x");
    crate::print_hex(uart_base, usbsts);
    uart::puts(uart_base, "\r\n");
    None
}

/// Send a command TRB and wait for completion
unsafe fn send_command(state: &mut XhciState, uart_base: usize, param: u64, status: u32, control: u32) -> Option<(u8, u8)> {
    // Write TRB to command ring
    let idx = state.cmd_ring_idx;
    let trb_ptr = (state.cmd_ring as *mut Trb).add(idx);
    core::ptr::write_volatile(&mut (*trb_ptr).param, param);
    core::ptr::write_volatile(&mut (*trb_ptr).status, status);
    // Set cycle bit in control
    let cycle_bit = if state.cmd_cycle { 1u32 } else { 0 };
    core::ptr::write_volatile(&mut (*trb_ptr).control, control | cycle_bit);

    // Advance command ring index
    state.cmd_ring_idx = (idx + 1) % state.cmd_ring_size;
    if state.cmd_ring_idx == 0 {
        state.cmd_cycle = !state.cmd_cycle;
    }

    // Clean command ring cache so RAM (software wrote, controller reads via DMA)
    cache_clean(state.cmd_ring + idx * 16, 16);
    // Ring command doorbell (slot 0, target 0)
    ring_doorbell(0, 0, state.db_base);

    // Wait for command completion
    poll_command_complete(state, uart_base).map(|(cc, slot, _)| (cc, slot))
}

/// Initialize a transfer ring by writing a Link TRB at the end
unsafe fn init_transfer_ring(ring_addr: usize, ring_size: usize, cycle: bool) {
    // Write a Link TRB at last index pointing back to start
    let link_trb = (ring_addr as *mut Trb).add(ring_size - 1);
    let cycle_bit = if cycle { 1 } else { 0 };
    core::ptr::write_volatile(&mut (*link_trb).param, ring_addr as u64);
    core::ptr::write_volatile(&mut (*link_trb).status, 0);
    core::ptr::write_volatile(&mut (*link_trb).control, (TRB_TYPE_LINK << 10) | cycle_bit | (1 << 1)); // Toggle Cycle
}

/// Poll event ring for a transfer event on given slot
/// Returns (completion_code, transfer_length)
unsafe fn poll_transfer_event(state: &mut XhciState, uart_base: usize, slot_id: u8) -> Option<(u8, u32)> {
    for i in 0..500000 {
        // Invalidate event ring cache before reading (controller writes via DMA)
        cache_invalidate(state.event_ring + state.event_ring_idx * 16, 64);
        let idx = state.event_ring_idx;
        let trb_ptr = (state.event_ring as *const Trb).add(idx);
        let trb = &*trb_ptr;

        let trb_cycle = (trb.control & 1) != 0;
        if trb_cycle != state.event_cycle {
            // Debug: show first few misses
            if i == 0 {
                uart::puts(uart_base, "  xfer_poll: idx=");
                crate::print_dec_u32(uart_base, idx as u32);
                uart::puts(uart_base, " expect_cycle=");
                uart::putc(uart_base, if state.event_cycle { b'1' } else { b'0' });
                uart::puts(uart_base, " got_cycle=");
                uart::putc(uart_base, if trb_cycle { b'1' } else { b'0' });
                uart::puts(uart_base, " ctl=0x");
                crate::print_hex(uart_base, trb.control);
                uart::puts(uart_base, "\r\n");
            }
            core::hint::spin_loop();
            continue;
        }

        let trb_type = (trb.control >> 10) & 0x3F;
        let cc = (trb.status >> 24) as u8;
        let evt_slot = ((trb.control >> 24) & 0xFF) as u8;
        let transfer_len = trb.status & 0xFFFFFF;

        // Advance event ring
        state.event_ring_idx = idx + 1;
        if state.event_ring_idx >= state.event_ring_size {
            state.event_ring_idx = 0;
            state.event_cycle = !state.event_cycle;
        }
        let int_base = state.run_base + 0x20;
        let deque_ptr = (state.event_ring + (idx + 1) * 16) as u64;
        mmio_write64(int_base, INT_ERDP_LO, deque_ptr | (1 << 3));

        if trb_type == TRB_EV_TRANSFER && evt_slot == slot_id {
            return Some((cc, transfer_len));
        }
        // Print unexpected events for debug
        if i < 5 || (i > 0 && i % 100000 == 0) {
            uart::puts(uart_base, "  evt: type=");
            crate::print_dec_u32(uart_base, trb_type);
            uart::puts(uart_base, " cc=");
            crate::print_dec_u32(uart_base, cc as u32);
            uart::puts(uart_base, " slot=");
            crate::print_dec_u32(uart_base, evt_slot as u32);
            uart::puts(uart_base, "\r\n");
        }
        if trb_type == TRB_EV_CMD_COMPLETE || trb_type == TRB_EV_PORT_CHANGE {
            continue;
        }
        core::hint::spin_loop();
    }
    // Timeout debug
    uart::puts(uart_base, "  xfer timeout: evt[");
    crate::print_dec_u32(uart_base, state.event_ring_idx as u32);
    uart::puts(uart_base, "] ctl=0x");
    let dbg_trb = (state.event_ring as *const Trb).add(state.event_ring_idx);
    crate::print_hex(uart_base, core::ptr::read_volatile(&(*dbg_trb).control));
    uart::puts(uart_base, " USBSTS=0x");
    crate::print_hex(uart_base, mmio_read32(state.op_base, OP_USBSTS));
    uart::puts(uart_base, "\r\n");
    None
}

/// Perform a USB control transfer on EP0
/// setup: 8-byte setup packet
/// data_buf: buffer for data stage (can be empty slice for no-data transfers)
/// data_in: true for IN (device-to-host), false for OUT (host-to-device)
/// Returns number of bytes transferred in data stage
unsafe fn control_transfer(
    state: &mut XhciState,
    uart_base: usize,
    setup: [u8; 8],
    data_buf: &mut [u8],
    data_in: bool,
) -> Option<usize> {
    let has_data = !data_buf.is_empty();
    let slot_id = state.device_slot;
    let ring = state.ep0_ring;
    let ring_size = 256usize;
    let mut idx = state.ep0_idx;
    let cycle = state.ep0_cycle;

    // Setup Stage TRB
    // Parameter = setup data (8 bytes inline via IDT)
    let setup_data = (setup[0] as u64)
        | ((setup[1] as u64) << 8)
        | ((setup[2] as u64) << 16)
        | ((setup[3] as u64) << 24)
        | ((setup[4] as u64) << 32)
        | ((setup[5] as u64) << 40)
        | ((setup[6] as u64) << 48)
        | ((setup[7] as u64) << 56);

    let trt = if has_data { if data_in { 2u32 } else { 3u32 } } else { 0u32 };
    let cycle_bit = if cycle { 1u32 } else { 0 };
    let setup_trb = (ring as *mut Trb).add(idx);
    core::ptr::write_volatile(&mut (*setup_trb).param, setup_data);
    core::ptr::write_volatile(&mut (*setup_trb).status, 8);
    // IDT=1 (bit 6): setup data is inline in the TRB parameter
    core::ptr::write_volatile(&mut (*setup_trb).control,
        (TRB_TYPE_SETUP_STAGE << 10) | (trt << 16) | cycle_bit | (1 << 6));
    idx = (idx + 1) % ring_size;

    // Data Stage TRB (if any)
    if has_data {
        let cycle_bit = if cycle { 1u32 } else { 0 };
        let dir_bit = if data_in { 1u32 << 16 } else { 0 };
        let data_trb = (ring as *mut Trb).add(idx);
        core::ptr::write_volatile(&mut (*data_trb).param, data_buf.as_ptr() as u64);
        // Status: [15:0]=length, [31:16] reserved
        core::ptr::write_volatile(&mut (*data_trb).status, data_buf.len() as u32);
        core::ptr::write_volatile(&mut (*data_trb).control,
            (TRB_TYPE_DATA_STAGE << 10) | dir_bit | cycle_bit);
        idx = (idx + 1) % ring_size;
    }

    // Status Stage TRB (always present)
    {
        let cycle_bit = if cycle { 1u32 } else { 0 };
        // Status direction: IN if no data or data was OUT, OUT if data was IN
        let status_dir = if !has_data || !data_in { 1u32 << 16 } else { 0 };
        let status_trb = (ring as *mut Trb).add(idx);
        core::ptr::write_volatile(&mut (*status_trb).param, 0);
        core::ptr::write_volatile(&mut (*status_trb).status, 0);
        core::ptr::write_volatile(&mut (*status_trb).control,
            (TRB_TYPE_STATUS_STAGE << 10) | status_dir | cycle_bit | (1 << 5)); // IOC=1
        idx = (idx + 1) % ring_size;
    }

    state.ep0_idx = idx;
    state.ep0_cycle = cycle;

    // Clean EP0 ring cache to RAM before ringing doorbell
    let trb_count = if has_data { 3 } else { 2 };
    let start_idx = (idx + ring_size - trb_count) % ring_size;
    cache_clean(ring + start_idx * 16, trb_count * 16);

    // Clear pending interrupt status
    let usbsts = mmio_read32(state.op_base, OP_USBSTS);
    mmio_write32(state.op_base, OP_USBSTS, usbsts);
    // Clear IMAN IP (Interrupt Pending) bit
    let int_base = state.run_base + 0x20;
    let iman = mmio_read32(int_base, INT_IMAN);
    if iman & 0x01 != 0 {
        mmio_write32(int_base, INT_IMAN, iman & !0x01 | 0x02); // clear IP, keep IE
    }

    // Ring doorbell for EP0 (DCI = 1)
    ring_doorbell(slot_id, 1, state.db_base);

    // Wait for transfer event
    let (cc, remainder) = poll_transfer_event(state, uart_base, slot_id)?;
    if cc != TRB_CC_SUCCESS && cc != TRB_CC_SHORT_PACKET {
        uart::puts(uart_base, "  ctrl xfer cc=");
        crate::print_dec_u32(uart_base, cc as u32);
        uart::puts(uart_base, "\r\n");
        return None;
    }

    if has_data && data_in {
        let transferred = (data_buf.len() as u32 - remainder) as usize;
        // Invalidate cache so CPU reads DMA-written data from RAM
        cache_invalidate(data_buf.as_ptr() as usize, transferred);
        Some(transferred)
    } else {
        Some(0)
    }
}

/// Public wrapper: EP0 control transfer OUT (host-to-device)
/// Uses pre-allocated ctrl_buf for DMA-safe data buffer
#[allow(dead_code)]
pub unsafe fn control_transfer_out(setup: &[u8; 8], data: &[u8]) -> bool {
    let state = match XHCI_STATE.as_mut() {
        Some(s) => s,
        None => return false,
    };
    let uart_base = UART;
    // Copy data to pre-allocated DMA buffer
    let buf = state.ctrl_buf as *mut u8;
    for (i, &b) in data.iter().enumerate() {
        core::ptr::write_volatile(buf.add(i), b);
    }
    cache_clean(state.ctrl_buf, data.len());
    let buf_slice = core::slice::from_raw_parts_mut(buf, data.len());
    control_transfer(state, uart_base, *setup, buf_slice, false).is_some()
}

/// Public wrapper: EP0 control transfer IN (device-to-host)
#[allow(dead_code)]
pub unsafe fn control_transfer_in(setup: &[u8; 8], data: &mut [u8]) -> Option<usize> {
    let state = match XHCI_STATE.as_mut() {
        Some(s) => s,
        None => return None,
    };
    let uart_base = UART;
    control_transfer(state, uart_base, *setup, data, true)
}

/// Send CDC SEND_ENCAPSULATED_COMMAND (class-specific EP0 OUT)
/// Uses pre-allocated ctrl_buf for DMA
#[allow(dead_code)]
pub unsafe fn send_encapsulated_command(msg: &[u8]) -> bool {
    let state = match XHCI_STATE.as_mut() {
        Some(s) => s,
        None => return false,
    };
    let uart_base = UART;
    let buf = state.ctrl_buf as *mut u8;
    for (i, &b) in msg.iter().enumerate() {
        core::ptr::write_volatile(buf.add(i), b);
    }
    cache_clean(state.ctrl_buf, msg.len());
    let setup: [u8; 8] = [
        0x21,       // bmRequestType: host-to-device, class, interface
        0x00,       // bRequest: SEND_ENCAPSULATED_COMMAND
        0x00, 0x00, // wValue: 0
        0x00, 0x00, // wIndex: interface 0
        (msg.len() & 0xFF) as u8, (msg.len() >> 8) as u8,
    ];
    let buf_slice = core::slice::from_raw_parts_mut(buf, msg.len());
    control_transfer(state, uart_base, setup, buf_slice, false).is_some()
}

/// Receive CDC GET_ENCAPSULATED_RESPONSE (class-specific EP0 IN)
#[allow(dead_code)]
pub unsafe fn get_encapsulated_response(buf: &mut [u8]) -> Option<usize> {
    let state = match XHCI_STATE.as_mut() {
        Some(s) => s,
        None => return None,
    };
    let uart_base = UART;
    let setup: [u8; 8] = [
        0xA1,       // bmRequestType: device-to-host, class, interface
        0x01,       // bRequest: GET_ENCAPSULATED_RESPONSE
        0x00, 0x00, // wValue: 0
        0x00, 0x00, // wIndex: interface 0
        (buf.len() & 0xFF) as u8, (buf.len() >> 8) as u8,
    ];
    let xfer_buf = state.ctrl_buf as *mut u8;
    let buf_slice = core::slice::from_raw_parts_mut(xfer_buf, buf.len());
    let result = control_transfer(state, uart_base, setup, buf_slice, true);
    if let Some(len) = result {
        cache_invalidate(state.ctrl_buf, len);
        for i in 0..len.min(buf.len()) {
            buf[i] = core::ptr::read_volatile(xfer_buf.add(i));
        }
    }
    result
}

/// Get USB descriptor into ctrl_buf
/// Returns actual length received
unsafe fn get_descriptor(
    state: &mut XhciState,
    uart_base: usize,
    desc_type: u8,
    desc_index: u8,
    len: u16,
) -> Option<usize> {
    let setup: [u8; 8] = [
        0x80,           // bmRequestType: device-to-host, standard, device
        0x06,           // bRequest: GET_DESCRIPTOR
        desc_index,     // wValue low: index
        desc_type,      // wValue high: descriptor type
        0x00, 0x00,     // wIndex: 0
        (len & 0xFF) as u8, (len >> 8) as u8, // wLength
    ];
    let buf = core::slice::from_raw_parts_mut(state.ctrl_buf as *mut u8, len as usize);
    control_transfer(state, uart_base, setup, buf, true)
}

/// Set USB configuration
unsafe fn set_configuration(
    state: &mut XhciState,
    uart_base: usize,
    config_value: u8,
) -> bool {
    let setup: [u8; 8] = [
        0x00,           // bmRequestType: host-to-device, standard, device
        0x09,           // bRequest: SET_CONFIGURATION
        config_value,   // wValue low: configuration value
        0x00,           // wValue high: 0
        0x00, 0x00,     // wIndex: 0
        0x00, 0x00,     // wLength: 0
    ];
    control_transfer(state, uart_base, setup, &mut [], false).is_some()
}

/// Set USB interface alternate setting
unsafe fn set_interface(
    state: &mut XhciState,
    uart_base: usize,
    interface: u8,
    alt_setting: u8,
) -> bool {
    let setup: [u8; 8] = [
        0x01,           // bmRequestType: host-to-device, standard, interface
        0x0B,           // bRequest: SET_INTERFACE
        alt_setting,    // wValue low: alternate setting
        0x00,           // wValue high: 0
        interface,      // wIndex low: interface number
        0x00,           // wIndex high: 0
        0x00, 0x00,     // wLength: 0
    ];
    control_transfer(state, uart_base, setup, &mut [], false).is_some()
}

/// Write an endpoint context at a given offset in the input context buffer
unsafe fn write_ep_context(
    input_ctx: usize,
    dci: usize,       // Device Context Index (1-based: 1=EP0, 2=EP1OUT, 3=EP1IN, etc.)
    ep_type: u32,     // EP Type: 2=Bulk OUT, 4=Control, 6=Bulk IN
    max_packet: u32,
    ring_addr: usize,
    max_burst: u32,
) {
    // EP context offset: Input Control (0x20) + Slot (0x20) + (dci-1)*32
    let offset = 0x20 + 0x20 + (dci - 1) * 32;
    let base = (input_ctx + offset) as *mut u32;

    // DW0: EP State = 0, Mult = 0, etc
    core::ptr::write_volatile(base, 0);
    // DW1: CErr[2:1]=3 | EP Type[5:3] | MaxBurst[15:8] | MaxPacketSize[31:16]
    core::ptr::write_volatile(base.add(1), (3 << 1) | (ep_type << 3) | (max_burst << 8) | (max_packet << 16));
    // DW2: TR Dequeue Pointer Low [31:4] + DCS[0]=1
    core::ptr::write_volatile(base.add(2), (ring_addr as u32 & 0xFFFFFFF0) | 1);
    // DW3: TR Dequeue Pointer High [63:32]
    core::ptr::write_volatile(base.add(3), (ring_addr >> 32) as u32);
    // DW4: Average TRB Length
    core::ptr::write_volatile(base.add(4), 256);
    // DW5-DW7: Reserved
    core::ptr::write_volatile(base.add(5), 0);
    core::ptr::write_volatile(base.add(6), 0);
    core::ptr::write_volatile(base.add(7), 0);
}

/// Scan ports for connected devices and enumerate CDC-ECM
pub fn enumerate_devices(uart_base: usize) {
    unsafe {
        let state = match XHCI_STATE.as_mut() {
            Some(s) => s,
            None => return,
        };

        uart::puts(uart_base, "[..] USB: scanning ports\r\n");

        // Test command ring with No-Op command
        let noop_result = send_command(state, uart_base, 0, 0, TRB_TYPE_NOOP << 10);
        match noop_result {
            Some((cc, _)) => {
                uart::puts(uart_base, "  No-Op: cc=");
                crate::print_dec_u32(uart_base, cc as u32);
                uart::puts(uart_base, "\r\n");
            }
            None => {
                uart::puts(uart_base, "  No-Op: timeout\r\n");
                return;
            }
        }

        // Scan all ports for connected devices
        for port in 1..=state.max_ports as usize {
            let portsc = mmio_read32(state.port_base, (port - 1) * 0x10 + PORT_SC);

            // Check if device connected (CCS=1)
            if portsc & PORTSC_CCS == 0 {
                continue;
            }

            uart::puts(uart_base, "  Port ");
            crate::print_dec_u32(uart_base, port as u32);
            let speed = (portsc >> 10) & 0xF;
            uart::puts(uart_base, " speed=");
            crate::print_dec_u32(uart_base, speed);
            uart::puts(uart_base, "\r\n");

            // Power on port if needed
            if portsc & PORTSC_PP == 0 {
                mmio_write32(state.port_base, (port - 1) * 0x10 + PORT_SC, portsc | PORTSC_PP);
                for _ in 0..100000 { core::hint::spin_loop(); }
            }

            // Reset port
            let portsc2 = mmio_read32(state.port_base, (port - 1) * 0x10 + PORT_SC);
            mmio_write32(state.port_base, (port - 1) * 0x10 + PORT_SC, portsc2 | PORTSC_PR);
            if !wait_for(
                || mmio_read32(state.port_base, (port - 1) * 0x10 + PORT_SC) & PORTSC_PR == 0,
                500000
            ) {
                uart::puts(uart_base, "  Port reset timeout\r\n");
                continue;
            }

            // Clear port reset change
            let portsc3 = mmio_read32(state.port_base, (port - 1) * 0x10 + PORT_SC);
            uart::puts(uart_base, "  PORTSC=0x");
            crate::print_hex(uart_base, portsc3);
            uart::puts(uart_base, "\r\n");
            mmio_write32(state.port_base, (port - 1) * 0x10 + PORT_SC, portsc3 | PORTSC_PRC | PORTSC_CSC);

            // Enable Slot
            let result = send_command(state, uart_base, 0, 0, TRB_TYPE_ENABLE_SLOT << 10);
            let (_cc, slot_id) = match result {
                Some((cc, slot)) if cc == TRB_CC_SUCCESS => (cc, slot),
                _ => {
                    uart::puts(uart_base, "  Enable Slot failed\r\n");
                    continue;
                }
            };

            uart::puts(uart_base, "  Slot ");
            crate::print_dec_u32(uart_base, slot_id as u32);
            uart::puts(uart_base, " allocated\r\n");

            state.device_slot = slot_id;
            state.device_port = port as u8;

            // Setup Input Context for Address Device
            let ic = state.input_ctx as *mut u8;
            // Clear input context
            for i in 0..4096 { core::ptr::write_volatile(ic.add(i), 0); }

            // Input Control Context: add_flags = slot(0) + ep0(1)
            let ic_u32 = ic as *mut u32;
            core::ptr::write_volatile(ic_u32, 0);         // drop_flags
            core::ptr::write_volatile(ic_u32.add(1), 0x03); // add_flags: slot + ep0

            // Slot context at offset 0x20 (Input Context base + 0x20)
            // DW0: Route String [19:0] | Speed [23:20] | Hub [26] | Context Entries [31:27]
            // DW1: Max Exit Latency [15:0] | Root Hub Port [23:16]
            // DW2: TT Hub Slot ID [7:0] | TT Port [15:8] | Interrupter Target [31:22]
            // DW3: (output-only: Device Address, Slot State)
            let slot_speed = (mmio_read32(state.port_base, (port - 1) * 0x10 + PORT_SC) >> 10) & 0xF;
            let slot_ctx = ic.add(0x20) as *mut u32;
            core::ptr::write_volatile(slot_ctx, (slot_speed << 20) | (1 << 27)); // speed + ctx_entries=1 (EP0 only)
            core::ptr::write_volatile(slot_ctx.add(1), (port as u32) << 16);     // root hub port number
            core::ptr::write_volatile(slot_ctx.add(2), 0);
            core::ptr::write_volatile(slot_ctx.add(3), 0); // output-only, leave 0

            // EP0 context at offset 0x40 (DCI 1)
            write_ep_context(state.input_ctx, 1, 4, 64, state.ep0_ring, 0);

            // Store device context address in DCBAA
            let dcbaa_ptr = state.dcbaa as *mut u64;
            core::ptr::write_volatile(dcbaa_ptr.add(slot_id as usize), state.device_ctx as u64);
            cache_clean(state.dcbaa + slot_id as usize * 8, 8);

            // Clean input context cache to RAM before controller reads via DMA
            cache_clean(state.input_ctx, 4096);

            // Address Device command (BSR=0, block until addressed)
            let input_ctx_phys = state.input_ctx as u64;
            let result2 = send_command(
                state,
                uart_base,
                input_ctx_phys,
                0,
                (TRB_TYPE_ADDRESS_DEV << 10) | ((slot_id as u32) << 24), // BSR=0
            );

            match result2 {
                Some((cc, _)) if cc == TRB_CC_SUCCESS => {
                    uart::puts(uart_base, "  Device addressed\r\n");
                    state.device_addr = slot_id as u8;
                }
                _ => {
                    uart::puts(uart_base, "  Address Device failed\r\n");
                    state.device_slot = 0;
                    continue;
                }
            }

            // Get Device Descriptor (8 bytes first to read max packet size)
            let _dev_desc_len = match get_descriptor(state, uart_base, 0x01, 0, 8) {
                Some(len) if len >= 8 => {
                    let buf = state.ctrl_buf as *const u8;
                    let b_max_packet_size0 = core::ptr::read_volatile(buf.add(7));
                    let vid = core::ptr::read_volatile(buf.add(8)) as u16
                        | ((core::ptr::read_volatile(buf.add(9)) as u16) << 8);
                    let pid = core::ptr::read_volatile(buf.add(10)) as u16
                        | ((core::ptr::read_volatile(buf.add(11)) as u16) << 8);
                    uart::puts(uart_base, "  USB: vid=0x");
                    crate::print_hex(uart_base, vid as u32);
                    uart::puts(uart_base, " pid=0x");
                    crate::print_hex(uart_base, pid as u32);
                    uart::puts(uart_base, " mps=");
                    crate::print_dec_u32(uart_base, b_max_packet_size0 as u32);
                    uart::puts(uart_base, "\r\n");
                    b_max_packet_size0
                }
                _ => {
                    uart::puts(uart_base, "  Get Device Descriptor failed\r\n");
                    state.device_slot = 0;
                    continue;
                }
            };

            // Get Configuration Descriptor (first 9 bytes to get total length)
            let config_total_len = match get_descriptor(state, uart_base, 0x02, 0, 9) {
                Some(len) if len >= 9 => {
                    let buf = state.ctrl_buf as *const u8;
                    let total_len = core::ptr::read_volatile(buf.add(2)) as u16
                        | ((core::ptr::read_volatile(buf.add(3)) as u16) << 8);
                    let num_interfaces = core::ptr::read_volatile(buf.add(4));
                    uart::puts(uart_base, "  Config: len=");
                    crate::print_dec_u32(uart_base, total_len as u32);
                    uart::puts(uart_base, " ifaces=");
                    crate::print_dec_u32(uart_base, num_interfaces as u32);
                    uart::puts(uart_base, "\r\n");
                    total_len
                }
                _ => {
                    uart::puts(uart_base, "  Get Config Desc (short) failed\r\n");
                    state.device_slot = 0;
                    continue;
                }
            };

            // Get full Configuration Descriptor
            let full_len = config_total_len.min(4096) as usize;
            if get_descriptor(state, uart_base, 0x02, 0, full_len as u16).is_some() {
                let buf = state.ctrl_buf as *const u8;
                let mut offset = 0usize;
                let mut found_cdc_ecm = false;
                let mut bulk_in_ep: u8 = 0;
                let mut bulk_out_ep: u8 = 0;
                let mut bulk_max_packet: u16 = 64;

                // Parse descriptor chain looking for CDC-ECM
                while offset + 4 < full_len {
                    let desc_len = core::ptr::read_volatile(buf.add(offset)) as usize;
                    let desc_type = core::ptr::read_volatile(buf.add(offset + 1));
                    if desc_len == 0 { break; }

                    if desc_type == 0x04 {
                        // Interface Descriptor
                        let iface_num = core::ptr::read_volatile(buf.add(offset + 2));
                        let iface_class = core::ptr::read_volatile(buf.add(offset + 5));
                        let iface_subclass = core::ptr::read_volatile(buf.add(offset + 6));
                        uart::puts(uart_base, "  Iface ");
                        crate::print_dec_u32(uart_base, iface_num as u32);
                        uart::puts(uart_base, " class=0x");
                        crate::print_hex(uart_base, iface_class as u32);
                        uart::puts(uart_base, " sub=0x");
                        crate::print_hex(uart_base, iface_subclass as u32);
                        uart::puts(uart_base, "\r\n");

                        // CDC: class=0x02 (Communications) with any CDC subclass,
                        // or class=0x0A (CDC Data). Both use bulk endpoints for Ethernet.
                        // Subclass 0x02=ACM, 0x06=ECM — QEMU usb-net uses 0x02.
                        if (iface_class == 0x02 && (iface_subclass == 0x02 || iface_subclass == 0x06))
                            || iface_class == 0x0A
                        {
                            found_cdc_ecm = true;
                            uart::puts(uart_base, "  -> CDC ethernet!\r\n");
                        }
                    }

                    if desc_type == 0x05 {
                        // Endpoint Descriptor
                        let ep_addr = core::ptr::read_volatile(buf.add(offset + 2));
                        let ep_attrs = core::ptr::read_volatile(buf.add(offset + 3));
                        let ep_max_packet = core::ptr::read_volatile(buf.add(offset + 4)) as u16
                            | ((core::ptr::read_volatile(buf.add(offset + 5)) as u16) << 8);
                        let ep_dir = ep_addr & 0x80;
                        let ep_num = ep_addr & 0x0F;
                        let ep_type_val = ep_attrs & 0x03;

                        if ep_type_val == 0x02 { // Bulk
                            if ep_dir != 0 {
                                bulk_in_ep = ep_num;
                            } else {
                                bulk_out_ep = ep_num;
                            }
                            bulk_max_packet = ep_max_packet;
                            uart::puts(uart_base, "  EP 0x");
                            crate::print_hex(uart_base, ep_addr as u32);
                            uart::puts(uart_base, " bulk ");
                            if ep_dir != 0 { uart::puts(uart_base, "IN"); } else { uart::puts(uart_base, "OUT"); }
                            uart::puts(uart_base, " mps=");
                            crate::print_dec_u32(uart_base, ep_max_packet as u32);
                            uart::puts(uart_base, "\r\n");
                        }
                    }

                    offset += desc_len;
                }

                if found_cdc_ecm && bulk_in_ep != 0 && bulk_out_ep != 0 {
                    uart::puts(uart_base, "[..] CDC-ECM found: IN=");
                    crate::print_dec_u32(uart_base, bulk_in_ep as u32);
                    uart::puts(uart_base, " OUT=");
                    crate::print_dec_u32(uart_base, bulk_out_ep as u32);
                    uart::puts(uart_base, "\r\n");

                    // Set Configuration (config 1 = CDC-ECM mode for QEMU usb-net)
                    if !set_configuration(state, uart_base, 1) {
                        uart::puts(uart_base, "[FAIL] Set Configuration\r\n");
                        state.device_slot = 0;
                        continue;
                    }
                    uart::puts(uart_base, "  Config set\r\n");

                    // Configure Endpoint for Bulk IN and Bulk OUT
                    // DCI: ep_out_num * 2 for OUT, ep_num * 2 + 1 for IN
                    let dci_out = (bulk_out_ep as usize) * 2;     // DCI for EP2 OUT
                    let dci_in = (bulk_in_ep as usize) * 2 + 1;   // DCI for EP2 IN

                    // Clear input context
                    let ic = state.input_ctx as *mut u8;
                    for i in 0..4096 { core::ptr::write_volatile(ic.add(i), 0); }

                    // Input Control: add_flags only for bulk endpoints (not slot/EP0)
                    // Bit 0 = A0 (reserved, must be 0 for Configure Endpoint)
                    let ic_u32 = ic as *mut u32;
                    core::ptr::write_volatile(ic_u32, 0); // drop_flags
                    let add_flags = 0x01 | (1 << dci_out) | (1 << dci_in); // include slot ctx
                    core::ptr::write_volatile(ic_u32.add(1), add_flags);

                    // Slot context: copy from device output context and update ctx_entries
                    cache_invalidate(state.device_ctx, 4096);
                    let dev_slot = (state.device_ctx + 0x00) as *const u32;
                    let slot_ctx = ic.add(0x20) as *mut u32;
                    // Copy existing slot context, update ctx_entries to max DCI
                    let existing_dw0 = core::ptr::read_volatile(dev_slot);
                    core::ptr::write_volatile(slot_ctx, (existing_dw0 & !(0x1F << 27)) | ((dci_in as u32) << 27));
                    core::ptr::write_volatile(slot_ctx.add(1), core::ptr::read_volatile(dev_slot.add(1)));
                    core::ptr::write_volatile(slot_ctx.add(2), core::ptr::read_volatile(dev_slot.add(2)));
                    core::ptr::write_volatile(slot_ctx.add(3), core::ptr::read_volatile(dev_slot.add(3)));

                    // Bulk OUT endpoint context
                    write_ep_context(state.input_ctx, dci_out, 2, bulk_max_packet as u32, state.bulk_out_ring, 0);

                    // Bulk IN endpoint context
                    write_ep_context(state.input_ctx, dci_in, 6, bulk_max_packet as u32, state.bulk_in_ring, 0);

                    // Clean input context to RAM before controller reads via DMA
                    cache_clean(state.input_ctx, 4096);

                    // Configure Endpoint command
                    let cfg_result = send_command(
                        state,
                        uart_base,
                        state.input_ctx as u64,
                        0,
                        (TRB_TYPE_CONFIGURE_EP << 10) | ((slot_id as u32) << 24),
                    );

                    match cfg_result {
                        Some((cc, _)) if cc == TRB_CC_SUCCESS => {
                            // Doorbell target = DCI (Device Context Index)
                            // DCI = ep_num * 2 + (direction ? 1 : 0)
                            state.bulk_in_dbl = (bulk_in_ep as u32) * 2 + 1;   // DCI for EP IN
                            state.bulk_out_dbl = bulk_out_ep as u32 * 2;     // DCI for EP OUT
                            state.max_packet = bulk_max_packet;

                            // Activate data interface (AltSetting 1) for CDC-ECM bulk endpoints
                            // Interface 1 is the CDC Data class (0x0A) with bulk IN/OUT
                            if set_interface(state, uart_base, 1, 1) {
                                uart::puts(uart_base, "  Data iface active\r\n");
                            } else {
                                uart::puts(uart_base, "[WARN] SET_INTERFACE 1/1 failed\r\n");
                            }

                            uart::puts(uart_base, "[OK] USB CDC-ECM ready\r\n");
                        }
                        Some((cc, _)) => {
                            uart::puts(uart_base, "[FAIL] Configure Endpoint cc=");
                            crate::print_dec_u32(uart_base, cc as u32);
                            uart::puts(uart_base, "\r\n");
                            state.device_slot = 0;
                            continue;
                        }
                        None => {
                            uart::puts(uart_base, "[FAIL] Configure Endpoint timeout\r\n");
                            state.device_slot = 0;
                            continue;
                        }
                    }
                } else {
                    uart::puts(uart_base, "[INFO] Not CDC-ECM, skipping\r\n");
                    state.device_slot = 0;
                }
            }

            // Stop at first device
            break;
        }

        if state.device_slot == 0 {
            uart::puts(uart_base, "[INFO] USB: no CDC-ECM device\r\n");
        }
    }
}

/// Check if xHCI is initialized and device is ready
pub fn is_ready() -> bool {
    unsafe { XHCI_STATE.as_ref().map(|s| s.device_slot != 0).unwrap_or(false) }
}

/// Get xHCI state for USB network driver
pub fn get_state() -> Option<&'static mut XhciState> {
    unsafe { XHCI_STATE.as_mut() }
}

/// Submit a bulk IN TRB to receive data from the device
/// Returns true if TRB was queued successfully
pub fn submit_bulk_in(buf: usize, buf_len: usize) -> bool {
    unsafe {
        let state = match XHCI_STATE.as_mut() {
            Some(s) => s,
            None => return false,
        };

        if state.device_slot == 0 {
            return false;
        }

        let idx = state.bulk_in_idx;
        let ring_size = 256usize;
        let trb_ptr = (state.bulk_in_ring as *mut Trb).add(idx);

        core::ptr::write_volatile(&mut (*trb_ptr).param, buf as u64);
        core::ptr::write_volatile(&mut (*trb_ptr).status, buf_len as u32);
        let cycle_bit = if state.bulk_in_cycle { 1u32 } else { 0 };
        // Normal TRB: type=1 (bits 10:15), IOC=1 (bit 5)
        core::ptr::write_volatile(&mut (*trb_ptr).control,
            (1 << 10) | (1 << 5) | cycle_bit);

        cache_clean(state.bulk_in_ring + idx * 16, 16);

        state.bulk_in_idx = (idx + 1) % ring_size;
        if state.bulk_in_idx == 0 {
            state.bulk_in_cycle = !state.bulk_in_cycle;
        }

        ring_doorbell(state.device_slot, state.bulk_in_dbl as u8, state.db_base);
        true
    }
}

/// Poll for a completed bulk IN transfer event (non-blocking)
/// Returns Some((data_buffer_addr, transfer_length)) if data is available
/// The data_buffer_addr is the actual DMA buffer address from the completed TRB
pub fn poll_event() -> Option<(usize, usize)> {
    unsafe {
        let state = XHCI_STATE.as_mut()?;
        if state.device_slot == 0 {
            return None;
        }

        // Try up to 16 events to drain non-transfer events
        for _ in 0..16 {
            cache_invalidate(state.event_ring + state.event_ring_idx * 16, 64);

            let idx = state.event_ring_idx;
            let trb_ptr = (state.event_ring as *const Trb).add(idx);
            let trb = &*trb_ptr;

            let trb_cycle = (trb.control & 1) != 0;
            if trb_cycle != state.event_cycle {
                break; // No new events — fall through to doorbell re-ring
            }

            let trb_type = (trb.control >> 10) & 0x3F;
            let cc = (trb.status >> 24) as u8;
            let slot_id = ((trb.control >> 24) & 0xFF) as u8;
            let endpoint_id = ((trb.control >> 16) & 0x1F) as u8; // DCI of completed transfer
            // Transfer Event: status[23:0] = remaining data length (not transferred)
            let remainder = trb.status & 0xFFFFFF;
            // param = TRB Pointer (address of the transfer TRB that completed)
            let trb_pointer = trb.param as usize;

            // Advance event ring
            state.event_ring_idx = idx + 1;
            if state.event_ring_idx >= state.event_ring_size {
                state.event_ring_idx = 0;
                state.event_cycle = !state.event_cycle;
            }

            // Update ERDP
            let int_base = state.run_base + 0x20;
            let deque_ptr = (state.event_ring + (idx + 1) * 16) as u64;
            mmio_write64(int_base, INT_ERDP_LO, deque_ptr | (1 << 3));

            if trb_type == TRB_EV_TRANSFER && slot_id == state.device_slot {
                if cc != TRB_CC_SUCCESS && cc != TRB_CC_SHORT_PACKET {
                    // Transfer error on any endpoint - skip
                    continue;
                }

                // Only return bulk IN completions (DCI matches bulk_in_dbl)
                if endpoint_id != state.bulk_in_dbl as u8 {
                    // This is a bulk OUT or EP0 completion - drain it
                    continue;
                }

                // Read the completed TRB to get the data buffer address
                cache_invalidate(trb_pointer, 16);
                let completed_trb = trb_pointer as *const Trb;
                let data_buf = core::ptr::read_volatile(&(*completed_trb).param) as usize;
                let trb_length = core::ptr::read_volatile(&(*completed_trb).status) as usize;

                // Calculate actual transfer length = TRB length - remainder
                let actual_len = if trb_length > remainder as usize {
                    trb_length - remainder as usize
                } else {
                    0
                };

                if actual_len == 0 {
                    // Empty transfer - re-submit this buffer
                    submit_bulk_in(data_buf, 4096);
                    continue;
                }

                return Some((data_buf, actual_len));
            }

            // Non-transfer events (cmd complete, port change) - just drain them
            core::hint::spin_loop();
        }

        // No event found - re-ring bulk IN doorbell to trigger retry
        // (QEMU xHCI may need this after device wakeup)
        ring_doorbell(state.device_slot, state.bulk_in_dbl as u8, state.db_base);

        None
    }
}

/// Debug dump of bulk IN endpoint state
#[allow(dead_code)]
pub unsafe fn dump_bulk_in_debug() {
    let state = match XHCI_STATE.as_mut() {
        Some(s) => s,
        None => return,
    };
    let uart_base = UART;

    // Show USBSTS
    let usbsts = mmio_read32(state.op_base, 0x04);
    uart::puts(uart_base, "  USBSTS=0x");
    crate::print_hex(uart_base, usbsts);

    // Show EP2IN output context (DCI 5)
    cache_invalidate(state.device_ctx, 4096);
    let ep5 = (state.device_ctx + 5 * 0x20) as *const u32;
    uart::puts(uart_base, " EP2IN:");
    for i in 0..5 {
        crate::print_hex(uart_base, core::ptr::read_volatile(ep5.add(i)));
        uart::putc(uart_base, b' ');
    }

    // Show first bulk IN TRB
    cache_invalidate(state.bulk_in_ring, 64);
    let bin_trb0 = state.bulk_in_ring as *const Trb;
    uart::puts(uart_base, "\r\n    bin: p=0x");
    crate::print_hex(uart_base, core::ptr::read_volatile(&(*bin_trb0).param) as u32);
    uart::puts(uart_base, " s=0x");
    crate::print_hex(uart_base, core::ptr::read_volatile(&(*bin_trb0).status));
    uart::puts(uart_base, " c=0x");
    crate::print_hex(uart_base, core::ptr::read_volatile(&(*bin_trb0).control));

    // Doorbell
    uart::puts(uart_base, " db=0x");
    crate::print_hex(uart_base, mmio_read32(state.db_base, (state.device_slot as usize) * 4));

    // Event ring dequeue pointer
    let int_base = state.run_base + 0x20;
    let erdp = mmio_read64(int_base, 0x18);
    uart::puts(uart_base, " erdp=0x");
    crate::print_hex(uart_base, erdp as u32);
    uart::puts(uart_base, "\r\n");
}
