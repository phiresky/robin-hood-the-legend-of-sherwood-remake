//! Resource file (.res) loading and management.
//!
//! The .res format bundles pictures, strings, wave paths, and mouse-cursor
//! metadata under integer resource IDs.
//!
//! ## File format (version 1.00)
//!
//! ```text
//! [4B "SRES"] [version via SbFile] [u32 resource_count]
//! for each resource:
//!   [4B type_tag] [u32 resource_id] [type-specific payload …]
//! ```
//!
//! Type tags: `PIC `, `PICC`, `BTTN`, `TOGL`, `NPTF`, `CUR `, `TEXT`,
//!            `WAVE`, `SLID`, `RDO `.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::binary_reader::Reader;
use crate::picture::Picture;
use robin_data_io::sbfile::{SbFile, SbFileSystem};
use robin_engine::coordinates::CursorHotspot;

#[cfg(test)]
include!("resource_wire_contract.rs");

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Resource identifier (signed 32-bit; `-1` is the "no resource" sentinel).
pub type ResourceId = i32;

/// Failure classified where archive bytes are acquired or decoded, without
/// reopening a potentially changed filesystem to infer what went wrong.
#[derive(Debug)]
pub enum ResourceAttachmentError {
    Unavailable(anyhow::Error),
    Malformed(anyhow::Error),
}

impl std::fmt::Display for ResourceAttachmentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(error) => write!(formatter, "archive unavailable: {error:#}"),
            Self::Malformed(error) => write!(formatter, "malformed archive: {error:#}"),
        }
    }
}

impl std::error::Error for ResourceAttachmentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unavailable(error) | Self::Malformed(error) => Some(error.as_ref()),
        }
    }
}

fn acquire_resource_bytes(
    path: &str,
    read: impl FnOnce() -> std::result::Result<robin_util::asset_fs::AssetBytes, i32>,
) -> std::result::Result<Option<robin_util::asset_fs::AssetBytes>, ResourceAttachmentError> {
    match read() {
        Ok(bytes) => Ok(Some(bytes)),
        Err(robin_data_io::sbfile::SBFILE_ERROR_FILE_NOT_FOUND) => Ok(None),
        Err(error) => Err(ResourceAttachmentError::Unavailable(anyhow!(
            "read resource file '{path}': error {error}"
        ))),
    }
}

/// Mouse-cursor metadata stored alongside cursor picture resources.
#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct MouseEntry {
    pub hotspot: CursorHotspot,
    pub flags: u16,
    pub frame_length: u16,
}

/// Shipping-only encoded picture payload.
///
/// Runtime callers still receive decoded [`Picture`] values. The compressed
/// form is used only inside `datadir.bin` so interface `.res` images do not
/// have to ship as raw RGB565 blobs.
#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct EncodedPicture {
    pub codec: EncodedPictureCodec,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub enum EncodedPictureCodec {
    /// JPEG XL, RGB-only, decoded back to RGB565.
    JxlRgb565,
    /// JPEG XL with alpha. RGB565 transparent-key pixels are encoded as
    /// alpha=0 and restored to the key color after decode.
    JxlRgba565Keyed,
}

impl EncodedPicture {
    pub fn jxl_rgba565_keyed(bytes: Vec<u8>) -> Self {
        Self {
            codec: EncodedPictureCodec::JxlRgba565Keyed,
            bytes,
        }
    }

    /// Inspect the image header without allocating or decoding frame pixels.
    pub fn dimensions(&self) -> Result<(u16, u16)> {
        match self.codec {
            EncodedPictureCodec::JxlRgb565 | EncodedPictureCodec::JxlRgba565Keyed => {
                Picture::jxl_dimensions(&self.bytes)
            }
        }
    }

    pub fn decode(&self) -> Result<Picture> {
        match self.codec {
            EncodedPictureCodec::JxlRgb565 => Picture::load_jxl_rgb565(&self.bytes),
            EncodedPictureCodec::JxlRgba565Keyed => Picture::load_jxl_rgba565_keyed(&self.bytes),
        }
    }
}

/// Pixel-derived geometry retained for engine setup before JXL frame decode.
/// Shadows count as opaque, matching the engine's transparent-key hit test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct PictureOpacityMetadata {
    pub width: u16,
    pub height: u16,
    pub opaque_bounds: Option<(u16, u16, u16, u16)>,
    /// Present only for frames whose pixels drive engine hit testing.
    pub hit_mask: Option<Vec<bool>>,
}

impl PictureOpacityMetadata {
    fn from_picture(picture: &Picture, with_mask: bool) -> Result<Self> {
        if !matches!(
            picture.pixel_format,
            crate::picture::PixelFormat::Rgb16 | crate::picture::PixelFormat::Rgb15
        ) {
            bail!("engine picture metadata requires a 16-bit keyed picture");
        }
        let pixels = usize::from(picture.width) * usize::from(picture.height);
        if picture.data.len() != pixels * 2 {
            bail!(
                "engine picture metadata: {} bytes for {}x{} picture",
                picture.data.len(),
                picture.width,
                picture.height
            );
        }
        Ok(Self {
            width: picture.width,
            height: picture.height,
            opaque_bounds: picture.opaque_bounds_16(),
            hit_mask: with_mask.then(|| {
                picture
                    .data
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|px| {
                        u16::from_le_bytes([px[0], px[1]])
                            != crate::frame_holder::TRANSPARENT_COLOR_16
                    })
                    .collect()
            }),
        })
    }

    fn validate(&self, dimensions: (u16, u16), with_mask: bool) -> Result<()> {
        if (self.width, self.height) != dimensions {
            bail!("engine picture metadata dimensions disagree with image header");
        }
        if let Some((x, y, width, height)) = self.opaque_bounds
            && (width == 0
                || height == 0
                || u32::from(x) + u32::from(width) > u32::from(self.width)
                || u32::from(y) + u32::from(height) > u32::from(self.height))
        {
            bail!("engine picture metadata opaque bounds exceed picture dimensions");
        }
        match &self.hit_mask {
            Some(mask) if mask.len() != usize::from(self.width) * usize::from(self.height) => {
                bail!("engine picture metadata hit mask length disagrees with dimensions");
            }
            None if with_mask => bail!("engine picture metadata is missing its required hit mask"),
            _ => {}
        }
        if let Some(mask) = &self.hit_mask {
            let mut bounds: Option<(u16, u16, u16, u16)> = None;
            for (index, opaque) in mask.iter().enumerate().filter(|(_, opaque)| **opaque) {
                debug_assert!(*opaque);
                let x = (index % usize::from(self.width)) as u16;
                let y = (index / usize::from(self.width)) as u16;
                bounds = Some(match bounds {
                    Some((left, top, right, bottom)) => {
                        (left.min(x), top.min(y), right.max(x), bottom.max(y))
                    }
                    None => (x, y, x, y),
                });
            }
            let bounds = bounds
                .map(|(left, top, right, bottom)| (left, top, right - left + 1, bottom - top + 1));
            if bounds != self.opaque_bounds {
                bail!("engine picture metadata hit mask disagrees with opaque bounds");
            }
        }
        Ok(())
    }

    pub fn into_hit_mask(self) -> Result<robin_engine::minimap::HitMask> {
        let mask = self
            .hit_mask
            .ok_or_else(|| anyhow!("picture metadata has no hit mask"))?;
        robin_engine::minimap::HitMask::from_opacity(self.width, self.height, mask)
            .map_err(|error| anyhow!(error))
    }
}

fn needs_engine_picture_metadata(id: ResourceId) -> bool {
    matches!(
        id,
        robin_engine::resource_ids::RHID_GROUND_FOCUS | robin_engine::resource_ids::RHMAP_CORNER
    )
}

fn needs_engine_picture_mask(id: ResourceId, sub_id: usize) -> bool {
    id == robin_engine::resource_ids::RHMAP_CORNER && sub_id == 1
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// Bookkeeping for a resource's origin on disk, used for recovery after
/// [`ResourceManager::dismiss_resource`].
#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
struct ResourceFileEntry {
    file_path: String,
    file_offset: u64,
    resource_type: [u8; 4],
}

const RES_VERSION_100: u32 = 0x0100;

// ---------------------------------------------------------------------------
// Free reader functions — parse resource payloads from a checked byte reader
// ---------------------------------------------------------------------------

fn read_picture(reader: &mut Reader<'_>, context: &str) -> Result<Picture> {
    // The original game reads the 12-byte
    // header and then exactly `ulPackedSize` payload bytes.
    let start = reader.position();
    let header: [u8; 12] = reader
        .take(12, format!("{context} Sixteen header"))?
        .try_into()
        .expect("the checked reader returned exactly 12 bytes");
    let packed_size = u32::from_le_bytes([header[8], header[9], header[10], header[11]]) as usize;
    reader.take(packed_size, format!("{context} Sixteen payload"))?;
    let length = reader.position() - start;
    let bytes = reader.range(start, length, format!("{context} Sixteen frame"))?;
    Picture::load_original_sixteen_from_bytes(bytes)
        .with_context(|| format!("{context} Sixteen frame"))
}

/// Read a single-picture resource (`PIC `).
fn read_single_picture(reader: &mut Reader<'_>, context: &str) -> Result<Vec<Option<Picture>>> {
    let _flags = reader.u32(format!("{context} flags"))?;
    let pic = read_picture(reader, &format!("{context} picture 0"))?;
    Ok(vec![Some(pic)])
}

/// Read a picture-collection resource (`PICC`).
fn read_picture_collection(reader: &mut Reader<'_>, context: &str) -> Result<Vec<Option<Picture>>> {
    let _flags = reader.u32(format!("{context} flags"))?;
    read_picture_slots(reader, context)
}

fn read_picture_slots(reader: &mut Reader<'_>, context: &str) -> Result<Vec<Option<Picture>>> {
    let count = reader.count_u32(format!("{context} picture count"), 12)?;
    let mut pics = Vec::with_capacity(count);
    for picture_index in 0..count {
        pics.push(Some(read_picture(
            reader,
            &format!("{context} picture {picture_index}"),
        )?));
    }
    Ok(pics)
}

fn flagged_picture_count(tag: &[u8; 4]) -> Option<usize> {
    match tag {
        b"BTTN" => Some(4),
        b"TOGL" => Some(5),
        b"NPTF" | b"SLID" => Some(6),
        b"RDO " => Some(7),
        _ => None,
    }
}

/// Read a "flagged" picture resource (BTTN, TOGL, NPTF, SLID, RDO).
/// `count` is the fixed number of sub-pictures for this widget type.
/// A bitmask controls which sub-pictures are actually present in the stream.
fn read_flagged_pictures(
    reader: &mut Reader<'_>,
    count: usize,
    context: &str,
) -> Result<Vec<Option<Picture>>> {
    let _flags = reader.u32(format!("{context} flags"))?;
    let bitmask = reader.u32(format!("{context} picture bitmask"))?;
    let mut pics = Vec::with_capacity(count);
    for i in 0..count {
        if bitmask & (1 << i) != 0 {
            pics.push(Some(read_picture(
                reader,
                &format!("{context} picture {i}"),
            )?));
        } else {
            pics.push(None);
        }
    }
    Ok(pics)
}

/// Read a cursor resource (`CUR `).
fn read_cursor(
    reader: &mut Reader<'_>,
    context: &str,
) -> Result<(MouseEntry, Vec<Option<Picture>>)> {
    let _flags = reader.u32(format!("{context} flags"))?;
    let mouse_flags = reader.u16(format!("{context} mouse flags"))?;
    let x = reader.u16(format!("{context} hotspot x"))?;
    let y = reader.u16(format!("{context} hotspot y"))?;
    let frame_length = reader.u16(format!("{context} frame length"))?;
    let pics = read_picture_slots(reader, context)?;

    let entry = MouseEntry {
        hotspot: CursorHotspot::new(x as f32, y as f32),
        flags: mouse_flags,
        frame_length,
    };
    Ok((entry, pics))
}

/// Read a string-table resource (`TEXT`).
/// Strings are little-endian UTF-16 on disk; we convert to UTF-8.
fn read_string_table(reader: &mut Reader<'_>, context: &str) -> Result<Vec<String>> {
    let _flags = reader.u32(format!("{context} flags"))?;
    let count = reader.u16(format!("{context} string count"))? as usize;
    reader.validate_count(
        count,
        2,
        format!("{context} string count"),
        reader.position() - 2,
    )?;
    let mut strings = Vec::with_capacity(count);

    // The original game stores each TEXT entry as
    // a 16-bit count followed by that many 16-bit code units.
    for string_index in 0..count {
        let char_count = reader.u16(format!("{context} string {string_index} length"))? as usize;
        reader.validate_count(
            char_count,
            2,
            format!("{context} string {string_index} UTF-16 data"),
            reader.position() - 2,
        )?;
        let encoded = reader.take(
            char_count * 2,
            format!("{context} string {string_index} UTF-16 data"),
        )?;
        let code_units = encoded
            .chunks_exact(2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]));
        strings.push(
            char::decode_utf16(code_units)
                .collect::<std::result::Result<String, _>>()
                .with_context(|| format!("{context} string {string_index}: invalid UTF-16"))?,
        );
    }
    Ok(strings)
}

/// Read a wave-table resource (`WAVE`).
/// Entries are narrow (ASCII) path strings on disk.
fn read_wave_table(reader: &mut Reader<'_>, context: &str) -> Result<Vec<String>> {
    let _flags = reader.u32(format!("{context} flags"))?;
    let count = reader.u16(format!("{context} wave count"))? as usize;
    reader.validate_count(
        count,
        2,
        format!("{context} wave count"),
        reader.position() - 2,
    )?;
    let mut waves = Vec::with_capacity(count);

    for wave_index in 0..count {
        let str_size = reader.u16(format!("{context} wave {wave_index} length"))? as usize;
        let encoded = reader.take(str_size, format!("{context} wave {wave_index} path"))?;
        // Original-game wave-table loading caps the materialized path at 4096 bytes
        // while still advancing past the full declared range.
        let buf = &encoded[..str_size.min(4096)];
        if str_size > 4096 {
            tracing::warn!("read_wave_table: string size {str_size} > 4096, truncating");
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        waves.push(String::from_utf8_lossy(&buf[..end]).to_string());
    }
    Ok(waves)
}

// ---------------------------------------------------------------------------
// ResourceManager
// ---------------------------------------------------------------------------

/// Manages .res resource files: loading, caching, reference counting.
///
/// Does **not** create draw-manager surfaces; it stores decoded [`Picture`]
/// data directly.  Delayed-load resources are loaded eagerly (simplification
/// for modern HW).
/// Resource values in v16 shipping wire order. Runtime recovery policy is
/// owned separately; adding runtime bookkeeping must not add payload fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct ResourceData {
    /// Picture collections keyed by resource ID.
    pictures: HashMap<ResourceId, Vec<Option<Picture>>>,
    /// Shipping-only compressed picture collections keyed by resource ID.
    #[serde(default)]
    encoded_pictures: HashMap<ResourceId, Vec<Option<EncodedPicture>>>,
    /// Mouse cursor metadata.
    mouse_entries: HashMap<ResourceId, MouseEntry>,
    /// Wide-string tables.
    strings: HashMap<ResourceId, Vec<String>>,
    /// Wave/sound-path tables.
    waves: HashMap<ResourceId, Vec<String>>,
    /// v16: selected engine geometry, exported before encoding interface pixels.
    picture_opacity: HashMap<ResourceId, Vec<Option<PictureOpacityMetadata>>>,
}

impl ResourceData {
    /// One ID denotes one resource, including all of its derived representations.
    fn remove(&mut self, id: ResourceId) {
        self.pictures.remove(&id);
        self.encoded_pictures.remove(&id);
        self.picture_opacity.remove(&id);
        self.mouse_entries.remove(&id);
        self.strings.remove(&id);
        self.waves.remove(&id);
    }
}

/// Legacy origin/reference metadata retained for exact serialized compatibility.
/// These fields are deliberately isolated from the resident resource values.
#[derive(Debug, Clone, Default, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
struct ResourceLifetime {
    /// Legacy serialized reference counts; no live owner increments them.
    /// TODO: remove this field only with an explicit shipping-format migration.
    references: HashMap<ResourceId, u32>,
    /// On-disk locations for recovery after dismiss.
    file_entries: HashMap<ResourceId, ResourceFileEntry>,
    /// Parsed shipping resources deliberately omit their legacy archive. They
    /// must never silently attempt to recover dismissed entries from a raw
    /// `.res` file that was not shipped.
    #[serde(default)]
    recovery_disabled: bool,
}

/// Process-local identity of a resource view. Never persisted with asset data.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResourceCacheIdentity {
    source: usize,
    generation: usize,
}

impl Default for ResourceCacheIdentity {
    fn default() -> Self {
        static NEXT_SOURCE: AtomicUsize = AtomicUsize::new(1);
        let source = NEXT_SOURCE
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .expect("resource cache identity exhausted");
        Self {
            source,
            generation: 0,
        }
    }
}

#[derive(Default, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct ResourceManager {
    #[serde(flatten)]
    data: ResourceData,
    #[serde(flatten)]
    lifetime: ResourceLifetime,
    /// Runtime authority is never granted by persisted resource metadata.
    #[serde(skip)]
    #[bitcode(skip)]
    files: Option<Arc<SbFileSystem>>,
    #[serde(skip)]
    #[bitcode(skip)]
    cache_identity: ResourceCacheIdentity,
}

/// Copy resident payloads and recovery metadata while sharing the bound file
/// reader. The new owner receives a fresh cache identity so renderer uploads
/// cannot be mistaken for those belonging to the source manager.
impl Clone for ResourceManager {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            lifetime: self.lifetime.clone(),
            files: self.files.clone(),
            cache_identity: ResourceCacheIdentity::default(),
        }
    }
}

impl std::fmt::Debug for ResourceManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceManager")
            .field("data", &self.data)
            .field("lifetime", &self.lifetime)
            .field("files_bound", &self.files.is_some())
            .finish()
    }
}

impl ResourceManager {
    /// Optional picture lookup: absence (including sparse/out-of-range slots)
    /// is `Ok(None)`; recovery and decoding failures remain errors.
    pub fn find_picture(&mut self, id: ResourceId, sub_id: usize) -> Result<Option<&Picture>> {
        Ok(self
            .find_pictures(id)?
            .and_then(|pictures| pictures.get(sub_id))
            .and_then(Option::as_ref))
    }

    /// Optional collection lookup without disguising broken registered assets
    /// as absent. Non-picture resource IDs do not identify pictures.
    pub fn find_pictures(&mut self, id: ResourceId) -> Result<Option<&[Option<Picture>]>> {
        if !self.has_picture_resource(id) {
            return Ok(None);
        }
        self.ensure_pictures_loaded(id)?;
        self.data
            .pictures
            .get(&id)
            .map(|pictures| Some(pictures.as_slice()))
            .ok_or_else(|| anyhow!("registered picture resource {id} recovered without pictures"))
    }

    pub fn cache_identity(&self) -> ResourceCacheIdentity {
        self.cache_identity
    }

    fn invalidate_picture_cache(&mut self) {
        self.cache_identity.generation = self
            .cache_identity
            .generation
            .checked_add(1)
            .expect("resource cache generation exhausted");
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// Construct with a caller-owned prepared reader, retained by raw clones.
    pub fn with_files(files: Arc<SbFileSystem>) -> Self {
        Self {
            files: Some(files),
            ..Self::default()
        }
    }

    /// Explicit compatibility boundary for tools using legacy mount setup.
    pub fn legacy_tool() -> Self {
        Self::with_files(Arc::new(SbFile::snapshot_legacy_file_system()))
    }

    /// Rebind decoded resource values at an authorized host boundary.
    pub fn bind_files(&mut self, files: Arc<SbFileSystem>) {
        self.invalidate_picture_cache();
        self.files = Some(files);
    }

    fn files(&self) -> Result<&SbFileSystem> {
        self.files.as_deref().context(
            "resource manager has no bound file reader; supply explicit resource authority",
        )
    }

    /// Borrow resident asset values independently of recovery/reference state.
    pub fn data(&self) -> &ResourceData {
        &self.data
    }

    // ===================================================================
    // Loading
    // ===================================================================

    /// Load a `.res` file, preferring the shipping datadir if present.
    ///
    /// `path` is interpreted as a key into `ShippingDatadir::res_files`
    /// (relative path under `Data/`, e.g. `"Interface/DEFAULT.RES"`).
    /// Falls back to legacy disk I/O via [`Self::attach_resource_file`].
    pub fn attach_or_from_shipping(
        &mut self,
        path: &str,
        shipping: Option<&crate::shipping_datadir::ShippingDatadir>,
    ) -> Result<()> {
        self.try_attach_or_from_shipping(path, shipping)?
            .with_context(|| format!("read resource file '{path}': file not found"))
    }

    /// Attach an optional archive. Only genuine acquisition absence is `None`;
    /// unavailable readers and malformed present archives remain errors.
    pub fn try_attach_or_from_shipping(
        &mut self,
        path: &str,
        shipping: Option<&crate::shipping_datadir::ShippingDatadir>,
    ) -> std::result::Result<Option<()>, ResourceAttachmentError> {
        if let Some(dd) = shipping {
            let locale = dd.active_locale_name();
            if crate::shipping_datadir::is_locale_overlay_key(path)
                && let Some(locale) = locale.as_deref()
                && let Some(src) = dd
                    .locale_resource(locale, path)
                    .map_err(ResourceAttachmentError::Unavailable)?
            {
                let rel = crate::shipping_datadir::canonical_shipping_asset_key(path);
                tracing::info!(
                    locale,
                    "Resource file {rel}: loaded from active shipping locale"
                );
                self.extend_from(src);
                return Ok(Some(()));
            }
            if let Some(locale) = locale.as_deref()
                && crate::shipping_datadir::is_optional_english_fallback_key(path)
                && let Some(src) = dd
                    .locale_resource("en-US", path)
                    .map_err(ResourceAttachmentError::Unavailable)?
            {
                let rel = crate::shipping_datadir::canonical_shipping_asset_key(path);
                tracing::info!(
                    locale,
                    "Resource file {rel}: using optional English fallback"
                );
                self.extend_from(src);
                return Ok(Some(()));
            }
            if locale.is_some() && crate::shipping_datadir::is_required_locale_key(path) {
                // The active raw locale bundle is authoritative here. This
                // deliberately errors on an incomplete pack rather than
                // silently mixing its UI with the top-level/default language.
                return self
                    .try_attach_resource_file(path)?
                    .map(Some)
                    .ok_or_else(|| {
                        ResourceAttachmentError::Unavailable(anyhow!(
                            "required selected-locale archive '{path}': file not found"
                        ))
                    });
            }
            // Keys in shipping.res_files omit any `Data/` prefix.
            let rel = path.strip_prefix("Data/").unwrap_or(path);
            if let Some(src) = dd.res_files.get(rel) {
                tracing::info!("Resource file {rel}: loaded from shipping datadir");
                self.extend_from(src);
                return Ok(Some(()));
            }
        }
        self.try_attach_resource_file(path)
    }

    fn try_attach_resource_file(
        &mut self,
        path: &str,
    ) -> std::result::Result<Option<()>, ResourceAttachmentError> {
        let files = self.files().map_err(ResourceAttachmentError::Unavailable)?;
        let Some(bytes) = acquire_resource_bytes(path, || files.read_shared(path))? else {
            return Ok(None);
        };
        self.attach_resource_bytes(&bytes, path)
            .map(Some)
            .map_err(ResourceAttachmentError::Malformed)
    }

    /// Open a `.res` file and load all resources into memory.
    /// Parsing failure leaves the currently attached resources unchanged.
    pub fn attach_resource_file(&mut self, path: &str) -> Result<()> {
        self.try_attach_resource_file(path)?
            .with_context(|| format!("read resource file '{path}': file not found"))
    }

    fn attach_resource_bytes(&mut self, bytes: &[u8], path: &str) -> Result<()> {
        let mut reader = Reader::new(bytes);

        // Validate magic
        let magic = reader.take_array::<4>("resource file magic")?;
        if &magic != b"SRES" {
            bail!(
                "not a resource file (bad magic {:?})",
                std::str::from_utf8(&magic).unwrap_or("????")
            );
        }

        let version = reader.u32("resource file version")?;

        let mut parsed = Self::new();
        match version {
            RES_VERSION_100 => parsed.load_file_resource_v100(&mut reader, path)?,
            _ => bail!("unsupported resource file version: 0x{version:04X}"),
        }
        self.merge_resources(parsed.data, parsed.lifetime);
        Ok(())
    }

    fn load_file_resource_v100(&mut self, reader: &mut Reader<'_>, file_path: &str) -> Result<()> {
        let num_resources = reader.count_u32("resource file entry count", 8)?;

        for resource_index in 0..num_resources {
            let type_tag = reader.take_array::<4>(format!("resource {resource_index} type"))?;
            let id = reader.u32(format!("resource {resource_index} id"))? as ResourceId;
            let context = format!(
                "resource {id} ({})",
                std::str::from_utf8(&type_tag).unwrap_or("non-ASCII type")
            );

            // Record the payload start used to recover dismissed resources.
            let offset = u64::try_from(reader.position())
                .with_context(|| format!("{context}: payload offset does not fit u64"))?;
            self.load_resource_data(reader, id, &type_tag)
                .with_context(|| context.clone())?;

            self.lifetime.references.insert(id, 0);
            self.lifetime.file_entries.insert(
                id,
                ResourceFileEntry {
                    file_path: file_path.to_string(),
                    file_offset: offset,
                    resource_type: type_tag,
                },
            );
        }
        Ok(())
    }

    /// Dispatch to the right reader based on the 4-byte type tag and store
    /// the results in the appropriate map(s).
    fn load_resource_data(
        &mut self,
        reader: &mut Reader<'_>,
        id: ResourceId,
        type_tag: &[u8; 4],
    ) -> Result<()> {
        let context = format!(
            "resource {id} ({})",
            std::str::from_utf8(type_tag).unwrap_or("non-ASCII type")
        );
        match type_tag {
            b"PIC " => {
                let pics = read_single_picture(reader, &context)?;
                self.data.remove(id);
                self.data.pictures.insert(id, pics);
            }
            b"PICC" => {
                let pics = read_picture_collection(reader, &context)?;
                self.data.remove(id);
                self.data.pictures.insert(id, pics);
            }
            b"BTTN" | b"TOGL" | b"NPTF" | b"SLID" | b"RDO " => {
                let count = flagged_picture_count(type_tag).expect("matched flagged picture tag");
                let pics = read_flagged_pictures(reader, count, &context)?;
                self.data.remove(id);
                self.data.pictures.insert(id, pics);
            }
            b"CUR " => {
                let (mouse, pics) = read_cursor(reader, &context)?;
                self.data.remove(id);
                self.data.pictures.insert(id, pics);
                self.data.mouse_entries.insert(id, mouse);
            }
            b"TEXT" => {
                let strs = read_string_table(reader, &context)?;
                self.data.remove(id);
                self.data.strings.insert(id, strs);
            }
            b"WAVE" => {
                let w = read_wave_table(reader, &context)?;
                self.data.remove(id);
                self.data.waves.insert(id, w);
            }
            _ => bail!(
                "unsupported resource type: {:?}",
                std::str::from_utf8(type_tag).unwrap_or("????")
            ),
        }
        Ok(())
    }

    // ===================================================================
    // Dismiss / recover
    // ===================================================================

    /// Evict picture data for a resource from memory.  Only picture-type
    /// resources (PIC, PICC, BTTN, TOGL, NPTF) are affected.
    pub fn dismiss_resource(&mut self, id: ResourceId) {
        // `-1` is the "no resource" sentinel (the on-disk `0xFFFFFFFF`
        // round-trips to `-1` as i32). Silently no-op so callers passing
        // the sentinel don't trip the unknown-id warning below.
        if id == -1 {
            return;
        }
        let Some(entry) = self.lifetime.file_entries.get(&id) else {
            tracing::warn!("dismiss_resource: unknown id {id}");
            return;
        };
        match &entry.resource_type {
            b"PIC " | b"PICC" | b"BTTN" | b"TOGL" | b"NPTF" => {
                self.invalidate_picture_cache();
                self.data.pictures.remove(&id);
            }
            _ => {}
        }
    }

    /// Re-load a resource from disk.  Called automatically by getters when the
    /// resource has been dismissed.
    fn recover_resource(&mut self, id: ResourceId) -> Result<()> {
        if self.lifetime.recovery_disabled {
            bail!(
                "resource {id}: recovery is disabled for parsed shipping resources; keep the resource resident"
            );
        }
        let entry = self
            .lifetime
            .file_entries
            .get(&id)
            .ok_or_else(|| anyhow!("resource {id}: no file entry for recovery"))?
            .clone();

        let bytes = self
            .files()?
            .read_shared(&entry.file_path)
            .map_err(|e| anyhow!("recovery read '{}': error {e}", entry.file_path))?;
        let offset = usize::try_from(entry.file_offset)
            .context("resource recovery offset does not fit usize")?;
        let mut reader = Reader::new(&bytes);
        reader.seek(offset, format!("resource {id} recovery payload offset"))?;
        self.load_resource_data(&mut reader, id, &entry.resource_type)
    }

    fn decode_picture_slots(
        id: ResourceId,
        slots: &[Option<EncodedPicture>],
    ) -> Result<Vec<Option<Picture>>> {
        slots
            .iter()
            .enumerate()
            .map(|(sub_id, slot)| {
                slot.as_ref()
                    .map(|picture| {
                        picture
                            .decode()
                            .with_context(|| format!("resource {id}/{sub_id}: decode JXL"))
                    })
                    .transpose()
            })
            .collect()
    }

    /// Ensure a picture resource is loaded (recover if dismissed).
    fn ensure_pictures_loaded(&mut self, id: ResourceId) -> Result<()> {
        if !self.data.pictures.contains_key(&id) {
            if let Some(encoded) = self.data.encoded_pictures.get(&id) {
                let decoded = Self::decode_picture_slots(id, encoded)?;
                self.data.pictures.insert(id, decoded);
                return Ok(());
            }
            self.recover_resource(id)?;
        }
        Ok(())
    }

    /// Decode every still-encoded (JXL) picture into its runtime [`Picture`]
    /// form, spreading the per-resource decodes across the rayon pool when
    /// one is available (always on native; on wasm only under the
    /// `wasm-threads` feature with an initialized pool, and then only when
    /// called from a rayon worker — never the browser main thread).
    ///
    /// A resource whose decode fails is logged and left encoded, so the lazy
    /// [`Self::ensure_pictures_loaded`] path reports the error at first use
    /// exactly as it would have without this warm-up. Returns the number of
    /// resources decoded.
    pub fn decode_all_encoded_pictures(&mut self) -> usize {
        let todo: Vec<(ResourceId, &[Option<EncodedPicture>])> = self
            .data
            .encoded_pictures
            .iter()
            .filter(|(id, _)| !self.data.pictures.contains_key(id))
            .map(|(id, slots)| (*id, slots.as_slice()))
            .collect();
        let decode_one = |(id, slots): (ResourceId, &[Option<EncodedPicture>])| {
            match Self::decode_picture_slots(id, slots) {
                Ok(decoded) => Some((id, decoded)),
                Err(error) => {
                    tracing::warn!("eager JXL decode failed (left for the lazy path): {error:#}");
                    None
                }
            }
        };
        #[cfg(any(not(target_arch = "wasm32"), feature = "wasm-threads"))]
        let use_pool = {
            #[cfg(target_arch = "wasm32")]
            {
                crate::wasm_threads::pool_threads() > 0
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                true
            }
        };
        #[cfg(not(any(not(target_arch = "wasm32"), feature = "wasm-threads")))]
        let use_pool = false;
        let decoded: Vec<Option<(ResourceId, Vec<Option<Picture>>)>> = if use_pool {
            #[cfg(any(not(target_arch = "wasm32"), feature = "wasm-threads"))]
            {
                use rayon::prelude::*;
                todo.into_par_iter().map(decode_one).collect()
            }
            #[cfg(not(any(not(target_arch = "wasm32"), feature = "wasm-threads")))]
            unreachable!("use_pool is statically false without a rayon dependency")
        } else {
            todo.into_iter().map(decode_one).collect()
        };
        let mut count = 0;
        for (id, pictures) in decoded.into_iter().flatten() {
            self.data.pictures.insert(id, pictures);
            count += 1;
        }
        count
    }

    /// True when no resources of any type are attached — e.g. every attach
    /// failed and callers should treat the archive as unavailable.
    pub fn is_empty(&self) -> bool {
        self.data.pictures.is_empty()
            && self.data.encoded_pictures.is_empty()
            && self.data.strings.is_empty()
            && self.data.waves.is_empty()
            && self.data.mouse_entries.is_empty()
    }

    /// Ensure a string resource is loaded (recover if missing).
    fn ensure_strings_loaded(&mut self, id: ResourceId) -> Result<()> {
        if !self.data.strings.contains_key(&id) {
            self.recover_resource(id)?;
        }
        Ok(())
    }

    /// Ensure a wave resource is loaded (recover if missing).
    fn ensure_waves_loaded(&mut self, id: ResourceId) -> Result<()> {
        if !self.data.waves.contains_key(&id) {
            self.recover_resource(id)?;
        }
        Ok(())
    }

    /// Ensure mouse entry is loaded.
    fn ensure_mouse_loaded(&mut self, id: ResourceId) -> Result<()> {
        if !self.data.mouse_entries.contains_key(&id) {
            self.recover_resource(id)?;
        }
        Ok(())
    }

    // ===================================================================
    // Picture getters
    // ===================================================================

    /// Get a single sub-picture by resource ID and sub-index.
    /// Auto-recovers dismissed resources.
    pub fn get_picture(&mut self, id: ResourceId, sub_id: usize) -> Result<&Picture> {
        self.ensure_pictures_loaded(id)?;
        self.data
            .pictures
            .get(&id)
            .ok_or_else(|| anyhow!("resource {id}: not found"))?
            .get(sub_id)
            .ok_or_else(|| anyhow!("resource {id}: sub_id {sub_id} out of range"))?
            .as_ref()
            .ok_or_else(|| anyhow!("resource {id}: sub_id {sub_id} is empty (not present)"))
    }

    /// Get the full picture collection for a resource.
    pub fn get_pictures(&mut self, id: ResourceId) -> Result<&[Option<Picture>]> {
        self.ensure_pictures_loaded(id)?;
        self.data
            .pictures
            .get(&id)
            .map(|v| v.as_slice())
            .ok_or_else(|| anyhow!("resource {id}: not found"))
    }

    /// Recover a dismissed collection, leaving resident JXL payloads encoded.
    fn ensure_picture_metadata_loaded(&mut self, id: ResourceId) -> Result<()> {
        if !self.data.pictures.contains_key(&id) && !self.data.encoded_pictures.contains_key(&id) {
            self.recover_resource(id)?;
        }
        Ok(())
    }

    /// Number of slots in a collection, including missing and zero-size frames.
    /// Does not decode resident JXL pictures.
    pub fn get_picture_count(&mut self, id: ResourceId) -> Result<usize> {
        self.ensure_picture_metadata_loaded(id)?;
        self.data
            .pictures
            .get(&id)
            .map(Vec::len)
            .or_else(|| self.data.encoded_pictures.get(&id).map(Vec::len))
            .ok_or_else(|| anyhow!("resource {id}: not found"))
    }

    /// Per-slot dimensions, preserving holes, without decoding JXL frame pixels.
    /// Malformed image headers remain errors rather than becoming empty frames.
    pub fn get_picture_dimensions(&mut self, id: ResourceId) -> Result<Vec<Option<(u16, u16)>>> {
        self.picture_dimensions(id)?.collect()
    }

    /// Recover once, then borrow the winning collection without collecting or decoding pixels.
    fn picture_dimensions(
        &mut self,
        id: ResourceId,
    ) -> Result<impl Iterator<Item = Result<Option<(u16, u16)>>> + '_> {
        self.ensure_picture_metadata_loaded(id)?;
        let decoded = self.data.pictures.get(&id);
        let encoded = if decoded.is_some() {
            None
        } else {
            Some(
                self.data
                    .encoded_pictures
                    .get(&id)
                    .ok_or_else(|| anyhow!("resource {id}: not found"))?,
            )
        };
        Ok(decoded
            .into_iter()
            .flatten()
            .map(|slot| Ok(slot.as_ref().map(|pic| (pic.width, pic.height))))
            .chain(
                encoded
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .map(move |(sub_id, slot)| {
                        slot.as_ref()
                            .map(EncodedPicture::dimensions)
                            .transpose()
                            .with_context(|| format!("resource {id}/{sub_id}: picture dimensions"))
                    }),
            ))
    }

    /// Count present frames with nonzero width and height, without decoding pixels.
    pub fn get_nonempty_picture_count(&mut self, id: ResourceId) -> Result<usize> {
        let mut count = 0;
        for dimensions in self.picture_dimensions(id)? {
            if dimensions?.is_some_and(|(width, height)| width > 0 && height > 0) {
                count += 1;
            }
        }
        Ok(count)
    }

    /// Read pixel-derived engine geometry without decoding shipping JXL frames.
    /// Resident decoded pictures take precedence, including after replacement.
    pub fn get_picture_opacity_metadata(
        &mut self,
        id: ResourceId,
    ) -> Result<Vec<Option<PictureOpacityMetadata>>> {
        self.ensure_picture_metadata_loaded(id)?;
        if let Some(pictures) = self.data.pictures.get(&id) {
            return pictures
                .iter()
                .enumerate()
                .map(|(index, picture)| {
                    picture
                        .as_ref()
                        .map(|picture| {
                            PictureOpacityMetadata::from_picture(
                                picture,
                                needs_engine_picture_mask(id, index),
                            )
                        })
                        .transpose()
                })
                .collect();
        }
        let dimensions = self.get_picture_dimensions(id)?;
        let metadata =
            self.data.picture_opacity.get(&id).ok_or_else(|| {
                anyhow!("resource {id}: missing exported engine picture metadata")
            })?;
        if metadata.len() != dimensions.len() {
            bail!("resource {id}: engine picture metadata slot count mismatch");
        }
        for (index, (metadata, dimensions)) in metadata.iter().zip(dimensions).enumerate() {
            match (metadata, dimensions) {
                (Some(metadata), Some(dimensions)) => metadata
                    .validate(dimensions, needs_engine_picture_mask(id, index))
                    .with_context(|| format!("resource {id}/{index}"))?,
                (None, None) => {}
                _ => bail!("resource {id}/{index}: engine picture metadata slot presence mismatch"),
            }
        }
        Ok(metadata.clone())
    }

    /// Export only the resources whose opaque pixels affect engine setup.
    /// The offline v15 migration tool decodes only these selected images,
    /// retaining their encoded payloads without re-encoding.
    pub fn prepare_engine_picture_metadata(&mut self) -> Result<()> {
        for id in self
            .picture_resource_ids()
            .into_iter()
            .filter(|&id| needs_engine_picture_metadata(id))
        {
            let metadata = if let Some(pictures) = self.data.pictures.get(&id) {
                pictures
                    .iter()
                    .enumerate()
                    .map(|(index, picture)| {
                        picture
                            .as_ref()
                            .map(|picture| {
                                PictureOpacityMetadata::from_picture(
                                    picture,
                                    needs_engine_picture_mask(id, index),
                                )
                            })
                            .transpose()
                    })
                    .collect::<Result<Vec<_>>>()?
            } else {
                self.data.encoded_pictures[&id]
                    .iter()
                    .enumerate()
                    .map(|(index, picture)| {
                        picture
                            .as_ref()
                            .map(|picture| {
                                PictureOpacityMetadata::from_picture(
                                    &picture.decode()?,
                                    needs_engine_picture_mask(id, index),
                                )
                            })
                            .transpose()
                    })
                    .collect::<Result<Vec<_>>>()?
            };
            self.data.picture_opacity.insert(id, metadata);
        }
        Ok(())
    }

    /// Maximum (width, height) across all sub-pictures of a resource.
    /// Reads JXL image headers without decoding frame pixels.
    pub fn get_dimension(&mut self, id: ResourceId) -> Result<(u16, u16)> {
        let mut max_w: u16 = 0;
        let mut max_h: u16 = 0;
        for dimensions in self.picture_dimensions(id)? {
            if let Some((width, height)) = dimensions? {
                max_w = max_w.max(width);
                max_h = max_h.max(height);
            }
        }
        if max_w == 0 && max_h == 0 {
            bail!("resource {id}: no valid sub-pictures");
        }
        Ok((max_w, max_h))
    }

    // ===================================================================
    // String / wave getters
    // ===================================================================

    /// Get a string by resource ID and sub-index.
    pub fn get_string(&mut self, id: ResourceId, sub_id: usize) -> Result<&str> {
        let strings = self.get_strings(id)?;
        strings
            .get(sub_id)
            .map(|s| s.as_str())
            .ok_or_else(|| anyhow!("string resource {id}: sub_id {sub_id} out of range"))
    }

    /// Borrow a whole text table after resolving its source once.
    pub fn get_strings(&mut self, id: ResourceId) -> Result<&[String]> {
        self.ensure_strings_loaded(id)?;
        self.data
            .strings
            .get(&id)
            .map(Vec::as_slice)
            .ok_or_else(|| anyhow!("string resource {id}: not found"))
    }

    /// Number of strings in a string-table resource.
    pub fn get_string_count(&mut self, id: ResourceId) -> Result<usize> {
        self.get_strings(id).map(<[String]>::len)
    }

    /// Number of strings already resident in a decoded resource manager.
    ///
    /// Shipping manifests contain eagerly decoded resources and are shared by
    /// reference, so validating a locale must not require mutable access or
    /// attempt delayed disk I/O.
    pub fn resident_string_count(&self, id: ResourceId) -> Option<usize> {
        self.data.strings.get(&id).map(Vec::len)
    }

    /// Get a wave/sound path by resource ID and sub-index.
    pub fn get_sample(&mut self, id: ResourceId, sub_id: usize) -> Result<&str> {
        self.ensure_waves_loaded(id)?;
        let waves = self
            .data
            .waves
            .get(&id)
            .ok_or_else(|| anyhow!("wave resource {id}: not found"))?;
        waves
            .get(sub_id)
            .map(|s| s.as_str())
            .ok_or_else(|| anyhow!("wave resource {id}: sub_id {sub_id} out of range"))
    }

    // ===================================================================
    // Mouse-cursor getters
    // ===================================================================

    /// Get the full mouse entry for a cursor resource.
    pub fn get_mouse_entry(&mut self, id: ResourceId) -> Result<&MouseEntry> {
        self.ensure_mouse_loaded(id)?;
        self.data
            .mouse_entries
            .get(&id)
            .ok_or_else(|| anyhow!("mouse resource {id}: not found"))
    }

    // ===================================================================
    // Existence queries (non-mutating)
    // ===================================================================

    /// True if a picture (or picture-like) resource is loaded or registered.
    pub fn has_picture_resource(&self, id: ResourceId) -> bool {
        self.data.pictures.contains_key(&id)
            || self.data.encoded_pictures.contains_key(&id)
            || self.lifetime.file_entries.get(&id).is_some_and(|entry| {
                matches!(
                    &entry.resource_type,
                    b"PIC " | b"PICC" | b"BTTN" | b"TOGL" | b"NPTF" | b"CUR " | b"SLID" | b"RDO "
                )
            })
    }

    /// Whether any payload or archive entry exists for this ID.
    /// This does not validate its type or readability: callers must still use
    /// the typed getter, so wrong-type resources cannot become optional absence.
    pub fn has_resource(&self, id: ResourceId) -> bool {
        self.lifetime.file_entries.contains_key(&id)
            || self.data.pictures.contains_key(&id)
            || self.data.encoded_pictures.contains_key(&id)
            || self.data.mouse_entries.contains_key(&id)
            || self.data.strings.contains_key(&id)
            || self.data.waves.contains_key(&id)
    }

    /// Sorted IDs of resident or encoded picture collections, independent of
    /// legacy archive metadata (which shipping manifests intentionally omit).
    pub fn picture_resource_ids(&self) -> Vec<ResourceId> {
        let mut ids: Vec<_> = self
            .data
            .pictures
            .keys()
            .chain(self.data.encoded_pictures.keys())
            .copied()
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// Borrow the raw picture list for a loaded id, if any.
    pub fn pictures_raw(&self, id: ResourceId) -> Option<&Vec<Option<Picture>>> {
        self.data.pictures.get(&id)
    }

    /// Replace currently loaded picture payloads with encoded shipping
    /// payloads. Non-picture resource metadata stays intact.
    /// Each collection is replaced only after all its frames encode successfully.
    /// On error, completed collections remain encoded and the failed collection
    /// retains its decoded pictures so the operation can be retried.
    pub fn encode_pictures_for_shipping<F>(&mut self, mut encode: F) -> Result<usize>
    where
        F: FnMut(&Picture) -> Result<EncodedPicture>,
    {
        self.invalidate_picture_cache();
        self.prepare_engine_picture_metadata()?;
        let ids: Vec<ResourceId> = self.data.pictures.keys().copied().collect();
        let mut encoded_count = 0usize;
        for id in ids {
            let pictures = self
                .data
                .pictures
                .get(&id)
                .expect("collected resident picture ID");
            let mut encoded_slots = Vec::with_capacity(pictures.len());
            for (sub_id, slot) in pictures.iter().enumerate() {
                encoded_slots.push(match slot {
                    Some(pic) => {
                        encoded_count += 1;
                        Some(encode(pic).with_context(|| {
                            format!("resource {id}/{sub_id}: encode picture for shipping")
                        })?)
                    }
                    None => None,
                });
            }
            self.data.encoded_pictures.insert(id, encoded_slots);
            self.data.pictures.remove(&id);
        }
        Ok(encoded_count)
    }

    /// Borrow the string list for a loaded id, if any.
    pub fn strings_raw(&self, id: ResourceId) -> Option<&Vec<String>> {
        self.data.strings.get(&id)
    }

    /// Borrow the wave path list for a loaded id, if any.
    pub fn waves_raw(&self, id: ResourceId) -> Option<&Vec<String>> {
        self.data.waves.get(&id)
    }

    /// Borrow the mouse cursor metadata for a loaded id, if any.
    pub fn mouse_entry(&self, id: ResourceId) -> Option<&MouseEntry> {
        self.data.mouse_entries.get(&id)
    }

    /// Sorted list of `(resource_id, type_tag)` for registered archive entries.
    /// Resident shipping payloads without archive metadata are not included.
    /// Used by the shipping converter to walk the manager in a stable order
    /// when re-serializing as a `.res` byte blob.
    pub fn resource_ids_with_types(&self) -> Vec<(ResourceId, [u8; 4])> {
        let mut out: Vec<(ResourceId, [u8; 4])> = self
            .lifetime
            .file_entries
            .iter()
            .map(|(&id, e)| (id, e.resource_type))
            .collect();
        out.sort_by_key(|(id, _)| *id);
        out
    }

    /// Re-serialize this `ResourceManager` to the on-disk `.res` byte format,
    /// emitting every embedded packed 16-bit picture with the chosen `packing`.
    /// The shipping converter uses `SixteenPacking::None` so the bzip2-only
    /// inner compression is gone, then lets the outer datadir zstd-22 catch
    /// the cross-picture redundancy.
    ///
    /// Note: original per-resource `flags` values are not preserved by the
    /// reader, so we emit `0` for them. Bitmasks for flagged-picture types
    /// (BTTN/TOGL/NPTF/SLID/RDO) are reconstructed from which `Option<Picture>`
    /// slots are `Some`. CUR mouse metadata is emitted from `MouseEntry`.
    pub fn write_to_res_bytes(&self, packing: crate::picture::SixteenPacking) -> Result<Vec<u8>> {
        let ids = self.resource_ids_with_types();
        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(b"SRES");
        out.extend_from_slice(&RES_VERSION_100.to_le_bytes());
        out.extend_from_slice(
            &u32::try_from(ids.len())
                .context("resource count exceeds u32")?
                .to_le_bytes(),
        );

        for (id, tag) in &ids {
            out.extend_from_slice(tag);
            out.extend_from_slice(&(*id as u32).to_le_bytes());
            match tag {
                b"PIC " => {
                    out.extend_from_slice(&0u32.to_le_bytes()); // flags
                    let pics = self.data.pictures.get(id).ok_or_else(|| {
                        anyhow!("PIC {id}: missing parsed pictures in ResourceManager")
                    })?;
                    let pic = pics
                        .first()
                        .and_then(|p| p.as_ref())
                        .ok_or_else(|| anyhow!("PIC {id}: empty"))?;
                    out.extend(pic.write_sixteen_to_bytes(packing)?);
                }
                b"PICC" => {
                    let pics = self
                        .data
                        .pictures
                        .get(id)
                        .ok_or_else(|| anyhow!("PICC {id}: missing"))?;
                    out.extend_from_slice(&0u32.to_le_bytes());
                    out.extend_from_slice(
                        &u32::try_from(pics.len())
                            .with_context(|| format!("resource {id}: picture count exceeds u32"))?
                            .to_le_bytes(),
                    );
                    for slot in pics {
                        let pic = slot
                            .as_ref()
                            .ok_or_else(|| anyhow!("PICC {id}: missing sub-picture"))?;
                        out.extend(pic.write_sixteen_to_bytes(packing)?);
                    }
                }
                b"BTTN" | b"TOGL" | b"NPTF" | b"SLID" | b"RDO " => {
                    let pics = self
                        .data
                        .pictures
                        .get(id)
                        .ok_or_else(|| anyhow!("{tag:?} {id}: missing"))?;
                    let count = flagged_picture_count(tag).expect("matched flagged picture tag");
                    if pics.len() > count {
                        bail!(
                            "{tag:?} {id}: {} picture slots exceed the supported {count}",
                            pics.len()
                        );
                    }
                    let mut bitmask: u32 = 0;
                    for (i, slot) in pics.iter().enumerate() {
                        if slot.is_some() {
                            bitmask |= 1 << i;
                        }
                    }
                    out.extend_from_slice(&0u32.to_le_bytes()); // flags
                    out.extend_from_slice(&bitmask.to_le_bytes());
                    for slot in pics.iter() {
                        if let Some(pic) = slot.as_ref() {
                            out.extend(pic.write_sixteen_to_bytes(packing)?);
                        }
                    }
                }
                b"CUR " => {
                    let pics = self
                        .data
                        .pictures
                        .get(id)
                        .ok_or_else(|| anyhow!("CUR {id}: missing"))?;
                    let mouse = self
                        .data
                        .mouse_entries
                        .get(id)
                        .ok_or_else(|| anyhow!("CUR {id}: missing mouse"))?;
                    out.extend_from_slice(&0u32.to_le_bytes()); // flags
                    out.extend_from_slice(&mouse.flags.to_le_bytes());
                    out.extend_from_slice(&(mouse.hotspot.x as u16).to_le_bytes());
                    out.extend_from_slice(&(mouse.hotspot.y as u16).to_le_bytes());
                    out.extend_from_slice(&mouse.frame_length.to_le_bytes());
                    out.extend_from_slice(
                        &u32::try_from(pics.len())
                            .with_context(|| format!("resource {id}: picture count exceeds u32"))?
                            .to_le_bytes(),
                    );
                    for slot in pics {
                        let pic = slot
                            .as_ref()
                            .ok_or_else(|| anyhow!("CUR {id}: missing sub-picture"))?;
                        out.extend(pic.write_sixteen_to_bytes(packing)?);
                    }
                }
                b"TEXT" => {
                    let strs = self
                        .data
                        .strings
                        .get(id)
                        .ok_or_else(|| anyhow!("TEXT {id}: missing"))?;
                    out.extend_from_slice(&0u32.to_le_bytes()); // flags
                    out.extend_from_slice(
                        &u16::try_from(strs.len())
                            .with_context(|| format!("TEXT {id}: string count exceeds u16"))?
                            .to_le_bytes(),
                    );
                    for s in strs {
                        let utf16: Vec<u16> = s.encode_utf16().collect();
                        out.extend_from_slice(
                            &u16::try_from(utf16.len())
                                .with_context(|| {
                                    format!("TEXT {id}: UTF-16 string length exceeds u16")
                                })?
                                .to_le_bytes(),
                        );
                        for c in &utf16 {
                            out.extend_from_slice(&c.to_le_bytes());
                        }
                    }
                }
                b"WAVE" => {
                    let waves = self
                        .data
                        .waves
                        .get(id)
                        .ok_or_else(|| anyhow!("WAVE {id}: missing"))?;
                    out.extend_from_slice(&0u32.to_le_bytes()); // flags
                    out.extend_from_slice(
                        &u16::try_from(waves.len())
                            .with_context(|| format!("WAVE {id}: path count exceeds u16"))?
                            .to_le_bytes(),
                    );
                    for w in waves {
                        // Original on-disk size includes the trailing NUL byte
                        // when the C side stored it; emit raw ASCII bytes
                        // verbatim. Length-prefixed, no NUL terminator added.
                        out.extend_from_slice(
                            &u16::try_from(w.len())
                                .with_context(|| {
                                    format!("WAVE {id}: path byte length exceeds u16")
                                })?
                                .to_le_bytes(),
                        );
                        out.extend_from_slice(w.as_bytes());
                    }
                }
                other => bail!(
                    "write_to_res_bytes: unsupported tag {:?}",
                    std::str::from_utf8(other).unwrap_or("????")
                ),
            }
        }
        Ok(out)
    }

    /// Merge a borrowed shipping resource manager, replacing matching collections.
    /// Decoded, encoded, and geometry caches must follow the same source generation.
    pub(crate) fn extend_from(&mut self, src: &ResourceManager) {
        self.merge_resources(src.data.clone(), src.lifetime.clone());
    }

    fn merge_resources(&mut self, data: ResourceData, lifetime: ResourceLifetime) {
        self.invalidate_picture_cache();
        // Clear the complete old resource, including its recovery origin.
        // The wire maps remain unchanged; replacement policy lives here.
        for &id in data
            .pictures
            .keys()
            .chain(data.encoded_pictures.keys())
            .chain(data.picture_opacity.keys())
            .chain(data.mouse_entries.keys())
            .chain(data.strings.keys())
            .chain(data.waves.keys())
            .chain(lifetime.file_entries.keys())
        {
            self.data.remove(id);
            self.lifetime.references.remove(&id);
            self.lifetime.file_entries.remove(&id);
        }
        self.data.pictures.extend(data.pictures);
        self.data.picture_opacity.extend(data.picture_opacity);
        self.data.encoded_pictures.extend(data.encoded_pictures);
        self.data.mouse_entries.extend(data.mouse_entries);
        self.data.strings.extend(data.strings);
        self.data.waves.extend(data.waves);
        self.lifetime.references.extend(lifetime.references);
        self.lifetime.file_entries.extend(lifetime.file_entries);
        self.lifetime.recovery_disabled |= lifetime.recovery_disabled;
    }

    /// Finalize an eagerly parsed resource manager for shipping without its
    /// source `.res` archive. Runtime resource payloads stay resident, and an
    /// accidental future dismiss/recover path fails with an explicit error.
    pub fn disable_recovery_for_shipping(&mut self) {
        self.lifetime.file_entries.clear();
        self.lifetime.recovery_disabled = true;
    }

    /// Dump all resources as a JSON value.
    /// Picture pixel data is omitted — only dimensions and format are included.
    pub fn dump_json(&self) -> serde_json::Value {
        let mut resources = BTreeMap::new();

        for (&id, entry) in &self.lifetime.file_entries {
            let type_tag = std::str::from_utf8(&entry.resource_type)
                .unwrap_or("????")
                .trim()
                .to_string();

            let data = match entry.resource_type {
                _ if self.data.strings.contains_key(&id) => {
                    let strings = &self.data.strings[&id];
                    serde_json::json!({
                        "type": type_tag,
                        "count": strings.len(),
                        "strings": strings,
                    })
                }
                _ if self.data.waves.contains_key(&id) => {
                    let waves = &self.data.waves[&id];
                    serde_json::json!({
                        "type": type_tag,
                        "count": waves.len(),
                        "paths": waves,
                    })
                }
                _ if self.data.pictures.contains_key(&id) => {
                    let pics = &self.data.pictures[&id];
                    let pic_info: Vec<_> = pics
                        .iter()
                        .map(|p| match p {
                            Some(pic) => serde_json::json!({
                                "width": pic.width,
                                "height": pic.height,
                                "format": format!("{:?}", pic.pixel_format),
                            }),
                            None => serde_json::Value::Null,
                        })
                        .collect();
                    let mut obj = serde_json::json!({
                        "type": type_tag,
                        "count": pics.len(),
                        "pictures": pic_info,
                    });
                    if let Some(mouse) = self.data.mouse_entries.get(&id) {
                        obj["cursor"] = serde_json::json!({
                            "hotspot_x": mouse.hotspot.x,
                            "hotspot_y": mouse.hotspot.y,
                            "flags": mouse.flags,
                            "frame_length": mouse.frame_length,
                        });
                    }
                    obj
                }
                _ => serde_json::json!({ "type": type_tag }),
            };

            resources.insert(id.to_string(), data);
        }

        serde_json::json!(resources)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_acquisition_classifies_the_single_read_without_reprobing() {
        use robin_data_io::sbfile::{SBFILE_ERROR_FILE_NOT_FOUND, SBFILE_ERROR_READ};
        for status in [SBFILE_ERROR_FILE_NOT_FOUND, SBFILE_ERROR_READ] {
            let calls = std::cell::Cell::new(0);
            let result = acquire_resource_bytes("fixture.res", || {
                calls.set(calls.get() + 1);
                Err(status)
            });
            assert_eq!(calls.get(), 1);
            match (status, result) {
                (SBFILE_ERROR_FILE_NOT_FOUND, Ok(None)) => {}
                (SBFILE_ERROR_READ, Err(ResourceAttachmentError::Unavailable(error))) => {
                    assert!(error.to_string().contains("fixture.res"));
                    assert!(error.to_string().contains("error -5"));
                }
                (_, result) => panic!("wrong acquisition classification: {result:?}"),
            }
        }
    }

    #[test]
    fn optional_attachment_keeps_authority_and_read_failure_distinct_from_absence() {
        let mut unbound = ResourceManager::new();
        assert!(matches!(
            unbound.try_attach_or_from_shipping("missing.res", None),
            Err(ResourceAttachmentError::Unavailable(_))
        ));
        let assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
        let mut manager = ResourceManager::with_files(Arc::new(SbFileSystem::new(assets)));
        assert_eq!(
            manager
                .try_attach_or_from_shipping("missing.res", None)
                .unwrap(),
            None
        );
        assert!(
            manager
                .attach_or_from_shipping("missing.res", None)
                .is_err()
        );
        assert!(matches!(
            manager.try_attach_or_from_shipping("../forbidden.res", None),
            Err(ResourceAttachmentError::Unavailable(_))
        ));
    }

    #[test]
    fn attachment_prefers_shipping_but_preserves_original_fallback_and_parse_errors() {
        let assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
        let mut original = ResourceManager::new();
        original.data.strings.insert(7, vec!["original".into()]);
        assets
            .install_preloaded_asset(
                "fixture.res",
                original
                    .write_to_res_bytes(crate::picture::SixteenPacking::None)
                    .unwrap(),
            )
            .unwrap();
        assets
            .install_preloaded_asset("broken.res", b"invalid archive".to_vec())
            .unwrap();
        let mut manager = ResourceManager::with_files(Arc::new(SbFileSystem::new(assets.clone())));
        let empty_shipping = crate::shipping_datadir::ShippingAssets::install(
            Arc::new(crate::shipping_datadir::ShippingDatadir::default()),
            assets.clone(),
        )
        .unwrap();
        assert_eq!(
            manager
                .try_attach_or_from_shipping("fixture.res", Some(empty_shipping.datadir()))
                .unwrap(),
            Some(())
        );
        assert_eq!(manager.get_string(7, 0).unwrap(), "original");
        let mut converted = ResourceManager::new();
        converted.data.strings.insert(7, vec!["shipping".into()]);
        let mut shipping = crate::shipping_datadir::ShippingDatadir::default();
        shipping.res_files.insert("fixture.res".into(), converted);
        let shipping =
            crate::shipping_datadir::ShippingAssets::install(Arc::new(shipping), assets).unwrap();
        manager
            .attach_or_from_shipping("fixture.res", Some(shipping.datadir()))
            .unwrap();
        assert_eq!(manager.get_string(7, 0).unwrap(), "shipping");
        let identity = manager.cache_identity();
        let error = manager
            .try_attach_or_from_shipping("broken.res", Some(shipping.datadir()))
            .unwrap_err();
        assert!(matches!(&error, ResourceAttachmentError::Malformed(_)));
        assert!(
            std::error::Error::source(&error)
                .unwrap()
                .to_string()
                .contains("bad magic")
        );
        assert_eq!(manager.cache_identity(), identity);
        assert_eq!(manager.get_string(7, 0).unwrap(), "shipping");
    }

    #[test]
    fn cursor_and_collection_readers_share_picture_payload_order() {
        let picture = Picture {
            width: 1,
            height: 1,
            pitch: 2,
            pixel_format: crate::picture::PixelFormat::Rgb16,
            data: vec![0x34, 0x12],
            palette: None,
        };
        let encoded = picture
            .write_sixteen_to_bytes(crate::picture::SixteenPacking::None)
            .unwrap();
        for count in [0u32, 2] {
            let mut collection = 0x12345678u32.to_le_bytes().to_vec();
            let mut cursor = collection.clone();
            for field in [5u16, 3, 4, 17] {
                cursor.extend_from_slice(&field.to_le_bytes());
            }
            for payload in [&mut collection, &mut cursor] {
                payload.extend_from_slice(&count.to_le_bytes());
                for _ in 0..count {
                    payload.extend_from_slice(&encoded);
                }
            }
            let mut collection_reader = Reader::new(&collection);
            let pictures = read_picture_collection(&mut collection_reader, "collection").unwrap();
            let mut cursor_reader = Reader::new(&cursor);
            let (entry, cursor_pictures) = read_cursor(&mut cursor_reader, "cursor").unwrap();
            assert_eq!(pictures.len(), count as usize);
            assert_eq!(
                serde_json::to_value(&pictures).unwrap(),
                serde_json::to_value(&cursor_pictures).unwrap()
            );
            for decoded in pictures.iter().flatten() {
                assert_eq!(decoded.data, picture.data);
            }
            assert_eq!(entry.flags, 5);
            assert_eq!(entry.hotspot, CursorHotspot::new(3.0, 4.0));
            assert_eq!(entry.frame_length, 17);
            assert_eq!(collection_reader.remaining(), 0);
            assert_eq!(cursor_reader.remaining(), 0);
        }
    }

    #[test]
    fn picture_presence_uses_registered_type_not_reference_bookkeeping() {
        let mut manager = ResourceManager::new();
        for tag in [b"TEXT", b"WAVE", b"NOPE"] {
            manager.lifetime.file_entries.insert(
                42,
                ResourceFileEntry {
                    file_path: "not-opened.res".to_owned(),
                    file_offset: 0,
                    resource_type: *tag,
                },
            );
            manager.lifetime.references.insert(42, 1);
            assert!(!manager.has_picture_resource(42));
            assert!(manager.find_pictures(42).unwrap().is_none());
        }
        for tag in [
            b"PIC ", b"PICC", b"BTTN", b"TOGL", b"NPTF", b"CUR ", b"SLID", b"RDO ",
        ] {
            manager
                .lifetime
                .file_entries
                .get_mut(&42)
                .unwrap()
                .resource_type = *tag;
            assert!(manager.has_picture_resource(42));
            // Registered-but-unreadable is an error, not absence.
            assert!(manager.find_pictures(42).is_err());
        }
        manager.lifetime.file_entries.clear();
        assert!(!manager.has_picture_resource(42));
        manager.data.pictures.insert(42, vec![None]);
        assert!(manager.has_picture_resource(42));
        assert_eq!(manager.find_pictures(42).unwrap().unwrap().len(), 1);
        manager.data.pictures.clear();
        manager.data.encoded_pictures.insert(42, vec![None; 2]);
        assert!(manager.has_picture_resource(42));
        assert_eq!(manager.find_pictures(42).unwrap().unwrap().len(), 2);
    }

    #[test]
    fn resource_writer_rejects_unrepresentable_lengths_and_widget_slots() {
        fn manager(tag: &[u8; 4]) -> ResourceManager {
            let mut manager = ResourceManager::new();
            manager.lifetime.file_entries.insert(
                42,
                ResourceFileEntry {
                    file_path: String::new(),
                    file_offset: 0,
                    resource_type: *tag,
                },
            );
            manager
        }
        let packing = crate::picture::SixteenPacking::None;
        let mut text = manager(b"TEXT");
        text.data.strings.insert(42, vec![String::new(); 65_536]);
        assert!(
            text.write_to_res_bytes(packing)
                .unwrap_err()
                .to_string()
                .contains("string count")
        );
        // Count UTF-16 code units, not UTF-8 bytes or Unicode scalar values.
        text.data.strings.insert(42, vec!["😀".repeat(32_768)]);
        assert!(
            text.write_to_res_bytes(packing)
                .unwrap_err()
                .to_string()
                .contains("UTF-16 string length")
        );
        let boundary = format!("{}a", "😀".repeat(32_767));
        text.data.strings.insert(42, vec![boundary.clone()]);
        let bytes = text.write_to_res_bytes(packing).unwrap();
        assert_eq!(
            read_string_table(&mut Reader::new(&bytes[20..]), "fixture").unwrap(),
            vec![boundary]
        );

        let mut waves = manager(b"WAVE");
        waves.data.waves.insert(42, vec![String::new(); 65_536]);
        assert!(
            waves
                .write_to_res_bytes(packing)
                .unwrap_err()
                .to_string()
                .contains("path count")
        );
        waves.data.waves.insert(42, vec!["é".repeat(32_768)]);
        assert!(
            waves
                .write_to_res_bytes(packing)
                .unwrap_err()
                .to_string()
                .contains("path byte length")
        );
        waves.data.waves.insert(42, vec!["a".repeat(65_535)]);
        assert!(waves.write_to_res_bytes(packing).is_ok());

        for (tag, count) in [
            (b"BTTN", 4),
            (b"TOGL", 5),
            (b"NPTF", 6),
            (b"SLID", 6),
            (b"RDO ", 7),
        ] {
            let mut pictures = manager(tag);
            pictures.data.pictures.insert(42, vec![None; count + 1]);
            assert!(
                pictures
                    .write_to_res_bytes(packing)
                    .unwrap_err()
                    .to_string()
                    .contains("picture slots")
            );
            pictures.data.pictures.insert(42, vec![None; count]);
            let bytes = pictures.write_to_res_bytes(packing).unwrap();
            let mut restored = ResourceManager::new();
            restored
                .load_resource_data(&mut Reader::new(&bytes[20..]), 42, tag)
                .unwrap();
            assert_eq!(restored.pictures_raw(42).unwrap().len(), count);
        }
    }

    #[test]
    fn failed_shipping_encoding_retains_the_collection_for_retry() {
        let mut manager = ResourceManager::new();
        manager.data.pictures.insert(
            42,
            vec![Some(Picture::default()), None, Some(Picture::default())],
        );
        manager
            .data
            .encoded_pictures
            .insert(42, vec![Some(EncodedPicture::jxl_rgba565_keyed(vec![7]))]);
        let originals = serde_json::to_value(&manager.data.pictures[&42]).unwrap();
        let original_storage = manager.data.pictures[&42].as_ptr();
        let mut calls = 0;
        let error = manager
            .encode_pictures_for_shipping(|_| {
                calls += 1;
                if calls == 2 {
                    bail!("injected encoder failure");
                }
                Ok(EncodedPicture::jxl_rgba565_keyed(vec![1]))
            })
            .unwrap_err();
        assert!(
            format!("{error:#}")
                .contains("resource 42/2: encode picture for shipping: injected encoder failure")
        );
        assert_eq!(calls, 2);
        assert_eq!(manager.data.pictures[&42].as_ptr(), original_storage);
        assert_eq!(
            serde_json::to_value(&manager.data.pictures[&42]).unwrap(),
            originals
        );
        assert_eq!(
            manager.data.encoded_pictures[&42][0]
                .as_ref()
                .unwrap()
                .bytes,
            vec![7]
        );

        let count = manager
            .encode_pictures_for_shipping(|_| Ok(EncodedPicture::jxl_rgba565_keyed(vec![2])))
            .unwrap();
        assert_eq!(count, 2);
        assert!(!manager.data.pictures.contains_key(&42));
        let encoded = &manager.data.encoded_pictures[&42];
        assert_eq!(encoded.len(), 3);
        assert!(encoded[1].is_none());
        for slot in [0, 2] {
            assert_eq!(encoded[slot].as_ref().unwrap().bytes, vec![2]);
        }
    }

    #[test]
    fn merging_picture_collections_replaces_all_old_representations() {
        let mut destination = ResourceManager::new();
        destination
            .data
            .pictures
            .insert(42, vec![Some(Picture::default())]);
        destination.data.picture_opacity.insert(42, vec![None]);
        destination.data.pictures.insert(99, vec![None; 3]);

        let mut encoded = ResourceManager::new();
        encoded.data.encoded_pictures.insert(42, vec![None; 2]);
        destination.extend_from(&encoded);
        assert!(destination.pictures_raw(42).is_none());
        assert!(!destination.data.picture_opacity.contains_key(&42));
        assert_eq!(destination.get_picture_count(42).unwrap(), 2);
        assert!(destination.find_picture(42, 0).unwrap().is_none());
        assert_eq!(destination.pictures_raw(42).unwrap().len(), 2);
        assert_eq!(destination.get_picture_count(99).unwrap(), 3);

        let mut decoded = ResourceManager::new();
        decoded.data.pictures.insert(42, vec![None]);
        decoded.data.picture_opacity.insert(42, vec![None]);
        destination.extend_from(&decoded);
        assert!(!destination.data.encoded_pictures.contains_key(&42));
        assert_eq!(destination.get_picture_count(42).unwrap(), 1);
        assert_eq!(destination.data.picture_opacity[&42], vec![None]);
        assert_eq!(destination.get_picture_count(99).unwrap(), 3);
        // The source is borrowed, not consumed or warmed as a side effect.
        assert!(encoded.pictures_raw(42).is_none());
        assert_eq!(encoded.data.encoded_pictures[&42].len(), 2);
    }

    #[test]
    fn picture_metadata_preserves_holes_zero_sizes_and_decoded_precedence() {
        let mut manager = ResourceManager::new();
        manager.disable_recovery_for_shipping();
        manager.data.pictures.insert(
            42,
            vec![
                None,
                Some(Picture::default()),
                Some(Picture {
                    width: 2,
                    height: 3,
                    ..Picture::default()
                }),
                Some(Picture {
                    width: 0,
                    height: 7,
                    ..Picture::default()
                }),
                Some(Picture {
                    width: 9,
                    height: 0,
                    ..Picture::default()
                }),
            ],
        );
        // A warmed decoded collection takes precedence over its encoded source.
        manager.data.encoded_pictures.insert(42, vec![]);
        assert_eq!(manager.get_picture_count(42).unwrap(), 5);
        assert_eq!(manager.get_nonempty_picture_count(42).unwrap(), 1);
        assert_eq!(manager.get_dimension(42).unwrap(), (9, 7));
        assert_eq!(
            manager.get_picture_dimensions(42).unwrap(),
            [None, Some((0, 0)), Some((2, 3)), Some((0, 7)), Some((9, 0))]
        );
        manager
            .data
            .pictures
            .insert(43, vec![None, Some(Picture::default())]);
        assert_eq!(manager.get_nonempty_picture_count(43).unwrap(), 0);
        assert!(manager.get_dimension(43).is_err());
        assert!(manager.get_picture_count(99).is_err());
        assert!(manager.get_nonempty_picture_count(99).is_err());
    }

    #[test]
    fn encoded_picture_counts_read_headers_without_decoding_pixels() {
        // A 2x3 solid red RGB image generated by cjxl 0.11.2 (-d 0 -e 1).
        let bytes = vec![
            255, 10, 16, 0, 2, 128, 72, 8, 2, 1, 0, 156, 2, 75, 24, 155, 156, 113, 132, 3, 56, 128,
            3, 56, 32, 74, 192, 57, 5, 1, 0, 32, 68, 128, 8, 16, 1, 34, 64, 228, 255, 145, 123,
            250, 30, 90, 103, 87, 85, 85, 85, 37, 73, 146, 16, 80, 119, 119, 119, 119, 119, 255,
            255, 255, 191, 85, 111, 102, 102, 102, 6, 254, 223, 191, 231, 191, 135, 198, 156, 115,
            174, 181, 207, 189, 73, 146, 36, 4, 84, 85, 85, 85, 85, 85, 255, 255, 255, 207, 189,
            175, 187, 187, 187, 27, 254, 223, 191, 231, 191, 135, 198, 156, 115, 174, 181, 207,
            189, 73, 146, 36, 4, 84, 85, 85, 85, 85, 85, 255, 255, 255, 207, 189, 175, 187, 187,
            187, 27, 254, 223, 191, 231, 191, 135, 198, 156, 115, 174, 181, 207, 189, 73, 146, 36,
            4, 84, 85, 85, 85, 85, 85, 255, 255, 255, 207, 189, 175, 187, 187, 187, 251, 2, 33, 0,
            120, 248, 123, 244, 99, 0, 0,
        ];
        let encoded = EncodedPicture {
            codec: EncodedPictureCodec::JxlRgb565,
            bytes,
        };
        let decoded = encoded.decode().unwrap();
        assert_eq!(
            encoded.dimensions().unwrap(),
            (decoded.width, decoded.height)
        );
        let mut manager = ResourceManager::new();
        manager.disable_recovery_for_shipping();
        manager
            .data
            .encoded_pictures
            .insert(42, vec![None, Some(encoded)]);
        assert_eq!(manager.get_picture_count(42).unwrap(), 2);
        assert_eq!(manager.get_nonempty_picture_count(42).unwrap(), 1);
        assert_eq!(manager.get_dimension(42).unwrap(), (2, 3));
        assert_eq!(
            manager.get_picture_dimensions(42).unwrap(),
            [None, Some((2, 3))]
        );
        assert!(manager.pictures_raw(42).is_none());

        // Eager and lazy decoding use the same borrowed slot interpretation.
        // Failed resources must retain their encoded source, without publishing
        // a partially decoded collection.
        let mut warmed = manager.clone();
        warmed.data.encoded_pictures.insert(
            43,
            vec![None, Some(EncodedPicture::jxl_rgba565_keyed(vec![]))],
        );
        let encoded_before = serde_json::to_value(&warmed.data.encoded_pictures).unwrap();
        assert_eq!(warmed.decode_all_encoded_pictures(), 1);
        assert_eq!(warmed.get_pictures(42).unwrap().len(), 2);
        assert!(warmed.get_pictures(42).unwrap()[0].is_none());
        assert_eq!(warmed.get_picture(42, 1).unwrap().data, decoded.data);
        assert!(!warmed.data.pictures.contains_key(&43));
        assert_eq!(
            serde_json::to_value(&warmed.data.encoded_pictures).unwrap(),
            encoded_before
        );
        assert!(
            warmed
                .get_picture(43, 1)
                .unwrap_err()
                .to_string()
                .contains("resource 43/1")
        );
        assert!(!warmed.data.pictures.contains_key(&43));
        assert_eq!(warmed.decode_all_encoded_pictures(), 0);

        // Header inspection must not start pixel decode or require frame data.
        let picture = manager.data.encoded_pictures.get_mut(&42).unwrap()[1]
            .as_mut()
            .unwrap();
        let header_len = (1..picture.bytes.len())
            .find(|&len| Picture::jxl_dimensions(&picture.bytes[..len]).is_ok())
            .unwrap();
        picture.bytes.truncate(header_len);
        assert!(picture.decode().is_err());
        assert_eq!(manager.get_nonempty_picture_count(42).unwrap(), 1);
        assert!(manager.pictures_raw(42).is_none());

        // Aggregates must inspect later slots too, not hide corruption after
        // a valid nonempty frame has already contributed dimensions.
        manager
            .data
            .encoded_pictures
            .get_mut(&42)
            .unwrap()
            .push(Some(EncodedPicture::jxl_rgba565_keyed(vec![])));
        for error in [
            manager.get_nonempty_picture_count(42).unwrap_err(),
            manager.get_dimension(42).unwrap_err(),
            manager.get_picture_dimensions(42).unwrap_err(),
        ] {
            assert!(error.to_string().contains("resource 42/2"), "{error:#}");
        }
        assert!(manager.pictures_raw(42).is_none());
    }

    #[test]
    fn picture_metadata_recovers_dismissed_legacy_collections() {
        let assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
        assets
            .install_preloaded_asset("buttons.res", resource_file(b"BTTN", 42, &[0; 8]))
            .unwrap();
        let files = Arc::new(SbFileSystem::new(assets).snapshot());
        let mut manager = ResourceManager::with_files(files);
        manager.attach_resource_file("buttons.res").unwrap();
        manager.dismiss_resource(42);
        assert!(manager.pictures_raw(42).is_none());
        assert_eq!(manager.get_picture_count(42).unwrap(), 4);
        manager.dismiss_resource(42);
        assert_eq!(manager.get_nonempty_picture_count(42).unwrap(), 0);
        assert_eq!(manager.get_picture_dimensions(42).unwrap(), [None; 4]);
    }

    #[test]
    fn malformed_picture_headers_are_errors_but_slot_counts_need_no_header() {
        let mut manager = ResourceManager::new();
        manager
            .data
            .encoded_pictures
            .insert(42, vec![Some(EncodedPicture::jxl_rgba565_keyed(vec![]))]);
        assert_eq!(manager.get_picture_count(42).unwrap(), 1);
        assert!(manager.get_nonempty_picture_count(42).is_err());
        assert!(manager.get_dimension(42).is_err());
        assert!(manager.pictures_raw(42).is_none());
    }

    #[test]
    fn mixed_archive_entries_have_one_stable_sorted_export_order() {
        let mut text = 0u32.to_le_bytes().to_vec();
        text.extend_from_slice(&1u16.to_le_bytes());
        text.extend_from_slice(&1u16.to_le_bytes());
        text.extend_from_slice(&(b'A' as u16).to_le_bytes());
        let empty_waves = [0u8; 6]; // flags and u16 count
        let empty_pictures = [0u8; 8]; // flags and u32 count
        let entries = [
            resource_file(b"TEXT", 90, &text),
            resource_file(b"WAVE", 3, &empty_waves),
            resource_file(b"PICC", 12, &empty_pictures),
        ];
        let mut bytes = entries[0][..12].to_vec();
        bytes[8..12].copy_from_slice(&3u32.to_le_bytes());
        let header = bytes.clone();
        for entry in &entries {
            bytes.extend_from_slice(&entry[12..]);
        }
        let mut manager = ResourceManager::new();
        manager.attach_resource_bytes(&bytes, "mixed.res").unwrap();
        manager.data.strings.insert(1, vec!["resident-only".into()]);
        assert_eq!(
            manager.resource_ids_with_types(),
            [(3, *b"WAVE"), (12, *b"PICC"), (90, *b"TEXT")]
        );
        let mut expected = header;
        for index in [1, 2, 0] {
            expected.extend_from_slice(&entries[index][12..]);
        }
        assert_eq!(
            manager
                .write_to_res_bytes(crate::picture::SixteenPacking::None)
                .unwrap(),
            expected
        );
    }

    #[test]
    fn shipping_picture_ids_include_encoded_and_decoded_without_archive_metadata() {
        let mut manager = ResourceManager::new();
        manager.data.pictures.insert(9, vec![None]);
        manager.data.pictures.insert(3, vec![None]);
        manager.data.encoded_pictures.insert(3, vec![None]);
        manager.data.encoded_pictures.insert(7, vec![None]);
        manager.data.strings.insert(5, vec!["text".into()]);
        manager.disable_recovery_for_shipping();
        assert!(manager.resource_ids_with_types().is_empty());
        assert_eq!(manager.picture_resource_ids(), [3, 7, 9]);
    }

    #[test]
    fn bound_reader_survives_clone_but_not_serialization() {
        let assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&(b'A' as u16).to_le_bytes());
        assets
            .install_preloaded_asset(
                "authority-fixture.res",
                resource_file(b"TEXT", 42, &payload),
            )
            .unwrap();
        let files = Arc::new(SbFileSystem::new(assets).snapshot());
        let mut manager = ResourceManager::with_files(files.clone());
        manager
            .attach_resource_file("authority-fixture.res")
            .unwrap();
        let mut clone = manager.clone();
        assert!(Arc::ptr_eq(clone.files.as_ref().unwrap(), &files));
        clone.recover_resource(42).unwrap();
        // TODO: flattened integer-key resource maps do not currently support
        // JSON deserialization. Exercise authority omission with empty maps;
        // the historical bitcode contract below covers populated resources.
        let json = serde_json::to_value(ResourceManager::with_files(files.clone())).unwrap();
        let mut decoded: ResourceManager = serde_json::from_value(json).unwrap();
        assert!(
            decoded
                .attach_resource_file("authority-fixture.res")
                .unwrap_err()
                .to_string()
                .contains("no bound file reader")
        );
        let mut decoded: ResourceManager = bitcode::decode(&bitcode::encode(&manager)).unwrap();
        assert!(
            decoded
                .recover_resource(42)
                .unwrap_err()
                .to_string()
                .contains("no bound file reader")
        );
        decoded.bind_files(files);
        decoded.recover_resource(42).unwrap();
        assert_eq!(decoded.strings_raw(42).unwrap(), &["A"]);
        assert!(
            ResourceManager::new()
                .attach_resource_file("authority-fixture.res")
                .unwrap_err()
                .to_string()
                .contains("no bound file reader")
        );
    }

    fn resource_file(resource_type: &[u8; 4], id: u32, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"SRES");
        bytes.extend_from_slice(&RES_VERSION_100.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(resource_type);
        bytes.extend_from_slice(&id.to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn failed_archive_attachment_preserves_resources_and_cache_identity() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&(b'B' as u16).to_le_bytes());
        let valid = resource_file(b"TEXT", 42, &payload);
        let mut malformed = valid.clone();
        malformed[8..12].copy_from_slice(&2u32.to_le_bytes());
        malformed.extend_from_slice(b"NOPE");
        malformed.extend_from_slice(&99u32.to_le_bytes());

        let mut manager = ResourceManager::new();
        manager.data.strings.insert(42, vec!["original".into()]);
        manager.data.strings.insert(99, vec!["unrelated".into()]);
        manager.lifetime.references.insert(42, 4);
        let before = serde_json::to_value(&manager).unwrap();
        let identity = manager.cache_identity;
        let error = manager
            .attach_resource_bytes(&malformed, "replacement.res")
            .unwrap_err();
        assert!(format!("{error:#}").contains("resource 99 (NOPE)"));
        assert_eq!(serde_json::to_value(&manager).unwrap(), before);
        assert_eq!(manager.cache_identity, identity);

        manager
            .attach_resource_bytes(&valid, "replacement.res")
            .unwrap();
        assert_eq!(manager.strings_raw(42).unwrap(), &["B"]);
        assert_eq!(manager.strings_raw(99).unwrap(), &["unrelated"]);
        assert_eq!(manager.lifetime.references[&42], 0);
        assert_eq!(
            manager.lifetime.file_entries[&42].file_path,
            "replacement.res"
        );
        assert_ne!(manager.cache_identity, identity);
    }

    #[test]
    fn replacement_removes_incompatible_values_and_old_recovery_origins() {
        for old_tag in [b"TEXT", b"WAVE", b"PICC"] {
            for new_tag in [b"TEXT", b"WAVE", b"PICC"] {
                let payload = |tag| {
                    if tag == b"PICC" {
                        &[0; 8][..]
                    } else {
                        &[0; 6][..]
                    }
                };
                let old = resource_file(old_tag, 42, payload(old_tag));
                let new = resource_file(new_tag, 42, payload(new_tag));
                for same_archive in [false, true] {
                    let mut manager = ResourceManager::new();
                    if same_archive {
                        let mut combined = old.clone();
                        combined[8..12].copy_from_slice(&2u32.to_le_bytes());
                        combined.extend_from_slice(&new[12..]);
                        manager
                            .attach_resource_bytes(&combined, "combined.res")
                            .unwrap();
                    } else {
                        manager.attach_resource_bytes(&old, "old.res").unwrap();
                        manager.attach_resource_bytes(&new, "new.res").unwrap();
                    }
                    assert_eq!(manager.data.strings.contains_key(&42), new_tag == b"TEXT");
                    assert_eq!(manager.data.waves.contains_key(&42), new_tag == b"WAVE");
                    assert_eq!(manager.data.pictures.contains_key(&42), new_tag == b"PICC");
                    assert_eq!(&manager.lifetime.file_entries[&42].resource_type, new_tag);
                }
            }
        }

        let mut destination = ResourceManager::new();
        destination
            .attach_resource_bytes(&resource_file(b"TEXT", 42, &[0; 6]), "old.res")
            .unwrap();
        destination
            .data
            .strings
            .insert(99, vec!["unrelated".into()]);
        let mut shipping = ResourceManager::new();
        shipping.data.waves.insert(42, vec!["new.wav".into()]);
        destination.extend_from(&shipping);
        assert!(destination.get_strings(42).is_err());
        assert!(destination.strings_raw(42).is_none());
        assert_eq!(destination.waves_raw(42).unwrap(), &["new.wav"]);
        assert!(!destination.lifetime.file_entries.contains_key(&42));
        assert!(!destination.lifetime.references.contains_key(&42));
        assert_eq!(destination.strings_raw(99).unwrap(), &["unrelated"]);
    }

    #[test]
    fn text_table_decoding_matches_strict_utf16_and_preserves_trailing_bytes() {
        let cases: &[&[u16]] = &[
            &[],
            &[0x41, 0, 0xE9, 0x96EA],
            &[0xD83D, 0xDE00],
            &[0xD800],
            &[0xDC00],
            &[0xD800, 0x41],
            &[0xD800, 0xD800, 0xDC00],
        ];
        for units in cases {
            let mut payload = Vec::new();
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&1u16.to_le_bytes());
            payload.extend_from_slice(&(units.len() as u16).to_le_bytes());
            for unit in *units {
                payload.extend_from_slice(&unit.to_le_bytes());
            }
            let end = payload.len();
            payload.extend_from_slice(&[0xAA, 0xBB]);
            let mut reader = Reader::new(&payload);
            let decoded = read_string_table(&mut reader, "fixture");
            match String::from_utf16(units) {
                Ok(expected) => assert_eq!(decoded.unwrap(), vec![expected]),
                Err(_) => assert!(
                    decoded
                        .unwrap_err()
                        .to_string()
                        .contains("string 0: invalid UTF-16")
                ),
            }
            assert_eq!(reader.position(), end);
            assert_eq!(reader.take(2, "trailer").unwrap(), &[0xAA, 0xBB]);
        }
    }

    #[test]
    fn invalid_utf16_is_a_contextual_error_not_an_empty_string() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u32.to_le_bytes()); // flags
        payload.extend_from_slice(&1u16.to_le_bytes()); // string count
        payload.extend_from_slice(&1u16.to_le_bytes()); // code-unit count
        payload.extend_from_slice(&0xD800u16.to_le_bytes()); // unpaired surrogate
        let bytes = resource_file(b"TEXT", 42, &payload);

        let error = ResourceManager::new()
            .attach_resource_bytes(&bytes, "malformed.res")
            .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("resource 42 (TEXT)"));
        assert!(message.contains("string 0: invalid UTF-16"));
    }

    #[test]
    fn truncated_utf16_range_is_rejected_before_allocating_code_units() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&3u16.to_le_bytes());
        payload.extend_from_slice(&(b'A' as u16).to_le_bytes());
        let bytes = resource_file(b"TEXT", 7, &payload);

        let error = ResourceManager::new()
            .attach_resource_bytes(&bytes, "truncated.res")
            .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("resource 7 (TEXT) string 0 UTF-16 data"));
        assert!(message.contains("only 2 remain"));
    }

    #[test]
    fn picture_payload_range_must_fit_the_resource_file() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u32.to_le_bytes()); // resource flags
        payload.extend_from_slice(&1u16.to_le_bytes()); // width
        payload.extend_from_slice(&1u16.to_le_bytes()); // height
        payload.extend_from_slice(&0u32.to_le_bytes()); // uncompressed
        payload.extend_from_slice(&4u32.to_le_bytes()); // declared payload size
        payload.extend_from_slice(&[0xAA, 0xBB]); // only half is present
        let bytes = resource_file(b"PIC ", 99, &payload);

        let error = ResourceManager::new()
            .attach_resource_bytes(&bytes, "bad-picture.res")
            .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("resource 99 (PIC ) picture 0 Sixteen payload"));
        assert!(message.contains("wanted 4 bytes, only 2 remain"));
    }

    #[test]
    fn impossible_resource_count_is_rejected_before_iteration() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"SRES");
        bytes.extend_from_slice(&RES_VERSION_100.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());

        let error = ResourceManager::new()
            .attach_resource_bytes(&bytes, "bad-count.res")
            .unwrap_err();
        assert!(error.to_string().contains("resource file entry count"));
        assert!(error.to_string().contains("count 4294967295"));
    }

    #[test]
    fn new_manager_is_empty() {
        let mgr = ResourceManager::new();
        assert!(!mgr.has_picture_resource(1));
        assert!(!mgr.has_resource(1));
    }

    #[test]
    fn dismiss_nonexistent_is_noop() {
        let mut mgr = ResourceManager::new();
        mgr.dismiss_resource(42); // should not panic
    }

    #[test]
    fn legacy_reference_counts_round_trip_without_classifying_resource_types() {
        let mut manager = ResourceManager::new();
        manager.lifetime.references.insert(42, 7);
        let mut restored: ResourceManager = bitcode::decode(&bitcode::encode(&manager)).unwrap();
        assert_eq!(restored.lifetime.references[&42], 7);
        assert!(!restored.has_picture_resource(42));
        assert!(!restored.has_resource(42));
        for (tag, expected) in [(b"TEXT", true), (b"WAVE", true), (b"PIC ", true)] {
            restored.lifetime.file_entries.insert(
                42,
                ResourceFileEntry {
                    file_path: "not-opened.res".to_owned(),
                    file_offset: 0,
                    resource_type: *tag,
                },
            );
            assert_eq!(restored.has_resource(42), expected);
        }
        restored.lifetime.file_entries.clear();
        restored
            .data
            .strings
            .insert(42, vec!["resident".to_owned()]);
        assert!(restored.has_resource(42));
    }
}

#[cfg(test)]
#[path = "resource_opacity_tests.rs"]
mod opacity_tests;

#[cfg(test)]
mod cache_lookup_tests {
    use super::*;

    #[test]
    fn identities_separate_sources_clones_and_deserialization() {
        let source = ResourceManager::new();
        let duplicate = source.clone();
        let decoded: ResourceManager =
            serde_json::from_value(serde_json::to_value(&source).unwrap()).unwrap();
        let binary: ResourceManager = bitcode::decode(&bitcode::encode(&source)).unwrap();
        let identities = [
            source.cache_identity(),
            duplicate.cache_identity(),
            decoded.cache_identity(),
            binary.cache_identity(),
        ];
        assert_eq!(
            identities
                .into_iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            4
        );
    }

    #[test]
    fn reload_invalidates_cache_but_rejected_attachment_preserves_identity() {
        let mut manager = ResourceManager::new();
        let initial = manager.cache_identity();
        manager.extend_from(&ResourceManager::new());
        let merged = manager.cache_identity();
        assert_ne!(initial, merged);
        assert!(
            manager
                .attach_resource_bytes(b"invalid", "fixture.res")
                .is_err()
        );
        assert_eq!(merged, manager.cache_identity());
    }

    #[test]
    fn optional_lookup_distinguishes_absence_sparse_and_corruption() {
        let mut manager = ResourceManager::new();
        assert!(manager.find_picture(1, 0).unwrap().is_none());
        manager.data.pictures.insert(1, vec![None]);
        assert!(manager.find_picture(1, 0).unwrap().is_none());
        assert!(manager.find_picture(1, usize::MAX).unwrap().is_none());
        manager.data.encoded_pictures.insert(
            2,
            vec![Some(EncodedPicture::jxl_rgba565_keyed(vec![0, 1, 2]))],
        );
        assert!(manager.find_picture(2, 0).is_err());
        manager.lifetime.file_entries.insert(
            3,
            ResourceFileEntry {
                file_path: "missing.res".into(),
                file_offset: 0,
                resource_type: *b"PIC ",
            },
        );
        assert!(manager.find_picture(3, 0).is_err());
        manager.lifetime.file_entries.insert(
            4,
            ResourceFileEntry {
                file_path: "missing.res".into(),
                file_offset: 0,
                resource_type: *b"TEXT",
            },
        );
        assert!(manager.find_picture(4, 0).unwrap().is_none());
    }
}
