// On-screen ASCII keyboard: the panel's lower half (touch-drag scrolls
// scrollback, taps hit keys). Input is evdev (type-B touch protocol).
// Specials row: SHF (one-shot shift) SYM (one-shot symbol page) SPC DEL ENT
// ESC. The symbol page replaces the letter rows for one keypress, then
// auto-clears. v1 has no TAB key.
//
// Layout math (1080x2340): 10 keys/row x 4 rows; keys scale to fit.

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
const SPEC_Y_ROW: usize = 4; // fifth row: SHF SYM SPC DEL ENT ESC

pub struct Kb {
    shift: bool,
    sym: bool,
}

pub struct KeyGeom {
    pub panel_y: usize, // panel top edge (px)
    pub scale: usize,   // glyph scale
    pub cell_w: usize,
    pub cell_h: usize,
}

impl Kb {
    pub fn new() -> Kb {
        Kb { shift: false, sym: false }
    }

    pub fn geom(w: usize, h: usize) -> KeyGeom {
        // cell height budget: half the screen, 5 rows incl. specials
        let panel_h = h / 2;
        let cell_h = panel_h / 5;
        let mut scale = (cell_h - 6) / 8;
        // width budget: 10 keys/row, 6*scale + 6 px per key
        let wscale = (w / 10).saturating_sub(6) / 6;
        scale = scale.min(wscale).max(2);
        let cell_w = 6 * scale + 6;
        let cell_h = 8 * scale + 6;
        KeyGeom {
            panel_y: h - cell_h * 5,
            scale,
            cell_w,
            cell_h,
        }
    }

    pub fn page_rows(&self) -> &'static [&'static str; 4] {
        if self.sym { &SYM_ROWS } else { &ROWS }
    }

    /// Key at panel coords -> bytes to write to the pty. Consumes one-shot
    /// shift/sym. Returns Some(vec![]) for the SHF/SYM toggles themselves.
    pub fn key_at(&mut self, g: &KeyGeom, x: usize, y: usize) -> Option<Vec<u8>> {
        if y < g.panel_y {
            return None;
        }
        let row = (y - g.panel_y) / g.cell_h;
        let col = x / g.cell_w;
        if row < 4 {
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
            let mut v = vec![0u8; 4];
            let s3 = ch.encode_utf8(&mut v);
            Some(s3.as_bytes().to_vec())
        } else if row == SPEC_Y_ROW {
            let kw = (g.cell_w * 10) / 6;
            let k = (x / kw).min(5);
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
                4 => Some(b"\r".to_vec()),
                _ => Some(vec![0x1b]),
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

    pub fn specials() -> [&'static str; 6] {
        ["SHF", "SYM", "SPC", "DEL", "ENT", "ESC"]
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
    Tap(usize, usize),
    Drag(isize), // signed pixel delta (positive = finger moved down)
    None,
}

pub struct TouchReader {
    fd: std::fs::File,
    sx: f32,
    sy: f32,
    raw_x: i32,
    raw_y: i32,
    down: bool,
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
        let mut sync = false;
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
                        // finger up
                        if self.down && !self.dragged {
                            let x = (self.raw_x as f32 * self.sx) as usize;
                            let y = (self.raw_y as f32 * self.sy) as usize;
                            out = Touch::Tap(
                                x.min(self.screen_w as usize - 1),
                                y.min(self.screen_h as usize - 1),
                            );
                        }
                        self.down = false;
                        self.dragged = false;
                    } else {
                        self.down = true;
                        self.dragged = false;
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
                (EV_SYN, _) => sync = true,
                _ => {}
            }
        }
        let _ = sync;
        out
    }
}
