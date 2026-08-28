//! Shipping datadir manifest and lazily loaded mission payloads.
//!
//! Produced by the `convert_datadir --format shipping` binary and loaded
//! at engine startup (see [`try_load`]). When a shipping datadir is
//! present, individual subsystem loaders (`ProfileManager::load_all_legacy_cpf`,
//! `FrameHolder::initialize_sprite_bank`, `ResourceManager::attach_resource_file`,
//! etc.) consult it instead of reading legacy files off disk.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::frame_holder::{FrameDictionary, UNMAPPED_DICT};
use crate::res_descr::LevelDescriptors;
use crate::resource_manager::{EncodedPicture, ResourceManager};
use crate::scb::ScbFile;
use robin_engine::level_data::LoadedLevel;
use robin_engine::profiles::ProfileManager;
use robin_engine::sprite_script::SpriteInfo;

/// Top-level shipping payload.
///
/// Keys mirror the on-disk relative path under `Data/` so loaders can find
/// things under the same names they use for legacy I/O (e.g.
/// `"Interface/DEFAULT.RES"`, `"Levels/Dem_Lei_MP.rhm"`).
#[derive(Debug, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct ShippingDatadir {
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
    /// Runtime-only source directory containing `datadir.bin` and its payloads.
    #[serde(skip)]
    #[bitcode(skip)]
    source_dir: Option<PathBuf>,
    /// Runtime-only HTTP base used by the browser build.
    #[serde(skip)]
    #[bitcode(skip)]
    remote_base_url: Option<String>,
    /// Runtime shared-byte view of boot `raw`. Installation moves into this
    /// bundle when the manifest has a unique owner, avoiding a second copy.
    #[serde(skip)]
    #[bitcode(skip)]
    boot_raw_bundle: OnceLock<Arc<robin_util::asset_fs::Bundle>>,
    /// Payloads already installed for this process. Kept out of the manifest.
    #[serde(skip)]
    #[bitcode(skip)]
    loaded_missions: RwLock<BTreeMap<String, Arc<ShippingMission>>>,
    #[serde(skip)]
    #[bitcode(skip)]
    active_mission: RwLock<Option<String>>,
    /// Exact static + dynamic exclamation closure for the active mission.
    #[serde(skip)]
    #[bitcode(skip)]
    active_exclamation_ids: RwLock<BTreeSet<u32>>,
    /// Runtime-only canonical locale selected for parsed-resource lookups.
    /// The corresponding raw overlay lives in `AssetVfs`' replaceable locale
    /// slot; both are changed by `set_active_locale*`.
    #[serde(skip)]
    #[bitcode(skip)]
    active_locale: RwLock<Option<String>>,
    /// Runtime-only, lazily shared copies of locale raw-file bundles. Locale
    /// switching can mount these on native, browser, and Android VFSes without
    /// re-cloning every byte on each switch.
    #[serde(skip)]
    #[bitcode(skip)]
    locale_bundle_cache: RwLock<BTreeMap<String, Arc<robin_util::asset_fs::Bundle>>>,
}

impl Default for ShippingDatadir {
    fn default() -> Self {
        Self {
            profiles: None,
            res_files: BTreeMap::new(),
            pak_files: BTreeMap::new(),
            red_files: BTreeMap::new(),
            levels: BTreeMap::new(),
            scripts: BTreeMap::new(),
            rhs_files: BTreeMap::new(),
            sprite_bank: None,
            raw: BTreeMap::new(),
            audio_durations_ms: BTreeMap::new(),
            audio_assets: BTreeMap::new(),
            missions: BTreeMap::new(),
            character_rhs_files: BTreeMap::new(),
            character_audio_files: BTreeMap::new(),
            character_exclamation_ids: BTreeMap::new(),
            mission_exclamation_ids: BTreeMap::new(),
            saved_world_rhs_files: Vec::new(),
            locales: BTreeMap::new(),
            source_dir: None,
            remote_base_url: None,
            boot_raw_bundle: OnceLock::new(),
            loaded_missions: RwLock::new(BTreeMap::new()),
            active_mission: RwLock::new(None),
            active_exclamation_ids: RwLock::new(BTreeSet::new()),
            active_locale: RwLock::new(None),
            locale_bundle_cache: RwLock::new(BTreeMap::new()),
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
    pub raw: robin_util::asset_fs::Bundle,
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
    /// Proto-level forest flag used by the original PC constructor to select
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
pub struct ShippingMission {
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
    /// Runtime shared-byte view of `raw`. Installation moves the decoded
    /// vectors here so the VFS and mission payload share the same allocation.
    #[serde(skip)]
    #[bitcode(skip)]
    raw_bundle: OnceLock<Arc<robin_util::asset_fs::Bundle>>,
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
    let mut try_pair =
        |cur: u32,
         r: u32,
         oc: (i32, i32),
         or_: (i32, i32),
         refs: &mut Vec<Option<crate::sprite_codec::SelfRef>>| {
            if r >= cur {
                return;
            }
            let (Some(&cur_pos), Some(&ref_pos)) = (batch_index.get(&cur), batch_index.get(&r))
            else {
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
    /// whose grid lives in a [`SpriteVqChunk`] blob until materialization.
    pub packed_data: Arc<Vec<u16>>,
}

/// Dispatcher state for worker-pool VQ chunk decode (wasm-threads builds).
///
/// Owns the set of in-flight decodes. The dispatching thread alternates
/// [`Self::dispatch_ready`] (move dependency-satisfied chunks onto the rayon
/// pool) with [`Self::apply_next`] (await one completion and write it into
/// the bank), so a family hub's variants unblock immediately when the hub
/// lands. The mission loader also drives this incrementally while part files
/// are still downloading.
///
/// Transient scheduling state, never serialized — deliberately no serde.
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
#[derive(Default)]
pub struct VqDecodeScheduler {
    in_flight: futures_util::stream::FuturesUnordered<
        futures_channel::oneshot::Receiver<(SpriteVqChunk, Result<Vec<(u32, Vec<u16>)>>, f64)>,
    >,
}

#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
impl VqDecodeScheduler {
    /// Move every chunk of `pending` whose base grids are materialized onto
    /// the worker pool. With `strict` readiness a chunk naming a base sprite
    /// that is missing from the bank is a hard error (the full mission
    /// payload is present, so the manifest is broken); lenient readiness
    /// treats it as "not yet" — the row is still being fetched.
    pub fn dispatch_ready(
        &mut self,
        bank: &ShippingSpriteBank,
        pending: &mut Vec<SpriteVqChunk>,
        rhs_files: &BTreeMap<String, RhsData>,
        strict: bool,
    ) -> Result<()> {
        // Longest-first dispatch: rayon's injected queue is FIFO, so this
        // starts the biggest blobs (family hubs — the heads of the longest
        // dependency chains) before the small variants pile onto the
        // workers. Chunk decode time tracks blob size closely.
        pending.sort_by_key(|chunk| std::cmp::Reverse(chunk.blob.len()));
        let mut index = 0;
        while index < pending.len() {
            let ready = if strict {
                bank.vq_chunk_bases_ready(&pending[index])?
            } else {
                bank.vq_chunk_ready_lenient(&pending[index], rhs_files)?
            };
            if !ready {
                index += 1;
                continue;
            }
            // Order-preserving removal (`swap_remove` would drag the
            // smallest chunk into the just-vacated slot and dispatch it
            // second). The list is tens of entries; O(n) shifting is noise.
            let chunk = pending.remove(index);
            let inputs = bank
                .prepare_vq_chunk_inputs(&chunk, rhs_files)
                .with_context(|| format!("decode VQ sprite chunk for {}", chunk.rhs))?;
            let (sender, receiver) = futures_channel::oneshot::channel();
            rayon::spawn(move || {
                // Workers can call JS imports of their own instantiation;
                // Date.now is the cheap cross-thread clock here.
                let started = js_sys::Date::now();
                let grids = ShippingSpriteBank::run_vq_chunk_decode(&chunk, &inputs);
                let elapsed = js_sys::Date::now() - started;
                // An unreceived result only means the dispatcher bailed out
                // on an earlier chunk's error; nothing to report.
                let _ = sender.send((chunk, grids, elapsed));
            });
            self.in_flight.push(receiver);
        }
        Ok(())
    }

    /// Await the next completed decode. `Ok(None)` when no decode is in
    /// flight. Cancel-safe: dropping the returned future before completion
    /// loses nothing (the mission loader races this against part fetches).
    pub async fn next_decoded(&mut self) -> Result<Option<(SpriteVqChunk, Vec<(u32, Vec<u16>)>)>> {
        use futures_util::StreamExt as _;
        let Some(result) = self.in_flight.next().await else {
            return Ok(None);
        };
        let (chunk, grids, decode_ms) =
            result.map_err(|_| anyhow!("VQ decode worker dropped its result"))?;
        let grids = grids.with_context(|| format!("decode VQ sprite chunk for {}", chunk.rhs))?;
        tracing::debug!(chunk = %chunk.rhs, decode_ms, "VQ sprite chunk decoded on worker");
        Ok(Some((chunk, grids)))
    }

    /// Await the next completed decode and write its grids into the bank.
    /// `Ok(false)` when no decode is in flight.
    pub async fn apply_next(&mut self, bank: &mut ShippingSpriteBank) -> Result<bool> {
        let Some((chunk, grids)) = self.next_decoded().await? else {
            return Ok(false);
        };
        bank.apply_decoded_vq_chunk(&chunk, grids)?;
        Ok(true)
    }

    /// True while at least one decode is running on the pool.
    pub fn has_in_flight(&self) -> bool {
        !self.in_flight.is_empty()
    }
}

/// Error for a fixpoint round that made no progress: every remaining chunk
/// names base sprites that never materialized, meaning the manifest omitted a
/// base RHS chunk from the mission payload.
fn vq_chunks_stuck_error(still_pending: &[SpriteVqChunk]) -> anyhow::Error {
    let stuck: Vec<String> = still_pending
        .iter()
        .map(|chunk| {
            let mut label = format!(
                "{} (base {}",
                chunk.rhs,
                chunk.base_rhs.as_deref().unwrap_or("?")
            );
            if !chunk.base2_rhs.is_empty() {
                label.push_str(&format!(", base2 {}", chunk.base2_rhs));
            }
            label.push(')');
            label
        })
        .collect();
    anyhow!(
        "VQ sprite chunks cannot be decoded because their base sprites never \
         materialized — base RHS chunk missing from the mission payload: {}",
        stuck.join(", ")
    )
}

/// Owned inputs for one chunk's grid decode, resolved from the bank by
/// [`ShippingSpriteBank::prepare_vq_chunk_inputs`]. `Send + 'static` (base
/// grids are `Arc`-shared with the bank rows) so the decode itself can run on
/// a rayon worker while the bank stays borrowed on the dispatching thread.
/// Transient decode state, never serialized — deliberately no serde derives.
struct VqChunkDecodeInputs {
    /// Per sprite: `(width / 4, height)` — the VQ grid dimensions.
    dims: Vec<(u16, u16)>,
    selfref: Vec<Option<crate::sprite_codec::SelfRef>>,
    base_grids: Vec<Option<Arc<Vec<u16>>>>,
    base2_grids: Vec<Option<Arc<Vec<u16>>>>,
}

impl ShippingSpriteBank {
    /// Decode every [`SpriteVqChunk`] blob back into per-sprite packed index
    /// data, consuming the chunk list.
    ///
    /// Chunks coded against a family base need that base's grids first; the
    /// base always arrives in a separate chunk of the same mission closure
    /// and mission parts may merge in any fetch-completion order, so decoding
    /// iterates to a fixpoint over the chunk list. A chunk whose base sprites
    /// are missing from the payload altogether is a hard error — the
    /// conversion lists the base RHS chunk as an explicit dependency, so its
    /// absence means a broken manifest, never something to paper over.
    pub fn materialize_vq_chunks(&mut self, rhs_files: &BTreeMap<String, RhsData>) -> Result<()> {
        let mut pending = std::mem::take(&mut self.vq_chunks);
        while !pending.is_empty() {
            let mut ready = Vec::new();
            let mut still_pending = Vec::new();
            for chunk in pending {
                if self.vq_chunk_bases_ready(&chunk)? {
                    ready.push(chunk);
                } else {
                    still_pending.push(chunk);
                }
            }
            let made_progress = !ready.is_empty();
            // Chunks within one fixpoint round are independent (their bases
            // are already materialized), so decode them in parallel on
            // native; wasm has no thread pool and stays serial.
            #[cfg(not(target_arch = "wasm32"))]
            let decoded: Vec<(SpriteVqChunk, Result<Vec<(u32, Vec<u16>)>>)> = {
                use rayon::prelude::*;
                ready
                    .into_par_iter()
                    .map(|chunk| {
                        let grids = self.decode_vq_chunk(&chunk, rhs_files);
                        (chunk, grids)
                    })
                    .collect()
            };
            #[cfg(target_arch = "wasm32")]
            let decoded: Vec<(SpriteVqChunk, Result<Vec<(u32, Vec<u16>)>>)> = ready
                .into_iter()
                .map(|chunk| {
                    let grids = self.decode_vq_chunk(&chunk, rhs_files);
                    (chunk, grids)
                })
                .collect();
            for (chunk, grids) in decoded {
                let grids =
                    grids.with_context(|| format!("decode VQ sprite chunk for {}", chunk.rhs))?;
                self.apply_decoded_vq_chunk(&chunk, grids)?;
            }
            if !made_progress {
                return Err(vq_chunks_stuck_error(&still_pending));
            }
            pending = still_pending;
        }
        Ok(())
    }

    /// Parallel wasm counterpart of [`Self::materialize_vq_chunks`]: each
    /// chunk's decode is dispatched to the rayon worker pool the moment its
    /// base grids exist, while this (main) thread only prepares inputs and
    /// applies results. Awaiting instead of blocking matters on the browser
    /// main thread, which must never `atomics.wait`.
    ///
    /// Unlike the serial fixpoint's rounds, there is no barrier here: one
    /// family's variants start decoding as soon as their own hub is applied,
    /// not when the slowest chunk of the previous round happens to finish —
    /// the wall time is bounded by the longest single dependency chain, and
    /// hub -> variant chains are at most a few links deep.
    ///
    /// Falls back to the serial [`Self::materialize_vq_chunks`] when the pool
    /// was never initialized (page not cross-origin isolated).
    #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
    pub async fn materialize_vq_chunks_parallel(
        &mut self,
        rhs_files: &BTreeMap<String, RhsData>,
    ) -> Result<()> {
        if crate::wasm_threads::pool_threads() == 0 {
            return self.materialize_vq_chunks(rhs_files);
        }
        let mut pending = std::mem::take(&mut self.vq_chunks);
        let mut scheduler = VqDecodeScheduler::default();
        loop {
            scheduler.dispatch_ready(self, &mut pending, rhs_files, true)?;
            if !scheduler.apply_next(self).await? {
                break;
            }
        }
        if pending.is_empty() {
            Ok(())
        } else {
            Err(vq_chunks_stuck_error(&pending))
        }
    }

    fn sprite_row(&self, id: u32) -> Option<&ShippingSprite> {
        self.sprites
            .binary_search_by_key(&id, |(id, _)| *id)
            .ok()
            .map(|position| &self.sprites[position].1)
    }

    /// Expected VQ grid length for a sprite row (tiles are 4x1 pixels).
    fn vq_grid_len(sprite: &ShippingSprite) -> usize {
        (sprite.width as usize / 4) * sprite.height as usize
    }

    /// `Ok(true)` when every base grid this chunk needs is materialized.
    /// `Ok(false)` when a base sprite exists but its grid is still pending
    /// (its own chunk decodes later in the fixpoint loop). An entirely
    /// missing or non-VQ base sprite is an error.
    fn vq_chunk_bases_ready(&self, chunk: &SpriteVqChunk) -> Result<bool> {
        if chunk.base2_rhs.is_empty() && chunk.base2_ids.iter().any(Option::is_some) {
            return Err(anyhow!(
                "VQ sprite chunk for {} carries base2 sprite ids without a base2 RHS",
                chunk.rhs
            ));
        }
        for (label, base_rhs, ids) in [
            (
                "base",
                chunk.base_rhs.as_deref().unwrap_or("?"),
                &chunk.base_ids,
            ),
            ("base2", chunk.base2_rhs.as_str(), &chunk.base2_ids),
        ] {
            for base_id in ids.iter().flatten() {
                let base = self.sprite_row(*base_id).ok_or_else(|| {
                    anyhow!(
                        "VQ sprite chunk for {} needs {label} sprite {base_id} from {base_rhs}, \
                         which is not part of this mission payload",
                        chunk.rhs
                    )
                })?;
                if base.dictionary_index == UNMAPPED_DICT {
                    return Err(anyhow!(
                        "VQ sprite chunk for {} names {label} sprite {base_id}, which is not \
                         dictionary-coded",
                        chunk.rhs
                    ));
                }
                let expected = Self::vq_grid_len(base);
                match base.packed_data.len() {
                    len if len == expected => {}
                    0 => return Ok(false),
                    len => {
                        return Err(anyhow!(
                            "{label} sprite {base_id} for chunk {} has {len} packed words, \
                             expected {expected}",
                            chunk.rhs
                        ));
                    }
                }
            }
        }
        Ok(true)
    }

    /// Streaming-time readiness: like [`Self::vq_chunk_bases_ready`], but
    /// while mission parts are still arriving nothing is allowed to be a
    /// "missing from the payload" error — a base sprite row, the chunk's own
    /// sprite rows, or its RHS metadata may simply not have been fetched yet,
    /// so all of those report `Ok(false)`. Structural contradictions in rows
    /// that DID arrive (non-VQ base, wrong grid length) still error: rows are
    /// immutable once merged, so waiting longer cannot fix them.
    #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
    fn vq_chunk_ready_lenient(
        &self,
        chunk: &SpriteVqChunk,
        rhs_files: &BTreeMap<String, RhsData>,
    ) -> Result<bool> {
        if chunk.self_refs && !rhs_files.contains_key(&chunk.rhs) {
            return Ok(false);
        }
        for sprite_id in &chunk.sprite_ids {
            match self.sprite_row(*sprite_id) {
                None => return Ok(false),
                Some(sprite) if sprite.dictionary_index == UNMAPPED_DICT => {
                    return Err(anyhow!(
                        "chunk for {} names sprite {sprite_id}, which is not dictionary-coded",
                        chunk.rhs
                    ));
                }
                Some(_) => {}
            }
        }
        if chunk.base2_rhs.is_empty() && chunk.base2_ids.iter().any(Option::is_some) {
            return Err(anyhow!(
                "VQ sprite chunk for {} carries base2 sprite ids without a base2 RHS",
                chunk.rhs
            ));
        }
        for ids in [&chunk.base_ids, &chunk.base2_ids] {
            for base_id in ids.iter().flatten() {
                let Some(base) = self.sprite_row(*base_id) else {
                    return Ok(false);
                };
                if base.dictionary_index == UNMAPPED_DICT {
                    return Err(anyhow!(
                        "VQ sprite chunk for {} names base sprite {base_id}, which is not \
                         dictionary-coded",
                        chunk.rhs
                    ));
                }
                let expected = Self::vq_grid_len(base);
                match base.packed_data.len() {
                    len if len == expected => {}
                    0 => return Ok(false),
                    len => {
                        return Err(anyhow!(
                            "base sprite {base_id} for chunk {} has {len} packed words, \
                             expected {expected}",
                            chunk.rhs
                        ));
                    }
                }
            }
        }
        Ok(true)
    }

    /// Resolve everything one chunk's decode needs from the bank into an
    /// owned, `Send + 'static` bundle, so [`Self::run_vq_chunk_decode`] can
    /// execute on any thread without borrowing `self`. Immutable so
    /// independent chunks of a fixpoint round can prepare/decode in parallel.
    fn prepare_vq_chunk_inputs(
        &self,
        chunk: &SpriteVqChunk,
        rhs_files: &BTreeMap<String, RhsData>,
    ) -> Result<VqChunkDecodeInputs> {
        let selfref: Vec<Option<crate::sprite_codec::SelfRef>> = if chunk.self_refs {
            let rhs_data = rhs_files.get(&chunk.rhs).ok_or_else(|| {
                anyhow!(
                    "VQ sprite chunk for {} declares self-references but its RHS metadata is \
                     not part of this mission payload",
                    chunk.rhs
                )
            })?;
            derive_chunk_self_refs(&rhs_data.profiles, &chunk.sprite_ids)
        } else {
            vec![None; chunk.sprite_ids.len()]
        };
        if chunk.base_ids.len() != chunk.sprite_ids.len() {
            return Err(anyhow!(
                "chunk lists {} sprites but {} base entries",
                chunk.sprite_ids.len(),
                chunk.base_ids.len()
            ));
        }
        if !chunk.base2_ids.is_empty() && chunk.base2_ids.len() != chunk.sprite_ids.len() {
            return Err(anyhow!(
                "chunk lists {} sprites but {} base2 entries",
                chunk.sprite_ids.len(),
                chunk.base2_ids.len()
            ));
        }
        let mut dims = Vec::with_capacity(chunk.sprite_ids.len());
        // Cloned `Arc`s keep the base grids alive independently of `self`, so
        // the decoded grids can be written back through `&mut self` below.
        let mut base_grids: Vec<Option<Arc<Vec<u16>>>> = Vec::with_capacity(chunk.base_ids.len());
        let mut base2_grids: Vec<Option<Arc<Vec<u16>>>> = Vec::with_capacity(chunk.base_ids.len());
        for (index, (sprite_id, base_id)) in
            chunk.sprite_ids.iter().zip(&chunk.base_ids).enumerate()
        {
            let sprite = self.sprite_row(*sprite_id).ok_or_else(|| {
                anyhow!("chunk names sprite {sprite_id}, which the payload does not contain")
            })?;
            if sprite.dictionary_index == UNMAPPED_DICT {
                return Err(anyhow!(
                    "chunk names sprite {sprite_id}, which is not dictionary-coded"
                ));
            }
            dims.push((sprite.width / 4, sprite.height));
            // Availability and length were proven by `vq_chunk_bases_ready`.
            let resolve = |base_id: &Option<u32>| {
                base_id.map(|base_id| {
                    Arc::clone(
                        &self
                            .sprite_row(base_id)
                            .expect("base sprite checked by vq_chunk_bases_ready")
                            .packed_data,
                    )
                })
            };
            base_grids.push(resolve(base_id));
            base2_grids.push(resolve(chunk.base2_ids.get(index).unwrap_or(&None)));
        }
        Ok(VqChunkDecodeInputs {
            dims,
            selfref,
            base_grids,
            base2_grids,
        })
    }

    /// Decode one chunk's blob into `(sprite id, grid)` pairs. Pure compute
    /// over the prepared inputs — no `self` access, so a wasm worker thread
    /// can run it against inputs prepared on the main thread.
    fn run_vq_chunk_decode(
        chunk: &SpriteVqChunk,
        inputs: &VqChunkDecodeInputs,
    ) -> Result<Vec<(u32, Vec<u16>)>> {
        fn as_slices(grids: &[Option<Arc<Vec<u16>>>]) -> Vec<Option<&[u16]>> {
            grids
                .iter()
                .map(|grid| grid.as_ref().map(|grid| grid.as_slice()))
                .collect()
        }
        let base_slices = as_slices(&inputs.base_grids);
        let base2_slices = as_slices(&inputs.base2_grids);
        let decoded = crate::sprite_codec::decode_grids_shipping(
            chunk.alphabet,
            &inputs.dims,
            Some(&base_slices),
            Some(&base2_slices),
            &inputs.selfref,
            &chunk.blob,
        )?;
        Ok(chunk.sprite_ids.iter().copied().zip(decoded).collect())
    }

    /// Prepare and decode in one step; the serial and rayon fixpoint rounds
    /// run this per chunk.
    fn decode_vq_chunk(
        &self,
        chunk: &SpriteVqChunk,
        rhs_files: &BTreeMap<String, RhsData>,
    ) -> Result<Vec<(u32, Vec<u16>)>> {
        let inputs = self.prepare_vq_chunk_inputs(chunk, rhs_files)?;
        Self::run_vq_chunk_decode(chunk, &inputs)
    }

    /// Decode and apply ONE lenient-ready chunk of `pending` on the calling
    /// thread. `Ok(true)` when a chunk was materialized; `Ok(false)` when
    /// nothing in `pending` is ready yet. The serial-fallback streaming
    /// loader calls this repeatedly (yielding to the browser between calls)
    /// to overlap decode with the remaining part downloads.
    #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
    pub fn materialize_next_ready_vq_chunk(
        &mut self,
        pending: &mut Vec<SpriteVqChunk>,
        rhs_files: &BTreeMap<String, RhsData>,
    ) -> Result<bool> {
        let Some(position) = pending
            .iter()
            .map(|chunk| self.vq_chunk_ready_lenient(chunk, rhs_files))
            .collect::<Result<Vec<bool>>>()?
            .iter()
            .position(|&ready| ready)
        else {
            return Ok(false);
        };
        let chunk = pending.swap_remove(position);
        let grids = self
            .decode_vq_chunk(&chunk, rhs_files)
            .with_context(|| format!("decode VQ sprite chunk for {}", chunk.rhs))?;
        self.apply_decoded_vq_chunk(&chunk, grids)?;
        Ok(true)
    }

    /// Write one chunk's decoded grids into the sprite rows.
    pub fn apply_decoded_vq_chunk(
        &mut self,
        _chunk: &SpriteVqChunk,
        grids: Vec<(u32, Vec<u16>)>,
    ) -> Result<()> {
        for (sprite_id, grid) in grids {
            let sprite_id = &sprite_id;
            let position = self
                .sprites
                .binary_search_by_key(sprite_id, |(id, _)| *id)
                .map_err(|_| anyhow!("sprite {sprite_id} disappeared during materialization"))?;
            let sprite = &mut self.sprites[position].1;
            if !sprite.packed_data.is_empty() {
                // The same bank sprite can be listed by two chunks of one
                // closure; both blobs must decode it identically.
                if *sprite.packed_data != grid {
                    return Err(anyhow!(
                        "sprite {sprite_id} decodes differently in two VQ chunks"
                    ));
                }
                continue;
            }
            sprite.packed_data = Arc::new(grid);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
//  I/O
// ---------------------------------------------------------------------------

impl ShippingDatadir {
    /// Iterate installed language packs in stable canonical-locale order.
    pub fn available_locales(&self) -> impl Iterator<Item = (&str, &ShippingLocale)> {
        self.locales
            .iter()
            .map(|(locale, assets)| (locale.as_str(), assets))
    }

    /// Resolve a canonical locale, Windows LCID, or retained alias to a pack.
    /// Invalid identifiers are errors; absent packs return `Ok(None)`.
    pub fn locale(&self, locale: &str) -> Result<Option<&ShippingLocale>> {
        if let Some((_, assets)) = self.locales.iter().find(|(_, assets)| {
            assets
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(locale.trim()))
        }) {
            return Ok(Some(assets));
        }
        let canonical = canonical_locale_id(locale)?;
        Ok(self.locales.get(&canonical))
    }

    /// Whether this manifest predates explicit locale metadata. Its top-level
    /// resources remain usable, but their language identity is unknowable and
    /// must not be advertised as a specific installed locale.
    pub fn is_legacy_single_locale(&self) -> bool {
        self.locales.is_empty()
    }

    pub fn locale_resource(&self, locale: &str, path: &str) -> Result<Option<&ResourceManager>> {
        let key = canonical_shipping_asset_key(path);
        Ok(self
            .locale(locale)?
            .and_then(|assets| assets.res_files.get(&key)))
    }

    pub fn locale_pak(&self, locale: &str, path: &str) -> Result<Option<&[EncodedPicture]>> {
        let key = canonical_shipping_asset_key(path);
        Ok(self
            .locale(locale)?
            .and_then(|assets| assets.pak_files.get(&key))
            .map(Vec::as_slice))
    }

    pub fn locale_level_descriptors(
        &self,
        locale: &str,
        path: &str,
    ) -> Result<Option<&LevelDescriptors>> {
        let key = canonical_shipping_asset_key(path);
        let filename = key.rsplit('/').next().unwrap_or(&key);
        Ok(self
            .locale(locale)?
            .and_then(|assets| assets.red_files.get(filename)))
    }

    pub fn locale_raw(&self, locale: &str, path: &str) -> Result<Option<&[u8]>> {
        let key = canonical_shipping_asset_key(path);
        Ok(self
            .locale(locale)?
            .and_then(|assets| assets.raw.get(&key))
            .map(Vec::as_slice))
    }

    /// Return a shareable VFS overlay for a locale. The first call clones the
    /// serialized raw map into an `Arc`; subsequent calls share that allocation.
    pub fn locale_bundle(&self, locale: &str) -> Result<Option<Arc<robin_util::asset_fs::Bundle>>> {
        let canonical = if let Some((canonical, _)) = self.locales.iter().find(|(_, assets)| {
            assets
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(locale.trim()))
        }) {
            canonical.clone()
        } else {
            canonical_locale_id(locale)?
        };
        let Some(assets) = self.locales.get(&canonical) else {
            return Ok(None);
        };
        if let Some(cached) = self
            .locale_bundle_cache
            .read()
            .expect("shipping locale bundle cache lock poisoned")
            .get(&canonical)
            .cloned()
        {
            return Ok(Some(cached));
        }

        // A locale is a complete text/UI overlay. English is only a fallback
        // for the two explicitly optional presentation families; allowing it
        // to fill arbitrary missing files would create a partly translated UI
        // and conceal an incomplete pack.
        let mut raw = self
            .locales
            .get("en-US")
            .filter(|_| canonical != "en-US")
            .map(|english| {
                english
                    .raw
                    .iter()
                    .filter(|(key, _)| is_optional_english_fallback_key(key))
                    .map(|(key, bytes)| (key.clone(), bytes.clone()))
                    .collect::<robin_util::asset_fs::Bundle>()
            })
            .unwrap_or_default();
        raw.extend(assets.raw.clone());
        let bundle = Arc::new(raw);
        let bundle = self
            .locale_bundle_cache
            .write()
            .expect("shipping locale bundle cache lock poisoned")
            .entry(canonical)
            .or_insert_with(|| bundle.clone())
            .clone();
        Ok(Some(bundle))
    }

    /// Atomically select the parsed and raw resources used by runtime
    /// language-aware loaders. `None` clears the locale overlay for legacy
    /// single-language manifests.
    pub fn set_active_locale(&self, locale: Option<&str>) -> Result<()> {
        let (canonical, bundle) = match locale {
            Some(locale) => {
                let canonical = canonical_locale_id(locale)?;
                if !self.locales.contains_key(&canonical) {
                    return Err(anyhow!("shipping locale {canonical} is not installed"));
                }
                let bundle = self
                    .locale_bundle(&canonical)?
                    .ok_or_else(|| anyhow!("shipping locale {canonical} has no raw bundle"))?;
                (Some(canonical), Some(bundle))
            }
            None => (None, None),
        };

        robin_util::asset_fs::install_locale_bundle(bundle)
            .context("install shipping locale bundle")?;
        *self
            .active_locale
            .write()
            .expect("shipping active locale lock poisoned") = canonical;
        Ok(())
    }

    pub fn active_locale_name(&self) -> Option<String> {
        self.active_locale
            .read()
            .expect("shipping active locale lock poisoned")
            .clone()
    }

    pub fn active_resource(&self, path: &str) -> Option<&ResourceManager> {
        if !is_locale_overlay_key(path) {
            return None;
        }
        let locale = self.active_locale_name()?;
        self.locale_resource(&locale, path).ok().flatten()
    }

    pub fn active_pak(&self, path: &str) -> Option<&[EncodedPicture]> {
        let locale = self.active_locale_name()?;
        self.locale_pak(&locale, path).ok().flatten()
    }

    pub fn localized_pak(&self, path: &str) -> Option<&[EncodedPicture]> {
        if self.active_locale_name().is_some() {
            // PAKs under locale roots commonly bake translated titles into
            // their pixels. A v5 manifest must not substitute the top-level
            // compatibility pack when the selected locale omitted one.
            self.active_pak(path)
        } else {
            let key = canonical_shipping_asset_key(path);
            self.pak_files.get(&key).map(Vec::as_slice)
        }
    }

    pub fn active_level_descriptors(&self, path: &str) -> Option<&LevelDescriptors> {
        let locale = self.active_locale_name()?;
        self.locale_level_descriptors(&locale, path).ok().flatten()
    }

    /// Resolve a descriptor in the selected locale. Compatibility manifests
    /// without locale metadata continue to use their top-level descriptor map;
    /// a v5 pack never silently substitutes a different language.
    pub fn localized_level_descriptors(&self, path: &str) -> Option<&LevelDescriptors> {
        if self.active_locale_name().is_some() {
            self.active_level_descriptors(path)
        } else {
            let key = canonical_shipping_asset_key(path);
            let filename = key.rsplit('/').next().unwrap_or(&key);
            self.red_files.get(filename)
        }
    }

    /// Localized profile metadata for presentation lookups such as mission
    /// titles. Engine construction must use the language-independent top-level
    /// `profiles` index so a client locale cannot alter simulation data.
    pub fn active_profiles(&self) -> Option<&ProfileManager> {
        let locale = self.active_locale_name()?;
        self.locales
            .get(&locale)
            .and_then(|assets| assets.profiles.as_ref())
    }

    /// Parse a shipping datadir blob: zstd decompress + native bitcode decode.
    pub fn load_from_file(path: &Path) -> Result<Self> {
        let compressed =
            robin_util::asset_fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let mut datadir = Self::from_compressed_bytes(&compressed)
            .with_context(|| format!("decode {}", path.display()))?;
        datadir.source_dir = path.parent().map(Path::to_path_buf);
        Ok(datadir)
    }

    /// Load through an explicit VFS instance.
    pub fn load_from_vfs(vfs: &robin_util::asset_fs::AssetVfs, path: &Path) -> Result<Self> {
        let compressed = vfs
            .read(path)
            .with_context(|| format!("read {}", path.display()))?;
        let mut datadir = Self::from_compressed_bytes(&compressed)
            .with_context(|| format!("decode {}", path.display()))?;
        datadir.source_dir = path.parent().map(Path::to_path_buf);
        Ok(datadir)
    }

    /// Parse a shipping datadir blob already in memory.  Used by the
    /// wasm-bindgen bootstrap, which fetches `datadir.bin` from JS,
    /// hands the bytes to Rust, and decodes here — bypassing the
    /// `asset_fs::read` path (which is bundle-only on wasm and the
    /// bundle isn't installed yet at this point).
    pub fn from_compressed_bytes(compressed: &[u8]) -> Result<Self> {
        // Streaming decoder with `windowLogMax=30` (1 GiB virtual) —
        // the cap zstd permits on 32-bit builds like wasm32. Shipping
        // blobs destined for wasm must be compressed with
        // `window_log <= 30` (the desktop encoder uses 31, which zstd
        // rejects on 32-bit targets — see `zstd_max_compress`).
        let mut decoder =
            zstd::stream::read::Decoder::new(compressed).context("zstd decoder init")?;
        decoder
            .window_log_max(30)
            .context("zstd window_log_max=30")?;
        let mut blob = Vec::with_capacity(compressed.len() * 4);
        std::io::Read::read_to_end(&mut decoder, &mut blob).context("zstd decompress")?;
        let dd = decode_native(&blob)?;
        tracing::info!(
            "loaded shipping datadir ({} → {} bytes)",
            compressed.len(),
            blob.len()
        );
        Ok(dd)
    }

    pub fn set_remote_base_url(&mut self, url: String) {
        self.remote_base_url = Some(url.trim_end_matches('/').to_owned());
    }

    pub fn remote_base_url(&self) -> Option<&str> {
        self.remote_base_url.as_deref()
    }

    pub fn mission_ref(&self, mission: &str) -> Option<&ShippingMissionRef> {
        self.missions.get(mission)
    }

    pub fn has_mission(&self, mission: &str) -> bool {
        self.missions.contains_key(mission) || self.levels.contains_key(mission)
    }

    pub fn source_file_path(&self, relative: &str) -> Result<PathBuf> {
        let source_dir = self.source_dir.as_ref().ok_or_else(|| {
            anyhow!("shipping manifest has no native source directory for {relative}")
        })?;
        Ok(source_dir.join(relative))
    }

    pub fn is_mission_loaded(&self, mission: &str) -> bool {
        self.loaded_missions
            .read()
            .expect("shipping mission lock poisoned")
            .contains_key(mission)
    }

    pub fn install_mission(&self, mission: &str, mut payload: ShippingMission) -> Result<()> {
        if !payload.levels.contains_key(mission) {
            return Err(anyhow!(
                "shipping payload for {mission} does not contain its level"
            ));
        }
        if let (Some(base), Some(bank)) = (self.sprite_bank.as_ref(), payload.sprite_bank.as_ref())
            && (base.signature != bank.signature || base.sprite_count != bank.sprite_count)
        {
            return Err(anyhow!(
                "shipping mission {mission} sprite bank is incompatible with boot dictionaries"
            ));
        }
        if let Some(bank) = payload.sprite_bank.as_mut() {
            bank.materialize_vq_chunks(&payload.rhs_files)
                .with_context(|| format!("materialize VQ sprite chunks for mission {mission}"))?;
        }
        let raw = std::mem::take(&mut payload.raw)
            .into_iter()
            .map(|(path, bytes)| (path, bytes.into()))
            .collect();
        payload
            .raw_bundle
            .set(Arc::new(raw))
            .map_err(|_| anyhow!("shipping mission {mission} raw bundle was already installed"))?;
        let mut loaded = self
            .loaded_missions
            .write()
            .expect("shipping mission lock poisoned");
        loaded.clear();
        loaded.insert(mission.to_owned(), Arc::new(payload));
        drop(loaded);
        *self
            .active_mission
            .write()
            .expect("shipping active mission lock poisoned") = None;
        self.activate_mission(mission)?;
        Ok(())
    }

    pub fn install_mission_parts(
        &self,
        mission: &str,
        parts: impl IntoIterator<Item = ShippingMission>,
    ) -> Result<()> {
        let mut merged = ShippingMission::default();
        for part in parts {
            merged.merge_from(part)?;
        }
        self.install_mission(mission, merged)
    }

    /// Synchronous native-file equivalent of the runtime's asynchronous
    /// mission loader. Developer tools use this when they open a converted
    /// datadir directly rather than entering the game session boundary.
    pub fn load_mission_from_source(&self, mission: &str) -> Result<()> {
        if self.is_mission_loaded(mission) {
            return self.activate_mission(mission);
        }
        let reference = self
            .mission_ref(mission)
            .ok_or_else(|| anyhow!("shipping datadir does not contain mission {mission}"))?;
        let mut merged = ShippingMission::default();
        for file in &reference.files {
            let path = self.source_file_path(file)?;
            let compressed = robin_util::asset_fs::read(&path)
                .with_context(|| format!("read {}", path.display()))?;
            merged.merge_part(
                decode_mission_compressed(&compressed)
                    .with_context(|| format!("decode {}", path.display()))?,
            )?;
        }
        self.install_mission_parts(mission, std::iter::once(merged))
    }

    pub fn activate_mission(&self, mission: &str) -> Result<()> {
        if self.active_mission_name().as_deref() == Some(mission) {
            return Ok(());
        }
        let payload = self
            .loaded_mission(mission)
            .ok_or_else(|| anyhow!("shipping mission {mission} has not been loaded"))?;
        let raw = payload
            .raw_bundle
            .get()
            .cloned()
            .ok_or_else(|| anyhow!("shipping mission {mission} has no installed raw bundle"))?;
        let raw_files = raw.len();
        let rhs_files = payload.rhs_files.len();
        robin_util::asset_fs::global()
            .replace_active_bundle(raw.clone())
            .context("mount shipping mission assets")?;
        if let Some(first_path) = raw.keys().next()
            && !robin_util::asset_fs::global()
                .try_exists(first_path)
                .with_context(|| format!("probe mounted shipping asset {first_path}"))?
        {
            return Err(anyhow!(
                "shipping mission {mission} mounted {raw_files} raw assets, but {first_path} is not visible"
            ));
        }
        robin_engine::sprite_script::replace_shipping_rhs(
            payload
                .rhs_files
                .iter()
                .map(|(path, rhs)| (path.as_str(), rhs.signature, rhs.profiles.as_slice())),
        );
        *self
            .active_mission
            .write()
            .expect("shipping active mission lock poisoned") = Some(mission.to_owned());
        tracing::info!(mission, raw_files, rhs_files, "activated shipping mission");
        Ok(())
    }

    pub fn loaded_mission(&self, mission: &str) -> Option<Arc<ShippingMission>> {
        self.loaded_missions
            .read()
            .expect("shipping mission lock poisoned")
            .get(mission)
            .cloned()
    }

    pub fn loaded_mission_count(&self) -> usize {
        self.loaded_missions
            .read()
            .expect("shipping mission lock poisoned")
            .len()
    }

    pub fn loaded_level(&self, mission: &str) -> Option<LoadedLevel> {
        self.loaded_mission(mission)
            .and_then(|payload| payload.levels.get(mission).cloned())
            .or_else(|| self.levels.get(mission).cloned())
    }

    pub fn mission_scripts(&self, mission: &str) -> BTreeMap<String, ScbFile> {
        self.loaded_mission(mission)
            .map(|payload| payload.scripts.clone())
            .unwrap_or_else(|| self.scripts.clone())
    }

    pub fn with_active_sprite_bank<R>(
        &self,
        use_bank: impl FnOnce(&ShippingSpriteBank, &[FrameDictionary]) -> R,
    ) -> Option<R> {
        let active = self
            .active_mission
            .read()
            .expect("shipping active mission lock poisoned")
            .clone();
        let loaded = active
            .as_deref()
            .and_then(|mission| self.loaded_mission(mission));
        let bank = loaded
            .as_ref()
            .and_then(|mission| mission.sprite_bank.as_ref())
            .or(self.sprite_bank.as_ref())?;
        let dictionaries = if bank.dictionaries.is_empty() {
            &self.sprite_bank.as_ref()?.dictionaries
        } else {
            &bank.dictionaries
        };
        Some(use_bank(bank, dictionaries))
    }

    pub fn active_mission_name(&self) -> Option<String> {
        self.active_mission
            .read()
            .expect("shipping active mission lock poisoned")
            .clone()
    }

    /// Publish the exact speech-profile closure selected at the asynchronous
    /// mission boundary. Process-wide audio caches use this instead of
    /// scanning every CPF actor and warning for intentionally unmounted data.
    pub fn set_active_exclamation_ids(&self, ids: BTreeSet<u32>) {
        *self
            .active_exclamation_ids
            .write()
            .expect("shipping active exclamation lock poisoned") = ids;
    }

    pub fn active_exclamation_ids(&self) -> Vec<u32> {
        self.active_exclamation_ids
            .read()
            .expect("shipping active exclamation lock poisoned")
            .iter()
            .copied()
            .collect()
    }

    /// Return the source-authoritative duration for boot or active-mission
    /// audio. Web artifacts use `.opus` keys even though legacy metadata asks
    /// for `.wav` or `.ogg`, so resolution includes that target extension.
    pub fn active_audio_duration_ms(&self, path: &Path) -> Option<u32> {
        self.active_audio_metadata(path)
            .map(|(_, duration)| duration)
    }

    /// Return encoded byte size and source duration without copying the VFS
    /// asset. The wasm sound cache only needs this bookkeeping because Web
    /// Audio owns both decoding and PCM playback storage.
    pub fn active_audio_metadata(&self, path: &Path) -> Option<(u32, u32)> {
        if let Some(asset) = self.find_audio_asset(path) {
            return Some((asset.encoded_size, asset.duration_ms));
        }
        let key = robin_util::asset_fs::bundle_key(path);
        let opus = Path::new(&key)
            .with_extension("opus")
            .to_string_lossy()
            .replace('\\', "/");
        let mission = self
            .active_mission_name()
            .and_then(|mission| self.loaded_mission(&mission))
            .and_then(|payload| {
                let duration = payload
                    .audio_durations_ms
                    .get(&key)
                    .or_else(|| payload.audio_durations_ms.get(&opus))
                    .copied()?;
                let bytes = payload.raw_bundle.get()?.get(&key).or_else(|| {
                    payload
                        .raw_bundle
                        .get()
                        .and_then(|bundle| bundle.get(&opus))
                })?;
                Some((u32::try_from(bytes.len()).ok()?, duration))
            });
        mission.or_else(|| {
            let duration = self
                .audio_durations_ms
                .get(&key)
                .or_else(|| self.audio_durations_ms.get(&opus))
                .copied()?;
            let bytes = self.raw_asset(&key).or_else(|| self.raw_asset(&opus))?;
            Some((u32::try_from(bytes.len()).ok()?, duration))
        })
    }

    /// Resolve legacy engine paths to a standalone browser audio asset.
    ///
    /// Callers may supply source extensions, bare sound names, paths relative
    /// to `Sounds/Exclamations`, or native absolute paths containing a `Data`
    /// component. All aliases resolve to the one catalog entry and therefore
    /// the same content URL/browser decode cache entry.
    pub fn remote_audio_asset(&self, path: &Path) -> Option<RemoteAudioAsset> {
        let asset = self.find_audio_asset(path)?;
        let base = self.remote_base_url.as_deref()?;
        Some(RemoteAudioAsset {
            url: format!("{}/{}", base.trim_end_matches('/'), asset.file),
            encoded_size: asset.encoded_size,
            duration_ms: asset.duration_ms,
            bundle_offset: asset.bundle_offset,
        })
    }

    /// Iterate every catalog key (normalized logical Opus path). Used by the
    /// browser's background audio prefetch to enumerate what exists.
    pub fn audio_catalog_keys(&self) -> impl Iterator<Item = &str> {
        self.audio_assets.keys().map(String::as_str)
    }

    /// Catalog keys the ACTIVE mission's payloads reference (its dialogue,
    /// required actor voices, music, ambience): the prefetch-first set.
    pub fn active_audio_keys(&self) -> Vec<String> {
        let Some(mission) = self.active_mission_name() else {
            return Vec::new();
        };
        let Some(payload) = self.loaded_mission(&mission) else {
            return Vec::new();
        };
        payload
            .audio_durations_ms
            .keys()
            .filter(|key| self.audio_assets.contains_key(*key))
            .cloned()
            .collect()
    }

    fn find_audio_asset(&self, path: &Path) -> Option<&ShippingAudioAsset> {
        audio_lookup_keys(path)
            .into_iter()
            .find_map(|key| self.audio_assets.get(&key))
    }

    /// Borrow one boot asset whether installation has moved it into the VFS
    /// shared-byte bundle or this manifest is still in converter/tool form.
    pub fn raw_asset(&self, key: &str) -> Option<&[u8]> {
        self.raw.get(key).map(Vec::as_slice).or_else(|| {
            self.boot_raw_bundle
                .get()
                .and_then(|bundle| bundle.get(key))
                .map(|bytes| bytes.as_ref())
        })
    }
}

/// English substitution is deliberately limited to optional recorded media.
/// All matching uses canonical shipping keys (relative to `Data/`).
pub fn is_optional_english_fallback_key(path: &str) -> bool {
    let key = canonical_shipping_asset_key(path);
    key == "sounds/exclamations"
        || key.starts_with("sounds/exclamations/")
        || key == "cinematics"
        || key.starts_with("cinematics/")
}

/// Text resources are required to come from the selected locale. They never
/// fall through to the compatibility/default maps of a shipping manifest.
pub fn is_required_locale_key(path: &str) -> bool {
    let key = canonical_shipping_asset_key(path);
    key == "text" || key.starts_with("text/") || key == "interface/start.sxt"
}

/// Locale selection is presentation-only. Even if a converted locale source
/// contains a complete Data tree, it must never replace simulation inputs such
/// as levels, scripts, or gameplay profile data.
pub fn is_locale_overlay_key(path: &str) -> bool {
    let key = canonical_shipping_asset_key(path);
    key == "text"
        || key.starts_with("text/")
        || key == "interface"
        || key.starts_with("interface/")
        || is_optional_english_fallback_key(&key)
}

impl ShippingMission {
    /// Borrow an installed raw asset without copying its encoded bytes.
    pub fn raw_asset(&self, key: &str) -> Option<&[u8]> {
        self.raw.get(key).map(Vec::as_slice).or_else(|| {
            self.raw_bundle
                .get()
                .and_then(|bundle| bundle.get(key))
                .map(|bytes| bytes.as_ref())
        })
    }

    /// Move-merge one independently decoded dependency into this payload.
    /// Loaders use this incrementally so compressed/decoded part shells can be
    /// released as soon as each bounded fetch completes.
    pub fn merge_part(&mut self, source: Self) -> Result<()> {
        self.merge_from(source)
    }

    fn merge_from(&mut self, mut source: Self) -> Result<()> {
        merge_unique_owned(&mut self.levels, source.levels, "level")?;
        merge_unique_owned(&mut self.scripts, source.scripts, "script")?;
        merge_unique_owned(&mut self.rhs_files, source.rhs_files, "RHS")?;
        merge_unique_owned(&mut self.raw, source.raw, "raw asset")?;
        merge_unique_owned(
            &mut self.audio_durations_ms,
            source.audio_durations_ms,
            "audio duration",
        )?;
        let Some(mut source_bank) = source.sprite_bank.take() else {
            return Ok(());
        };
        let bank = self.sprite_bank.get_or_insert_with(|| ShippingSpriteBank {
            signature: source_bank.signature,
            dictionaries: std::mem::take(&mut source_bank.dictionaries),
            sprite_count: source_bank.sprite_count,
            sprites: Vec::new(),
            vq_chunks: Vec::new(),
        });
        if bank.signature != source_bank.signature || bank.sprite_count != source_bank.sprite_count
        {
            return Err(anyhow!("shipping sprite-bank parts are incompatible"));
        }
        if bank.dictionaries.is_empty() {
            bank.dictionaries = std::mem::take(&mut source_bank.dictionaries);
        } else if !source_bank.dictionaries.is_empty()
            && bitcode::encode(&bank.dictionaries) != bitcode::encode(&source_bank.dictionaries)
        {
            return Err(anyhow!("shipping sprite-bank dictionaries conflict"));
        }
        bank.vq_chunks.append(&mut source_bank.vq_chunks);
        for (index, sprite) in source_bank.sprites {
            if index >= bank.sprite_count {
                return Err(anyhow!(
                    "shipping sprite-bank part contains out-of-range sprite {index} (bank has {} slots)",
                    bank.sprite_count
                ));
            }
            match bank
                .sprites
                .binary_search_by_key(&index, |(index, _)| *index)
            {
                Ok(position) => {
                    let existing = &bank.sprites[position].1;
                    // The streaming wasm loader materializes VQ grids while
                    // later parts are still downloading, so `existing` may
                    // already carry a decoded grid whose incoming twin is
                    // still the empty-`packed_data` VQ placeholder. Compare
                    // with the materialized side blanked; any chunk that
                    // decodes this sprite again still proves grid equality
                    // in `apply_decoded_vq_chunk`. Native installs merge
                    // strictly before materialization, where this branch
                    // cannot trigger.
                    let blanked;
                    let comparable =
                        if !existing.packed_data.is_empty() && sprite.packed_data.is_empty() {
                            blanked = ShippingSprite {
                                packed_data: Arc::new(Vec::new()),
                                ..existing.clone()
                            };
                            &blanked
                        } else {
                            existing
                        };
                    if bitcode::encode(comparable) != bitcode::encode(&sprite) {
                        return Err(anyhow!(
                            "shipping sprite-bank parts conflict at sprite {index}"
                        ));
                    }
                }
                Err(position) => bank.sprites.insert(position, (index, sprite)),
            }
        }
        Ok(())
    }
}

fn merge_unique_owned<K, V>(dst: &mut BTreeMap<K, V>, src: BTreeMap<K, V>, kind: &str) -> Result<()>
where
    K: Ord + std::fmt::Debug,
{
    for (key, value) in src {
        if dst.contains_key(&key) {
            return Err(anyhow!("duplicate shipping {kind} key {key:?}"));
        }
        dst.insert(key, value);
    }
    Ok(())
}

fn audio_lookup_keys(path: &Path) -> Vec<String> {
    let mut raw = path.to_string_lossy().replace('\\', "/");
    while let Some(rest) = raw.strip_prefix("./") {
        raw = rest.to_owned();
    }
    let lowercase = raw.to_ascii_lowercase();
    let key = if let Some(index) = lowercase.find("/data/") {
        raw[index + "/data/".len()..].to_owned()
    } else if lowercase.starts_with("data/") {
        raw["data/".len()..].to_owned()
    } else {
        raw.trim_start_matches('/').to_owned()
    }
    .to_ascii_lowercase();

    let mut bases = vec![key.clone()];
    if let Some(rest) = key.strip_prefix("exclamations/") {
        bases.push(format!("sounds/exclamations/{rest}"));
    }
    if !key.starts_with("sounds/") && !key.starts_with("musics/") {
        bases.push(format!("sounds/{key}"));
        bases.push(format!("sounds/exclamations/{key}"));
    }

    let mut keys = Vec::with_capacity(bases.len() * 2);
    for base in bases {
        keys.push(base.clone());
        let opus = Path::new(&base)
            .with_extension("opus")
            .to_string_lossy()
            .replace('\\', "/");
        if opus != base {
            keys.push(opus);
        }
    }
    keys
}

// Schema v10: star-2 family topology — [`SpriteVqChunk`] gains `base2_rhs` /
// `base2_ids`, letting third-and-later family members code each tile against
// TWO already-decoded siblings (schema v9 introduced the per-chunk
// `sprite_codec` blobs and single-base cross-variant coding). Both the boot
// manifest and the mission chunk layout changed, so both magics advance —
// bitcode is not self-describing, and a versioned magic mismatch is the only
// thing standing between an old binary and a misparse. (The datadir magic is
// spelled `RHDDNA10` because the tag is exactly 8 bytes; the u32 version
// beside it is the authoritative number.)
//
// Schema v11 / mission v6: identical container layout, but the VQ blobs are
// coded with PPM exclusion disabled (`EXCL_SOURCE_CAP = 0` in
// `sprite_codec`) — a decode-speed/size trade (~+1.6% rhs bytes for
// -35..43% decode time). The constant is part of the bitstream contract, so
// chunks encoded either way are mutually undecodable and both magics
// advance.
//
// Schema v12 / mission v7: binary escape coding in the VQ codec. The
// hit-vs-escape decision at each PPM chain level is now one LZMA-style
// adaptive bit (11-bit probability per SEE bucket) instead of SEE-priced
// escape mass folded into the coding interval — this removes both per-level
// divisions from the escape path (another decode-speed/size trade; the
// container layout is unchanged, but the entropy bitstream is incompatible,
// so both magics advance).
//
// Datadir v13: `ShippingAudioAsset` gains `bundle_offset` — small audio
// assets ship concatenated into one bundle per logical group instead of
// thousands of tiny standalone files. Mission chunk layout is unchanged
// (the audio catalog lives only in `datadir.bin`), so only the datadir
// magic advances.
//
// Datadir v14: `ShippingDatadir::locales` adds explicit canonicalized
// multi-locale payloads. Mission payloads remain structurally unchanged.
const SHIPPING_DATADIR_MAGIC: [u8; 8] = *b"RHDDNA14";
const SHIPPING_MISSION_MAGIC: [u8; 8] = *b"RHMISN07";
pub const SHIPPING_DATADIR_VERSION: u32 = 14;
pub const SHIPPING_MISSION_VERSION: u32 = 7;

/// Encode the versioned native-bitcode payload stored inside `datadir.bin`.
pub fn encode_native(datadir: &ShippingDatadir) -> Vec<u8> {
    let payload = bitcode::encode(datadir);
    let mut encoded = Vec::with_capacity(12 + payload.len());
    encoded.extend_from_slice(&SHIPPING_DATADIR_MAGIC);
    encoded.extend_from_slice(&SHIPPING_DATADIR_VERSION.to_le_bytes());
    encoded.extend_from_slice(&payload);
    encoded
}

fn decode_native(encoded: &[u8]) -> Result<ShippingDatadir> {
    let Some((header, payload)) = encoded.split_at_checked(12) else {
        return Err(anyhow!(
            "shipping datadir is shorter than its native header"
        ));
    };
    if header[..8] != SHIPPING_DATADIR_MAGIC {
        return Err(anyhow!(
            "shipping datadir is not native format version {SHIPPING_DATADIR_VERSION}; regenerate datadir.bin"
        ));
    }
    let version = u32::from_le_bytes(header[8..12].try_into().expect("fixed header length"));
    if version != SHIPPING_DATADIR_VERSION {
        return Err(anyhow!(
            "unsupported shipping datadir version {version}; expected {SHIPPING_DATADIR_VERSION}"
        ));
    }
    bitcode::decode(payload).map_err(|error| anyhow!("native bitcode decode: {error:?}"))
}

pub fn encode_mission_native(mission: &ShippingMission) -> Vec<u8> {
    let payload = bitcode::encode(mission);
    let mut encoded = Vec::with_capacity(12 + payload.len());
    encoded.extend_from_slice(&SHIPPING_MISSION_MAGIC);
    encoded.extend_from_slice(&SHIPPING_MISSION_VERSION.to_le_bytes());
    encoded.extend_from_slice(&payload);
    encoded
}

pub fn decode_mission_compressed(compressed: &[u8]) -> Result<ShippingMission> {
    let blob = zstd_decompress(compressed)?;
    let Some((header, payload)) = blob.split_at_checked(12) else {
        return Err(anyhow!(
            "shipping mission payload is shorter than its header"
        ));
    };
    if header[..8] != SHIPPING_MISSION_MAGIC {
        return Err(anyhow!("shipping mission payload has invalid magic"));
    }
    let version = u32::from_le_bytes(header[8..12].try_into().expect("fixed header length"));
    if version != SHIPPING_MISSION_VERSION {
        return Err(anyhow!(
            "unsupported shipping mission version {version}; expected {SHIPPING_MISSION_VERSION}"
        ));
    }
    bitcode::decode(payload).map_err(|error| anyhow!("native bitcode mission decode: {error:?}"))
}

fn zstd_decompress(compressed: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = zstd::stream::read::Decoder::new(compressed).context("zstd decoder init")?;
    decoder
        .window_log_max(30)
        .context("zstd window_log_max=30")?;
    let mut blob = Vec::with_capacity(compressed.len() * 4);
    std::io::Read::read_to_end(&mut decoder, &mut blob).context("zstd decompress")?;
    Ok(blob)
}

/// zstd level 22 with adaptive windows capped at the native 31-bit maximum.
pub fn zstd_max_compress(bytes: &[u8]) -> Result<Vec<u8>> {
    zstd_compress_with_window(bytes, 31)
}

/// zstd level 22 with an adaptive `windowLog` capped by the caller (10..=31).
/// Pledging the input size lets zstd advertise only the window this frame can
/// actually use. Split RHS chunks consequently require at most about 16 MiB
/// instead of claiming a 1 GiB wasm decoder window, with effectively neutral
/// compressed size.
pub fn zstd_compress_with_window(bytes: &[u8], max_window_log: u32) -> Result<Vec<u8>> {
    use zstd::stream::raw::CParameter;
    use zstd::stream::write::Encoder;
    if !(10..=31).contains(&max_window_log) {
        return Err(anyhow!(
            "zstd maximum window_log must be in 10..=31, got {max_window_log}"
        ));
    }
    let content_window_log = usize::BITS - bytes.len().saturating_sub(1).leading_zeros();
    let window_log = content_window_log.clamp(10, max_window_log);
    let mut out = Vec::new();
    let mut enc = Encoder::new(&mut out, 22).context("zstd encoder")?;
    enc.set_pledged_src_size(Some(bytes.len() as u64))
        .context("zstd pledged source size")?;
    enc.set_parameter(CParameter::WindowLog(window_log))
        .with_context(|| format!("zstd window_log={window_log}"))?;
    enc.set_parameter(CParameter::EnableLongDistanceMatching(true))
        .context("zstd long=1")?;
    std::io::Write::write_all(&mut enc, bytes).context("zstd write")?;
    enc.finish().context("zstd finish")?;
    Ok(out)
}

/// Convenience: look for `<data_dir>/datadir.bin`. Returns `Ok(None)` if
/// the file isn't present (legacy datadir), `Ok(Some(_))` on success.
pub fn try_load(data_dir: &Path) -> Result<Option<ShippingDatadir>> {
    let path = data_dir.join("datadir.bin");
    match robin_util::asset_fs::read(&path) {
        Ok(compressed) => {
            let mut datadir = ShippingDatadir::from_compressed_bytes(&compressed)
                .with_context(|| format!("decode {}", path.display()))?;
            datadir.source_dir = Some(data_dir.to_path_buf());
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
    match vfs.read(&path) {
        Ok(compressed) => {
            let mut datadir = ShippingDatadir::from_compressed_bytes(&compressed)
                .with_context(|| format!("decode {}", path.display()))?;
            datadir.source_dir = Some(data_dir.to_path_buf());
            Ok(Some(datadir))
        }
        Err(robin_util::asset_fs::AssetError::NotFound(_)) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

// ---------------------------------------------------------------------------
//  Process-global accessor
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

impl ShippingAssets {
    pub fn install(
        mut datadir: Arc<ShippingDatadir>,
        vfs: Arc<robin_util::asset_fs::AssetVfs>,
    ) -> Result<Self> {
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
        datadir
            .boot_raw_bundle
            .set(raw.clone())
            .map_err(|_| anyhow!("shipping boot raw bundle was already installed"))?;
        vfs.mount_bundle_first(raw)
            .context("mount shipping raw asset bundle")?;
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

/// Install a shipping datadir as the process-wide instance so lower-level
/// loaders can consult it for pre-parsed data. Installation and VFS mount
/// failures are returned to the startup boundary.
pub fn install_global(dd: Arc<ShippingDatadir>) -> Result<Arc<ShippingDatadir>> {
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
        german
            .raw
            .insert("text/level.res".into(), vec![7, 8, 9]);
        datadir.locales.insert("de-DE".into(), german);

        let encoded = encode_native(&datadir);
        assert_eq!(&encoded[..8], b"RHDDNA14");
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

        let legacy_unversioned = bitcode::encode(&datadir);
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
        assert_eq!(first.get("text/dialogue.wav"), Some(&vec![4, 2]));
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
        assert_eq!(bundle.get("text/level.res"), Some(&vec![5]));
        assert!(!bundle.contains_key("interface/start.sxt"));
        assert_eq!(bundle.get("sounds/exclamations/robin.wav"), Some(&vec![3]));
        assert_eq!(bundle.get("cinematics/intro.ogg"), Some(&vec![4]));
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
        assert_eq!(&encoded[..8], b"RHMISN07");
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
        };
        let bank = |sprites| ShippingSpriteBank {
            signature: 42,
            dictionaries: Vec::new(),
            sprite_count: 2,
            sprites,
            vq_chunks: Vec::new(),
        };
        let mut merged = ShippingMission {
            sprite_bank: Some(bank(Vec::new())),
            ..ShippingMission::default()
        };
        merged
            .merge_from(ShippingMission {
                sprite_bank: Some(bank(vec![(0, sprite(10))])),
                ..ShippingMission::default()
            })
            .unwrap();
        merged
            .merge_from(ShippingMission {
                sprite_bank: Some(bank(vec![(1, sprite(20))])),
                ..ShippingMission::default()
            })
            .unwrap();

        let sprites = &merged.sprite_bank.unwrap().sprites;
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
        }
    }

    fn vq_sprite(packed: Vec<u16>) -> ShippingSprite {
        ShippingSprite {
            width: VQ_DIMS.0,
            height: VQ_DIMS.1,
            dictionary_index: 0,
            packed_data: Arc::new(packed),
        }
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
        ShippingMission {
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
            ..ShippingMission::default()
        }
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
        ShippingMission {
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
            ..ShippingMission::default()
        }
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
        ShippingMission {
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
            ..ShippingMission::default()
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
    fn shipping_installation_propagates_invalid_bundle_path() {
        let vfs = Arc::new(AssetVfs::new());
        let mut datadir = ShippingDatadir::default();
        datadir
            .raw
            .insert("../escape.dat".to_string(), b"bad".to_vec());

        let error = ShippingAssets::install(Arc::new(datadir), vfs).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("mount shipping raw asset bundle")
        );
    }
}
