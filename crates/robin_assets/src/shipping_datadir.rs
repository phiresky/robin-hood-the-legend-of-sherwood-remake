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
pub use runtime::{
    LocaleLayer, ShippingLookup, StagedMissionInstall, is_locale_overlay_key,
    is_optional_english_fallback_key, is_required_locale_key,
};
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
pub use scheduler::VqDecodeScheduler;
#[cfg(test)]
use scheduler::vq_downstream_costs;
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
pub use scheduler_rle::RleJxlDecodeScheduler;
#[cfg(test)]
use scheduler_rle::order_rle_chunks_by_size;
pub use sprite_bank::{
    RLE_JXL_DECODE_WORK_PER_BYTE, SpriteChunkKinds, SpriteChunkMaterializer, SpriteChunkStage,
    SpriteMaterializeProgress, VQ_DECODE_WORK_PER_BYTE,
};

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

impl ShippingDatadirPayload {
    /// Parsed shipping archives are resident; their source `.res` is never
    /// shipped. Older converters serialized locale managers with the
    /// converter host's absolute archive paths and recovery enabled, so a
    /// browser could attempt to read `/home/...` paths. Decoding enforces the
    /// shipping invariant on every serialized manager (shared and per-locale)
    /// so already-published manifests are safe without regeneration.
    pub fn disable_persisted_resource_recovery(&mut self) {
        let mut leaked = Vec::new();
        let shared = self
            .res_files
            .iter_mut()
            .map(|(key, manager)| (String::new(), key, manager));
        let localized = self.locales.iter_mut().flat_map(|(locale, pack)| {
            pack.res_files
                .iter_mut()
                .map(move |(key, manager)| (locale.clone(), key, manager))
        });
        for (locale, key, manager) in shared.chain(localized) {
            if manager.has_recovery_file_entries() {
                leaked.push(if locale.is_empty() {
                    key.clone()
                } else {
                    format!("{locale}:{key}")
                });
            }
            manager.disable_recovery_for_shipping();
        }
        if !leaked.is_empty() {
            static WARNED: std::sync::Once = std::sync::Once::new();
            WARNED.call_once(|| {
                tracing::warn!(
                    archives = ?leaked,
                    "shipping manifest persisted legacy resource recovery paths; ignoring them (regenerate the datadir to drop them)"
                );
            });
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
/// the small cinematic catalog; video payloads are separate files. Keeping the raw overlay in the platform-neutral manifest makes
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

impl ShippingMission {
    /// [`ShippingMissionPayload::browser_image_blobs`] plus the AVIF images
    /// already moved into this mission's sealed raw bundle — the images an
    /// installed mission still decodes after publication (terrain maps,
    /// minimaps read through [`Self::raw_asset`]).
    pub fn installed_browser_image_blobs(&self) -> Vec<&[u8]> {
        let mut blobs = self.payload.browser_image_blobs();
        if let Some(bundle) = self.raw_bundle.get() {
            blobs.extend(
                bundle
                    .values()
                    .map(AsRef::as_ref)
                    .filter(|bytes| crate::browser_images::is_avif(bytes)),
            );
        }
        blobs
    }
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

fn push_browser_images_of<'a>(
    blobs: &mut Vec<&'a [u8]>,
    res_files: &'a BTreeMap<String, ResourceManager>,
    pak_files: &'a BTreeMap<String, Vec<EncodedPicture>>,
    raw: &'a BTreeMap<String, Vec<u8>>,
) {
    for manager in res_files.values() {
        blobs.extend(manager.browser_image_blobs());
    }
    for pictures in pak_files.values() {
        blobs.extend(
            pictures
                .iter()
                .filter_map(EncodedPicture::browser_image_bytes),
        );
    }
    blobs.extend(
        raw.values()
            .map(Vec::as_slice)
            .filter(|bytes| crate::browser_images::is_avif(bytes)),
    );
}

impl ShippingDatadirPayload {
    /// Every encoded image of the boot payload (datadir and all locales)
    /// that the web runtime must have the browser decode before synchronous
    /// consumers (interface pictures, loading-screen pak pictures) use it.
    pub fn boot_browser_image_blobs(&self) -> Vec<&[u8]> {
        let mut blobs = Vec::new();
        push_browser_images_of(&mut blobs, &self.res_files, &self.pak_files, &self.raw);
        for locale in self.locales.values() {
            push_browser_images_of(
                &mut blobs,
                &locale.res_files,
                &locale.pak_files,
                &locale.raw,
            );
        }
        blobs
    }
}

impl ShippingMissionPayload {
    /// Every encoded image of this mission payload (RLE sprite atlases,
    /// terrain maps, minimaps) that the browser must decode before sprite
    /// materialization and level loading use it.
    pub fn browser_image_blobs(&self) -> Vec<&[u8]> {
        let mut blobs: Vec<&[u8]> = self
            .sprite_bank
            .iter()
            .flat_map(|bank| bank.rle_jxl_chunks.iter())
            .flat_map(|chunk| chunk.jxl_blobs.iter())
            .map(Vec::as_slice)
            .filter(|bytes| crate::browser_images::is_avif(bytes))
            .collect();
        blobs.extend(
            self.raw
                .values()
                .map(Vec::as_slice)
                .filter(|bytes| crate::browser_images::is_avif(bytes)),
        );
        blobs
    }
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
#[path = "shipping_v18_contract.rs"]
mod v18_contract;

#[cfg(test)]
#[path = "shipping_v9_contract.rs"]
mod v9_contract;

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
mod tests;
