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
    /// Single/Volatile sound sources currently playing, with the (sim)
    /// frame on which the engine will apply their finish transition
    /// (`active = false` for Single, `sources.delete` for Volatile).
    /// Populated at activation time using the host-supplied
    /// `source_durations` table so rollback replay produces identical
    /// `sources` state without depending on the audio backend's wall-clock
    /// playback-completion events. Looped and Delayed sources are
    /// never scheduled here (Looped never finishes on its own; Delayed
    /// re-rolls its timer sim-side in `perform_hourglass`).
    pub playing_sources: Vec<PlayingSource>,
    /// Source indices that were `active` at the last
    /// `SuspendAllSoundSources` call.  Populated by the
    /// `SuspendAll` command drain and consumed by the paired
    /// `ResumeAll` so every previously-active source resumes —
    /// hourglass channel-stop clears the active flag, so we need
    /// this stash to restore the active set on resume.
    pub suspended_active_sources: Vec<u32>,
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
