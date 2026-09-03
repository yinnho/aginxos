//! voiced — 语音对话守护（M42a，产品定义 2026-09-04：手机即智能体）。
//!
//! 产品面唯一的输入是语音（PTT=按住音量下键）和眼（M42b），输出是嘴
//! （TTS）和脸（/run/voice/face → aterm 渲染）。协议是封闭词表的确定性
//! 状态机（protocol.rs），没有 LLM——WiFi 必须在 LLM 可用之前连得上。
//!
//! 调试面（收据阶梯，从嘴/耳单器官到全环）：
//!   voiced --say "文本"          只测嘴（TTS→扬声器）
//!   voiced --hear <wav文件>      只测耳（WAV→ASR→打印文本）
//!   voiced --inject "文本"       喂状态机走全流程（不出声，Act 真执行）
//!   voiced --face                打印当前屏面 JSON
//!
//! 没有嘴耳同开的回环自检：M18 的硬件收据写明 MM1 边放边采会把放音叠
//! 进采集（数字回环是失真副本，880Hz 可验、语音不可认，2026-09-04 实测
//! ASR 出"うん、うん"）——产品路径本来也是顺序的：PTT 采完才 TTS。

mod audio;
mod face;
mod ptt;
mod protocol;

use protocol::{Act, Ev, Out, Vm};
use std::process::Command;
use std::time::{Duration, Instant};

const TIMEOUT_SECS: u64 = 45; // 提示后无语音的退出时限
const JOIN_BUDGET_SECS: u32 = 90;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--say") => {
            let text = args.get(2).expect("usage: voiced --say <text>");
            let brain = audio::Brain::from_env().expect("AGINXBRAIN_API_KEY not set");
            brain.speak(text).expect("speak failed");
        }
        Some("--hear") => {
            let path = args.get(2).expect("usage: voiced --hear <wav>");
            let brain = audio::Brain::from_env().expect("AGINXBRAIN_API_KEY not set");
            let wav = std::fs::read(path).expect("read wav");
            let text = brain.asr(&wav).expect("asr failed");
            println!("{text}");
        }
        Some("--inject") => {
            let text = args.get(2).expect("usage: voiced --inject <text>").clone();
            let mut vm = Vm::new();
            face::write(&vm, false, false);
            let outs = vm.step(Ev::Heard(text));
            run_outs(&mut vm, outs, None);
        }
        Some("--face") => match face::read() {
            Some(s) => println!("{s}"),
            None => println!("(no face)"),
        },
        _ => daemon(),
    }
}

fn daemon() {
    let brain = audio::Brain::from_env();
    let mut vm = Vm::new();
    let mut ptt = ptt::Ptt::open();
    if ptt.is_none() {
        eprintln!("voiced: no {} — PTT dead, face only", ptt::PTT_DEV);
    }
    face::write(&vm, false, false);
    eprintln!("voiced: up (brain={}, ptt={})", brain.is_some(), ptt.is_some());

    let mut capturing: Option<std::process::Child> = None;
    let mut deadline: Option<Instant> = None;

    let mut pollfds = [libc::pollfd { fd: -1, events: libc::POLLIN, revents: 0 }];
    loop {
        // ---- PTT ----
        if let Some(p) = ptt.as_mut() {
            pollfds[0].fd = p.fd();
            let n = unsafe { libc::poll(pollfds.as_mut_ptr(), 1, 200) };
            if n > 0 && pollfds[0].revents & libc::POLLIN != 0 {
                for ev in p.poll() {
                    match ev {
                        ptt::PttEv::Down => {
                            if capturing.is_none() {
                                match audio::capture_start() {
                                    Ok(c) => {
                                        capturing = Some(c);
                                        deadline = None; // 采集中不计时
                                        face::write(&vm, true, false);
                                    }
                                    Err(e) => eprintln!("voiced: cap start {e}"),
                                }
                            }
                        }
                        ptt::PttEv::Up => {
                            if let Some(mut c) = capturing.take() {
                                let _ = c.kill();
                                let _ = c.wait();
                                if let Some(brain) = brain.as_ref() {
                                    if let Some(wav) = audio::capture_take() {
                                        face::write(&vm, false, true);
                                        match brain.asr(&wav) {
                                            Ok(text) => {
                                                eprintln!("voiced: heard {text:?}");
                                                let outs = vm.step(Ev::Heard(text));
                                                run_outs(&mut vm, outs, Some(brain));
                                            }
                                            Err(e) => {
                                                eprintln!("voiced: asr {e}");
                                                let outs = vm.step(Ev::Heard("没听懂".into()));
                                                // asr 失败提示本身也要能说——但 asr
                                                // 挂了多半网络不通，TTS 也挂；只刷屏
                                                for o in outs {
                                                    if o == Out::Show {
                                                        face::write(&vm, false, false);
                                                    }
                                                }
                                            }
                                        }
                                    } else {
                                        // 误触（<0.1s）
                                        face::write(&vm, false, false);
                                    }
                                }
                                face::write(&vm, false, false);
                            }
                        }
                    }
                }
            }
        } else {
            std::thread::sleep(Duration::from_millis(200));
        }

        // ---- 超时 ----
        if capturing.is_none() && !matches!(vm.state_name(), "idle") {
            let dl = *deadline.get_or_insert_with(|| Instant::now() + Duration::from_secs(TIMEOUT_SECS));
            if Instant::now() >= dl {
                deadline = None;
                let outs = vm.step(Ev::Timeout);
                run_outs(&mut vm, outs, brain.as_ref());
            }
        } else {
            deadline = None;
        }
    }
}

/// 落地状态机输出：Say→TTS+屏，Act→执行并把结果喂回状态机。
fn run_outs(vm: &mut Vm, outs: Vec<Out>, brain: Option<&audio::Brain>) {
    let mut followups: Vec<Ev> = Vec::new();
    for o in outs {
        match o {
            Out::Say(s) => {
                face::write(vm, false, true);
                if let Some(b) = brain {
                    if let Err(e) = b.speak(&s) {
                        eprintln!("voiced: tts {e}");
                    }
                } else {
                    eprintln!("voiced: (mute) {s}");
                }
            }
            Out::Show => {}
            Out::Act(a) => match a {
                Act::Scan => {
                    face::write(vm, false, true);
                    match scan_ssids() {
                        Ok(list) => followups.push(Ev::ScanDone(list)),
                        Err(e) => {
                            eprintln!("voiced: scan {e}");
                            followups.push(Ev::ScanDone(Vec::new()));
                        }
                    }
                }
                Act::Join { ssid, psk } => {
                    face::write(vm, false, true);
                    followups.push(Ev::JoinDone(join_wifi(&ssid, &psk)));
                }
                Act::Status => {
                    if let Some(b) = brain {
                        let o = vm.inject_say(&status_text());
                        if let Out::Say(s) = o {
                            if let Err(e) = b.speak(&s) {
                                eprintln!("voiced: tts {e}");
                            }
                        }
                    }
                }
            },
        }
    }
    face::write(vm, false, false);
    for ev in followups {
        let outs = vm.step(ev);
        run_outs(vm, outs, brain);
    }
}

// ---------------- 执行件 ----------------

/// nlscan wlan0 → 去重（保信号最强）、滤 hidden、按信号排序，cap 10（序数上限）。
fn scan_ssids() -> Result<Vec<String>, String> {
    let out = Command::new("/bin/nlscan")
        .arg("wlan0")
        .output()
        .map_err(|e| format!("spawn: {e}"))?;
    let txt = String::from_utf8_lossy(&out.stdout);
    // 行形状: "<mac>  ch=<n>  -68.00  dBm  <ssid 可能 \xNN 转义>" — dBm 是
    // 独立 token，SSID 从它之后开始（2026-09-04 设备定格；此前把 dBm 当
    // SSID 前缀，列表全是 "dBm xxx"）。
    let mut seen: Vec<(String, f32)> = Vec::new();
    for line in txt.lines() {
        let line = line.trim_end();
        let toks: Vec<&str> = line.split_whitespace().collect();
        // mac, ch=, dbm 数值, "dBm", ssid...（ssid 可含空格，join 回去）
        let dbm = match toks.get(2).and_then(|d| d.trim_end_matches("dBm").parse::<f32>().ok()) {
            Some(v) => v,
            None => continue,
        };
        let ssid_start = if toks.get(3) == Some(&"dBm") { 4 } else { 3 };
        let ssid_esc: String = toks.iter().skip(ssid_start).copied().collect::<Vec<_>>().join(" ");
        if ssid_esc.is_empty() || ssid_esc.contains("<hidden>") {
            continue;
        }
        let ssid = unescape_hex(&ssid_esc);
        // 邻居 AP 会有二进制 SSID（\x04\x00…）——念不出来也画不出来，滤掉
        if ssid.is_empty() || ssid.chars().any(|c| c.is_control()) {
            continue;
        }
        if let Some(slot) = seen.iter_mut().find(|(s, _)| *s == ssid) {
            if dbm > slot.1 {
                slot.1 = dbm;
            }
        } else {
            seen.push((ssid, dbm));
        }
    }
    seen.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    Ok(seen.into_iter().take(10).map(|(s, _)| s).collect())
}

/// busybox 输出把非 ASCII 打成 \xe5\x87 字样；解回 UTF-8。
fn unescape_hex(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if i + 3 < bytes.len() + 1 && bytes[i] == b'\\' && bytes[i + 1] == b'x' {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(bytes[i + 2]), hex(bytes[i + 3])) {
                out.push((h * 16 + l) as u8);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// wifi-join wlan0 ssid psk，然后读 wlan0 的 IPv4。
fn join_wifi(ssid: &str, psk: &str) -> Result<String, String> {
    let mut child = Command::new("/bin/wifi-join")
        .args(["wlan0", ssid, psk])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn: {e}"))?;
    audio::wait_limited(&mut child, JOIN_BUDGET_SECS).map_err(|e| format!("wifi-join {e}"))?;
    // dhcp 在 wifi-join 里；地址落不落直接看
    for _ in 0..10 {
        if let Ok(out) = Command::new("ip").args(["-4", "addr", "show", "wlan0"]).output() {
            let txt = String::from_utf8_lossy(&out.stdout);
            for line in txt.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("inet ") {
                    if let Some(ip) = rest.split_whitespace().next() {
                        if ip != "127.0.0.1" {
                            return Ok(ip.to_string());
                        }
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err("没拿到地址".into())
}

/// 状态一句话：时间 + 电池 + 网络。
fn status_text() -> String {
    let time = Command::new("date").arg("+%H点%M分").output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let bat = std::fs::read_to_string("/sys/class/power_supply/battery/capacity")
        .ok()
        .and_then(|s| s.trim().parse::<u8>().ok())
        .unwrap_or(0);
    let ip = Command::new("ip").args(["-4", "addr", "show", "wlan0"]).output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|t| {
            t.lines().find_map(|l| {
                let l = l.trim();
                l.strip_prefix("inet ")?.split_whitespace().next().map(String::from)
            })
        })
        .unwrap_or_else(|| "无网络".into());
    format!("{time}，电池{bat}%，{ip}。")
}
