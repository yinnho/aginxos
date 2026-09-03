//! 音频件（M42a）：采集（snd-cap 子进程）、WAV 封装、brain ASR/TTS HTTP、
//! 放音（snd-play 子进程）。
//!
//! brain 音频形状（2026-09-04 Mac 侧探明并实测，非文档推断）：
//! - ASR：POST /v1/chat/completions {"model":"audio", content 块
//!   {"type":"input_audio","input_audio":{"data":<b64 wav>,"format":"wav"}}}
//!   → choices[0].message.content。中文实测近完美（含"大写A小写B数字3"）。
//!   注意 type 必须是 "input_audio"；model 必须 "audio"（"asr" 会掉进严格
//!   chat 反序列化拒掉 input_audio 块）。
//! - TTS：POST {"model":"tts","messages":[user text],"audio_format":"wav",
//!   "sample_rate":48000} → {"output":{"audio":"/audio/<id>.mp3"}}（URL 后缀
//!   是假的，内容实为 WAV）→ GET 下载 → RIFF S16LE mono 48k，正是 snd-play
//!   吃的格式。voice 缺省 longxiaochun_v2（"Cherry" 会 418）。

use std::fs;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

pub const PCM_CAP: &str = "/dev/snd/pcmC0D0c";
pub const PCM_PLAY: &str = "/dev/snd/pcmC0D0p";
pub const SND_CAP: &str = "/bin/snd-cap";
pub const SND_PLAY: &str = "/bin/snd-play";
pub const RATE: u32 = 48_000; // M18 听 recipe 的已证形状（MM1 mono 48k）
pub const CHANS: u32 = 1;
pub const VOL: &str = "75"; // M18 收据：60 清晰、80 更响；75 折中
pub const CAP_MAX_SECS: u32 = 30;

fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(120)))
        .build();
    ureq::Agent::new_with_config(config)
}

pub struct Brain {
    base: String,
    key: String,
    agent: ureq::Agent,
}

/// ASR 候选词提示——词表契约（见 asr() 注释：当前 brain audio 模型无视它）。
const ASR_HINT: &str = "你听到的是一台中文设备的语音指令录音。请把听到的话原样转写成简体中文。\
    可能出现的指令词：无线、网络、连网、上网、状态、时间、几点、电池、取消、算了、\
    第一个、第二个、第三个、第四个、第五个、对、不对、是的、否、密码、大写、小写、\
    数字零到数字九、完了、删掉、退格、你在吗、帮助。";

impl Brain {
    /// key 从环境 AGINXBRAIN_API_KEY 读（agsvc 单元 env_file 注入）。
    pub fn from_env() -> Option<Brain> {
        let key = std::env::var("AGINXBRAIN_API_KEY").ok()?;
        let base = std::env::var("AGINXBRAIN_URL")
            .unwrap_or_else(|_| "https://brain.aginx.net".to_string());
        Some(Brain { base, key, agent: agent() })
    }

    /// WAV 字节 → 文本
    ///
    /// 2026-09-04 设备法医收据：brain 的 audio 模型无视 system 消息（有无
    /// ASR_HINT 结果逐字相同），且对本机采集链音频一律幻听或返回空——同一
    /// 句「连接无线网络」文件直喂全对（电平 12%/高通 200Hz 都过），经扬声
    /// 器隔空进麦克风后满刻度放大也废（→「很遗憾的呃。」）。云 ASR 与未
    /// 校准的 rt5514 裸 DMIC 链不兼容；真修法是 M42d 本地 ASR 换后端。
    /// ASR_HINT 保留当词表契约，换尊重 system 的后端时直接生效。
    pub fn asr(&self, wav: &[u8]) -> Result<String, String> {
        let b64 = b64_encode(wav);
        let body = serde_json::json!({
            "model": "audio",
            "messages": [
                { "role": "system", "content": ASR_HINT },
                {
                    "role": "user",
                    "content": [{
                        "type": "input_audio",
                        "input_audio": { "data": b64, "format": "wav" }
                    }]
                }
            ]
        });
        let resp = self
            .agent
            .post(format!("{}/v1/chat/completions", self.base))
            .header("Authorization", &format!("Bearer {}", self.key))
            .header("Content-Type", "application/json")
            .send(&body.to_string())
            .map_err(|e| format!("asr post: {e}"))?;
        let status = resp.status().as_u16();
        let txt = resp.into_body().read_to_string().map_err(|e| format!("asr body: {e}"))?;
        if status != 200 {
            return Err(format!("asr http {status}: {}", &txt[..txt.len().min(200)]));
        }
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| format!("asr json: {e}"))?;
        v.pointer("/choices/0/message/content")
            .and_then(|c| c.as_str())
            .map(String::from)
            .ok_or_else(|| format!("asr no content: {}", &txt[..txt.len().min(200)]))
    }

    /// 文本 → WAV 字节（S16LE mono 48k）
    pub fn tts(&self, text: &str) -> Result<Vec<u8>, String> {
        let body = serde_json::json!({
            "model": "tts",
            "messages": [{ "role": "user", "content": text }],
            "audio_format": "wav",
            "sample_rate": RATE
        });
        let resp = self
            .agent
            .post(format!("{}/v1/chat/completions", self.base))
            .header("Authorization", &format!("Bearer {}", self.key))
            .header("Content-Type", "application/json")
            .send(&body.to_string())
            .map_err(|e| format!("tts post: {e}"))?;
        let status = resp.status().as_u16();
        let txt = resp.into_body().read_to_string().map_err(|e| format!("tts body: {e}"))?;
        if status != 200 {
            return Err(format!("tts http {status}: {}", &txt[..txt.len().min(200)]));
        }
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| format!("tts json: {e}"))?;
        let path = v
            .pointer("/output/audio")
            .and_then(|a| a.as_str())
            .ok_or_else(|| format!("tts no audio url: {}", &txt[..txt.len().min(200)]))?;
        // GET 音频（URL 是 brain 的相对路径）
        let resp = self
            .agent
            .get(&format!("{}{}", self.base, path))
            .header("Authorization", &format!("Bearer {}", self.key))
            .call()
            .map_err(|e| format!("tts fetch: {e}"))?;
        if resp.status().as_u16() != 200 {
            return Err(format!("tts fetch http {}", resp.status()));
        }
        let mut buf = Vec::new();
        resp.into_body()
            .into_reader()
            .read_to_end(&mut buf)
            .map_err(|e| format!("tts read: {e}"))?;
        if buf.len() < 44 || &buf[0..4] != b"RIFF" {
            return Err("tts not wav".into());
        }
        Ok(buf)
    }

    /// 说一句话：TTS → 拆 WAV 头 → 复制成 L=R 立体声 → snd-play 阻塞放完。
    /// 音长上限 = 样本数/Rate + 5s 余量，防止挂死。
    ///
    /// 立体声不是可选的：QUIN_TDM_RX_0 后端是双通道，mono FE 在这张卡上
    /// 会话健康但无声（2026-09-04 收据：mono rms 26 / dft880 0.2，stereo
    /// rms 3650 / dft880 2396）。M18 的原收据也是 stereo。
    pub fn speak(&self, text: &str) -> Result<(), String> {
        let wav = self.tts(text)?;
        let (off, len) = wav_data_span(&wav)?;
        let raw = &wav[off..off + len];
        let samples = len / 2; // S16 mono in
        let mut stereo = Vec::with_capacity(len * 2);
        for s in raw.chunks_exact(2) {
            stereo.extend_from_slice(s);
            stereo.extend_from_slice(s); // L = R
        }
        let tmp = "/tmp/voiced-tts.raw";
        fs::write(tmp, &stereo).map_err(|e| format!("tts tmp: {e}"))?;
        let budget = samples / RATE as usize + 5;
        let mut child = Command::new(SND_PLAY)
            .args([PCM_PLAY, tmp, &RATE.to_string(), "2", VOL])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("snd-play spawn: {e}"))?;
        wait_limited(&mut child, budget as u32)
    }
}

// ---------------- capture ----------------

/// 起一次最长 CAP_MAX_SECS 的采集；PTT 松手时 kill，采到多少算多少。
pub fn capture_start() -> std::io::Result<Child> {
    Command::new(SND_CAP)
        .args([
            PCM_CAP,
            &CAP_MAX_SECS.to_string(),
            "/tmp/voiced-cap.raw",
            &RATE.to_string(),
            &CHANS.to_string(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

/// 读采集产物并封 WAV。过短（<0.1s）返回 None（误触）。
pub fn capture_take() -> Option<Vec<u8>> {
    let raw = fs::read("/tmp/voiced-cap.raw").ok()?;
    let raw = &raw[..raw.len() - raw.len() % 2]; // 整样本截齐
    if raw.len() < (RATE as usize / 10) * 2 {
        return None;
    }
    Some(wav_wrap(raw, RATE, CHANS))
}

// ---------------- wav ----------------

pub fn wav_wrap(raw: &[u8], rate: u32, chans: u32) -> Vec<u8> {
    let mut h = Vec::with_capacity(44 + raw.len());
    let byte_rate = rate * chans * 2;
    let block_align = (chans * 2) as u16;
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&((36 + raw.len()) as u32).to_le_bytes());
    h.extend_from_slice(b"WAVE");
    h.extend_from_slice(b"fmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes()); // PCM
    h.extend_from_slice(&(chans as u16).to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&byte_rate.to_le_bytes());
    h.extend_from_slice(&block_align.to_le_bytes());
    h.extend_from_slice(&16u16.to_le_bytes()); // bits
    h.extend_from_slice(b"data");
    h.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    h.extend_from_slice(raw);
    h
}

/// 遍历 RIFF 块找 data 的 (offset, len)。TTS 产物块序不保证 44 定长。
pub fn wav_data_span(wav: &[u8]) -> Result<(usize, usize), String> {
    if wav.len() < 12 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return Err("not riff".into());
    }
    let mut off = 12;
    while off + 8 <= wav.len() {
        let id = &wav[off..off + 4];
        let sz = u32::from_le_bytes([wav[off + 4], wav[off + 5], wav[off + 6], wav[off + 7]]) as usize;
        let body = off + 8;
        if body + sz > wav.len() {
            // brain 的 TTS 是流式合成：未知长度打成 0x7fffffff 哨兵
            // (2026-09-04 设备收据)。data 块实际跑到文件尾——只有 data
            // 可以这么收敛，fmt 被截是真错误。
            if id == b"data" {
                let len = wav.len() - body - (wav.len() - body) % 2;
                return Ok((body, len));
            }
            return Err("riff chunk overruns".into());
        }
        if id == b"data" {
            return Ok((body, sz));
        }
        off = body + sz + (sz % 2); // 块按 2 对齐
    }
    Err("no data chunk".into())
}

// ---------------- base64（标准字母表，含 padding；手写避免加依赖） ----------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

// ---------------- 子进程等待 ----------------

/// 轮询等子进程退出，超秒数杀之（返回错误）。wifi-join/snd-play 都可能挂。
pub fn wait_limited(child: &mut Child, secs: u32) -> Result<(), String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(secs as u64);
    loop {
        match child.try_wait() {
            Ok(Some(st)) => {
                return if st.success() {
                    Ok(())
                } else {
                    Err(format!("exit {st}"))
                };
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("timeout".into());
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => return Err(format!("wait: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64_known_vectors() {
        assert_eq!(b64_encode(b""), "");
        assert_eq!(b64_encode(b"f"), "Zg==");
        assert_eq!(b64_encode(b"fo"), "Zm8=");
        assert_eq!(b64_encode(b"foo"), "Zm9v");
        assert_eq!(b64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn wav_wrap_and_span_roundtrip() {
        let raw = vec![0u8; 1000];
        let wav = wav_wrap(&raw, RATE, 1);
        let (off, len) = wav_data_span(&wav).unwrap();
        assert_eq!(&wav[off..off + len], &raw[..]);
        // fmt 块非 data，跳过后命中 data
        assert_eq!(off, 44);
    }

    #[test]
    fn wav_span_handles_extra_chunks() {
        // RIFF + 一个 junk 块（奇数长度，吃 padding）+ data
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&0u32.to_le_bytes());
        w.extend_from_slice(b"WAVE");
        w.extend_from_slice(b"junk");
        w.extend_from_slice(&3u32.to_le_bytes());
        w.extend_from_slice(b"abc");
        w.extend_from_slice(&[0]); // padding
        w.extend_from_slice(b"data");
        w.extend_from_slice(&2u32.to_le_bytes());
        w.extend_from_slice(b"xy");
        let (off, len) = wav_data_span(&w).unwrap();
        assert_eq!(&w[off..off + len], b"xy");
    }
}
