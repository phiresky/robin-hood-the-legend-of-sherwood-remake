//! Main sound manager.
//!
//! Orchestrates all sound playback: background music, ambient sound sources,
//! sound effects, combat sounds, exclamations, jingles, and dialogue.
//! Uses [`SoundCache`](crate::sound_cache::SoundCache) for sample management,
//! [`SoundGeometry`](crate::sound_geometry::SoundGeometry) for spatial audio,
//! and [`SoundSourceManager`](crate::sound_source::SoundSourceManager) for
//! ambient emitters.

use serde::{Deserialize, Serialize};

use crate::geo2d::{self, GeoPoint2D};
use crate::profiles::{ArmorMaterial, WeaponMaterial};
use crate::sound_cache::{Material, SampleLoader, SoundCache};
use crate::sound_config::SoundConfig;
use crate::sound_geometry::*;
use crate::sound_source::*;

// ─── Constants ──────────────────────────────────────────────────────

const MUSIC_MODE_WEIGHT: u32 = 128;
const DIALOGUE_ATTENUATION: f32 = 0.3;
/// FX shorter than this (ms) are played immediately; longer ones are pending.
const SAMPLE_LENGTH_POSITIONING_THRESHOLD: u32 = 800;
/// Default number of mixing channels (matches SDL_mixer setup).
pub const NUM_CHANNELS: u32 = 8;
const EXCLAMATION_VARIANT_NONE: i32 = -1;

/// Channel index sentinel: no channel assigned.
const CHANNEL_NONE: i32 = -1;
/// Channel index sentinel: sound should be played at next opportunity.
const CHANNEL_TO_PLAY: i32 = -2;

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

// Sim-side sound classification enums live in robin_engine::sound_kinds.
pub use robin_engine::sound_kinds::{ExclamationGroup, ImpactKind, Jingle, MusicMode, StrikeKind};

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

// ─── Channel tracking ───────────────────────────────────────────────

/// Identifies which cache sub-system a channel's sound comes from,
/// so we can update `playing` counts when the channel stops.
#[derive(Debug, Clone)]
enum CacheKey {
    FxIndex(usize),
    CombatFx(u32),
    Source(u32),
    SpeechIndex(usize),
    Menu(u32),
}

/// Per-channel bookkeeping.
///
/// `actor_id` is the actor identifier used by [`SoundManager::stop_exclamation`].
#[derive(Debug, Clone)]
struct ChannelInfo {
    sound_type: SoundType,
    cache_key: Option<CacheKey>,
    /// For [`SoundType::Exclamation`]: actor identifier for stop lookups.
    actor_id: Option<u32>,
}

impl Default for ChannelInfo {
    fn default() -> Self {
        Self {
            sound_type: SoundType::None,
            cache_key: None,
            actor_id: None,
        }
    }
}

// ─── Pending sound ──────────────────────────────────────────────────

/// A sound playing or waiting to play, tracked over time.
#[derive(Debug, Clone)]
struct PendingSoundInfo {
    settings: SoundSettings,
    /// Current channel: [`CHANNEL_NONE`], [`CHANNEL_TO_PLAY`], or a real index.
    channel: i32,
    /// Timestamp (ms) when the sound started.
    start_time_ms: u32,
    /// Duration of the sample (ms). 0 = not yet determined.
    length_ms: u32,
    /// For exclamations: actor identifier (for AI callback on finish).
    actor_id: Option<u32>,
    /// For source sounds: index into the source manager.
    source_index: Option<usize>,
    /// Speech variant for exclamation cache lookups.
    speech_variant: Option<u32>,
    /// Host-only resolved sample metadata for random exclamations.
    resolved_entry: Option<CacheEntryInfo>,
}

/// A short FX queued to be played in the current frame's [`SoundManager::hourglass`].
#[derive(Debug, Clone)]
struct FxToPlay {
    settings: SoundSettings,
    params: PlayingParameters,
}

// ─── Cache entry info (extracted to avoid borrow conflicts) ─────────

/// Extracted cache entry metadata, owned (no borrow on the cache).
#[derive(Debug, Clone)]
struct CacheEntryInfo {
    file_name: String,
    sample_length_ms: u32,
    loop_sample: bool,
    cache_key: Option<CacheKey>,
}

// ─── Audio backend trait ────────────────────────────────────────────

/// Abstracts audio hardware operations.
///
/// In production this wraps SDL\_mixer; in tests, a mock that records calls.
/// The backend owns loaded audio resources and manages its own sample cache.
pub trait AudioBackend {
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
    /// Set music volume \[0–255\].
    fn set_music_volume(&mut self, volume: u16);
    /// Get music volume \[0–255\].
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

// SoundSimState now lives in robin_engine::sound_kinds (re-exported via
// `crate::sound` stub from engine). Re-export here for callers that
// reach for it via robin_rs::sound::SoundSimState.
pub use robin_engine::sound_kinds::SoundSimState;

/// Main sound manager. Orchestrates all audio in the game.
///
/// Owns the sound cache, geometry engine, and sound-related host
/// state. Audio hardware is accessed through
/// [`AudioBackend`]. The sim-state portion (source list, finished
/// exclamations) lives on [`SoundSimState`] so it survives rollback
/// snapshots that reset `HostState` via `Clone → Default`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoundManager {
    // ── Sub-systems ──
    pub sound_cache: SoundCache,
    pub geometry_engine: SoundGeometry,

    // ── Configuration ──
    sound_enabled: bool,
    music_directory: String,
    sound_system_ready: bool,
    active: bool,
    sound_mode: SoundMode,
    use_3d_sound: bool,
    /// Backend-reported "can do positional/3D sound" capability.
    /// Cached at `initialize` time; the sounds menu reads it via
    /// [`SoundManager::can_3d_sound`] to gate the EAX radio.
    #[serde(skip)]
    can_3d_sound: bool,
    /// Backend-reported "can do EAX environmental reverb" capability.
    /// Cached at `initialize`; the sounds menu reads it via
    /// [`SoundManager::can_eax_sound`] to choose between the "EAX" and
    /// "3D" label.
    #[serde(skip)]
    can_eax_sound: bool,
    forest_level: bool,

    // ── Channel tracking ──
    num_channels: u32,
    #[serde(skip)]
    channel_info: Vec<ChannelInfo>,

    // ── Pending sounds ──
    #[serde(skip)]
    pending_sounds: Vec<PendingSoundInfo>,
    #[serde(skip)]
    fx_to_play: Vec<FxToPlay>,

    // ── Music state ──
    music_mode: MusicMode,
    loop_index: i16,
    quiet_mode_weight: u32,
    alert_mode_weight: u32,
    fight_mode_weight: u32,

    // ── Flags ──
    #[serde(skip)]
    has_mission_music: bool,
    #[serde(skip)]
    has_menu_music: bool,
    load_music: bool,
    start_music: bool,
    #[serde(skip)]
    update_music: bool,
    #[serde(skip)]
    update_pending_sounds: bool,

    // ── Jingle / Dialog ──
    #[serde(skip)]
    jingle_channel: i32,
    #[serde(skip)]
    stop_jingle: bool,
    dialog_mode: bool,
    #[serde(skip)]
    dialog_finished: bool,
    #[serde(skip)]
    has_dialog: bool,
    #[serde(skip)]
    stop_dialog: bool,

    // ── Deferred jingle ──
    #[serde(skip)]
    pending_jingle: Option<Jingle>,
}

impl Default for SoundManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SoundManager {
    pub fn new() -> Self {
        Self {
            sound_cache: SoundCache::new(),
            geometry_engine: SoundGeometry::new(),

            sound_enabled: true,
            music_directory: String::new(),
            sound_system_ready: false,
            active: false,
            sound_mode: SoundMode::Suspended,
            use_3d_sound: false,
            can_3d_sound: false,
            can_eax_sound: false,
            forest_level: false,

            num_channels: 0,
            channel_info: Vec::new(),

            pending_sounds: Vec::new(),
            fx_to_play: Vec::new(),

            music_mode: MusicMode::Quiet,
            loop_index: 0,
            quiet_mode_weight: 0,
            alert_mode_weight: 0,
            fight_mode_weight: 0,

            has_mission_music: false,
            has_menu_music: false,
            load_music: false,
            start_music: false,
            update_music: false,
            update_pending_sounds: false,

            jingle_channel: -1,
            stop_jingle: false,
            dialog_mode: false,
            dialog_finished: true,
            has_dialog: false,
            stop_dialog: false,

            pending_jingle: None,
        }
    }

    // ── Accessors ────────────────────────────────────────────────────

    pub fn is_ready(&self) -> bool {
        self.sound_system_ready
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn music_mode(&self) -> MusicMode {
        self.music_mode
    }

    pub fn quiet_mode_weight(&self) -> u32 {
        self.quiet_mode_weight
    }

    pub fn alert_mode_weight(&self) -> u32 {
        self.alert_mode_weight
    }

    pub fn fight_mode_weight(&self) -> u32 {
        self.fight_mode_weight
    }

    pub fn is_new_music_starting(&self) -> bool {
        self.start_music
    }

    pub fn stream_relative_position(&self) -> u16 {
        // C++ RHSound::GetStreamRelativePosition was left unimplemented and
        // always returned 0; keep DisplayInfo parity until stream timing is
        // intentionally wired through the backend.
        0
    }

    pub fn listen_point(&self) -> GeoPoint2D {
        self.geometry_engine.listen_point()
    }

    pub fn is_dialog_finished(&self) -> bool {
        if self.sound_system_ready {
            self.dialog_finished
        } else {
            true
        }
    }

    pub fn num_pending_sounds(&self) -> usize {
        self.pending_sounds.len()
    }

    pub fn set_music_directory(&mut self, dir: impl Into<String>) {
        self.music_directory = dir.into();
    }

    pub fn set_sound_enabled(&mut self, enabled: bool) {
        self.sound_enabled = enabled;
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
        if !self.sound_enabled {
            self.sound_system_ready = false;
            return Ok(());
        }

        self.num_channels = backend.num_channels();
        self.channel_info
            .resize_with(self.num_channels as usize, ChannelInfo::default);

        // Cache hardware capabilities so the sounds menu and
        // `apply_sound_settings` can gate on them without re-querying
        // the backend.
        self.can_3d_sound = backend.can_3d_sound();
        self.can_eax_sound = backend.can_eax_sound();

        self.sound_system_ready = true;
        self.active = true;
        // Always force `use_3d_sound = false` before the optional
        // `want_3d_sound` upgrade so a 3D-incapable backend lands in 2D
        // regardless of the request.
        self.use_3d_sound = false;
        self.sound_cache.use_3d_sound = false;

        if want_3d_sound {
            if self.can_3d_sound {
                self.use_3d_sound = true;
                self.sound_cache.use_3d_sound = true;
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
        self.can_3d_sound
    }

    /// Whether the active backend can do EAX environmental reverb.
    ///
    /// Cached at `initialize` time. Used by the sounds menu to swap
    /// the EAX radio's label for "3D" when only positional (non-EAX)
    /// 3D is available.
    pub fn can_eax_sound(&self) -> bool {
        self.can_eax_sound
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
        let new_3d = config.sound_3d && self.can_3d_sound;
        let changed_3d = self.use_3d_sound != new_3d;

        if (force || changed_3d) && self.sound_system_ready {
            self.use_3d_sound = new_3d;
            self.sound_cache.use_3d_sound = new_3d;

            if config.sound_8bit {
                tracing::warn!(
                    "sound_8bit is set but the kira backend does not implement \
                     per-sample resampling; flag persisted but inactive"
                );
            }

            // Invalidate the cache so the next sample lookup
            // re-resolves under the new mode flag.
            self.sound_cache.invalidate_cache();

            // We only re-activate here (no deactivate) because
            // `deactivate` requires the source manager mutably and is
            // invoked by the host on mission teardown anyway.
            // Re-activation rebuilds source pendings in the new 3D
            // mode without dropping in-flight channels.
            if self.active
                && let Some(srcs) = sources
            {
                let forest = self.forest_level;
                self.active = false;
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
        let att = if self.dialog_mode {
            DIALOGUE_ATTENUATION
        } else {
            1.0
        };

        self.geometry_engine
            .set_fx_volume(config.fx_volume as f32 / 9.0 * att);
        self.geometry_engine
            .set_music_volume(config.music_volume as f32 / 9.0 * att);
        self.geometry_engine
            .set_exclamation_volume(config.exclamation_volume as f32 / 9.0 * att);
        self.geometry_engine
            .set_dialogue_volume(config.dialogue_volume as f32 / 9.0);

        self.update_music = true;
        self.update_pending_sounds = true;
    }

    // ── Listen point ─────────────────────────────────────────────────

    /// Update the listener position and zoom level.
    pub fn set_listen_point(&mut self, position: GeoPoint2D, zoom_level: f32) {
        if self.geometry_engine.listen_point() != position {
            self.geometry_engine.set_listen_point(position);
            self.update_pending_sounds = true;
        }
        if (self.geometry_engine.zoom_factor() - zoom_level).abs() > f32::EPSILON {
            self.geometry_engine.set_zoom_factor(zoom_level);
            self.update_music = true;
            self.update_pending_sounds = true;
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
    /// `host.sound = self.sound`. `sources` is the just-restored
    /// engine source list (`engine.sound_sim().sources`).
    ///
    /// The kira backend does not expose a byte-offset stream resume,
    /// so loaded saves restart the music loop from the top. Tracked
    /// separately as a known divergence in `parity-audit/RHsound-14.md`.
    pub fn after_load(&mut self, sources: &SoundSourceManager) {
        // Clear transient channel bookkeeping. `#[serde(skip)]` reset
        // most of the heavy state, but `channel_info` was resized once
        // at `initialize` time; re-zero each entry so a stale
        // SoundType from before the load doesn't leak into the new
        // session.
        for info in &mut self.channel_info {
            *info = ChannelInfo::default();
        }

        // Re-arm pendings against the restored source list. `activate`
        // is idempotent when `sound_system_ready` is false (host
        // re-initialises before this is called only when audio is
        // enabled).
        if self.sound_system_ready {
            // Avoid double-flagging `load_music` etc.: temporarily
            // clear `active` so `activate` walks the source list and
            // pushes pendings as if from scratch, then re-applies the
            // active flag and clears stop_jingle/stop_dialog.
            self.active = false;
            self.activate(self.forest_level, sources);
        }

        // Reset music flags so the next hourglass kicks mission music
        // and re-resolves any restored pendings.
        self.load_music = false;
        self.start_music = true;
        self.update_pending_sounds = true;
    }

    // ── Activation / Deactivation ────────────────────────────────────

    /// Activate the sound engine for a mission.
    pub fn activate(&mut self, forest_level: bool, sources: &SoundSourceManager) {
        if !self.sound_system_ready {
            return;
        }

        // If the cache validation pre-flight noticed a missing sample
        // (only enabled when the host called
        // `SoundCache::validate_data` before activate), terminate the
        // game with a fatal error. Per the project's "no fake data"
        // rule we panic rather than silently continue — a missing
        // sample at activation means the data shipped with the build
        // is incomplete.
        if !self.sound_cache.data_check_succeed() {
            panic!("FATAL: RHSound: Some samples are missing!");
        }

        // A non-empty FxToPlay residue at activation means the prior
        // session's queue wasn't drained. Surface it as a warning so
        // dev builds can flag a stuck pipeline.
        if !self.fx_to_play.is_empty() {
            tracing::warn!(
                count = self.fx_to_play.len(),
                "Fx to play list not empty at sound activation"
            );
        }

        self.forest_level = forest_level;

        // Start all currently-active sound sources
        for i in 0..sources.num_sources() {
            if sources.get(i).is_some_and(|s| s.active) {
                self.start_sound_source_pending(i, sources);
            }
        }

        self.quiet_mode_weight = 0;
        self.alert_mode_weight = 0;
        self.fight_mode_weight = 0;
        self.load_music = true;
        self.stop_jingle = false;
        self.stop_dialog = false;
        self.active = true;
    }

    /// Deactivate the sound engine.
    pub fn deactivate(
        &mut self,
        clear_data: bool,
        backend: &mut dyn AudioBackend,
        sources: &mut SoundSourceManager,
    ) {
        backend.halt_music();
        self.has_mission_music = false;
        self.has_menu_music = false;

        // Stop all non-menu channels
        for i in 0..self.num_channels as usize {
            if self
                .channel_info
                .get(i)
                .is_some_and(|c| c.sound_type != SoundType::MenuFx)
            {
                self.stop_channel(i as i32, backend);
            }
        }

        if clear_data {
            sources.clear();
            self.sound_cache.flush(false);
        } else {
            self.suspend_all_sound_sources(backend);
            self.sound_cache.invalidate_cache();
        }

        self.pending_sounds.clear();

        for info in &mut self.channel_info {
            *info = ChannelInfo::default();
        }

        self.active = false;
    }

    // ── Sound mode ───────────────────────────────────────────────────

    /// Set the operational sound mode (suspended/resumed/menu/mission).
    pub fn set_mode(&mut self, new_mode: SoundMode, backend: &mut dyn AudioBackend) {
        if !self.sound_system_ready || new_mode == self.sound_mode {
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
                self.has_mission_music = false;

                // Open and play menu music
                let path = format!("{}/Menu.wav", self.music_directory);
                if backend.play_music(&path, true) {
                    self.has_menu_music = true;
                }

                // Pause non-menu channels
                for i in 0..self.num_channels as usize {
                    if self
                        .channel_info
                        .get(i)
                        .is_some_and(|c| c.sound_type != SoundType::MenuFx)
                    {
                        backend.pause_channels(i as i32);
                    }
                }

                backend.set_music_volume(self.geometry_engine.get_volume_for_music(false) / 2);
            }
            SoundMode::Mission => {
                // Don't halt menu music here — it would create a silence
                // gap until hourglass() runs and loads mission music. The
                // subsequent play_music() call on the single music track
                // seamlessly replaces the menu stream.
                self.load_music = true;
                backend.resume_channels(-1);
            }
        }

        self.sound_mode = new_mode;
    }

    // ── Music mode ───────────────────────────────────────────────────

    /// Adjust music mode weights based on gameplay alerts.
    pub fn set_music_mode(&mut self, mode: MusicMode) {
        match mode {
            MusicMode::Quiet => {
                if !self.forest_level {
                    self.quiet_mode_weight = (self.quiet_mode_weight + MUSIC_MODE_WEIGHT).min(256);
                    return;
                }
                // Forest levels: Quiet becomes Alert
                self.alert_mode_weight = (self.alert_mode_weight + MUSIC_MODE_WEIGHT).min(256);
                if self.music_mode < MusicMode::Alert {
                    self.load_music = true;
                }
            }
            MusicMode::Alert => {
                self.alert_mode_weight = (self.alert_mode_weight + MUSIC_MODE_WEIGHT).min(256);
                if self.music_mode < MusicMode::Alert {
                    self.load_music = true;
                }
            }
            MusicMode::Fight => {
                self.fight_mode_weight = (self.fight_mode_weight + MUSIC_MODE_WEIGHT).min(256);
                if self.music_mode < MusicMode::Fight {
                    self.load_music = true;
                }
            }
        }
    }

    /// Force the music mode immediately (resets all weights).
    pub fn force_music_mode(&mut self, mode: MusicMode) {
        match mode {
            MusicMode::Quiet => {
                self.quiet_mode_weight = if self.forest_level {
                    0
                } else {
                    MUSIC_MODE_WEIGHT
                };
                self.alert_mode_weight = if self.forest_level {
                    MUSIC_MODE_WEIGHT
                } else {
                    0
                };
                self.fight_mode_weight = 0;
            }
            MusicMode::Alert => {
                self.quiet_mode_weight = 0;
                self.alert_mode_weight = MUSIC_MODE_WEIGHT;
                self.fight_mode_weight = 0;
            }
            MusicMode::Fight => {
                self.quiet_mode_weight = 0;
                self.alert_mode_weight = 0;
                self.fight_mode_weight = MUSIC_MODE_WEIGHT;
            }
        }
        self.load_music = true;
    }

    /// Called when the current music track finishes.
    pub fn on_music_finished(&mut self, alert_status: AlertStatus) {
        match alert_status {
            AlertStatus::Green => self.set_music_mode(MusicMode::Quiet),
            AlertStatus::Yellow => self.set_music_mode(MusicMode::Alert),
            AlertStatus::Red => self.set_music_mode(MusicMode::Fight),
        }
        self.load_music = true;
    }

    // ── FX playback ──────────────────────────────────────────────────

    /// Queue a sound effect for playback. Actual playback happens in [`hourglass`](Self::hourglass).
    pub fn play_fx(
        &mut self,
        fx_id: u32,
        position: GeoPoint2D,
        material: Option<Material>,
        loader: &SampleLoader,
        rng: &mut dyn FnMut(u32) -> u32,
        sources: &SoundSourceManager,
    ) -> bool {
        if !self.active {
            return true;
        }

        let settings = SoundSettings {
            sound_type: SoundType::Fx,
            position,
            identifier: fx_id,
            source: SoundSettingsSource::Position {
                material: material.map_or(Material::NUM_MATERIALS as u8, |m| m as u8),
            },
        };

        let length_ms = self.get_sample_length_ms(&settings, loader, rng, sources);
        if length_ms == 0 {
            return false;
        }

        if length_ms < SAMPLE_LENGTH_POSITIONING_THRESHOLD {
            let is_material = self.sound_cache.is_material_fx(fx_id);
            if let Some(params) = self
                .geometry_engine
                .get_logical_playing_params(&settings, is_material)
            {
                self.fx_to_play.push(FxToPlay { settings, params });
            }
        } else {
            self.pending_sounds.push(PendingSoundInfo {
                settings,
                channel: CHANNEL_TO_PLAY,
                start_time_ms: 0,
                length_ms: 0,
                actor_id: None,
                source_index: None,
                speech_variant: None,
                resolved_entry: None,
            });
        }

        true
    }

    /// Play a strike combat FX.
    #[allow(clippy::too_many_arguments)]
    pub fn play_strike_fx(
        &mut self,
        strike_kind: StrikeKind,
        weapon1: WeaponMaterial,
        weapon2: WeaponMaterial,
        position: GeoPoint2D,
        backend: &mut dyn AudioBackend,
        loader: &SampleLoader,
        rng: &mut dyn FnMut(u32) -> u32,
        sources: &SoundSourceManager,
    ) -> bool {
        if !self.active {
            return true;
        }

        let variant = rng(2);
        let combo = STRIKE_MATERIAL_TABLE[weapon1 as usize][weapon2 as usize];
        let identifier = (strike_kind as u32 * MAX_STRIKE_FX + combo) * 2 + variant;

        let settings = SoundSettings {
            sound_type: SoundType::CombatFx,
            position,
            identifier,
            source: SoundSettingsSource::Position { material: 0 },
        };

        if let Some(params) = self
            .geometry_engine
            .get_logical_playing_params(&settings, false)
        {
            self.play_sound_now(&settings, &params, backend, loader, rng, sources);
            return true;
        }
        false
    }

    /// Play an impact combat FX.
    #[allow(clippy::too_many_arguments)]
    pub fn play_impact_fx(
        &mut self,
        impact_kind: ImpactKind,
        weapon: WeaponMaterial,
        armor: ArmorMaterial,
        position: GeoPoint2D,
        backend: &mut dyn AudioBackend,
        loader: &SampleLoader,
        rng: &mut dyn FnMut(u32) -> u32,
        sources: &SoundSourceManager,
    ) -> bool {
        if !self.active {
            return true;
        }

        // Identifier formula: 3 * MAX_STRIKE_FX + impact_kind *
        // MAX_IMPACT_FX + combo. The offset is `3 * 10 = 30`, NOT `3 *
        // 10 * 2 = 60`, even though strike FX occupy cache slots 0..59
        // (3 kinds × 10 combos × 2 variants) and `ila_*` starts at
        // slot 60. With offset 30 the lookup lands inside the
        // LIGHT_PARADE strike range — this is the original game's
        // behaviour (what players hear as the "sword hit" sound), so
        // we preserve it.
        let identifier = 3 * MAX_STRIKE_FX
            + impact_kind as u32 * MAX_IMPACT_FX
            + IMPACT_MATERIAL_TABLE[weapon as usize][armor as usize];

        let settings = SoundSettings {
            sound_type: SoundType::CombatFx,
            position,
            identifier,
            source: SoundSettingsSource::Position { material: 0 },
        };

        if let Some(params) = self
            .geometry_engine
            .get_logical_playing_params(&settings, false)
        {
            self.play_sound_now(&settings, &params, backend, loader, rng, sources);
            return true;
        }
        false
    }

    /// Queue a strike (parry) sound effect for deferred playback.
    ///
    /// Used when a sword strike is parried.
    pub fn queue_strike_fx(
        &mut self,
        strike_kind: StrikeKind,
        weapon1: WeaponMaterial,
        weapon2: WeaponMaterial,
        position: GeoPoint2D,
    ) {
        if !self.active {
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
            .geometry_engine
            .get_logical_playing_params(&settings, false)
            .is_some()
        {
            self.pending_sounds.push(PendingSoundInfo {
                settings,
                channel: CHANNEL_TO_PLAY,
                start_time_ms: 0,
                length_ms: 0,
                speech_variant: None,
                source_index: None,
                actor_id: None,
                resolved_entry: None,
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
        position: GeoPoint2D,
    ) {
        if !self.active {
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
            .geometry_engine
            .get_logical_playing_params(&settings, false)
            .is_some()
        {
            self.pending_sounds.push(PendingSoundInfo {
                settings,
                channel: CHANNEL_TO_PLAY,
                start_time_ms: 0,
                length_ms: 0,
                speech_variant: None,
                source_index: None,
                actor_id: None,
                resolved_entry: None,
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
    pub fn queue_fx(&mut self, fx_id: u32, position: GeoPoint2D, material: Option<Material>) {
        if !self.active {
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
            .geometry_engine
            .get_logical_playing_params(&settings, false)
            .is_some()
        {
            self.pending_sounds.push(PendingSoundInfo {
                settings,
                channel: CHANNEL_TO_PLAY,
                start_time_ms: 0,
                length_ms: 0,
                speech_variant: None,
                source_index: None,
                actor_id: None,
                resolved_entry: None,
            });
        }
    }

    // ── Exclamation management ───────────────────────────────────────

    /// Play an exclamation (character speech).
    pub fn play_exclamation(
        &mut self,
        group: ExclamationGroup,
        profile_id: u32,
        exclamation_id: u16,
        variant: i32,
        position: GeoPoint2D,
        actor_id: Option<u32>,
    ) {
        tracing::trace!(
            ?group,
            profile_id,
            exclamation_id,
            variant,
            ?actor_id,
            active = self.active,
            "play_exclamation"
        );
        if !self.active {
            return;
        }

        let pt = if group == ExclamationGroup::Pc {
            let mut lp = self.geometry_engine.listen_point();
            if self.use_3d_sound {
                lp.x -= 20.0;
                lp.y -= 20.0;
            }
            lp
        } else {
            position
        };

        let excl_id = (profile_id & 0xFFFF_0000) | exclamation_id as u32;
        let speech_variant = if variant == EXCLAMATION_VARIANT_NONE {
            None
        } else {
            Some(variant as u32)
        };

        let settings = SoundSettings {
            sound_type: SoundType::Exclamation,
            position: pt,
            identifier: excl_id,
            source: SoundSettingsSource::Position { material: 0 },
        };

        self.pending_sounds.push(PendingSoundInfo {
            settings,
            channel: CHANNEL_TO_PLAY,
            start_time_ms: 0,
            length_ms: 0,
            actor_id,
            source_index: None,
            speech_variant,
            resolved_entry: None,
        });
    }

    fn stop_exclamation_channel(&mut self, actor_id: u32, backend: &mut dyn AudioBackend) -> bool {
        if !self.active {
            return true;
        }

        let mut found_channel = false;
        for i in 0..self.num_channels as usize {
            if self.channel_info.get(i).is_some_and(|c| {
                c.sound_type == SoundType::Exclamation && c.actor_id == Some(actor_id)
            }) {
                self.stop_channel(i as i32, backend);
                found_channel = true;
            }
        }

        found_channel
    }

    /// Stop the currently playing exclamation channel without dropping pending speech.
    pub fn stop_exclamation_channel_only(
        &mut self,
        actor_id: u32,
        backend: &mut dyn AudioBackend,
    ) -> bool {
        self.stop_exclamation_channel(actor_id, backend)
    }

    /// Drop queued exclamations for an actor without touching the currently playing channel.
    pub fn drop_pending_exclamations(&mut self, actor_id: u32) {
        self.pending_sounds.retain(|p| {
            !(p.settings.sound_type == SoundType::Exclamation && p.actor_id == Some(actor_id))
        });
    }

    /// Stop an exclamation by actor ID. Returns true if one was playing on a channel.
    pub fn stop_exclamation(&mut self, actor_id: u32, backend: &mut dyn AudioBackend) -> bool {
        let found_channel = self.stop_exclamation_channel(actor_id, backend);
        self.drop_pending_exclamations(actor_id);

        found_channel
    }

    // ── Jingle management ────────────────────────────────────────────

    /// Play a jingle sound effect.
    pub fn play_jingle(&mut self, jingle: Jingle, backend: &mut dyn AudioBackend) {
        if !self.sound_system_ready {
            return;
        }

        let path = format!("Data/Sounds/{}", JINGLE_FILES[jingle as usize]);

        if jingle == Jingle::MissionWon || jingle == Jingle::MissionLost {
            backend.halt_music();
            self.has_mission_music = false;
        }

        if let Some(channel) = backend.play_jingle(&path) {
            self.jingle_channel = channel;
            backend.set_channel_volume(channel, self.geometry_engine.get_volume_for_jingle() / 2);
            self.update_channel_info(channel, SoundType::Jingle, None, None);
        } else {
            tracing::error!(path = %path, "missing jingle file: unable to open jingle stream");
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
        if !self.sound_system_ready {
            return true;
        }

        let file_name = {
            let entry = self.sound_cache.get_menu_sample(menu_sound_id, loader);
            match entry {
                Some(e) if e.is_loaded() => e.file_name.clone(),
                _ => return false,
            }
        };

        if let Some(channel) = backend.play_sound(&file_name, false) {
            backend.set_channel_volume(channel, self.geometry_engine.fx_volume_byte());
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
        self.pending_jingle = Some(jingle);
    }

    // ── Dialogue management ──────────────────────────────────────────

    /// Enter dialogue mode (attenuates non-dialogue volumes).
    pub fn enter_dialogue(&mut self, config: &SoundConfig) {
        self.dialog_mode = true;
        self.apply_volumes(config);
    }

    /// Leave dialogue mode (restores normal volumes).
    pub fn leave_dialogue(&mut self, config: &SoundConfig) {
        self.dialog_mode = false;
        self.apply_volumes(config);
    }

    /// Play a dialogue WAV file as a music stream.
    pub fn play_dialog(&mut self, file_path: &str, backend: &mut dyn AudioBackend) -> bool {
        if !self.sound_system_ready {
            return true;
        }

        backend.halt_music();
        self.has_mission_music = false;
        self.dialog_finished = true;

        if backend.play_music(file_path, false) {
            backend.set_music_volume(self.geometry_engine.get_volume_for_dialogue() / 2);
            self.dialog_finished = false;
            self.has_dialog = true;
        }

        true
    }

    pub fn pause_dialog(&self, backend: &mut dyn AudioBackend) -> bool {
        if self.sound_system_ready {
            backend.pause_music();
        }
        true
    }

    pub fn resume_dialog(&self, backend: &mut dyn AudioBackend) -> bool {
        if self.sound_system_ready {
            backend.resume_music();
        }
        true
    }

    pub fn close_dialog(&mut self, backend: &mut dyn AudioBackend) -> bool {
        if self.sound_system_ready {
            backend.halt_music();
            self.has_dialog = false;
        }
        true
    }

    pub fn get_dialog_volume(&self, backend: &dyn AudioBackend) -> f32 {
        if self.sound_system_ready {
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
        position: GeoPoint2D,
        zoom: f32,
    ) {
        self.set_listen_point(position, zoom);

        for i in 0..sources.num_sources() {
            let should_start =
                sources.get(i).is_some_and(|s| s.active) && !self.is_source_pending(i);
            if should_start {
                self.start_sound_source_pending(i, sources);
            }
        }
    }

    /// Suspend all sound sources (stop and remove from pending list).
    pub fn suspend_all_sound_sources(&mut self, backend: &mut dyn AudioBackend) {
        let channels_to_stop: Vec<i32> = self
            .pending_sounds
            .iter()
            .filter(|p| p.settings.sound_type == SoundType::Source && p.channel >= 0)
            .map(|p| p.channel)
            .collect();

        for ch in channels_to_stop {
            self.stop_channel(ch, backend);
        }

        self.pending_sounds
            .retain(|p| p.settings.sound_type != SoundType::Source);
    }

    /// Kick the host audio backend to start a channel for a newly
    /// activated sound source.  The sim owns the `active` flag and has
    /// already flipped it to `true` inside `perform_hourglass`
    /// (paired with the deactivate side); host reads, not writes.
    pub fn activate_sound_source(&mut self, sources: &SoundSourceManager, index: usize) {
        if sources.get(index).is_some_and(|s| s.active) {
            self.start_sound_source_pending(index, sources);
        }
    }

    /// Deactivate a specific sound source.
    ///
    /// Walks the pending list, stops the channel of the **first**
    /// matching entry, and erases just that entry. Earlier Rust used
    /// `retain` which dropped *every* match — that broke when
    /// duplicate pendings exist.
    pub fn deactivate_sound_source(&mut self, index: usize, backend: &mut dyn AudioBackend) {
        if let Some(pos) = self.pending_sounds.iter().position(|p| {
            p.settings.sound_type == SoundType::Source && p.source_index == Some(index)
        }) {
            let channel = self.pending_sounds[pos].channel;
            if channel >= 0 {
                self.stop_channel(channel, backend);
            }
            self.pending_sounds.remove(pos);
        }
    }

    /// Create and register a new sound source (runtime, e.g. from scripts).
    ///
    /// Returns `Some(index)` when sound is active, `None` when inactive
    /// — the registration is silently skipped when inactive.
    pub fn create_sound_source(
        &mut self,
        sources: &mut SoundSourceManager,
        source: SoundSource,
    ) -> Option<usize> {
        if self.active {
            Some(sources.add(source))
        } else {
            None
        }
    }

    /// Delete a sound source.
    pub fn delete_sound_source(
        &mut self,
        sources: &mut SoundSourceManager,
        index: usize,
        backend: &mut dyn AudioBackend,
    ) -> bool {
        if self.active {
            self.deactivate_sound_source(index, backend);
            sources.delete(index).is_some()
        } else {
            true
        }
    }

    // ── Hourglass (main update) ──────────────────────────────────────

    /// Main per-frame sound update. Call once per game tick.
    ///
    /// Updates timers, plays pending sounds, manages music transitions,
    /// and updates cache TTLs.
    pub fn hourglass(
        &mut self,
        backend: &mut dyn AudioBackend,
        loader: &SampleLoader,
        rng: &mut dyn FnMut(u32) -> u32,
        alert_status: AlertStatus,
        sources: &SoundSourceManager,
        pending_play_delayed_sources: &mut Vec<usize>,
    ) {
        if !self.active {
            return;
        }
        if !self.pending_sounds.is_empty() {
            tracing::trace!(
                pending = self.pending_sounds.len(),
                "sound hourglass: processing pending"
            );
        }
        // `engine.sound_sim.finished_exclamations` is now sim-side:
        // the engine schedules each exclamation's MYTALK finish at
        // emit time using `exclamation_durations`, then drains the
        // matured ones into `finished_exclamations` at the top of
        // every `perform_hourglass`. Audio-backend completion still
        // fires for actual playback, but it doesn't write any sim
        // state.

        // Play deferred jingle (queued from script commands)
        if let Some(jingle) = self.pending_jingle.take() {
            self.play_jingle(jingle, backend);
        }

        // Check for music/dialog finished callbacks
        if backend.take_music_finished() {
            if self.has_dialog {
                self.dialog_finished = true;
                self.stop_dialog = true;
            } else if self.has_mission_music {
                self.on_music_finished(alert_status);
            }
        }

        // Check jingle channel finished
        if self.jingle_channel >= 0 && !backend.is_channel_playing(self.jingle_channel) {
            self.jingle_channel = -1;
            self.stop_jingle = true;
        }

        // ── Start delayed sound sources the engine flagged ─────
        // Engine ticks the timer down inside `perform_hourglass`,
        // emits `SoundCommand::PlayDelayedSource(idx)` when it hits
        // zero, and immediately re-rolls the timer using `sim_rng`.
        // We just drain the queue and start playback. (Source timer
        // reset used to live here, driven by audio-backend playback
        // completion + a host RNG, which broke rollback determinism.)
        for idx in pending_play_delayed_sources.drain(..) {
            if !self.is_source_pending(idx) && sources.get(idx).is_some_and(|s| s.active) {
                self.start_sound_source_pending(idx, sources);
            }
        }

        // ── Update channel playing state ──
        self.update_all_channels_info(backend);

        // ── Handle deferred cleanup ──
        if self.stop_jingle {
            backend.free_jingle();
            self.stop_jingle = false;
        }
        if self.stop_dialog {
            backend.halt_music();
            self.has_dialog = false;
            self.stop_dialog = false;
        }

        // ── Decay music mode weights ──
        self.quiet_mode_weight = self.quiet_mode_weight.saturating_sub(1);
        self.alert_mode_weight = self.alert_mode_weight.saturating_sub(1);
        self.fight_mode_weight = self.fight_mode_weight.saturating_sub(1);

        // ── Music loop selection ──
        if self.load_music {
            let mut mode = MusicMode::Quiet;
            let mut weight = self.quiet_mode_weight;
            if self.alert_mode_weight > weight {
                mode = MusicMode::Alert;
                weight = self.alert_mode_weight;
            }
            if self.fight_mode_weight > weight {
                mode = MusicMode::Fight;
            }
            self.music_mode = mode;

            // Choose a random loop index
            let pool_size = match mode {
                MusicMode::Quiet => self.sound_cache.quiet_music_pool.len(),
                MusicMode::Alert => self.sound_cache.alert_music_pool.len(),
                MusicMode::Fight => self.sound_cache.fight_music_pool.len(),
            };
            if pool_size > 0 {
                self.loop_index = rng(pool_size as u32) as i16;
            }

            self.load_music = false;
            self.start_music = true;
        }

        if self.start_music {
            if let Some(name) = self.select_music_loop() {
                let path = format!("{}/{}.wav", self.music_directory, name);
                if backend.play_music(&path, false) {
                    self.has_mission_music = true;
                    // play_music replaces any previously playing stream
                    // (e.g., the menu music carried over from the loading
                    // screen). Clear the flag now that the new mission
                    // track has taken over.
                    self.has_menu_music = false;
                }
            }
            self.start_music = false;
            self.update_music = true;
        }

        if self.update_music {
            backend.set_music_volume(self.geometry_engine.get_volume_for_music(true) / 2);
            self.update_music = false;
        }

        // ── Play queued short FX ──
        let fx_list = std::mem::take(&mut self.fx_to_play);
        for fx in &fx_list {
            self.play_sound_now(&fx.settings, &fx.params, backend, loader, rng, sources);
        }

        // ── Process pending sounds ──
        self.process_pending_sounds(backend, loader, rng, sources);

        // ── Update cache TTLs ──
        self.sound_cache.update_cache_state();
    }

    /// Process all pending sounds: expire finished ones, update params, play new.
    fn process_pending_sounds(
        &mut self,
        backend: &mut dyn AudioBackend,
        loader: &SampleLoader,
        rng: &mut dyn FnMut(u32) -> u32,
        sources: &SoundSourceManager,
    ) {
        let now = backend.get_ticks();

        // ── Pass 1: initialize lengths and remove finished sounds ──
        let mut finished: Vec<PendingSoundInfo> = Vec::new();
        let mut i = 0;
        while i < self.pending_sounds.len() {
            // Initialize length if needed
            if self.pending_sounds[i].length_ms == 0 {
                self.pending_sounds[i].start_time_ms = now;
                let settings = self.pending_sounds[i].settings.clone();
                let sv = self.pending_sounds[i].speech_variant;
                let entry = self.get_entry_info(&settings, sv, false, loader, rng, sources);
                let length = entry.as_ref().map_or(0, |i| i.sample_length_ms);
                self.pending_sounds[i].length_ms = length;
                if settings.sound_type == SoundType::Exclamation {
                    self.pending_sounds[i].resolved_entry = entry;
                }
            }

            // Compute elapsed time
            let elapsed = {
                let ps = &self.pending_sounds[i];
                if ps.length_ms == 0 {
                    // Sample not available → expire immediately
                    ps.length_ms
                } else if ps.settings.sound_type == SoundType::Source
                    && ps
                        .source_index
                        .and_then(|idx| sources.get(idx))
                        .is_some_and(|s| s.source_kind == SoundSourceKind::Looped)
                {
                    0 // Looping sources never expire
                } else {
                    time_elapsed(ps.start_time_ms, now)
                }
            };

            if elapsed >= self.pending_sounds[i].length_ms {
                let ps = &self.pending_sounds[i];
                if ps.settings.sound_type == SoundType::Exclamation {
                    tracing::trace!(
                        actor_id = ?ps.actor_id,
                        identifier = ps.settings.identifier,
                        length_ms = ps.length_ms,
                        "exclamation expired in Pass 1 (length_ms=0 means sample missing)"
                    );
                }
                finished.push(self.pending_sounds.remove(i));
            } else {
                i += 1;
            }
        }

        // Handle finished sounds.  Channel cleanup stays host-side
        // (pure SDL state, not in the rollback hash); the kind-specific
        // sim transition for finished `Source`-type sounds (Single
        // `active = false`, Volatile `sources.delete`) now fires from
        // the sim-side drain in `Engine::perform_hourglass` using
        // `SoundSimState::playing_sources` scheduled at activation
        // time. Exclamation finish callbacks are likewise sim-side
        // from `playing_exclamations`.
        // We erase pending sounds on logical completion, but do not
        // halt their mixer channels here. The channel either ends
        // naturally or is stopped by explicit stop/deactivate paths.

        // ── Pass 2: update params if listen point changed ──
        if self.update_pending_sounds {
            for i in 0..self.pending_sounds.len() {
                let settings = self.pending_sounds[i].settings.clone();
                let low_priority = settings.sound_type == SoundType::Fx
                    && self.sound_cache.is_material_fx(settings.identifier);

                if let Some(mut params) = self
                    .geometry_engine
                    .get_logical_playing_params(&settings, low_priority)
                {
                    let channel = self.pending_sounds[i].channel;
                    match channel {
                        CHANNEL_NONE => {
                            self.pending_sounds[i].channel = CHANNEL_TO_PLAY;
                        }
                        CHANNEL_TO_PLAY => {}
                        ch => {
                            if self.use_3d_sound {
                                SoundGeometry::get_3d_playing_params(&mut params);
                                backend.set_channel_position_3d(ch, params.position_3d);
                            } else {
                                SoundGeometry::get_2d_playing_params(&mut params);
                            }
                            backend.set_channel_volume(ch, params.volume_2d);
                        }
                    }
                } else {
                    let channel = self.pending_sounds[i].channel;
                    if channel >= 0 {
                        backend.halt_channel(channel);
                    }
                    self.pending_sounds[i].channel = CHANNEL_NONE;
                }
            }
            self.update_pending_sounds = false;
        }

        // ── Pass 3: play sounds marked CHANNEL_TO_PLAY ──
        let now = backend.get_ticks();
        for i in 0..self.pending_sounds.len() {
            if self.pending_sounds[i].channel != CHANNEL_TO_PLAY {
                continue;
            }

            let settings = self.pending_sounds[i].settings.clone();
            let low_priority = settings.sound_type == SoundType::Fx
                && self.sound_cache.is_material_fx(settings.identifier);

            let Some(params) = self
                .geometry_engine
                .get_logical_playing_params(&settings, low_priority)
            else {
                if settings.sound_type == SoundType::Exclamation {
                    tracing::trace!(
                        actor_id = ?self.pending_sounds[i].actor_id,
                        identifier = settings.identifier,
                        "exclamation skipped: no logical playing params"
                    );
                }
                self.pending_sounds[i].channel = CHANNEL_NONE;
                continue;
            };

            let speech_variant = self.pending_sounds[i].speech_variant;
            let entry_info = if settings.sound_type == SoundType::Exclamation {
                self.pending_sounds[i].resolved_entry.clone()
            } else {
                self.get_entry_info(&settings, speech_variant, true, loader, rng, sources)
            };
            let Some(info) = entry_info else {
                if settings.sound_type == SoundType::Exclamation {
                    tracing::trace!(
                        actor_id = ?self.pending_sounds[i].actor_id,
                        identifier = settings.identifier,
                        "exclamation skipped: no entry_info (sample file missing?)"
                    );
                }
                continue;
            };

            // Compute play position
            let start = self.pending_sounds[i].start_time_ms;
            let length = self.pending_sounds[i].length_ms;
            let elapsed = time_elapsed(start, now);
            let mut position = if length > 0 {
                elapsed as f32 / length as f32
            } else {
                0.0
            };

            // Handle looping
            if self.pending_sounds[i].settings.sound_type == SoundType::Source
                && let Some(idx) = self.pending_sounds[i].source_index
                && sources
                    .get(idx)
                    .is_some_and(|s| s.source_kind == SoundSourceKind::Looped)
            {
                position -= position.floor();
            }
            position = position.clamp(0.0, 0.999);

            let mut hw_params = params;
            if self.use_3d_sound {
                SoundGeometry::get_3d_playing_params(&mut hw_params);
            } else {
                SoundGeometry::get_2d_playing_params(&mut hw_params);
            }

            let play_result = if self.use_3d_sound {
                backend.play_sound_3d(
                    &info.file_name,
                    info.loop_sample,
                    position,
                    hw_params.position_3d,
                )
            } else {
                backend.play_sound_at(&info.file_name, info.loop_sample, position)
            };

            if let Some(channel) = play_result {
                backend.set_channel_volume(channel, hw_params.volume_2d);

                let actor_id = self.pending_sounds[i].actor_id;

                self.update_channel_info(channel, settings.sound_type, info.cache_key, actor_id);

                self.pending_sounds[i].channel = channel;
                if settings.sound_type == SoundType::Exclamation {
                    tracing::trace!(
                        actor_id = ?actor_id,
                        file = info.file_name.as_str(),
                        channel,
                        volume_2d = hw_params.volume_2d,
                        "exclamation playing"
                    );
                }
            } else if settings.sound_type == SoundType::Exclamation {
                tracing::trace!(
                    actor_id = ?self.pending_sounds[i].actor_id,
                    file = info.file_name.as_str(),
                    "exclamation skipped: backend.play_sound_at returned None"
                );
            }
        }
    }

    // ── Private helpers ──────────────────────────────────────────────

    /// Select the current music loop name from the appropriate pool.
    fn select_music_loop(&self) -> Option<String> {
        let pool = match self.music_mode {
            MusicMode::Quiet => &self.sound_cache.quiet_music_pool,
            MusicMode::Alert => &self.sound_cache.alert_music_pool,
            MusicMode::Fight => &self.sound_cache.fight_music_pool,
        };
        if pool.is_empty() {
            None
        } else {
            Some(pool[self.loop_index as usize % pool.len()].clone())
        }
    }

    /// Add a sound source to the pending sounds list.
    fn start_sound_source_pending(&mut self, source_index: usize, sources: &SoundSourceManager) {
        let src = match sources.get(source_index) {
            Some(s) => s,
            None => return,
        };

        let settings = SoundSettings {
            sound_type: SoundType::Source,
            position: src.shape.first().copied().unwrap_or(geo2d::pt(0.0, 0.0)),
            identifier: src.id,
            source: SoundSettingsSource::SoundSource {
                info: src.to_source_info(),
                speech_variant: -1,
            },
        };

        self.pending_sounds.push(PendingSoundInfo {
            settings,
            channel: CHANNEL_TO_PLAY,
            start_time_ms: 0,
            length_ms: 0,
            actor_id: None,
            source_index: Some(source_index),
            speech_variant: None,
            resolved_entry: None,
        });
    }

    /// Check if a sound source is already in the pending list.
    fn is_source_pending(&self, source_index: usize) -> bool {
        self.pending_sounds.iter().any(|p| {
            p.settings.sound_type == SoundType::Source && p.source_index == Some(source_index)
        })
    }

    /// Play a sound immediately using pre-computed playing params.
    fn play_sound_now(
        &mut self,
        settings: &SoundSettings,
        params: &PlayingParameters,
        backend: &mut dyn AudioBackend,
        loader: &SampleLoader,
        rng: &mut dyn FnMut(u32) -> u32,
        sources: &SoundSourceManager,
    ) -> i32 {
        let info = match self.get_entry_info(settings, None, true, loader, rng, sources) {
            Some(i) => i,
            None => return CHANNEL_NONE,
        };

        let mut hw_params = params.clone();
        let play_result = if self.use_3d_sound {
            SoundGeometry::get_3d_playing_params(&mut hw_params);
            backend.play_sound_3d(
                &info.file_name,
                info.loop_sample,
                0.0,
                hw_params.position_3d,
            )
        } else {
            SoundGeometry::get_2d_playing_params(&mut hw_params);
            backend.play_sound(&info.file_name, info.loop_sample)
        };

        let Some(channel) = play_result else {
            return CHANNEL_NONE;
        };

        backend.set_channel_volume(channel, hw_params.volume_2d);

        self.update_channel_info(channel, settings.sound_type, info.cache_key, None);

        channel
    }

    /// Stop a channel and clear its bookkeeping.
    fn stop_channel(&mut self, channel: i32, backend: &mut dyn AudioBackend) {
        if channel >= 0 {
            backend.halt_channel(channel);
            self.update_channel_info(channel, SoundType::None, None, None);
        }
    }

    /// Update channel bookkeeping (playing counts, etc.).
    fn update_channel_info(
        &mut self,
        channel: i32,
        sound_type: SoundType,
        cache_key: Option<CacheKey>,
        actor_id: Option<u32>,
    ) {
        let idx = channel as usize;
        if idx >= self.channel_info.len() {
            return;
        }

        // Decrement playing count on the old entry
        if let Some(key) = self.channel_info[idx].cache_key.clone() {
            self.adjust_cache_playing(&key, false);
        }

        self.channel_info[idx] = ChannelInfo {
            sound_type,
            cache_key: cache_key.clone(),
            actor_id,
        };

        // Increment playing count on the new entry
        if let Some(ref key) = cache_key {
            self.adjust_cache_playing(key, true);
        }
    }

    /// Update all channels: clear info for channels that stopped playing.
    fn update_all_channels_info(&mut self, backend: &dyn AudioBackend) {
        for i in 0..self.num_channels as usize {
            if i < self.channel_info.len()
                && self.channel_info[i].sound_type != SoundType::None
                && !backend.is_channel_playing(i as i32)
            {
                if let Some(key) = self.channel_info[i].cache_key.clone() {
                    self.adjust_cache_playing(&key, false);
                }
                self.channel_info[i] = ChannelInfo::default();
            }
        }
    }

    /// Increment or decrement the playing count on a cache entry.
    fn adjust_cache_playing(&mut self, key: &CacheKey, increment: bool) {
        let delta = |entry: &mut crate::sound_cache::SoundCacheEntry| {
            if increment {
                entry.playing += 1;
            } else {
                entry.playing = entry.playing.saturating_sub(1);
            }
        };

        match key {
            CacheKey::FxIndex(idx) => {
                if let Some(entry) = self.sound_cache.fx_cache.entries.get_mut(*idx) {
                    delta(entry);
                }
            }
            CacheKey::CombatFx(id) => {
                if let Some(entry) = self.sound_cache.combat_fx_cache.entries.get_mut(id) {
                    delta(entry);
                }
            }
            CacheKey::Source(id) => {
                if let Some(entry) = self.sound_cache.source_cache.entries.get_mut(id) {
                    delta(entry);
                }
            }
            CacheKey::SpeechIndex(idx) => {
                if let Some(entry) = self.sound_cache.speech_cache.entries.get_mut(*idx) {
                    delta(entry);
                }
            }
            CacheKey::Menu(id) => {
                if let Some(entry) = self.sound_cache.menu_cache.entries.get_mut(id) {
                    delta(entry);
                }
            }
        }
    }

    // ── Cache entry info extraction ──────────────────────────────────

    /// Get sample length in ms for given sound settings.
    fn get_sample_length_ms(
        &mut self,
        settings: &SoundSettings,
        loader: &SampleLoader,
        rng: &mut dyn FnMut(u32) -> u32,
        sources: &SoundSourceManager,
    ) -> u32 {
        self.get_entry_info(settings, None, false, loader, rng, sources)
            .map_or(0, |i| i.sample_length_ms)
    }

    /// Extract cache entry info (file name, length, etc.) without holding a
    /// borrow on the cache. Calls the appropriate cache getter internally.
    fn get_entry_info(
        &mut self,
        settings: &SoundSettings,
        speech_variant: Option<u32>,
        sample_present: bool,
        loader: &SampleLoader,
        rng: &mut dyn FnMut(u32) -> u32,
        sources: &SoundSourceManager,
    ) -> Option<CacheEntryInfo> {
        match settings.sound_type {
            SoundType::Source => {
                let looping = sources
                    .find_by_sample_id(settings.identifier)
                    .and_then(|idx| sources.get(idx))
                    .is_some_and(|s| s.source_kind == SoundSourceKind::Looped);

                let entry = self.sound_cache.get_source_sample(
                    sample_present,
                    settings.identifier,
                    looping,
                    loader,
                )?;
                if sample_present && !entry.is_loaded() {
                    return None;
                }
                Some(CacheEntryInfo {
                    file_name: entry.file_name.clone(),
                    sample_length_ms: entry.sample_length_ms,
                    loop_sample: entry.loop_sample,
                    cache_key: Some(CacheKey::Source(settings.identifier)),
                })
            }
            SoundType::Fx | SoundType::MenuFx => {
                let material = match &settings.source {
                    SoundSettingsSource::Position { material: m } => material_from_u8(*m),
                    _ => None,
                };
                let idx = self.sound_cache.get_fx_sample(
                    sample_present,
                    settings.identifier,
                    material,
                    loader,
                    rng,
                )?;
                let entry = &self.sound_cache.fx_cache.entries[idx];
                if sample_present && !entry.is_loaded() {
                    return None;
                }
                Some(CacheEntryInfo {
                    file_name: entry.file_name.clone(),
                    sample_length_ms: entry.sample_length_ms,
                    loop_sample: entry.loop_sample,
                    cache_key: Some(CacheKey::FxIndex(idx)),
                })
            }
            SoundType::CombatFx => {
                let entry = self.sound_cache.get_combat_fx_sample(
                    sample_present,
                    settings.identifier,
                    loader,
                )?;
                if sample_present && !entry.is_loaded() {
                    return None;
                }
                Some(CacheEntryInfo {
                    file_name: entry.file_name.clone(),
                    sample_length_ms: entry.sample_length_ms,
                    loop_sample: entry.loop_sample,
                    cache_key: Some(CacheKey::CombatFx(settings.identifier)),
                })
            }
            SoundType::Exclamation => {
                let idx = self.sound_cache.get_exclamation_sample(
                    sample_present,
                    settings.identifier,
                    speech_variant,
                    loader,
                    rng,
                )?;
                let entry = &self.sound_cache.speech_cache.entries[idx];
                if sample_present && !entry.is_loaded() {
                    return None;
                }
                Some(CacheEntryInfo {
                    file_name: entry.file_name.clone(),
                    sample_length_ms: entry.sample_length_ms,
                    loop_sample: entry.loop_sample,
                    cache_key: Some(CacheKey::SpeechIndex(idx)),
                })
            }
            _ => None,
        }
    }
}

// ─── Free functions ─────────────────────────────────────────────────

/// Compute time elapsed between `start` and `now`, handling 32-bit wrap.
fn time_elapsed(start: u32, now: u32) -> u32 {
    if start > now {
        (!start).wrapping_add(1).wrapping_add(now)
    } else {
        now - start
    }
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
mod tests {
    use super::*;

    /// Minimal mock audio backend for testing.
    struct MockBackend {
        channels_playing: Vec<bool>,
        music_finished_flag: bool,
        ticks: u32,
        next_channel: i32,
        music_volume: u16,
    }

    impl MockBackend {
        fn new() -> Self {
            Self {
                channels_playing: vec![false; NUM_CHANNELS as usize],
                music_finished_flag: false,
                ticks: 0,
                next_channel: 0,
                music_volume: 0,
            }
        }
    }

    impl AudioBackend for MockBackend {
        fn play_sound(&mut self, _file: &str, _looping: bool) -> Option<i32> {
            let ch = self.next_channel;
            if (ch as usize) < self.channels_playing.len() {
                self.channels_playing[ch as usize] = true;
                self.next_channel = (ch + 1) % NUM_CHANNELS as i32;
                Some(ch)
            } else {
                None
            }
        }
        fn play_sound_at(&mut self, file: &str, looping: bool, _pos: f32) -> Option<i32> {
            self.play_sound(file, looping)
        }
        fn halt_channel(&mut self, ch: i32) {
            if let Some(v) = self.channels_playing.get_mut(ch as usize) {
                *v = false;
            }
        }
        fn set_channel_volume(&mut self, _ch: i32, _vol: u16) {}
        fn is_channel_playing(&self, ch: i32) -> bool {
            self.channels_playing
                .get(ch as usize)
                .copied()
                .unwrap_or(false)
        }
        fn pause_channels(&mut self, _ch: i32) {}
        fn resume_channels(&mut self, _ch: i32) {}
        fn play_music(&mut self, _path: &str, _looping: bool) -> bool {
            true
        }
        fn halt_music(&mut self) {}
        fn pause_music(&mut self) {}
        fn resume_music(&mut self) {}
        fn set_music_volume(&mut self, v: u16) {
            self.music_volume = v;
        }
        fn get_music_volume(&self) -> u16 {
            self.music_volume
        }
        fn take_music_finished(&mut self) -> bool {
            let v = self.music_finished_flag;
            self.music_finished_flag = false;
            v
        }
        fn play_jingle(&mut self, _path: &str) -> Option<i32> {
            self.play_sound("jingle", false)
        }
        fn free_jingle(&mut self) {}
        fn get_ticks(&self) -> u32 {
            self.ticks
        }
        fn num_channels(&self) -> u32 {
            NUM_CHANNELS
        }
    }

    #[test]
    fn sound_manager_new() {
        let mgr = SoundManager::new();
        assert!(!mgr.is_ready());
        assert!(!mgr.is_active());
        assert_eq!(mgr.music_mode(), MusicMode::Quiet);
    }

    #[test]
    fn initialize_and_activate() {
        let mut mgr = SoundManager::new();
        let mut backend = MockBackend::new();
        mgr.initialize(&mut backend, false).unwrap();
        assert!(mgr.is_ready());

        let sources = SoundSourceManager::new();
        mgr.activate(false, &sources);
        assert!(mgr.is_active());
    }

    #[test]
    fn music_mode_weights() {
        let mut mgr = SoundManager::new();
        mgr.set_music_mode(MusicMode::Quiet);
        assert_eq!(mgr.quiet_mode_weight(), MUSIC_MODE_WEIGHT);

        mgr.set_music_mode(MusicMode::Fight);
        assert_eq!(mgr.fight_mode_weight(), MUSIC_MODE_WEIGHT);

        mgr.force_music_mode(MusicMode::Alert);
        assert_eq!(mgr.quiet_mode_weight(), 0);
        assert_eq!(mgr.alert_mode_weight(), MUSIC_MODE_WEIGHT);
        assert_eq!(mgr.fight_mode_weight(), 0);
    }

    #[test]
    fn music_mode_forest() {
        let mut mgr = SoundManager::new();
        mgr.forest_level = true;
        // In forest levels, Quiet falls through to Alert
        mgr.set_music_mode(MusicMode::Quiet);
        assert_eq!(mgr.quiet_mode_weight(), 0);
        assert_eq!(mgr.alert_mode_weight(), MUSIC_MODE_WEIGHT);
    }

    #[test]
    fn time_elapsed_basic() {
        assert_eq!(time_elapsed(100, 200), 100);
        assert_eq!(time_elapsed(0, 0), 0);
    }

    #[test]
    fn time_elapsed_wrap() {
        assert_eq!(time_elapsed(u32::MAX - 10, 5), 16);
    }

    #[test]
    fn strike_material_table_symmetric() {
        // Indexed loop intentional: we need to compare [i][j] with the
        // transposed [j][i], so an iterator over rows alone wouldn't help.
        #[allow(clippy::needless_range_loop)]
        for i in 0..4 {
            for j in 0..4 {
                assert_eq!(
                    STRIKE_MATERIAL_TABLE[i][j], STRIKE_MATERIAL_TABLE[j][i],
                    "STRIKE_TABLE[{i}][{j}] != [{j}][{i}]"
                );
            }
        }
    }

    #[test]
    fn combat_fx_strike_id_range() {
        for kind in 0..3u32 {
            for combo in 0..10u32 {
                for variant in 0..2u32 {
                    let id = (kind * MAX_STRIKE_FX + combo) * 2 + variant;
                    assert!(id < 60, "Strike ID {id} out of range");
                }
            }
        }
    }

    #[test]
    fn combat_fx_impact_id_range() {
        // The PlayImpactFx quirk: `3 * MAX_STRIKE_FX` as offset, not
        // `3 * 10 * 2`. Ids land inside the parade range (30..54),
        // which is the sound players recognize as "sword hit".
        for kind in 0..2u32 {
            for combo in 0..12u32 {
                let id = 3 * MAX_STRIKE_FX + kind * MAX_IMPACT_FX + combo;
                assert!((30..54).contains(&id), "Impact ID {id} out of range 30..54");
            }
        }
    }

    #[test]
    fn deactivate_clears_state() {
        let mut mgr = SoundManager::new();
        let mut backend = MockBackend::new();
        mgr.initialize(&mut backend, false).unwrap();
        let mut sources = SoundSourceManager::new();
        mgr.activate(false, &sources);
        assert!(mgr.is_active());

        mgr.deactivate(true, &mut backend, &mut sources);
        assert!(!mgr.is_active());
        assert_eq!(mgr.num_pending_sounds(), 0);
    }

    #[test]
    fn jingle_files_count() {
        assert_eq!(JINGLE_FILES.len(), 8);
    }

    #[test]
    fn dialog_finished_when_not_ready() {
        let mgr = SoundManager::new();
        assert!(mgr.is_dialog_finished());
    }

    #[test]
    fn serde_roundtrip() {
        let mut mgr = SoundManager::new();
        mgr.music_mode = MusicMode::Fight;
        mgr.quiet_mode_weight = 100;
        mgr.forest_level = true;

        let json = serde_json::to_string(&mgr).unwrap();
        let restored: SoundManager = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.music_mode, MusicMode::Fight);
        assert_eq!(restored.quiet_mode_weight, 100);
        assert!(restored.forest_level);
        // Transient state should be default after deserialization
        assert!(restored.pending_sounds.is_empty());
        assert!(restored.channel_info.is_empty());
    }

    #[test]
    fn material_from_u8_valid() {
        assert_eq!(material_from_u8(0), Some(Material::Ground));
        assert_eq!(material_from_u8(8), Some(Material::Hole));
        assert_eq!(material_from_u8(9), None);
        assert_eq!(material_from_u8(255), None);
    }

    #[test]
    fn hourglass_empty_does_not_crash() {
        let mut mgr = SoundManager::new();
        let mut backend = MockBackend::new();
        mgr.initialize(&mut backend, false).unwrap();
        let sources = SoundSourceManager::new();
        mgr.activate(false, &sources);

        let loader: Box<SampleLoader> = Box::new(|_| None);
        let mut rng = |_: u32| 0u32;

        let mut pending_play = Vec::new();
        mgr.hourglass(
            &mut backend,
            &loader,
            &mut rng,
            AlertStatus::Green,
            &sources,
            &mut pending_play,
        );
        // Should not panic or crash
    }

    #[test]
    fn listen_point_triggers_update() {
        let mut mgr = SoundManager::new();
        mgr.update_pending_sounds = false;
        mgr.set_listen_point(geo2d::pt(100.0, 200.0), 1.0);
        assert!(mgr.update_pending_sounds);
    }
}
