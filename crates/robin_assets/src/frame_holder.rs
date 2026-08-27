//! Sprite frame storage, dictionary-based decompression, and cache management.
//!
//! Manages:
//! - A bank of packed (run-length or vector-quantized) sprites loaded from
//!   `robinhood.bks` / `robinhood.dic`.
//! - Color lookup dictionaries for vector-quantized sprites, with day/night/fog
//!   variants.
//! - A paging/cache system for on-demand sprite loading.
//! - Pixel-level decompression and color effects.
//!
//! Uses standard Rust allocation (`Vec<u16>`) for sprite data instead of
//! a custom memory manager.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::binary_reader::{Reader, checked_range};
use robin_engine::sbfile::SbFile;

// ---------------------------------------------------------------------------
// SpriteVariant
// ---------------------------------------------------------------------------

// SpriteVariant lives in robin_engine (Decision 3C). Re-exported here
// for backward-compat with existing callers.
pub use robin_engine::sprite_variant::SpriteVariant;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Shadow key color in 16-bit RGB565 (blue channel only).
pub const SHADOW_KEY: u16 = 0x001F;

/// Transparent color in 16-bit RGB565.
pub const TRANSPARENT_COLOR_16: u16 = 0x07C0;

/// Transparent color in 15-bit RGB555.
pub const TRANSPARENT_COLOR_15: u16 = 0x03E0;

/// Default fog color (white).
pub const FOG_COLOR: u16 = 0xFFFF;

/// Default fog intensity percentage.
pub const FOG_INTENSITY: u16 = 60;

/// Default night effect intensity.
pub const NIGHT_INTENSITY: u16 = 50;

/// Night fog color in RGB565 — `CreateColor(0, 30, 60)` packed.
/// R: 0, G: (30 & 0xFC) << 3 = 0xE0, B: 60 >> 3 = 7.
pub const NIGHT_FOG_COLOR_16: u16 = 0x00E7;

// ---------------------------------------------------------------------------
// PackedSprite
// ---------------------------------------------------------------------------

/// A single sprite's packed (compressed) pixel data and metadata.
///
/// Sprites can be in two packed formats:
/// - **Run-length** (dictionary_index == `UNMAPPED_DICT`): per-scanline start/end
///   indices followed by pixel data.
/// - **Vector-quantized** (dictionary_index is valid): per-scanline groups of 4
///   pixels indexed into a [`FrameDictionary`].
///
/// The whole bank is eager-loaded at mission load and we rely on the OS page
/// cache instead of a custom paging system.  `SHADOW_KEY` pixels are replaced
/// with the ambient shadow colour during decompression, so no separate
/// pre-application step is needed.
///
/// Runtime-loaded resource sprites (per-feature renderers in `markers.rs` /
/// `titbit_renderer.rs`) build GPU textures directly from `ResourceManager`
/// pictures and never touch the shared `FrameHolder`, so the plain non-paged
/// RLE branch is unreachable here and [`FrameHolder::uncompress_frame`]
/// always takes the ArnoLaw path for RLE sprites.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackedSprite {
    pub width: u16,
    pub height: u16,
    /// Size of packed data in bytes.
    pub packed_size: u32,
    /// Owned packed pixel data, for sprites that don't live in the bank
    /// file (shipping datadirs, runtime-added PNG overlays). Bank-backed
    /// sprites keep this `None` and use [`Self::bank_span`] instead —
    /// resolve either through [`FrameHolder::packed_data`].
    #[serde(skip)]
    pub packed_data: Option<Vec<u16>>,
    /// Word range of this sprite's packed data in the shared bank
    /// storage, when the data is bank-backed rather than owned.
    #[serde(skip)]
    pub bank_span: Option<BankSpan>,
    /// Original runtime-loaded RGBA pixels, when this sprite came from a PNG
    /// overlay instead of the legacy bank.
    #[serde(skip)]
    pub rgba_data: Option<Vec<u8>>,
    /// Index into the dictionary table, or [`UNMAPPED_DICT`] for RLE sprites.
    pub dictionary_index: u16,
}

/// Sentinel: no dictionary (run-length encoded sprite).
pub const UNMAPPED_DICT: u16 = 0xFFFF;

/// A sprite's packed-data range in the shared bank storage, in u16 words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankSpan {
    pub offset_words: u32,
    pub len_words: u32,
}

/// Backing storage for the sprite bank's packed pixel data.
///
/// The fullgame bank is ~600 MB while a mission touches well under 1%
/// of it (measured ~0.3% for a full Lincoln session), so the
/// loose-datadir path memory-maps the file and sprites hold word spans
/// into the map — pages fault in only for sprites actually drawn.
/// Owned storage is the fallback when no native file path exists (wasm,
/// zip-overlay datadirs).
#[derive(Debug)]
enum BankStorage {
    #[cfg(not(target_arch = "wasm32"))]
    Mapped(memmap2::Mmap),
    Owned(Vec<u16>),
}

impl BankStorage {
    fn words(&self) -> &[u16] {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            BankStorage::Mapped(map) => bytemuck::try_cast_slice(&map[..])
                .expect("bank mapping was validated as even-length at load; maps are page-aligned"),
            BankStorage::Owned(words) => words,
        }
    }
}

/// Open the sprite bank's backing storage: memory-map the native file
/// when one resolves, otherwise fall back to reading it fully into
/// memory (wasm, zip-overlay datadirs).
fn open_bank_storage(bks_path: &str) -> Result<BankStorage> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(native) = robin_engine::sbfile::resolve_data_path(bks_path) {
        let file = std::fs::File::open(&native)
            .with_context(|| format!("open sprite bank '{}'", native.display()))?;
        // SAFETY: read-only shared mapping. The datadir is treated as
        // immutable while the game runs; truncating/rewriting the bank
        // underneath a running game is out of contract.
        let map = unsafe { memmap2::Mmap::map(&file) }
            .with_context(|| format!("mmap sprite bank '{}'", native.display()))?;
        if !map.len().is_multiple_of(2) {
            return Err(anyhow!(
                "sprite bank '{}' must contain 16-bit words (odd byte length {})",
                native.display(),
                map.len()
            ));
        }
        tracing::info!(
            "Sprite bank: memory-mapped {} ({:.0} MB)",
            native.display(),
            map.len() as f64 / 1e6
        );
        return Ok(BankStorage::Mapped(map));
    }

    let bytes = SbFile::open(bks_path, 0)
        .map_err(|e| anyhow!("open sprite bank '{bks_path}': error {e}"))?
        .into_bytes();
    if !bytes.len().is_multiple_of(2) {
        return Err(anyhow!(
            "sprite bank must contain 16-bit words (odd byte length {})",
            bytes.len()
        ));
    }
    // One copy into an owned, aligned word buffer. cast_slice assumes LE
    // host byte order, matching the only targets we ship.
    let words = match bytemuck::try_cast_slice::<u8, u16>(&bytes) {
        Ok(words) => words.to_vec(),
        Err(_) => bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect(),
    };
    Ok(BankStorage::Owned(words))
}

impl Default for PackedSprite {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            packed_size: 0,
            packed_data: None,
            bank_span: None,
            rgba_data: None,
            dictionary_index: UNMAPPED_DICT,
        }
    }
}

// ---------------------------------------------------------------------------
// BankSpriteIndex — file format struct from robinhood.dic
// ---------------------------------------------------------------------------

/// On-disk index entry for a sprite in the bank file (pack 2, 14 bytes).
#[derive(Debug, Clone, Copy)]
pub struct BankSpriteIndex {
    pub width: u16,
    pub height: u16,
    pub position: u32,
    pub size: u32,
    pub dictionary: u16,
}

impl BankSpriteIndex {
    /// Read one complete little-endian on-disk index record.
    pub fn from_le_bytes(data: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(data);
        Ok(Self {
            width: reader.u16("sprite index width")?,
            height: reader.u16("sprite index height")?,
            position: reader.u32("sprite index bank position")?,
            size: reader.u32("sprite index packed size")?,
            dictionary: reader.u16("sprite index dictionary")?,
        })
    }

    pub const PACKED_SIZE: usize = 14; // 2+2+4+4+2
}

fn bank_span_for_index(
    bank_len_words: usize,
    index: &BankSpriteIndex,
    sprite_index: usize,
) -> Result<Option<BankSpan>> {
    if !index.position.is_multiple_of(2) || !index.size.is_multiple_of(2) {
        return Err(anyhow!(
            "sprite index record {sprite_index}: bank byte range {}..+{} is not 16-bit aligned",
            index.position,
            index.size
        ));
    }
    if index.size == 0 {
        return Ok(None);
    }

    let word_offset = usize::try_from(index.position / 2)
        .context("sprite bank word offset does not fit usize")?;
    let word_count =
        usize::try_from(index.size / 2).context("sprite bank word count does not fit usize")?;
    checked_range(
        word_offset,
        word_count,
        bank_len_words,
        format!("sprite index record {sprite_index} bank range"),
    )?;
    Ok(Some(BankSpan {
        offset_words: index.position / 2,
        len_words: index.size / 2,
    }))
}

// ---------------------------------------------------------------------------
// FrameDictionary
// ---------------------------------------------------------------------------

/// A color lookup dictionary for vector-quantized sprites.
///
/// Each entry is 4 consecutive u16 pixels (stored as a u64 / `UOCTA`).
/// During decompression, a single dictionary index is expanded to 4 pixels.
#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct FrameDictionary {
    /// Number of entries (each entry = 4 pixels = 8 bytes).
    num_entries: u16,
    /// The raw dictionary data: `num_entries` groups of 4 u16 pixels,
    /// stored flat as `[u16; num_entries * 4]`.
    values: Vec<u16>,
    /// CRC32 checksum for deduplication.
    checksum: u32,
    /// Shadow color currently in use.
    shadow_color: u16,
    /// Whether night color effect has been applied.
    night_applied: bool,
    /// Whether 16→15 bit conversion has been applied.
    converted_to_15bit: bool,
    /// Whether fog color effect has been applied.
    fog_applied: bool,
}

impl Default for FrameDictionary {
    fn default() -> Self {
        Self {
            num_entries: 0,
            values: Vec::new(),
            checksum: 0,
            shadow_color: SHADOW_KEY,
            night_applied: false,
            converted_to_15bit: false,
            fog_applied: false,
        }
    }
}

impl FrameDictionary {
    /// Create from raw u16 data read from the bank index file.
    ///
    /// `data` must contain exactly `num_entries * 4` u16 values.
    pub fn from_raw(num_entries: u16, data: Vec<u16>) -> Self {
        assert_eq!(data.len(), num_entries as usize * 4);

        // Calculate CRC32 checksum over the raw bytes
        let byte_data: Vec<u8> = data.iter().flat_map(|w| w.to_le_bytes()).collect();
        let checksum = crc32_hash(&byte_data);

        let mut dict = Self {
            num_entries,
            values: data,
            checksum,
            shadow_color: SHADOW_KEY,
            ..Default::default()
        };

        // Fix near-transparent colors (0x7E0 → 0x7C0).
        for v in &mut dict.values {
            if *v == 0x07E0 {
                *v = TRANSPARENT_COLOR_16;
            }
        }

        dict
    }

    pub fn checksum(&self) -> u32 {
        self.checksum
    }

    pub fn num_entries(&self) -> u16 {
        self.num_entries
    }

    pub fn shadow_color(&self) -> u16 {
        self.shadow_color
    }

    /// Get the raw u16 data for a given index (4 pixels).
    pub fn lookup_pixels(&self, index: u16) -> &[u16] {
        let base = index as usize * 4;
        &self.values[base..base + 4]
    }

    /// Apply the "Arno law" shadow color replacement.
    ///
    /// Replaces old shadow key (`0x1F`) with the new shadow color, bumping
    /// any pixels that happen to equal the new shadow color.
    pub fn apply_arno_law(&mut self, new_shadow_color: u16) -> bool {
        if new_shadow_color == self.shadow_color {
            return true;
        }

        for v in &mut self.values {
            if *v == new_shadow_color {
                *v += 1;
            }
            if *v == self.shadow_color {
                *v = new_shadow_color;
            }
        }

        self.shadow_color = new_shadow_color;
        true
    }

    /// Apply fog blending effect at the given intensity and fog color.
    pub fn apply_fog_effect(&mut self, level: u16, fog_color: u16) -> bool {
        if self.fog_applied {
            return true;
        }

        apply_fog_blend_16(&mut self.values, level, fog_color, self.shadow_color);
        self.fog_applied = true;
        true
    }

    /// Create a variant copy with the specified effect applied.
    pub fn with_variant(base: &FrameDictionary, variant: SpriteVariant) -> Self {
        let mut dict = base.clone();

        match variant {
            SpriteVariant::Night => {
                dict.apply_fog_effect(NIGHT_INTENSITY, NIGHT_FOG_COLOR_16);
            }
            SpriteVariant::Fog => {
                dict.apply_fog_effect(FOG_INTENSITY, FOG_COLOR);
            }
            SpriteVariant::Day => {
                // No effect
            }
        }

        dict
    }
}

// ---------------------------------------------------------------------------
// FrameHolder
// ---------------------------------------------------------------------------

/// Central sprite bank manager: stores packed sprites, dictionaries, and
/// handles decompression and caching.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FrameHolder {
    /// All packed sprites loaded from the bank.
    sprites: Vec<PackedSprite>,

    /// Day (default) dictionaries.
    dictionaries: Vec<FrameDictionary>,
    /// Night variant dictionaries (generated on demand).
    dictionaries_night: Vec<FrameDictionary>,
    /// Fog variant dictionaries (generated on demand).
    dictionaries_fog: Vec<FrameDictionary>,

    /// Global shadow intensity.
    shadow: u16,
    /// Global "blip" shadow intensity (for highlighted characters).
    blip_shadow: u16,

    /// Bank file signature for validation.
    signature: u32,

    /// Shared backing storage for bank-backed sprites' packed data.
    /// `None` until a bank is loaded, or when every sprite owns its
    /// data (shipping datadirs). Shared across clones.
    #[serde(skip)]
    bank: Option<Arc<BankStorage>>,

    /// Runtime statistics of which bank sprites actually get read.
    /// Shared across clones (the engine's pixel-opacity handle counts
    /// into the same set as the render path); replaced on bank reload.
    #[serde(skip)]
    usage: Arc<UsageTracker>,
}

/// Atomically published immutable [`FrameHolder`] generations for engine-side
/// pixel hit testing.
///
/// Rendering reads the host's current `Arc<FrameHolder>` directly. The engine
/// cannot own that concrete asset type, so [`PixelOpacityLookup`] points at this
/// synchronized publisher instead. Ambiance shadow-key changes are applied to
/// a new COW generation and then published here, keeping every cloned
/// `LevelAssets` handle on the same dictionary generation as the renderer.
///
/// [`PixelOpacityLookup`]: robin_engine::engine::PixelOpacityLookup
#[derive(Debug)]
pub struct PublishedFrameHolder {
    current: RwLock<Arc<FrameHolder>>,
}

impl PublishedFrameHolder {
    pub fn new(current: Arc<FrameHolder>) -> Self {
        Self {
            current: RwLock::new(current),
        }
    }

    /// Publish the renderer's new immutable frame-holder generation.
    pub fn publish(&self, current: Arc<FrameHolder>) {
        *self
            .current
            .write()
            .expect("published frame-holder write lock poisoned") = current;
    }

    /// Snapshot the exact generation currently used by engine hit testing.
    pub fn snapshot(&self) -> Arc<FrameHolder> {
        Arc::clone(
            &self
                .current
                .read()
                .expect("published frame-holder read lock poisoned"),
        )
    }
}

/// Tracks which bank sprites have had their packed data read, and how many
/// packed bytes that covers.
///
/// Answers "how much of the bank does a level actually touch" — the number
/// that decides whether lazy (mmap-backed) bank loading is worth pursuing.
/// Recording is one relaxed `fetch_or` per read; a running summary is
/// emitted on the `sprite_bank_usage` debug target every 1024 new sprites
/// (`RUST_LOG=sprite_bank_usage=debug`).
#[derive(Debug, Default)]
pub struct UsageTracker {
    /// One bit per sprite index, set on first read.
    bits: OnceLock<Vec<AtomicU64>>,
    unique: AtomicUsize,
    bytes: AtomicU64,
    /// Total packed bytes in the bank, stamped at load.
    total_bytes: AtomicU64,
}

impl UsageTracker {
    fn record(&self, sprite_index: usize, packed_size: u32, num_sprites: usize) {
        let bits = self.bits.get_or_init(|| {
            std::iter::repeat_with(|| AtomicU64::new(0))
                .take(num_sprites.div_ceil(64))
                .collect()
        });
        let Some(word) = bits.get(sprite_index / 64) else {
            return;
        };
        let mask = 1u64 << (sprite_index % 64);
        if word.fetch_or(mask, Ordering::Relaxed) & mask != 0 {
            return; // already counted
        }
        let unique = self.unique.fetch_add(1, Ordering::Relaxed) + 1;
        let bytes =
            self.bytes.fetch_add(packed_size as u64, Ordering::Relaxed) + packed_size as u64;
        if unique.is_power_of_two() || unique.is_multiple_of(1024) {
            let total = self.total_bytes.load(Ordering::Relaxed).max(1);
            tracing::debug!(
                target: "sprite_bank_usage",
                unique,
                touched_mb = bytes as f64 / 1e6,
                total_mb = total as f64 / 1e6,
                pct = (bytes as f64 / total as f64) * 100.0,
                "sprite bank usage"
            );
        }
    }
}

impl FrameHolder {
    pub fn new() -> Self {
        Self {
            shadow: 40,
            blip_shadow: 60,
            ..Default::default()
        }
    }

    // -- Sprite accessors --

    /// Raw slice of every sprite entry in the bank.
    pub fn sprites(&self) -> &[PackedSprite] {
        &self.sprites
    }

    /// Raw slice of the per-bank dictionaries (variant-quantization tables).
    pub fn dictionaries(&self) -> &[FrameDictionary] {
        &self.dictionaries
    }

    pub fn num_sprites(&self) -> usize {
        self.sprites.len()
    }

    pub fn sprite_width(&self, index: u32) -> u16 {
        self.sprites[index as usize].width
    }

    pub fn sprite_height(&self, index: u32) -> u16 {
        self.sprites[index as usize].height
    }

    /// Packed pixel data for a sprite, or `None` for sentinel (zero-size)
    /// entries that were never populated.
    pub fn packed_data(&self, sprite_index: u32) -> Option<&[u16]> {
        self.record_usage(sprite_index as usize);
        self.sprite_packed_slice(sprite_index as usize)
    }

    /// Resolve a sprite's packed words: owned data first, then its span
    /// into the shared bank storage.
    fn sprite_packed_slice(&self, sprite_index: usize) -> Option<&[u16]> {
        let sprite = self.sprites.get(sprite_index)?;
        if let Some(owned) = sprite.packed_data.as_deref() {
            return Some(owned);
        }
        let span = sprite.bank_span?;
        let words = self.bank.as_ref()?.words();
        words.get(span.offset_words as usize..(span.offset_words + span.len_words) as usize)
    }

    /// Count `sprite_index` as touched in the shared usage statistics.
    fn record_usage(&self, sprite_index: usize) {
        if let Some(sprite) = self.sprites.get(sprite_index) {
            self.usage
                .record(sprite_index, sprite.packed_size, self.sprites.len());
        }
    }

    pub fn rgba_data(&self, sprite_index: u32) -> Option<&[u8]> {
        self.sprites
            .get(sprite_index as usize)
            .and_then(|sprite| sprite.rgba_data.as_deref())
    }

    /// Append a runtime-created RGBA sprite as an RLE bank entry.
    ///
    /// Hackable overlay datadirs use this for `.rhs.d` PNG frames.  Fully
    /// transparent RGBA pixels become the engine transparent key; exact blue
    /// `(0, 0, 255)` pixels become [`SHADOW_KEY`], matching converted legacy
    /// character PNGs. The source RGBA is retained so the renderer can upload
    /// PNG overlay frames without losing semi-transparent pixels.
    pub fn append_rgba_sprite(&mut self, width: u16, height: u16, rgba: &[u8]) -> u32 {
        assert_eq!(rgba.len(), width as usize * height as usize * 4);
        let mut packed = Vec::new();
        let w = width as usize;
        for y in 0..height as usize {
            let row = &rgba[y * w * 4..(y + 1) * w * 4];
            let mut first = None;
            let mut last = None;
            for x in 0..w {
                if row[x * 4 + 3] >= 128 {
                    first.get_or_insert(x);
                    last = Some(x);
                }
            }
            let Some(first) = first else {
                packed.push(0xFFFF);
                packed.push(0xFFFF);
                continue;
            };
            let last = last.unwrap();
            packed.push(first as u16);
            packed.push(last as u16);
            for x in first..=last {
                let i = x * 4;
                let color = if row[i + 3] < 128 {
                    TRANSPARENT_COLOR_16
                } else if row[i] == 0 && row[i + 1] == 0 && row[i + 2] == 255 {
                    SHADOW_KEY
                } else {
                    let r = (row[i] as u16 >> 3) & 0x1F;
                    let g = (row[i + 1] as u16 >> 2) & 0x3F;
                    let b = (row[i + 2] as u16 >> 3) & 0x1F;
                    let mut c = (r << 11) | (g << 5) | b;
                    if c == TRANSPARENT_COLOR_16 || c == SHADOW_KEY {
                        c = c.saturating_add(1);
                    }
                    c
                };
                packed.push(color);
            }
        }
        let index = self.sprites.len() as u32;
        self.sprites.push(PackedSprite {
            width,
            height,
            packed_size: (packed.len() * 2) as u32,
            packed_data: Some(packed),
            bank_span: None,
            rgba_data: Some(rgba.to_vec()),
            dictionary_index: UNMAPPED_DICT,
        });
        index
    }

    // -- Shadow --

    pub fn global_shadow(&self) -> u16 {
        self.shadow
    }

    pub fn set_global_shadow(&mut self, shadow: u16) {
        self.shadow = shadow;
    }

    pub fn global_blip_shadow(&self) -> u16 {
        self.blip_shadow
    }

    pub fn set_global_blip_shadow(&mut self, shadow: u16) {
        self.blip_shadow = shadow;
    }

    pub fn signature(&self) -> u32 {
        self.signature
    }

    // -- Dictionary management --

    /// Add a dictionary, deduplicating by checksum. Returns the index.
    pub fn add_dictionary(&mut self, dict: FrameDictionary) -> u16 {
        if dict.num_entries() == 0 {
            return UNMAPPED_DICT;
        }

        // Check if an identical dictionary already exists
        for (i, existing) in self.dictionaries.iter().enumerate() {
            if existing.checksum() == dict.checksum() {
                return i as u16;
            }
        }

        self.dictionaries.push(dict);
        (self.dictionaries.len() - 1) as u16
    }

    /// Get a reference to a dictionary by index.
    pub fn dictionary(&self, index: u16) -> Option<&FrameDictionary> {
        self.dictionaries.get(index as usize)
    }

    /// Generate night variant dictionaries from the day set.
    pub fn generate_night_dictionaries(&mut self) {
        self.dictionaries_night = self
            .dictionaries
            .iter()
            .map(|d| FrameDictionary::with_variant(d, SpriteVariant::Night))
            .collect();
    }

    /// Generate fog variant dictionaries from the day set.
    pub fn generate_fog_dictionaries(&mut self) {
        self.dictionaries_fog = self
            .dictionaries
            .iter()
            .map(|d| FrameDictionary::with_variant(d, SpriteVariant::Fog))
            .collect();
    }

    /// Drop variant dictionaries to free memory.
    pub fn drop_variant_dictionaries(&mut self, variant: SpriteVariant) {
        match variant {
            SpriteVariant::Night => self.dictionaries_night.clear(),
            SpriteVariant::Fog => self.dictionaries_fog.clear(),
            SpriteVariant::Day => {} // never drop the base set
        }
    }

    /// Apply Arno law shadow color to all dictionaries.
    pub fn apply_arno_law(&mut self, shadow_color: u16) {
        for d in &mut self.dictionaries {
            d.apply_arno_law(shadow_color);
        }
        for d in &mut self.dictionaries_fog {
            d.apply_arno_law(shadow_color);
        }
        for d in &mut self.dictionaries_night {
            d.apply_arno_law(shadow_color);
        }
    }

    // -- File I/O: sprite bank initialization --

    /// Load the sprite bank from `robinhood.dic` (index) and `robinhood.bks`
    /// (packed pixel data) in the given data directory.
    ///
    /// The `.dic` file contains:
    /// - `u32` signature
    /// - `u16` num_dictionaries, then for each: `u16` num_entries + `num_entries * 8` bytes
    /// - `u32` num_sprites, then for each: 14-byte [`BankSpriteIndex`]
    ///
    /// The `.bks` file is the raw packed sprite data referenced by byte-position offsets.
    pub fn initialize_sprite_bank(&mut self, data_dir: &str) -> Result<()> {
        self.initialize_sprite_bank_with_progress(data_dir, &mut |_| {}, None)
    }
}

/// Progress/status update emitted by the sprite-bank loader.
///
/// Carries either a unit-delta tick for smooth bar motion or a named
/// sub-phase with the local fraction at which the sub-phase ENDS
/// (matches [`LoadingScreenRenderer::set_status`] target semantics).
/// The caller maps `Phase(text, local_end)` onto the overall
/// loading-screen target range.
#[derive(Debug, Clone, Copy)]
pub enum ProgressUpdate<'a> {
    Tick(f32),
    Phase(&'a str, f32),
}

impl FrameHolder {
    /// Populate from a pre-parsed shipping sprite bank (no legacy I/O).
    /// Used by `initialize_sprite_bank_with_progress` when a shipping
    /// datadir is available.
    fn load_from_shipping(&mut self, bank: &crate::shipping_datadir::ShippingSpriteBank) {
        self.signature = bank.signature;
        self.dictionaries = bank.dictionaries.clone();
        self.bank = None;
        self.sprites.clear();
        self.sprites.reserve(bank.sprites.len());
        for slot in &bank.sprites {
            match slot {
                Some(s) => self.sprites.push(PackedSprite {
                    width: s.width,
                    height: s.height,
                    packed_size: (s.packed_data.len() * 2) as u32,
                    packed_data: Some(s.packed_data.clone()),
                    bank_span: None,
                    rgba_data: None,
                    dictionary_index: s.dictionary_index,
                }),
                None => self.sprites.push(PackedSprite::default()),
            }
        }
        self.reset_usage_tracker();
    }

    /// Same as [`initialize_sprite_bank`] but with a progress-update
    /// callback.
    ///
    /// Emits [`ProgressUpdate::Tick`] deltas for smooth bar motion and
    /// [`ProgressUpdate::Phase`] sub-phase names (mapped by the caller
    /// onto the overall loading-bar target).
    pub fn initialize_sprite_bank_with_progress(
        &mut self,
        data_dir: &str,
        progress: &mut dyn FnMut(ProgressUpdate),
        shipping: Option<&crate::shipping_datadir::ShippingDatadir>,
    ) -> Result<()> {
        // Shipping datadir short-circuits the entire .bks/.dic read.
        if let Some(dd) = shipping
            && let Some(bank) = &dd.sprite_bank
        {
            tracing::info!(
                "Sprite bank: loaded from shipping datadir ({} sprites)",
                bank.sprites.len()
            );
            self.load_from_shipping(bank);
            progress(ProgressUpdate::Tick(1.0));
            return Ok(());
        }

        progress(ProgressUpdate::Phase("Reading sprite bank file...", 0.30));
        progress(ProgressUpdate::Tick(0.5));

        let bks_path = format!("{}/Data/robinhood.bks", data_dir);
        let dic_path = format!("{}/Data/robinhood.dic", data_dir);

        let storage = open_bank_storage(&bks_path)?;
        let bank_len_words = storage.words().len();
        progress(ProgressUpdate::Tick(1.0));

        progress(ProgressUpdate::Phase(
            "Parsing sprite dictionaries...",
            0.92,
        ));
        let dic_bytes = SbFile::read_all(&dic_path)
            .map_err(|e| anyhow!("read sprite index '{dic_path}': error {e}"))?;
        self.load_sprite_index_bytes(&dic_bytes, bank_len_words, progress)?;
        self.bank = Some(Arc::new(storage));

        tracing::info!(
            "Sprite bank loaded: {} dictionaries, {} sprites",
            self.dictionaries.len(),
            self.sprites.len()
        );
        self.reset_usage_tracker();

        Ok(())
    }

    /// Start a fresh usage-statistics epoch for the just-loaded bank.
    fn reset_usage_tracker(&mut self) {
        let total: u64 = self.sprites.iter().map(|s| s.packed_size as u64).sum();
        let usage = UsageTracker::default();
        usage.total_bytes.store(total, Ordering::Relaxed);
        self.usage = Arc::new(usage);
    }

    fn load_sprite_index_bytes(
        &mut self,
        bytes: &[u8],
        bank_len_words: usize,
        progress: &mut dyn FnMut(ProgressUpdate),
    ) -> Result<()> {
        let mut reader = Reader::new(bytes);

        // Original provenance: `original-code/RHframeholder.cpp`,
        // `RHFrameHolder::InitializeSpriteBank`, reads the signature,
        // dictionary table, then packed `BankSpriteIndex` records in this order.
        self.signature = reader.u32("sprite index signature")?;

        let num_dicts = reader.u16("sprite index dictionary count")? as usize;
        reader.validate_count(num_dicts, 2, "sprite index dictionary count", 4)?;
        tracing::info!("Reading {num_dicts} dictionaries");

        let mut dict_conversion: HashMap<u16, u16> = HashMap::new();

        for i in 0..num_dicts {
            let num_entries = reader.u16(format!("dictionary {i} entry count"))?;
            // Each entry is 4 u16 pixels = 8 bytes.
            let byte_count = usize::from(num_entries) * 8;
            let raw_bytes = reader.take(byte_count, format!("dictionary {i} pixels"))?;
            let data: Vec<u16> = bytemuck::cast_slice::<u8, u16>(raw_bytes).to_vec();
            let dict = FrameDictionary::from_raw(num_entries, data);
            let real_index = self.add_dictionary(dict);
            dict_conversion.insert(i as u16, real_index);
        }
        // Sentinel: unmapped dictionary stays unmapped
        dict_conversion.insert(0xFFFF, UNMAPPED_DICT);

        progress(ProgressUpdate::Tick(1.0));
        progress(ProgressUpdate::Phase("Unpacking sprite table...", 1.0));

        // Read sprite entries
        let num_sprites =
            reader.count_u32("sprite index sprite count", BankSpriteIndex::PACKED_SIZE)?;
        tracing::info!("Reading {num_sprites} sprites");

        self.sprites.clear();
        self.sprites.reserve(num_sprites);

        for sprite_index in 0..num_sprites {
            let bytes = reader.take(
                BankSpriteIndex::PACKED_SIZE,
                format!("sprite index record {sprite_index}"),
            )?;
            let idx = BankSpriteIndex::from_le_bytes(bytes)
                .with_context(|| format!("sprite index record {sprite_index}"))?;

            let dict_index = dict_conversion
                .get(&idx.dictionary)
                .copied()
                .ok_or_else(|| {
                    anyhow!(
                        "sprite index record {sprite_index}: dictionary {} is not defined",
                        idx.dictionary
                    )
                })?;

            let bank_span = bank_span_for_index(bank_len_words, &idx, sprite_index)?;

            self.sprites.push(PackedSprite {
                width: idx.width,
                height: idx.height,
                packed_size: idx.size,
                packed_data: None,
                bank_span,
                rgba_data: None,
                dictionary_index: dict_index,
            });
        }

        Ok(())
    }

    /// Convenience constructor: create a [`FrameHolder`] and load the sprite
    /// bank from `.dic` / `.bks` files in `data_dir`.
    pub fn from_data_dir(data_dir: &str) -> Result<Self> {
        let mut holder = Self::new();
        holder
            .initialize_sprite_bank(data_dir)
            .context("initializing sprite bank")?;
        Ok(holder)
    }

    // -- Decompression (high-level) --

    /// Decompress a sprite into a pixel buffer, applying variant color effects.
    ///
    /// Detects whether the sprite uses RLE or dictionary-based compression
    /// and dispatches accordingly.
    ///
    /// For RLE sprites the `SHADOW_KEY` marker pixels are replaced with
    /// the ambient `shadow_color` during decompression (ArnoLaw).
    /// Night/fog effects are post-processed on the decompressed pixels
    /// and a 15-bit conversion is applied if `bit_depth != 16`.
    ///
    /// For dictionary (vector-quantized) sprites the appropriate variant
    /// dictionary (day/night/fog) is selected — effects are pre-baked
    /// into the variant dictionaries so no post-processing is needed.
    pub fn uncompress_frame(
        &self,
        dst: &mut [u16],
        pitch_words: usize,
        sprite_index: u32,
        variant: SpriteVariant,
        shadow_color: u16,
        bit_depth: u16,
    ) {
        let idx = sprite_index as usize;
        self.record_usage(idx);
        let sprite = &self.sprites[idx];
        let packed = self
            .sprite_packed_slice(idx)
            .expect("sprite data not loaded");
        let width = sprite.width as usize;
        let height = sprite.height as usize;
        let dict_idx = sprite.dictionary_index;

        if dict_idx == UNMAPPED_DICT {
            // RLE sprite: bake the ambient shadow colour in as we decompress.
            decompress_rle_arno_law(
                packed,
                dst,
                pitch_words,
                width,
                height,
                TRANSPARENT_COLOR_16,
                shadow_color,
            );

            // Apply variant effects on the decompressed pixels.
            // Process width*height contiguous pixels from the viewport start.
            let pixel_count = width * height;
            match variant {
                SpriteVariant::Day => {}
                SpriteVariant::Night => {
                    apply_fog_effect_viewport(
                        &mut dst[..pixel_count],
                        NIGHT_INTENSITY,
                        NIGHT_FOG_COLOR_16,
                        shadow_color,
                    );
                }
                SpriteVariant::Fog => {
                    apply_fog_effect_viewport(
                        &mut dst[..pixel_count],
                        FOG_INTENSITY,
                        FOG_COLOR,
                        shadow_color,
                    );
                }
            }

            // Convert to 15-bit if needed
            if bit_depth != 16 {
                convert_decompressed_to_15bit(dst, pitch_words, width, height, shadow_color);
            }
        } else {
            // Vector-quantized sprite — select variant dictionary
            let dict = match variant {
                SpriteVariant::Day => &self.dictionaries[dict_idx as usize],
                SpriteVariant::Night => &self.dictionaries_night[dict_idx as usize],
                SpriteVariant::Fog => &self.dictionaries_fog[dict_idx as usize],
            };
            decompress_vector(packed, dst, pitch_words, width, height, dict);
        }
    }

    /// Decompress a sprite, replacing shadow pixels with zero.
    ///
    /// Only supported for RLE sprites; vector-quantized sprites are a
    /// deliberate no-op.
    pub fn uncompress_frame_wipe_shadow(
        &self,
        dst: &mut [u16],
        pitch_words: usize,
        sprite_index: u32,
        bit_depth: u16,
    ) {
        let idx = sprite_index as usize;
        self.record_usage(idx);
        let sprite = &self.sprites[idx];
        let packed = self
            .sprite_packed_slice(idx)
            .expect("sprite data not loaded");
        let width = sprite.width as usize;
        let height = sprite.height as usize;
        let dict_idx = sprite.dictionary_index;

        if dict_idx == UNMAPPED_DICT {
            let transparent = if bit_depth == 16 {
                TRANSPARENT_COLOR_16
            } else {
                TRANSPARENT_COLOR_15
            };
            decompress_rle_wipe_shadow(
                packed,
                dst,
                pitch_words,
                width,
                height,
                transparent,
                SHADOW_KEY,
            );
        }
        // Vector-quantized: intentional no-op.
    }

    /// Decompress a sprite, replacing non-shadow/non-transparent pixels with a color.
    ///
    /// Only supported for RLE sprites; panics on vector-quantized.
    #[allow(clippy::too_many_arguments)]
    pub fn uncompress_frame_into_shadow(
        &self,
        dst: &mut [u16],
        pitch_words: usize,
        sprite_index: u32,
        variant: SpriteVariant,
        shadow_color: u16,
        replacement_color: u16,
        bit_depth: u16,
    ) {
        let idx = sprite_index as usize;
        self.record_usage(idx);
        let sprite = &self.sprites[idx];
        let packed = self
            .sprite_packed_slice(idx)
            .expect("sprite data not loaded");
        let width = sprite.width as usize;
        let height = sprite.height as usize;
        let dict_idx = sprite.dictionary_index;

        assert!(
            dict_idx == UNMAPPED_DICT,
            "uncompress_frame_into_shadow not supported for vector-quantized sprites"
        );

        let transparent = if bit_depth == 16 {
            TRANSPARENT_COLOR_16
        } else {
            TRANSPARENT_COLOR_15
        };

        decompress_rle_into_shadow(
            packed,
            dst,
            pitch_words,
            width,
            height,
            transparent,
            shadow_color,
            replacement_color,
        );

        // Apply variant effects on the decompressed pixels
        let pixel_count = width * height;
        match variant {
            SpriteVariant::Day => {}
            SpriteVariant::Night => {
                apply_fog_effect_viewport(
                    &mut dst[..pixel_count],
                    NIGHT_INTENSITY,
                    NIGHT_FOG_COLOR_16,
                    shadow_color,
                );
            }
            SpriteVariant::Fog => {
                apply_fog_effect_viewport(
                    &mut dst[..pixel_count],
                    FOG_INTENSITY,
                    FOG_COLOR,
                    shadow_color,
                );
            }
        }
    }

    // -- Per-pixel hit testing --

    /// Check if a pixel at local coordinates (`x`, `y`) within a sprite frame
    /// is opaque (hittable).
    ///
    /// Returns `true` if the pixel is not the transparent key color and
    /// not a night shadow (unless `blue_pixels_are_in` is set, the
    /// behaviour used by blipped entities).
    ///
    /// Reads directly from packed sprite data without full decompression —
    /// walks RLE scanlines or dictionary indices to the target pixel.
    pub fn is_pixel_opaque(
        &self,
        sprite_index: u32,
        x: u16,
        y: u16,
        blue_pixels_are_in: bool,
    ) -> bool {
        let idx = sprite_index as usize;
        if idx >= self.sprites.len() {
            return false;
        }
        self.record_usage(idx);
        let sprite = &self.sprites[idx];
        let Some(packed) = self.sprite_packed_slice(idx) else {
            return false;
        };

        let width = sprite.width as usize;
        let height = sprite.height as usize;
        if x as usize >= width || y as usize >= height {
            return false;
        }

        let pixel = if sprite.dictionary_index == UNMAPPED_DICT {
            rle_pixel_at(packed, x as usize, y as usize)
        } else {
            let dict = &self.dictionaries[sprite.dictionary_index as usize];
            dict_pixel_at(packed, width, x as usize, y as usize, dict)
        };

        if pixel == TRANSPARENT_COLOR_16 {
            return false;
        }

        if !blue_pixels_are_in {
            // RLE packed data still holds raw `SHADOW_KEY` markers (the
            // ArnoLaw replacement happens lazily inside
            // `decompress_rle_arno_law`). Dictionary opacity must use the
            // dictionary's own bound shadow color: accepting a separate
            // Weather color here lets a stale/partially-published ambiance
            // generation turn shadow pixels opaque.
            let is_shadow = if sprite.dictionary_index == UNMAPPED_DICT {
                pixel == SHADOW_KEY
            } else {
                pixel == self.dictionaries[sprite.dictionary_index as usize].shadow_color()
            };
            if is_shadow {
                return false;
            }
        }

        true
    }
}

impl robin_engine::engine::PixelOpacityLookup for FrameHolder {
    fn is_pixel_opaque(&self, bank_id: u32, x: u16, y: u16, blue_pixels_are_in: bool) -> bool {
        FrameHolder::is_pixel_opaque(self, bank_id, x, y, blue_pixels_are_in)
    }
}

impl robin_engine::engine::PixelOpacityLookup for PublishedFrameHolder {
    fn is_pixel_opaque(&self, bank_id: u32, x: u16, y: u16, blue_pixels_are_in: bool) -> bool {
        self.current
            .read()
            .expect("published frame-holder read lock poisoned")
            .is_pixel_opaque(bank_id, x, y, blue_pixels_are_in)
    }
}

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

/// Decompress RLE, wiping shadow pixels to zero (for shadow extraction).
pub fn decompress_rle_wipe_shadow(
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
                let color = src[src_pos];
                src_pos += 1;
                dst[dst_pos] = if color == shadow_color { 0 } else { color };
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

/// Decompress RLE, replacing non-shadow/non-transparent pixels with a
/// replacement color (for "into the shadow" rendering).
#[allow(clippy::too_many_arguments)]
pub fn decompress_rle_into_shadow(
    src: &[u16],
    dst: &mut [u16],
    pitch_words: usize,
    width: usize,
    height: usize,
    transparent_color: u16,
    shadow_color: u16,
    replacement_color: u16,
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
                let color = src[src_pos];
                src_pos += 1;
                dst[dst_pos] = if color == shadow_color || color == transparent_color {
                    color
                } else {
                    replacement_color
                };
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
fn rle_pixel_at(src: &[u16], x: usize, y: usize) -> u16 {
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
fn dict_pixel_at(src: &[u16], width: usize, x: usize, y: usize, dict: &FrameDictionary) -> u16 {
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

// ---------------------------------------------------------------------------
// Color effect functions (16-bit RGB565)
// ---------------------------------------------------------------------------

/// Pack RGB components (0–31 for R/B, 0–63 for G) into RGB565.
pub fn pack_rgb565(r: u16, g: u16, b: u16) -> u16 {
    ((r & 0x1F) << 11) | ((g & 0x3F) << 5) | (b & 0x1F)
}

/// Unpack RGB565 into (R, G, B) components.
pub fn unpack_rgb565(color: u16) -> (u16, u16, u16) {
    let r = (color & 0xF800) >> 11;
    let g = (color & 0x07E0) >> 5;
    let b = color & 0x001F;
    (r, g, b)
}

/// Scale all non-transparent, non-shadow pixels by `level/100`.
///
/// The green channel is extracted and repacked with the 5-bit mask
/// `0x07C0`, not the full RGB565 6-bit `0x07E0` mask. The LSB of the
/// 6-bit green channel is therefore discarded before scaling — a
/// deliberate quirk of the original asm code that we preserve for
/// pixel-exact parity.
/// Blend pixels toward a fog color at the given intensity.
///
/// Uses the same 5-bit green mask (`0x07C0`) as [`apply_color_scale_16`].
fn apply_fog_blend_16(data: &mut [u16], level: u16, fog_color: u16, shadow_color: u16) {
    let fog_r = (fog_color & 0xF800) >> 11;
    let fog_g = (fog_color & 0x07C0) >> 5;
    let fog_b = fog_color & 0x001F;

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

// ---------------------------------------------------------------------------
// CRC32 helper
// ---------------------------------------------------------------------------

/// Simple CRC32 computation (compatible with zlib crc32).
fn crc32_hash(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sprite_index_bytes(position: u32, size: u32, dictionary: u16) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&position.to_le_bytes());
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&dictionary.to_le_bytes());
        bytes
    }

    #[test]
    fn sprite_index_requires_the_full_fourteen_byte_header() {
        let error = BankSpriteIndex::from_le_bytes(&[0; 13]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("sprite index dictionary at byte 12")
        );
    }

    #[test]
    fn truncated_sprite_table_is_rejected_before_reserving_entries() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x1234_5678u32.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 13]);

        let mut holder = FrameHolder::new();
        let error = holder
            .load_sprite_index_bytes(&bytes, 0, &mut |_| {})
            .unwrap_err();
        assert!(error.to_string().contains("sprite index sprite count"));
        assert!(error.to_string().contains("only 13 remain"));
    }

    #[test]
    fn sprite_bank_range_must_fit_the_backing_bank() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x1234_5678u32.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&sprite_index_bytes(2, 4, UNMAPPED_DICT));

        let mut holder = FrameHolder::new();
        let error = holder
            .load_sprite_index_bytes(&bytes, 2, &mut |_| {})
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("sprite index record 0 bank range")
        );
        assert!(error.to_string().contains("exceeds buffer length 2"));
    }

    #[test]
    fn sprite_bank_range_must_be_word_aligned() {
        let index =
            BankSpriteIndex::from_le_bytes(&sprite_index_bytes(1, 2, UNMAPPED_DICT)).unwrap();
        let error = bank_span_for_index(1, &index, 7).unwrap_err();
        assert!(error.to_string().contains("sprite index record 7"));
        assert!(error.to_string().contains("not 16-bit aligned"));
    }

    #[test]
    fn test_sprite_variant_values() {
        assert_eq!(SpriteVariant::Day as u32, 0);
        assert_eq!(SpriteVariant::Night as u32, 1);
        assert_eq!(SpriteVariant::Fog as u32, 2);
    }

    #[test]
    fn test_pack_unpack_rgb565() {
        let color = pack_rgb565(31, 63, 31); // white
        assert_eq!(color, 0xFFFF);

        let (r, g, b) = unpack_rgb565(color);
        assert_eq!(r, 31);
        assert_eq!(g, 63);
        assert_eq!(b, 31);

        let black = pack_rgb565(0, 0, 0);
        assert_eq!(black, 0);

        let red = pack_rgb565(31, 0, 0);
        assert_eq!(red, 0xF800);
    }

    #[test]
    fn append_rgba_sprite_keeps_source_rgba_for_runtime_uploads() {
        let mut holder = FrameHolder::default();
        let rgba = [
            255, 0, 0, 255, //
            0, 0, 0, 96,
        ];

        let id = holder.append_rgba_sprite(2, 1, &rgba);

        assert_eq!(holder.rgba_data(id), Some(rgba.as_slice()));
        assert!(holder.packed_data(id).is_some());
        assert_eq!(holder.sprite_width(id), 2);
        assert_eq!(holder.sprite_height(id), 1);
    }

    #[test]
    fn test_frame_dictionary_lookup() {
        // 2 entries, each 4 pixels
        let data = vec![
            0x1111, 0x2222, 0x3333, 0x4444, // entry 0
            0x5555, 0x6666, 0x7777, 0x8888, // entry 1
        ];
        let dict = FrameDictionary::from_raw(2, data);

        let pixels = dict.lookup_pixels(0);
        assert_eq!(pixels, &[0x1111, 0x2222, 0x3333, 0x4444]);

        let pixels = dict.lookup_pixels(1);
        assert_eq!(pixels, &[0x5555, 0x6666, 0x7777, 0x8888]);
    }

    #[test]
    fn test_frame_dictionary_arno_law() {
        let data = vec![
            SHADOW_KEY, 0x0100, 0x0200, SHADOW_KEY, // entry with shadow pixels
        ];
        let mut dict = FrameDictionary::from_raw(1, data);

        let new_shadow = 0x0040;
        dict.apply_arno_law(new_shadow);

        let pixels = dict.lookup_pixels(0);
        assert_eq!(pixels[0], new_shadow); // was SHADOW_KEY, now new_shadow
        assert_eq!(pixels[3], new_shadow);
        assert_eq!(dict.shadow_color(), new_shadow);
    }

    #[test]
    fn test_decompress_rle_basic() {
        // A 4x2 sprite: first row has pixels at positions 1-2, second row is empty
        let src = vec![
            1, 2, 0xAAAA, 0xBBBB, // row 0: first=1, size=2, 2 pixels
            0xFFFF, 0xFFFF, // row 1: empty
        ];
        let mut dst = vec![0u16; 8]; // 4x2, no padding

        // Non-shadow pixels pass through unchanged regardless of shadow_color.
        decompress_rle_arno_law(&src, &mut dst, 4, 4, 2, TRANSPARENT_COLOR_16, 0x0040);

        // Row 0: transparent, 0xAAAA, 0xBBBB, transparent
        assert_eq!(dst[0], TRANSPARENT_COLOR_16);
        assert_eq!(dst[1], 0xAAAA);
        assert_eq!(dst[2], 0xBBBB);
        assert_eq!(dst[3], TRANSPARENT_COLOR_16);

        // Row 1: all transparent
        assert_eq!(dst[4], TRANSPARENT_COLOR_16);
        assert_eq!(dst[5], TRANSPARENT_COLOR_16);
        assert_eq!(dst[6], TRANSPARENT_COLOR_16);
        assert_eq!(dst[7], TRANSPARENT_COLOR_16);
    }

    #[test]
    fn test_decompress_vector_basic() {
        let dict_data = vec![
            0x0001, 0x0002, 0x0003, 0x0004, // entry 0
            0x0005, 0x0006, 0x0007, 0x0008, // entry 1
        ];
        let dict = FrameDictionary::from_raw(2, dict_data);

        // 8x1 sprite: 2 dictionary indices per row
        let src = vec![0, 1];
        let mut dst = vec![0u16; 8];

        decompress_vector(&src, &mut dst, 8, 8, 1, &dict);

        assert_eq!(
            dst,
            vec![
                0x0001, 0x0002, 0x0003, 0x0004, 0x0005, 0x0006, 0x0007, 0x0008
            ]
        );
    }

    #[test]
    fn test_frame_holder_new() {
        let fh = FrameHolder::new();
        assert_eq!(fh.global_shadow(), 40);
        assert_eq!(fh.global_blip_shadow(), 60);
        assert_eq!(fh.num_sprites(), 0);
    }

    #[test]
    fn test_frame_holder_add_dictionary_dedup() {
        let mut fh = FrameHolder::new();

        let data1 = vec![0x1111, 0x2222, 0x3333, 0x4444];
        let dict1 = FrameDictionary::from_raw(1, data1.clone());
        let idx1 = fh.add_dictionary(dict1);

        let dict2 = FrameDictionary::from_raw(1, data1);
        let idx2 = fh.add_dictionary(dict2);

        // Same checksum → same index
        assert_eq!(idx1, idx2);
        assert_eq!(fh.dictionaries.len(), 1);
    }

    #[test]
    fn test_frame_holder_generate_variants() {
        let mut fh = FrameHolder::new();

        let data = vec![0xF800, 0x07E0, 0x001F, 0x0000]; // red, green (fixed to 0x7C0), blue, black
        let dict = FrameDictionary::from_raw(1, data);
        fh.add_dictionary(dict);

        assert!(fh.dictionaries_night.is_empty());
        fh.generate_night_dictionaries();
        assert_eq!(fh.dictionaries_night.len(), 1);

        fh.generate_fog_dictionaries();
        assert_eq!(fh.dictionaries_fog.len(), 1);

        fh.drop_variant_dictionaries(SpriteVariant::Night);
        assert!(fh.dictionaries_night.is_empty());
        assert_eq!(fh.dictionaries_fog.len(), 1);
    }

    #[test]
    fn test_crc32() {
        let data = b"hello";
        let hash = crc32_hash(data);
        assert_eq!(hash, 0x3610A686);
    }

    #[test]
    fn test_packed_sprite_serde_roundtrip() {
        let sprite = PackedSprite {
            width: 64,
            height: 48,
            packed_size: 1024,
            dictionary_index: 3,
            ..Default::default()
        };

        let json = serde_json::to_string(&sprite).unwrap();
        let back: PackedSprite = serde_json::from_str(&json).unwrap();
        assert_eq!(back.width, 64);
        assert_eq!(back.height, 48);
        assert_eq!(back.dictionary_index, 3);
        // packed_data is skipped
        assert!(back.packed_data.is_none());
    }

    #[test]
    fn test_frame_dictionary_fog_effect() {
        let data = vec![
            pack_rgb565(31, 0, 0), // pure red
            TRANSPARENT_COLOR_16,  // transparent (skip)
            SHADOW_KEY,            // shadow (skip)
            pack_rgb565(0, 0, 31), // pure blue
        ];
        let mut dict = FrameDictionary::from_raw(1, data);

        dict.apply_fog_effect(50, pack_rgb565(31, 63, 31)); // 50% white fog

        let pixels = dict.lookup_pixels(0);
        // Red should be darkened and blended toward white
        assert_ne!(pixels[0], pack_rgb565(31, 0, 0));
        // Transparent and shadow should be unchanged
        assert_eq!(pixels[1], TRANSPARENT_COLOR_16);
        assert_eq!(pixels[2], SHADOW_KEY);
    }

    // -- Integration tests (require game data) --

    fn data_dir() -> Option<String> {
        std::env::var("ROBINHOOD_DATA_DIR").ok()
    }

    #[test]
    fn test_initialize_sprite_bank_from_game_data() {
        let Some(dir) = data_dir() else {
            eprintln!("ROBINHOOD_DATA_DIR not set, skipping integration test");
            return;
        };

        let holder = FrameHolder::from_data_dir(&dir).expect("failed to load sprite bank");

        // The game has a non-trivial number of sprites and dictionaries
        assert!(
            holder.num_sprites() > 1000,
            "expected >1000 sprites, got {}",
            holder.num_sprites()
        );
        assert!(
            holder.dictionaries.len() > 10,
            "expected >10 dictionaries, got {}",
            holder.dictionaries.len()
        );
        assert_ne!(holder.signature(), 0, "signature should be non-zero");

        // Spot-check: first sprite should have reasonable dimensions
        let w = holder.sprite_width(0);
        let h = holder.sprite_height(0);
        assert!(w > 0 && w < 2000, "sprite 0 width {w} out of range");
        assert!(h > 0 && h < 2000, "sprite 0 height {h} out of range");
    }

    #[test]
    fn test_sprite_bank_packed_data_present() {
        let Some(dir) = data_dir() else {
            return;
        };

        let holder = FrameHolder::from_data_dir(&dir).unwrap();

        // A sample of sprites should have packed data loaded
        let mut loaded = 0u32;
        let sample = holder.num_sprites().min(100);
        for i in 0..sample {
            if holder.packed_data(i as u32).is_some() {
                loaded += 1;
            }
        }
        assert!(
            loaded > sample as u32 / 2,
            "expected most sprites to have packed data, got {loaded}/{sample}"
        );
    }

    // -- Viewport effect tests --

    #[test]
    fn test_apply_fog_effect_viewport_skips_transparent_and_shadow() {
        let shadow = 0x0040u16;
        let mut data = vec![0xF800, TRANSPARENT_COLOR_16, shadow, 0x001F];

        apply_fog_effect_viewport(&mut data, 50, FOG_COLOR, shadow);

        // Transparent and shadow pixels must be unchanged
        assert_eq!(data[1], TRANSPARENT_COLOR_16);
        assert_eq!(data[2], shadow);
        // Non-transparent pixels must be modified
        assert_ne!(data[0], 0xF800);
        assert_ne!(data[3], 0x001F);
    }

    #[test]
    fn test_apply_fog_effect_viewport_level_zero_replaces_with_fog() {
        // At level 0: pixel contribution is 0, fog contribution is 100%
        let shadow = 0x0040u16;
        let fog_color = pack_rgb565(16, 0, 0); // some red fog
        let mut data = vec![pack_rgb565(31, 62, 31)]; // bright pixel

        apply_fog_effect_viewport(&mut data, 0, fog_color, shadow);

        // Result should be pure fog color (extracted with 5-bit green mask)
        let fog_r = (fog_color & 0xF800) >> 11;
        let fog_g = (fog_color & 0x07C0) >> 5;
        let fog_b = fog_color & 0x001F;
        let expected = ((fog_r << 11) & 0xF800) | ((fog_g << 5) & 0x07C0) | (fog_b & 0x001F);
        assert_eq!(data[0], expected);
    }

    #[test]
    fn test_convert_decompressed_to_15bit() {
        let shadow = 0x0040u16; // use a non-blue shadow
        // RGB565 pure red: 0xF800 → ((0xF800 & 0xFFC0) >> 1) | (0xF800 & 0x1F) = 0x7C00
        let mut data = vec![0xF800, shadow, 0x07E0, 0x001F];

        convert_decompressed_to_15bit(&mut data, 4, 4, 1, shadow);

        assert_eq!(data[0], 0x7C00); // red converted
        assert_eq!(data[1], shadow); // shadow unchanged
        assert_eq!(data[2], 0x03E0); // green converted
        // Blue 0x001F: upper bits are 0, only lower 5 bits → unchanged
        assert_eq!(data[3], 0x001F);
    }

    #[test]
    fn test_convert_decompressed_to_16bit_bit_exact() {
        // Bit-exact check of the conversion formula:
        // `((pixel << 1) & 0xFFE0) | (pixel & 0x1F)`.
        //
        // Red and green round-trip cleanly; blue's MSB (bit 4) leaks into
        // G's LSB (bit 5) after the shift — a known quirk of the formula
        // we preserve for pixel-exact parity.
        let shadow = 0x0040u16;
        // 0x7C00 (RGB555 red=31) → 0xF800 (RGB565 red=31)
        // 0x03E0 (RGB555 green=31) → 0x07C0 (RGB565 green at upper 5 of 6 bits)
        // 0x001F (RGB555 blue=31) → 0x003F (green LSB contaminated by blue MSB shift)
        let mut data = vec![0x7C00u16, 0x03E0, 0x001F, shadow];
        convert_decompressed_to_16bit(&mut data, 4, 4, 1, shadow);
        assert_eq!(data[0], 0xF800);
        assert_eq!(data[1], 0x07C0);
        assert_eq!(data[2], 0x003F);
        assert_eq!(data[3], shadow);
    }

    #[test]
    fn test_convert_decompressed_to_16bit_with_pitch() {
        let shadow = 0x0040u16;
        let mut data = vec![
            0x7C00u16, 0x03E0, 0x0000, 0x0000, // row 0 + padding
            0x001F, shadow, 0x0000, 0x0000, // row 1 + padding
        ];
        convert_decompressed_to_16bit(&mut data, 4, 2, 2, shadow);
        assert_eq!(data[0], 0xF800);
        assert_eq!(data[1], 0x07C0);
        assert_eq!(data[2], 0x0000); // padding untouched
        assert_eq!(data[4], 0x003F); // blue → shifted-up with G-LSB contamination
        assert_eq!(data[5], shadow);
    }

    #[test]
    fn test_convert_decompressed_to_15bit_with_pitch() {
        let shadow = 0x0040u16;
        // 2x2 with pitch=4 (2 padding words per row)
        let mut data = vec![
            0xF800, 0x07E0, 0x0000, 0x0000, // row 0 + padding
            0x001F, shadow, 0x0000, 0x0000, // row 1 + padding
        ];

        convert_decompressed_to_15bit(&mut data, 4, 2, 2, shadow);

        assert_eq!(data[0], 0x7C00); // red converted
        assert_eq!(data[1], 0x03E0); // green converted
        assert_eq!(data[2], 0x0000); // padding untouched
        // Blue 0x001F: upper bits are 0 → unchanged
        assert_eq!(data[4], 0x001F);
        assert_eq!(data[5], shadow); // shadow untouched
    }

    // -- High-level uncompress tests --

    #[test]
    fn test_uncompress_frame_rle_day() {
        let mut fh = FrameHolder::new();

        let packed = vec![
            1u16, 2, 0xAAAA, 0xBBBB, // row 0: first=1, size=2
            0xFFFF, 0xFFFF, // row 1: empty
        ];
        fh.sprites.push(PackedSprite {
            width: 4,
            height: 2,
            packed_size: (packed.len() * 2) as u32,
            packed_data: Some(packed),
            bank_span: None,
            rgba_data: None,
            dictionary_index: UNMAPPED_DICT,
        });

        let mut dst = vec![0u16; 8];
        fh.uncompress_frame(&mut dst, 4, 0, SpriteVariant::Day, SHADOW_KEY, 16);

        assert_eq!(dst[0], TRANSPARENT_COLOR_16);
        assert_eq!(dst[1], 0xAAAA);
        assert_eq!(dst[2], 0xBBBB);
        assert_eq!(dst[3], TRANSPARENT_COLOR_16);
        // Row 1: all transparent
        for &px in &dst[4..8] {
            assert_eq!(px, TRANSPARENT_COLOR_16);
        }
    }

    #[test]
    fn test_uncompress_frame_rle_paged_arno_law() {
        let mut fh = FrameHolder::new();

        // Sprite with SHADOW_KEY (0x1F) pixels — paged uses ArnoLaw replacement
        let packed = vec![
            0u16, 1, SHADOW_KEY, 0x1234, // row 0: first=0, size=1
        ];
        fh.sprites.push(PackedSprite {
            width: 2,
            height: 1,
            packed_size: (packed.len() * 2) as u32,
            packed_data: Some(packed),
            bank_span: None,
            rgba_data: None,
            dictionary_index: UNMAPPED_DICT,
        });

        let shadow_color = 0x0040u16;
        let mut dst = vec![0u16; 2];
        fh.uncompress_frame(&mut dst, 2, 0, SpriteVariant::Day, shadow_color, 16);

        // SHADOW_KEY (0x1F) should be replaced with shadow_color
        assert_eq!(dst[0], shadow_color);
        // Non-shadow pixel unchanged
        assert_eq!(dst[1], 0x1234);
    }

    #[test]
    fn test_uncompress_frame_vector_day() {
        let mut fh = FrameHolder::new();

        let dict_data = vec![
            0x0001, 0x0002, 0x0003, 0x0004, // entry 0
            0x0005, 0x0006, 0x0007, 0x0008, // entry 1
        ];
        let dict = FrameDictionary::from_raw(2, dict_data);
        fh.add_dictionary(dict);
        fh.generate_night_dictionaries();
        fh.generate_fog_dictionaries();

        let packed = vec![0u16, 1]; // two dictionary indices
        fh.sprites.push(PackedSprite {
            width: 8,
            height: 1,
            packed_size: (packed.len() * 2) as u32,
            packed_data: Some(packed),
            bank_span: None,
            rgba_data: None,
            dictionary_index: 0,
        });

        let mut dst = vec![0u16; 8];
        fh.uncompress_frame(&mut dst, 8, 0, SpriteVariant::Day, SHADOW_KEY, 16);

        assert_eq!(
            dst,
            vec![
                0x0001, 0x0002, 0x0003, 0x0004, 0x0005, 0x0006, 0x0007, 0x0008
            ]
        );
    }

    #[test]
    fn test_uncompress_frame_rle_night_applies_effect() {
        let mut fh = FrameHolder::new();

        // Single-pixel sprite with a bright color
        let packed = vec![0u16, 0, 0xF800]; // first=0, size=0, one red pixel
        fh.sprites.push(PackedSprite {
            width: 1,
            height: 1,
            packed_size: (packed.len() * 2) as u32,
            packed_data: Some(packed),
            bank_span: None,
            rgba_data: None,
            dictionary_index: UNMAPPED_DICT,
        });

        let mut dst = vec![0u16; 1];
        fh.uncompress_frame(&mut dst, 1, 0, SpriteVariant::Night, SHADOW_KEY, 16);

        // Night effect should darken the red pixel
        assert_ne!(dst[0], 0xF800);
        assert_ne!(dst[0], TRANSPARENT_COLOR_16);
    }

    #[test]
    fn test_uncompress_frame_wipe_shadow() {
        let mut fh = FrameHolder::new();

        let packed = vec![
            0u16, 1, SHADOW_KEY, 0x1234, // row 0: shadow + normal pixel
        ];
        fh.sprites.push(PackedSprite {
            width: 2,
            height: 1,
            packed_size: (packed.len() * 2) as u32,
            packed_data: Some(packed),
            bank_span: None,
            rgba_data: None,
            dictionary_index: UNMAPPED_DICT,
        });

        let mut dst = vec![0xFFFFu16; 2];
        fh.uncompress_frame_wipe_shadow(&mut dst, 2, 0, 16);

        // Shadow pixel wiped to 0
        assert_eq!(dst[0], 0);
        // Normal pixel preserved
        assert_eq!(dst[1], 0x1234);
    }

    #[test]
    fn test_uncompress_frame_into_shadow() {
        let mut fh = FrameHolder::new();

        let shadow_color = 0x0040u16;
        let packed = vec![
            0u16,
            2,
            shadow_color,
            TRANSPARENT_COLOR_16,
            0x1234, // 3 pixels
        ];
        fh.sprites.push(PackedSprite {
            width: 3,
            height: 1,
            packed_size: (packed.len() * 2) as u32,
            packed_data: Some(packed),
            bank_span: None,
            rgba_data: None,
            dictionary_index: UNMAPPED_DICT,
        });

        let replacement = 0x0800u16;
        let mut dst = vec![0u16; 3];
        fh.uncompress_frame_into_shadow(
            &mut dst,
            3,
            0,
            SpriteVariant::Day,
            shadow_color,
            replacement,
            16,
        );

        // Shadow and transparent pixels kept as-is
        assert_eq!(dst[0], shadow_color);
        assert_eq!(dst[1], TRANSPARENT_COLOR_16);
        // Other pixels replaced
        assert_eq!(dst[2], replacement);
    }
}
