//! Shell command processing
//!
//! Ported from kernel/src/shell.rs — same command structure,
//! but using std::fs, std::net, and Linux process APIs.

use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Execute a single command string
pub fn execute(input: &str, out: &mut dyn Write) {
    let cmd = input.trim();
    if cmd.is_empty() {
        return;
    }

    let parts: Vec<&str> = cmd.splitn(2, ' ').collect();
    let name = parts[0];
    let rest = parts.get(1).copied().unwrap_or("");

    match name {
        "help" => cmd_help(out),
        "version" => cmd_version(out),
        "uptime" => cmd_uptime(out),
        "mem" | "free" => cmd_mem(out),
        "ls" => cmd_ls(rest, out),
        "cat" => cmd_cat(rest, out),
        "pwd" => cmd_pwd(out),
        "cd" => cmd_cd(rest),
        "mkdir" => cmd_mkdir(rest, out),
        "rm" => cmd_rm(rest, out),
        "cp" => cmd_cp(rest, out),
        "mv" => cmd_mv(rest, out),
        "echo" => writeln!(out, "{}", rest).unwrap(),
        "clear" => write!(out, "\x1B[2J\x1B[H").unwrap(),
        "whoami" => cmd_whoami(out),
        "hostname" => cmd_hostname(out),
        "uname" => cmd_uname(out),
        "ps" => cmd_ps(out),
        "kill" => cmd_kill(rest, out),
        "exec" => cmd_exec(rest, out),
        "spawn" => cmd_spawn(rest, out),
        "tasks" => cmd_tasks(out),
        "ifconfig" | "ip" => cmd_ifconfig(out),
        "ping" => cmd_ping(rest, out),
        "dns" | "nslookup" => cmd_dns(rest, out),
        "httpget" | "curl" => cmd_httpget(rest, out),
        "telnet" => cmd_telnet(rest, out),
        "listen" => cmd_listen(rest, out),
        "send" => cmd_send(rest, out),
        "netstat" => cmd_netstat(out),
        "blkinfo" | "df" => cmd_df(out),
        "wifi_scan" => cmd_wifi_scan(out),
        "wifi_connect" => cmd_wifi_connect(rest, out),
        "wifi_disconnect" => cmd_wifi_disconnect(out),
        "wifi_status" => cmd_wifi_status(out),
        "cell_scan" | "cellular_scan" => cmd_cell_scan(out),
        "cell_connect" | "cellular_connect" => cmd_cell_connect(rest, out),
        "cell_disconnect" | "cellular_disconnect" => cmd_cell_disconnect(out),
        "cell_status" | "cellular_status" => cmd_cell_status(out),
        "reboot" => cmd_reboot(),
        "halt" | "shutdown" => cmd_shutdown(),
        "exit" | "quit" => writeln!(out, "Bye.").unwrap(),
        _ => writeln!(out, "? unknown command: {}", name).unwrap(),
    }
    out.flush().unwrap();
}

// -- Commands --------------------------------------------------------------------

fn cmd_help(out: &mut dyn Write) {
    writeln!(out, "Aginx Shell Commands:").unwrap();
    let cmds = [
        ("help",              "Show this help"),
        ("version",           "Show version"),
        ("uptime",            "System uptime"),
        ("mem / free",        "Memory info"),
        ("ls [path]",         "List directory"),
        ("cat <file>",        "Print file contents"),
        ("pwd",               "Print working directory"),
        ("cd <dir>",          "Change directory"),
        ("mkdir <dir>",       "Create directory"),
        ("rm <path>",         "Remove file/dir"),
        ("cp <src> <dst>",    "Copy file"),
        ("mv <src> <dst>",    "Move/rename file"),
        ("echo <text>",       "Print text"),
        ("whoami",            "Current user"),
        ("hostname",          "System hostname"),
        ("uname",             "System info"),
        ("ps",                "List processes"),
        ("kill <pid>",        "Kill process"),
        ("exec <cmd>",        "Execute external command"),
        ("spawn <cmd>",       "Spawn background process"),
        ("tasks",             "List spawned tasks"),
        ("ifconfig / ip",     "Network interfaces"),
        ("ping <host>",       "Ping host"),
        ("dns <domain>",      "DNS lookup"),
        ("httpget <url>",     "HTTP GET request"),
        ("telnet <host:port>","TCP connect"),
        ("listen <port>",     "Listen for TCP connections"),
        ("send <msg>",        "Send to connected peer"),
        ("netstat",           "Network connections"),
        ("wifi_scan",         "Scan WiFi networks"),
        ("wifi_connect <s> <p>", "Connect to WiFi"),
        ("wifi_disconnect",   "Disconnect WiFi"),
        ("wifi_status",       "WiFi status"),
        ("cell_scan",         "Scan cellular modems"),
        ("cell_connect <apn>", "Connect 4G (APN)"),
        ("cell_disconnect",   "Disconnect 4G"),
        ("cell_status",       "Cellular status"),
        ("df / blkinfo",      "Disk usage"),
        ("reboot",            "Reboot system"),
        ("halt",              "Shutdown system"),
        ("exit",              "Exit shell"),
    ];
    for (name, desc) in &cmds {
        writeln!(out, "  {:20} {}", name, desc).unwrap();
    }
}

fn cmd_version(out: &mut dyn Write) {
    writeln!(out, "Aginx Daemon v{} [{}]", crate::VERSION, std::env::consts::ARCH).unwrap();
}

fn cmd_uptime(out: &mut dyn Write) {
    let uptime = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    if let Ok(content) = fs::read_to_string("/proc/uptime") {
        let secs: f64 = content.split_whitespace().next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        let d = secs as u64 / 86400;
        let h = (secs as u64 % 86400) / 3600;
        let m = (secs as u64 % 3600) / 60;
        let s = secs as u64 % 60;
        if d > 0 {
            writeln!(out, "up {}d {:02}:{:02}:{:02}", d, h, m, s).unwrap();
        } else {
            writeln!(out, "up {:02}:{:02}:{:02}", h, m, s).unwrap();
        }
    } else {
        writeln!(out, "epoch: {}s", uptime).unwrap();
    }
}

fn cmd_mem(out: &mut dyn Write) {
    if let Ok(content) = fs::read_to_string("/proc/meminfo") {
        for line in content.lines().take(5) {
            writeln!(out, "  {}", line).unwrap();
        }
    }
}

fn cmd_ls(path: &str, out: &mut dyn Write) {
    let p = if path.is_empty() { "." } else { path };
    match fs::read_dir(p) {
        Ok(entries) => {
            let mut count = 0u32;
            let mut total_size: u64 = 0;
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let meta = entry.metadata().ok();
                let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                let kind = meta.as_ref().map(|m| {
                    if m.is_dir() { 'd' } else if m.is_symlink() { 'l' } else { '-' }
                }).unwrap_or('?');
                writeln!(out, "  {} {:>10} {}", kind, size, name).unwrap();
                count += 1;
                total_size += size;
            }
            writeln!(out, "  ({} entries, {} bytes)", count, total_size).unwrap();
        }
        Err(e) => writeln!(out, "[FAIL] ls: {}", e).unwrap(),
    }
}

fn cmd_cat(path: &str, out: &mut dyn Write) {
    if path.is_empty() {
        writeln!(out, "Usage: cat <file>").unwrap();
        return;
    }
    match fs::read_to_string(path) {
        Ok(content) => write!(out, "{}", content).unwrap(),
        Err(e) => writeln!(out, "[FAIL] cat: {}", e).unwrap(),
    }
}

fn cmd_pwd(out: &mut dyn Write) {
    match std::env::current_dir() {
        Ok(p) => writeln!(out, "{}", p.display()).unwrap(),
        Err(e) => writeln!(out, "[FAIL] {}", e).unwrap(),
    }
}

fn cmd_cd(path: &str) {
    let p = if path.is_empty() { "/" } else { path };
    if let Err(e) = std::env::set_current_dir(p) {
        eprintln!("[FAIL] cd: {}", e);
    }
}

fn cmd_mkdir(path: &str, out: &mut dyn Write) {
    if path.is_empty() {
        writeln!(out, "Usage: mkdir <dir>").unwrap();
        return;
    }
    match fs::create_dir_all(path) {
        Ok(()) => writeln!(out, "[OK] Created {}", path).unwrap(),
        Err(e) => writeln!(out, "[FAIL] mkdir: {}", e).unwrap(),
    }
}

fn cmd_rm(path: &str, out: &mut dyn Write) {
    if path.is_empty() {
        writeln!(out, "Usage: rm <path>").unwrap();
        return;
    }
    let result = if Path::new(path).is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    match result {
        Ok(()) => writeln!(out, "[OK] Removed {}", path).unwrap(),
        Err(e) => writeln!(out, "[FAIL] rm: {}", e).unwrap(),
    }
}

fn cmd_cp(args: &str, out: &mut dyn Write) {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 2 {
        writeln!(out, "Usage: cp <src> <dst>").unwrap();
        return;
    }
    match fs::copy(parts[0], parts[1]) {
        Ok(n) => writeln!(out, "[OK] Copied {} bytes", n).unwrap(),
        Err(e) => writeln!(out, "[FAIL] cp: {}", e).unwrap(),
    }
}

fn cmd_mv(args: &str, out: &mut dyn Write) {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 2 {
        writeln!(out, "Usage: mv <src> <dst>").unwrap();
        return;
    }
    match fs::rename(parts[0], parts[1]) {
        Ok(()) => writeln!(out, "[OK] Moved").unwrap(),
        Err(e) => writeln!(out, "[FAIL] mv: {}", e).unwrap(),
    }
}

fn cmd_whoami(out: &mut dyn Write) {
    match std::env::var("USER") {
        Ok(u) => writeln!(out, "{}", u).unwrap(),
        Err(_) => {
            unsafe {
                let uid = libc::getuid();
                writeln!(out, "uid={}", uid).unwrap();
            }
        }
    }
}

fn cmd_hostname(out: &mut dyn Write) {
    let name = std::fs::read_to_string("/etc/hostname")
        .unwrap_or_else(|_| "localhost".into());
    writeln!(out, "{}", name.trim()).unwrap();
}

fn utsname_to_string(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

fn cmd_uname(out: &mut dyn Write) {
    unsafe {
        let mut uts: libc::utsname = std::mem::zeroed();
        libc::uname(&mut uts);
        // Convert fields to byte slices regardless of platform type (macOS: [i8; 256], Linux: [u8; 65])
        let sysname = utsname_to_string(unsafe { &*(&uts.sysname as *const _ as *const [u8; 256]) });
        let nodename = utsname_to_string(unsafe { &*(&uts.nodename as *const _ as *const [u8; 256]) });
        let release = utsname_to_string(unsafe { &*(&uts.release as *const _ as *const [u8; 256]) });
        let version = utsname_to_string(unsafe { &*(&uts.version as *const _ as *const [u8; 256]) });
        let machine = utsname_to_string(unsafe { &*(&uts.machine as *const _ as *const [u8; 256]) });
        writeln!(out, "{} {} {} {} {}", sysname, nodename, release, version, machine).unwrap();
    }
}

fn cmd_ps(out: &mut dyn Write) {
    if let Ok(entries) = fs::read_dir("/proc") {
        writeln!(out, "{:>6} {:>6} {}", "PID", "STAT", "CMD").unwrap();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Ok(pid) = name.parse::<u32>() {
                let stat_path = format!("/proc/{}/stat", pid);
                if let Ok(stat) = fs::read_to_string(&stat_path) {
                    let fields: Vec<&str> = stat.split_whitespace().collect();
                    if fields.len() >= 2 {
                        let comm = fields.get(1).unwrap_or(&"?");
                        let state = fields.get(2).unwrap_or(&"?");
                        let comm = comm.trim_matches('(').trim_matches(')');
                        writeln!(out, "{:>6} {:>6} {}", pid, state, comm).unwrap();
                    }
                }
            }
        }
    }
}

fn cmd_kill(args: &str, out: &mut dyn Write) {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.is_empty() {
        writeln!(out, "Usage: kill <pid> [signal]").unwrap();
        return;
    }
    if let Ok(pid) = parts[0].parse::<i32>() {
        let sig = parts.get(1).and_then(|s| s.parse::<i32>().ok()).unwrap_or(15);
        unsafe {
            let ret = libc::kill(pid, sig);
            if ret == 0 {
                writeln!(out, "[OK] Sent signal {} to {}", sig, pid).unwrap();
            } else {
                writeln!(out, "[FAIL] kill: {}", io::Error::last_os_error()).unwrap();
            }
        }
    }
}

fn cmd_exec(args: &str, out: &mut dyn Write) {
    if args.is_empty() {
        writeln!(out, "Usage: exec <command>").unwrap();
        return;
    }
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.is_empty() { return; }
    match Command::new(parts[0])
        .args(&parts[1..])
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
    {
        Ok(status) => {
            writeln!(out, "[exit {}]", status.code().unwrap_or(-1)).unwrap();
        }
        Err(e) => writeln!(out, "[FAIL] {}", e).unwrap(),
    }
}

fn cmd_spawn(args: &str, out: &mut dyn Write) {
    if args.is_empty() {
        writeln!(out, "Usage: spawn <command>").unwrap();
        return;
    }
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.is_empty() { return; }
    match Command::new(parts[0])
        .args(&parts[1..])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => {
            crate::task::register(child.id(), args);
            writeln!(out, "[OK] Spawned PID {}", child.id()).unwrap();
        }
        Err(e) => writeln!(out, "[FAIL] {}", e).unwrap(),
    }
}

fn cmd_tasks(out: &mut dyn Write) {
    let tasks = crate::task::list();
    if tasks.is_empty() {
        writeln!(out, "No spawned tasks").unwrap();
    } else {
        writeln!(out, "{:>6} {}", "PID", "CMD").unwrap();
        for (pid, cmd) in &tasks {
            writeln!(out, "{:>6} {}", pid, cmd).unwrap();
        }
    }
}

fn cmd_ifconfig(out: &mut dyn Write) {
    if let Ok(entries) = fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            let ifname = entry.file_name().to_string_lossy().into_owned();
            let mac = fs::read_to_string(format!("/sys/class/net/{}/address", ifname))
                .unwrap_or_default().trim().to_string();
            let state = fs::read_to_string(format!("/sys/class/net/{}/operstate", ifname))
                .unwrap_or_default().trim().to_string();
            let mtu = fs::read_to_string(format!("/sys/class/net/{}/mtu", ifname))
                .unwrap_or_default().trim().to_string();

            write!(out, "{}: mac={} state={} mtu={}", ifname, mac, state, mtu).unwrap();

            // Read IP from sysfs (IPv4 only)
            if let Ok(addrs) = fs::read_dir(format!("/sys/class/net/{}/brport", ifname)) {
                let _ = addrs; // ignore
            }
            // Try to get IP from ioctl
            #[cfg(target_os = "linux")]
            {
                let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
                if fd >= 0 {
                    #[repr(C)]
                    struct Ifreq { name: [u8; 16], addr: libc::sockaddr_in }
                    let mut ifr: Ifreq = unsafe { std::mem::zeroed() };
                    let b = ifname.as_bytes();
                    ifr.name[..b.len()].copy_from_slice(b);
                    unsafe {
                        if libc::ioctl(fd, libc::SIOCGIFADDR as _, &mut ifr) == 0 {
                            let ip = u32::from_be(ifr.addr.sin_addr.s_addr);
                            write!(out, " ip={}.{}.{}.{}", (ip >> 24) & 0xff, (ip >> 16) & 0xff, (ip >> 8) & 0xff, ip & 0xff).unwrap();
                        }
                    }
                    unsafe { libc::close(fd); }
                }
            }
            writeln!(out).unwrap();
        }
    }
}

fn cmd_ping(args: &str, out: &mut dyn Write) {
    let host = args.split_whitespace().next().unwrap_or("");
    if host.is_empty() {
        writeln!(out, "Usage: ping <host>").unwrap();
        return;
    }

    // Resolve hostname to IP
    let ip_str = match std::net::ToSocketAddrs::to_socket_addrs(
        &format!("{}:0", host)[..]
    ) {
        Ok(mut addrs) => match addrs.next() {
            Some(addr) => addr.ip().to_string(),
            None => {
                writeln!(out, "[FAIL] Cannot resolve {}", host).unwrap();
                return;
            }
        },
        Err(e) => {
            writeln!(out, "[FAIL] resolve: {}", e).unwrap();
            return;
        }
    };

    // Try TCP connect to common ports as connectivity check (works with NAT/QEMU)
    writeln!(out, "[..] {} -> {}", host, ip_str).unwrap();
    let ports = [80, 443, 22];
    let mut reachable = false;
    for &port in &ports {
        match TcpStream::connect_timeout(
            &format!("{}:{}", ip_str, port).parse().unwrap(),
            std::time::Duration::from_secs(2),
        ) {
            Ok(_) => {
                writeln!(out, "[OK] {}:{} reachable", ip_str, port).unwrap();
                reachable = true;
                break;
            }
            Err(_) => continue,
        }
    }
    if !reachable {
        // At least DNS resolved - report that
        writeln!(out, "[INFO] {} resolved to {} but no TCP port responded", host, ip_str).unwrap();
    }
}

fn getpid() -> u16 { unsafe { libc::getpid() as u16 } }

fn cmd_dns(args: &str, out: &mut dyn Write) {
    let domain = args.split_whitespace().next().unwrap_or("");
    if domain.is_empty() {
        writeln!(out, "Usage: dns <domain>").unwrap();
        return;
    }

    // Use QEMU user-mode DNS server at 10.0.2.3
    let dns_server = std::env::var("DNS_SERVER").unwrap_or_else(|_| "10.0.2.3".into());

    // Build a simple DNS A record query
    let mut query = vec![0u8; 12];
    // Transaction ID
    query[0] = 0x12;
    query[1] = 0x34;
    // Flags: standard query
    query[2] = 0x01;
    query[3] = 0x00;
    // Questions: 1
    query[5] = 0x01;

    // Encode domain name
    for label in domain.split('.') {
        let bytes = label.as_bytes();
        query.push(bytes.len() as u8);
        query.extend_from_slice(bytes);
    }
    query.push(0); // root label
    // Type A = 1, Class IN = 1
    query.push(0); query.push(1);
    query.push(0); query.push(1);

    // Send via UDP
    match std::net::UdpSocket::bind("0.0.0.0:0") {
        Ok(sock) => {
            sock.set_read_timeout(Some(std::time::Duration::from_secs(3))).ok();
            match sock.send_to(&query, format!("{}:53", dns_server)) {
                Ok(_) => {
                    let mut buf = [0u8; 512];
                    match sock.recv_from(&mut buf) {
                        Ok((n, _)) => {
                            // Parse response
                            if n < 12 { return writeln!(out, "[FAIL] Bad response").unwrap(); }
                            let answers = ((buf[6] as u16) << 8) | (buf[7] as u16);
                            // Skip question section
                            let mut pos = 12;
                            while pos < n && buf[pos] != 0 {
                                pos += buf[pos] as usize + 1;
                            }
                            pos += 5; // null byte + type(2) + class(2)

                            if answers == 0 {
                                writeln!(out, "[FAIL] No answers for {}", domain).unwrap();
                            }
                            let mut found = 0;
                            for _ in 0..answers {
                                if pos + 12 > n { break; }
                                // Skip name (may be pointer)
                                if buf[pos] & 0xc0 == 0xc0 { pos += 2; }
                                else { while pos < n && buf[pos] != 0 { pos += buf[pos] as usize + 1; } pos += 1; }
                                if pos + 10 > n { break; }
                                let rtype = ((buf[pos] as u16) << 8) | (buf[pos+1] as u16);
                                let rdlen = ((buf[pos+8] as u16) << 8) | (buf[pos+9] as u16);
                                pos += 10;
                                if rtype == 1 && rdlen == 4 && pos + 4 <= n {
                                    let ip = format!("{}.{}.{}.{}", buf[pos], buf[pos+1], buf[pos+2], buf[pos+3]);
                                    writeln!(out, "  {} -> {}", domain, ip).unwrap();
                                    found += 1;
                                }
                                pos += rdlen as usize;
                            }
                            if found == 0 {
                                writeln!(out, "[FAIL] No A records for {}", domain).unwrap();
                            }
                        }
                        Err(e) => writeln!(out, "[FAIL] recv: {}", e).unwrap(),
                    }
                }
                Err(e) => writeln!(out, "[FAIL] send: {}", e).unwrap(),
            }
        }
        Err(e) => writeln!(out, "[FAIL] socket: {}", e).unwrap(),
    }
}

fn cmd_httpget(args: &str, out: &mut dyn Write) {
    let url = args.split_whitespace().next().unwrap_or("");
    if url.is_empty() {
        writeln!(out, "Usage: httpget <url>").unwrap();
        return;
    }
    writeln!(out, "[..] GET {}...", url).unwrap();

    // Parse URL: http://host:port/path
    let url_str = url.strip_prefix("http://").unwrap_or(url);
    let (host_port, path) = match url_str.find('/') {
        Some(i) => (&url_str[..i], &url_str[i..]),
        None => (url_str, "/"),
    };
    let (host, port) = match host_port.rfind(':') {
        Some(i) => (&host_port[..i], host_port[i+1..].parse::<u16>().unwrap_or(80)),
        None => (host_port, 80u16),
    };

    let addr = format!("{}:{}", host, port);
    let sock_addr = match std::net::ToSocketAddrs::to_socket_addrs(&addr[..]) {
        Ok(mut addrs) => match addrs.next() {
            Some(a) => a,
            None => { writeln!(out, "[FAIL] Cannot resolve {}", host).unwrap(); return; }
        },
        Err(e) => { writeln!(out, "[FAIL] resolve: {}", e).unwrap(); return; }
    };
    match TcpStream::connect_timeout(&sock_addr, std::time::Duration::from_secs(5)) {
        Ok(mut stream) => {
            stream.set_read_timeout(Some(std::time::Duration::from_secs(10))).ok();
            let request = format!("GET {} HTTP/1.0\r\nHost: {}\r\nConnection: close\r\n\r\n", path, host);
            if let Err(e) = stream.write_all(request.as_bytes()) {
                writeln!(out, "[FAIL] write: {}", e).unwrap();
                return;
            }
            let mut response = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                match stream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => response.extend_from_slice(&buf[..n]),
                    Err(_) => break,
                }
            }
            let body = String::from_utf8_lossy(&response);
            let mut in_headers = true;
            let mut line_count = 0;
            for line in body.lines() {
                if in_headers {
                    if line.is_empty() {
                        in_headers = false;
                        writeln!(out, "---").unwrap();
                    } else if line_count < 5 {
                        writeln!(out, "  {}", line).unwrap();
                    }
                    continue;
                }
                if line_count < 20 {
                    writeln!(out, "  {}", line).unwrap();
                    line_count += 1;
                }
            }
            if line_count >= 20 {
                writeln!(out, "  ... (truncated)").unwrap();
            }
        }
        Err(e) => writeln!(out, "[FAIL] connect: {}", e).unwrap(),
    }
}

fn cmd_telnet(args: &str, out: &mut dyn Write) {
    let target = args.split_whitespace().next().unwrap_or("");
    if target.is_empty() {
        writeln!(out, "Usage: telnet <host:port>").unwrap();
        return;
    }
    match TcpStream::connect(target) {
        Ok(stream) => {
            writeln!(out, "[OK] Connected to {}", target).unwrap();
            crate::tcp_server::set_peer(stream);
        }
        Err(e) => writeln!(out, "[FAIL] connect: {}", e).unwrap(),
    }
}

fn cmd_listen(args: &str, out: &mut dyn Write) {
    let port: u16 = args.split_whitespace().next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(9091);
    writeln!(out, "[..] Listening on {}...", port).unwrap();
    match crate::tcp_server::start(port) {
        Ok(()) => {}
        Err(e) => writeln!(out, "[FAIL] listen: {}", e).unwrap(),
    }
}

fn cmd_send(args: &str, out: &mut dyn Write) {
    if let Some(mut stream) = crate::tcp_server::get_peer() {
        let msg = if !args.is_empty() {
            format!("{}\n", args)
        } else {
            return writeln!(out, "Usage: send <message>").unwrap();
        };
        match stream.write_all(msg.as_bytes()) {
            Ok(()) => writeln!(out, "[OK] Sent {} bytes", msg.len()).unwrap(),
            Err(e) => writeln!(out, "[FAIL] send: {}", e).unwrap(),
        }
        crate::tcp_server::set_peer(stream);
    } else {
        writeln!(out, "[FAIL] Not connected").unwrap();
    }
}

fn cmd_netstat(out: &mut dyn Write) {
    match Command::new("ss").args(["-tlnp"]).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            write!(out, "{}", stdout).unwrap();
        }
        Err(_) => {
            if let Ok(content) = fs::read_to_string("/proc/net/tcp") {
                write!(out, "{}", content).unwrap();
            }
        }
    }
}

fn cmd_df(out: &mut dyn Write) {
    match Command::new("df").args(["-h"]).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            write!(out, "{}", stdout).unwrap();
        }
        Err(e) => writeln!(out, "[FAIL] df: {}", e).unwrap(),
    }
}

fn cmd_reboot() {
    #[cfg(target_os = "linux")]
    unsafe { libc::reboot(libc::LINUX_REBOOT_CMD_RESTART); }
    #[cfg(not(target_os = "linux"))]
    eprintln!("[FAIL] reboot only supported on Linux");
}

fn cmd_shutdown() {
    #[cfg(target_os = "linux")]
    unsafe { libc::reboot(libc::LINUX_REBOOT_CMD_POWER_OFF); }
    #[cfg(not(target_os = "linux"))]
    eprintln!("[FAIL] shutdown only supported on Linux");
}

fn cmd_wifi_scan(out: &mut dyn Write) {
    crate::wifi::scan(out);
}

fn cmd_wifi_connect(args: &str, out: &mut dyn Write) {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 2 {
        writeln!(out, "Usage: wifi_connect <ssid> <password>").unwrap();
        return;
    }
    crate::wifi::connect(parts[0], parts[1], out);
}

fn cmd_wifi_disconnect(out: &mut dyn Write) {
    crate::wifi::disconnect(out);
}

fn cmd_wifi_status(out: &mut dyn Write) {
    crate::wifi::status(out);
}

fn cmd_cell_scan(out: &mut dyn Write) {
    crate::cellular::scan(out);
}

fn cmd_cell_connect(args: &str, out: &mut dyn Write) {
    let apn = args.split_whitespace().next().unwrap_or("");
    if apn.is_empty() {
        writeln!(out, "Usage: cell_connect <apn>").unwrap();
        writeln!(out, "Common APNs:").unwrap();
        writeln!(out, "  China Mobile: cmnet").unwrap();
        writeln!(out, "  China Unicom: 3gnet").unwrap();
        writeln!(out, "  China Telecom: ctnet").unwrap();
        return;
    }
    crate::cellular::connect(apn, out);
}

fn cmd_cell_disconnect(out: &mut dyn Write) {
    crate::cellular::disconnect(out);
}

fn cmd_cell_status(out: &mut dyn Write) {
    crate::cellular::status(out);
}

// -- Interactive Shell ------------------------------------------------------------

/// Run interactive shell on stdin/stdout
pub fn run_interactive() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    write!(out, "# ").unwrap();
    out.flush().unwrap();

    for line in stdin.lock().lines() {
        match line {
            Ok(input) => {
                if input == "exit" || input == "quit" {
                    writeln!(out, "Bye.").unwrap();
                    return;
                }
                execute(&input, &mut out);
            }
            Err(_) => break,
        }
        write!(out, "# ").unwrap();
        out.flush().unwrap();
    }
}

/// Run shell on a TCP stream
pub fn run_on_stream(stream: TcpStream) {
    let mut out = stream.try_clone().expect("clone stream");
    let reader = io::BufReader::new(stream);

    writeln!(out, "Aginx Daemon v{} [{}]", crate::VERSION, std::env::consts::ARCH).unwrap();
    write!(out, "# ").unwrap();
    out.flush().unwrap();

    for line in reader.lines() {
        match line {
            Ok(input) => {
                if input == "exit" || input == "quit" {
                    writeln!(out, "Bye.").unwrap();
                    return;
                }
                execute(&input, &mut out);
            }
            Err(_) => return,
        }
        write!(out, "# ").unwrap();
        out.flush().unwrap();
    }
}
