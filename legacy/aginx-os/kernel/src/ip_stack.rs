//! Custom TCP/IP stack for Aginx OS
//!
//! Replaces smoltcp to avoid QEMU ARM64 TCG codegen hang.
//! Implements: ARP, IPv4, ICMP Echo, DHCP, TCP

use core::hint::spin_loop;

// Conditional UART imports
#[cfg(not(feature = "board-redfin"))]
use crate::uart;
#[cfg(feature = "board-redfin")]
use crate::qup_uart as uart;

use crate::platform::UART;

// ─── Constants ────────────────────────────────────────────────────────────────

const ETH_TYPE_IPV4: u16 = 0x0800;
const ETH_TYPE_ARP: u16 = 0x0806;
const IP_PROTO_ICMP: u8 = 1;
const IP_PROTO_TCP: u8 = 6;
const IP_PROTO_UDP: u8 = 17;

const ICMP_ECHO_REQUEST: u8 = 8;
const ICMP_ECHO_REPLY: u8 = 0;

const TCP_SYN: u16 = 0x02;
const TCP_ACK: u16 = 0x10;
const TCP_FIN: u16 = 0x01;
const TCP_RST: u16 = 0x04;
const TCP_PSH: u16 = 0x08;

const DHCP_DISCOVER: u8 = 1;
const DHCP_OFFER: u8 = 2;
const DHCP_REQUEST: u8 = 3;
const DHCP_ACK: u8 = 5;

const ARP_CACHE_SIZE: usize = 8;
const TCP_SLOT_COUNT: usize = 4;
const TCP_BUF_SIZE: usize = 2048;
const MSS: usize = 1460;

const BROADCAST_MAC: [u8; 6] = [0xFF; 6];

// ─── TCP State ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum TcpConnState {
    Closed = 0,
    Listen = 1,
    SynSent = 2,
    SynReceived = 3,
    Established = 4,
    CloseWait = 5,
    LastAck = 6,
    FinWait1 = 7,
    FinWait2 = 8,
    Closing = 9,
    TimeWait = 10,
}

// ─── TCP Connection ──────────────────────────────────────────────────────────

struct TcpConn {
    state: TcpConnState,
    local_port: u16,
    remote_port: u16,
    remote_ip: [u8; 4],
    // Sequence numbers
    snd_una: u32,  // oldest unacked seq
    snd_nxt: u32,  // next seq to send
    snd_wnd: u16,  // send window
    rcv_nxt: u32,  // next expected seq
    rcv_wnd: u16,  // receive window
    iss: u32,      // initial send seq
    // Buffers
    rx_buf: [u8; TCP_BUF_SIZE],
    rx_len: usize,  // data in rx buffer
    rx_read: usize, // read cursor
    tx_buf: [u8; TCP_BUF_SIZE],
    tx_len: usize,  // unsent data in tx buffer
    // Timers
    retransmit_at: u64, // timestamp for retransmit
    retransmit_count: u8,
    // Flags
    used: bool,
    kind: u8, // 0=free, 1=listen, 2=connect
}

// ─── ARP Entry ───────────────────────────────────────────────────────────────

struct ArpEntry {
    ip: [u8; 4],
    mac: [u8; 6],
    valid: bool,
}

// ─── IP Stack State ─────────────────────────────────────────────────────────

struct IpStack {
    mac: [u8; 6],
    ip: [u8; 4],
    gateway: [u8; 4],
    netmask: [u8; 4],
    dns_server: [u8; 4],
    ip_acquired: bool,

    // ARP cache
    arp_cache: [ArpEntry; ARP_CACHE_SIZE],

    // DHCP state
    dhcp_xid: u32,
    dhcp_state: u8, // 0=init, 1=selecting, 2=requesting, 3=bound
    dhcp_server_ip: [u8; 4],
    dhcp_requested_ip: [u8; 4],

    // TCP connections
    tcp_conns: [TcpConn; TCP_SLOT_COUNT],

    // IP packet ID
    ip_id: u16,

    // ICMP ping state
    icmp_ident: u16,
    icmp_seq: u16,
    icmp_recv_count: u8,
}

static mut IP_STACK: *mut IpStack = core::ptr::null_mut();

// ─── Helper Functions ────────────────────────────────────────────────────────

fn mac_eq(a: &[u8; 6], b: &[u8; 6]) -> bool {
    a[0] == b[0] && a[1] == b[1] && a[2] == b[2] && a[3] == b[3] && a[4] == b[4] && a[5] == b[5]
}

fn ip_eq(a: &[u8; 4], b: &[u8; 4]) -> bool {
    a[0] == b[0] && a[1] == b[1] && a[2] == b[2] && a[3] == b[3]
}

fn ip_is_zero(ip: &[u8; 4]) -> bool {
    ip[0] == 0 && ip[1] == 0 && ip[2] == 0 && ip[3] == 0
}

fn read_be16(buf: &[u8], off: usize) -> u16 {
    ((buf[off] as u16) << 8) | (buf[off + 1] as u16)
}

fn read_be32(buf: &[u8], off: usize) -> u32 {
    ((buf[off] as u32) << 24) | ((buf[off + 1] as u32) << 16) |
    ((buf[off + 2] as u32) << 8) | (buf[off + 3] as u32)
}

fn write_be16(buf: &mut [u8], off: usize, val: u16) {
    buf[off] = (val >> 8) as u8;
    buf[off + 1] = val as u8;
}

fn write_be32(buf: &mut [u8], off: usize, val: u32) {
    buf[off] = (val >> 24) as u8;
    buf[off + 1] = (val >> 16) as u8;
    buf[off + 2] = (val >> 8) as u8;
    buf[off + 3] = val as u8;
}

/// Compute IP checksum (one's complement sum) with volatile reads
#[inline(never)]
fn checksum(buf: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let ptr = buf.as_ptr();
    let len = buf.len();
    let mut i = 0;
    while i + 1 < len {
        let hi = unsafe { core::ptr::read_volatile(ptr.add(i)) };
        let lo = unsafe { core::ptr::read_volatile(ptr.add(i + 1)) };
        sum += ((hi as u32) << 8) | (lo as u32);
        i += 2;
    }
    if i < len {
        let b = unsafe { core::ptr::read_volatile(ptr.add(i)) };
        sum += (b as u32) << 8;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !sum as u16
}

/// Checksum with pseudo-header for TCP/UDP
#[inline(never)]
fn checksum_with_pseudo(src_ip: &[u8; 4], dst_ip: &[u8; 4], proto: u8, data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    // Pseudo header — 16-bit big-endian words
    sum += ((src_ip[0] as u32) << 8) | (src_ip[1] as u32);
    sum += ((src_ip[2] as u32) << 8) | (src_ip[3] as u32);
    sum += ((dst_ip[0] as u32) << 8) | (dst_ip[1] as u32);
    sum += ((dst_ip[2] as u32) << 8) | (dst_ip[3] as u32);
    sum += proto as u32;
    sum += data.len() as u32;
    // Data — volatile reads to match volatile writes
    let ptr = data.as_ptr();
    let len = data.len();
    let mut i = 0;
    while i + 1 < len {
        let hi = unsafe { core::ptr::read_volatile(ptr.add(i)) };
        let lo = unsafe { core::ptr::read_volatile(ptr.add(i + 1)) };
        sum += ((hi as u32) << 8) | (lo as u32);
        i += 2;
    }
    if i < len {
        let b = unsafe { core::ptr::read_volatile(ptr.add(i)) };
        sum += (b as u32) << 8;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !sum as u16
}

fn get_timestamp_ms() -> u64 {
    crate::interrupt::get_ticks() as u64 * 10 // 10ms per tick
}

/// Heap allocate zeroed memory (separated to avoid QEMU ARM64 codegen hang)
#[inline(never)]
fn heap_alloc_zeroed(size: usize) -> *mut u8 {
    unsafe {
        let layout = alloc::alloc::Layout::from_size_align(size, 8).unwrap();
        alloc::alloc::alloc_zeroed(layout)
    }
}

/// Heap allocate zeroed memory with specific alignment
#[inline(never)]
fn heap_alloc_zeroed_aligned(size: usize, align: usize) -> *mut u8 {
    unsafe {
        let layout = alloc::alloc::Layout::from_size_align(size, align).unwrap();
        alloc::alloc::alloc_zeroed(layout)
    }
}

/// Deallocate heap memory
#[inline(never)]
fn heap_dealloc(ptr: *mut u8, size: usize, align: usize) {
    unsafe {
        let layout = alloc::alloc::Layout::from_size_align(size, align).unwrap();
        alloc::alloc::dealloc(ptr, layout);
    }
}

// ─── ARP Test (debug) ─────────────────────────────────────────────────────

/// Test ARP by sending a request for the gateway
pub fn arp_resolve_test() {
    unsafe {
        if IP_STACK.is_null() { return; }
        let stack = &mut *IP_STACK;
        let gw = stack.gateway;
        uart::puts(UART, "  ARP req GW\r\n");
        arp_send_request(stack, &gw);
        uart::puts(UART, "  ARP sent\r\n");
    }
}

// ─── Ethernet Frame Construction ─────────────────────────────────────────────

/// Build and transmit an Ethernet frame (pads to 60 bytes minimum)
/// Volatile copy from potentially volatile-written memory
#[inline(always)]
unsafe fn volatile_copy(dst: *mut u8, src: *const u8, len: usize) {
    for i in 0..len {
        core::ptr::write_volatile(dst.add(i), core::ptr::read_volatile(src.add(i)));
    }
}

#[inline(never)]
fn eth_send(dst_mac: &[u8; 6], src_mac: &[u8; 6], ethertype: u16, payload: &[u8]) {
    let total_len = 14 + payload.len();
    if total_len > 1514 { return; }
    let send_len = if total_len < 60 { 60 } else { total_len };
    // Use heap buffer with volatile writes to avoid TCG hang
    let frame = heap_alloc_zeroed_aligned(send_len, 8);
    if frame.is_null() { return; }
    unsafe {
        // Ethernet header
        for i in 0..6 { core::ptr::write_volatile(frame.add(i), dst_mac[i]); }
        for i in 0..6 { core::ptr::write_volatile(frame.add(6 + i), src_mac[i]); }
        core::ptr::write_volatile(frame.add(12), (ethertype >> 8) as u8);
        core::ptr::write_volatile(frame.add(13), ethertype as u8);
        // Payload — use volatile read for each byte (source may be volatile-written memory)
        for i in 0..payload.len() {
            let b = core::ptr::read_volatile(payload.as_ptr().add(i));
            core::ptr::write_volatile(frame.add(14 + i), b);
        }
        let slice = core::slice::from_raw_parts(frame, send_len);
        crate::net::transmit(slice);
        heap_dealloc(frame, send_len, 8);
    }
}

// ─── ARP ─────────────────────────────────────────────────────────────────────

#[inline(never)]
fn arp_send_request(stack: &IpStack, target_ip: &[u8; 4]) {
    // Build ARP request using volatile writes to avoid TCG hang
    let pkt_size = 28usize;
    let pkt_buf = heap_alloc_zeroed_aligned(pkt_size, 8);
    if pkt_buf.is_null() { return; }
    unsafe {
        // HTYPE=1 (Ethernet)
        core::ptr::write_volatile(pkt_buf, 0x00);
        core::ptr::write_volatile(pkt_buf.add(1), 0x01);
        // PTYPE=0x0800 (IPv4)
        core::ptr::write_volatile(pkt_buf.add(2), 0x08);
        core::ptr::write_volatile(pkt_buf.add(3), 0x00);
        // HLEN=6, PLEN=4
        core::ptr::write_volatile(pkt_buf.add(4), 6);
        core::ptr::write_volatile(pkt_buf.add(5), 4);
        // OPER=1 (request)
        core::ptr::write_volatile(pkt_buf.add(6), 0x00);
        core::ptr::write_volatile(pkt_buf.add(7), 0x01);
        // SHA (our MAC)
        for i in 0..6 { core::ptr::write_volatile(pkt_buf.add(8 + i), stack.mac[i]); }
        // SPA (our IP)
        for i in 0..4 { core::ptr::write_volatile(pkt_buf.add(14 + i), stack.ip[i]); }
        // THA = 0 (already zeroed)
        // TPA (target IP)
        for i in 0..4 { core::ptr::write_volatile(pkt_buf.add(24 + i), target_ip[i]); }
        let slice = core::slice::from_raw_parts(pkt_buf, pkt_size);
        eth_send(&BROADCAST_MAC, &stack.mac, ETH_TYPE_ARP, slice);
        heap_dealloc(pkt_buf, pkt_size, 8);
    }
}

#[inline(never)]
fn arp_process(stack: &mut IpStack, data: &[u8]) {
    if data.len() < 28 { return; }
    // Copy ARP data using volatile reads to avoid TCG codegen hang
    let mut buf: [u8; 28] = [0; 28];
    let copy_len = if data.len() < 28 { data.len() } else { 28 };
    unsafe {
        let src = data.as_ptr();
        for i in 0..copy_len {
            buf[i] = core::ptr::read_volatile(src.add(i));
        }
    }
    let oper = ((buf[6] as u16) << 8) | (buf[7] as u16);
    let spa = [buf[14], buf[15], buf[16], buf[17]];
    let sha = &buf[8..14];
    let tpa = [buf[24], buf[25], buf[26], buf[27]];

    // Learn ARP from any packet
    if !ip_is_zero(&spa) {
        arp_cache_update(stack, &spa, sha);
    }

    if oper == 1 {
        // ARP Request: is it asking for our IP?
        if ip_eq(&tpa, &stack.ip) {
            let mut reply: [u8; 28] = [0; 28];
            write_be16(&mut reply, 0, 1);
            write_be16(&mut reply, 2, 0x0800);
            reply[4] = 6; reply[5] = 4;
            write_be16(&mut reply, 6, 2); // reply
            reply[8..14].copy_from_slice(&stack.mac);
            reply[14..18].copy_from_slice(&stack.ip);
            reply[18..24].copy_from_slice(sha);
            reply[24..28].copy_from_slice(&spa);
            let dst = [sha[0], sha[1], sha[2], sha[3], sha[4], sha[5]];
            eth_send(&dst, &stack.mac, ETH_TYPE_ARP, &reply);
        }
    }
}

fn arp_cache_update(stack: &mut IpStack, ip: &[u8; 4], mac: &[u8]) {
    // Update existing or find empty slot
    for i in 0..ARP_CACHE_SIZE {
        let e = &mut stack.arp_cache[i];
        if e.valid && ip_eq(&e.ip, ip) {
            e.mac.copy_from_slice(mac);
            return;
        }
    }
    // Find empty slot
    for i in 0..ARP_CACHE_SIZE {
        let e = &mut stack.arp_cache[i];
        if !e.valid {
            e.valid = true;
            e.ip = *ip;
            e.mac.copy_from_slice(mac);
            return;
        }
    }
    // Evict oldest (simple: overwrite first entry)
    let e = &mut stack.arp_cache[0];
    e.valid = true;
    e.ip = *ip;
    e.mac.copy_from_slice(mac);
}

#[inline(never)]
fn arp_lookup(stack: &IpStack, ip: &[u8; 4]) -> Option<[u8; 6]> {
    for i in 0..ARP_CACHE_SIZE {
        let e = &stack.arp_cache[i];
        let valid = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(e.valid)) };
        if valid {
            let cache_ip = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(e.ip)) };
            if ip_eq(&cache_ip, ip) {
                let mac = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(e.mac)) };
                return Some(mac);
            }
        }
    }
    None
}

/// Resolve IP to MAC. Non-blocking: sends ARP request if not cached,
/// returns None if reply hasn't arrived yet (will be processed on next poll).
#[inline(never)]
fn arp_resolve(stack: *mut IpStack, ip: &[u8; 4]) -> Option<[u8; 6]> {
    unsafe {
        if let Some(mac) = arp_lookup(&*stack, ip) {
            return Some(mac);
        }
        // Send ARP request — reply will be processed on next poll() call
        arp_send_request(&*stack, ip);
        None
    }
}

// ─── IPv4 ────────────────────────────────────────────────────────────────────

#[inline(never)]
fn ipv4_send(stack: &mut IpStack, dst_ip: &[u8; 4], proto: u8, payload: &[u8]) {
    let dst_mac = match arp_resolve(stack, dst_ip) {
        Some(m) => m,
        None => { return; }
    };

    let total_len = 20 + payload.len();
    if total_len > 1500 { return; }
    let buf_size = if total_len < 600 { 600 } else { total_len };
    // Use heap allocation with volatile writes to avoid TCG codegen hang
    let pkt = heap_alloc_zeroed_aligned(buf_size, 8);
    if pkt.is_null() { return; }
    unsafe {
        // Version=4, IHL=5, TOS=0
        core::ptr::write_volatile(pkt, 0x45);
        core::ptr::write_volatile(pkt.add(2), (total_len as u16 >> 8) as u8);
        core::ptr::write_volatile(pkt.add(3), total_len as u8);
        stack.ip_id = stack.ip_id.wrapping_add(1);
        core::ptr::write_volatile(pkt.add(4), (stack.ip_id >> 8) as u8);
        core::ptr::write_volatile(pkt.add(5), stack.ip_id as u8);
        // Flags=Don't Fragment
        core::ptr::write_volatile(pkt.add(6), 0x40);
        core::ptr::write_volatile(pkt.add(7), 0x00);
        core::ptr::write_volatile(pkt.add(8), 64); // TTL
        core::ptr::write_volatile(pkt.add(9), proto);
        // Checksum = 0 (compute later)
        for i in 0..4 { core::ptr::write_volatile(pkt.add(12 + i), stack.ip[i]); }
        for i in 0..4 { core::ptr::write_volatile(pkt.add(16 + i), dst_ip[i]); }
        // Compute header checksum
        let hdr = core::slice::from_raw_parts(pkt, 20);
        let cksum = checksum(hdr);
        core::ptr::write_volatile(pkt.add(10), (cksum >> 8) as u8);
        core::ptr::write_volatile(pkt.add(11), cksum as u8);
        // Copy payload — volatile read from source (may be volatile-written memory)
        for i in 0..payload.len() {
            let b = core::ptr::read_volatile(payload.as_ptr().add(i));
            core::ptr::write_volatile(pkt.add(20 + i), b);
        }
        // Pass raw pointer to eth_send via slice (eth_send does volatile reads)
        let slice = core::slice::from_raw_parts(pkt, total_len);
        eth_send(&dst_mac, &stack.mac, ETH_TYPE_IPV4, slice);
        // Note: heap_dealloc is a no-op with bump allocator
    }
}

/// Static buffer for IPv4 payload copy
static mut IPV4_BUF: [u8; 2048] = [0; 2048];

#[inline(never)]
fn ipv4_process(stack: *mut IpStack, data: &[u8]) {
    if data.len() < 20 { return; }
    // Volatile copy to avoid TCG hang
    let copy_len = if data.len() > 2048 { 2048 } else { data.len() };
    unsafe {
        let src = data.as_ptr();
        for i in 0..copy_len {
            IPV4_BUF[i] = core::ptr::read_volatile(src.add(i));
        }
    }
    let data = unsafe { &IPV4_BUF[..copy_len] };
    let version = data[0] >> 4;
    if version != 4 { return; }
    let ihl = (data[0] & 0xF) as usize * 4;
    if ihl < 20 || data.len() < ihl { return; }

    let total_len = ((data[2] as usize) << 8) | (data[3] as usize);
    if data.len() < total_len { return; }

    let proto = data[9];
    let src_ip = [data[12], data[13], data[14], data[15]];
    let dst_ip = [data[16], data[17], data[18], data[19]];
    let payload = &data[ihl..total_len];

    unsafe {
        let s = &mut *stack;
        // Only process packets destined for us (or broadcast)
        if !ip_eq(&dst_ip, &s.ip) && !ip_is_zero(&s.ip) { return; }

        match proto {
            IP_PROTO_ICMP => icmp_process(s, &src_ip, payload),
            IP_PROTO_TCP => tcp_process(s, &src_ip, payload),
            IP_PROTO_UDP => udp_process(s, &src_ip, &dst_ip, payload),
            _ => {}
        }
    }
}

// ─── ICMP ────────────────────────────────────────────────────────────────────

#[inline(never)]
fn icmp_send_echo_request(stack: &mut IpStack, dst_ip: &[u8; 4], ident: u16, seq: u16) {
    let msg = b"AginxOS ping!!!";
    let pkt_len = 8 + msg.len();
    let pkt = heap_alloc_zeroed_aligned(pkt_len, 8);
    if pkt.is_null() { return; }
    unsafe {
        core::ptr::write_volatile(pkt, ICMP_ECHO_REQUEST);
        core::ptr::write_volatile(pkt.add(4), (ident >> 8) as u8);
        core::ptr::write_volatile(pkt.add(5), ident as u8);
        core::ptr::write_volatile(pkt.add(6), (seq >> 8) as u8);
        core::ptr::write_volatile(pkt.add(7), seq as u8);
        for i in 0..msg.len() {
            core::ptr::write_volatile(pkt.add(8 + i), msg[i]);
        }
        let slice = core::slice::from_raw_parts(pkt, pkt_len);
        let cksum = checksum(slice);
        core::ptr::write_volatile(pkt.add(2), (cksum >> 8) as u8);
        core::ptr::write_volatile(pkt.add(3), cksum as u8);
        let slice = core::slice::from_raw_parts(pkt, pkt_len);
        ipv4_send(stack, dst_ip, IP_PROTO_ICMP, slice);
    }
}

#[inline(never)]
fn icmp_send_echo_reply(stack: &mut IpStack, dst_ip: &[u8; 4], orig: &[u8]) {
    let len = core::cmp::min(orig.len(), 120);
    if len == 0 { return; }
    let pkt = heap_alloc_zeroed_aligned(len, 8);
    if pkt.is_null() { return; }
    unsafe {
        for i in 0..len {
            core::ptr::write_volatile(pkt.add(i), orig[i]);
        }
        core::ptr::write_volatile(pkt, ICMP_ECHO_REPLY);
        core::ptr::write_volatile(pkt.add(2), 0);
        core::ptr::write_volatile(pkt.add(3), 0);
        let slice = core::slice::from_raw_parts(pkt, len);
        let cksum = checksum(slice);
        core::ptr::write_volatile(pkt.add(2), (cksum >> 8) as u8);
        core::ptr::write_volatile(pkt.add(3), cksum as u8);
        let slice = core::slice::from_raw_parts(pkt, len);
        ipv4_send(stack, dst_ip, IP_PROTO_ICMP, slice);
    }
}

#[inline(never)]
fn icmp_process(stack: &mut IpStack, src_ip: &[u8; 4], data: &[u8]) {
    if data.len() < 8 { return; }
    match data[0] {
        ICMP_ECHO_REQUEST => {
            icmp_send_echo_reply(stack, src_ip, data);
        }
        ICMP_ECHO_REPLY => {
            if data.len() >= 8 {
                let _ident = read_be16(data, 4);
                let seq = read_be16(data, 6);
                // Check if this matches our ping
                if seq <= stack.icmp_seq as u16 {
                    stack.icmp_recv_count += 1;
                }
            }
        }
        _ => {}
    }
}

// ─── DHCP Client (disabled — using static IP for QEMU SLIRP) ─────────────────
//
// DHCP is disabled because writing the IP checksum in dhcp_send triggers a
// QEMU ARM64 TCG codegen hang. Instead, we use static IP configuration
// (10.0.2.15/24, gateway 10.0.2.2) which is the default QEMU SLIRP assignment.

#[inline(never)]
fn dhcp_send(stack: &mut IpStack, msg_type: u8) {
    // Build complete DHCP frame on heap and transmit
    let frame_size: usize = 307; // 14+20+8+265
    let frame = heap_alloc_zeroed_aligned(350, 8);
    if frame.is_null() { return; }

    unsafe {
        let p = frame;

        // --- Ethernet header (14 bytes) ---
        core::ptr::write_volatile(p, 0xFF);
        core::ptr::write_volatile(p.add(1), 0xFF);
        core::ptr::write_volatile(p.add(2), 0xFF);
        core::ptr::write_volatile(p.add(3), 0xFF);
        core::ptr::write_volatile(p.add(4), 0xFF);
        core::ptr::write_volatile(p.add(5), 0xFF);
        core::ptr::write_volatile(p.add(6), stack.mac[0]);
        core::ptr::write_volatile(p.add(7), stack.mac[1]);
        core::ptr::write_volatile(p.add(8), stack.mac[2]);
        core::ptr::write_volatile(p.add(9), stack.mac[3]);
        core::ptr::write_volatile(p.add(10), stack.mac[4]);
        core::ptr::write_volatile(p.add(11), stack.mac[5]);
        core::ptr::write_volatile(p.add(12), 0x08);
        core::ptr::write_volatile(p.add(13), 0x00);

        // --- IP header (20 bytes, offset 14-33) ---
        // Static fields written individually (no loops)
        core::ptr::write_volatile(p.add(14), 0x45);
        core::ptr::write_volatile(p.add(15), 0x00);
        core::ptr::write_volatile(p.add(16), 0x01);
        core::ptr::write_volatile(p.add(17), 0x25);
        // ip_id at offset 18-19
        let ip_id = stack.ip_id.wrapping_add(1);
        stack.ip_id = ip_id;
        core::ptr::write_volatile(p.add(18), (ip_id >> 8) as u8);
        core::ptr::write_volatile(p.add(19), ip_id as u8);
        core::ptr::write_volatile(p.add(20), 0x40);
        core::ptr::write_volatile(p.add(21), 0x00);
        core::ptr::write_volatile(p.add(22), 0x40);
        core::ptr::write_volatile(p.add(23), 0x11);
        // IP checksum = 0 (DHCP disabled — static IP used instead)
        // src IP = 0.0.0.0 (already zero)
        // dst IP = broadcast
        core::ptr::write_volatile(p.add(30), 0xFF);
        core::ptr::write_volatile(p.add(31), 0xFF);
        core::ptr::write_volatile(p.add(32), 0xFF);
        core::ptr::write_volatile(p.add(33), 0xFF);

        // --- UDP header (8 bytes, offset 34-41) ---
        // src port = 68 (0x0044), dst port = 67 (0x0043)
        core::ptr::write_volatile(p.add(34), 0x00);
        core::ptr::write_volatile(p.add(35), 0x44);
        core::ptr::write_volatile(p.add(36), 0x00);
        core::ptr::write_volatile(p.add(37), 0x43);
        // UDP len = 273 (0x0111)
        core::ptr::write_volatile(p.add(38), 0x01);
        core::ptr::write_volatile(p.add(39), 0x11);

        // --- DHCP payload (offset 42+) ---
        let dp = p.add(42);
        core::ptr::write_volatile(dp, 1); // BOOTREQUEST
        core::ptr::write_volatile(dp.add(1), 1); // htype = Ethernet
        core::ptr::write_volatile(dp.add(2), 6); // hlen
        core::ptr::write_volatile(dp.add(4), (stack.dhcp_xid >> 24) as u8);
        core::ptr::write_volatile(dp.add(5), (stack.dhcp_xid >> 16) as u8);
        core::ptr::write_volatile(dp.add(6), (stack.dhcp_xid >> 8) as u8);
        core::ptr::write_volatile(dp.add(7), stack.dhcp_xid as u8);
        core::ptr::write_volatile(dp.add(10), 0x80); // broadcast flag
        core::ptr::write_volatile(dp.add(28), stack.mac[0]);
        core::ptr::write_volatile(dp.add(29), stack.mac[1]);
        core::ptr::write_volatile(dp.add(30), stack.mac[2]);
        core::ptr::write_volatile(dp.add(31), stack.mac[3]);
        core::ptr::write_volatile(dp.add(32), stack.mac[4]);
        core::ptr::write_volatile(dp.add(33), stack.mac[5]);
        // Magic cookie
        core::ptr::write_volatile(dp.add(236), 0x63);
        core::ptr::write_volatile(dp.add(237), 0x82);
        core::ptr::write_volatile(dp.add(238), 0x53);
        core::ptr::write_volatile(dp.add(239), 0x63);
        // Options
        core::ptr::write_volatile(dp.add(240), 53);
        core::ptr::write_volatile(dp.add(241), 1);
        core::ptr::write_volatile(dp.add(242), msg_type);
        core::ptr::write_volatile(dp.add(243), 61);
        core::ptr::write_volatile(dp.add(244), 7);
        core::ptr::write_volatile(dp.add(245), 1);
        core::ptr::write_volatile(dp.add(246), stack.mac[0]);
        core::ptr::write_volatile(dp.add(247), stack.mac[1]);
        core::ptr::write_volatile(dp.add(248), stack.mac[2]);
        core::ptr::write_volatile(dp.add(249), stack.mac[3]);
        core::ptr::write_volatile(dp.add(250), stack.mac[4]);
        core::ptr::write_volatile(dp.add(251), stack.mac[5]);
        if msg_type == DHCP_REQUEST {
            core::ptr::write_volatile(dp.add(252), 50);
            core::ptr::write_volatile(dp.add(253), 4);
            core::ptr::write_volatile(dp.add(254), stack.dhcp_requested_ip[0]);
            core::ptr::write_volatile(dp.add(255), stack.dhcp_requested_ip[1]);
            core::ptr::write_volatile(dp.add(256), stack.dhcp_requested_ip[2]);
            core::ptr::write_volatile(dp.add(257), stack.dhcp_requested_ip[3]);
            core::ptr::write_volatile(dp.add(258), 54);
            core::ptr::write_volatile(dp.add(259), 4);
            core::ptr::write_volatile(dp.add(260), stack.dhcp_server_ip[0]);
            core::ptr::write_volatile(dp.add(261), stack.dhcp_server_ip[1]);
            core::ptr::write_volatile(dp.add(262), stack.dhcp_server_ip[2]);
            core::ptr::write_volatile(dp.add(263), stack.dhcp_server_ip[3]);
            core::ptr::write_volatile(dp.add(264), 255);
        } else {
            core::ptr::write_volatile(dp.add(252), 255);
        }

        {
            let slice = core::slice::from_raw_parts(p, frame_size);
            crate::net::transmit(slice);
        }
    }
    heap_dealloc(frame, 350, 8);
}

#[inline(never)]
fn udp_send_raw(stack: &IpStack, src_ip: &[u8; 4], src_port: u16, dst_ip: &[u8; 4], dst_port: u16, payload: &[u8]) {
    let udp_len = 8 + payload.len();
    let total_ip = 20 + udp_len;
    let buf_size = total_ip.max(600);
    let buf_raw = heap_alloc_zeroed(buf_size);
    if buf_raw.is_null() { return; }
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_raw, buf_size) };

    // Build the entire packet using volatile writes to avoid QEMU ARM64 codegen hang
    let p = buf.as_mut_ptr();
    unsafe {
        // UDP header at offset 20
        core::ptr::write_volatile(p.add(20), (src_port >> 8) as u8);
        core::ptr::write_volatile(p.add(21), src_port as u8);
        core::ptr::write_volatile(p.add(22), (dst_port >> 8) as u8);
        core::ptr::write_volatile(p.add(23), dst_port as u8);
        core::ptr::write_volatile(p.add(24), (udp_len as u16 >> 8) as u8);
        core::ptr::write_volatile(p.add(25), udp_len as u16 as u8);
        // Copy payload
        for i in 0..payload.len() {
            core::ptr::write_volatile(p.add(28 + i), payload[i]);
        }
        // IP header at offset 0
        core::ptr::write_volatile(p, 0x45);
        core::ptr::write_volatile(p.add(2), (total_ip as u16 >> 8) as u8);
        core::ptr::write_volatile(p.add(3), total_ip as u16 as u8);
        let ip_id = stack.ip_id.wrapping_add(1);
        core::ptr::write_volatile(p.add(4), (ip_id >> 8) as u8);
        core::ptr::write_volatile(p.add(5), ip_id as u8);
        core::ptr::write_volatile(p.add(6), 0x40);
        core::ptr::write_volatile(p.add(7), 0x00);
        core::ptr::write_volatile(p.add(8), 64);
        core::ptr::write_volatile(p.add(9), IP_PROTO_UDP);
        for i in 0..4 { core::ptr::write_volatile(p.add(12 + i), src_ip[i]); }
        for i in 0..4 { core::ptr::write_volatile(p.add(16 + i), dst_ip[i]); }
    }
    let cksum = checksum(&buf[..20]);
    unsafe {
        core::ptr::write_volatile(p.add(10), (cksum >> 8) as u8);
        core::ptr::write_volatile(p.add(11), cksum as u8);
    }
    eth_send(&BROADCAST_MAC, &stack.mac, ETH_TYPE_IPV4, &buf[..total_ip]);
    heap_dealloc(buf_raw, buf_size, 8);
}

#[inline(never)]
fn dhcp_process(stack: &mut IpStack, data: &[u8]) {
    // data is the UDP payload (DHCP packet)
    if data.len() < 240 { return; }
    if data[0] != 2 { return; } // BOOTREPLY
    let xid = read_be32(data, 4);
    if xid != stack.dhcp_xid { return; }
    // Verify MAC
    if !mac_eq(&data[28..34].try_into().unwrap_or([0; 6]), &stack.mac) { return; }

    // Parse options
    let mut msg_type: u8 = 0;
    let mut offered_ip = [0u8; 4];
    let mut server_ip = [0u8; 4];
    let mut subnet_mask = [255, 255, 255, 0];
    let mut router_ip = [0u8; 4];
    let mut dns_ip = [0u8; 4];

    // Get offered IP from yiaddr
    offered_ip.copy_from_slice(&data[16..20]);
    // Get server IP from siaddr
    server_ip.copy_from_slice(&data[20..24]);

    // Parse DHCP options starting at offset 240
    let mut off = 240;
    while off + 1 < data.len() {
        let opt = data[off];
        if opt == 255 { break; } // End
        if opt == 0 { off += 1; continue; } // Padding
        if off + 1 >= data.len() { break; }
        let len = data[off + 1] as usize;
        if off + 2 + len > data.len() { break; }
        match opt {
            53 => { if len >= 1 { msg_type = data[off + 2]; } }
            1 => { if len >= 4 { subnet_mask.copy_from_slice(&data[off + 2..off + 6]); } }
            3 => { if len >= 4 { router_ip.copy_from_slice(&data[off + 2..off + 6]); } }
            6 => { if len >= 4 { dns_ip.copy_from_slice(&data[off + 2..off + 6]); } }
            54 => { if len >= 4 { server_ip.copy_from_slice(&data[off + 2..off + 6]); } }
            _ => {}
        }
        off += 2 + len;
    }

    match msg_type {
        DHCP_OFFER => {
            stack.dhcp_requested_ip = offered_ip;
            stack.dhcp_server_ip = server_ip;
            stack.dhcp_state = 2; // requesting
            dhcp_send(stack, DHCP_REQUEST);
        }
        DHCP_ACK => {
            stack.ip = offered_ip;
            stack.netmask = subnet_mask;
            stack.gateway = router_ip;
            stack.dns_server = dns_ip;
            stack.ip_acquired = true;
            stack.dhcp_state = 3; // bound
            // Add gateway to ARP cache proactively
            // (will be resolved on first use anyway)
        }
        _ => {}
    }
}

// ─── UDP Process (DHCP only) ────────────────────────────────────────────────

#[inline(never)]
fn udp_process(stack: &mut IpStack, _src_ip: &[u8; 4], _dst_ip: &[u8; 4], data: &[u8]) {
    if data.len() < 8 { return; }
    let dst_port = read_be16(data, 2);
    let udp_len = read_be16(data, 4) as usize;
    if data.len() < udp_len { return; }
    let payload = &data[8..udp_len];

    if dst_port == 68 {
        // DHCP response
        dhcp_process(stack, payload);
    }
}

// ─── TCP ─────────────────────────────────────────────────────────────────────

fn tcp_find_listen(stack: &IpStack, port: u16) -> Option<usize> {
    for i in 0..TCP_SLOT_COUNT {
        let c = &stack.tcp_conns[i];
        if c.used && c.kind == 1 && c.local_port == port && c.state == TcpConnState::Listen {
            return Some(i);
        }
    }
    None
}

fn tcp_find_established(stack: &IpStack, local_port: u16, remote_port: u16, remote_ip: &[u8; 4]) -> Option<usize> {
    for i in 0..TCP_SLOT_COUNT {
        let c = &stack.tcp_conns[i];
        if c.used && c.local_port == local_port && c.remote_port == remote_port && ip_eq(&c.remote_ip, remote_ip) {
            return Some(i);
        }
    }
    None
}

#[inline(never)]
fn tcp_send_packet(src_ip: &[u8; 4], conn: &TcpConn, flags: u16, data: &[u8]) {
    let tcp_hdr_len = 20;
    let total = tcp_hdr_len + data.len();
    if total > 1500 { return; }
    let buf_size = if total < 600 { 600 } else { total };
    let pkt = heap_alloc_zeroed_aligned(buf_size, 8);
    if pkt.is_null() { return; }
    unsafe {
        core::ptr::write_volatile(pkt, (conn.local_port >> 8) as u8);
        core::ptr::write_volatile(pkt.add(1), conn.local_port as u8);
        core::ptr::write_volatile(pkt.add(2), (conn.remote_port >> 8) as u8);
        core::ptr::write_volatile(pkt.add(3), conn.remote_port as u8);
        core::ptr::write_volatile(pkt.add(4), (conn.snd_nxt >> 24) as u8);
        core::ptr::write_volatile(pkt.add(5), (conn.snd_nxt >> 16) as u8);
        core::ptr::write_volatile(pkt.add(6), (conn.snd_nxt >> 8) as u8);
        core::ptr::write_volatile(pkt.add(7), conn.snd_nxt as u8);
        core::ptr::write_volatile(pkt.add(8), (conn.rcv_nxt >> 24) as u8);
        core::ptr::write_volatile(pkt.add(9), (conn.rcv_nxt >> 16) as u8);
        core::ptr::write_volatile(pkt.add(10), (conn.rcv_nxt >> 8) as u8);
        core::ptr::write_volatile(pkt.add(11), conn.rcv_nxt as u8);
        core::ptr::write_volatile(pkt.add(12), 5 << 4);
        core::ptr::write_volatile(pkt.add(13), flags as u8);
        core::ptr::write_volatile(pkt.add(14), (conn.rcv_wnd >> 8) as u8);
        core::ptr::write_volatile(pkt.add(15), conn.rcv_wnd as u8);
        for i in 0..data.len() {
            core::ptr::write_volatile(pkt.add(20 + i), data[i]);
        }
        let slice = core::slice::from_raw_parts(pkt, total);
        let cksum = checksum_with_pseudo(src_ip, &conn.remote_ip, IP_PROTO_TCP, slice);
        core::ptr::write_volatile(pkt.add(16), (cksum >> 8) as u8);
        core::ptr::write_volatile(pkt.add(17), cksum as u8);
        let slice = core::slice::from_raw_parts(pkt, total);
        ipv4_send(unsafe { &mut *IP_STACK }, &conn.remote_ip, IP_PROTO_TCP, slice);
    }
}

#[inline(never)]
fn tcp_process(stack: &mut IpStack, src_ip: &[u8; 4], data: &[u8]) {
    if data.len() < 20 { return; }
    let src_port = read_be16(data, 0);
    let dst_port = read_be16(data, 2);
    let seq = read_be32(data, 4);
    let ack = read_be32(data, 8);
    let flags = data[13] as u16;
    let _window = read_be16(data, 14);
    let tcp_hdr_len = ((data[12] >> 4) as usize) * 4;
    if tcp_hdr_len < 20 || data.len() < tcp_hdr_len { return; }
    let payload = &data[tcp_hdr_len..];
    let has_syn = (flags & TCP_SYN) != 0;
    let has_ack = (flags & TCP_ACK) != 0;
    let has_fin = (flags & TCP_FIN) != 0;
    let has_rst = (flags & TCP_RST) != 0;

    // Capture our IP before borrowing connection
    let my_ip = stack.ip;

    // Find matching connection
    let conn_idx = if let Some(idx) = tcp_find_established(stack, dst_port, src_port, src_ip) {
        Some(idx)
    } else if has_syn {
        tcp_find_listen(stack, dst_port)
    } else {
        None
    };

    let conn_idx = match conn_idx {
        Some(i) => i,
        None => {
            // RST for unknown connections
            if has_ack {
                let rst_conn = TcpConn {
                    state: TcpConnState::Closed, local_port: dst_port, remote_port: src_port,
                    remote_ip: *src_ip, snd_una: 0, snd_nxt: ack, snd_wnd: 0,
                    rcv_nxt: seq, rcv_wnd: TCP_BUF_SIZE as u16, iss: 0,
                    rx_buf: [0; TCP_BUF_SIZE], rx_len: 0, rx_read: 0,
                    tx_buf: [0; TCP_BUF_SIZE], tx_len: 0,
                    retransmit_at: 0, retransmit_count: 0, used: true, kind: 0,
                };
                tcp_send_packet(&my_ip, &rst_conn, TCP_RST, &[]);
            }
            return;
        }
    };

    let conn = &mut stack.tcp_conns[conn_idx];

    if has_rst {
        conn.state = TcpConnState::Closed;
        conn.used = false;
        conn.kind = 0;
        return;
    }

    match conn.state {
            TcpConnState::Listen => {
                if has_syn {
                    // Incoming connection: SYN_RECEIVED
                    conn.remote_ip = *src_ip;
                    conn.remote_port = src_port;
                    conn.rcv_nxt = seq.wrapping_add(1);
                    conn.iss = get_timestamp_ms() as u32;
                    conn.snd_nxt = conn.iss;
                    conn.snd_una = conn.iss;
                    conn.rcv_wnd = TCP_BUF_SIZE as u16;
                    conn.snd_wnd = _window;
                    conn.state = TcpConnState::SynReceived;
                    // Send SYN+ACK
                    uart::puts(UART, "[TCP] SYN+ACK\r\n");
                    tcp_send_packet(&my_ip, conn, TCP_SYN | TCP_ACK, &[]);
                    conn.snd_nxt = conn.snd_nxt.wrapping_add(1);
                }
            }
            TcpConnState::SynSent => {
                if has_syn && has_ack {
                    // SYN+ACK received: ESTABLISHED
                    conn.rcv_nxt = seq.wrapping_add(1);
                    conn.snd_una = ack;
                    conn.state = TcpConnState::Established;
                    conn.snd_wnd = _window;
                    // Send ACK
                    tcp_send_packet(&my_ip, conn, TCP_ACK, &[]);
                }
            }
            TcpConnState::SynReceived => {
                if has_ack {
                    conn.snd_una = ack;
                    conn.state = TcpConnState::Established;
                    conn.snd_wnd = _window;
                    uart::puts(UART, "[TCP] ESTABLISHED\r\n");
                }
            }
            TcpConnState::Established => {
                if has_fin {
                    conn.rcv_nxt = conn.rcv_nxt.wrapping_add(1);
                    // ACK the FIN
                    tcp_send_packet(&my_ip, conn, TCP_ACK, &[]);
                    conn.state = TcpConnState::CloseWait;
                } else if has_ack {
                    conn.snd_una = ack;
                    conn.snd_wnd = _window;
                    // Handle incoming data
                    if !payload.is_empty() && seq == conn.rcv_nxt {
                        let space = TCP_BUF_SIZE - conn.rx_len;
                        let copy_len = core::cmp::min(payload.len(), space);
                        if copy_len > 0 {
                            // Append to circular buffer using volatile writes
                            let write_start = (conn.rx_read + conn.rx_len) % TCP_BUF_SIZE;
                            for i in 0..copy_len {
                                let b = unsafe { core::ptr::read_volatile(payload.as_ptr().add(i)) };
                                unsafe { core::ptr::write_volatile(conn.rx_buf.as_mut_ptr().add((write_start + i) % TCP_BUF_SIZE), b); }
                            }
                            conn.rx_len += copy_len;
                            conn.rcv_nxt = conn.rcv_nxt.wrapping_add(copy_len as u32);
                            conn.rcv_wnd = (TCP_BUF_SIZE - conn.rx_len) as u16;
                            // ACK received data
                            tcp_send_packet(&my_ip, conn, TCP_ACK, &[]);
                        }
                    }
                    // Send pending data if window opened
                    if conn.tx_len > 0 && conn.snd_nxt < conn.snd_una.wrapping_add(conn.snd_wnd as u32) {
                        tcp_flush_tx(stack, conn_idx);
                    }
                }
            }
            TcpConnState::CloseWait => {
                // Application needs to close
            }
            TcpConnState::LastAck => {
                if has_ack {
                    conn.state = TcpConnState::Closed;
                    conn.used = false;
                    conn.kind = 0;
                }
            }
            TcpConnState::FinWait1 => {
                if has_fin && has_ack {
                    conn.rcv_nxt = conn.rcv_nxt.wrapping_add(1);
                    tcp_send_packet(&my_ip, conn, TCP_ACK, &[]);
                    conn.state = TcpConnState::TimeWait;
                    conn.retransmit_at = get_timestamp_ms() + 2000; // 2s TIME_WAIT
                } else if has_fin {
                    conn.rcv_nxt = conn.rcv_nxt.wrapping_add(1);
                    tcp_send_packet(&my_ip, conn, TCP_ACK, &[]);
                    conn.state = TcpConnState::Closing;
                } else if has_ack {
                    conn.snd_una = ack;
                    conn.state = TcpConnState::FinWait2;
                }
            }
            TcpConnState::FinWait2 => {
                if has_fin {
                    conn.rcv_nxt = conn.rcv_nxt.wrapping_add(1);
                    tcp_send_packet(&my_ip, conn, TCP_ACK, &[]);
                    conn.state = TcpConnState::TimeWait;
                    conn.retransmit_at = get_timestamp_ms() + 2000;
                }
            }
            TcpConnState::Closing => {
                if has_ack {
                    conn.state = TcpConnState::TimeWait;
                    conn.retransmit_at = get_timestamp_ms() + 2000;
                }
            }
            TcpConnState::TimeWait => {
                // Timeout will clean up
            }
            _ => {}
        }
}

#[inline(never)]
fn tcp_flush_tx(stack: &mut IpStack, idx: usize) {
    let my_ip = stack.ip;

    // Read tx_len with volatile to avoid TCG hang on struct field access
    let conn_ptr = unsafe { stack.tcp_conns.as_mut_ptr().add(idx) };
    let tx_len = unsafe { core::ptr::read_volatile(core::ptr::addr_of_mut!((*conn_ptr).tx_len)) };
    if tx_len == 0 { return; }

    let send_len = if tx_len < MSS { tx_len } else { MSS };

    // Allocate send buffer on heap instead of stack to avoid TCG hang
    let send_buf = heap_alloc_zeroed_aligned(send_len, 8);
    if send_buf.is_null() { return; }

    unsafe {
        // Copy from tx_buf to send_buf using volatile
        let src = (*conn_ptr).tx_buf.as_ptr();
        for i in 0..send_len {
            core::ptr::write_volatile(send_buf.add(i), core::ptr::read_volatile(src.add(i)));
        }
    }

    // Send the packet
    {
        let conn = unsafe { &stack.tcp_conns[idx] };
        let send_slice = unsafe { core::slice::from_raw_parts(send_buf, send_len) };
        tcp_send_packet(&my_ip, conn, TCP_ACK | TCP_PSH, send_slice);
    }

    heap_dealloc(send_buf, send_len, 8);

    // Update connection state using volatile writes
    let conn = unsafe { &mut stack.tcp_conns[idx] };
    let snd_nxt = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(conn.snd_nxt)) }.wrapping_add(send_len as u32);
    unsafe { core::ptr::write_volatile(core::ptr::addr_of_mut!(conn.snd_nxt), snd_nxt); }

    let remaining = tx_len - send_len;
    if remaining > 0 {
        unsafe {
            let src = conn.tx_buf.as_ptr().add(send_len);
            let dst = conn.tx_buf.as_mut_ptr();
            for i in 0..remaining {
                core::ptr::write_volatile(dst.add(i), core::ptr::read_volatile(src.add(i)));
            }
        }
    }
    unsafe { core::ptr::write_volatile(core::ptr::addr_of_mut!(conn.tx_len), remaining); }
    unsafe { core::ptr::write_volatile(core::ptr::addr_of_mut!(conn.retransmit_at), get_timestamp_ms() + 200); }
    unsafe { core::ptr::write_volatile(core::ptr::addr_of_mut!(conn.retransmit_count), 0); }
}

// ─── Packet Receive Loop ────────────────────────────────────────────────────

/// Receive and process one packet from the network driver
fn net_receive_loop(stack: *mut IpStack) {
    loop {
        let result = crate::net::receive();
        match result {
            Some((buf, len, desc_idx)) => {
                if len < 14 { crate::net::release_rx_buffer(buf, desc_idx); continue; }
                let data = unsafe { core::slice::from_raw_parts(buf, len) };
                let ethertype = read_be16(data, 12);
                let payload = &data[14..];
                match ethertype {
                    0x0806 => arp_process(unsafe { &mut *stack }, payload),
                    0x0800 => ipv4_process(stack, payload),
                    _ => {}
                }
                crate::net::release_rx_buffer(buf, desc_idx);
            }
            None => break,
        }
    }
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Initialize the TCP/IP stack
pub fn init() -> bool {
    unsafe {
        if !IP_STACK.is_null() { return true; }

        let mac = match crate::net::get_mac() {
            Some(m) => m,
            None => {
                uart::puts(UART, "[FAIL] TCP: no device\r\n");
                return false;
            }
        };

        // Allocate IpStack on heap
        let layout = alloc::alloc::Layout::from_size_align(core::mem::size_of::<IpStack>(), 8).unwrap();
        let ptr = alloc::alloc::alloc_zeroed(layout) as *mut IpStack;
        if ptr.is_null() {
            uart::puts(UART, "[FAIL] TCP: alloc\r\n");
            return false;
        }

        // Initialize using volatile writes
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*ptr).mac), mac);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*ptr).ip), [10, 0, 2, 15]);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*ptr).gateway), [10, 0, 2, 2]);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*ptr).netmask), [255, 255, 255, 0]);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*ptr).dns_server), [10, 0, 2, 3]);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*ptr).ip_acquired), true);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*ptr).dhcp_xid), 0x12345678);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*ptr).dhcp_state), 3); // bound
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*ptr).ip_id), 0x4000u16);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*ptr).icmp_ident), 0x1234u16);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*ptr).icmp_seq), 0);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*ptr).icmp_recv_count), 0);

        // Initialize ARP cache entries as invalid
        for i in 0..ARP_CACHE_SIZE {
            core::ptr::write_volatile(core::ptr::addr_of_mut!((*ptr).arp_cache[i].valid), false);
        }

        // Pre-populate ARP cache with QEMU SLIRP gateway MAC (10.0.2.2)
        // QEMU SLIRP uses 52:55:0a:00:02:02 for the gateway
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*ptr).arp_cache[0].valid), true);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*ptr).arp_cache[0].ip), [10, 0, 2, 2]);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*ptr).arp_cache[0].mac), [0x52, 0x55, 0x0a, 0x00, 0x02, 0x02]);

        // Initialize TCP connections as unused
        for i in 0..TCP_SLOT_COUNT {
            core::ptr::write_volatile(core::ptr::addr_of_mut!((*ptr).tcp_conns[i].used), false);
            core::ptr::write_volatile(core::ptr::addr_of_mut!((*ptr).tcp_conns[i].kind), 0);
            core::ptr::write_volatile(core::ptr::addr_of_mut!((*ptr).tcp_conns[i].state), TcpConnState::Closed);
        }

        IP_STACK = ptr;

        uart::puts(UART, "[OK] TCP/IP stack initialized\r\n");
        uart::puts(UART, "  IP: 10.0.2.15/24 GW: 10.0.2.2\r\n");

        true
    }
}

/// Static buffer for copying received packets (avoids TCG hang from reading DMA memory directly)
static mut RECV_BUF: [u8; 2048] = [0; 2048];

/// Copy packet data from DMA buffer using volatile reads to avoid TCG codegen hang
#[inline(never)]
unsafe fn copy_packet_volatile(src: *mut u8, dst: &mut [u8], len: usize) {
    let copy_len = if len > dst.len() { dst.len() } else { len };
    for i in 0..copy_len {
        dst[i] = core::ptr::read_volatile(src.add(i));
    }
}

/// Process a received packet from the static buffer (separated to avoid TCG codegen hang)
#[inline(never)]
fn process_packet(len: usize) {
    unsafe {
        if len < 14 { return; }
        // Read ethertype using volatile reads from RECV_BUF
        let b12 = core::ptr::read_volatile(RECV_BUF.as_ptr().add(12));
        let b13 = core::ptr::read_volatile(RECV_BUF.as_ptr().add(13));
        let ethertype = ((b12 as u16) << 8) | (b13 as u16);
        // Create slice for payload (after copy, data is in regular memory)
        let data = &RECV_BUF[..len];
        let payload = &data[14..];
        match ethertype {
            0x0806 => arp_process(&mut *IP_STACK, payload),
            0x0800 => ipv4_process(IP_STACK, payload),
            _ => {}
        }
    }
}

/// Poll the network stack — process one pending packet and drive timers
#[inline(never)]
pub fn poll() -> usize {
    unsafe {
        if IP_STACK.is_null() { return 0; }
        let stack = &mut *IP_STACK;

        // Process one incoming packet
        let result = crate::net::receive();
        match result {
            Some((buf, len, desc_idx)) => {
                if len >= 14 {
                    // Copy packet data from DMA buffer using volatile reads
                    // to avoid QEMU ARM64 TCG codegen hang
                    copy_packet_volatile(buf, &mut RECV_BUF, len);
                    process_packet(len);
                }
                crate::net::release_rx_buffer(buf, desc_idx);
                return 1;
            }
            None => {}
        }

        // Drive TCP retransmit timers
        let now = get_timestamp_ms();
        for i in 0..TCP_SLOT_COUNT {
            let conn = &mut stack.tcp_conns[i];
            if !conn.used { continue; }

            // TIME_WAIT timeout
            if conn.state == TcpConnState::TimeWait && now >= conn.retransmit_at {
                conn.state = TcpConnState::Closed;
                conn.used = false;
                conn.kind = 0;
                continue;
            }

            // Retransmit check
            if conn.tx_len == 0 && conn.retransmit_count < 5 && now >= conn.retransmit_at && conn.retransmit_at > 0 {
                if conn.state == TcpConnState::Established || conn.state == TcpConnState::SynSent ||
                   conn.state == TcpConnState::SynReceived || conn.state == TcpConnState::FinWait1 ||
                   conn.state == TcpConnState::LastAck {
                    // Don't retransmit data that was already acked — just reset timer
                    conn.retransmit_at = now + 200;
                    conn.retransmit_count += 1;
                }
            }
        }

        0
    }
}

/// Wait for DHCP to obtain an IP address
pub fn wait_for_dhcp(max_iters: u32) -> bool {
    // Check immediately — static IP config has ip_acquired=true already
    unsafe {
        if !IP_STACK.is_null() && (*IP_STACK).ip_acquired {
            return true;
        }
    }
    for i in 0..max_iters {
        poll();
        crate::net::flush_rx();
        unsafe {
            if !IP_STACK.is_null() && (*IP_STACK).ip_acquired {
                return true;
            }
        }
        if i > 0 && i % 500 == 0 {
            uart::putc(UART, b'.');
        }
        for _ in 0..50_000 { spin_loop(); }
    }
    false
}

/// Get current IP address
pub fn get_ip() -> Option<[u8; 4]> {
    unsafe {
        if IP_STACK.is_null() { return None; }
        let s = &*IP_STACK;
        if s.ip_acquired { Some(s.ip) } else { None }
    }
}

/// Get gateway address
pub fn get_gateway() -> Option<[u8; 4]> {
    unsafe {
        if IP_STACK.is_null() { return None; }
        let s = &*IP_STACK;
        if s.ip_acquired && !ip_is_zero(&s.gateway) { Some(s.gateway) } else { None }
    }
}

/// Get DNS server
pub fn get_dns_server() -> Option<[u8; 4]> {
    unsafe {
        if IP_STACK.is_null() { return None; }
        let s = &*IP_STACK;
        if !ip_is_zero(&s.dns_server) { Some(s.dns_server) } else { None }
    }
}

/// Get MAC address
pub fn get_mac() -> Option<[u8; 6]> {
    unsafe {
        if IP_STACK.is_null() { return None; }
        Some((*IP_STACK).mac)
    }
}

/// Print network status
pub fn print_status() {
    unsafe {
        if IP_STACK.is_null() {
            uart::puts(UART, "Network: not initialized\r\n");
            return;
        }
        let s = &*IP_STACK;
        uart::puts(UART, "Network Status:\r\n");
        uart::puts(UART, "  MAC: ");
        for (i, b) in s.mac.iter().enumerate() {
            if i > 0 { uart::putc(UART, b':'); }
            crate::print_hex_byte(UART, *b);
        }
        uart::puts(UART, "\r\n");
        if s.ip_acquired {
            uart::puts(UART, "  IP: ");
            crate::print_ip(UART, s.ip);
            uart::puts(UART, "\r\n  GW: ");
            crate::print_ip(UART, s.gateway);
            uart::puts(UART, "\r\n");
        } else {
            uart::puts(UART, "  IP: (waiting for DHCP)\r\n");
        }
    }
}

// ─── TCP Slot API ────────────────────────────────────────────────────────────

/// Allocate a free TCP slot
pub fn tcp_slot_alloc() -> Option<usize> {
    unsafe {
        if IP_STACK.is_null() { return None; }
        let s = &mut *IP_STACK;
        for i in 0..TCP_SLOT_COUNT {
            if !s.tcp_conns[i].used {
                return Some(i);
            }
        }
        None
    }
}

/// Set up a TCP listen slot — all volatile writes to keep codegen simple
unsafe fn tcp_slot_listen_write(slot: usize, port: u16) -> bool {
    let s = &mut *IP_STACK;
    if slot >= TCP_SLOT_COUNT { return false; }
    let conn = &mut s.tcp_conns[slot];

    // Zero the connection struct byte by byte (volatile)
    let ptr = conn as *mut TcpConn as *mut u8;
    for i in 0..core::mem::size_of::<TcpConn>() {
        core::ptr::write_volatile(ptr.add(i), 0);
    }

    // Set fields via volatile writes
    core::ptr::write_volatile(core::ptr::addr_of_mut!(conn.used), true);
    core::ptr::write_volatile(core::ptr::addr_of_mut!(conn.kind), 1u8);
    core::ptr::write_volatile(core::ptr::addr_of_mut!(conn.state), TcpConnState::Listen);
    core::ptr::write_volatile(core::ptr::addr_of_mut!(conn.local_port), port);
    let ts = get_timestamp_ms() as u32;
    core::ptr::write_volatile(core::ptr::addr_of_mut!(conn.snd_nxt), ts);
    core::ptr::write_volatile(core::ptr::addr_of_mut!(conn.snd_una), ts);
    core::ptr::write_volatile(core::ptr::addr_of_mut!(conn.rcv_wnd), TCP_BUF_SIZE as u16);

    true
}

/// Public wrapper for TCP listen
pub fn tcp_slot_listen(slot: usize, port: u16) -> bool {
    unsafe {
        if IP_STACK.is_null() { return false; }
        tcp_slot_listen_write(slot, port)
    }
}

/// Initiate TCP connection
pub fn tcp_slot_connect(slot: usize, ip: [u8; 4], port: u16) -> bool {
    unsafe {
        if IP_STACK.is_null() { return false; }
        let s = &mut *IP_STACK;
        if slot >= TCP_SLOT_COUNT { return false; }
        if !s.ip_acquired { return false; }
        let my_ip = s.ip;
        let conn = &mut s.tcp_conns[slot];
        conn.used = true;
        conn.kind = 2; // connect
        conn.state = TcpConnState::SynSent;
        conn.local_port = 49152 + (crate::interrupt::get_ticks() as u16) % 16384;
        conn.remote_port = port;
        conn.remote_ip = ip;
        conn.rx_len = 0;
        conn.rx_read = 0;
        conn.tx_len = 0;
        conn.iss = get_timestamp_ms() as u32;
        conn.snd_nxt = conn.iss;
        conn.snd_una = conn.iss;
        conn.rcv_nxt = 0;
        conn.rcv_wnd = TCP_BUF_SIZE as u16;
        conn.snd_wnd = MSS as u16;
        conn.retransmit_at = get_timestamp_ms() + 200;
        conn.retransmit_count = 0;
        // Send SYN
        tcp_send_packet(&my_ip, conn, TCP_SYN, &[]);
        conn.snd_nxt = conn.snd_nxt.wrapping_add(1);
        uart::puts(UART, "[..] TCP connecting to ");
        crate::print_ip(UART, ip);
        uart::puts(UART, ":");
        crate::print_dec_u32(UART, port as u32);
        uart::puts(UART, "\r\n");
        true
    }
}

/// Check TCP slot state
pub fn tcp_slot_state(slot: usize) -> TcpConnState {
    unsafe {
        if IP_STACK.is_null() { return TcpConnState::Closed; }
        if slot >= TCP_SLOT_COUNT { return TcpConnState::Closed; }
        (*IP_STACK).tcp_conns[slot].state
    }
}

/// Send data on TCP slot (buffer only, no flush)
pub fn tcp_slot_send(slot: usize, data: &[u8]) -> bool {
    unsafe {
        if IP_STACK.is_null() { return false; }
        if slot >= TCP_SLOT_COUNT { return false; }
        let s = &mut *IP_STACK;
        let conn = &mut s.tcp_conns[slot];
        if conn.state != TcpConnState::Established && conn.state != TcpConnState::CloseWait { return false; }
        // Buffer data using volatile writes to avoid TCG codegen hang
        let space = TCP_BUF_SIZE - conn.tx_len;
        if data.len() > space { return false; }
        let dst = conn.tx_buf.as_mut_ptr().add(conn.tx_len);
        let src = data.as_ptr();
        for i in 0..data.len() {
            core::ptr::write_volatile(dst.add(i), core::ptr::read_volatile(src.add(i)));
        }
        conn.tx_len += data.len();
        true // Buffer data only, caller must flush
    }
}

/// Flush pending TX data on a TCP slot
#[inline(never)]
pub fn tcp_slot_flush(slot: usize) -> bool {
    unsafe {
        if IP_STACK.is_null() { return false; }
        if slot >= TCP_SLOT_COUNT { return false; }
        let s = &mut *IP_STACK;
        tcp_flush_tx(s, slot);
        true
    }
}

/// Receive data from TCP slot
#[inline(never)]
pub fn tcp_slot_recv(slot: usize, buf: &mut [u8]) -> usize {
    unsafe {
        if IP_STACK.is_null() { return 0; }
        if slot >= TCP_SLOT_COUNT { return 0; }
        let conn = &mut (*IP_STACK).tcp_conns[slot];
        if conn.rx_len == 0 { return 0; }
        let copy_len = core::cmp::min(buf.len(), conn.rx_len);
        // Volatile copy to avoid TCG codegen hang
        for i in 0..copy_len {
            let idx = (conn.rx_read + i) % TCP_BUF_SIZE;
            buf[i] = core::ptr::read_volatile(conn.rx_buf.as_ptr().add(idx));
        }
        conn.rx_read = (conn.rx_read + copy_len) % TCP_BUF_SIZE;
        conn.rx_len -= copy_len;
        conn.rcv_wnd = (TCP_BUF_SIZE - conn.rx_len) as u16;
        copy_len
    }
}

/// Close TCP slot
pub fn tcp_slot_close(slot: usize) {
    unsafe {
        if IP_STACK.is_null() { return; }
        if slot >= TCP_SLOT_COUNT { return; }
        let s = &mut *IP_STACK;
        let my_ip = s.ip;
        let conn = &mut s.tcp_conns[slot];
        match conn.state {
            TcpConnState::Established => {
                conn.state = TcpConnState::FinWait1;
                tcp_send_packet(&my_ip, conn, TCP_FIN | TCP_ACK, &[]);
                conn.snd_nxt = conn.snd_nxt.wrapping_add(1);
            }
            TcpConnState::CloseWait => {
                conn.state = TcpConnState::LastAck;
                tcp_send_packet(&my_ip, conn, TCP_FIN | TCP_ACK, &[]);
                conn.snd_nxt = conn.snd_nxt.wrapping_add(1);
            }
            _ => {
                conn.state = TcpConnState::Closed;
                conn.used = false;
                conn.kind = 0;
            }
        }
    }
}

/// Blocking send on TCP slot
pub fn tcp_slot_send_blocking(slot: usize, data: &[u8]) {
    let mut sent = 0;
    let mut tries = 0;
    while sent < data.len() {
        poll();
        let chunk = &data[sent..];
        if tcp_slot_send(slot, chunk) {
            tcp_slot_flush(slot);
            sent += chunk.len();
            tries = 0;
        } else {
            tries += 1;
            if tries > 100 { return; }
            for _ in 0..50_000 { spin_loop(); }
        }
    }
}

/// Check for incoming TCP data on slot 0 and print to UART
pub fn check_tcp_data() {
    let mut buf = [0u8; 256];
    let n = tcp_slot_recv(0, &mut buf);
    if n > 0 {
        uart::puts(UART, "TCP recv: ");
        for &b in &buf[..n] {
            if b >= 0x20 && b < 0x7F { uart::putc(UART, b); }
            else { uart::putc(UART, b'.'); }
        }
        uart::puts(UART, "\r\n");
    }
}

// ─── ICMP Ping ──────────────────────────────────────────────────────────────

/// Send ICMP ping
pub fn ping(ip: [u8; 4], count: u8) -> u8 {
    unsafe {
        if IP_STACK.is_null() { return 0; }
        let stack = &mut *IP_STACK;
        if !stack.ip_acquired {
            uart::puts(UART, "[FAIL] No IP address\r\n");
            return 0;
        }

        let ident = stack.icmp_ident;
        let mut received: u8 = 0;

        for seq in 0..count {
            stack.icmp_recv_count = 0;
            stack.icmp_seq = seq as u16;
            icmp_send_echo_request(stack, &ip, ident, seq as u16);

            // Wait for reply
            let mut got_reply = false;
            for i in 0..2000u32 {
                poll();
                crate::net::flush_rx();
                if stack.icmp_recv_count > 0 {
                    uart::puts(UART, "  Reply from ");
                    crate::print_ip(UART, ip);
                    uart::puts(UART, " seq=");
                    crate::print_dec_u32(UART, seq as u32);
                    uart::puts(UART, " OK\r\n");
                    received += 1;
                    got_reply = true;
                    break;
                }
                if seq == 0 && i > 0 && i % 500 == 0 { uart::putc(UART, b'.'); }
                for _ in 0..100_000 { spin_loop(); }
            }
            if !got_reply {
                uart::puts(UART, "  seq=");
                crate::print_dec_u32(UART, seq as u32);
                uart::puts(UART, " timeout\r\n");
            }
        }
        received
    }
}
