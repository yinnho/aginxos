//! TCP shell server — listens on a port and serves shell sessions
//!
//! Supports multiple concurrent connections via fork.
//! Each connection gets its own shell session.

use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU32, Ordering};

static CONNECTION_COUNT: AtomicU32 = AtomicU32::new(0);

// Peer connection for send command
static mut PEER_STREAM: Option<TcpStream> = None;

pub fn set_peer(stream: TcpStream) {
    unsafe {
        PEER_STREAM = Some(stream);
    }
}

pub fn get_peer() -> Option<TcpStream> {
    unsafe {
        PEER_STREAM.take()
    }
}

/// Start TCP server on the given port.
/// Blocks forever (or until error).
pub fn start(port: u16) -> std::io::Result<()> {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", port))?;
    eprintln!("[OK] Listening on 0.0.0.0:{}", port);

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let addr = stream.peer_addr().ok()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| "?".into());

                let count = CONNECTION_COUNT.fetch_add(1, Ordering::Relaxed);
                eprintln!("[TCP] Client #{} connected from {}", count + 1, addr);

                // Fork a child process for each connection
                unsafe {
                    match libc::fork() {
                        0 => {
                            // Child process — handle this connection
                            // Close the listener in child
                            drop(listener);
                            crate::shell::run_on_stream(stream);
                            libc::_exit(0);
                        }
                        -1 => {
                            eprintln!("[FAIL] fork failed");
                        }
                        _ => {
                            // Parent — close the stream, continue accepting
                            drop(stream);
                        }
                    }
                }

                // Reap zombies
                while unsafe { libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) } > 0 {}
            }
            Err(e) => {
                eprintln!("[FAIL] Accept: {}", e);
            }
        }
    }
    Ok(())
}
