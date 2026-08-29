//! Lossy-JXL representation of RLE sprites (map patches / ambient
//! animation frames) for the WEB shipping format.
//!
//! Research basis: docs/COMPRESSION.md "Follow-up: the RLE/patch bucket —
//! lossy JXL WINS here (2026-08-29)" and the methodology prototyped in
//! `examples/jxl_sprite_probe.rs`. An RLE sprite is shipped as
//!
//! - a region of a JXL image (a per-animation-group atlas, or a
//!   single-sprite image), carrying only the OPAQUE RGB values, and
//! - a lossless 2-bit per-pixel class map that reconstructs the RLE run
//!   extents and every key literal exactly.
//!
//! Only opaque RGB is lossy; transparency, shadows, and in-run
//! transparent-key literals are bit-exact by construction. This is why the
//! path is web-only: composited RGB565 framebuffers are no longer
//! bit-identical to the native build, which parity traces screenshot.

use anyhow::{Result, anyhow, bail};

use crate::frame_holder::{SHADOW_KEY, TRANSPARENT_COLOR_16};

/// 2-bit per-pixel classes. `CL_KEYLIT` marks the rare in-run literal that
/// carries the transparent-key VALUE — its position is inside the RLE run,
/// so it is not background, and the class map is what keeps the distinction
/// after the lossy round trip.
pub const CL_TRANS: u8 = 0; // transparent / outside any RLE run
pub const CL_SHADOW: u8 = 1; // shadow-key literal
pub const CL_OPAQUE: u8 = 2; // real color literal (the only lossy class)
pub const CL_KEYLIT: u8 = 3; // literal with the transparent-key value

/// Decode one RLE sprite's packed words to a full canvas of pixels and
/// classes. Returns the number of packed words consumed — sprites whose
/// stream carries trailing words beyond the row walk cannot round-trip
/// through a canvas representation and must keep their exact packed words.
pub fn decode_rle_canvas(
    width: usize,
    height: usize,
    packed: &[u16],
) -> Result<(Vec<u16>, Vec<u8>, usize)> {
    let mut pixels = vec![TRANSPARENT_COLOR_16; width * height];
    let mut classes = vec![CL_TRANS; width * height];
    let mut p = 0usize;
    for y in 0..height {
        let first = *packed.get(p).ok_or_else(|| anyhow!("rle: truncated ctl"))?;
        let last = *packed
            .get(p + 1)
            .ok_or_else(|| anyhow!("rle: truncated ctl"))?;
        p += 2;
        if last == 0xFFFF {
            continue;
        }
        let (first, last) = (first as usize, last as usize);
        if first > last || last >= width {
            bail!("rle: bad run {first}..={last} in width {width}");
        }
        let run = last + 1 - first;
        let lits = packed
            .get(p..p + run)
            .ok_or_else(|| anyhow!("rle: truncated literals"))?;
        for (k, &c) in lits.iter().enumerate() {
            let i = y * width + first + k;
            pixels[i] = c;
            classes[i] = match c {
                SHADOW_KEY => CL_SHADOW,
                TRANSPARENT_COLOR_16 => CL_KEYLIT,
                _ => CL_OPAQUE,
            };
        }
        p += run;
    }
    Ok((pixels, classes, p))
}

/// Pack per-pixel classes into the shipped 2-bit map (4 px/byte, row-major,
/// byte-aligned per sprite).
pub fn pack_class_map(classes: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; classes.len().div_ceil(4)];
    for (i, &c) in classes.iter().enumerate() {
        out[i / 4] |= c << ((i % 4) * 2);
    }
    out
}

/// Inverse of [`pack_class_map`] for a sprite of `n` pixels.
pub fn unpack_class_map(bits: &[u8], n: usize) -> Result<Vec<u8>> {
    if bits.len() != n.div_ceil(4) {
        bail!(
            "class map is {} bytes for {n} pixels (expected {})",
            bits.len(),
            n.div_ceil(4)
        );
    }
    Ok((0..n).map(|i| (bits[i / 4] >> ((i % 4) * 2)) & 3).collect())
}

/// Byte length of one sprite's packed class map.
pub fn class_map_len(width: u16, height: u16) -> usize {
    (width as usize * height as usize).div_ceil(4)
}

/// RGB565 -> RGB888 with the usual bit-replication expansion (matches the
/// terrain/interface JXL paths and the research probe).
pub fn expand565(px: u16) -> [u8; 3] {
    let r = ((px >> 11) & 0x1F) as u8;
    let g = ((px >> 5) & 0x3F) as u8;
    let b = (px & 0x1F) as u8;
    [
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
    ]
}

/// RGB888 -> RGB565 truncating requantization (the probe's scoring rule).
pub fn quant565(r: u8, g: u8, b: u8) -> u16 {
    (((r as u16) & 0xF8) << 8) | (((g as u16) & 0xFC) << 3) | (((b as u16) & 0xF8) >> 3)
}

/// Requantized opaque pixels have no guarantee of avoiding the key values
/// (0 collisions measured over 25.5M px, but the guarantee comes from
/// here): a colliding value gets its low green bit flipped, which is at
/// most one 6-bit green step away and cannot land on the other key.
pub fn dodge_keys(px: u16) -> u16 {
    if px == TRANSPARENT_COLOR_16 || px == SHADOW_KEY {
        px ^ 0x0020
    } else {
        px
    }
}

/// Decode a JXL blob to `(width, height, RGBA8)` via jxl-rs — the same
/// decoder the runtime uses for terrain maps (`Picture::load_jxl_rgb565`).
/// Sprite atlases are encoded RGBA (alpha 0 marks don't-care pixels), but a
/// fully-opaque single-sprite image may come back without an alpha channel.
pub fn decode_jxl_rgba8(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>)> {
    use jxl::api::{
        JxlColorType, JxlDataFormat, JxlDecoder, JxlDecoderOptions, JxlOutputBuffer,
        JxlPixelFormat, ProcessingResult, states,
    };
    let mut input: &[u8] = bytes;
    let dec = JxlDecoder::<states::Initialized>::new(JxlDecoderOptions::default());
    let mut dec_with_image = match dec.process(&mut input, None) {
        Ok(ProcessingResult::Complete { result }) => result,
        Ok(ProcessingResult::NeedsMoreInput { .. }) => bail!("jxl: truncated header"),
        Err(e) => bail!("jxl: header error: {e:?}"),
    };
    let (w, h) = dec_with_image.basic_info().size;
    if w == 0 || h == 0 {
        bail!("jxl: zero-sized image");
    }
    let has_alpha = !dec_with_image.basic_info().extra_channels.is_empty();
    dec_with_image.set_pixel_format(JxlPixelFormat {
        color_type: if has_alpha {
            JxlColorType::Rgba
        } else {
            JxlColorType::Rgb
        },
        color_data_format: Some(JxlDataFormat::U8 { bit_depth: 8 }),
        extra_channel_format: if has_alpha { vec![None] } else { vec![] },
    });
    let dec_with_frame = match dec_with_image.process(&mut input, None) {
        Ok(ProcessingResult::Complete { result }) => result,
        Ok(ProcessingResult::NeedsMoreInput { .. }) => bail!("jxl: truncated frame header"),
        Err(e) => bail!("jxl: frame header error: {e:?}"),
    };
    let ch = if has_alpha { 4 } else { 3 };
    let stride = w * ch;
    let mut pixels = vec![0u8; stride * h];
    let mut bufs = vec![JxlOutputBuffer::new(&mut pixels, h, stride)];
    match dec_with_frame.process(&mut input, &mut bufs, None) {
        Ok(ProcessingResult::Complete { .. }) => {}
        Ok(ProcessingResult::NeedsMoreInput { .. }) => bail!("jxl: truncated frame"),
        Err(e) => bail!("jxl: frame error: {e:?}"),
    }
    drop(bufs);
    let rgba = if has_alpha {
        pixels
    } else {
        let mut rgba = vec![255u8; w * h * 4];
        for i in 0..w * h {
            rgba[i * 4..i * 4 + 3].copy_from_slice(&pixels[i * 3..i * 3 + 3]);
        }
        rgba
    };
    Ok((w, h, rgba))
}

/// Rebuild one sprite's exact-format packed RLE words from its class map
/// and a lossy RGBA region decode.
///
/// `rgba` / `rgba_width` describe the decoded blob; the sprite occupies the
/// `width x height` region at `(x0, y0)`. Run extents and key literals come
/// entirely from `classes`; only `CL_OPAQUE` literals take their (dodged,
/// requantized) value from the decoded image. Empty rows are emitted as
/// `(0xFFFF, 0xFFFF)`, which every runtime RLE walker treats identically to
/// the source encodings.
pub fn reconstruct_rle_packed(
    width: usize,
    height: usize,
    classes: &[u8],
    rgba: &[u8],
    rgba_width: usize,
    x0: usize,
    y0: usize,
) -> Result<Vec<u16>> {
    if classes.len() != width * height {
        bail!(
            "class map has {} entries for a {width}x{height} sprite",
            classes.len()
        );
    }
    let rgba_height = rgba.len() / (rgba_width * 4).max(1);
    if x0 + width > rgba_width || y0 + height > rgba_height {
        bail!(
            "sprite region {width}x{height}@({x0},{y0}) exceeds decoded {rgba_width}x{rgba_height} image"
        );
    }
    let mut packed = Vec::with_capacity(2 * height + width * height / 2);
    for y in 0..height {
        let row = &classes[y * width..(y + 1) * width];
        let Some(first) = row.iter().position(|&c| c != CL_TRANS) else {
            packed.push(0xFFFF);
            packed.push(0xFFFF);
            continue;
        };
        let last = row
            .iter()
            .rposition(|&c| c != CL_TRANS)
            .expect("row has a first non-transparent pixel");
        packed.push(first as u16);
        packed.push(last as u16);
        for x in first..=last {
            let value = match row[x] {
                CL_KEYLIT => TRANSPARENT_COLOR_16,
                CL_SHADOW => SHADOW_KEY,
                CL_OPAQUE => {
                    let off = ((y0 + y) * rgba_width + x0 + x) * 4;
                    dodge_keys(quant565(rgba[off], rgba[off + 1], rgba[off + 2]))
                }
                CL_TRANS => {
                    bail!("class map marks pixel ({x},{y}) transparent inside run {first}..={last}")
                }
                other => bail!("invalid class {other} at ({x},{y})"),
            };
            packed.push(value);
        }
    }
    Ok(packed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_map_roundtrips() {
        let classes = [
            CL_TRANS, CL_OPAQUE, CL_SHADOW, CL_KEYLIT, CL_OPAQUE, CL_TRANS, CL_OPAQUE,
        ];
        let packed = pack_class_map(&classes);
        assert_eq!(packed.len(), 2);
        assert_eq!(unpack_class_map(&packed, classes.len()).unwrap(), classes);
        assert!(unpack_class_map(&packed, 12).is_err());
    }

    #[test]
    fn canvas_and_reconstruction_roundtrip_the_rle_structure() {
        // 4x3: full row, empty row, run with shadow + key literal.
        let width = 4usize;
        let src: Vec<u16> = vec![
            0,
            3,
            100,
            200,
            300,
            400, // row 0: full run
            0xFFFF,
            0xFFFF, // row 1: empty
            1,
            2,
            SHADOW_KEY,
            TRANSPARENT_COLOR_16, // row 2: run 1..=2
        ];
        let (pixels, classes, used) = decode_rle_canvas(width, 3, &src).unwrap();
        assert_eq!(used, src.len());
        assert_eq!(pixels[4], TRANSPARENT_COLOR_16);
        assert_eq!(classes[width * 2 + 1], CL_SHADOW);
        assert_eq!(classes[width * 2 + 2], CL_KEYLIT);

        // Exact opaque values -> reconstruction reproduces the words
        // (empty rows normalize to the 0xFFFF,0xFFFF convention).
        let mut rgba = vec![0u8; width * 3 * 4];
        for (i, &px) in pixels.iter().enumerate() {
            if classes[i] == CL_OPAQUE {
                let [r, g, b] = expand565(px);
                rgba[i * 4..i * 4 + 4].copy_from_slice(&[r, g, b, 255]);
            }
        }
        let rebuilt = reconstruct_rle_packed(width, 3, &classes, &rgba, width, 0, 0).unwrap();
        assert_eq!(rebuilt, src);
    }

    #[test]
    fn key_collisions_are_dodged() {
        assert_eq!(dodge_keys(0x1234), 0x1234);
        let t = dodge_keys(TRANSPARENT_COLOR_16);
        let s = dodge_keys(SHADOW_KEY);
        for v in [t, s] {
            assert_ne!(v, TRANSPARENT_COLOR_16);
            assert_ne!(v, SHADOW_KEY);
        }
    }

    #[test]
    fn quantization_is_stable_through_expansion() {
        for px in [0u16, 0xFFFF, 0x1234, TRANSPARENT_COLOR_16, SHADOW_KEY] {
            let [r, g, b] = expand565(px);
            assert_eq!(quant565(r, g, b), px);
        }
    }
}
