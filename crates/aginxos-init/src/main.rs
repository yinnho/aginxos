//! AginxOS early init (pid 1).
//!
//! - Mount essentials
//! - Paint a solid color via DRM dumb buffer (or fbdev fallback)
//! - Optionally hand off to stock Android first-stage init (`/init.android`)

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;

fn klog(msg: &str) {
    let line = format!("aginxos-init: {msg}\n");
    // Always mirror to stdout when available (adb shell splash-test).
    let _ = std::io::stdout().write_all(line.as_bytes());
    let _ = std::io::stdout().flush();
    if let Ok(mut f) = OpenOptions::new().write(true).open("/dev/kmsg") {
        let _ = f.write_all(line.as_bytes());
    }
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
            "mount {fstype}->{target} errno={}",
            std::io::Error::last_os_error()
        ));
    } else {
        klog(&format!("mounted {fstype}->{target}"));
    }
}

/// Try Linux framebuffer fill (BGRX/RGBX 32-bit).
fn try_fb_splash(color: u32) -> Result<(), String> {
    let mut fb = File::options()
        .read(true)
        .write(true)
        .open("/dev/fb0")
        .map_err(|e| format!("open fb0: {e}"))?;

    // struct fb_var_screeninfo / fb_fix_screeninfo — only grab sizes via FBIOPUT-free reads.
    // FBIOPUT not required for fill; use FBIOGET_VSCREENINFO / FBIOGET_FSCREENINFO.
    const FBIOGET_VSCREENINFO: libc::Ioctl = 0x4600 as libc::Ioctl;
    const FBIOGET_FSCREENINFO: libc::Ioctl = 0x4602 as libc::Ioctl;

    #[repr(C)]
    #[derive(Default)]
    struct FbVar {
        xres: u32,
        yres: u32,
        xres_virtual: u32,
        yres_virtual: u32,
        xoffset: u32,
        yoffset: u32,
        bits_per_pixel: u32,
        grayscale: u32,
        // rest ignored; keep large enough buffer
        rest: [u32; 32],
    }

    #[repr(C)]
    #[derive(Default)]
    struct FbFix {
        id: [u8; 16],
        smem_start: usize,
        smem_len: u32,
        rest: [u32; 32],
    }

    let mut var = FbVar::default();
    let mut fix = FbFix::default();
    let fd = fb.as_raw_fd();
    let rc = unsafe { libc::ioctl(fd, FBIOGET_VSCREENINFO, &mut var as *mut _) };
    if rc != 0 {
        return Err(format!("FBIOGET_VSCREENINFO: {}", std::io::Error::last_os_error()));
    }
    let rc = unsafe { libc::ioctl(fd, FBIOGET_FSCREENINFO, &mut fix as *mut _) };
    if rc != 0 {
        return Err(format!("FBIOGET_FSCREENINFO: {}", std::io::Error::last_os_error()));
    }

    let bpp = var.bits_per_pixel;
    let w = var.xres;
    let h = var.yres;
    let len = fix.smem_len as usize;
    if w == 0 || h == 0 || len == 0 {
        return Err(format!("bad fb geometry {w}x{h} len={len} bpp={bpp}"));
    }
    klog(&format!("fb0 {w}x{h} bpp={bpp} smem_len={len}"));

    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        return Err(format!("mmap fb: {}", std::io::Error::last_os_error()));
    }

    unsafe {
        if bpp == 32 || bpp == 24 {
            let pixels = (len / 4).min((w * h) as usize);
            let slice = std::slice::from_raw_parts_mut(ptr as *mut u32, pixels);
            for p in slice.iter_mut() {
                *p = color;
            }
        } else if bpp == 16 {
            // rough RGB565 from 0x00RRGGBB
            let r = ((color >> 16) & 0xff) as u16;
            let g = ((color >> 8) & 0xff) as u16;
            let b = (color & 0xff) as u16;
            let c16 = ((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3);
            let pixels = (len / 2).min((w * h) as usize);
            let slice = std::slice::from_raw_parts_mut(ptr as *mut u16, pixels);
            for p in slice.iter_mut() {
                *p = c16;
            }
        } else {
            libc::munmap(ptr, len);
            return Err(format!("unsupported bpp {bpp}"));
        }
        libc::munmap(ptr, len);
    }
    let _ = fb.flush();
    Ok(())
}

// --- Minimal DRM dumb-buffer splash (no libdrm) ---

const DRM_IOCTL_BASE: u8 = b'd';
const fn drm_iowr(nr: u8, size: usize) -> libc::Ioctl {
    // _IOWR('d', nr, size) on linux
    let dir: u64 = 3; // write|read
    let typ = DRM_IOCTL_BASE as u64;
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
struct DrmModeGetConnector {
    encoders_ptr: u64,
    modes_ptr: u64,
    props_ptr: u64,
    prop_values_ptr: u64,
    count_modes: u32,
    count_props: u32,
    count_encoders: u32,
    encoder_id: u32,
    connector_id: u32,
    connector_type: u32,
    connector_type_id: u32,
    connection: u32,
    mm_width: u32,
    mm_height: u32,
    subpixel: u32,
    pad: u32,
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

fn ioctl_raw(fd: RawFd, req: libc::Ioctl, arg: *mut libc::c_void) -> i32 {
    unsafe { libc::ioctl(fd, req, arg) }
}

fn try_drm_splash(color: u32) -> Result<(), String> {
    // Prefer card0; fall through cards.
    let mut last_err = String::from("no /dev/dri/card*");
    for n in 0..4 {
        let path = format!("/dev/dri/card{n}");
        if !Path::new(&path).exists() {
            continue;
        }
        match drm_splash_on_card(&path, color) {
            Ok(()) => return Ok(()),
            Err(e) => {
                klog(&format!("drm {path}: {e}"));
                last_err = e;
            }
        }
    }
    Err(last_err)
}

fn drm_splash_on_card(path: &str, color: u32) -> Result<(), String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("open: {e}"))?;
    let fd = file.as_raw_fd();

    // Become DRM master if possible (ignore failure).
    const DRM_IOCTL_SET_MASTER: libc::Ioctl = 0x641e as libc::Ioctl; // _IO('d', 0x1e)
    let _ = unsafe { libc::ioctl(fd, DRM_IOCTL_SET_MASTER) };
    const DRM_IOCTL_DROP_MASTER: libc::Ioctl = 0x641f as libc::Ioctl;
    let _ = DRM_IOCTL_DROP_MASTER;

    // GETRESOURCES
    let req_get_res = drm_iowr(0xA0, std::mem::size_of::<DrmModeCardRes>());
    let mut res = unsafe { std::mem::zeroed::<DrmModeCardRes>() };
    if ioctl_raw(fd, req_get_res, &mut res as *mut _ as *mut _) != 0 {
        return Err(format!("GETRESOURCES count: {}", std::io::Error::last_os_error()));
    }
    let mut connector_ids = vec![0u32; res.count_connectors as usize];
    let mut crtc_ids = vec![0u32; res.count_crtcs as usize];
    let mut encoder_ids = vec![0u32; res.count_encoders as usize];
    res.connector_id_ptr = connector_ids.as_mut_ptr() as u64;
    res.crtc_id_ptr = crtc_ids.as_mut_ptr() as u64;
    res.encoder_id_ptr = encoder_ids.as_mut_ptr() as u64;
    res.fb_id_ptr = 0;
    if ioctl_raw(fd, req_get_res, &mut res as *mut _ as *mut _) != 0 {
        return Err(format!("GETRESOURCES ids: {}", std::io::Error::last_os_error()));
    }
    klog(&format!(
        "drm {path} crtcs={} connectors={} encoders={}",
        crtc_ids.len(),
        connector_ids.len(),
        encoder_ids.len()
    ));
    if crtc_ids.is_empty() {
        return Err("no crtcs".into());
    }

    // Path A (best for continuous splash): reuse an already-programmed CRTC mode.
    let req_getcrtc = drm_iowr(0xA1, std::mem::size_of::<DrmModeCrtc>());
    let req_setcrtc = drm_iowr(0xA2, std::mem::size_of::<DrmModeCrtc>());
    let req_create = drm_iowr(0xB2, std::mem::size_of::<DrmModeCreateDumb>());
    let req_addfb = drm_iowr(0xAE, std::mem::size_of::<DrmModeFbCmd>());
    let req_map = drm_iowr(0xB3, std::mem::size_of::<DrmModeMapDumb>());

    let mut active: Option<(u32, DrmModeModeInfo, u32 /*width*/, u32 /*height*/)> = None;
    for &crtc_id in &crtc_ids {
        let mut crtc = unsafe { std::mem::zeroed::<DrmModeCrtc>() };
        crtc.crtc_id = crtc_id;
        if ioctl_raw(fd, req_getcrtc, &mut crtc as *mut _ as *mut _) != 0 {
            continue;
        }
        if crtc.mode_valid == 0 {
            continue;
        }
        let w = crtc.mode.hdisplay as u32;
        let h = crtc.mode.vdisplay as u32;
        if w == 0 || h == 0 {
            continue;
        }
        klog(&format!(
            "drm active crtc={crtc_id} fb={} mode={w}x{h}",
            crtc.fb_id
        ));
        active = Some((crtc_id, crtc.mode, w, h));
        break;
    }

    // Path B: pick first connector mode if nothing active yet.
    let mut chosen_conn = 0u32;
    if active.is_none() {
        let req_get_conn = drm_iowr(0xA7, std::mem::size_of::<DrmModeGetConnector>());
        for &cid in &connector_ids {
            let mut conn = unsafe { std::mem::zeroed::<DrmModeGetConnector>() };
            conn.connector_id = cid;
            if ioctl_raw(fd, req_get_conn, &mut conn as *mut _ as *mut _) != 0 {
                continue;
            }
            if conn.count_modes == 0 {
                continue;
            }
            let mut modes =
                vec![unsafe { std::mem::zeroed::<DrmModeModeInfo>() }; conn.count_modes as usize];
            conn.modes_ptr = modes.as_mut_ptr() as u64;
            if ioctl_raw(fd, req_get_conn, &mut conn as *mut _ as *mut _) != 0 {
                continue;
            }
            if modes.is_empty() {
                continue;
            }
            let mode = modes[0];
            let w = mode.hdisplay as u32;
            let h = mode.vdisplay as u32;
            if w == 0 || h == 0 {
                continue;
            }
            chosen_conn = cid;
            active = Some((crtc_ids[0], mode, w, h));
            klog(&format!("drm connector={cid} mode={w}x{h}"));
            break;
        }
    }

    let Some((crtc_id, mode, width, height)) = active else {
        return Err("no active CRTC/mode".into());
    };

    // CREATE_DUMB + fill
    let mut dumb = DrmModeCreateDumb {
        height,
        width,
        bpp: 32,
        flags: 0,
        handle: 0,
        pitch: 0,
        size: 0,
    };
    if ioctl_raw(fd, req_create, &mut dumb as *mut _ as *mut _) != 0 {
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
    if ioctl_raw(fd, req_addfb, &mut fb as *mut _ as *mut _) != 0 {
        return Err(format!("ADDFB: {}", std::io::Error::last_os_error()));
    }

    let mut map = DrmModeMapDumb {
        handle: dumb.handle,
        pad: 0,
        offset: 0,
    };
    if ioctl_raw(fd, req_map, &mut map as *mut _ as *mut _) != 0 {
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
        return Err(format!("mmap dumb: {}", std::io::Error::last_os_error()));
    }
    unsafe {
        let pitch_px = (dumb.pitch / 4) as usize;
        let base = ptr as *mut u32;
        for y in 0..height as usize {
            for x in 0..width as usize {
                *base.add(y * pitch_px + x) = color;
            }
        }
        // Draw a thick white border so it's obvious even if color is wrong.
        let border = 0x00FF_FFFF;
        let t = 20usize.min(width as usize / 10).min(height as usize / 10);
        for y in 0..height as usize {
            for x in 0..width as usize {
                if x < t || y < t || x >= width as usize - t || y >= height as usize - t {
                    *base.add(y * pitch_px + x) = border;
                }
            }
        }
        libc::munmap(ptr, len);
    }

    // SETCRTC with new fb (reuse mode).
    let mut conn_for_crtc = if chosen_conn != 0 {
        vec![chosen_conn]
    } else if !connector_ids.is_empty() {
        vec![connector_ids[0]]
    } else {
        Vec::new()
    };
    let mut crtc = unsafe { std::mem::zeroed::<DrmModeCrtc>() };
    crtc.crtc_id = crtc_id;
    crtc.fb_id = fb.fb_id;
    crtc.mode = mode;
    crtc.mode_valid = 1;
    crtc.x = 0;
    crtc.y = 0;
    if !conn_for_crtc.is_empty() {
        crtc.set_connectors_ptr = conn_for_crtc.as_mut_ptr() as u64;
        crtc.count_connectors = conn_for_crtc.len() as u32;
    }
    if ioctl_raw(fd, req_setcrtc, &mut crtc as *mut _ as *mut _) != 0 {
        let err = std::io::Error::last_os_error();
        // Try without connector list (some drivers accept fb replace only).
        crtc.set_connectors_ptr = 0;
        crtc.count_connectors = 0;
        if ioctl_raw(fd, req_setcrtc, &mut crtc as *mut _ as *mut _) != 0 {
            return Err(format!("SETCRTC: {err} / {}", std::io::Error::last_os_error()));
        }
    }

    klog(&format!(
        "drm splash ok crtc={crtc_id} fb_id={} {width}x{height} color={color:#08x}",
        fb.fb_id
    ));
    Ok(())
}

fn write_boot_marker() {
    // Survives into early Android if metadata is usable; best-effort.
    for path in [
        "/aginxos/BOOTED",
        "/dev/aginxos_booted",
        "/metadata/aginxos_booted",
    ] {
        if let Some(parent) = Path::new(path).parent() {
            let _ = fs::create_dir_all(parent);
        }
        if fs::write(path, b"aginxos-init\n").is_ok() {
            klog(&format!("marker written {path}"));
        }
    }
    // pmsg is world-writable on Android; format is free-text for some kernels.
    if let Ok(mut f) = OpenOptions::new().write(true).open("/dev/pmsg0") {
        let _ = f.write_all(b"aginxos-init: boot marker\n");
    }
}

fn list_dir(label: &str, path: &str) {
    match fs::read_dir(path) {
        Ok(rd) => {
            let names: Vec<_> = rd
                .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
                .collect();
            klog(&format!("{label}: {}", names.join(",")));
        }
        Err(e) => klog(&format!("{label}: {e}")),
    }
}

fn wait_for_path(path: &str, total_ms: u64) -> bool {
    let step = 100;
    let mut waited = 0;
    while waited <= total_ms {
        if Path::new(path).exists() {
            return true;
        }
        thread::sleep(Duration::from_millis(step));
        waited += step;
    }
    Path::new(path).exists()
}

fn try_sysfs_backlight_max() {
    let roots = ["/sys/class/backlight", "/sys/devices"];
    for root in roots {
        let Ok(rd) = fs::read_dir(root) else { continue };
        for ent in rd.flatten() {
            let p = ent.path().join("brightness");
            if p.exists() {
                let max_p = ent.path().join("max_brightness");
                let val = fs::read_to_string(&max_p)
                    .ok()
                    .and_then(|s| s.trim().parse::<u32>().ok())
                    .unwrap_or(255);
                if fs::write(&p, format!("{val}")).is_ok() {
                    klog(&format!("backlight {} -> {val}", p.display()));
                }
            }
        }
    }
    // Direct known Pixel path
    let p = Path::new("/sys/class/backlight/panel0-backlight/brightness");
    if p.exists() {
        let _ = fs::write(p, "4095");
        let _ = fs::write(p, "255");
        let _ = fs::write(p, "2047");
        klog("wrote panel0-backlight brightness");
    }
}

fn paint_splash(color: u32) -> bool {
    try_sysfs_backlight_max();
    // Poll for DRM node — driver may probe late in early boot.
    let _ = wait_for_path("/dev/dri/card0", 3000);
    match try_drm_splash(color) {
        Ok(()) => {
            klog("splash: DRM ok");
            true
        }
        Err(e) => {
            klog(&format!("splash: DRM failed ({e}), try fb"));
            for fb in ["/dev/fb0", "/dev/graphics/fb0"] {
                if !Path::new(fb).exists() {
                    continue;
                }
                // try_fb_splash opens /dev/fb0 only — temporarily symlink not possible without root dirs.
                if fb == "/dev/fb0" {
                    match try_fb_splash(color) {
                        Ok(()) => {
                            klog("splash: fb0 ok");
                            return true;
                        }
                        Err(e2) => klog(&format!("splash: fb0 failed ({e2})")),
                    }
                }
            }
            false
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let splash_test = args.iter().any(|a| a == "splash-test");
    let hold_only = args.iter().any(|a| a == "hold") || Path::new("/aginxos/hold").exists();

    klog(&format!(
        "starting v{} pid={} splash_test={splash_test} hold_only={hold_only}",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    ));

    if !splash_test {
        // Only mount if not already present (hybrid handoff to Android init).
        if !Path::new("/proc/self").exists() {
            mount("proc", "proc", "/proc", 0, "");
        }
        if !Path::new("/sys/class").exists() {
            mount("sysfs", "sysfs", "/sys", 0, "");
        }
        if !Path::new("/dev/null").exists() {
            mount("devtmpfs", "devtmpfs", "/dev", 0, "mode=0755");
            if !Path::new("/dev/null").exists() {
                mount("tmpfs", "tmpfs", "/dev", 0, "mode=0755");
            }
        }
        mkdir_p("/dev/pts");
        let _ = mount("devpts", "devpts", "/dev/pts", 0, "");
        mkdir_p("/tmp");
    }

    klog("filesystems ready");
    list_dir("input", "/dev/input");
    list_dir("dri", "/dev/dri");
    list_dir("graphics", "/dev/graphics");
    write_boot_marker();

    // Cycle colors so a human can notice even if one frame is missed.
    let colors = [
        0x00_22_CC_44, // green
        0x00_CC_22_22, // red
        0x00_22_44_CC, // blue
        0x00_EE_EE_22, // yellow
    ];
    for (i, color) in colors.iter().enumerate() {
        klog(&format!("paint frame {i} color={color:#08x}"));
        let ok = paint_splash(*color);
        klog(&format!("paint frame {i} ok={ok}"));
        thread::sleep(Duration::from_millis(if splash_test { 800 } else { 2500 }));
    }

    if splash_test {
        klog("splash-test done, exiting");
        return;
    }

    if Path::new("/bin/aginxos-probe").exists() || Path::new("/aginxos/aginxos-probe").exists() {
        let probe = if Path::new("/bin/aginxos-probe").exists() {
            "/bin/aginxos-probe"
        } else {
            "/aginxos/aginxos-probe"
        };
        klog(&format!("running {probe}"));
        match Command::new(probe).output() {
            Ok(out) => {
                for line in String::from_utf8_lossy(&out.stdout).lines() {
                    klog(line);
                }
            }
            Err(e) => klog(&format!("probe failed: {e}")),
        }
    }

    // Hold so human can see last color, then hand off if hybrid ramdisk.
    klog("holding last frame 6s");
    thread::sleep(Duration::from_secs(6));

    if hold_only {
        klog("hold mode — not handing off (long-press power to leave)");
        loop {
            thread::sleep(Duration::from_secs(30));
            klog("heartbeat");
        }
    }

    for handoff in [
        "/init.android",
        "/system/bin/init.android",
        "/system/bin/init",
    ] {
        if Path::new(handoff).exists() {
            klog(&format!("handoff -> {handoff}"));
            let err = Command::new(handoff).exec();
            klog(&format!("handoff {handoff} failed: {err}"));
        }
    }

    klog("no android init found — bring-up hold (long-press power to leave)");
    loop {
        thread::sleep(Duration::from_secs(30));
        klog("heartbeat");
    }
}
