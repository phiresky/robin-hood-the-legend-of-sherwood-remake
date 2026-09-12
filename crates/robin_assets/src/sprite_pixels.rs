//! Pure sprite decompression and color effects; inputs are validated at asset admission.
use crate::frame_holder::{FrameDictionary, SHADOW_KEY, TRANSPARENT_COLOR_16};

// ---------------------------------------------------------------------------
// Decompression functions
// ---------------------------------------------------------------------------

/// Decompress a run-length encoded sprite, replacing `SHADOW_KEY` markers
/// with the ambient shadow colour (the "ArnoLaw" shadow substitution).
///
/// The packed format per scanline is:
/// - `first` (u16): index of first non-transparent pixel, or 0xFFFF
/// - `size` (u16): index of last non-transparent pixel, or 0xFFFF
/// - Then `(size - first + 1)` pixel values
///
/// `dst` must be at least `width * height` elements, with pitch-based row stride.
pub fn decompress_rle_arno_law(
    src: &[u16],
    dst: &mut [u16],
    pitch_words: usize,
    width: usize,
    height: usize,
    transparent_color: u16,
    shadow_color: u16,
) {
    let mut src_pos = 0;
    let mut dst_pos = 0;

    for _y in 0..height {
        let first = src[src_pos];
        src_pos += 1;
        let size = src[src_pos];
        src_pos += 1;

        if first != 0xFFFF && first > 0 {
            for _ in 0..first {
                dst[dst_pos] = transparent_color;
                dst_pos += 1;
            }
        }

        if size != 0xFFFF {
            let end = size + 1;
            let run = end - first;
            for _ in 0..run {
                let mut color = src[src_pos];
                src_pos += 1;
                if color != SHADOW_KEY {
                    if color == shadow_color {
                        color += 1;
                    }
                } else {
                    color = shadow_color;
                }
                dst[dst_pos] = color;
                dst_pos += 1;
            }
            let remaining = (width as u16).saturating_sub(end);
            for _ in 0..remaining {
                dst[dst_pos] = transparent_color;
                dst_pos += 1;
            }
        } else {
            for _ in 0..width {
                dst[dst_pos] = transparent_color;
                dst_pos += 1;
            }
        }

        dst_pos += pitch_words - width;
    }
}

/// Decompress a vector-quantized sprite using a dictionary.
///
/// Width must be a multiple of 4.
pub fn decompress_vector(
    src: &[u16],
    dst: &mut [u16],
    pitch_words: usize,
    width: usize,
    height: usize,
    dictionary: &FrameDictionary,
) {
    assert!(
        width.is_multiple_of(4),
        "VQ sprite width must be a multiple of 4"
    );

    let mut src_pos = 0;
    let mut dst_pos = 0;

    for _y in 0..height {
        for _x in (0..width).step_by(4) {
            let dict_index = src[src_pos];
            src_pos += 1;

            let pixels = dictionary.lookup_pixels(dict_index);
            dst[dst_pos] = pixels[0];
            dst[dst_pos + 1] = pixels[1];
            dst[dst_pos + 2] = pixels[2];
            dst[dst_pos + 3] = pixels[3];
            dst_pos += 4;
        }
        // Skip pitch padding
        dst_pos += pitch_words - width;
    }
}

// ---------------------------------------------------------------------------
// Per-pixel lookup helpers (used by `FrameHolder::is_pixel_opaque`)
// ---------------------------------------------------------------------------

/// Read a single pixel from RLE-compressed sprite data without full
/// decompression.
///
/// Walks scanlines to reach row `y`, then reads pixel `x` in that row.
/// Returns the raw pixel color, or [`TRANSPARENT_COLOR_16`] if the pixel
/// is in a transparent region or outside the run.
pub(crate) fn rle_pixel_at(src: &[u16], x: usize, y: usize) -> u16 {
    let mut src_pos = 0;

    for row in 0..=y {
        let first = src[src_pos];
        src_pos += 1;
        let size = src[src_pos];
        src_pos += 1;

        if row == y {
            if size == 0xFFFF {
                return TRANSPARENT_COLOR_16;
            }
            let first_usize = first as usize;
            let size_usize = size as usize;
            if x < first_usize || x > size_usize {
                return TRANSPARENT_COLOR_16;
            }
            return src[src_pos + (x - first_usize)];
        }

        // Skip pixel data for this row
        if size != 0xFFFF {
            src_pos += (size - first) as usize + 1;
        }
    }

    TRANSPARENT_COLOR_16
}

/// Read a single pixel from vector-quantized (dictionary) sprite data.
///
/// Each row has `width / 4` dictionary indices, each mapping to 4 pixels.
pub(crate) fn dict_pixel_at(
    src: &[u16],
    width: usize,
    x: usize,
    y: usize,
    dict: &FrameDictionary,
) -> u16 {
    let indices_per_row = width / 4;
    let src_pos = y * indices_per_row + x / 4;
    let dict_index = src[src_pos];
    let pixels = dict.lookup_pixels(dict_index);
    pixels[x % 4]
}

// ---------------------------------------------------------------------------
// Viewport-level color effects
// ---------------------------------------------------------------------------

/// Apply fog/night blending effect to decompressed pixel data.
///
/// Uses 5-bit green-channel handling (mask `0x07C0` instead of full
/// `0x07E0`) — the transparent color constant doubles as the green bit
/// mask, so the LSB of the 6-bit green channel is discarded.
pub fn apply_fog_effect_viewport(data: &mut [u16], level: u16, fog_color: u16, shadow_color: u16) {
    // Extract fog color components using the 5-bit green mask.
    let fog_r = (fog_color & 0xF800) >> 11;
    let fog_g = (fog_color & 0x07C0) >> 5;
    let fog_b = fog_color & 0x001F;

    // Pre-scale fog contribution by inverse level
    let inv_level = 100 - level;
    let fog_r = fog_r * inv_level / 100;
    let fog_g = fog_g * inv_level / 100;
    let fog_b = fog_b * inv_level / 100;

    for pixel in data.iter_mut() {
        if *pixel != TRANSPARENT_COLOR_16 && *pixel != shadow_color {
            let r = (*pixel & 0xF800) >> 11;
            let g = (*pixel & 0x07C0) >> 5;
            let b = *pixel & 0x001F;

            let r2 = r * level / 100 + fog_r;
            let g2 = g * level / 100 + fog_g;
            let b2 = b * level / 100 + fog_b;

            *pixel = ((r2 << 11) & 0xF800) | ((g2 << 5) & 0x07C0) | (b2 & 0x001F);
        }
    }
}

/// Convert decompressed pixel data from 16-bit RGB565 to 15-bit RGB555.
///
/// Processes row by row respecting pitch, skipping shadow-colored pixels.
pub fn convert_decompressed_to_15bit(
    dst: &mut [u16],
    pitch_words: usize,
    width: usize,
    height: usize,
    shadow_color: u16,
) {
    let mut row_start = 0;
    for _ in 0..height {
        for i in 0..width {
            let pixel = dst[row_start + i];
            if pixel != shadow_color {
                dst[row_start + i] = ((pixel & 0xFFC0) >> 1) | (pixel & 0x1F);
            }
        }
        row_start += pitch_words;
    }
}

/// Convert decompressed pixel data from 15-bit RGB555 back to 16-bit RGB565.
///
/// Widens 5-bit green back to 6-bit green by shifting R and G left by
/// one (mask `0xFFE0` preserves R and the upper 6 bits of G), preserving
/// blue. Inverse of [`convert_decompressed_to_15bit`]. Skips shadow-colored
/// pixels.
///
/// Kept for symmetry with the 15-bit variant; only the editor/builder
/// call sites exercise this path, no runtime caller does today.
pub fn convert_decompressed_to_16bit(
    dst: &mut [u16],
    pitch_words: usize,
    width: usize,
    height: usize,
    shadow_color: u16,
) {
    let mut row_start = 0;
    for _ in 0..height {
        for i in 0..width {
            let pixel = dst[row_start + i];
            if pixel != shadow_color {
                dst[row_start + i] = ((pixel << 1) & 0xFFE0) | (pixel & 0x1F);
            }
        }
        row_start += pitch_words;
    }
}
