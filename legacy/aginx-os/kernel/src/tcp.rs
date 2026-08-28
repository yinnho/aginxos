//! TCP/IP stack — thin wrapper over custom ip_stack
//!
//! Provides type-compatible API with the old smoltcp-based implementation.

use crate::ip_stack::TcpConnState;

// Conditional UART imports
#[cfg(not(feature = "board-redfin"))]
use crate::uart;
#[cfg(feature = "board-redfin")]
use crate::qup_uart as uart;

use crate::platform::UART;

// ─── Smoltcp-compatible address wrappers ──────────────────────────────────────

/// IPv4 address wrapper (compatible with smoltcp's Ipv4Address API)
#[derive(Clone, Copy)]
pub struct Ipv4Address(pub [u8; 4]);

impl Ipv4Address {
    pub fn octets(&self) -> [u8; 4] { self.0 }
    pub fn is_unspecified(&self) -> bool { self.0 == [0, 0, 0, 0] }
}

/// Ethernet address wrapper (compatible with smoltcp's EthernetAddress API)
#[derive(Clone, Copy)]
pub struct EthernetAddress(pub [u8; 6]);

// ─── TCP State (compatible with old smoltcp-based TcpState) ───────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TcpState {
    None,
    Listening,
    Connected,
    Closed,
}

fn map_state(s: TcpConnState) -> TcpState {
    match s {
        TcpConnState::Established => TcpState::Connected,
        TcpConnState::CloseWait => TcpState::Closed,
        TcpConnState::Closed => TcpState::Closed,
        TcpConnState::Listen => TcpState::Listening,
        TcpConnState::SynSent | TcpConnState::SynReceived => TcpState::Listening,
        _ => TcpState::None,
    }
}

// ─── Public API delegating to ip_stack ─────────────────────────────────────────

pub fn init() -> bool {
    crate::ip_stack::init()
}

pub fn poll() -> usize {
    crate::ip_stack::poll()
}

pub fn wait_for_dhcp(max_iters: u32) -> bool {
    crate::ip_stack::wait_for_dhcp(max_iters)
}

pub fn get_ip() -> Option<Ipv4Address> {
    crate::ip_stack::get_ip().map(Ipv4Address)
}

pub fn get_gateway() -> Option<Ipv4Address> {
    crate::ip_stack::get_gateway().map(Ipv4Address)
}

pub fn get_mac() -> Option<EthernetAddress> {
    crate::ip_stack::get_mac().map(EthernetAddress)
}

pub fn get_dns_server() -> Option<Ipv4Address> {
    crate::ip_stack::get_dns_server().map(Ipv4Address)
}

pub fn print_status() {
    crate::ip_stack::print_status()
}

pub fn tcp_slot_alloc() -> Option<usize> {
    crate::ip_stack::tcp_slot_alloc()
}

pub fn tcp_slot_listen(slot: usize, port: u16) -> bool {
    crate::ip_stack::tcp_slot_listen(slot, port)
}

pub fn tcp_slot_connect(slot: usize, ip: [u8; 4], port: u16) -> bool {
    crate::ip_stack::tcp_slot_connect(slot, ip, port)
}

pub fn tcp_slot_state(slot: usize) -> TcpState {
    map_state(crate::ip_stack::tcp_slot_state(slot))
}

pub fn tcp_slot_send(slot: usize, data: &[u8]) -> bool {
    crate::ip_stack::tcp_slot_send(slot, data)
}

pub fn tcp_slot_recv(slot: usize, buf: &mut [u8]) -> usize {
    crate::ip_stack::tcp_slot_recv(slot, buf)
}

pub fn tcp_slot_close(slot: usize) {
    crate::ip_stack::tcp_slot_close(slot)
}

pub fn tcp_slot_send_blocking(slot: usize, data: &[u8]) {
    crate::ip_stack::tcp_slot_send_blocking(slot, data)
}

// Legacy slot-0 wrappers
pub fn tcp_listen(port: u16) -> bool { tcp_slot_listen(0, port) }
pub fn tcp_listen_state() -> TcpState { tcp_slot_state(0) }
pub fn tcp_connect(ip: [u8; 4], port: u16) -> bool { tcp_slot_connect(0, ip, port) }
pub fn tcp_connect_state() -> TcpState { tcp_slot_state(0) }
pub fn tcp_send(data: &[u8]) -> bool { tcp_slot_send(0, data) }
pub fn tcp_recv(buf: &mut [u8]) -> usize { tcp_slot_recv(0, buf) }
pub fn tcp_close() { tcp_slot_close(0) }
pub fn tcp_close_listen() { tcp_slot_close(0) }
pub fn tcp_send_str(data: &[u8]) -> bool { tcp_slot_send(0, data) }
pub fn tcp_send_str_blocking(data: &[u8]) { tcp_slot_send_blocking(0, data) }

pub fn check_tcp_data() {
    crate::ip_stack::check_tcp_data()
}

pub fn ping(ip: [u8; 4], count: u8) -> u8 {
    crate::ip_stack::ping(ip, count)
}

pub fn dns_resolve(_name: &[u8]) -> Option<Ipv4Address> {
    uart::puts(UART, "[FAIL] DNS not implemented (Phase 2)\r\n");
    None
}

// Stub DNS functions for compatibility
#[allow(dead_code)]
pub fn dns_start_query(_name: &[u8]) -> bool { false }

#[allow(dead_code)]
pub fn dns_poll() -> Option<Ipv4Address> { None }

/// Get UART address (kept for compatibility)
#[allow(dead_code)]
pub fn uart_addr() -> usize { UART }
