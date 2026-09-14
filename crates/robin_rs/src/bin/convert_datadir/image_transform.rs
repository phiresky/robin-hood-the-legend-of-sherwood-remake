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

/// Decode a packed 16-bit minimap (`.min`) and encode it as keyed RGBA JXL.
/// Unlike `.map` terrain, minimaps carry the exact transparent key around
/// the playfield; see [`encode_minimap_picture_to_jxl`].
pub(super) fn transcode_minimap_to_jxl(src: &Path, quality: Option<u8>) -> Result<Vec<u8>> {
    let mut file =
        SbFile::open(&src.to_string_lossy()).map_err(|e| anyhow!("open {}: {e}", src.display()))?;
    let pic = Picture::load_sixteen_from_stream(&mut file)
        .with_context(|| format!("decode {}", src.display()))?;
    encode_minimap_picture_to_jxl(&pic, quality)
}

/// Minimaps use the interface pictures' keyed encoding: the pixel class
/// (transparent / shadow / opaque) lives in a losslessly coded alpha channel,
/// so a lossy colour pass cannot turn the transparent key into near-green
/// that the renderer and minimap hit mask would treat as visible. The runtime
/// decodes it with `Picture::load_minimap_from_bytes`.
pub(super) fn encode_minimap_picture_to_jxl(pic: &Picture, quality: Option<u8>) -> Result<Vec<u8>> {
    // TODO: at `quality = None` the keyed encoder takes the `to_rgba8888`
    // path (alpha only marks the transparent key), so a lossless minimap
    // pixel that is exactly SHADOW_KEY decodes nudged by one step. Harmless
    // for minimaps; revisit if lossless JXL minimaps ever ship.
    transcode_picture_to_jxl_rgba_keyed(pic, quality)
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
    let Some(codec) = format.web_codec() else {
        bail!("raw interface pak pictures should stay in dd.raw, not dd.pak_files");
    };
    pictures
        .iter()
        .enumerate()
        .map(|(idx, pic)| {
            encode_interface_picture(pic, codec)
                .with_context(|| format!("interface pak picture {idx}: encode {}", codec.label()))
        })
        .collect()
}

/// One interface picture (`.res` or `.pak`) in the chosen shipping codec.
pub(super) fn encode_interface_picture(
    pic: &Picture,
    codec: WebImageCodec,
) -> Result<EncodedPicture> {
    match codec {
        WebImageCodec::Jxl(quality) => Ok(EncodedPicture::jxl_rgba565_keyed(
            transcode_picture_to_jxl_rgba_keyed(pic, quality)?,
        )),
        WebImageCodec::Avif(quality) => encode_keyed_picture_avif(pic, quality),
    }
}

/// `avifenc` worker threads. `-j` changes the output bytes slightly, so it is
/// fixed per asset kind — never derived from the machine's core count — to
/// keep conversions reproducible across hosts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AvifJobs {
    /// `-j 1`: the converter already parallelizes across these images.
    Single,
    /// `-j all`: big images encoded one at a time (terrain maps, minimaps).
    All,
}

/// libaom encoder speed of the web recipe (`avifenc -s 2`).
const AVIF_ENCODER_SPEED: &str = "2";

/// Which representation a keyed AVIF-recipe interface picture ships as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KeyedPictureEncoding {
    /// Exact `EncodedPicture::rgb565_raw` (`w*h*2 + 4` bytes).
    Rgb565Raw,
    /// Lossy keyed RGBA AVIF.
    Avif,
}

/// Tiny-picture policy of the AVIF interface recipe: ship the exact raw
/// RGB565 bytes when they are no larger than the AVIF, or when the AVIF's
/// opaque PSNR565 falls below [`KEYED_PICTURE_MIN_PSNR_DB`]; otherwise the
/// AVIF. `avif_opaque_psnr_db` is only evaluated (it needs a decode) when the
/// size comparison does not already pick raw. Measured on the Demo interface
/// corpus: 231 of 947 pictures ship raw and none of the AVIFs is below 20 dB.
pub(super) fn choose_keyed_picture_encoding(
    width: u16,
    height: u16,
    avif_bytes: usize,
    avif_opaque_psnr_db: impl FnOnce() -> Result<f64>,
) -> Result<KeyedPictureEncoding> {
    let raw_bytes = usize::from(width) * usize::from(height) * 2 + 4;
    if raw_bytes <= avif_bytes || avif_opaque_psnr_db()? < KEYED_PICTURE_MIN_PSNR_DB {
        Ok(KeyedPictureEncoding::Rgb565Raw)
    } else {
        Ok(KeyedPictureEncoding::Avif)
    }
}

/// Class-marked, edge-extended RGBA for a keyed `Rgb16` picture — the same
/// encoder input the lossy keyed JXL path builds — plus its source canvas.
/// A fully opaque picture encodes without an alpha item (libavif omits an
/// all-255 alpha plane); the runtime keyed decoder reads that as all-opaque.
fn keyed_canvas_rgba(pic: &Picture) -> Result<(Vec<u16>, Vec<u8>)> {
    let canvas = picture_rgb16_canvas(pic)?;
    let mut rgba = robin_assets::rle_jxl::canvas_to_rgba(&canvas)?;
    robin_assets::rle_jxl::smear_invisible_rgb(&mut rgba, pic.width as usize, pic.height as usize);
    Ok((canvas, rgba))
}

/// Decode a keyed AVIF natively exactly as the runtime reconstructs it.
pub(super) fn decode_keyed_avif(bytes: &[u8]) -> Result<Picture> {
    use robin_assets::browser_images;
    let decoded = browser_images::decode_avif_rgba8(bytes).context("decode keyed AVIF")?;
    browser_images::rgba_to_rgb565_keyed(&decoded)
}

/// Keyed interface picture for the AVIF recipe: class-marked RGBA AVIF with
/// lossless alpha, or exact raw RGB565 per [`choose_keyed_picture_encoding`].
pub(super) fn encode_keyed_picture_avif(pic: &Picture, quality: u8) -> Result<EncodedPicture> {
    use robin_assets::frame_holder::TRANSPARENT_COLOR_16;
    use robin_assets::picture::PixelFormat;

    if pic.pixel_format != PixelFormat::Rgb16 {
        // Mirrors the JXL path's non-RGB16 branch: only the transparent key
        // maps to alpha.
        // TODO: no class verification or raw fallback for non-RGB16 interface
        // pictures (the JXL path has none either); check whether any exist.
        let mut rgba = pic.to_rgba8888(Some(TRANSPARENT_COLOR_16));
        robin_assets::rle_jxl::smear_invisible_rgb(
            &mut rgba,
            pic.width as usize,
            pic.height as usize,
        );
        let encoded = encode_pixels_to_avif(
            u32::from(pic.width),
            u32::from(pic.height),
            &rgba,
            png::ColorType::Rgba,
            quality,
            AvifJobs::Single,
        )?;
        return Ok(EncodedPicture::avif_rgba565_keyed(encoded));
    }

    let (canvas, rgba) = keyed_canvas_rgba(pic)?;
    let encoded = encode_pixels_to_avif(
        u32::from(pic.width),
        u32::from(pic.height),
        &rgba,
        png::ColorType::Rgba,
        quality,
        AvifJobs::Single,
    )?;
    let mut psnr = None;
    let choice = choose_keyed_picture_encoding(pic.width, pic.height, encoded.len(), || {
        let decoded = decode_keyed_avif(&encoded)?;
        verify_decoded_picture_classes(&decoded, &canvas)?;
        let score = keyed_opaque_psnr565(&decoded, &canvas)?;
        psnr = Some(score);
        Ok(score)
    })?;
    match choice {
        KeyedPictureEncoding::Avif => Ok(EncodedPicture::avif_rgba565_keyed(encoded)),
        KeyedPictureEncoding::Rgb565Raw => {
            tracing::debug!(
                width = pic.width,
                height = pic.height,
                avif_bytes = encoded.len(),
                avif_psnr_db = psnr,
                "keyed picture ships as exact raw RGB565"
            );
            let tight = Picture {
                width: pic.width,
                height: pic.height,
                pitch: pic
                    .width
                    .checked_mul(2)
                    .context("picture row exceeds u16")?,
                pixel_format: PixelFormat::Rgb16,
                data: canvas.iter().copied().flat_map(u16::to_le_bytes).collect(),
                palette: None,
            };
            EncodedPicture::rgb565_raw(&tight)
        }
    }
}

/// Decode a packed 16-bit minimap (`.min`) and encode it as keyed RGBA AVIF.
pub(super) fn transcode_minimap_to_avif(src: &Path, quality: u8) -> Result<Vec<u8>> {
    encode_minimap_picture_to_avif(&load_sixteen_file(src)?, quality)
}

/// Keyed RGBA AVIF minimap (see [`encode_minimap_picture_to_jxl`] for why
/// minimaps are keyed). Always AVIF — minimaps live in `payload.raw`, not as
/// `EncodedPicture`, so there is no raw fallback — and the conversion fails
/// if any pixel class does not survive.
pub(super) fn encode_minimap_picture_to_avif(pic: &Picture, quality: u8) -> Result<Vec<u8>> {
    let (canvas, rgba) = keyed_canvas_rgba(pic)?;
    let encoded = encode_pixels_to_avif(
        u32::from(pic.width),
        u32::from(pic.height),
        &rgba,
        png::ColorType::Rgba,
        quality,
        AvifJobs::All,
    )?;
    let decoded = decode_keyed_avif(&encoded).context("decode keyed AVIF minimap")?;
    verify_decoded_picture_classes(&decoded, &canvas).context("keyed AVIF minimap classes")?;
    let psnr = keyed_opaque_psnr565(&decoded, &canvas)?;
    tracing::debug!(
        width = pic.width,
        height = pic.height,
        bytes = encoded.len(),
        psnr_db = psnr,
        "encoded keyed AVIF minimap"
    );
    Ok(encoded)
}

/// Decode a packed 16-bit (`.map`) terrain image and encode it as opaque
/// AVIF (RGB input, so no alpha item).
pub(super) fn transcode_sixteen_to_avif(src: &Path, quality: u8) -> Result<Vec<u8>> {
    transcode_picture_to_avif(&load_sixteen_file(src)?, quality)
}

/// Opaque terrain AVIF. Checks the container facts the runtime's opaque
/// loader relies on; the pixels are not decoded here.
// TODO: score terrain PSNR565 once the native AVIF decoder is fast enough for
// full-size maps in the conversion loop.
pub(super) fn transcode_picture_to_avif(pic: &Picture, quality: u8) -> Result<Vec<u8>> {
    let rgb = picture_to_rgb888(pic)?;
    let encoded = encode_pixels_to_avif(
        u32::from(pic.width),
        u32::from(pic.height),
        &rgb,
        png::ColorType::Rgb,
        quality,
        AvifJobs::All,
    )?;
    let info = robin_assets::browser_images::avif_info(&encoded)?;
    anyhow::ensure!(
        !info.has_alpha && (info.width, info.height) == (pic.width, pic.height),
        "terrain AVIF came back as {}x{} alpha={} for a {}x{} opaque picture",
        info.width,
        info.height,
        info.has_alpha,
        pic.width,
        pic.height
    );
    Ok(encoded)
}

fn load_sixteen_file(src: &Path) -> Result<Picture> {
    let mut file =
        SbFile::open(&src.to_string_lossy()).map_err(|e| anyhow!("open {}: {e}", src.display()))?;
    Picture::load_sixteen_from_stream(&mut file)
        .with_context(|| format!("decode {}", src.display()))
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
        let encoded = transcode_pixels_to_jxl(pic, rgba.clone(), png::ColorType::Rgba, quality, 9)?;
        let decoded =
            Picture::load_jxl_rgba565_keyed(&encoded).context("decode keyed interface picture")?;
        verify_decoded_picture_classes(&decoded, &canvas)?;
        let psnr = keyed_opaque_psnr565(&decoded, &canvas)?;
        if psnr >= KEYED_PICTURE_MIN_PSNR_DB {
            return Ok(encoded);
        }
        // Lossy VarDCT wrecks tiny keyed pictures (measured on the Demo
        // interface corpus: every picture with a side under 8 px scored
        // 8-19 dB at q80, and q90 did not rescue them). Lossless modular
        // of the same class-marked RGBA is exact and, for exactly these
        // pictures, was no larger (27 pictures: 1982 vs 2065 bytes).
        let lossless = transcode_pixels_to_jxl(pic, rgba, png::ColorType::Rgba, None, 9)?;
        let decoded = Picture::load_jxl_rgba565_keyed(&lossless)
            .context("decode lossless keyed interface picture")?;
        verify_decoded_picture_classes(&decoded, &canvas)?;
        let lossless_psnr = keyed_opaque_psnr565(&decoded, &canvas)?;
        anyhow::ensure!(
            lossless_psnr == f64::INFINITY,
            "lossless keyed picture {}x{} did not round-trip exactly ({lossless_psnr:.2} dB)",
            pic.width,
            pic.height
        );
        tracing::debug!(
            width = pic.width,
            height = pic.height,
            lossy_psnr_db = psnr,
            lossy_bytes = encoded.len(),
            lossless_bytes = lossless.len(),
            "keyed picture below the lossy quality floor; shipping lossless JXL"
        );
        return Ok(lossless);
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

/// Opaque-pixel quality floor for lossy keyed pictures (interface art and
/// minimaps). Below it the picture ships as lossless JXL instead. Scored
/// like the RLE sprite path's `member_quality` gate.
///
/// Measured on the Demo interface corpus at q80 (947 pictures): the damaged
/// pictures are exactly the 25 with a side under 8 px (8.2-19.3 dB); the
/// next picture scores 20.2 dB and is visually fine. A 20 dB floor catches
/// all 25 and the lossless set is 90 bytes SMALLER in total; a 24 dB floor
/// would pull in 88 pictures for +37.8 KB (+1.7%) without visible gain.
pub(super) const KEYED_PICTURE_MIN_PSNR_DB: f64 = 20.0;

/// PSNR over the source's opaque pixels, scored on the RGB565 values the
/// runtime sees (bit-replicated back to 8 bits), like `member_quality`.
/// `INFINITY` when every opaque pixel is exact (or there are none).
pub(super) fn keyed_opaque_psnr565(decoded: &Picture, source: &[u16]) -> Result<f64> {
    use robin_assets::rle_jxl::{CL_OPAQUE, class_of, expand565};
    let pixel_count = usize::from(decoded.width) * usize::from(decoded.height);
    anyhow::ensure!(
        pixel_count == source.len(),
        "keyed picture scored {} decoded pixels against {} source pixels",
        pixel_count,
        source.len()
    );
    let (mut sse, mut samples) = (0.0f64, 0u64);
    for (&want, got) in source.iter().zip(picture_rgb16_pixels(decoded)?) {
        if class_of(want) != CL_OPAQUE {
            continue;
        }
        let (a, b) = (expand565(want), expand565(got));
        for channel in 0..3 {
            let d = f64::from(a[channel]) - f64::from(b[channel]);
            sse += d * d;
        }
        samples += 3;
    }
    if samples == 0 || sse == 0.0 {
        return Ok(f64::INFINITY);
    }
    Ok(10.0 * (255.0f64 * 255.0 / (sse / samples as f64)).log10())
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
        JxlDecodeSpeed::DEFAULT,
    )
}

/// Encoder options that trade bytes for decoder speed. `DEFAULT` passes no
/// flags at all, so recipes that do not opt in stay byte-identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct JxlDecodeSpeed {
    /// cjxl `--faster_decoding` level; 0 = cjxl default (flag omitted).
    pub(super) faster_decoding: u8,
    /// cjxl `--epf` strength; `None` = encoder chooses (flag omitted).
    pub(super) epf: Option<u8>,
}

impl JxlDecodeSpeed {
    pub(super) const DEFAULT: Self = Self {
        faster_decoding: 0,
        epf: None,
    };
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
    decode_speed: JxlDecodeSpeed,
) -> Result<Vec<u8>> {
    let png_bytes = encode_png(width, height, pixels, color)?;

    let mut cmd = Command::new("cjxl");
    let effort = effort.to_string();
    if let Some(q) = quality {
        // `--alpha_distance=0` keeps the extra channel mathematically
        // lossless while the color channels stay lossy. cjxl 0.12 defaults
        // it to 0 already; passing it explicitly means a future default
        // change cannot silently corrupt the sprite class channel (which
        // `member_quality` would then catch as a hard error anyway).
        cmd.args(["-q", &q.to_string(), "--alpha_distance=0", "-e", &effort]);
    } else {
        cmd.args(["-d", "0", "--modular=1", "-e", &effort]);
    }
    if decode_speed.faster_decoding > 0 {
        cmd.arg(format!(
            "--faster_decoding={}",
            decode_speed.faster_decoding
        ));
    }
    if let Some(epf) = decode_speed.epf {
        cmd.arg(format!("--epf={epf}"));
    }
    // Input and output last: PNG on stdin, JXL on stdout.
    cmd.args(["-", "-"]);
    let out = run_with_stdin(
        cmd,
        &png_bytes,
        "cjxl",
        "spawn cjxl (is it installed?)".to_owned(),
    )?;
    Ok(out.stdout)
}

/// Encode raw pixel data to AVIF with the pinned `avifenc` (libavif 1.4.2 on
/// libaom 3.15.0; see `scripts/install_pinned_avif_tools.sh`). PNG goes in
/// on stdin; avifenc insists on an output file, so it writes into a private
/// temporary directory.
///
/// Recipe (docs measurements): colour quality `quality`, 4:4:4, speed 2,
/// libavif's default still-image tuning (tune is deliberately not passed),
/// CICP 1/13/6 full range pinned explicitly (byte-identical to the current
/// defaults, but a future default change cannot alter colour signalling).
/// RGBA input always codes alpha losslessly (`--qalpha 100`): keyed images
/// carry their pixel class there.
pub(super) fn encode_pixels_to_avif(
    width: u32,
    height: u32,
    pixels: &[u8],
    color: png::ColorType,
    quality: u8,
    jobs: AvifJobs,
) -> Result<Vec<u8>> {
    let has_alpha = match color {
        png::ColorType::Rgba => true,
        png::ColorType::Rgb => false,
        other => bail!("AVIF encode supports RGB or RGBA input, not {other:?}"),
    };
    anyhow::ensure!(quality <= 100, "AVIF quality {quality} is above 100");
    let png_bytes = encode_png(width, height, pixels, color)?;
    let directory = tempfile::tempdir().context("create avifenc output directory")?;
    let output = directory.path().join("image.avif");

    let mut cmd = Command::new("avifenc");
    cmd.args([
        "--stdin",
        "--input-format",
        "png",
        "-q",
        &quality.to_string(),
        "-y",
        "444",
        "-s",
        AVIF_ENCODER_SPEED,
        "-j",
        match jobs {
            AvifJobs::Single => "1",
            AvifJobs::All => "all",
        },
        "--cicp",
        "1/13/6",
        "--range",
        "full",
    ]);
    if has_alpha {
        cmd.args(["--qalpha", "100"]);
    }
    cmd.arg(&output);
    run_with_stdin(
        cmd,
        &png_bytes,
        "avifenc",
        "spawn avifenc: the pinned libavif 1.4.2 / libaom 3.15.0 avifenc must be on PATH \
         (build it with scripts/install_pinned_avif_tools.sh)"
            .to_owned(),
    )?;
    let encoded =
        fs::read(&output).with_context(|| format!("read avifenc output {}", output.display()))?;
    anyhow::ensure!(
        robin_assets::browser_images::is_avif(&encoded),
        "avifenc output ({} bytes) is not an AVIF file",
        encoded.len()
    );
    Ok(encoded)
}

/// Minimal 8-bit PNG of raw pixels, the encoder CLIs' input format.
fn encode_png(width: u32, height: u32, pixels: &[u8], color: png::ColorType) -> Result<Vec<u8>> {
    let mut png_bytes: Vec<u8> = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png_bytes, width, height);
        enc.set_color(color);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().context("png header")?;
        w.write_image_data(pixels).context("png data")?;
        w.finish().context("png finish")?;
    }
    Ok(png_bytes)
}

/// Run an encoder CLI with `input` on stdin, failing with its stderr on a
/// non-zero exit. stdin is fed from a scoped thread while the parent drains
/// stdout/stderr, so an input larger than the pipe buffer cannot deadlock.
fn run_with_stdin(
    mut cmd: Command,
    input: &[u8],
    tool: &str,
    spawn_context: String,
) -> Result<std::process::Output> {
    use std::io::Write as _;
    use std::process::Stdio;

    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context(spawn_context)?;
    let mut stdin = child
        .stdin
        .take()
        .with_context(|| format!("{tool} stdin was requested piped"))?;
    let (out, write_result) = std::thread::scope(|scope| {
        let writer = scope.spawn(move || {
            let result = stdin.write_all(input);
            // Explicit drop closes the pipe so the encoder sees EOF.
            drop(stdin);
            result
        });
        // `wait_with_output` drains stdout and stderr concurrently while
        // the writer thread feeds stdin.
        let out = child
            .wait_with_output()
            .with_context(|| format!("{tool} wait"));
        let write_result = writer.join().expect("encoder stdin writer thread panicked");
        (out, write_result)
    });
    let out = out?;
    if !out.status.success() {
        bail!(
            "{tool} failed: exit {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    write_result.with_context(|| format!("write PNG to {tool}"))?;
    Ok(out)
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
