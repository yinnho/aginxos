// On-screen ASCII keyboard: the panel's lower half. Input is evdev
// (type-B touch protocol); keys fire on finger-DOWN (a tap only read as
// complete at finger-up was a big chunk of the perceived typing lag).
// Drag-scroll of scrollback is armed only by touches starting ABOVE the
// keyboard, so dragging across keys no longer scrolls.
//
// M17: the keyboard is a key table (inputd shape) — every keycap is a
// KeyDef (label + Act), and hit tests return typed input::InputEvents,
// never raw pty bytes. Text keys (letters, symbols, space) come back as
// TextInputEvent, control keys as KeyEvent; encoding to bytes happens
// once, in the terminal layer (input::encode). Voice (M18) injects
// TextInputEvent through the same path in main.rs.
//
// Extra-keys row (borrowed from Termux's ExtraKeysView): a slim row above
// the letter rows with ESC TAB CTL and arrows; DEL + arrows repeat while
// held (Termux PRIMARY_REPETITIVE_KEYS). Specials row: SHF (one-shot
// shift) SYM (one-shot symbol page) SPC DEL ENT. CTL is a one-shot
// modifier: letter -> KeyEvent::Ctrl (CTL c = 0x03 when encoded).
//
// Layout math (1080x2340): a plain 10-column grid — every row divides the
// usable width evenly (28 px side margins, 24 px bottom margin) with ONE
// uniform keycap gap (≈0.8% of span), so the four letter rows are identical
// and edge-to-edge with the extra-keys and specials rows (2026-09-02: the
// QWERTY stagger and fixed-width cells are gone — all rows have 10 keys,
// so all rows are the same row).

use crate::input::{Dir, InputEvent, KeyEvent};
use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::io::AsRawFd;

const ROWS: [&str; 4] = [
    "1234567890",
    "QWERTYUIOP",
    "ASDFGHJKL?",
    "ZXCVBNM.,-",
];
// SYM page: 30 shell punctuation chars + the letter row stays for mixing.
const SYM_ROWS: [&str; 4] = [
    "!@#$%^&*()",
    "\"';:=+[]\\|",
    "<>_`~{}/-?",
    "ZXCVBNM.,-",
];
const SPEC_Y_ROW: usize = 4; // fifth row: SHF SYM SPC DEL ENT

/// What a keycap does. The letter pages stay char grids (ROWS/SYM_ROWS)
/// because their behavior is combinatorial (page x shift x ctrl), so they
/// bypass the table; the fixed rows spell their actions out here. The
/// vocabulary grows with the table — add a KeyDef, not a match arm.
pub enum Act {
    /// Fixed key: emits this event every tap.
    Ev(InputEvent),
    /// Space — a Text event, composed at hit time (" ".into() is not const).
    Space,
    /// One-shot modifiers — toggle state, compose nothing themselves.
    Shift,
    Sym,
    Ctrl,
}

pub struct KeyDef {
    pub label: &'static str,
    pub act: Act,
}

// Extra-keys row (above the letter rows), Termux default order:
// ESC TAB CTL LEFT DOWN UP RIGHT. Arrows are font glyphs 0x10-0x13.
pub const EXTRA_KEYS: [KeyDef; 7] = [
    KeyDef { label: "ESC", act: Act::Ev(InputEvent::Key(KeyEvent::Esc)) },
    KeyDef { label: "TAB", act: Act::Ev(InputEvent::Key(KeyEvent::Tab)) },
    KeyDef { label: "CTL", act: Act::Ctrl },
    KeyDef { label: "\u{10}", act: Act::Ev(InputEvent::Key(KeyEvent::Arrow(Dir::Left))) },
    KeyDef { label: "\u{11}", act: Act::Ev(InputEvent::Key(KeyEvent::Arrow(Dir::Down))) },
    KeyDef { label: "\u{12}", act: Act::Ev(InputEvent::Key(KeyEvent::Arrow(Dir::Up))) },
    KeyDef { label: "\u{13}", act: Act::Ev(InputEvent::Key(KeyEvent::Arrow(Dir::Right))) },
];

pub const SPECIALS: [KeyDef; 5] = [
    KeyDef { label: "SHF", act: Act::Shift },
    KeyDef { label: "SYM", act: Act::Sym },
    KeyDef { label: "SPC", act: Act::Space },
    KeyDef { label: "DEL", act: Act::Ev(InputEvent::Key(KeyEvent::Backspace)) },
    KeyDef { label: "ENT", act: Act::Ev(InputEvent::Key(KeyEvent::Enter)) },
];

pub const KB_M: usize = 28; // side margin (px)
pub const KB_B: usize = 24; // bottom margin (px)
// Row height cap: keeps the terminal area identical to the pre-grid layout
// on redfin — the h/2 height budget would allow 229, far taller than a
// 10-column grid needs now that the width no longer shrinks the cells.
const KB_ROW_H: usize = 118;

pub struct Kb {
    shift: bool,
    sym: bool,
    ctrl: bool,
}

pub struct KeyGeom {
    pub panel_y: usize, // letter panel top edge (px)
    pub extra_y: usize, // extra-keys row top edge (px)
    pub extra_h: usize,
    pub x_off: usize, // side margin
    pub gap: usize, // uniform keycap gap, H+V (≈0.8% of span)
    pub label_scale: usize, // letter labels: ~half the cap, not edge-to-edge
    pub span: usize, // usable width inside the margins
    pub cell_w: usize,
    pub cell_h: usize,
}

impl Kb {
    pub fn new() -> Kb {
        Kb { shift: false, sym: false, ctrl: false }
    }

    pub fn geom(w: usize, h: usize) -> KeyGeom {
        // 10-column grid: every row divides the span into equal cells — no
        // stagger, no fixed cell size — so all four letter rows are
        // identical (10 keys each) and flush with the extra-keys and
        // specials rows. One gap constant spaces every keycap, H and V.
        let span = w - 2 * KB_M;
        let cell_w = span / 10;
        let cell_h = (h / 2 / 5).min(KB_ROW_H);
        let gap = span / 128; // ≈0.8% of span: 8 px on the 1080 panel
        // letter labels: ~half the keycap so rows read as separate keys
        let label_scale = ((cell_w - 24) / 6).min((cell_h - 24) / 8).max(2);
        let panel_y = h - KB_B - cell_h * 5;
        let extra_h = cell_h * 3 / 4;
        KeyGeom {
            panel_y,
            extra_y: panel_y - extra_h - 8,
            extra_h,
            x_off: KB_M,
            gap,
            label_scale,
            span,
            cell_w,
            cell_h,
        }
    }



    /// Extra-keys row hit test. y in [extra_y, panel_y).
    pub fn extra_key_at(&mut self, g: &KeyGeom, x: usize, y: usize) -> Option<InputEvent> {
        if y < g.extra_y || y >= g.panel_y || x < g.x_off {
            return None;
        }
        let kw = g.span / 7;
        let k = ((x - g.x_off) / kw).min(6);
        match &EXTRA_KEYS[k].act {
            Act::Ctrl => {
                self.ctrl = !self.ctrl;
                Some(InputEvent::Text(String::new())) // consumed, no output
            }
            Act::Ev(ev) => Some(ev.clone()),
            _ => None,
        }
    }

    pub fn page_rows(&self) -> &'static [&'static str; 4] {
        if self.sym { &SYM_ROWS } else { &ROWS }
    }

    /// Key at panel coords -> input event. Text keys (page/shift/ctrl
    /// compositing) come back as TextInputEvent; specials as KeyEvent.
    /// Consumes one-shot shift/sym. Modifier toggles return empty Text
    /// (consumed, no output).
    pub fn key_at(&mut self, g: &KeyGeom, x: usize, y: usize) -> Option<InputEvent> {
        if y < g.panel_y || x < g.x_off {
            return None;
        }
        let x = x - g.x_off;
        let row = (y - g.panel_y) / g.cell_h;
        if row < 4 {
            let col = x / g.cell_w;
            let s = if self.sym { SYM_ROWS[row] } else { ROWS[row] };
            if col >= s.len() {
                return None;
            }
            let mut ch = s.as_bytes()[col] as char;
            if !self.sym {
                if self.shift {
                    if ch.is_ascii_uppercase() {
                        // layout stores caps; unshifted means lowercase
                    } else {
                        ch = ch.to_ascii_uppercase();
                    }
                } else if ch.is_ascii_uppercase() {
                    ch = ch.to_ascii_lowercase();
                }
                if ch == '?' && !self.shift {
                    ch = '/';
                }
                if self.shift {
                    const DIGIT_SHIFT: &[(char, char)] = &[
                        ('1', '!'), ('2', '@'), ('3', '#'), ('4', '$'), ('5', '%'),
                        ('6', '^'), ('7', '&'), ('8', '*'), ('9', '('), ('0', ')'),
                        ('-', '_'), (',', '<'), ('.', '>'), ('/', '?'),
                    ];
                    for &(d, s2) in DIGIT_SHIFT {
                        if ch == d {
                            ch = s2;
                        }
                    }
                }
            }
            self.shift = false;
            self.sym = false;
            let ctrl = self.ctrl;
            self.ctrl = false;
            if ctrl && ch.is_ascii_alphabetic() {
                return Some(InputEvent::Key(KeyEvent::Ctrl(ch)));
            }
            Some(InputEvent::Text(ch.to_string()))
        } else if row == SPEC_Y_ROW {
            let kw = g.span / 5;
            let k = (x / kw).min(4);
            match &SPECIALS[k].act {
                Act::Shift => {
                    self.shift = !self.shift;
                    Some(InputEvent::Text(String::new()))
                }
                Act::Sym => {
                    self.sym = !self.sym;
                    Some(InputEvent::Text(String::new()))
                }
                Act::Ev(ev) => Some(ev.clone()),
                Act::Space => Some(InputEvent::Text(" ".into())),
                _ => None,
            }
        } else {
            None
        }
    }

    pub fn shift_on(&self) -> bool {
        self.shift
    }

    pub fn sym_on(&self) -> bool {
        self.sym
    }

    pub fn ctrl_on(&self) -> bool {
        self.ctrl
    }
}

// ---------------- evdev reader ----------------

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct input_event {
    sec: i64,
    usec: i64,
    type_: u16,
    code: u16,
    value: i32,
}

const EV_SYN: u16 = 0;
const EV_ABS: u16 = 3;
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;

pub enum Touch {
    Down(usize, usize), // finger landed (keys fire here, not at lift)
    Tap(usize, usize),  // finger lifted without a drag
    Up,                 // finger lifted after a drag (still ends any hold)
    Drag(isize),        // signed pixel delta (positive = finger moved down)
    None,
}

pub struct TouchReader {
    fd: std::fs::File,
    sx: f32,
    sy: f32,
    raw_x: i32,
    raw_y: i32,
    down: bool,
    pending_down: bool,
    /// The y of THIS touch hasn't been seen yet (tracking-id came first):
    /// the first ABS_MT_POSITION_Y anchors start_y, instead of inheriting
    /// the previous touch's position and instantly reading as a 30 px
    /// "drag". Keeps both event orders working — firmware that reports
    /// positions before tracking-id and synthetic frames after it.
    fresh: bool,
    /// A POSITION_Y was already read in the current frame (before its
    /// tracking-id — real firmware order). Lets the tracking-id handler
    /// tell "position seen" (anchor now) from "position pending" (anchor
    /// at the next y, i.e. synthetic-frame order).
    y_in_frame: bool,
    start_y: i32,
    last_y: i32,
    dragged: bool,
    screen_w: i32,
    screen_h: i32,
}

impl TouchReader {
    pub fn open(path: &str, screen_w: i32, screen_h: i32) -> Option<TouchReader> {
        let fd = OpenOptions::new().read(true).open(path).ok()?;
        // Kernel 4.19 reports the panel-native ranges for both axes; scale
        // to actual fb size.
        Some(TouchReader {
            fd,
            sx: screen_w as f32 / 1080.0,
            sy: screen_h as f32 / 2340.0,
            raw_x: 0,
            raw_y: 0,
            down: false,
            pending_down: false,
            fresh: false,
            y_in_frame: false,
            start_y: 0,
            last_y: 0,
            dragged: false,
            screen_w,
            screen_h,
        })
    }

    pub fn raw_fd(&self) -> i32 {
        self.fd.as_raw_fd()
    }

    pub fn poll(&mut self) -> Touch {
        let mut buf = [0u8; 24 * 8];
        let n = match self.fd.read(&mut buf) {
            Ok(n) => n,
            Err(_) => return Touch::None,
        };
        let mut out = Touch::None;
        for chunk in buf[..n].chunks_exact(24) {
            let ev = unsafe { std::ptr::read_unaligned(chunk.as_ptr() as *const input_event) };
            match (ev.type_, ev.code) {
                (EV_ABS, ABS_MT_SLOT) => {
                    if ev.value != 0 {
                        return Touch::None; // multi-touch: bail on this frame
                    }
                }
                (EV_ABS, ABS_MT_TRACKING_ID) => {
                    if ev.value == -1 {
                        if std::env::var("ATERM_DEBUG").is_ok() {
                            eprintln!("aterm: lift: down={} dragged={}", self.down, self.dragged);
                        }
                        // finger up — ALWAYS reported (hold-repeat cleanup
                        // depends on it, even when the gesture was a drag)
                        self.pending_down = false;
                        if self.down && !self.dragged {
                            let x = (self.raw_x as f32 * self.sx) as usize;
                            let y = (self.raw_y as f32 * self.sy) as usize;
                            out = Touch::Tap(
                                x.min(self.screen_w as usize - 1),
                                y.min(self.screen_h as usize - 1),
                            );
                        } else if self.down {
                            out = Touch::Up;
                        }
                        self.down = false;
                        self.dragged = false;
                    } else {
                        self.down = true;
                        self.dragged = false;
                        self.pending_down = true;
                        // Anchor start_y for the drag threshold. Real
                        // firmware reports position before tracking-id, so
                        // raw_y is this touch's; synthetic frames put
                        // tracking-id first and raw_y is the PREVIOUS
                        // touch's position — anchoring from it would read
                        // as an instant 30 px drag and kill the tap.
                        if self.y_in_frame {
                            self.start_y = self.raw_y;
                            self.last_y = self.raw_y;
                            self.fresh = false;
                        } else {
                            self.fresh = true; // first y anchors
                        }
                    }
                }
                (EV_ABS, ABS_MT_POSITION_X) => self.raw_x = ev.value,
                (EV_ABS, ABS_MT_POSITION_Y) => {
                    self.raw_y = ev.value;
                    self.y_in_frame = true;
                    if self.fresh {
                        // first y of this touch: anchor, no drag judgment
                        self.fresh = false;
                        self.start_y = ev.value;
                        self.last_y = ev.value;
                    } else if self.down {
                        let dy = self.raw_y - self.last_y;
                        if (self.raw_y - self.start_y).abs() > 30 {
                            self.dragged = true;
                        }
                        if self.dragged && dy != 0 {
                            out = Touch::Drag((dy as f32 * self.sy) as isize);
                            self.last_y = self.raw_y;
                        }
                    }
                }
                (EV_SYN, _) => {
                    // coords for this frame are settled: report the press
                    if self.pending_down && self.down {
                        self.pending_down = false;
                        let x = (self.raw_x as f32 * self.sx) as usize;
                        let y = (self.raw_y as f32 * self.sy) as usize;
                        out = Touch::Down(
                            x.min(self.screen_w as usize - 1),
                            y.min(self.screen_h as usize - 1),
                        );
                    }
                    self.y_in_frame = false; // frame boundary
                }
                _ => {}
            }
        }
        out
    }
}

// ---------------- key events (power / volume) ----------------

const EV_KEY: u16 = 1;
pub const KEY_POWER: u16 = 116;

/// Non-touch key events from one evdev node. qpnp_pon (/dev/input/event1)
/// carries power + volume-down on redfin; we only act on KEY_POWER, whose
/// presence in the node's KEY bitmap was confirmed via /proc/bus/input
/// (2026-08-31). qpnp_pon has no EV_REP, so every event is a clean
/// press (1) or release (0) — value 2 autorepeat never appears.
pub struct KeyReader {
    fd: std::fs::File,
}

impl KeyReader {
    pub fn open(path: &str) -> Option<KeyReader> {
        Some(KeyReader {
            fd: OpenOptions::new().read(true).open(path).ok()?,
        })
    }

    pub fn raw_fd(&self) -> i32 {
        self.fd.as_raw_fd()
    }

    pub fn poll(&mut self) -> Vec<(u16, bool)> {
        let mut buf = [0u8; 24 * 8];
        let n = match self.fd.read(&mut buf) {
            Ok(n) => n,
            Err(_) => return Vec::new(),
        };
        let mut out = Vec::new();
        for chunk in buf[..n].chunks_exact(24) {
            let ev = unsafe { std::ptr::read_unaligned(chunk.as_ptr() as *const input_event) };
            if ev.type_ == EV_KEY && (ev.value == 0 || ev.value == 1) {
                out.push((ev.code, ev.value == 1));
            }
        }
        out
    }
}
