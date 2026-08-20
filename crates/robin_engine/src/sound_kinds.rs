//! Sim-side sound classification enums.
//!
//! These IDs describe *what* sound to play; the host (`robin_rs::sound`)
//! resolves them to actual samples and dispatches playback.

use serde::{Deserialize, Serialize};

use crate::sound_source::SoundSourceManager;

/// Sim-state portion of the sound system. Owned by `EngineInner`, included in
/// rollback snapshots. Host-side `SoundManager` (in robin_rs) tracks the
/// non-sim playback machinery (channels, cache, music backend).
#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SoundSimState {
    pub sources: SoundSourceManager,
    /// Exact v48 sound director/backend snapshot retained across save import.
    ///
    /// Sources, music mode/weights, loop identity, and geometry are
    /// authoritative simulation inputs. The readiness/3D/active/channel/
    /// stream members describe the Original host backend and are retained for
    /// lossless save compatibility but never drive Rust audio output.
    pub legacy_v48: Option<LegacyV48SoundState>,
    /// Exclamations that finished this frame: `(actor_id, exclamation_id)`.
    /// Populated by the engine each tick from `playing_exclamations`
    /// entries whose `finish_frame` has elapsed; consumed by the AI tick
    /// (MYTALK callback) within the same `perform_hourglass` call.
    pub finished_exclamations: Vec<(u32, u32)>,
    /// Exclamations currently playing, with the (sim) frame on which
    /// they'll move into `finished_exclamations`. Lives sim-side so
    /// rollback re-runs of the tick reproduce the same MYTALK timing —
    /// the audio backend's wall-clock playback completion is no longer
    /// what drives `finished_exclamations`.
    pub playing_exclamations: Vec<PlayingExclamation>,
    /// Speech requests waiting for the following logical sound-manager
    /// update, in Original pending-list order.
    pub pending_exclamations: Vec<PendingExclamation>,
    /// Concrete sample durations resolved at the preceding sound-manager
    /// boundary and applied at the start of the next engine frame.
    pub resolved_exclamations: Vec<ResolvedExclamation>,
    /// True only while `resolved_exclamations` came from an authoritative
    /// Original parity trace rather than Rust's live host sound manager.
    ///
    /// A recorded host resolution can lack a Rust logical request when the
    /// Original engine alone asked its host to play a line. Live resolutions
    /// remain strictly paired with Rust requests.
    #[serde(default)]
    pub replay_injected_resolved_exclamations: bool,
    /// Single/Volatile/Delayed sound sources currently playing, with the (sim)
    /// frame on which the engine will apply their finish transition
    /// (`active = false` for Single, `sources.delete` for Volatile).
    /// Populated at activation time using the host-supplied
    /// `source_durations` table so rollback replay produces identical
    /// `sources` state without depending on the audio backend's wall-clock
    /// playback-completion events. Looped sources are never scheduled;
    /// Delayed sources use completion to re-roll their next timer, matching
    /// the original `StopSoundSource` ordering.
    pub playing_sources: Vec<PlayingSource>,
    /// Source indices that were `active` at the last
    /// `SuspendAllSoundSources` call.  Populated by the
    /// `SuspendAll` command drain and consumed by the paired
    /// `ResumeAll` so every previously-active source resumes —
    /// hourglass channel-stop clears the active flag, so we need
    /// this stash to restore the active set on resume.
    pub suspended_active_sources: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct LegacyV48SoundState {
    pub sound_system_ready: bool,
    pub three_d_sound: bool,
    pub active: bool,
    pub listen_point: crate::coordinates::MapPoint,
    pub zoom_factor: f32,
    pub music_mode: MusicMode,
    /// Uninitialized dummy channel bytes written by Original. Retained only
    /// for compatibility; Rust must never branch on this value.
    pub dummy_channel: i16,
    pub quiet_mode_weight: u32,
    pub alert_mode_weight: u32,
    pub fight_mode_weight: u32,
    pub loop_index: i16,
    /// Original v48 writes zero, then uses the value only to seek the host
    /// music stream during load.
    pub stream_position: u32,
}

/// A scheduled exclamation finish. `actor_id` and `exclamation_id`
/// match the `(actor_id, excl_id)` tuple the AI MYTALK handler reads
/// out of `finished_exclamations`.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct PlayingExclamation {
    pub actor_id: u32,
    pub exclamation_id: u32,
    pub finish_frame: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct PendingExclamation {
    pub actor_id: u32,
    pub group: ExclamationGroup,
    pub profile_id: u32,
    pub exclamation_id: u16,
    pub variant: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct ResolvedExclamation {
    pub actor_id: u32,
    pub identifier: u32,
    pub exclamation_id: u16,
    pub duration_frames: u32,
}

/// A scheduled sound-source finish. `source_index` is the index into
/// `SoundSimState::sources`; `finish_frame` is the sim frame on which
/// the drain inside `perform_hourglass` will apply the kind-specific
/// finish transition.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct PlayingSource {
    pub source_index: u32,
    pub finish_frame: u32,
}

/// Current music mood.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub enum MusicMode {
    Quiet = 0,
    Alert = 1,
    Fight = 2,
}

/// Exclamation actor group.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub enum ExclamationGroup {
    Pc,
    Soldier,
    Civilian,
    Vip,
}

/// Strike type for combat FX.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum StrikeKind {
    Swipe = 0,
    LightParade = 1,
    HeavyParade = 2,
}

/// Impact type for combat FX.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum ImpactKind {
    LightArmor = 0,
    HeavyArmor = 1,
}

/// Jingle type.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub enum Jingle {
    NewPeasantCalled = 0,
    MissionWon = 1,
    MissionLost = 2,
    CashWon = 3,
    QuickActionSucceeded = 4,
    QuickActionFailed = 5,
    TrapTriggered = 6,
    PcInComa = 7,
}
