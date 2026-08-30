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
//! [`ExternalFacts`], and explicitly admitted host [`ExternalAction`]s.
//! Host UI scratch never crosses [`super::Engine::advance_frame`]. Commands
//! carry every host-resolved fact they need, while presentation/input/dev
//! changes leave through typed output events or action results.
//!
//! Journals and disk replays retain this whole value, not just
//! [`SimulationFrameInput::commands`], so reconstruction cannot lose
//! host-resolved facts, an hourglass gate, or a late command.
//!
//! This transaction advances the engine's Original-compatible simulation tick
//! counter only when the hourglass runs. The host's history/network/replay
//! frame is owned separately by `TimelineRuntime`: a committed skipped-
//! hourglass frame advances that cursor without pretending an engine tick ran.

use serde::{Deserialize, Serialize};

use super::{ConsoleResponse, DirectorCompletion, SideEffects};
use crate::campaign::Campaign;
use crate::console::ConsoleCommand;
use crate::element::EntityId;
use crate::game_operation::GameCode;
use crate::messenger::SimpleMessage;
use crate::player_command::{PlayerCommand, PlayerId, PlayerInput};
use crate::sound::ResolvedExclamation;

/// Original-compatible engine simulation clock.
///
/// This counts completed hourglass ticks inside deterministic engine state. It
/// is not a network/history frame and not a replay host-record ordinal.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(transparent)]
pub struct SimulationTick(u32);

impl SimulationTick {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn number(self) -> u32 {
        self.0
    }
}

/// A fully resolved, deterministic command admitted to a simulation frame.
///
/// This newtype is the migration seam between the broad historical
/// [`PlayerCommand`] enum and the simulation-frame API. Raw platform/UI
/// actions must be resolved before constructing it. Its transparent wire
/// representation preserves the existing [`PlayerInput`] schema.
#[derive(
    Clone,
    Debug,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
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

/// Validation policy for a host sound-manager boundary.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
pub enum SoundBoundaryPolicy {
    /// Resolutions came from the live Rust host and must match the simulation's
    /// pending speech FIFO.
    Live,
    /// Resolutions came from an Original trace and may describe Original-only
    /// speech for which Rust has no corresponding logical request.
    Replay,
}

/// The single sound-manager boundary which may precede a simulation frame.
#[derive(
    Clone,
    Debug,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SoundBoundary {
    /// Whether FIFO validation follows live Rust or Original replay rules.
    pub policy: SoundBoundaryPolicy,
    /// Concrete speech durations resolved at this boundary, in callback order.
    pub resolutions: Vec<ResolvedExclamation>,
}

/// An Original-trace gate-search result for a postponed DropAle seek.
///
/// Original resolves this only when the pending movement is instructed, which
/// may be several frames after the command created it. Parity adapters resolve
/// the trace identity outside the engine, then admit the concrete result with
/// the frame whose hourglass consumes it.
#[derive(
    Clone,
    Debug,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RecordedDropAleRoute {
    pub actor: EntityId,
    pub destination: crate::coordinates::MapPoint,
    pub goal_sector: crate::sector::SectorNumber,
    pub goal_sector_index: crate::fast_find_grid::SectorIndex,
    pub goal_layer: u16,
    pub recorded_gate_path: crate::gate::RecordedGatePath,
}

impl SoundBoundary {
    pub fn live(resolutions: Vec<ResolvedExclamation>) -> Self {
        Self {
            policy: SoundBoundaryPolicy::Live,
            resolutions,
        }
    }

    pub fn replay(resolutions: Vec<ResolvedExclamation>) -> Self {
        Self {
            policy: SoundBoundaryPolicy::Replay,
            resolutions,
        }
    }
}

/// Nondeterministic host observations made authoritative before entering the
/// simulation frame.
///
/// The structure encodes the only legal Original phase order: every director
/// completion produced by `RHEngine::PerformDirectorWork` during `Refresh`,
/// followed by zero or one `RHSound::Hourglass` boundary and any recorded
/// delayed-route results consumed by the upcoming hourglass. A caller cannot
/// put a director completion after sound or encode multiple sound boundaries
/// (`original-code/RHgame.cpp:1867-1926`, `RHengine.cpp:4172`,
/// `RHsound.cpp:2125-2250`).
#[derive(
    Clone,
    Debug,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ExternalFacts {
    /// Camera-director completions produced during the preceding refresh, in
    /// termination order.
    pub director_completions: Vec<DirectorCompletion>,
    /// The following sound-manager phase, when that host boundary was crossed.
    pub sound_boundary: Option<SoundBoundary>,
    /// Recorded gate-search results for postponed DropAle seeks which become
    /// instructible in this frame, in trace order.
    pub recorded_drop_ale_routes: Vec<RecordedDropAleRoute>,
}

impl ExternalFacts {
    pub fn new(
        director_completions: Vec<DirectorCompletion>,
        sound_boundary: Option<SoundBoundary>,
    ) -> Self {
        Self {
            director_completions,
            sound_boundary,
            recorded_drop_ale_routes: Vec::new(),
        }
    }

    pub fn with_director_completions(
        mut self,
        director_completions: Vec<DirectorCompletion>,
    ) -> Self {
        self.director_completions = director_completions;
        self
    }

    pub fn with_sound_boundary(mut self, sound_boundary: SoundBoundary) -> Self {
        self.sound_boundary = Some(sound_boundary);
        self
    }

    pub fn with_recorded_drop_ale_routes(
        mut self,
        recorded_drop_ale_routes: Vec<RecordedDropAleRoute>,
    ) -> Self {
        self.recorded_drop_ale_routes = recorded_drop_ale_routes;
        self
    }

    pub fn is_empty(&self) -> bool {
        self.director_completions.is_empty()
            && self.sound_boundary.is_none()
            && self.recorded_drop_ale_routes.is_empty()
    }
}

/// An explicitly admitted host action that can mutate simulation state.
///
/// Console submissions can run before the main hourglass; HTTP native/console
/// RPCs run after it. Both need a deterministic journal representation so
/// rewind and rollback do not silently omit the mutation. A live adapter may
/// execute an action immediately in its own no-hourglass admission to produce a
/// synchronous reply, then place the same value in the enclosing host frame's
/// journal record.
#[derive(
    Clone,
    Debug,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
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
    /// Commit the campaign-map ransom/blazon conversion while the host keeps
    /// `PerformHourglass` paused. The enclosing frame journals this action
    /// even when the live modal needs its result synchronously.
    CampaignBuyBlazon {
        mission_index: u32,
    },
    /// Acknowledge the pseudo-mission debrief after the modal is dismissed.
    /// Original performs this on the paused campaign-map boundary.
    AcknowledgePseudoMissionDebrief,
}

/// Serializable result of an admitted host action.
#[derive(
    Clone,
    Debug,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
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
    CampaignBuyBlazon {
        closed_by_cascade: bool,
    },
    AcknowledgePseudoMissionDebrief,
}

/// Owned/serializable form of [`ConsoleResponse`] used at the frame boundary.
#[derive(
    Clone,
    Debug,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
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
#[derive(
    Clone,
    Debug,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SimulationFrameInput {
    /// Ordered facts observed between the preceding tick and these commands.
    pub external_facts: ExternalFacts,
    /// Host actions admitted before player commands and the hourglass.
    pub external_actions: Vec<ExternalAction>,
    /// Commands admitted before `PerformHourglass`, in dispatch order.
    pub commands: Vec<SimCommand>,
    /// Host/native actions admitted after the hourglass and before late
    /// player commands. Live RPC adapters record actions here after executing
    /// them synchronously through a separate no-hourglass admission.
    pub post_external_actions: Vec<ExternalAction>,
    /// Commands admitted after `PerformHourglass`, in dispatch order.
    ///
    /// Original parity uses this phase for input resolved by a nested refresh.
    /// Live host RPC/modal ingress also belongs here when it occurs after the
    /// main tick.
    pub post_commands: Vec<SimCommand>,
    /// Whether this host iteration crosses `RHEngine::PerformHourglass`.
    /// Paused, console, modal-transition, load, and level-next iterations set
    /// this to false; this is distinct from `simulation_body_allowed`.
    pub run_hourglass: bool,
    pub simulation_body_allowed: bool,
    /// Cross the one-shot mission `PostInitialize` stage after late commands.
    /// Graphical play normally does this in a separate no-hourglass admission
    /// after presentation; headless/reconstruction may combine it.
    pub run_post_initialize: bool,
}

impl Default for SimulationFrameInput {
    fn default() -> Self {
        Self {
            external_facts: ExternalFacts::default(),
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

    pub fn with_external_facts(mut self, external_facts: ExternalFacts) -> Self {
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
/// The host-local `SideEffects::pending_minimap_position` remains available in
/// memory even though the `SideEffects` Serde representation deliberately
/// skips it.
#[derive(
    Clone,
    Debug,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
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
#[derive(
    Clone,
    Debug,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
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
    /// Ordered effects emitted by post-hourglass external actions and player
    /// commands. These must be delivered even when no hourglass or one-shot
    /// PostInitialize stage runs; otherwise a modal-issued command can strand
    /// its acknowledgement behind the modal that is waiting for it.
    pub post_boundary_events: SimEvents,
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
#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    thiserror::Error,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum FrameAdvanceError {
    #[error("director completion {index} rejected {completion:?}: {reason}")]
    DirectorCompletionRejected {
        index: usize,
        completion: DirectorCompletion,
        reason: String,
    },
    #[error("{policy:?} sound boundary rejected: {reason}")]
    SoundBoundaryRejected {
        policy: SoundBoundaryPolicy,
        reason: String,
    },
    #[error("recorded DropAle route {index} for {actor:?} rejected: {reason}")]
    RecordedDropAleRouteRejected {
        index: usize,
        actor: EntityId,
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_util::state_hash::StateHash;
    use std::hash::{DefaultHasher, Hasher};

    fn deterministic_hash(value: &impl StateHash) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.state_hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn external_facts_serde_preserves_phases_and_sound_policy() {
        let facts = ExternalFacts::new(
            vec![DirectorCompletion::CameraGoto],
            Some(SoundBoundary::replay(vec![ResolvedExclamation {
                actor_id: 7,
                identifier: 0x4651_003e,
                exclamation_id: 62,
                duration_frames: 24,
            }])),
        );

        let encoded = serde_json::to_value(&facts).expect("serialize external facts");
        assert_eq!(encoded["director_completions"][0]["command"], "camera_goto");
        assert_eq!(encoded["sound_boundary"]["policy"], "replay");
        let decoded: ExternalFacts =
            serde_json::from_value(encoded).expect("deserialize external facts");
        assert!(matches!(
            decoded.director_completions.as_slice(),
            [DirectorCompletion::CameraGoto]
        ));
        let boundary = decoded.sound_boundary.expect("sound boundary");
        assert_eq!(boundary.policy, SoundBoundaryPolicy::Replay);
        assert_eq!(boundary.resolutions.len(), 1);
        assert_eq!(boundary.resolutions[0].actor_id, 7);
    }

    #[test]
    fn external_fact_hash_distinguishes_policy_and_an_empty_boundary() {
        let live_empty =
            ExternalFacts::default().with_sound_boundary(SoundBoundary::live(Vec::new()));
        let replay_empty =
            ExternalFacts::default().with_sound_boundary(SoundBoundary::replay(Vec::new()));

        assert_ne!(
            deterministic_hash(&ExternalFacts::default()),
            deterministic_hash(&replay_empty),
            "an explicit empty replay boundary still drains the Original sound phase"
        );
        assert_ne!(
            deterministic_hash(&live_empty),
            deterministic_hash(&replay_empty),
            "live and replay validation policy is authoritative"
        );
    }

    #[test]
    fn native_frame_codec_preserves_console_external_actions() {
        let action = ExternalAction::ConsoleCommand {
            command: ConsoleCommand::GiveMoney {
                amount: 1_000,
                show_help: true,
            },
            selected_view_element: None,
        };

        let encoded = bitcode::encode(&action);
        let decoded: ExternalAction =
            bitcode::decode(&encoded).expect("decode native console external action");

        assert!(matches!(
            decoded,
            ExternalAction::ConsoleCommand {
                command: ConsoleCommand::GiveMoney {
                    amount: 1_000,
                    show_help: true,
                },
                selected_view_element: None,
            }
        ));
    }

    #[test]
    fn simulation_frame_input_deserialization_requires_every_field() {
        let complete = serde_json::to_value(SimulationFrameInput::default())
            .expect("serialize complete frame input");

        for field in [
            "external_facts",
            "external_actions",
            "commands",
            "post_external_actions",
            "post_commands",
            "run_hourglass",
            "simulation_body_allowed",
            "run_post_initialize",
        ] {
            let mut incomplete = complete.clone();
            incomplete
                .as_object_mut()
                .expect("frame input serializes as an object")
                .remove(field);

            let error = serde_json::from_value::<SimulationFrameInput>(incomplete)
                .expect_err("schema-12 frame inputs must include every current field");
            assert!(
                error
                    .to_string()
                    .contains(&format!("missing field `{field}`")),
                "removing {field} produced unexpected error: {error}"
            );
        }
    }
}
