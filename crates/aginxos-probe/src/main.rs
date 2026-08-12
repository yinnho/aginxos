//! AginxOS bring-up probe.
//!
//! Cross-compile for the phone:
//!   cargo build -p aginxos-probe --release --target aarch64-unknown-linux-musl
//!
//! On-device (Android shell first, later AginxOS rootfs):
//!   adb push target/aarch64-unknown-linux-musl/release/aginxos-probe /data/local/tmp/
//!   adb shell chmod +x /data/local/tmp/aginxos-probe
//!   adb shell /data/local/tmp/aginxos-probe

use std::fs;
use std::io::{self, Write};
use std::path::Path;

fn main() {
    println!("AginxOS probe {}", env!("CARGO_PKG_VERSION"));
    println!("uid={}", unsafe { libc::getuid() });

    print_file("kernel", Path::new("/proc/version"));
    print_file("hostname", Path::new("/proc/sys/kernel/hostname"));
    list_dir("input", Path::new("/dev/input"));
    list_dir("dri", Path::new("/dev/dri"));

    let _ = io::stdout().flush();
}

fn print_file(label: &str, path: &Path) {
    match fs::read_to_string(path) {
        Ok(s) => print!("{label}: {s}"),
        Err(e) => println!("{label}: <unavailable: {e}>"),
    }
}

fn list_dir(label: &str, path: &Path) {
    match fs::read_dir(path) {
        Ok(entries) => {
            let mut names: Vec<String> = entries
                .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
                .collect();
            names.sort();
            if names.is_empty() {
                println!("{label}: (empty) {}", path.display());
            } else {
                println!("{label}: {} → {}", path.display(), names.join(", "));
            }
        }
        Err(e) => println!("{label}: <unavailable: {e}>"),
    }
}
