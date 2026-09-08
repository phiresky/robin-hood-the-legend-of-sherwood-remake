//! Image transformation while preserving the legacy conversion formats.
use super::*;

/// Decode a packed 16-bit (`.map`) image and re-encode it as JXL via
/// the `cjxl` CLI. `quality = None` → lossless modular (`-d 0 --modular=1`);
/// `Some(q)` → VarDCT at quality `q`. Use effort 7: effort 9 did not
/// produce a meaningful size win for this content and is much slower.
/// Maps are opaque; RGB-only PNG input avoids redundant extra JXL channels.
pub(super) fn transcode_sixteen_to_jxl(src: &Path, quality: Option<u8>) -> Result<Vec<u8>> {
    let mut file = SbFile::open(&src.to_string_lossy(), SB_FILE_READ)
        .map_err(|e| anyhow!("open {}: {e}", src.display()))?;
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
    use robin_assets::rle_jxl::class_of;

    let decoded =
        Picture::load_jxl_rgba565_keyed(encoded).context("decode keyed interface picture")?;
    let round_trip = picture_rgb16_canvas(&decoded)?;
    if round_trip.len() != source.len() {
        bail!(
            "keyed interface picture round-tripped {} pixels, expected {}",
            round_trip.len(),
            source.len()
        );
    }
    for (index, (&want, &got)) in source.iter().zip(round_trip.iter()).enumerate() {
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
    let width = pic.width as usize;
    let height = pic.height as usize;
    let pitch = if pic.pitch == 0 {
        width * 2
    } else {
        pic.pitch as usize
    };
    let mut canvas = Vec::with_capacity(width * height);
    for y in 0..height {
        let row = pic
            .data
            .get(y * pitch..y * pitch + width * 2)
            .ok_or_else(|| anyhow!("picture row {y} is short of {width} RGB565 pixels"))?;
        canvas.extend(
            row.as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes([c[0], c[1]])),
        );
    }
    Ok(canvas)
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

    let n = pic.width as usize * pic.height as usize;
    let mut rgb = Vec::with_capacity(n * 3);
    match pic.pixel_format {
        PixelFormat::Rgb16 => {
            if pic.data.len() < n * 2 {
                bail!("RGB565 picture data is truncated");
            }
            for i in 0..n {
                let lo = pic.data[i * 2] as u16;
                let hi = pic.data[i * 2 + 1] as u16;
                let px = lo | (hi << 8);
                let r5 = ((px >> 11) & 0x1F) as u8;
                let g6 = ((px >> 5) & 0x3F) as u8;
                let b5 = (px & 0x1F) as u8;
                rgb.push((r5 << 3) | (r5 >> 2));
                rgb.push((g6 << 2) | (g6 >> 4));
                rgb.push((b5 << 3) | (b5 >> 2));
            }
        }
        _ => {
            let rgba = pic.to_rgba8888(None);
            for px in rgba.as_chunks::<4>().0 {
                rgb.extend_from_slice(&px[..3]);
            }
        }
    }
    Ok(rgb)
}
