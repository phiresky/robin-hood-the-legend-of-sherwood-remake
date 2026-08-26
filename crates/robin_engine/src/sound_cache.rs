//! Sound sample cache manager.
//!
//! Manages multiple caches of sound effects (FX, combat FX, source, speech,
//! menu) with TTL-based eviction. Samples are loaded on demand and unloaded
//! when their time-to-live expires and they are not playing.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Loader function type: given a filename, returns `(pcm_data, size_bytes, duration_ms)`.
pub type SampleLoader = dyn Fn(&str) -> Option<(Vec<u8>, u32, u32)>;

/// Try to load a single entry, then immediately unload it. Returns
/// `true` if the load succeeded. Used by `SoundCache::validate_data`
/// to drive the per-entry pre-flight check.
fn validate_entry(entry: &mut SoundCacheEntry, is_3d: bool, loader: &SampleLoader) -> bool {
    if entry.file_name.is_empty() {
        return true;
    }
    if entry.load_sample(is_3d, 0, loader) {
        entry.unload_sample();
        true
    } else {
        tracing::warn!("SoundCache: Unable to load sample \"{}\"", entry.file_name);
        false
    }
}

// ---------------------------------------------------------------------------
// TTL settings
// ---------------------------------------------------------------------------

pub const FXCACHE_TTL_INIT: u32 = 400;
pub const FXCACHE_TTL_INCREMENT: u32 = 200;
pub const FXCACHE_TTL_DECREMENT: u32 = 5;

pub const SOURCECACHE_TTL_INIT: u32 = 400;
pub const SOURCECACHE_TTL_INCREMENT: u32 = 200;
pub const SOURCECACHE_TTL_DECREMENT: u32 = 2;

pub const SPEECHCACHE_TTL_INIT: u32 = 400;
pub const SPEECHCACHE_TTL_INCREMENT: u32 = 200;
pub const SPEECHCACHE_TTL_DECREMENT: u32 = 5;

pub const MENUCACHE_TTL_INIT: u32 = 400;
pub const MENUCACHE_TTL_INCREMENT: u32 = 200;
pub const MENUCACHE_TTL_DECREMENT: u32 = 10;

pub const SOUND_BANK_VERSION: u32 = 1;
pub const CACHE_BLOCK_SIZE: usize = 200;

pub const FX_BANK_FILE: &str = "/robin hood.fxg";
pub const MENU_SOUND_BANK_FILE: &str = "/Menu/menu.fxg";

// ---------------------------------------------------------------------------
// Material enum
// ---------------------------------------------------------------------------

#[repr(u32)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum Material {
    Ground = 0,
    Wood = 1,
    Stone = 2,
    Grass = 3,
    Leaves = 4,
    Water = 5,
    Bush = 6,
    Ice = 7,
    Hole = 8,
}

impl Material {
    pub const NUM_MATERIALS: usize = 9;
}

// ---------------------------------------------------------------------------
// Widget noisy event
// ---------------------------------------------------------------------------

/// Number of widget noisy events used when loading menu sound banks.
pub const WIDGET_NOISY_EVENT_COUNT: u32 = 2;

// ---------------------------------------------------------------------------
// Combat FX tables
// ---------------------------------------------------------------------------

pub const STRIKE_FX_LIST: &[&str] = &[
    "wowo", "wost", "woci", "wosw", "stst", "stci", "stsw", "cici", "cisw", "swsw",
];

pub const IMPACT_FX_LIST: &[&str] = &[
    "wolt", "wocm", "wopl", "stlt", "stcm", "stpl", "cilt", "cicm", "cipl", "swlt", "swcm", "swpl",
];

// ---------------------------------------------------------------------------
// Sound group types
// ---------------------------------------------------------------------------

#[repr(u32)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum SoundGroupType {
    MaterialGroup = 0,
    RandomGroup = 1,
    Fx = 2,
}

impl SoundGroupType {
    pub fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::MaterialGroup),
            1 => Some(Self::RandomGroup),
            2 => Some(Self::Fx),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// SoundCacheEntry
// ---------------------------------------------------------------------------

/// A single cached sound sample.
///
/// Stores the raw PCM bytes and metadata. The actual audio backend
/// handle is managed externally; `sample_data` being `Some` means
/// "loaded".
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SoundCacheEntry {
    pub file_name: String,
    /// Raw sample bytes (None = not loaded).
    pub sample_data: Option<Vec<u8>>,
    pub sample_size: u32,
    pub sample_length_ms: u32,
    pub loop_sample: bool,
    pub playing: u16,
    pub time_to_live: u32,
}

impl SoundCacheEntry {
    pub fn new(file_name: impl Into<String>) -> Self {
        Self {
            file_name: file_name.into(),
            ..Default::default()
        }
    }

    /// Simulate loading a sample (sets TTL, marks as loaded).
    ///
    /// Delegates to a provided loader closure so the cache logic is
    /// independent of the audio backend.
    pub fn load_sample(&mut self, _is_3d: bool, ttl_init: u32, loader: &SampleLoader) -> bool {
        if let Some((data, size, length_ms)) = loader(&self.file_name) {
            self.sample_data = Some(data);
            if self.sample_size == 0 {
                self.sample_size = size;
                self.sample_length_ms = length_ms;
            }
            self.time_to_live = ttl_init;
            true
        } else {
            false
        }
    }

    /// Unload the sample data, resetting playback state.
    pub fn unload_sample(&mut self) {
        self.sample_data = None;
        self.time_to_live = 0;
        self.playing = 0;
    }

    /// Whether sample data is currently loaded.
    pub fn is_loaded(&self) -> bool {
        self.sample_data.is_some()
    }
}

// ---------------------------------------------------------------------------
// SoundGroup
// ---------------------------------------------------------------------------

/// A group of sound entries that can be selected by material, randomly, or
/// as a single FX.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SoundGroup {
    pub group_type: SoundGroupType,
    /// Indices into the owning `IndexedCache::entries` vec.
    pub entry_indices: Vec<usize>,
    /// Number of "gap" slots for random selection (probability of silence).
    pub gaps: u16,
}

// ---------------------------------------------------------------------------
// Cache statistics
// ---------------------------------------------------------------------------

#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct CacheStats {
    pub hits: u32,
    pub misses: u32,
    pub data_size: u32,
}

// ---------------------------------------------------------------------------
// IndexedCache — vector-backed cache used for FX and Speech
// ---------------------------------------------------------------------------

/// A cache where entries are stored in a contiguous `Vec` and referenced
/// by index. Sound groups map IDs to groups whose entry lists contain
/// indices into `entries`.
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct IndexedCache {
    pub entries: Vec<SoundCacheEntry>,
    pub groups: BTreeMap<u32, SoundGroup>,
    pub stats: CacheStats,
}

impl IndexedCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a new entry and return its index.
    pub fn add_entry(&mut self, file_name: impl Into<String>) -> usize {
        let idx = self.entries.len();
        self.entries.push(SoundCacheEntry::new(file_name));
        idx
    }

    /// Add a sound group. Panics if the group ID already exists.
    pub fn add_group(
        &mut self,
        group_id: u32,
        group_type: SoundGroupType,
        gaps: u16,
    ) -> &mut SoundGroup {
        if self.groups.contains_key(&group_id) {
            panic!("Sound group {group_id} already allocated");
        }
        self.groups.insert(
            group_id,
            SoundGroup {
                group_type,
                entry_indices: Vec::new(),
                gaps,
            },
        );
        self.groups.get_mut(&group_id).unwrap()
    }

    /// Invalidate all loaded entries (unload their samples).
    pub fn invalidate(&mut self) {
        for entry in &mut self.entries {
            if entry.is_loaded() {
                self.stats.data_size = self.stats.data_size.saturating_sub(entry.sample_size);
                entry.unload_sample();
            }
        }
    }

    /// Decrement TTLs and unload expired, non-playing entries.
    pub fn update_ttl(&mut self, decrement: u32) {
        for entry in &mut self.entries {
            if entry.playing == 0 && entry.time_to_live > 0 {
                entry.time_to_live = entry.time_to_live.saturating_sub(decrement);
                if entry.time_to_live == 0 && entry.is_loaded() {
                    self.stats.data_size = self.stats.data_size.saturating_sub(entry.sample_size);
                    entry.unload_sample();
                }
            }
        }
    }

    /// Flush all groups and entries.
    pub fn flush(&mut self) {
        self.groups.clear();
        self.entries.clear();
    }

    /// Get an entry for playback from a group, handling FX / material / random
    /// selection. Returns `None` if the group or entry is not found, or if a
    /// random gap is selected.
    #[allow(clippy::too_many_arguments)]
    pub fn get_fx_sample(
        &mut self,
        sample_present: bool,
        sample_id: u32,
        material: Option<Material>,
        is_3d: bool,
        ttl_init: u32,
        ttl_increment: u32,
        loader: &SampleLoader,
        rng: &mut dyn FnMut(u32) -> u32,
    ) -> Option<usize> {
        let cache_entry_index = {
            let group = self.groups.get(&sample_id)?;
            match group.group_type {
                SoundGroupType::Fx => group.entry_indices.first().copied(),
                SoundGroupType::MaterialGroup => {
                    let mat = material? as usize;
                    if mat >= Material::NUM_MATERIALS {
                        tracing::warn!("Incorrect material type supplied (SFX id {})", sample_id);
                        return None;
                    }
                    group.entry_indices.get(mat).copied()
                }
                SoundGroupType::RandomGroup => {
                    let count = group.entry_indices.len() as u32;
                    let total = count + group.gaps as u32;
                    if total == 0 {
                        return None;
                    }
                    let selected = rng(total);
                    if selected < count {
                        Some(group.entry_indices[selected as usize])
                    } else {
                        None // gap — silence
                    }
                }
            }
        };

        let idx = cache_entry_index?;
        let entry = &mut self.entries[idx];

        if sample_present {
            if entry.is_loaded() {
                self.stats.hits += 1;
                entry.time_to_live += ttl_increment;
            } else if entry.load_sample(is_3d, ttl_init, loader) {
                self.stats.misses += 1;
                self.stats.data_size += entry.sample_size;
            }
        } else if entry.sample_length_ms == 0 && entry.load_sample(is_3d, ttl_init, loader) {
            self.stats.misses += 1;
            self.stats.data_size += entry.sample_size;
        }

        Some(idx)
    }

    /// Get an exclamation/speech sample. Variant == u32::MAX means random.
    #[allow(clippy::too_many_arguments)]
    pub fn get_exclamation_sample(
        &mut self,
        sample_present: bool,
        exclamation_id: u32,
        variant: Option<u32>,
        is_3d: bool,
        ttl_init: u32,
        ttl_increment: u32,
        loader: &SampleLoader,
        rng: &mut dyn FnMut(u32) -> u32,
    ) -> Option<usize> {
        let group = self.groups.get(&exclamation_id)?;

        assert!(
            group.group_type == SoundGroupType::RandomGroup,
            "Speech groups MUST be random groups"
        );

        let entry_indices = group.entry_indices.clone();
        let gaps = group.gaps;
        let count = entry_indices.len();

        let idx = if let Some(v) = variant {
            if (v as usize) < count {
                entry_indices[v as usize]
            } else {
                tracing::warn!("Trying to get a speech variant that doesn't exist");
                return None;
            }
        } else {
            // Random selection
            let total = count as u32 + gaps as u32;
            if total == 0 {
                return None;
            }
            let selected = rng(total);
            if (selected as usize) < count {
                entry_indices[selected as usize]
            } else {
                return None; // gap
            }
        };

        let entry = &mut self.entries[idx];

        if sample_present {
            if entry.is_loaded() {
                self.stats.hits += 1;
                entry.time_to_live += ttl_increment;
            } else if entry.load_sample(is_3d, ttl_init, loader) {
                self.stats.misses += 1;
                self.stats.data_size += entry.sample_size;
            } else {
                tracing::trace!(
                    exclamation_id = format!("{exclamation_id:#010x}"),
                    idx,
                    file = entry.file_name.as_str(),
                    "get_exclamation_sample: load_sample FAILED (sample_present)"
                );
            }
        } else if entry.sample_length_ms == 0 {
            if entry.load_sample(is_3d, ttl_init, loader) {
                self.stats.misses += 1;
                self.stats.data_size += entry.sample_size;
            } else {
                tracing::trace!(
                    exclamation_id = format!("{exclamation_id:#010x}"),
                    idx,
                    file = entry.file_name.as_str(),
                    "get_exclamation_sample: load_sample FAILED (length query)"
                );
            }
        }

        Some(idx)
    }
}

// ---------------------------------------------------------------------------
// MappedCache — map-backed cache used for CombatFX, Source, Menu
// ---------------------------------------------------------------------------

/// A cache where entries are stored in a `BTreeMap<u32, SoundCacheEntry>`
/// keyed by sample ID.
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct MappedCache {
    pub entries: BTreeMap<u32, SoundCacheEntry>,
    pub stats: CacheStats,
}

impl MappedCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a sound entry by ID.
    pub fn add_entry(&mut self, id: u32, file_name: impl Into<String>) {
        self.entries.insert(id, SoundCacheEntry::new(file_name));
    }

    /// Look up and optionally load a sample.
    #[allow(clippy::too_many_arguments)]
    pub fn get_sample(
        &mut self,
        sample_present: bool,
        sample_id: u32,
        loop_sample: bool,
        is_3d: bool,
        ttl_init: u32,
        ttl_increment: u32,
        loader: &SampleLoader,
    ) -> Option<&mut SoundCacheEntry> {
        let entry = self.entries.get_mut(&sample_id)?;
        entry.loop_sample = loop_sample;

        if sample_present {
            if entry.is_loaded() {
                self.stats.hits += 1;
                entry.time_to_live += ttl_increment;
            } else if entry.load_sample(is_3d, ttl_init, loader) {
                self.stats.misses += 1;
                self.stats.data_size += entry.sample_size;
            }
        } else if entry.sample_length_ms == 0 && entry.load_sample(is_3d, ttl_init, loader) {
            self.stats.misses += 1;
            self.stats.data_size += entry.sample_size;
        }

        Some(entry)
    }

    /// Invalidate (unload) all loaded entries.
    pub fn invalidate(&mut self) {
        for entry in self.entries.values_mut() {
            if entry.is_loaded() {
                self.stats.data_size = self.stats.data_size.saturating_sub(entry.sample_size);
                entry.unload_sample();
            }
        }
    }

    /// Decrement TTLs, unloading expired non-playing entries.
    pub fn update_ttl(&mut self, decrement: u32) {
        for entry in self.entries.values_mut() {
            if entry.playing == 0 && entry.time_to_live > 0 {
                entry.time_to_live = entry.time_to_live.saturating_sub(decrement);
                if entry.time_to_live == 0 && entry.is_loaded() {
                    self.stats.data_size = self.stats.data_size.saturating_sub(entry.sample_size);
                    entry.unload_sample();
                }
            }
        }
    }

    /// Flush (remove) all entries.
    ///
    /// Does not touch `stats` — wipe the stats via
    /// `SoundCache::reset_cache_stats` (called from `SoundCache::flush`).
    pub fn flush(&mut self) {
        self.entries.clear();
    }

    /// Find the ID for a given entry by file name match (reverse lookup).
    pub fn id_for_entry_by_filename(&self, file_name: &str) -> Option<u32> {
        self.entries
            .iter()
            .find(|(_, e)| e.file_name == file_name)
            .map(|(&id, _)| id)
    }
}

// ---------------------------------------------------------------------------
// FX bank file parsing
// ---------------------------------------------------------------------------

/// Parsed element from an FX bank file.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct FxBankElement {
    pub element_type: SoundGroupType,
    pub element_id: u32,
    pub logical_volume: u16,
    pub file_name: Option<String>,
    pub sub_elements: Vec<FxBankSubElement>,
    pub gaps_count: u16,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct FxBankSubElement {
    pub element_type: u32,
    pub group_id: u32,
    pub logical_volume: u16,
    pub file_name: String,
}

/// Parse an FX bank file from raw bytes. Returns the list of elements.
///
/// File format: `"FXBK"` magic, u32 header_size, u32 version, u32 count,
/// then `count` elements.
pub fn parse_fx_bank(data: &[u8]) -> Result<Vec<FxBankElement>, String> {
    if data.len() < 16 {
        return Err("FX bank file too short".into());
    }
    if &data[0..4] != b"FXBK" {
        return Err("Invalid FX bank magic".into());
    }
    // Skip header_size (4 bytes) and version (4 bytes)
    let mut pos = 12;

    let count = read_u32_le(data, &mut pos)?;
    let mut elements = Vec::new();
    let mut i = 0u32;

    while i < count {
        let el_type_raw = read_u32_le(data, &mut pos)?;
        let el_id = read_u32_le(data, &mut pos)?;
        let logical_volume = read_u16_le(data, &mut pos)?;

        if el_id & 0xFFFF0000 != 0 {
            return Err(format!("Invalid FX ID: {el_id:#x}"));
        }

        let el_type = SoundGroupType::from_u32(el_type_raw)
            .ok_or_else(|| format!("Unknown element type: {el_type_raw}"))?;

        match el_type {
            SoundGroupType::RandomGroup | SoundGroupType::MaterialGroup => {
                let gaps_count = read_u16_le(data, &mut pos)?;
                let grp_count = read_u32_le(data, &mut pos)?;

                let mut sub_elements = Vec::with_capacity(grp_count as usize);
                for _ in 0..grp_count {
                    let sub_type = read_u32_le(data, &mut pos)?;
                    let sub_id = read_u32_le(data, &mut pos)?;
                    let sub_vol = read_u16_le(data, &mut pos)?;
                    let sub_name = read_fx_filename(data, &mut pos)?;
                    sub_elements.push(FxBankSubElement {
                        element_type: sub_type,
                        group_id: sub_id,
                        logical_volume: sub_vol,
                        file_name: sub_name,
                    });
                }

                elements.push(FxBankElement {
                    element_type: el_type,
                    element_id: el_id,
                    logical_volume,
                    file_name: None,
                    sub_elements,
                    gaps_count,
                });

                i += grp_count;
            }
            SoundGroupType::Fx => {
                let name = read_fx_filename(data, &mut pos)?;
                elements.push(FxBankElement {
                    element_type: el_type,
                    element_id: el_id,
                    logical_volume,
                    file_name: Some(name),
                    sub_elements: Vec::new(),
                    gaps_count: 0,
                });
            }
        }

        i += 1;
    }

    Ok(elements)
}

/// Parse a menu sound bank. Returns `(group_id, element_index, filename)` tuples
/// for each menu sound entry (limited to `WIDGET_NOISY_EVENT_COUNT` per group).
pub fn parse_menu_bank(data: &[u8]) -> Result<Vec<(u32, String)>, String> {
    if data.len() < 16 {
        return Err("Menu bank file too short".into());
    }
    if &data[0..4] != b"FXBK" {
        return Err("Invalid menu bank magic".into());
    }
    let mut pos = 12;
    let count = read_u32_le(data, &mut pos)?;
    let mut results = Vec::new();

    let mut i = 0u32;
    while i < count {
        let group_type = read_u32_le(data, &mut pos)?;
        let group_id = read_u32_le(data, &mut pos)?;
        let _logical_volume = read_u16_le(data, &mut pos)?;

        if group_type != SoundGroupType::MaterialGroup as u32 {
            return Err("Invalid menu sound bank: unexpected group type".into());
        }

        let _gaps_count = read_u16_le(data, &mut pos)?;
        let grp_count = read_u32_le(data, &mut pos)?;

        // Only read WIDGET_NOISY_EVENT_COUNT elements; skip the rest
        let to_read = grp_count.min(WIDGET_NOISY_EVENT_COUNT);
        for el_idx in 0..to_read {
            let _el_type = read_u32_le(data, &mut pos)?;
            let _el_grp_id = read_u32_le(data, &mut pos)?;
            let _el_vol = read_u16_le(data, &mut pos)?;
            let name = read_fx_filename(data, &mut pos)?;
            let full_name = format!("/Menu/{name}");
            let combined_id = (group_id << 16) | el_idx;
            results.push((combined_id, full_name));
        }

        // Skip remaining
        for _ in to_read..grp_count {
            let _el_type = read_u32_le(data, &mut pos)?;
            let _el_grp_id = read_u32_le(data, &mut pos)?;
            let _el_vol = read_u16_le(data, &mut pos)?;
            let name_len = read_u16_le(data, &mut pos)?;
            pos += name_len as usize;
            if pos > data.len() {
                return Err("Menu bank: unexpected end of data".into());
            }
        }

        i += grp_count + 1;
    }

    Ok(results)
}

/// Parse an exclamation definition file.
///
/// Format: `"NEUF"` magic, u32 version (must be 1), u32 table_id,
/// u32 num_exclamations, then for each: u32 num_variants, then
/// u32 variant_index for each variant.
///
/// Returns `(table_id, [(action_id, variant_indices)])`.
/// The `table_id` is the resource manager collection ID used to resolve
/// variant indices to WAV file paths.
#[allow(clippy::type_complexity)]
pub fn parse_exclamation_file(
    data: &[u8],
    prefix_id: u32,
) -> Result<(u32, Vec<(u32, Vec<u32>)>), String> {
    if data.len() < 16 {
        return Err("Exclamation file too short".into());
    }
    if &data[0..4] != b"NEUF" {
        return Err("Invalid exclamation file magic".into());
    }
    let mut pos = 4;
    let version = read_u32_le(data, &mut pos)?;
    if version != 1 {
        return Err(format!("Invalid exclamation file version: {version}"));
    }
    let table_id = read_u32_le(data, &mut pos)?;
    let num_exclamations = read_u32_le(data, &mut pos)?;

    let mut results = Vec::with_capacity(num_exclamations as usize);

    for excl_idx in 0..num_exclamations {
        let num_variants = read_u32_le(data, &mut pos)?;
        let action_id = prefix_id | (excl_idx & 0xFFFF);

        let mut variant_indices = Vec::with_capacity(num_variants as usize);
        for _ in 0..num_variants {
            let variant_index = read_u32_le(data, &mut pos)?;
            variant_indices.push(variant_index);
        }

        results.push((action_id, variant_indices));
    }

    Ok((table_id, results))
}

// Binary reading helpers

fn read_u32_le(data: &[u8], pos: &mut usize) -> Result<u32, String> {
    if *pos + 4 > data.len() {
        return Err("Unexpected end of data reading u32".into());
    }
    let val = u32::from_le_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
    *pos += 4;
    Ok(val)
}

fn read_u16_le(data: &[u8], pos: &mut usize) -> Result<u16, String> {
    if *pos + 2 > data.len() {
        return Err("Unexpected end of data reading u16".into());
    }
    let val = u16::from_le_bytes([data[*pos], data[*pos + 1]]);
    *pos += 2;
    Ok(val)
}

fn read_fx_filename(data: &[u8], pos: &mut usize) -> Result<String, String> {
    let name_len = read_u16_le(data, pos)? as usize;
    if *pos + name_len > data.len() {
        return Err("Unexpected end of data reading filename".into());
    }
    let raw = &data[*pos..*pos + name_len];
    *pos += name_len;
    let mut name = String::from_utf8_lossy(raw).into_owned();
    // Ensure .wav extension
    if name_len > 4 && !name[name_len - 4..].starts_with('.') {
        name.push_str(".wav");
    }
    Ok(name)
}

// ---------------------------------------------------------------------------
// SoundCache — the main cache manager
// ---------------------------------------------------------------------------

/// The main sound cache manager, holding multiple sub-caches for different
/// sound categories.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SoundCache {
    /// FX cache (vector-indexed, with sound groups).
    pub fx_cache: IndexedCache,
    /// Combat FX cache (mapped by ID).
    pub combat_fx_cache: MappedCache,
    /// Sound source cache (mapped by ID).
    pub source_cache: MappedCache,
    /// Speech/exclamation cache (vector-indexed, with sound groups).
    pub speech_cache: IndexedCache,
    /// Menu sound cache (mapped by ID).
    pub menu_cache: MappedCache,

    /// Music pools for different alert modes.
    pub quiet_music_pool: Vec<String>,
    pub alert_music_pool: Vec<String>,
    pub fight_music_pool: Vec<String>,

    pub use_3d_sound: bool,

    /// Whether the `check_sound_data` pre-flight has passed for every
    /// entry added so far. Initialised to `true` and only ever flipped
    /// to `false` by sample-load failures during validation. The audio
    /// activation path reads this via [`SoundCache::data_check_succeed`]
    /// and fatal-errors when it's false.
    ///
    /// Entries are added without an attached loader, so validation runs
    /// as a separate pass: callers invoke [`SoundCache::validate_data`]
    /// after the cache is populated to drive the equivalent load+unload
    /// check. Saved to disk so a missing-sample status survives serde
    /// round-trips.
    pub data_check_succeeded: bool,
}

impl Default for SoundCache {
    fn default() -> Self {
        Self::new()
    }
}

impl SoundCache {
    pub fn new() -> Self {
        Self {
            fx_cache: IndexedCache::new(),
            combat_fx_cache: MappedCache::new(),
            source_cache: MappedCache::new(),
            speech_cache: IndexedCache::new(),
            menu_cache: MappedCache::new(),
            quiet_music_pool: Vec::new(),
            alert_music_pool: Vec::new(),
            fight_music_pool: Vec::new(),
            use_3d_sound: false,
            data_check_succeeded: true,
        }
    }

    /// Return whether `check_sound_data` validation has succeeded for
    /// every entry.
    pub fn data_check_succeed(&self) -> bool {
        self.data_check_succeeded
    }

    /// Run the `check_sound_data` pre-flight against every cached
    /// entry: try to load each sample, then unload it. Sets
    /// [`SoundCache::data_check_succeeded`] to `false` if any load
    /// fails. Executed as a single sweep here because `add_entry`
    /// doesn't have a loader at insert time.
    ///
    /// Returns `true` when every entry loaded cleanly (i.e. the flag
    /// stayed at its starting `true` value).
    pub fn validate_data(&mut self, loader: &SampleLoader) -> bool {
        let is_3d = self.use_3d_sound;
        let mut ok = true;

        // Walk every sub-cache. `add_entry` registers the file_name
        // but never attempts I/O; this pass performs the deferred load.
        for entry in &mut self.fx_cache.entries {
            ok &= validate_entry(entry, is_3d, loader);
        }
        for entry in self.combat_fx_cache.entries.values_mut() {
            ok &= validate_entry(entry, is_3d, loader);
        }
        for entry in self.source_cache.entries.values_mut() {
            ok &= validate_entry(entry, is_3d, loader);
        }
        for entry in &mut self.speech_cache.entries {
            ok &= validate_entry(entry, is_3d, loader);
        }
        for entry in self.menu_cache.entries.values_mut() {
            ok &= validate_entry(entry, is_3d, loader);
        }

        if !ok {
            self.data_check_succeeded = false;
        }
        ok
    }

    /// Initialize the FX cache from parsed FX bank elements and combat FX tables.
    pub fn initialize_fx_cache(&mut self, fx_bank_elements: &[FxBankElement]) {
        for element in fx_bank_elements {
            match element.element_type {
                SoundGroupType::RandomGroup | SoundGroupType::MaterialGroup => {
                    let gaps = if element.element_type == SoundGroupType::RandomGroup {
                        element.gaps_count
                    } else {
                        0
                    };
                    // Collect entry indices first, then create group
                    let indices: Vec<usize> = element
                        .sub_elements
                        .iter()
                        .map(|sub| self.fx_cache.add_entry(&sub.file_name))
                        .collect();
                    let group =
                        self.fx_cache
                            .add_group(element.element_id, element.element_type, gaps);
                    group.entry_indices = indices;
                }
                SoundGroupType::Fx => {
                    let name = element
                        .file_name
                        .as_deref()
                        .expect("FX element must have a filename");
                    let idx = self.fx_cache.add_entry(name);
                    let group = self
                        .fx_cache
                        .add_group(element.element_id, SoundGroupType::Fx, 0);
                    group.entry_indices.push(idx);
                }
            }
        }

        // Combat FX cache initialization
        self.initialize_combat_fx_cache();
    }

    /// Initialize the combat FX cache entries from the strike/impact tables.
    fn initialize_combat_fx_cache(&mut self) {
        let mut id: u32 = 0;

        // Strike FX: ssw1/ssw2 variants
        for strike in STRIKE_FX_LIST {
            self.combat_fx_cache
                .add_entry(id, format!("ssw1{strike}.wav"));
            id += 1;
            self.combat_fx_cache
                .add_entry(id, format!("ssw2{strike}.wav"));
            id += 1;
        }

        // Strike FX: slp1/slp2 variants
        for strike in STRIKE_FX_LIST {
            self.combat_fx_cache
                .add_entry(id, format!("slp1{strike}.wav"));
            id += 1;
            self.combat_fx_cache
                .add_entry(id, format!("slp2{strike}.wav"));
            id += 1;
        }

        // Strike FX: shp1/shp2 variants
        for strike in STRIKE_FX_LIST {
            self.combat_fx_cache
                .add_entry(id, format!("shp1{strike}.wav"));
            id += 1;
            self.combat_fx_cache
                .add_entry(id, format!("shp2{strike}.wav"));
            id += 1;
        }

        // Impact FX: ila_ variants
        for impact in IMPACT_FX_LIST {
            self.combat_fx_cache
                .add_entry(id, format!("ila_{impact}.wav"));
            id += 1;
        }

        // Impact FX: iha_ variants
        for impact in IMPACT_FX_LIST {
            self.combat_fx_cache
                .add_entry(id, format!("iha_{impact}.wav"));
            id += 1;
        }
    }

    /// Initialize the sound source cache from a set of required IDs.
    pub fn initialize_sound_source_cache(&mut self, required_ids: &BTreeSet<u32>) {
        for &id in required_ids {
            let file_name = format!("snd_{id:03}.wav");
            self.source_cache.add_entry(id, file_name);
        }
        // Cache stats are reset after the source cache is reseeded.
        self.reset_cache_stats();
    }

    /// Eagerly stamp `loop_sample` on every source-cache entry from the
    /// matching `SoundSource::source_kind`. Must run once after
    /// [`SoundCache::initialize_sound_source_cache`] so the loop flag is
    /// correct before any cache fetch — without this pass, `loop_sample`
    /// would only be populated lazily on the first `get_source_sample`
    /// call.
    pub fn finalize_sound_sources(&mut self, sources: &crate::sound_source::SoundSourceManager) {
        for (id, looping) in sources.get_loop_flags() {
            if let Some(entry) = self.source_cache.entries.get_mut(&id) {
                entry.loop_sample = looping;
            }
        }
    }

    /// Initialize the menu cache from parsed menu bank data.
    pub fn initialize_menu_cache(&mut self, menu_entries: &[(u32, String)]) {
        for (id, file_name) in menu_entries {
            self.menu_cache.add_entry(*id, file_name.clone());
        }
    }

    /// Initialize exclamations for a single profile.
    ///
    /// `sample_paths` maps variant indices to their file paths (resolved
    /// by the resource manager externally).
    pub fn initialize_exclamations_for_profile(&mut self, exclamations: &[(u32, Vec<String>)]) {
        for (action_id, sample_paths) in exclamations {
            // Collect entry indices first, then create group
            let indices: Vec<usize> = sample_paths
                .iter()
                .map(|path| {
                    let full_path = format!("Exclamations/{path}");
                    self.speech_cache.add_entry(full_path)
                })
                .collect();
            tracing::trace!(
                action_id = format!("{action_id:#010x}"),
                n_samples = sample_paths.len(),
                "initialize_exclamations_for_profile group"
            );
            let group = self
                .speech_cache
                .add_group(*action_id, SoundGroupType::RandomGroup, 0);
            group.entry_indices = indices;
        }
    }

    /// Initialize music pools from mission profile data.
    pub fn initialize_music(&mut self, green_music: &str, yellow_music: &str, red_music: &str) {
        self.quiet_music_pool.clear();
        self.alert_music_pool.clear();
        self.fight_music_pool.clear();

        self.quiet_music_pool.push(green_music.to_string());
        self.alert_music_pool.push(yellow_music.to_string());
        self.fight_music_pool.push(red_music.to_string());

        // Non-fatal warnings gated on the global `check_sound_data` flag.
        let check = crate::engine::GlobalOptions::global()
            .as_ref()
            .map(|o| o.check_sound_data)
            .unwrap_or(false);
        if check {
            if green_music.is_empty() {
                tracing::warn!("SoundCache: missing green music loop");
            }
            if yellow_music.is_empty() {
                tracing::warn!("SoundCache: missing yellow music loop");
            }
            if red_music.is_empty() {
                tracing::warn!("SoundCache: missing red music loop");
            }
        }
    }

    /// Add a sound source entry by ID.
    pub fn add_sound_source_entry(&mut self, id: u32, wave_name: &str, loop_sample: bool) {
        self.source_cache.add_entry(id, wave_name);
        if let Some(entry) = self.source_cache.entries.get_mut(&id) {
            entry.loop_sample = loop_sample;
        }
    }

    /// Flush caches. If `flush_all`, also flushes FX, combat, speech, and menu.
    pub fn flush(&mut self, flush_all: bool) {
        if flush_all {
            self.fx_cache.flush();
            self.combat_fx_cache.flush();
            self.speech_cache.flush();
            self.menu_cache.flush();
        }

        self.source_cache.flush();

        self.quiet_music_pool.clear();
        self.alert_music_pool.clear();
        self.fight_music_pool.clear();

        // Reset stats unconditionally regardless of `flush_all`,
        // wiping FX/combat/source/speech stats.
        self.reset_cache_stats();
    }

    /// Zero hits/misses/data_size on FX, combat FX, source, and speech
    /// sub-caches without disturbing entries. Menu stats are
    /// intentionally left alone.
    pub fn reset_cache_stats(&mut self) {
        self.fx_cache.stats = CacheStats::default();
        self.combat_fx_cache.stats = CacheStats::default();
        self.source_cache.stats = CacheStats::default();
        self.speech_cache.stats = CacheStats::default();
    }

    /// Check if a given FX sample ID uses material-based selection.
    pub fn is_material_fx(&self, sample_id: u32) -> bool {
        assert!(sample_id < 65536, "sample_id must fit in 16 bits");
        self.fx_cache
            .groups
            .get(&sample_id)
            .is_some_and(|g| g.group_type == SoundGroupType::MaterialGroup)
    }

    /// Get an FX sample. Returns the index into the FX cache entries.
    pub fn get_fx_sample(
        &mut self,
        sample_present: bool,
        sample_id: u32,
        material: Option<Material>,
        loader: &SampleLoader,
        rng: &mut dyn FnMut(u32) -> u32,
    ) -> Option<usize> {
        assert!(sample_id < 65536, "sample_id must fit in 16 bits");
        self.fx_cache.get_fx_sample(
            sample_present,
            sample_id,
            material,
            self.use_3d_sound,
            FXCACHE_TTL_INIT,
            FXCACHE_TTL_INCREMENT,
            loader,
            rng,
        )
    }

    /// Get a combat FX sample.
    pub fn get_combat_fx_sample(
        &mut self,
        sample_present: bool,
        sample_id: u32,
        loader: &SampleLoader,
    ) -> Option<&mut SoundCacheEntry> {
        self.combat_fx_cache.get_sample(
            sample_present,
            sample_id,
            false,
            self.use_3d_sound,
            FXCACHE_TTL_INIT,
            FXCACHE_TTL_INCREMENT,
            loader,
        )
    }

    /// Get a source sample.
    pub fn get_source_sample(
        &mut self,
        sample_present: bool,
        sample_id: u32,
        loop_sample: bool,
        loader: &SampleLoader,
    ) -> Option<&mut SoundCacheEntry> {
        self.source_cache.get_sample(
            sample_present,
            sample_id,
            loop_sample,
            self.use_3d_sound,
            SOURCECACHE_TTL_INIT,
            SOURCECACHE_TTL_INCREMENT,
            loader,
        )
    }

    /// Get an exclamation/speech sample. Returns index into speech cache.
    pub fn get_exclamation_sample(
        &mut self,
        sample_present: bool,
        exclamation_id: u32,
        variant: Option<u32>,
        loader: &SampleLoader,
        rng: &mut dyn FnMut(u32) -> u32,
    ) -> Option<usize> {
        self.speech_cache.get_exclamation_sample(
            sample_present,
            exclamation_id,
            variant,
            self.use_3d_sound,
            SPEECHCACHE_TTL_INIT,
            SPEECHCACHE_TTL_INCREMENT,
            loader,
            rng,
        )
    }

    /// Get a menu sample.
    ///
    /// Hits land on `menu_cache.stats.hits`; misses are booked against
    /// `source_cache.stats` (the menu cache reuses the source cache's
    /// stats counter for the miss/load accounting path).
    pub fn get_menu_sample(
        &mut self,
        sample_id: u32,
        loader: &SampleLoader,
    ) -> Option<&mut SoundCacheEntry> {
        let loaded_size: Option<u32> = {
            let entry = self.menu_cache.entries.get_mut(&sample_id)?;
            if entry.is_loaded() {
                self.menu_cache.stats.hits += 1;
                entry.time_to_live += MENUCACHE_TTL_INCREMENT;
                None
            } else if entry.load_sample(false, MENUCACHE_TTL_INIT, loader) {
                Some(entry.sample_size)
            } else {
                None
            }
        };

        if let Some(size) = loaded_size {
            self.source_cache.stats.misses += 1;
            self.source_cache.stats.data_size += size;
        }

        self.menu_cache.entries.get_mut(&sample_id)
    }

    /// Update all cache TTLs, unloading expired entries.
    pub fn update_cache_state(&mut self) {
        self.fx_cache.update_ttl(FXCACHE_TTL_DECREMENT);
        self.combat_fx_cache.update_ttl(FXCACHE_TTL_DECREMENT);
        self.source_cache.update_ttl(SOURCECACHE_TTL_DECREMENT);
        self.speech_cache.update_ttl(SPEECHCACHE_TTL_DECREMENT);
        self.menu_cache.update_ttl(MENUCACHE_TTL_DECREMENT);
    }

    /// Invalidate all caches (unload all samples).
    pub fn invalidate_cache(&mut self) {
        self.fx_cache.invalidate();
        self.combat_fx_cache.invalidate();
        self.source_cache.invalidate();
        self.speech_cache.invalidate();
        self.menu_cache.invalidate();
    }

    /// Get music by index from the quiet pool.
    pub fn get_quiet_music(&self, index: usize) -> Option<&str> {
        self.quiet_music_pool.get(index).map(|s| s.as_str())
    }

    /// Get music by index from the alert pool.
    pub fn get_alert_music(&self, index: usize) -> Option<&str> {
        self.alert_music_pool.get(index).map(|s| s.as_str())
    }

    /// Get music by index from the fight pool.
    pub fn get_fight_music(&self, index: usize) -> Option<&str> {
        self.fight_music_pool.get(index).map(|s| s.as_str())
    }

    /// Get combined cache statistics: `[fx, source, speech, global]`.
    pub fn get_cache_stats(&self) -> [CacheStats; 4] {
        let fx = self.fx_cache.stats.clone();
        let source = self.source_cache.stats.clone();
        let speech = self.speech_cache.stats.clone();
        let global = CacheStats {
            hits: fx.hits + source.hits + speech.hits,
            misses: fx.misses + source.misses + speech.misses,
            data_size: fx.data_size + source.data_size + speech.data_size,
        };
        [fx, source, speech, global]
    }

    /// Find the ID for a source cache entry by file name.
    pub fn get_id_for_source_entry(&self, file_name: &str) -> Option<u32> {
        self.source_cache.id_for_entry_by_filename(file_name)
    }

    /// Get a source cache entry by ID.
    pub fn get_source_entry_for_id(&self, id: u32) -> Option<&SoundCacheEntry> {
        self.source_cache.entries.get(&id)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_loader(name: &str) -> Option<(Vec<u8>, u32, u32)> {
        // Return fake data sized by the name length
        let size = (name.len() as u32) * 100;
        Some((vec![0u8; size as usize], size, size / 4))
    }

    fn make_rng(seed: u32) -> impl FnMut(u32) -> u32 {
        let mut state = seed;
        move |max| {
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            (state >> 16) % max
        }
    }

    #[test]
    fn cache_entry_load_unload() {
        let mut entry = SoundCacheEntry::new("test.wav");
        assert!(!entry.is_loaded());
        assert_eq!(entry.sample_size, 0);

        let loaded = entry.load_sample(false, 400, &dummy_loader);
        assert!(loaded);
        assert!(entry.is_loaded());
        assert_eq!(entry.time_to_live, 400);
        assert!(entry.sample_size > 0);

        entry.unload_sample();
        assert!(!entry.is_loaded());
        assert_eq!(entry.time_to_live, 0);
        assert_eq!(entry.playing, 0);
    }

    #[test]
    fn cache_entry_failed_load() {
        let mut entry = SoundCacheEntry::new("missing.wav");
        let fail_loader = |_: &str| -> Option<(Vec<u8>, u32, u32)> { None };
        let loaded = entry.load_sample(false, 400, &fail_loader);
        assert!(!loaded);
        assert!(!entry.is_loaded());
    }

    #[test]
    fn indexed_cache_add_and_lookup() {
        let mut cache = IndexedCache::new();
        let idx = cache.add_entry("sound1.wav");
        assert_eq!(idx, 0);
        let idx2 = cache.add_entry("sound2.wav");
        assert_eq!(idx2, 1);
        assert_eq!(cache.entries.len(), 2);
        assert_eq!(cache.entries[0].file_name, "sound1.wav");
    }

    #[test]
    #[should_panic(expected = "already allocated")]
    fn indexed_cache_duplicate_group_panics() {
        let mut cache = IndexedCache::new();
        cache.add_group(1, SoundGroupType::Fx, 0);
        cache.add_group(1, SoundGroupType::Fx, 0);
    }

    #[test]
    fn indexed_cache_fx_single() {
        let mut cache = IndexedCache::new();
        let idx = cache.add_entry("boom.wav");
        let group = cache.add_group(42, SoundGroupType::Fx, 0);
        group.entry_indices.push(idx);

        let mut rng = make_rng(1);
        let result = cache.get_fx_sample(
            true,
            42,
            None,
            false,
            FXCACHE_TTL_INIT,
            FXCACHE_TTL_INCREMENT,
            &dummy_loader,
            &mut rng,
        );
        assert_eq!(result, Some(0));
        assert!(cache.entries[0].is_loaded());
        assert_eq!(cache.stats.misses, 1);

        // Second access should be a hit
        let result2 = cache.get_fx_sample(
            true,
            42,
            None,
            false,
            FXCACHE_TTL_INIT,
            FXCACHE_TTL_INCREMENT,
            &dummy_loader,
            &mut rng,
        );
        assert_eq!(result2, Some(0));
        assert_eq!(cache.stats.hits, 1);
    }

    #[test]
    fn indexed_cache_material_group() {
        let mut cache = IndexedCache::new();

        // Add 9 entries (one per material)
        let mut indices = Vec::new();
        for i in 0..Material::NUM_MATERIALS {
            indices.push(cache.add_entry(format!("mat_{i}.wav")));
        }

        let group = cache.add_group(10, SoundGroupType::MaterialGroup, 0);
        group.entry_indices = indices;

        let mut rng = make_rng(1);
        let result = cache.get_fx_sample(
            true,
            10,
            Some(Material::Wood),
            false,
            FXCACHE_TTL_INIT,
            FXCACHE_TTL_INCREMENT,
            &dummy_loader,
            &mut rng,
        );
        assert_eq!(result, Some(Material::Wood as usize));
        assert!(cache.entries[Material::Wood as usize].is_loaded());
    }

    #[test]
    fn indexed_cache_random_group_with_gaps() {
        let mut cache = IndexedCache::new();
        let idx0 = cache.add_entry("r0.wav");
        let idx1 = cache.add_entry("r1.wav");
        let group = cache.add_group(20, SoundGroupType::RandomGroup, 2);
        group.entry_indices.push(idx0);
        group.entry_indices.push(idx1);

        // With 2 sounds + 2 gaps = total 4 slots.
        // Some selections will produce None (gaps), some will produce entries.
        let mut hits = 0;
        let mut gaps = 0;
        for seed in 0..100 {
            let mut rng = make_rng(seed);
            // Reset loaded state for clean test
            cache.entries[0].sample_data = None;
            cache.entries[1].sample_data = None;
            match cache.get_fx_sample(
                true,
                20,
                None,
                false,
                FXCACHE_TTL_INIT,
                FXCACHE_TTL_INCREMENT,
                &dummy_loader,
                &mut rng,
            ) {
                Some(_) => hits += 1,
                None => gaps += 1,
            }
        }
        // With fair RNG, we expect roughly 50/50
        assert!(hits > 0, "Should have some hits");
        assert!(gaps > 0, "Should have some gap selections");
    }

    #[test]
    fn mapped_cache_get_sample() {
        let mut cache = MappedCache::new();
        cache.add_entry(5, "test5.wav");

        let entry = cache.get_sample(
            true,
            5,
            false,
            false,
            SOURCECACHE_TTL_INIT,
            SOURCECACHE_TTL_INCREMENT,
            &dummy_loader,
        );
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert!(entry.is_loaded());
        assert_eq!(cache.stats.misses, 1);
    }

    #[test]
    fn mapped_cache_missing_id() {
        let mut cache = MappedCache::new();
        let entry = cache.get_sample(
            true,
            999,
            false,
            false,
            SOURCECACHE_TTL_INIT,
            SOURCECACHE_TTL_INCREMENT,
            &dummy_loader,
        );
        assert!(entry.is_none());
    }

    #[test]
    fn ttl_eviction_indexed() {
        let mut cache = IndexedCache::new();
        let idx = cache.add_entry("evict.wav");
        cache.entries[idx].load_sample(false, 10, &dummy_loader);
        cache.stats.data_size += cache.entries[idx].sample_size;

        assert!(cache.entries[idx].is_loaded());

        // Decrement enough to reach zero
        cache.update_ttl(5);
        assert!(cache.entries[idx].is_loaded());
        assert_eq!(cache.entries[idx].time_to_live, 5);

        cache.update_ttl(5);
        assert!(!cache.entries[idx].is_loaded());
        assert_eq!(cache.stats.data_size, 0);
    }

    #[test]
    fn ttl_eviction_mapped() {
        let mut cache = MappedCache::new();
        cache.add_entry(1, "evict.wav");
        let entry = cache.entries.get_mut(&1).unwrap();
        entry.load_sample(false, 10, &dummy_loader);
        cache.stats.data_size = entry.sample_size;

        cache.update_ttl(11);
        assert!(!cache.entries.get(&1).unwrap().is_loaded());
        assert_eq!(cache.stats.data_size, 0);
    }

    #[test]
    fn playing_prevents_eviction() {
        let mut cache = MappedCache::new();
        cache.add_entry(1, "playing.wav");
        let entry = cache.entries.get_mut(&1).unwrap();
        entry.load_sample(false, 5, &dummy_loader);
        entry.playing = 1; // currently playing
        cache.stats.data_size = entry.sample_size;

        cache.update_ttl(100);
        // Should NOT be evicted because it's playing
        assert!(cache.entries.get(&1).unwrap().is_loaded());
    }

    #[test]
    fn combat_fx_initialization() {
        let mut sc = SoundCache::new();
        sc.initialize_fx_cache(&[]);

        // Expected count:
        // 3 prefixes * 10 strike * 2 variants = 60
        // 2 prefixes * 12 impact = 24
        // Total = 84
        assert_eq!(sc.combat_fx_cache.entries.len(), 84);

        // Check first entry
        assert_eq!(
            sc.combat_fx_cache.entries.get(&0).unwrap().file_name,
            "ssw1wowo.wav"
        );
    }

    #[test]
    fn sound_source_initialization() {
        let mut sc = SoundCache::new();
        let mut ids = BTreeSet::new();
        ids.insert(1);
        ids.insert(42);
        ids.insert(100);
        sc.initialize_sound_source_cache(&ids);

        assert_eq!(sc.source_cache.entries.len(), 3);
        assert_eq!(
            sc.source_cache.entries.get(&42).unwrap().file_name,
            "snd_042.wav"
        );
    }

    #[test]
    fn music_pool() {
        let mut sc = SoundCache::new();
        sc.initialize_music("green.ogg", "yellow.ogg", "red.ogg");

        assert_eq!(sc.get_quiet_music(0), Some("green.ogg"));
        assert_eq!(sc.get_alert_music(0), Some("yellow.ogg"));
        assert_eq!(sc.get_fight_music(0), Some("red.ogg"));
        assert_eq!(sc.get_quiet_music(1), None);
    }

    #[test]
    fn flush_partial_and_full() {
        let mut sc = SoundCache::new();
        sc.initialize_fx_cache(&[]);
        sc.initialize_music("a.ogg", "b.ogg", "c.ogg");

        let mut ids = BTreeSet::new();
        ids.insert(5);
        sc.initialize_sound_source_cache(&ids);

        // Partial flush — only source + music
        sc.flush(false);
        assert!(!sc.combat_fx_cache.entries.is_empty());
        assert!(sc.source_cache.entries.is_empty());
        assert!(sc.quiet_music_pool.is_empty());

        // Re-add source for full flush test
        let mut ids2 = BTreeSet::new();
        ids2.insert(10);
        sc.initialize_sound_source_cache(&ids2);

        sc.flush(true);
        assert!(sc.combat_fx_cache.entries.is_empty());
        assert!(sc.fx_cache.entries.is_empty());
        assert!(sc.speech_cache.entries.is_empty());
        assert!(sc.menu_cache.entries.is_empty());
    }

    #[test]
    fn is_material_fx() {
        let mut sc = SoundCache::new();

        // Add a material group
        let mut indices = Vec::new();
        for i in 0..Material::NUM_MATERIALS {
            indices.push(sc.fx_cache.add_entry(format!("m{i}.wav")));
        }
        let group = sc.fx_cache.add_group(100, SoundGroupType::MaterialGroup, 0);
        group.entry_indices = indices;

        // Add a non-material group
        let idx = sc.fx_cache.add_entry("single.wav");
        let group2 = sc.fx_cache.add_group(101, SoundGroupType::Fx, 0);
        group2.entry_indices.push(idx);

        assert!(sc.is_material_fx(100));
        assert!(!sc.is_material_fx(101));
        assert!(!sc.is_material_fx(999));
    }

    #[test]
    fn cache_stats_aggregation() {
        let mut sc = SoundCache::new();
        sc.fx_cache.stats = CacheStats {
            hits: 10,
            misses: 2,
            data_size: 1000,
        };
        sc.source_cache.stats = CacheStats {
            hits: 5,
            misses: 1,
            data_size: 500,
        };
        sc.speech_cache.stats = CacheStats {
            hits: 3,
            misses: 4,
            data_size: 300,
        };

        let stats = sc.get_cache_stats();
        assert_eq!(stats[0].hits, 10); // FX
        assert_eq!(stats[1].hits, 5); // Source
        assert_eq!(stats[2].hits, 3); // Speech
        assert_eq!(stats[3].hits, 18); // Global sum
        assert_eq!(stats[3].data_size, 1800);
    }

    #[test]
    fn update_cache_state_all() {
        let mut sc = SoundCache::new();
        sc.initialize_fx_cache(&[]);

        // Load a combat fx entry
        if let Some(entry) = sc.combat_fx_cache.entries.get_mut(&0) {
            entry.load_sample(false, 10, &dummy_loader);
            sc.combat_fx_cache.stats.data_size = entry.sample_size;
        }

        // Should decrement TTL
        sc.update_cache_state();
        let entry = sc.combat_fx_cache.entries.get(&0).unwrap();
        assert_eq!(
            entry.time_to_live,
            10u32.saturating_sub(FXCACHE_TTL_DECREMENT)
        );
    }

    #[test]
    fn serde_roundtrip() {
        let mut sc = SoundCache::new();
        sc.initialize_music("a.ogg", "b.ogg", "c.ogg");
        sc.use_3d_sound = true;

        let json = serde_json::to_string(&sc).unwrap();
        let restored: SoundCache = serde_json::from_str(&json).unwrap();

        assert!(restored.use_3d_sound);
        assert_eq!(restored.get_quiet_music(0), Some("a.ogg"));
    }

    #[test]
    fn parse_fx_bank_basic() {
        // Build a minimal FX bank: magic + header_size + version + count(1) + one FX element
        let mut data = Vec::new();
        data.extend_from_slice(b"FXBK");
        data.extend_from_slice(&0u32.to_le_bytes()); // header_size (unused)
        data.extend_from_slice(&1u32.to_le_bytes()); // version
        data.extend_from_slice(&1u32.to_le_bytes()); // count

        // One TYPE_FX element
        data.extend_from_slice(&2u32.to_le_bytes()); // TYPE_FX
        data.extend_from_slice(&42u32.to_le_bytes()); // element ID
        data.extend_from_slice(&100u16.to_le_bytes()); // logical volume
        // Filename: "boom.wav" (8 chars)
        let name = b"boom.wav";
        data.extend_from_slice(&(name.len() as u16).to_le_bytes());
        data.extend_from_slice(name);

        let elements = parse_fx_bank(&data).unwrap();
        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].element_id, 42);
        assert_eq!(elements[0].file_name.as_deref(), Some("boom.wav"));
    }

    #[test]
    fn parse_fx_bank_invalid_magic() {
        let data = b"NOPE0000000000000000";
        assert!(parse_fx_bank(data).is_err());
    }

    #[test]
    fn parse_exclamation_basic() {
        let mut data = Vec::new();
        data.extend_from_slice(b"NEUF");
        data.extend_from_slice(&1u32.to_le_bytes()); // version
        data.extend_from_slice(&99u32.to_le_bytes()); // table_id
        data.extend_from_slice(&2u32.to_le_bytes()); // num_exclamations

        // Exclamation 0: 2 variants
        data.extend_from_slice(&2u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes()); // variant 0
        data.extend_from_slice(&1u32.to_le_bytes()); // variant 1

        // Exclamation 1: 1 variant
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&5u32.to_le_bytes()); // variant 5

        let prefix = 0x00010000u32;
        let (table_id, result) = parse_exclamation_file(&data, prefix).unwrap();
        assert_eq!(table_id, 99);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, prefix);
        assert_eq!(result[0].1, vec![0, 1]);
        assert_eq!(result[1].0, prefix | 1);
        assert_eq!(result[1].1, vec![5]);
    }

    #[test]
    fn exclamation_sample_specific_variant() {
        let mut sc = SoundCache::new();
        let exclamations = vec![(100u32, vec!["hello.wav".to_string(), "hi.wav".to_string()])];
        sc.initialize_exclamations_for_profile(&exclamations);

        let mut rng = make_rng(1);
        let result = sc.get_exclamation_sample(true, 100, Some(1), &dummy_loader, &mut rng);
        assert!(result.is_some());
        let idx = result.unwrap();
        assert_eq!(
            sc.speech_cache.entries[idx].file_name,
            "Exclamations/hi.wav"
        );
    }

    #[test]
    fn exclamation_sample_random_variant() {
        let mut sc = SoundCache::new();
        let exclamations = vec![(
            200u32,
            vec![
                "a.wav".to_string(),
                "b.wav".to_string(),
                "c.wav".to_string(),
            ],
        )];
        sc.initialize_exclamations_for_profile(&exclamations);

        let mut seen = std::collections::HashSet::new();
        for seed in 0..50 {
            let mut rng = make_rng(seed);
            // Reset loaded state
            for e in &mut sc.speech_cache.entries {
                e.sample_data = None;
            }
            if let Some(idx) = sc.get_exclamation_sample(true, 200, None, &dummy_loader, &mut rng) {
                seen.insert(sc.speech_cache.entries[idx].file_name.clone());
            }
        }
        // Should have picked multiple different variants
        assert!(seen.len() > 1, "Random selection should produce variety");
    }

    #[test]
    fn invalidate_cache_all() {
        let mut sc = SoundCache::new();
        sc.initialize_fx_cache(&[]);

        // Load some combat fx
        for id in 0..5 {
            if let Some(entry) = sc.combat_fx_cache.entries.get_mut(&id) {
                entry.load_sample(false, 100, &dummy_loader);
            }
        }

        sc.invalidate_cache();

        for entry in sc.combat_fx_cache.entries.values() {
            assert!(!entry.is_loaded());
        }
    }

    #[test]
    fn data_check_succeed_default_true() {
        let sc = SoundCache::new();
        assert!(sc.data_check_succeed());
    }

    #[test]
    fn validate_data_all_present() {
        let mut sc = SoundCache::new();
        sc.add_sound_source_entry(1, "a.wav", false);
        sc.add_sound_source_entry(2, "b.wav", false);
        // dummy_loader always succeeds — flag stays true.
        assert!(sc.validate_data(&dummy_loader));
        assert!(sc.data_check_succeed());
        // Entries should be unloaded after the validate sweep.
        for entry in sc.source_cache.entries.values() {
            assert!(!entry.is_loaded());
        }
    }

    #[test]
    fn validate_data_missing_sample_clears_flag() {
        let mut sc = SoundCache::new();
        sc.add_sound_source_entry(1, "a.wav", false);
        sc.add_sound_source_entry(2, "missing.wav", false);
        let loader = |name: &str| -> Option<(Vec<u8>, u32, u32)> {
            if name == "missing.wav" {
                None
            } else {
                Some((vec![0u8; 4], 4, 100))
            }
        };
        assert!(!sc.validate_data(&loader));
        assert!(!sc.data_check_succeed());
    }

    #[test]
    fn source_entry_lookup() {
        let mut sc = SoundCache::new();
        sc.add_sound_source_entry(42, "mysound.wav", true);

        let entry = sc.get_source_entry_for_id(42);
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().file_name, "mysound.wav");
        assert!(entry.unwrap().loop_sample);

        assert_eq!(sc.get_id_for_source_entry("mysound.wav"), Some(42));
        assert_eq!(sc.get_id_for_source_entry("nope.wav"), None);
    }
}
