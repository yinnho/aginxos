//! Cellular (4G/LTE) modem management module
//!
//! Supports connecting via AT commands to USB/serial LTE modems.
//! Typical modems: Quectel EC20/EC25, Sierra Wireless, Huawei ME909s.
//! Uses serial port (/dev/ttyUSB2 or /dev/ttyACM0) for AT command interface.

use std::fs;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::time::Duration;

/// Scan for connected modem devices
pub fn scan(out: &mut dyn Write) {
    let modems = find_modem_ports();
    if modems.is_empty() {
        writeln!(out, "[FAIL] No cellular modem found").unwrap();
        writeln!(out, "[INFO] Check /dev/ttyUSB*, /dev/ttyACM*, /dev/ttyHS*").unwrap();
        return;
    }

    writeln!(out, "Found {} modem port(s):", modems.len()).unwrap();
    for port in &modems {
        writeln!(out, "  {}", port).unwrap();
        // Try to query modem info
        query_modem_info(port, out);
    }
}

/// Connect to cellular network using the modem
pub fn connect(apn: &str, out: &mut dyn Write) {
    let ports = find_modem_ports();
    let modem_port = match ports.first() {
        Some(p) => p.clone(),
        None => {
            writeln!(out, "[FAIL] No cellular modem found").unwrap();
            return;
        }
    };

    writeln!(out, "[..] Connecting via {} (APN: {})...", modem_port, apn).unwrap();

    #[cfg(target_os = "linux")]
    {
        let mut at = match open_serial(&modem_port) {
            Some(s) => s,
            None => {
                writeln!(out, "[FAIL] Cannot open {}", modem_port).unwrap();
                return;
            }
        };

        // Check modem is responsive
        if !at_command(&mut at, "AT", out) {
            writeln!(out, "[FAIL] Modem not responding").unwrap();
            return;
        }

        // Check SIM status
        at_command(&mut at, "AT+CPIN?", out);
        at_command(&mut at, "AT+COPS?", out);

        // Set APN
        let apn_cmd = format!("AT+CGDCONT=1,\"IP\",\"{}\"", apn);
        at_command(&mut at, &apn_cmd, out);

        // Dial (modem-specific)
        // Quectel: AT+QNWACT or ATD*99#
        // Generic: ATD*99# or ATD*99***1#
        let dial_cmds = [
            "ATD*99#",
            "ATD*99***1#",
            "AT+CGACT=1,1",
        ];

        let mut connected = false;
        for dial in &dial_cmds {
            at.port.write_all(format!("{}\r\n", dial).as_bytes()).ok();
            at.port.flush().ok();

            let mut response = String::new();
            let mut buf = [0u8; 256];
            let start = std::time::Instant::now();

            while start.elapsed() < Duration::from_secs(10) {
                match at.port.read(&mut buf) {
                    Ok(n) => {
                        response.push_str(&String::from_utf8_lossy(&buf[..n]));
                        if response.contains("CONNECT") || response.contains("OK") {
                            connected = true;
                            break;
                        }
                        if response.contains("ERROR") || response.contains("NO CARRIER") {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }

            if connected {
                break;
            }
        }

        if connected {
            writeln!(out, "[OK] Modem connected").unwrap();

            // Try to get IP via DHCP on the modem interface (wwan0/usb0)
            configure_data_interface(out);
        } else {
            writeln!(out, "[FAIL] Dial failed, trying pppd...").unwrap();
            try_ppp(&modem_port, apn, out);
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        writeln!(out, "[FAIL] Cellular only supported on Linux").unwrap();
    }
}

/// Disconnect cellular modem
pub fn disconnect(out: &mut dyn Write) {
    #[cfg(target_os = "linux")]
    {
        // Try ATH (hangup)
        let ports = find_modem_ports();
        if let Some(port) = ports.first() {
            if let Some(mut at) = open_serial(port) {
                let _ = at.port.write_all(b"ATH\r\n");
                let _ = at.port.flush();
                std::thread::sleep(Duration::from_millis(500));
                let _ = at.port.write_all(b"AT+CGACT=0,1\r\n");
                let _ = at.port.flush();
            }
        }

        // Kill pppd if running
        let _ = std::process::Command::new("killall")
            .arg("pppd")
            .output();

        // Bring down wwan0 if exists
        let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
        if fd >= 0 {
            for iface in &["wwan0", "usb0", "wwp0s20u4"] {
                #[repr(C)]
                struct Ifreq {
                    name: [u8; 16],
                    flags: i16,
                }
                unsafe {
                    let mut ifr: Ifreq = std::mem::zeroed();
                    let b = iface.as_bytes();
                    ifr.name[..b.len()].copy_from_slice(b);
                    libc::ioctl(fd, libc::SIOCGIFFLAGS as _, &mut ifr);
                    ifr.flags &= !(libc::IFF_UP as i16);
                    libc::ioctl(fd, libc::SIOCSIFFLAGS as _, &mut ifr);
                }
            }
            unsafe { libc::close(fd); }
        }
    }
    writeln!(out, "[OK] Cellular disconnected").unwrap();
}

/// Show cellular modem status
pub fn status(out: &mut dyn Write) {
    let ports = find_modem_ports();
    if ports.is_empty() {
        writeln!(out, "Cellular: No modem found").unwrap();
        return;
    }

    writeln!(out, "Cellular modem: {}", ports[0]).unwrap();

    #[cfg(target_os = "linux")]
    {
        if let Some(mut at) = open_serial(&ports[0]) {
            at_command(&mut at, "AT+CPIN?", out);       // SIM status
            at_command(&mut at, "AT+COPS?", out);        // Operator
            at_command(&mut at, "AT+CSQ", out);          // Signal quality
            at_command(&mut at, "AT+CGREG?", out);       // Network registration
            at_command(&mut at, "AT+CGDCONT?", out);     // PDP context (APN)
            at_command(&mut at, "AT+CGACT?", out);       // PDP activation status
            at_command(&mut at, "AT+QNWCFG=\"nwmode\"", out); // Network mode (Quectel)
        }

        // Check wwan0 interface status
        for iface in &["wwan0", "usb0", "wwp0s20u4"] {
            let carrier = format!("/sys/class/net/{}/carrier", iface);
            if Path::new(&carrier).exists() {
                if let Ok(state) = fs::read_to_string(&carrier) {
                    writeln!(out, "  {}: link {}", iface,
                        if state.trim() == "1" { "up" } else { "down" }).unwrap();
                }
            }
        }
    }
}

// --- Internal helpers ---

fn find_modem_ports() -> Vec<String> {
    let mut ports = Vec::new();
    let patterns = [
        "/dev/ttyUSB0", "/dev/ttyUSB1", "/dev/ttyUSB2",
        "/dev/ttyUSB3", "/dev/ttyACM0", "/dev/ttyACM1",
        "/dev/ttyHS0", "/dev/ttyHS1", "/dev/ttyHS2",
        "/dev/ttyQCMI0", "/dev/smd11",
    ];

    for path in &patterns {
        if Path::new(path).exists() {
            ports.push(path.to_string());
        }
    }

    // Scan /dev for modem devices
    if let Ok(entries) = fs::read_dir("/dev") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("ttyUSB") || name.starts_with("ttyACM") ||
               name.starts_with("ttyHS") || name.starts_with("wwan") {
                let path = format!("/dev/{}", name);
                if !ports.contains(&path) {
                    ports.push(path);
                }
            }
        }
    }

    ports
}

struct SerialPort {
    port: std::fs::File,
}

#[cfg(target_os = "linux")]
fn open_serial(path: &str) -> Option<SerialPort> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;

    let port = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY | libc::O_NDELAY)
        .open(path)
        .ok()?;

    // Configure serial port: 115200 8N1
    unsafe {
        let mut termios: libc::termios = std::mem::zeroed();
        libc::tcgetattr(port.as_raw_fd(), &mut termios);

        // Baud rate
        libc::cfsetispeed(&mut termios, libc::B115200);
        libc::cfsetospeed(&mut termios, libc::B115200);

        // 8N1, no flow control
        termios.c_cflag |= libc::CREAD | libc::CLOCAL;
        termios.c_cflag &= !(libc::PARENB | libc::CSTOPB | libc::CSIZE);
        termios.c_cflag |= libc::CS8;
        termios.c_cflag &= !(libc::CRTSCTS);

        // Raw input
        termios.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ECHOE | libc::ISIG);
        termios.c_iflag &= !(libc::IXON | libc::IXOFF | libc::IXANY);
        termios.c_iflag &= !(libc::ICRNL | libc::INLCR);
        termios.c_oflag &= !(libc::OPOST | libc::ONLCR);

        // Timeout: 100ms
        termios.c_cc[libc::VMIN] = 0;
        termios.c_cc[libc::VTIME] = 1;

        libc::tcsetattr(port.as_raw_fd(), libc::TCSANOW, &mut termios);

        // Clear O_NDELAY for blocking reads
        let flags = libc::fcntl(port.as_raw_fd(), libc::F_GETFL, 0);
        libc::fcntl(port.as_raw_fd(), libc::F_SETFL, flags & !libc::O_NONBLOCK);
    }

    Some(SerialPort { port })
}

#[cfg(not(target_os = "linux"))]
fn open_serial(_path: &str) -> Option<SerialPort> { None }

fn at_command(serial: &mut SerialPort, cmd: &str, out: &mut dyn Write) -> bool {
    serial.port.write_all(format!("{}\r\n", cmd).as_bytes()).ok();
    serial.port.flush().ok();

    let mut response = String::new();
    let mut buf = [0u8; 256];
    let start = std::time::Instant::now();

    while start.elapsed() < Duration::from_secs(3) {
        match serial.port.read(&mut buf) {
            Ok(n) => {
                response.push_str(&String::from_utf8_lossy(&buf[..n]));
                if response.contains("OK") || response.contains("ERROR") ||
                   response.contains("+CME ERROR") {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let success = response.contains("OK");
    // Print non-trivial response lines
    for line in response.lines() {
        let line = line.trim();
        if !line.is_empty() && line != cmd && line != "OK" && line != "AT" {
            writeln!(out, "  {}", line).unwrap();
        }
    }
    success
}

fn query_modem_info(port: &str, out: &mut dyn Write) {
    #[cfg(target_os = "linux")]
    {
        if let Some(mut at) = open_serial(port) {
            at_command(&mut at, "ATI", out);       // Modem identification
            at_command(&mut at, "AT+CPIN?", out);   // SIM status
            at_command(&mut at, "AT+CSQ", out);     // Signal quality
        }
    }
}

#[cfg(target_os = "linux")]
fn configure_data_interface(out: &mut dyn Write) {
    // After CONNECT, the modem enters data mode.
    // The kernel may create a wwan0 or cdc-wdm device.
    // Try DHCP on common modem network interfaces
    let ifaces = ["wwan0", "usb0", "wwp0s20u4", "enx0", "cdc-wdm0"];

    // Wait for interface to appear
    std::thread::sleep(Duration::from_secs(2));

    for iface in &ifaces {
        if !Path::new(&format!("/sys/class/net/{}", iface)).exists() {
            continue;
        }

        // Bring interface up
        let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
        if fd < 0 { continue; }

        #[repr(C)]
        struct Ifreq {
            name: [u8; 16],
            flags: i16,
        }
        unsafe {
            let mut ifr: Ifreq = std::mem::zeroed();
            let b = iface.as_bytes();
            ifr.name[..b.len()].copy_from_slice(b);
            libc::ioctl(fd, libc::SIOCGIFFLAGS as _, &mut ifr);
            ifr.flags |= (libc::IFF_UP | libc::IFF_RUNNING) as i16;
            libc::ioctl(fd, libc::SIOCSIFFLAGS as _, &mut ifr);
        }
        unsafe { libc::close(fd); }

        // Try DHCP
        if let Some((ip, gw, dns)) = crate::run_dhcp(iface) {
            writeln!(out, "[OK] Cellular IP: {} on {}", ip, iface).unwrap();
            if let Some(dns_ip) = dns {
                let _ = fs::write("/etc/resolv.conf",
                    format!("nameserver {}\n", dns_ip));
            }
            return;
        }
    }

    // If DHCP fails, try udhcpc
    let _ = std::process::Command::new("udhcpc")
        .args(["-i", "wwan0", "-q"])
        .output();
}

#[cfg(target_os = "linux")]
fn try_ppp(port: &str, apn: &str, out: &mut dyn Write) {
    // Create pppd options file
    let ppp_opts = format!(
        "{}\n\
         115200\n\
         debug\n\
         noauth\n\
         defaultroute\n\
         usepeerdns\n\
         connect \"/usr/sbin/chat -v '' AT OK ATD*99# CONNECT\"\n",
        port
    );

    if fs::write("/tmp/ppp-options", &ppp_opts).is_err() {
        writeln!(out, "[FAIL] Cannot write ppp options").unwrap();
        return;
    }

    let result = std::process::Command::new("pppd")
        .args(["file", "/tmp/ppp-options"])
        .output();

    match result {
        Ok(output) => {
            if output.status.success() {
                writeln!(out, "[OK] pppd started").unwrap();
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                writeln!(out, "[FAIL] pppd: {}", stderr.trim()).unwrap();
            }
        }
        Err(e) => {
            writeln!(out, "[FAIL] pppd not found: {}", e).unwrap();
            writeln!(out, "[INFO] PPP support requires pppd package").unwrap();
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn configure_data_interface(_out: &mut dyn Write) {}
#[cfg(not(target_os = "linux"))]
fn try_ppp(_port: &str, _apn: &str, _out: &mut dyn Write) {}
