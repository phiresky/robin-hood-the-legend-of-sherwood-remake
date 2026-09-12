//! Image transformation while preserving the legacy conversion formats.
use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyed_class_verification_allows_color_loss_but_rejects_class_loss() {
        use robin_assets::frame_holder::{SHADOW_KEY, TRANSPARENT_COLOR_16};
        let source = [TRANSPARENT_COLOR_16, SHADOW_KEY, 0xf800];
        let mut picture = Picture {
            width: 1,
            height: 3,
            pitch: 3,
            pixel_format: robin_assets::picture::PixelFormat::Rgb16,
            data: [TRANSPARENT_COLOR_16, SHADOW_KEY, 0xffff]
                .into_iter()
                .flat_map(|word| {
                    let [lo, hi] = word.to_le_bytes();
                    [lo, hi, 0xaa]
                })
                .collect(),
            palette: None,
        };
        verify_decoded_picture_classes(&picture, &source).unwrap();
        assert!(
            verify_decoded_picture_classes(&picture, &source[..2])
                .unwrap_err()
                .to_string()
                .contains("expected 2")
        );
        picture.data[3..5].copy_from_slice(&0xffff_u16.to_le_bytes());
        assert!(
            verify_decoded_picture_classes(&picture, &source)
                .unwrap_err()
                .to_string()
                .contains("pixel 1 changed class")
        );
        picture.data.truncate(7);
        assert!(verify_decoded_picture_classes(&picture, &source).is_err());
    }

    #[test]
    fn rgb565_conversions_skip_padding_and_share_validation() {
        let mut picture = Picture {
            width: 1,
            height: 2,
            pitch: 3,
            pixel_format: robin_assets::picture::PixelFormat::Rgb16,
            data: vec![0x00, 0xf8, 0xaa, 0x1f, 0x00],
            palette: None,
        };
        assert_eq!(picture_rgb16_canvas(&picture).unwrap(), [0xf800, 0x001f]);
        assert_eq!(picture_to_rgb888(&picture).unwrap(), [255, 0, 0, 0, 0, 255]);
        picture.data.pop();
        assert!(picture_rgb16_canvas(&picture).is_err());
        assert!(picture_to_rgb888(&picture).is_err());
        picture.pitch = 1;
        assert!(picture_rgb16_canvas(&picture).is_err());
        assert!(picture_to_rgb888(&picture).is_err());
        picture.pixel_format = robin_assets::picture::PixelFormat::Unset;
        assert!(picture_rgb16_canvas(&picture).is_err());
    }

    #[test]
    fn tightly_packed_rgb565_retains_every_color() {
        let picture = Picture {
            width: 256,
            height: 256,
            pitch: 0,
            pixel_format: robin_assets::picture::PixelFormat::Rgb16,
            data: (0..=u16::MAX).flat_map(u16::to_le_bytes).collect(),
            palette: None,
        };
        assert!(
            picture_rgb16_canvas(&picture)
                .unwrap()
                .into_iter()
                .eq(0..=u16::MAX)
        );
        let rgb = picture_to_rgb888(&picture).unwrap();
        for (word, pixel) in (0..=u16::MAX).zip(rgb.chunks_exact(3)) {
            let r = ((word >> 11) & 31) as u8;
            let g = ((word >> 5) & 63) as u8;
            let b = (word & 31) as u8;
            assert_eq!(
                pixel,
                [
                    (r << 3) | (r >> 2),
                    (g << 2) | (g >> 4),
                    (b << 3) | (b >> 2)
                ]
            );
        }
    }
}

/// Decode a packed 16-bit (`.map`) image and re-encode it as JXL via
/// the `cjxl` CLI. `quality = None` → lossless modular (`-d 0 --modular=1`);
/// `Some(q)` → VarDCT at quality `q`. Use effort 7: effort 9 did not
/// produce a meaningful size win for this content and is much slower.
/// Maps are opaque; RGB-only PNG input avoids redundant extra JXL channels.
pub(super) fn transcode_sixteen_to_jxl(src: &Path, quality: Option<u8>) -> Result<Vec<u8>> {
    let mut file =
        SbFile::open(&src.to_string_lossy()).map_err(|e| anyhow!("open {}: {e}", src.display()))?;
    let pic = Picture::load_sixteen_from_stream(&mut file)
        .with_context(|| format!("decode {}", src.display()))?;
    transcode_picture_to_jxl(&pic, quality)
}

pub(super) fn is_interface_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    normalized == "interface/default.res"
        || normalized == "interface/loading.pak"
        || normalized.starts_with("interface/")
        || normalized.contains("/data/interface/")
}

/// Files represented authoritatively by parsed shipping fields, plus a legacy
/// launcher slideshow that the Rust runtime never consumes. Keeping these raw
/// copies doubles their decoded wasm heap cost without providing a fallback.
pub(super) fn omit_boot_raw(path: &str) -> bool {
    matches!(
        path.replace('\\', "/").to_ascii_lowercase().as_str(),
        "interface/default.res"
            | "text/level.res"
            | "text/actors.res"
            | "sounds/exclamations/actors.res"
            | "configuration/profile.cpf"
            | "configuration/keyset1.cfg"
            | "configuration/keyset2.cfg"
            | "interface/slideshow_in.pak"
    )
}

pub(super) fn encode_interface_pak_pictures(
    pictures: &[Picture],
    format: InterfaceImageFormat,
) -> Result<Vec<EncodedPicture>> {
    let Some(q) = format.jxl_quality() else {
        bail!("raw interface pak pictures should stay in dd.raw, not dd.pak_files");
    };
    pictures
        .iter()
        .enumerate()
        .map(|(idx, pic)| {
            Ok(EncodedPicture::jxl_rgba565_keyed(
                transcode_picture_to_jxl_rgba_keyed(pic, q).with_context(|| {
                    format!(
                        "interface pak picture {idx}: encode JXL {}",
                        jxl_quality_label(q)
                    )
                })?,
            ))
        })
        .collect()
}

pub(super) fn jxl_quality_label(quality: Option<u8>) -> String {
    quality
        .map(|q| format!("q{q}"))
        .unwrap_or_else(|| "lossless".to_string())
}

/// RGBA with the sprite transparency key mapped to alpha; effort 9 because
/// interface art is small and encoded once.
///
/// `to_rgba8888` writes flat black `(0,0,0,0)` for keyed pixels, which drags
/// edge pixels dark under lossy DCT (the same bleeding the RLE sprite path
/// avoids). At a lossy quality the invisible RGB is free to be anything, so
/// edge-extend it instead; at `-d 0` the keyed pixels are coded exactly as
/// given, so leave the existing bytes alone.
pub(super) fn transcode_picture_to_jxl_rgba_keyed(
    pic: &Picture,
    quality: Option<u8>,
) -> Result<Vec<u8>> {
    use robin_assets::frame_holder::TRANSPARENT_COLOR_16;
    use robin_assets::picture::PixelFormat;

    // RGB16 interface art carries BOTH key colors as literal pixel values:
    // transparent (bright green) and SHADOW_KEY (pure blue). Neither may be
    // coded as color — VarDCT ringing bleeds them into visible neighbours,
    // and a lossy round trip breaks the exact `== SHADOW_KEY` comparisons
    // the cursor and UI paths run, which renders shadows as raw blue. Carry
    // both classes in the alpha channel instead (the scheme the RLE sprite
    // atlases already use, with alpha coded losslessly) and edge-extend the
    // color underneath.
    if pic.pixel_format == PixelFormat::Rgb16 && quality.is_some() {
        let canvas = picture_rgb16_canvas(pic)?;
        let mut rgba = robin_assets::rle_jxl::canvas_to_rgba(&canvas)?;
        robin_assets::rle_jxl::smear_invisible_rgb(
            &mut rgba,
            pic.width as usize,
            pic.height as usize,
        );
        let encoded = transcode_pixels_to_jxl(pic, rgba, png::ColorType::Rgba, quality, 9)?;
        verify_keyed_picture_classes(&encoded, &canvas)?;
        return Ok(encoded);
    }

    let mut rgba = pic.to_rgba8888(Some(TRANSPARENT_COLOR_16));
    if quality.is_some() {
        // `to_rgba8888` emits alpha 255 for every visible pixel, which is
        // exactly the "opaque" marker the smear treats as a color source.
        robin_assets::rle_jxl::smear_invisible_rgb(
            &mut rgba,
            pic.width as usize,
            pic.height as usize,
        );
    }
    transcode_pixels_to_jxl(pic, rgba, png::ColorType::Rgba, quality, 9)
}

/// Decode a just-encoded keyed picture and fail the conversion if any pixel
/// changed CLASS. Color is lossy by design, but transparent and shadow are
/// exact comparisons at runtime (`px == SHADOW_KEY` in the cursor and UI
/// paths), so a class that shifts is silent corruption — a blue cursor
/// shadow, or a hole where art should be. Cheap next to the encode.
pub(super) fn verify_keyed_picture_classes(encoded: &[u8], source: &[u16]) -> Result<()> {
    let decoded =
        Picture::load_jxl_rgba565_keyed(encoded).context("decode keyed interface picture")?;
    verify_decoded_picture_classes(&decoded, source)
}

fn verify_decoded_picture_classes(decoded: &Picture, source: &[u16]) -> Result<()> {
    use robin_assets::rle_jxl::class_of;
    let round_trip = picture_rgb16_pixels(decoded)?;
    let pixel_count = usize::from(decoded.width) * usize::from(decoded.height);
    if pixel_count != source.len() {
        bail!(
            "keyed interface picture round-tripped {} pixels, expected {}",
            pixel_count,
            source.len()
        );
    }
    for (index, (&want, got)) in source.iter().zip(round_trip).enumerate() {
        if class_of(want) != class_of(got) {
            bail!(
                "keyed interface picture pixel {index} changed class: source {want:#06x} \
                 decoded {got:#06x} — the alpha channel is not surviving the encode"
            );
        }
    }
    Ok(())
}

/// Row-major RGB565 words of an `Rgb16` picture, dropping any pitch padding.
pub(super) fn picture_rgb16_canvas(pic: &Picture) -> Result<Vec<u16>> {
    let pixels = picture_rgb16_pixels(pic)?;
    let mut canvas = Vec::with_capacity(usize::from(pic.width) * usize::from(pic.height));
    canvas.extend(pixels);
    Ok(canvas)
}

fn picture_rgb16_pixels(pic: &Picture) -> Result<impl Iterator<Item = u16> + '_> {
    anyhow::ensure!(
        pic.pixel_format == robin_assets::picture::PixelFormat::Rgb16,
        "RGB565 conversion requires an Rgb16 picture"
    );
    let width = pic.width as usize;
    let height = pic.height as usize;
    let pitch = if pic.pitch == 0 {
        width * 2
    } else {
        pic.pitch as usize
    };
    anyhow::ensure!(
        pitch >= width * 2,
        "RGB565 picture pitch is shorter than a row"
    );
    let required = if height == 0 || width == 0 {
        0
    } else {
        (height - 1)
            .checked_mul(pitch)
            .and_then(|start| start.checked_add(width * 2))
            .context("RGB565 picture layout exceeds address space")?
    };
    anyhow::ensure!(
        pic.data.len() >= required,
        "RGB565 picture data is truncated"
    );
    Ok((0..height).flat_map(move |y| {
        // Empty-width pictures have no rows to read, regardless of pitch.
        let start = if width == 0 { 0 } else { y * pitch };
        pic.data[start..start + width * 2]
            .chunks_exact(2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
    }))
}

/// Opaque RGB (maps); effort 7 — effort 9 did not produce a meaningful size
/// win for this content and is much slower.
pub(super) fn transcode_picture_to_jxl(pic: &Picture, quality: Option<u8>) -> Result<Vec<u8>> {
    let rgb = picture_to_rgb888(pic)?;
    transcode_pixels_to_jxl(pic, rgb, png::ColorType::Rgb, quality, 7)
}

/// Encode raw pixel data to JXL by piping a minimal PNG through the `cjxl`
/// CLI (PNG on stdin → JXL on stdout). `quality = None` → lossless modular
/// (`-d 0 --modular=1`); `Some(q)` → VarDCT at quality `q`.
///
/// stdin is fed from a scoped thread while the parent drains stdout/stderr,
/// so a picture larger than the pipe buffer cannot deadlock the exchange.
pub(super) fn transcode_pixels_to_jxl(
    pic: &Picture,
    pixels: Vec<u8>,
    color: png::ColorType,
    quality: Option<u8>,
    effort: u8,
) -> Result<Vec<u8>> {
    encode_pixels_to_jxl(
        pic.width as u32,
        pic.height as u32,
        &pixels,
        color,
        quality,
        effort,
    )
}

/// Dimension-explicit form of [`transcode_pixels_to_jxl`]; the RLE sprite
/// atlas path has no `Picture` to borrow dims from.
pub(super) fn encode_pixels_to_jxl(
    width: u32,
    height: u32,
    pixels: &[u8],
    color: png::ColorType,
    quality: Option<u8>,
    effort: u8,
) -> Result<Vec<u8>> {
    use std::io::Write as _;
    use std::process::Stdio;

    let mut png_bytes: Vec<u8> = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png_bytes, width, height);
        enc.set_color(color);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().context("png header")?;
        w.write_image_data(pixels).context("png data")?;
        w.finish().context("png finish")?;
    }

    let mut cmd = Command::new("cjxl");
    let effort = effort.to_string();
    if let Some(q) = quality {
        // `--alpha_distance=0` keeps the extra channel mathematically
        // lossless while the color channels stay lossy. cjxl 0.12 defaults
        // it to 0 already; passing it explicitly means a future default
        // change cannot silently corrupt the sprite class channel (which
        // `member_quality` would then catch as a hard error anyway).
        cmd.args([
            "-q",
            &q.to_string(),
            "--alpha_distance=0",
            "-e",
            &effort,
            "-",
            "-",
        ]);
    } else {
        cmd.args(["-d", "0", "--modular=1", "-e", &effort, "-", "-"]);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn cjxl (is it installed?)")?;
    let mut stdin = child.stdin.take().expect("cjxl stdin was requested piped");
    let (out, write_result) = std::thread::scope(|scope| {
        let writer = scope.spawn(move || {
            let result = stdin.write_all(&png_bytes);
            // Explicit drop closes the pipe so cjxl sees EOF.
            drop(stdin);
            result
        });
        // `wait_with_output` drains stdout and stderr concurrently while
        // the writer thread feeds stdin.
        let out = child.wait_with_output().context("cjxl wait");
        let write_result = writer.join().expect("cjxl stdin writer thread panicked");
        (out, write_result)
    });
    let out = out?;
    if !out.status.success() {
        bail!(
            "cjxl failed: exit {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    write_result.context("write PNG to cjxl")?;
    Ok(out.stdout)
}

pub(super) fn picture_to_rgb888(pic: &Picture) -> Result<Vec<u8>> {
    use robin_assets::picture::PixelFormat;

    match pic.pixel_format {
        PixelFormat::Rgb16 => {
            let pixels = picture_rgb16_pixels(pic)?;
            let mut rgb = Vec::with_capacity(usize::from(pic.width) * usize::from(pic.height) * 3);
            rgb.extend(pixels.flat_map(robin_assets::rle_jxl::expand565));
            Ok(rgb)
        }
        _ => {
            let rgba = pic.to_rgba8888(None);
            Ok(rgba
                .chunks_exact(4)
                .flat_map(|px| px[..3].iter().copied())
                .collect())
        }
    }
}
