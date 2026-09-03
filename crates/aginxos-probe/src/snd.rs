//! snd — raw S16_LE PCM playback on the q6 front-end, no alsa-lib.
//!
//! M41 audio half: the A/V-sync clock for `vidc play`. The setup idiom is
//! lifted from M18's device-proven snd-play.c (boot/rootfs/src/snd-play.c):
//! HW_REFINE first, then pin format/rate/ch only (the cDSP rejects pinned
//! period/buffer quanta), loose bounds, start_threshold=rate/4, WRITEI in
//! period-sized chunks. Two deviations, both deliberate:
//!
//! - the fd stays O_NONBLOCK: a blocking WRITEI can sleep a full period
//!   inside the substream lock path, which would stall the decode loop's
//!   DELAY queries. The feeder polls POLLOUT instead — snd_pcm poll is the
//!   same mechanism alsa-lib itself uses.
//! - SNDRV_PCM_IOCTL_DELAY is the one new primitive: frames queued but not
//!   yet played, i.e. the played-frames counter that makes the audio device
//!   the master clock.
//!
//! Layouts are frozen against the same _Static_asserts the C header carries
//! (sizes 608/136/24 and the ioctl request words below, all verified against
//! the redfin kernel's snd_pcm dispatch in M18).

use std::fs;
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

// --- uapi structs (sound/asound.h subset; sizes frozen) ---------------------

#[repr(C)]
struct SndMask {
    bits: [u32; 8], // 256-bit bitmap
}
#[repr(C)]
#[derive(Clone, Copy)]
struct SndInterval {
    min: u32,
    max: u32,
    flags: u32, // openmin/openmax/integer/empty bitfield; always 0 here
}

#[repr(C)]
struct SndPcmHwParams {
    flags: u32,
    masks: [SndMask; 3], // ids 0..2: ACCESS FORMAT SUBFORMAT
    mres: [SndMask; 5],
    intervals: [SndInterval; 12], // ids 8..19
    ires: [SndInterval; 9],
    rmask: u32,
    cmask: u32,
    info: u32,
    msbits: u32,
    rate_num: u32,
    rate_den: u32,
    fifo_size: u64,
    reserved: [u8; 64],
}

#[repr(C)]
struct SndPcmSwParams {
    tstamp_mode: i32,
    period_step: u32,
    sleep_min: u32, // long-obsolete, must be 0
    avail_min: u64,
    xfer_align: u64, // obsolete, must be 1
    start_threshold: u64,
    stop_threshold: u64,
    silence_threshold: u64,
    silence_size: u64,
    boundary: u64, // kernel writes this back
    proto: u32,    // >=6.12 only; 4.19: reserved
    tstamp_type: u32,
    reserved: [u8; 56],
}

#[repr(C)]
struct SndXferi {
    result: i64, // snd_pcm_sframes_t
    buf: *const i16,
    frames: u64,
}

// ioctl request words, frozen from snd-pcm-uapi.h asserts (device-verified
// M18: numbers matched against the kernel's snd_pcm_common_ioctl dispatch).
const SNDRV_PCM_IOCTL_HW_REFINE: u32 = 0xc260_4110;
const SNDRV_PCM_IOCTL_HW_PARAMS: u32 = 0xc260_4111;
const SNDRV_PCM_IOCTL_SW_PARAMS: u32 = 0xc088_4113;
const SNDRV_PCM_IOCTL_PREPARE: u32 = 0x4140;
const SNDRV_PCM_IOCTL_DRAIN: u32 = 0x4144;
const SNDRV_PCM_IOCTL_WRITEI_FRAMES: u32 = 0x4018_4150;
/// _IOR('A', 0x21, snd_pcm_sframes_t) — frames queued but not yet played.
const SNDRV_PCM_IOCTL_DELAY: u32 = 0x8008_4121;

// hw param ids (uapi), split mask vs interval
const P_ACCESS: usize = 0;
const P_FORMAT: usize = 1;
const P_CHANNELS: usize = 10;
const P_RATE: usize = 11;
const P_PERIOD_SIZE: usize = 13;
const P_BUFFER_SIZE: usize = 17;

const SNDRV_PCM_ACCESS_RW_INTERLEAVED: u32 = 3;
const SNDRV_PCM_FORMAT_S16_LE: u32 = 2;

fn ioctl<T>(fd: i32, req: u32, arg: *mut T) -> i32 {
    unsafe { libc::ioctl(fd, req as _, arg) }
}

fn mask_one(m: &mut SndMask, bit: u32) {
    *m = SndMask { bits: [0; 8] };
    m.bits[(bit / 32) as usize] |= 1u32 << (bit % 32);
}

fn iv<'a>(p: &'a mut SndPcmHwParams, id: usize) -> &'a mut SndInterval {
    &mut p.intervals[id - 8] // interval ids run 8..19
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

// --- player -----------------------------------------------------------------

/// One configured, PREPAREd q6 playback stream (MM1 → QUIN_TDM_RX_0, the
/// mixer routing audio-bringup bakes at boot). All methods take &self: the
/// only state is the fd, and the kernel serializes ioctls per substream —
/// the feeder thread's WRITEI and the decode loop's DELAY share it safely.
pub struct PcmPlayer {
    file: fs::File,
    pub rate: u32,
    pub chans: u32,
}

impl PcmPlayer {
    /// `dev` e.g. "/dev/snd/pcmC0D0p" (MM1 playback — the path M18 proved).
    /// Stays O_NONBLOCK for the lifetime (see module doc).
    pub fn open(dev: &str, rate: u32, chans: u32) -> Result<PcmPlayer, String> {
        let file = fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(dev)
            .map_err(|e| format!("open {dev}: {e}"))?;

        // HW_REFINE with everything open first (fills the kernel's defaults),
        // then pin format/rate/ch. M18 lesson kept: pinning period/buffer to
        // nice round numbers makes HW_PARAMS EINVAL — the cDSP only accepts
        // its own quantum, so period/buffer stay loosely bounded.
        let mut hp: SndPcmHwParams = unsafe { std::mem::zeroed() };
        for m in hp.masks.iter_mut() {
            m.bits = [!0u32; 8];
        }
        for v in hp.intervals.iter_mut() {
            *v = SndInterval { min: 0, max: u32::MAX, flags: 0 };
        }
        hp.rmask = !0;
        if ioctl(file.as_raw_fd(), SNDRV_PCM_IOCTL_HW_REFINE, &mut hp) < 0 {
            return Err(format!("open {dev}: HW_REFINE errno={}", errno()));
        }
        mask_one(&mut hp.masks[P_ACCESS], SNDRV_PCM_ACCESS_RW_INTERLEAVED);
        mask_one(&mut hp.masks[P_FORMAT], SNDRV_PCM_FORMAT_S16_LE);
        iv(&mut hp, P_CHANNELS).min = chans;
        iv(&mut hp, P_CHANNELS).max = chans;
        iv(&mut hp, P_RATE).min = rate;
        iv(&mut hp, P_RATE).max = rate;
        iv(&mut hp, P_PERIOD_SIZE).min = 16;
        iv(&mut hp, P_PERIOD_SIZE).max = rate / 2;
        iv(&mut hp, P_BUFFER_SIZE).min = rate;
        iv(&mut hp, P_BUFFER_SIZE).max = rate.saturating_mul(4);
        hp.rmask = (1 << P_ACCESS)
            | (1 << P_FORMAT)
            | (1 << P_CHANNELS)
            | (1 << P_RATE)
            | (1 << P_PERIOD_SIZE)
            | (1 << P_BUFFER_SIZE);
        if ioctl(file.as_raw_fd(), SNDRV_PCM_IOCTL_HW_PARAMS, &mut hp) < 0 {
            return Err(format!("open {dev}: HW_PARAMS errno={}", errno()));
        }

        let mut sp: SndPcmSwParams = unsafe { std::mem::zeroed() };
        sp.avail_min = (rate / 4) as u64;
        sp.xfer_align = 1; // obsolete but old kernels validate it
        sp.start_threshold = (rate / 4) as u64; // first period kicks off playback
        sp.stop_threshold = rate as u64 * 4;
        if ioctl(file.as_raw_fd(), SNDRV_PCM_IOCTL_SW_PARAMS, &mut sp) < 0 {
            return Err(format!("open {dev}: SW_PARAMS errno={}", errno()));
        }
        if ioctl(file.as_raw_fd(), SNDRV_PCM_IOCTL_PREPARE, std::ptr::null_mut::<u8>()) < 0 {
            return Err(format!("open {dev}: PREPARE errno={}", errno()));
        }
        Ok(PcmPlayer { file, rate, chans })
    }

    /// Nonblocking interleaved write. Returns frames accepted (0 = no room
    /// yet — poll POLLOUT and retry; partial counts are normal).
    pub fn write(&self, samples: &[i16]) -> Result<u64, String> {
        let frames = samples.len() / self.chans as usize;
        if frames == 0 {
            return Ok(0);
        }
        let mut x = SndXferi {
            result: 0,
            buf: samples.as_ptr(),
            frames: frames as u64,
        };
        if ioctl(self.file.as_raw_fd(), SNDRV_PCM_IOCTL_WRITEI_FRAMES, &mut x) < 0 {
            let e = errno();
            if e == libc::EAGAIN || e == libc::EINTR {
                return Ok(0);
            }
            return Err(format!("WRITEI errno={e}"));
        }
        Ok(x.result.max(0) as u64)
    }

    /// Frames queued to the device but not yet played (uapi DELAY); -1 on
    /// error. Safe alongside a feeder's WRITEI: both are short ioctls here
    /// (nothing blocks — the fd is O_NONBLOCK).
    pub fn delay(&self) -> i64 {
        let mut d: i64 = 0;
        if ioctl(self.file.as_raw_fd(), SNDRV_PCM_IOCTL_DELAY, &mut d) < 0 {
            return -1;
        }
        d
    }

    /// Wait until the stream can accept more frames (POLLOUT side of the
    /// snd_pcm poll contract), bounded by `timeout_ms`.
    pub fn poll_writable(&self, timeout_ms: i32) {
        let mut pfd = libc::pollfd {
            fd: self.file.as_raw_fd(),
            events: libc::POLLOUT,
            revents: 0,
        };
        unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
    }

    pub fn drain(&self) {
        ioctl(
            self.file.as_raw_fd(),
            SNDRV_PCM_IOCTL_DRAIN,
            std::ptr::null_mut::<u8>(),
        );
    }
}

// --- A/V clock ---------------------------------------------------------------

/// Audio-side master clock: a feeder thread pumps the raw PCM file while the
/// decode loop queries the played position (written − delay) and gates each
/// frame's scanout on it. The feeder is NOT started at construction — the
/// decoder calls `start()` when its first capture frame is in hand, so the
/// audio timeline begins against a ready display path instead of against
/// ~1 s of decode setup.
pub struct AvAudio {
    player: Arc<PcmPlayer>,
    written: Arc<AtomicI64>, // frames accepted by WRITEI so far
    stop: Arc<AtomicBool>,
    done: Arc<AtomicBool>, // feeder hit EOF and drained
    handle: Option<std::thread::JoinHandle<()>>,
    started: AtomicBool,
    start_at: Instant, // fallback anchor if DELAY misbehaves on this FE
    delay_ok: AtomicBool,
    rate: u32,
    pcm_path: String,
    vol: i32,
}

impl AvAudio {
    /// Configure the stream (HW/SW params + PREPARE) without playing yet.
    /// `pcm_path` is raw interleaved S16_LE at `rate`/`chans`; `vol` 0..100
    /// scales it (integer math, same as snd-play).
    pub fn new(dev: &str, rate: u32, chans: u32, pcm_path: &str, vol: i32) -> Result<AvAudio, String> {
        Ok(AvAudio {
            player: Arc::new(PcmPlayer::open(dev, rate, chans)?),
            written: Arc::new(AtomicI64::new(0)),
            stop: Arc::new(AtomicBool::new(false)),
            done: Arc::new(AtomicBool::new(false)),
            handle: None,
            started: AtomicBool::new(false),
            start_at: Instant::now(),
            delay_ok: AtomicBool::new(true),
            rate,
            pcm_path: pcm_path.to_string(),
            vol,
        })
    }

    /// Feeder already running?
    pub fn started(&self) -> bool {
        self.started.load(Ordering::SeqCst)
    }

    /// Kick off the feeder thread (idempotent).
    pub fn start(&mut self) -> Result<(), String> {
        if self.started.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.start_at = Instant::now();
        let player = Arc::clone(&self.player);
        let written = Arc::clone(&self.written);
        let stop = Arc::clone(&self.stop);
        let done = Arc::clone(&self.done);
        let path = self.pcm_path.clone();
        let vol = self.vol;
        let chans = player.chans as usize;
        let period = player.rate as usize / 4; // one period per write
        self.handle = Some(
            std::thread::Builder::new()
                .name("av-pcm-feeder".into())
                .spawn(move || {
                    let data = match fs::read(&path) {
                        Ok(d) => d,
                        Err(e) => {
                            eprintln!("snd: read {path}: {e}");
                            return;
                        }
                    };
                    let samples: Vec<i16> = data
                        .chunks_exact(2)
                        .map(|c| i16::from_le_bytes([c[0], c[1]]))
                        .collect();
                    let mut scaled = samples;
                    if vol != 100 {
                        for s in scaled.iter_mut() {
                            let v = (*s as i32) * vol / 100;
                            *s = v.clamp(-32768, 32767) as i16;
                        }
                    }
                    // Speaker guard (observed 2026-09-03): the CS35L41 smart
                    // amps run WITHOUT their protection firmware (no cs35l41
                    // fw in /vendor/firmware, no ACDB HAL to calibrate it),
                    // so drive is unbounded — loud or bass-heavy content
                    // buzzes the speakers, worse the louder it gets (same
                    // file clean on a Mac; beeps/TTS never triggered it).
                    // Until the vendor protection chain is ported, music/
                    // video through this path gets a one-pole 180 Hz
                    // high-pass (excursion) plus a running peak limiter at
                    // ~-4 dBFS (peaks), applied after the volume knob.
                    let hp_a = (-2.0 * std::f32::consts::PI * 180.0 / player.rate as f32).exp();
                    let mut hp_x = vec![0.0f32; chans];
                    let mut hp_y = vec![0.0f32; chans];
                    let mut env = 0.0f32;
                    let mut g = 1.0f32;
                    const CEIL: f32 = 20_300.0; // ≈ -4.2 dBFS
                    for (i, s) in scaled.iter_mut().enumerate() {
                        let c = i % chans;
                        let x = *s as f32;
                        let y = x - hp_x[c] + hp_a * hp_y[c];
                        hp_x[c] = x;
                        hp_y[c] = y;
                        env = env.max(y.abs()) * 0.999_92;
                        let target = (CEIL / env.max(1.0)).min(1.0);
                        g = if target < g {
                            target // attack: pull down immediately
                        } else {
                            g + (target - g) * 0.000_2 // release: ease back up
                        };
                        *s = (y * g).clamp(-32768.0, 32767.0) as i16;
                    }
                    let mut off = 0usize;
                    while !stop.load(Ordering::SeqCst) && off < scaled.len() {
                        let end = (off + period * chans).min(scaled.len());
                        match player.write(&scaled[off..end]) {
                            Ok(0) => player.poll_writable(5),
                            Ok(n) => {
                                written.fetch_add(n as i64, Ordering::SeqCst);
                                off += n as usize * chans;
                            }
                            Err(e) => {
                                eprintln!("snd: {e}");
                                return;
                            }
                        }
                    }
                    // let the tail play out before the main loop tears down
                    // (nonblock DRAIN: flips to DRAINING, returns EAGAIN, the
                    // buffer empties on its own)
                    player.drain();
                    done.store(true, Ordering::SeqCst);
                    eprintln!(
                        "snd: feeder done ({} frames queued)",
                        written.load(Ordering::SeqCst)
                    );
                })
                .map_err(|e| format!("spawn feeder: {e}"))?,
        );
        Ok(())
    }

    /// Played audio time in µs since stream start: frames accepted minus
    /// frames still queued. After the feeder's EOF drain the substream lands
    /// in SETUP, where DELAY answers EBADFD (do_pcm_hwsync has no case for
    /// it) — that's not an error, the audio is simply finished, so the gate
    /// opens fully. A DELAY failure while still feeding is real: flip to a
    /// wall-clock fallback and say so once.
    pub fn played_us(&self) -> i64 {
        if self.delay_ok.load(Ordering::SeqCst) {
            let d = self.player.delay();
            if d < 0 {
                if self.done.load(Ordering::SeqCst) {
                    return self.written.load(Ordering::SeqCst) * 1_000_000 / self.rate as i64;
                }
                self.delay_ok.store(false, Ordering::SeqCst);
                eprintln!("snd: DELAY errno={} — falling back to wall clock", errno());
            } else {
                let w = self.written.load(Ordering::SeqCst);
                let played = (w - d).max(0);
                return played * 1_000_000 / self.rate as i64;
            }
        }
        self.start_at.elapsed().as_micros() as i64
    }

    /// Block until the audio clock reaches `pts_us` (minus a small submit
    /// lead so the vblank-timed SETPLANE lands on the audio instant).
    /// After the feeder drains, the sample counter freezes at the audio
    /// end — a pts past it (video one frame longer than the audio track;
    /// observed as a permanent hang on a 45.25 s track vs 45.27 s video)
    /// would spin forever. Both clocks tick 1x from the same origin, so
    /// the tail paces on the wall clock anchored at feeder start instead.
    pub fn wait_until(&self, pts_us: i64) {
        const LEAD_US: i64 = 4_000;
        loop {
            let now = if self.done.load(Ordering::SeqCst) {
                self.start_at.elapsed().as_micros() as i64
            } else {
                self.played_us()
            };
            if now + LEAD_US >= pts_us {
                return;
            }
            let remain = (pts_us - LEAD_US - now).clamp(1_000, 4_000) as u64;
            std::thread::sleep(Duration::from_micros(remain));
        }
    }

    /// Wait for the feeder to finish its file and drain the tail.
    pub fn finish(&mut self) {
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for AvAudio {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.finish();
    }
}
