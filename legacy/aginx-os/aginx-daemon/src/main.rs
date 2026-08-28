//! Aginx Daemon — Linux userspace agent runtime
//!
//! Runs as PID 1 (init) or a regular daemon on any Linux system.
//! Supports: aarch64, x86_64, and any Linux target.
//!
//! Usage:
//!   ./aginx-daemon                    # interactive shell on stdin
//!   ./aginx-daemon --listen 9091      # TCP shell server
//!   ./aginx-daemon --listen 9091 --shell  # both

mod shell;
mod tcp_server;
mod task;
mod proto;

use std::env;
use std::io::{self, Write};

const VERSION: &str = "0.1.0";
const DEFAULT_PORT: u16 = 9091;

fn print_banner() {
    println!("Aginx Daemon v{} [{}]", VERSION, env::consts::ARCH);
    println!("  Linux {} ({})", sys_info().0, sys_info().1);
}

/// Get (kernel_version, hostname)
fn sys_info() -> (String, String) {
    let kernel = unsafe {
        let mut uts: libc::utsname = std::mem::zeroed();
        libc::uname(&mut uts);
        // uts.release is [u8; 65]
        let end = uts.release.iter().position(|&b| b == 0).unwrap_or(64);
        String::from_utf8_lossy(&uts.release[..end]).into_owned()
    };
    let hostname = std::fs::read_to_string("/etc/hostname")
        .unwrap_or_else(|_| "localhost".into())
        .trim()
        .to_string();
    (kernel, hostname)
}

fn usage() {
    eprintln!("Usage: aginx-daemon [OPTIONS]");
    eprintln!("  --listen PORT   Start TCP shell server (default: {})", DEFAULT_PORT);
    eprintln!("  --shell         Interactive stdin shell");
    eprintln!("  --daemon        Run as background daemon");
    eprintln!("  --help          Show this help");
}

fn main() {
    let args: Vec<String> = env::args().collect();

    let mut listen_port: Option<u16> = None;
    let mut interactive = false;
    let mut daemon_mode = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--listen" => {
                i += 1;
                listen_port = Some(args[i].parse().unwrap_or(DEFAULT_PORT));
            }
            "--shell" => interactive = true,
            "--daemon" => daemon_mode = true,
            "--help" | "-h" => { usage(); return; }
            _ => {}
        }
        i += 1;
    }

    // Default: both interactive and TCP if nothing specified
    if !interactive && listen_port.is_none() {
        interactive = true;
    }

    if daemon_mode {
        // Simple daemonize: fork, setsid, close stdio
        unsafe {
            match libc::fork() {
                0 => {
                    libc::setsid();
                    // Redirect stdio to /dev/null
                    let devnull = libc::open(b"/dev/null\0".as_ptr() as *const i8,
                        libc::O_RDWR, 0);
                    libc::dup2(devnull, 0);
                    libc::dup2(devnull, 1);
                    libc::dup2(devnull, 2);
                    if devnull > 2 { libc::close(devnull); }
                }
                _ => return,
            }
        }
    }

    print_banner();

    // Start TCP server if requested
    if let Some(port) = listen_port {
        match tcp_server::start(port) {
            Ok(_) => {}
            Err(e) => eprintln!("[FAIL] TCP listen on {}: {}", port, e),
        }
        return; // TCP server handles everything
    }

    // Interactive shell
    if interactive {
        shell::run_interactive();
    }
}
