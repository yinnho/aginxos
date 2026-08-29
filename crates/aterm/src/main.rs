// aterm — AginxOS on-device terminal (M11).
//
// bootcard's DRM path + 5x8 font, a vte-parsed cell grid (black bg, green /
// white text — the fixed phosphor palette), an openpty child (sh / codex /
// grok / aclone), an evdev on-screen keyboard (tap = key, drag = scrollback),
// and a launcher (clone / codex / grok / sh). Started by rcS's aterm-handoff
// once boot finishes: bootcard never exits on its own and holds DRM master
// forever, so the handoff kills it by /run/bootcard.pid and takes the panel.
//
// Host verification: `aterm --ppm out.ppm` renders the launcher into a P6
// PPM without touching DRM (same pattern as bootcard --ppm).

mod drm;
mod font;
mod kb;
mod launch;
mod term;

use drm::Drm;
use kb::{Kb, KeyGeom, Touch, TouchReader};
use std::io::Write as _;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::time::{Duration, Instant};
use term::{Style, Term};

const BG: u32 = 0x00000000;
const GREEN: u32 = 0x0034D399;
const WHITE: u32 = 0x00F5F7FA;
const DIM: u32 = 0x001E3A2E; // key outlines / separators
const KEYCAP: u32 = 0x000A1410; // key fill
const UNAVAIL: u32 = 0x00115A3F; // dimmed green for missing apps

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

fn draw_text(pix: &mut [u32], pitch: usize, w: usize, h: usize, font: &[[u8; 8]; 128], x: i32, y: i32, s: &str, scale: usize, c: u32) -> i32 {
    let mut cx = x;
    for &b in s.as_bytes() {
        let g = font[(b & 127) as usize];
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
    s.len() * 6 * scale
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
            libc::setenv(
                b"TERM\0".as_ptr() as *const _,
                b"xterm-256color\0".as_ptr() as *const _,
                1,
            );
            libc::setenv(b"HOME\0".as_ptr() as *const _, b"/var/home\0".as_ptr() as *const _, 1);
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

// ---------------- modes ----------------

enum Mode {
    Launcher,
    Running(Child),
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
            let tw = text_w(e.label, scale) as i32;
            let ty = y0 + (g.bh as i32 - 8 * scale as i32) / 2;
            let tc = if e.avail { GREEN } else { UNAVAIL };
            draw_text(pix, self.pitch, self.w, self.h, self.font, g.bx as i32 + (g.bw as i32 - tw) / 2, ty, e.label, scale, tc);
            if !e.avail {
                draw_centered(pix, self.pitch, self.w, self.h, self.font, y0 + g.bh as i32 - 30, "(NOT INSTALLED)", 2, UNAVAIL);
            }
        }
        // hint line
        draw_centered(pix, self.pitch, self.w, self.h, self.font, g.kb_panel_y as i32 - 40, "TAP TO START", 3, UNAVAIL);
    }

    fn toolbar(&self, pix: &mut [u32], running: bool) {
        let (w, h) = (self.w, self.h);
        fill_rect(pix, self.pitch, w, h, 0, 44, w as i32, 2, DIM);
        draw_text(pix, self.pitch, w, h, self.font, 12, 10, "SH", 3, GREEN);
        if running {
            draw_text(pix, self.pitch, w, h, self.font, w as i32 - 12 - text_w("BACK", 3) as i32, 10, "BACK", 3, GREEN);
        }
    }

    fn terminal(&self, pix: &mut [u32], t: &Term, area_top: usize, area_h: usize, scale: usize, blink_on: bool) {
        let (w, h) = (self.w, self.h);
        fill_rect(pix, self.pitch, w, h, 0, area_top as i32, w as i32, area_h as i32, BG);
        let cell_w = 6 * scale;
        let cell_h = 8 * scale;
        for row in 0..t.rows {
            let line = t.render_line(row);
            let y = area_top + row * cell_h;
            let mut x = 0usize;
            for cell in &line {
                if cell.ch != ' ' {
                    let c = match cell.style {
                        Style::Normal => GREEN,
                        Style::Bright => WHITE,
                        Style::Inverse => {
                            fill_rect(pix, self.pitch, w, h, x as i32, y as i32, cell_w as i32, cell_h as i32, GREEN);
                            BG
                        }
                    };
                    let g = self.font[(cell.ch as u8 & 127) as usize];
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
                x += cell_w;
            }
        }
        if t.cursor_visible && t.view_offset == 0 && blink_on {
            fill_rect(
                pix,
                self.pitch,
                w,
                h,
                (t.cursor_x * cell_w) as i32,
                (area_top + t.cursor_y * cell_h + cell_h - 2) as i32,
                cell_w as i32,
                2,
                GREEN,
            );
        }
    }

    fn keyboard(&self, pix: &mut [u32], kg: &KeyGeom, kb: &Kb) {
        let (w, h) = (self.w, self.h);
        fill_rect(pix, self.pitch, w, h, 0, kg.panel_y as i32, w as i32, (h - kg.panel_y) as i32, 0x00050A08);
        fill_rect(pix, self.pitch, w, h, 0, kg.panel_y as i32 - 2, w as i32, 2, DIM);
        for (r, row) in kb.page_rows().iter().enumerate() {
            for (col, ch) in row.chars().enumerate() {
                let x0 = col * kg.cell_w + 3;
                let y0 = kg.panel_y + r * kg.cell_h + 3;
                self.keycap(pix, x0, y0, kg.cell_w - 6, kg.cell_h - 6, &ch.to_string(), kg.scale, false);
            }
        }
        let kw = kg.cell_w * 10 / 6;
        for (i, name) in Kb::specials().iter().enumerate() {
            let x0 = i * kw + 3;
            let y0 = kg.panel_y + 4 * kg.cell_h + 3;
            let active = (i == 0 && kb.shift_on()) || (i == 1 && kb.sym_on());
            self.keycap(pix, x0, y0, kw - 6, kg.cell_h - 6, name, kg.scale, active);
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
        let ls = if label.len() > 1 { 2 } else { scale };
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

fn host_ppm(out: &str) {
    let font = font::font_init();
    let (w, h) = (1080usize, 2340usize);
    let pitch = w;
    let mut pix = vec![0u32; pitch * h];
    let kg = Kb::geom(w, h);
    let lg = launch::Geom::new(w, h, kg.panel_y);
    let r = Render { font: &font, w, h, pitch };
    let entries = launch::entries();
    r.launcher(&mut pix, &entries, &lg);
    r.keyboard(&mut pix, &kg, &kb0());

    // second frame: terminal view with a fake session
    let area_top0 = lg.toolbar_h;
    let area_h0 = kg.panel_y - area_top0;
    let sc0 = 2usize;
    let mut t = Term::new(w / (6 * sc0), area_h0 / (8 * sc0));
    let mut parser = vte::Parser::new();
    let demo: &[u8] = b"root@aginxos:~# uname -a\r\nLinux aginxos 5.4.61-android13 aarch64\r\nroot@aginxos:~# \x1b[1mecho $HOME | tr a-z A-Z\x1b[0m\r\n/VAR/HOME\r\nroot@aginxos:~# ";
    for &b in demo {
        parser.advance(&mut t, b);
    }
    let mut pix2 = vec![0u32; pitch * h];
    fill_rect(&mut pix2, pitch, w, h, 0, 0, w as i32, h as i32, BG);
    r.toolbar(&mut pix2, true);
    r.terminal(&mut pix2, &t, area_top0, area_h0, sc0, true);
    r.keyboard(&mut pix2, &kg, &kb0());
    let term_path = format!("{}-term", out);
    if let Err(e) = ppm_dump(out, &pix, w, h, pitch) {
        eprintln!("ppm: {e}");
    }
    if let Err(e) = ppm_dump(&term_path, &pix2, w, h, pitch) {
        eprintln!("ppm: {e}");
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
    let lg = launch::Geom::new(w, h, kg.panel_y);

    // Terminal geometry: glyph scale 2 (12x16 px cells — the readable
    // minimum on a 1080x2340 panel), cols/rows derived from the area.
    let scale = 2usize;
    let area_top = lg.toolbar_h;
    let area_h = kg.panel_y - area_top;
    let term_cols = (w / (6 * scale)).max(20);
    let term_rows = (area_h / (8 * scale)).max(4);

    let mut term = Term::new(term_cols, term_rows);
    let mut parser = vte::Parser::new();
    let mut mode = Mode::Launcher;
    // Debug/headless path: ATERM_START=<bin> skips the launcher and spawns
    // the program immediately (e.g. ATERM_START=/bin/sh).
    if let Ok(prog) = std::env::var("ATERM_START") {
        // leak: aterm is a forever-process
        let prog: &'static str = Box::leak(prog.into_boxed_str());
        match spawn_shell(term_cols as u16, term_rows as u16, &[prog]) {
            Ok(c) => mode = Mode::Running(c),
            Err(e) => eprintln!("aterm: ATERM_START spawn: {e}"),
        }
    }
    let mut touch = TouchReader::open("/dev/input/event2", w as i32, h as i32);
    let mut entries = launch::entries();

    // First frame BEFORE the mode set (panel snapshots at SETCRTC).
    {
        let r = Render { font: &font, w, h, pitch };
        let buf = d.back_buf();
        match &mode {
            Mode::Launcher => r.launcher(buf, &entries, &lg),
            Mode::Running(_) => {
                fill_rect(buf, pitch, w, h, 0, 0, w as i32, h as i32, BG);
                r.toolbar(buf, true);
                r.terminal(buf, &term, area_top, area_h, scale, true);
            }
        }
        r.keyboard(buf, &kg, &kb);
    }
    if let Err(e) = d.initial_modeset() {
        eprintln!("aterm: modeset: {e}");
        std::process::exit(1);
    }

    let mut last_blink = Instant::now();
    let mut blink_on = false;

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
                        if term.view_offset > 0 {
                            term.view_offset = 0; // new output jumps to live
                        }
                        redraw = true;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => break,
                }
            }
            if child_exited(child.pid) {
                mode = Mode::Launcher;
                entries = launch::entries();
                term = Term::new(term_cols, term_rows);
                parser = vte::Parser::new();
                redraw = true;
            }
        }

        // input
        let mut fds = [libc::pollfd { fd: -1, events: libc::POLLIN, revents: 0 }; 2];
        let mut nfds = 0usize;
        if let Some(t) = touch.as_ref() {
            fds[nfds].fd = t.raw_fd();
            nfds += 1;
        }
        if let Mode::Running(c) = &mode {
            fds[nfds].fd = c.master.as_raw_fd();
            nfds += 1;
        }
        let timeout: libc::c_int = if redraw { 0 } else { 400 };
        let nready = unsafe { libc::poll(fds.as_mut_ptr(), nfds as libc::nfds_t, timeout) };
        if nready > 0 {
            let mut i = 0;
            if touch.is_some() {
                if fds[i].revents & libc::POLLIN != 0 {
                    match touch.as_mut().unwrap().poll() {
                        Touch::Tap(x, y) => {
                            eprintln!("aterm: tap x={x} y={y}");
                            if y < lg.toolbar_h {
                                match lg.toolbar_hit(x, y, matches!(mode, Mode::Running(_))) {
                                    Some(launch::Toolbar::Back) => {
                                        if let Mode::Running(c) = &mode {
                                            unsafe { libc::kill(c.pid, libc::SIGHUP) };
                                        }
                                    }
                                    Some(launch::Toolbar::Sh) => {
                                        if let Mode::Running(c) = &mode {
                                            unsafe { libc::kill(c.pid, libc::SIGHUP) };
                                        }
                                        match spawn_shell(term_cols as u16, term_rows as u16, &[launch::BIN_SH]) {
                                            Ok(c) => {
                                                mode = Mode::Running(c);
                                                term = Term::new(term_cols, term_rows);
                                                parser = vte::Parser::new();
                                            }
                                            Err(e) => eprintln!("aterm: spawn sh: {e}"),
                                        }
                                    }
                                    None => {}
                                }
                                redraw = true;
                            } else if y >= kg.panel_y {
                                if let Some(bytes) = kb.key_at(&kg, x, y) {
                                    eprintln!("aterm: key {:02x?}", bytes);
                                    if let Mode::Running(c) = &mut mode {
                                        let _ = c.master.write_all(&bytes);
                                    }
                                    redraw = true;
                                }
                            } else {
                                match &mut mode {
                                    Mode::Launcher => {
                                        if let Some(i2) = lg.button_at(x, y, entries.len()) {
                                            if entries[i2].avail {
                                                let prog = entries[i2].bin;
                                                match spawn_shell(term_cols as u16, term_rows as u16, &[prog]) {
                                                    Ok(c) => {
                                                        mode = Mode::Running(c);
                                                        term = Term::new(term_cols, term_rows);
                                                        parser = vte::Parser::new();
                                                    }
                                                    Err(e) => eprintln!("aterm: spawn: {e}"),
                                                }
                                                redraw = true;
                                            }
                                        }
                                    }
                                    Mode::Running(_) => {}
                                }
                            }
                        }
                        Touch::Drag(dy) => {
                            if let Mode::Running(_) = mode {
                                let lines = dy / (8 * scale) as isize;
                                if lines != 0 {
                                    term.scroll_view(lines); // finger down = view earlier? keep natural: drag down shows newer
                                    redraw = true;
                                }
                            }
                        }
                        Touch::None => {}
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

        // blink toggle
        if last_blink.elapsed() > Duration::from_millis(500) {
            blink_on = !blink_on;
            last_blink = Instant::now();
            if matches!(mode, Mode::Running(_)) {
                redraw = true;
            }
        }

        if redraw || term.dirty {
            term.dirty = false;
            let r = Render { font: &font, w, h, pitch };
            let buf = d.back_buf();
            match &mode {
                Mode::Launcher => {
                    r.launcher(buf, &entries, &lg);
                }
                Mode::Running(_) => {
                    fill_rect(buf, pitch, w, h, 0, 0, w as i32, h as i32, BG);
                    r.toolbar(buf, true);
                    r.terminal(buf, &term, area_top, area_h, scale, blink_on);
                }
            }
            r.keyboard(buf, &kg, &kb);
            d.present();
        }
    }
}
