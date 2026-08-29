//! Lossy-JXL representation of RLE sprites (map patches / ambient
//! animation frames) for the WEB shipping format.
//!
//! Research basis: docs/COMPRESSION.md "Follow-up: the RLE/patch bucket —
//! lossy JXL WINS here (2026-08-29)" and the methodology prototyped in
//! `examples/jxl_sprite_probe.rs`. An RLE sprite ships as a region of ONE
//! RGBA JXL image (a per-animation-group atlas, or a single-sprite image):
//!
//! - the **color channels** carry the visible RGB, coded lossily (VarDCT);
//! - the **alpha channel** carries the per-pixel CLASS, coded losslessly
//!   (`cjxl --alpha_distance=0`): transparent, shadow, or opaque. A lossy
//!   color channel cannot be trusted to say which pixels are keyed, so
//!   this channel is what makes the sprite exactly reconstructible as a
//!   raster.
//!
//! Alpha is a class marker, NOT a blend factor. Materialization turns the
//! decoded image straight into the RGB565 canvas the sprite bank hands to
//! its consumers, with `SHADOW_KEY` in the shadow pixels and
//! `TRANSPARENT_COLOR_16` in the transparent ones — byte-for-byte the
//! shape a natively converted RLE sprite decompresses to. The
//! ambience-dependent shadow substitution still happens downstream in
//! [`crate::frame_holder`], unchanged, so nothing composites the stored
//! RGB at partial opacity.
//!
//! Only visible RGB is lossy, which is why this path is web-only:
//! composited RGB565 framebuffers are no longer bit-identical to the
//! native build, and parity traces screenshot those.

use anyhow::{Result, bail};

use crate::frame_holder::{SHADOW_KEY, TRANSPARENT_COLOR_16};

/// Per-pixel classes. The canvas value determines the class, so this is a
/// view of the pixel rather than data that needs storing beside it.
pub const CL_TRANS: u8 = 0; // transparent (outside a run, or an in-run key literal)
pub const CL_SHADOW: u8 = 1; // shadow-key literal
pub const CL_OPAQUE: u8 = 2; // real color — the only lossy class

/// Alpha byte each class is stored as. Values are spread across the range
/// so a hypothetical off-by-one from some future encoder is a loud error
/// rather than a silently reclassified pixel.
pub const ALPHA_TRANS: u8 = 0;
pub const ALPHA_SHADOW: u8 = 128;
pub const ALPHA_OPAQUE: u8 = 255;

/// Class of a canvas pixel.
///
/// An in-run literal that carries the transparent-key VALUE classifies as
/// transparent, which is not a loss of information: every consumer in
/// [`crate::frame_holder`] (the ArnoLaw blit, both shadow-extraction
/// blits, and the hit-test lookup) already produces exactly
/// `TRANSPARENT_COLOR_16` for such a literal, identical to an
/// outside-the-run pixel.
pub fn class_of(pixel: u16) -> u8 {
    match pixel {
        TRANSPARENT_COLOR_16 => CL_TRANS,
        SHADOW_KEY => CL_SHADOW,
        _ => CL_OPAQUE,
    }
}

pub fn class_to_alpha(class: u8) -> Result<u8> {
    Ok(match class {
        CL_TRANS => ALPHA_TRANS,
        CL_SHADOW => ALPHA_SHADOW,
        CL_OPAQUE => ALPHA_OPAQUE,
        other => bail!("invalid sprite pixel class {other}"),
    })
}

pub fn alpha_to_class(alpha: u8) -> Result<u8> {
    Ok(match alpha {
        ALPHA_TRANS => CL_TRANS,
        ALPHA_SHADOW => CL_SHADOW,
        ALPHA_OPAQUE => CL_OPAQUE,
        other => bail!(
            "sprite alpha {other} is not one of the class markers \
             ({ALPHA_TRANS}/{ALPHA_SHADOW}/{ALPHA_OPAQUE}) — the encoder did not code the alpha \
             channel losslessly"
        ),
    })
}

/// Decode one RLE sprite's packed words into the full RGB565 canvas the
/// sprite bank's consumers see, with the key values in place. Returns the
/// number of packed words consumed — a sprite whose stream carries
/// trailing words beyond the row walk cannot round-trip through a canvas
/// and must keep its exact packed words.
pub fn decode_rle_canvas(width: usize, height: usize, packed: &[u16]) -> Result<(Vec<u16>, usize)> {
    let mut pixels = vec![TRANSPARENT_COLOR_16; width * height];
    let mut p = 0usize;
    for y in 0..height {
        let (Some(&first), Some(&last)) = (packed.get(p), packed.get(p + 1)) else {
            bail!("rle: truncated control words");
        };
        p += 2;
        if last == 0xFFFF {
            if first != 0xFFFF && first != 0 {
                // `decompress_rle_arno_law` emits `first` leading pixels
                // even for an empty row, so this shape would draw shifted;
                // it does not occur in real banks. Refuse rather than
                // normalize it away silently (the converter then keeps
                // such a sprite's exact words).
                bail!("rle: empty row with nonzero first={first}");
            }
            continue;
        }
        let (first, last) = (first as usize, last as usize);
        if first > last || last >= width {
            bail!("rle: bad run {first}..={last} in width {width}");
        }
        let run = last + 1 - first;
        let Some(literals) = packed.get(p..p + run) else {
            bail!("rle: truncated literals");
        };
        pixels[y * width + first..y * width + first + run].copy_from_slice(literals);
        p += run;
    }
    Ok((pixels, p))
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

/// RGB888 -> RGB565 truncating requantization.
pub fn quant565(r: u8, g: u8, b: u8) -> u16 {
    (((r as u16) & 0xF8) << 8) | (((g as u16) & 0xFC) << 3) | (((b as u16) & 0xF8) >> 3)
}

/// Requantized visible pixels have no guarantee of avoiding the key values
/// (0 collisions were measured over 25.5M px, but the guarantee comes from
/// here): a colliding value gets its low green bit flipped, which is at
/// most one 6-bit green step away and cannot land on the other key.
pub fn dodge_keys(px: u16) -> u16 {
    if px == TRANSPARENT_COLOR_16 || px == SHADOW_KEY {
        px ^ 0x0020
    } else {
        px
    }
}

/// Encoder-side RGBA for one canvas: visible pixels expand 565 -> 888 in
/// the color channels, and EVERY pixel's alpha carries its class marker.
/// Invisible pixels get color 0 here and are edge-extended by
/// [`smear_invisible_rgb`] afterwards — never the literal key colors,
/// which are bright green/blue and would bleed through VarDCT ringing.
pub fn canvas_to_rgba(pixels: &[u16]) -> Result<Vec<u8>> {
    let mut rgba = vec![0u8; pixels.len() * 4];
    for (i, &px) in pixels.iter().enumerate() {
        let class = class_of(px);
        if class == CL_OPAQUE {
            rgba[i * 4..i * 4 + 3].copy_from_slice(&expand565(px));
        }
        rgba[i * 4 + 3] = class_to_alpha(class)?;
    }
    Ok(rgba)
}

/// How far the encoder-side edge extension reaches. Beyond a couple of
/// VarDCT blocks an invisible pixel cannot influence a visible one, and
/// leaving distant background flat keeps large empty atlas gutters cheap.
pub const SMEAR_RADIUS: usize = 8;

/// Edge-extend the color channels of an RGBA buffer: every pixel that is
/// not [`ALPHA_OPAQUE`] takes the color of its nearest opaque pixel (BFS,
/// 4-connected, up to [`SMEAR_RADIUS`]), leaving alpha untouched.
///
/// Invisible RGB is discarded at decode (classes come from alpha), so this
/// is purely an encoder-side choice — and a load-bearing one: the key
/// colors must never be coded literally, while flat black (what the older
/// keyed paths write) still drags edge pixels dark under lossy DCT.
/// Continuing the neighbouring color gives the DCT a smooth signal across
/// the sprite boundary instead.
pub fn smear_invisible_rgb(rgba: &mut [u8], width: usize, height: usize) {
    if width == 0 || height == 0 {
        return;
    }
    debug_assert_eq!(rgba.len(), width * height * 4);
    let mut frontier: Vec<u32> = (0..width * height)
        .filter(|&i| rgba[i * 4 + 3] == ALPHA_OPAQUE)
        .map(|i| i as u32)
        .collect();
    if frontier.is_empty() || frontier.len() == width * height {
        return;
    }
    let mut filled: Vec<bool> = (0..width * height)
        .map(|i| rgba[i * 4 + 3] == ALPHA_OPAQUE)
        .collect();
    let mut next: Vec<u32> = Vec::new();
    for _ in 0..SMEAR_RADIUS {
        next.clear();
        for &index in &frontier {
            let index = index as usize;
            let (x, y) = (index % width, index / width);
            let neighbors = [
                (x > 0).then(|| index - 1),
                (x + 1 < width).then(|| index + 1),
                (y > 0).then(|| index - width),
                (y + 1 < height).then(|| index + width),
            ];
            for target in neighbors.into_iter().flatten() {
                if filled[target] {
                    continue;
                }
                filled[target] = true;
                let source = index * 4;
                let color = [rgba[source], rgba[source + 1], rgba[source + 2]];
                rgba[target * 4..target * 4 + 3].copy_from_slice(&color);
                next.push(target as u32);
            }
        }
        if next.is_empty() {
            break;
        }
        std::mem::swap(&mut frontier, &mut next);
    }
}

/// Decode a JXL blob to `(width, height, RGBA8)` via jxl-rs — the same
/// decoder the runtime uses for terrain maps (`Picture::load_jxl_rgb565`).
///
/// The image MUST carry a straight (non-premultiplied) alpha channel:
/// premultiplication scales the color channels by alpha, which would both
/// destroy the smeared edge colors and make visible-region RGB depend on a
/// channel this format uses as a class marker.
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
    match dec_with_image.basic_info().extra_channels.first() {
        Some(channel) if channel.ec_type == jxl::headers::extra_channels::ExtraChannel::Alpha => {
            if channel.alpha_associated {
                bail!(
                    "jxl: sprite image carries premultiplied (associated) alpha; the class \
                     channel must be straight alpha"
                );
            }
        }
        _ => bail!("jxl: sprite image has no alpha channel to carry pixel classes"),
    }
    dec_with_image.set_pixel_format(JxlPixelFormat {
        color_type: JxlColorType::Rgba,
        color_data_format: Some(JxlDataFormat::U8 { bit_depth: 8 }),
        extra_channel_format: vec![None],
    });
    let dec_with_frame = match dec_with_image.process(&mut input, None) {
        Ok(ProcessingResult::Complete { result }) => result,
        Ok(ProcessingResult::NeedsMoreInput { .. }) => bail!("jxl: truncated frame header"),
        Err(e) => bail!("jxl: frame header error: {e:?}"),
    };
    let stride = w * 4;
    let mut pixels = vec![0u8; stride * h];
    let mut bufs = vec![JxlOutputBuffer::new(&mut pixels, h, stride)];
    match dec_with_frame.process(&mut input, &mut bufs, None) {
        Ok(ProcessingResult::Complete { .. }) => {}
        Ok(ProcessingResult::NeedsMoreInput { .. }) => bail!("jxl: truncated frame"),
        Err(e) => bail!("jxl: frame error: {e:?}"),
    }
    drop(bufs);
    Ok((w, h, pixels))
}

/// Turn a decoded RGBA image into the RGB565 canvas the sprite bank
/// serves: class (and therefore every keyed pixel) comes from the
/// losslessly coded alpha channel, and only visible pixels take their
/// requantized, key-dodged value from the lossy color channels.
///
/// This is the whole load-time step — the packed RLE run format is never
/// rebuilt, because nothing downstream reads runs: every consumer
/// decompresses to exactly this raster anyway.
pub fn canvas_from_rgba(rgba: &[u8]) -> Result<Vec<u16>> {
    if !rgba.len().is_multiple_of(4) {
        bail!("decoded image is not whole RGBA pixels");
    }
    rgba.chunks_exact(4)
        .map(|px| {
            Ok(match alpha_to_class(px[3])? {
                CL_TRANS => TRANSPARENT_COLOR_16,
                CL_SHADOW => SHADOW_KEY,
                _ => dodge_keys(quant565(px[0], px[1], px[2])),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_alpha_mapping_roundtrips_and_rejects_lossy_alpha() {
        for class in [CL_TRANS, CL_SHADOW, CL_OPAQUE] {
            assert_eq!(
                alpha_to_class(class_to_alpha(class).unwrap()).unwrap(),
                class
            );
        }
        // Values one step off a marker (what a lossy alpha pass would
        // produce) must fail loudly rather than reclassify the pixel.
        assert!(alpha_to_class(ALPHA_OPAQUE - 1).is_err());
        assert!(alpha_to_class(ALPHA_SHADOW + 1).is_err());
    }

    #[test]
    fn canvas_survives_a_lossless_rgba_roundtrip() {
        // 4x3: full row, empty row, run with shadow + in-run key literal.
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
        let (canvas, used) = decode_rle_canvas(width, 3, &src).unwrap();
        assert_eq!(used, src.len());
        assert_eq!(canvas[4], TRANSPARENT_COLOR_16);
        assert_eq!(canvas[width * 2 + 1], SHADOW_KEY);
        // The in-run transparent-key literal is a transparent pixel: every
        // consumer produces exactly this value for it.
        assert_eq!(canvas[width * 2 + 2], TRANSPARENT_COLOR_16);

        let rgba = canvas_to_rgba(&canvas).unwrap();
        assert_eq!(canvas_from_rgba(&rgba).unwrap(), canvas);
    }

    #[test]
    fn smear_extends_color_into_invisible_pixels_without_touching_alpha() {
        // 3x1: opaque red, transparent, shadow. Both invisible pixels take
        // the red color; alpha (the class) is untouched, so the decoded
        // canvas is unchanged by smearing.
        let canvas = [0xF800u16, TRANSPARENT_COLOR_16, SHADOW_KEY];
        let mut rgba = canvas_to_rgba(&canvas).unwrap();
        assert_eq!(&rgba[4..8], &[0, 0, 0, ALPHA_TRANS]);
        smear_invisible_rgb(&mut rgba, 3, 1);
        let red = expand565(0xF800);
        assert_eq!(&rgba[4..7], &red);
        assert_eq!(&rgba[8..11], &red);
        assert_eq!(
            [rgba[3], rgba[7], rgba[11]],
            [255, ALPHA_TRANS, ALPHA_SHADOW]
        );
        assert_eq!(canvas_from_rgba(&rgba).unwrap(), canvas);
    }

    #[test]
    fn smear_is_a_noop_without_opaque_pixels() {
        let mut rgba = canvas_to_rgba(&[TRANSPARENT_COLOR_16; 4]).unwrap();
        smear_invisible_rgb(&mut rgba, 2, 2);
        assert!(rgba.iter().all(|&b| b == 0));
    }

    #[test]
    fn key_collisions_are_dodged() {
        assert_eq!(dodge_keys(0x1234), 0x1234);
        for value in [dodge_keys(TRANSPARENT_COLOR_16), dodge_keys(SHADOW_KEY)] {
            assert_ne!(value, TRANSPARENT_COLOR_16);
            assert_ne!(value, SHADOW_KEY);
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
