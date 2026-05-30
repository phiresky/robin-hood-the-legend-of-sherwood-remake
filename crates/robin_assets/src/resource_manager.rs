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

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::picture::{Picture, read_bytes, read_tag, read_u16, read_u32, seek_to};
use robin_engine::coordinates::CursorHotspot;
use robin_engine::sbfile::SbFile;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Resource identifier (signed 32-bit; `-1` is the "no resource" sentinel).
pub type ResourceId = i32;

/// Mouse-cursor metadata stored alongside cursor picture resources.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodedPicture {
    pub codec: EncodedPictureCodec,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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

    pub fn decode(&self) -> Result<Picture> {
        match self.codec {
            EncodedPictureCodec::JxlRgb565 => Picture::load_jxl_rgb565(&self.bytes),
            EncodedPictureCodec::JxlRgba565Keyed => Picture::load_jxl_rgba565_keyed(&self.bytes),
        }
    }
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// Bookkeeping for a resource's origin on disk, used for recovery after
/// [`ResourceManager::dismiss_resource`].
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ResourceFileEntry {
    file_path: String,
    file_offset: u64,
    resource_type: [u8; 4],
}

const RES_VERSION_100: u32 = 0x0100;

fn merge_resource_manager(dst: &mut ResourceManager, src: &ResourceManager) {
    dst.extend_from(src);
}

// ---------------------------------------------------------------------------
// Free reader functions — parse resource payloads from an open stream
// ---------------------------------------------------------------------------

/// Read a single-picture resource (`PIC `).
/// Returns `(flags, pictures)`.
fn read_single_picture(file: &mut SbFile) -> Result<(u32, Vec<Option<Picture>>)> {
    let flags = read_u32(file)?;
    let pic = Picture::load_sixteen_from_stream(file)?;
    Ok((flags, vec![Some(pic)]))
}

/// Read a picture-collection resource (`PICC`).
fn read_picture_collection(file: &mut SbFile) -> Result<(u32, Vec<Option<Picture>>)> {
    let flags = read_u32(file)?;
    let count = read_u32(file)? as usize;
    let mut pics = Vec::with_capacity(count);
    for _ in 0..count {
        pics.push(Some(Picture::load_sixteen_from_stream(file)?));
    }
    Ok((flags, pics))
}

/// Read a "flagged" picture resource (BTTN, TOGL, NPTF, SLID, RDO).
/// `count` is the fixed number of sub-pictures for this widget type.
/// A bitmask controls which sub-pictures are actually present in the stream.
fn read_flagged_pictures(file: &mut SbFile, count: usize) -> Result<(u32, Vec<Option<Picture>>)> {
    let flags = read_u32(file)?;
    let bitmask = read_u32(file)?;
    let mut pics = Vec::with_capacity(count);
    for i in 0..count {
        if bitmask & (1 << i) != 0 {
            pics.push(Some(Picture::load_sixteen_from_stream(file)?));
        } else {
            pics.push(None);
        }
    }
    Ok((flags, pics))
}

/// Read a cursor resource (`CUR `).
fn read_cursor(file: &mut SbFile) -> Result<(u32, MouseEntry, Vec<Option<Picture>>)> {
    let flags = read_u32(file)?;
    let mouse_flags = read_u16(file)?;
    let x = read_u16(file)?;
    let y = read_u16(file)?;
    let frame_length = read_u16(file)?;
    let count = read_u32(file)? as usize;

    let mut pics = Vec::with_capacity(count);
    for _ in 0..count {
        pics.push(Some(Picture::load_sixteen_from_stream(file)?));
    }

    let entry = MouseEntry {
        hotspot: CursorHotspot::new(x as f32, y as f32),
        flags: mouse_flags,
        frame_length,
    };
    Ok((flags, entry, pics))
}

/// Read a string-table resource (`TEXT`).
/// Strings are UCS-2 (u16 per char) on disk; we convert to UTF-8.
fn read_string_table(file: &mut SbFile) -> Result<Vec<String>> {
    let _flags = read_u32(file)?;
    let count = read_u16(file)? as usize;
    let mut strings = Vec::with_capacity(count);

    for _ in 0..count {
        let char_count = read_u16(file)? as usize;
        let mut chars = Vec::with_capacity(char_count);
        for _ in 0..char_count {
            chars.push(read_u16(file)?);
        }
        strings.push(String::from_utf16(&chars).unwrap_or_default());
    }
    Ok(strings)
}

/// Read a wave-table resource (`WAVE`).
/// Entries are narrow (ASCII) path strings on disk.
fn read_wave_table(file: &mut SbFile) -> Result<Vec<String>> {
    let _flags = read_u32(file)?;
    let count = read_u16(file)? as usize;
    let mut waves = Vec::with_capacity(count);

    for _ in 0..count {
        let str_size = read_u16(file)? as usize;
        // Wave-path strings are capped at 4096 bytes: read 4096, skip the rest.
        let read_size = str_size.min(4096);
        let buf = read_bytes(file, read_size)?;
        if str_size > 4096 {
            tracing::warn!("read_wave_table: string size {str_size} > 4096, truncating");
            file.skip((str_size - 4096) as i64, 1);
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
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ResourceManager {
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
    /// Reference counts per resource.
    references: HashMap<ResourceId, u32>,
    /// On-disk locations for recovery after dismiss.
    file_entries: HashMap<ResourceId, ResourceFileEntry>,
}

impl ResourceManager {
    pub fn new() -> Self {
        Self {
            pictures: HashMap::new(),
            encoded_pictures: HashMap::new(),
            mouse_entries: HashMap::new(),
            strings: HashMap::new(),
            waves: HashMap::new(),
            references: HashMap::new(),
            file_entries: HashMap::new(),
        }
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
        if let Some(dd) = shipping {
            // Keys in shipping.res_files omit any `Data/` prefix.
            let rel = path.strip_prefix("Data/").unwrap_or(path);
            if let Some(src) = dd.res_files.get(rel) {
                tracing::info!("Resource file {rel}: loaded from shipping datadir");
                merge_resource_manager(self, src);
                return Ok(());
            }
        }
        self.attach_resource_file(path)
    }

    /// Open a `.res` file and load all resources into memory.
    pub fn attach_resource_file(&mut self, path: &str) -> Result<()> {
        let mut file =
            SbFile::open(path, 0).map_err(|e| anyhow!("open resource file '{path}': error {e}"))?;

        // Validate magic
        let magic = read_tag(&mut file)?;
        if &magic != b"SRES" {
            bail!(
                "not a resource file (bad magic {:?})",
                std::str::from_utf8(&magic).unwrap_or("????")
            );
        }

        // Version
        file.serialize_version()
            .map_err(|e| anyhow!("read version: error {e}"))?;
        let version = file.get_version();

        match version {
            RES_VERSION_100 => self.load_file_resource_v100(&mut file, path),
            _ => bail!("unsupported resource file version: 0x{version:04X}"),
        }
    }

    fn load_file_resource_v100(&mut self, file: &mut SbFile, file_path: &str) -> Result<()> {
        let num_resources = read_u32(file)?;

        for _ in 0..num_resources {
            let type_tag = read_tag(file)?;
            let id = read_u32(file)? as ResourceId;

            // Initialize reference count
            self.references.insert(id, 0);

            // Record file entry before reading payload (offset = current pos)
            let offset = file.tell();
            self.file_entries.insert(
                id,
                ResourceFileEntry {
                    file_path: file_path.to_string(),
                    file_offset: offset,
                    resource_type: type_tag,
                },
            );

            self.load_resource_data(file, id, &type_tag)?;
        }
        Ok(())
    }

    /// Dispatch to the right reader based on the 4-byte type tag and store
    /// the results in the appropriate map(s).
    fn load_resource_data(
        &mut self,
        file: &mut SbFile,
        id: ResourceId,
        type_tag: &[u8; 4],
    ) -> Result<()> {
        match type_tag {
            b"PIC " => {
                let (_, pics) = read_single_picture(file)?;
                self.pictures.insert(id, pics);
            }
            b"PICC" => {
                let (_, pics) = read_picture_collection(file)?;
                self.pictures.insert(id, pics);
            }
            b"BTTN" => {
                let (_, pics) = read_flagged_pictures(file, 4)?;
                self.pictures.insert(id, pics);
            }
            b"TOGL" => {
                let (_, pics) = read_flagged_pictures(file, 5)?;
                self.pictures.insert(id, pics);
            }
            b"NPTF" => {
                let (_, pics) = read_flagged_pictures(file, 6)?;
                self.pictures.insert(id, pics);
            }
            b"CUR " => {
                let (_, mouse, pics) = read_cursor(file)?;
                self.pictures.insert(id, pics);
                self.mouse_entries.insert(id, mouse);
            }
            b"TEXT" => {
                let strs = read_string_table(file)?;
                self.strings.insert(id, strs);
            }
            b"WAVE" => {
                let w = read_wave_table(file)?;
                self.waves.insert(id, w);
            }
            b"SLID" => {
                let (_, pics) = read_flagged_pictures(file, 6)?;
                self.pictures.insert(id, pics);
            }
            b"RDO " => {
                let (_, pics) = read_flagged_pictures(file, 7)?;
                self.pictures.insert(id, pics);
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
        let Some(entry) = self.file_entries.get(&id) else {
            tracing::warn!("dismiss_resource: unknown id {id}");
            return;
        };
        match &entry.resource_type {
            b"PIC " | b"PICC" | b"BTTN" | b"TOGL" | b"NPTF" => {
                self.pictures.remove(&id);
            }
            _ => {}
        }
    }

    /// Re-load a resource from disk.  Called automatically by getters when the
    /// resource has been dismissed.
    fn recover_resource(&mut self, id: ResourceId) -> Result<()> {
        let entry = self
            .file_entries
            .get(&id)
            .ok_or_else(|| anyhow!("resource {id}: no file entry for recovery"))?
            .clone();

        let mut file = SbFile::open(&entry.file_path, 0)
            .map_err(|e| anyhow!("recovery open '{}': error {e}", entry.file_path))?;
        seek_to(&mut file, entry.file_offset)?;
        self.load_resource_data(&mut file, id, &entry.resource_type)
    }

    /// Ensure a picture resource is loaded (recover if dismissed).
    fn ensure_pictures_loaded(&mut self, id: ResourceId) -> Result<()> {
        if !self.pictures.contains_key(&id) {
            if let Some(encoded) = self.encoded_pictures.get(&id).cloned() {
                let mut decoded = Vec::with_capacity(encoded.len());
                for (sub_id, slot) in encoded.into_iter().enumerate() {
                    decoded.push(match slot {
                        Some(pic) => Some(
                            pic.decode()
                                .with_context(|| format!("resource {id}/{sub_id}: decode JXL"))?,
                        ),
                        None => None,
                    });
                }
                self.pictures.insert(id, decoded);
                return Ok(());
            }
            self.recover_resource(id)?;
        }
        Ok(())
    }

    /// Ensure a string resource is loaded (recover if missing).
    fn ensure_strings_loaded(&mut self, id: ResourceId) -> Result<()> {
        if !self.strings.contains_key(&id) {
            self.recover_resource(id)?;
        }
        Ok(())
    }

    /// Ensure a wave resource is loaded (recover if missing).
    fn ensure_waves_loaded(&mut self, id: ResourceId) -> Result<()> {
        if !self.waves.contains_key(&id) {
            self.recover_resource(id)?;
        }
        Ok(())
    }

    /// Ensure mouse entry is loaded.
    fn ensure_mouse_loaded(&mut self, id: ResourceId) -> Result<()> {
        if !self.mouse_entries.contains_key(&id) {
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
        self.pictures
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
        self.pictures
            .get(&id)
            .map(|v| v.as_slice())
            .ok_or_else(|| anyhow!("resource {id}: not found"))
    }

    /// Number of sub-pictures in a collection.
    pub fn get_picture_count(&mut self, id: ResourceId) -> Result<usize> {
        self.ensure_pictures_loaded(id)?;
        self.pictures
            .get(&id)
            .map(|v| v.len())
            .ok_or_else(|| anyhow!("resource {id}: not found"))
    }

    /// Maximum (width, height) across all sub-pictures of a resource.
    pub fn get_dimension(&mut self, id: ResourceId) -> Result<(u16, u16)> {
        self.ensure_pictures_loaded(id)?;
        let pics = self
            .pictures
            .get(&id)
            .ok_or_else(|| anyhow!("resource {id}: not found"))?;

        let mut max_w: u16 = 0;
        let mut max_h: u16 = 0;
        for pic in pics.iter().flatten() {
            max_w = max_w.max(pic.width);
            max_h = max_h.max(pic.height);
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
        self.ensure_strings_loaded(id)?;
        let strings = self
            .strings
            .get(&id)
            .ok_or_else(|| anyhow!("string resource {id}: not found"))?;
        strings
            .get(sub_id)
            .map(|s| s.as_str())
            .ok_or_else(|| anyhow!("string resource {id}: sub_id {sub_id} out of range"))
    }

    /// Number of strings in a string-table resource.
    pub fn get_string_count(&mut self, id: ResourceId) -> Result<usize> {
        self.ensure_strings_loaded(id)?;
        self.strings
            .get(&id)
            .map(|v| v.len())
            .ok_or_else(|| anyhow!("string resource {id}: not found"))
    }

    /// Get a wave/sound path by resource ID and sub-index.
    pub fn get_sample(&mut self, id: ResourceId, sub_id: usize) -> Result<&str> {
        self.ensure_waves_loaded(id)?;
        let waves = self
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
        self.mouse_entries
            .get(&id)
            .ok_or_else(|| anyhow!("mouse resource {id}: not found"))
    }

    // ===================================================================
    // Reference counting
    // ===================================================================

    /// Increment the reference count for a resource.
    pub fn add_reference(&mut self, id: ResourceId) -> Result<()> {
        let count = self
            .references
            .get_mut(&id)
            .ok_or_else(|| anyhow!("add_reference: resource {id} not found"))?;
        *count += 1;
        Ok(())
    }

    /// Decrement the reference count.  When it reaches zero the resource's
    /// picture data is dismissed (evicted from memory).
    pub fn release_reference(&mut self, id: ResourceId) -> Result<()> {
        let count = self
            .references
            .get_mut(&id)
            .ok_or_else(|| anyhow!("release_reference: resource {id} not found"))?;
        if *count == 0 {
            bail!("release_reference: resource {id} already at zero");
        }
        *count -= 1;
        if *count == 0 {
            self.dismiss_resource(id);
        }
        Ok(())
    }

    // ===================================================================
    // Existence queries (non-mutating)
    // ===================================================================

    /// True if a picture (or picture-like) resource is loaded or registered.
    pub fn has_picture_resource(&self, id: ResourceId) -> bool {
        self.pictures.contains_key(&id)
            || self.encoded_pictures.contains_key(&id)
            || self.references.contains_key(&id)
    }

    /// True if a text resource is loaded or registered.
    pub fn has_text_resource(&self, id: ResourceId) -> bool {
        self.strings.contains_key(&id) || self.references.contains_key(&id)
    }

    /// Iterate over all loaded resources. Yields `(id, type_tag)`.
    pub fn iter_entries(&self) -> impl Iterator<Item = (ResourceId, [u8; 4])> + '_ {
        self.file_entries
            .iter()
            .map(|(&id, e)| (id, e.resource_type))
    }

    /// Borrow the raw picture list for a loaded id, if any.
    pub fn pictures_raw(&self, id: ResourceId) -> Option<&Vec<Option<Picture>>> {
        self.pictures.get(&id)
    }

    /// Replace currently loaded picture payloads with encoded shipping
    /// payloads. Non-picture resource metadata stays intact.
    pub fn encode_pictures_for_shipping<F>(&mut self, mut encode: F) -> Result<usize>
    where
        F: FnMut(&Picture) -> Result<EncodedPicture>,
    {
        let ids: Vec<ResourceId> = self.pictures.keys().copied().collect();
        let mut encoded_count = 0usize;
        for id in ids {
            let Some(pictures) = self.pictures.remove(&id) else {
                continue;
            };
            let mut encoded_slots = Vec::with_capacity(pictures.len());
            for (sub_id, slot) in pictures.into_iter().enumerate() {
                encoded_slots.push(match slot {
                    Some(pic) => {
                        encoded_count += 1;
                        Some(encode(&pic).with_context(|| {
                            format!("resource {id}/{sub_id}: encode picture for shipping")
                        })?)
                    }
                    None => None,
                });
            }
            self.encoded_pictures.insert(id, encoded_slots);
        }
        Ok(encoded_count)
    }

    /// Borrow the string list for a loaded id, if any.
    pub fn strings_raw(&self, id: ResourceId) -> Option<&Vec<String>> {
        self.strings.get(&id)
    }

    /// Borrow the wave path list for a loaded id, if any.
    pub fn waves_raw(&self, id: ResourceId) -> Option<&Vec<String>> {
        self.waves.get(&id)
    }

    /// Borrow the mouse cursor metadata for a loaded id, if any.
    pub fn mouse_entry(&self, id: ResourceId) -> Option<&MouseEntry> {
        self.mouse_entries.get(&id)
    }

    /// Sorted list of `(resource_id, type_tag)` for every loaded resource.
    /// Used by the shipping converter to walk the manager in a stable order
    /// when re-serializing as a `.res` byte blob.
    pub fn resource_ids_with_types(&self) -> Vec<(ResourceId, [u8; 4])> {
        let mut out: Vec<(ResourceId, [u8; 4])> = self
            .file_entries
            .iter()
            .map(|(&id, e)| (id, e.resource_type))
            .collect();
        out.sort_by_key(|(id, _)| *id);
        out
    }

    /// Re-serialize this `ResourceManager` to the on-disk `.res` byte format,
    /// emitting every embedded `SBPictureSixteen` with the chosen `packing`.
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
        out.extend_from_slice(&(ids.len() as u32).to_le_bytes());

        for (id, tag) in &ids {
            out.extend_from_slice(tag);
            out.extend_from_slice(&(*id as u32).to_le_bytes());
            match tag {
                b"PIC " => {
                    out.extend_from_slice(&0u32.to_le_bytes()); // flags
                    let pics = self.pictures.get(id).ok_or_else(|| {
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
                        .pictures
                        .get(id)
                        .ok_or_else(|| anyhow!("PICC {id}: missing"))?;
                    out.extend_from_slice(&0u32.to_le_bytes());
                    out.extend_from_slice(&(pics.len() as u32).to_le_bytes());
                    for slot in pics {
                        let pic = slot
                            .as_ref()
                            .ok_or_else(|| anyhow!("PICC {id}: missing sub-picture"))?;
                        out.extend(pic.write_sixteen_to_bytes(packing)?);
                    }
                }
                b"BTTN" | b"TOGL" | b"NPTF" | b"SLID" | b"RDO " => {
                    let pics = self
                        .pictures
                        .get(id)
                        .ok_or_else(|| anyhow!("{tag:?} {id}: missing"))?;
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
                        .pictures
                        .get(id)
                        .ok_or_else(|| anyhow!("CUR {id}: missing"))?;
                    let mouse = self
                        .mouse_entries
                        .get(id)
                        .ok_or_else(|| anyhow!("CUR {id}: missing mouse"))?;
                    out.extend_from_slice(&0u32.to_le_bytes()); // flags
                    out.extend_from_slice(&mouse.flags.to_le_bytes());
                    out.extend_from_slice(&(mouse.hotspot.x as u16).to_le_bytes());
                    out.extend_from_slice(&(mouse.hotspot.y as u16).to_le_bytes());
                    out.extend_from_slice(&mouse.frame_length.to_le_bytes());
                    out.extend_from_slice(&(pics.len() as u32).to_le_bytes());
                    for slot in pics {
                        let pic = slot
                            .as_ref()
                            .ok_or_else(|| anyhow!("CUR {id}: missing sub-picture"))?;
                        out.extend(pic.write_sixteen_to_bytes(packing)?);
                    }
                }
                b"TEXT" => {
                    let strs = self
                        .strings
                        .get(id)
                        .ok_or_else(|| anyhow!("TEXT {id}: missing"))?;
                    out.extend_from_slice(&0u32.to_le_bytes()); // flags
                    out.extend_from_slice(&(strs.len() as u16).to_le_bytes());
                    for s in strs {
                        let utf16: Vec<u16> = s.encode_utf16().collect();
                        out.extend_from_slice(&(utf16.len() as u16).to_le_bytes());
                        for c in &utf16 {
                            out.extend_from_slice(&c.to_le_bytes());
                        }
                    }
                }
                b"WAVE" => {
                    let waves = self
                        .waves
                        .get(id)
                        .ok_or_else(|| anyhow!("WAVE {id}: missing"))?;
                    out.extend_from_slice(&0u32.to_le_bytes()); // flags
                    out.extend_from_slice(&(waves.len() as u16).to_le_bytes());
                    for w in waves {
                        // Original on-disk size includes the trailing NUL byte
                        // when the C side stored it; emit raw ASCII bytes
                        // verbatim. Length-prefixed, no NUL terminator added.
                        out.extend_from_slice(&(w.len() as u16).to_le_bytes());
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

    /// Take ownership of the internal maps so a shipping-datadir source
    /// can be spliced in wholesale. Only used by the shipping loader; the
    /// runtime doesn't otherwise need to reach past the accessors above.
    pub(crate) fn extend_from(&mut self, src: &ResourceManager) {
        // Entries from `src` overwrite any existing ids with the same key.
        self.pictures
            .extend(src.pictures.iter().map(|(k, v)| (*k, v.clone())));
        self.encoded_pictures
            .extend(src.encoded_pictures.iter().map(|(k, v)| (*k, v.clone())));
        self.mouse_entries
            .extend(src.mouse_entries.iter().map(|(k, v)| (*k, v.clone())));
        self.strings
            .extend(src.strings.iter().map(|(k, v)| (*k, v.clone())));
        self.waves
            .extend(src.waves.iter().map(|(k, v)| (*k, v.clone())));
        self.references
            .extend(src.references.iter().map(|(k, v)| (*k, *v)));
        self.file_entries
            .extend(src.file_entries.iter().map(|(k, v)| (*k, v.clone())));
    }

    /// Dump all resources as a JSON value.
    /// Picture pixel data is omitted — only dimensions and format are included.
    pub fn dump_json(&self) -> serde_json::Value {
        let mut resources = BTreeMap::new();

        for (&id, entry) in &self.file_entries {
            let type_tag = std::str::from_utf8(&entry.resource_type)
                .unwrap_or("????")
                .trim()
                .to_string();

            let data = match entry.resource_type {
                _ if self.strings.contains_key(&id) => {
                    let strings = &self.strings[&id];
                    serde_json::json!({
                        "type": type_tag,
                        "count": strings.len(),
                        "strings": strings,
                    })
                }
                _ if self.waves.contains_key(&id) => {
                    let waves = &self.waves[&id];
                    serde_json::json!({
                        "type": type_tag,
                        "count": waves.len(),
                        "paths": waves,
                    })
                }
                _ if self.pictures.contains_key(&id) => {
                    let pics = &self.pictures[&id];
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
                    if let Some(mouse) = self.mouse_entries.get(&id) {
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
    fn new_manager_is_empty() {
        let mgr = ResourceManager::new();
        assert!(!mgr.has_picture_resource(1));
        assert!(!mgr.has_text_resource(1));
    }

    #[test]
    fn dismiss_nonexistent_is_noop() {
        let mut mgr = ResourceManager::new();
        mgr.dismiss_resource(42); // should not panic
    }

    #[test]
    fn reference_counting() {
        let mut mgr = ResourceManager::new();
        mgr.references.insert(1, 0);
        mgr.add_reference(1).unwrap();
        mgr.add_reference(1).unwrap();
        assert_eq!(mgr.references[&1], 2);
        mgr.release_reference(1).unwrap();
        assert_eq!(mgr.references[&1], 1);
    }

    #[test]
    fn release_at_zero_is_error() {
        let mut mgr = ResourceManager::new();
        mgr.references.insert(1, 0);
        assert!(mgr.release_reference(1).is_err());
    }
}
