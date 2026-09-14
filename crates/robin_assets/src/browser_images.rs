//! Browser-decoded web images (AVIF).
//!
//! The web recipe ships every lossy image — RLE sprite atlases, keyed
//! interface pictures, terrain maps and keyed minimaps — as AVIF, and the
//! web runtime decodes them with the browser's native AVIF decoder. That
//! decoder is asynchronous JavaScript, while the Rust decode sites
//! (resource-manager pictures, terrain loaders, sprite materialization) are
//! synchronous and partly run on worker threads that cannot await JS.
//!
//! So decoding is split in two:
//!
//! 1. The async boot and mission-install paths (in `robin_rs`) collect every
//!    AVIF blob they are about to use, have the browser decode them to
//!    straight (non-premultiplied) RGBA8, and [`insert_decoded`] the pixels.
//! 2. The synchronous consumers look the pixels up by content hash with
//!    [`decoded_rgba`]. A missing entry is a hard error naming the image —
//!    never substituted pixels.
//!
//! Exactness contract: the alpha channel of every keyed image is coded
//! losslessly, and AV1 decoding is normatively bit-exact, so the pixel CLASS
//! (transparent / shadow / opaque) the engine hashes and hit-tests against is
//! identical across browsers. Only opaque RGB is lossy; browsers may differ
//! in YUV->RGB rounding there, which affects display colour only (the
//! requantized value is key-dodged, so it can never become a key).
//!
//! The cache is a process-global map. On `wasm-threads` builds statics live
//! in shared memory, so pool workers see what the main thread inserted.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use anyhow::{Context, Result, anyhow, bail, ensure};
use sha2::{Digest, Sha256};

use crate::frame_holder::{SHADOW_KEY, TRANSPARENT_COLOR_16};
use crate::picture::{Picture, PixelFormat};

/// Straight RGBA8 pixels of one decoded image, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedRgba {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Lifetime class of a cache entry. Boot images (interface pictures, pak
/// pictures) are used for the whole session; mission images are dropped
/// when the next mission's images are predecoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageScope {
    Boot,
    Mission,
}

pub type ImageKey = [u8; 32];

type Cache = HashMap<ImageKey, (ImageScope, Arc<DecodedRgba>)>;

static CACHE: LazyLock<RwLock<Cache>> = LazyLock::new(|| RwLock::new(HashMap::new()));

/// Content key of an encoded image.
pub fn image_key(bytes: &[u8]) -> ImageKey {
    Sha256::digest(bytes).into()
}

/// Container facts of an AVIF image, read without decoding pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvifInfo {
    pub width: u16,
    pub height: u16,
    pub has_alpha: bool,
}

/// True for an ISOBMFF file whose `ftyp` box names the `avif` brand (major
/// or compatible). Cheap: inspects only the first box.
pub fn is_avif(bytes: &[u8]) -> bool {
    if bytes.len() < 16 || &bytes[4..8] != b"ftyp" {
        return false;
    }
    let size = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    if size < 16 || size > bytes.len() {
        return false;
    }
    // major_brand (4) + minor_version (4), then compatible brands.
    if &bytes[8..12] == b"avif" {
        return true;
    }
    bytes[16..size]
        .chunks_exact(4)
        .any(|brand| brand == b"avif")
}

/// Dimensions and alpha presence from the AVIF container and the primary
/// item's AV1 sequence header.
pub fn avif_info(bytes: &[u8]) -> Result<AvifInfo> {
    ensure!(is_avif(bytes), "not an AVIF image");
    let data = avif_parse::read_avif(&mut &bytes[..])
        .map_err(|error| anyhow!("parse AVIF container: {error:?}"))?;
    let primary = data
        .primary_item_metadata()
        .map_err(|error| anyhow!("parse AVIF sequence header: {error:?}"))?;
    ensure!(
        !data.premultiplied_alpha,
        "AVIF image signals premultiplied alpha; pixel classes must be straight alpha"
    );
    Ok(AvifInfo {
        width: u16::try_from(primary.max_frame_width.get())
            .context("AVIF image width exceeds u16")?,
        height: u16::try_from(primary.max_frame_height.get())
            .context("AVIF image height exceeds u16")?,
        has_alpha: data.alpha_item.is_some(),
    })
}

/// Decode an AVIF to straight RGBA8 natively (desktop, Android, converter
/// quality gates). Produces the same pixels the browser path records: AV1
/// decode is normatively exact, alpha is copied straight, and colour uses
/// libavif/libyuv's full-range BT.601 fixed-point conversion.
#[cfg(not(target_arch = "wasm32"))]
pub fn decode_avif_rgba8(bytes: &[u8]) -> Result<DecodedRgba> {
    native_av1::decode_avif_rgba8(bytes)
}

/// rav1d-backed AVIF decode (native targets).
///
/// TODO: rav1d 1.1.0 panics on some corrupt AV1 tile data (observed:
/// `Option::unwrap()` on `None` at `src/decode.rs:4997` for a flipped final
/// payload byte). The panic happens inside its `extern "C"` API, so it aborts
/// the process and cannot be caught with `catch_unwind`. Converter output
/// and sha256-verified datadirs are unaffected, but a hostile or corrupt
/// AVIF in a native mod could crash a native client. Fix options: report
/// upstream and pin a fixed release, call a non-FFI entry point if rav1d
/// exposes one, or decode untrusted mod images in a child process.
#[cfg(not(target_arch = "wasm32"))]
mod native_av1 {
    use std::ffi::c_void;
    use std::ptr::NonNull;

    use anyhow::{Context, Result, anyhow, bail, ensure};
    use rav1d::include::dav1d::data::Dav1dData;
    use rav1d::include::dav1d::dav1d::{Dav1dContext, Dav1dSettings};
    use rav1d::include::dav1d::headers::{
        DAV1D_MC_BT470BG, DAV1D_MC_BT601, DAV1D_MC_IDENTITY, DAV1D_MC_UNKNOWN,
        DAV1D_PIXEL_LAYOUT_I400, DAV1D_PIXEL_LAYOUT_I444,
    };
    use rav1d::include::dav1d::picture::Dav1dPicture;

    use super::DecodedRgba;

    /// One decoded 8-bit AV1 frame: plane 0 always, planes 1/2 for 4:4:4.
    struct Frame {
        width: usize,
        height: usize,
        layout: u32,
        /// AV1 sequence header `matrix_coefficients` (CICP).
        matrix: u32,
        /// AV1 sequence header `color_range`: 1 = full, 0 = limited.
        full_range: bool,
        planes: [Vec<u8>; 3],
    }

    /// dav1d's EAGAIN (`-EAGAIN` on every supported libc).
    fn is_again(result: i32) -> bool {
        result == -(libc_eagain())
    }

    const fn libc_eagain() -> i32 {
        // EAGAIN is 11 on Linux/Android and on MinGW's errno table.
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        {
            35
        }
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        {
            11
        }
    }

    /// Owns a dav1d context for exactly one still image.
    struct Decoder(Option<Dav1dContext>);

    impl Decoder {
        fn open() -> Result<Self> {
            let mut settings = std::mem::MaybeUninit::<Dav1dSettings>::uninit();
            // SAFETY: `settings` is valid to write to.
            unsafe {
                rav1d::src::lib::dav1d_default_settings(
                    NonNull::new(settings.as_mut_ptr()).expect("stack pointer is non-null"),
                )
            };
            // SAFETY: `dav1d_default_settings` fully initialized it.
            let mut settings = unsafe { settings.assume_init() };
            // A single still frame: no frame threading, deterministic output.
            settings.n_threads = 1;
            settings.max_frame_delay = 1;
            settings.apply_grain = 0;
            settings.all_layers = 0;
            let mut context: Option<Dav1dContext> = None;
            // SAFETY: both pointers are valid for the call.
            let result = unsafe {
                rav1d::src::lib::dav1d_open(
                    NonNull::new(&mut context as *mut _),
                    NonNull::new(&mut settings as *mut _),
                )
            };
            ensure!(result.0 == 0, "dav1d_open failed ({})", result.0);
            ensure!(context.is_some(), "dav1d_open returned no context");
            Ok(Self(context))
        }

        fn decode(&mut self, obus: &[u8]) -> Result<Frame> {
            let context = self.0.clone();
            let mut data = Dav1dData::default();
            // SAFETY: `obus` outlives the decode (the data is fully consumed
            // and unreferenced before this function returns) and no free
            // callback is needed for borrowed bytes.
            let result = unsafe {
                rav1d::src::lib::dav1d_data_wrap(
                    NonNull::new(&mut data as *mut _),
                    NonNull::new(obus.as_ptr().cast_mut()),
                    obus.len(),
                    Some(noop_free),
                    None,
                )
            };
            ensure!(result.0 == 0, "dav1d_data_wrap failed ({})", result.0);
            let mut picture = Dav1dPicture::default();
            let mut got_picture = false;
            let outcome = (|| -> Result<Frame> {
                loop {
                    if data.sz > 0 {
                        // SAFETY: context is open; `data` is a valid wrapped buffer.
                        let sent = unsafe {
                            rav1d::src::lib::dav1d_send_data(
                                context.clone(),
                                NonNull::new(&mut data as *mut _),
                            )
                        };
                        if sent.0 != 0 && !is_again(sent.0) {
                            bail!("dav1d_send_data failed ({})", sent.0);
                        }
                    }
                    // SAFETY: context is open; `picture` is valid to write to.
                    let got = unsafe {
                        rav1d::src::lib::dav1d_get_picture(
                            context.clone(),
                            NonNull::new(&mut picture as *mut _),
                        )
                    };
                    if got.0 == 0 {
                        got_picture = true;
                        return copy_frame(&picture);
                    }
                    if !is_again(got.0) {
                        bail!("dav1d_get_picture failed ({})", got.0);
                    }
                    if data.sz == 0 {
                        bail!("AV1 item ended without producing a frame");
                    }
                }
            })();
            if got_picture {
                // SAFETY: `picture` was produced by dav1d_get_picture.
                unsafe {
                    rav1d::src::lib::dav1d_picture_unref(NonNull::new(&mut picture as *mut _))
                };
            }
            if data.sz > 0 {
                // SAFETY: `data` is a wrapped buffer not yet fully consumed.
                unsafe { rav1d::src::lib::dav1d_data_unref(NonNull::new(&mut data as *mut _)) };
            }
            outcome
        }
    }

    impl Drop for Decoder {
        fn drop(&mut self) {
            if self.0.is_some() {
                // SAFETY: the context came from dav1d_open and is closed once.
                unsafe { rav1d::src::lib::dav1d_close(NonNull::new(&mut self.0 as *mut _)) };
            }
        }
    }

    unsafe extern "C" fn noop_free(
        _data: *const u8,
        _user_data: Option<rav1d::src::send_sync_non_null::SendSyncNonNull<c_void>>,
    ) {
    }

    fn copy_frame(picture: &Dav1dPicture) -> Result<Frame> {
        ensure!(
            picture.p.bpc == 8,
            "AVIF image is {}-bit; only 8-bit is supported",
            picture.p.bpc
        );
        let width = usize::try_from(picture.p.w).context("negative AV1 frame width")?;
        let height = usize::try_from(picture.p.h).context("negative AV1 frame height")?;
        ensure!(width > 0 && height > 0, "AV1 frame has zero dimensions");
        let layout = picture.p.layout;
        let sequence = picture
            .seq_hdr
            .ok_or_else(|| anyhow!("AV1 frame has no sequence header"))?;
        // SAFETY: dav1d keeps the sequence header alive with the picture,
        // which the caller unrefs only after copy_frame returns.
        let sequence = unsafe { sequence.as_ref() };
        let (matrix, full_range) = (sequence.mtrx, sequence.color_range != 0);
        let plane_count = match layout {
            DAV1D_PIXEL_LAYOUT_I400 => 1,
            DAV1D_PIXEL_LAYOUT_I444 => 3,
            other => bail!(
                "AVIF image uses chroma layout {other}; only 4:4:4 colour and 4:0:0 alpha are supported"
            ),
        };
        let mut planes: [Vec<u8>; 3] = Default::default();
        for (index, plane) in planes.iter_mut().enumerate().take(plane_count) {
            let stride = usize::try_from(picture.stride[usize::from(index > 0)])
                .context("negative AV1 plane stride")?;
            ensure!(
                stride >= width,
                "AV1 plane stride {stride} is below width {width}"
            );
            let base = picture.data[index]
                .ok_or_else(|| anyhow!("AV1 frame is missing plane {index}"))?
                .as_ptr()
                .cast::<u8>();
            plane.reserve_exact(width * height);
            for row in 0..height {
                // SAFETY: dav1d guarantees `stride * height` addressable bytes
                // per plane for a 4:4:4/4:0:0 8-bit picture it returned.
                let line = unsafe { std::slice::from_raw_parts(base.add(row * stride), width) };
                plane.extend_from_slice(line);
            }
        }
        Ok(Frame {
            width,
            height,
            layout,
            matrix,
            full_range,
            planes,
        })
    }

    /// YUV -> RGB the way libavif converts for our encodes: the identity
    /// matrix (lossless AVIF codes RGB as GBR) is a plane copy; full-range
    /// BT.601-family matrices use libyuv's JPEG constants. Anything else is
    /// refused rather than converted with guessed coefficients.
    fn color_converter(frame: &Frame) -> Result<fn(u8, u8, u8) -> [u8; 3]> {
        match (frame.matrix, frame.full_range) {
            (DAV1D_MC_IDENTITY, _) => Ok(|y, u, v| [v, y, u]),
            (DAV1D_MC_BT601 | DAV1D_MC_BT470BG | DAV1D_MC_UNKNOWN, true) => Ok(yuv_jpeg_to_rgb),
            (matrix, full_range) => bail!(
                "AVIF colour uses matrix coefficients {matrix} with {} range; only identity and \
                 full-range BT.601 are supported",
                if full_range { "full" } else { "limited" }
            ),
        }
    }

    /// libyuv `YuvPixel` with `kYuvJPEGConstants` (full-range BT.601),
    /// the conversion libavif applies for matrix coefficients 6 / full range
    /// — so native output equals `avifdec` and the browsers byte for byte.
    #[inline]
    fn yuv_jpeg_to_rgb(y: u8, u: u8, v: u8) -> [u8; 3] {
        const UB: i32 = 113;
        const UG: i32 = 22;
        const VG: i32 = 46;
        const VR: i32 = 90;
        const YG: u32 = 16320;
        const YB: i32 = 32;
        let y1 = ((u32::from(y) * 0x0101 * YG) >> 16) as i32 + YB;
        let ui = i32::from(u) - 128;
        let vi = i32::from(v) - 128;
        let clamp = |value: i32| (value >> 6).clamp(0, 255) as u8;
        [
            clamp(y1 + vi * VR),
            clamp(y1 - (ui * UG + vi * VG)),
            clamp(y1 + ui * UB),
        ]
    }

    pub(super) fn decode_avif_rgba8(bytes: &[u8]) -> Result<DecodedRgba> {
        let data = avif_parse::read_avif(&mut &bytes[..])
            .map_err(|error| anyhow!("parse AVIF container: {error:?}"))?;
        ensure!(
            !data.premultiplied_alpha,
            "AVIF image signals premultiplied alpha; pixel classes must be straight alpha"
        );
        let mut decoder = Decoder::open()?;
        let color = decoder
            .decode(&data.primary_item)
            .context("decode AVIF colour item")?;
        ensure!(
            color.layout == DAV1D_PIXEL_LAYOUT_I444,
            "AVIF colour item must be 4:4:4"
        );
        let alpha = match data.alpha_item.as_deref() {
            Some(obus) => {
                // A fresh context: the alpha item is an independent sequence.
                let mut alpha_decoder = Decoder::open()?;
                let alpha = alpha_decoder
                    .decode(obus)
                    .context("decode AVIF alpha item")?;
                ensure!(
                    (alpha.width, alpha.height) == (color.width, color.height),
                    "AVIF alpha item is {}x{} but colour is {}x{}",
                    alpha.width,
                    alpha.height,
                    color.width,
                    color.height
                );
                Some(alpha)
            }
            None => None,
        };
        let convert = color_converter(&color)?;
        let pixels = color.width * color.height;
        let mut rgba = Vec::with_capacity(pixels * 4);
        let [y_plane, u_plane, v_plane] = &color.planes;
        for index in 0..pixels {
            let [r, g, b] = convert(y_plane[index], u_plane[index], v_plane[index]);
            let a = alpha.as_ref().map_or(255, |alpha| alpha.planes[0][index]);
            rgba.extend_from_slice(&[r, g, b, a]);
        }
        Ok(DecodedRgba {
            width: u32::try_from(color.width).context("AVIF width exceeds u32")?,
            height: u32::try_from(color.height).context("AVIF height exceeds u32")?,
            rgba,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::yuv_jpeg_to_rgb;

        #[test]
        fn jpeg_constants_reproduce_reference_points() {
            // Neutral chroma maps grey to grey (libyuv rounds 255 to 255).
            for y in [0u8, 16, 128, 235, 255] {
                let [r, g, b] = yuv_jpeg_to_rgb(y, 128, 128);
                assert_eq!((r, g), (g, b));
                assert!((i32::from(r) - i32::from(y)).abs() <= 1, "y {y} -> {r}");
            }
            assert_eq!(yuv_jpeg_to_rgb(0, 128, 128), [0, 0, 0]);
            assert_eq!(yuv_jpeg_to_rgb(255, 128, 128), [255, 255, 255]);
        }
    }
}

/// Record the browser's decode of `bytes`. The pixel buffer must match the
/// container dimensions exactly.
pub fn insert_decoded(bytes: &[u8], scope: ImageScope, decoded: DecodedRgba) -> Result<()> {
    let info = avif_info(bytes)?;
    ensure!(
        (decoded.width, decoded.height) == (u32::from(info.width), u32::from(info.height)),
        "decoded AVIF is {}x{} but its container says {}x{}",
        decoded.width,
        decoded.height,
        info.width,
        info.height
    );
    let expected = decoded.width as usize * decoded.height as usize * 4;
    ensure!(
        decoded.rgba.len() == expected,
        "decoded AVIF {}x{} carries {} RGBA bytes, expected {expected}",
        decoded.width,
        decoded.height,
        decoded.rgba.len()
    );
    let mut cache = CACHE
        .write()
        .map_err(|_| anyhow!("decoded image cache lock poisoned"))?;
    let entry = cache
        .entry(image_key(bytes))
        .or_insert_with(|| (scope, Arc::new(decoded)));
    // A boot image reused by a mission must survive mission eviction.
    if scope == ImageScope::Boot {
        entry.0 = ImageScope::Boot;
    }
    Ok(())
}

/// Whether `bytes` already has decoded pixels (lets callers skip
/// re-decoding shared images).
pub fn is_decoded(bytes: &[u8]) -> Result<bool> {
    Ok(CACHE
        .read()
        .map_err(|_| anyhow!("decoded image cache lock poisoned"))?
        .contains_key(&image_key(bytes)))
}

/// The decoded pixels of `bytes`: the browser's predecode on the web (a
/// missing entry is a hard error), or a native rav1d decode elsewhere when
/// nothing was predecoded (native results are not cached; native only
/// decodes the occasional mod terrain map, converter gates and tests).
pub fn decoded_rgba(bytes: &[u8]) -> Result<Arc<DecodedRgba>> {
    let key = image_key(bytes);
    if let Some((_, decoded)) = CACHE
        .read()
        .map_err(|_| anyhow!("decoded image cache lock poisoned"))?
        .get(&key)
    {
        return Ok(Arc::clone(decoded));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let decoded = decode_avif_rgba8(bytes).with_context(|| {
            format!(
                "native AVIF decode ({} bytes, sha256 {})",
                bytes.len(),
                hex(&key)
            )
        })?;
        Ok(Arc::new(decoded))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let dims = avif_info(bytes)
            .map(|info| format!("{}x{}", info.width, info.height))
            .unwrap_or_else(|error| format!("unparseable: {error:#}"));
        bail!(
            "AVIF image ({} bytes, {dims}, sha256 {}) was not decoded by the browser before use; \
             every AVIF blob must go through the async predecode step first",
            bytes.len(),
            hex(&key)
        );
    }
}

/// Drop every entry of `scope`.
pub fn clear_scope(scope: ImageScope) -> Result<()> {
    CACHE
        .write()
        .map_err(|_| anyhow!("decoded image cache lock poisoned"))?
        .retain(|_, (entry_scope, _)| *entry_scope != scope);
    Ok(())
}

/// Keep only the mission-scoped entries whose encoded bytes are in `keep`
/// (boot entries are untouched). Called when a mission install is
/// published: its sprite atlases are materialized by then, and every older
/// mission payload has been released, so only the installed mission's
/// still-encoded images (terrain maps, minimaps) are needed later.
pub fn retain_mission_images(keep: &[&[u8]]) -> Result<()> {
    let keep: std::collections::HashSet<ImageKey> =
        keep.iter().map(|bytes| image_key(bytes)).collect();
    CACHE
        .write()
        .map_err(|_| anyhow!("decoded image cache lock poisoned"))?
        .retain(|key, (scope, _)| *scope == ImageScope::Boot || keep.contains(key));
    Ok(())
}

fn hex(key: &ImageKey) -> String {
    key.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn rgb565_picture(width: u32, height: u32, data: Vec<u8>) -> Result<Picture> {
    let width = u16::try_from(width).context("decoded picture width exceeds u16")?;
    let height = u16::try_from(height).context("decoded picture height exceeds u16")?;
    ensure!(
        width > 0 && height > 0,
        "decoded picture has zero dimensions"
    );
    let pitch = width
        .checked_mul(2)
        .context("decoded picture row exceeds u16 pitch")?;
    Ok(Picture {
        width,
        height,
        pitch,
        pixel_format: PixelFormat::Rgb16,
        data,
        palette: None,
    })
}

/// Keyed RGBA -> RGB565 picture: alpha carries the pixel CLASS (coded
/// losslessly), colour is requantized for opaque pixels only. The bands and
/// the key-collision nudge are the keyed JPEG XL decoder's, so both formats
/// reconstruct identical classes.
pub fn rgba_to_rgb565_keyed(decoded: &DecodedRgba) -> Result<Picture> {
    let mut data = Vec::with_capacity(decoded.rgba.len() / 2);
    for px in decoded.rgba.as_chunks::<4>().0 {
        let value = match px[3] {
            a if a < 64 => TRANSPARENT_COLOR_16,
            a if a < 192 => SHADOW_KEY,
            _ => match robin_util::color::rgb565(px[0], px[1], px[2]) {
                TRANSPARENT_COLOR_16 => TRANSPARENT_COLOR_16 + 1,
                SHADOW_KEY => SHADOW_KEY - 1,
                other => other,
            },
        };
        data.extend_from_slice(&value.to_le_bytes());
    }
    rgb565_picture(decoded.width, decoded.height, data)
}

/// Opaque RGBA -> RGB565 picture (terrain maps carry no alpha item; the
/// browser reports alpha 255 everywhere, which is not consulted).
pub fn rgba_to_rgb565_opaque(decoded: &DecodedRgba) -> Result<Picture> {
    let mut data = Vec::with_capacity(decoded.rgba.len() / 2);
    for px in decoded.rgba.as_chunks::<4>().0 {
        data.extend_from_slice(&robin_util::color::rgb565(px[0], px[1], px[2]).to_le_bytes());
    }
    rgb565_picture(decoded.width, decoded.height, data)
}

/// Decode a keyed AVIF (interface picture or minimap) from its predecoded
/// pixels.
pub fn load_avif_rgb565_keyed(bytes: &[u8]) -> Result<Picture> {
    let info = avif_info(bytes)?;
    if !info.has_alpha {
        bail!("keyed AVIF picture has no alpha item to carry pixel classes");
    }
    let decoded = decoded_rgba(bytes)?;
    rgba_to_rgb565_keyed(&decoded)
}

/// Decode an opaque AVIF terrain map from its predecoded pixels.
pub fn load_avif_rgb565_opaque(bytes: &[u8]) -> Result<Picture> {
    let info = avif_info(bytes)?;
    if info.has_alpha {
        bail!("terrain AVIF carries an alpha item; keyed images must use the keyed decoder");
    }
    let decoded = decoded_rgba(bytes)?;
    rgba_to_rgb565_opaque(&decoded)
}

#[cfg(test)]
#[path = "browser_images_tests.rs"]
mod avif_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyed_bands_match_the_class_markers_and_dodge_keys() {
        let decoded = DecodedRgba {
            width: 4,
            height: 1,
            rgba: vec![
                1, 2, 3, 0, // transparent
                9, 9, 9, 128, // shadow
                0, 0xFF, 0, 255, // pure green -> would be 0x07E0, not a key
                0, 0xF8, 0, 255, // requantizes to 0x07C0 = transparent key -> dodged
            ],
        };
        let picture = rgba_to_rgb565_keyed(&decoded).unwrap();
        let words: Vec<u16> = picture
            .data
            .chunks_exact(2)
            .map(|w| u16::from_le_bytes([w[0], w[1]]))
            .collect();
        assert_eq!(words[0], TRANSPARENT_COLOR_16);
        assert_eq!(words[1], SHADOW_KEY);
        assert_eq!(words[2], 0x07E0);
        assert_eq!(words[3], TRANSPARENT_COLOR_16 + 1);
        assert_eq!((picture.width, picture.height, picture.pitch), (4, 1, 8));
    }

    #[test]
    fn avif_signature_requires_the_avif_brand() {
        assert!(!is_avif(b""));
        assert!(!is_avif(b"\xff\x0a not avif at all"));
        let mut ftyp = Vec::new();
        ftyp.extend_from_slice(&20u32.to_be_bytes());
        ftyp.extend_from_slice(b"ftypmif1\0\0\0\0mif1");
        assert!(!is_avif(&ftyp));
        ftyp[16..20].copy_from_slice(b"avif");
        assert!(is_avif(&ftyp));
    }
}
