//! WiFi management module
//!
//! Supports scanning, connecting to WPA2 networks.
//! Uses nl80211/ioctl for direct kernel communication.

use std::fs;
use std::io::Write;

/// Scan for WiFi networks
pub fn scan(out: &mut dyn Write) {
    // Check if wireless extensions are available
    let ifname = find_wireless_interface();
    if ifname.is_empty() {
        writeln!(out, "[FAIL] No wireless interface found").unwrap();
        return;
    }
    writeln!(out, "Scanning on {}...", ifname).unwrap();

    // Trigger scan via ioctl (SIOCSIWSCAN)
    #[cfg(target_os = "linux")]
    {
        let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
        if fd < 0 {
            writeln!(out, "[FAIL] Cannot open socket").unwrap();
            return;
        }

        // Trigger scan
        let mut scan_req: [u8; 32] = [0; 32];
        let c_ifname = std::ffi::CString::new(ifname.as_str()).unwrap();
        unsafe {
            // SIOCSIWSCAN = 0x8B18 (trigger scan)
            // We use a simplified approach - write to /sys then read results
            libc::ioctl(fd, 0x8B18, &scan_req as *const _);
        }
        unsafe { libc::close(fd); }

        // Read scan results from nl80211 via /proc or netlink
        // For now, try to use iw-like approach via /sys/class/net/{}/
        read_scan_results(&ifname, out);
    }

    #[cfg(not(target_os = "linux"))]
    {
        writeln!(out, "[FAIL] WiFi only supported on Linux").unwrap();
    }
}

/// Connect to a WiFi network
pub fn connect(ssid: &str, password: &str, out: &mut dyn Write) {
    let ifname = find_wireless_interface();
    if ifname.is_empty() {
        writeln!(out, "[FAIL] No wireless interface found").unwrap();
        return;
    }

    writeln!(out, "[..] Connecting to '{}' on {}...", ssid, ifname).unwrap();

    #[cfg(target_os = "linux")]
    {
        // Write wpa_supplicant config
        let wpa_conf = format!(
            "ctrl_interface=/var/run/wpa_supplicant\n\
             network={{\n\
             \tssid=\"{}\"\n\
             \tpsk=\"{}\"\n\
             \tkey_mgmt=WPA-PSK\n\
             }}\n",
            ssid, password
        );

        // Create config directory
        fs::create_dir_all("/var/run/wpa_supplicant").ok();
        if fs::write("/tmp/wpa_supplicant.conf", &wpa_conf).is_err() {
            // Try alternate method: use iw commands
            connect_raw(&ifname, ssid, password, out);
            return;
        }

        // Try to run wpa_supplicant
        let wpa_result = std::process::Command::new("wpa_supplicant")
            .args([
                "-i", &ifname,
                "-c", "/tmp/wpa_supplicant.conf",
                "-B", // background
            ])
            .output();

        match wpa_result {
            Ok(output) => {
                if output.status.success() {
                    // Wait for connection
                    std::thread::sleep(std::time::Duration::from_secs(3));
                    // Run DHCP
                    run_dhcp_on_interface(&ifname, out);
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    writeln!(out, "[FAIL] wpa_supplicant: {}", stderr.trim()).unwrap();
                    // Fallback to raw method
                    connect_raw(&ifname, ssid, password, out);
                }
            }
            Err(_) => {
                // No wpa_supplicant binary, try raw connection
                connect_raw(&ifname, ssid, password, out);
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        writeln!(out, "[FAIL] WiFi only supported on Linux").unwrap();
    }
}

/// Disconnect from WiFi
pub fn disconnect(out: &mut dyn Write) {
    let ifname = find_wireless_interface();
    if ifname.is_empty() {
        writeln!(out, "[FAIL] No wireless interface found").unwrap();
        return;
    }

    // Kill wpa_supplicant if running
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("killall")
            .arg("wpa_supplicant")
            .output();

        // Bring interface down
        let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
        if fd >= 0 {
            #[repr(C)]
            struct Ifreq {
                name: [u8; 16],
                flags: i16,
            }
            unsafe {
                let mut ifr: Ifreq = std::mem::zeroed();
                let b = ifname.as_bytes();
                ifr.name[..b.len()].copy_from_slice(b);
                libc::ioctl(fd, libc::SIOCGIFFLAGS as _, &mut ifr);
                ifr.flags &= !(libc::IFF_UP as i16);
                libc::ioctl(fd, libc::SIOCSIFFLAGS as _, &mut ifr);
                libc::close(fd);
            }
        }
    }
    writeln!(out, "[OK] Disconnected from WiFi").unwrap();
}

/// Show WiFi status
pub fn status(out: &mut dyn Write) {
    let ifname = find_wireless_interface();
    if ifname.is_empty() {
        writeln!(out, "WiFi: No wireless interface").unwrap();
        return;
    }

    writeln!(out, "WiFi interface: {}", ifname).unwrap();

    // Check if associated
    #[cfg(target_os = "linux")]
    {
        // Read from /sys/class/net/{}/wireless/
        let wireless_path = format!("/sys/class/net/{}/wireless", ifname);
        if std::path::Path::new(&wireless_path).exists() {
            // Read signal level
            if let Ok(level) = fs::read_to_string(format!(
                "/sys/class/net/{}/device/net/{}/statistics/rx_bytes", ifname, ifname
            )) {
                writeln!(out, "  RX bytes: {}", level.trim()).unwrap();
            }
        }

        // Check wpa_supplicant status
        let wpa_ctrl = "/var/run/wpa_supplicant";
        if std::path::Path::new(wpa_ctrl).exists() {
            if let Ok(entries) = fs::read_dir(wpa_ctrl) {
                for entry in entries.flatten() {
                    writeln!(out, "  wpa_supplicant: active ({})", entry.file_name().to_string_lossy()).unwrap();
                }
            }
        } else {
            writeln!(out, "  Status: not connected").unwrap();
        }

        // Check carrier
        if let Ok(carrier) = fs::read_to_string(format!("/sys/class/net/{}/carrier", ifname)) {
            let state = carrier.trim();
            writeln!(out, "  Link: {}", if state == "1" { "up" } else { "down" }).unwrap();
        }
    }
}

// --- Internal helpers ---

fn find_wireless_interface() -> String {
    // Check common wireless interface names
    for name in &["wlan0", "wlp2s0", "wlp0s20f3", "wlan1"] {
        let wireless = format!("/sys/class/net/{}/wireless", name);
        let phy80211 = format!("/sys/class/net/{}/phy80211", name);
        if std::path::Path::new(&wireless).exists() || std::path::Path::new(&phy80211).exists() {
            return name.to_string();
        }
    }
    // Scan all interfaces
    if let Ok(entries) = fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "lo" { continue; }
            let wireless = format!("/sys/class/net/{}/wireless", name);
            let phy80211 = format!("/sys/class/net/{}/phy80211", name);
            if std::path::Path::new(&wireless).exists() || std::path::Path::new(&phy80211).exists() {
                return name;
            }
        }
    }
    String::new()
}

#[cfg(target_os = "linux")]
fn read_scan_results(ifname: &str, out: &mut dyn Write) {
    // Try using 'iw' command first
    let result = std::process::Command::new("iw")
        .args(["dev", ifname, "scan"])
        .output();

    match result {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut count = 0;
            for line in stdout.lines() {
                let line = line.trim();
                if line.starts_with("BSS") {
                    count += 1;
                }
                if line.starts_with("SSID:") {
                    writeln!(out, "  {}", line).unwrap();
                }
                if line.contains("signal:") {
                    writeln!(out, "    {}", line).unwrap();
                }
            }
            writeln!(out, "Found {} networks", count).unwrap();
        }
        _ => {
            // Fallback: parse /proc/net/wireless
            if let Ok(content) = fs::read_to_string("/proc/net/wireless") {
                write!(out, "{}", content).unwrap();
            } else {
                writeln!(out, "[FAIL] Cannot scan (iw not available)").unwrap();
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn connect_raw(ifname: &str, ssid: &str, _password: &str, out: &mut dyn Write) {
    // Try using 'iw' to connect (WPA2 requires wpa_supplicant in practice)
    // For open networks, iw dev wlan0 connect "$SSID" works
    let result = std::process::Command::new("iw")
        .args(["dev", ifname, "connect", ssid])
        .output();

    match result {
        Ok(output) => {
            if output.status.success() {
                writeln!(out, "[OK] Connected to {}", ssid).unwrap();
                std::thread::sleep(std::time::Duration::from_secs(2));
                run_dhcp_on_interface(ifname, out);
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                writeln!(out, "[FAIL] iw connect: {}", stderr.trim()).unwrap();
                writeln!(out, "[INFO] WPA2 networks require wpa_supplicant").unwrap();
            }
        }
        Err(e) => {
            writeln!(out, "[FAIL] No 'iw' command: {}", e).unwrap();
        }
    }
}

#[cfg(target_os = "linux")]
fn run_dhcp_on_interface(ifname: &str, out: &mut dyn Write) {
    // Try dhclient first
    let result = std::process::Command::new("dhclient")
        .arg(ifname)
        .output();

    match result {
        Ok(output) if output.status.success() => {
            writeln!(out, "[OK] DHCP configured on {}", ifname).unwrap();
        }
        _ => {
            // Try udhcpc (busybox)
            let result2 = std::process::Command::new("udhcpc")
                .args(["-i", ifname, "-q"])
                .output();
            match result2 {
                Ok(output) if output.status.success() => {
                    writeln!(out, "[OK] DHCP configured on {}", ifname).unwrap();
                }
                _ => {
                    // Use our built-in DHCP client
                    if let Some((ip, gw, dns)) = crate::run_dhcp(ifname) {
                        // Configure the interface
                        let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
                        if fd >= 0 {
                            #[repr(C)]
                            struct IfreqAddr { name: [u8; 16], addr: libc::sockaddr_in }
                            unsafe {
                                let mut ifr: IfreqAddr = std::mem::zeroed();
                                let b = ifname.as_bytes();
                                ifr.name[..b.len()].copy_from_slice(b);
                                ifr.addr.sin_family = libc::AF_INET as u16;
                                ifr.addr.sin_addr.s_addr = u32::from(ip).to_be();
                                libc::ioctl(fd, libc::SIOCSIFADDR as _, &mut ifr);
                            }
                            unsafe { libc::close(fd); }
                        }
                        if let Some(dns_ip) = dns {
                            let _ = fs::write("/etc/resolv.conf", format!("nameserver {}\n", dns_ip));
                        }
                        writeln!(out, "[OK] DHCP: {} on {}", ip, ifname).unwrap();
                    } else {
                        writeln!(out, "[FAIL] DHCP failed on {}", ifname).unwrap();
                    }
                }
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn read_scan_results(_ifname: &str, _out: &mut dyn Write) {}
#[cfg(not(target_os = "linux"))]
fn connect_raw(_ifname: &str, _ssid: &str, _password: &str, _out: &mut dyn Write) {}
#[cfg(not(target_os = "linux"))]
fn run_dhcp_on_interface(_ifname: &str, _out: &mut dyn Write) {}
