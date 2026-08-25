//! Transaction boundary around one deterministic engine hourglass.
//!
//! The Original's game loop finishes input/message translation before calling
//! `RHEngine::PerformHourglass`, then performs widgets, rendering, and sound
//! afterwards (`original-code/RHgame.cpp:1562-1915`). This module names that
//! boundary without changing the established hourglass implementation or its
//! phase ordering.
//!
//! This is an engine transaction, not a complete `RHGame` host-loop frame.
//! [`super::Engine::advance_frame`] always admits exactly one
//! `PerformHourglass`; host iterations that gate it off, post-hourglass
//! callbacks, rendering, widgets, sound, and lifecycle hooks remain outside
//! the modeled transaction.
//!
//! [`SimulationFrameInput`] deliberately contains only authoritative inputs:
//! resolved [`SimCommand`]s and host observations represented as
//! [`ExternalFact`]s. Host UI scratch remains an adapter argument to
//! [`super::Engine::advance_frame`] during migration and cannot be serialized
//! accidentally as part of the frame input.
//!
//! TODO(architecture): remove those host scratch arguments once remaining
//! command/tick handlers emit sim events instead of editing presentation state.
//!
//! TODO(architecture): migrate replay, rewind, rollback, and multiplayer
//! journals from command-only entries to a phase-complete record based on
//! [`SimulationFrameInput`]. A checkpoint taken after an external fact captures
//! that one boundary, but cannot reconstruct the same fact when replaying an
//! earlier journal span. Before parity-trace migration, the input also needs an
//! explicit post-hourglass command phase: a nested `Refresh` during commands
//! such as `DisplayPopupText` can record a resolved orientation that must apply
//! after `PerformHourglass`, not in the pre-hourglass
//! [`SimulationFrameInput::commands`] phase. The same ordered ingress must
//! capture current host mutation routes (HTTP native/batch/console and
//! single-player command dispatch) that can run after the tick but before the
//! history entry is committed; a pre-hourglass command-only journal loses
//! those mutations too.
//!
//! TODO(architecture): model host-loop iterations where the Original's gate
//! skips `PerformHourglass` but `Refresh`/director work, sound `Hourglass`, and
//! the first `PostInitialize` still run (`original-code/RHgame.cpp:1867-1919`).
//! `simulation_body_allowed = false` is not that boundary: it still runs the
//! mission/script phase and advances the engine clock. Until the journal has a
//! distinct no-hourglass boundary, this API is not a universal host-frame
//! driver for paused, console, modal-transition, or load/next states.
//!
//! TODO(architecture): choose one timeline owner before production migration.
//! This transaction advances `EngineInner::frame_counter`, but the current
//! drivers separately own and increment `EngineManager::sim_frame`. A history
//! commit must not infer one counter from the other until that split ownership
//! is removed or represented explicitly.

use serde::{Deserialize, Serialize};

use super::{DirectorCompletion, SideEffects};
use crate::game_operation::GameCode;
use crate::player_command::{PlayerCommand, PlayerId, PlayerInput};
use crate::sound::ResolvedExclamation;

/// A fully resolved, deterministic command admitted to a simulation frame.
///
/// This newtype is the migration seam between the broad historical
/// [`PlayerCommand`] enum and the simulation-frame API. Raw platform/UI
/// actions must be resolved before constructing it. Its transparent wire
/// representation preserves the existing [`PlayerInput`] schema.
#[derive(Clone, Debug, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
#[serde(transparent)]
pub struct SimCommand(PlayerInput);

impl SimCommand {
    pub fn new(player_id: PlayerId, command: PlayerCommand) -> Self {
        Self(PlayerInput::new(player_id, command))
    }

    pub fn host(command: PlayerCommand) -> Self {
        Self(PlayerInput::host(command))
    }

    pub fn player_input(&self) -> &PlayerInput {
        &self.0
    }

    pub fn into_player_input(self) -> PlayerInput {
        self.0
    }
}

impl From<PlayerInput> for SimCommand {
    fn from(input: PlayerInput) -> Self {
        Self(input)
    }
}

impl From<PlayerCommand> for SimCommand {
    fn from(command: PlayerCommand) -> Self {
        Self::host(command)
    }
}

impl From<SimCommand> for PlayerInput {
    fn from(command: SimCommand) -> Self {
        command.into_player_input()
    }
}

/// A nondeterministic host observation made authoritative before entering the
/// simulation frame.
///
/// Facts are applied in vector order, before every [`SimCommand`]. This is
/// significant for Original parity: director completions and the sound
/// manager's resolution boundary occur after one `PerformHourglass` but before
/// the following frame's translated input messages. When a frame contains both,
/// Original order is all director completions produced by `Refresh`, followed
/// by the sound-manager boundary (`original-code/RHgame.cpp:1879-1915`).
///
/// TODO(architecture): replace this free-form vector with phase-typed fields,
/// or validate its phase ordering, so callers cannot encode a sound boundary
/// before a director completion or more than one host sound boundary.
#[derive(Clone, Debug, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ExternalFact {
    /// A camera-director command completed during the preceding render pass.
    DirectorCompletion(DirectorCompletion),
    /// Concrete speech durations resolved by the live host sound manager.
    ///
    /// Live resolutions must match the simulation's pending speech FIFO.
    LiveSoundBoundary(Vec<ResolvedExclamation>),
    /// Authoritative speech durations imported from an Original parity trace.
    ///
    /// Original-only speech may have no matching Rust logical request, so this
    /// variant uses the replay-specific validation policy.
    ReplaySoundBoundary(Vec<ResolvedExclamation>),
}

/// Authoritative inputs modeled by one admitted engine-hourglass transaction.
///
/// External facts always run before commands. `simulation_body_allowed`
/// closes only the actor/world body gate; mission scripts/messages and the
/// mission clock still advance, matching the existing gated-hourglass rules.
/// It must not be used to represent a host iteration where `PerformHourglass`
/// was skipped entirely. It also does not yet carry authoritative commands
/// produced by a nested refresh after the main hourglass body.
#[derive(Clone, Debug, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SimulationFrameInput {
    pub external_facts: Vec<ExternalFact>,
    /// Commands admitted before `PerformHourglass`, in dispatch order.
    ///
    /// TODO(architecture): add a typed post-hourglass command phase for
    /// Original parity traces whose nested refresh records a late resolved
    /// orientation, and route post-tick host command/native/console mutations
    /// through it. Moving those operations into this vector changes behavior.
    pub commands: Vec<SimCommand>,
    #[serde(default = "simulation_body_allowed_default")]
    pub simulation_body_allowed: bool,
}

const fn simulation_body_allowed_default() -> bool {
    true
}

impl Default for SimulationFrameInput {
    fn default() -> Self {
        Self {
            external_facts: Vec::new(),
            commands: Vec::new(),
            simulation_body_allowed: true,
        }
    }
}

impl SimulationFrameInput {
    pub fn new(commands: Vec<SimCommand>) -> Self {
        Self {
            commands,
            ..Self::default()
        }
    }

    pub fn from_player_inputs(commands: Vec<PlayerInput>) -> Self {
        Self::new(commands.into_iter().map(SimCommand::from).collect())
    }

    pub fn with_external_facts(mut self, external_facts: Vec<ExternalFact>) -> Self {
        self.external_facts = external_facts;
        self
    }

    pub fn with_simulation_body_allowed(mut self, allowed: bool) -> Self {
        self.simulation_body_allowed = allowed;
        self
    }
}

/// Ordered output events produced by the modeled hourglass transaction.
///
/// [`SideEffects`] remains the compatibility payload while callers migrate to
/// the frame API. Keeping it behind this type makes the sim-to-host direction
/// explicit without re-encoding or reordering any existing event fields.
/// During the host-scratch migration this payload still includes the
/// adapter-only `SideEffects::pending_minimap_position`; that field can depend
/// on [`super::HostDisplayState`] and is deliberately skipped by `SideEffects`
/// serialization. It is available through [`Self::side_effects`] in memory.
#[derive(Clone, Debug, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
#[serde(transparent)]
pub struct SimEvents(SideEffects);

impl SimEvents {
    pub fn game_code(&self) -> GameCode {
        self.0.code
    }

    pub fn side_effects(&self) -> &SideEffects {
        &self.0
    }

    pub fn into_side_effects(self) -> SideEffects {
        self.0
    }
}

impl From<SideEffects> for SimEvents {
    fn from(side_effects: SideEffects) -> Self {
        Self(side_effects)
    }
}

impl From<SimEvents> for SideEffects {
    fn from(events: SimEvents) -> Self {
        events.into_side_effects()
    }
}

/// Result of one admitted engine-hourglass transaction.
#[derive(Clone, Debug, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SimulationFrameOutput {
    /// Engine frame counter on entry.
    pub frame_before: u32,
    /// Engine frame counter after the hourglass. A presentation-only freeze
    /// frame can leave this equal to `frame_before`.
    pub frame_after: u32,
    /// Output for the host to consume after the transaction. Until host scratch
    /// is fully disentangled, this includes the adapter-only minimap
    /// persistence effect documented on [`SimEvents`].
    pub events: SimEvents,
    /// Canonical deterministic engine-state hash after this modeled
    /// transaction, before any unmodeled post-hourglass ingress.
    pub state_hash: u64,
}

impl SimulationFrameOutput {
    pub fn game_code(&self) -> GameCode {
        self.events.game_code()
    }
}

/// An authoritative external fact was incompatible with the current
/// deterministic state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum FrameAdvanceError {
    #[error("external fact {index} rejected {completion:?}: {reason}")]
    DirectorCompletionRejected {
        index: usize,
        completion: DirectorCompletion,
        reason: String,
    },
}
