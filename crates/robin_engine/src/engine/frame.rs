//! Transaction boundary around one deterministic host-admitted engine frame.
//!
//! The Original's game loop finishes input/message translation before calling
//! `RHEngine::PerformHourglass`, then performs widgets, rendering, and sound
//! afterwards (`original-code/RHgame.cpp:1562-1915`). This module names that
//! boundary without changing the established hourglass implementation or its
//! phase ordering.
//!
//! This is the authoritative engine portion of an `RHGame` host-loop frame.
//! It can represent the Original gate skipping `PerformHourglass`, preserves
//! distinct pre/post-hourglass command phases, and can cross the one-shot
//! `PostInitialize` lifecycle boundary. Rendering, widgets, and audio playback
//! remain host responsibilities, but every gameplay mutation they resolve is
//! admitted through [`super::Engine::advance_frame`].
//!
//! [`SimulationFrameInput`] deliberately contains only authoritative inputs:
//! resolved [`SimCommand`]s, host observations represented as
//! [`ExternalFact`]s, and explicitly admitted host [`ExternalAction`]s.
//! Host UI scratch never crosses [`super::Engine::advance_frame`]. Commands
//! carry every host-resolved fact they need, while presentation/input/dev
//! changes leave through typed output events or action results.
//!
//! Journals should retain this whole value, not just
//! [`SimulationFrameInput::commands`], so a
//! reconstruction cannot lose host-resolved facts, an hourglass gate, or a
//! late command. The current replay file remains command-only; its adapter
//! constructs the explicit defaults used by that legacy format.
//!
//! TODO(architecture): choose one timeline owner before production migration.
//! This transaction advances `EngineInner::frame_counter`, but the current
//! drivers separately own and increment `EngineManager::sim_frame`. A history
//! commit must not infer one counter from the other until that split ownership
//! is removed or represented explicitly.

use serde::{Deserialize, Serialize};

use super::{ConsoleResponse, DirectorCompletion, SideEffects};
use crate::campaign::Campaign;
use crate::console::ConsoleCommand;
use crate::element::EntityId;
use crate::game_operation::GameCode;
use crate::messenger::SimpleMessage;
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

/// An explicitly admitted host action that can mutate simulation state.
///
/// Console submissions can run before the main hourglass; HTTP native/console
/// RPCs run after it. Both need a deterministic journal representation so
/// rewind and rollback do not silently omit the mutation. A live adapter may
/// execute an action immediately in its own no-hourglass admission to produce a
/// synchronous reply, then place the same value in the enclosing host frame's
/// journal record.
#[derive(Clone, Debug, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExternalAction {
    Native {
        name: String,
        args: Vec<i32>,
        this_actor: Option<i32>,
    },
    /// A parsed simulation-affecting console command. Host-only developer
    /// commands are resolved and applied before admission.
    ConsoleCommand {
        command: ConsoleCommand,
        selected_view_element: Option<EntityId>,
    },
    SimpleMessage {
        message: SimpleMessage,
    },
    EzekielInstakill {
        target: EntityId,
    },
    ReplaceCampaign {
        campaign: Campaign,
    },
}

/// Serializable result of an admitted host action.
#[derive(Clone, Debug, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ExternalActionResult {
    Native(Result<i32, String>),
    ConsoleCommand {
        response: FrameConsoleResponse,
        selected_view_element: Option<EntityId>,
    },
    SimpleMessage,
    EzekielInstakill(bool),
    ReplaceCampaign,
}

/// Owned/serializable form of [`ConsoleResponse`] used at the frame boundary.
#[derive(Clone, Debug, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum FrameConsoleResponse {
    Ok(String),
    Unknown,
    NotImplemented(String),
    LoadCampaignRequested(String),
    DeityInvoked,
}

impl From<ConsoleResponse> for FrameConsoleResponse {
    fn from(response: ConsoleResponse) -> Self {
        match response {
            ConsoleResponse::Ok(message) => Self::Ok(message),
            ConsoleResponse::Unknown => Self::Unknown,
            ConsoleResponse::NotImplemented(name) => Self::NotImplemented(name.to_owned()),
            ConsoleResponse::LoadCampaignRequested(path) => Self::LoadCampaignRequested(
                path.into_os_string()
                    .into_string()
                    .unwrap_or_else(|_| panic!("console campaign path is not valid UTF-8")),
            ),
            ConsoleResponse::DeityInvoked => Self::DeityInvoked,
        }
    }
}

/// Authoritative inputs modeled by one admitted engine-hourglass transaction.
///
/// Ordering is external facts, pre-hourglass external actions, commands, the
/// hourglass, post-hourglass external actions, late commands, then optional
/// `PostInitialize`. `simulation_body_allowed` closes only the actor/world body
/// gate; mission scripts/messages and the mission clock still advance. It must
/// not represent a host iteration where `PerformHourglass` was skipped
/// entirely; use `run_hourglass` for that host gate.
#[derive(Clone, Debug, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SimulationFrameInput {
    pub external_facts: Vec<ExternalFact>,
    /// Host actions admitted before player commands and the hourglass.
    #[serde(default)]
    pub external_actions: Vec<ExternalAction>,
    /// Commands admitted before `PerformHourglass`, in dispatch order.
    pub commands: Vec<SimCommand>,
    /// Host/native actions admitted after the hourglass and before late
    /// player commands. Live RPC adapters record actions here after executing
    /// them synchronously through a separate no-hourglass admission.
    #[serde(default)]
    pub post_external_actions: Vec<ExternalAction>,
    /// Commands admitted after `PerformHourglass`, in dispatch order.
    ///
    /// Original parity uses this phase for input resolved by a nested refresh.
    /// Live host RPC/modal ingress also belongs here when it occurs after the
    /// main tick.
    #[serde(default)]
    pub post_commands: Vec<SimCommand>,
    /// Whether this host iteration crosses `RHEngine::PerformHourglass`.
    /// Paused, console, modal-transition, load, and level-next iterations set
    /// this to false; this is distinct from `simulation_body_allowed`.
    #[serde(default = "run_hourglass_default")]
    pub run_hourglass: bool,
    #[serde(default = "simulation_body_allowed_default")]
    pub simulation_body_allowed: bool,
    /// Cross the one-shot mission `PostInitialize` stage after late commands.
    /// Graphical play normally does this in a separate no-hourglass admission
    /// after presentation; headless/reconstruction may combine it.
    #[serde(default)]
    pub run_post_initialize: bool,
}

const fn run_hourglass_default() -> bool {
    true
}

const fn simulation_body_allowed_default() -> bool {
    true
}

impl Default for SimulationFrameInput {
    fn default() -> Self {
        Self {
            external_facts: Vec::new(),
            external_actions: Vec::new(),
            commands: Vec::new(),
            post_external_actions: Vec::new(),
            post_commands: Vec::new(),
            run_hourglass: true,
            simulation_body_allowed: true,
            run_post_initialize: false,
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

    pub fn with_external_actions(mut self, actions: Vec<ExternalAction>) -> Self {
        self.external_actions = actions;
        self
    }

    pub fn with_simulation_body_allowed(mut self, allowed: bool) -> Self {
        self.simulation_body_allowed = allowed;
        self
    }

    pub fn with_post_commands(mut self, commands: Vec<SimCommand>) -> Self {
        self.post_commands = commands;
        self
    }

    pub fn with_post_external_actions(mut self, actions: Vec<ExternalAction>) -> Self {
        self.post_external_actions = actions;
        self
    }

    pub fn with_hourglass(mut self, run: bool) -> Self {
        self.run_hourglass = run;
        self
    }

    pub fn with_post_initialize(mut self, run: bool) -> Self {
        self.run_post_initialize = run;
        self
    }

    /// A host iteration which crosses no hourglass and admits no commands.
    pub fn no_hourglass() -> Self {
        Self::default().with_hourglass(false)
    }

    pub fn player_inputs(&self) -> Vec<PlayerInput> {
        self.commands
            .iter()
            .map(|command| command.player_input().clone())
            .collect()
    }

    pub fn post_player_inputs(&self) -> Vec<PlayerInput> {
        self.post_commands
            .iter()
            .map(|command| command.player_input().clone())
            .collect()
    }
}

/// Ordered output events produced by the modeled hourglass transaction.
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

/// Result of one admitted engine-hourglass transaction.
#[derive(Clone, Debug, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SimulationFrameOutput {
    /// Engine frame counter on entry.
    pub frame_before: u32,
    /// Engine frame counter after the hourglass. A presentation-only freeze
    /// frame can leave this equal to `frame_before`.
    pub frame_after: u32,
    /// True exactly when this admission ran `PerformHourglass`.
    pub hourglass_ran: bool,
    /// Output for the host to consume after the transaction.
    pub events: SimEvents,
    /// Ordered effects produced by the optional one-shot lifecycle stage.
    pub post_initialize_events: Option<SimEvents>,
    /// Results for pre- then post-hourglass external actions, in order.
    pub external_action_results: Vec<ExternalActionResult>,
    /// Canonical deterministic engine-state hash after the full modeled
    /// transaction.
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
