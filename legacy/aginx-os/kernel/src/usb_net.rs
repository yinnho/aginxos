//! USB Network Driver (CDC-ECM)
//!
//! Provides raw Ethernet frame TX/RX over USB bulk endpoints via xHCI.
//! Works with QEMU's usb-net device in CDC-ECM mode (config 1).

use core::alloc::Layout;

#[cfg(not(feature = "board-redfin"))]
use crate::uart;
#[cfg(feature = "board-redfin")]
use crate::qup_uart as uart;

use crate::platform::UART;

const MAX_FRAME: usize = 1514;
const NUM_RX_BUFS: usize = 8;

/// USB network state
#[allow(dead_code)]
struct UsbNetState {
    mac: [u8; 6],
    /// Pre-allocated DMA receive buffers
    rx_bufs: [usize; NUM_RX_BUFS],
    /// Number of bulk IN TRBs currently submitted
    rx_submitted: usize,
}

static mut USB_NET: Option<UsbNetState> = None;
static mut TX_COUNT: u32 = 0;
static mut RX_COUNT: u32 = 0;

/// Page-aligned DMA allocation
fn dma_alloc(size: usize) -> Option<usize> {
    let page_size = 4096;
    let aligned_size = ((size + page_size - 1) / page_size) * page_size;
    let layout = Layout::from_size_align(aligned_size, page_size).ok()?;
    unsafe {
        let ptr = alloc::alloc::alloc_zeroed(layout);
        if ptr.is_null() { None } else { Some(ptr as usize) }
    }
}

/// Initialize USB network driver
#[allow(dead_code)]
pub fn init() -> bool {
    unsafe {
        if USB_NET.is_some() {
            return true;
        }

        let xhci = match crate::xhci::get_state() {
            Some(s) => s,
            None => {
                uart::puts(UART, "[SKIP] USB-Net: no xHCI\r\n");
                return false;
            }
        };

        if xhci.device_slot == 0 {
            uart::puts(UART, "[SKIP] USB-Net: no CDC device\r\n");
            return false;
        }

        // CDC-ECM mode: no RNDIS init needed, raw Ethernet frames on bulk endpoints

        // QEMU usb-net default MAC
        let mac: [u8; 6] = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];

        // Pre-allocate RX buffers and submit bulk IN TRBs
        let mut rx_bufs: [usize; NUM_RX_BUFS] = [0; NUM_RX_BUFS];
        let mut rx_submitted = 0usize;
        for i in 0..NUM_RX_BUFS {
            let buf = match dma_alloc(MAX_FRAME + 64) {
                Some(b) => b,
                None => break,
            };
            rx_bufs[i] = buf;
            if crate::xhci::submit_bulk_in(buf, MAX_FRAME + 64) {
                rx_submitted += 1;
            }
        }

        USB_NET = Some(UsbNetState {
            mac,
            rx_bufs,
            rx_submitted,
        });

        uart::puts(UART, "[OK] USB-Net: MAC=52:54:00:12:34:56 rx_bufs=");
        uart::putc(UART, b'0' + (rx_submitted as u8));
        uart::puts(UART, "\r\n");
        true
    }
}

/// Get MAC address
pub fn get_mac() -> Option<[u8; 6]> {
    unsafe { USB_NET.as_ref().map(|s| s.mac) }
}

/// Transmit a raw Ethernet frame via USB bulk OUT endpoint
/// CDC-ECM mode: sends raw frame data, no header wrapping needed
pub fn transmit(data: &[u8]) -> bool {
    unsafe {
        let xhci = match crate::xhci::get_state() {
            Some(s) => s,
            None => return false,
        };

        if xhci.device_slot == 0 || data.len() > MAX_FRAME || data.len() == 0 {
            return false;
        }

        // Allocate DMA buffer and copy frame data
        let total = data.len();
        let buf = match dma_alloc(total.max(64)) {
            Some(b) => b,
            None => return false,
        };
        for (i, &b) in data.iter().enumerate() {
            core::ptr::write_volatile((buf + i) as *mut u8, b);
        }
        crate::xhci::cache_clean(buf, total);

        // Queue Normal TRB on bulk OUT ring
        let idx = xhci.bulk_out_idx;
        let ring_size = 256usize;
        let trb_ptr = (xhci.bulk_out_ring as *mut crate::xhci::Trb).add(idx);

        core::ptr::write_volatile(&mut (*trb_ptr).param, buf as u64);
        core::ptr::write_volatile(&mut (*trb_ptr).status, total as u32);
        let cycle_bit = if xhci.bulk_out_cycle { 1u32 } else { 0 };
        core::ptr::write_volatile(&mut (*trb_ptr).control,
            (1 << 10) | (1 << 5) | cycle_bit);

        crate::xhci::cache_clean(xhci.bulk_out_ring + idx * 16, 16);

        xhci.bulk_out_idx = (idx + 1) % ring_size;
        if xhci.bulk_out_idx == 0 {
            xhci.bulk_out_cycle = !xhci.bulk_out_cycle;
        }

        crate::xhci::ring_doorbell(xhci.device_slot, xhci.bulk_out_dbl as u8, xhci.db_base);

        // CDC-ECM requires a short packet to signal end of frame.
        // If frame length is a multiple of max packet size (64), send ZLP.
        if total % 64 == 0 {
            let zlp_idx = xhci.bulk_out_idx;
            let zlp_ptr = (xhci.bulk_out_ring as *mut crate::xhci::Trb).add(zlp_idx);
            let zlp_cycle = if xhci.bulk_out_cycle { 1u32 } else { 0 };
            core::ptr::write_volatile(&mut (*zlp_ptr).param, 0u64);
            core::ptr::write_volatile(&mut (*zlp_ptr).status, 0u32);
            core::ptr::write_volatile(&mut (*zlp_ptr).control,
                (1u32 << 10) | (1u32 << 5) | zlp_cycle);
            crate::xhci::cache_clean(xhci.bulk_out_ring + zlp_idx * 16, 16);
            xhci.bulk_out_idx = (zlp_idx + 1) % 256;
            if xhci.bulk_out_idx == 0 {
                xhci.bulk_out_cycle = !xhci.bulk_out_cycle;
            }
            crate::xhci::ring_doorbell(xhci.device_slot, xhci.bulk_out_dbl as u8, xhci.db_base);
        }

        TX_COUNT += 1;
        true
    }
}

/// Check for received Ethernet frame (non-blocking)
/// Returns Some((buffer_ptr, length)) if frame available
/// CDC-ECM mode: raw Ethernet frame, no header to strip
pub fn receive() -> Option<(usize, usize)> {
    unsafe {
        let (buf_addr, transfer_len) = crate::xhci::poll_event()?;

        if transfer_len == 0 || buf_addr == 0 {
            crate::xhci::submit_bulk_in(buf_addr, MAX_FRAME + 64);
            return None;
        }

        crate::xhci::cache_invalidate(buf_addr, transfer_len);

        RX_COUNT += 1;
        Some((buf_addr, transfer_len))
    }
}

/// Release received buffer and re-submit for next receive
pub fn release_rx_buffer(buf: usize) {
    crate::xhci::submit_bulk_in(buf, MAX_FRAME + 64);
}

/// Get TX/RX packet counts for debugging
#[allow(dead_code)]
pub fn get_counts() -> (u32, u32) {
    unsafe { (TX_COUNT, RX_COUNT) }
}
