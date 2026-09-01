//! agupd — A/B self-updater (M14).
//!
//! Flow: pull manifest → download images → verify sha256 → write the
//! INACTIVE slot's partitions → `agboot-ok set-active` (GPT attrs) →
//! reboot. Fully autonomous: on this ABL (r3-0.6) the GPT attributes
//! alone select the OS slot — proven on device 2026-09-02 with
//! attrs-only switches in both directions, no bootloader involvement.
//!
//! Rollback (all observed on device 2026-09-02): the staged slot boots
//! with succ=0/tries=7; ABL drains one try per boot that rcS does not
//! mark successful; at tries=0 ABL marks the slot unbootable,
//! re-activates the still-successful other slot, and cold-reboots into
//! it by itself. Failure classes that do NOT auto-rollback and need
//! host rescue: a zeroed/garbled boot header (ABL drops to fastboot),
//! and a kernel that hangs early (device sits dark — no watchdog yet;
//! a forced power-cycle drains one more try). sha256 verification at
//! apply time is what keeps those classes from ever being written.
//!
//! The GPT surgery deliberately lives in agsvc's agboot-ok — one audited
//! implementation of the 4K-block multi-LUN attribute rewrite. This crate
//! only orchestrates and streams bytes.
//!
//! Ordering rule (why this is crash-safe mid-update): partition bytes are
//! written and fsynced FIRST; the GPT attr flip is the last write before
//! reboot. A crash at any earlier point leaves both slots exactly as they
//! were. A crash after the flip but before reboot is harmless — the next
//! boot just takes the new slot early.
//!
//! Manifest (JSON, local path or https URL; image urls likewise):
//!   { "version": "…",
//!     "boot":          { "url": "…", "sha256": "hex", "size": N },
//!     "vendor_boot":   { … },   // optional, only when modules move
//!     "dtbo":          { … },   // optional
//!     "vbmeta":        { … },   // optional — must chain with the images
//!     "vbmeta_system": { … } }  // optional
//!
//! HTTPS goes through agdl (M10) — the phone's only TLS fetcher.

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::process::Command;

use serde::Deserialize;

#[derive(Deserialize)]
struct Image {
    url: String,
    sha256: String,
    size: Option<u64>,
}

#[derive(Deserialize)]
struct Manifest {
    version: String,
    boot: Image,
    vendor_boot: Option<Image>,
    dtbo: Option<Image>,
    vbmeta: Option<Image>,
    vbmeta_system: Option<Image>,
}

fn die(msg: &str) -> ! {
    eprintln!("agupd: {msg}");
    std::process::exit(1);
}

/// Active slot from the kernel cmdline — `_a`/`_b`. Never guessed: an
/// updater that flashes the wrong slot is a brick.
fn active_slot() -> String {
    std::fs::read_to_string("/proc/cmdline")
        .ok()
        .and_then(|c| {
            c.split_whitespace().find_map(|t| {
                let v = t.strip_prefix("androidboot.slot_suffix=")?;
                let v = v.trim_matches('"');
                (v == "_a" || v == "_b").then(|| v.to_string())
            })
        })
        .unwrap_or_else(|| die("no androidboot.slot_suffix on cmdline — refusing to flash"))
}

fn other(slot: &str) -> String {
    if slot == "_a" { "_b".into() } else { "_a".into() }
}

/// Download `url` to `out` if remote (via agdl), or point at it if local.
fn stage(url: &str, out: &str) -> String {
    if url.starts_with('/') {
        if !std::path::Path::new(url).is_file() {
            die(&format!("local source {url} missing"));
        }
        return url.to_string();
    }
    if !url.starts_with("https://") && !url.starts_with("http://") {
        die(&format!("unsupported url {url}"));
    }
    let st = Command::new("/usr/bin/agdl")
        .arg(url)
        .arg(out)
        .status()
        .unwrap_or_else(|e| die(&format!("cannot run agdl: {e}")));
    if !st.success() {
        die(&format!("agdl failed on {url}"));
    }
    out.to_string()
}

fn sha256_hex_file(path: &str) -> String {
    let mut f = File::open(path).unwrap_or_else(|e| die(&format!("open {path}: {e}")));
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 4 << 20];
    loop {
        let n = f.read(&mut buf).unwrap_or_else(|e| die(&format!("read {path}: {e}")));
        if n == 0 { break; }
        h.update(&buf[..n]);
    }
    hex(&h.finish())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Partition byte size via BLKGETSIZE64 — the image must fit, full stop.
fn part_size(dev: &str) -> u64 {
    let f = File::open(dev).unwrap_or_else(|e| die(&format!("open {dev}: {e}")));
    let mut sz: libc::c_ulonglong = 0;
    unsafe {
        if libc::ioctl(f.as_raw_fd(), 0x80081272u32 as libc::c_int, &mut sz) != 0 {
            die(&format!("BLKGETSIZE64 on {dev}: {}", std::io::Error::last_os_error()));
        }
    }
    sz
}

/// Stream `img` onto raw partition `dev`, fsync at the end. Nothing is
/// read back — the sha256 was checked on the staging file.
fn write_part(img: &str, dev: &str) -> u64 {
    let ps = part_size(dev);
    let mut src = File::open(img).unwrap_or_else(|e| die(&format!("open {img}: {e}")));
    let meta = src.metadata().unwrap_or_else(|e| die(&format!("stat {img}: {e}")));
    if meta.len() > ps {
        die(&format!("{img} is {} bytes, {dev} only {ps}", meta.len()));
    }
    let dst = OpenOptions::new().write(true).open(dev)
        .unwrap_or_else(|e| die(&format!("open {dev} rw: {e} — is the slot mounted?")));
    let mut buf = vec![0u8; 4 << 20];
    let mut off: u64 = 0;
    loop {
        let n = src.read(&mut buf).unwrap_or_else(|e| die(&format!("read {img}: {e}")));
        if n == 0 { break; }
        let mut done = 0usize;
        while done < n {
            let w = unsafe {
                libc::pwrite(dst.as_raw_fd(), buf[done..n].as_ptr() as *const _, (n - done) as _, off as libc::off_t)
            };
            if w <= 0 {
                die(&format!("pwrite {dev} at {off}: {}", std::io::Error::last_os_error()));
            }
            done += w as usize;
            off += w as u64;
        }
    }
    unsafe { libc::fsync(dst.as_raw_fd()) };
    off
}

fn space_at(dir: &str) -> u64 {
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    let c = match std::ffi::CString::new(dir) { Ok(c) => c, Err(_) => return 0 };
    if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
        return 0;
    }
    (st.f_bavail as i64).max(0) as u64 * st.f_bsize as u64
}

/// First usable staging dir — userdata (`/` on this rootfs) keeps large
/// downloads off tmpfs.
fn staging_root() -> String {
    for d in ["/data/update", "/update", "/tmp/agupd"] {
        let _ = std::fs::create_dir_all(d);
        if space_at(d) > 0 {
            return d.to_string();
        }
    }
    die("no usable staging directory")
}

fn fetch_manifest(src: &str) -> Manifest {
    let body = if src.starts_with('/') {
        std::fs::read_to_string(src).unwrap_or_else(|e| die(&format!("read {src}: {e}")))
    } else {
        let staged = stage(src, &format!("{}/manifest.json", staging_root()));
        std::fs::read_to_string(&staged).unwrap_or_else(|e| die(&format!("read {staged}: {e}")))
    };
    serde_json::from_str(&body).unwrap_or_else(|e| die(&format!("manifest parse: {e}")))
}

fn verify(img: &Image, path: &str) {
    let meta = std::fs::metadata(path).unwrap_or_else(|e| die(&format!("stat {path}: {e}")));
    if let Some(sz) = img.size {
        if meta.len() != sz {
            die(&format!("{path}: size {} != manifest {}", meta.len(), sz));
        }
    }
    let got = sha256_hex_file(path);
    if !got.eq_ignore_ascii_case(&img.sha256) {
        die(&format!("{path}: sha256 {got} != manifest {}", img.sha256));
    }
}

fn current_version() -> String {
    std::fs::read_to_string("/etc/aginx-version").map(|s| s.trim().to_string()).unwrap_or_else(|_| "unknown".into())
}

fn cmd_apply(src: &str, no_reboot: bool) {
    let m = fetch_manifest(src);
    let act = active_slot();
    let tgt = other(&act);
    println!("agupd: running {} → applying {} to slot {tgt}", current_version(), m.version);

    let root = staging_root();
    let parts: Vec<(&str, &Image)> = vec![("boot", &m.boot)].into_iter()
        .chain(m.vendor_boot.iter().map(|i| ("vendor_boot", i)))
        .chain(m.dtbo.iter().map(|i| ("dtbo", i)))
        .chain(m.vbmeta.iter().map(|i| ("vbmeta", i)))
        .chain(m.vbmeta_system.iter().map(|i| ("vbmeta_system", i)))
        .collect();
    for (name, img) in &parts {
        let staged = format!("{root}/{name}.img");
        let staged = stage(&img.url, &staged);
        verify(img, &staged);
        let dev = format!("/dev/block/by-name/{name}{tgt}");
        let n = write_part(&staged, &dev);
        println!("agupd: {name}: {n} bytes → {dev} (sha256 ok)");
        if staged != img.url {
            let _ = std::fs::remove_file(&staged);
        }
    }
    let st = Command::new("/usr/bin/agboot-ok")
        .arg("set-active").arg(tgt.trim_start_matches('_'))
        .status()
        .unwrap_or_else(|e| die(&format!("run agboot-ok: {e}")));
    if !st.success() {
        die("agboot-ok set-active failed — slots untouched, safe to retry");
    }
    println!("agupd: slot {tgt} staged — rebooting into update (auto-rollback after 7 unmarked boots)");
    if no_reboot {
        println!("agupd: --no-reboot given, not rebooting");
        return;
    }
    Command::new("/bin/reboot2").arg("reboot").status()
        .unwrap_or_else(|e| die(&format!("reboot2: {e}")));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("status") => {
            println!("slot {} version {}", active_slot(), current_version());
            let _ = Command::new("/usr/bin/agboot-ok").arg("status").status();
        }
        Some("apply") => {
            let src = args.get(1).unwrap_or_else(|| die("usage: agupd apply <manifest> [--no-reboot]"));
            cmd_apply(src, args.iter().any(|a| a == "--no-reboot"));
        }
        Some("sha256") => {
            // self-check against busybox sha256sum; also used to build manifests
            let f = args.get(1).unwrap_or_else(|| die("usage: agupd sha256 <file>"));
            println!("{}", sha256_hex_file(f));
        }
        Some("write-part") => {
            // escape hatch / test primitive: verified write, no slot logic
            let img = args.get(1).unwrap_or_else(|| die("usage: agupd write-part <img> <part>"));
            let part = args.get(2).unwrap_or_else(|| die("usage: agupd write-part <img> <part>"));
            let n = write_part(img, part);
            println!("agupd: {n} bytes → {part}");
        }
        _ => {
            eprintln!("usage: agupd <status|apply|write-part|sha256> …");
            std::process::exit(2);
        }
    }
}

/// SHA-256 (FIPS 180-4), streaming. In-crate on purpose: the updater must
/// not grow a dependency tree of its own.
struct Sha256 {
    h: [u32; 8],
    len: u64,
    buf: [u8; 64],
    n: usize,
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

impl Sha256 {
    fn new() -> Sha256 {
        Sha256 { h: [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19], len: 0, buf: [0; 64], n: 0 }
    }

    fn block(&mut self, b: &[u8]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = self.h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ ((!v[4]) & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v[7] = v[6]; v[6] = v[5]; v[5] = v[4];
            v[4] = v[3].wrapping_add(t1);
            v[3] = v[2]; v[2] = v[1]; v[1] = v[0];
            v[0] = t1.wrapping_add(t2);
        }
        for i in 0..8 {
            self.h[i] = self.h[i].wrapping_add(v[i]);
        }
    }

    fn update(&mut self, mut d: &[u8]) {
        self.len = self.len.wrapping_add(d.len() as u64);
        while !d.is_empty() {
            let take = (64 - self.n).min(d.len());
            self.buf[self.n..self.n + take].copy_from_slice(&d[..take]);
            self.n += take;
            d = &d[take..];
            if self.n == 64 {
                let b = self.buf;
                self.block(&b);
                self.n = 0;
            }
        }
    }

    fn finish(mut self) -> [u8; 32] {
        let bits = self.len.wrapping_mul(8);
        self.update(&[0x80]);
        while self.n != 56 {
            self.update(&[0]);
        }
        // update() would count the length bytes into len — irrelevant now
        let mut b = self.buf;
        b[56..64].copy_from_slice(&bits.to_be_bytes());
        self.block(&b);
        let mut out = [0u8; 32];
        for (i, h) in self.h.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&h.to_be_bytes());
        }
        out
    }
}
