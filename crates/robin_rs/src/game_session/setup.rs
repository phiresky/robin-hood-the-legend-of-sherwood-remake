//! Mission-startup helpers extracted from `game_session`:
//! audio bank loading, sound-duration tables, level/sprite-bank
//! initialization, sprite renderer setup, and the Kira audio backend
//! bootstrap.

use std::collections::BTreeMap;

use crate::audio_backend::KiraAudioBackend;
use crate::cursor::CursorRenderer;
use crate::game::Game;
use crate::host::Host;
use crate::hud_text::HudFonts;
use crate::input::ThreadedInput;
use crate::input_translator::{GameKey, InputTranslator};
use crate::main_entry::{current_mission_id, picture_to_surface};
use crate::markers::SelectionMarkRenderer;
use crate::mouse_trail::MouseTrailRenderer;
use crate::renderer::{Renderer, TRANSPARENT_COLOR_KEY_16};
use crate::sound::{NUM_CHANNELS, SoundMode};
use crate::titbit_renderer::TitbitRenderer;
use crate::ui_panel::{PortraitCache, load_localized_character_names};
use robin_assets::frame_holder as assets_frame_holder;
use robin_assets::res_descr as assets_res_descr;
use robin_assets::resource_manager::ResourceManager;
use robin_assets::scb as assets_scb;
use robin_engine::campaign::Campaign;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::coordinates::{
    ScreenSize, SpriteAnchor, SpriteFrameOffset, SpriteLocalPoint, SpriteSize,
};
use robin_engine::engine as engine_api;
use robin_engine::engine::{Engine, LevelAssets};
use robin_engine::minimap::HitMask;
use robin_engine::player_command::PlayerCommand;
use robin_engine::profiles as engine_profiles;
use robin_engine::profiles::MissionLocation;
use robin_engine::resource_ids;
use robin_engine::sbfile as engine_sbfile;
use robin_engine::scb as engine_scb;
use robin_engine::script_manager as engine_script_manager;
use robin_engine::sound::ExclamationGroup;
use robin_engine::sound_cache as engine_sound_cache;
use robin_engine::sprite_script::{NONANIMATION_END, SpriteInfo, SpriteScript, UNMAPPED};
use robin_engine::titbit::SpriteRow;

/// Wall-clock step timer for the mission setup phase.  Each [`step`] logs the
/// time since the previous step at debug level (info when it crossed
/// `SLOW_STEP_MS`), so a slow launch shows exactly which setup stage ate the
/// time on both native and wasm builds.
pub(crate) struct PhaseTimer {
    phase: &'static str,
    started: web_time::Instant,
    last: web_time::Instant,
}

impl PhaseTimer {
    const SLOW_STEP_MS: u64 = 50;

    pub(crate) fn new(phase: &'static str) -> Self {
        let now = web_time::Instant::now();
        Self {
            phase,
            started: now,
            last: now,
        }
    }

    /// Log the elapsed time of the step that just finished.
    pub(crate) fn step(&mut self, label: &str) {
        let now = web_time::Instant::now();
        let ms = now.duration_since(self.last).as_millis() as u64;
        self.last = now;
        if ms >= Self::SLOW_STEP_MS {
            tracing::info!(elapsed_ms = ms, "{}: {label}", self.phase);
        } else {
            tracing::debug!(elapsed_ms = ms, "{}: {label}", self.phase);
        }
    }

    /// Log the total time since construction.
    pub(crate) fn total(&self) {
        tracing::info!(
            elapsed_ms = self.started.elapsed().as_millis() as u64,
            "{}: total",
            self.phase
        );
    }
}

// Tail-phase loading targets share one monotonic schedule. Keeping these in
// one place prevents a slow earlier phase (notably map decompression) from
// advancing beyond a later phase's ceiling and making the loading bar stall.
// Referenced by the wasm synchronous map-decode branch and the
// monotonic-schedule test; native decodes the map on a worker thread
// without loading-bar status updates.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(super) const LOADING_MAP_DECODE_PROGRESS: f32 = 0.85;
pub(super) const LOADING_SPRITE_VARIANTS_PROGRESS: f32 = 0.88;
pub(super) const LOADING_AUDIO_PROGRESS: f32 = 0.91;
pub(super) const LOADING_DESCRIPTORS_PROGRESS: f32 = 0.95;
pub(super) const LOADING_HUD_FONTS_PROGRESS: f32 = 0.98;
pub(super) const LOADING_FINAL_PROGRESS: f32 = 1.0;

#[derive(Debug, serde::Deserialize)]
struct HackableRhsManifest {
    pixel_format: HackableRhsPixelFormat,
    profiles: Vec<HackableRhsProfile>,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum HackableRhsPixelFormat {
    Rgba,
    LegacyColorKeys,
}

#[derive(Debug, serde::Deserialize)]
struct HackableRhsProfile {
    name: String,
    width: f32,
    height: f32,
    center_x: f32,
    center_y: f32,
    rows: Vec<HackableRhsRow>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CustomMissionTextPatch {
    #[serde(default)]
    popup_texts: BTreeMap<usize, String>,
    #[serde(default)]
    short_briefings: BTreeMap<usize, String>,
    #[serde(default)]
    dialogues: BTreeMap<usize, Vec<String>>,
}

#[derive(Debug, serde::Deserialize)]
struct HackableRhsRow {
    action_id: u16,
    action_done: u16,
    average_speed: f32,
    #[serde(default, rename = "direction")]
    _direction: u16,
    hotspot_x: f32,
    hotspot_y: f32,
    path: String,
    frames: Vec<HackableRhsFrame>,
}

#[derive(Debug, serde::Deserialize)]
struct HackableRhsFrame {
    file: String,
    delay: u16,
    distance: u16,
    offset_x: f32,
    offset_y: f32,
    sound_id: u16,
}

const HACKABLE_RHS_CACHE_VERSION: u32 = 2;
// Retain the original filename so v1 caches can be repaired in place without
// decoding hundreds of thousands of source PNGs again.
const HACKABLE_RHS_CACHE_FILE: &str = ".robin-rhs-cache-v1.zst";

#[derive(Debug, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode)]
struct HackableRhsCache {
    version: u32,
    manifest_hash: [u8; 32],
    sources: Vec<HackableRhsCacheSource>,
    frames: Vec<assets_frame_holder::RuntimeSprite>,
    profiles: Vec<HackableRhsCacheProfile>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode)]
struct HackableRhsCacheSource {
    relative_path: String,
    len: u64,
    modified_secs: u64,
    modified_nanos: u32,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode)]
struct HackableRhsCacheProfile {
    name: String,
    info: SpriteInfo,
}

fn overlay_roots_from_env() -> Vec<std::path::PathBuf> {
    engine_sbfile::SbFile::overlay_paths()
        .into_iter()
        .map(std::path::PathBuf::from)
        .collect()
}

fn decode_png_rgba(path: &std::path::Path) -> Result<(u16, u16, Vec<u8>), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    decode_png_rgba_bytes(&bytes, &path.display().to_string())
}

fn decode_png_rgba_bytes(bytes: &[u8], source: &str) -> Result<(u16, u16, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("decode {source}: {e}"))?;
    let mut buf = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| format!("unknown PNG output size for {source}"))?
    ];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("read frame {source}: {e}"))?;
    let data = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity(info.width as usize * info.height as usize * 4);
            for px in data.as_chunks::<3>().0 {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            out
        }
        png::ColorType::Grayscale => {
            let mut out = Vec::with_capacity(info.width as usize * info.height as usize * 4);
            for &value in data {
                out.extend_from_slice(&[value, value, value, 255]);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity(info.width as usize * info.height as usize * 4);
            for px in data.as_chunks::<2>().0 {
                out.extend_from_slice(&[px[0], px[0], px[0], px[1]]);
            }
            out
        }
        other => {
            return Err(format!(
                "PNG decoder did not expand color type {other:?} for {source}"
            ));
        }
    };
    Ok((info.width as u16, info.height as u16, rgba))
}

fn current_hackable_character_filenames(
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
) -> Option<std::collections::HashSet<String>> {
    let mission_index = campaign.current_mission_idx?;
    let mission = campaign.missions.get(mission_index)?.profile(profiles);
    let descriptor_path =
        robin_engine::level_data::hackable_level_descriptor_path(&mission.mission_filename);
    if !engine_sbfile::SbFile::exists(&descriptor_path) {
        return None;
    }
    let bytes = match engine_sbfile::SbFile::read_all(&descriptor_path) {
        Ok(bytes) => bytes,
        Err(status) => {
            tracing::warn!("Failed to read {descriptor_path}: SBFile error {status}");
            return Some(std::collections::HashSet::new());
        }
    };
    let descriptor: robin_engine::level_data::HackableLevelDescriptor =
        match serde_json::from_slice(&bytes) {
            Ok(descriptor) => descriptor,
            Err(error) => {
                tracing::warn!("Failed to parse {descriptor_path}: {error}");
                return Some(std::collections::HashSet::new());
            }
        };
    let mut filenames = std::collections::HashSet::new();
    for soldier in descriptor.soldiers {
        let profile = match soldier.profile {
            robin_engine::level_data::HackableSoldierProfile::Identifier(identifier) => profiles
                .soldier_idx_by_identifier(&identifier)
                .ok()
                .and_then(|index| profiles.get_soldier(index)),
            robin_engine::level_data::HackableSoldierProfile::LegacyIndex(index) => {
                profiles.get_soldier(index)
            }
        };
        if let Some(profile) = profile {
            filenames.insert(profile.filename.clone());
        }
    }
    Some(filenames)
}

fn hackable_manifest_hash(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest as _;
    sha2::Sha256::digest(bytes).into()
}

fn hackable_source_stamp(
    root: &std::path::Path,
    relative_path: &str,
) -> Result<HackableRhsCacheSource, String> {
    let path = root.join(relative_path);
    let metadata = std::fs::metadata(&path)
        .map_err(|error| format!("stat hackable sprite {}: {error}", path.display()))?;
    let modified = metadata
        .modified()
        .map_err(|error| format!("read mtime for {}: {error}", path.display()))?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("invalid mtime for {}: {error}", path.display()))?;
    Ok(HackableRhsCacheSource {
        relative_path: relative_path.to_owned(),
        len: metadata.len(),
        modified_secs: modified.as_secs(),
        modified_nanos: modified.subsec_nanos(),
    })
}

fn hackable_cache_sources_are_current(
    root: &std::path::Path,
    cache: &HackableRhsCache,
    manifest_hash: [u8; 32],
) -> bool {
    cache.manifest_hash == manifest_hash
        && cache.sources.iter().all(|source| {
            hackable_source_stamp(root, &source.relative_path).is_ok_and(|current| {
                current.len == source.len
                    && current.modified_secs == source.modified_secs
                    && current.modified_nanos == source.modified_nanos
            })
        })
}

fn read_hackable_cache(
    root: &std::path::Path,
    manifest_hash: [u8; 32],
) -> Option<HackableRhsCache> {
    let cache_path = root.join(HACKABLE_RHS_CACHE_FILE);
    let compressed = match std::fs::read(&cache_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::warn!("Failed to read {}: {error}", cache_path.display());
            return None;
        }
    };
    let encoded = match zstd::stream::decode_all(std::io::Cursor::new(compressed)) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!("Failed to decompress {}: {error}", cache_path.display());
            return None;
        }
    };
    let mut cache: HackableRhsCache = match bitcode::decode(&encoded) {
        Ok(cache) => cache,
        Err(error) => {
            tracing::warn!("Failed to decode {}: {error}", cache_path.display());
            return None;
        }
    };
    if !hackable_cache_sources_are_current(root, &cache, manifest_hash) {
        return None;
    }
    match cache.version {
        HACKABLE_RHS_CACHE_VERSION => Some(cache),
        1 => {
            // Version 1 eagerly installed walking fallbacks while reading
            // rows. A later explicit RunningUpright row therefore could not
            // replace the fallback and resolved to WalkingUpright. Rebuild
            // only the small action tables; packed pixels remain valid.
            for profile in &mut cache.profiles {
                profile.info.conversion = std::sync::Arc::new(hackable_animation_conversion(
                    profile.info.scripts.as_ref(),
                ));
            }
            cache.version = HACKABLE_RHS_CACHE_VERSION;
            if let Err(error) = write_hackable_cache(root, &cache) {
                tracing::warn!("Failed to upgrade {}: {error}", cache_path.display());
            }
            Some(cache)
        }
        version => {
            tracing::warn!(
                "Ignoring {} with unsupported cache version {version}",
                cache_path.display()
            );
            None
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn write_hackable_cache(root: &std::path::Path, cache: &HackableRhsCache) -> Result<(), String> {
    use std::io::Write as _;

    let encoded = bitcode::encode(cache);
    let compressed = zstd::stream::encode_all(std::io::Cursor::new(encoded), 3)
        .map_err(|error| format!("compress hackable sprite cache: {error}"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(root).map_err(|error| {
        format!(
            "create hackable sprite cache in {}: {error}",
            root.display()
        )
    })?;
    temporary
        .write_all(&compressed)
        .map_err(|error| format!("write hackable sprite cache in {}: {error}", root.display()))?;
    temporary
        .persist(root.join(HACKABLE_RHS_CACHE_FILE))
        .map_err(|error| {
            format!(
                "persist hackable sprite cache in {}: {error}",
                root.display()
            )
        })?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn write_hackable_cache(_root: &std::path::Path, _cache: &HackableRhsCache) -> Result<(), String> {
    Ok(())
}

fn hackable_animation_conversion(scripts: &[SpriteScript]) -> Vec<u16> {
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    for (row_index, script) in scripts.iter().enumerate() {
        if let Some(slot) = conversion.get_mut(script.action_id as usize)
            && *slot == UNMAPPED
        {
            *slot = row_index as u16;
        }
    }

    // Minimal hackable characters may only provide idle and walking loops.
    // Install fallbacks only after every authored action has claimed its own
    // slot, so a real run row always wins over the walking fallback.
    for (source, aliases) in [
        (3usize, &[0usize, 1, 2, 4, 8][..]),
        (6, &[5, 7, 9, 10, 11, 12][..]),
    ] {
        let source_row = conversion[source];
        if source_row == UNMAPPED {
            continue;
        }
        for &alias in aliases {
            if conversion[alias] == UNMAPPED {
                conversion[alias] = source_row;
            }
        }
    }
    conversion
}

fn build_hackable_cache(
    root: &std::path::Path,
    manifest_hash: [u8; 32],
    manifest: HackableRhsManifest,
) -> (HackableRhsCache, bool) {
    let mut frames = Vec::new();
    let mut sources = Vec::new();
    let mut local_frames = std::collections::HashMap::<String, u32>::new();
    let mut profiles = Vec::with_capacity(manifest.profiles.len());
    let mut complete = true;
    let legacy_color_keys = matches!(
        manifest.pixel_format,
        HackableRhsPixelFormat::LegacyColorKeys
    );

    for profile in manifest.profiles {
        let mut scripts = Vec::with_capacity(profile.rows.len());
        for row in profile.rows {
            let mut script = SpriteScript {
                action_id: row.action_id,
                action_done: row.action_done,
                average_speed: row.average_speed,
                hotspot: SpriteLocalPoint::new(row.hotspot_x, row.hotspot_y),
                ..SpriteScript::default()
            };
            for frame in row.frames {
                let relative_path = std::path::Path::new(&row.path)
                    .join(&frame.file)
                    .to_string_lossy()
                    .into_owned();
                let local_id = if let Some(local_id) = local_frames.get(&relative_path) {
                    *local_id
                } else {
                    let frame_path = root.join(&relative_path);
                    let (width, height, rgba) = match decode_png_rgba(&frame_path) {
                        Ok(decoded) => decoded,
                        Err(error) => {
                            tracing::warn!("{error}");
                            complete = false;
                            continue;
                        }
                    };
                    let source = match hackable_source_stamp(root, &relative_path) {
                        Ok(source) => source,
                        Err(error) => {
                            tracing::warn!("{error}");
                            complete = false;
                            continue;
                        }
                    };
                    let local_id = frames.len() as u32;
                    frames.push(assets_frame_holder::FrameHolder::pack_runtime_rgba_sprite(
                        width,
                        height,
                        &rgba,
                        legacy_color_keys,
                    ));
                    sources.push(source);
                    local_frames.insert(relative_path, local_id);
                    local_id
                };
                script.frame_ids.push(local_id);
                script.delays.push(frame.delay);
                script.distances.push(frame.distance);
                script
                    .offsets
                    .push(SpriteFrameOffset::new(frame.offset_x, frame.offset_y));
                script.sound_ids.push(frame.sound_id);
                script.sum_distance = script.sum_distance.saturating_add(frame.distance);
            }
            scripts.push(script);
        }
        let conversion = hackable_animation_conversion(&scripts);
        profiles.push(HackableRhsCacheProfile {
            name: profile.name,
            info: SpriteInfo {
                scripts: std::sync::Arc::new(scripts),
                conversion: std::sync::Arc::new(conversion),
                size: SpriteSize::new(profile.width, profile.height),
                center: SpriteAnchor::new(profile.center_x, profile.center_y),
            },
        });
    }

    (
        HackableRhsCache {
            version: HACKABLE_RHS_CACHE_VERSION,
            manifest_hash,
            sources,
            frames,
            profiles,
        },
        complete,
    )
}

fn install_hackable_cache(
    host: &mut Host,
    assets: &mut LevelAssets,
    filename: &str,
    cache: HackableRhsCache,
) -> Result<(), String> {
    let frame_ids: Vec<u32> = cache
        .frames
        .into_iter()
        .map(|frame| host.frame_holder_mut().append_runtime_sprite(frame))
        .collect();
    for profile in cache.profiles {
        let mut info = profile.info;
        let scripts = std::sync::Arc::make_mut(&mut info.scripts);
        for script in scripts {
            for frame_id in &mut script.frame_ids {
                *frame_id = *frame_ids.get(*frame_id as usize).ok_or_else(|| {
                    format!(
                        "hackable sprite cache {filename}/{} references missing local frame {}",
                        profile.name, *frame_id
                    )
                })?;
            }
        }
        let cache_key = format!("{filename}/{}", profile.name);
        assets.sprite_scriptor_mut().insert(cache_key.clone(), info);
        tracing::info!("Loaded hackable character profile {cache_key}");
    }
    Ok(())
}

fn preload_hackable_character_dirs(host: &mut Host, assets: &mut LevelAssets, campaign: &Campaign) {
    let mission_filenames = current_hackable_character_filenames(campaign, &assets.profile_manager);
    for root in overlay_roots_from_env() {
        let chars = root.join("Data/Characters");
        let mission_scoped = chars.join("mission-scoped.json").is_file();
        let Ok(entries) = std::fs::read_dir(&chars) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let Some(filename) = name.strip_suffix(".rhs.d") else {
                continue;
            };
            if mission_scoped
                && !mission_filenames
                    .as_ref()
                    .is_some_and(|filenames| filenames.contains(filename))
            {
                continue;
            }

            let manifest_path = path.join("manifest.json");
            let Ok(manifest_bytes) = std::fs::read(&manifest_path) else {
                continue;
            };
            let manifest_hash = hackable_manifest_hash(&manifest_bytes);
            if let Some(cache) = read_hackable_cache(&path, manifest_hash) {
                tracing::info!("Using compiled hackable sprite cache for {filename}");
                if let Err(error) = install_hackable_cache(host, assets, filename, cache) {
                    tracing::warn!("{error}");
                }
                continue;
            }

            let manifest: HackableRhsManifest = match serde_json::from_slice(&manifest_bytes) {
                Ok(manifest) => manifest,
                Err(error) => {
                    tracing::warn!("Failed to parse {}: {error}", manifest_path.display());
                    continue;
                }
            };
            let (cache, complete) = build_hackable_cache(&path, manifest_hash, manifest);
            if complete {
                if let Err(error) = write_hackable_cache(&path, &cache) {
                    tracing::warn!("Failed to cache hackable sprites for {filename}: {error}");
                } else {
                    tracing::info!("Compiled hackable sprite cache for {filename}");
                }
            } else {
                tracing::warn!(
                    "Hackable sprite source {filename} was incomplete; not writing a compiled cache"
                );
            }
            if let Err(error) = install_hackable_cache(host, assets, filename, cache) {
                tracing::warn!("{error}");
            }
        }
    }
}

/// Load mission-specific sound banks and switch to mission music.
///
/// Loads the FX / menu / exclamation caches, populates the music pool
/// from the mission profile, and switches the mixer to mission mode after
/// required script startup and before the loading screen closes. Pure
/// host-side work — reads
/// profile/sound metadata off the engine but does not mutate it.
pub(super) fn setup_mission_audio(
    host: &mut Host,
    backend: Option<&mut KiraAudioBackend>,
    engine: &Engine,
    assets: &mut LevelAssets,
    profiles: &engine_profiles::ProfileManager,
    location: MissionLocation,
    sound_dir: &str,
) {
    let mut timer = PhaseTimer::new("mission audio setup");
    let loader = crate::audio_backend::create_sample_loader(std::path::PathBuf::from(sound_dir));

    // FX bank, menu bank, and exclamation cache come pre-parsed from
    // the process-wide asset cache (warmed on a background thread at
    // startup); only the registration into this mission's sound cache
    // happens here.
    let asset_cache = crate::process_asset_cache::get_or_build(host.shipping.as_deref(), profiles);
    timer.step("process asset cache");
    if let Some(elements) = asset_cache.fx_bank.as_ref() {
        host.audio.sound.sound_cache.initialize_fx_cache(elements);
        tracing::info!("Loaded FX bank: {} elements", elements.len());
    }
    if let Some(entries) = asset_cache.menu_bank.as_ref() {
        host.audio.sound.sound_cache.initialize_menu_cache(entries);
        tracing::info!("Loaded menu sound bank: {} entries", entries.len());
    }
    for resolved in &asset_cache.exclamations {
        host.audio
            .sound
            .sound_cache
            .initialize_exclamations_for_profile(resolved);
    }

    // Initialize music pools from the mission profile.
    let campaign = engine.campaign();
    if let Some(idx) = campaign.current_mission_idx {
        let prof = campaign.missions[idx].profile(profiles);
        host.audio.sound.sound_cache.initialize_music(
            &prof.green_music,
            &prof.yellow_music,
            &prof.red_music,
        );
    }

    // Populate the sound-source cache from the IDs collected during
    // proto-level loading, then finalize.  Without this the source
    // cache is empty and looped/ambient sources cannot play.
    host.audio
        .sound
        .sound_cache
        .initialize_sound_source_cache(&assets.sound_source_required_ids);
    host.audio
        .sound
        .sound_cache
        .finalize_sound_sources(&engine.sound_sim().sources);
    timer.step("bank registration");
    let installed_languages = host
        .application_context
        .installed_languages()
        .unwrap_or_else(|error| panic!("speech timing lost localization service: {error}"));
    let canonical_voice_pack = select_canonical_voice_pack(
        installed_languages,
        host.transport.speech_timing_locale.as_deref(),
        host.transport.net.is_some(),
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let (canonical_base, canonical_loader, canonical_speech_cache) = match canonical_voice_pack {
        Some(pack) => {
            let base = if pack.data_root.is_empty() {
                std::path::PathBuf::from(format!(
                    "__shipping_language_pack__/{}/{}",
                    pack.locale, sound_dir
                ))
            } else {
                std::path::PathBuf::from(&pack.data_root).join(sound_dir)
            };
            let loader = crate::audio_backend::create_language_pack_sample_loader(
                std::path::PathBuf::from(sound_dir),
                pack.clone(),
                host.shipping.clone(),
            );
            let mut cache = engine_sound_cache::SoundCache::new();
            let canonical_exclamations =
                crate::process_asset_cache::build_exclamations_for_language(
                    &pack,
                    host.shipping.as_deref(),
                    profiles,
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "authoritative speech timing pack `{}` is incomplete: {error}",
                        pack.locale
                    )
                });
            for exclamations in canonical_exclamations {
                cache.initialize_exclamations_for_profile(&exclamations);
            }
            (base, loader, cache.speech_cache)
        }
        None => {
            if host.transport.net.is_some() {
                tracing::info!("authoritative multiplayer speech timing uses base Data/Sounds");
            } else {
                tracing::warn!(
                    "No validated voice pack is installed; speech timing uses the active/base audio data"
                );
            }
            (
                std::path::PathBuf::from(sound_dir),
                crate::audio_backend::create_sample_loader(std::path::PathBuf::from(sound_dir)),
                host.audio.sound.sound_cache.speech_cache.clone(),
            )
        }
    };
    populate_sound_duration_tables(
        host,
        assets,
        profiles,
        sound_dir,
        &canonical_loader,
        &canonical_base,
        &canonical_speech_cache,
    );
    timer.step("sound duration tables");

    // Per-entry sample validation block.  When
    // `gGlobalOptions.bCheckSoundData` is set, the engine validates
    // each sample as it's added; we run the equivalent load+unload
    // sweep here because `add_entry` doesn't have a loader at insert
    // time. The resulting `data_check_succeeded` flag is consulted by
    // `SoundManager::activate` (fatal panic on miss).
    let check = host.application_context.options().check_sound_data;
    if check {
        host.audio.sound.sound_cache.validate_data(&loader);
        timer.step("check_sound_data validation");
    }

    if let Some(backend) = backend {
        host.audio.sound.activate(
            location == MissionLocation::Sherwood,
            &engine.sound_sim().sources,
        );
        // Switch from menu music to mission music during the final loading
        // stage. SetMode(Mission) halts the menu stream and
        // re-raises load_music so mission music starts from the pool.
        host.audio.sound.set_mode(SoundMode::Mission, backend);
        timer.step("mixer activation");
    }
    timer.total();
}

fn select_canonical_voice_pack(
    installed_languages: Vec<crate::localization::LanguagePack>,
    authoritative_locale: Option<&str>,
    multiplayer: bool,
) -> Result<Option<crate::localization::LanguagePack>, String> {
    if let Some(authoritative_locale) = authoritative_locale {
        return installed_languages
            .into_iter()
            .find(|pack| pack.has_voice && pack.locale == authoritative_locale)
            .map(Some)
            .ok_or_else(|| {
                format!(
                    "authoritative multiplayer speech timing pack `{authoritative_locale}` is not installed or has no voice data"
                )
            });
    }
    if multiplayer {
        // Welcome explicitly selected the base installation's Data/Sounds.
        // Do not auto-select a presentation language independently per peer.
        return Ok(None);
    }
    Ok(installed_languages
        .into_iter()
        .filter(|pack| pack.has_voice)
        .min_by_key(|pack| (pack.locale != "en-US", pack.locale.clone())))
}

fn populate_sound_duration_tables(
    host: &Host,
    assets: &mut LevelAssets,
    profiles: &engine_profiles::ProfileManager,
    sound_dir: &str,
    canonical_speech_loader: &engine_sound_cache::SampleLoader,
    canonical_speech_dir: &std::path::Path,
    canonical_speech_cache: &engine_sound_cache::IndexedCache,
) {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Arc;

    // Durations only depend on the sample files, so they're served from
    // the persistent one-file cache; the probe (a full sample read) is
    // only hit for files the cache hasn't seen, and those probes run in
    // parallel on native.
    let mut duration_cache = crate::audio_duration_cache::AudioDurationCache::load();
    let sample_base =
        crate::audio_duration_cache::SampleResolver::new(std::path::Path::new(sound_dir));
    let sound_base = std::path::PathBuf::from(sound_dir);
    let probe = move |name: &str| crate::audio_backend::sample_duration_ms(&sound_base, name);

    // Ordinary sound-source timing remains tied to the active/base sound
    // directory. Logical speech timing is resolved independently below from
    // the canonical pack selected by the multiplayer handshake.
    let needed = host
        .audio
        .sound
        .sound_cache
        .source_cache
        .entries
        .values()
        .map(|entry| entry.file_name.clone());
    let durations = duration_cache.durations_for(&sample_base, needed, &probe);
    let canonical_speech_base =
        crate::audio_duration_cache::SampleResolver::new(canonical_speech_dir);

    fn frames_from_ms(ms: u32) -> u32 {
        ((ms.saturating_add(39)) / 40).max(1)
    }

    let mut groups_by_profile: BTreeMap<u32, BTreeSet<ExclamationGroup>> = BTreeMap::new();
    for profile in &profiles.characters {
        if profile.exclamation_id != 0 {
            groups_by_profile
                .entry(profile.exclamation_id)
                .or_default()
                .insert(ExclamationGroup::Pc);
        }
    }
    for profile in &profiles.soldiers {
        if profile.exclamation_id != 0 {
            let entry = groups_by_profile.entry(profile.exclamation_id).or_default();
            // AI `Say` uses the civilian bank for ordinary soldier
            // remarks, while direct hit/death speech still tags
            // soldiers distinctly.
            entry.insert(ExclamationGroup::Civilian);
            entry.insert(ExclamationGroup::Soldier);
            if profile.vip {
                entry.insert(ExclamationGroup::Vip);
            }
        }
    }
    for profile in &profiles.civilians {
        if profile.exclamation_id != 0 {
            let entry = groups_by_profile.entry(profile.exclamation_id).or_default();
            entry.insert(ExclamationGroup::Civilian);
            if profile.civilian_type == engine_profiles::CivilianType::Vip {
                entry.insert(ExclamationGroup::Vip);
            }
        }
    }

    let mut exclamation_durations = BTreeMap::new();
    for (&group_id, group) in &canonical_speech_cache.groups {
        let profile_prefix = group_id & 0xFFFF_0000;
        let exclamation_id = (group_id & 0xFFFF) as u16;
        let duration_ms = group
            .entry_indices
            .iter()
            .map(|&index| {
                let entry = canonical_speech_cache.entries.get(index).unwrap_or_else(|| {
                    panic!(
                        "authoritative speech group {group_id} references missing cache entry {index}"
                    )
                });
                duration_cache
                    .duration_ms(
                        &canonical_speech_base,
                        &entry.file_name,
                        canonical_speech_loader,
                    )
                    .unwrap_or_else(|| {
                        panic!(
                            "authoritative speech sample `{}` for group {group_id} is unavailable",
                            entry.file_name
                        )
                    })
            })
            .max()
            .unwrap_or_else(|| panic!("authoritative speech group {group_id} has no samples"));
        let frames = frames_from_ms(duration_ms);
        for (&profile_id, groups) in &groups_by_profile {
            if profile_id & 0xFFFF_0000 != profile_prefix {
                continue;
            }
            for &group_kind in groups {
                exclamation_durations.insert((group_kind, profile_id, exclamation_id), frames);
            }
        }
    }

    let mut source_durations = BTreeMap::new();
    for (&sample_id, entry) in &host.audio.sound.sound_cache.source_cache.entries {
        if let Some(duration_ms) = durations.get(&entry.file_name).copied() {
            source_durations.insert(sample_id, frames_from_ms(duration_ms));
        }
    }
    duration_cache.save_if_dirty();

    tracing::info!(
        exclamations = exclamation_durations.len(),
        sources = source_durations.len(),
        "Populated deterministic sound duration tables"
    );
    assets.exclamation_durations = Arc::new(exclamation_durations);
    assets.source_durations = Arc::new(source_durations);
}

/// Frontend-only resources loaded while the interactive loading screen is
/// still visible.
///
/// This process data is consumed by `InteractiveFrontend` and deliberately is
/// not serialized as part of deterministic mission state.
pub(super) struct LoadedInteractiveResources {
    pub(super) level_descriptors: Option<assets_res_descr::LevelDescriptors>,
    pub(super) hud_fonts: Option<HudFonts>,
}

fn descriptor_mission_id(campaign: &Campaign, profiles: &engine_profiles::ProfileManager) -> u32 {
    let current_id = current_mission_id(campaign, profiles);
    let current_profile = campaign
        .current_mission_idx
        .and_then(|index| campaign.missions.get(index))
        .map(|mission| mission.profile(profiles))
        .expect("descriptor lookup requires a current campaign mission");
    let patch_path = format!(
        "Data/Levels/{}.characters.patch.json",
        current_profile.mission_filename
    );
    if !engine_sbfile::SbFile::exists(&patch_path) {
        return current_id;
    }
    let bytes = engine_sbfile::SbFile::read_all(&patch_path)
        .unwrap_or_else(|status| panic!("read descriptor alias patch {patch_path}: {status}"));
    let patch: serde_json::Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("parse descriptor alias patch {patch_path}: {error}"));
    let alias = patch
        .get("descriptor_mission")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("{patch_path} is missing descriptor_mission"));
    let mut matches = profiles
        .missions
        .iter()
        .filter(|profile| profile.mission_filename == alias);
    let id = matches
        .next()
        .unwrap_or_else(|| panic!("{patch_path} descriptor mission {alias:?} does not exist"))
        .id;
    assert!(
        matches.next().is_none(),
        "{patch_path} descriptor mission {alias:?} is ambiguous"
    );
    id
}

fn apply_custom_mission_text_patch(
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
    descriptors: &mut assets_res_descr::LevelDescriptors,
) {
    let mission_filename = campaign
        .current_mission_idx
        .and_then(|index| campaign.missions.get(index))
        .map(|mission| mission.profile(profiles).mission_filename.as_str())
        .expect("custom mission text lookup requires a current campaign mission");
    let path = format!("Data/Levels/{mission_filename}.text.patch.json");
    if !engine_sbfile::SbFile::exists(&path) {
        return;
    }
    let bytes = engine_sbfile::SbFile::read_all(&path)
        .unwrap_or_else(|status| panic!("read custom mission text patch {path}: {status}"));
    let patch: CustomMissionTextPatch = serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("parse custom mission text patch {path}: {error}"));
    let install = |target: &mut Vec<Option<String>>, values: BTreeMap<usize, String>| {
        if let Some(max_index) = values.keys().next_back().copied() {
            target.resize(target.len().max(max_index + 1), None);
        }
        for (index, text) in values {
            target[index] = Some(text);
        }
    };
    install(&mut descriptors.custom_popup_texts, patch.popup_texts);
    install(
        &mut descriptors.custom_short_briefings,
        patch.short_briefings,
    );
    if let Some(max_index) = patch.dialogues.keys().next_back().copied() {
        descriptors.custom_dialogue_texts.resize(
            descriptors.custom_dialogue_texts.len().max(max_index + 1),
            None,
        );
    }
    for (index, sentences) in patch.dialogues {
        let expected = descriptors
            .dialogues
            .get(index)
            .unwrap_or_else(|| panic!("{path} dialogue {index} has no base descriptor"))
            .portrait_ids
            .len();
        assert_eq!(
            sentences.len(),
            expected,
            "{path} dialogue {index} requires {expected} sentences"
        );
        descriptors.custom_dialogue_texts[index] = Some(sentences);
    }
    tracing::info!("Applied custom mission text patch {path}");
}

/// Process-only resources acquired before the deterministic engine is built.
///
/// The text and interface archives provide both construction metadata and the
/// later interactive frontend caches. The optional backend owns the native
/// audio device. None of these values belongs in an engine snapshot.
pub(super) struct MissionProcessResources {
    pub(super) text: ResourceManager,
    /// The `DEFAULT.RES` interface archive; `Some` until frontend assembly
    /// consumes it via [`Self::take_interface`].
    interface: Option<PendingInterfaceResources>,
    pub(super) audio_backend: Option<KiraAudioBackend>,
}

/// The interface archive, possibly off on a worker getting its JXL pictures
/// eagerly decoded while the level loads (see
/// [`MissionProcessResources::start_interface_decode`]). The joined pair is
/// `(cursor, menu)` — two identical fully-decoded `DEFAULT.RES` views, one
/// for the mission sprite caches and one owned by the in-game menus.
enum PendingInterfaceResources {
    Ready {
        cursor: ResourceManager,
    },
    #[cfg(not(target_arch = "wasm32"))]
    Thread(std::thread::JoinHandle<(ResourceManager, ResourceManager)>),
    #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
    Pool(robin_assets::wasm_threads::PoolReceiver<(ResourceManager, ResourceManager)>),
}

/// Worker-side body of the interface pre-decode: decode every encoded (JXL)
/// picture once, then duplicate the decoded manager for the menu owner —
/// a memcpy of decoded pixels, far cheaper than a second decode pass.
fn decode_interface_managers(mut cursor: ResourceManager) -> (ResourceManager, ResourceManager) {
    let started = web_time::Instant::now();
    let decoded = cursor.decode_all_encoded_pictures();
    let menu = cursor.duplicate();
    tracing::info!(
        resources = decoded,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "interface pictures pre-decoded off the loading path"
    );
    (cursor, menu)
}

/// Engine-construction resources for true headless mode. This owner contains
/// no renderer, input device, HUD, menu, font, or native audio backend.
pub(super) struct HeadlessEngineResources {
    pub(super) text: ResourceManager,
    cursor: ResourceManager,
}

impl HeadlessEngineResources {
    pub(super) fn load(host: &Host) -> Self {
        let mut text = ResourceManager::new();
        if let Err(error) =
            text.attach_or_from_shipping("Data/Text/Level.res", host.shipping.as_deref())
        {
            tracing::warn!("Failed to load text resource file: {error}");
        }

        let mut cursor = ResourceManager::new();
        if let Err(error) =
            cursor.attach_or_from_shipping("Data/Interface/DEFAULT.RES", host.shipping.as_deref())
        {
            tracing::warn!("Failed to load cursor resource file: {error}");
        }
        Self { text, cursor }
    }

    pub(super) fn engine_setup_resources(
        &mut self,
        host: &mut Host,
    ) -> (
        Option<engine_api::GroundMarkSpriteData>,
        Vec<u16>,
        Option<engine_api::MinimapWidgetSetup>,
    ) {
        let ground_mark_sprite = extract_ground_mark_sprite_data(&mut self.cursor);
        if let Some(data) = ground_mark_sprite.as_ref() {
            host.install_trajectory_ground_mark_sprite(data);
        }
        (
            ground_mark_sprite,
            extract_titbit_row_frame_counts(&mut self.cursor),
            extract_minimap_widget_setup(&mut self.cursor),
        )
    }
}

impl MissionProcessResources {
    pub(super) fn load(host: &mut Host, game: &Game) -> Self {
        let audio_backend = init_audio_backend(host, game);

        let mut text = ResourceManager::new();
        if let Err(error) =
            text.attach_or_from_shipping("Data/Text/Level.res", host.shipping.as_deref())
        {
            tracing::warn!("Failed to load text resource file: {error}");
        }

        let mut cursor = ResourceManager::new();
        if let Err(error) =
            cursor.attach_or_from_shipping("Data/Interface/DEFAULT.RES", host.shipping.as_deref())
        {
            tracing::warn!("Failed to load cursor resource file: {error}");
        }

        Self {
            text,
            interface: Some(PendingInterfaceResources::Ready { cursor }),
            audio_backend,
        }
    }

    /// The interface archive for pre-engine metadata extraction. Panics if
    /// the pre-decode was already dispatched — extraction must come first.
    fn interface_cursor_mut(&mut self) -> &mut ResourceManager {
        match self.interface.as_mut() {
            Some(PendingInterfaceResources::Ready { cursor }) => cursor,
            _ => panic!("interface archive already dispatched to the pre-decode worker"),
        }
    }

    /// Move the interface archive onto a worker that eagerly decodes every
    /// encoded (JXL) picture, so frontend assembly finds them ready instead
    /// of decoding hundreds of interface images on the loading path. No-op
    /// when no worker can run it (single-threaded wasm) — the lazy per-
    /// resource decode then behaves exactly as before.
    pub(super) fn start_interface_decode(&mut self) {
        let Some(PendingInterfaceResources::Ready { cursor }) = self.interface.take() else {
            panic!("interface pre-decode dispatched twice");
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            let handle = std::thread::Builder::new()
                .name("interface-decode".into())
                .spawn(move || decode_interface_managers(cursor))
                .expect("failed to spawn interface decode thread");
            self.interface = Some(PendingInterfaceResources::Thread(handle));
        }
        #[cfg(target_arch = "wasm32")]
        {
            #[cfg(feature = "wasm-threads")]
            if robin_assets::wasm_threads::pool_threads() > 0 {
                self.interface = Some(PendingInterfaceResources::Pool(
                    robin_assets::wasm_threads::start_on_pool(move || {
                        decode_interface_managers(cursor)
                    }),
                ));
                return;
            }
            self.interface = Some(PendingInterfaceResources::Ready { cursor });
        }
    }

    /// Collect the `(cursor, menu)` interface managers for frontend
    /// assembly, waiting for the pre-decode worker when one is running.
    /// Never blocks the wasm main thread (the pool variant is awaited).
    pub(super) async fn take_interface(&mut self) -> (ResourceManager, ResourceManager) {
        match self
            .interface
            .take()
            .expect("interface archive already consumed")
        {
            PendingInterfaceResources::Ready { cursor } => {
                // No worker ran: hand the menus their own lazily-decoded
                // copy, exactly like the old second attach.
                let menu = cursor.duplicate();
                (cursor, menu)
            }
            #[cfg(not(target_arch = "wasm32"))]
            PendingInterfaceResources::Thread(handle) => {
                handle.join().expect("interface decode thread panicked")
            }
            #[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
            PendingInterfaceResources::Pool(receiver) => receiver
                .await
                .expect("interface decode worker dropped its result"),
        }
    }

    pub(super) fn engine_setup_resources(
        &mut self,
        host: &mut Host,
    ) -> (
        Option<engine_api::GroundMarkSpriteData>,
        Vec<u16>,
        Option<engine_api::MinimapWidgetSetup>,
    ) {
        let cursor = self.interface_cursor_mut();
        let ground_mark_sprite = extract_ground_mark_sprite_data(cursor);
        let titbit_rows = extract_titbit_row_frame_counts(cursor);
        let minimap_widget = extract_minimap_widget_setup(cursor);
        if let Some(data) = ground_mark_sprite.as_ref() {
            host.install_trajectory_ground_mark_sprite(data);
        }
        (ground_mark_sprite, titbit_rows, minimap_widget)
    }

    pub(super) fn resolve_short_briefings(
        &mut self,
        level_descriptors: Option<&assets_res_descr::LevelDescriptors>,
    ) -> std::collections::HashMap<u32, String> {
        let Some(descriptor) = level_descriptors else {
            return std::collections::HashMap::new();
        };
        let table_id = descriptor.short_briefing.text_table_id;
        let mut resolved = match self.text.get_string_count(table_id) {
            Ok(count) => (0..count)
                .filter_map(|index| {
                    self.text
                        .get_string(table_id, index)
                        .ok()
                        .map(|text| (index as u32, text.to_string()))
                })
                .collect(),
            Err(error) => {
                tracing::warn!(
                    "Short-briefing text table {table_id} unavailable in Level.res: {error}"
                );
                std::collections::HashMap::new()
            }
        };
        for (index, text) in descriptor.custom_short_briefings.iter().enumerate() {
            if let Some(text) = text {
                resolved.insert(index as u32, text.clone());
            }
        }
        resolved
    }
}

/// Pre-decode the background map + minimap and attach the interface /
/// text resource files while the loading screen is still visible.
///
/// Second progress-closure scope (the first one was dropped at the end
/// of the CPU-only loading block, so audio setup could borrow the window).
/// The closure must be dropped before we close the loading screen and hand
/// the GPU context to the game renderer.
///
/// Runs the slow CPU work *before* closing the loading screen:
///  - bzip2-decompress `.map` / `.min` + mask composition
///  - attach interface / text resource files (pure file I/O)
///  - load HUD font glyphs
///  - load level-descriptor `.red` file
///
/// Everything that needs the game renderer happens after the close.
/// `.map` / `.min` and resource attachments happen well before the
/// loading screen closes.
#[allow(clippy::too_many_arguments)]
pub(super) fn pre_decode_maps_and_resources(
    mut event_pump: Option<&mut crate::window::GameWindow>,
    loading_screen: &mut Option<crate::loading_screen::LoadingScreenRenderer>,
    engine: &mut Engine,
    profiles: &engine_profiles::ProfileManager,
    host: &Host,
    game: &Game,
) -> LoadedInteractiveResources {
    let mut timer = PhaseTimer::new("descriptor+font setup");
    tick_progress(loading_screen, event_pump.as_deref_mut(), 1.0);
    tick_progress(loading_screen, event_pump.as_deref_mut(), 1.0);

    // `text_res` (Data/Text/Level.res) + `cursor_res` (DEFAULT.RES) were
    // attached earlier so `Engine::new` could absorb the peasant name
    // pool, ground-mark sprite data, and titbit row counts.

    if let Some(ls) = loading_screen.as_mut() {
        ls.set_status("Loading level descriptors...", LOADING_DESCRIPTORS_PROGRESS);
    }

    // Level descriptors (`.red` file) and HUD fonts — file I/O only.
    let mut level_descriptors = (|| {
        let campaign = engine.campaign();
        let mission_id = descriptor_mission_id(campaign, profiles);
        let filename = assets_res_descr::red_filename(mission_id);
        if let Some(dd) = host.shipping.as_deref()
            && let Some(desc) = dd.localized_level_descriptors(&filename)
        {
            tracing::info!(
                "Level descriptors {filename}: loaded from shipping datadir ({} dialogues)",
                desc.dialogues.len()
            );
            return Some(desc.clone());
        }
        let path = format!("Data/Text/{filename}");
        match assets_res_descr::load(&path) {
            Ok(desc) => {
                tracing::info!(
                    "Loaded level descriptors from {path}: {} dialogues",
                    desc.dialogues.len()
                );
                Some(desc)
            }
            Err(e) => {
                tracing::warn!("Failed to load level descriptors from {path}: {e}");
                None
            }
        }
    })();
    if let Some(descriptors) = level_descriptors.as_mut() {
        apply_custom_mission_text_patch(engine.campaign(), profiles, descriptors);
    }
    timer.step("level descriptors");
    tick_progress(loading_screen, event_pump.as_deref_mut(), 1.0);

    if let Some(ls) = loading_screen.as_mut() {
        ls.set_status("Loading HUD fonts...", LOADING_HUD_FONTS_PROGRESS);
    }
    let hud_fonts = HudFonts::load();
    timer.step("HUD fonts");
    tick_progress(loading_screen, event_pump.as_deref_mut(), 1.0);

    // Background + minimap bitmaps are pre-decoded inside
    // `load_level_and_sprite_bank` — they must be decoded *before*
    // `Engine::new` so the engine can be constructed with real grid
    // dimensions (RAII).  This function now only handles the
    // post-engine resources (level descriptors + HUD fonts).  Let the
    // caller use the bg/mm from `load_level_and_sprite_bank`.
    let _ = (engine, game, host, event_pump);

    if let Some(ls) = loading_screen.as_mut() {
        ls.set_status("Finalizing...", LOADING_FINAL_PROGRESS);
    }
    LoadedInteractiveResources {
        level_descriptors,
        hud_fonts,
    }
}

/// Tick the loading-screen progress bar by `delta` and drain any
/// pending WM resize events so the canvas stays in sync during a
/// long-running mission load.
pub(super) fn tick_progress(
    loading_screen: &mut Option<crate::loading_screen::LoadingScreenRenderer>,
    event_pump: Option<&mut crate::window::GameWindow>,
    delta: f32,
) {
    if let Some(ls) = loading_screen.as_mut() {
        ls.increment(delta);
        if let Some(event_pump) = event_pump {
            ls.drain_events(event_pump);
        }
    }
}

/// Bundle of host-side renderers + caches populated from DEFAULT.RES
/// during mission setup.  Returned by [`load_mission_sprites`].
pub(super) struct MissionSprites {
    pub(super) cursor_renderer: CursorRenderer,
    pub(super) selection_mark_renderer: SelectionMarkRenderer,
    pub(super) mouse_trail_renderer: Option<MouseTrailRenderer>,
    pub(super) titbit_renderer: TitbitRenderer,
    pub(super) portrait_cache: PortraitCache,
}

/// Load the cursor, minimap button/dots, ground-focus marker, selection
/// mark, mouse trail, titbits, portraits, and peasant names — every
/// renderer that takes its frames from `DEFAULT.RES` / `Level.res`
/// during mission start-up.
///
/// Each subsystem fetches its surfaces from the shared interface bank.
/// Also pushes the derived data into the engine
/// (`setup_minimap_widget`, `set_ground_mark_sprite_data`,
/// `set_titbit_row_frame_counts`, `set_peasant_names`). The frame-holder
/// dictionaries were already finalized and published by the CPU loading phase.
pub(super) fn load_mission_sprites(
    engine: &mut Engine,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    renderer: &mut Renderer,
    cursor_res: &mut ResourceManager,
    text_res: &mut ResourceManager,
) -> MissionSprites {
    // ── Cursor setup ──
    // `cursor_res` (DEFAULT.RES) was pre-attached above while the
    // loading screen was still visible.
    let mut timer = PhaseTimer::new("mission sprite setup");
    let mut cursor_renderer = CursorRenderer::new();
    cursor_renderer.init(renderer);

    // Load the default game cursor.
    if !cursor_renderer.load_cursor(resource_ids::RHMOUSE_DEFAULT, cursor_res, renderer) {
        tracing::warn!("Failed to load default cursor — using fallback arrow");
    }
    timer.step("cursor");

    // ── Minimap corner button ──
    // Corner-sprite dims + hit mask were pre-computed from
    // `cursor_res` and handed to `Engine::new` via
    // `EngineArgs::minimap_widget`; this block only uploads the
    // corner GPU textures and stashes the corner size host-side for
    // the HUD layout.
    match cursor_res.get_dimension(resource_ids::RHMAP_CORNER) {
        Ok((btn_w, btn_h)) => {
            host.minimap_corner_size = ScreenSize::new(btn_w as f32, btn_h as f32);
            if let Ok(pics) = cursor_res.get_pictures(resource_ids::RHMAP_CORNER) {
                let corner_surfaces: Vec<u32> = pics
                    .iter()
                    .filter_map(|opt| opt.as_ref().map(|p| picture_to_surface(renderer, p)))
                    .collect();
                if !corner_surfaces.is_empty() {
                    host.minimap_corner_surfaces = corner_surfaces;
                }
            }
            tracing::info!(
                "Minimap corner button: {}x{}, button at ({:.0}, {:.0}), map at ({:.0}, {:.0})",
                btn_w,
                btn_h,
                host.engine_display.minimap().button_box().top_left().x,
                host.engine_display.minimap().button_box().top_left().y,
                host.engine_display.minimap().map_box().top_left().x,
                host.engine_display.minimap().map_box().top_left().y,
            );
        }
        _ => {
            tracing::warn!("Failed to load RHMAP_CORNER resource — minimap button unavailable");
        }
    }

    // ── Minimap dot sprites (RHMAP_ITEMS) ──
    // 21 dot sprites (hero/enemy/civilian/scroll/etc.). Upload all
    // frames at mission start so `render_minimap` can blit them
    // without touching the resource manager each frame.
    match cursor_res.get_pictures(resource_ids::RHMAP_ITEMS) {
        Ok(pics) => {
            let surfaces: Vec<(u32, u16, u16)> = pics
                .iter()
                .map(|opt| match opt {
                    Some(p) => (picture_to_surface(renderer, p), p.width, p.height),
                    None => (0, 0, 0),
                })
                .collect();
            tracing::info!("Loaded RHMAP_ITEMS: {} dot frames", surfaces.len());
            host.minimap_dot_surfaces = surfaces;
        }
        Err(e) => {
            tracing::warn!("Failed to load RHMAP_ITEMS resource — minimap dots disabled: {e}");
        }
    }

    // ── Destination marker sprite (RHID_GROUND_FOCUS) ──
    // Loads a row of sprite frames from the global DEFAULT.RES
    // resource bank that get blitted at the click destination after a
    // move order is issued.
    match cursor_res.get_pictures(resource_ids::RHID_GROUND_FOCUS) {
        Ok(pics) => {
            let first_pic = pics.iter().find_map(|opt| opt.as_ref());
            let surfaces: Vec<(u32, u16, u16)> = pics
                .iter()
                .filter_map(|opt| {
                    opt.as_ref().map(|p| {
                        let id = picture_to_surface(renderer, p);
                        (id, p.width, p.height)
                    })
                })
                .collect();
            if surfaces.is_empty() {
                tracing::warn!("RHID_GROUND_FOCUS has no frames — destination marker disabled");
            } else {
                // The ground-mark sprite MoveBox is the auto-cropped
                // tight bounds of frame 0 (the dictionary-packed
                // sprite scans non-0x07C0 pixels and records the
                // cropped w/h plus a per-frame offset).  We store the
                // uncropped Picture; scan for the opaque bounds so the
                // half-diagonal lines up exactly.  Fall back to the
                // raw Picture size if the scan can't run (non-16-bit).
                let (cw, ch) = first_pic
                    .and_then(|p| p.opaque_bounds_16().map(|(_, _, cw, ch)| (cw, ch)))
                    .unwrap_or((surfaces[0].1, surfaces[0].2));
                tracing::info!(
                    "Loaded RHID_GROUND_FOCUS: {} frames, raw {}x{}, cropped {}x{}",
                    surfaces.len(),
                    surfaces[0].1,
                    surfaces[0].2,
                    cw,
                    ch,
                );
            }
            // Sprite-data + half-diagonal were absorbed into the
            // engine at construction via
            // `EngineArgs::ground_mark_sprite`; the GPU surfaces below
            // are pure host-side rendering state.
            host.ground_mark_surfaces = surfaces;
        }
        Err(e) => {
            tracing::warn!("Failed to load RHID_GROUND_FOCUS resource: {e}");
        }
    }

    timer.step("minimap + ground-focus sprites");

    // ── Selection mark renderer ──
    // Loads RHID_GROUND_SELECT (green idle) and RHID_GROUND_SELECT_SWORD
    // (red combat) sprites from DEFAULT.RES.
    let dynamic_ambience_visuals = host
        .application_context
        .active_profile_snapshot()
        .map(|profile| profile.graphic_config.dynamic_ambience_visuals)
        .unwrap_or(true);
    let visual_shadow_color = if dynamic_ambience_visuals {
        engine.weather().night_color
    } else {
        engine.initial_mission_night_color()
    };
    let mut selection_mark_renderer = SelectionMarkRenderer::new();
    selection_mark_renderer.load(cursor_res, renderer, visual_shadow_color);
    timer.step("selection marks");

    // ── Swordfight mouse-trail renderer ──
    // Loads RHID_MOUSE_TRAIL, builds the 32-level alpha pattern table,
    // and creates one managed surface per alpha level.  Rendered each
    // frame while the player drags the left mouse button during a
    // swordfight.
    let mouse_trail_renderer = match cursor_res.get_picture(resource_ids::RHID_MOUSE_TRAIL, 0) {
        Ok(pic) => {
            let r = MouseTrailRenderer::from_picture(pic, renderer);
            if r.is_none() {
                tracing::warn!(
                    "RHID_MOUSE_TRAIL picture was not in an RGB16 format or was empty — swordfight trail disabled"
                );
            } else {
                tracing::info!("Loaded RHID_MOUSE_TRAIL: pattern height {}", pic.height);
            }
            r
        }
        Err(e) => {
            tracing::warn!("Failed to load RHID_MOUSE_TRAIL resource: {e}");
            None
        }
    };

    // ── Titbit renderer ──
    // Loads the GPU textures for every titbit sprite row from the
    // `titbit_texture_creator` we created up front (before the
    // renderer's canvas borrow).
    let mut titbit_renderer = TitbitRenderer::new();
    titbit_renderer.load(
        cursor_res,
        renderer.gpu(),
        visual_shadow_color,
        renderer.scale_mode(),
    );
    // Row frame counts were absorbed by the engine at construction via
    // `EngineArgs::titbit_row_frame_counts`; no post-load setter needed.
    timer.step("titbit renderer");

    // ── Portrait pictures (character faces in the bottom panel) ──
    // Portraits live in the same DEFAULT.RES file as cursors.
    let mut portrait_cache = PortraitCache::new();
    portrait_cache.load(cursor_res, renderer);
    timer.step("portrait cache");

    // ── Localized character names ──
    // `text_res` (Data/Text/Level.res) was pre-attached in the loading-
    // screen block above.  The peasant firstname/surname *pool* is
    // loaded earlier and handed to `Engine::new` via `EngineArgs`; this
    // call assigns per-civilian display names on top of that pool.
    //
    // The localized hero names + generated peasant names live entirely
    // in the host-side `PortraitCache.localized_names` map; the engine
    // never stores them on entities (HUD code resolves on demand from
    // the profile tables + this map).
    portrait_cache.install_localized_names(load_localized_character_names(text_res));
    portrait_cache.generate_peasant_names(
        text_res,
        engine,
        &mut host.frontend.engine_display,
        &mut host.frontend.input,
        assets,
    );
    timer.step("localized + peasant names");
    timer.total();

    MissionSprites {
        cursor_renderer,
        selection_mark_renderer,
        mouse_trail_renderer,
        titbit_renderer,
        portrait_cache,
    }
}

/// Pre-compute the minimap corner-button widget setup from
/// `cursor_res`: corner-sprite dimensions plus the pixel-level hit
/// mask built from frame 1 of `RHMAP_CORNER` (the frame used for the
/// pixel-level `IsRealPoint` test).  Returns `None` when the resource
/// is missing or has no pictures.
pub(super) fn extract_minimap_widget_setup(
    cursor_res: &mut ResourceManager,
) -> Option<engine_api::MinimapWidgetSetup> {
    let (btn_w, btn_h) = cursor_res.get_dimension(resource_ids::RHMAP_CORNER).ok()?;
    let corner_size = ScreenSize::new(btn_w as f32, btn_h as f32);
    let mut button_hit_mask = None;
    if let Ok(pics) = cursor_res.get_pictures(resource_ids::RHMAP_CORNER)
        && let Some(Some(pic)) = pics.get(1)
    {
        let pixels: Vec<u16> = pic
            .data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        button_hit_mask = Some(HitMask::from_pixels_u16(
            pic.width,
            pic.height,
            &pixels,
            TRANSPARENT_COLOR_KEY_16,
        ));
    }
    Some(engine_api::MinimapWidgetSetup {
        corner_size,
        button_hit_mask,
    })
}

/// Pre-compute the destination-marker (`RHID_GROUND_FOCUS`) sprite
/// metadata from `cursor_res`: half-diagonal (half-width, half-height)
/// in world pixel units plus the per-frame `(w, h)` sizes.
///
/// Returns `None` if the resource is missing or has no frames — the
/// caller passes that through to [`EngineArgs::ground_mark_sprite`]
/// and the engine leaves the marker disabled.
pub(super) fn extract_ground_mark_sprite_data(
    cursor_res: &mut ResourceManager,
) -> Option<engine_api::GroundMarkSpriteData> {
    let pics = cursor_res
        .get_pictures(resource_ids::RHID_GROUND_FOCUS)
        .ok()?;
    let first_pic = pics.iter().find_map(|opt| opt.as_ref())?;
    let frame_sizes: Vec<(u16, u16)> = pics
        .iter()
        .filter_map(|opt| opt.as_ref().map(|p| (p.width, p.height)))
        .collect();
    if frame_sizes.is_empty() {
        return None;
    }
    // The destination marker uses the auto-cropped tight bounds of
    // frame 0; we store the uncropped Picture, so scan for the opaque
    // bounds and fall back to the raw size when the scan can't run.
    let (cw, ch) = first_pic
        .opaque_bounds_16()
        .map(|(_, _, cw, ch)| (cw, ch))
        .unwrap_or((frame_sizes[0].0, frame_sizes[0].1));
    // Per-frame offset = (x_min, y_min) of the opaque region.  Used
    // by `IsOnScreen`/`GenerateBlitBox` so the cull AABB tracks the
    // opaque region instead of the full uncropped surface.  Defaults
    // to (0, 0) for any frame whose opaque-bounds scan can't run
    // (non-16-bit or fully transparent).
    let per_frame_offsets: Vec<(i16, i16)> = pics
        .iter()
        .map(|opt| {
            opt.as_ref()
                .and_then(|p| p.opaque_bounds_16())
                .map(|(x, y, _, _)| (x as i16, y as i16))
                .unwrap_or((0, 0))
        })
        .collect();
    Some(engine_api::GroundMarkSpriteData {
        half_w: cw as f32 * 0.5,
        half_h: ch as f32 * 0.5,
        frame_sizes,
        per_frame_offsets,
    })
}

/// Pre-compute titbit sprite-row frame counts from `cursor_res`.
/// Indexed by `SpriteRow` discriminant.  Counts sub-pictures without
/// decoding them — enough for `TitbitManager::num_frames_for_row` to
/// drive animation.
pub(super) fn extract_titbit_row_frame_counts(cursor_res: &mut ResourceManager) -> Vec<u16> {
    use crate::titbit_renderer::titbit_sprite_row_resources;
    let num_rows = SpriteRow::NumberOfRows as usize;
    let mut counts = vec![0u16; num_rows];
    for &(row, res_id) in titbit_sprite_row_resources() {
        let n = cursor_res
            .get_pictures(res_id)
            .map(|pics| {
                pics.iter()
                    .filter(|o| o.as_ref().is_some_and(|p| p.width > 0 && p.height > 0))
                    .count() as u16
            })
            .unwrap_or(0);
        let idx = row as usize;
        if idx < counts.len() {
            counts[idx] = n;
        }
    }
    counts
}

/// Load the 22-firstname / 22-surname peasant name pool from
/// `Level.res` — the civilian display-name branch.  Sub-IDs 100-121
/// hold firstnames, 122-143 surnames, under one of three menu text
/// tables (full / demo / demo2).
pub fn load_peasant_name_pool(text_res: &mut ResourceManager) -> (Vec<String>, Vec<String>) {
    use crate::ui_panel::menu_text_string;
    const FIRSTNAME_BASE: usize = 100;
    const SURNAME_BASE: usize = 122;
    const NAME_COUNT: usize = 22;
    let firstnames: Vec<String> = (0..NAME_COUNT)
        .filter_map(|i| menu_text_string(text_res, FIRSTNAME_BASE + i).map(|(s, _, _)| s))
        .collect();
    let surnames: Vec<String> = (0..NAME_COUNT)
        .filter_map(|i| menu_text_string(text_res, SURNAME_BASE + i).map(|(s, _, _)| s))
        .collect();
    (firstnames, surnames)
}

/// Load the fixed localized VIP names selected by
/// `RHPCStatus::GenerateName`. Keys are the canonical French profile
/// identities stored in CPF and mission data.
pub fn load_fixed_vip_name_map(
    text_res: &mut ResourceManager,
) -> std::collections::BTreeMap<String, String> {
    use crate::ui_panel::menu_text_string;
    const VIP_NAME_BASE: usize = 144;
    const PROFILE_NAMES: [&str; 7] = [
        "Robin des bois",
        "Robin des villes",
        "Will Ecarlate",
        "Petit Jean",
        "Frere Tuck",
        "Lady Marianne",
        "Stutely",
    ];

    PROFILE_NAMES
        .into_iter()
        .enumerate()
        .filter_map(|(offset, profile_name)| {
            menu_text_string(text_res, VIP_NAME_BASE + offset)
                .map(|(localized, _, _)| (profile_name.to_owned(), localized))
        })
        .collect()
}

/// Run the CPU-only loading phase: sprite bank, campaign install +
/// level load (folded into a single `Engine::new` constructor call),
/// CLI-flag apply, mission script StartUp, Sherwood production
/// bonuses, and night/fog sprite-variant generation.
///
/// Constructs and returns the freshly-initialized `Engine`,
/// `LevelAssets`, and `DevState` — none of them are needed before this
/// phase, so the constructors live at the bottom of the loading
/// pipeline where all the required data is already in hand.
///
/// All slow work (map decompression, entity spawn, script init)
/// happens between `Initialize` and `Close` of the loading screen.
/// The `progress` closure captures the loading screen + event-pump
/// fields so each call ticks the bar and drains WM events.
/// CPU-loaded mission state consumed by the frontend-specific bootstrap.
///
/// This is a process-lifetime ownership seam, not persisted game state: the
/// decoded bitmaps are host upload scratch and `LevelAssets` contains runtime
/// caches. Consequently it deliberately does not implement serde.
pub(super) struct LoadedMissionCore {
    pub(super) engine: Engine,
    /// Exact campaign input captured immediately before `Engine::new`.
    /// Level initialization mutates the engine-owned campaign (notably,
    /// Sherwood clears `mission_team_indices` after spawning PCs), so replay
    /// reconstruction must use this pre-construction snapshot rather than
    /// `engine.campaign()`.
    pub(super) replay_campaign: Campaign,
    pub(super) assets: engine_api::LevelAssets,
    pub(super) dev: engine_api::DevState,
    pub(super) pre_decoded_background: Option<engine_api::level_loading::PreDecodedBackground>,
    pub(super) pre_decoded_minimap: Option<engine_api::level_loading::PreDecodedMinimap>,
    /// Still-running background+minimap decode for interactive missions
    /// (`defer_terrain_join`): joined during frontend assembly, right before
    /// the GPU upload needs the pixels.
    pub(super) pending_terrain: Option<crate::level_loading_host::PendingTerrainDecode>,
    /// Grid dimensions the engine was constructed with, for the divergence
    /// assert at the deferred join.
    pub(super) bg_pixel_dims: (f32, f32),
    pub(super) pre_decoded_ambience_backgrounds: Vec<(
        engine_api::Ambiance,
        engine_api::level_loading::PreDecodedBackground,
    )>,
    pub(super) pre_decoded_ambience_minimaps: Vec<(
        engine_api::Ambiance,
        engine_api::level_loading::PreDecodedMinimap,
    )>,
    pub(super) engine_rng_seed: u64,
    pub(super) engine_sim_config: engine_api::SimConfig,
}

pub(super) struct MissionLoadError {
    pub(super) message: String,
    pub(super) campaign: Campaign,
}

impl MissionLoadError {
    fn new(campaign: Campaign, message: String) -> Self {
        Self { message, campaign }
    }
}

#[cfg(test)]
fn initial_rng_seed(
    args: &crate::main_entry::CliArgs,
    multiplayer_seed: Option<u64>,
) -> Result<u64, String> {
    if let Some(data) = args.replay_data.as_ref() {
        crate::replay_format::validate_replay_data(data)
            .map_err(|e| format!("failed to validate requested replay data: {e}"))?;
        Ok(data.header.rng_seed)
    } else if let Some(spec) = args.replay.as_deref() {
        crate::replay_format::load_replay_spec(spec)
            .map(|data| data.header.rng_seed)
            .map_err(|e| format!("failed to load requested replay `{spec}`: {e}"))
    } else {
        Ok(multiplayer_seed.unwrap_or(0))
    }
}

pub(crate) fn initial_sim_config(args: &crate::main_entry::CliArgs) -> engine_api::SimConfig {
    let mut sim_config = args.global_options.sim_config();
    sim_config.golden_eye |= args.goldeneye;
    if args
        .mission
        .as_deref()
        .is_some_and(robin_engine::level_data::hackable_level_exists)
    {
        // Hackable JSON levels are unscripted sandboxes, not legacy scripted
        // missions, so requiring an SCB StartUp class would reject them.
        sim_config.script_enabled = false;
    }
    sim_config
}

#[cfg(test)]
fn construct_with_initial_rng_seed<T>(
    args: &crate::main_entry::CliArgs,
    multiplayer_seed: Option<u64>,
    construct: impl FnOnce(u64) -> Result<T, String>,
) -> Result<(T, u64), String> {
    let rng_seed = initial_rng_seed(args, multiplayer_seed)?;
    construct(rng_seed).map(|value| (value, rng_seed))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn load_level_and_sprite_bank(
    mut event_pump: Option<&mut crate::window::GameWindow>,
    loading_screen: &mut Option<crate::loading_screen::LoadingScreenRenderer>,
    host: &mut Host,
    game: &mut Game,
    campaign: Campaign,
    profiles: &engine_profiles::ProfileManager,
    text_res: &mut ResourceManager,
    args: &crate::main_entry::CliArgs,
    _screen_width: f32,
    _screen_height: f32,
    ground_mark_sprite: Option<engine_api::GroundMarkSpriteData>,
    titbit_row_frame_counts: Vec<u16>,
    minimap_widget: Option<engine_api::MinimapWidgetSetup>,
    authoritative_rng_seed: u64,
    authoritative_sim_config: engine_api::SimConfig,
    defer_terrain_join: bool,
) -> Result<LoadedMissionCore, MissionLoadError> {
    let mut assets = engine_api::LevelAssets::new();
    // Stamp the canonical loaded profile manager onto LevelAssets — the
    // engine reads profiles via `&assets.profile_manager` everywhere now
    // (Campaign no longer owns its own copy).
    assets.profile_manager = std::sync::Arc::new(profiles.clone());
    let mut dev = engine_api::DevState::new();
    dev.debug.surface_display = game.global_options.options().debug_surfaces;
    let mut timer = PhaseTimer::new("level+bank setup");

    // Load the mission binaries FIRST — they're cheap and they carry the
    // mission header (map filename + ambiance), which lets the slow
    // background-map decode start on a worker thread (native) while this
    // thread continues with the sprite bank, scripts, and minimap.
    let mission_name = campaign.current_mission_idx.map(|i| {
        campaign.missions[i]
            .profile(&assets.profile_manager)
            .mission_filename
            .clone()
    });
    let level_directory = game.global_options.level_directory.clone();
    let loaded_result = {
        let mut progress = |delta: f32| {
            tick_progress(loading_screen, event_pump.as_deref_mut(), delta);
        };
        if let Some(name) = mission_name.as_deref()
            && let Some(level) = host
                .shipping
                .as_ref()
                .and_then(|datadir| datadir.loaded_level(name))
        {
            tracing::info!(mission = name, "level loaded from shipping mission payload");
            progress(1.0);
            progress(1.0);
            Ok(level)
        } else {
            engine_api::level_loading::load_mission_for_campaign(
                &campaign,
                &assets.profile_manager,
                &level_directory,
                &mut progress,
            )
            .map_err(|e| format!("Level load failed: {e}"))
        }
    };
    let loaded = match loaded_result {
        Ok(loaded) => loaded,
        Err(message) => return Err(MissionLoadError::new(campaign, message)),
    };
    timer.step("mission binaries");

    let authored_initial_ambiance = engine_api::Ambiance::from_raw(loaded.mission.header.ambiance);
    let effective_initial_ambiance = loaded
        .mission
        .ambience_schedule
        .iter()
        .take_while(|cue| cue.at_seconds == 0)
        .last()
        .map_or(authored_initial_ambiance, |cue| cue.ambiance);
    let ambiance_dir = effective_initial_ambiance.directory().to_string();
    let map_name = loaded.mission.header.map_filename.clone();

    // Start the background-map + minimap decode (bzip2/JXL — the slowest
    // CPU-only step of level setup) off the loading path so it overlaps the
    // rest of this function and, for interactive missions, everything up to
    // the GPU upload during frontend assembly: `Engine::new` needs only the
    // pixel *dimensions* (probed cheaply from the map header below).
    // Native uses a dedicated thread; wasm uses the rayon worker pool when
    // the `wasm-threads` build initialized one, and otherwise decodes
    // synchronously right here (single-threaded browser fallback — the
    // progress closure keeps feeding the loading bar in that case).
    let pending_terrain = crate::level_loading_host::PendingTerrainDecode::start(
        &map_name,
        &ambiance_dir,
        &level_directory,
        host.shipping.clone(),
    );
    timer.step("terrain decode start");

    // Install the sprite bank — must happen before entity sprite
    // loading in initialize_for_mission. The parsed bank comes from the
    // process-wide asset cache (warmed on a background thread at
    // startup); the mission gets a clone so its runtime overlay sprites
    // don't leak into later missions. Bank sprites carry mmap spans,
    // not pixel data, so the clone is cheap.
    if let Some(ls) = loading_screen.as_mut() {
        ls.set_status("Loading sprite bank...", 0.56);
    }
    {
        let asset_cache =
            crate::process_asset_cache::get_or_build(host.shipping.as_deref(), profiles);
        match asset_cache.sprite_bank.as_ref() {
            Some(bank) => *host.frame_holder_mut() = bank.clone(),
            None => tracing::warn!("Sprite bank unavailable in process asset cache"),
        }
        tick_progress(loading_screen, event_pump.as_deref_mut(), 1.0);
    }
    timer.step("sprite bank from process asset cache");
    preload_hackable_character_dirs(host, &mut assets, &campaign);
    timer.step("hackable character preload");
    // Publish the sprite-bank signature into LevelAssets so engine-side
    // sprite-script loaders can detect bank changes.
    assets.bank_signature = host.frame_holder.signature();
    tick_progress(loading_screen, event_pump.as_deref_mut(), 1.0);

    if let Some(ls) = loading_screen.as_mut() {
        ls.set_status("Initializing level...", 0.73);
    }

    // Engine LevelAssets already owns profile_manager (loaded at startup);
    // Campaign no longer has its own copy.

    // Hand the engine the parsed mission scripts it'll need.
    //
    // Engine doesn't depend on robin_assets, so it can't open `.scb`
    // files itself; the host parses them (preferring shipping, falling
    // back to disk for the current mission), decodes immutable bytecode,
    // and stores the programs in `LevelAssets` before level load.
    let mut scripts: std::collections::BTreeMap<String, engine_scb::ScbFile> = mission_name
        .as_deref()
        .and_then(|name| host.shipping.as_ref().map(|dd| dd.mission_scripts(name)))
        .unwrap_or_default()
        .into_iter()
        .collect();
    if let Some(name) = mission_name.as_ref()
        && !scripts.contains_key(name)
    {
        let path = format!("Data/Levels/{name}.scb");
        match engine_sbfile::SbFile::read_all(&path)
            .map_err(|e| format!("read {path}: error {e}"))
            .and_then(|b| assets_scb::parse_bytes(&b).map_err(|e| format!("parse {path}: {e}")))
        {
            Ok(scb) => {
                scripts.insert(name.clone(), scb);
            }
            Err(e) => tracing::warn!("Mission script {name}: {e}"),
        }
    }
    let legacy_capture_scb = args.mission_start_legacy_save.as_ref().map(|_| {
        let name = mission_name
            .as_ref()
            .expect("legacy frame-zero capture requires a current mission");
        scripts
            .get(name)
            .unwrap_or_else(|| panic!("legacy frame-zero capture has no mission script {name}"))
            .clone()
    });
    let script_programs = scripts
        .iter()
        .map(|(name, scb)| {
            (
                name.clone(),
                std::sync::Arc::new(engine_script_manager::ScriptProgram::from_scb(scb.clone())),
            )
        })
        .collect();
    assets.scripts.mission_programs = std::sync::Arc::new(script_programs);
    timer.step("mission scripts");

    // Initialize Game's per-mission state from the campaign before we
    // hand it off to the engine.
    game.initialize_for_mission(&campaign, &assets.profile_manager);

    // Construct the engine with campaign ownership + level load folded
    // in.  The old split constructor followed by
    // `initialize_from_campaign` + `initialize` sequence collapses to
    // this single call.  Mission script was already loaded inside
    // `load_level()` → `load_mission_script()` so the level loader
    // does not re-load it.
    (assets.peasant_firstnames, assets.peasant_surnames) = load_peasant_name_pool(text_res);
    assets.fixed_vip_names = load_fixed_vip_name_map(text_res);

    // Run the single-threaded wasm fallback decode here — the exact point
    // the old synchronous branch used — so the loading bar behaves the same
    // when no worker pool exists. Threaded decodes pass through untouched.
    let pending_terrain = {
        let mut sync_progress = |u: assets_frame_holder::ProgressUpdate| match u {
            assets_frame_holder::ProgressUpdate::Tick(d) => {
                tick_progress(loading_screen, event_pump.as_deref_mut(), d);
            }
            assets_frame_holder::ProgressUpdate::Phase(text, _local) => {
                if let Some(ls) = loading_screen.as_mut() {
                    ls.set_status(text, LOADING_MAP_DECODE_PROGRESS);
                }
            }
        };
        pending_terrain.decode_inline_if_pending(&mut sync_progress)
    };

    // `Engine::new` needs the background bitmap's pixel dimensions to size
    // the fast-find grid. They are probed cheaply from the map header while
    // the decode keeps running; when the probe cannot say (missing/corrupt
    // map, or no map at all) the decode outcome is resolved right here so
    // the existing pre-engine error path reports it.
    let mut pre_decoded_bg: Option<engine_api::level_loading::PreDecodedBackground> = None;
    let mut pre_decoded_mm: Option<engine_api::level_loading::PreDecodedMinimap> = None;
    let install_decoded_terrain =
        |decoded: crate::level_loading_host::DecodedTerrainBitmaps,
         bg: &mut Option<engine_api::level_loading::PreDecodedBackground>,
         mm: &mut Option<engine_api::level_loading::PreDecodedMinimap>|
         -> Result<(f32, f32), String> {
            let background = decoded.background?;
            let dims = background
                .as_ref()
                .map(|b| (b.width as f32, b.height as f32))
                .unwrap_or((0.0, 0.0));
            *bg = background;
            *mm = decoded.minimap;
            Ok(dims)
        };
    let (bg_pixel_dims, bg_pending) = match pending_terrain.try_take_ready() {
        Ok(decoded) => {
            match install_decoded_terrain(decoded, &mut pre_decoded_bg, &mut pre_decoded_mm) {
                Ok(dims) => (dims, None),
                Err(message) => return Err(MissionLoadError::new(campaign, message)),
            }
        }
        Err(pending) => match crate::level_loading_host::probe_background_map_dims(
            &map_name,
            &ambiance_dir,
            &level_directory,
            host.shipping.as_deref(),
        ) {
            Some((w, h)) => ((w as f32, h as f32), Some(pending)),
            None => {
                let decoded = pending.join_now_or_redecode(&mut |_| {});
                match install_decoded_terrain(decoded, &mut pre_decoded_bg, &mut pre_decoded_mm) {
                    Ok(dims) => (dims, None),
                    Err(message) => return Err(MissionLoadError::new(campaign, message)),
                }
            }
        },
    };
    timer.step("background map dims");

    // Decode every additional authored ambience once during mission loading.
    // Feature 14's active-mission-only prefetch remains scoped to this load;
    // nothing is retained in the process cache for unrelated missions.
    let mut scheduled_ambiances = Vec::new();
    if authored_initial_ambiance != effective_initial_ambiance {
        scheduled_ambiances.push(authored_initial_ambiance);
    }
    for cue in &loaded.mission.ambience_schedule {
        if cue.ambiance != effective_initial_ambiance
            && !scheduled_ambiances.contains(&cue.ambiance)
        {
            scheduled_ambiances.push(cue.ambiance);
        }
    }
    let mut pre_decoded_ambience_backgrounds = Vec::new();
    let mut pre_decoded_ambience_minimaps = Vec::new();
    for ambiance in scheduled_ambiances {
        let dir = ambiance.directory();
        let mut update = |u: assets_frame_holder::ProgressUpdate| match u {
            assets_frame_holder::ProgressUpdate::Tick(delta) => {
                tick_progress(loading_screen, event_pump.as_deref_mut(), delta);
            }
            assets_frame_holder::ProgressUpdate::Phase(text, _local) => {
                if let Some(screen) = loading_screen.as_mut() {
                    screen.set_status(text, LOADING_MAP_DECODE_PROGRESS);
                }
            }
        };
        let decoded = crate::level_loading_host::pre_decode_background_map(
            &map_name,
            dir,
            &level_directory,
            host.shipping.as_deref(),
            &mut update,
        )
        .map_err(|error| {
            MissionLoadError::new(
                campaign.clone(),
                format!("{ambiance:?} background map load failed: {error}"),
            )
        })?;
        if let Some(decoded) = decoded {
            let decoded_dims = (decoded.width as f32, decoded.height as f32);
            if bg_pixel_dims == (0.0, 0.0) || decoded_dims == bg_pixel_dims {
                pre_decoded_ambience_backgrounds.push((ambiance, decoded));
            } else {
                tracing::warn!(
                    ?ambiance,
                    ?decoded_dims,
                    ?bg_pixel_dims,
                    "ignoring runtime ambience background with mismatched dimensions"
                );
            }
        }
        let mut progress = |delta: f32| {
            tick_progress(loading_screen, event_pump.as_deref_mut(), delta);
        };
        if let Some(decoded) = crate::level_loading_host::pre_decode_minimap(
            &map_name,
            dir,
            &level_directory,
            host.shipping.as_deref(),
            &mut progress,
        ) {
            pre_decoded_ambience_minimaps.push((ambiance, decoded));
        }
    }

    // Resolve the engine's initial RNG seed before construction so
    // `Engine::new` is the only site that touches RNG state during
    // setup. Campaign selection has already advanced the single-player /
    // replay sequence. A negotiated multiplayer mission seed remains the
    // authority for a network mission.
    if let Some(mm) = minimap_widget {
        host.engine_display.setup_minimap_widget(
            engine_coordinates::ScreenPoint::new(_screen_width - 83.0, 38.0),
            mm.corner_size,
            mm.button_hit_mask,
            _screen_width,
            _screen_height,
        );
    }

    let (rng_seed, sim_config) = if host.transport.net.is_some() {
        let rng_seed = host.transport.mission_seed.unwrap_or_else(|| {
            panic!("active multiplayer transport is missing its Welcome mission seed")
        });
        let sim_config = host.transport.mission_sim_config.unwrap_or_else(|| {
            panic!("active multiplayer transport is missing its Welcome SimConfig")
        });
        (rng_seed, sim_config)
    } else {
        (authoritative_rng_seed, authoritative_sim_config)
    };
    // This is the only point at which setup transfers campaign ownership.
    // Every fallible file/decode step above borrows the session campaign, and
    // the preserving constructor returns the exact allocation on ingestion
    // failure.
    let replay_campaign = campaign.clone();
    let mut engine = {
        let mut progress = |delta: f32| {
            tick_progress(loading_screen, event_pump.as_deref_mut(), delta);
        };
        match Engine::new_preserving_campaign(engine_api::EngineArgs {
            campaign,
            level: engine_api::LevelLoadArgs {
                assets: &mut assets,
                level_directory: &level_directory,
                progress: &mut progress,
                loaded,
                bg_pixel_dims,
            },
            ground_mark_sprite,
            titbit_row_frame_counts,
            rng_seed,
            original_rng_replay: None,
            sim_config,
        }) {
            Ok(engine) => engine,
            Err((error, campaign)) => {
                return Err(MissionLoadError::new(
                    campaign,
                    format!("Level init failed: {error}"),
                ));
            }
        }
    };
    timer.step("engine construction");

    // Engine construction ran on probed header dimensions. Callers that
    // cannot defer (true-headless bootstrap) collect the decoded pixels now
    // — a decode failure still fails the mission load (via the replay
    // campaign clone — `campaign` moved into the engine), and diverging
    // dimensions would corrupt the already-built grid, so that is a hard
    // error rather than a fallback. Interactive callers instead carry the
    // pending decode into frontend assembly and join right before the GPU
    // upload, so the decode also overlaps audio setup, descriptors, HUD
    // fonts, and renderer bring-up.
    let mut pending_terrain_out = None;
    if let Some(pending) = bg_pending {
        if defer_terrain_join {
            pending_terrain_out = Some(pending);
        } else {
            let decoded = pending.join_blocking();
            let background = match decoded.background {
                Ok(background) => background,
                Err(message) => return Err(MissionLoadError::new(replay_campaign, message)),
            };
            if let Some(bg) = background.as_ref() {
                assert_eq!(
                    (bg.width as f32, bg.height as f32),
                    bg_pixel_dims,
                    "background map header dimensions diverge from decoded bitmap"
                );
            }
            pre_decoded_bg = background;
            pre_decoded_mm = decoded.minimap;
            timer.step("background map join");
        }
    }

    if let Some(save_bytes) = args.mission_start_legacy_save.as_ref() {
        let mission_scb = legacy_capture_scb
            .as_ref()
            .expect("legacy frame-zero capture lost its mission script");
        let save = robin_engine::legacy_save::initialized::decode_initialized_v48_save(
            save_bytes.clone(),
            "frame-zero parity capture",
            &engine,
            &assets,
            mission_scb,
            &robin_engine::legacy_save::body::LegacySaveBodyLimits::default(),
        )
        .map_err(|error| {
            MissionLoadError::new(
                replay_campaign.clone(),
                format!("decode frame-zero Original save: {error}"),
            )
        })?;
        let loaded_host = robin_engine::legacy_save::adopt_engine::adopt_known_linux_v48_replay(
            &mut engine,
            &assets,
            &save,
        )
        .map_err(|error| {
            MissionLoadError::new(
                replay_campaign.clone(),
                format!("adopt frame-zero Original save: {error}"),
            )
        })?;
        loaded_host.apply_display_to(&mut host.engine_display);
        host.selected_view_element = loaded_host.selected_view_element();
        tracing::info!("adopted Original v48 save for frame-zero viewport capture");
    }
    if rng_seed != 0 {
        tracing::info!(seed = rng_seed, "engine RNG seeded at construction");
    }
    host.viewport
        .set_level_size(bg_pixel_dims.0, bg_pixel_dims.1);

    // Multiplayer snapshots are cached after the host seat is bootstrapped
    // and then refreshed at the same sampling point as state hashes. That
    // gives early handshakes a frame-0 snapshot while late joiners still
    // adopt a hash-aligned state.

    // GoldenEye is now applied inside `Engine::new` via
    // `EngineArgs::goldeneye` — no post-construction dispatch.
    dev.debug.all_view_cones = args.view_cones;
    tick_progress(loading_screen, event_pump.as_deref_mut(), 1.0);

    if let Some(ls) = loading_screen.as_mut() {
        ls.set_status(
            "Generating sprite variants...",
            LOADING_SPRITE_VARIANTS_PROGRESS,
        );
    }

    // Original's RHengine.cpp ambiance setup regenerates the Night/Fog
    // dictionaries and then immediately calls ApplyArnoLaw on that same
    // mpFrameHolder; RHframeholder.cpp applies the key to base and variant
    // dictionaries together. Preserve that single-generation boundary here:
    // finish every dictionary mutation before publishing engine hit testing.
    crate::level_loading_host::initialize_sprite_variants(host, &engine);
    let dynamic_visuals = host
        .application_context
        .active_profile_snapshot()
        .map(|profile| profile.graphic_config.dynamic_ambience_visuals)
        .unwrap_or(true);
    let initial_shadow_key = if dynamic_visuals {
        engine.weather().night_color
    } else {
        engine.initial_mission_night_color()
    };
    host.frame_holder_mut().apply_arno_law(initial_shadow_key);
    // Engine cannot depend on robin_assets, so LevelAssets holds the trait
    // publisher. Runtime ambiance rebinds replace the immutable generation
    // inside it rather than leaving an Arc::make_mut copy detached.
    assets.pixel_opacity = Some(host.publish_frame_holder_opacity());
    tick_progress(loading_screen, event_pump, 1.0);
    timer.step("sprite variants + Arno's Law");
    timer.total();

    Ok(LoadedMissionCore {
        engine,
        replay_campaign,
        assets,
        dev,
        pre_decoded_background: pre_decoded_bg,
        pre_decoded_minimap: pre_decoded_mm,
        pending_terrain: pending_terrain_out,
        bg_pixel_dims,
        pre_decoded_ambience_backgrounds,
        pre_decoded_ambience_minimaps,
        engine_rng_seed: rng_seed,
        engine_sim_config: sim_config,
    })
}

/// Install the local deterministic seat and publish the host's authoritative
/// frame-zero state. This admission setup is shared by interactive and true-
/// headless missions and deliberately has no renderer, UI, input-device, or
/// audio dependency.
pub(super) fn setup_local_seat_and_multiplayer_snapshot(
    engine: &mut Engine,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    args: &crate::main_entry::CliArgs,
) {
    // Clients adopt the server snapshot (which already includes seat 0) and
    // receive their own ConnectSeat through the server-ordered input stream.
    if args.connect.is_some() {
        return;
    }

    let nickname = args.mp_nickname.clone();
    engine
        .advance_frame(
            assets,
            engine_api::SimulationFrameInput::new(vec![engine_api::SimCommand::from(
                PlayerCommand::ConnectSeat {
                    player_id: host.transport.local_seat,
                    nickname,
                },
            )])
            .with_hourglass(false),
        )
        .expect("bootstrap ConnectSeat admission");
    tracing::info!(
        seat = ?host.transport.local_seat,
        "bootstrap ConnectSeat applied to local engine",
    );
    if let Some(net) = host.transport.net.as_ref() {
        net.publish_initial_snapshot(0, engine);
        net.send_ready_to_sim(0);
        tracing::info!("multiplayer: cached and published frame-0 host snapshot");
    }
}

/// Build `ThreadedInput` + `InputTranslator`, load the active profile's
/// key bindings into both the host cache and the translator, push the
/// `DisplayMap` accelerator into the engine minimap, center the camera
/// on the first PC, and grab the mouse for edge-scrolling.
///
/// Bundles the pre-loop actions performed during mission initialization.
pub(super) fn setup_input_and_camera(
    engine: &mut Engine,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    args: &crate::main_entry::CliArgs,
    window_width: u32,
    window_height: u32,
    mission_idx: usize,
) -> (ThreadedInput, InputTranslator) {
    let mut threaded_input = ThreadedInput::new();
    threaded_input.set_clipping(robin_engine::coordinates::ScreenBBox::from_coords(
        0.0,
        0.0,
        window_width as f32,
        window_height as f32,
    ));
    let mut input_translator = InputTranslator::new(window_width as f32, window_height as f32);

    // Host construction snapshots the active profile's bindings from the
    // ApplicationContext. The Original copies that active config at this
    // exact input-translator boundary (`ReflectActiveKeyConfig`).
    input_translator.load_bindings_from_keyconfig(&host.key_config);

    // The `DisplayMap` minimap accelerator is stored host-side on
    // `host.minimap_fast_key` — the game loop reads it out to emit a
    // minimap-toggle command on key release.  Rebind via the pause
    // menu updates the same host field.
    host.minimap_fast_key = input_translator.get_binding(GameKey::DisplayMap);

    // Install the four HUD-adjacent edge-scroll dead-zone strips so
    // edge-scroll ignores the cursor while it's parked on or beside
    // the bottom HUD panels.
    input_translator.install_hud_dead_zones();

    tracing::info!(
        "Entering mission game loop ({} entities, mission idx: {})",
        engine.entity_count(),
        mission_idx,
    );

    // Bootstrap the local seat: apply `ConnectSeat(local_seat,
    // nickname)` directly to the engine — setup, not gameplay
    // input.  This creates the host's `SeatState`, defaults the
    // CameraState, and centers `view_position` on the first PC's
    // world coords (handler in `engine/commands.rs`).  Going
    // through `dispatch_local_command` would wire-route in MP and
    // make the local engine miss its own seat at frame 0; instead,
    // setup-state is what `InitialSnapshot` captures and ships to
    // joining peers, so they adopt a state that already includes
    // the host's seat.
    //
    // Only SP / server processes bootstrap directly:
    //
    // - **SP (`--connect == None && --server == None`)**: net is
    //   None, just create the seat locally.
    // - **Server (`--server`)**: snapshot is taken AFTER this so
    //   joining clients adopt an engine that already has seat 0.
    //
    // **Clients (`--connect`)** intentionally do NOT bootstrap —
    // they adopt the server's `InitialSnapshot` (which already has
    // the server's seat) and dispatch their own `ConnectSeat` as a
    // per-frame input later, landing at `sim_frame +
    // INPUT_DELAY_FRAMES` symmetrically on every machine.
    //
    // **Headless dedicated server** is a future scope: a `--server`
    // process without a local seat.  Today every `--server` is
    // also a player — keeping that path intact below.
    setup_local_seat_and_multiplayer_snapshot(engine, host, assets, args);
    let camera_focus = engine
        .pc_ids()
        .first()
        .and_then(|&pc_id| engine.get_entity(pc_id))
        .map(|entity| entity.element_data().position_map())
        .or_else(|| {
            spectator_actor_centroid(engine.active_entity_positions().filter_map(
                |(id, position)| {
                    engine
                        .get_entity(id)
                        .is_some_and(|entity| entity.human_data().is_some())
                        .then_some(position)
                },
            ))
        });
    if let Some(position) = camera_focus {
        host.viewport.center_on_point(position);
    }

    (threaded_input, input_translator)
}

fn spectator_actor_centroid(
    positions: impl Iterator<Item = engine_coordinates::MapPoint>,
) -> Option<engine_coordinates::MapPoint> {
    let (sum_x, sum_y, count) = positions.fold((0.0, 0.0, 0_u32), |acc, position| {
        (acc.0 + position.x, acc.1 + position.y, acc.2 + 1)
    });
    (count != 0)
        .then(|| engine_coordinates::MapPoint::new(sum_x / count as f32, sum_y / count as f32))
}

/// Initialize the Kira audio backend and switch the host sound
/// manager into `SoundMode::Menu` so menu music plays during the
/// loading screen.
pub(super) fn init_audio_backend(host: &mut Host, game: &Game) -> Option<KiraAudioBackend> {
    if !game.global_options.sound_enabled {
        tracing::info!("sound disabled via `-NOSOUND`; skipping audio backend init");
        return None;
    }
    let mut audio_backend =
        match KiraAudioBackend::new(&game.global_options.sound_directory, NUM_CHANNELS) {
            Ok(backend) => Some(backend),
            Err(e) => {
                tracing::warn!("Failed to initialize audio: {}. Sound disabled.", e);
                None
            }
        };
    if let Some(backend) = audio_backend.as_mut() {
        host.audio
            .sound
            .set_music_directory(&game.global_options.music_directory);
        // Read the active profile's 3D-sound preference and forward
        // it to the sound manager.  The backend grants the request
        // only when `can_3d_sound()` is true; the kira backend never
        // is, so this lands in 2D with a non-fatal warning.
        let sound_config = host
            .application_context
            .active_profile_snapshot()
            .unwrap_or_else(|error| panic!("audio setup requires an active profile: {error}"))
            .sound_config;
        let want_3d = sound_config.sound_3d;
        if let Err(e) = host.audio.sound.initialize(backend, want_3d) {
            tracing::warn!("Sound manager init failed: {}", e);
        }
        // Apply volumes before set_mode(Menu) so menu music isn't silent.
        host.audio.sound.apply_volumes(&sound_config);
        host.audio.sound.set_mode(SoundMode::Menu, backend);
    }
    audio_backend
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::localization::LanguagePack;
    use robin_engine::replay::{ReplayData, ReplayFile, ReplayHeader};
    use std::cell::Cell;
    use std::collections::BTreeMap;
    use std::io::Write;

    fn language_pack(locale: &str, has_voice: bool) -> LanguagePack {
        LanguagePack {
            locale: locale.to_owned(),
            native_name: locale.to_owned(),
            data_root: locale.to_owned(),
            has_voice,
            has_cinematics: false,
            voice_uses_english_fallback: false,
            cinematics_use_english_fallback: false,
            mission_names: BTreeMap::new(),
        }
    }

    #[test]
    fn multiplayer_explicit_base_timing_never_auto_selects_a_locale_pack() {
        let installed = vec![language_pack("de-DE", true), language_pack("en-US", true)];
        assert_eq!(
            select_canonical_voice_pack(installed, None, true).unwrap(),
            None
        );
    }

    #[test]
    fn explicit_locale_timing_is_strict_and_single_player_still_prefers_english() {
        let installed = vec![language_pack("de-DE", true), language_pack("en-US", true)];
        assert_eq!(
            select_canonical_voice_pack(installed.clone(), Some("de-DE"), true)
                .unwrap()
                .map(|pack| pack.locale),
            Some("de-DE".to_owned())
        );
        assert!(
            select_canonical_voice_pack(installed.clone(), Some("fr-FR"), true)
                .unwrap_err()
                .contains("not installed")
        );
        assert_eq!(
            select_canonical_voice_pack(installed, None, false)
                .unwrap()
                .map(|pack| pack.locale),
            Some("en-US".to_owned())
        );
    }

    #[test]
    fn png_decoder_expands_indexed_pixels_and_palette_transparency() {
        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, 2, 1);
            encoder.set_color(png::ColorType::Indexed);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_palette(vec![0, 255, 0, 0, 0, 255]);
            encoder.set_trns(vec![0, 127]);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[0, 1]).unwrap();
        }

        let (width, height, rgba) = decode_png_rgba_bytes(&encoded, "indexed test").unwrap();

        assert_eq!((width, height), (2, 1));
        assert_eq!(rgba, [0, 255, 0, 0, 0, 0, 255, 127]);
    }

    #[test]
    fn hackable_animation_fallbacks_do_not_override_authored_actions() {
        let script = |action_id| SpriteScript {
            action_id,
            ..SpriteScript::default()
        };
        let conversion = hackable_animation_conversion(&[script(3), script(6), script(10)]);

        assert_eq!(conversion[9], 1, "missing transition reuses walking");
        assert_eq!(conversion[10], 2, "authored running row must win");

        let minimal = hackable_animation_conversion(&[script(3), script(6)]);
        assert_eq!(minimal[10], 1, "minimal sprites may reuse walking");
    }

    #[test]
    fn hackable_sprite_cache_round_trips_and_invalidates_changed_sources() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("frame.png"), b"source").unwrap();
        let manifest_hash = hackable_manifest_hash(b"manifest");
        let cache = HackableRhsCache {
            version: HACKABLE_RHS_CACHE_VERSION,
            manifest_hash,
            sources: vec![hackable_source_stamp(directory.path(), "frame.png").unwrap()],
            frames: vec![assets_frame_holder::RuntimeSprite {
                width: 2,
                height: 1,
                packed_data: vec![0, 1, 0x1234, 0x5678],
                rgba_data: None,
            }],
            profiles: Vec::new(),
        };

        write_hackable_cache(directory.path(), &cache).unwrap();
        let decoded = read_hackable_cache(directory.path(), manifest_hash).unwrap();
        assert_eq!(decoded.frames.len(), 1);
        assert_eq!(decoded.frames[0].packed_data, cache.frames[0].packed_data);

        std::fs::write(directory.path().join("frame.png"), b"source changed").unwrap();
        assert!(read_hackable_cache(directory.path(), manifest_hash).is_none());
        assert!(read_hackable_cache(directory.path(), [7; 32]).is_none());
    }

    #[test]
    fn version_one_hackable_cache_repairs_walking_over_run_alias() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("frame.png"), b"source").unwrap();
        let manifest_hash = hackable_manifest_hash(b"manifest");
        let scripts = vec![
            SpriteScript {
                action_id: 6,
                ..SpriteScript::default()
            },
            SpriteScript {
                action_id: 10,
                ..SpriteScript::default()
            },
        ];
        let mut broken_conversion = vec![UNMAPPED; NONANIMATION_END];
        broken_conversion[6] = 0;
        broken_conversion[10] = 0;
        let cache = HackableRhsCache {
            version: 1,
            manifest_hash,
            sources: vec![hackable_source_stamp(directory.path(), "frame.png").unwrap()],
            frames: Vec::new(),
            profiles: vec![HackableRhsCacheProfile {
                name: "test".to_owned(),
                info: SpriteInfo {
                    scripts: std::sync::Arc::new(scripts),
                    conversion: std::sync::Arc::new(broken_conversion),
                    size: SpriteSize::new(1.0, 1.0),
                    center: SpriteAnchor::new(0.0, 0.0),
                },
            }],
        };
        write_hackable_cache(directory.path(), &cache).unwrap();

        let upgraded = read_hackable_cache(directory.path(), manifest_hash).unwrap();
        assert_eq!(upgraded.version, HACKABLE_RHS_CACHE_VERSION);
        assert_eq!(upgraded.profiles[0].info.conversion[6], 0);
        assert_eq!(upgraded.profiles[0].info.conversion[10], 1);
    }

    #[test]
    fn spectator_camera_focus_is_actor_centroid() {
        let focus = spectator_actor_centroid(
            [
                engine_coordinates::MapPoint::new(100.0, 200.0),
                engine_coordinates::MapPoint::new(300.0, 400.0),
            ]
            .into_iter(),
        )
        .unwrap();
        assert_eq!(focus, engine_coordinates::MapPoint::new(200.0, 300.0));
        assert!(spectator_actor_centroid(std::iter::empty()).is_none());
    }

    #[test]
    fn loading_tail_phase_targets_are_monotonic() {
        let targets = [
            LOADING_MAP_DECODE_PROGRESS,
            LOADING_SPRITE_VARIANTS_PROGRESS,
            LOADING_AUDIO_PROGRESS,
            LOADING_DESCRIPTORS_PROGRESS,
            LOADING_HUD_FONTS_PROGRESS,
            LOADING_FINAL_PROGRESS,
        ];

        assert!(targets.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(targets.last().copied(), Some(1.0));
    }

    fn replay_data(seed: u64) -> ReplayData {
        ReplayFile {
            header: ReplayHeader {
                mission_id: "Dem_Lei_MP".into(),
                rng_seed: seed,
                sim_config: engine_api::SimConfig::default(),
                version: robin_engine::replay::REPLAY_SCHEMA_VERSION,
                total_frames: 0,
                campaign: bitcode::encode(&Campaign::default()),
            },
            frames: BTreeMap::new(),
            hashes: BTreeMap::new(),
            save_markers: BTreeMap::new(),
            load_backs: BTreeMap::new(),
        }
        .into()
    }

    fn replay_file(contents: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        file.flush().unwrap();
        file
    }

    fn assert_engine_construction_not_reached(args: &crate::main_entry::CliArgs) -> String {
        let constructed = Cell::new(false);
        let result = construct_with_initial_rng_seed(args, Some(0xfeed), |seed| {
            constructed.set(true);
            Ok(seed)
        });
        assert!(!constructed.get());
        result.unwrap_err()
    }

    #[test]
    fn missing_requested_replay_fails_before_engine_construction() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.rhrec.jsonl");
        let args = crate::main_entry::CliArgs {
            replay: Some(missing.to_string_lossy().into_owned()),
            ..Default::default()
        };

        let error = assert_engine_construction_not_reached(&args);

        assert!(error.contains("failed to load requested replay"));
        assert!(error.contains("io:"));
    }

    #[test]
    fn malformed_replay_header_fails_before_engine_construction() {
        let file = replay_file("not a replay header\n");
        let args = crate::main_entry::CliArgs {
            replay: Some(file.path().to_string_lossy().into_owned()),
            ..Default::default()
        };

        let error = assert_engine_construction_not_reached(&args);

        assert!(error.contains("jsonl decode failed: bad header:"));
    }

    #[test]
    fn unsupported_replay_version_fails_before_engine_construction() {
        let file = replay_file(
            r#"{"mission_id":"Dem_Lei_MP","rng_seed":42,"version":999,"total_frames":0,"campaign":null}
"#,
        );
        let args = crate::main_entry::CliArgs {
            replay: Some(file.path().to_string_lossy().into_owned()),
            ..Default::default()
        };

        let error = assert_engine_construction_not_reached(&args);

        assert!(error.contains("unsupported replay schema version 999"));
    }

    #[test]
    fn explicit_replay_data_reaches_engine_construction_with_header_seed() {
        let args = crate::main_entry::CliArgs {
            replay: Some("this path must not be loaded".into()),
            replay_data: Some(replay_data(0xdead_beef)),
            ..Default::default()
        };
        let constructed = Cell::new(false);

        let (constructed_seed, rng_seed) =
            construct_with_initial_rng_seed(&args, Some(0xfeed), |seed| {
                constructed.set(true);
                Ok(seed)
            })
            .unwrap();

        assert!(constructed.get());
        assert_eq!(constructed_seed, 0xdead_beef);
        assert_eq!(rng_seed, 0xdead_beef);
    }
}
