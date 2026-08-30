// agboot-ok — mark the active A/B slot boot-successful in the GPT
// partition-table attributes (M16; the fastboot-loop fix).
//
// On redfin the slot state is NOT a bootloader_control block in misc —
// misc's vendor space holds recovery's "theme-dark" string and nothing
// else (probed 2026-08-31: no TCAB anywhere in misc, devinfo, ssd,
// uefivarstore, klog; only klog's UEFI log tail changes across boots).
// The store is the GPT itself: bootctrl.lito.so links gpt_disk_*/gpt_utils_*
// and keeps the slot flags in the partition-entry attribute u64 of every
// *_a / *_b entry, replicated across the per-LUN GPTs of /dev/sda../sdf
// (sdb/sdc carry the xbl chains). Observed bits:
//
//   48-51  priority
//   52-55  tries remaining (drains one per unmarked-successful boot)
//   56     successful boot
//
// Before this tool our boots ran unmarked: boot_a sat at pri=15 tries=0
// succ=0, one ABL view away from "slot a unbootable" — after which it
// falls through to slot b (stock boot_b + stock vendor_boot_b on our ext4
// userdata, which Android first_stage would format).
//
// `agboot-ok` sets successful + tries=7 on every *<suffix> entry of the
// active slot (suffix from androidboot.slot_suffix on the kernel cmdline),
// rewriting the primary and backup entry arrays with refreshed CRCs and
// both GPT headers. Attribute bits only — start/length/name are never
// touched, so the running kernel's partition view stays valid.
// `agboot-ok status` dumps the whole slot table read-only.
use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;

const TRIES: u64 = 7;
const ATTR_TRIES: u64 = 52;
const ATTR_SUCCESS: u64 = 56;

struct Gpt {
    dev: String,
    /// Logical block size — the redfin UFS LUNs are 4K-block, so the GPT
    /// header sits at byte 4096, not 512. Read per disk, never assumed.
    lbs: u64,
    hdr: Vec<u8>,
    entries: Vec<u8>,
    entry_lba: u64,
    num: usize,
    esz: usize,
    /// Backup GPT: its entries LBA and header LBA.
    bak_entries_lba: u64,
    bak_hdr_lba: u64,
    bak_hdr: Vec<u8>,
}

fn rd(f: &File, buf: &mut [u8], off: u64) -> bool {
    let n = unsafe { libc::pread(f.as_raw_fd(), buf.as_mut_ptr() as *mut _, buf.len() as _, off as libc::off_t) };
    n == buf.len() as isize
}

fn wr(f: &File, buf: &[u8], off: u64) -> bool {
    let n = unsafe { libc::pwrite(f.as_raw_fd(), buf.as_ptr() as *const _, buf.len() as _, off as libc::off_t) };
    n == buf.len() as isize
}

fn u32at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn u64at(b: &[u8], o: usize) -> u64 {
    let mut v = [0u8; 8];
    v.copy_from_slice(&b[o..o + 8]);
    u64::from_le_bytes(v)
}

fn put32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

impl Gpt {
    fn open(dev: &str) -> Option<Gpt> {
        let f = File::open(dev).ok()?;
        let lbs = std::fs::read_to_string(format!(
            "/sys/class/block/{}/queue/logical_block_size",
            dev.trim_start_matches("/dev/")
        ))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|&b| b >= 512)
        .unwrap_or(512);
        let mut hdr = vec![0u8; lbs as usize];
        if !rd(&f, &mut hdr, lbs) {
            if std::env::var("AGBOOT_DEBUG").is_ok() { eprintln!("dbg: {dev}: header read failed"); }
            return None; // not a GPT disk (or not a disk at all)
        }
        if &hdr[0..8] != b"EFI PART" {
            if std::env::var("AGBOOT_DEBUG").is_ok() { eprintln!("dbg: {dev}: bad sig {:?}", &hdr[0..8]); }
            return None;
        }
        let num = u32at(&hdr, 80) as usize;
        let esz = u32at(&hdr, 84) as usize;
        if num == 0 || esz == 0 || num * esz > 1 << 20 {
            return None;
        }
        let entry_lba = u64at(&hdr, 72);
        let mut entries = vec![0u8; num * esz];
        if !rd(&f, &mut entries, entry_lba * lbs) {
            if std::env::var("AGBOOT_DEBUG").is_ok() { eprintln!("dbg: {dev}: entries read failed at lba {entry_lba}"); }
            return None;
        }
        if u32at(&hdr, 88) != crc32(&entries) {
            eprintln!("agboot-ok: {dev}: primary entries crc mismatch — skipping");
            return None;
        }
        let bak_hdr_lba = u64at(&hdr, 32);
        let mut bak_hdr = vec![0u8; lbs as usize];
        if !rd(&f, &mut bak_hdr, bak_hdr_lba * lbs) || &bak_hdr[0..8] != b"EFI PART" {
            if std::env::var("AGBOOT_DEBUG").is_ok() { eprintln!("dbg: {dev}: backup header unreadable at lba {bak_hdr_lba}"); }
            return None; // truncated tail read; skip disk rather than guess
        }
        let bak_entries_lba = u64at(&bak_hdr, 72);
        Some(Gpt { dev: dev.to_string(), lbs, hdr, entries, entry_lba, num, esz, bak_entries_lba, bak_hdr_lba, bak_hdr })
    }

    /// (name, attrs, entry offset) for every non-empty entry.
    fn entries(&self) -> Vec<(String, u64, usize)> {
        let mut out = Vec::new();
        for i in 0..self.num {
            let e = i * self.esz;
            let raw = &self.entries[e + 56..e + 128];
            if raw.iter().all(|&b| b == 0) {
                continue;
            }
            let name: String = raw
                .chunks_exact(2)
                .take_while(|c| c != &[0, 0])
                .map(|c| u16::from_le_bytes([c[0], c[1]]) as u8 as char)
                .collect();
            out.push((name, u64at(&self.entries, e + 48), e));
        }
        out
    }

    fn set_attrs(&mut self, off: usize, attrs: u64) {
        self.entries[off + 48..off + 56].copy_from_slice(&attrs.to_le_bytes());
    }

    /// Write entries + both CRC fields back to primary and backup GPT.
    fn commit(&mut self, f: &File) -> Result<(), String> {
        let ecrc = crc32(&self.entries);
        let lbs = self.lbs;
        for (hdr, lba) in [(self.hdr.as_mut_slice(), 1u64), (self.bak_hdr.as_mut_slice(), self.bak_hdr_lba)] {
            put32(hdr, 88, ecrc);
            put32(hdr, 16, 0);
            let hcrc = crc32(&hdr[..92]);
            put32(hdr, 16, hcrc);
            if !wr(f, hdr, lba * lbs) {
                return Err(format!("header write at lba {lba}"));
            }
        }
        for (buf, lba) in [(self.entries.as_slice(), self.entry_lba), (self.entries.as_slice(), self.bak_entries_lba)] {
            if !wr(f, buf, lba * lbs) {
                return Err(format!("entries write at lba {lba}"));
            }
        }
        unsafe { libc::fsync(f.as_raw_fd()) };
        Ok(())
    }
}

fn main() {
    let dbg = std::env::var("AGBOOT_DEBUG").is_ok();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let status = args.first().map(String::as_str) == Some("status");

    let suffix = std::fs::read_to_string("/proc/cmdline")
        .ok()
        .and_then(|c| {
            c.split_whitespace().find_map(|t| {
                let v = t.strip_prefix("androidboot.slot_suffix=")?;
                let v = v.trim_matches('"');
                (v == "_a" || v == "_b").then(|| v.to_string())
            })
        })
        .unwrap_or_else(|| {
            eprintln!("agboot-ok: no androidboot.slot_suffix on cmdline, assuming _a");
            "_a".to_string()
        });

    // Every LUN holding an A/B GPT. Names, not /dev/sdX guesses: resolve
    // through /dev/block/by-name so we only touch disks we can see.
    let mut disks: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/dev/block/by-name") {
        for e in rd.filter_map(|e| e.ok()) {
            if let Ok(t) = std::fs::read_link(e.path()) {
                if let Some(d) = t.to_str().and_then(|s| s.strip_prefix("/dev/")) {
                    let d = d.trim_start_matches("sd").trim_end_matches(|c: char| c.is_ascii_digit());
                    let dev = format!("/dev/sd{d}");
                    if !disks.contains(&dev) {
                        disks.push(dev);
                    }
                }
            }
        }
    }
    disks.sort();
    if dbg {
        eprintln!("dbg: suffix={suffix} disks={disks:?}");
    }

    let mut marked = 0usize;
    for dev in &disks {
        let mut g = match Gpt::open(dev) {
            Some(g) => g,
            None => continue,
        };
        let targets: Vec<(String, u64, usize)> = g
            .entries()
            .into_iter()
            .filter(|(n, _, _)| n.ends_with(&suffix))
            .collect();
        if targets.is_empty() {
            continue;
        }
        if status {
            for (n, a, _) in g.entries() {
                if n.ends_with("_a") || n.ends_with("_b") {
                    println!(
                        "{:14} {:18} pri={} tries={} succ={} unboot={}",
                        dev,
                        n,
                        (a >> 48) & 0xF,
                        (a >> 52) & 0xF,
                        (a >> 56) & 1,
                        (a >> 57) & 1
                    );
                }
            }
            continue;
        }
        let f = OpenOptions::new().write(true).open(dev).unwrap_or_else(|e| {
            eprintln!("agboot-ok: open {dev} rw: {e}");
            std::process::exit(1);
        });
        for (n, a, off) in &targets {
            let na = ((*a & !(0xF << ATTR_TRIES)) | (TRIES << ATTR_TRIES)) | (1 << ATTR_SUCCESS);
            g.set_attrs(*off, na);
            println!("agboot-ok: {dev} {n}: tries {}→{} succ {}→1", (a >> 52) & 0xF, TRIES, (a >> 56) & 1);
        }
        match g.commit(&f) {
            Ok(()) => marked += targets.len(),
            Err(e) => eprintln!("agboot-ok: {dev}: {e} — NOT committed"),
        }
    }
    if !status {
        if marked == 0 {
            eprintln!("agboot-ok: no *{suffix} entries found — nothing marked");
            std::process::exit(1);
        }
        println!("agboot-ok: slot {suffix} marked successful on {marked} entries");
        let _ = dbg;
    }
}

/// Standard CRC-32 (reflected 0xEDB88320, init/xorout 0xFFFFFFFF) — the
/// same zlib crc32 the GPT spec and gpt_utils use.
fn crc32(buf: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, t) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *t = c;
    }
    let mut crc = 0xFFFF_FFFFu32;
    for &b in buf {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}
