use std::env;
use std::process::Command;

fn main() {
    let target = env::var("TARGET").unwrap_or_default();

    // Only compile entry.S for aarch64 targets
    if target.starts_with("aarch64") {
        let out_dir = env::var("OUT_DIR").unwrap();
        let obj = format!("{}/entry.o", out_dir);
        let archive = format!("{}/libentry.a", out_dir);

        // Compile entry.S to ELF object file using clang
        let status = Command::new("clang")
            .args(&[
                "-target", "aarch64-unknown-none",
                "-nostdlib",
                "-ffreestanding",
                "-c", "src/entry.S",
                "-o", &obj,
            ])
            .status()
            .expect("failed to execute clang");

        if !status.success() {
            panic!("clang failed to compile entry.S");
        }

        // macOS /usr/bin/ar+ranlib expect Mach-O and drop aarch64 ELF members.
        // Link the object directly.
        let _ = archive;
        println!("cargo:rustc-link-arg={}", obj);
    }

    // Tell Cargo to rerun if assembly changes
    println!("cargo:rerun-if-changed=src/entry.S");
}
