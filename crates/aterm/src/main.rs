// aterm — AginxOS on-device terminal (M11).
//
// bootcard's DRM path + 5x8 font, a vte-parsed cell grid (black bg, green /
// white text — the fixed phosphor palette), an openpty child (sh / codex /
// grok / aclone), an evdev on-screen keyboard (tap = key, drag = scrollback),
// and a launcher (clone / codex / grok / sh). Started by rcS's aterm-handoff
// once boot finishes: bootcard never exits on its own and holds DRM master
// forever, so the handoff kills it by /run/bootcard.pid and takes the panel.
//
// M15 power management: the qpnp_pon power key (event1) blanks the panel
// (connector DPMS off — the same path that darkened the screen when a DRM
// master dropped), a second short press or any touch wakes it, 60 s idle
// blanks too, holding the key ~1.2 s (or the launcher's POWER OFF / RESTART
// buttons) runs `reboot2 poweroff|reboot`.
//
// M17 input split: the keyboard hit tests return typed InputEvents
// (KeyEvent vs TextInputEvent, input.rs) and EVERY write to the pty goes
// through inject() — the same entry point M18's voice input will call
// with recognized text. ATERM_INJECT=1 watches /run/aterm.inject: any
// process drops text there, it types into the session verbatim (that's
// the voice path, testable without audio).
//
// Host verification: `aterm --ppm out.ppm` renders the launcher into a P6
// PPM without touching DRM (same pattern as bootcard --ppm).

mod cjk;
mod drm;
mod font;
mod input;
mod kb;
mod launch;
mod photos;
mod term;

use drm::Drm;
use input::InputEvent;
use kb::{Act, Kb, KeyDef, KeyGeom, KeyReader, Touch, TouchReader, KEY_POWER};
use std::io::Write as _;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::time::{Duration, Instant};
use term::{Style, Term};

const BG: u32 = 0x00000000;
const GREEN: u32 = 0x0034D399;
const WHITE: u32 = 0x00F5F7FA;
const DIM: u32 = 0x001E3A2E; // key outlines / separators
const ROW_GAP: usize = 8; // extra px between terminal text rows
const KEYCAP: u32 = 0x000A1410; // key fill
const UNAVAIL: u32 = 0x00115A3F; // dimmed green for missing apps

// M15 power: short press (< POWER_HOLD) toggles blank; hold at or beyond it
// shuts down; IDLE_BLANK without input blanks the screen.
const POWER_HOLD: Duration = Duration::from_millis(1200);
const IDLE_BLANK: Duration = Duration::from_secs(60);

fn fill_rect(pix: &mut [u32], pitch: usize, w: usize, h: usize, x: i32, y: i32, rw: i32, rh: i32, c: u32) {
    let (mut x, mut y, mut rw, mut rh) = (x, y, rw, rh);
    if rw <= 0 || rh <= 0 {
        return;
    }
    if x < 0 {
        rw += x;
        x = 0;
    }
    if y < 0 {
        rh += y;
        y = 0;
    }
    if x + rw > w as i32 {
        rw = w as i32 - x;
    }
    if y + rh > h as i32 {
        rh = h as i32 - y;
    }
    if rw <= 0 || rh <= 0 {
        return;
    }
    for j in 0..rh as usize {
        let row = (y as usize + j) * pitch + x as usize;
        for i in 0..rw as usize {
            pix[row + i] = c;
        }
    }
}

// Glyph lookup for terminal cells. The built-in font is 7-bit ASCII only,
// but the TUIs we host (grok, codex) draw borders/spinners with Unicode
// box-drawing, blocks and braille. Render those procedurally in the same
// 5x8 bitmap format instead of truncating the codepoint to a random ASCII
// glyph. Anything else non-ASCII falls back to '?'.
fn glyph(font: &[[u8; 8]; 128], ch: char) -> [u8; 8] {
    const V: u8 = 0x04; // center column
    const H: u8 = 0x1F; // full row
    const L: u8 = 0x1C; // row, left of center
    const R: u8 = 0x07; // row, right of center
    match ch {
        c if (c as u32) < 128 => font[c as usize],
        '─' | '╌' | '┄' => [0, 0, 0, H, 0, 0, 0, 0],
        '━' => [0, 0, 0, H, H, 0, 0, 0],
        '│' | '┆' | '┊' => [V; 8],
        '┃' => [0x0C; 8],
        '┌' | '╭' => [0, 0, 0, R, V, V, V, V],
        '┐' | '╮' => [0, 0, 0, L, V, V, V, V],
        '└' | '╰' => [V, V, V, R, 0, 0, 0, 0],
        '┘' | '╯' => [V, V, V, L, 0, 0, 0, 0],
        '├' => [V, V, V, R, V, V, V, V],
        '┤' => [V, V, V, L, V, V, V, V],
        '┬' => [0, 0, 0, H, V, V, V, V],
        '┴' => [V, V, V, H, 0, 0, 0, 0],
        '┼' => [V, V, V, H, V, V, V, V],
        '═' => [0, 0, H, 0, H, 0, 0, 0],
        '║' => [0x0A; 8],
        '╔' => [0, 0, 0x0E, 0x0A, 0x0A, 0x0A, 0x0A, 0x0A],
        '╗' => [0, 0, 0x1A, 0x0A, 0x0A, 0x0A, 0x0A, 0x0A],
        '╚' => [0x0A, 0x0A, 0x0A, 0x0A, 0x0E, 0, 0, 0],
        '╝' => [0x0A, 0x0A, 0x0A, 0x0A, 0x1A, 0, 0, 0],
        '█' => [H; 8],
        '▀' => [H, H, H, H, 0, 0, 0, 0],
        '▄' => [0, 0, 0, 0, H, H, H, H],
        '▌' => [L; 8],
        '▐' => [R; 8],
        '░' => [0x11, 0, 0x04, 0, 0x11, 0, 0x04, 0],
        '▒' => [0x15, 0x0A, 0x15, 0x0A, 0x15, 0x0A, 0x15, 0x0A],
        '▪' | '▫' | '•' | '·' => [0, 0, 0, 0x06, 0x06, 0, 0, 0],
        '❯' | '›' => [0x10, 0x08, 0x04, 0x02, 0x04, 0x08, 0x10, 0],
        '✓' => [0, 0x01, 0x01, 0x0A, 0x0A, 0x04, 0, 0],
        '✗' | '×' => [0x11, 0x0A, 0x04, 0x04, 0x0A, 0x11, 0, 0],
        '…' => [0, 0, 0, 0, 0, 0, 0x15, 0],
        '→' => [0, 0, 0x04, 0x02, H, 0x02, 0x04, 0],
        '←' => [0, 0, 0x04, 0x08, H, 0x08, 0x04, 0],
        '↑' => [0x04, 0x0E, 0x15, 0x04, 0x04, 0x04, 0x04, 0],
        '↓' => [0x04, 0x04, 0x04, 0x04, 0x15, 0x0E, 0x04, 0],
        // Braille patterns: 2x4 dot matrix encoded in the low byte.
        c @ '\u{2800}'..='\u{28FF}' => {
            let b = c as u32 - 0x2800;
            let mut g = [0u8; 8];
            if b & 0x01 != 0 { g[1] |= 0x08; }
            if b & 0x02 != 0 { g[3] |= 0x08; }
            if b & 0x04 != 0 { g[5] |= 0x08; }
            if b & 0x40 != 0 { g[7] |= 0x08; }
            if b & 0x08 != 0 { g[1] |= 0x02; }
            if b & 0x10 != 0 { g[3] |= 0x02; }
            if b & 0x20 != 0 { g[5] |= 0x02; }
            if b & 0x80 != 0 { g[7] |= 0x02; }
            g
        }
        _ => font['?' as usize],
    }
}

// M38a: iterate CHARS, not bytes — a UTF-8 hanzi used to truncate to four
// garbage ASCII cells. Wide chars (CJK etc.) render through the ab_glyph
// path spanning two cells; ASCII keeps the 5x8 bitmap.
fn draw_text(pix: &mut [u32], pitch: usize, w: usize, h: usize, font: &[[u8; 8]; 128], x: i32, y: i32, s: &str, scale: usize, c: u32) -> i32 {
    let mut cx = x;
    for ch in s.chars() {
        if cjk::char_width(ch) == 2 {
            let box_w = 12 * scale;
            let box_h = 8 * scale;
            if !cjk::draw(pix, pitch, w, h, cx, y, box_w, box_h, box_h as f32, ch, c) {
                let g = font['?' as usize];
                for r in 0..8 {
                    for col in 0..5 {
                        if g[r] & (0x10 >> col) != 0 {
                            fill_rect(
                                pix,
                                pitch,
                                w,
                                h,
                                cx + (col * scale) as i32,
                                y + (r * scale) as i32,
                                scale as i32,
                                scale as i32,
                                c,
                            );
                        }
                    }
                }
            }
            cx += (12 * scale) as i32;
            continue;
        }
        if (ch as u32) >= 0x80
            && cjk::draw(pix, pitch, w, h, cx, y, 6 * scale, 8 * scale, (8 * scale) as f32 * 0.8, ch, c)
        {
            // narrow non-ASCII (—, ·, …) from the CJK subset; bitmap is ASCII-only
            cx += (6 * scale) as i32;
            continue;
        }
        let g = glyph(font, ch);
        for r in 0..8 {
            for col in 0..5 {
                if g[r] & (0x10 >> col) != 0 {
                    fill_rect(
                        pix,
                        pitch,
                        w,
                        h,
                        cx + (col * scale) as i32,
                        y + (r * scale) as i32,
                        scale as i32,
                        scale as i32,
                        c,
                    );
                }
            }
        }
        cx += (6 * scale) as i32;
    }
    cx
}

fn text_w(s: &str, scale: usize) -> usize {
    s.chars()
        .map(|ch| if cjk::char_width(ch) == 2 { 12 } else { 6 })
        .sum::<usize>()
        * scale
}

fn draw_centered(pix: &mut [u32], pitch: usize, w: usize, h: usize, font: &[[u8; 8]; 128], y: i32, s: &str, scale: usize, c: u32) {
    let tw = text_w(s, scale) as i32;
    draw_text(pix, pitch, w, h, font, (w as i32 - tw) / 2, y, s, scale, c);
}

// ---------------- pty ----------------

struct Child {
    master: std::fs::File,
    pid: libc::pid_t,
}

/// `agpkg available` — optional packages not yet installed, capped at 12
/// (picker row geometry is unsigned arithmetic; scrolling is later).
fn read_available() -> Vec<String> {
    std::process::Command::new(launch::BIN_AGPKG)
        .arg("available")
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .take(12)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn spawn_shell(cols: u16, rows: u16, argv: &[&str]) -> Result<Child, String> {
    let mut master: libc::c_int = -1;
    let mut slave: libc::c_int = -1;
    let mut ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut ws,
        )
    };
    if rc != 0 {
        return Err(format!("openpty: {}", std::io::Error::last_os_error()));
    }
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err("fork failed".into());
    }
    if pid == 0 {
        unsafe {
            libc::setsid();
            libc::ioctl(slave, libc::TIOCSCTTY as _, 0);
            libc::dup2(slave, 0);
            libc::dup2(slave, 1);
            libc::dup2(slave, 2);
            libc::close(master);
            if slave > 2 {
                libc::close(slave);
            }
            // SIG_IGN survives exec, and aterm's own ancestry carries one:
            // rcS's busybox sh ignores HUP+INT (observed SigIgn 0x1006 on
            // device, 2026-08-31), adbd ignores INT for adb-run instances.
            // Without this reset every terminal job is immune to ^C — the
            // bytes reach the ldisc, kill_pgrp fires, the disposition
            // discards the signal. Rust std's ignored SIGPIPE is also
            // inherited; shells want the default back.
            for sig in [
                libc::SIGHUP,
                libc::SIGINT,
                libc::SIGQUIT,
                libc::SIGTERM,
                libc::SIGTSTP,
                libc::SIGTTIN,
                libc::SIGTTOU,
                libc::SIGPIPE,
            ] {
                libc::signal(sig, libc::SIG_DFL);
            }
            let empty: libc::sigset_t = std::mem::zeroed();
            libc::sigprocmask(libc::SIG_SETMASK, &empty, std::ptr::null_mut());
            libc::setenv(
                b"TERM\0".as_ptr() as *const _,
                b"xterm-256color\0".as_ptr() as *const _,
                1,
            );
            libc::setenv(b"HOME\0".as_ptr() as *const _, b"/home\0".as_ptr() as *const _, 1);
            let prog = std::ffi::CString::new(argv[0]).unwrap();
            let owned: Vec<std::ffi::CString> =
                argv.iter().map(|a| std::ffi::CString::new(*a).unwrap()).collect();
            let mut cargv: Vec<*const libc::c_char> =
                owned.iter().map(|c| c.as_ptr()).collect();
            cargv.push(std::ptr::null());
            libc::execv(prog.as_ptr(), cargv.as_ptr());
            // exec failed — say so on the pty, then die
            let msg = b"aterm: exec failed\r\n";
            libc::write(1, msg.as_ptr() as *const _, msg.len());
            libc::_exit(127);
        }
    }
    unsafe {
        libc::close(slave);
        let flags = libc::fcntl(master, libc::F_GETFL);
        libc::fcntl(master, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
    Ok(Child {
        master: unsafe { std::fs::File::from_raw_fd(master) },
        pid,
    })
}

fn child_exited(pid: libc::pid_t) -> bool {
    let mut status: libc::c_int = 0;
    let r = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
    r == pid
}

/// The one input path: encode a KeyEvent/TextInputEvent for the child's
/// current terminal mode and write it to the pty, pulling the echo into
/// the same render frame (the keystroke fast-path from M11). The on-screen
/// keyboard, hold-repeat and — from M18 — voice ASR all come through
/// here; nothing else writes typed input to the pty.
fn inject(mode: &mut Mode, term: &mut Term, parser: &mut vte::Parser, ev: &InputEvent) {
    let bytes = input::encode(ev, term.app_cursor);
    if std::env::var("ATERM_DEBUG").is_ok() {
        eprintln!("aterm: inject {:?} appcur={} -> {} bytes {:?}", ev, term.app_cursor, bytes.len(), String::from_utf8_lossy(&bytes));
    }
    if bytes.is_empty() {
        return; // modifier toggle — consumed by the keyboard, no output
    }
    if let Mode::Running(c) = mode {
        let _ = c.master.write_all(&bytes);
        let mut pfd = libc::pollfd {
            fd: c.master.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut pfd, 1, 15) } > 0 {
            let mut buf2 = [0u8; 8192];
            if let Ok(n) = std::io::Read::read(&mut c.master, &mut buf2) {
                for &b in &buf2[..n] {
                    parser.advance(term, b);
                }
                term.jump_live();
            }
        }
    }
}

// ---------------- modes ----------------

enum Mode {
    Launcher,
    Running(Child),
    /// Optional-package picker (launcher "+" tile): rows come from
    /// `agpkg available`; a tap runs `agpkg opt-in <name>` synchronously
    /// (INSTALLING frame drawn first) and refreshes both lists.
    Picker,
    /// Photo viewer (launcher PHOTOS tile, M39): list screen of
    /// /home/photos, then a full-frame view with tap-sides paging.
    /// Decode is libjpeg-turbo (no JPEG decode hardware on SM7250).
    Photos(photos::Photos),
}

// ---------------- render ----------------

struct Render<'a> {
    font: &'a [[u8; 8]; 128],
    w: usize,
    h: usize,
    pitch: usize,
}

impl<'a> Render<'a> {
    fn launcher(&self, pix: &mut [u32], entries: &[launch::Entry], g: &launch::Geom) {
        fill_rect(pix, self.pitch, self.w, self.h, 0, 0, self.w as i32, self.h as i32, BG);
        draw_centered(pix, self.pitch, self.w, self.h, self.font, g.toolbar_h as i32 + 14, "AGINXOS", 5, GREEN);
        for (i, e) in entries.iter().enumerate() {
            let y0 = (g.by0 + i * (g.bh + g.gap)) as i32;
            let c = if e.avail { DIM } else { 0x000F1A14 };
            // button outline
            fill_rect(pix, self.pitch, self.w, self.h, g.bx as i32, y0, g.bw as i32, 3, c);
            fill_rect(pix, self.pitch, self.w, self.h, g.bx as i32, y0 + g.bh as i32 - 3, g.bw as i32, 3, c);
            fill_rect(pix, self.pitch, self.w, self.h, g.bx as i32, y0, 3, g.bh as i32, c);
            fill_rect(pix, self.pitch, self.w, self.h, (g.bx + g.bw - 3) as i32, y0, 3, g.bh as i32, c);
            let scale = 5;
            let tw = text_w(e.label.as_str(), scale) as i32;
            let ty = y0 + (g.bh as i32 - 8 * scale as i32) / 2;
            let tc = if e.avail { GREEN } else { UNAVAIL };
            draw_text(pix, self.pitch, self.w, self.h, self.font, g.bx as i32 + (g.bw as i32 - tw) / 2, ty, e.label.as_str(), scale, tc);
            if !e.avail {
                draw_centered(pix, self.pitch, self.w, self.h, self.font, y0 + g.bh as i32 - 30, "(NOT INSTALLED)", 2, UNAVAIL);
            }
        }
        // hint line
        draw_centered(pix, self.pitch, self.w, self.h, self.font, g.kb_panel_y as i32 - 40, "TAP TO START", 3, UNAVAIL);
    }

    /// Optional-package picker ("+" tile): same row geometry as the
    /// launcher. status_line is the last install result ("" = hint).
    /// The caller caps the list at 12 rows — Geom arithmetic is unsigned
    /// and a long list would underflow; scrolling is a later milestone.
    fn picker(&self, pix: &mut [u32], names: &[String], status_line: &str, g: &launch::Geom) {
        fill_rect(pix, self.pitch, self.w, self.h, 0, 0, self.w as i32, self.h as i32, BG);
        self.toolbar(pix, g.m, g.toolbar_h);
        draw_centered(pix, self.pitch, self.w, self.h, self.font, g.toolbar_h as i32 + 14, "SELECT PKGS", 5, GREEN);
        if names.is_empty() {
            draw_centered(pix, self.pitch, self.w, self.h, self.font, (self.h as i32 - 24) / 2, "(NONE AVAILABLE)", 3, UNAVAIL);
        }
        for (i, n) in names.iter().enumerate() {
            let y0 = (g.by0 + i * (g.bh + g.gap)) as i32;
            let c = DIM;
            fill_rect(pix, self.pitch, self.w, self.h, g.bx as i32, y0, g.bw as i32, 3, c);
            fill_rect(pix, self.pitch, self.w, self.h, g.bx as i32, y0 + g.bh as i32 - 3, g.bw as i32, 3, c);
            fill_rect(pix, self.pitch, self.w, self.h, g.bx as i32, y0, 3, g.bh as i32, c);
            fill_rect(pix, self.pitch, self.w, self.h, (g.bx + g.bw - 3) as i32, y0, 3, g.bh as i32, c);
            let tw = text_w(n.as_str(), 5) as i32;
            let ty = y0 + (g.bh as i32 - 8 * 5) / 2;
            draw_text(pix, self.pitch, self.w, self.h, self.font, g.bx as i32 + (g.bw as i32 - tw) / 2, ty, n.as_str(), 5, GREEN);
        }
        let line = if status_line.is_empty() { "TAP TO INSTALL" } else { status_line };
        let lc = if status_line.is_empty() { UNAVAIL } else { GREEN };
        draw_centered(pix, self.pitch, self.w, self.h, self.font, g.kb_panel_y as i32 - 40, line, 3, lc);
    }

    /// Full-cover frame shown while `agpkg opt-in` runs (synchronous —
    /// the event loop is blocked, so this must be painted + presented
    /// before the Command).
    fn installing(&self, pix: &mut [u32], name: &str) {
        fill_rect(pix, self.pitch, self.w, self.h, 0, 0, self.w as i32, self.h as i32, BG);
        draw_centered(pix, self.pitch, self.w, self.h, self.font, (self.h as i32 - 8 * 5) / 2 - 60, "INSTALLING", 5, GREEN);
        draw_centered(pix, self.pitch, self.w, self.h, self.font, (self.h as i32 - 8 * 5) / 2 + 60, name, 5, WHITE);
    }

    /// LOADING frame while a JPEG decodes (same synchronous-block pattern
    /// as `installing` — paint, present, then block in libjpeg).
    fn loading(&self, pix: &mut [u32]) {
        fill_rect(pix, self.pitch, self.w, self.h, 0, 0, self.w as i32, self.h as i32, BG);
        draw_centered(pix, self.pitch, self.w, self.h, self.font, (self.h as i32 - 8 * 5) / 2, "LOADING", 5, GREEN);
    }

    /// Photo list screen (M39): picker-style rows of /home/photos
    /// basenames, newest first, capped at 12 rows like the picker (Geom
    /// arithmetic is unsigned; scrolling is a later milestone).
    fn photos_list(&self, pix: &mut [u32], p: &photos::Photos, g: &launch::Geom) {
        fill_rect(pix, self.pitch, self.w, self.h, 0, 0, self.w as i32, self.h as i32, BG);
        self.toolbar(pix, g.m, g.toolbar_h);
        draw_centered(pix, self.pitch, self.w, self.h, self.font, g.toolbar_h as i32 + 14, "PHOTOS", 5, GREEN);
        let names = p.names();
        if names.is_empty() {
            draw_centered(pix, self.pitch, self.w, self.h, self.font, (self.h as i32 - 24) / 2, "(NO PHOTOS)", 3, UNAVAIL);
            draw_centered(pix, self.pitch, self.w, self.h, self.font, (self.h as i32 - 24) / 2 + 60, "AG CAM-SHOT --JPEG-OUT /HOME/PHOTOS/...", 2, UNAVAIL);
        } else {
            for (i, n) in names.iter().take(12).enumerate() {
                let y0 = (g.by0 + i * (g.bh + g.gap)) as i32;
                fill_rect(pix, self.pitch, self.w, self.h, g.bx as i32, y0, g.bw as i32, 3, DIM);
                fill_rect(pix, self.pitch, self.w, self.h, g.bx as i32, y0 + g.bh as i32 - 3, g.bw as i32, 3, DIM);
                fill_rect(pix, self.pitch, self.w, self.h, g.bx as i32, y0, 3, g.bh as i32, DIM);
                fill_rect(pix, self.pitch, self.w, self.h, (g.bx + g.bw - 3) as i32, y0, 3, g.bh as i32, DIM);
                let n: String = n.chars().take(24).collect();
                let tw = text_w(&n, 5) as i32;
                let ty = y0 + (g.bh as i32 - 8 * 5) / 2;
                draw_text(pix, self.pitch, self.w, self.h, self.font, g.bx as i32 + (g.bw as i32 - tw) / 2, ty, &n, 5, GREEN);
            }
        }
        let total = names.len();
        let footer = if !p.err.is_empty() {
            p.err.clone()
        } else if total == 0 {
            "TAP BACK TO EXIT".to_string()
        } else if total > 12 {
            format!("{total} PHOTOS - NEWEST 12")
        } else {
            format!("{total} PHOTO{} - TAP TO VIEW", if total == 1 { "" } else { "S" })
        };
        let fc = if !p.err.is_empty() { GREEN } else { UNAVAIL };
        draw_centered(pix, self.pitch, self.w, self.h, self.font, g.kb_panel_y as i32 - 40, &footer, 3, fc);
    }

    /// Full-screen photo view: decoded bitmap blitted 1:1, centered under
    /// the toolbar (decode already DCT-scaled to fit the box), filename in
    /// the footer. Tap the right half for next, left for previous.
    fn photo_view(&self, pix: &mut [u32], p: &photos::Photos, g: &launch::Geom) {
        fill_rect(pix, self.pitch, self.w, self.h, 0, 0, self.w as i32, self.h as i32, BG);
        self.toolbar(pix, g.m, g.toolbar_h);
        if let Some(b) = &p.img {
            let dx = ((self.w - b.w as usize) / 2) as i32;
            let avail_h = self.h - g.toolbar_h as usize;
            let dy = (g.toolbar_h as i32 + ((avail_h - b.h as usize) / 2) as i32).max(g.toolbar_h as i32);
            for j in 0..b.h as usize {
                let py = dy + j as i32;
                if py < 0 || py >= self.h as i32 {
                    continue;
                }
                for i in 0..b.w as usize {
                    let px = dx + i as i32;
                    if px < 0 || px >= self.w as i32 {
                        continue;
                    }
                    pix[py as usize * self.pitch + px as usize] = b.pix[j * b.w as usize + i];
                }
            }
        }
        let name = p.names().get(p.sel).cloned().unwrap_or_default();
        draw_centered(pix, self.pitch, self.w, self.h, self.font, g.kb_panel_y as i32 - 36, &name, 3, WHITE);
        draw_centered(pix, self.pitch, self.w, self.h, self.font, self.h as i32 - 30, "< TAP TO PAGE >", 2, DIM);
    }

    /// Header strip: [BACK] at the right, like the launcher header —
    /// nothing else, so the content below stays uncovered.
    fn toolbar(&self, pix: &mut [u32], m: usize, strip_h: usize) {
        let (w, h) = (self.w, self.h);
        fill_rect(pix, self.pitch, w, h, m as i32, strip_h as i32, (w - 2 * m) as i32, 2, DIM);
        let ty = (strip_h as i32 - 8 * 3) / 2;
        draw_text(pix, self.pitch, w, h, self.font, (w - m) as i32 - text_w("BACK", 3) as i32, ty, "BACK", 3, GREEN);
    }

    /// Row-damaged render: only rows the Term marked dirty are repainted
    /// (bg fill + glyphs + cursor). The full-screen fill is gone — the
    /// canvas in main() persists between frames.
    fn terminal(&self, pix: &mut [u32], t: &Term, area_top: usize, _area_h: usize, scale: usize, blink_on: bool, x_off: usize) {
        let (w, h) = (self.w, self.h);
        let cell_w = 6 * scale;
        let cell_h = 8 * scale;
        let stride = cell_h + ROW_GAP;
        for row in 0..t.rows {
            if !t.row_dirty()[row] {
                continue;
            }
            let y = area_top + row * stride;
            fill_rect(pix, self.pitch, w, h, x_off as i32, y as i32, (w - 2 * x_off) as i32, stride as i32, BG);
            let line = t.render_line(row);
            let mut x = x_off;
            for cell in &line {
                if cell.ch == term::WIDE_TAIL {
                    x += cell_w;
                    continue;
                }
                if cell.ch != ' ' {
                    let mut c = match cell.style {
                        Style::Normal => GREEN,
                        Style::Bright => WHITE,
                        Style::Inverse => GREEN,
                    };
                    if matches!(cell.style, Style::Inverse) {
                        let wcells = if cjk::char_width(cell.ch) == 2 { 2 * cell_w } else { cell_w };
                        fill_rect(pix, self.pitch, w, h, x as i32, y as i32, wcells as i32, cell_h as i32, GREEN);
                        c = BG;
                    }
                    if cjk::char_width(cell.ch) == 2
                        && cjk::draw(pix, self.pitch, w, h, x as i32, y as i32, 2 * cell_w, cell_h, cell_h as f32, cell.ch, c)
                    {
                        // rendered from the CJK subset
                    } else if (cell.ch as u32) >= 0x80
                        && cjk::draw(pix, self.pitch, w, h, x as i32, y as i32, cell_w, cell_h, cell_h as f32 * 0.8, cell.ch, c)
                    {
                        // narrow non-ASCII (—, ·, …, °): width-1 but only the
                        // CJK subset has the glyph — bitmap font is ASCII-only
                    } else {
                        let g = glyph(self.font, cell.ch);
                        for r in 0..8 {
                            for col in 0..5 {
                                if g[r] & (0x10 >> col) != 0 {
                                    fill_rect(
                                        pix,
                                        self.pitch,
                                        w,
                                        h,
                                        (x + col * scale) as i32,
                                        (y + r * scale) as i32,
                                        scale as i32,
                                        scale as i32,
                                        c,
                                    );
                                }
                            }
                        }
                    }
                }
                x += cell_w;
            }
            if row == t.cursor_y && t.cursor_visible && t.view_offset == 0 && blink_on {
                fill_rect(
                    pix,
                    self.pitch,
                    w,
                    h,
                    (x_off + t.cursor_x * cell_w) as i32,
                    (y + cell_h - 2) as i32,
                    cell_w as i32,
                    2,
                    GREEN,
                );
            }
        }
    }

    fn keyboard(&self, pix: &mut [u32], kg: &KeyGeom, kb: &Kb) {
        let (w, h) = (self.w, self.h);
        let m = kg.x_off;
        fill_rect(pix, self.pitch, w, h, m as i32, kg.extra_y as i32, (w - 2 * m) as i32, (h - kg.extra_y) as i32, 0x00050A08);
        fill_rect(pix, self.pitch, w, h, m as i32, kg.extra_y as i32 - 2, (w - 2 * m) as i32, 2, DIM);
        // One gap constant spaces every keycap on the keyboard (2026-09-02):
        // caps are cells inset by gap/2, so H and V seams are all gap wide.
        let gi = kg.gap / 2;
        // extra-keys row (Termux): ESC TAB CTL < v ^ > — labels from the
        // key table, arrows drawn bigger than text labels
        let ekw = (w - 2 * m) / kb::EXTRA_KEYS.len();
        for (i, kd) in kb::EXTRA_KEYS.iter().enumerate() {
            let x0 = m + i * ekw + gi;
            let y0 = kg.extra_y + gi;
            let active = self.mod_active(kd, kb);
            let ks = if i >= 3 { 5 } else { 3 };
            self.keycap(pix, x0, y0, ekw - kg.gap, kg.extra_h - kg.gap, kd.label, ks, active);
        }
        for (r, row) in kb.page_rows().iter().enumerate() {
            for (col, ch) in row.chars().enumerate() {
                let x0 = m + col * kg.cell_w + gi;
                let y0 = kg.panel_y + r * kg.cell_h + gi;
                self.keycap(pix, x0, y0, kg.cell_w - kg.gap, kg.cell_h - kg.gap, &ch.to_string(), kg.label_scale, false);
            }
        }
        let kw = (w - 2 * m) / 5;
        for (i, kd) in kb::SPECIALS.iter().enumerate() {
            let x0 = m + i * kw + gi;
            let y0 = kg.panel_y + 4 * kg.cell_h + gi;
            let active = self.mod_active(kd, kb);
            self.keycap(pix, x0, y0, kw - kg.gap, kg.cell_h - kg.gap, kd.label, 4, active);
        }
    }

    /// Modifier keycaps light up while their one-shot is armed.
    fn mod_active(&self, kd: &KeyDef, kb: &Kb) -> bool {
        match kd.act {
            Act::Ctrl => kb.ctrl_on(),
            Act::Shift => kb.shift_on(),
            Act::Sym => kb.sym_on(),
            _ => false,
        }
    }

    fn keycap(&self, pix: &mut [u32], x0: usize, y0: usize, kw: usize, kh: usize, label: &str, scale: usize, active: bool) {
        let (w, h) = (self.w, self.h);
        let edge = if active { GREEN } else { DIM };
        fill_rect(pix, self.pitch, w, h, x0 as i32, y0 as i32, kw as i32, 2, edge);
        fill_rect(pix, self.pitch, w, h, x0 as i32, (y0 + kh) as i32 - 2, kw as i32, 2, edge);
        fill_rect(pix, self.pitch, w, h, x0 as i32, y0 as i32, 2, kh as i32, edge);
        fill_rect(pix, self.pitch, w, h, (x0 + kw) as i32 - 2, y0 as i32, 2, kh as i32, edge);
        fill_rect(pix, self.pitch, w, h, x0 as i32 + 2, y0 as i32 + 2, kw as i32 - 4, kh as i32 - 4, KEYCAP);
        let ls = scale;
        let tw = text_w(label, ls) as i32;
        let tc = if active { WHITE } else { GREEN };
        draw_text(
            pix,
            self.pitch,
            w,
            h,
            self.font,
            x0 as i32 + (kw as i32 - tw) / 2,
            y0 as i32 + (kh as i32 - 8 * ls as i32) / 2,
            label,
            ls,
            tc,
        );
    }
}

// ---------------- PPM host mode ----------------

fn ppm_dump(path: &str, pix: &[u32], w: usize, h: usize, pitch: usize) -> std::io::Result<()> {
    let mut f = std::fs::File::create(path)?;
    write!(f, "P6\n{} {}\n255\n", w, h)?;
    let mut row = Vec::with_capacity(w * 3);
    for y in 0..h {
        row.clear();
        for x in 0..w {
            let p = pix[y * pitch + x];
            row.push(((p >> 16) & 0xff) as u8);
            row.push(((p >> 8) & 0xff) as u8);
            row.push((p & 0xff) as u8);
        }
        f.write_all(&row)?;
    }
    Ok(())
}

fn kb0() -> Kb {
    Kb::new()
}

/// M15 shutdown: draw a farewell frame, show it, then hand the machine to
/// `reboot2 poweroff` (sync + reboot(RB_POWER_OFF) — the PMIC cuts power).
/// Never returns.
fn power_off(d: &mut Drm, font: &[[u8; 8]; 128], canvas: &mut [u32], blanked: bool) {
    let (w, h, pitch) = (d.width as usize, d.height as usize, d.pitch_px());
    fill_rect(canvas, pitch, w, h, 0, 0, w as i32, h as i32, BG);
    draw_centered(canvas, pitch, w, h, font, (h as i32 - 8 * 5) / 2, "POWERING OFF", 5, GREEN);
    d.back_buf().copy_from_slice(canvas);
    if blanked {
        d.dpms(true); // relatch the farewell frame even if we were blanked
    } else {
        d.present();
    }
    let _ = std::process::Command::new(launch::BIN_REBOOT2).arg("poweroff").spawn();
    std::process::exit(0);
}

fn host_ppm(out: &str) {
    let font = font::font_init();
    let (w, h) = (1080usize, 2340usize);
    let pitch = w;
    let mut pix = vec![0u32; pitch * h];
    let kg = Kb::geom(w, h);
    let entries = launch::entries();
    let lg = launch::Geom::new(w, h, kg.extra_y, entries.len());
    let r = Render { font: &font, w, h, pitch };
    r.launcher(&mut pix, &entries, &lg);
    r.keyboard(&mut pix, &kg, &kb0());

    // second frame: terminal view with a fake session (M38a: includes a
    // UTF-8 Chinese line so the wide-cell put + ab_glyph render path is
    // exercised on the host — ATERM_CJK_FONT points at the subset)
    let area_top0 = lg.toolbar_h + 20;
    let area_h0 = kg.extra_y - area_top0;
    let sc0 = 6usize;
    let mut t = Term::new((w - 2 * kb::KB_M) / (6 * sc0), area_h0 / (8 * sc0));
    let mut parser = vte::Parser::new();
    let demo_owned = std::env::var("ATERM_PPM_DEMO").unwrap_or_else(|_| {
        "root@aginxos:~# uname -a\r\nLinux aginxos 5.4.61-android13 aarch64\r\nroot@aginxos:~# \x1b[1mecho '你好，世界'\x1b[0m\r\n你好，世界 — 化身·互联·记忆在线\r\nroot@aginxos:~# ".to_string()
    });
    let demo: &[u8] = demo_owned.as_bytes();
    for &b in demo {
        parser.advance(&mut t, b);
    }
    let mut pix2 = vec![0u32; pitch * h];
    fill_rect(&mut pix2, pitch, w, h, 0, 0, w as i32, h as i32, BG);
    r.toolbar(&mut pix2, kb::KB_M, lg.toolbar_h);
    r.terminal(&mut pix2, &t, area_top0, area_h0, sc0, true, kb::KB_M);
    r.keyboard(&mut pix2, &kg, &kb0());
    let term_path = format!("{}-term", out);
    if let Err(e) = ppm_dump(out, &pix, w, h, pitch) {
        eprintln!("ppm: {e}");
    }
    if let Err(e) = ppm_dump(&term_path, &pix2, w, h, pitch) {
        eprintln!("ppm: {e}");
    }

    // third frame (M39): photo view — ATERM_PHOTOS_DEMO=<file.jpg> decodes
    // through agimg (DCT-scaled to the panel box) and renders the real
    // viewer screen, so the decode+blit path is host-verifiable.
    if let Ok(demo) = std::env::var("ATERM_PHOTOS_DEMO") {
        let bytes = std::fs::read(&demo).unwrap_or_default();
        let mut p = photos::Photos {
            files: vec![demo.clone()],
            sel: 0,
            img: agimg::decode_scaled(&bytes, w as u32, (h - lg.toolbar_h) as u32),
            view: true,
            err: String::new(),
        };
        if p.img.is_none() {
            p.err = "DECODE FAILED".into();
            p.view = false;
        }
        let mut pix3 = vec![0u32; pitch * h];
        if p.view {
            r.photo_view(&mut pix3, &p, &lg);
        } else {
            r.photos_list(&mut pix3, &p, &lg);
        }
        let photo_path = format!("{}-photo", out);
        if let Err(e) = ppm_dump(&photo_path, &pix3, w, h, pitch) {
            eprintln!("ppm: {e}");
        }
        println!("wrote {photo_path}");
    }
    println!("wrote {out} and {term_path}");
}

// ---------------- main ----------------

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "--ppm" {
        host_ppm(args.get(2).map(|s| s.as_str()).unwrap_or("/tmp/aterm.ppm"));
        return;
    }

    let font = font::font_init();
    let mut d = match Drm::wait_up() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("aterm: {e}");
            std::process::exit(1);
        }
    };
    let (w, h) = (d.width as usize, d.height as usize);
    let pitch = d.pitch_px();

    let mut kb = Kb::new();
    let kg = Kb::geom(w, h);
    let mut entries = launch::entries();
    // Picker state ("+" tile): optional packages from `agpkg available`
    // and the last install result line.
    let mut pkgs: Vec<String> = Vec::new();
    let mut pk_status = String::new();
    let lg = launch::Geom::new(w, h, kg.extra_y, entries.len());

    // Terminal geometry: glyph scale is per-app — sh keeps 5 (30x40 px
    // cells, 34 cols inside the 28 px side margins), the PC-designed TUIs
    // (codex/grok) get 3 (18x24 px, ~56 cols) so their 80-col layouts fit.
    let mut scale = 5usize;
    let area_top = lg.toolbar_h + 20;
    // Keyboard starts hidden; a tap in the terminal area summons/dismisses
    // it and the terminal rows grow/shrink to match (child gets SIGWINCH).
    let area_bottom = |vis: bool| if vis { kg.extra_y } else { h - 24 };
    let rows_for = |vis: bool, sc: usize| ((area_bottom(vis) - area_top) / (8 * sc + ROW_GAP)).max(4);
    let cols_for = |sc: usize| ((w - 2 * kb::KB_M) / (6 * sc)).max(20);
    let mut term_cols = cols_for(scale);
    let mut kb_visible = false;

    let mut term = Term::new(term_cols, rows_for(kb_visible, scale));
    let mut parser = vte::Parser::new();
    let mut mode = Mode::Launcher;
    // Debug/headless path: ATERM_START=<bin> skips the launcher and spawns
    // the program immediately (e.g. ATERM_START=/bin/sh).
    if let Ok(prog) = std::env::var("ATERM_START") {
        // leak: aterm is a forever-process
        let prog: &'static str = Box::leak(prog.into_boxed_str());
        scale = launch::scale_for(prog);
        term_cols = cols_for(scale);
        term = Term::new(term_cols, rows_for(kb_visible, scale));
        match spawn_shell(term_cols as u16, rows_for(kb_visible, scale) as u16, &[prog]) {
            Ok(c) => mode = Mode::Running(c),
            Err(e) => eprintln!("aterm: ATERM_START spawn: {e}"),
        }
    } else if !std::path::Path::new("/etc/wifi.conf").exists()
        && std::path::Path::new(launch::BIN_WIZARD).is_file()
    {
        // First boot / wiped userdata: no network credentials yet, so the
        // wizard is the setup UI (SYSTEM.md §9.2) instead of the launcher.
        scale = launch::scale_for(launch::BIN_WIZARD);
        term_cols = cols_for(scale);
        term = Term::new(term_cols, rows_for(kb_visible, scale));
        match spawn_shell(
            term_cols as u16,
            rows_for(kb_visible, scale) as u16,
            &[launch::BIN_WIZARD],
        ) {
            Ok(c) => mode = Mode::Running(c),
            Err(e) => eprintln!("aterm: wizard spawn: {e}"),
        }
    }
    let mut touch = TouchReader::open("/dev/input/event2", w as i32, h as i32);
    // M15: qpnp_pon keys (power + volume-down) on event1 — hardcoded like
    // the touch node, per HARDWARE.md.
    let mut pwr = KeyReader::open("/dev/input/event1");
    // M15 blank state
    let mut blanked = false;
    let mut last_input = Instant::now();
    let mut power_down: Option<Instant> = None;

    // Persistent canvas: renderers repaint only damaged rows into it, and
    // each present() memcpy's it into the back buffer (~10 MB, ~1 ms) so
    // double-buffer semantics survive partial redraws.
    let mut canvas = vec![0u32; pitch * h];
    // First frame BEFORE the mode set (panel snapshots at SETCRTC).
    {
        let r = Render { font: &font, w, h, pitch };
        let buf = &mut canvas[..];
        match &mode {
            Mode::Launcher => r.launcher(buf, &entries, &lg),
            Mode::Picker => r.picker(buf, &pkgs, &pk_status, &lg),
            Mode::Photos(p) => {
                if p.view {
                    r.photo_view(buf, p, &lg);
                } else {
                    r.photos_list(buf, p, &lg);
                }
            }
            Mode::Running(_) => {
                fill_rect(buf, pitch, w, h, 0, 0, w as i32, h as i32, BG);
                r.toolbar(buf, lg.m, lg.toolbar_h);
                r.terminal(buf, &term, area_top, area_bottom(kb_visible) - area_top, scale, true, lg.m);
            }
        }
        if kb_visible {
            r.keyboard(buf, &kg, &kb);
        }
        d.back_buf().copy_from_slice(&canvas);
    }
    if let Err(e) = d.initial_modeset() {
        eprintln!("aterm: modeset: {e}");
        std::process::exit(1);
    }

    let mut last_blink = Instant::now();
    let mut blink_on = false;
    let mut kb_dirty = true;
    // Hold-to-repeat (DEL / arrows), Termux-style: the event + next fire
    // deadline. Repeats go through inject() like every other input.
    let mut held: Option<(InputEvent, Instant)> = None;
    let mut down_y = 0usize; // where the current touch started
    // M17 debug/voice hook: ATERM_INJECT=1 watches /run/aterm.inject —
    // any process can drop text there and it types into the running
    // session as TextInputEvent, verbatim (\r included if written). This
    // is the exact path M18's ASR callback takes, testable without audio.
    let inject_file = std::env::var("ATERM_INJECT").ok().as_deref() == Some("1");

    loop {
        // drain pty output
        let mut redraw = false;
        if let Mode::Running(child) = &mut mode {
            let mut buf = [0u8; 8192];
            loop {
                match std::io::Read::read(&mut child.master, &mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        for &b in &buf[..n] {
                            parser.advance(&mut term, b);
                        }
                        term.jump_live(); // new output jumps to live
                        redraw = true;
                        // active output keeps the screen awake
                        last_input = Instant::now();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => break,
                }
            }
            if child_exited(child.pid) {
                mode = Mode::Launcher;
                entries = launch::entries();
                kb_visible = false;
                scale = 5;
                term_cols = cols_for(scale);
                term = Term::new(term_cols, rows_for(false, scale));
                parser = vte::Parser::new();
                redraw = true;
            }
        }

        // input (touch / power key / pty)
        let mut fds = [libc::pollfd { fd: -1, events: libc::POLLIN, revents: 0 }; 3];
        let mut nfds = 0usize;
        if let Some(t) = touch.as_ref() {
            fds[nfds].fd = t.raw_fd();
            nfds += 1;
        }
        if let Some(p) = pwr.as_ref() {
            fds[nfds].fd = p.raw_fd();
            nfds += 1;
        }
        if let Mode::Running(c) = &mode {
            fds[nfds].fd = c.master.as_raw_fd();
            nfds += 1;
        }
        let timeout: libc::c_int = if redraw {
            0
        } else if held.is_some() || power_down.is_some() {
            30
        } else {
            400
        };
        let nready = unsafe { libc::poll(fds.as_mut_ptr(), nfds as libc::nfds_t, timeout) };
        if nready > 0 {
            let mut i = 0;
            if touch.is_some() {
                if fds[i].revents & libc::POLLIN != 0 {
                    let ev = touch.as_mut().unwrap().poll();
                    last_input = Instant::now();
                    if blanked {
                        // Any touch wakes the screen; the waking gesture
                        // itself is swallowed so it doesn't also type or
                        // scroll.
                        blanked = false;
                        d.dpms(true);
                        redraw = true;
                    } else {
                    match ev {
                        // Keys fire on finger-DOWN. Waiting for finger-up
                        // added the whole rest-of-finger time to every
                        // keystroke — the main source of "typing lag".
                        Touch::Down(x, y) => {
                            down_y = y;
                            if std::env::var("ATERM_DEBUG").is_ok() {
                                eprintln!("aterm: touch down {x},{y} kbvis={kb_visible} mode={}", matches!(mode, Mode::Running(_)));
                            }
                            if y < lg.toolbar_h {
                                // BACK fires on press, same as keys
                                if lg.toolbar_hit(x, y, matches!(mode, Mode::Running(_) | Mode::Picker | Mode::Photos(_)))
                                    == Some(launch::Toolbar::Back)
                                {
                                    if let Mode::Running(c) = &mode {
                                        unsafe { libc::kill(c.pid, libc::SIGHUP) };
                                    } else if matches!(mode, Mode::Picker) {
                                        mode = Mode::Launcher;
                                    } else if let Mode::Photos(p) = &mut mode {
                                        // view -> list -> launcher, one BACK each
                                        if p.view {
                                            p.view = false;
                                            p.img = None; // free the ~3 MB
                                        } else {
                                            mode = Mode::Launcher;
                                        }
                                    }
                                    redraw = true;
                                }
                            } else if y < kg.extra_y {
                                if let Mode::Launcher = &mut mode {
                                    if let Some(i2) = lg.button_at(x, y, entries.len()) {
                                        if entries[i2].picker {
                                            pkgs = read_available();
                                            pk_status.clear();
                                            mode = Mode::Picker;
                                            redraw = true;
                                        } else if entries[i2].photos {
                                            mode = Mode::Photos(photos::Photos::scan());
                                            redraw = true;
                                        } else if entries[i2].avail {
                                            let prog = entries[i2].bin.as_str();
                                            if prog == launch::BIN_REBOOT2 {
                                                // these draw their own frame
                                                // and never come back — no
                                                // pty round-trip
                                                if entries[i2].args.first().map(String::as_str) == Some("poweroff") {
                                                    power_off(&mut d, &font, &mut canvas, blanked);
                                                }
                                                fill_rect(&mut canvas, pitch, w, h, 0, 0, w as i32, h as i32, BG);
                                                draw_centered(&mut canvas, pitch, w, h, &font, (h as i32 - 8 * 5) / 2, "RESTARTING", 5, GREEN);
                                                d.back_buf().copy_from_slice(&canvas);
                                                d.dpms(true); // relatch the frame (crtc may be off)
                                                let _ = std::process::Command::new(launch::BIN_REBOOT2)
                                                    .arg("reboot")
                                                    .spawn();
                                                std::process::exit(0);
                                            }
                                            // Registry entries carry their
                                            // own scale; PC-designed TUIs
                                            // need ~56 cols to breathe, the
                                            // phone-native UIs keep the big
                                            // touch glyphs.
                                            scale = entries[i2].scale;
                                            term_cols = cols_for(scale);
                                            let argv: Vec<&str> = std::iter::once(prog)
                                                .chain(entries[i2].args.iter().map(String::as_str))
                                                .collect();
                                            match spawn_shell(term_cols as u16, rows_for(false, scale) as u16, &argv) {
                                                Ok(c) => {
                                                    mode = Mode::Running(c);
                                                    kb_visible = false;
                                                    term = Term::new(term_cols, rows_for(false, scale));
                                                    parser = vte::Parser::new();
                                                    kb_dirty = true;
                                                    // wipe launcher pixels below the header —
                                                    // row-damage rendering only repaints
                                                    // terminal rows, so launcher art (the
                                                    // AGINXOS title top sliver) would linger
                                                    fill_rect(&mut canvas, pitch, w, h, 0, lg.toolbar_h as i32, w as i32, (h - lg.toolbar_h) as i32, BG);
                                                }
                                                Err(e) => eprintln!("aterm: spawn: {e}"),
                                            }
                                            redraw = true;
                                        }
                                    }
                                } else if let Mode::Picker = &mut mode {
                                    if let Some(i2) = lg.button_at(x, y, pkgs.len()) {
                                        if let Some(name) = pkgs.get(i2).cloned() {
                                            // synchronous install: paint the
                                            // frame first, the event loop is
                                            // about to block on agdl
                                            {
                                                let r = Render { font: &font, w, h, pitch };
                                                r.installing(&mut canvas[..], &name);
                                                d.back_buf().copy_from_slice(&canvas);
                                                d.present();
                                            }
                                            let out = std::process::Command::new(launch::BIN_AGPKG)
                                                .arg("opt-in")
                                                .arg(&name)
                                                .output();
                                            pk_status = match out {
                                                Ok(o) if o.status.success() => format!("INSTALLED {name}"),
                                                Ok(_) => format!("FAILED {name}"),
                                                Err(e) => format!("FAILED {name}: {e}"),
                                            };
                                            // opt-in seeds /var/apps — the
                                            // registry may have grown; the
                                            // installed name leaves the list
                                            entries = launch::entries();
                                            pkgs = read_available();
                                            redraw = true;
                                        }
                                    }
                                } else if let Mode::Photos(p) = &mut mode {
                                    // decode box: full width, below the BACK strip
                                    let (mw, mh) = (w as u32, (h - lg.toolbar_h) as u32);
                                    let n = p.names().len();
                                    if p.view {
                                        // paint-first, then block in libjpeg
                                        // (the INSTALLING pattern)
                                        {
                                            let r = Render { font: &font, w, h, pitch };
                                            r.loading(&mut canvas[..]);
                                            d.back_buf().copy_from_slice(&canvas);
                                            d.present();
                                        }
                                        p.step(if x >= w / 2 { 1 } else { -1 }, mw, mh);
                                        redraw = true;
                                    } else if let Some(i2) = lg.button_at(x, y, n.min(12)) {
                                        {
                                            let r = Render { font: &font, w, h, pitch };
                                            r.loading(&mut canvas[..]);
                                            d.back_buf().copy_from_slice(&canvas);
                                            d.present();
                                        }
                                        p.open(i2, mw, mh);
                                        redraw = true;
                                    }
                                }
                            }
                            if kb_visible && y >= kg.extra_y {
                                let ev = if y >= kg.panel_y {
                                    kb.key_at(&kg, x, y)
                                } else {
                                    kb.extra_key_at(&kg, x, y)
                                };
                                if let Some(ev) = ev {
                                    inject(&mut mode, &mut term, &mut parser, &ev);
                                    if input::repeatable(&ev) {
                                        held = Some((ev, Instant::now() + Duration::from_millis(400)));
                                    }
                                    kb_dirty = true; // modifier highlight may flip
                                    redraw = true;
                                }
                            }
                        }
                        // Finger lifted: everything fired at Down already.
                        // A tap in the terminal area (no drag) summons or
                        // dismisses the keyboard; rows resize + SIGWINCH.
                        Touch::Tap(_x, y) => {
                            held = None;
                            if std::env::var("ATERM_DEBUG").is_ok() {
                                eprintln!("aterm: touch tap y={y} kbvis={kb_visible}");
                            }
                            let kb_bot = if kb_visible { kg.extra_y } else { h };
                            if let Mode::Running(c) = &mode {
                                if y >= lg.toolbar_h && y < kb_bot {
                                    kb_visible = !kb_visible;
                                    let nr = rows_for(kb_visible, scale);
                                    term.resize_rows(nr);
                                    let ws = libc::winsize {
                                        ws_row: nr as u16,
                                        ws_col: term_cols as u16,
                                        ws_xpixel: 0,
                                        ws_ypixel: 0,
                                    };
                                    unsafe {
                                        libc::ioctl(c.master.as_raw_fd(), libc::TIOCSWINSZ as _, &ws);
                                    }
                                    // layout changed — wipe everything below
                                    // the header and repaint from scratch
                                    fill_rect(&mut canvas, pitch, w, h, 0, lg.toolbar_h as i32, w as i32, (h - lg.toolbar_h) as i32, BG);
                                    kb_dirty = true;
                                    redraw = true;
                                }
                            }
                        }
                        // Scrollback drag only counts if the touch STARTED
                        // in the terminal area (dragging across keys types
                        // nothing and scrolls nothing).
                        Touch::Up => {
                            held = None;
                        }
                        Touch::Drag(dy) => {
                            held = None; // finger slid off the key
                            let kb_bot = if kb_visible { kg.extra_y } else { h };
                            if down_y < kb_bot {
                                if let Mode::Running(_) = mode {
                                    let lines = dy / (8 * scale) as isize;
                                    if lines != 0 {
                                        term.scroll_view(lines);
                                        redraw = true;
                                    }
                                }
                            }
                        }
                        Touch::None => {}
                    }
                    }
                }
                i += 1;
            }
            if pwr.is_some() {
                if fds[i].revents & libc::POLLIN != 0 {
                    for (code, down) in pwr.as_mut().unwrap().poll() {
                        last_input = Instant::now();
                        if code != KEY_POWER {
                            continue; // volume-down rides the same node
                        }
                        if down {
                            power_down = Some(Instant::now());
                        } else if let Some(t) = power_down.take() {
                            // short press toggles blank; a long press was
                            // already acted on by the hold check below
                            if t.elapsed() < POWER_HOLD {
                                if blanked {
                                    blanked = false;
                                    d.dpms(true);
                                    redraw = true;
                                } else {
                                    blanked = true;
                                    d.dpms(false);
                                }
                            }
                        }
                    }
                }
                i += 1;
            }
            if let Mode::Running(_) = mode {
                if i < nfds && fds[i].revents & (libc::POLLIN | libc::POLLHUP) != 0 {
                    // pty readable — next loop iteration drains it
                    redraw = true;
                }
            }
        }

        // power key held >= POWER_HOLD: shutdown (fires while still down)
        if let Some(t) = power_down {
            if t.elapsed() >= POWER_HOLD {
                power_off(&mut d, &font, &mut canvas, blanked);
            }
        }
        // idle blank
        if !blanked && last_input.elapsed() >= IDLE_BLANK {
            blanked = true;
            d.dpms(false);
        }

        // hold-to-repeat for DEL / arrows
        if let Some((ev, next)) = &mut held {
            if Instant::now() >= *next {
                inject(&mut mode, &mut term, &mut parser, ev);
                *next = Instant::now() + Duration::from_millis(60);
                redraw = true;
            }
        }

        // voice-path hook: file content types into the session, consumed
        if inject_file {
            if let Ok(s) = std::fs::read_to_string("/run/aterm.inject") {
                let _ = std::fs::remove_file("/run/aterm.inject");
                if !s.is_empty() {
                    last_input = Instant::now();
                    inject(&mut mode, &mut term, &mut parser, &InputEvent::Text(s));
                    redraw = true;
                }
            }
        }

        // blink toggle — repaint only the cursor's row
        if last_blink.elapsed() > Duration::from_millis(500) {
            blink_on = !blink_on;
            last_blink = Instant::now();
            if matches!(mode, Mode::Running(_)) && term.view_offset == 0 {
                term.mark_row(term.cursor_y);
                redraw = true;
            }
        }

        // while blanked the framebuffer is not scanned out — skip render
        // and present entirely (pty keeps draining above, output renders
        // at wake)
        if !blanked && (redraw || term.dirty) {
            term.dirty = false;
            let r = Render { font: &font, w, h, pitch };
            let buf = &mut canvas[..];
            match &mode {
                Mode::Launcher => {
                    // launcher() full-covers the canvas
                    r.launcher(buf, &entries, &lg);
                }
                Mode::Picker => {
                    // picker() full-covers the canvas
                    r.picker(buf, &pkgs, &pk_status, &lg);
                }
                Mode::Photos(p) => {
                    // both photo screens full-cover the canvas
                    if p.view {
                        r.photo_view(buf, p, &lg);
                    } else {
                        r.photos_list(buf, p, &lg);
                    }
                }
                Mode::Running(_) => {
                    r.terminal(buf, &term, area_top, area_bottom(kb_visible) - area_top, scale, blink_on, lg.m);
                    if kb_dirty {
                        r.toolbar(buf, lg.m, lg.toolbar_h);
                        if kb_visible {
                            r.keyboard(buf, &kg, &kb);
                        }
                    }
                }
            }
            term.clear_row_dirty();
            kb_dirty = false;
            d.back_buf().copy_from_slice(&canvas);
            let t0 = Instant::now();
            d.present();
            let el = t0.elapsed();
            if el > Duration::from_millis(25) {
                eprintln!("aterm: slow present {}ms", el.as_millis());
            }
        }
    }
}
