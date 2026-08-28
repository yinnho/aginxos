//! ath11k WiFi Driver for Aginx OS
//!
//! Driver for Qualcomm QCA6390 WiFi chip (Pixel 5)
//!
//! Architecture:
//! ```
//! ┌─────────────────────────────────────┐
//! │     NetworkScheme (network:)        │
//! ├─────────────────────────────────────┤
//! │     Ath11kDevice                    │
//! │     ├── WMI (Wireless Management)   │
//! │     ├── HTT (Hardware Transport)    │
//! │     └── MHI (Modem Host Interface)  │
//! ├─────────────────────────────────────┤
//! │     PCIe (via pcid scheme)          │
//! └─────────────────────────────────────┘
//! ```

#![no_std]
#![no_main]

extern crate alloc;

mod device;
mod mhi;
mod wmi;
mod htt;

use alloc::boxed::Box;
use alloc::string::String;
use driver_network::{NetworkAdapter, NetworkScheme};
use aginx_event::EventQueue;

/// Driver entry point
#[no_mangle]
pub fn main() -> ! {
    // Initialize logging
    // log::init();

    // Connect to pcid scheme to get PCI device
    let pci_device = match pcid_connect() {
        Ok(dev) => dev,
        Err(e) => {
            // Log error and exit
            loop {
                core::hint::spin_loop();
            }
        }
    };

    // Create device instance
    let device = match device::Ath11kDevice::new(pci_device) {
        Ok(dev) => dev,
        Err(e) => {
            loop {
                core::hint::spin_loop();
            }
        }
    };

    // Create network scheme
    let mut scheme = NetworkScheme::new(device, "network.wlan0");

    // Create event queue
    let mut event_queue = EventQueue::new();

    // Subscribe to IRQ events
    // event_queue.subscribe(irq_fd, aginx_event::EVENT_READ, IRQ_TOKEN);

    // Subscribe to scheme events
    // event_queue.subscribe(scheme_fd, aginx_event::EVENT_READ, SCHEME_TOKEN);

    // Main event loop
    loop {
        scheme.tick();

        // Process events
        // event_queue.process(|event| {
        //     match event.user_data {
        //         IRQ_TOKEN => handle_irq(&mut scheme),
        //         SCHEME_TOKEN => handle_scheme_request(&mut scheme),
        //         _ => {}
        //     }
        // });
    }
}

/// Connect to pcid scheme and get QCA6390 device
fn pcid_connect() -> Result<PciDevice, PciError> {
    // Open pcid scheme
    // Query for QCA6390: vendor=0x17CB, device=0x1101
    // Map BARs
    // Return device handle
    Err(PciError::NotFound)
}

/// PCI device handle
struct PciDevice {
    bar0: *mut u8,      // MMIO base
    irq: u32,           // IRQ number
}

#[derive(Debug)]
enum PciError {
    NotFound,
    NoResource,
    MapFailed,
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
