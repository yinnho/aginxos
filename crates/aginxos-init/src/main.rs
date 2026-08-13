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
    if connector_ids.is_empty() || crtc_ids.is_empty() {
        return Err("no connectors/crtcs".into());
    }

    // Find first connected connector with modes.
    let req_get_conn = drm_iowr(0xA7, std::mem::size_of::<DrmModeGetConnector>());
    let mut chosen_conn = 0u32;
    let mut mode = unsafe { std::mem::zeroed::<DrmModeModeInfo>() };
    let mut encoder_id = 0u32;
    for &cid in &connector_ids {
        let mut conn = unsafe { std::mem::zeroed::<DrmModeGetConnector>() };
        conn.connector_id = cid;
        if ioctl_raw(fd, req_get_conn, &mut conn as *mut _ as *mut _) != 0 {
            continue;
        }
        if conn.count_modes == 0 {
            continue;
        }
        let mut modes = vec![unsafe { std::mem::zeroed::<DrmModeModeInfo>() }; conn.count_modes as usize];
        let mut encs = vec![0u32; conn.count_encoders as usize];
        conn.modes_ptr = modes.as_mut_ptr() as u64;
        conn.encoders_ptr = encs.as_mut_ptr() as u64;
        if ioctl_raw(fd, req_get_conn, &mut conn as *mut _ as *mut _) != 0 {
            continue;
        }
        // connection: 1 = connected
        if conn.connection != 1 && conn.connection != 2 {
            // 2 unknown — still try
        }
        if modes.is_empty() {
            continue;
        }
        chosen_conn = cid;
        mode = modes[0];
        encoder_id = conn.encoder_id;
        if encoder_id == 0 && !encs.is_empty() {
            encoder_id = encs[0];
        }
        break;
    }
    if chosen_conn == 0 {
        return Err("no usable connector".into());
    }

    let width = mode.hdisplay as u32;
    let height = mode.vdisplay as u32;
    if width == 0 || height == 0 {
        return Err(format!("bad mode {width}x{height}"));
    }
    klog(&format!(
        "drm {path} connector={chosen_conn} mode={width}x{height}"
    ));

    // CREATE_DUMB
    let req_create = drm_iowr(0xB2, std::mem::size_of::<DrmModeCreateDumb>());
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

    // ADDFB
    let req_addfb = drm_iowr(0xAE, std::mem::size_of::<DrmModeFbCmd>());
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

    // MAP_DUMB + mmap
    let req_map = drm_iowr(0xB3, std::mem::size_of::<DrmModeMapDumb>());
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
        let pixels = len / 4;
        let slice = std::slice::from_raw_parts_mut(ptr as *mut u32, pixels);
        // DRM often XRGB8888 little-endian: 0x00RRGGBB works on many panels.
        for p in slice.iter_mut() {
            *p = color;
        }
        libc::munmap(ptr, len);
    }

    // SETCRTC
    let req_setcrtc = drm_iowr(0xA2, std::mem::size_of::<DrmModeCrtc>());
    let mut conn_for_crtc = [chosen_conn];
    let mut crtc = unsafe { std::mem::zeroed::<DrmModeCrtc>() };
    crtc.crtc_id = crtc_ids[0];
    crtc.fb_id = fb.fb_id;
    crtc.set_connectors_ptr = conn_for_crtc.as_mut_ptr() as u64;
    crtc.count_connectors = 1;
    crtc.mode = mode;
    crtc.mode_valid = 1;
    crtc.x = 0;
    crtc.y = 0;
    // Prefer CRTC linked via encoder if we can guess.
    let _ = encoder_id;
    if ioctl_raw(fd, req_setcrtc, &mut crtc as *mut _ as *mut _) != 0 {
        // try each crtc
        let mut ok = false;
        for &crtc_id in &crtc_ids {
            crtc.crtc_id = crtc_id;
            if ioctl_raw(fd, req_setcrtc, &mut crtc as *mut _ as *mut _) == 0 {
                ok = true;
                break;
            }
        }
        if !ok {
            return Err(format!("SETCRTC: {}", std::io::Error::last_os_error()));
        }
    }

    klog(&format!("drm splash ok fb_id={} color={color:#08x}", fb.fb_id));
    Ok(())
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

fn main() {
    klog(&format!(
        "starting v{} pid={}",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    ));

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
    if !Path::new("/tmp").is_dir() {
        mount("tmpfs", "tmpfs", "/tmp", 0, "");
    }

    klog("filesystems ready");
    list_dir("input", "/dev/input");
    list_dir("dri", "/dev/dri");

    // Bright green (XRGB) — obvious "AginxOS init is alive" signal.
    let color = 0x00_22_CC_44;
    match try_drm_splash(color) {
        Ok(()) => klog("splash: DRM ok"),
        Err(e) => {
            klog(&format!("splash: DRM failed ({e}), try fb0"));
            match try_fb_splash(color) {
                Ok(()) => klog("splash: fb0 ok"),
                Err(e2) => klog(&format!("splash: fb0 failed ({e2})")),
            }
        }
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

    // Hold splash so human can see it, then hand off if hybrid ramdisk.
    klog("holding splash 4s");
    thread::sleep(Duration::from_secs(4));

    if Path::new("/init.android").exists() {
        // Drop our mounts carefully? Android first-stage usually tolerates existing mounts.
        let _ = Command::new("/init.android").exec();
        klog("handoff returned unexpectedly");
    }

    klog("no /init.android — staying in bring-up hold (long-press power to leave)");
    loop {
        thread::sleep(Duration::from_secs(30));
        klog("heartbeat");
    }
}
