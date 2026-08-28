//! 8x16 bitmap font for framebuffer text rendering
//!
//! Each character is 8 pixels wide, 16 pixels tall, stored as 16 bytes.
//! Based on the classic PC BIOS font.

/// 8x16 bitmap font data for printable ASCII (0x20-0x7E)
/// Each entry is 16 bytes, one byte per scanline, MSB = leftmost pixel
pub static FONT: &[u8] = include_bytes!("font8x16.bin");

pub const CHAR_W: usize = 8;
pub const CHAR_H: usize = 16;

/// Get the bitmap data for a character
pub fn char_bitmap(c: u8) -> &'static [u8] {
    if c < 0x20 || c > 0x7E {
        // Return space for non-printable characters
        return &FONT[0 * CHAR_H..(0 + 1) * CHAR_H];
    }
    let idx = (c - 0x20) as usize;
    &FONT[idx * CHAR_H..(idx + 1) * CHAR_H]
}
