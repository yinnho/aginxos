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

mod snd;
mod vidc;

fn main() {
    // bring-up subcommands (M41+); bare invocation keeps the classic probe dump
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        match args[1].as_str() {
            "vidc" => {
                let code = match vidc::run(&args[2..]) {
                    Ok(()) => 0,
                    Err(e) => {
                        eprintln!("vidc: {e}");
                        1
                    }
                };
                std::process::exit(code);
            }
            other => {
                eprintln!("unknown subcommand: {other} (try: vidc caps | vidc decode <in.h264> <out> [n] | vidc show <in.h264> | vidc play <in.h264> <in.s16> [vol] [w h fps] | vidc enc <in.yuv> <out.h264> <w> <h>)");
                std::process::exit(2);
            }
        }
    }

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
