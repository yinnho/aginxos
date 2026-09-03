//! agsecretd — the secret sidecar daemon (M36; SYSTEM.md §7.1).
//!
//! Flags exist for tests and the adb dev loop; production runs bare from
//! its agsvc unit with the default paths. Exits nonzero only on startup
//! problems (bind/store) — the accept loop serves until killed.

use std::path::PathBuf;

fn main() {
    let mut sock = PathBuf::from(agsecret::DEFAULT_SOCKET);
    let mut store = PathBuf::from(agsecret::DEFAULT_STORE);
    let mut policy = PathBuf::from(agsecret::DEFAULT_POLICY);
    let mut log = PathBuf::from(agsecret::DEFAULT_LOG);

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--socket" => sock = PathBuf::from(args.next().unwrap_or_default()),
            "--store" => store = PathBuf::from(args.next().unwrap_or_default()),
            "--policy" => policy = PathBuf::from(args.next().unwrap_or_default()),
            "--log" => log = PathBuf::from(args.next().unwrap_or_default()),
            "--help" | "-h" => {
                println!("usage: agsecretd [--socket P] [--store P] [--policy P] [--log P]");
                return;
            }
            other => {
                eprintln!("agsecretd: unknown flag {other}");
                std::process::exit(2);
            }
        }
    }

    if let Err(e) = agsecret::serve::serve(&sock, &store, &policy, &log) {
        eprintln!("agsecretd: {e}");
        std::process::exit(1);
    }
}
