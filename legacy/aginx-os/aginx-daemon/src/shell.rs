//! Shell command processing
//!
//! Ported from kernel/src/shell.rs — same command structure,
//! but using std::fs, std::net, and Linux process APIs.

use std::fs;
use std::io::{self, BufRead, Write};
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
        "reboot" => cmd_reboot(),
        "halt" | "shutdown" => cmd_shutdown(),
        "exit" | "quit" => writeln!(out, "Bye.").unwrap(),
        _ => writeln!(out, "? unknown command: {}", name).unwrap(),
    }
    out.flush().unwrap();
}

// ── Commands ──────────────────────────────────────────────────────────────────

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

    // Read /proc/uptime for actual system uptime
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

fn cmd_uname(out: &mut dyn Write) {
    unsafe {
        let mut uts: libc::utsname = std::mem::zeroed();
        libc::uname(&mut uts);
        let to_str = |buf: &[u8; 65]| {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(64);
            String::from_utf8_lossy(&buf[..end]).into_owned()
        };
        writeln!(out, "{} {} {} {} {}",
            to_str(&uts.sysname),
            to_str(&uts.nodename),
            to_str(&uts.release),
            to_str(&uts.version),
            to_str(&uts.machine),
        ).unwrap();
    }
}

fn cmd_ps(out: &mut dyn Write) {
    // Read /proc for process list
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
                        // comm is wrapped in ()
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
    // Read network interfaces from /sys/class/net
    if let Ok(entries) = fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            let ifname = entry.file_name().to_string_lossy().into_owned();
            // Read address
            let addr_path = format!("/sys/class/net/{}/address", ifname);
            let mac = fs::read_to_string(&addr_path)
                .unwrap_or_default()
                .trim()
                .to_string();

            // Read operstate
            let state_path = format!("/sys/class/net/{}/operstate", ifname);
            let state = fs::read_to_string(&state_path)
                .unwrap_or_default()
                .trim()
                .to_string();

            writeln!(out, "{}: mac={} state={}", ifname, mac, state).unwrap();
        }
    }

    // Try to get IP addresses via /proc/net/fib_trie or just run ip addr
    if let Ok(output) = Command::new("ip").args(["addr", "show"]).output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("inet ") || line.contains("inet6 ") {
                writeln!(out, "  {}", line.trim()).unwrap();
            }
        }
    }
}

fn cmd_ping(args: &str, out: &mut dyn Write) {
    let host = args.split_whitespace().next().unwrap_or("");
    if host.is_empty() {
        writeln!(out, "Usage: ping <host>").unwrap();
        return;
    }
    writeln!(out, "[..] Pinging {} (3 packets)...", host).unwrap();
    match Command::new("ping")
        .args(["-c", "3", "-W", "2", host])
        .output()
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Print last 2 lines (summary)
            let lines: Vec<&str> = stdout.lines().collect();
            for line in lines.iter().rev().take(2) {
                writeln!(out, "  {}", line).unwrap();
            }
        }
        Err(e) => writeln!(out, "[FAIL] ping: {}", e).unwrap(),
    }
}

fn cmd_dns(args: &str, out: &mut dyn Write) {
    let domain = args.split_whitespace().next().unwrap_or("");
    if domain.is_empty() {
        writeln!(out, "Usage: dns <domain>").unwrap();
        return;
    }
    // Use getent or nslookup
    match Command::new("getent").args(["hosts", domain]).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.is_empty() {
                writeln!(out, "[FAIL] Not found").unwrap();
            } else {
                for line in stdout.lines() {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if let Some(ip) = parts.first() {
                        writeln!(out, "  {} -> {}", domain, ip).unwrap();
                    }
                }
            }
        }
        Err(e) => writeln!(out, "[FAIL] dns: {}", e).unwrap(),
    }
}

fn cmd_httpget(args: &str, out: &mut dyn Write) {
    let url = args.split_whitespace().next().unwrap_or("");
    if url.is_empty() {
        writeln!(out, "Usage: httpget <url>").unwrap();
        return;
    }
    writeln!(out, "[..] GET {}...", url).unwrap();
    match Command::new("curl").args(["-s", "-m", "10", url]).output() {
        Ok(output) => {
            let body = String::from_utf8_lossy(&output.stdout);
            let lines: Vec<&str> = body.lines().collect();
            for line in lines.iter().take(20) {
                writeln!(out, "  {}", line).unwrap();
            }
            if lines.len() > 20 {
                writeln!(out, "  ... ({} more lines)", lines.len() - 20).unwrap();
            }
        }
        Err(e) => writeln!(out, "[FAIL] httpget: {}", e).unwrap(),
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
            // Fallback: read /proc/net/tcp
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
    unsafe { libc::reboot(libc::LINUX_REBOOT_CMD_RESTART); }
}

fn cmd_shutdown() {
    unsafe { libc::reboot(libc::LINUX_REBOOT_CMD_POWER_OFF); }
}

// ── Interactive Shell ─────────────────────────────────────────────────────────

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
