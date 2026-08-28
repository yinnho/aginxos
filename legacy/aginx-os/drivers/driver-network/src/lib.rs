//! Network driver interface for Aginx OS
//!
//! Provides the NetworkAdapter trait that all network drivers must implement.

#![no_std]

extern crate alloc;

use alloc::string::String;
use aginx_scheme::{Error, Scheme, Socket};
use aginx_syscall::{Fd, OFlag, Stat};

/// Network adapter trait
///
/// All network drivers must implement this trait.
/// The driver provides raw packet access, and smolnetd handles TCP/IP.
pub trait NetworkAdapter {
    /// Get the MAC address of the adapter
    fn mac_address(&mut self) -> [u8; 6];

    /// Check if a packet is available to read (non-blocking)
    fn has_packet(&mut self) -> bool;

    /// Receive a packet into the buffer
    /// Returns the number of bytes read, or WouldBlock if no packet
    fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, Error>;

    /// Transmit a packet from the buffer
    /// Returns the number of bytes written
    fn transmit(&mut self, buffer: &[u8]) -> Result<usize, Error>;

    /// Handle interrupt (called by ISR)
    /// Returns true if interrupt was handled
    fn handle_interrupt(&mut self) -> bool;

    /// Get link status (true if connected)
    fn link_up(&self) -> bool;

    /// Get adapter name
    fn name(&self) -> &str;
}

/// Network statistics
#[derive(Default, Clone, Copy)]
pub struct NetworkStats {
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_errors: u64,
    pub tx_errors: u64,
    pub rx_dropped: u64,
    pub tx_dropped: u64,
}

/// Network scheme wrapper
///
/// Wraps a NetworkAdapter and exposes it as a scheme (network:)
pub struct NetworkScheme<T: NetworkAdapter> {
    adapter: T,
    name: String,
    stats: NetworkStats,
}

impl<T: NetworkAdapter> NetworkScheme<T> {
    /// Create a new network scheme
    pub fn new(adapter: T, name: &str) -> Self {
        NetworkScheme {
            adapter,
            name: String::from(name),
            stats: NetworkStats::default(),
        }
    }

    /// Get the MAC address as a string
    pub fn mac_string(&mut self) -> String {
        let mac = self.adapter.mac_address();
        alloc::format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        )
    }

    /// Get network statistics
    pub fn stats(&self) -> &NetworkStats {
        &self.stats
    }

    /// Process pending packets (call in event loop)
    pub fn tick(&mut self) {
        // Process any pending interrupts
        self.adapter.handle_interrupt();
    }

    /// Get reference to adapter
    pub fn adapter(&self) -> &T {
        &self.adapter
    }

    /// Get mutable reference to adapter
    pub fn adapter_mut(&mut self) -> &mut T {
        &mut self.adapter
    }
}

/// File types for network scheme
enum NetworkFd {
    /// Main data endpoint (read/write packets)
    Data,
    /// MAC address endpoint
    Mac,
    /// Statistics endpoint
    Stats,
    /// Link status endpoint
    Link,
}

impl<T: NetworkAdapter> Scheme for NetworkScheme<T> {
    fn open(&mut self, path: &str, flags: OFlag) -> Result<Fd, Error> {
        match path {
            "" | "/" => Ok(0), // Data endpoint
            "/mac" => Ok(1),   // MAC address
            "/stats" => Ok(2), // Statistics
            "/link" => Ok(3),  // Link status
            _ => Err(Error::NoSuchEntry),
        }
    }

    fn close(&mut self, fd: Fd) -> Result<(), Error> {
        Ok(())
    }

    fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Error> {
        match fd {
            0 => {
                // Read packet
                let len = self.adapter.receive(buf)?;
                self.stats.rx_packets += 1;
                self.stats.rx_bytes += len as u64;
                Ok(len)
            }
            1 => {
                // Read MAC address
                let mac = self.adapter.mac_address();
                if buf.len() >= 6 {
                    buf[..6].copy_from_slice(&mac);
                    Ok(6)
                } else {
                    Err(Error::InvalidArgs)
                }
            }
            2 => {
                // Read stats (JSON format)
                let stats_str = alloc::format!(
                    "{{\"rx\":{},\"tx\":{},\"rx_bytes\":{},\"tx_bytes\":{}}}",
                    self.stats.rx_packets,
                    self.stats.tx_packets,
                    self.stats.rx_bytes,
                    self.stats.tx_bytes
                );
                let bytes = stats_str.as_bytes();
                let len = bytes.len().min(buf.len());
                buf[..len].copy_from_slice(&bytes[..len]);
                Ok(len)
            }
            3 => {
                // Read link status
                buf[0] = if self.adapter.link_up() { b'1' } else { b'0' };
                Ok(1)
            }
            _ => Err(Error::BadAddress),
        }
    }

    fn write(&mut self, fd: Fd, buf: &[u8]) -> Result<usize, Error> {
        match fd {
            0 => {
                // Write packet
                let len = self.adapter.transmit(buf)?;
                self.stats.tx_packets += 1;
                self.stats.tx_bytes += len as u64;
                Ok(len)
            }
            _ => Err(Error::NotPermitted),
        }
    }

    fn fstat(&mut self, fd: Fd, stat: &mut Stat) -> Result<(), Error> {
        // Fill in stat structure
        Ok(())
    }

    fn dup(&mut self, fd: Fd, buf: &[u8]) -> Result<Fd, Error> {
        // Duplicate fd
        Ok(fd)
    }

    fn name(&self) -> &str {
        &self.name
    }
}
