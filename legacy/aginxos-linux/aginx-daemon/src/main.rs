//! Aginx Daemon — Linux userspace agent runtime
//!
//! Runs as PID 1 (init) or a regular daemon on any Linux system.
//! Supports: aarch64, x86_64, and any Linux target.
//!
//! Usage:
//!   ./aginx-daemon                    # interactive shell on stdin
//!   ./aginx-daemon --listen 9091      # TCP shell server
//!   ./aginx-daemon --listen 9091 --shell  # both

mod shell;
mod tcp_server;
mod task;
mod proto;
mod wifi;
mod cellular;

use std::os::fd::FromRawFd;
use std::env;

const VERSION: &str = "0.1.0";
const DEFAULT_PORT: u16 = 9091;

fn print_banner() {
    println!("Aginx Daemon v{} [{}]", VERSION, env::consts::ARCH);
    println!("  Linux {} ({})", sys_info().0, sys_info().1);
}

/// Get (kernel_version, hostname)
fn sys_info() -> (String, String) {
    let kernel = unsafe {
        let mut uts: libc::utsname = std::mem::zeroed();
        libc::uname(&mut uts);
        let bytes: &[u8] = &*(&uts.release as *const _ as *const [u8; 256]);
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(256);
        String::from_utf8_lossy(&bytes[..end]).into_owned()
    };
    let hostname = std::fs::read_to_string("/etc/hostname")
        .unwrap_or_else(|_| "localhost".into())
        .trim()
        .to_string();
    (kernel, hostname)
}

fn usage() {
    eprintln!("Usage: aginx-daemon [OPTIONS]");
    eprintln!("  --listen PORT   Start TCP shell server (default: {})", DEFAULT_PORT);
    eprintln!("  --shell         Interactive stdin shell");
    eprintln!("  --daemon        Run as background daemon");
    eprintln!("  --help          Show this help");
}

// ── PID 1 (init) support ──────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn mount_filesystems() {
    use std::fs;
    let mounts = [
        ("proc", "/proc", "proc"),
        ("sysfs", "/sys", "sysfs"),
        ("devtmpfs", "/dev", "devtmpfs"),
        ("tmpfs", "/tmp", "tmpfs"),
    ];
    for (source, target, fstype) in &mounts {
        fs::create_dir_all(target).ok();
        let c_source = std::ffi::CString::new(*source).unwrap();
        let c_target = std::ffi::CString::new(*target).unwrap();
        let c_fstype = std::ffi::CString::new(*fstype).unwrap();
        unsafe {
            let ret = libc::mount(
                c_source.as_ptr(), c_target.as_ptr(), c_fstype.as_ptr(),
                0, std::ptr::null(),
            );
            if ret == 0 {
                eprintln!("[init] Mounted {}", target);
            } else {
                eprintln!("[init] mount {} failed: {}", target, std::io::Error::last_os_error());
            }
        }
    }
    // Create /dev/pts
    fs::create_dir_all("/dev/pts").ok();
    let c_pts = std::ffi::CString::new("devpts").unwrap();
    let c_pts_target = std::ffi::CString::new("/dev/pts").unwrap();
    let c_pts_type = std::ffi::CString::new("devpts").unwrap();
    unsafe {
        libc::mount(c_pts.as_ptr(), c_pts_target.as_ptr(), c_pts_type.as_ptr(), 0, std::ptr::null());
    }
    // Mount configfs for USB gadget
    fs::create_dir_all("/sys/kernel/config").ok();
    let c_cfg = std::ffi::CString::new("configfs").unwrap();
    let c_cfg_target = std::ffi::CString::new("/sys/kernel/config").unwrap();
    let c_cfg_type = std::ffi::CString::new("configfs").unwrap();
    unsafe {
        let ret = libc::mount(c_cfg.as_ptr(), c_cfg_target.as_ptr(), c_cfg_type.as_ptr(), 0, std::ptr::null());
        if ret == 0 {
            eprintln!("[init] Mounted configfs");
        }
    }
    // Mount debugfs
    fs::create_dir_all("/sys/kernel/debug").ok();
    let c_dbg = std::ffi::CString::new("debugfs").unwrap();
    let c_dbg_target = std::ffi::CString::new("/sys/kernel/debug").unwrap();
    let c_dbg_type = std::ffi::CString::new("debugfs").unwrap();
    unsafe {
        libc::mount(c_dbg.as_ptr(), c_dbg_target.as_ptr(), c_dbg_type.as_ptr(), 0, std::ptr::null());
    }
}

#[cfg(target_os = "linux")]
fn setup_console() {
    unsafe {
        let console = std::ffi::CString::new("/dev/console").unwrap();
        let fd = libc::open(console.as_ptr(), libc::O_RDWR, 0);
        if fd >= 0 {
            libc::dup2(fd, 0);
            libc::dup2(fd, 1);
            libc::dup2(fd, 2);
            if fd > 2 {
                libc::close(fd);
            }
        }
        // Create /dev/null if missing
        let null_path = std::ffi::CString::new("/dev/null").unwrap();
        if libc::open(null_path.as_ptr(), libc::O_RDONLY, 0) < 0 {
            libc::mknod(null_path.as_ptr(), libc::S_IFCHR | 0o666, libc::makedev(1, 3));
        }
    }
}

#[cfg(target_os = "linux")]
fn setup_hostname() {
    // Write /etc/hostname if missing
    if std::fs::read_to_string("/etc/hostname").is_err() {
        std::fs::write("/etc/hostname", "aginx\n").ok();
    }
    // Set kernel hostname
    let hostname = std::ffi::CString::new("aginx").unwrap();
    unsafe {
        libc::sethostname(hostname.as_ptr(), 6);
    }
}

#[cfg(target_os = "linux")]
fn setup_network() {
    use std::net::Ipv4Addr;

    #[repr(C)]
    struct IfreqAddr {
        ifr_name: [u8; 16],
        ifr_addr: libc::sockaddr_in,
    }

    #[repr(C)]
    struct IfreqFlags {
        ifr_name: [u8; 16],
        ifr_flags: i16,
    }

    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
    if fd < 0 {
        eprintln!("[init] Failed to create socket for network config");
        return;
    }

    unsafe fn set_addr(fd: i32, ifname: &str, addr: Ipv4Addr) {
        let mut ifr: IfreqAddr = std::mem::zeroed();
        let b = ifname.as_bytes();
        ifr.ifr_name[..b.len()].copy_from_slice(b);
        ifr.ifr_addr.sin_family = libc::AF_INET as u16;
        ifr.ifr_addr.sin_addr.s_addr = u32::from(addr).to_be();
        libc::ioctl(fd, libc::SIOCSIFADDR as _, &mut ifr as *mut _);
    }

    unsafe fn set_netmask(fd: i32, ifname: &str, addr: Ipv4Addr) {
        let mut ifr: IfreqAddr = std::mem::zeroed();
        let b = ifname.as_bytes();
        ifr.ifr_name[..b.len()].copy_from_slice(b);
        ifr.ifr_addr.sin_family = libc::AF_INET as u16;
        ifr.ifr_addr.sin_addr.s_addr = u32::from(addr).to_be();
        libc::ioctl(fd, libc::SIOCSIFNETMASK as _, &mut ifr as *mut _);
    }

    unsafe fn set_up(fd: i32, ifname: &str) {
        let mut ifr: IfreqFlags = std::mem::zeroed();
        let b = ifname.as_bytes();
        ifr.ifr_name[..b.len()].copy_from_slice(b);
        libc::ioctl(fd, libc::SIOCGIFFLAGS as _, &mut ifr as *mut _);
        ifr.ifr_flags |= (libc::IFF_UP | libc::IFF_RUNNING) as i16;
        libc::ioctl(fd, libc::SIOCSIFFLAGS as _, &mut ifr as *mut _);
    }

    // lo
    unsafe {
        set_addr(fd, "lo", Ipv4Addr::new(127, 0, 0, 1));
        set_up(fd, "lo");
    }

    // Find the first non-lo interface
    let ifname = find_primary_interface();

    // Detect environment: QEMU vs real hardware
    let is_qemu = detect_qemu();

    if is_qemu {
        // QEMU user-mode networking: static config
        unsafe {
            set_addr(fd, ifname, Ipv4Addr::new(10, 0, 2, 15));
            set_netmask(fd, ifname, Ipv4Addr::new(255, 255, 255, 0));
            set_up(fd, ifname);
        }
        let _ = std::fs::write("/etc/resolv.conf", "nameserver 10.0.2.3\n");
        add_default_route(fd, ifname, Ipv4Addr::new(10, 0, 2, 2));
        eprintln!("[init] Network up ({}=10.0.2.15/24 gw=10.0.2.2) [QEMU]", ifname);
    } else {
        // Real hardware: try DHCP first, fall back to link-local
        unsafe { set_up(fd, ifname); }
        eprintln!("[init] Real hardware detected, trying DHCP on {}...", ifname);

        if let Some((ip, gw, dns)) = run_dhcp(ifname) {
            unsafe {
                set_addr(fd, ifname, ip);
                set_netmask(fd, ifname, Ipv4Addr::new(255, 255, 255, 0));
            }
            if let Some(dns_ip) = dns {
                let _ = std::fs::write("/etc/resolv.conf",
                    format!("nameserver {}\n", dns_ip));
            } else {
                let _ = std::fs::write("/etc/resolv.conf", "nameserver 8.8.8.8\n");
            }
            if let Some(gw_ip) = gw {
                add_default_route(fd, ifname, gw_ip);
            }
            eprintln!("[init] Network up ({}={}) [DHCP]", ifname, ip);
        } else {
            // Fallback: link-local 169.254.x.x
            eprintln!("[init] DHCP failed, using link-local");
            unsafe {
                set_addr(fd, ifname, Ipv4Addr::new(169, 254, 1, 1));
                set_netmask(fd, ifname, Ipv4Addr::new(255, 255, 0, 0));
            }
            let _ = std::fs::write("/etc/resolv.conf", "nameserver 8.8.8.8\n");
            eprintln!("[init] Network up ({}=169.254.1.1/16) [link-local]", ifname);
        }
    }

    unsafe { libc::close(fd); }
}

#[cfg(target_os = "linux")]
fn find_primary_interface() -> &'static str {
    // Check common names first
    for name in &["eth0", "ens0", "enp0s3", "wlan0", "usb0", "rndis0"] {
        if std::path::Path::new(&format!("/sys/class/net/{}", name)).exists() {
            return name;
        }
    }
    // Scan /sys/class/net for any non-lo interface
    if let Ok(entries) = std::fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name != "lo" {
                return Box::leak(name.into_boxed_str());
            }
        }
    }
    "eth0"
}

#[cfg(target_os = "linux")]
fn detect_qemu() -> bool {
    // Check /proc/device-tree or DMI for QEMU indicators
    if let Ok(model) = std::fs::read_to_string("/sys/class/dmi/id/sys_vendor") {
        if model.to_lowercase().contains("qemu") { return true; }
    }
    if let Ok(model) = std::fs::read_to_string("/sys/class/dmi/id/product_name") {
        if model.to_lowercase().contains("qemu") ||
           model.to_lowercase().contains("kvm") ||
           model.to_lowercase().contains("virtual") { return true; }
    }
    // Check device tree (ARM)
    if let Ok(compat) = std::fs::read_to_string("/sys/firmware/devicetree/base/compatible") {
        if compat.contains("qemu") || compat.contains("virt") { return true; }
    }
    // Check for QEMU's e1000 with fixed MAC prefix
    if let Ok(mac) = std::fs::read_to_string("/sys/class/net/eth0/address") {
        let mac = mac.trim();
        if mac.starts_with("52:54:00:") { return true; } // QEMU default
    }
    false
}

#[cfg(target_os = "linux")]
fn add_default_route(fd: i32, ifname: &str, gateway: std::net::Ipv4Addr) {
    #[repr(C)]
    struct IfreqFlags {
        ifr_name: [u8; 16],
        ifr_flags: i16,
    }

    let rt_fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
    if rt_fd >= 0 {
        unsafe {
            let mut ifr: IfreqFlags = std::mem::zeroed();
            let b = ifname.as_bytes();
            ifr.ifr_name[..b.len()].copy_from_slice(b);
            let mut rtentry: libc::rtentry = std::mem::zeroed();
            let mut dst: libc::sockaddr_in = std::mem::zeroed();
            dst.sin_family = libc::AF_INET as u16;
            let mut gw: libc::sockaddr_in = std::mem::zeroed();
            gw.sin_family = libc::AF_INET as u16;
            gw.sin_addr.s_addr = u32::from(gateway).to_be();
            rtentry.rt_dst = *(&dst as *const _ as *const _);
            rtentry.rt_gateway = *(&gw as *const _ as *const _);
            rtentry.rt_flags = (libc::RTF_UP | libc::RTF_GATEWAY) as u16;
            rtentry.rt_dev = ifr.ifr_name.as_ptr() as *mut _;
            let ret = libc::ioctl(rt_fd, libc::SIOCADDRT as _, &mut rtentry);
            if ret == 0 {
                eprintln!("[init] Default gateway: {}", gateway);
            }
        }
        unsafe { libc::close(rt_fd); }
    }
}

#[cfg(target_os = "linux")]
pub fn run_dhcp(ifname: &str) -> Option<(std::net::Ipv4Addr, Option<std::net::Ipv4Addr>, Option<std::net::Ipv4Addr>)> {
    use std::net::{Ipv4Addr, UdpSocket};

    // Minimal DHCP client: DISCOVER -> OFFER -> REQUEST -> ACK
    let mac = std::fs::read_to_string(format!("/sys/class/net/{}/address", ifname))
        .ok()?
        .trim()
        .to_string();
    let mac_bytes: Vec<u8> = mac.split(':')
        .filter_map(|b| u8::from_str_radix(b, 16).ok())
        .collect();
    if mac_bytes.len() != 6 { return None; }

    // Build DHCP Discover packet
    let mut packet = vec![0u8; 300];
    // op=1 (BOOTREQUEST), htype=1 (Ethernet), hlen=6, hops=0
    packet[0] = 1; packet[1] = 1; packet[2] = 6;
    // xid (transaction ID)
    let xid: u32 = unsafe { libc::getpid() as u32 };
    packet[4..8].copy_from_slice(&xid.to_be_bytes());
    // flags = broadcast (0x8000)
    packet[10..12].copy_from_slice(&0x8000u16.to_be_bytes());
    // chaddr (client MAC)
    packet[28..34].copy_from_slice(&mac_bytes);
    // Magic cookie: 99.130.83.99
    packet[236..240].copy_from_slice(&[99, 130, 83, 99]);
    // DHCP Option 53: DHCP Discover
    packet[240] = 53; packet[241] = 1; packet[242] = 1;
    // Option 255: end
    packet[243] = 255;

    // Create raw UDP socket for DHCP
    let sock = UdpSocket::bind("0.0.0.0:68").ok()?;
    sock.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok();
    sock.set_broadcast(true).ok()?;

    // Send discover
    if sock.send_to(&packet, "255.255.255.255:67").is_err() {
        return None;
    }

    // Wait for offer
    let mut buf = [0u8; 1024];
    for _ in 0..3 {
        match sock.recv_from(&mut buf) {
            Ok((n, _)) if n > 240 => {
                // Verify xid matches
                if buf[4..8] != xid.to_be_bytes() { continue; }
                // Check DHCP message type
                if buf[236..240] != [99, 130, 83, 99] { continue; }

                // Parse options to find DHCP message type
                let mut msg_type = 0u8;
                let mut offered_ip = None;
                let mut server_ip = None;
                let mut dns_ip = None;
                let mut router_ip = None;

                let yiaddr = &buf[16..20]; // your IP
                let siaddr = &buf[20..24]; // server IP

                let mut pos = 240;
                while pos < n - 1 {
                    let opt = buf[pos];
                    if opt == 255 { break; }
                    if opt == 0 { pos += 1; continue; }
                    if pos + 1 >= n { break; }
                    let len = buf[pos + 1] as usize;
                    if pos + 2 + len > n { break; }

                    match opt {
                        1 => msg_type = buf[pos + 2], // DHCP Message Type
                        3 if len == 4 => { // Router
                            router_ip = Some(Ipv4Addr::new(
                                buf[pos+2], buf[pos+3], buf[pos+4], buf[pos+5]));
                        }
                        6 => { // DNS
                            if len >= 4 {
                                dns_ip = Some(Ipv4Addr::new(
                                    buf[pos+2], buf[pos+3], buf[pos+4], buf[pos+5]));
                            }
                        }
                        54 if len == 4 => { // Server Identifier
                            server_ip = Some(Ipv4Addr::new(
                                buf[pos+2], buf[pos+3], buf[pos+4], buf[pos+5]));
                        }
                        _ => {}
                    }
                    pos += 2 + len;
                }

                if yiaddr[0] != 0 {
                    offered_ip = Some(Ipv4Addr::new(yiaddr[0], yiaddr[1], yiaddr[2], yiaddr[3]));
                }
                if server_ip.is_none() {
                    server_ip = Some(Ipv4Addr::new(siaddr[0], siaddr[1], siaddr[2], siaddr[3]));
                }

                if msg_type == 2 && offered_ip.is_some() {
                    // Got DHCPOFFER, send DHCPREQUEST
                    let server = server_ip.unwrap_or(Ipv4Addr::new(0, 0, 0, 0));
                    let offered = offered_ip.unwrap();

                    let mut req = vec![0u8; 300];
                    req[0] = 1; req[1] = 1; req[2] = 6;
                    req[4..8].copy_from_slice(&xid.to_be_bytes());
                    req[10..12].copy_from_slice(&0x8000u16.to_be_bytes());
                    req[28..34].copy_from_slice(&mac_bytes);
                    req[236..240].copy_from_slice(&[99, 130, 83, 99]);
                    // Option 53: DHCP Request
                    req[240] = 53; req[241] = 1; req[242] = 3;
                    // Option 50: Requested IP
                    req[243] = 50; req[244] = 4;
                    req[245..249].copy_from_slice(&offered.octets());
                    // Option 54: Server ID
                    req[249] = 54; req[250] = 4;
                    req[251..255].copy_from_slice(&server.octets());
                    // End
                    req[255] = 255;

                    let _ = sock.send_to(&req, "255.255.255.255:67");

                    // Wait for ACK
                    if let Ok((n2, _)) = sock.recv_from(&mut buf) {
                        if n2 > 240 && buf[4..8] == xid.to_be_bytes() {
                            let ack_ip = Ipv4Addr::new(buf[16], buf[17], buf[18], buf[19]);
                            if ack_ip != Ipv4Addr::UNSPECIFIED {
                                return Some((ack_ip, router_ip, dns_ip));
                            }
                        }
                    }
                    return Some((offered, router_ip, dns_ip));
                }
            }
            _ => continue,
        }
    }
    None
}


#[cfg(target_os = "linux")]
fn load_modules() {
    // Load kernel modules from /lib/modules/ (vendor_boot provides these)
    // Uses finit_module syscall
    let mod_dir = std::path::Path::new("/lib/modules");
    if !mod_dir.exists() { return; }

    // Collect all .ko files, then load in multiple passes to handle dependencies
    let mut modules: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(mod_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "ko").unwrap_or(false) {
                modules.push(path);
            }
        }
    }
    // Also check subdirectories (vendor_boot puts modules in /lib/modules/)
    if let Ok(entries) = std::fs::read_dir(mod_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Ok(sub) = std::fs::read_dir(&path) {
                    for se in sub.flatten() {
                        let sp = se.path();
                        if sp.extension().map(|e| e == "ko").unwrap_or(false) {
                            modules.push(sp);
                        }
                    }
                }
            }
        }
    }

    // Priority modules: load display and USB first
    let priority = [
        "clk-", "smd", "smem", "qcom-scm", "qcom_scm",
        "pinctrl", "regmap", "spmi", "pmic",
        "dispcc", "gpu", "drm", "msm_drm",
        "phy-qcom", "ufs", "ice", "Inline",
        "usb_f_rndis", "usb_f_ecm", "libcomposite",
    ];

    // Sort: priority modules first
    modules.sort_by_key(|p| {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let mut best = priority.len();
        for (i, prefix) in priority.iter().enumerate() {
            if name.contains(prefix) { best = i; break; }
        }
        best
    });

    // Load in multiple passes (up to 3) to handle dependencies
    let mut remaining = modules;
    for pass in 0..3 {
        let mut still_remaining = Vec::new();
        let mut loaded = 0;
        for path in remaining.drain(..) {
            if load_one_module(&path) {
                loaded += 1;
            } else {
                still_remaining.push(path);
            }
        }
        eprintln!("[init] Module pass {}: loaded {}, remaining {}", pass, loaded, still_remaining.len());
        if still_remaining.is_empty() || loaded == 0 { break; }
        remaining = still_remaining;
    }
}

#[cfg(target_os = "linux")]
fn load_one_module(path: &std::path::Path) -> bool {
    let c_path = std::ffi::CString::new(path.to_string_lossy().into_owned()).unwrap();
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDONLY, 0) };
    if fd < 0 { return false; }

    // finit_module(fd, "", 0) - syscall 249 on aarch64, 313 on x86_64
    let sysno = if cfg!(target_arch = "x86_64") { 313 } else { 249 };
    let ret = unsafe { libc::syscall(sysno, fd, b"\0".as_ptr(), 0) };
    unsafe { libc::close(fd); }
    if ret == 0 {
        eprintln!("[init] Loaded {}", path.file_name().unwrap().to_string_lossy());
        true
    } else {
        false // Probably dependency missing, will retry next pass
    }
}

#[cfg(target_os = "linux")]
fn setup_framebuffer() {
    // Try to open framebuffer device
    let c_fb = std::ffi::CString::new("/dev/fb0").unwrap();
    let fd = unsafe { libc::open(c_fb.as_ptr(), libc::O_RDWR) };
    if fd < 0 {
        eprintln!("[init] No framebuffer device");
        return;
    }

    // FBIOGET_VSCREENINFO = 0x4600
    // fb_var_screeninfo: xres(0), yres(4), xres_virtual(8), yres_virtual(12),
    //   xoffset(16), yoffset(20), bits_per_pixel(24)
    let mut vinfo = [0u8; 160];
    let vinfo_ret = unsafe { libc::ioctl(fd, 0x4600, vinfo.as_mut_ptr() as *mut _) };

    let (xres, yres, bpp) = if vinfo_ret == 0 {
        let xres = u32::from_ne_bytes(vinfo[0..4].try_into().unwrap());
        let yres = u32::from_ne_bytes(vinfo[4..8].try_into().unwrap());
        let bpp = u32::from_ne_bytes(vinfo[24..28].try_into().unwrap());
        (xres, yres, bpp)
    } else {
        // Assume Pixel 5 defaults: 1080x2340 32bpp
        (1080u32, 2340u32, 32u32)
    };

    let screen_bytes = (xres * yres * (bpp / 8)) as usize;

    // mmap the framebuffer
    let fb_ptr = unsafe {
        libc::mmap(std::ptr::null_mut(), screen_bytes,
            libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0)
    };

    if fb_ptr == libc::MAP_FAILED {
        // Fallback: just write() solid color
        eprintln!("[init] fb mmap failed, trying write()");
        let pixel: [u8; 4] = [0x00, 0x00, 0x80, 0xFF]; // RGBA dark blue
        let row = pixel.repeat(xres as usize);
        use std::io::Write;
        let mut fb_file = unsafe { std::fs::File::from_raw_fd(fd) };
        for _ in 0..yres {
            let _ = fb_file.write(&row);
        }
        return;
    }

    let fb = unsafe { std::slice::from_raw_parts_mut(fb_ptr as *mut u32, screen_bytes / 4) };

    // Fill with dark blue
    let blue: u32 = 0xFF800000; // ARGB: alpha=FF, blue=80, green=00, red=00
    for pixel in fb.iter_mut() {
        *pixel = blue;
    }

    // Draw "Aginx OS" text using 8x8 bitmap font (scaled 4x)
    let font = include_bytes!("font8x8.bin");
    let text = b"Aginx OS v0.1.0";
    let scale = 4u32;
    let start_x = (xres - (text.len() as u32 * 8 * scale)) / 2;
    let start_y = yres / 3;
    let white: u32 = 0xFFFFFFFF;

    for (ci, &ch) in text.iter().enumerate() {
        let base_x = start_x + (ci as u32) * 8 * scale;
        let glyph_offset = (ch as usize) * 8;
        for row in 0..8u32 {
            let glyph_row = if (glyph_offset + row as usize) < font.len() {
                font[glyph_offset + row as usize]
            } else {
                0u8
            };
            for bit in 0..8u32 {
                if glyph_row & (1 << bit) != 0 {
                    // Draw scaled pixel
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let px = base_x + bit * scale + dx;
                            let py = start_y + row * scale + dy;
                            if px < xres && py < yres {
                                fb[(py * xres + px) as usize] = white;
                            }
                        }
                    }
                }
            }
        }
    }

    unsafe {
        libc::munmap(fb_ptr, screen_bytes);
        libc::close(fd);
    }
    eprintln!("[init] Framebuffer: {}x{} {}bpp", xres, yres, bpp);
}

#[cfg(target_os = "linux")]
fn auto_connect_wifi() {
    // Only on real hardware (not QEMU)
    if detect_qemu() { return; }

    // Find wireless interface
    let mut wlan = String::new();
    if let Ok(entries) = std::fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "lo" { continue; }
            let wireless = format!("/sys/class/net/{}/wireless", name);
            let phy80211 = format!("/sys/class/net/{}/phy80211", name);
            if std::path::Path::new(&wireless).exists() || std::path::Path::new(&phy80211).exists() {
                wlan = name;
                break;
            }
        }
    }

    if wlan.is_empty() {
        eprintln!("[init] No WiFi interface found, skipping WiFi");
        return;
    }

    eprintln!("[init] Found WiFi interface: {}", wlan);

    // Try to mount vendor partition for WiFi firmware
    let vendor_devs = [
        "/dev/block/by-name/vendor",
        "/dev/block/bootdevice/by-name/vendor",
        "/dev/block/sda20",
        "/dev/block/sda19",
        "/dev/block/sda18",
    ];
    for dev in &vendor_devs {
        if std::path::Path::new(dev).exists() {
            std::fs::create_dir_all("/vendor").ok();
            let c_dev = std::ffi::CString::new(*dev).unwrap();
            let c_dir = std::ffi::CString::new("/vendor").unwrap();
            let c_ext4 = std::ffi::CString::new("ext4").unwrap();
            unsafe {
                let ret = libc::mount(
                    c_dev.as_ptr(), c_dir.as_ptr(), c_ext4.as_ptr(),
                    libc::MS_RDONLY, std::ptr::null(),
                );
                if ret == 0 {
                    eprintln!("[init] Mounted vendor from {}", dev);
                    break;
                }
            }
        }
    }

    // Also try system partition for tools
    let system_devs = [
        "/dev/block/by-name/system",
        "/dev/block/bootdevice/by-name/system",
    ];
    for dev in &system_devs {
        if std::path::Path::new(dev).exists() {
            std::fs::create_dir_all("/system").ok();
            let c_dev = std::ffi::CString::new(*dev).unwrap();
            let c_dir = std::ffi::CString::new("/system").unwrap();
            let c_ext4 = std::ffi::CString::new("ext4").unwrap();
            unsafe {
                let ret = libc::mount(
                    c_dev.as_ptr(), c_dir.as_ptr(), c_ext4.as_ptr(),
                    libc::MS_RDONLY, std::ptr::null(),
                );
                if ret == 0 {
                    eprintln!("[init] Mounted system from {}", dev);
                    break;
                }
            }
        }
    }

    // Bring up WiFi interface
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
    if fd >= 0 {
        #[repr(C)]
        struct IfreqFlags { name: [u8; 16], flags: i16 }
        let mut ifr: IfreqFlags = unsafe { std::mem::zeroed() };
        let b = wlan.as_bytes();
        ifr.name[..b.len()].copy_from_slice(b);
        unsafe {
            libc::ioctl(fd, libc::SIOCGIFFLAGS as _, &mut ifr);
            ifr.flags |= (libc::IFF_UP | libc::IFF_RUNNING) as i16;
            libc::ioctl(fd, libc::SIOCSIFFLAGS as _, &mut ifr);
        }
        unsafe { libc::close(fd); }
        eprintln!("[init] Brought up {}", wlan);
    }

    // Wait for WiFi driver to initialize and load firmware
    std::thread::sleep(std::time::Duration::from_secs(3));

    // Write wpa_supplicant config
    let wpa_conf = format!(
        "ctrl_interface=/var/run/wpa_supplicant\n\
         network={{\n\
         \tssid=\"Legrand AP\"\n\
         \tpsk=\"1234567890\"\n\
         \tkey_mgmt=WPA-PSK\n\
         }}\n"
    );
    std::fs::create_dir_all("/var/run/wpa_supplicant").ok();
    std::fs::write("/tmp/wpa_supplicant.conf", &wpa_conf).ok();

    // Try wpa_supplicant from various locations
    let wpa_paths = [
        "/vendor/bin/hw/wpa_supplicant",
        "/system/bin/wpa_supplicant",
        "/usr/sbin/wpa_supplicant",
        "wpa_supplicant",
    ];

    let mut wpa_started = false;
    for wpa in &wpa_paths {
        if std::path::Path::new(wpa).exists() || *wpa == "wpa_supplicant" {
            eprintln!("[init] Trying wpa_supplicant: {}", wpa);
            let result = std::process::Command::new(wpa)
                .args([
                    "-O", "/var/run/wpa_supplicant",
                    "-i", &wlan,
                    "-c", "/tmp/wpa_supplicant.conf",
                    "-B",
                ])
                .output();

            match result {
                Ok(output) => {
                    if output.status.success() {
                        eprintln!("[init] wpa_supplicant started");
                        wpa_started = true;
                        break;
                    } else {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        eprintln!("[init] wpa_supplicant failed: {}", stderr.trim());
                    }
                }
                Err(e) => {
                    eprintln!("[init] Cannot run {}: {}", wpa, e);
                }
            }
        }
    }

    if wpa_started {
        // Wait for connection
        eprintln!("[init] Waiting for WiFi connection...");
        std::thread::sleep(std::time::Duration::from_secs(5));

        // Run DHCP on wlan0
        if let Some((ip, gw, dns)) = run_dhcp(&wlan) {
            // Configure IP
            let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
            if fd >= 0 {
                #[repr(C)]
                struct IfreqAddr { name: [u8; 16], addr: libc::sockaddr_in }
                unsafe {
                    let mut ifr: IfreqAddr = std::mem::zeroed();
                    let b = wlan.as_bytes();
                    ifr.name[..b.len()].copy_from_slice(b);
                    ifr.addr.sin_family = libc::AF_INET as u16;
                    ifr.addr.sin_addr.s_addr = u32::from(ip).to_be();
                    libc::ioctl(fd, libc::SIOCSIFADDR as _, &mut ifr);

                    // netmask
                    let mut ifr2: IfreqAddr = std::mem::zeroed();
                    ifr2.name[..b.len()].copy_from_slice(b);
                    ifr2.addr.sin_family = libc::AF_INET as u16;
                    ifr2.addr.sin_addr.s_addr = u32::from(std::net::Ipv4Addr::new(255,255,255,0)).to_be();
                    libc::ioctl(fd, libc::SIOCSIFNETMASK as _, &mut ifr2);
                }
                unsafe { libc::close(fd); }
            }
            if let Some(dns_ip) = dns {
                let _ = std::fs::write("/etc/resolv.conf", format!("nameserver {}\n", dns_ip));
            } else {
                let _ = std::fs::write("/etc/resolv.conf", "nameserver 8.8.8.8\n");
            }
            if let Some(gw_ip) = gw {
                let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
                if fd >= 0 {
                    add_default_route(fd, &wlan, gw_ip);
                    unsafe { libc::close(fd); }
                }
            }
            eprintln!("[init] WiFi connected: {} IP={}", wlan, ip);
        } else {
            eprintln!("[init] WiFi DHCP failed");
        }
    } else {
        eprintln!("[init] Could not start wpa_supplicant, WiFi not connected");
    }
}

#[cfg(target_os = "linux")]
fn setup_usb_gadget() {
    // Set up USB RNDIS/ECM gadget for USB networking
    // This makes the phone appear as a USB Ethernet device to the host Mac
    use std::io::Write;

    let gadget_dir = "/sys/kernel/config/usb_gadget/g1";
    let func = "ecm.us0"; // Try ECM first (simpler, works on Mac/Linux)

    // Create gadget
    std::fs::create_dir_all(gadget_dir).ok();

    // Set VID/PID (Google USB ID)
    let _ = std::fs::write(format!("{}/idVendor", gadget_dir), "0x18d1");
    let _ = std::fs::write(format!("{}/idProduct", gadget_dir), "0x4ee4");

    // Set device class
    std::fs::create_dir_all(format!("{}/strings/0x409", gadget_dir)).ok();
    let _ = std::fs::write(format!("{}/strings/0x409/serialnumber", gadget_dir), "aginx0001");
    let _ = std::fs::write(format!("{}/strings/0x409/manufacturer", gadget_dir), "Aginx");
    let _ = std::fs::write(format!("{}/strings/0x409/product", gadget_dir), "Aginx OS");

    // Create config
    std::fs::create_dir_all(format!("{}/configs/c.1/strings/0x409", gadget_dir)).ok();
    let _ = std::fs::write(format!("{}/configs/c.1/strings/0x409/configuration", gadget_dir), "net");

    // Create ECM function
    std::fs::create_dir_all(format!("{}/functions/{}", gadget_dir, func)).ok();

    // Link function to config
    let _ = std::os::unix::fs::symlink(
        format!("{}/functions/{}", gadget_dir, func),
        format!("{}/configs/c.1/{}", gadget_dir, func.split('.').next().unwrap_or("ecm")),
    );

    // Find UDC (USB Device Controller)
    let udc = if let Ok(entries) = std::fs::read_dir("/sys/class/udc") {
        entries.flatten().next().map(|e| e.file_name().to_string_lossy().into_owned())
    } else { None };

    if let Some(udc_name) = udc {
        let _ = std::fs::write(format!("{}/UDC", gadget_dir), &udc_name);
        eprintln!("[init] USB gadget active on UDC: {}", udc_name);

        // Wait for usb0 interface to appear
        for _ in 0..10 {
            if std::path::Path::new("/sys/class/net/usb0").exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }

        if std::path::Path::new("/sys/class/net/usb0").exists() {
            // Configure usb0 with static IP
            let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
            if fd >= 0 {
                #[repr(C)]
                struct IfreqAddr { name: [u8; 16], addr: libc::sockaddr_in }
                #[repr(C)]
                struct IfreqFlags { name: [u8; 16], flags: i16 }

                unsafe {
                    // Set IP
                    let mut ifr: IfreqAddr = std::mem::zeroed();
                    ifr.name[..4].copy_from_slice(b"usb0");
                    ifr.addr.sin_family = libc::AF_INET as u16;
                    ifr.addr.sin_addr.s_addr = u32::from(std::net::Ipv4Addr::new(192, 168, 42, 1)).to_be();
                    libc::ioctl(fd, libc::SIOCSIFADDR as _, &mut ifr);

                    // Set netmask
                    let mut ifr2: IfreqAddr = std::mem::zeroed();
                    ifr2.name[..4].copy_from_slice(b"usb0");
                    ifr2.addr.sin_family = libc::AF_INET as u16;
                    ifr2.addr.sin_addr.s_addr = u32::from(std::net::Ipv4Addr::new(255, 255, 255, 0)).to_be();
                    libc::ioctl(fd, libc::SIOCSIFNETMASK as _, &mut ifr2);

                    // Bring up
                    let mut ifr3: IfreqFlags = std::mem::zeroed();
                    ifr3.name[..4].copy_from_slice(b"usb0");
                    libc::ioctl(fd, libc::SIOCGIFFLAGS as _, &mut ifr3);
                    ifr3.flags |= (libc::IFF_UP | libc::IFF_RUNNING) as i16;
                    libc::ioctl(fd, libc::SIOCSIFFLAGS as _, &mut ifr3);
                }
                unsafe { libc::close(fd); }
            }
            eprintln!("[init] USB network: usb0 = 192.168.42.1/24");
            let _ = std::fs::write("/etc/resolv.conf", "nameserver 8.8.8.8\n");
        } else {
            eprintln!("[init] usb0 interface not found after gadget setup");
        }
    } else {
        eprintln!("[init] No UDC found, USB gadget not available");
    }
}

#[cfg(target_os = "linux")]
fn reap_zombies() {
    loop {
        unsafe {
            let pid = libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG);
            if pid <= 0 { break; }
            eprintln!("[init] Reaped zombie PID {}", pid);
        }
    }
}

#[cfg(target_os = "linux")]
fn run_as_init() -> ! {
    eprintln!("[init] Aginx Daemon starting as PID 1...");

    // 1. Mount virtual filesystems
    mount_filesystems();

    // 2. Setup console stdio
    setup_console();

    // 3. Setup hostname
    setup_hostname();

    // 4. Load kernel modules (e1000, etc.)
    load_modules();

    // 5. Show framebuffer (confirms daemon is running on real hardware)
    setup_framebuffer();

    // 6. Setup network
    setup_network();

    // 7. Setup USB gadget networking (for real hardware)
    if !detect_qemu() {
        setup_usb_gadget();
    }

    // 8. Auto-connect WiFi on real hardware
    if !detect_qemu() {
        auto_connect_wifi();
    }

    // 9. Start TCP server in background thread
    let port = DEFAULT_PORT;
    std::thread::spawn(move || {
        eprintln!("[init] Starting TCP server on port {}...", port);
        match tcp_server::start(port) {
            Ok(_) => eprintln!("[init] TCP server stopped"),
            Err(e) => eprintln!("[init] TCP server error: {}", e),
        }
    });

    // 10. Print banner
    print_banner();

    // DEBUG: If real hardware and no network after 15s, reboot to confirm daemon is running
    if !detect_qemu() {
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(15));
            eprintln!("[init] DEBUG: Auto-reboot after 15s to confirm daemon ran");
            unsafe { libc::reboot(libc::LINUX_REBOOT_CMD_RESTART); }
        });
    }

    // 9. Print banner
    print_banner();

    // 10. Run console shell
    loop {
        shell::run_interactive();
        eprintln!("[init] Shell exited, restarting...");
        reap_zombies();
    }
}

// ── Entry point ────────────────────────────────────────────────────────────────

fn main() {
    // Detect if running as PID 1 (init)
    #[cfg(target_os = "linux")]
    {
        let is_init = unsafe { libc::getpid() == 1 };
        if is_init {
            run_as_init();
        }
    }

    let args: Vec<String> = env::args().collect();

    let mut listen_port: Option<u16> = None;
    let mut interactive = false;
    let mut daemon_mode = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--listen" => {
                i += 1;
                listen_port = Some(args[i].parse().unwrap_or(DEFAULT_PORT));
            }
            "--shell" => interactive = true,
            "--daemon" => daemon_mode = true,
            "--help" | "-h" => { usage(); return; }
            _ => {}
        }
        i += 1;
    }

    if !interactive && listen_port.is_none() {
        interactive = true;
    }

    if daemon_mode {
        unsafe {
            match libc::fork() {
                0 => {
                    libc::setsid();
                    let devnull = libc::open(b"/dev/null\0".as_ptr() as *const _,
                        libc::O_RDWR, 0);
                    libc::dup2(devnull, 0);
                    libc::dup2(devnull, 1);
                    libc::dup2(devnull, 2);
                    if devnull > 2 { libc::close(devnull); }
                }
                _ => return,
            }
        }
    }

    print_banner();

    if let Some(port) = listen_port {
        match tcp_server::start(port) {
            Ok(_) => {}
            Err(e) => eprintln!("[FAIL] TCP listen on {}: {}", port, e),
        }
        return;
    }

    if interactive {
        shell::run_interactive();
    }
}
