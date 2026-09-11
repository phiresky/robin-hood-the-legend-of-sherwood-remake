//! Image/texture loading and pixel format handling.
//!
//! [`Picture`] holds pixel data and provides format conversions. Loaders
//! support legacy Sixteen RGB565 pictures and JPEG XL terrain/interface images.
//! Stream loaders accept an already-open `SbFile`; buffered loaders accept byte
//! slices. Sixteen stream loading consumes one picture at the current position,
//! allowing multiple sequential pictures in resources and native fonts.

use std::io::Read;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::binary_reader::Reader;
use robin_data_io::sbfile::SbFile;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Pixel format for picture data.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode,
)]
pub enum PixelFormat {
    /// Sentinel for a default-constructed picture with no pixel data yet.
    /// Makes format-dependent ops on a fresh `Picture` fail explicitly
    /// rather than silently treating empty data as `Rgb16`.
    Unset,
    /// 1-bit black/white
    Bw,
    /// 8-bit indexed color
    Paletized,
    /// 15-bit RGB (5-5-5)
    Rgb15,
    /// 16-bit RGB (5-6-5)
    Rgb16,
    /// 24-bit RGB (8-8-8)
    Rgb24,
    /// 32-bit RGBA (8-8-8-8)
    Rgb32,
}

impl PixelFormat {
    /// Bits per pixel for this format. Panics on `Unset` — querying the
    /// pixel size of an unloaded picture is a bug.
    pub fn bits_per_pixel(self) -> u32 {
        match self {
            Self::Unset => panic!("bits_per_pixel called on PixelFormat::Unset"),
            Self::Bw => 1,
            Self::Paletized => 8,
            Self::Rgb15 | Self::Rgb16 => 16,
            Self::Rgb24 => 24,
            Self::Rgb32 => 32,
        }
    }

    /// Bytes per row for a given width. Panics on `Unset` — see
    /// [`Self::bits_per_pixel`].
    pub fn bytes_per_row(self, width: u16) -> usize {
        let w = width as usize;
        match self {
            Self::Unset => panic!("bytes_per_row called on PixelFormat::Unset"),
            Self::Bw => w.div_ceil(8),
            Self::Paletized => w,
            Self::Rgb15 | Self::Rgb16 => w * 2,
            Self::Rgb24 => w * 3,
            Self::Rgb32 => w * 4,
        }
    }
}

/// Palette entry for `Paletized` format.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct RgbQuad {
    pub r: u16,
    pub g: u16,
    pub b: u16,
}

/// Compression method for the "Sixteen" format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SixteenPacking {
    None = 0,
    Zip = 1,
    Bzip = 2,
}

impl SixteenPacking {
    fn from_u32(v: u32) -> Result<Self> {
        match v {
            0 => Ok(Self::None),
            1 => Ok(Self::Zip),
            2 => Ok(Self::Bzip),
            _ => bail!("unsupported SixteenPacking value: {v}"),
        }
    }
}

// ---------------------------------------------------------------------------
// SbFile reading helpers  (pub(crate) — shared with resource_manager)
// ---------------------------------------------------------------------------

pub fn read_u16(file: &mut SbFile) -> Result<u16> {
    Ok(robin_data_io::legacy_io::LegacyReader::new(file).read_u16("picture u16")?)
}

pub fn read_u32(file: &mut SbFile) -> Result<u32> {
    Ok(robin_data_io::legacy_io::LegacyReader::new(file).read_u32("picture u32")?)
}

pub fn read_i32(file: &mut SbFile) -> Result<i32> {
    Ok(robin_data_io::legacy_io::LegacyReader::new(file).read_i32("picture i32")?)
}

pub(crate) fn read_bytes(file: &mut SbFile, len: usize) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; len];
    robin_data_io::legacy_io::LegacyReader::new(file).read_bytes("picture bytes", &mut buf)?;
    Ok(buf)
}

/// Decompress a bzip2-compressed Sixteen payload.  The legacy picture
/// format uses `SixteenPacking::Bzip`; wasm builds ship blobs
/// pre-converted by `convert_datadir` so they never see this variant,
/// and the `bzip2` dependency is scoped native-only.
#[cfg(not(target_arch = "wasm32"))]
fn decompress_sixteen_bzip(compressed: &[u8], expected: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    bzip2::read::BzDecoder::new(compressed)
        .take(expected as u64 + 1)
        .read_to_end(&mut out)
        .context("bzip2 decompression of Sixteen picture failed")?;
    Ok(out)
}

#[cfg(target_arch = "wasm32")]
fn decompress_sixteen_bzip(_compressed: &[u8], _expected: usize) -> Result<Vec<u8>> {
    anyhow::bail!(
        "Sixteen picture is bzip2-packed — legacy encoding not supported in \
         wasm builds; re-run `convert_datadir --format shipping` on the data"
    )
}

/// Compress a Sixteen payload with bzip2.  Native-only for the same
/// reason as `decompress_sixteen_bzip`.
#[cfg(not(target_arch = "wasm32"))]
fn compress_sixteen_bzip(data: &[u8]) -> Result<Vec<u8>> {
    use std::io::Write;
    let mut enc = bzip2::write::BzEncoder::new(
        Vec::with_capacity(data.len() / 2),
        bzip2::Compression::best(),
    );
    enc.write_all(data).context("bzip2 encode")?;
    enc.finish().context("bzip2 finalize")
}

#[cfg(target_arch = "wasm32")]
fn compress_sixteen_bzip(_data: &[u8]) -> Result<Vec<u8>> {
    anyhow::bail!("bzip2 encoding is not available in wasm builds")
}

/// Check whether a buffer starts with a JPEG XL magic signature
/// (either the naked codestream marker `0xFF 0x0A` or the ISOBMFF
/// container `JXL ` box header).
fn is_jxl_signature(bytes: &[u8]) -> bool {
    // Naked JXL codestream.
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0x0A {
        return true;
    }
    // ISOBMFF container: 12-byte signature box `[len=0xC] "JXL " 0x0D 0x0A 0x87 0x0A`.
    if bytes.len() >= 12 && &bytes[..12] == b"\x00\x00\x00\x0CJXL \r\n\x87\n" {
        return true;
    }
    false
}

/// Seek to an absolute byte position (SEEK_SET).
pub(crate) fn seek_to(file: &mut SbFile, pos: u64) -> Result<()> {
    let error = file.skip(pos as i64, 0); // 0 = SEEK_SET
    if error != robin_data_io::sbfile::SBFILE_NO_ERROR {
        bail!("seek to {pos}: error {error}");
    }
    Ok(())
}

/// Distributes jxl-rs section decoding across the rayon pool. jxl-rs
/// calls [`jxl::api::JxlParallelRunner::run`] with an index-addressed task
/// set (one entry per group/pass section), which maps directly onto a
/// parallel iterator.
///
/// Threading contract: the rayon join parks worker threads with
/// `atomics.wait` on wasm, so this runner must only be used from a rayon
/// worker there — never from the browser main thread (which traps on
/// `atomics.wait`). Native threads have no such restriction.
#[cfg(any(not(target_arch = "wasm32"), feature = "wasm-threads"))]
pub(crate) struct RayonJxlRunner;

#[cfg(any(not(target_arch = "wasm32"), feature = "wasm-threads"))]
impl jxl::api::JxlParallelRunner for RayonJxlRunner {
    fn num_threads(&self) -> usize {
        rayon::current_num_threads()
    }

    fn run(
        &mut self,
        num: usize,
        fun: &jxl::api::JxlParallelRunnerFun<'_>,
    ) -> std::result::Result<(), jxl::error::Error> {
        use rayon::prelude::*;
        (0..num).into_par_iter().try_for_each(fun)
    }
}

/// Uninhabited stand-in on single-threaded wasm builds so the shared JXL
/// decode body type-checks; [`rayon_jxl_runner`] never constructs it there.
#[cfg(all(target_arch = "wasm32", not(feature = "wasm-threads")))]
pub(crate) enum RayonJxlRunner {}

#[cfg(all(target_arch = "wasm32", not(feature = "wasm-threads")))]
impl jxl::api::JxlParallelRunner for RayonJxlRunner {
    fn num_threads(&self) -> usize {
        match *self {}
    }

    fn run(
        &mut self,
        _num: usize,
        _fun: &jxl::api::JxlParallelRunnerFun<'_>,
    ) -> std::result::Result<(), jxl::error::Error> {
        match *self {}
    }
}

/// Resolve the section runner for a JXL decode. `None` means decode
/// serially: parallelism not requested, or (wasm) the worker pool was never
/// initialized so rayon has no threads to run on.
pub(crate) fn rayon_jxl_runner(parallel: bool) -> Option<RayonJxlRunner> {
    if !parallel {
        return None;
    }
    #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
    if crate::wasm_threads::pool_threads() == 0 {
        return None;
    }
    #[cfg(all(target_arch = "wasm32", not(feature = "wasm-threads")))]
    return None;
    #[cfg(any(not(target_arch = "wasm32"), feature = "wasm-threads"))]
    Some(RayonJxlRunner)
}

// ---------------------------------------------------------------------------
// Picture
// ---------------------------------------------------------------------------

/// An in-memory image with raw pixel data.
#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct Picture {
    pub width: u16,
    pub height: u16,
    /// Row stride in bytes.
    pub pitch: u16,
    pub pixel_format: PixelFormat,
    /// Raw pixel data — layout depends on `pixel_format`.
    /// For 16-bit formats each pixel is two little-endian bytes.
    pub data: Vec<u8>,
    pub palette: Option<Vec<RgbQuad>>,
}

impl Default for Picture {
    /// Zero-init geometry/data plus a "no format yet" sentinel so that
    /// format-dependent ops on a fresh `Picture` fail explicitly.
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            pitch: 0,
            pixel_format: PixelFormat::Unset,
            data: Vec::new(),
            palette: None,
        }
    }
}

impl Picture {
    /// Tight bounds `(x_min, y_min, width, height)` of the non-transparent
    /// region of a 16-bit (RGB565 or RGB15) picture, using the engine's
    /// `0x07C0` color key. Returns `None` for other formats or if the image
    /// is fully transparent.
    ///
    /// This is the auto-crop step used when packing sprites: it scans for
    /// pixels `!= 0x07C0` and records the offset plus the tight
    /// `width/height` — those values feed the sprite's reported size and
    /// per-frame offset entries used by ground-mark loading and screen-clip
    /// / blit-box generation.
    pub fn opaque_bounds_16(&self) -> Option<(u16, u16, u16, u16)> {
        if !matches!(self.pixel_format, PixelFormat::Rgb16 | PixelFormat::Rgb15) {
            return None;
        }
        let w = self.width as usize;
        let h = self.height as usize;
        if self.data.len() < w * h * 2 || w == 0 || h == 0 {
            return None;
        }
        const KEY: u16 = 0x07C0;
        let mut x_min = usize::MAX;
        let mut y_min = usize::MAX;
        let mut x_max = 0usize;
        let mut y_max = 0usize;
        for y in 0..h {
            let row = y * w * 2;
            for x in 0..w {
                let off = row + x * 2;
                let px = u16::from_le_bytes([self.data[off], self.data[off + 1]]);
                if px != KEY {
                    if x < x_min {
                        x_min = x;
                    }
                    if x > x_max {
                        x_max = x;
                    }
                    if y < y_min {
                        y_min = y;
                    }
                    if y > y_max {
                        y_max = y;
                    }
                }
            }
        }
        if x_min > x_max {
            return None;
        }
        Some((
            x_min as u16,
            y_min as u16,
            (x_max - x_min + 1) as u16,
            (y_max - y_min + 1) as u16,
        ))
    }

    // =======================================================================
    // Sixteen format
    // 16-bit RGB565, optionally compressed with zlib or bzip2.
    // This is the format used inside .res resource files.
    // =======================================================================

    /// Decode a terrain bitmap (`.map` / `.min`) from a file, auto-detecting
    /// either the legacy Sixteen (bzip2-RGB565) or JPEG XL format.
    /// The shipping converter optionally transcodes maps to JXL via the
    /// `--map-format jxl-{lossless,q90}` flag; this helper makes the loader
    /// transparent to that choice.
    ///
    /// Starts at the current stream position. Sixteen consumes one picture;
    /// JPEG XL consumes the remainder of the stream.
    ///
    /// Always returns the picture in `PixelFormat::Rgb16` so downstream code
    /// (which expects RGB565 pixels for the GPU upload path) is unchanged.
    pub fn load_terrain_from_stream(file: &mut SbFile) -> Result<Self> {
        // Peek 12 bytes to identify JXL (which has a 2- or 12-byte signature),
        // then either slurp the rest and hand it to the JXL decoder, or
        // rewind and parse as the legacy Sixteen format.
        let start = file.tell();
        let mut head = [0u8; 12];
        file.serialize_bytes(&mut head)
            .map_err(|e| anyhow!("read terrain header: {e}"))?;
        if is_jxl_signature(&head) {
            let total = usize::try_from(
                file.get_size()
                    .checked_sub(start)
                    .context("terrain starts past end of file")?,
            )
            .context("terrain byte length exceeds addressable memory")?;
            let mut blob = Vec::with_capacity(total);
            blob.extend_from_slice(&head);
            blob.resize(total, 0);
            file.serialize_bytes(&mut blob[head.len()..])
                .map_err(|e| anyhow!("read terrain body: {e}"))?;
            return Self::load_jxl_rgb565(&blob);
        }
        // Legacy Sixteen format: rewind the 12 peeked bytes and parse.
        seek_to(file, start)?;
        Self::load_sixteen_from_stream(file)
    }

    /// Pixel dimensions of a terrain bitmap (`.map` / `.min`) without
    /// decoding the pixels: the Sixteen header carries them directly, and
    /// JXL exposes them after the (cheap) image-info decoder stage.
    ///
    /// Lets level setup hand `Engine::new` its grid dimensions while the
    /// full bitmap decode still runs on a worker thread.
    pub fn terrain_dimensions(bytes: &[u8]) -> Result<(u16, u16)> {
        if is_jxl_signature(bytes) {
            return Self::jxl_dimensions(bytes);
        }
        let mut reader = Reader::new(bytes);
        let x_size = reader.u16("Sixteen frame width")?;
        let y_size = reader.u16("Sixteen frame height")?;
        Ok((x_size, y_size))
    }

    /// Read JPEG XL image dimensions without decoding its frame pixels.
    /// Works for both RGB terrain and keyed RGBA interface pictures.
    pub fn jxl_dimensions(bytes: &[u8]) -> Result<(u16, u16)> {
        use jxl::api::{JxlDecoder, JxlDecoderOptions, ProcessingResult, states};

        let mut input = bytes;
        let dec = JxlDecoder::<states::Initialized>::new(JxlDecoderOptions::default());
        let dec_with_image = match dec.process(&mut input, None) {
            Ok(ProcessingResult::Complete { result }) => result,
            Ok(ProcessingResult::NeedsMoreInput { .. }) => {
                bail!("jxl: decoder requested more input but we provided the whole blob")
            }
            Err(e) => bail!("jxl: decoder error reading image info: {e:?}"),
        };
        let (w, h) = dec_with_image.basic_info().size;
        Ok((
            u16::try_from(w).context("jxl picture width exceeds u16")?,
            u16::try_from(h).context("jxl picture height exceeds u16")?,
        ))
    }

    /// Same dispatch as [`Self::load_terrain_from_stream`] but on an
    /// already-buffered byte slice — used when the bytes come from the
    /// shipping datadir's `raw` map rather than from disk.
    pub fn load_terrain_from_bytes(bytes: &[u8]) -> Result<Self> {
        if is_jxl_signature(bytes) {
            return Self::load_jxl_rgb565(bytes);
        }
        Self::load_sixteen_from_bytes(bytes)
    }

    /// [`Self::load_terrain_from_bytes`] with rayon-parallel JXL section
    /// decoding — see [`Self::load_jxl_rgb565_parallel`] for the threading
    /// contract (on wasm this must only run on a worker, never the browser
    /// main thread).
    pub fn load_terrain_from_bytes_parallel(bytes: &[u8]) -> Result<Self> {
        if is_jxl_signature(bytes) {
            return Self::load_jxl_rgb565_parallel(bytes);
        }
        // The legacy Sixteen format is one bzip2/zlib stream — nothing to
        // parallelize.
        Self::load_sixteen_from_bytes(bytes)
    }

    /// Serialize this picture in the on-disk Sixteen format.
    /// Layout: `[u16 width][u16 height][u32 packing][u32 packed_size][data…]`.
    /// Only `Rgb16` pictures are supported (matches the on-disk pixel format).
    pub fn write_sixteen_to_bytes(&self, packing: SixteenPacking) -> Result<Vec<u8>> {
        use std::io::Write;
        if self.pixel_format != PixelFormat::Rgb16 {
            bail!(
                "write_sixteen_to_bytes: pixel_format must be Rgb16, got {:?}",
                self.pixel_format
            );
        }
        let (pitch, expected) = Self::rgb565_layout(self.width, self.height)?;
        if self.pitch != pitch || self.data.len() != expected {
            bail!(
                "Sixteen source buffer is inconsistent: expected pitch {pitch} and {expected} bytes, got pitch {} and {} bytes",
                self.pitch,
                self.data.len()
            );
        }
        let payload: std::borrow::Cow<'_, [u8]> = match packing {
            SixteenPacking::None => std::borrow::Cow::Borrowed(&self.data),
            SixteenPacking::Zip => {
                // Use Z_DEFAULT_COMPRESSION (level 6) so byte-level diffing
                // against original `.res` files is closer (still
                // wire-compatible across any level).
                let mut enc = flate2::write::ZlibEncoder::new(
                    Vec::with_capacity(self.data.len() / 2),
                    flate2::Compression::default(),
                );
                enc.write_all(&self.data).context("zlib encode")?;
                std::borrow::Cow::Owned(enc.finish().context("zlib finalize")?)
            }
            SixteenPacking::Bzip => std::borrow::Cow::Owned(compress_sixteen_bzip(&self.data)?),
        };
        let mut out = Vec::with_capacity(12 + payload.len());
        out.extend_from_slice(&self.width.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.extend_from_slice(&(packing as u32).to_le_bytes());
        out.extend_from_slice(
            &u32::try_from(payload.len())
                .context("Sixteen packed payload exceeds u32")?
                .to_le_bytes(),
        );
        out.extend_from_slice(&payload);
        Ok(out)
    }

    /// Inline Sixteen-format decoder for an in-memory blob, to support
    /// the shipping `dd.raw` path without needing an `SbFile` cursor type.
    pub fn load_sixteen_from_bytes(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(bytes);
        let x_size = reader.u16("Sixteen frame width")?;
        let y_size = reader.u16("Sixteen frame height")?;
        let packing_raw = reader.u32("Sixteen frame packing")?;
        let packed_size = reader.u32("Sixteen frame packed size")? as usize;
        let packing = SixteenPacking::from_u32(packing_raw)?;
        let payload = reader.take(packed_size, "Sixteen frame payload")?;
        Self::decode_sixteen_payload(x_size, y_size, packing, payload)
    }

    fn rgb565_layout(width: u16, height: u16) -> Result<(u16, usize)> {
        let pitch = width
            .checked_mul(2)
            .context("RGB565 row pitch exceeds u16")?;
        let length = usize::from(pitch)
            .checked_mul(usize::from(height))
            .context("RGB565 pixel byte length exceeds address space")?;
        Ok((pitch, length))
    }

    fn jxl_rgb565_layout(w: usize, h: usize) -> Result<(u16, u16, u16)> {
        if w == 0 || h == 0 {
            bail!("jxl: decoded image has zero dimensions");
        }
        let width = u16::try_from(w).context("jxl picture width exceeds u16")?;
        let height = u16::try_from(h).context("jxl picture height exceeds u16")?;
        let (pitch, _) = Self::rgb565_layout(width, height)?;
        Ok((width, height, pitch))
    }

    fn decode_sixteen_payload(
        width: u16,
        height: u16,
        packing: SixteenPacking,
        payload: &[u8],
    ) -> Result<Self> {
        let (pitch, expected) = Self::rgb565_layout(width, height)?;
        let data = match packing {
            SixteenPacking::None => {
                if payload.len() != expected {
                    bail!(
                        "Sixteen pixel payload: expected {expected} bytes, got {}",
                        payload.len()
                    );
                }
                payload.to_vec()
            }
            SixteenPacking::Zip => {
                let mut out = Vec::new();
                flate2::read::ZlibDecoder::new(payload)
                    .take(expected as u64 + 1)
                    .read_to_end(&mut out)
                    .context("zlib decompression of Sixteen picture failed")?;
                out
            }
            SixteenPacking::Bzip => decompress_sixteen_bzip(payload, expected)?,
        };
        if data.len() != expected {
            bail!(
                "Sixteen pixel payload: expected {expected} bytes, got {}",
                data.len()
            );
        }
        Ok(Self {
            width,
            height,
            pitch,
            pixel_format: PixelFormat::Rgb16,
            data,
            palette: None,
        })
    }

    /// Decode a JPEG XL byte slice into an `Rgb16` `Picture`. The JXL is
    /// requested as RGB8 (no alpha — terrain bitmaps are fully opaque,
    /// and the converter is careful to write 3-channel JXL) and then the
    /// pixels are collapsed back into the engine's RGB565 representation.
    pub fn load_jxl_rgb565(bytes: &[u8]) -> Result<Self> {
        Self::load_jxl_rgb565_impl(bytes, false)
    }

    /// [`Self::load_jxl_rgb565`] with the section decode and the RGB565
    /// collapse spread across the rayon pool. Threading contract: safe from
    /// any native thread; on wasm this must only be called from a rayon
    /// worker (`wasm-threads` builds), never from the browser main thread —
    /// rayon joins park with `atomics.wait`, which the main thread forbids.
    /// Falls back to the serial decode when no pool is available.
    pub fn load_jxl_rgb565_parallel(bytes: &[u8]) -> Result<Self> {
        Self::load_jxl_rgb565_impl(bytes, true)
    }

    fn load_jxl_rgb565_impl(bytes: &[u8], parallel: bool) -> Result<Self> {
        use jxl::api::{
            JxlColorType, JxlDataFormat, JxlDecoder, JxlDecoderOptions, JxlOutputBuffer,
            JxlPixelFormat, ProcessingResult, states,
        };

        let mut input: &[u8] = bytes;
        let dec = JxlDecoder::<states::Initialized>::new(JxlDecoderOptions::default());

        let mut dec_with_image = match dec.process(&mut input, None) {
            Ok(ProcessingResult::Complete { result }) => result,
            Ok(ProcessingResult::NeedsMoreInput { .. }) => {
                bail!("jxl: decoder requested more input but we provided the whole blob")
            }
            Err(e) => bail!("jxl: decoder error reading image info: {e:?}"),
        };

        let (w, h) = dec_with_image.basic_info().size;
        let (width, height, pitch) = Self::jxl_rgb565_layout(w, h)?;

        // Maps are 3-channel by construction (see `transcode_sixteen_to_jxl`
        // in the converter — it feeds cjxl an RGB-only PNG). Anything with
        // extra channels is unexpected; bail loudly so it's noticed rather
        // than silently corrupting the pixel layout.
        let extras = dec_with_image.basic_info().extra_channels.len();
        if extras != 0 {
            bail!(
                "jxl: terrain bitmap has {} extra channels (expected 0 for RGB-only); \
                 reconvert with the current converter",
                extras
            );
        }
        dec_with_image
            .set_pixel_format(JxlPixelFormat {
                color_type: JxlColorType::Rgb,
                color_data_format: Some(JxlDataFormat::U8 { bit_depth: 8 }),
                extra_channel_format: Vec::new(),
            })
            .map_err(|error| anyhow::anyhow!("jxl: invalid output pixel format: {error:?}"))?;

        // Advance from WithImageInfo → WithFrameInfo (no buffers yet).
        let dec_with_frame = match dec_with_image.process(&mut input, None) {
            Ok(ProcessingResult::Complete { result }) => result,
            Ok(ProcessingResult::NeedsMoreInput { .. }) => {
                bail!("jxl: decoder requested more input reading frame header")
            }
            Err(e) => bail!("jxl: decoder error reading frame info: {e:?}"),
        };

        let stride = w * 3;
        let byte_length = stride
            .checked_mul(h)
            .context("jxl RGB buffer size overflow")?;
        let mut rgb = vec![0u8; byte_length];
        let mut output_bufs = vec![JxlOutputBuffer::new(&mut rgb, h, stride)];
        let mut runner = rayon_jxl_runner(parallel);
        let runner_ref = runner
            .as_mut()
            .map(|r| r as &mut dyn jxl::api::JxlParallelRunner);
        match dec_with_frame.process(&mut input, &mut output_bufs, runner_ref) {
            Ok(ProcessingResult::Complete { .. }) => {}
            Ok(ProcessingResult::NeedsMoreInput { .. }) => {
                bail!("jxl: decoder requested more input while finishing frame")
            }
            Err(e) => bail!("jxl: decoder error processing frame: {e:?}"),
        };
        drop(output_bufs);

        // Collapse RGB888 → RGB565. Chunked writes into a preallocated
        // buffer — the per-pixel `Vec` growth path was a measurable slice
        // of map decode on wasm (single-threaded, no vectorizer at -Oz).
        let pixel_count = w * h;
        let mut data = vec![0u8; pixel_count * 2];
        let collapse_row = |dst_row: &mut [u8], src_row: &[u8]| {
            for (dst, src) in dst_row
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .zip(src_row.as_chunks::<3>().0.iter())
            {
                let px = robin_util::color::rgb565(src[0], src[1], src[2]);
                dst.copy_from_slice(&px.to_le_bytes());
            }
        };
        if runner.is_some() {
            // Row-parallel collapse on the pool (same threading contract as
            // the section decode above).
            #[cfg(any(not(target_arch = "wasm32"), feature = "wasm-threads"))]
            {
                use rayon::prelude::*;
                data.par_chunks_exact_mut(w * 2)
                    .zip(rgb.par_chunks_exact(stride))
                    .for_each(|(dst_row, src_row)| collapse_row(dst_row, src_row));
            }
        } else {
            for (dst_row, src_row) in data.chunks_exact_mut(w * 2).zip(rgb.chunks_exact(stride)) {
                collapse_row(dst_row, src_row);
            }
        }

        Ok(Self {
            width,
            height,
            pitch,
            pixel_format: PixelFormat::Rgb16,
            data,
            palette: None,
        })
    }

    /// Decode a JPEG XL RGBA byte slice into an `Rgb16` `Picture`, restoring
    /// fully transparent pixels to the engine's RGB565 transparent key.
    pub fn load_jxl_rgba565_keyed(bytes: &[u8]) -> Result<Self> {
        use jxl::api::{
            JxlColorType, JxlDataFormat, JxlDecoder, JxlDecoderOptions, JxlOutputBuffer,
            JxlPixelFormat, ProcessingResult, states,
        };

        let mut input: &[u8] = bytes;
        let dec = JxlDecoder::<states::Initialized>::new(JxlDecoderOptions::default());

        let mut dec_with_image = match dec.process(&mut input, None) {
            Ok(ProcessingResult::Complete { result }) => result,
            Ok(ProcessingResult::NeedsMoreInput { .. }) => {
                bail!("jxl: decoder requested more input but we provided the whole blob")
            }
            Err(e) => bail!("jxl: decoder error reading image info: {e:?}"),
        };

        let (w, h) = dec_with_image.basic_info().size;
        let (width, height, pitch) = Self::jxl_rgb565_layout(w, h)?;
        if dec_with_image.basic_info().extra_channels.is_empty() {
            bail!("jxl: keyed interface picture has no alpha channel");
        }
        dec_with_image
            .set_pixel_format(JxlPixelFormat {
                color_type: JxlColorType::Rgba,
                color_data_format: Some(JxlDataFormat::U8 { bit_depth: 8 }),
                // Alpha is included in RGBA output, so don't request a separate
                // extra-channel buffer.
                extra_channel_format: vec![None],
            })
            .map_err(|error| anyhow::anyhow!("jxl: invalid output pixel format: {error:?}"))?;

        let dec_with_frame = match dec_with_image.process(&mut input, None) {
            Ok(ProcessingResult::Complete { result }) => result,
            Ok(ProcessingResult::NeedsMoreInput { .. }) => {
                bail!("jxl: decoder requested more input reading frame header")
            }
            Err(e) => bail!("jxl: decoder error reading frame info: {e:?}"),
        };

        let stride = w * 4;
        let byte_length = stride
            .checked_mul(h)
            .context("jxl RGBA buffer size overflow")?;
        let mut rgba = vec![0u8; byte_length];
        let mut output_bufs = vec![JxlOutputBuffer::new(&mut rgba, h, stride)];
        match dec_with_frame.process(&mut input, &mut output_bufs, None) {
            Ok(ProcessingResult::Complete { .. }) => {}
            Ok(ProcessingResult::NeedsMoreInput { .. }) => {
                bail!("jxl: decoder requested more input while finishing frame")
            }
            Err(e) => bail!("jxl: decoder error processing frame: {e:?}"),
        };
        drop(output_bufs);

        let pixel_count = w * h;
        let mut data = Vec::with_capacity(pixel_count * 2);
        for i in 0..pixel_count {
            let off = i * 4;
            // Alpha carries the pixel CLASS, not opacity: the converter
            // codes it losslessly so the game's exact key comparisons keep
            // working after a lossy color pass. Banded rather than exact so
            // artifacts produced before the shadow class existed (alpha was
            // only 0 or 255, and lossily coded) still decode correctly.
            let px = match rgba[off + 3] {
                a if a < 64 => crate::frame_holder::TRANSPARENT_COLOR_16,
                a if a < 192 => crate::frame_holder::SHADOW_KEY,
                _ => {
                    let px = robin_util::color::rgb565(rgba[off], rgba[off + 1], rgba[off + 2]);
                    // A visible pixel whose lossy colour lands exactly on a
                    // key would be read as transparent or shadow by the
                    // runtime's exact comparisons. Nudge it one step in the
                    // dominant channel — imperceptible, and it cannot
                    // collide (the same trick the original game's shadow
                    // pass uses).
                    match px {
                        crate::frame_holder::TRANSPARENT_COLOR_16 => px + 1,
                        crate::frame_holder::SHADOW_KEY => px - 1,
                        _ => px,
                    }
                }
            };
            data.extend_from_slice(&px.to_le_bytes());
        }

        Ok(Self {
            width,
            height,
            pitch,
            pixel_format: PixelFormat::Rgb16,
            data,
            palette: None,
        })
    }

    /// Load a "Sixteen" format picture from an already-open stream.
    ///
    /// Wire format: `[u16 width][u16 height][u32 packing][u32 packed_size][data…]`
    pub fn load_sixteen_from_stream(file: &mut SbFile) -> Result<Self> {
        let x_size = read_u16(file)?;
        let y_size = read_u16(file)?;
        let packing = SixteenPacking::from_u32(read_u32(file)?)?;
        let packed_size = read_u32(file)? as usize;

        let remaining = file.get_size().saturating_sub(file.tell());
        if packed_size as u64 > remaining {
            bail!("Sixteen packed payload exceeds remaining stream bytes");
        }
        let payload = read_bytes(file, packed_size)?;
        Self::decode_sixteen_payload(x_size, y_size, packing, &payload)
    }

    // =======================================================================
    // RGBA conversion
    // =======================================================================

    fn pixel_count(&self) -> usize {
        self.width as usize * self.height as usize
    }

    /// Convert pixel data to RGBA8888 (bytes: R, G, B, A).
    ///
    /// If `transparent_color` is set, pixels matching that value become fully
    /// transparent (alpha = 0). Currently supports RGB16/15/24/32 input.
    pub fn to_rgba8888(&self, transparent_color: Option<u16>) -> Vec<u8> {
        let n = self.pixel_count();
        let mut rgba = Vec::with_capacity(n * 4);

        match self.pixel_format {
            PixelFormat::Rgb16 => {
                for i in 0..n {
                    let px = u16::from_le_bytes([self.data[i * 2], self.data[i * 2 + 1]]);
                    if transparent_color == Some(px) {
                        rgba.extend_from_slice(&[0, 0, 0, 0]);
                    } else {
                        let r5 = ((px >> 11) & 0x1F) as u8;
                        let g6 = ((px >> 5) & 0x3F) as u8;
                        let b5 = (px & 0x1F) as u8;
                        // Expand 5/6-bit to 8-bit with proper rounding
                        rgba.push((r5 << 3) | (r5 >> 2));
                        rgba.push((g6 << 2) | (g6 >> 4));
                        rgba.push((b5 << 3) | (b5 >> 2));
                        rgba.push(0xFF);
                    }
                }
            }
            PixelFormat::Rgb15 => {
                for i in 0..n {
                    let px = u16::from_le_bytes([self.data[i * 2], self.data[i * 2 + 1]]);
                    if transparent_color == Some(px) {
                        rgba.extend_from_slice(&[0, 0, 0, 0]);
                    } else {
                        let r5 = ((px >> 10) & 0x1F) as u8;
                        let g5 = ((px >> 5) & 0x1F) as u8;
                        let b5 = (px & 0x1F) as u8;
                        rgba.push((r5 << 3) | (r5 >> 2));
                        rgba.push((g5 << 3) | (g5 >> 2));
                        rgba.push((b5 << 3) | (b5 >> 2));
                        rgba.push(0xFF);
                    }
                }
            }
            PixelFormat::Rgb24 => {
                for i in 0..n {
                    rgba.push(self.data[i * 3]);
                    rgba.push(self.data[i * 3 + 1]);
                    rgba.push(self.data[i * 3 + 2]);
                    rgba.push(0xFF);
                }
            }
            PixelFormat::Rgb32 => {
                rgba.extend_from_slice(&self.data);
            }
            _ => panic!(
                "to_rgba8888: unsupported pixel format {:?}",
                self.pixel_format
            ),
        }

        rgba
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_format_bpp() {
        assert_eq!(PixelFormat::Bw.bits_per_pixel(), 1);
        assert_eq!(PixelFormat::Paletized.bits_per_pixel(), 8);
        assert_eq!(PixelFormat::Rgb15.bits_per_pixel(), 16);
        assert_eq!(PixelFormat::Rgb16.bits_per_pixel(), 16);
        assert_eq!(PixelFormat::Rgb24.bits_per_pixel(), 24);
        assert_eq!(PixelFormat::Rgb32.bits_per_pixel(), 32);
    }

    #[test]
    fn sixteen_decoders_share_exact_size_and_stream_boundary_checks() {
        let picture = Picture {
            width: 2,
            height: 1,
            pitch: 4,
            pixel_format: PixelFormat::Rgb16,
            data: vec![1, 2, 3, 4],
            palette: None,
        };
        let packings = [
            SixteenPacking::None,
            SixteenPacking::Zip,
            #[cfg(not(target_arch = "wasm32"))]
            SixteenPacking::Bzip,
        ];
        for packing in packings {
            let bytes = picture.write_sixteen_to_bytes(packing).unwrap();
            assert_eq!(
                Picture::load_sixteen_from_bytes(&bytes).unwrap().data,
                picture.data
            );
            let vfs = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
            let mut stream_bytes = bytes.clone();
            stream_bytes.extend_from_slice(&42u32.to_le_bytes());
            vfs.install_preloaded_asset("sixteen-fixture.bin", stream_bytes)
                .unwrap();
            let files = robin_data_io::sbfile::SbFileSystem::new(vfs.clone());
            let mut file = files.open("sixteen-fixture.bin", 0).unwrap();
            assert_eq!(
                Picture::load_sixteen_from_stream(&mut file).unwrap().data,
                picture.data
            );
            assert_eq!(read_u32(&mut file).unwrap(), 42);

            for width in [1u16, 3, 32_768] {
                let mut invalid = bytes.clone();
                invalid[..2].copy_from_slice(&width.to_le_bytes());
                assert!(Picture::load_sixteen_from_bytes(&invalid).is_err());
                vfs.install_preloaded_asset("sixteen-fixture.bin", invalid)
                    .unwrap();
                let mut file = files.open("sixteen-fixture.bin", 0).unwrap();
                assert!(Picture::load_sixteen_from_stream(&mut file).is_err());
            }
            assert!(Picture::load_sixteen_from_bytes(&bytes[..bytes.len() - 1]).is_err());
            let mut oversized = bytes.clone();
            oversized[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
            vfs.install_preloaded_asset("sixteen-fixture.bin", oversized)
                .unwrap();
            let mut file = files.open("sixteen-fixture.bin", 0).unwrap();
            assert!(Picture::load_sixteen_from_stream(&mut file).is_err());
        }
        let mut invalid = picture.clone();
        invalid.pitch = 3;
        assert!(
            invalid
                .write_sixteen_to_bytes(SixteenPacking::None)
                .is_err()
        );
        invalid.pitch = 4;
        invalid.data.pop();
        assert!(invalid.write_sixteen_to_bytes(SixteenPacking::Zip).is_err());
    }

    #[test]
    fn terrain_stream_loading_respects_the_starting_position() {
        let picture = Picture {
            width: 2,
            height: 1,
            pitch: 4,
            pixel_format: PixelFormat::Rgb16,
            data: vec![1, 2, 3, 4],
            palette: None,
        };
        let vfs = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
        let files = robin_data_io::sbfile::SbFileSystem::new(vfs.clone());
        for packing in [SixteenPacking::None, SixteenPacking::Zip] {
            let encoded = picture.write_sixteen_to_bytes(packing).unwrap();
            for prefix_len in [0, 3, 17] {
                let mut bytes = vec![0xff; prefix_len];
                bytes.extend_from_slice(&encoded);
                bytes.extend_from_slice(&42u32.to_le_bytes());
                vfs.install_preloaded_asset("terrain-fixture.bin", bytes)
                    .unwrap();
                let mut file = files.open("terrain-fixture.bin", 0).unwrap();
                seek_to(&mut file, prefix_len as u64).unwrap();
                let decoded = Picture::load_terrain_from_stream(&mut file).unwrap();
                assert_eq!((decoded.width, decoded.height, decoded.pitch), (2, 1, 4));
                assert_eq!(decoded.data, picture.data);
                assert_eq!(read_u32(&mut file).unwrap(), 42);
            }
        }

        // An incomplete JXL image must reach the decoder, not over-read the
        // stream because the prefix was incorrectly included in its length.
        let jxl_header = b"\x00\x00\x00\x0CJXL \r\n\x87\n";
        let expected_error = Picture::load_terrain_from_bytes(jxl_header)
            .unwrap_err()
            .to_string();
        for prefix_len in [0, 3, 17] {
            let mut bytes = vec![0xff; prefix_len];
            bytes.extend_from_slice(jxl_header);
            vfs.install_preloaded_asset("terrain-fixture.bin", bytes)
                .unwrap();
            let mut file = files.open("terrain-fixture.bin", 0).unwrap();
            seek_to(&mut file, prefix_len as u64).unwrap();
            assert_eq!(
                Picture::load_terrain_from_stream(&mut file)
                    .unwrap_err()
                    .to_string(),
                expected_error
            );
            assert_eq!(file.tell(), file.get_size());
        }
    }

    #[test]
    fn jxl_rgb565_layout_rejects_unrepresentable_dimensions() {
        assert_eq!(Picture::jxl_rgb565_layout(2, 3).unwrap(), (2, 3, 4));
        assert_eq!(
            Picture::jxl_rgb565_layout(32_767, 65_535).unwrap(),
            (32_767, 65_535, 65_534)
        );
        for (width, height) in [
            (0, 1),
            (1, 0),
            (32_768, 1),
            (65_536, 1),
            (1, 65_536),
            (usize::MAX, usize::MAX),
        ] {
            assert!(
                Picture::jxl_rgb565_layout(width, height).is_err(),
                "{width}x{height} must not fit an RGB565 Picture"
            );
        }
    }

    #[test]
    fn sixteen_packing_from_u32() {
        assert_eq!(SixteenPacking::from_u32(0).unwrap(), SixteenPacking::None);
        assert_eq!(SixteenPacking::from_u32(1).unwrap(), SixteenPacking::Zip);
        assert_eq!(SixteenPacking::from_u32(2).unwrap(), SixteenPacking::Bzip);
        assert!(SixteenPacking::from_u32(99).is_err());
    }

    #[test]
    fn opaque_bounds_16_crops_transparent_borders() {
        // 6x4 RGB565 picture. The key is 0x07C0; fill a 3x2 opaque box at
        // (1,1) → (3,2) with a non-key color (0xFFFF = white).
        let w: u16 = 6;
        let h: u16 = 4;
        let key: u16 = 0x07C0;
        let ink: u16 = 0xFFFF;
        let mut data = vec![0u8; (w as usize) * (h as usize) * 2];
        for y in 0..h as usize {
            for x in 0..w as usize {
                let px = if (1..=3).contains(&x) && (1..=2).contains(&y) {
                    ink
                } else {
                    key
                };
                let off = (y * w as usize + x) * 2;
                data[off..off + 2].copy_from_slice(&px.to_le_bytes());
            }
        }
        let pic = Picture {
            width: w,
            height: h,
            pitch: w * 2,
            pixel_format: PixelFormat::Rgb16,
            data,
            palette: None,
        };
        assert_eq!(pic.opaque_bounds_16(), Some((1, 1, 3, 2)));
    }

    #[test]
    fn opaque_bounds_16_returns_none_when_fully_transparent() {
        let w: u16 = 4;
        let h: u16 = 4;
        let mut data = vec![0u8; (w as usize) * (h as usize) * 2];
        let key: u16 = 0x07C0;
        for px in data.as_chunks_mut::<2>().0 {
            px.copy_from_slice(&key.to_le_bytes());
        }
        let pic = Picture {
            width: w,
            height: h,
            pitch: w * 2,
            pixel_format: PixelFormat::Rgb16,
            data,
            palette: None,
        };
        assert_eq!(pic.opaque_bounds_16(), None);
    }

    #[test]
    fn bytes_per_row_calculations() {
        assert_eq!(PixelFormat::Bw.bytes_per_row(12), 2);
        assert_eq!(PixelFormat::Bw.bytes_per_row(16), 2);
        assert_eq!(PixelFormat::Bw.bytes_per_row(1), 1);
        assert_eq!(PixelFormat::Rgb16.bytes_per_row(10), 20);
        assert_eq!(PixelFormat::Rgb24.bytes_per_row(10), 30);
        assert_eq!(PixelFormat::Rgb32.bytes_per_row(10), 40);
    }

    #[test]
    fn to_rgba8888_rgb16_transparent() {
        let transparent: u16 = 0x07C0;
        let red = 0xF800u16; // pure red in RGB565
        let mut pic = Picture {
            width: 2,
            height: 1,
            pitch: 4,
            pixel_format: PixelFormat::Rgb16,
            data: vec![],
            palette: None,
        };
        pic.data.extend_from_slice(&transparent.to_le_bytes());
        pic.data.extend_from_slice(&red.to_le_bytes());

        let rgba = pic.to_rgba8888(Some(transparent));
        assert_eq!(rgba.len(), 8); // 2 pixels * 4 bytes

        // First pixel should be transparent
        assert_eq!(rgba[0..4], [0, 0, 0, 0]);

        // Second pixel should be opaque red
        assert_eq!(rgba[7], 0xFF); // alpha
        assert!(rgba[4] > 200); // R channel (248 expected)
        assert_eq!(rgba[5], 0); // G
        assert_eq!(rgba[6], 0); // B
    }

    #[test]
    fn to_rgba8888_rgb24() {
        let pic = Picture {
            width: 1,
            height: 1,
            pitch: 3,
            pixel_format: PixelFormat::Rgb24,
            data: vec![0x11, 0x22, 0x33],
            palette: None,
        };

        let rgba = pic.to_rgba8888(None);
        assert_eq!(rgba, vec![0x11, 0x22, 0x33, 0xFF]);
    }

    // -- Integration tests (require game data) --

    #[cfg(feature = "engine-adapters")]
    fn data_dir() -> Option<String> {
        std::env::var("ROBINHOOD_DATA_DIR").ok()
    }

    #[test]
    #[cfg(feature = "engine-adapters")]
    fn test_load_res_file() {
        let Some(dir) = data_dir() else {
            eprintln!("ROBINHOOD_DATA_DIR not set, skipping integration test");
            return;
        };

        use crate::resource_manager::ResourceManager;

        let mut mgr = ResourceManager::legacy_tool();
        let res_path = format!("{}/Data/menu.res", dir);
        mgr.attach_resource_file(&res_path)
            .expect("failed to load menu.res");

        // menu.res should contain picture resources
        // Resource ID 1 is typically the first resource
        let count = mgr.get_picture_count(1);
        assert!(count.is_ok(), "expected resource 1 to exist in menu.res");
        let count = count.unwrap();
        assert!(count > 0, "expected at least one sub-picture");

        // Verify picture has reasonable dimensions
        let pic = mgr.get_picture(1, 0).unwrap();
        assert!(
            pic.width > 0 && pic.width < 4096,
            "width {} out of range",
            pic.width
        );
        assert!(
            pic.height > 0 && pic.height < 4096,
            "height {} out of range",
            pic.height
        );
        assert_eq!(pic.pixel_format, PixelFormat::Rgb16);
        assert!(!pic.data.is_empty());
    }

    #[test]
    #[cfg(feature = "engine-adapters")]
    fn test_picture_to_rgba_from_res() {
        let Some(dir) = data_dir() else {
            return;
        };

        use crate::resource_manager::ResourceManager;

        let mut mgr = ResourceManager::legacy_tool();
        let res_path = format!("{}/Data/menu.res", dir);
        mgr.attach_resource_file(&res_path).unwrap();

        let pic = mgr.get_picture(1, 0).unwrap();
        let rgba = pic.to_rgba8888(Some(0x07C0));

        let expected_len = pic.width as usize * pic.height as usize * 4;
        assert_eq!(
            rgba.len(),
            expected_len,
            "RGBA buffer size mismatch: {} vs expected {}",
            rgba.len(),
            expected_len
        );

        // Should have some non-transparent pixels
        let non_transparent = rgba.chunks(4).filter(|px| px[3] != 0).count();
        assert!(
            non_transparent > 0,
            "picture should have at least some visible pixels"
        );
    }
}
