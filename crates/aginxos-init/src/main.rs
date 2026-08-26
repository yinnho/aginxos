//! AginxOS early init via vendor_boot `rdinit=/aginxos/aginxos-init`.
//!
//! Feature flags (empty files in ramdisk):
//!   /aginxos/hold         — do not hand off to Android
//!   /aginxos/load-modules — load /lib/modules/modules.load
//!   /aginxos/splash       — attempt DRM solid-color frames
//!   /aginxos/storage      — load the UFS chain, create block nodes
//!
//! Operator subcommand: `/aginxos/aginxos-init reboot [mode]`.
//! Default with only `hold`: mount basics + heartbeat (safest bring-up).

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

fn klog(msg: &str) {
    let line = format!("aginxos-init: {msg}\n");
    let _ = std::io::stdout().write_all(line.as_bytes());
    let _ = std::io::stdout().flush();
    if let Ok(mut f) = OpenOptions::new().write(true).open("/dev/kmsg") {
        let _ = f.write_all(line.as_bytes());
    }
}

fn flag(path: &str) -> bool {
    Path::new(path).exists()
}

fn mkdir_p(path: &str) {
    let _ = fs::create_dir_all(path);
}

fn mount(fstype: &str, source: &str, target: &str, data: &str) {
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
            0,
            dat.as_ptr() as *const libc::c_void,
        )
    };
    if rc != 0 {
        klog(&format!(
            "mount {fstype}->{target}: {}",
            std::io::Error::last_os_error()
        ));
    } else {
        klog(&format!("mounted {fstype}->{target}"));
    }
}

fn ensure_basics() {
    if !Path::new("/proc/self").exists() {
        mount("proc", "proc", "/proc", "");
    }
    if !Path::new("/sys/class").exists() {
        mount("sysfs", "sysfs", "/sys", "");
    }
    if !Path::new("/dev/null").exists() {
        mount("devtmpfs", "devtmpfs", "/dev", "mode=0755");
        if !Path::new("/dev/null").exists() {
            mount("tmpfs", "tmpfs", "/dev", "mode=0755");
        }
    }
}

fn load_module(path: &Path) -> Result<(), String> {
    let f = File::open(path).map_err(|e| format!("open: {e}"))?;
    let fd = f.as_raw_fd();
    let params = std::ffi::CString::new("").unwrap();
    // 438 on aarch64 — the previously hardcoded 313 is x86_64 and cost a full
    // flash cycle to discover (ENOSYS from every storage module, 2026-08-27).
    let rc = unsafe { libc::syscall(libc::SYS_finit_module, fd, params.as_ptr(), 0i32) };
    if rc == 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::EEXIST) {
        Ok(())
    } else {
        Err(format!("{err}"))
    }
}

/// Load a small allow-list only (not full modules.load — that panics on redfin).
fn load_safe_modules() {
    // Empty for now: full modules.load caused bootloop.
    // Next iteration: add one module at a time after minimal HOLD is proven.
    let allow: &[&str] = &[];
    let base = Path::new("/lib/modules");
    let mut ok = 0usize;
    for name in allow {
        let p = base.join(name);
        match load_module(&p) {
            Ok(()) => {
                ok += 1;
                klog(&format!("mod ok {name}"));
            }
            Err(e) => klog(&format!("mod fail {name}: {e}")),
        }
    }
    klog(&format!("safe modules done ok={ok}"));
}

// --- storage bring-up (/aginxos/storage) ---

/// UFS chain, live-verified 2026-08-27 (see docs/HARDWARE.md). Order from the
/// modules' modinfo depends=; the reboot-reason chain (qcom_hwspinlock etc.)
/// already ran in the trampoline's modules.usb before this PID 1 takeover.
const UFS_MODULES: &[&str] = &[
    "phy-qcom-ufs.ko",
    "phy-qcom-ufs-qmp-v4.ko",
    "phy-qcom-ufs-qmp-v4-lito.ko", // lito = SM7250 = this SoC
    "ufshcd-core.ko",
    "ufshcd-pltfrm.ko",
    "ufs_qcom.ko",
];

/// glibc-compatible makedev (new large encoding) — the libc crate does not
/// export one we can rely on.
fn makedev(ma: u32, mi: u32) -> libc::dev_t {
    (((mi & 0xff) | ((ma & 0xfff) << 8)) as libc::dev_t)
        | (((mi & !0xff) as libc::dev_t) << 12)
        | (((ma & !0xfff) as libc::dev_t) << 32)
}

fn partitions_have(disk: &str) -> bool {
    let Ok(content) = fs::read_to_string("/proc/partitions") else {
        return false;
    };
    content.lines().any(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        f.len() == 4 && f[3] == disk
    })
}

/// /dev is tmpfs with no ueventd: nodes for the UFS LUNs must be mknod'd by
/// hand from /proc/partitions (the trampoline does the same for console fds).
fn create_block_nodes() -> usize {
    let Ok(content) = fs::read_to_string("/proc/partitions") else {
        return 0;
    };
    let mut made = 0usize;
    for line in content.lines().skip(2) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() != 4 {
            continue;
        }
        let (Ok(ma), Ok(mi), name) = (f[0].parse::<u32>(), f[1].parse::<u32>(), f[3]) else {
            continue;
        };
        let path = format!("/dev/{name}");
        if Path::new(&path).exists() {
            continue;
        }
        let c_path = std::ffi::CString::new(path.clone()).unwrap();
        let rc = unsafe {
            libc::mknod(c_path.as_ptr(), libc::S_IFBLK | 0o600, makedev(ma, mi))
        };
        if rc == 0 {
            made += 1;
        } else {
            klog(&format!("mknod {name}: {}", std::io::Error::last_os_error()));
        }
    }
    made
}

/// GPT partition entries of one UFS LUN: (name, first_lba). All six LUNs use
/// 4096-byte logical blocks (probed 2026-08-27); entries start at LBA2 and are
/// 128 bytes each, name UTF-16LE at offset 56, first_lba u64 LE at offset 32.
fn gpt_entries(disk: &str) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let Ok(mut f) = File::open(format!("/dev/{disk}")) else {
        return out;
    };
    if f.seek(SeekFrom::Start(2 * 4096)).is_err() {
        return out;
    }
    let mut buf = [0u8; 128 * 128];
    if f.read(&mut buf).unwrap_or(0) != buf.len() {
        return out;
    }
    for chunk in buf.chunks_exact(128) {
        if chunk[..16].iter().all(|&b| b == 0) {
            break; // first unused entry ends the array
        }
        let mut units = Vec::new();
        for pair in chunk[56..].chunks_exact(2) {
            if pair == [0, 0] {
                break;
            }
            units.push(u16::from_le_bytes([pair[0], pair[1]]));
        }
        let name = String::from_utf16_lossy(&units);
        let first_lba = u64::from_le_bytes(chunk[32..40].try_into().unwrap());
        out.push((name, first_lba));
    }
    out
}

/// Android-style /dev/block/by-name symlinks: GPT name -> /dev/<disk><N>.
/// A GPT first_lba is in 4096-byte units; /sys .../<part>/start is in
/// 512-byte units, so part start == first_lba * 8.
fn create_by_name_links() -> usize {
    let _ = fs::create_dir_all("/dev/block/by-name");
    let mut made = 0usize;
    // whole disks = /proc/partitions names with no digit suffix
    let disks: Vec<String> = fs::read_to_string("/proc/partitions")
        .map(|c| {
            c.lines()
                .skip(2)
                .filter_map(|l| {
                    let f: Vec<&str> = l.split_whitespace().collect();
                    (f.len() == 4 && !f[3].contains(|c: char| c.is_ascii_digit()))
                        .then(|| f[3].to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    for disk in disks {
        // map start-sector (512 units) -> partition name, from /sys/block
        let mut by_start: Vec<(u64, String)> = Vec::new();
        if let Ok(entries) = fs::read_dir(format!("/sys/block/{disk}")) {
            for e in entries.flatten() {
                let part = e.file_name().into_string().unwrap_or_default();
                if !part.starts_with(&disk) || part.len() <= disk.len() {
                    continue;
                }
                if let Ok(start) =
                    fs::read_to_string(format!("/sys/block/{disk}/{part}/start"))
                {
                    if let Ok(s) = start.trim().parse::<u64>() {
                        by_start.push((s, part));
                    }
                }
            }
        }
        for (name, first_lba) in gpt_entries(&disk) {
            let Some((_, part)) = by_start.iter().find(|(s, _)| *s == first_lba * 8) else {
                continue;
            };
            let target = format!("/dev/{part}");
            let link = format!("/dev/block/by-name/{name}");
            let _ = fs::remove_file(&link);
            if std::os::unix::fs::symlink(&target, &link).is_ok() {
                made += 1;
            }
        }
    }
    made
}

/// Storage bring-up as a PID 1 responsibility: load the UFS chain, wait for
/// the LUN scan, expose the partitions. misc (sda3) becomes writable from a
/// clean boot — no manual insmod.
fn bring_up_storage() {
    let base = Path::new("/lib/modules");
    let mut ok = 0usize;
    for name in UFS_MODULES {
        match load_module(&base.join(name)) {
            Ok(()) => ok += 1,
            Err(e) => klog(&format!("storage mod fail {name}: {e}")),
        }
    }
    // The LUN scan follows the ufshcd probe almost immediately (observed
    // <100 ms) — poll briefly rather than race it.
    for _ in 0..20 {
        if partitions_have("sda") {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    if !partitions_have("sda") {
        klog(&format!("storage: sda did not appear (mods ok={ok})"));
        return;
    }
    let made = create_block_nodes();
    let links = create_by_name_links();
    let misc = if Path::new("/dev/block/by-name/misc").exists() {
        "by-name ok"
    } else {
        "by-name MISSING misc"
    };
    klog(&format!(
        "storage up: ufs mods ok={ok}, {made} block nodes, {links} by-name links ({misc})"
    ));
}

fn load_modules_from_list() {
    // Kept for opt-in experiments; not used by default.
    let list = Path::new("/lib/modules/modules.load");
    let base = Path::new("/lib/modules");
    let Ok(f) = File::open(list) else {
        klog("no modules.load");
        return;
    };
    let mut ok = 0usize;
    let mut fail = 0usize;
    for line in BufReader::new(f).lines().flatten() {
        let name = line.trim();
        if name.is_empty() || name.starts_with('#') {
            continue;
        }
        let path = if name.starts_with('/') {
            PathBuf::from(name)
        } else {
            base.join(name)
        };
        match load_module(&path) {
            Ok(()) => ok += 1,
            Err(_) => fail += 1,
        }
    }
    klog(&format!("modules.load ok={ok} fail={fail}"));
}

// --- optional DRM (only if /aginxos/splash) ---

const fn drm_iowr(nr: u8, size: usize) -> libc::Ioctl {
    let dir: u64 = 3;
    let typ = b'd' as u64;
    ((dir << 30) | ((size as u64) << 16) | (typ << 8) | (nr as u64)) as libc::Ioctl
}

#[repr(C)]
struct DrmModeCardRes {
    fb_id_ptr: u64,
    crtc_id_ptr: u64,
    connector_id_ptr: u64,
    encoder_id_ptr: u64,
    count_fbs: u32,
    count_crtcs: u32,
    count_connectors: u32,
    count_encoders: u32,
    min_width: u32,
    max_width: u32,
    min_height: u32,
    max_height: u32,
}

#[repr(C)]
struct DrmModeCreateDumb {
    height: u32,
    width: u32,
    bpp: u32,
    flags: u32,
    handle: u32,
    pitch: u32,
    size: u64,
}

#[repr(C)]
struct DrmModeMapDumb {
    handle: u32,
    pad: u32,
    offset: u64,
}

#[repr(C)]
struct DrmModeFbCmd {
    fb_id: u32,
    width: u32,
    height: u32,
    pitch: u32,
    bpp: u32,
    depth: u32,
    handle: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DrmModeModeInfo {
    clock: u32,
    hdisplay: u16,
    hsync_start: u16,
    hsync_end: u16,
    htotal: u16,
    hskew: u16,
    vdisplay: u16,
    vsync_start: u16,
    vsync_end: u16,
    vtotal: u16,
    vscan: u16,
    vrefresh: u32,
    flags: u32,
    type_: u32,
    name: [u8; 32],
}

#[repr(C)]
struct DrmModeCrtc {
    set_connectors_ptr: u64,
    count_connectors: u32,
    crtc_id: u32,
    fb_id: u32,
    x: u32,
    y: u32,
    gamma_size: u32,
    mode_valid: u32,
    mode: DrmModeModeInfo,
}

fn drm_paint(color: u32) -> Result<(), String> {
    let path = "/dev/dri/card0";
    if !Path::new(path).exists() {
        return Err("no card0".into());
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("open: {e}"))?;
    let fd = file.as_raw_fd();
    let _ = unsafe { libc::ioctl(fd, 0x641e as libc::Ioctl) };

    let req_res = drm_iowr(0xA0, std::mem::size_of::<DrmModeCardRes>());
    let mut res = unsafe { std::mem::zeroed::<DrmModeCardRes>() };
    if unsafe { libc::ioctl(fd, req_res, &mut res as *mut _) } != 0 {
        return Err(format!("GETRESOURCES: {}", std::io::Error::last_os_error()));
    }
    let mut crtcs = vec![0u32; res.count_crtcs as usize];
    let mut connectors = vec![0u32; res.count_connectors as usize];
    res.crtc_id_ptr = crtcs.as_mut_ptr() as u64;
    res.connector_id_ptr = connectors.as_mut_ptr() as u64;
    if unsafe { libc::ioctl(fd, req_res, &mut res as *mut _) } != 0 {
        return Err(format!("GETRESOURCES2: {}", std::io::Error::last_os_error()));
    }
    if crtcs.is_empty() {
        return Err("no crtcs".into());
    }

    let req_getcrtc = drm_iowr(0xA1, std::mem::size_of::<DrmModeCrtc>());
    let req_setcrtc = drm_iowr(0xA2, std::mem::size_of::<DrmModeCrtc>());
    let req_create = drm_iowr(0xB2, std::mem::size_of::<DrmModeCreateDumb>());
    let req_addfb = drm_iowr(0xAE, std::mem::size_of::<DrmModeFbCmd>());
    let req_map = drm_iowr(0xB3, std::mem::size_of::<DrmModeMapDumb>());

    let mut crtc_id = 0u32;
    let mut mode = unsafe { std::mem::zeroed::<DrmModeModeInfo>() };
    let mut width = 0u32;
    let mut height = 0u32;
    for &id in &crtcs {
        let mut c = unsafe { std::mem::zeroed::<DrmModeCrtc>() };
        c.crtc_id = id;
        if unsafe { libc::ioctl(fd, req_getcrtc, &mut c as *mut _) } != 0 {
            continue;
        }
        if c.mode_valid == 0 {
            continue;
        }
        width = c.mode.hdisplay as u32;
        height = c.mode.vdisplay as u32;
        if width == 0 || height == 0 {
            continue;
        }
        crtc_id = id;
        mode = c.mode;
        break;
    }
    if crtc_id == 0 {
        return Err("no active crtc/mode".into());
    }

    let mut dumb = DrmModeCreateDumb {
        height,
        width,
        bpp: 32,
        flags: 0,
        handle: 0,
        pitch: 0,
        size: 0,
    };
    if unsafe { libc::ioctl(fd, req_create, &mut dumb as *mut _) } != 0 {
        return Err(format!("CREATE_DUMB: {}", std::io::Error::last_os_error()));
    }
    let mut fb = DrmModeFbCmd {
        fb_id: 0,
        width,
        height,
        pitch: dumb.pitch,
        bpp: 32,
        depth: 24,
        handle: dumb.handle,
    };
    if unsafe { libc::ioctl(fd, req_addfb, &mut fb as *mut _) } != 0 {
        return Err(format!("ADDFB: {}", std::io::Error::last_os_error()));
    }
    let mut map = DrmModeMapDumb {
        handle: dumb.handle,
        pad: 0,
        offset: 0,
    };
    if unsafe { libc::ioctl(fd, req_map, &mut map as *mut _) } != 0 {
        return Err(format!("MAP_DUMB: {}", std::io::Error::last_os_error()));
    }
    let len = dumb.size as usize;
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            map.offset as i64,
        )
    };
    if ptr == libc::MAP_FAILED {
        return Err(format!("mmap: {}", std::io::Error::last_os_error()));
    }
    unsafe {
        let pitch = (dumb.pitch / 4) as usize;
        let base = ptr as *mut u32;
        let border = 0x00FF_FFFF;
        let t = 24usize;
        for y in 0..height as usize {
            for x in 0..width as usize {
                let edge = x < t || y < t || x + t >= width as usize || y + t >= height as usize;
                *base.add(y * pitch + x) = if edge { border } else { color };
            }
        }
        libc::munmap(ptr, len);
    }

    let mut crtc = unsafe { std::mem::zeroed::<DrmModeCrtc>() };
    crtc.crtc_id = crtc_id;
    crtc.fb_id = fb.fb_id;
    crtc.mode = mode;
    crtc.mode_valid = 1;
    if !connectors.is_empty() {
        crtc.set_connectors_ptr = connectors.as_mut_ptr() as u64;
        crtc.count_connectors = 1;
    }
    if unsafe { libc::ioctl(fd, req_setcrtc, &mut crtc as *mut _) } != 0 {
        crtc.set_connectors_ptr = 0;
        crtc.count_connectors = 0;
        if unsafe { libc::ioctl(fd, req_setcrtc, &mut crtc as *mut _) } != 0 {
            return Err(format!("SETCRTC: {}", std::io::Error::last_os_error()));
        }
    }
    klog(&format!("DRM ok {width}x{height} color={color:#08x}"));
    Ok(())
}

fn handoff_android() -> ! {
    // Match the proven C trampoline: execve(first_stage, ["/init"], environ)
    extern "C" {
        static environ: *const *const libc::c_char;
    }
    let path = "/aginxos/first_stage_init";
    if Path::new(path).exists() {
        let c_path = std::ffi::CString::new(path).unwrap();
        let arg0 = std::ffi::CString::new("/init").unwrap();
        let argv = [arg0.as_ptr(), std::ptr::null()];
        // Do not log here if possible — keep world pristine; still try once.
        unsafe {
            libc::execve(c_path.as_ptr(), argv.as_ptr(), environ);
        }
        klog(&format!(
            "execve {path} failed: {}",
            std::io::Error::last_os_error()
        ));
    } else {
        klog("missing /aginxos/first_stage_init");
    }
    klog("handoff exhausted — holding");
    loop {
        thread::sleep(Duration::from_secs(30));
        klog("heartbeat");
    }
}

/// PID 1 inherits every orphan, including the forked usb-console branch.
/// Reap finished children so they do not linger as zombies between beats.
fn reap_zombies() {
    loop {
        let rc = unsafe { libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) };
        if rc <= 0 {
            break;
        }
        klog(&format!("reaped child pid {rc}"));
    }
}

/// Operator reboot via the usb console: `/aginxos/aginxos-init reboot [mode]`.
/// RESTART2 with a mode string only reaches fastboot when msm-poweroff's
/// restart handler is loaded (translates the string into the PMIC PON
/// scratch register — verified 2026-08-27: "bootloader" → fastboot in 6 s).
fn do_reboot(mode: Option<&str>) -> i32 {
    unsafe { libc::sync() };
    // Raw syscall: musl's reboot(2) wrapper takes no RESTART2 mode argument.
    const MAGIC1: libc::c_ulong = 0xfee1_dead;
    const MAGIC2: libc::c_ulong = 0x2812_1969; // LINUX_REBOOT_MAGIC2 (RESTART2 family)
    const CMD_RESTART: libc::c_ulong = 0x0123_4567;
    const CMD_RESTART2: libc::c_ulong = 0xa1b2_c3d4;
    let rc = match mode {
        Some(m) => {
            let Ok(c) = std::ffi::CString::new(m) else {
                eprintln!("aginxos-init: bad mode string");
                return 1;
            };
            unsafe {
                libc::syscall(libc::SYS_reboot, MAGIC1, MAGIC2, CMD_RESTART2, c.as_ptr())
            }
        }
        None => unsafe { libc::syscall(libc::SYS_reboot, MAGIC1, MAGIC2, CMD_RESTART, 0) },
    };
    if rc != 0 {
        eprintln!(
            "aginxos-init: reboot({}) failed: {}",
            mode.unwrap_or(""),
            std::io::Error::last_os_error()
        );
        return 1;
    }
    0
}

fn main() {
    // `aginxos-init reboot [mode]` — operator command, not the init path.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("reboot") {
        std::process::exit(do_reboot(args.get(2).map(String::as_str)));
    }

    let hold = flag("/aginxos/hold");
    let do_modules = flag("/aginxos/load-modules");
    let do_splash = flag("/aginxos/splash");
    let do_full_modules = flag("/aginxos/load-modules-full");
    let do_storage = flag("/aginxos/storage");
    // Immediate handoff without mounting (cleanest path to Android).
    let handoff_only =
        !hold && !do_modules && !do_splash && !do_full_modules && !do_storage;

    klog(&format!(
        "start v{} pid={} hold={hold} handoff_only={handoff_only} modules={do_modules} splash={do_splash} storage={do_storage}",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    ));

    if handoff_only {
        // Do not mount anything — leave a pristine world for first-stage init.
        klog("handoff-only: exec first_stage immediately");
        handoff_android();
    }

    ensure_basics();
    klog("basics ok");

    let _ = fs::create_dir_all("/aginxos");
    let _ = fs::write("/dev/aginxos_ran", b"1\n");

    if do_full_modules {
        klog("loading FULL modules.load (risky)");
        load_modules_from_list();
    } else if do_modules {
        load_safe_modules();
    } else {
        klog("skip modules (safe default)");
    }

    if do_storage {
        bring_up_storage();
    } else {
        klog("skip storage (safe default)");
    }

    if do_splash {
        for (i, c) in [
            0x00_22_CC_44u32,
            0x00_CC_22_22,
            0x00_22_44_CC,
            0x00_EE_EE_22,
        ]
        .iter()
        .enumerate()
        {
            match drm_paint(*c) {
                Ok(()) => klog(&format!("splash frame {i} ok")),
                Err(e) => klog(&format!("splash frame {i}: {e}")),
            }
            thread::sleep(Duration::from_secs(2));
        }
    } else {
        klog("skip splash (safe default)");
    }

    if hold {
        klog("HOLD — aginxos-init is PID 1 (VolDown+Power for fastboot restore)");
        let mut n = 0u32;
        loop {
            thread::sleep(Duration::from_secs(10));
            n += 1;
            reap_zombies();
            klog(&format!("hold heartbeat {n}"));
        }
    }

    klog("handoff to Android");
    handoff_android();
}
