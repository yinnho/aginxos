//! Simple framebuffer driver for Pixel 5 (1080x2340, 32bpp)

pub const WIDTH: usize = 1080;
pub const HEIGHT: usize = 2340;
pub const BPP: usize = 4; // bytes per pixel (BGRA)
pub const STRIDE: usize = WIDTH * BPP;
pub const SCALE: usize = 12; // 12x font scaling → 96×96 per char

/// 8×8 VGA font, printable ASCII 32–126 (95 chars × 8 bytes)
const FONT: [u8; 760] = *include_bytes!("font8x8.bin");

/// RGB to 32bpp pixel value (BGRA in memory)
const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    ((b as u32) << 16) | ((g as u32) << 8) | (r as u32)
}

pub const WHITE: u32 = rgb(0xFF, 0xFF, 0xFF);
pub const GREEN: u32 = rgb(0x00, 0xFF, 0x00);
pub const BLACK: u32 = 0;

pub struct Framebuffer {
    base: *mut u32,
}

impl Framebuffer {
    pub fn new(phys_addr: u64) -> Self {
        Self {
            base: phys_addr as *mut u32,
        }
    }

    pub fn clear(&self, color: u32) {
        for y in 0..HEIGHT {
            let row = unsafe { core::slice::from_raw_parts_mut(self.base.add(y * STRIDE / 4), WIDTH) };
            for x in 0..WIDTH {
                row[x] = color;
            }
        }
    }

    pub fn draw_pixel(&self, x: usize, y: usize, color: u32) {
        if x < WIDTH && y < HEIGHT {
            unsafe {
                core::ptr::write_volatile(self.base.add(y * STRIDE / 4 + x), color);
            }
        }
    }

    /// Draw a single character at pixel position (px, py), scaled by SCALE
    pub fn draw_char(&self, px: usize, py: usize, ch: u8, color: u32) {
        self.draw_char_scaled(px, py, ch, color, SCALE);
    }

    /// Draw a single character at pixel position (px, py) with custom scale
    pub fn draw_char_scaled(&self, px: usize, py: usize, ch: u8, color: u32, scale: usize) {
        let idx = ch as usize;
        if idx < 32 || idx > 126 {
            return;
        }
        let glyph = &FONT[(idx - 32) * 8..(idx - 32) * 8 + 8];

        for row in 0..8u8 {
            let bits = glyph[row as usize];
            for col in 0..8 {
                if bits & (0x80 >> col) != 0 {
                    let sx = px + col * scale;
                    let sy = py + row as usize * scale;
                    for dy in 0..scale {
                        for dx in 0..scale {
                            self.draw_pixel(sx + dx, sy + dy, color);
                        }
                    }
                }
            }
        }
    }

    /// Draw text string at pixel position (px, py)
    pub fn draw_str(&self, px: usize, py: usize, text: &str, color: u32) {
        let mut x = px;
        let char_w = 8 * SCALE; // 24px per char at 3x scale
        for ch in text.bytes() {
            self.draw_char(x, py, ch, color);
            x += char_w;
        }
    }

    /// Draw text centered horizontally at row py
    pub fn draw_centered(&self, py: usize, text: &str, color: u32) {
        let char_w = 8 * SCALE;
        let text_w = text.len() * char_w;
        let px = (WIDTH.saturating_sub(text_w)) / 2;
        self.draw_str(px, py, text, color);
    }

    /// Fill rectangle
    pub fn fill_rect(&self, x: usize, y: usize, w: usize, h: usize, color: u32) {
        for dy in 0..h {
            for dx in 0..w {
                self.draw_pixel(x + dx, y + dy, color);
            }
        }
    }

    /// Scroll up by `lines` text rows at `scale` pixels per row
    pub fn scroll_up(&self, lines: usize, scale: usize, margin_top: usize) {
        let bytes_per_pixel = 4;
        let line_h = 8 * scale;
        let scroll_px = lines * line_h;
        let total = WIDTH * (HEIGHT - margin_top) * bytes_per_pixel;
        let src_off = (margin_top + scroll_px) * WIDTH * bytes_per_pixel;
        let dst_off = margin_top * WIDTH * bytes_per_pixel;

        if src_off >= total + dst_off {
            return;
        }

        unsafe {
            let base = self.base as *mut u8;
            core::ptr::copy(
                base.add(src_off),
                base.add(dst_off),
                total - scroll_px * WIDTH * bytes_per_pixel,
            );
            // Clear bottom lines
            let clear_start = HEIGHT - scroll_px;
            for y in clear_start..HEIGHT {
                let row = core::slice::from_raw_parts_mut(
                    self.base.add(y * STRIDE / 4),
                    WIDTH,
                );
                for x in 0..WIDTH {
                    row[x] = BLACK;
                }
            }
        }
    }
}

// ─── Scrolling Console ──────────────────────────────────────────────

pub const CON_SCALE: usize = 4; // console uses 4x → 32×32 per char
pub const CON_COLS: usize = WIDTH / (8 * CON_SCALE); // 33
pub const CON_ROWS: usize = (HEIGHT - 128) / (8 * CON_SCALE); // ~69 (reserve top 128px)
pub const CON_MARGIN_TOP: usize = 128;

pub struct Console {
    pub fb: Framebuffer,
    pub col: usize,
    pub row: usize,
    color: u32,
}

impl Console {
    pub fn new(fb: Framebuffer) -> Self {
        Self {
            fb,
            col: 0,
            row: 0,
            color: WHITE,
        }
    }

    pub fn set_color(&mut self, color: u32) {
        self.color = color;
    }

    /// Move cursor and paint that row green so the next puts stay put.
    pub fn clear_line(&mut self, row: usize) {
        let ch = 8 * CON_SCALE;
        let py = CON_MARGIN_TOP + row * ch;
        self.fb.fill_rect(0, py, WIDTH, ch, GREEN);
        self.col = 0;
        self.row = row;
    }

    fn scroll(&mut self) {
        self.fb.scroll_up(1, CON_SCALE, CON_MARGIN_TOP);
        if self.row > 0 {
            self.row -= 1;
        }
    }

    pub fn putc(&mut self, c: u8) {
        let cw = 8 * CON_SCALE;
        let ch = 8 * CON_SCALE;
        if c == b'\n' {
            self.col = 0;
            self.row += 1;
            if self.row >= CON_ROWS {
                self.scroll();
            }
            return;
        }
        if c == b'\r' {
            self.col = 0;
            return;
        }
        if c == b'\x08' {
            if self.col > 0 {
                self.col -= 1;
                // Erase character
                let px = self.col * cw;
                let py = CON_MARGIN_TOP + self.row * ch;
                self.fb.fill_rect(px, py, cw, ch, BLACK);
            }
            return;
        }
        // Scroll if at end of row
        if self.col >= CON_COLS {
            self.col = 0;
            self.row += 1;
            if self.row >= CON_ROWS {
                self.scroll();
            }
        }
        let px = self.col * cw;
        let py = CON_MARGIN_TOP + self.row * ch;
        self.fb.draw_char_scaled(px, py, c, self.color, CON_SCALE);
        self.col += 1;
    }

    pub fn puts(&mut self, s: &str) {
        for c in s.bytes() {
            if c == b'\n' {
                self.putc(b'\r');
            }
            self.putc(c);
        }
    }

    pub fn flush(&self) {
        let fb_addr = self.fb.base as usize;
        let total_bytes = WIDTH * HEIGHT * BPP;
        unsafe {
            for off in (0..total_bytes).step_by(64) {
                core::arch::asm!("dc civac, {}", in(reg) (fb_addr + off));
            }
            core::arch::asm!("dsb sy");
        }
    }

    /// Write a hex value with label, for debugging
    fn put_hex(&mut self, label: &str, val: u32) {
        self.puts(label);
        self.puts("=0x");
        for i in (0..8).rev() {
            let nibble = ((val >> (i * 4)) & 0xF) as u8;
            self.putc(if nibble < 10 { b'0' + nibble } else { b'a' + nibble - 10 });
        }
        self.puts(" ");
    }

    /// Write a bare 32-bit hex value (no prefix)
    pub fn put_hex32(&mut self, val: u32) {
        for i in (0..8).rev() {
            let nibble = ((val >> (i * 4)) & 0xF) as u8;
            self.putc(if nibble < 10 { b'0' + nibble } else { b'a' + nibble - 10 });
        }
    }

    /// Write a bare 8-bit hex value (no prefix)
    pub fn put_hex8(&mut self, val: u8) {
        let hi = (val >> 4) & 0xF;
        let lo = val & 0xF;
        self.putc(if hi < 10 { b'0' + hi } else { b'a' + hi - 10 });
        self.putc(if lo < 10 { b'0' + lo } else { b'a' + lo - 10 });
    }

    /// Write a bare 16-bit hex value (no prefix)
    pub fn put_hex16(&mut self, val: u16) {
        for i in (0..4).rev() {
            let nibble = ((val >> (i * 4)) & 0xF) as u8;
            self.putc(if nibble < 10 { b'0' + nibble } else { b'a' + nibble - 10 });
        }
    }
}

impl crate::qup_uart::DebugOutput for Console {
    fn debug_str(&mut self, s: &str) {
        self.puts(s);
        self.flush();
    }

    fn debug_hex(&mut self, label: &str, val: u32) {
        self.put_hex(label, val);
        self.flush();
    }
}
