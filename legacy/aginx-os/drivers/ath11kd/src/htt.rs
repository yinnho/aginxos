//! HTT (Hardware Transport Layer) Protocol
//!
//! HTT provides data plane operations for ath11k:
//! - TX/RX packet handling
//! - Buffer management
//! - Statistics

use alloc::vec::Vec;

/// HTT message types
#[repr(u8)]
pub enum HttType {
    /// TX data
    Tx = 0x01,
    /// TX completion
    TxCompletion = 0x02,
    /// RX data
    Rx = 0x03,
    /// RX flush
    RxFlush = 0x04,
    /// Statistics
    Stats = 0x05,
    /// HTC message
    Htc = 0x06,
}

/// HTT TX descriptor
#[repr(C)]
pub struct HttTxDesc {
    /// Message type (HTT_TYPE_TX)
    msg_type: u8,
    /// Pkt length (MSDU length)
    pkt_len: u16,
    /// TX descriptor info
    desc_info: u32,
    /// Fragment info
    frag_info: u64,
}

/// HTT RX indication
#[repr(C)]
pub struct HttRxInd {
    /// Message type (HTT_TYPE_RX)
    msg_type: u8,
    /// RX info
    info0: u32,
    /// PPDU start info
    ppdu_start: u64,
    /// MPDU range info
    mpdu_range: u32,
    /// Number of MPDUs
    num_mpdu_ranges: u8,
}

/// HTT TX completion
#[repr(C)]
pub struct HttTxCompletion {
    /// Message type
    msg_type: u8,
    /// Status
    status: u8,
    /// Number of acks
    num_acks: u16,
    /// Peer ID
    peer_id: u16,
    /// TID
    tid: u8,
}

/// TX completion status
pub enum TxStatus {
    Success = 0,
    Discard = 1,
    NoAck = 2,
    Drop = 3,
}

/// HTT manager
pub struct HttManager {
    /// TX buffer pool
    tx_buffers: Vec<TxBuffer>,
    /// RX buffer pool
    rx_buffers: Vec<RxBuffer>,
    /// Statistics
    stats: HttStats,
}

/// TX buffer
struct TxBuffer {
    /// Buffer ID
    id: u16,
    /// DMA address
    dma_addr: u64,
    /// Buffer size
    size: usize,
    /// In use flag
    in_use: bool,
}

/// RX buffer
struct RxBuffer {
    /// Buffer ID
    id: u16,
    /// DMA address
    dma_addr: u64,
    /// Buffer size
    size: usize,
    /// Ready flag (has packet)
    ready: bool,
}

/// HTT statistics
#[derive(Default)]
pub struct HttStats {
    pub tx_packets: u64,
    pub tx_bytes: u64,
    pub tx_errors: u64,
    pub rx_packets: u64,
    pub rx_bytes: u64,
    pub rx_errors: u64,
}

impl HttManager {
    /// Create new HTT manager
    pub fn new(tx_buffers: usize, rx_buffers: usize) -> Self {
        // TODO: Allocate DMA buffers
        HttManager {
            tx_buffers: Vec::new(),
            rx_buffers: Vec::new(),
            stats: HttStats::default(),
        }
    }

    /// Allocate TX buffer
    pub fn alloc_tx_buffer(&mut self) -> Option<u16> {
        for (i, buf) in self.tx_buffers.iter_mut().enumerate() {
            if !buf.in_use {
                buf.in_use = true;
                return Some(buf.id);
            }
        }
        None
    }

    /// Free TX buffer
    pub fn free_tx_buffer(&mut self, id: u16) {
        if let Some(buf) = self.tx_buffers.iter_mut().find(|b| b.id == id) {
            buf.in_use = false;
        }
    }

    /// Queue packet for transmission
    pub fn queue_tx(&mut self, buffer_id: u16, data: &[u8]) -> Result<(), HttError> {
        // TODO: Build HTT TX descriptor and queue to TX ring
        self.stats.tx_packets += 1;
        self.stats.tx_bytes += data.len() as u64;
        Ok(())
    }

    /// Poll for received packets
    pub fn poll_rx(&mut self) -> Option<RxPacket> {
        // TODO: Check RX ring for completed packets
        None
    }

    /// Handle TX completion
    pub fn handle_tx_completion(&mut self, comp: &HttTxCompletion) {
        match comp.status {
            0 => self.stats.tx_packets += 1,
            _ => self.stats.tx_errors += 1,
        }
    }
}

/// Received packet
pub struct RxPacket {
    /// Buffer ID
    pub buffer_id: u16,
    /// Data length
    pub len: usize,
    /// RSSI
    pub rssi: i32,
}

#[derive(Debug)]
pub enum HttError {
    NoBuffer,
    QueueFull,
    InvalidBuffer,
}

impl Default for HttManager {
    fn default() -> Self {
        Self::new(256, 256)
    }
}
