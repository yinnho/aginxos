//! MHI (Modem Host Interface) Protocol
//!
//! MHI is the transport layer for Qualcomm WiFi/Modem chips.
//! It provides:
//! - Command Channel (for control)
//! - Event Channel (for notifications)
//! - Transfer Channels (for data)
//!
//! Ring structure:
//! ```
//! ┌─────────────────────────────────────┐
//! │        Ring Buffer (DMA)            │
//! ├─────────────────────────────────────┤
//! │  Element 0 │ Element 1 │ ... │ N    │
//! ├─────────────────────────────────────┤
//! │  WP (Write Ptr) │ RP (Read Ptr)     │
//! └─────────────────────────────────────┘
//! ```

use bitflags::bitflags;

/// MHI context (ring structures)
pub struct MhiContext {
    /// Command ring
    cmd_ring: Ring,
    /// Event ring
    event_ring: Ring,
    /// Transfer ring for TX
    tx_ring: Ring,
    /// Transfer ring for RX
    rx_ring: Ring,
}

/// Generic ring structure
pub struct Ring {
    /// Ring base address (DMA)
    base: *mut RingElement,
    /// Number of elements
    count: usize,
    /// Read pointer (device)
    rp: usize,
    /// Write pointer (host)
    wp: usize,
}

/// Ring element
#[repr(C)]
pub struct RingElement {
    /// Buffer pointer (DMA address)
    buffer_ptr: u64,
    /// Buffer size
    buffer_size: u32,
    /// Reserved
    _reserved: u32,
}

/// MHI channel IDs
pub enum ChannelId {
    /// WMI control (host -> device)
    WmiControlTx = 0,
    /// WMI control (device -> host)
    WmiControlRx = 1,
    /// HTT data TX
    HttTx = 3,
    /// HTT data RX
    HttRx = 4,
}

/// MHI commands
pub enum Command {
    /// Reset channel
    ResetChan { chan: u16 },
    /// Start channel
    StartChan { chan: u16 },
    /// Power up sequence
    PcuPm,
}

/// MHI states
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MhiState {
    Reset,
    Ready,
    MissionMode,
    Error,
}

/// MHI registers (MMIO offsets)
pub mod regs {
    pub const MHIREGLEN: usize = 0x00;
    pub const MHIVER: usize = 0x08;
    pub const MHICFG: usize = 0x10;
    pub const CHDBOFF: usize = 0x18;
    pub const ERDBOFF: usize = 0x20;
    pub const BHIOFF: usize = 0x28;
    pub const DEBUGOFF: usize = 0x30;
    pub const MHICTRL: usize = 0x38;
    pub const MHISTATUS: usize = 0x48;
    pub const CCABAP_LOWER: usize = 0x58;
    pub const CCABAP_HIGHER: usize = 0x5C;
    pub const ECABAP_LOWER: usize = 0x60;
    pub const ECABAP_HIGHER: usize = 0x64;
    pub const CRCBAP_LOWER: usize = 0x68;
    pub const CRCBAP_HIGHER: usize = 0x6C;
    pub const CRDB_LOWER: usize = 0x70;
    pub const CRDB_HIGHER: usize = 0x74;
    pub const MHICTRLBUTTON: usize = 0x78;
}

bitflags! {
    /// MHI control flags
    pub struct MhiCtrl: u32 {
        const RESET = 1 << 0;
        const WAKEUP = 1 << 1;
        const SLEEP = 1 << 2;
    }
}

impl Ring {
    /// Create a new ring
    pub fn new(count: usize) -> Self {
        Ring {
            base: core::ptr::null_mut(),
            count,
            rp: 0,
            wp: 0,
        }
    }

    /// Check if ring is empty
    pub fn is_empty(&self) -> bool {
        self.rp == self.wp
    }

    /// Check if ring is full
    pub fn is_full(&self) -> bool {
        (self.wp + 1) % self.count == self.rp
    }

    /// Get next write slot
    pub fn write_slot(&mut self) -> Option<&mut RingElement> {
        if self.is_full() {
            return None;
        }
        unsafe { Some(&mut *self.base.add(self.wp)) }
    }

    /// Commit write
    pub fn commit_write(&mut self) {
        self.wp = (self.wp + 1) % self.count;
    }

    /// Get next read slot
    pub fn read_slot(&self) -> Option<&RingElement> {
        if self.is_empty() {
            return None;
        }
        unsafe { Some(&*self.base.add(self.rp)) }
    }

    /// Commit read
    pub fn commit_read(&mut self) {
        self.rp = (self.rp + 1) % self.count;
    }
}

impl MhiContext {
    /// Initialize MHI context
    pub fn new() -> Self {
        MhiContext {
            cmd_ring: Ring::new(32),
            event_ring: Ring::new(64),
            tx_ring: Ring::new(512),
            rx_ring: Ring::new(512),
        }
    }

    /// Send command
    pub fn send_command(&mut self, cmd: Command) -> Result<(), MhiError> {
        // TODO: Queue command to cmd_ring
        Ok(())
    }

    /// Poll for events
    pub fn poll_events(&mut self) -> Option<Event> {
        // TODO: Check event_ring for events
        None
    }
}

/// MHI event
pub enum Event {
    /// Transfer complete
    TransferComplete { chan: u16, status: u32 },
    /// Command complete
    CommandComplete { seq: u16, status: u32 },
    /// State change
    StateChange(MhiState),
}

#[derive(Debug)]
pub enum MhiError {
    RingFull,
    RingEmpty,
    Timeout,
    InvalidState,
}
