//! AginxOS early init via vendor_boot `rdinit=/aginxos/aginxos-init`.
//!
//! Feature flags (empty files in ramdisk):
//!   /aginxos/hold         — do not hand off to Android
//!   /aginxos/load-modules — load /lib/modules/modules.load
//!   /aginxos/splash       — attempt DRM solid-color frames
//!   /aginxos/storage      — load the UFS chain, create block nodes
//!   /aginxos/super        — parse super, map+mount its _a sub-partitions
//!                           (implies storage)
//!   /aginxos/rootfs       — mount the ext4 rootfs on userdata and
//!                           switch_root into busybox init (implies storage)
//!
//! Operator subcommands:
//!   /aginxos/aginxos-init reboot [mode]
//!   /aginxos/aginxos-init parse-super <file>   (host-side metadata check)
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

// --- super / dynamic partitions (/aginxos/super) ---

/// liblp metadata lives at a fixed offset in super: geometry @0x1000,
/// header @0x3000 (bootstrapped from a dump 2026-08-27 — see HARDWARE.md;
/// the online sources were unreachable, the format is self-describing).
const SUPER_GEOMETRY_OFF: u64 = 0x1000;
const SUPER_HEADER_OFF: u64 = 0x3000;
const LP_GEOMETRY_MAGIC: u32 = 0x616c_4467;
const LP_HEADER_MAGIC: u32 = 0x414c_5030;

#[derive(PartialEq)]
struct SuperPart {
    name: String,
    num_sectors: u64,
    /// Linear source offset inside super, 512-byte sectors.
    source_sector: u64,
}

fn read_at(f: &mut File, off: u64, len: usize) -> Option<Vec<u8>> {
    f.seek(SeekFrom::Start(off)).ok()?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf).ok()?;
    Some(buf)
}

/// Parse the liblp geometry+header+tables out of super (a block device or a
/// dumped metadata file). Only single-extent LINEAR partitions are returned;
/// multi-extent or non-linear ones are skipped and logged by the caller.
///
/// Layout (verified byte-by-byte against a dump 2026-08-27): tables live at
/// header+header_size; partition name is plain ASCII[36]; extents are packed
/// {u64 num_sectors; u32 target_type; u64 source_data} with source_data at
/// offset 12 — NOT 16, which produced overlapping extents until the raw
/// hexdump settled it.
fn parse_super(f: &mut File) -> Result<Vec<SuperPart>, String> {
    let geo = read_at(f, SUPER_GEOMETRY_OFF, 32)
        .ok_or_else(|| "read geometry".to_string())?;
    if u32::from_le_bytes(geo[0..4].try_into().unwrap()) != LP_GEOMETRY_MAGIC {
        return Err("bad geometry magic".into());
    }
    let hdr = read_at(f, SUPER_HEADER_OFF, 0x80)
        .ok_or_else(|| "read header".to_string())?;
    if u32::from_le_bytes(hdr[0..4].try_into().unwrap()) != LP_HEADER_MAGIC {
        return Err("bad header magic".into());
    }
    let header_size = u32::from_le_bytes(hdr[8..12].try_into().unwrap()) as u64;
    let tables_base = SUPER_HEADER_OFF + header_size;
    // Table descriptors at header+0x50: 4 x {u32 offset, u32 num_entries,
    // u32 entry_size} in order partitions, extents, groups, block_devices.
    // Offsets are relative to the end of the header.
    let desc = |i: usize| -> (u64, usize, usize) {
        let d = &hdr[0x50 + i * 12..];
        let off = u32::from_le_bytes(d[0..4].try_into().unwrap()) as u64;
        let n = u32::from_le_bytes(d[4..8].try_into().unwrap()) as usize;
        let sz = u32::from_le_bytes(d[8..12].try_into().unwrap()) as usize;
        (tables_base + off, n, sz)
    };
    let (p_off, p_n, p_sz) = desc(0);
    let (e_off, e_n, e_sz) = desc(1);
    let parts = read_at(f, p_off, p_n * p_sz).ok_or_else(|| "read partitions".to_string())?;
    let exts = read_at(f, e_off, e_n * e_sz).ok_or_else(|| "read extents".to_string())?;

    let mut out = Vec::new();
    for p in parts.chunks_exact(p_sz) {
        if p[..16].iter().all(|&b| b == 0) {
            break; // unused entry ends the table
        }
        let name = String::from_utf8_lossy(&p[..36]).to_string();
        let name = name.split('\0').next().unwrap_or("").to_string();
        let first_extent = u32::from_le_bytes(p[40..44].try_into().unwrap()) as usize;
        let num_extents = u32::from_le_bytes(p[44..48].try_into().unwrap()) as usize;
        if num_extents != 1 || first_extent >= e_n {
            klog(&format!("super: skip {name} ({num_extents} extents)"));
            continue;
        }
        let e = &exts[first_extent * e_sz..];
        let num_sectors = u64::from_le_bytes(e[0..8].try_into().unwrap());
        let target_type = u32::from_le_bytes(e[8..12].try_into().unwrap());
        let source = u64::from_le_bytes(e[12..20].try_into().unwrap());
        if target_type != 0 {
            klog(&format!("super: skip {name} (target_type {target_type})"));
            continue;
        }
        out.push(SuperPart {
            name,
            num_sectors,
            source_sector: source,
        });
    }
    Ok(out)
}

/// Extents must tile super without overlap and stay inside the partition.
/// This doubles as corruption detection: right after the UFS probe the first
/// metadata reads came back garbage (kernel saw linear start=30726 where the
/// real value is 3072 — boot run 2026-08-27, t+44.8s; minutes later the same
/// read was correct). Two consecutive identical, valid reads are required.
fn tiles(parts: &[SuperPart], super_sectors: Option<u64>) -> bool {
    let mut spans: Vec<(u64, u64)> = parts
        .iter()
        .map(|p| (p.source_sector, p.source_sector + p.num_sectors))
        .collect();
    spans.sort_unstable();
    for w in spans.windows(2) {
        if w[0].1 > w[1].0 {
            return false; // overlap
        }
    }
    if let Some(sz) = super_sectors {
        if let Some(&(_, end)) = spans.last() {
            if end > sz {
                return false; // beyond super
            }
        }
    }
    true
}

fn parse_super_stable(
    f: &mut File,
    super_sectors: Option<u64>,
) -> Result<Vec<SuperPart>, String> {
    let mut prev: Option<Vec<SuperPart>> = None;
    for attempt in 0..10 {
        let parts = parse_super(f)?;
        if !tiles(&parts, super_sectors) {
            klog(&format!("super: read {attempt} failed validation, retrying"));
            prev = None; // corrupted read — require two fresh identical ones
        } else if let Some(p) = &prev {
            if *p == parts {
                if attempt > 0 {
                    klog(&format!("super: stable after {attempt} retries"));
                }
                return Ok(parts);
            }
            klog("super: consecutive reads differ, retrying");
        }
        prev = Some(parts);
        thread::sleep(Duration::from_millis(50));
    }
    Err("metadata reads never stabilized".into())
}

// device-mapper ioctls (linux/dm-ioctl.h layout; _IOWR size = sizeof header).
#[repr(C)]
struct DmIoctlHdr {
    version: [u32; 3],
    data_size: u32,
    data_start: u32,
    target_count: u32,
    open_count: u32,
    flags: u32,
    event_nr: u32,
    padding1: u32,
    dev: u64,
    name: [u8; 16],
    uuid: [u8; 129],
    data: [u8; 7],
}

#[repr(C)]
struct DmTargetSpec {
    sector_start: u64,
    length: u64,
    status: i32,
    next: u32,
    target_type: [u8; 16],
}

const fn dm_iowr(nr: u8) -> libc::Ioctl {
    let hdr_size = std::mem::size_of::<DmIoctlHdr>() as u64;
    ((3u64 << 30) | (hdr_size << 16) | (0xfdu64 << 8) | nr as u64) as libc::Ioctl
}
const DM_VERSION: libc::Ioctl = dm_iowr(0x00);
const DM_DEV_CREATE: libc::Ioctl = dm_iowr(0x03);
const DM_DEV_REMOVE: libc::Ioctl = dm_iowr(0x04);
const DM_DEV_SUSPEND: libc::Ioctl = dm_iowr(0x06);
const DM_TABLE_LOAD: libc::Ioctl = dm_iowr(0x09);

const DM_BUF: usize = 4096;

fn dm_hdr(buf: &mut [u8; DM_BUF], version: [u32; 3], name: &str) -> *mut DmIoctlHdr {
    *buf = [0u8; DM_BUF];
    for (i, v) in version.iter().enumerate() {
        buf[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    buf[12..16].copy_from_slice(&(DM_BUF as u32).to_le_bytes());
    buf[16..20].copy_from_slice(&(std::mem::size_of::<DmIoctlHdr>() as u32).to_le_bytes());
    let b = name.as_bytes();
    let n = b.len().min(15);
    buf[48..48 + n].copy_from_slice(&b[..n]);
    buf.as_mut_ptr() as *mut DmIoctlHdr
}

/// Map one linear window of `src_dev` ("maj:min") and mount it ext4 ro.
/// Returns the mount point on success.
fn map_and_mount(
    ctl: &mut File,
    version: [u32; 3],
    name: &str,
    src_dev: &str,
    start: u64,
    len: u64,
) -> Result<String, String> {
    let err = || std::io::Error::last_os_error();
    let mut buf = [0u8; DM_BUF];
    unsafe { libc::ioctl(ctl.as_raw_fd(), DM_DEV_REMOVE, dm_hdr(&mut buf, version, name)) };

    let h = dm_hdr(&mut buf, version, name);
    if unsafe { libc::ioctl(ctl.as_raw_fd(), DM_DEV_CREATE, h) } != 0 {
        return Err(format!("DM_DEV_CREATE: {}", err()));
    }
    let dev = unsafe { (*h).dev };
    let dm_minor = (dev & 0xff) | ((dev >> 12) & 0xfff_00);

    let h = dm_hdr(&mut buf, version, name);
    unsafe {
        (*h).target_count = 1;
        let spec = &mut *(buf
            .as_mut_ptr()
            .add(std::mem::size_of::<DmIoctlHdr>()) as *mut DmTargetSpec);
        spec.sector_start = 0;
        spec.length = len;
        spec.status = 0;
        spec.next = 0;
        spec.target_type[..6].copy_from_slice(b"linear");
        // Params string + explicit NUL. Rust Strings are not NUL-terminated:
        // copying len+1 bytes reads one byte PAST the heap allocation, which
        // overwrote the zeroed buffer with garbage — product_a's start came
        // out as "30726" (a stray '6') and the others failed sscanf outright
        // (found 2026-08-27; see HARDWARE.md).
        let params = format!("{src_dev} {start}");
        let dst = buf.as_mut_ptr().add(std::mem::size_of::<DmIoctlHdr>() + 40);
        dst.copy_from(params.as_bytes().as_ptr(), params.len());
        *dst.add(params.len()) = 0;
        if libc::ioctl(ctl.as_raw_fd(), DM_TABLE_LOAD, h) != 0 {
            return Err(format!("DM_TABLE_LOAD: {}", err()));
        }
    }

    // flags=0 on DEV_SUSPEND means resume (no DM_SUSPEND_FLAG set).
    let h = dm_hdr(&mut buf, version, name);
    if unsafe { libc::ioctl(ctl.as_raw_fd(), DM_DEV_SUSPEND, h) } != 0 {
        return Err(format!("DM_DEV_RESUME: {}", err()));
    }

    let devpath = format!("/dev/dm-{dm_minor}");
    let c_path = std::ffi::CString::new(devpath.clone()).unwrap();
    let rc = unsafe { libc::mknod(c_path.as_ptr(), libc::S_IFBLK | 0o600, dev as libc::dev_t) };
    if rc != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
        return Err(format!("mknod {devpath}: {}", err()));
    }
    let _ = std::os::unix::fs::symlink(&devpath, format!("/dev/block/mapper/{name}"));

    let mnt = format!("/{name}");
    mkdir_p(&mnt);
    let src = std::ffi::CString::new(devpath.clone()).unwrap();
    let tgt = std::ffi::CString::new(mnt.clone()).unwrap();
    let fst = std::ffi::CString::new("ext4").unwrap();
    let rc = unsafe {
        libc::mount(
            src.as_ptr(),
            tgt.as_ptr(),
            fst.as_ptr(),
            libc::MS_RDONLY,
            std::ptr::null(),
        )
    };
    if rc != 0 {
        return Err(format!("mount ext4 ro {devpath}: {}", err()));
    }
    Ok(mnt)
}

/// Parse super and expose its big _a sub-partitions as dm-linear + ext4 ro
/// mounts at /system_a, /vendor_a, /product_a (live-proven 2026-08-27 with
/// the same recipe via a scratch C tool — see HARDWARE.md).
fn bring_up_super() {
    const ALLOW: &[&str] = &["system", "vendor", "product", "system_ext"];
    let Some(super_dev) = fs::read_link("/dev/block/by-name/super")
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
    else {
        klog("super: no /dev/block/by-name/super (storage flag?)");
        return;
    };
    // major:minor of the super partition from /proc/partitions.
    let sm = fs::read_to_string("/proc/partitions").ok().and_then(|c| {
        c.lines().find_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.len() == 4 && f[3] == super_dev)
                .then(|| (format!("{}:{}", f[0], f[1]), f[2].parse::<u64>().ok()))
        })
    });
    let Some((src_dev, blocks)) = sm else {
        klog(&format!("super: {super_dev} not in /proc/partitions"));
        return;
    };
    let super_sectors = blocks.map(|kb| kb * 2); // /proc/partitions is 1K blocks

    let Ok(mut f) = File::open(format!("/dev/{super_dev}")) else {
        klog(&format!("super: open /dev/{super_dev} failed"));
        return;
    };
    let parts = match parse_super_stable(&mut f, super_sectors) {
        Ok(p) => p,
        Err(e) => {
            klog(&format!("super: parse failed: {e}"));
            return;
        }
    };
    drop(f);

    mkdir_p("/dev/block/mapper");
    // /dev/mapper itself must exist before the control node can go in it —
    // caught on the first boot-time run (ENOENT, 2026-08-27): the live test
    // had inherited a hand-made directory and masked this.
    mkdir_p("/dev/mapper");
    let control = "/dev/mapper/control";
    if !Path::new(control).exists() {
        let c = std::ffi::CString::new(control).unwrap();
        let rc =
            unsafe { libc::mknod(c.as_ptr(), libc::S_IFCHR | 0o600, makedev(10, 236)) };
        if rc != 0 {
            klog(&format!(
                "super: mknod mapper/control: {}",
                std::io::Error::last_os_error()
            ));
            return;
        }
    }
    let mut ctl = match OpenOptions::new().read(true).write(true).open(control) {
        Ok(f) => f,
        Err(e) => {
            klog(&format!("super: open mapper/control: {e}"));
            return;
        }
    };
    let mut buf = [0u8; DM_BUF];
    let h = dm_hdr(&mut buf, [4, 39, 0], "");
    if unsafe { libc::ioctl(ctl.as_raw_fd(), DM_VERSION, h) } != 0 {
        klog(&format!("super: DM_VERSION: {}", std::io::Error::last_os_error()));
        return;
    }
    let version = unsafe { (*h).version };
    klog(&format!(
        "super: {} parts, dm {}.{}, src {src_dev}",
        parts.len(),
        version[0],
        version[1]
    ));

    let mut mounted = Vec::new();
    for base in ALLOW {
        let name = format!("{base}_a");
        let Some(p) = parts.iter().find(|p| p.name == name) else {
            continue;
        };
        let (len, start) = (p.num_sectors, p.source_sector);
        klog(&format!("super: {name} {len} sectors @ {start}"));
        match map_and_mount(&mut ctl, version, &name, &src_dev, start, len) {
            Ok(mnt) => {
                klog(&format!("super: mounted {name} at {mnt}"));
                mounted.push(name);
            }
            Err(e) => klog(&format!("super: {name} failed: {e}")),
        }
    }
    klog(&format!("super up: mounted {} [{}]", mounted.len(), mounted.join(",")));
}

// --- rootfs switch (/aginxos/rootfs) ---

/// Terminal parking state: PID 1 must never exit (that is a kernel panic),
/// so every failure path in the switch_root sequence ends here with the
/// trampoline's adbd console still alive for a post-mortem.
fn hold_forever(reason: &str) -> ! {
    klog(reason);
    klog("HOLD — aginxos-init is PID 1 (VolDown+Power for fastboot restore)");
    let mut n = 0u32;
    loop {
        thread::sleep(Duration::from_secs(10));
        n += 1;
        reap_zombies();
        klog(&format!("hold heartbeat {n}"));
    }
}

/// Remount every mount except the initramfs root and the new root itself
/// into the new root, so the busybox world inherits /proc, /sys and /dev
/// (with the ffs gadget mount adbd needs) already up. With MS_MOVE the
/// initramfs view is emptied (used by the real flow, after adbd is killed);
/// with MS_BIND the initramfs keeps its mounts so the old adbd — whose
/// shell service aborts the moment it loses them (observed 2026-08-27:
/// "Failed to get SELinux context" SIGABRT once /proc had been moved away;
/// exec-out then died on the missing /dev/ptmx) — stays a working console,
/// and the new world is reachable through its /proc/<pid>/root.
///
/// The table is read fully before the first change — iterating /proc/mounts
/// while remounting /proc is a self-inflicted race. Submounts travel with
/// their parent (MS_REC for the bind case); stale entries for them are
/// recognized by prefix and skipped.
fn remounts_into(newroot: &str, move_not_bind: bool) -> usize {
    // /dev on this initramfs is NOT a mount: the trampoline mknod'd its
    // nodes (console, urandom, __properties__, block nodes) straight into
    // the initramfs rootfs /dev directory, so /proc/mounts never lists it
    // and a pure mount-loop silently skips it. The new root then has no
    // /dev/urandom and the respawned adbd dies at its first getentropy —
    // "getentropy failed: No such file or directory" in /var/adbd.log,
    // which is exactly how the first ROOTFS boot went dark (2026-08-27;
    // no pstore on this kernel, so the log was the only witness). Bind it
    // explicitly — MS_MOVE needs a real mount — with MS_REC so the devpts
    // and ffs submounts travel too.
    let mut done = 0usize;
    let mut done_from: Vec<String> = Vec::new();
    if Path::new("/dev/null").exists() && !Path::new(&format!("{newroot}/dev/null")).exists() {
        let target = format!("{newroot}/dev");
        mkdir_p(&target);
        let src = std::ffi::CString::new("/dev").unwrap();
        let tgt = std::ffi::CString::new(target).unwrap();
        let rc = unsafe {
            libc::mount(
                src.as_ptr(),
                tgt.as_ptr(),
                std::ptr::null(),
                libc::MS_BIND | libc::MS_REC,
                std::ptr::null(),
            )
        };
        if rc == 0 {
            done += 1;
            done_from.push("/dev".to_string());
        } else {
            klog(&format!("rootfs: bind /dev: {}", std::io::Error::last_os_error()));
        }
    }
    let Ok(table) = fs::read_to_string("/proc/mounts") else {
        return done;
    };
    let flags = if move_not_bind {
        libc::MS_MOVE
    } else {
        libc::MS_BIND | libc::MS_REC
    };
    for line in table.lines() {
        // /proc/mounts escapes spaces as \040; our mountpoints have none.
        let Some(mp) = line.split(' ').nth(1).map(|m| m.replace("\\040", " ")) else {
            continue;
        };
        if mp == "/" || mp == newroot || mp.starts_with(&format!("{newroot}/")) {
            continue;
        }
        if done_from.iter().any(|m| mp.starts_with(&format!("{m}/"))) {
            continue; // already traveled inside a remounted parent
        }
        let target = format!("{newroot}{mp}");
        mkdir_p(&target);
        let src = std::ffi::CString::new(mp.clone()).unwrap();
        let tgt = std::ffi::CString::new(target).unwrap();
        let rc = unsafe {
            libc::mount(src.as_ptr(), tgt.as_ptr(), std::ptr::null(), flags, std::ptr::null())
        };
        if rc == 0 {
            done += 1;
            done_from.push(mp);
        } else {
            klog(&format!("rootfs: remount {mp}: {}", std::io::Error::last_os_error()));
        }
    }
    done
}

/// Stop the trampoline's adbd (our child) so the respawned one can re-open
/// the ffs endpoints: ep0 is single-open, a second adbd would EBUSY-loop.
/// TERM, brief wait, then KILL, then reap.
fn kill_adbd() {
    let mut pids: Vec<i32> = Vec::new();
    if let Ok(entries) = fs::read_dir("/proc") {
        for e in entries.flatten() {
            let Ok(name) = e.file_name().into_string() else {
                continue;
            };
            if !name.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            let comm = fs::read_to_string(format!("/proc/{name}/comm")).unwrap_or_default();
            if comm.trim() == "adbd" {
                if let Ok(pid) = name.parse::<i32>() {
                    pids.push(pid);
                }
            }
        }
    }
    for &pid in &pids {
        unsafe { libc::kill(pid, libc::SIGTERM) };
    }
    for _ in 0..30 {
        if pids.iter().all(|p| !Path::new(&format!("/proc/{p}")).exists()) {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    for &pid in &pids {
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    for pid in pids {
        unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) };
    }
    klog("rootfs: old adbd stopped");
}

/// /aginxos/rootfs: mount the ext4 rootfs on userdata, switch_root into it,
/// exec busybox init as the new PID 1. Everything that can fail runs while
/// the console is still alive; only the irreversible tail (MS_MOVE, chroot,
/// execve) runs after the old adbd is killed. Never returns — success is an
/// execve, failure parks in `hold_forever`.
fn switch_to_rootfs() -> ! {
    const NEWROOT: &str = "/newroot";
    let Some(dev) = fs::read_link("/dev/block/by-name/userdata")
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
    else {
        hold_forever("rootfs: no /dev/block/by-name/userdata (storage flag?)");
    };
    let devpath = format!("/dev/{dev}");
    klog(&format!("rootfs: userdata is {devpath}"));
    mkdir_p(NEWROOT);
    let src = std::ffi::CString::new(devpath.clone()).unwrap();
    let tgt = std::ffi::CString::new(NEWROOT).unwrap();
    let fst = std::ffi::CString::new("ext4").unwrap();
    let rc = unsafe {
        libc::mount(
            src.as_ptr(),
            tgt.as_ptr(),
            fst.as_ptr(),
            0, // no MS_RDONLY: rw, so rcS can normalize ownership
            std::ptr::null(),
        )
    };
    if rc != 0 {
        hold_forever(&format!(
            "rootfs: mount {devpath} ext4 rw: {}",
            std::io::Error::last_os_error()
        ));
    }
    klog("rootfs: userdata mounted rw at /newroot");
    // The next PID 1 must exist before the console is burned down.
    for must in [
        "/newroot/sbin/init",
        "/newroot/bin/busybox",
        "/newroot/etc/inittab",
        "/newroot/system/bin/adbd",
    ] {
        if !Path::new(must).exists() {
            hold_forever(&format!("rootfs: {must} missing on userdata"));
        }
    }

    // Diagnostic mode (/aginxos/keep-adbd): the trampoline's adbd survives
    // the switch (its fds pin the ffs endpoints open), leaving a live console
    // to inspect the busybox world with — but only if its mounts are NOT
    // moved out from under it, so this path bind-mounts instead.
    let keep = flag("/aginxos/keep-adbd");
    if keep {
        klog("rootfs: keep-adbd set — old adbd stays as the console");
    } else {
        kill_adbd();
    }
    let n = remounts_into(NEWROOT, !keep);
    klog(&format!(
        "rootfs: {} mounts {} into {NEWROOT}",
        n,
        if keep { "bind-mounted" } else { "moved" }
    ));

    // Irreversible tail. chdir first so "." is unambiguously in the new root;
    // then MS_MOVE it onto / (the ext4 becomes the root — the initramfs
    // cannot be unmounted, it is shadowed beneath and keeps serving text
    // pages to anything still executing from it); chroot; exec.
    let c_new = std::ffi::CString::new(NEWROOT).unwrap();
    let c_root = std::ffi::CString::new("/").unwrap();
    if unsafe { libc::chdir(c_new.as_ptr()) } != 0 {
        let e = std::io::Error::last_os_error();
        hold_forever(&format!("rootfs: chdir {NEWROOT}: {e}"));
    }
    if unsafe {
        libc::mount(
            c_new.as_ptr(),
            c_root.as_ptr(),
            std::ptr::null(),
            libc::MS_MOVE,
            std::ptr::null(),
        )
    } != 0
    {
        let e = std::io::Error::last_os_error();
        hold_forever(&format!("rootfs: MS_MOVE {NEWROOT}->/: {e}"));
    }
    if unsafe { libc::chroot(b".\0".as_ptr() as *const libc::c_char) } != 0 {
        let e = std::io::Error::last_os_error();
        hold_forever(&format!("rootfs: chroot: {e}"));
    }
    if unsafe { libc::chdir(c_root.as_ptr()) } != 0 {
        let e = std::io::Error::last_os_error();
        hold_forever(&format!("rootfs: chdir /: {e}"));
    }
    klog("rootfs: exec /sbin/init (busybox)");
    let c_init = std::ffi::CString::new("/sbin/init").unwrap();
    let arg0 = std::ffi::CString::new("/sbin/init").unwrap();
    let argv = [arg0.as_ptr(), std::ptr::null()];
    let env_home = std::ffi::CString::new("HOME=/").unwrap();
    let env_path = std::ffi::CString::new("PATH=/sbin:/bin:/usr/sbin:/usr/bin:/system/bin").unwrap();
    let env_term = std::ffi::CString::new("TERM=linux").unwrap();
    let envp = [
        env_home.as_ptr(),
        env_path.as_ptr(),
        env_term.as_ptr(),
        std::ptr::null(),
    ];
    unsafe { libc::execve(c_init.as_ptr(), argv.as_ptr(), envp.as_ptr()) };
    hold_forever(&format!(
        "rootfs: execve /sbin/init: {}",
        std::io::Error::last_os_error()
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
    // `aginxos-init mount-super` — live test of the super flag on a running
    // console (no flash cycle needed).
    if args.get(1).map(String::as_str) == Some("mount-super") {
        ensure_basics();
        if !partitions_have("sda") {
            bring_up_storage();
        }
        bring_up_super();
        std::process::exit(0);
    }
    // `aginxos-init parse-super <file>` — host-side metadata sanity check.
    if args.get(1).map(String::as_str) == Some("parse-super") {
        let Some(path) = args.get(2) else {
            eprintln!("usage: aginxos-init parse-super <file>");
            std::process::exit(2);
        };
        let mut f = File::open(path).expect("open");
        match parse_super_stable(&mut f, None) {
            Ok(parts) => {
                for p in parts {
                    println!("{}: {} sectors @ {}", p.name, p.num_sectors, p.source_sector);
                }
            }
            Err(e) => {
                eprintln!("parse failed: {e}");
                std::process::exit(1);
            }
        }
        std::process::exit(0);
    }

    let hold = flag("/aginxos/hold");
    let do_modules = flag("/aginxos/load-modules");
    let do_splash = flag("/aginxos/splash");
    let do_full_modules = flag("/aginxos/load-modules-full");
    let do_super = flag("/aginxos/super");
    let do_rootfs = flag("/aginxos/rootfs");
    let do_storage = flag("/aginxos/storage") || do_super || do_rootfs;
    // Immediate handoff without mounting (cleanest path to Android).
    let handoff_only =
        !hold && !do_modules && !do_splash && !do_full_modules && !do_storage;

    klog(&format!(
        "start v{} pid={} hold={hold} handoff_only={handoff_only} modules={do_modules} splash={do_splash} storage={do_storage} super={do_super} rootfs={do_rootfs}",
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

    if do_super {
        bring_up_super();
    } else {
        klog("skip super (safe default)");
    }

    // Diverges: success execve's busybox init in the new root, failure parks
    // in hold_forever — splash/hold below are unreachable with this flag.
    if do_rootfs {
        switch_to_rootfs();
    } else {
        klog("skip rootfs (safe default)");
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
        hold_forever("HOLD");
    }

    klog("handoff to Android");
    handoff_android();
}
