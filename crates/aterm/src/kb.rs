// On-screen ASCII keyboard: the panel's lower half. Input is evdev
// (type-B touch protocol); keys fire on finger-DOWN (a tap only read as
// complete at finger-up was a big chunk of the perceived typing lag).
// Drag-scroll of scrollback is armed only by touches starting ABOVE the
// keyboard, so dragging across keys no longer scrolls.
//
// Extra-keys row (borrowed from Termux's ExtraKeysView): a slim row above
// the letter rows with ESC TAB CTL and arrows; DEL + arrows repeat while
// held (Termux PRIMARY_REPETITIVE_KEYS). Specials row: SHF (one-shot
// shift) SYM (one-shot symbol page) SPC DEL ENT. CTL is a one-shot
// modifier: letter -> control byte (CTL c = 0x03).
//
// Layout math (1080x2340): 10 keys/row x 4 rows, 28 px side margins,
// 24 px bottom margin; keys scale to fit.

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
// Extra-keys row (above the letter rows), Termux default order:
// ESC TAB CTL LEFT DOWN UP RIGHT. Arrows are font glyphs 0x10-0x13.
pub const EXTRA: [&str; 7] = ["ESC", "TAB", "CTL", "\u{10}", "\u{11}", "\u{12}", "\u{13}"];
pub const KB_M: usize = 28; // side margin (px)
pub const KB_B: usize = 24; // bottom margin (px)

/// DEL and the arrows repeat while held (Termux repetitive keys).
pub fn repeatable(bytes: &[u8]) -> bool {
    bytes == [0x7f]
        || (bytes.len() == 3 && bytes[0] == 0x1b && bytes[1] == b'['
            && matches!(bytes[2], b'A'..=b'D'))
}

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
    pub label_scale: usize, // letter labels: ~half the cap, not edge-to-edge
    pub span: usize, // usable width inside the margins
    pub cell_w: usize,
    pub cell_h: usize,
}

impl KeyGeom {
    /// QWERTY stagger: each letter row indents a quarter key further.
    pub fn row_off(&self, row: usize) -> usize {
        self.cell_w * row / 4
    }
}

impl Kb {
    pub fn new() -> Kb {
        Kb { shift: false, sym: false, ctrl: false }
    }

    pub fn geom(w: usize, h: usize) -> KeyGeom {
        // cell height budget: half the screen, 5 rows incl. specials
        let panel_h = h / 2;
        let cell_h = panel_h / 5;
        let mut scale = (cell_h - 6) / 8;
        // width budget: 10 keys/row + QWERTY stagger (up to 3/4 key)
        let wscale = (((w - 2 * KB_M) * 4 / 43).saturating_sub(6)) / 6;
        scale = scale.min(wscale).max(2);
        let cell_w = 6 * scale + 6;
        let cell_h = 8 * scale + 6;
        // letter labels: ~half the keycap so rows read as separate keys
        let label_scale = ((cell_w - 24) / 6).min((cell_h - 24) / 8).max(2);
        let panel_y = h - KB_B - cell_h * 5;
        let extra_h = cell_h * 3 / 4;
        KeyGeom {
            panel_y,
            extra_y: panel_y - extra_h - 8,
            extra_h,
            x_off: KB_M,
            label_scale,
            span: w - 2 * KB_M,
            cell_w,
            cell_h,
        }
    }



    /// Extra-keys row hit test. y in [extra_y, panel_y).
    pub fn extra_key_at(&mut self, g: &KeyGeom, x: usize, y: usize) -> Option<Vec<u8>> {
        if y < g.extra_y || y >= g.panel_y || x < g.x_off {
            return None;
        }
        let kw = g.span / 7;
        let k = ((x - g.x_off) / kw).min(6);
        match k {
            0 => Some(vec![0x1b]),
            1 => Some(b"\t".to_vec()),
            2 => {
                self.ctrl = !self.ctrl;
                Some(vec![])
            }
            3 => Some(b"\x1b[D".to_vec()),
            4 => Some(b"\x1b[B".to_vec()),
            5 => Some(b"\x1b[A".to_vec()),
            _ => Some(b"\x1b[C".to_vec()),
        }
    }

    pub fn page_rows(&self) -> &'static [&'static str; 4] {
        if self.sym { &SYM_ROWS } else { &ROWS }
    }

    /// Key at panel coords -> bytes to write to the pty. Consumes one-shot
    /// shift/sym. Returns Some(vec![]) for the SHF/SYM toggles themselves.
    pub fn key_at(&mut self, g: &KeyGeom, x: usize, y: usize) -> Option<Vec<u8>> {
        if y < g.panel_y || x < g.x_off {
            return None;
        }
        let x = x - g.x_off;
        let row = (y - g.panel_y) / g.cell_h;
        if row < 4 {
            if x < g.row_off(row) {
                return None;
            }
            let x = x - g.row_off(row);
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
                return Some(vec![(ch.to_ascii_lowercase() as u8) & 0x1f]);
            }
            let mut v = vec![0u8; 4];
            let s3 = ch.encode_utf8(&mut v);
            Some(s3.as_bytes().to_vec())
        } else if row == SPEC_Y_ROW {
            let kw = g.span / 5;
            let k = (x / kw).min(4);
            match k {
                0 => {
                    self.shift = !self.shift;
                    Some(vec![])
                }
                1 => {
                    self.sym = !self.sym;
                    Some(vec![])
                }
                2 => Some(b" ".to_vec()),
                3 => Some(vec![0x7f]),
                _ => Some(b"\r".to_vec()),
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

    pub fn specials() -> [&'static str; 5] {
        ["SHF", "SYM", "SPC", "DEL", "ENT"]
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
                        self.start_y = self.raw_y;
                        self.last_y = self.raw_y;
                    }
                }
                (EV_ABS, ABS_MT_POSITION_X) => self.raw_x = ev.value,
                (EV_ABS, ABS_MT_POSITION_Y) => {
                    self.raw_y = ev.value;
                    if self.down {
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
                }
                _ => {}
            }
        }
        out
    }
}
