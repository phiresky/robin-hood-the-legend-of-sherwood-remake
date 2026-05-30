//! Small color helpers used by both the renderer and sim-facing code
//! that bakes constant colors (e.g. entity outline colors).

/// Pack (r, g, b) into a 16-bit RGB565 word.
#[inline]
pub const fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 & 0xF8) << 8) | ((g as u16 & 0xFC) << 3) | ((b as u16) >> 3)
}
