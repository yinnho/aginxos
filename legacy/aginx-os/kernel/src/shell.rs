//! Shell module: output abstraction, command dispatch, line editor

use core::hint::spin_loop;

// Conditional UART imports
#[cfg(not(feature = "board-redfin"))]
use crate::uart;
#[cfg(feature = "board-redfin")]
use crate::qup_uart as uart;

#[cfg(not(feature = "board-redfin"))]
use alloc::vec::Vec;
use alloc::alloc::{alloc_zeroed, alloc, Layout};

// UART base address
use crate::platform::UART as U;

// ─── Output Abstraction ─────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum OutputDest {
    Uart,
    Tcp,
    Capture,
}

static mut OUTPUT: OutputDest = OutputDest::Uart;

pub fn set_output(dest: OutputDest) {
    unsafe { OUTPUT = dest; }
}

#[allow(dead_code)]
pub fn output() -> OutputDest {
    unsafe { OUTPUT }
}

pub fn puts(s: &str) {
    unsafe {
        match OUTPUT {
            OutputDest::Uart => uart::puts(U, s),
            OutputDest::Tcp => { crate::tcp::tcp_slot_send_blocking(OUTPUT_TCP_SLOT, s.as_bytes()); }
            OutputDest::Capture => {
                for &b in s.as_bytes() {
                    if CAPTURE_LEN < CAPTURE_BUF.len() {
                        CAPTURE_BUF[CAPTURE_LEN] = b;
                        CAPTURE_LEN += 1;
                    }
                }
            }
        }
    }
}

pub fn putc(c: u8) {
    unsafe {
        match OUTPUT {
            OutputDest::Uart => uart::putc(U, c),
            OutputDest::Tcp => { crate::tcp::tcp_slot_send_blocking(OUTPUT_TCP_SLOT, &[c]); }
            OutputDest::Capture => {
                if CAPTURE_LEN < CAPTURE_BUF.len() {
                    CAPTURE_BUF[CAPTURE_LEN] = c;
                    CAPTURE_LEN += 1;
                }
            }
        }
    }
}

/// Write to a specific output destination (for line editor)
pub fn putc_to(dest: OutputDest, c: u8) {
    match dest {
        OutputDest::Uart => uart::putc(U, c),
        OutputDest::Tcp => unsafe { crate::tcp::tcp_slot_send_blocking(OUTPUT_TCP_SLOT, &[c]); }
        OutputDest::Capture => unsafe {
            if CAPTURE_LEN < CAPTURE_BUF.len() {
                CAPTURE_BUF[CAPTURE_LEN] = c;
                CAPTURE_LEN += 1;
            }
        }
    }
}

/// Write raw bytes to a specific output destination
pub fn write_to(dest: OutputDest, s: &[u8]) {
    match dest {
        OutputDest::Uart => uart::write_bytes(U, s),
        OutputDest::Tcp => unsafe { crate::tcp::tcp_slot_send_blocking(OUTPUT_TCP_SLOT, s); }
        OutputDest::Capture => unsafe {
            for &b in s {
                if CAPTURE_LEN < CAPTURE_BUF.len() {
                    CAPTURE_BUF[CAPTURE_LEN] = b;
                    CAPTURE_LEN += 1;
                }
            }
        }
    }
}

// ─── Output Capture ─────────────────────────────────────────────────────────

static mut CAPTURE_BUF: [u8; 4096] = [0; 4096];
static mut CAPTURE_LEN: usize = 0;
static mut CAPTURE_PREV: OutputDest = OutputDest::Uart;

/// Start capturing output to the internal buffer
pub fn capture_start() {
    unsafe {
        CAPTURE_PREV = OUTPUT;
        CAPTURE_LEN = 0;
        OUTPUT = OutputDest::Capture;
    }
}

/// End capture and return captured data. Restores previous output mode.
pub fn capture_end() -> &'static [u8] {
    unsafe {
        let data = &CAPTURE_BUF[..CAPTURE_LEN];
        OUTPUT = CAPTURE_PREV;
        data
    }
}

// ─── Background Network Service State ────────────────────────────────────────

static mut NET_SERVICE_SLOT: usize = 0;
static mut NET_SERVICE_PORT: u16 = 0;
static mut NET_SERVICE_STOP: bool = false;
static mut NET_SERVICE_RUNNING: bool = false;
static mut OUTPUT_TCP_SLOT: usize = 0;

/// Saved ELR/SPSR for inline EL0 test restoration (accessed from entry.S)
#[export_name = "SAVED_ELR_FOR_EL0"]
pub static mut SAVED_ELR_FOR_EL0: u64 = 0;
#[export_name = "SAVED_SPSR_FOR_EL0"]
pub static mut SAVED_SPSR_FOR_EL0: u64 = 0;

/// Check if a background network service is running (slots 1+)
pub fn is_net_service_active() -> bool {
    unsafe { NET_SERVICE_RUNNING }
}

/// Read a little-endian u64 from a byte slice at the given offset
fn read_u64_le(buf: &[u8], off: usize) -> u64 {
    if off + 8 > buf.len() { return 0; }
    let mut val: u64 = 0;
    for i in 0..8 {
        val |= (buf[off + i] as u64) << (i * 8);
    }
    val
}

/// Set output to a specific TCP slot
pub fn set_output_to_slot(slot: usize) {
    unsafe {
        OUTPUT = OutputDest::Tcp;
        OUTPUT_TCP_SLOT = slot;
    }
}

#[inline(never)]
pub fn print_dec(mut n: u32) {
    if n == 0 { putc(b'0'); return; }
    let mut buf: [u8; 10] = [0; 10];
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
        if i >= 10 { break; }
    }
    while i > 0 { i -= 1; putc(buf[i]); }
}

pub fn print_hex(val: u32) {
    for i in (0..8).rev() {
        let nibble = ((val >> (i * 4)) & 0xF) as u8;
        putc(if nibble < 10 { b'0' + nibble } else { b'a' + nibble - 10 });
    }
}

pub fn print_hex_byte(val: u8) {
    let hi = (val >> 4) & 0xF;
    let lo = val & 0xF;
    putc(if hi < 10 { b'0' + hi } else { b'a' + hi - 10 });
    putc(if lo < 10 { b'0' + lo } else { b'a' + lo - 10 });
}

pub fn print_ip(ip: [u8; 4]) {
    for i in 0..4 {
        if i > 0 { putc(b'.'); }
        print_dec(ip[i] as u32);
    }
}

// ─── Helper Functions ───────────────────────────────────────────────────────

pub fn eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() { return false; }
    for i in 0..a.len() { if a[i] != b[i] { return false; } }
    true
}

pub fn starts_with(a: &[u8], b: &[u8]) -> bool {
    a.len() >= b.len() && eq(&a[..b.len()], b)
}

pub fn skip_spaces(s: &[u8]) -> &[u8] {
    let mut i = 0;
    while i < s.len() && s[i] == b' ' { i += 1; }
    &s[i..]
}

pub fn split_at_space(s: &[u8]) -> (&[u8], &[u8]) {
    for i in 0..s.len() {
        if s[i] == b' ' {
            return (&s[..i], &s[i..]);
        }
    }
    (s, &[])
}

pub fn parse_ip(s: &[u8]) -> Option<[u8; 4]> {
    let mut octets = [0u8; 4];
    let mut idx = 0;
    let mut val: u8 = 0;
    let mut has_digit = false;

    for &b in s {
        if b == b'.' {
            if !has_digit || idx >= 3 { return None; }
            octets[idx] = val;
            idx += 1;
            val = 0;
            has_digit = false;
        } else if b >= b'0' && b <= b'9' {
            has_digit = true;
            val = val.wrapping_mul(10).wrapping_add(b - b'0');
        } else {
            break;
        }
    }

    if !has_digit || idx != 3 { return None; }
    octets[idx] = val;
    Some(octets)
}

pub fn parse_u16(s: &[u8]) -> Option<u16> {
    let mut val: u16 = 0;
    let mut has_digit = false;
    for &b in s {
        if b >= b'0' && b <= b'9' {
            has_digit = true;
            val = val.wrapping_mul(10).wrapping_add((b - b'0') as u16);
        } else {
            break;
        }
    }
    if has_digit { Some(val) } else { None }
}

pub fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub fn parse_u32_hex(s: &[u8]) -> Option<u32> {
    let mut val: u32 = 0;
    let mut has_digit = false;
    for &b in s {
        if let Some(n) = hex_nibble(b) {
            has_digit = true;
            val = val.wrapping_shl(4) | (n as u32);
        } else if b == b'x' || b == b'X' {
            if val == 0 { continue; } else { break; }
        } else {
            break;
        }
    }
    if has_digit { Some(val) } else { None }
}

// ─── Command Dispatch ───────────────────────────────────────────────────────

/// Test task for spawn command (QEMU only)
#[cfg(not(feature = "board-redfin"))]
extern "C" fn test_task() -> ! {
    let mut count = 0u32;
    loop {
        puts("[task2] tick ");
        print_dec(count);
        puts("\r\n");
        count += 1;
        for _ in 0..500000 { spin_loop(); }
    }
}

/// Background shell server task — accepts TCP connections and serves remote shell
#[cfg(not(feature = "board-redfin"))]
extern "C" fn shellserver_task() -> ! {
    let port = unsafe { NET_SERVICE_PORT };
    let slot = unsafe { NET_SERVICE_SLOT };

    loop {
        // Check stop flag
        if unsafe { NET_SERVICE_STOP } {
            break;
        }

        // Listen on port
        if !crate::tcp::tcp_slot_listen(slot, port) {
            uart::puts(U, "[shellserver] listen failed, retrying...\r\n");
            for _ in 0..500000 { spin_loop(); }
            continue;
        }

        // Wait for connection
        let mut connected = false;
        for _ in 0..30000u32 {
            if unsafe { NET_SERVICE_STOP } { break; }
            crate::tcp::poll();
            match crate::tcp::tcp_slot_state(slot) {
                crate::tcp::TcpState::Connected => { connected = true; break; }
                crate::tcp::TcpState::Closed => break,
                _ => {}
            }
            for _ in 0..50000 { spin_loop(); }
        }

        if !connected {
            crate::tcp::tcp_slot_close(slot);
            if unsafe { NET_SERVICE_STOP } { break; }
            continue;
        }

        uart::puts(U, "[shellserver] client connected\r\n");

        // Send banner
        crate::tcp::tcp_slot_send_blocking(slot, b"Aginx OS remote shell\r\n# ");

        set_output_to_slot(slot);

        let mut editor = LineEditor::new();
        let mut tcp_buf = [0u8; 256];

        loop {
            if unsafe { NET_SERVICE_STOP } { break; }
            crate::tcp::poll();

            let n = crate::tcp::tcp_slot_recv(slot, &mut tcp_buf);
            for &c in &tcp_buf[..n] {
                if editor.feed(c, OutputDest::Tcp) == InputAction::Complete {
                    let line = editor.line();
                    if line.len() > 0 {
                        execute_command(line);
                    }
                    editor.reset();
                    set_output_to_slot(slot);
                    putc(b'#');
                    putc(b' ');
                }
            }

            match crate::tcp::tcp_slot_state(slot) {
                crate::tcp::TcpState::Closed | crate::tcp::TcpState::None => break,
                _ => {}
            }

            // Check mailbox for inter-task messages
            let mut mail_buf = [0u8; 256];
            let mail_n = crate::task::ipc_recv(&mut mail_buf);
            if mail_n > 0 {
                crate::tcp::tcp_slot_send_blocking(slot, &mail_buf[..mail_n]);
            }

            for _ in 0..50000 { spin_loop(); }
        }

        set_output(OutputDest::Uart);
        crate::tcp::tcp_slot_close(slot);
        uart::puts(U, "[shellserver] client disconnected, re-listening\r\n");
    }

    // Cleanup and exit
    set_output(OutputDest::Uart);
    crate::tcp::tcp_slot_close(slot);
    unsafe {
        NET_SERVICE_RUNNING = false;
        NET_SERVICE_STOP = false;
    }
    crate::task::task_exit()
}

/// Background agent protocol server task
#[cfg(not(feature = "board-redfin"))]
extern "C" fn agentserver_task() -> ! {
    let port = unsafe { NET_SERVICE_PORT };
    let slot = unsafe { NET_SERVICE_SLOT };

    loop {
        if unsafe { NET_SERVICE_STOP } { break; }

        if !crate::tcp::tcp_slot_listen(slot, port) {
            uart::puts(U, "[agentserver] listen failed, retrying...\r\n");
            for _ in 0..500000 { spin_loop(); }
            continue;
        }

        let mut connected = false;
        for _ in 0..30000u32 {
            if unsafe { NET_SERVICE_STOP } { break; }
            crate::tcp::poll();
            match crate::tcp::tcp_slot_state(slot) {
                crate::tcp::TcpState::Connected => { connected = true; break; }
                crate::tcp::TcpState::Closed => break,
                _ => {}
            }
            for _ in 0..50000 { spin_loop(); }
        }

        if !connected {
            crate::tcp::tcp_slot_close(slot);
            if unsafe { NET_SERVICE_STOP } { break; }
            continue;
        }

        uart::puts(U, "[agentserver] client connected\r\n");

        let mut line_buf = [0u8; 256];
        let mut line_len = 0usize;
        let mut tcp_buf = [0u8; 256];

        loop {
            if unsafe { NET_SERVICE_STOP } { break; }
            crate::tcp::poll();
            let n = crate::tcp::tcp_slot_recv(slot, &mut tcp_buf);

            for &c in &tcp_buf[..n] {
                if c == b'\n' || c == b'\r' {
                    if line_len > 0 {
                        agent_handle_line(slot, &line_buf[..line_len]);
                        line_len = 0;
                    }
                } else if line_len < 255 {
                    line_buf[line_len] = c;
                    line_len += 1;
                }
            }

            match crate::tcp::tcp_slot_state(slot) {
                crate::tcp::TcpState::Closed | crate::tcp::TcpState::None => break,
                _ => {}
            }

            for _ in 0..50000 { spin_loop(); }
        }

        crate::tcp::tcp_slot_close(slot);
        uart::puts(U, "[agentserver] client disconnected, re-listening\r\n");
    }

    set_output(OutputDest::Uart);
    crate::tcp::tcp_slot_close(slot);
    unsafe {
        NET_SERVICE_RUNNING = false;
        NET_SERVICE_STOP = false;
    }
    crate::task::task_exit()
}

/// Execute a shell command. Output goes to whatever OUTPUT is set to.
/// Commands with blocking inner loops (ping, httpget, dhclient, listen, connect)
/// still output to UART since they have their own polling loops.
pub fn execute_command(cmd: &[u8]) {
    let len = cmd.len();
    if len == 0 { return; }

    if eq(cmd, b"help") {
        puts("Commands: help version uptime mem alloc pci net poll send status dhclient ping listen connect sendmsg virtio tasks spawn halt ifconfig clear echo reboot blkinfo blkread blkwrite mkfs ls cat rm writefile usbprobe dns nslookup httpget telnet agent shellserver agentserver netstop mail exec execdirect spmi\r\n");
    } else if eq(cmd, b"version") {
        puts("Aginx v0.2.0 aarch64 (MMU enabled)\r\n");
    } else if eq(cmd, b"reboot") {
        puts("Rebooting...\r\n");
        loop { unsafe { core::arch::asm!("brk 0"); } }
    } else if eq(cmd, b"halt") {
        puts("Halted.\r\n");
        loop { unsafe { core::arch::asm!("wfi"); } }
    } else if eq(cmd, b"spmi") {
        // SPMI diagnostic: scan observer channels directly
        #[cfg(feature = "board-redfin")]
        {
            let ver = crate::spmi::get_version();
            puts("SPMI ver=0x"); print_hex(ver);
            let apid_cnt = crate::spmi::get_apid_count();
            puts(" apids="); print_dec(apid_cnt);
            puts("\r\n");
            // Scan all observer channels
            let max = if apid_cnt > 256 { 256u32 } else { apid_cnt };
            for apid in 0..max {
                let (_, status, rdata) = crate::spmi::obs_cmd_read(apid, 0x04, 4);
                if (status & 1) != 0 && (status & 6) == 0 && rdata != 0 {
                    let typ = (rdata & 0xFF) as u8;
                    let sub = ((rdata >> 8) & 0xFF) as u8;
                    puts(" A"); print_dec(apid);
                    puts(" t=0x"); print_hex_byte(typ);
                    puts(" s=0x"); print_hex_byte(sub);
                    puts("\r\n");
                }
            }
            puts("scan done\r\n");
        }
        #[cfg(not(feature = "board-redfin"))]
        { puts("SPMI not available on QEMU\r\n"); }
    } else if eq(cmd, b"clear") {
        puts("\x1B[2J\x1B[H");
    } else if starts_with(cmd, b"echo ") {
        let rest = skip_spaces(&cmd[5..len]);
        puts(core::str::from_utf8(rest).unwrap_or(""));
        puts("\r\n");
    } else if eq(cmd, b"uptime") {
        let ticks = crate::interrupt::get_ticks();
        let t = ticks as u32;
        let secs = t / 100;
        let tenths = (t % 100) / 10;
        print_dec(secs);
        putc(b'.');
        putc(b'0' + tenths as u8);
        puts("s\r\n");
    } else if eq(cmd, b"mem") {
        let free = crate::frame_alloc::free_count();
        puts("free: 0x");
        print_hex(free as u32);
        puts(" pages (0x");
        let kb = free * 4;
        print_hex(kb as u32);
        puts(" KB)\r\n");
    } else if eq(cmd, b"alloc") {
        #[cfg(not(feature = "board-redfin"))]
        {
            let mut v: Vec<u8> = Vec::new();
            for i in 0..100u8 { v.push(i); }
            puts("Vec at 0x");
            print_hex(v.as_ptr() as u32);
            puts(" len=0x");
            print_hex(v.len() as u32);
            puts("\r\n");
        }
    } else if eq(cmd, b"pci") {
        match crate::pci::get_virtio_net() {
            Some(info) => {
                puts("virtio-net at 00:");
                print_hex((info.dev as u32) << 16);
                puts(".0 irq=");
                print_dec(info.irq_line as u32);
                puts(" BAR0=0x");
                print_hex(crate::pci::read_bar(info, 0) as u32);
                puts("\r\n");
            }
            None => { puts("No virtio-net found\r\n"); }
        }
    } else if eq(cmd, b"net") {
        match crate::net::get_mac() {
            Some(mac) => {
                puts("MAC: ");
                for (i, b) in mac.iter().enumerate() { if i > 0 { putc(b':'); } print_hex_byte(*b); }
                puts("\r\n");
            }
            None => { puts("Net not initialized\r\n"); }
        }
    } else if eq(cmd, b"poll") {
        puts("Polling network..\r\n");
        let count = crate::net::poll();
        puts("Received ");
        print_dec(count as u32);
        puts(" packets\r\n");
    } else if eq(cmd, b"send") {
        let mac = match crate::net::get_mac() {
            Some(m) => m,
            None => { puts("No MAC\r\n"); return; }
        };
        let mut test_frame: [u8; 64] = [0; 64];
        test_frame[0..6].copy_from_slice(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
        test_frame[6..12].copy_from_slice(&mac);
        test_frame[12] = 0x08; test_frame[13] = 0x06;
        test_frame[14] = 0x00; test_frame[15] = 0x01;
        test_frame[16] = 0x08; test_frame[17] = 0x00;
        test_frame[18] = 0x06; test_frame[19] = 0x04;
        test_frame[20] = 0x00; test_frame[21] = 0x01;
        test_frame[22..28].copy_from_slice(&mac);
        test_frame[22..26].copy_from_slice(&[10, 0, 2, 15]);
        test_frame[32..36].copy_from_slice(&[10, 0, 2, 2]);
        if crate::net::transmit(&test_frame[..42]) { puts("Sent test frame\r\n"); } else { puts("Send failed\r\n"); }
    } else if eq(cmd, b"status") {
        crate::tcp::print_status();
    } else if eq(cmd, b"dhclient") {
        // UART-only: has blocking loop with uart::puts
        uart::puts(U, "Polling for DHCP...\r\n");
        let mut count: u32 = 0;
        for i in 0..5000u32 {
            let pkts = crate::tcp::poll();
            count += pkts as u32;
            if crate::tcp::get_ip().is_some() { break; }
            for _ in 0..2000000 { spin_loop(); }
            if i % 500 == 0 { uart::putc(U, b'.'); }
        }
        uart::puts(U, "\r\nPolled ");
        crate::print_dec_u32(U, count);
        uart::puts(U, " times\r\n");
        if let Some(ip) = crate::tcp::get_ip() {
            uart::puts(U, "Got IP: ");
            for i in 0..4 { if i > 0 { uart::putc(U, b'.'); } crate::print_dec_u32(U, ip.octets()[i] as u32); }
            uart::puts(U, "\r\n");
        } else { uart::puts(U, "DHCP timeout\r\n"); }
    } else if starts_with(cmd, b"ping ") {
        // UART-only: tcp::ping has blocking loop
        let rest = skip_spaces(&cmd[5..len]);
        let ip = match parse_ip(rest) {
            Some(ip) => ip,
            None => { puts("Usage: ping <ip>\r\n"); return; }
        };
        uart::puts(U, "PING ");
        crate::print_ip(U, ip);
        uart::puts(U, " x3\r\n");
        let replies = crate::tcp::ping(ip, 3);
        uart::puts(U, "--- ");
        crate::print_ip(U, ip);
        uart::puts(U, " ping stats ---\r\n  ");
        crate::print_dec_u32(U, replies as u32);
        uart::puts(U, "/3 received\r\n");
    } else if starts_with(cmd, b"listen ") {
        // Non-blocking: set up TCP listen, return to shell
        let rest = skip_spaces(&cmd[7..len]);
        let port = match parse_u16(rest) {
            Some(p) => p,
            None => { puts("Usage: listen <port>\r\n"); return; }
        };
        if crate::tcp::tcp_listen(port) {
            puts("[OK] listening on :");
            crate::print_dec_u32(U, port as u32);
            puts("\r\n");
        } else {
            puts("[FAIL] listen\r\n");
        }
    } else if starts_with(cmd, b"connect ") {
        // UART-only: blocking wait loop
        let rest = skip_spaces(&cmd[8..len]);
        let (ip_bytes, rest) = split_at_space(rest);
        let ip = match parse_ip(ip_bytes) {
            Some(ip) => ip,
            None => { puts("Usage: connect <ip> <port>\r\n"); return; }
        };
        let port_rest = skip_spaces(rest);
        let port = match parse_u16(port_rest) {
            Some(p) => p,
            None => { puts("Usage: connect <ip> <port>\r\n"); return; }
        };
        if crate::tcp::tcp_connect(ip, port) {
            for i in 0..10000u32 {
                crate::tcp::poll(); crate::tcp::check_tcp_data();
                match crate::tcp::tcp_connect_state() {
                    crate::tcp::TcpState::Connected => { uart::puts(U, "[OK] TCP connected!\r\n"); break; }
                    crate::tcp::TcpState::Closed => { uart::puts(U, "[FAIL] TCP connection refused\r\n"); break; }
                    _ => {}
                }
                if i > 0 && i % 2000 == 0 { uart::putc(U, b'.'); }
                for _ in 0..100000 { spin_loop(); }
            }
        }
    } else if starts_with(cmd, b"sendmsg ") {
        let msg = &cmd[8..len];
        if crate::tcp::tcp_send(msg) {
            puts("[OK] Sent ");
            print_dec(msg.len() as u32);
            puts(" bytes\r\n");
        }
    } else if eq(cmd, b"virtio") {
        crate::net::dump_state();
    } else if eq(cmd, b"usbprobe") {
        if crate::xhci::is_ready() {
            puts("USB device: slot=");
            let state = crate::xhci::get_state().unwrap();
            print_dec(state.device_slot as u32);
            puts(" port=");
            print_dec(state.device_port as u32);
            puts("\r\n");
        } else {
            puts("No USB device\r\n");
        }
    } else if starts_with(cmd, b"nslookup ") {
        let name = skip_spaces(&cmd[9..len]);
        if name.is_empty() {
            puts("Usage: nslookup <hostname>\r\n");
        } else if let Some(addr) = crate::tcp::dns_resolve(name) {
            puts("DNS: ");
            print_ip(addr.octets());
            puts("\r\n");
        } else {
            puts("[FAIL] DNS resolution failed\r\n");
        }
    } else if starts_with(cmd, b"dns") {
        let name = skip_spaces(&cmd[4..len]);
        if name.is_empty() {
            if let Some(dns) = crate::tcp::get_dns_server() {
                puts("DNS server: ");
                print_ip(dns.octets());
                puts("\r\n");
            } else {
                puts("No DNS server configured\r\n");
            }
        } else if let Some(addr) = crate::tcp::dns_resolve(name) {
            puts("DNS: ");
            print_ip(addr.octets());
            puts("\r\n");
        } else {
            puts("[FAIL] DNS resolution failed\r\n");
        }
    } else if eq(cmd, b"tasks") {
        crate::task::print_task_list();
    } else if eq(cmd, b"spawn") {
        #[cfg(not(feature = "board-redfin"))]
        {
            match crate::task::task_create("test", test_task) {
                Some(id) => { puts("[OK] Created task "); print_dec(id); puts("\r\n"); }
                None => { puts("[FAIL] No free task slots\r\n"); }
            }
        }
    } else if eq(cmd, b"usertest") {
        #[cfg(not(feature = "board-redfin"))]
        {
            // Direct EL0 test: ERET to EL0, write to UART, SVC back to EL1
            puts("[..] Direct ERET to EL0 test...\r\n");
            unsafe {
                // Set up a small user stack
                let user_stack_size = 4096usize;
                let layout = Layout::from_size_align(user_stack_size, 16).unwrap();
                let user_stack_base = alloc_zeroed(layout);
                let user_stack_top = user_stack_base as u64 + user_stack_size as u64;

                // Set SP_EL0 for user stack
                core::arch::asm!("msr SP_EL0, {}", in(reg) user_stack_top);

                let elr_ptr = &raw mut SAVED_ELR_FOR_EL0;
                let spsr_ptr = &raw mut SAVED_SPSR_FOR_EL0;
                core::arch::asm!(
                    // Save ALL callee-saved registers before ERET (x19-x30)
                    "stp x29, x30, [sp, #-96]!",
                    "stp x27, x28, [sp, #16]",
                    "stp x25, x26, [sp, #32]",
                    "stp x23, x24, [sp, #48]",
                    "stp x21, x22, [sp, #64]",
                    "stp x19, x20, [sp, #80]",

                    // Save return address and SPSR to globals
                    "adr x10, 4f",
                    "str x10, [{elr_ptr}]",
                    "mov x12, #5",           // SPSR = EL1h
                    "str x12, [{spsr_ptr}]",

                    // Set ELR to EL0 code, SPSR to EL0t, and ERET
                    "ldr x11, =0x09000000",
                    "adr x10, 2f",
                    "msr ELR_EL1, x10",
                    "mov x10, #0",
                    "msr SPSR_EL1, x10",      // EL0t
                    "eret",

                    // === EL0 code starts here ===
                    "2:",
                    "mov w0, #'['",
                    "strb w0, [x11]",
                    "mov w0, #'E'",
                    "strb w0, [x11]",
                    "mov w0, #'L'",
                    "strb w0, [x11]",
                    "mov w0, #'0'",
                    "strb w0, [x11]",
                    "mov w0, #']'",
                    "strb w0, [x11]",
                    "mov w0, #' '",
                    "strb w0, [x11]",
                    "mov w0, #'o'",
                    "strb w0, [x11]",
                    "mov w0, #'k'",
                    "strb w0, [x11]",
                    "mov w0, #'\n'",
                    "strb w0, [x11]",

                    // SVC to return to EL1 (syscall 99)
                    "mov x8, #99",
                    "mov x0, #0",
                    "svc #0",

                    // Should not reach here
                    "3: wfi",
                    "b 3b",

                    // === Return point after SVC handler restores ELR ===
                    "4:",
                    // Restore callee-saved registers (matches 96-byte save)
                    "ldp x19, x20, [sp, #80]",
                    "ldp x21, x22, [sp, #64]",
                    "ldp x23, x24, [sp, #48]",
                    "ldp x25, x26, [sp, #32]",
                    "ldp x27, x28, [sp, #16]",
                    "ldp x29, x30, [sp], #96",
                    // Write "[OK] Back from EL0\n" directly to UART
                    // (Can't use Rust puts() here — compiler doesn't know
                    //  label 4 is reachable, so won't emit string pointer load)
                    "ldr x11, =0x09000000",
                    "mov w0, #'['",
                    "strb w0, [x11]",
                    "mov w0, #'O'",
                    "strb w0, [x11]",
                    "mov w0, #'K'",
                    "strb w0, [x11]",
                    "mov w0, #']'",
                    "strb w0, [x11]",
                    "mov w0, #' '",
                    "strb w0, [x11]",
                    "mov w0, #'B'",
                    "strb w0, [x11]",
                    "mov w0, #'a'",
                    "strb w0, [x11]",
                    "mov w0, #'c'",
                    "strb w0, [x11]",
                    "mov w0, #'k'",
                    "strb w0, [x11]",
                    "mov w0, #' '",
                    "strb w0, [x11]",
                    "mov w0, #'f'",
                    "strb w0, [x11]",
                    "mov w0, #'r'",
                    "strb w0, [x11]",
                    "mov w0, #'o'",
                    "strb w0, [x11]",
                    "mov w0, #'m'",
                    "strb w0, [x11]",
                    "mov w0, #' '",
                    "strb w0, [x11]",
                    "mov w0, #'E'",
                    "strb w0, [x11]",
                    "mov w0, #'L'",
                    "strb w0, [x11]",
                    "mov w0, #'0'",
                    "strb w0, [x11]",
                    "mov w0, #'\n'",
                    "strb w0, [x11]",

                    elr_ptr = in(reg) elr_ptr,
                    spsr_ptr = in(reg) spsr_ptr,
                    out("x10") _,
                    out("x11") _,
                    out("x12") _,
                );
            }
            // "[OK] \n" already written directly from asm label 4
        }
    } else if starts_with(cmd, b"execdirect ") {
        // Direct ERET to EL0 with loaded binary (no scheduler involvement)
        let rest = skip_spaces(&cmd[11..len]);
        if rest.is_empty() {
            puts("Usage: execdirect <filename>\r\n");
        } else if !crate::fs::is_mounted() {
            puts("[FAIL] FS not mounted\r\n");
        } else {
            let mut buf = [0u8; 4096];
            match crate::fs::read(rest, &mut buf) {
                Some(n) if n > 0 => {
                    // Load code to heap
                    let code_size = ((n + 4095) / 4096) * 4096;
                    let layout = Layout::from_size_align(code_size, 4096).unwrap();
                    let code_ptr = unsafe { alloc_zeroed(layout) };
                    if code_ptr.is_null() {
                        puts("[FAIL] alloc\r\n");
                        return;
                    }
                    unsafe {
                        for i in 0..n {
                            core::ptr::write_volatile(code_ptr.add(i), buf[i]);
                        }
                        core::arch::asm!("dsb sy", "isb");
                    }
                    let code_pa = code_ptr as usize;

                    // Set up user stack
                    let stack_layout = Layout::from_size_align(8192, 16).unwrap();
                    let stack_ptr = unsafe { alloc::alloc::alloc_zeroed(stack_layout) };
                    if stack_ptr.is_null() {
                        puts("[FAIL] stack alloc\r\n");
                        return;
                    }
                    let stack_top = stack_ptr as usize + 8192;

                    puts("[OK] Loaded ");
                    print_dec(n as u32);
                    puts(" bytes PA=0x");
                    print_hex(code_pa as u32);
                    puts("\r\n");

                    // Direct ERET to EL0 (like usertest, but with dynamic code)
                    unsafe {
                        // Disable IRQ during EL0 test (prevent scheduler interference)
                        core::arch::asm!("msr DAIFSet, #0x4");

                        core::arch::asm!("msr SP_EL0, {}", in(reg) stack_top as u64);

                        let elr_ptr = &raw mut SAVED_ELR_FOR_EL0;
                        let spsr_ptr = &raw mut SAVED_SPSR_FOR_EL0;

                        core::arch::asm!(
                            // Save callee-saved registers
                            "stp x29, x30, [sp, #-96]!",
                            "stp x27, x28, [sp, #16]",
                            "stp x25, x26, [sp, #32]",
                            "stp x23, x24, [sp, #48]",
                            "stp x21, x22, [sp, #64]",
                            "stp x19, x20, [sp, #80]",

                            // Save return point
                            "adr x10, 4f",
                            "str x10, [{elr_ptr}]",
                            "mov x12, #5",
                            "str x12, [{spsr_ptr}]",

                            // ERET to user code
                            "mov x10, {entry}",
                            "msr ELR_EL1, x10",
                            "mov x10, #0",
                            "msr SPSR_EL1, x10",
                            "eret",

                            // Return point (SVC 99 brings us back here)
                            "4:",
                            "ldp x19, x20, [sp, #80]",
                            "ldp x21, x22, [sp, #64]",
                            "ldp x23, x24, [sp, #48]",
                            "ldp x25, x26, [sp, #32]",
                            "ldp x27, x28, [sp, #16]",
                            "ldp x29, x30, [sp], #96",

                            elr_ptr = in(reg) elr_ptr,
                            spsr_ptr = in(reg) spsr_ptr,
                            entry = in(reg) code_pa as u64,
                            out("x10") _,
                            out("x12") _,
                        );

                        // Re-enable IRQ
                        core::arch::asm!("msr DAIFClr, #0x4");
                    }
                    puts("[OK] Back from EL0\r\n");
                }
                Some(_) => { puts("[FAIL] Empty file\r\n"); }
                None => { puts("[FAIL] File not found\r\n"); }
            }
        }
    } else if eq(cmd, b"ifconfig") {
        if let Some(ip) = crate::tcp::get_ip() {
            puts("  inet ");
            print_ip(ip.octets());
            puts("  netmask 255.255.255.0\r\n");
        } else {
            puts("  inet (no IP)\r\n");
        }
        if let Some(gw) = crate::tcp::get_gateway() {
            puts("  gateway ");
            print_ip(gw.octets());
            puts("\r\n");
        }
        if let Some(mac) = crate::tcp::get_mac() {
            puts("  ether ");
            for (i, b) in mac.0.iter().enumerate() { if i > 0 { putc(b':'); } print_hex_byte(*b); }
            puts("\r\n");
        }
    } else if eq(cmd, b"blkinfo") {
        let cap = crate::blk::capacity();
        if cap > 0 {
            puts("Block device: ");
            print_dec(cap as u32);
            puts(" sectors (");
            print_dec((cap * 512 / (1024 * 1024)) as u32);
            puts(" MB)\r\n");
        } else {
            puts("No block device\r\n");
        }
    } else if starts_with(cmd, b"blkread ") {
        let rest = skip_spaces(&cmd[8..len]);
        let sector = match parse_u32_hex(rest) {
            Some(s) => s,
            None => { puts("Usage: blkread <sector>\r\n"); return; }
        };
        crate::blk::hexdump_sector(sector as u64);
    } else if starts_with(cmd, b"blkwrite ") {
        let rest = skip_spaces(&cmd[9..len]);
        let (sector_str, hex_rest) = split_at_space(rest);
        let sector = match parse_u32_hex(sector_str) {
            Some(s) => s,
            None => { puts("Usage: blkwrite <sector> <hex>\r\n"); return; }
        };
        let hex_bytes = skip_spaces(hex_rest);
        let mut buf = [0u8; 512];
        let mut buf_len = 0usize;
        let mut i = 0;
        while i + 1 < hex_bytes.len() && buf_len < 512 {
            let hi = hex_nibble(hex_bytes[i]);
            let lo = hex_nibble(hex_bytes[i + 1]);
            if hi.is_none() || lo.is_none() { break; }
            buf[buf_len] = (hi.unwrap() << 4) | lo.unwrap();
            buf_len += 1;
            i += 2;
            if i < hex_bytes.len() && hex_bytes[i] == b' ' { i += 1; }
        }
        if buf_len == 0 {
            puts("Usage: blkwrite <sector> <hex>\r\n");
        } else if crate::blk::write_block(sector as u64, &buf) {
            puts("[OK] Wrote ");
            print_dec(buf_len as u32);
            puts(" bytes to sector ");
            print_dec(sector);
            puts("\r\n");
        } else {
            puts("[FAIL] Write failed\r\n");
        }
    } else if eq(cmd, b"mkfs") {
        if crate::fs::format() {
            puts("[OK] Filesystem formatted\r\n");
        } else {
            puts("[FAIL] Format failed\r\n");
        }
    } else if eq(cmd, b"ls") {
        if !crate::fs::is_mounted() {
            puts("[FAIL] FS not mounted (use mkfs)\r\n");
        } else {
            crate::fs::list_files();
        }
    } else if starts_with(cmd, b"cat ") {
        let name = skip_spaces(&cmd[4..len]);
        if !crate::fs::is_mounted() {
            puts("[FAIL] FS not mounted\r\n");
        } else {
            let mut buf = [0u8; 4096];
            match crate::fs::read(name, &mut buf) {
                Some(n) => {
                    for &b in &buf[..n] {
                        if b >= 0x20 && b < 0x7F {
                            putc(b);
                        } else if b == b'\n' {
                            puts("\r\n");
                        } else {
                            putc(b'.');
                        }
                    }
                    puts("\r\n");
                }
                None => puts("[FAIL] File not found\r\n"),
            }
        }
    } else if starts_with(cmd, b"rm ") {
        let name = skip_spaces(&cmd[3..len]);
        if !crate::fs::is_mounted() {
            puts("[FAIL] FS not mounted\r\n");
        } else if crate::fs::delete(name) {
            puts("[OK] Deleted\r\n");
        } else {
            puts("[FAIL] Delete failed\r\n");
        }
    } else if starts_with(cmd, b"writefile ") {
        let rest = skip_spaces(&cmd[10..len]);
        let (name, text_raw) = split_at_space(rest);
        let text = skip_spaces(text_raw);
        if name.is_empty() {
            puts("Usage: writefile <name> <text>  (use \\n for newlines)\r\n");
        } else if !crate::fs::is_mounted() {
            puts("[FAIL] FS not mounted\r\n");
        } else {
            // Process escape sequences in text: \n -> newline, \\ -> backslash
            let mut processed = [0u8; 4096];
            let mut plen = 0;
            let mut i = 0;
            while i < text.len() && plen < processed.len() - 1 {
                if text[i] == b'\\' && i + 1 < text.len() {
                    match text[i + 1] {
                        b'n' => { processed[plen] = b'\n'; plen += 1; i += 2; }
                        b'r' => { processed[plen] = b'\r'; plen += 1; i += 2; }
                        b'\\' => { processed[plen] = b'\\'; plen += 1; i += 2; }
                        _ => { processed[plen] = text[i]; plen += 1; i += 1; }
                    }
                } else {
                    processed[plen] = text[i];
                    plen += 1;
                    i += 1;
                }
            }
            if crate::fs::create(name, &processed[..plen]) {
                puts("[OK] File written\r\n");
            } else {
                puts("[FAIL] Write failed\r\n");
            }
        }
    } else if starts_with(cmd, b"httpget ") {
        // UART-only: complex blocking loop
        cmd_httpget(cmd, len);
    } else if starts_with(cmd, b"telnet ") {
        // Remote shell server
        let rest = skip_spaces(&cmd[7..len]);
        let port = match parse_u16(rest) {
            Some(p) => p,
            None => { puts("Usage: telnet <port>\r\n"); return; }
        };
        cmd_telnet(port);
    } else if starts_with(cmd, b"agent ") {
        // Agent protocol server
        let rest = skip_spaces(&cmd[6..len]);
        let port = match parse_u16(rest) {
            Some(p) => p,
            None => { puts("Usage: agent <port>\r\n"); return; }
        };
        cmd_agent(port);
    } else if starts_with(cmd, b"shellserver ") {
        #[cfg(not(feature = "board-redfin"))]
        {
            let rest = skip_spaces(&cmd[12..len]);
            let port = match parse_u16(rest) {
                Some(p) => p,
                None => { puts("Usage: shellserver <port>\r\n"); return; }
            };
            let slot = match crate::tcp::tcp_slot_alloc() {
                Some(s) if s > 0 => s,  // Don't use slot 0 (reserved for legacy)
                _ => { puts("[FAIL] No free TCP slots\r\n"); return; }
            };
            unsafe {
                NET_SERVICE_SLOT = slot;
                NET_SERVICE_PORT = port;
                NET_SERVICE_STOP = false;
                NET_SERVICE_RUNNING = true;
            }
            match crate::task::task_create("shellserver", shellserver_task) {
                Some(id) => {
                    puts("[OK] Shell server on :");
                    print_dec(port as u32);
                    puts(" slot=");
                    print_dec(slot as u32);
                    puts(" task=");
                    print_dec(id);
                    puts("\r\n");
                }
                None => {
                    unsafe { NET_SERVICE_RUNNING = false; crate::tcp::tcp_slot_close(slot); }
                    puts("[FAIL] No free task slots\r\n");
                }
            }
        }
    } else if starts_with(cmd, b"agentserver ") {
        #[cfg(not(feature = "board-redfin"))]
        {
            let rest = skip_spaces(&cmd[12..len]);
            let port = match parse_u16(rest) {
                Some(p) => p,
                None => { puts("Usage: agentserver <port>\r\n"); return; }
            };
            let slot = match crate::tcp::tcp_slot_alloc() {
                Some(s) if s > 0 => s,
                _ => { puts("[FAIL] No free TCP slots\r\n"); return; }
            };
            unsafe {
                NET_SERVICE_SLOT = slot;
                NET_SERVICE_PORT = port;
                NET_SERVICE_STOP = false;
                NET_SERVICE_RUNNING = true;
            }
            match crate::task::task_create("agentserver", agentserver_task) {
                Some(id) => {
                    puts("[OK] Agent server on :");
                    print_dec(port as u32);
                    puts(" slot=");
                    print_dec(slot as u32);
                    puts(" task=");
                    print_dec(id);
                    puts("\r\n");
                }
                None => {
                    unsafe { NET_SERVICE_RUNNING = false; crate::tcp::tcp_slot_close(slot); }
                    puts("[FAIL] No free task slots\r\n");
                }
            }
        }
    } else if eq(cmd, b"netstop") {
        if !is_net_service_active() {
            puts("[OK] No network service running\r\n");
        } else {
            unsafe { NET_SERVICE_STOP = true; }
            puts("[..] Stopping network service...\r\n");
        }
    } else if eq(cmd, b"mail") {
        let mut mail_buf = [0u8; 256];
        let n = crate::task::ipc_recv(&mut mail_buf);
        if n == 0 {
            puts("(empty)\r\n");
        } else {
            for &b in &mail_buf[..n] {
                if b >= 0x20 && b < 0x7F { putc(b); } else { putc(b'.'); }
            }
            puts("\r\n");
        }
    } else if starts_with(cmd, b"exec ") {
        let rest = skip_spaces(&cmd[5..len]);
        if rest.is_empty() {
            puts("Usage: exec <filename>\r\n");
        } else if !crate::fs::is_mounted() {
            puts("[FAIL] FS not mounted\r\n");
        } else {
            // Read file from filesystem
            let mut buf = [0u8; 4096];
            match crate::fs::read(rest, &mut buf) {
                Some(n) if n > 0 => {
                    // Check if it's a shell script (starts with #! or doesn't look like AArch64 code)
                    let is_script = n >= 2 && buf[0] == b'#' && buf[1] == b'!';
                    if is_script {
                        // Execute as shell script: run each line as a command
                        puts("[..] Running script\r\n");
                        let mut line_start = 0;
                        for i in 0..=n {
                            let c = if i < n { buf[i] } else { b'\n' };
                            if c == b'\n' || c == b'\r' {
                                if i > line_start {
                                    let line = &buf[line_start..i];
                                    // Skip comments and empty lines
                                    let trimmed = skip_spaces(line);
                                    if !trimmed.is_empty() && trimmed[0] != b'#' {
                                        execute_command(trimmed);
                                    }
                                }
                                line_start = i + 1;
                            }
                        }
                        puts("[OK] Script done\r\n");
                    } else {
                        // Load as EL0 binary (flat binary or ELF)
                        let is_elf = n >= 4 && buf[0] == 0x7F && buf[1] == b'E' && buf[2] == b'L' && buf[3] == b'F';

                        if is_elf {
                            // Parse ELF64: extract loadable segments
                            // ELF64 header: e_entry at offset 24, e_phoff at 32, e_phentsize at 54, e_phnum at 56
                            if n < 64 {
                                puts("[FAIL] ELF too small\r\n");
                                return;
                            }
                            let entry_va = read_u64_le(&buf, 24) as usize;
                            let phoff = read_u64_le(&buf, 32) as usize;
                            let phentsize = read_u64_le(&buf, 54) as usize;
                            let phnum = read_u64_le(&buf, 56) as usize;

                            // Find the first loadable segment (PT_LOAD = 1)
                            let mut seg_vaddr: usize = 0;
                            let mut seg_offset: usize = 0;
                            let mut seg_filesz: usize = 0;
                            let mut found = false;

                            for i in 0..phnum {
                                let off = phoff + i * phentsize;
                                if off + phentsize > n { break; }
                                let p_type = read_u64_le(&buf, off) as u32;
                                if p_type == 1 { // PT_LOAD
                                    seg_offset = read_u64_le(&buf, off + 8) as usize;
                                    seg_vaddr = read_u64_le(&buf, off + 16) as usize;
                                    seg_filesz = read_u64_le(&buf, off + 32) as usize;
                                    found = true;
                                    break;
                                }
                            }

                            if !found {
                                puts("[FAIL] No PT_LOAD segment\r\n");
                                return;
                            }

                            // Allocate code memory at VA-aligned address
                            // We need PA that maps to VA 0x10000 via user page table
                            // Allocate enough for the full segment
                            let code_size = ((seg_filesz + 4095) / 4096) * 4096;
                            let layout = Layout::from_size_align(code_size, 4096).unwrap();
                            let code_ptr = unsafe { alloc_zeroed(layout) };
                            if code_ptr.is_null() {
                                puts("[FAIL] alloc\r\n");
                                return;
                            }

                            // Copy segment data
                            if seg_offset + seg_filesz <= n {
                                unsafe {
                                    for i in 0..seg_filesz {
                                        core::ptr::write_volatile(
                                            code_ptr.add(i),
                                            buf[seg_offset + i]
                                        );
                                    }
                                }
                            }

                            // Flush instruction cache
                            unsafe { core::arch::asm!("dsb sy", "isb"); }

                            let code_pa = code_ptr as usize;

                            // Allocate user stack
                            let stack_layout = Layout::from_size_align(8192, 4096).unwrap();
                            let stack_ptr = unsafe { alloc::alloc::alloc_zeroed(stack_layout) };
                            if stack_ptr.is_null() {
                                puts("[FAIL] stack alloc\r\n");
                                return;
                            }
                            let stack_top = stack_ptr as usize + 8192;

                            // Compute identity-mapped entry PA:
                            // Code is loaded at code_pa (heap alloc, identity-mapped by kernel page table).
                            // The ELF wants entry_va, which is seg_vaddr + offset within segment.
                            // Since we loaded segment at code_pa, the entry PA = code_pa + (entry_va - seg_vaddr).
                            let entry_pa = code_pa + entry_va - seg_vaddr;

                            puts("[OK] ELF loaded ");
                            print_dec(seg_filesz as u32);
                            puts(" bytes PA=0x");
                            print_hex(code_pa as u32);
                            puts(" entry=0x");
                            print_hex(entry_pa as u32);
                            puts("\r\n");

                            // Use PA-based entry (identity-mapped by kernel page table).
                            // TTBR0 switching disabled — user_ttbr0 = 0.
                            match crate::task::task_create_user("user", entry_pa, 8192, 0) {
                                Some(id) => {
                                    puts("[OK] User task ");
                                    print_dec(id);
                                    puts(" started\r\n");
                                }
                                None => {
                                    puts("[FAIL] No free task slots\r\n");
                                }
                            }
                        } else {
                            // Flat binary: load at VA 0x10000
                            let code_size = ((n + 4095) / 4096) * 4096;
                            let layout = Layout::from_size_align(code_size, 4096).unwrap();
                            let code_ptr = unsafe { alloc_zeroed(layout) };
                            if code_ptr.is_null() {
                                puts("[FAIL] alloc\r\n");
                                return;
                            }
                            unsafe {
                                for i in 0..n {
                                    core::ptr::write_volatile(code_ptr.add(i), buf[i]);
                                }
                            }
                            unsafe { core::arch::asm!("dsb sy", "isb"); }

                            let code_pa = code_ptr as usize;
                            let stack_layout = Layout::from_size_align(8192, 4096).unwrap();
                            let stack_ptr = unsafe { alloc::alloc::alloc_zeroed(stack_layout) };
                            if stack_ptr.is_null() {
                                puts("[FAIL] stack alloc\r\n");
                                return;
                            }
                            let stack_top = stack_ptr as usize + 8192;

                            puts("[OK] Loaded ");
                            print_dec(n as u32);
                            puts(" bytes PA=0x");
                            print_hex(code_pa as u32);
                            puts("\r\n");

                            // Flat binary: entry is at code PA (identity-mapped by kernel page table).
                            match crate::task::task_create_user("user", code_pa, 8192, 0) {
                                Some(id) => {
                                    puts("[OK] User task ");
                                    print_dec(id);
                                    puts(" started\r\n");
                                }
                                None => {
                                    puts("[FAIL] No free task slots\r\n");
                                }
                            }
                        }
                    }
                }
                Some(_) => { puts("[FAIL] Empty file\r\n"); }
                None => { puts("[FAIL] File not found\r\n"); }
            }
        }
    } else {
        putc(b'?');
        putc(b'\r');
        putc(b'\n');
    }
}

// ─── HTTP GET (UART-only) ──────────────────────────────────────────────────

fn cmd_httpget(cmd: &[u8], len: usize) {
    let rest = skip_spaces(&cmd[8..len]);
    let (host, path_rest) = split_at_space(rest);
    let path = if path_rest.is_empty() { b"/" } else { skip_spaces(path_rest) };
    if host.is_empty() {
        puts("Usage: httpget <host> [<path>]\r\n");
        return;
    }
    uart::puts(U, "DNS: resolving ");
    uart::puts(U, core::str::from_utf8(host).unwrap_or("?"));
    uart::puts(U, "...\r\n");
    let addr = match crate::tcp::dns_resolve(host) {
        Some(a) => a,
        None => { uart::puts(U, "[FAIL] DNS\r\n"); return; }
    };
    uart::puts(U, "  -> ");
    crate::print_ip(U, addr.octets());
    uart::puts(U, "\r\n");
    if !crate::tcp::tcp_connect(addr.octets(), 80) {
        uart::puts(U, "[FAIL] TCP connect\r\n"); return;
    }
    let mut connected = false;
    for i in 0..10000u32 {
        crate::tcp::poll();
        match crate::tcp::tcp_connect_state() {
            crate::tcp::TcpState::Connected => { connected = true; break; }
            crate::tcp::TcpState::Closed => { break; }
            _ => {}
        }
        if i > 0 && i % 3000 == 0 { uart::putc(U, b'.'); }
        for _ in 0..100000 { spin_loop(); }
    }
    if !connected {
        uart::puts(U, "[FAIL] TCP timeout\r\n"); return;
    }
    uart::puts(U, "[OK] TCP connected\r\n");
    let mut req: [u8; 256] = [0; 256];
    let mut pos = 0;
    macro_rules! wr { ($s:expr) => { { let b = $s; req[pos..pos+b.len()].copy_from_slice(b); pos += b.len(); } } }
    wr!(b"GET ");
    wr!(path);
    wr!(b" HTTP/1.0\r\nHost: ");
    wr!(host);
    wr!(b"\r\n\r\n");
    if crate::tcp::tcp_send(&req[..pos]) {
        uart::puts(U, "[OK] HTTP request sent\r\n");
    } else {
        uart::puts(U, "[FAIL] Send failed\r\n"); return;
    }
    for _ in 0..50 {
        crate::tcp::poll();
        for _ in 0..200000 { spin_loop(); }
    }
    let mut total = 0usize;
    for i in 0..20000u32 {
        crate::tcp::poll();
        let mut buf = [0u8; 512];
        let n = crate::tcp::tcp_recv(&mut buf);
        if n > 0 {
            total += n;
            for &b in &buf[..n] {
                if b >= 0x20 && b < 0x7F {
                    uart::putc(U, b);
                } else if b == b'\n' {
                    uart::puts(U, "\r\n");
                } else if b == b'\r' {
                    // skip
                } else {
                    uart::putc(U, b'.');
                }
            }
        }
        let cstate = crate::tcp::tcp_connect_state();
        if cstate == crate::tcp::TcpState::Closed || cstate == crate::tcp::TcpState::None {
            break;
        }
        for _ in 0..100000 { spin_loop(); }
        if i > 0 && i % 10000 == 0 && total > 0 { break; }
    }
    uart::puts(U, "\r\n--- ");
    crate::print_dec_u32(U, total as u32);
    uart::puts(U, " bytes received ---\r\n");
}

// ─── Remote Shell (telnet) ─────────────────────────────────────────────────

fn cmd_telnet(port: u16) {
    if !crate::tcp::tcp_listen(port) {
        puts("[FAIL] listen\r\n");
        return;
    }
    puts("[..] Waiting for connection...\r\n");

    let mut connected = false;
    for _ in 0..30000u32 {
        crate::tcp::poll();
        match crate::tcp::tcp_listen_state() {
            crate::tcp::TcpState::Connected => { connected = true; break; }
            crate::tcp::TcpState::Closed => break,
            _ => {}
        }
        for _ in 0..50000 { spin_loop(); }
    }

    if !connected {
        puts("[FAIL] timeout\r\n");
        crate::tcp::tcp_close_listen();
        return;
    }

    puts("[OK] Remote shell connected\r\n");

    // Send banner over TCP
    crate::tcp::tcp_send_str_blocking(b"Aginx OS remote shell\r\n# ");

    set_output(OutputDest::Tcp);

    let mut editor = LineEditor::new();
    let mut tcp_buf = [0u8; 256];

    loop {
        crate::tcp::poll();

        let n = crate::tcp::tcp_recv(&mut tcp_buf);
        for &c in &tcp_buf[..n] {
            if editor.feed(c, OutputDest::Tcp) == InputAction::Complete {
                let line = editor.line();
                if line.len() > 0 {
                    execute_command(line);
                }
                editor.reset();
                set_output(OutputDest::Tcp);
                putc(b'#');
                putc(b' ');
            }
        }

        match crate::tcp::tcp_listen_state() {
            crate::tcp::TcpState::Closed | crate::tcp::TcpState::None => break,
            _ => {}
        }

        // Check UART for 'q' to quit
        if uart::has_data(U) {
            let qc = uart::getc(U);
            if qc == b'q' { break; }
        }

        for _ in 0..50000 { spin_loop(); }
    }

    set_output(OutputDest::Uart);
    crate::tcp::tcp_close();
    puts("# telnet session ended\r\n");
}

// ─── Agent Protocol Server ─────────────────────────────────────────────────

fn cmd_agent(port: u16) {
    if !crate::tcp::tcp_listen(port) {
        puts("[FAIL] listen\r\n");
        return;
    }
    puts("[..] Agent server waiting on :");
    print_dec(port as u32);
    puts("...\r\n");

    let mut connected = false;
    for _ in 0..30000u32 {
        crate::tcp::poll();
        match crate::tcp::tcp_listen_state() {
            crate::tcp::TcpState::Connected => { connected = true; break; }
            crate::tcp::TcpState::Closed => break,
            _ => {}
        }
        for _ in 0..50000 { spin_loop(); }
    }

    if !connected {
        puts("[FAIL] timeout\r\n");
        crate::tcp::tcp_close_listen();
        return;
    }

    puts("[OK] Agent client connected\r\n");

    let mut line_buf = [0u8; 256];
    let mut line_len = 0usize;
    let mut tcp_buf = [0u8; 256];

    loop {
        crate::tcp::poll();
        let n = crate::tcp::tcp_recv(&mut tcp_buf);

        for &c in &tcp_buf[..n] {
            if c == b'\n' || c == b'\r' {
                if line_len > 0 {
                    agent_handle_line(0, &line_buf[..line_len]);
                    line_len = 0;
                }
            } else if line_len < 255 {
                line_buf[line_len] = c;
                line_len += 1;
            }
        }

        match crate::tcp::tcp_listen_state() {
            crate::tcp::TcpState::Closed | crate::tcp::TcpState::None => break,
            _ => {}
        }

        if uart::has_data(U) {
            let qc = uart::getc(U);
            if qc == b'q' { break; }
        }

        for _ in 0..50000 { spin_loop(); }
    }

    crate::tcp::tcp_close();
    puts("# agent session ended\r\n");
}

fn agent_handle_line(slot: usize, line: &[u8]) {
    let send = |data: &[u8]| crate::tcp::tcp_slot_send_blocking(slot, data);

    if starts_with(line, b"exec ") {
        let cmd = skip_spaces(&line[5..]);
        set_output_to_slot(slot);
        execute_command(cmd);
        send(b"end\n");
        set_output(OutputDest::Uart);
    } else if starts_with(line, b"execcap ") {
        let cmd = skip_spaces(&line[8..]);
        capture_start();
        execute_command(cmd);
        let data = capture_end();
        send(b"status=ok\noutput=");
        send(data);
        send(b"\nend\n");
    } else if starts_with(line, b"send ") {
        let rest = skip_spaces(&line[5..]);
        let (target, msg_rest) = split_at_space(rest);
        let msg = skip_spaces(msg_rest);
        if target.is_empty() || msg.is_empty() {
            send(b"status=error usage: send <task> <msg>\n");
        } else {
            let target_str = match core::str::from_utf8(target) {
                Ok(s) => s,
                Err(_) => { send(b"status=error invalid target\n"); return; }
            };
            if crate::task::ipc_send(target_str, msg) {
                send(b"status=ok\n");
            } else {
                send(b"status=error target not found\n");
            }
        }
    } else if starts_with(line, b"upload ") {
        let rest = skip_spaces(&line[7..]);
        let (name, data_rest) = split_at_space(rest);
        let data = skip_spaces(data_rest);
        if !crate::fs::is_mounted() {
            send(b"status=error no fs\n");
        } else if name.is_empty() || data.is_empty() {
            send(b"status=error usage: upload <name> <data>\n");
        } else if crate::fs::create(name, data) {
            send(b"status=ok\n");
        } else {
            send(b"status=error\n");
        }
    } else if starts_with(line, b"download ") {
        let name = skip_spaces(&line[9..]);
        let mut buf = [0u8; 4096];
        match crate::fs::read(name, &mut buf) {
            Some(n) => {
                send(b"status=ok\ndata=");
                send(&buf[..n]);
                send(b"\nend\n");
            }
            None => {
                send(b"status=error not found\n");
            }
        }
    } else if eq(line, b"list") {
        send(b"status=ok\n");
        if crate::fs::is_mounted() {
            let files = crate::fs::list();
            for f in files {
                let name_len = f.name.iter().position(|&b| b == 0).unwrap_or(64);
                send(b"entry=");
                send(&f.name[..name_len]);
                send(b" ");
                let mut sbuf = [0u8; 10];
                let mut si = 0usize;
                let mut sz = f.size as u32;
                if sz == 0 { sbuf[0] = b'0'; si = 1; }
                else { while sz > 0 { sbuf[si] = b'0' + (sz % 10) as u8; sz /= 10; si += 1; } }
                let mut dec = [0u8; 10];
                for j in 0..si { dec[j] = sbuf[si - 1 - j]; }
                send(&dec[..si]);
                send(b"\n");
            }
        }
        send(b"end\n");
    } else if eq(line, b"status") {
        send(b"status=ok\n");
        if let Some(ip) = crate::tcp::get_ip() {
            send(b"ip=");
            let octets = ip.octets();
            for i in 0..4 {
                if i > 0 { send(b"."); }
                let mut sbuf = [0u8; 3];
                let mut si = 0;
                let mut v = octets[i] as u32;
                if v == 0 { sbuf[0] = b'0'; si = 1; }
                else { while v > 0 { sbuf[si] = b'0' + (v % 10) as u8; v /= 10; si += 1; } }
                let mut dec = [0u8; 3];
                for j in 0..si { dec[j] = sbuf[si - 1 - j]; }
                send(&dec[..si]);
            }
            send(b"\n");
        }
        let ticks = crate::interrupt::get_ticks();
        let secs = ticks as u32 / 100;
        send(b"uptime=");
        let mut sbuf = [0u8; 10];
        let mut si = 0usize;
        let mut v = secs;
        if v == 0 { sbuf[0] = b'0'; si = 1; }
        else { while v > 0 { sbuf[si] = b'0' + (v % 10) as u8; v /= 10; si += 1; } }
        let mut dec = [0u8; 10];
        for j in 0..si { dec[j] = sbuf[si - 1 - j]; }
        send(&dec[..si]);
        send(b"s\n");

        let free = crate::frame_alloc::free_count();
        send(b"mem_free=");
        let mut sbuf2 = [0u8; 10];
        let mut si2 = 0usize;
        let mut v2 = free as u32;
        if v2 == 0 { sbuf2[0] = b'0'; si2 = 1; }
        else { while v2 > 0 { sbuf2[si2] = b'0' + (v2 % 10) as u8; v2 /= 10; si2 += 1; } }
        let mut dec2 = [0u8; 10];
        for j in 0..si2 { dec2[j] = sbuf2[si2 - 1 - j]; }
        send(&dec2[..si2]);
        send(b" pages\nend\n");
    } else {
        send(b"status=error unknown command\n");
    }
}

// ─── Startup Script ─────────────────────────────────────────────────────────

pub fn run_autoexec() {
    if !crate::fs::is_mounted() { return; }

    let mut buf = [0u8; 4096];
    let n = match crate::fs::read(b"startup.cfg", &mut buf) {
        Some(n) if n > 0 => n,
        _ => return,
    };

    set_output(OutputDest::Uart);
    puts("[..] Running startup.cfg\r\n");

    let mut line_start = 0;
    for i in 0..=n {
        let c = if i < n { buf[i] } else { b'\n' };
        if c == b'\n' || c == b'\r' {
            if i > line_start {
                let line = &buf[line_start..i];
                // Skip comments and empty lines
                if !line.is_empty() && line[0] != b'#' {
                    execute_command(line);
                }
            }
            line_start = i + 1;
        }
    }
    puts("[OK] startup.cfg done\r\n");
}

// ─── Line Editor with History ───────────────────────────────────────────────

const LINE_BUF_SIZE: usize = 64;
const HISTORY_SIZE: usize = 8;

#[derive(Clone, Copy, PartialEq)]
pub enum InputAction {
    None,
    Complete,
}

pub struct LineEditor {
    buf: [u8; LINE_BUF_SIZE],
    len: usize,
    cursor: usize,
    history: [[u8; LINE_BUF_SIZE]; HISTORY_SIZE],
    history_len: [usize; HISTORY_SIZE],
    history_count: usize,
    history_pos: usize,
    escape_state: u8, // 0=normal, 1=ESC, 2=ESC[
}

impl LineEditor {
    pub fn new() -> Self {
        Self {
            buf: [0; LINE_BUF_SIZE],
            len: 0,
            cursor: 0,
            history: [[0; LINE_BUF_SIZE]; HISTORY_SIZE],
            history_len: [0; HISTORY_SIZE],
            history_count: 0,
            history_pos: 0,
            escape_state: 0,
        }
    }

    pub fn line(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn reset(&mut self) {
        self.len = 0;
        self.cursor = 0;
        self.escape_state = 0;
        self.history_pos = 0;
    }

    pub fn feed(&mut self, c: u8, dest: OutputDest) -> InputAction {
        if self.escape_state > 0 {
            return self.handle_escape(c, dest);
        }

        match c {
            b'\r' | b'\n' => {
                if self.len > 0 {
                    self.push_history();
                }
                putc_to(dest, b'\r');
                putc_to(dest, b'\n');
                return InputAction::Complete;
            }
            0x01 => {
                // Ctrl+A: move to beginning
                self.move_cursor_to(0, dest);
            }
            0x05 => {
                // Ctrl+E: move to end
                self.move_cursor_to(self.len, dest);
            }
            0x0B => {
                // Ctrl+K: kill to end of line
                let killed = self.len - self.cursor;
                if killed > 0 {
                    // Erase from cursor to end
                    for _ in 0..killed { putc_to(dest, b' '); }
                    for _ in 0..killed {
                        write_to(dest, b"\x1B[D");
                    }
                    self.len = self.cursor;
                }
            }
            0x08 | 0x7F => {
                // Backspace
                if self.cursor > 0 {
                    self.cursor -= 1;
                    // Shift buf left
                    for i in self.cursor..self.len - 1 {
                        self.buf[i] = self.buf[i + 1];
                    }
                    self.len -= 1;
                    // Move cursor back
                    putc_to(dest, 0x08);
                    // Redraw from cursor to end
                    for i in self.cursor..self.len {
                        putc_to(dest, self.buf[i]);
                    }
                    putc_to(dest, b' ');
                    // Move cursor back to correct position
                    let move_back = self.len - self.cursor + 1;
                    for _ in 0..move_back {
                        write_to(dest, b"\x1B[D");
                    }
                }
            }
            0x1B => {
                // ESC: start escape sequence
                self.escape_state = 1;
            }
            0x20..=0x7E => {
                if self.len < LINE_BUF_SIZE - 1 {
                    // Insert at cursor
                    for i in (self.cursor..self.len).rev() {
                        self.buf[i + 1] = self.buf[i];
                    }
                    self.buf[self.cursor] = c;
                    self.cursor += 1;
                    self.len += 1;
                    // Redraw from cursor-1 to end
                    for i in self.cursor - 1..self.len {
                        putc_to(dest, self.buf[i]);
                    }
                    // Move cursor back to correct position
                    for _ in self.cursor..self.len {
                        write_to(dest, b"\x1B[D");
                    }
                }
            }
            _ => {}
        }
        InputAction::None
    }

    fn handle_escape(&mut self, c: u8, dest: OutputDest) -> InputAction {
        match self.escape_state {
            1 => {
                if c == b'[' {
                    self.escape_state = 2;
                } else {
                    self.escape_state = 0;
                }
            }
            2 => {
                match c {
                    b'A' => {
                        // Up arrow
                        if self.history_count > 0 && self.history_pos < self.history_count {
                            self.history_pos += 1;
                            self.restore_from_history(dest);
                        }
                    }
                    b'B' => {
                        // Down arrow
                        if self.history_pos > 0 {
                            self.history_pos -= 1;
                            self.restore_from_history(dest);
                        } else if self.history_pos == 0 {
                            // Clear line (restore to blank)
                            self.clear_line(dest);
                            self.len = 0;
                            self.cursor = 0;
                        }
                    }
                    b'C' => {
                        // Right arrow: move cursor right
                        if self.cursor < self.len {
                            write_to(dest, b"\x1B[C");
                            self.cursor += 1;
                        }
                    }
                    b'D' => {
                        // Left arrow: move cursor left
                        if self.cursor > 0 {
                            write_to(dest, b"\x1B[D");
                            self.cursor -= 1;
                        }
                    }
                    _ => {}
                }
                self.escape_state = 0;
            }
            _ => { self.escape_state = 0; }
        }
        InputAction::None
    }

    fn move_cursor_to(&mut self, pos: usize, dest: OutputDest) {
        if pos == self.cursor { return; }
        if pos < self.cursor {
            for _ in pos..self.cursor { write_to(dest, b"\x1B[D"); }
        } else {
            for _ in self.cursor..pos { write_to(dest, b"\x1B[C"); }
        }
        self.cursor = pos;
    }

    fn clear_line(&mut self, dest: OutputDest) {
        // Move cursor to start of input, clear to end
        for _ in 0..self.cursor { write_to(dest, b"\x1B[D"); }
        for _ in 0..self.len { putc_to(dest, b' '); }
        for _ in 0..self.len { write_to(dest, b"\x1B[D"); }
    }

    fn restore_from_history(&mut self, dest: OutputDest) {
        self.clear_line(dest);
        if self.history_pos > 0 {
            let idx = (self.history_count - self.history_pos) % HISTORY_SIZE;
            self.len = self.history_len[idx];
            self.buf[..self.len].copy_from_slice(&self.history[idx][..self.len]);
        } else {
            self.len = 0;
        }
        self.cursor = self.len;
        for i in 0..self.len {
            putc_to(dest, self.buf[i]);
        }
    }

    fn push_history(&mut self) {
        if self.len == 0 { return; }
        // Check if same as last history entry
        if self.history_count > 0 {
            let last = (self.history_count - 1) % HISTORY_SIZE;
            if self.history_len[last] == self.len &&
               eq(&self.history[last][..self.len], &self.buf[..self.len]) {
                return;
            }
        }
        let idx = self.history_count % HISTORY_SIZE;
        self.history[idx][..self.len].copy_from_slice(&self.buf[..self.len]);
        self.history_len[idx] = self.len;
        self.history_count += 1;
    }
}
