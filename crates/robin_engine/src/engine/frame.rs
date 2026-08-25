//! Transaction boundary for one deterministic simulation frame.
//!
//! The Original's game loop finishes input/message translation before calling
//! `RHEngine::PerformHourglass`, then performs widgets, rendering, and sound
//! afterwards (`original-code/RHgame.cpp:1562-1915`). This module names that
//! boundary without changing the established hourglass implementation or its
//! phase ordering.
//!
//! [`SimulationFrameInput`] deliberately contains only authoritative inputs:
//! resolved [`SimCommand`]s and host observations represented as
//! [`ExternalFact`]s. Host UI scratch remains an adapter argument to
//! [`super::Engine::advance_frame`] during migration and cannot be serialized
//! accidentally as part of the frame input.
//!
//! TODO(architecture): remove those host scratch arguments once remaining
//! command/tick handlers emit sim events instead of editing presentation state.

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
/// the following frame's translated input messages.
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

/// All authoritative inputs for one admitted simulation frame.
///
/// External facts always run before commands. `simulation_body_allowed`
/// closes only the actor/world body gate; mission scripts/messages and the
/// mission clock still advance, matching the existing gated-hourglass rules.
#[derive(Clone, Debug, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SimulationFrameInput {
    pub external_facts: Vec<ExternalFact>,
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

/// Ordered simulation-originated events produced by one frame.
///
/// [`SideEffects`] remains the compatibility payload while callers migrate to
/// the frame API. Keeping it behind this type makes the sim-to-host direction
/// explicit without re-encoding or reordering any existing event fields.
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

/// Result of one complete simulation-frame transaction.
#[derive(Clone, Debug, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SimulationFrameOutput {
    /// Engine frame counter on entry.
    pub frame_before: u32,
    /// Engine frame counter after the hourglass. A presentation-only freeze
    /// frame can leave this equal to `frame_before`.
    pub frame_after: u32,
    /// Simulation-originated output for the host to consume after the frame.
    pub events: SimEvents,
    /// Canonical deterministic engine-state hash after the frame.
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
