# Aginx Driver Research Notes

## Redox Network Driver Architecture

### Overview

Redox OS drivers are **userspace daemons**, not kernel modules. They communicate with the kernel through the **scheme** mechanism (Plan 9 style resource abstraction).

### Key Repositories

| Repository | Description |
|------------|-------------|
| `redox-os/base` (ID: 2336) | **Active** - Contains all drivers, netstack, init |
| `redox-os/drivers` (ID: 22) | **ARCHIVED** - Merged into base |
| `redox-os/kernel` | Microkernel |

### Driver Location in Base Repo

```
drivers/
├── net/                    # Network drivers
│   ├── driver-network/     # NetworkScheme trait library
│   ├── e1000d/             # Intel e1000 driver
│   ├── virtio-netd/        # VirtIO network driver
│   ├── rtl8168d/           # Realtek driver
│   ├── rtl8139d/           # Realtek driver
│   └── ixgbed/             # Intel 10Gb driver
├── storage/                # Storage drivers
├── pcid/                   # PCI daemon
├── pcid-spawner/           # PCI device spawner
└── initfs.toml             # PCI driver configuration
```

---

## NetworkAdapter Trait (Core Interface)

```rust
// From: drivers/net/driver-network/src/lib.rs

pub trait NetworkAdapter {
    /// MAC address of the adapter
    fn mac_address(&mut self) -> [u8; 6];

    /// Amount of data available to read (non-blocking check)
    fn available_for_read(&mut self) -> usize;

    /// Read a packet, returns Ok(None) if no packet available
    fn read_packet(&mut self, buf: &mut [u8]) -> Result<Option<usize>>;

    /// Write a packet to the network
    fn write_packet(&mut self, buf: &[u8]) -> Result<usize>;
}
```

---

## NetworkScheme Wrapper

The `NetworkScheme<T>` struct wraps a `NetworkAdapter` and exposes it as a Redox scheme:

```rust
// Create the scheme
let mut scheme = NetworkScheme::new(
    || device,  // FnOnce() -> T where T: NetworkAdapter
    daemon,     // daemon::Daemon
    format!("network.{}", name),  // Must start with "network"
);

// Event loop
loop {
    for event in event_queue {
        match event.user_data {
            Source::Irq => {
                // Handle interrupt
                if device.irq() {
                    scheme.tick()?;
                }
            }
            Source::Scheme => {
                // Handle scheme requests
                scheme.tick()?;
            }
        }
    }
}
```

### Scheme Endpoints

| Path | Description |
|------|-------------|
| `/scheme/network.X/` | Data endpoint (read/write packets) |
| `/scheme/network.X/mac` | Read MAC address (6 bytes) |

---

## Driver Template

```rust
// drivers/net/ath11kd/src/main.rs

use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;

use driver_network::NetworkScheme;
use event::{user_data, EventQueue};
use pcid_interface::PciFunctionHandle;

pub mod device;  // Ath11kDevice implementation

fn main() {
    pcid_interface::pci_daemon(daemon);
}

fn daemon(daemon: daemon::Daemon, mut pcid_handle: PciFunctionHandle) -> ! {
    let pci_config = pcid_handle.config();

    let mut name = pci_config.func.name();
    name.push_str("_ath11k");

    common::setup_logging("net", "pci", &name, common::output_level(), common::file_level());

    let irq = pci_config.func.legacy_interrupt_line
        .expect("ath11k: no legacy interrupts supported");

    let mut irq_file = irq.irq_handle("ath11kd");

    // Map PCIe BARs
    let bar0 = unsafe { pcid_handle.map_bar(0) };

    // Initialize device
    let mut scheme = NetworkScheme::new(
        move || unsafe {
            device::Ath11kDevice::new(bar0.ptr.as_ptr() as usize, ...)
                .expect("ath11k: failed to init")
        },
        daemon,
        format!("network.{name}"),
    );

    user_data! {
        enum Source {
            Irq,
            Scheme,
        }
    }

    let event_queue = EventQueue::<Source>::new().expect("ath11k: failed to create event queue");

    event_queue.subscribe(irq_file.as_raw_fd() as usize, Source::Irq, event::EventFlags::READ)
        .expect("ath11k: failed to subscribe to IRQ fd");
    event_queue.subscribe(scheme.event_handle().raw(), Source::Scheme, event::EventFlags::READ)
        .expect("ath11k: failed to subscribe to scheme fd");

    libredox::call::setrens(0, 0).expect("ath11k: failed to enter null namespace");

    scheme.tick().unwrap();

    for event in event_queue.map(|e| e.expect("ath11k: failed to get event")) {
        match event.user_data {
            Source::Irq => {
                let mut irq = [0; 8];
                irq_file.read(&mut irq).unwrap();
                if unsafe { scheme.adapter().irq() } {
                    irq_file.write(&mut irq).unwrap();
                    scheme.tick().expect("ath11k: failed to handle IRQ")
                }
            }
            Source::Scheme => scheme.tick().expect("ath11k: failed to handle scheme op"),
        }
    }
    unreachable!()
}
```

---

## PCI Configuration

Create `/lib/pcid.d/ath11k.toml`:

```toml
# QCA6390 WiFi (ath11k)
# Class: 0x02 (Network controller)
# Subclass: 0x80 (Other)
# Vendor: 0x17CB (Qualcomm)
# Device: 0x1101 (QCA6390)

[[drivers]]
name = "QCA6390 WiFi"
class = 2
subclass = 0x80
vendor = 0x17CB
device = 0x1101
command = ["/usr/lib/drivers/ath11kd"]
```

---

## Network Stack Integration (smolnetd)

smolnetd automatically discovers network adapters:

```rust
fn get_network_adapter() -> Result<String> {
    for entry_res in fs::read_dir("/scheme")? {
        let scheme = entry.file_name().into_string()?;
        if scheme.starts_with("network") {
            return Ok(scheme);
        }
    }
    bail!("no network adapter found");
}
```

It then:
1. Opens `/scheme/network.X/` for packet I/O
2. Reads `/scheme/network.X/mac` for MAC address
3. Uses **smoltcp** for TCP/IP stack
4. Exposes schemes: `ip:`, `tcp:`, `udp:`, `icmp:`, `netcfg:`

---

## Key Libraries

| Crate | Purpose |
|-------|---------|
| `driver-network` | NetworkAdapter trait, NetworkScheme wrapper |
| `pcid_interface` | PCI device access, BAR mapping |
| `redox_scheme` | Scheme implementation (Socket, Response) |
| `event` | Event queue for IRQ/scheme events |
| `common` | Logging, DMA allocation |
| `libredox` | Low-level Redox syscalls |
| `syscall` | System call definitions |

---

## e1000d Reference Implementation

### Device Structure

```rust
pub struct Intel8254x {
    base: usize,                          // MMIO base address
    mac_address: [u8; 6],
    receive_buffer: [Dma<[u8; 16384]>; 16],  // DMA buffers
    receive_ring: Dma<[Rd; 16]>,            // RX descriptor ring
    receive_index: usize,
    transmit_buffer: [Dma<[u8; 16384]>; 16],
    transmit_ring: Dma<[Td; 16]>,           // TX descriptor ring
    transmit_ring_free: usize,
    transmit_index: usize,
    transmit_clean_index: usize,
}
```

### Key Methods

```rust
impl Intel8254x {
    pub unsafe fn new(base: usize) -> Result<Self> { ... }
    pub unsafe fn irq(&self) -> bool { ... }
    pub unsafe fn read_reg(&self, register: u32) -> u32 { ... }
    pub unsafe fn write_reg(&self, register: u32, data: u32) -> u32 { ... }
}
```

---

## ath11k Implementation Challenges

### 1. MHI Protocol (Modem Host Interface)

QCA6390 uses MHI over PCIe for communication. This is Qualcomm-specific and has **no Rust implementation**.

**Linux reference:**
- `drivers/bus/mhi/` - MHI bus driver
- `drivers/net/wireless/ath/ath11k/mhi.c` - ath11k MHI integration

**Key structures:**
- Command Ring (CR)
- Event Ring (ER)
- Transfer Ring (TR)

### 2. Firmware Loading

QCA6390 requires firmware files:
- `amss.bin` - Main firmware
- `m3.bin` - DSP firmware
- `board-2.bin` - Board data file

These must be extracted from Android and loaded via MHI.

### 3. WMI (Wireless Management Interface)

For control operations:
- Scan requests
- Connect/disconnect
- Channel configuration

### 4. HTT (Hardware Transport Layer)

For data path:
- TX/RX packet processing
- Buffer management

---

## Development Path for ath11kd

### Phase 1: PCIe Layer
- [ ] Map PCIe BARs
- [ ] Configure MSI interrupts
- [ ] DMA buffer allocation

### Phase 2: MHI Layer
- [ ] Implement ring buffer structures
- [ ] Command/Event/Transfer channels
- [ ] Firmware download protocol

### Phase 3: WMI Layer
- [ ] Message serialization
- [ ] Scan command
- [ ] Connect command

### Phase 4: HTT Layer
- [ ] Data packet TX/RX
- [ ] Buffer queue management

### Phase 5: NetworkAdapter Integration
- [ ] Implement NetworkAdapter trait
- [ ] Register as network scheme

---

## Build System Integration

Add to `base/Cargo.toml`:

```toml
[workspace.dependencies.ath11kd]
path = "drivers/net/ath11kd"

[[bin]]
name = "ath11kd"
path = "drivers/net/ath11kd/src/main.rs"
```

Add to build script (recipe.toml):

```toml
BINS+=(ath11kd)
```

---

## References

- [Redox Book - Schemes](https://doc.redox-os.org/book/schemes.html)
- [Linux ath11k driver](https://github.com/torvalds/linux/tree/master/drivers/net/wireless/ath/ath11k)
- [Linux MHI bus](https://github.com/torvalds/linux/tree/master/drivers/bus/mhi)
- [QCA6390 firmware](https://github.com/kvalo/ath11k-firmware)

---

*Document version: v1.0*
*Created: 2026-03-24*
*Project: aginx - Agent OS for Pixel 5*
