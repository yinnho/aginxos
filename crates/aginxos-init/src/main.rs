//! First userspace process in the AginxOS test boot.img.
//! Built static (musl) so it needs no shell/busybox in the ramdisk.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;

fn klog(msg: &str) {
    let line = format!("aginxos-init: {msg}\n");
    if let Ok(mut f) = OpenOptions::new().write(true).open("/dev/kmsg") {
        let _ = f.write_all(line.as_bytes());
    }
    // Also try console if present
    if let Ok(mut f) = OpenOptions::new().write(true).open("/dev/console") {
        let _ = f.write_all(line.as_bytes());
    }
}

fn mkdir_p(path: &str) {
    let _ = fs::create_dir_all(path);
}

fn mount(fstype: &str, source: &str, target: &str, flags: libc::c_ulong, data: &str) {
    mkdir_p(target);
    let src = std::ffi::CString::new(source).unwrap();
    let tgt = std::ffi::CString::new(target).unwrap();
    let fst = std::ffi::CString::new(fstype).unwrap();
    let dat = std::ffi::CString::new(data).unwrap();
    let rc = unsafe {
        libc::mount(
            src.as_ptr(),
            tgt.as_ptr(),
            fst.as_ptr(),
            flags,
            dat.as_ptr() as *const libc::c_void,
        )
    };
    if rc != 0 {
        klog(&format!(
            "mount {fstype} on {target} failed errno={}",
            std::io::Error::last_os_error()
        ));
    } else {
        klog(&format!("mounted {fstype} -> {target}"));
    }
}

fn main() {
    // Kernel may call us as pid 1.
    klog(&format!(
        "starting v{} pid={}",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    ));

    mount("proc", "proc", "/proc", 0, "");
    mount("sysfs", "sysfs", "/sys", 0, "");
    // devtmpfs preferred; fall back to tmpfs
    mount("devtmpfs", "devtmpfs", "/dev", 0, "mode=0755");
    if !Path::new("/dev/null").exists() {
        mount("tmpfs", "tmpfs", "/dev", 0, "mode=0755");
    }
    mkdir_p("/dev/pts");
    mount("devpts", "devpts", "/dev/pts", 0, "");
    mkdir_p("/tmp");
    mount("tmpfs", "tmpfs", "/tmp", 0, "");

    klog("filesystems ready");

    // List input/drm for bring-up breadcrumbs
    for dir in ["/dev/input", "/dev/dri"] {
        match fs::read_dir(dir) {
            Ok(rd) => {
                let names: Vec<_> = rd
                    .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
                    .collect();
                klog(&format!("{dir}: {}", names.join(",")));
            }
            Err(e) => klog(&format!("{dir}: {e}")),
        }
    }

    if Path::new("/bin/aginxos-probe").exists() {
        klog("running /bin/aginxos-probe");
        match Command::new("/bin/aginxos-probe").output() {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                for line in stdout.lines() {
                    klog(line);
                }
                let stderr = String::from_utf8_lossy(&out.stderr);
                for line in stderr.lines() {
                    klog(&format!("probe-err: {line}"));
                }
            }
            Err(e) => klog(&format!("probe spawn failed: {e}")),
        }
    } else {
        klog("no /bin/aginxos-probe");
    }

    klog("bring-up hold: sleeping (reboot phone to leave)");
    // Stay alive so kernel does not panic on init exit.
    loop {
        thread::sleep(Duration::from_secs(60));
        klog("heartbeat");
    }
}
