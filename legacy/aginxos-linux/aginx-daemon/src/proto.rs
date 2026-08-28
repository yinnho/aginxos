//! Agent protocol — compatible with existing Aginx TCP protocol
//!
//! Line-based protocol for remote shell and agent communication.
//! Supports the same commands as the kernel shell, plus agent-specific ones.

use std::io::{self, BufRead, Write};
use std::net::TcpStream;

/// Handle an agent protocol connection.
/// This is used by the TCP server for network-based agent communication.
pub fn handle_agent_connection(stream: TcpStream) {
    let peer = stream.peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "?".into());

    eprintln!("[agent] Connection from {}", peer);

    let mut out = stream.try_clone().expect("clone");
    let reader = io::BufReader::new(stream);

    // Send banner
    writeln!(out, "Aginx Agent v{}", crate::VERSION).unwrap();
    out.flush().unwrap();

    for line in reader.lines() {
        match line {
            Ok(input) => {
                let trimmed = input.trim();
                if trimmed.is_empty() { continue; }

                // Check if it's an agent protocol command
                if trimmed.starts_with("upload ") || trimmed.starts_with("download ") ||
                   trimmed.starts_with("execcap ") || trimmed.starts_with("exec ") ||
                   trimmed.starts_with("send ") || trimmed.starts_with("list") ||
                   trimmed.starts_with("status") {
                    // Process as agent protocol
                    handle_agent_command(trimmed, &mut out);
                } else {
                    // Process as shell command
                    crate::shell::execute(trimmed, &mut out as &mut dyn Write);
                }

                writeln!(out).unwrap();
                out.flush().unwrap();
            }
            Err(_) => break,
        }
    }

    eprintln!("[agent] Disconnected from {}", peer);
}

fn handle_agent_command(cmd: &str, out: &mut dyn Write) {
    let parts: Vec<&str> = cmd.splitn(3, ' ').collect();
    match parts.first() {
        Some(&"upload") => {
            let name = parts.get(1).unwrap_or(&"");
            let data = parts.get(2).unwrap_or(&"");
            if name.is_empty() {
                writeln!(out, "status=error usage: upload <name> <data>").unwrap();
            } else {
                match std::fs::write(name, data.as_bytes()) {
                    Ok(()) => writeln!(out, "status=ok").unwrap(),
                    Err(e) => writeln!(out, "status=error {}", e).unwrap(),
                }
            }
        }
        Some(&"download") => {
            let name = parts.get(1).unwrap_or(&"");
            match std::fs::read_to_string(name) {
                Ok(data) => {
                    writeln!(out, "status=ok").unwrap();
                    writeln!(out, "data={}", data).unwrap();
                    writeln!(out, "end").unwrap();
                }
                Err(e) => writeln!(out, "status=error {}", e).unwrap(),
            }
        }
        Some(&"list") => {
            writeln!(out, "status=ok").unwrap();
            if let Ok(entries) = std::fs::read_dir(".") {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    writeln!(out, "entry={} {}", name, size).unwrap();
                }
            }
        }
        Some(&"status") => {
            writeln!(out, "status=ok").unwrap();
            writeln!(out, "arch={}", std::env::consts::ARCH).unwrap();
            writeln!(out, "version={}", crate::VERSION).unwrap();
        }
        _ => {
            crate::shell::execute(cmd, out);
        }
    }
}
