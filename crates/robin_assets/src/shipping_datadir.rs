//! Shipping datadir manifest and lazily loaded mission payloads.
//!
//! Produced by the `convert_datadir --format shipping` binary and loaded
//! at engine startup (see [`try_load`]). When a shipping datadir is
//! present, individual subsystem loaders (`ProfileManager::load_all_legacy_cpf`,
//! `FrameHolder::initialize_sprite_bank`, `ResourceManager::attach_resource_file`,
//! etc.) consult it instead of reading legacy files off disk.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::frame_holder::{FrameDictionary, UNMAPPED_DICT};
use crate::res_descr::LevelDescriptors;
use crate::resource_manager::{EncodedPicture, ResourceManager};
use crate::scb::ScbFile;
use robin_engine::level_data::LoadedLevel;
use robin_engine::profiles::ProfileManager;
use robin_engine::sprite_script::SpriteInfo;

mod codec;
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
mod decode_jobs;
mod runtime;
#[cfg(any(test, all(target_arch = "wasm32", feature = "wasm-threads")))]
mod scheduler;
#[cfg(any(test, all(target_arch = "wasm32", feature = "wasm-threads")))]
mod scheduler_rle;
mod sprite_bank;
#[cfg(test)]
use codec::{SHIPPING_DATADIR_MAGIC, SHIPPING_MISSION_MAGIC};
pub use codec::{
    SHIPPING_DATADIR_VERSION, SHIPPING_EXPANDED_BYTE_LIMIT, SHIPPING_MISSION_VERSION,
    decode_mission_compressed, decompress_shipping_with_limit, encode_mission_native,
    encode_native, zstd_compress_with_window, zstd_max_compress,
};
use codec::{decode_native, zstd_decompress};
#[cfg(test)]
use runtime::audio_lookup_keys;
pub use runtime::{
    is_locale_overlay_key, is_optional_english_fallback_key, is_required_locale_key,
};
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
pub use scheduler::VqDecodeScheduler;
#[cfg(test)]
use scheduler::vq_downstream_costs;
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
pub use scheduler_rle::RleJxlDecodeScheduler;
#[cfg(test)]
use scheduler_rle::order_rle_chunks_by_size;

/// Top-level shipping payload.
///
/// Keys mirror the on-disk relative path under `Data/` so loaders can find
/// things under the same names they use for legacy I/O (e.g.
/// `"Interface/DEFAULT.RES"`, `"Levels/Dem_Lei_MP.rhm"`).
#[derive(Default, Debug, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct ShippingDatadirPayload {
    pub profiles: Option<ProfileManager>,
    pub res_files: std::collections::BTreeMap<String, ResourceManager>,
    pub pak_files: std::collections::BTreeMap<String, Vec<EncodedPicture>>,
    pub red_files: std::collections::BTreeMap<String, LevelDescriptors>,
    /// Keyed by mission base name (no extension), e.g. `"Dem_Lei_MP"`.
    pub levels: std::collections::BTreeMap<String, LoadedLevel>,
    pub scripts: std::collections::BTreeMap<String, ScbFile>,
    /// Keyed by the full relative path `Characters/<name>.rhs`.
    pub rhs_files: std::collections::BTreeMap<String, RhsData>,
    /// Packed sprite pool. See [`ShippingSpriteBank`].
    pub sprite_bank: Option<ShippingSpriteBank>,
    /// Terrain bitmaps and other not-yet-parsed binary blobs, keyed by
    /// relative path (e.g. `Levels/Day/leicester.map`).
    pub raw: std::collections::BTreeMap<String, Vec<u8>>,
    /// Source-authoritative durations for boot audio stored in `raw`.
    pub audio_durations_ms: BTreeMap<String, u32>,
    /// Standalone browser audio, keyed by the normalized logical Opus path
    /// (for example `sounds/snd_001.opus`). The encoded bytes intentionally do
    /// not live in this bitcode manifest or any mission payload.
    pub audio_assets: BTreeMap<String, ShippingAudioAsset>,
    /// Independently compressed payload to fetch before starting each mission.
    pub missions: BTreeMap<String, ShippingMissionRef>,
    /// Content-addressed RHS payloads required when a character profile can
    /// participate in the selected mission. Keys are stable CPF character
    /// profile indices; values include that exact physical character RHS and
    /// the object/projectile RHS files enabled by its actions.
    pub character_rhs_files: BTreeMap<u32, Vec<String>>,
    /// Content-addressed localized voice payloads for each CPF character
    /// profile. Runtime party/reinforcement selection uses the same profile
    /// closure as `character_rhs_files`, avoiding every PC voice in every
    /// mission reference.
    pub character_audio_files: BTreeMap<u32, Vec<String>>,
    /// Exclamation profile id corresponding to each CPF character profile.
    pub character_exclamation_ids: BTreeMap<u32, u32>,
    /// Authored soldier/civilian/required/rescue exclamation ids for each
    /// mission. Dynamic party ids are unioned at the mission-load boundary.
    pub mission_exclamation_ids: BTreeMap<String, Vec<u32>>,
    /// Conservative RHS closure used only when constructing a mission around
    /// an already-decoded saved world. Saved entities may contain object types
    /// that are neither authored by the destination mission nor implied by its
    /// current party, so save launches must not silently omit their masters.
    pub saved_world_rhs_files: Vec<String>,
    /// Language packs keyed by canonical BCP-47 locale (`"en-US"`,
    /// `"de-DE"`, ...). Windows' invariant LCID 2047 is represented as
    /// `"und"`; its legacy `"2047"` and `"neutral"` names remain accepted
    /// aliases but are not promoted to a made-up language identity.
    #[serde(default)]
    pub locales: BTreeMap<String, ShippingLocale>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct ShippingRuntime {
    #[serde(skip)]
    installation_id: OnceLock<u64>,
    /// Runtime-only source directory containing `datadir.bin` and its payloads.
    source_dir: Option<PathBuf>,
    /// Runtime-only HTTP base used by the browser build.
    remote_base_url: Option<String>,
    /// Runtime VFS and shared boot bytes are published together after a
    /// successful mount, never independently during a failed installation.
    #[serde(skip)]
    installed: OnceLock<ShippingInstallation>,
    #[serde(skip)]
    installation_lock: Mutex<()>,
    /// Payloads already installed for this process. Kept out of the manifest.
    #[serde(skip)]
    loaded_missions: RwLock<BTreeMap<String, Arc<ShippingMission>>>,
    /// Host-authenticated compressed split payloads staged by the browser
    /// before mission selection. Values remain shared so decoding does not
    /// copy a potentially large package file merely to cross the async seam.
    #[serde(skip)]
    preloaded_files: RwLock<BTreeMap<String, Arc<Vec<u8>>>>,
    /// Exact static + dynamic exclamation closure for the active mission.
    #[serde(skip)]
    active_exclamation_ids: RwLock<BTreeSet<u32>>,
    /// Runtime-only, lazily shared copies of locale raw-file bundles. Locale
    /// switching can mount these on native, browser, and Android VFSes without
    /// re-cloning every byte on each switch.
    #[serde(skip)]
    locale_bundle_cache: RwLock<BTreeMap<String, Arc<robin_util::asset_fs::Bundle>>>,
}

/// Decoded shipping data. Runtime lookups require a successful
/// `ShippingAssets::install`; decoding alone grants no filesystem authority.
/// Runtime caches and publication state never participate in shipping bytes.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ShippingDatadir {
    #[serde(flatten)]
    payload: ShippingDatadirPayload,
    #[serde(skip)]
    runtime: ShippingRuntime,
}

impl std::ops::Deref for ShippingDatadir {
    type Target = ShippingDatadirPayload;
    fn deref(&self) -> &Self::Target {
        &self.payload
    }
}
impl std::ops::DerefMut for ShippingDatadir {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.payload
    }
}
impl ShippingDatadir {
    /// Stable process-local identity; unlike addresses, identities are never reused.
    pub fn installation_id(&self) -> u64 {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        *self.runtime.installation_id.get_or_init(|| {
            NEXT.try_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |id| id.checked_add(1),
            )
            .expect("shipping installation identity exhausted")
        })
    }
    /// Explicitly installed runtime authority. Decoded converter data has none.
    pub fn asset_vfs(&self) -> &Arc<robin_util::asset_fs::AssetVfs> {
        &self
            .runtime
            .installed
            .get()
            .expect(
                "runtime shipping lookup requires ShippingAssets::install; decoded data has no VFS",
            )
            .vfs
    }

    pub fn selection_snapshot(&self) -> robin_util::asset_fs::AssetSelection {
        self.asset_vfs().selection_snapshot()
    }

    pub fn payload(&self) -> &ShippingDatadirPayload {
        &self.payload
    }
    pub fn payload_mut(&mut self) -> &mut ShippingDatadirPayload {
        &mut self.payload
    }
    pub fn from_payload(payload: ShippingDatadirPayload) -> Self {
        Self {
            payload,
            runtime: ShippingRuntime::default(),
        }
    }
}

/// Serializable reference to one content-addressed standalone audio file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct ShippingAudioAsset {
    /// Path relative to the directory containing `datadir.bin`. For a
    /// bundled asset this names the logical-group bundle file shared with
    /// its neighbors (see `bundle_offset`).
    pub file: String,
    pub encoded_size: u32,
    /// Duration derived from the source asset, not from the transcoded stream.
    pub duration_ms: u32,
    /// When set, this asset's encoded bytes are the
    /// `bundle_offset..bundle_offset + encoded_size` slice of `file`.
    /// Small assets are concatenated into one bundle per logical group
    /// (per-actor voice, per-mission dialogue, common sfx, menu) so the
    /// browser fetches one file per group instead of thousands of tiny
    /// requests; large assets (music, ambience) stay standalone.
    pub bundle_offset: Option<u32>,
}

/// Browser-ready standalone audio reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteAudioAsset {
    /// URL of the asset — of its group bundle when `bundle_offset` is set.
    pub url: String,
    pub encoded_size: u32,
    pub duration_ms: u32,
    /// Slice start within the bundle at `url` (see [`ShippingAudioAsset`]).
    pub bundle_offset: Option<u32>,
}

/// A complete language overlay embedded in a shipping datadir.
///
/// Parsed resource maps avoid reparsing hot UI/mission text during a switch;
/// `raw` contains the same locale's VFS-visible files, including speech and
/// cinematics. Keeping the raw overlay in the platform-neutral manifest makes
/// the representation identical for native, browser, and Android builds.
#[derive(Default, Debug, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct ShippingLocale {
    /// Original Windows locale directory name when the pack came from a
    /// legacy datadir (for example `"1031"`).
    pub source_lcid: Option<String>,
    /// Discovery aliases retained from the input. These are metadata only;
    /// the containing `ShippingDatadir::locales` key is authoritative.
    pub aliases: BTreeSet<String>,
    pub profiles: Option<ProfileManager>,
    pub res_files: BTreeMap<String, ResourceManager>,
    pub pak_files: BTreeMap<String, Vec<EncodedPicture>>,
    pub red_files: BTreeMap<String, LevelDescriptors>,
    /// VFS bundle keys relative to `Data/`, normalized to lowercase `/` paths.
    pub raw: BTreeMap<String, Vec<u8>>,
}

/// Map a legacy Windows LCID to its canonical shipping locale key.
pub fn locale_id_from_lcid(lcid: &str) -> Option<&'static str> {
    Some(match lcid {
        "1028" => "zh-TW",
        "1029" => "cs-CZ",
        "1031" => "de-DE",
        "1033" => "en-US",
        "1036" => "fr-FR",
        "1040" => "it-IT",
        "1041" => "ja-JP",
        "1042" => "ko-KR",
        "1045" => "pl-PL",
        "1046" => "pt-BR",
        "1049" => "ru-RU",
        "1054" => "th-TH",
        "2047" => "und",
        "2052" => "zh-CN",
        "2070" => "pt-PT",
        "3082" => "es-ES",
        _ => return None,
    })
}

/// Canonicalize a shipping locale lookup. Numeric LCIDs and `neutral` are
/// accepted so old launch/config values can discover the new canonical keys.
pub fn canonical_locale_id(locale: &str) -> Result<String> {
    let locale = locale.trim();
    if locale.is_empty() {
        return Err(anyhow!("locale id must not be empty"));
    }
    if let Some(mapped) = locale_id_from_lcid(locale) {
        return Ok(mapped.to_owned());
    }
    if locale.eq_ignore_ascii_case("neutral") {
        return Ok("und".to_owned());
    }

    let normalized = locale.replace('_', "-");
    let parts: Vec<&str> = normalized.split('-').collect();
    if parts.is_empty()
        || parts[0].len() < 2
        || parts[0].len() > 8
        || parts
            .iter()
            .any(|part| part.is_empty() || part.len() > 8 || !part.is_ascii())
        || !parts[0].bytes().all(|byte| byte.is_ascii_alphabetic())
        || parts[1..]
            .iter()
            .any(|part| !part.bytes().all(|byte| byte.is_ascii_alphanumeric()))
    {
        return Err(anyhow!("invalid BCP-47 locale id '{locale}'"));
    }

    let mut canonical = Vec::with_capacity(parts.len());
    canonical.push(parts[0].to_ascii_lowercase());
    for part in &parts[1..] {
        let value = if part.len() == 4 && part.bytes().all(|byte| byte.is_ascii_alphabetic()) {
            let mut chars = part.chars();
            let first = chars
                .next()
                .expect("four-character script subtag is non-empty")
                .to_ascii_uppercase();
            format!("{first}{}", chars.as_str().to_ascii_lowercase())
        } else if (part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_alphabetic()))
            || (part.len() == 3 && part.bytes().all(|byte| byte.is_ascii_digit()))
        {
            part.to_ascii_uppercase()
        } else {
            part.to_ascii_lowercase()
        };
        canonical.push(value);
    }
    Ok(canonical.join("-"))
}

/// Normalize a path used as a parsed-resource or raw-bundle key.
pub fn canonical_shipping_asset_key(path: &str) -> String {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    normalized
        .strip_prefix("data/")
        .unwrap_or(&normalized)
        .trim_start_matches('/')
        .to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct ShippingMissionRef {
    /// Proto-level forest flag used by original-game PC initialization to select
    /// RobinHood (forest) or RobinTown (non-forest) before RHS dependencies
    /// are fetched.
    pub forest_level: bool,
    /// Paths relative to the directory containing `datadir.bin`. Shared RHS
    /// and terrain payloads can be named by several missions without being
    /// stored twice.
    pub files: Vec<String>,
}

/// All data whose lifetime starts when one mission is selected.
#[derive(Default, Debug, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct ShippingMissionPayload {
    pub levels: BTreeMap<String, LoadedLevel>,
    pub scripts: BTreeMap<String, ScbFile>,
    pub rhs_files: BTreeMap<String, RhsData>,
    pub sprite_bank: Option<ShippingSpriteBank>,
    pub raw: BTreeMap<String, Vec<u8>>,
    /// Exact durations from the source assets, keyed like `raw`.
    ///
    /// Web shipping may transcode WAV/Vorbis to Opus. Simulation timing must
    /// continue to use the authoritative source duration rather than codec
    /// delay, resampling, or a browser decoder's rounded duration.
    pub audio_durations_ms: BTreeMap<String, u32>,
}

/// Decoded/staged mission. Only payload participates in the v8 wire contract;
/// raw bundle ownership is established at the consuming preparation boundary.
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct ShippingMission {
    #[serde(flatten)]
    pub payload: ShippingMissionPayload,
    #[serde(skip)]
    raw_bundle: OnceLock<Arc<robin_util::asset_fs::Bundle>>,
    #[serde(skip)]
    sprite_streaming: crate::late_sprites::SpriteStreaming,
}

impl std::ops::Deref for ShippingMission {
    type Target = ShippingMissionPayload;
    fn deref(&self) -> &Self::Target {
        &self.payload
    }
}
impl std::ops::DerefMut for ShippingMission {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.payload
    }
}

/// Proof that compressed chunks and the raw bundle have crossed preparation.
/// This wrapper cannot be constructed by a caller with an unprepared payload.
#[derive(Debug, Serialize, Deserialize)]
struct PreparedShippingMission {
    mission: ShippingMission,
}

#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct RhsData {
    pub signature: u32,
    pub profiles: Vec<(String, SpriteInfo)>,
}

/// Shipping-ready sprite bank. Unlike the runtime [`crate::frame_holder::FrameHolder`],
/// this carries every sprite's packed pixel data inline (the runtime
/// version marks `packed_data` `#[serde(skip)]` so savegames stay small).
#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct ShippingSpriteBank {
    pub signature: u32,
    pub dictionaries: Vec<FrameDictionary>,
    /// Total number of slots in the original bank. The runtime expands the
    /// sparse entries below into this many slots once, after all mission
    /// chunks have been combined.
    pub sprite_count: u32,
    /// Sorted `(bank id, sprite)` entries. Mission RHS chunks normally use a
    /// tiny fraction of the global bank, so storing a dense `Vec<Option<_>>`
    /// here used hundreds of MiB of transient wasm heap while decoding.
    pub sprites: Vec<(u32, ShippingSprite)>,
    /// Schema v9: `sprite_codec` context-model blobs, one per converted RHS
    /// chunk. Sprites listed by a chunk carry empty `packed_data`; their VQ
    /// index grids are decoded out of the blob by
    /// [`ShippingSpriteBank::materialize_vq_chunks`] at mission install time.
    pub vq_chunks: Vec<SpriteVqChunk>,
    /// Schema v13 (web recipe only): lossy-JXL payloads for RLE patch /
    /// ambient-animation sprites. Like VQ chunk sprites, the listed rows
    /// carry empty `packed_data` until
    /// [`ShippingSpriteBank::materialize_rle_jxl_chunks`] rebuilds their
    /// exact-format RLE words at mission install time.
    pub rle_jxl_chunks: Vec<SpriteRleJxlChunk>,
}

/// One RHS chunk's VQ sprite grids, coded with [`crate::sprite_codec`].
///
/// The grids of `sprite_ids` (a strictly ascending bank-id order established
/// at conversion) are concatenated into a single adaptive-model `blob`;
/// `base_ids[i]` names the family-base sprite whose materialized grid is the
/// cross-variant context for `sprite_ids[i]` (`None` = coded standalone).
/// Schema v10 adds a star-2 topology: `base2_ids[i]` optionally names a
/// SECOND already-decoded sibling whose grid joins the coding context (a
/// sprite may only carry a `base2` when it also carries a `base`). Base
/// sprites always live in different chunks of the same mission closure (the
/// family hub chunks), which the conversion lists as explicit mission
/// dependencies.
#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct SpriteVqChunk {
    /// Relative RHS path this chunk was built from (diagnostics only).
    pub rhs: String,
    /// Relative RHS path of the family base this chunk is coded against.
    pub base_rhs: Option<String>,
    /// Relative RHS path of the second family hub providing `base2_ids`
    /// contexts. Empty when this chunk uses no second predecessor.
    pub base2_rhs: String,
    /// Codec alphabet: the largest `num_entries()` of the dictionaries
    /// referenced by the chunk's VQ sprites.
    pub alphabet: u16,
    /// Bank ids of the coded sprites, in blob (encode) order.
    pub sprite_ids: Vec<u32>,
    /// Per sprite: bank id of the base sprite providing cross-variant
    /// context, or `None` for standalone coding. Same length as `sprite_ids`.
    pub base_ids: Vec<Option<u32>>,
    /// Per sprite: bank id of the second-predecessor sprite, aligned with
    /// `sprite_ids`. Must be empty (or all `None`) when `base2_rhs` is empty;
    /// a `Some` entry requires the matching `base_ids` entry to be `Some`.
    pub base2_ids: Vec<Option<u32>>,
    /// When set, standalone sprites in this blob use within-chunk
    /// self-references (temporal predecessor / adjacent camera direction),
    /// derived at decode time from this chunk's shipped RHS script metadata
    /// via [`derive_chunk_self_refs`] — the derivation is part of the
    /// bitstream contract and ships no bytes of its own.
    pub self_refs: bool,
    /// `sprite_codec::encode_grids_shipping` output for all grids of this
    /// chunk.
    pub blob: Vec<u8>,
}

/// One RHS chunk's lossy-JXL RLE sprite payload (schema v13, WEB recipe
/// only — native shipping keeps exact RLE words because parity traces
/// screenshot composited RGB565 framebuffers).
///
/// Each listed sprite is a `width x height` region of one RGBA JXL image
/// in `jxl_blobs` (a per-animation-group atlas or a single-sprite image;
/// the converter decides per sprite and keeps exact RLE when that is
/// smaller). The color channels carry the opaque RGB lossily; the ALPHA
/// channel carries the per-pixel class losslessly (see [`crate::rle_jxl`]),
/// which is what reconstructs run extents, shadow-key literals, and
/// transparent-key literals EXACTLY — there is no sidecar structure data.
/// Only opaque RGB values are lossy (requantized to RGB565 at
/// materialization).
///
/// Unlike VQ chunks there are no cross-chunk dependencies: every sprite id
/// listed here appears in exactly one chunk of the whole tree (sprites
/// referenced by several RHS chunks keep exact RLE words instead, because
/// two independent lossy encodes of one bank slot would conflict at merge).
#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct SpriteRleJxlChunk {
    /// Relative RHS path this chunk was built from (diagnostics only).
    pub rhs: String,
    /// Encoded RGBA JXL images: lossy color, lossless class-marker alpha.
    pub jxl_blobs: Vec<Vec<u8>>,
    /// Bank ids of the coded sprites, strictly ascending.
    pub sprite_ids: Vec<u32>,
    /// Per sprite, aligned with `sprite_ids`: which blob and the top-left
    /// pixel of its region inside that blob.
    pub placements: Vec<RleJxlPlacement>,
}

/// Placement of one sprite inside its chunk's JXL blob list.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct RleJxlPlacement {
    pub blob: u32,
    pub x: u16,
    pub y: u16,
}

/// Derive the deterministic within-chunk auxiliary references for a VQ
/// chunk from its RHS script metadata. Converter and materialization run
/// this same rule, so the reference map itself never ships.
///
/// Rule (validated in `sprite_compression_probe --code-aux`): for each
/// sprite, the first offset-aligned temporal predecessor in any script row
/// (`ref < cur` keeps blob order causal; the x offset delta must be a
/// multiple of the 4-pixel tile width), else the first aligned
/// adjacent-camera-direction neighbor within the same action group.
pub fn derive_chunk_self_refs(
    profiles: &[(String, SpriteInfo)],
    sprite_ids: &[u32],
) -> Vec<Option<crate::sprite_codec::SelfRef>> {
    let batch_index: std::collections::HashMap<u32, u32> = sprite_ids
        .iter()
        .enumerate()
        .map(|(index, &id)| (id, index as u32))
        .collect();
    let mut refs: Vec<Option<crate::sprite_codec::SelfRef>> = vec![None; sprite_ids.len()];
    let try_pair = |cur: u32,
                    r: u32,
                    oc: (i32, i32),
                    or_: (i32, i32),
                    refs: &mut Vec<Option<crate::sprite_codec::SelfRef>>| {
        if r >= cur {
            return;
        }
        let (Some(&cur_pos), Some(&ref_pos)) = (batch_index.get(&cur), batch_index.get(&r)) else {
            return;
        };
        if refs[cur_pos as usize].is_some() {
            return;
        }
        let (dx, dy) = (oc.0 - or_.0, oc.1 - or_.1);
        if dx % 4 != 0 {
            return;
        }
        refs[cur_pos as usize] = Some(crate::sprite_codec::SelfRef {
            grid: ref_pos,
            dtx: dx / 4,
            dy,
        });
    };
    let off = |s: &robin_engine::sprite_script::SpriteScript, k: usize| {
        s.offsets
            .get(k)
            .map(|o| (o.x.round() as i32, o.y.round() as i32))
            .unwrap_or((0, 0))
    };
    // Pass 1: temporal predecessors.
    for (_name, info) in profiles {
        for s in info.scripts.iter() {
            for k in 1..s.frame_ids.len() {
                try_pair(
                    s.frame_ids[k],
                    s.frame_ids[k - 1],
                    off(s, k),
                    off(s, k - 1),
                    &mut refs,
                );
            }
        }
    }
    // Pass 2: adjacent camera directions for whatever is still uncovered.
    for (_name, info) in profiles {
        let mut by_action: BTreeMap<u16, Vec<&robin_engine::sprite_script::SpriteScript>> =
            BTreeMap::new();
        for s in info.scripts.iter() {
            by_action.entry(s.action_id).or_default().push(s);
        }
        for rows in by_action.values() {
            for d in 1..rows.len() {
                let (ra, rb) = (rows[d - 1], rows[d]);
                for k in 0..ra.frame_ids.len().min(rb.frame_ids.len()) {
                    try_pair(
                        rb.frame_ids[k],
                        ra.frame_ids[k],
                        off(rb, k),
                        off(ra, k),
                        &mut refs,
                    );
                    try_pair(
                        ra.frame_ids[k],
                        rb.frame_ids[k],
                        off(ra, k),
                        off(rb, k),
                        &mut refs,
                    );
                }
            }
        }
    }
    refs
}

#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct ShippingSprite {
    pub width: u16,
    pub height: u16,
    pub dictionary_index: u16,
    /// Packed pixel data (RLE or dictionary-indexed). Empty for VQ sprites
    /// whose grid lives in a [`SpriteVqChunk`] blob until materialization,
    /// and for web-lossy RLE sprites, which materialize into `raster`
    /// instead and never have packed words at all.
    pub packed_data: Arc<Vec<u16>>,
    /// Runtime-only: decoded RGB565 atlas window produced by
    /// [`ShippingSpriteBank::materialize_rle_jxl_chunks`]. Never
    /// serialized — the shipped form is the JXL blob it came from.
    #[serde(skip)]
    #[bitcode(skip)]
    pub raster: Option<crate::frame_holder::SpriteRaster>,
}

// ---------------------------------------------------------------------------
//  I/O
// ---------------------------------------------------------------------------

/// Convenience: look for `<data_dir>/datadir.bin`. Returns `Ok(None)` if
/// the file isn't present (legacy datadir), `Ok(Some(_))` on success.
pub fn try_load(data_dir: &Path) -> Result<Option<ShippingDatadir>> {
    let path = data_dir.join("datadir.bin");
    match robin_util::asset_fs::read_shared(&path) {
        Ok(compressed) => {
            let mut datadir = ShippingDatadir::from_compressed_bytes(&compressed)
                .with_context(|| format!("decode {}", path.display()))?;
            datadir.runtime.source_dir = Some(data_dir.to_path_buf());
            Ok(Some(datadir))
        }
        Err(robin_util::asset_fs::AssetError::NotFound(_)) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

/// Instance form of [`try_load`]. Existence and open failures stay distinct:
/// only a genuine not-found result selects the legacy loose-file path.
pub fn try_load_from(
    vfs: &robin_util::asset_fs::AssetVfs,
    data_dir: &Path,
) -> Result<Option<ShippingDatadir>> {
    let path = data_dir.join("datadir.bin");
    match vfs.read_shared(&path) {
        Ok(compressed) => {
            let mut datadir = ShippingDatadir::from_compressed_bytes(&compressed)
                .with_context(|| format!("decode {}", path.display()))?;
            datadir.runtime.source_dir = Some(data_dir.to_path_buf());
            Ok(Some(datadir))
        }
        Err(robin_util::asset_fs::AssetError::NotFound(_)) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

// ---------------------------------------------------------------------------
//  Explicit installation and legacy process-global adapter
// ---------------------------------------------------------------------------

/// A parsed shipping payload and the VFS it was mounted into.
///
/// Keeping these together prevents startup from publishing parsed data while
/// silently failing to publish its raw-file mount (or vice versa).
#[derive(Debug)]
pub struct ShippingAssets {
    datadir: Arc<ShippingDatadir>,
    vfs: Arc<robin_util::asset_fs::AssetVfs>,
}

/// Published as one unit only after the raw mount succeeds. Runtime-only;
/// serde deliberately excludes this capability from the shipping wire format.
#[derive(Debug, Serialize, Deserialize)]
struct ShippingInstallation {
    #[serde(skip)]
    vfs: Arc<robin_util::asset_fs::AssetVfs>,
    #[serde(skip)]
    boot_raw_bundle: Arc<robin_util::asset_fs::Bundle>,
}

impl ShippingAssets {
    /// Bind decoded data once. Validation failure publishes neither a mount
    /// nor an installation; retained callers may correct and retry the decode.
    /// Concurrent attempts on the same decode are serialized: exactly one
    /// succeeds, and losing VFSes are not modified. Distinct decodes may be
    /// installed independently, including into independent application VFSes.
    pub fn install(
        mut datadir: Arc<ShippingDatadir>,
        vfs: Arc<robin_util::asset_fs::AssetVfs>,
    ) -> Result<Self> {
        // Reject ordinary duplicate calls before copying a shared boot bundle.
        // The locked check below also covers two first-time concurrent callers.
        if datadir.runtime.installed.get().is_some() {
            return Err(anyhow!("shipping datadir is already bound to a VFS"));
        }
        let raw: robin_util::asset_fs::Bundle = if let Some(unique) = Arc::get_mut(&mut datadir) {
            std::mem::take(&mut unique.raw)
                .into_iter()
                .map(|(path, bytes)| (path, bytes.into()))
                .collect()
        } else {
            datadir
                .raw
                .iter()
                .map(|(path, bytes)| (path.clone(), bytes.clone().into()))
                .collect()
        };
        let raw = Arc::new(raw);
        let installation = datadir
            .runtime
            .installation_lock
            .lock()
            .expect("shipping installation lock poisoned");
        if datadir.runtime.installed.get().is_some() {
            return Err(anyhow!("shipping datadir is already bound to a VFS"));
        }
        vfs.mount_bundle_first(raw.clone())
            .context("mount shipping raw asset bundle")?;
        datadir
            .runtime
            .installed
            .set(ShippingInstallation {
                vfs: vfs.clone(),
                boot_raw_bundle: raw,
            })
            .expect("shipping installation is serialized");
        drop(installation);
        Ok(Self { datadir, vfs })
    }

    pub fn datadir(&self) -> &Arc<ShippingDatadir> {
        &self.datadir
    }

    pub fn vfs(&self) -> &Arc<robin_util::asset_fs::AssetVfs> {
        &self.vfs
    }
}

static GLOBAL: OnceLock<Arc<ShippingAssets>> = OnceLock::new();

#[cfg(test)]
#[path = "shipping_v16_contract.rs"]
mod v16_contract;

#[cfg(test)]
#[path = "shipping_v8_contract.rs"]
mod v8_contract;

/// Explicit legacy adapter: install a shipping datadir as the process-wide instance so lower-level
/// loaders can consult it for pre-parsed data. Installation and VFS mount
/// failures are returned to the startup boundary.
pub fn install_global(dd: Arc<ShippingDatadir>) -> Result<Arc<ShippingDatadir>> {
    static INSTALL_LOCK: Mutex<()> = Mutex::new(());
    let _installation = INSTALL_LOCK
        .lock()
        .expect("global shipping installation lock poisoned");
    if GLOBAL.get().is_some() {
        return Err(anyhow!("shipping datadir already installed"));
    }
    let installed = Arc::new(ShippingAssets::install(
        dd,
        robin_util::asset_fs::global().clone(),
    )?);
    GLOBAL
        .set(installed)
        .map_err(|_| anyhow!("shipping datadir concurrently installed"))?;
    Ok(global()
        .expect("shipping global was set immediately above")
        .clone())
}

/// Access the installed shipping datadir, if any.
pub fn global() -> Option<&'static Arc<ShippingDatadir>> {
    GLOBAL.get().map(|installed| installed.datadir())
}

/// Access the co-owned runtime shipping/VFS installation.
pub fn global_assets() -> Option<&'static Arc<ShippingAssets>> {
    GLOBAL.get()
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_util::asset_fs::{AssetVfs, Bundle};

    #[test]
    fn shared_manifest_loads_preserve_source_and_distinguish_missing_from_corrupt() {
        let vfs = AssetVfs::new();
        let root = Path::new("shared-manifest-fixture");
        let path = root.join("datadir.bin");
        assert!(try_load_from(&vfs, root).unwrap().is_none());
        assert!(ShippingDatadir::load_from_vfs(&vfs, &path).is_err());

        let mut fixture = ShippingDatadir::default();
        fixture.raw.insert("fixture.bin".to_owned(), vec![7, 8, 9]);
        let compressed = zstd_compress_with_window(&encode_native(&fixture), 30).unwrap();
        vfs.install_preloaded_asset(path.to_str().unwrap(), compressed)
            .unwrap();

        let required = ShippingDatadir::load_from_vfs(&vfs, &path).unwrap();
        let optional = try_load_from(&vfs, root).unwrap().unwrap();
        for loaded in [&required, &optional] {
            assert_eq!(loaded.raw["fixture.bin"], [7, 8, 9]);
            assert_eq!(
                loaded.source_file_path("part.bin").unwrap(),
                root.join("part.bin")
            );
        }

        vfs.install_preloaded_asset(path.to_str().unwrap(), b"not zstd".to_vec())
            .unwrap();
        for error in [
            ShippingDatadir::load_from_vfs(&vfs, &path).unwrap_err(),
            try_load_from(&vfs, root).unwrap_err(),
        ] {
            assert!(
                error
                    .to_string()
                    .contains(&format!("decode {}", path.display()))
            );
        }
        assert_eq!(required.raw["fixture.bin"], [7, 8, 9]);
    }

    fn install_fixture(datadir: ShippingDatadir) -> ShippingDatadir {
        let ShippingAssets { datadir, .. } =
            ShippingAssets::install(Arc::new(datadir), Arc::new(AssetVfs::new())).unwrap();
        Arc::try_unwrap(datadir).unwrap()
    }

    #[test]
    fn captured_locale_keeps_pak_and_descriptor_policy_after_selection_changes() {
        let mut datadir = ShippingDatadir::default();
        datadir
            .pak_files
            .insert("interface/title.pak".into(), vec![]);
        datadir
            .pak_files
            .insert("interface/missing.pak".into(), vec![]);
        let mut shared = LevelDescriptors::default();
        shared.custom_short_briefings.push(Some("Base text".into()));
        datadir.red_files.insert("RHLevelSB.red".into(), shared);
        let mut locale = ShippingLocale::default();
        locale.pak_files.insert(
            "interface/title.pak".into(),
            vec![EncodedPicture::jxl_rgba565_keyed(vec![1])],
        );
        datadir.locales.insert("de-DE".into(), locale);
        let datadir = install_fixture(datadir);
        datadir.set_active_locale(Some("de-DE")).unwrap();
        let captured = datadir.active_locale();
        datadir.set_active_locale(None).unwrap();

        assert_eq!(
            datadir
                .localized_pak_for_locale("Data/Interface/Title.pak", captured)
                .unwrap()
                .len(),
            1
        );
        assert!(
            datadir
                .localized_pak_for_locale("Data/Interface/Missing.pak", captured)
                .is_none()
        );
        assert!(
            datadir
                .localized_level_descriptors_for_locale("RHLevelSB.red", captured)
                .is_none()
        );
        // Subsequent independent lookups see the newly selected base assets.
        assert_eq!(
            datadir
                .localized_pak("Data/Interface/Title.pak")
                .unwrap()
                .len(),
            0
        );
        assert!(
            datadir
                .localized_level_descriptors("RHLevelSB.red")
                .is_some()
        );
    }

    #[test]
    fn valid_selected_locale_keeps_missing_assets_optional() {
        let mut datadir = ShippingDatadir::default();
        datadir
            .locales
            .insert("de-DE".into(), ShippingLocale::default());
        let datadir = install_fixture(datadir);
        datadir.set_active_locale(Some("de-DE")).unwrap();
        assert!(datadir.active_resource("Data/Text/Level.res").is_none());
        assert!(datadir.active_pak("Data/Interface/Missing.pak").is_none());
        assert!(datadir.active_level_descriptors("missing.red").is_none());
        assert!(datadir.active_profiles().is_none());
    }

    #[test]
    #[should_panic(expected = "invalid active shipping locale")]
    fn invalid_active_locale_cannot_masquerade_as_missing_resource() {
        let datadir = install_fixture(ShippingDatadir::default());
        // Generic VFS selection can be configured independently of this
        // manifest. Shipping lookup must detect that broken invariant.
        datadir
            .asset_vfs()
            .select_locale(Some("@bad@".into()), None)
            .unwrap();
        datadir.active_resource("Data/Text/Level.res");
    }

    #[test]
    #[should_panic(expected = "is not installed")]
    fn uninstalled_active_locale_cannot_fall_back_to_shared_descriptors() {
        let datadir = install_fixture(ShippingDatadir::default());
        datadir
            .asset_vfs()
            .select_locale(Some("de-DE".into()), None)
            .unwrap();
        datadir.localized_level_descriptors("RHLevelSB.red");
    }

    #[test]
    #[ignore = "requires ROBIN_BROWSER_CONTENT_FIXTURE pointing to the retained Demo shipping blob"]
    fn retained_demo_descriptor_resolves_actual_localized_popup_and_briefing() {
        let path = std::env::var("ROBIN_BROWSER_CONTENT_FIXTURE")
            .expect("set ROBIN_BROWSER_CONTENT_FIXTURE to the retained Demo shipping blob");
        let datadir = ShippingDatadir::load_from_file(Path::new(&path)).unwrap();
        let datadir = install_fixture(datadir);
        datadir.set_active_locale(Some("1033")).unwrap();
        assert!(datadir.active_level_descriptors("RHLevelSB.red").is_none());
        let descriptor = datadir
            .localized_level_descriptors("RHLevelSB.red")
            .unwrap();
        let mut text = datadir
            .active_resource("Data/Text/Level.res")
            .unwrap()
            .clone();
        assert!(
            !text
                .get_string(descriptor.popup_text.text_table_id, 0)
                .unwrap()
                .is_empty()
        );
        assert!(
            !text
                .get_string(descriptor.short_briefing.text_table_id, 0)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn shared_descriptor_indices_work_with_demo_locale_overlay() {
        let mut datadir = ShippingDatadir::default();
        let mut descriptor = LevelDescriptors::default();
        descriptor.short_briefing.text_table_id = 123;
        descriptor.popup_text.text_table_id = 456;
        datadir.red_files.insert("RHLevelSB.red".into(), descriptor);
        datadir
            .locales
            .insert("en-US".into(), ShippingLocale::default());
        datadir
            .locales
            .insert("de-DE".into(), ShippingLocale::default());
        let mut datadir = install_fixture(datadir);
        datadir.set_active_locale(Some("1033")).unwrap();
        let resolved = datadir
            .localized_level_descriptors("Data\\Text\\RHLevelSB.red")
            .unwrap();
        assert_eq!(resolved.short_briefing.text_table_id, 123);
        assert_eq!(resolved.popup_text.text_table_id, 456);
        // Shared indices must not make an absent translated string table appear.
        datadir
            .res_files
            .insert("text/level.res".into(), ResourceManager::new());
        datadir.set_active_locale(Some("de-DE")).unwrap();
        assert!(
            datadir
                .localized_level_descriptors("rhlevelsb.red")
                .is_some()
        );
        assert!(datadir.active_resource("Data/Text/Level.res").is_none());
        assert!(datadir.localized_level_descriptors("missing.red").is_none());
    }

    #[test]
    fn localized_descriptor_override_precedes_shared_metadata() {
        let mut datadir = ShippingDatadir::default();
        datadir
            .red_files
            .insert("RHLevelSB.red".into(), LevelDescriptors::default());
        let mut locale = ShippingLocale::default();
        let mut translated = LevelDescriptors::default();
        translated.short_briefing.text_table_id = 789;
        translated
            .custom_short_briefings
            .push(Some("Localized objective".into()));
        locale.red_files.insert("rhlevelsb.red".into(), translated);
        datadir.locales.insert("de-DE".into(), locale);
        let datadir = install_fixture(datadir);
        datadir.set_active_locale(Some("de-DE")).unwrap();
        let resolved = datadir
            .localized_level_descriptors("RHLevelSB.red")
            .unwrap();
        assert_eq!(resolved.short_briefing.text_table_id, 789);
        assert_eq!(
            resolved.custom_short_briefings[0].as_deref(),
            Some("Localized objective")
        );
    }

    #[test]
    #[should_panic(expected = "ambiguous shared level descriptor")]
    fn shared_descriptor_rejects_ambiguous_case_aliases() {
        let mut datadir = ShippingDatadir::default();
        datadir
            .red_files
            .insert("RHLevelSB.red".into(), LevelDescriptors::default());
        datadir
            .red_files
            .insert("rhlevelsb.red".into(), LevelDescriptors::default());
        let datadir = install_fixture(datadir);
        datadir.localized_level_descriptors("RHLevelSB.red");
    }

    #[test]
    fn shared_descriptor_authored_strings_do_not_cross_locale_boundary() {
        let mut datadir = ShippingDatadir::default();
        let mut descriptor = LevelDescriptors::default();
        descriptor
            .custom_popup_texts
            .push(Some("Not translated".into()));
        datadir.red_files.insert("rhlevelsb.red".into(), descriptor);
        datadir
            .locales
            .insert("de-DE".into(), ShippingLocale::default());
        let datadir = install_fixture(datadir);
        datadir.set_active_locale(Some("de-DE")).unwrap();
        assert!(
            datadir
                .localized_level_descriptors("RHLevelSB.red")
                .is_none()
        );
        datadir.set_active_locale(None).unwrap();
        assert!(
            datadir
                .localized_level_descriptors("RHLevelSB.red")
                .is_some()
        );
    }

    #[test]
    fn expanded_decode_limit_includes_exact_boundary_and_truncation() {
        let compressed = zstd_max_compress(&vec![7; 4096]).unwrap();
        assert_eq!(
            decompress_shipping_with_limit(&compressed, 4096).unwrap(),
            vec![7; 4096]
        );
        assert!(
            decompress_shipping_with_limit(&compressed, 4095)
                .unwrap_err()
                .to_string()
                .contains("exceeds")
        );
        assert!(decompress_shipping_with_limit(&compressed[..compressed.len() / 2], 4096).is_err());
    }

    #[test]
    fn duplicate_preload_error_preserves_authenticated_bytes() {
        let datadir = ShippingDatadir::default();
        datadir
            .cache_preloaded_file("payload".into(), vec![1])
            .unwrap();
        assert!(
            datadir
                .cache_preloaded_file("payload".into(), vec![2])
                .is_err()
        );
        assert_eq!(datadir.preloaded_file("payload").unwrap().as_slice(), &[1]);
    }

    #[test]
    fn mission_replacement_retires_only_its_own_stream_after_success() {
        fn payload(name: &str) -> ShippingMission {
            let level = LoadedLevel::hackable_from_json(
                br#"{
                "map_filename":"test", "spawn":[5,5],
                "walkable_polygon":[[0,0],[100,0],[100,100],[0,100]]
            }"#,
            )
            .unwrap();
            let mut mission = ShippingMission::default();
            mission.levels.insert(name.into(), level);
            mission
        }
        let first = ShippingAssets::install(
            Arc::new(ShippingDatadir::default()),
            Arc::new(AssetVfs::new()),
        )
        .unwrap();
        let second = ShippingAssets::install(
            Arc::new(ShippingDatadir::default()),
            Arc::new(AssetVfs::new()),
        )
        .unwrap();
        first
            .datadir()
            .install_mission("old", payload("old"))
            .unwrap();
        second
            .datadir()
            .install_mission("old", payload("old"))
            .unwrap();
        let retained = first.datadir().loaded_mission("old").unwrap();
        let old = retained.sprite_streaming().publisher(3, 300);
        let independent = second
            .datadir()
            .loaded_mission("old")
            .unwrap()
            .sprite_streaming()
            .publisher(3, 300);
        let mut invalid = payload("bad");
        invalid.raw.insert("../escape".into(), vec![1]);
        assert!(first.datadir().install_mission("bad", invalid).is_err());
        assert!(!old.is_retired());
        assert!(old.publish_chunk(100, &[(7, Arc::new(vec![1]))]));
        first.datadir().activate_mission("old").unwrap();
        assert!(
            !old.is_retired(),
            "same mission restart preserves publication"
        );
        first
            .datadir()
            .install_mission("new", payload("new"))
            .unwrap();
        assert!(old.is_retired());
        assert!(!old.publish_chunk(100, &[(8, Arc::new(vec![2]))]));
        assert!(!independent.is_retired());
        assert!(independent.publish_chunk(100, &[(7, Arc::new(vec![3]))]));
    }

    #[test]
    fn mission_publication_rejects_invalid_raw_before_replacing_selection() {
        let datadir = install_fixture(ShippingDatadir::default());
        let good = ShippingMission::default();
        good.raw_bundle
            .set(Arc::new(BTreeMap::from([("old".into(), vec![1].into())])))
            .unwrap();
        datadir
            .runtime
            .loaded_missions
            .write()
            .unwrap()
            .insert("old".into(), Arc::new(good));
        datadir.activate_mission("old").unwrap();
        let snapshot = datadir.selection_snapshot();
        let bad = ShippingMission::default();
        bad.raw_bundle
            .set(Arc::new(BTreeMap::from([(
                "../escape".into(),
                vec![2].into(),
            )])))
            .unwrap();
        assert!(datadir.publish_mission("new", &bad).is_err());
        assert_eq!(datadir.selection_snapshot().generation, snapshot.generation);
        assert_eq!(datadir.active_mission_name().as_deref(), Some("old"));
        assert!(datadir.active_mission_payload().is_some());
        assert_eq!(datadir.asset_vfs().read("old").unwrap(), [1]);
    }

    #[test]
    fn captured_mission_resources_survive_later_activation() {
        use robin_engine::coordinates::{SpriteAnchor, SpriteSize};
        use robin_engine::sprite_script::{FrameKind, SpriteScriptor};

        let datadir = install_fixture(ShippingDatadir::default());
        for (name, width) in [("first", 12.0), ("second", 24.0)] {
            let mut mission = ShippingMission::default();
            mission.raw_bundle.set(Arc::new(BTreeMap::new())).unwrap();
            mission.payload.rhs_files.insert(
                "Characters/Robin.rhs".into(),
                RhsData {
                    signature: 77,
                    profiles: vec![(
                        "Robin".into(),
                        SpriteInfo {
                            scripts: Arc::new(vec![]),
                            conversion: Arc::new(vec![]),
                            size: SpriteSize::new(width, 8.0),
                            center: SpriteAnchor::ZERO,
                        },
                    )],
                },
            );
            datadir
                .runtime
                .loaded_missions
                .write()
                .unwrap()
                .insert(name.into(), Arc::new(mission));
        }
        datadir.activate_mission("first").unwrap();
        let first = datadir.mission_resource_environment("first").unwrap();
        datadir.activate_mission("second").unwrap();
        let second = datadir.mission_resource_environment("second").unwrap();
        assert!(datadir.mission_resource_environment("first").is_err());
        for (resources, width) in [(first, 12.0), (second, 24.0)] {
            let mut scriptor = SpriteScriptor::with_resources(resources);
            let info = scriptor
                .load(
                    "Data/Characters/Robin.rhs",
                    "Robin",
                    "Robin",
                    FrameKind::Character,
                    |_| Ok(()),
                )
                .unwrap();
            assert_eq!(info.size, SpriteSize::new(width, 8.0));
        }
    }

    #[test]
    fn mission_snapshots_pin_matching_parsed_and_raw_payloads() {
        let datadir = Arc::new(install_fixture(ShippingDatadir::default()));
        for name in ["first", "second"] {
            let payload = ShippingMission::default();
            payload
                .raw_bundle
                .set(Arc::new(BTreeMap::from([(
                    "marker".into(),
                    name.as_bytes().into(),
                )])))
                .unwrap();
            datadir
                .runtime
                .loaded_missions
                .write()
                .unwrap()
                .insert(name.into(), Arc::new(payload));
        }
        datadir.activate_mission("first").unwrap();
        let writer = datadir.clone();
        let handle = std::thread::spawn(move || {
            for _ in 0..50 {
                writer.activate_mission("second").unwrap();
                writer.activate_mission("first").unwrap();
            }
        });
        let competing_writer = datadir.clone();
        let competing = std::thread::spawn(move || {
            for _ in 0..50 {
                competing_writer.activate_mission("first").unwrap();
                competing_writer.activate_mission("second").unwrap();
            }
        });
        for _ in 0..500 {
            let (selection, payload) = datadir.mission_selection_snapshot();
            let payload = payload.unwrap();
            assert!(Arc::ptr_eq(
                selection.active_bundle.as_ref().unwrap(),
                payload.raw_bundle.get().unwrap()
            ));
            assert_eq!(
                selection.active_bundle.as_ref().unwrap()["marker"].as_ref(),
                selection.mission.as_ref().unwrap().as_bytes()
            );
        }
        handle.join().unwrap();
        competing.join().unwrap();
    }

    #[test]
    fn installed_locale_selection_is_isolated_and_failure_preserves_generation() {
        fn install() -> ShippingAssets {
            let mut datadir = ShippingDatadir::default();
            for (name, value) in [("en-US", 1), ("de-DE", 2)] {
                let mut locale = ShippingLocale::default();
                locale.raw.insert("text/fixture".into(), vec![value]);
                datadir.locales.insert(name.into(), locale);
            }
            ShippingAssets::install(Arc::new(datadir), Arc::new(AssetVfs::new())).unwrap()
        }
        let first = install();
        let second = install();
        first.datadir().set_active_locale(Some("en-US")).unwrap();
        second.datadir().set_active_locale(Some("de-DE")).unwrap();
        assert_eq!(first.vfs().read("text/fixture").unwrap(), [1]);
        assert_eq!(second.vfs().read("text/fixture").unwrap(), [2]);
        assert_eq!(
            first.datadir().active_locale_name().as_deref(),
            Some("en-US")
        );
        let old = first.vfs().selection_snapshot();
        assert!(first.datadir().set_active_locale(Some("fr-FR")).is_err());
        assert_eq!(first.vfs().selection_snapshot().generation, old.generation);
        assert_eq!(first.vfs().read("text/fixture").unwrap(), [1]);
    }

    #[test]
    fn native_shipping_format_roundtrips_and_rejects_legacy_payloads() {
        let mut datadir = ShippingDatadir::default();
        datadir.raw.insert("test.bin".into(), vec![1, 2, 3]);
        datadir
            .audio_durations_ms
            .insert("musics/menu.opus".into(), 9_876);
        datadir.audio_assets.insert(
            "sounds/arrow.opus".into(),
            ShippingAudioAsset {
                file: "audio/assets/0123.opus".into(),
                encoded_size: 456,
                duration_ms: 789,
                bundle_offset: None,
            },
        );
        datadir.missions.insert(
            "MissionOne".into(),
            ShippingMissionRef {
                forest_level: true,
                files: vec!["missions/mission-one.rhmission.zst".into()],
            },
        );
        datadir
            .character_rhs_files
            .insert(7, vec!["rhs/character-seven.rhmission.zst".into()]);
        datadir
            .character_audio_files
            .insert(7, vec!["audio/character-seven.rhmission.zst".into()]);
        datadir.character_exclamation_ids.insert(7, 0x5043_5248);
        datadir
            .mission_exclamation_ids
            .insert("MissionOne".into(), vec![0x534F_4C44]);
        datadir.saved_world_rhs_files = vec!["rhs/saved-objects.rhmission.zst".into()];
        let mut german = ShippingLocale {
            source_lcid: Some("1031".into()),
            ..ShippingLocale::default()
        };
        german.raw.insert("text/level.res".into(), vec![7, 8, 9]);
        datadir.locales.insert("de-DE".into(), german);

        let encoded = encode_native(&datadir);
        assert_eq!(&encoded[..8], b"RHDDNA16");
        assert_eq!(&encoded[..8], &SHIPPING_DATADIR_MAGIC);
        let decoded = decode_native(&encoded).expect("decode native shipping datadir");
        assert_eq!(decoded.raw.get("test.bin"), Some(&vec![1, 2, 3]));
        assert_eq!(
            decoded.audio_durations_ms.get("musics/menu.opus"),
            Some(&9_876)
        );
        assert_eq!(
            decoded.audio_assets.get("sounds/arrow.opus"),
            Some(&ShippingAudioAsset {
                file: "audio/assets/0123.opus".into(),
                encoded_size: 456,
                duration_ms: 789,
                bundle_offset: None,
            })
        );
        assert_eq!(
            decoded.mission_ref("MissionOne").unwrap().files,
            vec!["missions/mission-one.rhmission.zst"]
        );
        assert!(decoded.mission_ref("MissionOne").unwrap().forest_level);
        assert_eq!(
            decoded.character_rhs_files.get(&7).unwrap(),
            &["rhs/character-seven.rhmission.zst"]
        );
        assert_eq!(
            decoded.character_audio_files.get(&7).unwrap(),
            &["audio/character-seven.rhmission.zst"]
        );
        assert_eq!(
            decoded.character_exclamation_ids.get(&7),
            Some(&0x5043_5248)
        );
        assert_eq!(
            decoded.mission_exclamation_ids.get("MissionOne").unwrap(),
            &[0x534F_4C44]
        );
        assert_eq!(
            decoded.saved_world_rhs_files,
            ["rhs/saved-objects.rhmission.zst"]
        );
        assert_eq!(
            decoded.locale_raw("1031", "Text/Level.res").unwrap(),
            Some([7, 8, 9].as_slice())
        );

        let mut previous_schema = encoded.clone();
        previous_schema[..8].copy_from_slice(b"RHDDNA13");
        let error = decode_native(&previous_schema).unwrap_err();
        assert!(error.to_string().contains("regenerate datadir.bin"));

        let legacy_unversioned = bitcode::encode(datadir.payload());
        let error = decode_native(&legacy_unversioned).unwrap_err();
        assert!(error.to_string().contains("regenerate datadir.bin"));
    }

    #[test]
    fn canonical_locale_ids_accept_legacy_aliases_without_inventing_identity() {
        assert_eq!(canonical_locale_id("1031").unwrap(), "de-DE");
        assert_eq!(canonical_locale_id("DE_de").unwrap(), "de-DE");
        assert_eq!(canonical_locale_id("zh-hant-tw").unwrap(), "zh-Hant-TW");
        assert_eq!(canonical_locale_id("2047").unwrap(), "und");
        assert_eq!(canonical_locale_id("neutral").unwrap(), "und");
        assert!(canonical_locale_id("../de-DE").is_err());
    }

    #[test]
    fn locale_lookup_and_bundle_use_canonical_keys() {
        let mut locale = ShippingLocale {
            source_lcid: Some("1031".into()),
            ..ShippingLocale::default()
        };
        locale.aliases.insert("1031".into());
        locale.raw.insert("text/dialogue.wav".into(), vec![4, 2]);
        let mut datadir = ShippingDatadir::default();
        datadir.locales.insert("de-DE".into(), locale);

        assert_eq!(
            datadir
                .locale_raw("1031", "Data\\Text\\Dialogue.wav")
                .unwrap(),
            Some([4, 2].as_slice())
        );
        assert!(datadir.locale("fr-FR").unwrap().is_none());
        let first = datadir.locale_bundle("de_de").unwrap().unwrap();
        let second = datadir.locale_bundle("1031").unwrap().unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(
            first.get("text/dialogue.wav").map(|bytes| bytes.as_ref()),
            Some([4, 2].as_slice())
        );
    }

    #[test]
    fn locale_bundle_only_uses_english_for_optional_recorded_media() {
        let mut english = ShippingLocale::default();
        english.raw.insert("text/level.res".into(), vec![1]);
        english.raw.insert("interface/start.sxt".into(), vec![2]);
        english
            .raw
            .insert("sounds/exclamations/robin.wav".into(), vec![3]);
        english.raw.insert("cinematics/intro.ogg".into(), vec![4]);

        let mut german = ShippingLocale::default();
        german.raw.insert("text/level.res".into(), vec![5]);

        let mut datadir = ShippingDatadir::default();
        datadir.locales.insert("en-US".into(), english);
        datadir.locales.insert("de-DE".into(), german);

        let bundle = datadir.locale_bundle("de-DE").unwrap().unwrap();
        assert_eq!(
            bundle.get("text/level.res").map(|bytes| bytes.as_ref()),
            Some([5].as_slice())
        );
        assert!(!bundle.contains_key("interface/start.sxt"));
        assert_eq!(
            bundle
                .get("sounds/exclamations/robin.wav")
                .map(|bytes| bytes.as_ref()),
            Some([3].as_slice())
        );
        assert_eq!(
            bundle
                .get("cinematics/intro.ogg")
                .map(|bytes| bytes.as_ref()),
            Some([4].as_slice())
        );
    }

    #[test]
    fn mission_payload_roundtrips_independently() {
        let mut mission = ShippingMission::default();
        mission
            .raw
            .insert("levels/day/map.min".into(), vec![9, 8, 7]);
        mission
            .audio_durations_ms
            .insert("sounds/arrow.opus".into(), 1_234);
        let encoded = encode_mission_native(&mission);
        assert_eq!(&encoded[..8], b"RHMISN08");
        let compressed = zstd_compress_with_window(&encoded, 30).unwrap();
        let decoded = decode_mission_compressed(&compressed).unwrap();
        assert_eq!(decoded.raw.get("levels/day/map.min"), Some(&vec![9, 8, 7]));
        assert_eq!(
            decoded.audio_durations_ms.get("sounds/arrow.opus"),
            Some(&1_234)
        );
    }

    #[test]
    fn mission_parts_merge_disjoint_sprite_slots() {
        let sprite = |value| ShippingSprite {
            width: 1,
            height: 1,
            dictionary_index: 0,
            packed_data: Arc::new(vec![value]),
            raster: None,
        };
        let bank = |sprites| ShippingSpriteBank {
            signature: 42,
            dictionaries: Vec::new(),
            sprite_count: 2,
            sprites,
            vq_chunks: Vec::new(),
            rle_jxl_chunks: Vec::new(),
        };
        let mut merged = ShippingMission::from_payload(ShippingMissionPayload {
            sprite_bank: Some(bank(Vec::new())),
            ..ShippingMissionPayload::default()
        });
        merged
            .merge_from(ShippingMission::from_payload(ShippingMissionPayload {
                sprite_bank: Some(bank(vec![(0, sprite(10))])),
                ..ShippingMissionPayload::default()
            }))
            .unwrap();
        merged
            .merge_from(ShippingMission::from_payload(ShippingMissionPayload {
                sprite_bank: Some(bank(vec![(1, sprite(20))])),
                ..ShippingMissionPayload::default()
            }))
            .unwrap();

        let sprites = &merged.payload.sprite_bank.unwrap().sprites;
        assert_eq!(sprites[0].1.packed_data.as_slice(), &[10]);
        assert_eq!(sprites[1].1.packed_data.as_slice(), &[20]);
    }

    /// Base VQ grid (sprite 0), variant VQ grid (sprite 1), second-variant VQ
    /// grid (sprite 3, star-2 coded against sprites 0 AND 1): 8x3 pixels =
    /// 2x3 tiles.
    const VQ_DIMS: (u16, u16) = (8, 3);
    const BASE_GRID: [u16; 6] = [5, 6, 7, 5, 6, 7];
    const VARIANT_GRID: [u16; 6] = [5, 6, 7, 5, 9, 7];
    const SECOND_VARIANT_GRID: [u16; 6] = [5, 6, 7, 5, 9, 8];
    const RLE_WORDS: [u16; 3] = [1, 2, 3];
    const VQ_ALPHABET: u16 = 16;

    fn vq_test_bank(
        sprites: Vec<(u32, ShippingSprite)>,
        vq_chunks: Vec<SpriteVqChunk>,
    ) -> ShippingSpriteBank {
        ShippingSpriteBank {
            signature: 77,
            dictionaries: Vec::new(),
            sprite_count: 4,
            sprites,
            vq_chunks,
            rle_jxl_chunks: Vec::new(),
        }
    }

    fn vq_sprite(packed: Vec<u16>) -> ShippingSprite {
        ShippingSprite {
            width: VQ_DIMS.0,
            height: VQ_DIMS.1,
            dictionary_index: 0,
            packed_data: Arc::new(packed),
            raster: None,
        }
    }

    #[test]
    fn rle_priority_uses_total_bytes_and_preserves_ties() {
        let make = |id, sizes: &[usize]| SpriteRleJxlChunk {
            rhs: "same.rhs".into(),
            sprite_ids: vec![id],
            placements: Vec::new(),
            jxl_blobs: sizes.iter().map(|&size| vec![0; size]).collect(),
        };
        let mut chunks = vec![
            make(1, &[2]),
            make(2, &[3, 4]),
            make(3, &[7]),
            make(4, &[5]),
        ];
        order_rle_chunks_by_size(&mut chunks);
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.sprite_ids[0])
                .collect::<Vec<_>>(),
            [2, 3, 4, 1]
        );
    }

    fn priority_chunk(id: u32, bases: &[u32], bytes: usize) -> SpriteVqChunk {
        SpriteVqChunk {
            rhs: "same.rhs".into(),
            base_rhs: None,
            base2_rhs: String::new(),
            alphabet: 1,
            sprite_ids: vec![id],
            base_ids: bases.iter().copied().map(Some).collect(),
            base2_ids: Vec::new(),
            self_refs: false,
            blob: vec![0; bytes],
        }
    }

    #[test]
    fn downstream_priority_distinguishes_groups_and_uses_longest_path() {
        let mut second = priority_chunk(2, &[0, 0], 30);
        second.base2_ids = vec![Some(1)];
        let chunks = vec![
            priority_chunk(0, &[], 2),
            priority_chunk(1, &[], 3),
            second,
            priority_chunk(3, &[2], 40),
            priority_chunk(4, &[0], 20),
            priority_chunk(5, &[999], 50),
        ];
        assert_eq!(vq_downstream_costs(&chunks), [72, 73, 70, 40, 20, 50]);
        let reversed: Vec<_> = chunks.into_iter().rev().collect();
        assert_eq!(vq_downstream_costs(&reversed), [50, 20, 40, 70, 73, 72]);
    }

    #[test]
    fn downstream_priority_allows_duplicate_providers_and_leaves_validation_to_bank() {
        let chunks = vec![
            priority_chunk(0, &[1], 2),
            priority_chunk(1, &[0], 3),
            priority_chunk(0, &[], 4),
            priority_chunk(2, &[1], 5),
        ];
        let costs = vq_downstream_costs(&chunks);
        assert_eq!(costs.len(), chunks.len());
        assert_eq!(costs[1], 8);
        assert_eq!(costs[3], 5);
    }

    /// Chunk mission for the family base: sprite 0 coded standalone.
    fn base_chunk_mission() -> ShippingMission {
        use crate::sprite_codec::{SpriteGrid, encode_grids};
        let blob = encode_grids(
            VQ_ALPHABET,
            &[SpriteGrid {
                cols: VQ_DIMS.0 / 4,
                rows: VQ_DIMS.1,
                indices: &BASE_GRID,
            }],
            None,
        )
        .unwrap();
        ShippingMission::from_payload(ShippingMissionPayload {
            sprite_bank: Some(vq_test_bank(
                vec![(0, vq_sprite(Vec::new()))],
                vec![SpriteVqChunk {
                    rhs: "Characters/Test00.rhs".into(),
                    base_rhs: None,
                    base2_rhs: String::new(),
                    alphabet: VQ_ALPHABET,
                    sprite_ids: vec![0],
                    base_ids: vec![None],
                    base2_ids: Vec::new(),
                    self_refs: false,
                    blob,
                }],
            )),
            ..ShippingMissionPayload::default()
        })
    }

    /// Chunk mission for the variant: sprite 1 coded against base sprite 0,
    /// plus an RLE sprite 2 that keeps raw packed words.
    fn variant_chunk_mission() -> ShippingMission {
        use crate::sprite_codec::{SpriteGrid, encode_grids};
        let blob = encode_grids(
            VQ_ALPHABET,
            &[SpriteGrid {
                cols: VQ_DIMS.0 / 4,
                rows: VQ_DIMS.1,
                indices: &VARIANT_GRID,
            }],
            Some(&[Some(&BASE_GRID)]),
        )
        .unwrap();
        ShippingMission::from_payload(ShippingMissionPayload {
            sprite_bank: Some(vq_test_bank(
                vec![
                    (1, vq_sprite(Vec::new())),
                    (
                        2,
                        ShippingSprite {
                            width: 4,
                            height: 1,
                            dictionary_index: UNMAPPED_DICT,
                            packed_data: Arc::new(RLE_WORDS.to_vec()),
                            raster: None,
                        },
                    ),
                ],
                vec![SpriteVqChunk {
                    rhs: "Characters/Test01.rhs".into(),
                    base_rhs: Some("Characters/Test00.rhs".into()),
                    base2_rhs: String::new(),
                    alphabet: VQ_ALPHABET,
                    sprite_ids: vec![1],
                    base_ids: vec![Some(0)],
                    base2_ids: Vec::new(),
                    self_refs: false,
                    blob,
                }],
            )),
            ..ShippingMissionPayload::default()
        })
    }

    /// Chunk mission for the third family member: sprite 3 star-2 coded
    /// against base sprite 0 AND sibling sprite 1 (both from other chunks).
    fn second_variant_chunk_mission() -> ShippingMission {
        use crate::sprite_codec::{SpriteGrid, encode_grids_multi};
        let blob = encode_grids_multi(
            VQ_ALPHABET,
            &[SpriteGrid {
                cols: VQ_DIMS.0 / 4,
                rows: VQ_DIMS.1,
                indices: &SECOND_VARIANT_GRID,
            }],
            Some(&[Some(&BASE_GRID)]),
            Some(&[Some(&VARIANT_GRID)]),
        )
        .unwrap();
        ShippingMission::from_payload(ShippingMissionPayload {
            sprite_bank: Some(vq_test_bank(
                vec![(3, vq_sprite(Vec::new()))],
                vec![SpriteVqChunk {
                    rhs: "Characters/Test02.rhs".into(),
                    base_rhs: Some("Characters/Test00.rhs".into()),
                    base2_rhs: "Characters/Test01.rhs".into(),
                    alphabet: VQ_ALPHABET,
                    sprite_ids: vec![3],
                    base_ids: vec![Some(0)],
                    base2_ids: vec![Some(1)],
                    self_refs: false,
                    blob,
                }],
            )),
            ..ShippingMissionPayload::default()
        })
    }

    /// Lossless 8x4 RGBA JXL atlas (`cjxl -d 0 --alpha_distance=0 -e 7`)
    /// holding two RLE sprites: A (4x4) at (0,0) and B (4x2) at (4,0),
    /// generated from the exact canvases of `RLE_A_WORDS` / `RLE_B_WORDS`
    /// — opaque pixels expanded 565 -> 888, and every pixel's alpha set to
    /// its class marker. Lossless + 565-representable colors means
    /// materialization must reproduce the source words bit-for-bit.
    const RLE_JXL_FIXTURE: &[u8] = &[
        0xFF, 0x0A, 0x18, 0x70, 0xB0, 0x12, 0x08, 0x00, 0x10, 0x00, 0x18, 0x01, 0x4B, 0x18, 0x93,
        0x8E, 0x83, 0x83, 0x84, 0x13, 0xC4, 0x63, 0x8B, 0xCA, 0x5D, 0x40, 0x16, 0x00, 0x7C, 0x30,
        0xE4, 0xEA, 0xA5, 0xF8, 0xDF, 0x8C, 0x8B, 0x31, 0x02, 0x46, 0xED, 0x77, 0x3F, 0xAA, 0xD1,
        0xA2, 0x2F, 0x10, 0x60, 0x7A, 0x67, 0x49, 0x52, 0x7C, 0x91, 0x51, 0x6C, 0x16, 0x20, 0x7E,
        0x31, 0x86, 0x20, 0x46, 0x21, 0x68, 0xAF, 0x6A, 0x5B, 0xBB, 0x5E, 0x77, 0xA3, 0xC3, 0x95,
        0x72, 0xE0, 0xC6, 0x69, 0x1E, 0xBC, 0x01,
    ];
    const RLE_A_WORDS: [u16; 16] = [
        0,
        3,
        0x1234,
        0x5678,
        0x9ABC,
        0xDEF0,
        0xFFFF,
        0xFFFF,
        1,
        2,
        crate::frame_holder::SHADOW_KEY,
        crate::frame_holder::TRANSPARENT_COLOR_16,
        2,
        3,
        0x0000,
        0xFFFF,
    ];
    const RLE_B_WORDS: [u16; 7] = [0, 1, 0x8410, 0x4208, 3, 3, 0xF800];

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn rle_jxl_atlas_parallelism_preserves_placement_order_and_errors() {
        let mut chunk = SpriteRleJxlChunk {
            rhs: "Animations/Day/parallel.rhs".into(),
            jxl_blobs: vec![RLE_JXL_FIXTURE.to_vec(); 3],
            sprite_ids: vec![5, 9],
            placements: vec![
                RleJxlPlacement {
                    blob: 2,
                    x: 0,
                    y: 0,
                },
                RleJxlPlacement {
                    blob: 0,
                    x: 4,
                    y: 0,
                },
            ],
        };
        let dims = [(4, 4), (4, 2)];
        for (threads, parallel) in [(1, true), (4, true), (4, false)] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            let rasters = pool
                .install(|| {
                    ShippingSpriteBank::run_rle_jxl_chunk_decode_with_parallelism(
                        &chunk, &dims, parallel,
                    )
                })
                .unwrap();
            assert_eq!(
                rasters.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
                [5, 9]
            );
            for ((_, raster), ((width, height), words)) in rasters
                .iter()
                .zip(dims.into_iter().zip([&RLE_A_WORDS[..], &RLE_B_WORDS[..]]))
            {
                let (expected, _) =
                    crate::rle_jxl::decode_rle_canvas(width as usize, height as usize, words)
                        .unwrap();
                let actual: Vec<_> = (0..height as usize)
                    .flat_map(|y| raster.row(y, width as usize).unwrap().iter().copied())
                    .collect();
                assert_eq!(actual, expected);
            }
        }
        // Even an unreferenced atlas must be validated. Parallel collection
        // must propagate its error rather than silently dropping it.
        chunk.jxl_blobs[1] = vec![0];
        for parallel in [false, true] {
            let error = ShippingSpriteBank::run_rle_jxl_chunk_decode_with_parallelism(
                &chunk, &dims, parallel,
            )
            .unwrap_err();
            assert!(format!("{error:#}").contains("RLE-JXL blob 1 of Animations/Day/parallel.rhs"));
        }
    }

    #[test]
    fn rle_jxl_chunks_materialize_exact_words_from_lossless_fixture() {
        use crate::rle_jxl;
        let sprite = |w: u16, h: u16| ShippingSprite {
            width: w,
            height: h,
            dictionary_index: UNMAPPED_DICT,
            packed_data: Arc::new(Vec::new()),
            raster: None,
        };
        let mission = ShippingMission::from_payload(ShippingMissionPayload {
            sprite_bank: Some(ShippingSpriteBank {
                signature: 7,
                dictionaries: Vec::new(),
                sprite_count: 16,
                sprites: vec![(5, sprite(4, 4)), (9, sprite(4, 2))],
                vq_chunks: Vec::new(),
                rle_jxl_chunks: vec![SpriteRleJxlChunk {
                    rhs: "Animations/Day/test.rhs".into(),
                    jxl_blobs: vec![RLE_JXL_FIXTURE.to_vec()],
                    sprite_ids: vec![5, 9],
                    placements: vec![
                        RleJxlPlacement {
                            blob: 0,
                            x: 0,
                            y: 0,
                        },
                        RleJxlPlacement {
                            blob: 0,
                            x: 4,
                            y: 0,
                        },
                    ],
                }],
            }),
            ..ShippingMissionPayload::default()
        });
        // Ship it the way the converter does, then materialize like a
        // mission install.
        let compressed = zstd_compress_with_window(&encode_mission_native(&mission), 30).unwrap();
        let mut decoded = decode_mission_compressed(&compressed).unwrap();
        let bank = decoded.sprite_bank.as_mut().unwrap();
        bank.materialize_rle_jxl_chunks().unwrap();
        assert!(bank.rle_jxl_chunks.is_empty());
        // Both sprites now window into ONE shared atlas — nothing was
        // copied out of it, and no RLE words were rebuilt.
        let rasters: Vec<_> = [5u32, 9]
            .iter()
            .map(|id| bank.sprite_row(*id).unwrap().raster.clone().unwrap())
            .collect();
        assert!(bank.sprite_row(5).unwrap().packed_data.is_empty());
        assert!(Arc::ptr_eq(&rasters[0].atlas, &rasters[1].atlas));
        assert_eq!((rasters[0].stride, rasters[0].x), (8, 0));
        assert_eq!(rasters[1].x, 4);
        // The raster is exactly the canvas the packed words decompress to:
        // lossless color plus the class-carrying alpha reproduces it.
        for (raster, words, width, height) in [
            (&rasters[0], &RLE_A_WORDS[..], 4usize, 4usize),
            (&rasters[1], &RLE_B_WORDS[..], 4, 2),
        ] {
            let (expected, used) = rle_jxl::decode_rle_canvas(width, height, words).unwrap();
            assert_eq!(used, words.len());
            let actual: Vec<u16> = (0..height)
                .flat_map(|y| raster.row(y, width).unwrap().iter().copied())
                .collect();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn vq_chunks_roundtrip_and_materialize_in_any_merge_order() {
        // Serialize each chunk exactly the way the converter ships it.
        let reload = |mission: &ShippingMission| {
            let compressed =
                zstd_compress_with_window(&encode_mission_native(mission), 30).unwrap();
            decode_mission_compressed(&compressed).unwrap()
        };
        // Fetch completion order is nondeterministic on wasm: merge the
        // star-2 chunk first (its base2 sibling itself decodes against the
        // family base), then the variant, then the base, and materialize.
        let mut merged = ShippingMission::default();
        merged
            .merge_part(reload(&second_variant_chunk_mission()))
            .unwrap();
        merged.merge_part(reload(&variant_chunk_mission())).unwrap();
        merged.merge_part(reload(&base_chunk_mission())).unwrap();
        let bank = merged.sprite_bank.as_mut().unwrap();
        bank.materialize_vq_chunks(&BTreeMap::new()).unwrap();

        assert!(bank.vq_chunks.is_empty());
        assert_eq!(
            bank.sprite_row(0).unwrap().packed_data.as_slice(),
            BASE_GRID
        );
        assert_eq!(
            bank.sprite_row(1).unwrap().packed_data.as_slice(),
            VARIANT_GRID
        );
        assert_eq!(
            bank.sprite_row(2).unwrap().packed_data.as_slice(),
            RLE_WORDS
        );
        assert_eq!(
            bank.sprite_row(3).unwrap().packed_data.as_slice(),
            SECOND_VARIANT_GRID
        );
    }

    #[test]
    fn variant_vq_chunk_without_base_chunk_is_an_error() {
        let mut merged = ShippingMission::default();
        merged.merge_part(variant_chunk_mission()).unwrap();
        let error = merged
            .sprite_bank
            .as_mut()
            .unwrap()
            .materialize_vq_chunks(&BTreeMap::new())
            .unwrap_err();
        assert!(
            error.to_string().contains("base sprite 0"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn star2_vq_chunk_without_base2_chunk_is_an_error() {
        // The base chunk arrives but the base2 sibling chunk never does: the
        // star-2 chunk must fail loudly, naming the missing base2 RHS.
        let mut merged = ShippingMission::default();
        merged.merge_part(second_variant_chunk_mission()).unwrap();
        merged.merge_part(base_chunk_mission()).unwrap();
        let error = merged
            .sprite_bank
            .as_mut()
            .unwrap()
            .materialize_vq_chunks(&BTreeMap::new())
            .unwrap_err();
        let message = format!("{error:#}");
        assert!(
            message.contains("base2 sprite 1") && message.contains("Characters/Test01.rhs"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn shipping_installation_owns_vfs_and_has_first_priority() {
        let vfs = Arc::new(AssetVfs::new());
        let mut loose = Bundle::new();
        loose.insert("shared.dat".to_string(), b"loose".to_vec().into());
        vfs.mount_bundle(Arc::new(loose)).unwrap();

        let mut datadir = ShippingDatadir::default();
        datadir
            .raw
            .insert("shared.dat".to_string(), b"shipping".to_vec());
        datadir
            .raw
            .insert("sounds/menu.opus".to_string(), vec![1, 2, 3, 4]);
        datadir
            .audio_durations_ms
            .insert("sounds/menu.opus".to_string(), 250);
        let installed = ShippingAssets::install(Arc::new(datadir), vfs.clone()).unwrap();

        assert!(Arc::ptr_eq(installed.vfs(), &vfs));
        assert!(installed.datadir().raw.is_empty());
        assert_eq!(
            installed.datadir().raw_asset("shared.dat"),
            Some(&b"shipping"[..])
        );
        assert_eq!(
            installed
                .datadir()
                .active_audio_metadata(Path::new("Data/Sounds/Menu.wav")),
            Some((4, 250))
        );
        assert_eq!(installed.vfs().read("shared.dat").unwrap(), b"shipping");
    }

    #[test]
    fn remote_audio_catalog_resolves_legacy_aliases() {
        let mut datadir = ShippingDatadir::default();
        datadir.set_remote_base_url("https://example.test/build/Data/".into());
        datadir.audio_assets.insert(
            "sounds/arrow.opus".into(),
            ShippingAudioAsset {
                file: "audio/assets/abc.opus".into(),
                encoded_size: 321,
                duration_ms: 654,
                bundle_offset: None,
            },
        );
        datadir.audio_assets.insert(
            "sounds/exclamations/expressions/alert.opus".into(),
            ShippingAudioAsset {
                file: "audio/assets/voice.opus".into(),
                encoded_size: 111,
                duration_ms: 222,
                bundle_offset: None,
            },
        );

        let expected = RemoteAudioAsset {
            url: "https://example.test/build/Data/audio/assets/abc.opus".into(),
            encoded_size: 321,
            duration_ms: 654,
            bundle_offset: None,
        };
        assert_eq!(
            datadir.remote_audio_asset(Path::new("Data/Sounds/Arrow.wav")),
            Some(expected.clone())
        );
        assert_eq!(
            datadir.remote_audio_asset(Path::new("arrow.wav")),
            Some(expected.clone())
        );
        assert_eq!(
            datadir.remote_audio_asset(Path::new("/games/Robin Hood/Data/Sounds/Arrow.ogg")),
            Some(expected)
        );
        assert_eq!(
            datadir
                .remote_audio_asset(Path::new("Expressions/Alert.wav"))
                .unwrap()
                .url,
            "https://example.test/build/Data/audio/assets/voice.opus"
        );
        assert_eq!(
            datadir.active_audio_metadata(Path::new("Data/Sounds/Arrow.wav")),
            Some((321, 654))
        );
    }

    #[test]
    fn audio_warmup_membership_is_exact_for_boot_and_active_mission() {
        let mut datadir = install_fixture(ShippingDatadir::default());
        for key in [
            "sounds/menu/click.opus",
            "sounds/exclamations/robin/alert.opus",
            "sounds/not-mounted.opus",
        ] {
            datadir.audio_assets.insert(
                key.into(),
                ShippingAudioAsset {
                    file: format!("audio/assets/{key}"),
                    encoded_size: 10,
                    duration_ms: 100,
                    bundle_offset: None,
                },
            );
        }
        datadir
            .audio_durations_ms
            .insert("sounds/menu/click.opus".into(), 100);
        let mut mission = ShippingMission::default();
        mission
            .audio_durations_ms
            .insert("sounds/exclamations/robin/alert.opus".into(), 100);
        datadir
            .runtime
            .loaded_missions
            .write()
            .unwrap()
            .insert("MissionA".into(), Arc::new(mission));
        datadir
            .asset_vfs()
            .select_mission(Some("MissionA".into()), Arc::new(Default::default()))
            .unwrap();

        assert_eq!(
            datadir.boot_audio_keys(),
            vec!["sounds/menu/click.opus".to_owned()]
        );
        assert_eq!(
            datadir.active_audio_keys(),
            vec!["sounds/exclamations/robin/alert.opus".to_owned()]
        );
    }

    #[test]
    fn shipping_installation_propagates_invalid_bundle_path() {
        let vfs = Arc::new(AssetVfs::new());
        let mut datadir = ShippingDatadir::default();
        datadir
            .raw
            .insert("../escape.dat".to_string(), b"bad".to_vec());

        let datadir = Arc::new(datadir);
        let generation = vfs.selection_snapshot().generation;
        let error = ShippingAssets::install(datadir.clone(), vfs.clone()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("mount shipping raw asset bundle")
        );
        assert!(datadir.runtime.installed.get().is_none());
        assert!(vfs.authority_snapshot().is_empty());
        assert_eq!(vfs.selection_snapshot().generation, generation);
        assert_eq!(datadir.raw_asset("../escape.dat"), Some(&b"bad"[..]));
        // A retained decode can be corrected and retried, not left half-bound.
        let mut datadir = Arc::try_unwrap(datadir).unwrap();
        datadir.raw.remove("../escape.dat");
        datadir.raw.insert("valid.dat".into(), b"good".to_vec());
        let installed = ShippingAssets::install(Arc::new(datadir), vfs).unwrap();
        assert_eq!(installed.vfs().read("valid.dat").unwrap(), b"good");
    }

    #[test]
    #[should_panic(expected = "decoded data has no VFS")]
    fn decoded_data_has_no_implicit_runtime_authority() {
        ShippingDatadir::default().asset_vfs();
    }

    #[test]
    fn installation_state_is_excluded_from_shared_payload_wire_bytes() {
        let mut datadir = ShippingDatadir::default();
        datadir.raw.insert("marker".into(), vec![42]);
        let datadir = Arc::new(datadir);
        let native = encode_native(&datadir);
        let json = serde_json::to_vec(&*datadir).unwrap();
        let installed =
            ShippingAssets::install(datadir.clone(), Arc::new(AssetVfs::new())).unwrap();
        assert_eq!(encode_native(installed.datadir()), native);
        assert_eq!(serde_json::to_vec(&**installed.datadir()).unwrap(), json);
    }

    #[test]
    fn concurrent_installation_publishes_only_one_mount() {
        let mut datadir = ShippingDatadir::default();
        datadir.raw.insert("marker".into(), vec![42]);
        let datadir = Arc::new(datadir);
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let results = std::thread::scope(|scope| {
            let jobs: Vec<_> = (0..2)
                .map(|_| {
                    let datadir = datadir.clone();
                    let barrier = barrier.clone();
                    scope.spawn(move || {
                        let vfs = Arc::new(AssetVfs::new());
                        let generation = vfs.selection_snapshot().generation;
                        barrier.wait();
                        let result = ShippingAssets::install(datadir, vfs.clone());
                        (vfs, generation, result)
                    })
                })
                .collect();
            jobs.into_iter()
                .map(|job| job.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(
            results
                .iter()
                .filter(|(_, _, result)| result.is_ok())
                .count(),
            1
        );
        for (vfs, generation, result) in results {
            if let Ok(installed) = result {
                assert!(Arc::ptr_eq(datadir.asset_vfs(), installed.vfs()));
                assert_eq!(vfs.read("marker").unwrap(), [42]);
                let before = vfs.selection_snapshot().generation;
                assert!(ShippingAssets::install(datadir.clone(), vfs.clone()).is_err());
                assert_eq!(vfs.selection_snapshot().generation, before);
            } else {
                assert!(vfs.read("marker").is_err());
                assert!(vfs.authority_snapshot().is_empty());
                assert_eq!(vfs.selection_snapshot().generation, generation);
            }
        }
    }
}
