//! Main sound manager.
//!
//! Orchestrates all sound playback: background music, ambient sound sources,
//! sound effects, combat sounds, exclamations, jingles, and dialogue.
//! Uses [`SoundCache`](robin_engine::sound_cache::SoundCache) for sample management,
//! [`SoundGeometry`](robin_engine::sound_geometry::SoundGeometry) for spatial audio,
//! and [`SoundSourceManager`](robin_engine::sound_source::SoundSourceManager) for
//! ambient emitters.

use robin_engine::coordinates::MapPoint;
use robin_engine::sound as engine_sound_kinds;
use serde::{Deserialize, Serialize};

use robin_engine::profiles::{ArmorMaterial, WeaponMaterial};
use robin_engine::sound_cache::{Material, SampleLoader, SoundCache, SoundCacheEntry};
use robin_engine::sound_config::SoundConfig;
use robin_engine::sound_geometry::*;
use robin_engine::sound_source::*;

// ─── Constants ──────────────────────────────────────────────────────

const MUSIC_MODE_WEIGHT: u32 = 128;
const DIALOGUE_ATTENUATION: f32 = 0.3;
/// Default number of concurrent sound-effect channels.
pub const NUM_CHANNELS: u32 = 8;
const EXCLAMATION_VARIANT_NONE: i32 = -1;

#[derive(Debug, Clone, Copy)]
pub struct ResolvedHostExclamation {
    pub actor_id: u32,
    pub identifier: u32,
    pub exclamation_id: u16,
    pub length_ms: u32,
}

// ─── Jingle file table ──────────────────────────────────────────────

const JINGLE_FILES: &[&str] = &[
    "jingle_01.wav", // NewPeasantCalled
    "jingle_02.wav", // MissionWon
    "jingle_03.wav", // MissionLost
    "jingle_04.wav", // CashWon
    "jingle_05.wav", // QuickActionSucceeded
    "jingle_06.wav", // QuickActionFailed
    "jingle_07.wav", // TrapTriggered
    "jingle_08.wav", // PcInComa
];

// ─── Combat FX tables ───────────────────────────────────────────────

const MAX_STRIKE_FX: u32 = 10;
const MAX_IMPACT_FX: u32 = 12;

/// Symmetric strike material table: `[weapon1][weapon2]` → combo index (0–9).
const STRIKE_MATERIAL_TABLE: [[u32; 4]; 4] = [
    //   Wood  Steel CastIron SteelAndWood
    [0, 1, 2, 3], // Wood
    [1, 4, 5, 6], // Steel
    [2, 5, 7, 8], // CastIron
    [3, 6, 8, 9], // SteelAndWood
];

/// Impact material table: `[weapon][armor]` → combo index (0–11).
const IMPACT_MATERIAL_TABLE: [[u32; 3]; 4] = [
    //  Leather Chainmail Plate
    [0, 1, 2],   // Wood
    [3, 4, 5],   // Steel
    [6, 7, 8],   // CastIron
    [9, 10, 11], // SteelAndWood
];

// ─── Enums ──────────────────────────────────────────────────────────

// Sim-side sound classification enums live in robin_engine::sound.
pub(crate) use robin_engine::sound::{ExclamationGroup, ImpactKind, Jingle, MusicMode, StrikeKind};

/// Sound engine operational mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SoundMode {
    Suspended,
    Resumed,
    Menu,
    Mission,
}

/// AI alert status — determines music mood when a song ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertStatus {
    Green,
    Yellow,
    Red,
}

// ─── Audio backend trait ────────────────────────────────────────────

/// Playback intent is independent of asset naming and locale resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackCategory {
    Effect,
    Voice,
    Ambience,
}

fn playback_category(sound_type: SoundType) -> PlaybackCategory {
    match sound_type {
        SoundType::Exclamation | SoundType::Dialog => PlaybackCategory::Voice,
        SoundType::Source => PlaybackCategory::Ambience,
        _ => PlaybackCategory::Effect,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackRequest<'a> {
    pub asset: &'a str,
    pub category: PlaybackCategory,
    pub looping: bool,
    pub fraction: f32,
    pub volume: u16,
    pub spatial_position: Option<[f32; 3]>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum PlaybackError {
    #[error("audio channel capacity exhausted")]
    Capacity,
    #[error("audio asset unavailable: {0}")]
    Asset(String),
    #[error("audio backend failure: {0}")]
    Backend(String),
}

/// Legacy SFX and music intentionally use different unity-gain values.
pub fn channel_gain(volume: u16) -> f32 {
    (f32::from(volume) / 255.0).min(1.0)
}

pub fn music_gain(volume: u16) -> f32 {
    (f32::from(volume) / 128.0).min(1.0)
}

/// Clamp out-of-range offsets consistently across native and browser playback.
pub fn playback_fraction(fraction: f32) -> f32 {
    if !fraction.is_finite() {
        tracing::warn!(fraction, "non-finite audio offset; starting at zero");
        return 0.0;
    }
    fraction.clamp(0.0, 0.999)
}

/// Abstracts audio hardware operations.
///
/// Production uses Kira; tests use a mock that records calls.
/// The backend owns loaded audio resources and manages its own sample cache.
pub trait AudioBackend {
    /// Submit explicit semantic intent. Legacy backends can keep implementing
    /// the channel methods; production browser playback uses the category to
    /// retain delayed dialogue independently of its filename.
    fn play_request(&mut self, request: PlaybackRequest<'_>) -> Option<i32> {
        let asset = request.asset;
        self.try_play_request(request)
            .map_err(|error| match error {
                PlaybackError::Capacity => {
                    tracing::debug!(asset, %error, "audio playback request rejected")
                }
                _ => tracing::warn!(asset, %error, "audio playback request rejected"),
            })
            .ok()
    }

    /// Typed failures distinguish resource resolution, capacity, and device
    /// failures. Browser success reserves ownership; asynchronous decode errors
    /// are subsequently logged and retire that reservation.
    fn try_play_request(&mut self, request: PlaybackRequest<'_>) -> Result<i32, PlaybackError> {
        let channel = if let Some(position) = request.spatial_position {
            self.play_sound_3d(request.asset, request.looping, request.fraction, position)
        } else {
            self.play_sound_at(request.asset, request.looping, request.fraction)
        }
        .ok_or_else(|| PlaybackError::Backend("legacy backend rejected playback".into()))?;
        self.set_channel_volume(channel, request.volume);
        Ok(channel)
    }
    /// Play a sample identified by file name. Returns channel index.
    fn play_sound(&mut self, file_name: &str, looping: bool) -> Option<i32>;
    /// Play at a fractional position \[0.0–1.0) within the sample.
    fn play_sound_at(&mut self, file_name: &str, looping: bool, position: f32) -> Option<i32>;
    /// Stop a channel immediately.
    fn halt_channel(&mut self, channel: i32);
    /// Set a channel's volume \[0–255\].
    fn set_channel_volume(&mut self, channel: i32, volume: u16);
    /// Check if a channel is currently playing.
    fn is_channel_playing(&self, channel: i32) -> bool;
    /// Pause all channels (channel == -1) or a specific one.
    fn pause_channels(&mut self, channel: i32);
    /// Resume all channels (channel == -1) or a specific one.
    fn resume_channels(&mut self, channel: i32);

    /// Load and play a music file. Returns true on success.
    fn play_music(&mut self, path: &str, looping: bool) -> bool;
    /// Stop and free current music.
    fn halt_music(&mut self);
    /// Pause music.
    fn pause_music(&mut self);
    /// Resume music.
    fn resume_music(&mut self);
    /// Store requested music volume; 128 is unity, higher values saturate.
    fn set_music_volume(&mut self, volume: u16);
    /// Get the last requested music volume, including while stopped.
    fn get_music_volume(&self) -> u16;
    /// Returns true if music finished since last check (clears the flag).
    fn take_music_finished(&mut self) -> bool;

    /// Load a WAV file and play it as a jingle on a regular channel.
    fn play_jingle(&mut self, path: &str) -> Option<i32>;
    /// Free jingle resources.
    fn free_jingle(&mut self);

    /// Current time in milliseconds.
    fn get_ticks(&self) -> u32;
    /// Number of available mixing channels.
    fn num_channels(&self) -> u32;

    /// Whether the backend can do positional/3D sound playback.
    ///
    /// Used by the sounds menu to disable the 3D / EAX radio when
    /// the active backend has no spatial mixer.
    fn can_3d_sound(&self) -> bool {
        false
    }

    /// Play a sample on a spatial track at `world_pos` (right-handed
    /// `[x, y, z]` ∈ ~[-1, 1]³ unit-direction from the listener), seeking
    /// `sample_pos` ∈ [0.0, 1.0). Returns the channel index.
    ///
    /// Only invoked when [`Self::can_3d_sound`] is true. The default
    /// impl falls back to [`Self::play_sound_at`], so backends without
    /// spatialisation degrade gracefully.
    fn play_sound_3d(
        &mut self,
        file_name: &str,
        looping: bool,
        sample_pos: f32,
        world_pos: [f32; 3],
    ) -> Option<i32> {
        let _ = world_pos;
        self.play_sound_at(file_name, looping, sample_pos)
    }

    /// Update the spatial position of a 3D-routed channel.
    ///
    /// No-op on backends that don't spatialise. Called when the listen
    /// point changes (camera move) so already-playing sounds re-pan.
    fn set_channel_position_3d(&mut self, channel: i32, world_pos: [f32; 3]) {
        let _ = (channel, world_pos);
    }

    /// Whether the backend can do EAX environmental reverb.
    ///
    /// EAX is a Creative-specific extension that has no kira analogue;
    /// the kira backend returns `false`. Used by the sounds menu to
    /// swap the "EAX" label for "3D" when only positional (non-EAX) 3D
    /// sound is available.
    fn can_eax_sound(&self) -> bool {
        false
    }
}

// ─── SoundManager ───────────────────────────────────────────────────

// SoundSimState now lives in robin_engine::sound (re-exported via
// `crate::sound` stub from engine). Re-export here for callers that
// reach for it via robin_rs::sound::SoundSimState.
pub use engine_sound_kinds::SoundSimState;

/// The historical sound wire value, without backend ownership or frame scratch.
/// Field order and the serde name intentionally match the original manager.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename = "SoundManager")]
pub struct PersistedSoundManager {
    sound_cache: SoundCache,
    geometry_engine: SoundGeometry,
    sound_enabled: bool,
    music_directory: String,
    sound_system_ready: bool,
    active: bool,
    sound_mode: SoundMode,
    use_3d_sound: bool,
    forest_level: bool,
    num_channels: u32,
    music_mode: MusicMode,
    loop_index: i16,
    quiet_mode_weight: u32,
    alert_mode_weight: u32,
    fight_mode_weight: u32,
    load_music: bool,
    start_music: bool,
    dialog_mode: bool,
}

impl PersistedSoundManager {
    /// Direct typed projection: never clones playback queues or backend handles.
    pub fn capture(manager: &SoundManager) -> Self {
        manager.persisted.clone()
    }

    /// Shared disk/replay reconstruction. Historical serde-skipped fields all
    /// defaulted, including dialog_finished=false (unlike a fresh manager).
    pub fn into_runtime(self) -> SoundManager {
        SoundManager {
            persisted: self,
            runtime: SoundRuntime::default(),
        }
    }
}

/// Live host ownership; deliberately absent from the persisted wire value.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SoundRuntime {
    can_3d_sound: bool,
    can_eax_sound: bool,
    channel_info: Vec<ChannelInfo>,
    pending_sounds: Vec<PendingSoundInfo>,
    fx_to_play: Vec<FxToPlay>,
    has_mission_music: bool,
    has_menu_music: bool,
    update_music: bool,
    update_pending_sounds: bool,
    jingle_channel: Option<i32>,
    stop_jingle: bool,
    dialog_finished: bool,
    has_dialog: bool,
    stop_dialog: bool,
    pending_jingle: Option<Jingle>,
}

#[derive(Debug, Clone)]
/// Orchestrates audio through [`AudioBackend`], with separate persisted values
/// and live playback ownership. Authoritative sound timing remains in
/// [`SoundSimState`]. Ordinary cloning preserves live state; persistence does not.
pub struct SoundManager {
    persisted: PersistedSoundManager,
    runtime: SoundRuntime,
}

// Serialize the persisted projection by reference. A serde `into` derive
// would clone the entire manager, including live channel and pending queues.
impl Serialize for SoundManager {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.persisted.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SoundManager {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        PersistedSoundManager::deserialize(deserializer).map(PersistedSoundManager::into_runtime)
    }
}

impl Default for SoundManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SoundManager {
    pub fn new() -> Self {
        Self {
            persisted: PersistedSoundManager {
                sound_cache: SoundCache::new(),
                geometry_engine: SoundGeometry::new(),
                sound_enabled: true,
                music_directory: String::new(),
                sound_system_ready: false,
                active: false,
                sound_mode: SoundMode::Suspended,
                use_3d_sound: false,
                forest_level: false,
                num_channels: 0,
                music_mode: MusicMode::Quiet,
                loop_index: 0,
                quiet_mode_weight: 0,
                alert_mode_weight: 0,
                fight_mode_weight: 0,
                load_music: false,
                start_music: false,
                dialog_mode: false,
            },
            runtime: SoundRuntime {
                dialog_finished: true,
                ..SoundRuntime::default()
            },
        }
    }

    pub fn sound_cache(&self) -> &SoundCache {
        &self.persisted.sound_cache
    }

    pub fn sound_cache_mut(&mut self) -> &mut SoundCache {
        &mut self.persisted.sound_cache
    }

    // ── Accessors ────────────────────────────────────────────────────

    pub fn is_ready(&self) -> bool {
        self.persisted.sound_system_ready
    }

    pub fn is_active(&self) -> bool {
        self.persisted.active
    }

    pub fn music_mode(&self) -> MusicMode {
        self.persisted.music_mode
    }

    pub fn quiet_mode_weight(&self) -> u32 {
        self.persisted.quiet_mode_weight
    }

    pub fn alert_mode_weight(&self) -> u32 {
        self.persisted.alert_mode_weight
    }

    pub fn fight_mode_weight(&self) -> u32 {
        self.persisted.fight_mode_weight
    }

    pub fn is_new_music_starting(&self) -> bool {
        self.persisted.start_music
    }

    pub fn stream_relative_position(&self) -> u16 {
        // The original game's relative stream-position query was left unimplemented and
        // always returned 0; keep DisplayInfo parity until stream timing is
        // intentionally wired through the backend.
        0
    }

    pub fn listen_point(&self) -> MapPoint {
        self.persisted.geometry_engine.listen_point()
    }

    pub fn is_dialog_finished(&self) -> bool {
        if self.persisted.sound_system_ready {
            self.runtime.dialog_finished
        } else {
            true
        }
    }

    pub fn num_pending_sounds(&self) -> usize {
        self.runtime.pending_sounds.len()
    }

    pub fn set_music_directory(&mut self, dir: impl Into<String>) {
        self.persisted.music_directory = dir.into();
    }

    // ── Initialization ───────────────────────────────────────────────

    /// Initialize the sound engine. Call once at startup.
    ///
    /// `want_3d_sound`: the caller requests 3D mode and the engine
    /// grants it only when the backend reports `can_3d_sound() ==
    /// true`. The kira backend never supports 3D today, so passing
    /// `true` here falls back to 2D with a non-fatal warning.
    ///
    /// Returns `Err` only if the request can't be honoured at all (3D
    /// unavailable when explicitly requested). The caller decides
    /// whether to surface that as a warning or hard-fail.
    pub fn initialize(
        &mut self,
        backend: &mut dyn AudioBackend,
        want_3d_sound: bool,
    ) -> Result<(), String> {
        if !self.persisted.sound_enabled {
            self.persisted.sound_system_ready = false;
            return Ok(());
        }

        self.persisted.num_channels = backend.num_channels();
        self.runtime
            .channel_info
            .resize_with(self.persisted.num_channels as usize, ChannelInfo::default);

        // Cache hardware capabilities so the sounds menu and
        // `apply_sound_settings` can gate on them without re-querying
        // the backend.
        self.runtime.can_3d_sound = backend.can_3d_sound();
        self.runtime.can_eax_sound = backend.can_eax_sound();

        self.persisted.sound_system_ready = true;
        self.persisted.active = true;
        // Always force `use_3d_sound = false` before the optional
        // `want_3d_sound` upgrade so a 3D-incapable backend lands in 2D
        // regardless of the request.
        self.persisted.use_3d_sound = false;
        self.persisted.sound_cache.use_3d_sound = false;

        if want_3d_sound {
            if self.runtime.can_3d_sound {
                self.persisted.use_3d_sound = true;
                self.persisted.sound_cache.use_3d_sound = true;
            } else {
                // Log a non-fatal warning and continue in 2D rather
                // than refusing to initialise. Returning `Err` here
                // would prevent the game from starting on any
                // kira-backed install.
                tracing::warn!(
                    "Want to use 3D sound on a backend that doesn't support it; falling back to 2D"
                );
            }
        }

        Ok(())
    }

    /// Whether the active backend can do positional/3D sound.
    ///
    /// Cached at `initialize` time. Used by the sounds menu to disable
    /// the EAX radio on 3D-incapable hardware.
    pub fn can_3d_sound(&self) -> bool {
        self.runtime.can_3d_sound
    }

    /// Whether the active backend can do EAX environmental reverb.
    ///
    /// Cached at `initialize` time. Used by the sounds menu to swap
    /// the EAX radio's label for "3D" when only positional (non-EAX)
    /// 3D is available.
    pub fn can_eax_sound(&self) -> bool {
        self.runtime.can_eax_sound
    }

    /// Apply changed sound options from a [`SoundConfig`].
    ///
    /// kira (the audio backend) does not expose a runtime device
    /// close/open, so the audio device is not torn down on a 3D-sound
    /// or 8-bit toggle. Instead we propagate the toggles that matter
    /// for the Rust pipeline and always push the new volumes.
    ///
    /// Toggle effects:
    /// - `sound_3d`: updates [`SoundManager::use_3d_sound`] +
    ///   [`SoundCache::use_3d_sound`] (the cache stamps this onto each
    ///   sample lookup so spatialised playback parameters match the
    ///   selected mode), and re-runs [`SoundManager::activate`] when the
    ///   sound system was active so source pendings rebuild against the
    ///   new mode. If `sources` is `None` (e.g. the in-menu caller does
    ///   not have the engine's source list at hand), the activate
    ///   round-trip is skipped — flag changes still take effect on the
    ///   next mission load.
    /// - `sound_8bit`: kira does not perform per-sample resampling; we
    ///   log a warning so the divergence is surfaced in dev builds and
    ///   leave the flag for future backend support.
    ///
    /// Returns `true` when anything changed (forces the caller to
    /// persist + re-display).
    pub fn apply_sound_settings(
        &mut self,
        force: bool,
        _backend: &mut dyn AudioBackend,
        config: &SoundConfig,
        sources: Option<&SoundSourceManager>,
    ) -> bool {
        // Honour the backend capability gate: a `sound_3d=true` request
        // on a 2D-only backend has to be clamped here, otherwise the
        // sample cache stamps `use_3d_sound = true` on lookups against
        // a backend that has no 3D pipeline.
        let new_3d = config.sound_3d && self.runtime.can_3d_sound;
        let changed_3d = self.persisted.use_3d_sound != new_3d;

        if (force || changed_3d) && self.persisted.sound_system_ready {
            self.persisted.use_3d_sound = new_3d;
            self.persisted.sound_cache.use_3d_sound = new_3d;

            if config.sound_8bit {
                tracing::warn!(
                    "sound_8bit is set but the kira backend does not implement \
                     per-sample resampling; flag persisted but inactive"
                );
            }

            // Invalidate the cache so the next sample lookup
            // re-resolves under the new mode flag.
            self.persisted.sound_cache.invalidate_cache();

            // We only re-activate here (no deactivate) because
            // `deactivate` requires the source manager mutably and is
            // invoked by the host on mission teardown anyway.
            // Re-activation rebuilds source pendings in the new 3D
            // mode without dropping in-flight channels.
            if self.persisted.active
                && let Some(srcs) = sources
            {
                let forest = self.persisted.forest_level;
                self.persisted.active = false;
                self.activate(forest, srcs);
            }
        }

        // Always re-push volumes.
        self.apply_volumes(config);

        // Without an audio-device re-init there is no failure mode to
        // propagate, so we always return whether something
        // user-visible changed.
        force || changed_3d
    }

    /// Apply volume settings from a [`SoundConfig`].
    pub fn apply_volumes(&mut self, config: &SoundConfig) {
        let att = if self.persisted.dialog_mode {
            DIALOGUE_ATTENUATION
        } else {
            1.0
        };

        self.persisted
            .geometry_engine
            .set_fx_volume(config.fx_volume as f32 / 9.0 * att);
        self.persisted
            .geometry_engine
            .set_music_volume(config.music_volume as f32 / 9.0 * att);
        self.persisted
            .geometry_engine
            .set_exclamation_volume(config.exclamation_volume as f32 / 9.0 * att);
        self.persisted
            .geometry_engine
            .set_dialogue_volume(config.dialogue_volume as f32 / 9.0);

        self.runtime.update_music = true;
        self.runtime.update_pending_sounds = true;
    }

    // ── Listen point ─────────────────────────────────────────────────

    /// Update the listener position and zoom level.
    pub fn set_listen_point(&mut self, position: MapPoint, zoom_level: f32) {
        if self.persisted.geometry_engine.listen_point() != position {
            self.persisted.geometry_engine.set_listen_point(position);
            self.runtime.update_pending_sounds = true;
        }
        if (self.persisted.geometry_engine.zoom_factor() - zoom_level).abs() > f32::EPSILON {
            self.persisted.geometry_engine.set_zoom_factor(zoom_level);
            self.runtime.update_music = true;
            self.runtime.update_pending_sounds = true;
        }
    }

    /// Post-deserialize hook for the save-load entry point.
    ///
    /// After the persisted scalar fields have been restored, re-arm
    /// the engine via `activate` and queue the next hourglass to
    /// (re)load music + (re)resolve pending sounds. Serde derives only
    /// restore field values, so without this hook a loaded
    /// `SoundManager` keeps the default `update_pending_sounds=false`
    /// and music never re-kicks.
    ///
    /// Call this from `GameSaveFile::apply_to` immediately after
    /// `host.audio.sound = self.sound`. `sources` is the just-restored
    /// engine source list (`engine.sound_sim().sources`).
    ///
    /// The kira backend does not expose a byte-offset stream resume, so loaded
    /// saves restart the music loop from the top. This is a known divergence
    /// from the original game.
    pub fn after_load(&mut self, sources: &SoundSourceManager) {
        // Clear transient channel bookkeeping. Typed reconstruction reset
        // most of the heavy state, but `channel_info` was resized once
        // at `initialize` time; re-zero each entry so a stale
        // SoundType from before the load doesn't leak into the new
        // session.
        for info in &mut self.runtime.channel_info {
            *info = ChannelInfo::default();
        }

        // Re-arm pendings against the restored source list. `activate`
        // is idempotent when `sound_system_ready` is false (host
        // re-initialises before this is called only when audio is
        // enabled).
        if self.persisted.sound_system_ready {
            // Avoid double-flagging `load_music` etc.: temporarily
            // clear `active` so `activate` walks the source list and
            // pushes pendings as if from scratch, then re-applies the
            // active flag and clears stop_jingle/stop_dialog.
            self.persisted.active = false;
            self.activate(self.persisted.forest_level, sources);
        }

        // Reset music flags so the next hourglass kicks mission music
        // and re-resolves any restored pendings.
        self.persisted.load_music = false;
        self.persisted.start_music = true;
        self.runtime.update_pending_sounds = true;
    }

    // ── Activation / Deactivation ────────────────────────────────────

    /// Activate the sound engine for a mission.
    pub fn activate(&mut self, forest_level: bool, sources: &SoundSourceManager) {
        if !self.persisted.sound_system_ready {
            return;
        }

        // If the cache validation pre-flight noticed a missing sample
        // (only enabled when the host called
        // `SoundCache::validate_data` before activate), terminate the
        // game with a fatal error. Per the project's "no fake data"
        // rule we panic rather than silently continue — a missing
        // sample at activation means the data shipped with the build
        // is incomplete.
        if !self.persisted.sound_cache.data_check_succeed() {
            panic!("FATAL: Some sound samples are missing!");
        }

        // A non-empty FxToPlay residue at activation means the prior
        // session's queue wasn't drained. Surface it as a warning so
        // dev builds can flag a stuck pipeline.
        if !self.runtime.fx_to_play.is_empty() {
            tracing::warn!(
                count = self.runtime.fx_to_play.len(),
                "Fx to play list not empty at sound activation"
            );
        }

        self.persisted.forest_level = forest_level;

        // Start all currently-active sound sources
        for i in 0..sources.num_sources() {
            if sources.get(i).is_some_and(|s| s.is_effectively_active()) {
                self.start_sound_source_pending(i, sources);
            }
        }

        self.persisted.quiet_mode_weight = 0;
        self.persisted.alert_mode_weight = 0;
        self.persisted.fight_mode_weight = 0;
        self.persisted.load_music = true;
        self.runtime.stop_jingle = false;
        self.runtime.stop_dialog = false;
        self.persisted.active = true;
    }

    /// Deactivate the sound engine.
    pub fn deactivate(
        &mut self,
        clear_data: bool,
        backend: &mut dyn AudioBackend,
        sources: &mut SoundSourceManager,
    ) {
        backend.halt_music();
        self.runtime.has_mission_music = false;
        self.runtime.has_menu_music = false;

        // Stop all non-menu channels
        for i in 0..self.persisted.num_channels as usize {
            if self
                .runtime
                .channel_info
                .get(i)
                .is_some_and(|c| c.sound_type != SoundType::MenuFx)
            {
                self.stop_channel(i as i32, backend);
            }
        }

        if clear_data {
            sources.clear();
            self.persisted.sound_cache.flush(false);
        } else {
            self.suspend_all_sound_sources(backend);
            self.persisted.sound_cache.invalidate_cache();
        }

        self.runtime.pending_sounds.clear();

        for info in &mut self.runtime.channel_info {
            *info = ChannelInfo::default();
        }

        self.persisted.active = false;
    }

    // ── Sound mode ───────────────────────────────────────────────────

    /// Set the operational sound mode (suspended/resumed/menu/mission).
    pub fn set_mode(&mut self, new_mode: SoundMode, backend: &mut dyn AudioBackend) {
        if !self.persisted.sound_system_ready || new_mode == self.persisted.sound_mode {
            return;
        }

        match new_mode {
            SoundMode::Suspended => {
                backend.pause_channels(-1);
                backend.pause_music();
            }
            SoundMode::Resumed => {
                backend.resume_channels(-1);
                backend.resume_music();
            }
            SoundMode::Menu => {
                backend.halt_music();
                self.runtime.has_mission_music = false;

                // Open and play menu music
                let path = format!("{}/Menu.wav", self.persisted.music_directory);
                if backend.play_music(&path, true) {
                    self.runtime.has_menu_music = true;
                }

                // Pause non-menu channels
                for i in 0..self.persisted.num_channels as usize {
                    if self
                        .runtime
                        .channel_info
                        .get(i)
                        .is_some_and(|c| c.sound_type != SoundType::MenuFx)
                    {
                        backend.pause_channels(i as i32);
                    }
                }

                backend.set_music_volume(
                    self.persisted.geometry_engine.get_volume_for_music(false) / 2,
                );
            }
            SoundMode::Mission => {
                // Don't halt menu music here — it would create a silence
                // gap until hourglass() runs and loads mission music. The
                // subsequent play_music() call on the single music track
                // seamlessly replaces the menu stream.
                self.persisted.load_music = true;
                backend.resume_channels(-1);
            }
        }

        self.persisted.sound_mode = new_mode;
    }

    // ── Music mode ───────────────────────────────────────────────────

    /// Adjust music mode weights based on gameplay alerts.
    pub fn set_music_mode(&mut self, mode: MusicMode) {
        let effective = if self.persisted.forest_level && mode == MusicMode::Quiet {
            MusicMode::Alert
        } else {
            mode
        };
        // Escalation already replaces the stream immediately. Retire the old
        // combat weights on de-escalation too, so calm gameplay cannot keep
        // selecting the previous combat pool until its weights decay.
        if effective < self.persisted.music_mode {
            self.force_music_mode(mode);
            return;
        }
        match mode {
            MusicMode::Quiet => {
                if !self.persisted.forest_level {
                    self.persisted.quiet_mode_weight =
                        (self.persisted.quiet_mode_weight + MUSIC_MODE_WEIGHT).min(256);
                    return;
                }
                // Forest levels: Quiet becomes Alert
                self.persisted.alert_mode_weight =
                    (self.persisted.alert_mode_weight + MUSIC_MODE_WEIGHT).min(256);
                if self.persisted.music_mode < MusicMode::Alert {
                    self.persisted.load_music = true;
                }
            }
            MusicMode::Alert => {
                self.persisted.alert_mode_weight =
                    (self.persisted.alert_mode_weight + MUSIC_MODE_WEIGHT).min(256);
                if self.persisted.music_mode < MusicMode::Alert {
                    self.persisted.load_music = true;
                }
            }
            MusicMode::Fight => {
                self.persisted.fight_mode_weight =
                    (self.persisted.fight_mode_weight + MUSIC_MODE_WEIGHT).min(256);
                if self.persisted.music_mode < MusicMode::Fight {
                    self.persisted.load_music = true;
                }
            }
        }
    }

    /// Force the music mode immediately (resets all weights).
    pub fn force_music_mode(&mut self, mode: MusicMode) {
        match mode {
            MusicMode::Quiet => {
                self.persisted.quiet_mode_weight = if self.persisted.forest_level {
                    0
                } else {
                    MUSIC_MODE_WEIGHT
                };
                self.persisted.alert_mode_weight = if self.persisted.forest_level {
                    MUSIC_MODE_WEIGHT
                } else {
                    0
                };
                self.persisted.fight_mode_weight = 0;
            }
            MusicMode::Alert => {
                self.persisted.quiet_mode_weight = 0;
                self.persisted.alert_mode_weight = MUSIC_MODE_WEIGHT;
                self.persisted.fight_mode_weight = 0;
            }
            MusicMode::Fight => {
                self.persisted.quiet_mode_weight = 0;
                self.persisted.alert_mode_weight = 0;
                self.persisted.fight_mode_weight = MUSIC_MODE_WEIGHT;
            }
        }
        self.persisted.load_music = true;
    }

    /// Called when the current music track finishes.
    pub fn on_music_finished(&mut self, alert_status: AlertStatus) {
        match alert_status {
            AlertStatus::Green => self.set_music_mode(MusicMode::Quiet),
            AlertStatus::Yellow => self.set_music_mode(MusicMode::Alert),
            AlertStatus::Red => self.set_music_mode(MusicMode::Fight),
        }
        self.persisted.load_music = true;
    }

    // ── FX playback ──────────────────────────────────────────────────

    /// Queue a strike (parry) sound effect for deferred playback.
    ///
    /// Used when a sword strike is parried.
    pub fn queue_strike_fx(
        &mut self,
        strike_kind: StrikeKind,
        weapon1: WeaponMaterial,
        weapon2: WeaponMaterial,
        position: MapPoint,
    ) {
        if !self.persisted.active {
            return;
        }

        // Variant 0 for deterministic queueing (real playback picks randomly).
        let combo = STRIKE_MATERIAL_TABLE[weapon1 as usize][weapon2 as usize];
        let identifier = (strike_kind as u32 * MAX_STRIKE_FX + combo) * 2;

        let settings = SoundSettings {
            sound_type: SoundType::CombatFx,
            position,
            identifier,
            source: SoundSettingsSource::Position { material: 0 },
        };

        if self
            .persisted
            .geometry_engine
            .get_logical_playing_params(&settings, false)
            .is_some()
        {
            self.runtime.pending_sounds.push(PendingSoundInfo {
                settings,
                channel: PendingChannel::Queued,
                start_time_ms: 0,
                length_ms: 0,
                speech_variant: None,
                source_index: None,
                actor_id: None,
            });
        }
    }

    /// Queue an impact sound effect for deferred playback.
    ///
    /// Unlike [`play_impact_fx`](Self::play_impact_fx) this does NOT need
    /// the audio backend — it pushes a `PendingSoundInfo` that will be
    /// resolved during the next [`hourglass`](Self::hourglass) call.
    /// Used by the combat system which runs inside `Engine` where the
    /// backend is not available.
    pub fn queue_impact_fx(
        &mut self,
        impact_kind: ImpactKind,
        weapon: WeaponMaterial,
        armor: ArmorMaterial,
        position: MapPoint,
    ) {
        if !self.persisted.active {
            return;
        }

        // Same 30-byte offset as play_impact_fx — see comment there.
        let identifier = 3 * MAX_STRIKE_FX
            + impact_kind as u32 * MAX_IMPACT_FX
            + IMPACT_MATERIAL_TABLE[weapon as usize][armor as usize];

        let settings = SoundSettings {
            sound_type: SoundType::CombatFx,
            position,
            identifier,
            source: SoundSettingsSource::Position { material: 0 },
        };

        if self
            .persisted
            .geometry_engine
            .get_logical_playing_params(&settings, false)
            .is_some()
        {
            self.runtime.pending_sounds.push(PendingSoundInfo {
                settings,
                channel: PendingChannel::Queued,
                start_time_ms: 0,
                length_ms: 0,
                speech_variant: None,
                source_index: None,
                actor_id: None,
            });
        }
    }

    /// Queue a generic sound effect by raw FX identifier for deferred playback.
    ///
    /// Used for projectile impacts, animation-frame sound triggers,
    /// and other FX. `material` is `Some(m)` for actor/projectile
    /// footsteps and cloth sounds (so the bank picks the right
    /// material variant); `None` for surface-independent FX
    /// (explosions, bell rings, etc).
    pub fn queue_fx(&mut self, fx_id: u32, position: MapPoint, material: Option<Material>) {
        if !self.persisted.active {
            return;
        }

        let settings = SoundSettings {
            sound_type: SoundType::Fx,
            position,
            identifier: fx_id,
            source: SoundSettingsSource::Position {
                material: material.map_or(Material::NUM_MATERIALS as u8, |m| m as u8),
            },
        };

        if self
            .persisted
            .geometry_engine
            .get_logical_playing_params(&settings, false)
            .is_some()
        {
            self.runtime.pending_sounds.push(PendingSoundInfo {
                settings,
                channel: PendingChannel::Queued,
                start_time_ms: 0,
                length_ms: 0,
                speech_variant: None,
                source_index: None,
                actor_id: None,
            });
        }
    }

    // ── Jingle management ────────────────────────────────────────────

    /// Play a jingle sound effect.
    pub fn play_jingle(&mut self, jingle: Jingle, backend: &mut dyn AudioBackend) {
        if !self.persisted.sound_system_ready {
            return;
        }

        let path = format!("Data/Sounds/{}", JINGLE_FILES[jingle as usize]);

        if jingle == Jingle::MissionWon || jingle == Jingle::MissionLost {
            backend.halt_music();
            self.runtime.has_mission_music = false;
            self.persisted.load_music = false;
            self.persisted.start_music = false;
        }

        if let Some(channel) = backend.play_jingle(&path) {
            self.runtime.jingle_channel = Some(channel);
            backend.set_channel_volume(
                channel,
                self.persisted.geometry_engine.get_volume_for_jingle() / 2,
            );
            self.update_channel_info(channel, SoundType::Jingle, None, None);
        } else {
            tracing::error!(path = %path, "unable to play jingle (see backend diagnostic)");
        }
    }

    // ── Menu sound ───────────────────────────────────────────────────

    /// Play a menu UI sound.
    pub fn play_menu_sound(
        &mut self,
        menu_sound_id: u32,
        backend: &mut dyn AudioBackend,
        loader: &SampleLoader,
    ) -> bool {
        if !self.persisted.sound_system_ready {
            return true;
        }

        let file_name = {
            let entry = self
                .persisted
                .sound_cache
                .get_menu_sample(menu_sound_id, loader);
            match entry {
                Some(e) if e.is_loaded() => e.file_name.clone(),
                _ => return false,
            }
        };

        if let Some(channel) = backend.play_sound(&file_name, false) {
            backend.set_channel_volume(channel, self.persisted.geometry_engine.fx_volume_byte());
            self.update_channel_info(
                channel,
                SoundType::MenuFx,
                Some(CacheKey::Menu(menu_sound_id)),
                None,
            );
            true
        } else {
            false
        }
    }

    /// Queue a jingle for playback in the next hourglass tick.
    /// Used when the audio backend isn't available at the call site
    /// (e.g. script command dispatch).
    pub fn queue_jingle(&mut self, jingle: Jingle) {
        self.runtime.pending_jingle = Some(jingle);
    }

    // ── Dialogue management ──────────────────────────────────────────

    /// Enter dialogue mode (attenuates non-dialogue volumes).
    pub fn enter_dialogue(&mut self, config: &SoundConfig) {
        self.persisted.dialog_mode = true;
        self.apply_volumes(config);
    }

    /// Leave dialogue mode (restores normal volumes).
    pub fn leave_dialogue(&mut self, config: &SoundConfig) {
        self.persisted.dialog_mode = false;
        self.apply_volumes(config);
    }

    /// Play a dialogue WAV file as a music stream.
    pub fn play_dialog(&mut self, file_path: &str, backend: &mut dyn AudioBackend) {
        if !self.persisted.sound_system_ready {
            return;
        }

        backend.halt_music();
        self.runtime.has_mission_music = false;
        self.runtime.dialog_finished = true;

        if backend.play_music(file_path, false) {
            backend.set_music_volume(self.persisted.geometry_engine.get_volume_for_dialogue() / 2);
            self.runtime.dialog_finished = false;
            self.runtime.has_dialog = true;
        }
    }

    pub fn close_dialog(&mut self, backend: &mut dyn AudioBackend) {
        if self.persisted.sound_system_ready {
            backend.halt_music();
            self.runtime.has_dialog = false;
        }
    }

    pub fn get_dialog_volume(&self, backend: &dyn AudioBackend) -> f32 {
        if self.persisted.sound_system_ready {
            backend.get_music_volume() as f32 * 2.0
        } else {
            0.0
        }
    }

    // ── Sound source management ──────────────────────────────────────

    /// Resume all active sound sources after a pause.
    pub fn resume_all_sound_sources(
        &mut self,
        sources: &SoundSourceManager,
        position: MapPoint,
        zoom: f32,
    ) {
        self.set_listen_point(position, zoom);

        for i in 0..sources.num_sources() {
            let should_start = sources.get(i).is_some_and(|s| s.is_effectively_active())
                && !self.is_source_pending(i);
            if should_start {
                self.start_sound_source_pending(i, sources);
            }
        }
    }

    /// Suspend all sound sources (stop and remove from pending list).
    pub fn suspend_all_sound_sources(&mut self, backend: &mut dyn AudioBackend) {
        let channels_to_stop: Vec<i32> = self
            .runtime
            .pending_sounds
            .iter()
            .filter(|p| p.settings.sound_type == SoundType::Source)
            .filter_map(|p| p.channel.assigned())
            .collect();

        for ch in channels_to_stop {
            self.stop_channel(ch, backend);
        }

        self.runtime
            .pending_sounds
            .retain(|p| p.settings.sound_type != SoundType::Source);
    }

    /// Kick the host audio backend to start a channel for a newly
    /// activated sound source.  The sim owns the `active` flag and has
    /// already flipped it to `true` inside `perform_hourglass`
    /// (paired with the deactivate side); host reads, not writes.
    pub fn activate_sound_source(&mut self, sources: &SoundSourceManager, index: usize) {
        if sources
            .get(index)
            .is_some_and(|s| s.is_effectively_active())
        {
            self.start_sound_source_pending(index, sources);
        }
    }

    /// Stop sources excluded by the new ambience and queue newly included
    /// active sources. Script activation remains independent of this gate.
    pub fn sync_ambience_sources(
        &mut self,
        sources: &SoundSourceManager,
        backend: &mut dyn AudioBackend,
    ) {
        let channels_to_stop: Vec<_> = self
            .runtime
            .pending_sounds
            .iter()
            .filter(|pending| {
                pending.settings.sound_type == SoundType::Source
                    && pending.source_index.is_some_and(|index| {
                        !sources
                            .get(index)
                            .is_some_and(|source| source.is_effectively_active())
                    })
            })
            .filter_map(|pending| pending.channel.assigned())
            .collect();
        for channel in channels_to_stop {
            self.stop_channel(channel, backend);
        }
        self.runtime.pending_sounds.retain(|pending| {
            pending.settings.sound_type != SoundType::Source
                || pending.source_index.is_some_and(|index| {
                    sources
                        .get(index)
                        .is_some_and(|source| source.is_effectively_active())
                })
        });
        for index in 0..sources.num_sources() {
            if sources
                .get(index)
                .is_some_and(|source| source.is_effectively_active())
                && !self.is_source_pending(index)
            {
                self.start_sound_source_pending(index, sources);
            }
        }
    }

    // ── Private helpers ──────────────────────────────────────────────

    /// Select the current music loop name from the appropriate pool.
    fn select_music_loop(&self) -> Option<String> {
        let pool = match self.persisted.music_mode {
            MusicMode::Quiet => &self.persisted.sound_cache.quiet_music_pool,
            MusicMode::Alert => &self.persisted.sound_cache.alert_music_pool,
            MusicMode::Fight => &self.persisted.sound_cache.fight_music_pool,
        };
        if pool.is_empty() {
            None
        } else {
            Some(pool[self.persisted.loop_index as usize % pool.len()].clone())
        }
    }
}

// ─── Free functions ─────────────────────────────────────────────────

/// Compute time elapsed between `start` and `now`, handling 32-bit wrap.
fn time_elapsed(start: u32, now: u32) -> u32 {
    now.wrapping_sub(start)
}

/// Convert a raw `u8` to a [`Material`] enum, or `None` if out of range.
fn material_from_u8(v: u8) -> Option<Material> {
    match v {
        0 => Some(Material::Ground),
        1 => Some(Material::Wood),
        2 => Some(Material::Stone),
        3 => Some(Material::Grass),
        4 => Some(Material::Leaves),
        5 => Some(Material::Water),
        6 => Some(Material::Bush),
        7 => Some(Material::Ice),
        8 => Some(Material::Hole),
        _ => None,
    }
}

// ─── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;

mod channels;
use channels::*;
mod exclamations;
