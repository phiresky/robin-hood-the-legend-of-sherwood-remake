//! Owning state and phase boundaries for one loaded mission.
//!
//! The native and headless loops still differ in input, modal, and
//! presentation work, but they share the same deterministic frame
//! bookkeeping through [`TimelineRuntime`].

use super::multiplayer::{MultiplayerAdmissionEvent, MultiplayerRollbackTelemetry};
use super::replay_init::ReplayAndRollback;
use crate::game::Game;
use crate::host::Host;
use crate::rewind::RewindBuffer;
use crate::save_file::{GameRuntimeSnapshot, ReplaySaveIdentity};
use robin_engine::engine::{DevState, Engine, LevelAssets};
use robin_engine::engine_manager::EngineManager;
use robin_engine::game_operation::GameCode;
use robin_engine::player_command::{FrameCommands, PlayerCommand};
use robin_engine::replay::{ReplayPlayer, ReplayRecorder};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

pub(super) use robin_engine::replay::{ReplayFrameOrdinal, TimelineFrame};

mod history;
pub(super) mod reconciliation;
use history::ReconstructionHistory;
use reconciliation::NetworkReconciliation;

/// Result of asking the timeline for the next replay-owned debugger step.
///
/// A live debugger step and an exhausted replay are deliberately different:
/// only the former may synthesize the normal empty/PostInitialize frame used
/// when stepping a non-replay session.
#[derive(Debug)]
pub(super) enum ReplayStepAdmission {
    NoActiveReplay,
    Recorded(robin_engine::replay::ReplayFrame),
    Finished { ordinal: u32, total_frames: u32 },
}

/// Explicit lockstep cursor transition carried by one persisted replay record.
/// It cannot be derived from hourglass admission: host actions, multiplayer
/// admission, debugger steps, and frozen engine ticks have distinct policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct TimelineTransition {
    pub(super) before: TimelineFrame,
    pub(super) after: TimelineFrame,
}

/// Common owned state for one loaded mission.
///
/// This deliberately stops at the simulation/host boundary. Renderer, input,
/// audio backend, and modal resources are owned by the graphical driver's
/// `InteractiveFrontend`. None of these process resources is serializable;
/// deterministic persistence remains the Engine snapshot.
pub(super) struct MissionWorld {
    host: Host,
    game: Game,
    manager: EngineManager,
    assets: Arc<LevelAssets>,
    dev: DevState,
}

/// Capability-limited borrow for wire ingress and rollback adoption.
///
/// Keeping `Game` and developer state out of this context makes it impossible
/// for the network prelude to mutate unrelated mission state while admitting
/// inputs or snapshots.
pub(super) struct MissionIngress<'a> {
    pub(super) host: &'a mut Host,
    pub(super) manager: &'a mut EngineManager,
    pub(super) assets: &'a mut Arc<LevelAssets>,
}

/// Read-only view used by pacing, diagnostics, and presentation decisions.
pub(super) struct MissionWorldView<'a> {
    pub(super) host: &'a Host,
    pub(super) manager: &'a EngineManager,
}

/// Host-only mutation capability for modal/effect drains.
pub(super) struct MissionHostPhase<'a> {
    pub(super) host: &'a mut Host,
}

/// Read-only simulation plus command outputs for live input.
/// The consumer cannot tick, restore, replace the engine, or edit frame/history
/// metadata. Snapshot/rewind/console work uses the explicit mutation boundary.
pub(super) struct MissionInputPhase<'a> {
    pub(super) host: &'a mut Host,
    pub(super) game: &'a mut Game,
    pub(super) engine: &'a Engine,
    pub(super) assets: &'a Arc<LevelAssets>,
    pub(super) dev: &'a mut DevState,
    pub(super) commands: FrameCommandBatch<'a>,
    pub(super) external_actions: FrameActionBatch<'a>,
}

/// Explicit privileged simulation mutation boundary: simulation, snapshot
/// adoption, rewind, console administration, and post-initialization.
/// These used to be equivalent input/operation/simulation phase wrappers.
/// This capability must not cross into rendering or live command producers.
pub(super) struct MissionMutation<'a> {
    pub(super) host: &'a mut Host,
    pub(super) game: &'a mut Game,
    pub(super) manager: &'a mut EngineManager,
    pub(super) assets: &'a Arc<LevelAssets>,
    pub(super) dev: &'a mut DevState,
}

/// Borrow issued at the final deterministic pre-tick boundary.
pub(super) struct MissionPreTickPhase<'a> {
    pub(super) host: &'a mut Host,
    pub(super) game: &'a mut Game,
    pub(super) manager: &'a mut EngineManager,
    pub(super) assets: &'a mut Arc<LevelAssets>,
}

/// Borrow issued only while process-side audio is drained.
pub(super) struct MissionAudioPhase<'a> {
    pub(super) audio: &'a mut crate::host::HostAudio,
    pub(super) viewport: &'a crate::host::ViewportState,
    pub(super) engine: &'a Engine,
    pub(super) assets: &'a Arc<LevelAssets>,
}

/// Render capability: host presentation can change, simulation and developer
/// state cannot. There is deliberately no EngineManager or mutable Engine.
pub(super) struct MissionPresentationPhase<'a> {
    pub(super) host: crate::host::HostPresentation<'a>,
    pub(super) game: &'a mut Game,
    pub(super) engine: &'a Engine,
    pub(super) assets: &'a Arc<LevelAssets>,
    pub(super) dev: &'a DevState,
}

impl MissionWorld {
    pub(super) fn new(
        host: Host,
        game: Game,
        manager: EngineManager,
        assets: Arc<LevelAssets>,
        dev: DevState,
    ) -> Self {
        Self {
            host,
            game,
            manager,
            assets,
            dev,
        }
    }

    pub(super) fn into_campaign_and_simulation(
        self,
    ) -> (
        robin_engine::campaign::Campaign,
        u64,
        robin_engine::engine::SimConfig,
    ) {
        self.manager.engine.into_campaign_and_simulation()
    }

    pub(super) fn preserve_multiplayer_session_for_next_mission(&mut self) {
        self.host.transport.preserve_session_for_next_mission();
    }

    pub(super) fn ingress(&mut self) -> MissionIngress<'_> {
        MissionIngress {
            host: &mut self.host,
            manager: &mut self.manager,
            assets: &mut self.assets,
        }
    }

    pub(super) fn view(&self) -> MissionWorldView<'_> {
        MissionWorldView {
            host: &self.host,
            manager: &self.manager,
        }
    }

    pub(super) fn host_phase(&mut self) -> MissionHostPhase<'_> {
        MissionHostPhase {
            host: &mut self.host,
        }
    }

    pub(super) fn input_phase<'a>(
        &'a mut self,
        frame: &'a mut MissionFrame,
    ) -> MissionInputPhase<'a> {
        MissionInputPhase {
            host: &mut self.host,
            game: &mut self.game,
            engine: &self.manager.engine,
            assets: &self.assets,
            dev: &mut self.dev,
            commands: FrameCommandBatch::new(&mut frame.commands),
            external_actions: FrameActionBatch::new(&mut frame.external_actions),
        }
    }

    pub(super) fn mutation(&mut self) -> MissionMutation<'_> {
        MissionMutation {
            host: &mut self.host,
            game: &mut self.game,
            manager: &mut self.manager,
            assets: &self.assets,
            dev: &mut self.dev,
        }
    }

    /// Cursor/orientation producers run after simulation and append to the
    /// post-command batch. They must never mutate the already-consumed inputs.
    pub(super) fn post_tick_input_phase<'a>(
        &'a mut self,
        frame: &'a mut MissionFrame,
    ) -> MissionInputPhase<'a> {
        MissionInputPhase {
            host: &mut self.host,
            game: &mut self.game,
            engine: &self.manager.engine,
            assets: &self.assets,
            dev: &mut self.dev,
            commands: FrameCommandBatch::new(&mut frame.post_commands),
            external_actions: FrameActionBatch::new(&mut frame.post_external_actions),
        }
    }

    pub(super) fn pre_tick_phase(&mut self) -> MissionPreTickPhase<'_> {
        MissionPreTickPhase {
            host: &mut self.host,
            game: &mut self.game,
            manager: &mut self.manager,
            assets: &mut self.assets,
        }
    }

    pub(super) fn audio_phase(&mut self) -> MissionAudioPhase<'_> {
        MissionAudioPhase {
            audio: &mut self.host.audio,
            viewport: &self.host.frontend.viewport,
            engine: &self.manager.engine,
            assets: &self.assets,
        }
    }

    pub(super) fn presentation_phase(&mut self) -> MissionPresentationPhase<'_> {
        MissionPresentationPhase {
            host: self.host.presentation(),
            game: &mut self.game,
            engine: &self.manager.engine,
            assets: &self.assets,
            dev: &self.dev,
        }
    }

    pub(super) fn dismiss_pending_modals(&mut self) -> usize {
        super::dismiss_pending_modals(&mut self.host)
    }
}

/// Mission-lifetime host controls which are neither deterministic Engine state
/// nor timeline resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct MissionControl {
    pub(super) manual_pause: bool,
    pub(super) step_forward_repeat_at_ms: Option<u32>,
    pub(super) step_back_repeat_at_ms: Option<u32>,
    pub(super) last_shadow_color: u16,
    pub(super) last_visual_ambiance: robin_engine::engine::Ambiance,
}

impl MissionControl {
    pub(super) fn new(
        manual_pause: bool,
        last_shadow_color: u16,
        last_visual_ambiance: robin_engine::engine::Ambiance,
    ) -> Self {
        Self {
            manual_pause,
            step_forward_repeat_at_ms: None,
            step_back_repeat_at_ms: None,
            last_shadow_color,
            last_visual_ambiance,
        }
    }
}

/// Ephemeral state for one host-loop iteration.
///
/// Both mission drivers use this shell. The graphical driver additionally uses
/// its modal-dismissal queue while the frontend owns native process resources.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct MissionFrame {
    pub(super) started_at_ms: u32,
    commands: FrameCommands,
    external_actions: Vec<robin_engine::engine::ExternalAction>,
    external_actions_applied: usize,
    post_commands: FrameCommands,
    post_external_actions: Vec<robin_engine::engine::ExternalAction>,
    post_external_actions_applied: usize,
    external_facts: robin_engine::engine::ExternalFacts,
    pub(super) run_hourglass: bool,
    pub(super) simulation_body_allowed: bool,
    pub(super) run_post_initialize: bool,
    /// Recorded lockstep/history transition for a disk-replay host frame.
    pub(super) replay_timeline_transition: Option<TimelineTransition>,
    pub(super) modal_dismissals: Vec<PlayerCommand>,
    pub(super) replay_modal_dismissals: super::modal_state::ReplayModalDismissals,
    pub(super) recorder_hash: Option<u64>,
    timeline_before: Option<TimelineFrame>,
    timeline_after: Option<TimelineFrame>,
    replay_record_consumed: bool,
    recorder_state: RecorderFrameState,
}

/// Whether this host iteration owns an open replay-recorder frame.
///
/// The token lives on [`MissionFrame`], so the command write and the eventual
/// `end_frame` cannot silently drift onto different iterations.  Only
/// [`TimelineRuntime`] may change it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum RecorderFrameState {
    Inactive,
    Open,
    Finished,
}

// These adapters expose only a fresh batch, never the journal they append to.
// Legacy command producers may clear/reorder their batch without invalidating
// already-admitted input or the applied-action cursor. Admission occurs once,
// at the end of the producer's scope, including its early-return paths.
macro_rules! frame_append_batch {
    ($name:ident, $batch:ty, $append:expr) => {
        #[derive(Serialize)]
        pub(super) struct $name<'a> {
            #[serde(skip)]
            journal: &'a mut $batch,
            staged: $batch,
        }

        impl<'a> $name<'a> {
            fn new(journal: &'a mut $batch) -> Self {
                Self {
                    journal,
                    staged: Default::default(),
                }
            }
        }

        impl<'de> Deserialize<'de> for $name<'_> {
            fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
                Err(serde::de::Error::custom(
                    "frame append authority cannot be deserialized",
                ))
            }
        }

        impl std::ops::Deref for $name<'_> {
            type Target = $batch;
            fn deref(&self) -> &Self::Target {
                &self.staged
            }
        }

        impl std::ops::DerefMut for $name<'_> {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.staged
            }
        }

        impl Drop for $name<'_> {
            fn drop(&mut self) {
                ($append)(self.journal, &mut self.staged);
            }
        }
    };
}

frame_append_batch!(
    FrameCommandBatch,
    FrameCommands,
    |journal: &mut FrameCommands, staged: &mut FrameCommands| {
        journal.commands.append(&mut staged.commands);
    }
);
frame_append_batch!(
    FrameActionBatch,
    Vec<robin_engine::engine::ExternalAction>,
    |journal: &mut Vec<robin_engine::engine::ExternalAction>,
     staged: &mut Vec<robin_engine::engine::ExternalAction>| {
        journal.append(staged);
    }
);

impl MissionFrame {
    pub(super) fn commands(&self) -> &[robin_engine::player_command::PlayerInput] {
        &self.commands.commands
    }

    pub(super) fn post_commands(&self) -> &[robin_engine::player_command::PlayerInput] {
        &self.post_commands.commands
    }

    pub(super) fn external_actions(&self) -> &[robin_engine::engine::ExternalAction] {
        &self.external_actions
    }

    pub(super) fn stage_commands(&mut self) -> FrameCommandBatch<'_> {
        FrameCommandBatch::new(&mut self.commands)
    }

    pub(super) fn stage_post_commands(&mut self) -> FrameCommandBatch<'_> {
        FrameCommandBatch::new(&mut self.post_commands)
    }

    pub(super) fn stage_external_actions(&mut self) -> FrameActionBatch<'_> {
        FrameActionBatch::new(&mut self.external_actions)
    }

    pub(super) fn stage_post_external_actions(&mut self) -> FrameActionBatch<'_> {
        FrameActionBatch::new(&mut self.post_external_actions)
    }

    /// Discard live pre-tick commands superseded by replay or a loaded state.
    pub(super) fn discard_commands(&mut self) {
        self.commands.commands.clear();
    }

    /// A terminal restore starts a new recording attempt, retaining scheduling
    /// policy but none of the replaced state's admitted effects or recorder token.
    pub(super) fn reset_after_terminal_restore(&mut self, hash: u64) {
        self.external_actions.clear();
        self.external_actions_applied = 0;
        self.post_commands.commands.clear();
        self.post_external_actions.clear();
        self.post_external_actions_applied = 0;
        self.external_facts = Default::default();
        self.modal_dismissals.clear();
        self.replay_modal_dismissals = Default::default();
        self.replay_timeline_transition = None;
        self.replay_record_consumed = false;
        self.recorder_state = RecorderFrameState::Inactive;
        self.recorder_hash = Some(hash);
    }

    pub(super) fn new(started_at_ms: u32) -> Self {
        Self {
            started_at_ms,
            commands: FrameCommands::new(),
            external_actions: Vec::new(),
            external_actions_applied: 0,
            post_commands: FrameCommands::new(),
            post_external_actions: Vec::new(),
            post_external_actions_applied: 0,
            external_facts: robin_engine::engine::ExternalFacts::default(),
            run_hourglass: true,
            simulation_body_allowed: true,
            run_post_initialize: true,
            replay_timeline_transition: None,
            modal_dismissals: Vec::new(),
            replay_modal_dismissals: super::modal_state::ReplayModalDismissals::default(),
            recorder_hash: None,
            timeline_before: None,
            timeline_after: None,
            replay_record_consumed: false,
            recorder_state: RecorderFrameState::Inactive,
        }
    }

    pub(super) fn authoritative_input(&self) -> robin_engine::engine::SimulationFrameInput {
        self.hourglass_input()
            .with_external_actions(self.external_actions.clone())
            .with_post_external_actions(self.post_external_actions.clone())
            .with_post_commands(
                self.post_commands
                    .commands
                    .iter()
                    .cloned()
                    .map(robin_engine::engine::SimCommand::from)
                    .collect(),
            )
            .with_post_initialize(self.run_post_initialize)
    }

    fn bind_timeline(&mut self, before: TimelineFrame) {
        assert!(
            self.timeline_before.replace(before).is_none(),
            "mission frame bound to the timeline more than once"
        );
    }

    fn rebind_timeline_after_discontinuity(&mut self, before: TimelineFrame) {
        self.timeline_before = Some(before);
        self.timeline_after = None;
    }

    pub(super) fn commit_timeline_after(&mut self, after: TimelineFrame) {
        if let Some(expected) = self.replay_timeline_transition {
            assert_eq!(
                expected.after, after,
                "replay host transaction produced the wrong lockstep frame"
            );
        }
        assert!(
            self.timeline_after.replace(after).is_none(),
            "mission frame timeline transition committed more than once"
        );
    }

    pub(super) fn timeline_transition(&self) -> TimelineTransition {
        TimelineTransition {
            before: self
                .timeline_before
                .expect("unbound mission frame cannot become a replay record"),
            after: self
                .timeline_after
                .expect("uncommitted mission frame cannot become a replay record"),
        }
    }

    /// Authoritative input for the pre-refresh simulation phase. Late host
    /// commands remain staged until the explicit post-initialize admission.
    pub(super) fn hourglass_input(&self) -> robin_engine::engine::SimulationFrameInput {
        robin_engine::engine::SimulationFrameInput::from_player_inputs(
            self.commands.commands.clone(),
        )
        .with_external_facts(self.external_facts.clone())
        .with_external_actions(self.unapplied_external_actions().to_vec())
        .with_simulation_body_allowed(self.simulation_body_allowed)
        .with_hourglass(self.run_hourglass)
    }

    /// Replace the authoritative portion of this host frame with a recorded
    /// transaction. Presentation-only modal acknowledgements remain owned by
    /// the live/replay adapter around the transaction.
    pub(super) fn adopt_authoritative_input(
        &mut self,
        input: robin_engine::engine::SimulationFrameInput,
    ) {
        self.commands.commands = input.player_inputs();
        self.post_commands.commands = input.post_player_inputs();
        self.external_actions = input.external_actions;
        self.external_actions_applied = 0;
        self.post_external_actions = input.post_external_actions;
        self.post_external_actions_applied = 0;
        self.external_facts = input.external_facts;
        self.run_hourglass = input.run_hourglass;
        self.simulation_body_allowed = input.simulation_body_allowed;
        self.run_post_initialize = input.run_post_initialize;
    }

    pub(super) fn record_applied_external_action(
        &mut self,
        action: robin_engine::engine::ExternalAction,
    ) {
        assert_eq!(
            self.external_actions_applied,
            self.external_actions.len(),
            "new synchronous console action recorded before replayed actions were applied",
        );
        self.external_actions_applied += 1;
        self.external_actions.push(action);
    }

    fn unapplied_external_actions(&self) -> &[robin_engine::engine::ExternalAction] {
        &self.external_actions[self.external_actions_applied..]
    }

    fn record_applied_post_external_actions(
        &mut self,
        actions: Vec<robin_engine::engine::ExternalAction>,
    ) {
        if !actions.is_empty() {
            assert_eq!(
                self.post_external_actions_applied,
                self.post_external_actions.len(),
                "new synchronous RPC action recorded before replayed actions were applied",
            );
            self.post_external_actions_applied += actions.len();
            self.post_external_actions.extend(actions);
        }
    }

    fn mark_post_external_actions_applied(&mut self) {
        self.post_external_actions_applied = self.post_external_actions.len();
    }

    pub(super) fn unapplied_post_external_actions(
        &self,
    ) -> &[robin_engine::engine::ExternalAction] {
        &self.post_external_actions[self.post_external_actions_applied..]
    }

    /// Adopt one complete replay frame, splitting presentation-only modal
    /// acknowledgements out of both command phases before engine admission.
    pub(super) fn inject_replay_input(&mut self, player: &mut ReplayPlayer) {
        assert!(
            !player.is_finished(),
            "replay injection requested after the replay finished"
        );
        self.replay_record_consumed = true;
        let recorded = player.next_frame().clone();
        self.replay_timeline_transition = Some(TimelineTransition {
            before: TimelineFrame::from_wire(recorded.timeline_before),
            after: TimelineFrame::from_wire(recorded.timeline_after),
        });
        self.rebind_timeline_after_discontinuity(
            self.replay_timeline_transition
                .expect("recorded replay transition was just installed")
                .before,
        );
        self.replay_modal_dismissals.begin_replay_frame();
        for control in recorded.host_controls {
            match control {
                robin_engine::replay::ReplayHostControl::ModalDismiss { modal, result } => self
                    .replay_modal_dismissals
                    .push_back(PlayerCommand::ModalDismiss {
                        kind: modal,
                        result,
                    }),
            }
        }
        self.adopt_authoritative_input(recorded.input);
    }

    pub(super) fn timeline_advances(&self, live_default: bool) -> bool {
        let Some(transition) = self.replay_timeline_transition else {
            return live_default;
        };
        if transition.after == transition.before {
            return false;
        }
        assert_eq!(
            transition.after,
            transition.before.next(),
            "replay timeline transition must stay put or advance exactly once"
        );
        true
    }

    pub(super) fn assert_replay_timeline_before(&self, current: TimelineFrame) {
        if let Some(transition) = self.replay_timeline_transition {
            assert_eq!(
                current, transition.before,
                "replay host transaction admitted at the wrong lockstep frame"
            );
        }
    }

    fn open_recording(&mut self) {
        assert_eq!(
            self.recorder_state,
            RecorderFrameState::Inactive,
            "recorder frame began more than once"
        );
        self.recorder_state = RecorderFrameState::Open;
    }

    fn close_recording(&mut self) -> bool {
        match self.recorder_state {
            RecorderFrameState::Inactive => {
                self.recorder_state = RecorderFrameState::Finished;
                false
            }
            RecorderFrameState::Finished => panic!("recorder frame finalized more than once"),
            RecorderFrameState::Open => {
                self.recorder_state = RecorderFrameState::Finished;
                true
            }
        }
    }
}

/// Owner of the common state for one active mission.
///
/// `TimelineRuntime` remains a focused component rather than growing Engine,
/// Host, and UI responsibilities. Both drivers borrow these three disjoint
/// fields; the graphical driver keeps native process resources in a separate
/// `InteractiveFrontend` owner.
pub(super) struct MissionRuntime {
    pub(super) http: crate::http_server::SessionIngress,
    pub(super) world: MissionWorld,
    pub(super) timeline: TimelineRuntime,
    pub(super) control: MissionControl,
    /// Graphical mission-end board preparation. Headless missions deliberately
    /// own no presentation/network task here.
    pub(super) leaderboard: Option<super::leaderboard_runtime::MissionLeaderboardRuntime>,
}

impl MissionRuntime {
    pub(super) fn new(
        world: MissionWorld,
        timeline: TimelineRuntime,
        control: MissionControl,
        leaderboard: Option<super::leaderboard_runtime::MissionLeaderboardRuntime>,
    ) -> Self {
        Self {
            http: crate::http_server::SessionIngress::attach(),
            world,
            timeline,
            control,
            leaderboard,
        }
    }

    pub(super) fn into_campaign_and_simulation(
        self,
    ) -> (
        robin_engine::campaign::Campaign,
        u64,
        robin_engine::engine::SimConfig,
    ) {
        self.world.into_campaign_and_simulation()
    }

    pub(super) fn preserve_multiplayer_session_for_next_mission(&mut self) {
        self.world.preserve_multiplayer_session_for_next_mission();
    }

    /// Open one host frame at the deterministic pre-command boundary.
    ///
    /// Network ingress remains a driver concern and must run before this
    /// method. That ordering is observable for late multiplayer inputs.
    pub(super) fn begin_frame(&mut self, now_ms: u32) -> MissionFrame {
        self.timeline.reset_execution_trace();
        let mut frame = MissionFrame::new(now_ms);
        self.timeline
            .open_frame(&mut frame, &self.world.manager.engine, &self.world.assets);
        frame
    }

    /// Apply the next replay frame, separating modal acknowledgements from
    /// deterministic engine commands.
    ///
    /// Callers decide whether playback is currently allowed (for example,
    /// the graphical driver freezes playback while paused). Once admitted,
    /// both drivers use this exact command injection contract.
    pub(super) fn inject_next_replay_frame(
        &mut self,
        frame: &mut MissionFrame,
    ) -> Result<(), String> {
        self.timeline.apply_playback_timeline_events(
            &mut self.world.host,
            &mut self.world.game,
            &mut self.world.manager,
            &self.world.assets,
        )?;
        let Some(player) = self.timeline.replay_player.as_mut() else {
            return Ok(());
        };
        frame.inject_replay_input(player);
        frame.assert_replay_timeline_before(self.timeline.current_frame());
        Ok(())
    }

    /// Advance the common simulation phase while preserving each driver's
    /// explicit pause/rewind policy.
    pub(super) fn run_tick(
        &mut self,
        policy: TickPolicy,
        mission_frame: &mut MissionFrame,
    ) -> Option<GameCode> {
        if policy.skip_tick || policy.paused {
            self.timeline.trace(FrameContractStage::PausedOrRewind);
        }
        self.timeline.trace(FrameContractStage::Simulation);
        self.timeline.run_simulation(|| {
            let mut display = std::mem::take(&mut self.world.host.frontend.engine_display);
            let mission_transitioning = !self
                .world
                .game
                .operation
                .is(robin_engine::game_operation::GameCode::LevelInProgress);
            mission_frame.run_hourglass &= !policy.skip_tick
                && self.world.game.should_run_hourglass(
                    false,
                    mission_transitioning,
                    policy.paused,
                );
            let frame = mission_frame.hourglass_input();
            let result = self.world.game.run_engine_tick(
                &mut self.world.host,
                &mut display,
                self.world.assets.as_ref(),
                &mut self.world.manager.engine,
                &mut self.world.dev,
                frame,
                false,
                policy.paused,
            );
            self.world.host.frontend.engine_display = display;
            result
        })
    }

    /// Drain host RPC requests at the shared post-tick boundary.
    pub(super) fn drain_host_rpc(&mut self, frame: &mut MissionFrame) {
        let application_context = self.world.host.application_context().clone();
        drain_post_tick_rpc(
            &mut self.http,
            &mut self.timeline,
            &mut self.world.host.frontend,
            &mut self.world.host.audio,
            &mut self.world.host.effects,
            &application_context,
            &self.world.host.transport,
            &mut self.world.manager.engine,
            &self.world.assets,
            &mut self.world.dev,
            frame,
        );
        self.timeline
            .trace(FrameContractStage::HostRpcAndTimelineCommit);
    }

    /// Cross the original game's deferred post-initialization boundary.
    ///
    /// Drivers intentionally choose when to call this: headless does so
    /// before frame-zero recorder commit, graphical does so after refresh.
    pub(super) fn run_post_initialize(&mut self, frame: &mut MissionFrame) -> bool {
        let Self {
            world, timeline, ..
        } = self;
        let initialized = timeline.cross_post_initialize(|| {
            let mut display = std::mem::take(&mut world.host.frontend.engine_display);
            let initialized = crate::sim_timeline::run_post_initialize_stage_with_actions(
                &mut world.host,
                &mut display,
                &world.assets,
                &mut world.manager.engine,
                &mut world.dev,
                frame.unapplied_post_external_actions(),
                &frame.post_commands.commands,
                frame.run_post_initialize,
            );
            world.host.frontend.engine_display = display;
            initialized
        });
        frame.run_post_initialize = initialized;
        initialized
    }
}

/// Shared post-tick effect/RPC boundary. Neither driver grants access to Game,
/// EngineManager (snapshot replacement), or the aggregate Host here. Only
/// presentation/audio effect domains and shared transport access without session
/// replacement authority are admitted.
/// Recorded effects must precede new requests; taints and applied actions must
/// enter this same frame before its timeline or recording is committed.
pub(super) fn drain_post_tick_rpc(
    http: &mut crate::http_server::SessionIngress,
    timeline: &mut TimelineRuntime,
    frontend: &mut crate::host::HostFrontend,
    audio: &mut crate::host::HostAudio,
    effects: &mut crate::host::HostEffectBatches,
    application_context: &crate::host::ApplicationContext,
    transport: &crate::host::HostTransport,
    engine: &mut Engine,
    assets: &LevelAssets,
    dev: &mut DevState,
    frame: &mut MissionFrame,
) {
    let pending_actions = frame.unapplied_post_external_actions().to_vec();
    if !pending_actions.is_empty() {
        crate::sim_timeline::run_post_external_action_stage(
            frontend,
            audio,
            effects,
            application_context,
            transport.local_seat(),
            assets,
            engine,
            dev,
            &pending_actions,
        );
        frame.mark_post_external_actions_applied();
    }
    let actions = http.drain(
        engine,
        frontend,
        transport.local_seat(),
        transport.net(),
        assets,
        &mut frame.stage_post_commands(),
    );
    timeline.record_input_taints(http.take_pending_replay_taints());
    frame.record_applied_post_external_actions(actions);
}

/// Driver-owned policy for the common engine-tick phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct TickPolicy {
    pub(super) skip_tick: bool,
    pub(super) paused: bool,
}

/// Driver-owned choices at the deterministic post-tick commit boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct FrameCommitPolicy {
    /// Buffered auto-replay already owns this frame's rewind slot.
    pub(super) store_rewind_commands: bool,
}

/// Coarse stages that both mission-loop implementations pass through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum MissionPhase {
    Input,
    Simulation,
    Bookkeeping,
    Presentation,
}

/// Which host driver is advancing the deterministic mission timeline.
///
/// This is intentionally distinct from `CliArgs::headless`: the graphical
/// driver can suppress drawing for tooling, while the dedicated headless
/// driver has a different modal and replay-completion contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum FrameContract {
    Graphical,
    Headless,
}

/// Behavior-sensitive checkpoint emitted by the code that performs the work.
///
/// This is a process-side diagnostic contract, not deterministic engine state.
/// Tests inspect traces produced through these same execution seams instead of
/// comparing a second, hand-maintained description of the loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FrameContractStage {
    NetworkIngress,
    TimelineBegin,
    InputAndMenus,
    OperationAndSave,
    SecondNetworkDrain,
    PreTickCommands,
    PausedOrRewind,
    Simulation,
    HostRpcAndTimelineCommit,
    ModalDrain,
    ManualTransactionBegin,
    RecorderCommit,
    AppEffects,
    Audio,
    Presentation,
    PostInitialize,
    Pacing,
    EarlyRestart,
    Exit,
}

#[derive(Default)]
struct FrameExecutionTrace {
    stages: Vec<FrameContractStage>,
}

impl FrameExecutionTrace {
    fn begin(&mut self, first: FrameContractStage) {
        self.stages.clear();
        self.emit(first);
    }

    fn emit(&mut self, stage: FrameContractStage) {
        assert_ne!(
            self.stages.last().copied(),
            Some(stage),
            "mission frame emitted duplicate adjacent phase {stage:?}"
        );
        self.stages.push(stage);
        tracing::trace!(?stage, "mission frame phase");
    }
}

/// The decision produced at the end of one host frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum FrameOutcome {
    Continue { sleep_ms: u32 },
    Exit(GameCode),
}

/// Inputs needed to turn elapsed host time into the next pacing delay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct FramePacing {
    pub(super) fast_forward_requested: bool,
    pub(super) headless: bool,
    pub(super) engine_fast_forward: bool,
    pub(super) slow_motion: bool,
    /// Absolute process-uptime deadline supplied by the host, for a
    /// multiplayer client. `None` keeps the local cadence.
    pub(super) host_deadline_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct FrameClock {
    started_at_ms: u32,
}

impl FrameClock {
    fn new() -> Self {
        Self { started_at_ms: 0 }
    }

    fn begin(&mut self, now_ms: u32) {
        self.started_at_ms = now_ms;
    }

    fn plan(&self, now_ms: u32, pacing: FramePacing) -> u32 {
        let elapsed_ms = now_ms.saturating_sub(self.started_at_ms);
        let target_ms = if pacing.fast_forward_requested || pacing.headless {
            0
        } else if pacing.engine_fast_forward {
            1
        } else if pacing.slow_motion {
            // Original-game behavior:
            // The original game loop waits for `40 * 10` while the messenger
            // reports slow motion, and 40 ms otherwise.
            robin_engine::engine::FRAME_TIME_MS * 10
        } else {
            robin_engine::engine::FRAME_TIME_MS
        };
        let local_sleep_ms = target_ms.saturating_sub(elapsed_ms);
        pacing
            .host_deadline_ms
            .map_or(local_sleep_ms, |deadline_ms| {
                (deadline_ms - i64::from(now_ms)).max(0) as u32
            })
    }
}

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum RecordingValidity {
    Linear,
    Invalid { reason: String },
}

/// The exact pre-command state captured when bootstrap admits its Restart
/// save. Keep it while persistence completes, rather than hashing a later state.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(super) struct BootstrapSaveBoundary {
    identity: ReplaySaveIdentity,
    marker: robin_engine::replay::ReplaySaveMarker,
}

impl BootstrapSaveBoundary {
    pub(super) fn capture(
        engine: &Engine,
        host: &Host,
        game: &Game,
        session_identity: Option<ReplaySaveIdentity>,
    ) -> Self {
        Self {
            identity: session_identity.unwrap_or_else(|| {
                GameRuntimeSnapshot::identity_of_live(engine, host, game)
                    .unwrap_or_else(|error| panic!("bootstrap save identity failed: {error:#}"))
            }),
            marker: robin_engine::replay::ReplaySaveMarker {
                state_hash: robin_engine::replay::state_hash(engine),
                timeline_frame: 0,
            },
        }
    }
}

/// Mission-lifetime replay, rollback, network, and frame-clock state.
///
/// This is deliberately not serializable: recorder writers, rollback
/// workers, and live network diagnostics are process resources, not game
/// state. Persisting them would create a fake/default runtime on restore.
pub(super) struct TimelineRuntime {
    /// Actual save/replay restoration awaiting process-owned lifecycle sync.
    state_restored: bool,
    /// Single authority for history, network, and replay frame identity.
    current_frame: TimelineFrame,
    /// Dense host-record position, separate from the lockstep cursor.
    replay_ordinal: ReplayFrameOrdinal,
    network: NetworkReconciliation,
    contract: FrameContract,
    phase: MissionPhase,
    clock: FrameClock,
    execution_trace: FrameExecutionTrace,
    pending_external_facts: robin_engine::engine::ExternalFacts,

    pub(super) replay_recorder: Option<ReplayRecorder>,
    sealed_replay_header: Option<robin_engine::replay::ReplayHeader>,
    recording_validity: RecordingValidity,
    bootstrap_save: Option<(ReplaySaveIdentity, robin_engine::replay::ReplaySaveMarker)>,
    pub(super) replay_player: Option<ReplayPlayer>,
    history: ReconstructionHistory,
    pub(super) start_paused: bool,
    pub(super) replay_finished_logged: bool,
    /// Recording side: complete save-payload identity → recorder frame, for every
    /// in-mission save written this session at a clean pre-command
    /// boundary.  A later in-mission load whose decoded payload identity is in
    /// this map is the same save coming back, and is recorded as a linear
    /// load-back to that frame instead of a timeline discontinuity.
    pub(super) recorded_save_frames_by_identity:
        BTreeMap<ReplaySaveIdentity, (ReplayFrameOrdinal, TimelineFrame)>,
    /// Playback side: complete game-save snapshots pinned at save-marker
    /// frames. A load-back applies the same engine/host/game restoration path
    /// as a live save load.
    pub(super) playback_pinned_saves: BTreeMap<u32, GameRuntimeSnapshot>,

    mp_admission: MultiplayerAdmission,
    pub(super) mp_host_frame_schedule: Option<(u32, u32)>,
    pub(super) last_mp_rollback: Option<MultiplayerRollbackTelemetry>,
    pub(super) last_mp_clock_ahead_log_ms: u32,
    pub(super) last_mp_sleep_correction_log_ms: u32,
    pub(super) last_mp_state_hash_frame: Option<u32>,
    pub(super) pending_mp_state_hash: Option<(u32, u64)>,
}

/// Network admission state for the deterministic mission timeline.
///
/// The transport owns handshakes and wire delivery; this state machine owns
/// the point at which a loaded mission may begin advancing simulation. Keeping
/// it in `TimelineRuntime` also keeps snapshot adoption ahead of replay and
/// rollback frame capture for both graphical and true-headless drivers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum MultiplayerAdmission {
    NotRequired,
    HostWaitingForBegin,
    HostWaitingForResyncBegin { snapshot_frame: u32 },
    PeerWaitingForSnapshot,
    PeerWaitingForBegin { snapshot_frame: u32 },
    WaitingForStart { frame: u32, start_epoch_ms: u64 },
    Running,
}

impl TimelineRuntime {
    pub(super) fn new(
        replay: ReplayAndRollback,
        contract: FrameContract,
        wait_for_multiplayer_start: bool,
        local_is_host: bool,
    ) -> Self {
        Self {
            state_restored: false,
            current_frame: TimelineFrame::ZERO,
            replay_ordinal: ReplayFrameOrdinal::ZERO,
            network: NetworkReconciliation::default(),
            contract,
            phase: MissionPhase::Presentation,
            clock: FrameClock::new(),
            execution_trace: FrameExecutionTrace::default(),
            pending_external_facts: robin_engine::engine::ExternalFacts::default(),
            replay_recorder: replay.recorder,
            sealed_replay_header: None,
            recording_validity: RecordingValidity::Linear,
            bootstrap_save: None,
            replay_player: replay.player,
            history: ReconstructionHistory::new(replay.rewind_buffer, replay.rollback_checker),
            start_paused: replay.start_paused,
            replay_finished_logged: false,
            recorded_save_frames_by_identity: BTreeMap::new(),
            playback_pinned_saves: BTreeMap::new(),
            mp_admission: match (wait_for_multiplayer_start, local_is_host) {
                (false, _) => MultiplayerAdmission::NotRequired,
                (true, true) => MultiplayerAdmission::HostWaitingForBegin,
                (true, false) => MultiplayerAdmission::PeerWaitingForSnapshot,
            },
            mp_host_frame_schedule: None,
            last_mp_rollback: None,
            last_mp_clock_ahead_log_ms: 0,
            last_mp_sleep_correction_log_ms: 0,
            last_mp_state_hash_frame: None,
            pending_mp_state_hash: None,
        }
    }

    pub(super) fn initially_paused(&self) -> bool {
        self.start_paused
    }

    pub(super) fn multiplayer_admission(&self) -> MultiplayerAdmission {
        self.mp_admission
    }

    pub(super) fn retained_history(&self) -> &RewindBuffer {
        &self.history.buffer
    }

    pub(super) fn pending_input_frame_count(&self) -> usize {
        self.network.pending_frame_count()
    }

    pub(super) fn reset_rollback_checker(&mut self) {
        self.history.reset_checker();
    }

    pub(super) fn begin_rewind_session(&mut self) {
        self.history.buffer.begin_session();
    }

    pub(super) fn end_rewind_session(&mut self) {
        self.history.buffer.end_session();
    }

    pub(super) fn begin_history_frame(
        &mut self,
        frame: u32,
        engine: &Engine,
        assets: &LevelAssets,
    ) {
        assert_eq!(
            frame,
            self.frame_number(),
            "history capture must use the authoritative timeline frame"
        );
        self.history.buffer.begin_frame(frame, engine, assets);
    }

    pub(super) fn commit_history_frame(
        &mut self,
        input: robin_engine::engine::SimulationFrameInput,
        host: &mut Host,
        engine: &Engine,
    ) {
        self.history.commit(input, host, engine);
    }

    pub(super) fn branch_history_at(&mut self, frame: u32) {
        self.history.buffer.truncate_future(frame);
        self.history.reset_checker();
        self.network.invalidate_after(frame);
    }

    pub(super) fn checkpoint_history(&mut self, engine: &Engine) {
        self.history
            .buffer
            .checkpoint_recent(self.frame_number(), engine);
    }

    pub(super) fn drain_network_inputs(
        &mut self,
        host: &mut Host,
        manager: &mut EngineManager,
        assets: &mut Arc<LevelAssets>,
    ) -> super::multiplayer::NetDrainResult {
        let result = super::multiplayer::drain_net_inputs(
            host,
            manager,
            self.frame_number(),
            &mut self.network,
            assets,
            &mut self.history.buffer,
        );
        if result.rewrote_sim_state {
            self.history.reset_checker();
        }
        result
    }

    #[cfg(test)]
    pub(super) fn append_history_fixture(
        &mut self,
        input: robin_engine::engine::SimulationFrameInput,
    ) {
        self.history.buffer.end_frame_input(input);
    }

    #[cfg(test)]
    pub(super) fn clear_recent_history_fixture(&mut self) {
        self.history.buffer.clear_recent_checkpoints();
    }

    #[cfg(test)]
    pub(super) fn reconstruct_history_fixture(
        &mut self,
        assets: &LevelAssets,
        frame: u32,
    ) -> Option<Engine> {
        self.history.buffer.rewind_to(assets, frame)
    }

    pub(super) const fn current_frame(&self) -> TimelineFrame {
        self.current_frame
    }

    pub(super) const fn frame_number(&self) -> u32 {
        self.current_frame.number()
    }

    /// Advance after one authoritative transaction has committed. This says
    /// nothing about whether that transaction executed an engine hourglass.
    pub(super) fn advance_frame(&mut self) -> TimelineFrame {
        self.current_frame = self.current_frame.next();
        self.current_frame
    }

    /// Reposition the authoritative cursor after snapshot adoption, load-back,
    /// or debugger rewind. Callers must have already replaced/restored Engine
    /// state at precisely this pre-transaction boundary.
    pub(super) fn adopt_frame(&mut self, frame: TimelineFrame) {
        self.current_frame = frame;
        self.network.discard_inputs_before(frame);
    }

    /// Restore the engine and every cursor/history consumer to a retained
    /// pre-transaction boundary. Session caching is controlled by the caller
    /// so hold-to-rewind can reuse reconstruction work across host frames.
    pub(super) fn restore_retained_frame(
        &mut self,
        manager: &mut EngineManager,
        assets: &LevelAssets,
        target: TimelineFrame,
    ) -> bool {
        let Some(engine) = self.history.buffer.rewind_to(assets, target.number()) else {
            return false;
        };
        let mapped_ordinal = if let Some(player) = self.replay_player.as_mut() {
            let Ok(ordinal) = player.seek_timeline_frame(target) else {
                return false;
            };
            Some(ordinal)
        } else {
            None
        };
        manager.engine = engine;
        self.adopt_frame(target);
        if let Some(ordinal) = mapped_ordinal {
            self.replay_ordinal = ordinal;
        }
        self.history.finish_restore(target.number());
        true
    }

    /// Reset every in-memory reconstruction of the current deterministic
    /// future after a whole-state discontinuity such as save-load adoption.
    ///
    /// Original-game loading is followed by
    /// post-load resynchronization; the original game has no command journal. The Rust
    /// equivalent must additionally invalidate all journals and checkpoints
    /// whose future was derived from the replaced state.
    fn reset_reconstruction_history(
        &mut self,
        target: TimelineFrame,
        engine: &Engine,
        assets: &LevelAssets,
    ) {
        self.adopt_frame(target);
        self.history.adopt_snapshot(target.number(), engine, assets);
    }

    /// Called by the host operation that publishes a replacement snapshot and
    /// resets the transport ready barrier. A spontaneous BeginSim is still an
    /// error; only this explicitly adopted frame can release the new gate.
    pub(super) fn begin_synchronized_step_resync(&mut self) {
        assert!(
            matches!(
                self.mp_admission,
                MultiplayerAdmission::Running
                    | MultiplayerAdmission::HostWaitingForResyncBegin { .. }
            ),
            "host resynchronization requested outside a running/adopting session"
        );
        self.mp_admission = MultiplayerAdmission::HostWaitingForResyncBegin {
            snapshot_frame: self.frame_number(),
        };
        self.network.abandon_prediction();
        self.last_mp_state_hash_frame = None;
        self.pending_mp_state_hash = None;
    }

    pub(super) fn remember_local_mp_hash(&mut self, frame: u32, hash: u64) {
        self.network.remember_local_hash(frame, hash);
    }

    pub(super) fn has_local_mp_hash(&self, frame: u32) -> bool {
        self.network.has_local_hash(frame)
    }

    pub(super) fn invalidate_local_mp_hashes_after(&mut self, frame: u32) {
        self.network.invalidate_after(frame);
    }

    pub(super) fn clear_local_mp_hashes(&mut self) {
        self.network.clear_local_hashes();
    }

    pub(super) fn take_due_mp_hash_comparisons(&mut self) -> Vec<(u32, u64, Option<u64>)> {
        self.network.take_due_comparisons(self.current_frame)
    }

    pub(super) fn apply_multiplayer_admission_events(
        &mut self,
        events: &[MultiplayerAdmissionEvent],
    ) {
        for event in events {
            if matches!(event, MultiplayerAdmissionEvent::HostResynchronizing { .. }) {
                self.network.clear_hashes();
                self.last_mp_state_hash_frame = None;
                self.pending_mp_state_hash = None;
            }
            if matches!(
                event,
                MultiplayerAdmissionEvent::Disconnected
                    | MultiplayerAdmissionEvent::InitialSnapshotAdopted { .. }
            ) {
                self.network.clear_local_hashes();
            }
            self.mp_admission = match (self.mp_admission, *event) {
                (
                    MultiplayerAdmission::Running | MultiplayerAdmission::WaitingForStart { .. },
                    MultiplayerAdmissionEvent::HostResynchronizing { frame },
                ) => MultiplayerAdmission::HostWaitingForResyncBegin {
                    snapshot_frame: frame,
                },
                (
                    MultiplayerAdmission::HostWaitingForResyncBegin { snapshot_frame },
                    MultiplayerAdmissionEvent::BeginSim {
                        frame,
                        start_epoch_ms,
                    },
                ) if snapshot_frame == frame => MultiplayerAdmission::WaitingForStart {
                    frame,
                    start_epoch_ms,
                },
                (_, MultiplayerAdmissionEvent::Disconnected) => {
                    MultiplayerAdmission::PeerWaitingForSnapshot
                }
                (
                    MultiplayerAdmission::PeerWaitingForSnapshot,
                    MultiplayerAdmissionEvent::InitialSnapshotAdopted { frame },
                ) => MultiplayerAdmission::PeerWaitingForBegin {
                    snapshot_frame: frame,
                },
                (
                    MultiplayerAdmission::PeerWaitingForBegin { snapshot_frame },
                    MultiplayerAdmissionEvent::BeginSim {
                        frame,
                        start_epoch_ms,
                    },
                ) if frame == snapshot_frame => MultiplayerAdmission::WaitingForStart {
                    frame,
                    start_epoch_ms,
                },
                (
                    MultiplayerAdmission::HostWaitingForBegin,
                    MultiplayerAdmissionEvent::BeginSim {
                        frame,
                        start_epoch_ms,
                    },
                ) => MultiplayerAdmission::WaitingForStart {
                    frame,
                    start_epoch_ms,
                },
                (state, event) => panic!(
                    "invalid multiplayer admission ordering: state {state:?}, event {event:?}"
                ),
            };
        }
    }

    /// Advance the wall-clock release gate and report whether simulation must
    /// remain held for multiplayer admission.
    pub(super) fn multiplayer_admission_paused(&mut self, now_epoch_ms: u64) -> bool {
        if let MultiplayerAdmission::WaitingForStart {
            frame,
            start_epoch_ms,
        } = self.mp_admission
            && now_epoch_ms >= start_epoch_ms
        {
            self.mp_admission = MultiplayerAdmission::Running;
            tracing::info!(frame, "multiplayer: synchronized start gate opened");
        }
        !matches!(
            self.mp_admission,
            MultiplayerAdmission::NotRequired | MultiplayerAdmission::Running
        )
    }

    pub(super) fn frame_contract(&self) -> FrameContract {
        self.contract
    }

    pub(super) fn begin_execution_trace(&mut self, stage: FrameContractStage) {
        self.execution_trace.begin(stage);
    }

    fn reset_execution_trace(&mut self) {
        self.execution_trace.stages.clear();
    }

    pub(super) fn trace(&mut self, stage: FrameContractStage) {
        self.execution_trace.emit(stage);
    }

    /// Execute the one-shot host dispatch, then record that the boundary was
    /// crossed. Both mission drivers use this seam while choosing their own
    /// presentation/recorder ordering around it.
    pub(super) fn cross_post_initialize<T>(&mut self, dispatch: impl FnOnce() -> T) -> T {
        let result = dispatch();
        self.trace(FrameContractStage::PostInitialize);
        result
    }

    #[cfg(test)]
    fn execution_trace(&self) -> &[FrameContractStage] {
        &self.execution_trace.stages
    }

    /// Capture timeline state into an already-created driver frame.
    ///
    /// Graphical networking can append current-frame inputs before this
    /// boundary; true headless creates an empty frame and opens it directly.
    pub(super) fn open_frame(
        &mut self,
        frame: &mut MissionFrame,
        engine: &Engine,
        assets: &LevelAssets,
    ) {
        frame.bind_timeline(self.current_frame());
        assert!(
            frame.external_facts.is_empty(),
            "timeline facts must be attached before replay/rewind input adoption",
        );
        frame.external_facts = std::mem::take(&mut self.pending_external_facts);
        frame.recorder_hash = self.begin_frame(frame.started_at_ms, engine, assets);
        self.trace(FrameContractStage::TimelineBegin);
    }

    /// A second ingress drain may reconstruct the open frame after late input
    /// invalidates both pending snapshot tiers. Re-open only that capture,
    /// preserving this host iteration's commands, facts, clock and ordinal.
    pub(super) fn reopen_after_pre_tick_network_rollback(
        &mut self,
        frame: &mut MissionFrame,
        engine: &Engine,
        assets: &LevelAssets,
    ) {
        assert_eq!(
            self.phase,
            MissionPhase::Input,
            "network rollback must precede simulation"
        );
        assert_eq!(
            frame.timeline_before,
            Some(self.current_frame()),
            "late-input rollback must reconstruct the already-open frame"
        );
        assert!(
            frame.timeline_after.is_none(),
            "cannot reopen an already committed frame"
        );
        self.history
            .buffer
            .begin_frame(self.frame_number(), engine, assets);
        // Recording samples the final pre-command state, not the speculative
        // state captured before the late input arrived. Do not call open_frame:
        // it would bind twice and consume external facts a second time.
        frame.recorder_hash = self.replay_recorder.as_ref().and_then(|_| {
            self.replay_ordinal
                .number()
                .is_multiple_of(25)
                .then(|| robin_engine::replay::state_hash(engine))
        });
    }

    pub(super) fn queue_sound_boundary(&mut self, boundary: robin_engine::engine::SoundBoundary) {
        assert!(
            self.pending_external_facts.sound_boundary.is_none(),
            "a host frame cannot cross more than one sound boundary",
        );
        self.pending_external_facts.sound_boundary = Some(boundary);
    }

    /// Start the input phase and capture the shared pre-command snapshots.
    ///
    /// `begin_frame` intentionally permits replacing any previous phase:
    /// native event handlers can restart the outer loop before simulation,
    /// abandoning that host frame exactly as the old loop did.
    pub(super) fn begin_frame(
        &mut self,
        now_ms: u32,
        engine: &Engine,
        assets: &LevelAssets,
    ) -> Option<u64> {
        self.phase = MissionPhase::Input;
        self.clock.begin(now_ms);
        self.pending_mp_state_hash = None;
        let current_frame = self.frame_number();
        self.history
            .buffer
            .begin_frame(current_frame, engine, assets);

        let recorder_hash = self.replay_recorder.as_ref().and_then(|_| {
            self.replay_ordinal
                .number()
                .is_multiple_of(25)
                .then(|| robin_engine::replay::state_hash(engine))
        });
        if let Some(player) = self.replay_player.as_ref()
            && !player.is_finished()
        {
            let frame = player.current_frame();
            assert_eq!(
                frame,
                self.replay_ordinal.number(),
                "replay player cursor diverged from timeline-owned host ordinal"
            );
            let is_terminal_frame = frame + 1 >= player.total_frames();
            // A pristine Restart records its first hash after restoration,
            // unlike ordinary pre-load frame hashes. Check it at the boundary.
            let pristine_restart = frame == 0
                && player
                    .load_back_for_frame(0)
                    .is_some_and(|load| load.to_frame == 0);
            if !is_terminal_frame
                && !pristine_restart
                && let Some(expected) = player.hash_for_frame(frame)
            {
                let actual = robin_engine::replay::state_hash(engine);
                if actual != expected {
                    tracing::error!(
                        "Replay desync at frame {frame}: expected {expected:016x}, got {actual:016x}"
                    );
                } else {
                    tracing::debug!("Replay hash OK @ frame {frame}: {actual:016x}");
                }
            }
        }
        recorder_hash
    }

    pub(super) fn begin_simulation(&mut self) {
        self.transition(MissionPhase::Input, MissionPhase::Simulation);
    }

    /// Execute exactly one simulation-stage action between the shared phase
    /// transitions. Driver-specific pause and rewind decisions belong in the
    /// closure, while the timeline owns the ordering invariant.
    pub(super) fn run_simulation<T>(&mut self, action: impl FnOnce() -> T) -> T {
        self.begin_simulation();
        let result = action();
        self.begin_bookkeeping();
        result
    }

    /// Commit rollback and rewind history for a frame already admitted by its
    /// driver.
    ///
    /// Pause/rewind admission and recorder ordering remain driver concerns.
    /// Once admitted, rollback verification and rewind history are one owner
    /// boundary. Graphical cursor admission occurs before manual debugger
    /// steps; headless admission occurs here immediately afterward.
    pub(super) fn commit_simulation_history(
        &mut self,
        host: &mut Host,
        manager: &mut EngineManager,
        frame: &MissionFrame,
        policy: FrameCommitPolicy,
    ) {
        assert!(
            matches!(
                self.phase,
                MissionPhase::Bookkeeping | MissionPhase::Presentation
            ),
            "simulation commit requested outside bookkeeping/presentation phase"
        );
        if policy.store_rewind_commands {
            self.history
                .commit(frame.authoritative_input(), host, &manager.engine);
        }
    }

    pub(super) fn begin_bookkeeping(&mut self) {
        self.transition(MissionPhase::Simulation, MissionPhase::Bookkeeping);
    }

    pub(super) fn begin_presentation(&mut self) {
        self.transition(MissionPhase::Bookkeeping, MissionPhase::Presentation);
    }

    pub(super) fn plan_frame_outcome(
        &self,
        now_ms: u32,
        pacing: FramePacing,
        exit: Option<GameCode>,
    ) -> FrameOutcome {
        assert_eq!(
            self.phase,
            MissionPhase::Presentation,
            "frame outcome requested before presentation phase"
        );
        if let Some(code) = exit {
            FrameOutcome::Exit(code)
        } else {
            FrameOutcome::Continue {
                sleep_ms: self.clock.plan(now_ms, pacing),
            }
        }
    }

    /// React to a completed in-mission save or load on the live side,
    /// keeping the recording one linear timeline.
    ///
    /// A save at a clean pre-command boundary becomes a save-marker record
    /// (state hash + frame) so a later load of it can be expressed as a
    /// load-back.  A load resets rewind history (the buffered timeline no
    /// longer describes the engine's future), drops commands already
    /// dispatched this frame (their effects were overwritten wholesale),
    /// and — when the decoded payload identity matches a save made this session —
    /// records a load-back to that save's frame.
    pub(super) fn note_save_load_event(
        &mut self,
        event: crate::main_entry::SaveLoadEvent,
        frame: &mut MissionFrame,
        engine: &Engine,
        assets: &LevelAssets,
    ) {
        match event {
            crate::main_entry::SaveLoadEvent::SaveWritten { identity } => {
                let replay_ordinal = self.replay_ordinal;
                let Some(recorder) = self.replay_recorder.as_mut() else {
                    return;
                };
                if !frame.commands.commands.is_empty() || !frame.modal_dismissals.is_empty() {
                    // The captured state includes commands applied earlier
                    // this frame, so it is not the pre-command boundary state
                    // playback pins at this frame.  Loading this save later
                    // will be treated as a foreign save.
                    tracing::warn!(
                        replay_ordinal = replay_ordinal.number(),
                        commands = frame.commands.commands.len(),
                        "replay: save captured mid-frame after commands; \
                         not recording a timeline save marker"
                    );
                    return;
                }
                let hash = robin_engine::replay::state_hash(engine);
                let marker_timeline = self.current_frame;
                recorder.write_save_marker(
                    replay_ordinal.number(),
                    robin_engine::replay::ReplaySaveMarker {
                        state_hash: hash,
                        timeline_frame: marker_timeline.number(),
                    },
                );
                self.recorded_save_frames_by_identity
                    .insert(identity, (replay_ordinal, marker_timeline));
                tracing::info!(
                    replay_ordinal = replay_ordinal.number(),
                    timeline_frame = marker_timeline.number(),
                    hash = format!("{hash:016x}"),
                    "replay: save marker recorded"
                );
            }
            crate::main_entry::SaveLoadEvent::LoadApplied {
                identity,
                is_continue,
            } => {
                self.state_restored = true;
                let reopened = self.reopen_recording_after_terminal_restore(identity);
                if reopened {
                    // The load replaced every effect admitted before it. Keep
                    // this host frame's scheduling flags, but give the new
                    // attempt a clean recording transaction.
                    frame.external_actions.clear();
                    frame.external_actions_applied = 0;
                    frame.post_commands.commands.clear();
                    frame.post_external_actions.clear();
                    frame.post_external_actions_applied = 0;
                    frame.external_facts = Default::default();
                    frame.modal_dismissals.clear();
                    frame.replay_modal_dismissals = Default::default();
                    frame.replay_timeline_transition = None;
                    frame.replay_record_consumed = false;
                    frame.recorder_state = RecorderFrameState::Inactive;
                    frame.recorder_hash = Some(robin_engine::replay::state_hash(engine));
                }
                let replay_ordinal = self.replay_ordinal;
                if let Some(recorder) = self.replay_recorder.as_mut() {
                    recorder.record_input_taint(
                        robin_engine::replay_rankability::InputTaintKind::StateLoad,
                        replay_ordinal.number(),
                    );
                }
                // The engine state jumped; buffered rewind history no longer
                // describes this timeline's future.
                let recorded_save = self
                    .recorded_save_frames_by_identity
                    .get(&identity)
                    .copied();
                let target = recorded_save.map_or(self.current_frame, |(_, timeline)| timeline);
                self.reset_reconstruction_history(target, engine, assets);
                frame.rebind_timeline_after_discontinuity(target);
                if !frame.commands.commands.is_empty() {
                    tracing::debug!(
                        dropped = frame.commands.commands.len(),
                        "replay: dropping commands dispatched before the load; \
                         their effects were overwritten by the loaded state"
                    );
                    frame.commands.commands.clear();
                }
                let Some(recorder) = self.replay_recorder.as_mut() else {
                    frame.recorder_state = RecorderFrameState::Inactive;
                    frame.recorder_hash = None;
                    return;
                };
                if let Some((to_ordinal, _)) = recorded_save {
                    recorder.write_load_back(
                        replay_ordinal.number(),
                        to_ordinal.number(),
                        is_continue,
                    );
                    tracing::info!(
                        replay_ordinal = replay_ordinal.number(),
                        to_ordinal = to_ordinal.number(),
                        "replay: load recorded as linear load-back"
                    );
                } else {
                    // TODO(replay): a load of a save from another session
                    // cannot be expressed as a load-back into this recording.
                    // Supporting it needs an embedded initial save, not a
                    // continuation that only appears to be replayable.
                    self.invalidate_recording(
                        "replay unavailable after loading a save not recorded in this session",
                    );
                    frame.recorder_state = RecorderFrameState::Inactive;
                    frame.recorder_hash = None;
                }
            }
        }
    }

    pub(super) fn note_state_restored(&mut self) {
        self.state_restored = true;
    }

    pub(super) fn take_state_restored(&mut self) -> bool {
        std::mem::take(&mut self.state_restored)
    }

    fn reopen_recording_after_terminal_restore(&mut self, identity: ReplaySaveIdentity) -> bool {
        let Some(header) = self.sealed_replay_header.as_ref() else {
            return false;
        };
        let Some((_, marker)) = self.bootstrap_save.filter(|(saved, _)| *saved == identity) else {
            // TODO(replay): arbitrary post-terminal saves need an embedded
            // initial snapshot. The original campaign header cannot reconstruct
            // them. Never export the previous terminal attempt as this run.
            let reason = "replay unavailable after post-terminal load of a non-bootstrap save";
            self.invalidate_recording(reason);
            return false;
        };
        match super::replay_init::restart_recording(header.clone()) {
            Ok(mut recorder) => {
                // Playback constructs the original pristine state, pins it, and
                // restores it before the first real input. This reproduces
                // persisted-state projection and normal post-load fixups without
                // inserting an invented simulation frame.
                recorder.write_save_marker(0, marker);
                self.replay_recorder = Some(recorder);
                self.recording_validity = RecordingValidity::Linear;
                self.sealed_replay_header = None;
                self.replay_ordinal = ReplayFrameOrdinal::ZERO;
                self.recorded_save_frames_by_identity.clear();
                self.recorded_save_frames_by_identity
                    .insert(identity, (ReplayFrameOrdinal::ZERO, TimelineFrame::ZERO));
                tracing::info!("Recording restarted mission from its bootstrap save boundary");
                true
            }
            Err(error) => {
                let reason =
                    format!("replay unavailable: could not start restarted recording: {error}");
                self.invalidate_recording(reason);
                false
            }
        }
    }

    fn invalidate_recording(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        // Preserve the original header so a later successful bootstrap restore
        // can start a new linear attempt. Never continue writing this attempt.
        if let Some(recorder) = self.replay_recorder.take() {
            self.sealed_replay_header = Some(recorder.into_recording_header());
        }
        self.recorded_save_frames_by_identity.clear();
        crate::replay_service::process().invalidate(reason.clone());
        tracing::warn!("{reason}");
        self.recording_validity = RecordingValidity::Invalid { reason };
    }

    /// Register a successfully completed bootstrap Restart save at frame zero.
    ///
    /// That save is captured during mission setup, immediately before
    /// runtime construction, so its payload is exactly the frame-0
    /// boundary state the replay header reconstructs.  Registering it here
    /// lets a later script-triggered restart record as a load-back to
    /// frame 0 instead of a timeline discontinuity.
    pub(super) fn register_bootstrap_save(&mut self, completed: Option<BootstrapSaveBoundary>) {
        let Some(BootstrapSaveBoundary { identity, marker }) = completed else {
            return;
        };
        let Some(recorder) = self.replay_recorder.as_mut() else {
            return;
        };
        assert_eq!(
            self.replay_ordinal,
            ReplayFrameOrdinal::ZERO,
            "bootstrap save must be registered before the first recorded frame"
        );
        recorder.write_save_marker(0, marker);
        self.bootstrap_save = Some((identity, marker));
        self.recorded_save_frames_by_identity
            .insert(identity, (ReplayFrameOrdinal::ZERO, TimelineFrame::ZERO));
    }

    /// Apply recorded save/load timeline events at the current playback
    /// frame's pre-command boundary, before that frame's commands are
    /// injected.
    ///
    /// Save markers pin a complete in-memory save payload (verified against
    /// the recorded engine-state hash); load-back records apply that payload
    /// through the normal engine/host/game restoration path.
    pub(super) fn apply_playback_timeline_events(
        &mut self,
        host: &mut Host,
        game: &mut Game,
        manager: &mut EngineManager,
        assets: &LevelAssets,
    ) -> Result<(), String> {
        let Some(player) = self.replay_player.as_ref() else {
            return Ok(());
        };
        if player.is_finished() {
            return Ok(());
        }
        if player.current_frame() != self.replay_ordinal.number() {
            return Err(format!(
                "replay player ordinal {} diverged from timeline runtime ordinal {}",
                player.current_frame(),
                self.replay_ordinal.number()
            ));
        }
        let adopted_timeline = apply_replay_timeline_events_at_boundary(
            player,
            self.current_frame,
            &mut self.playback_pinned_saves,
            &mut self.history.buffer,
            host,
            game,
            manager,
            assets,
        )?;
        if let Some(target) = adopted_timeline {
            self.state_restored = true;
            // The boundary helper resets rewind itself because debugger-step
            // callers use it directly. TimelineRuntime additionally rebases
            // every reconstruction consumer on the adopted pre-tick state.
            self.reset_reconstruction_history(target, &manager.engine, assets);
        }
        Ok(())
    }

    /// Consume a replay record outside the normal outer-frame lifecycle.
    /// Debugger/manual stepping owns its own transaction, so it advances the
    /// dense replay ordinal immediately instead of deferring it to
    /// [`Self::finish_recording`].
    pub(super) fn consume_replay_frame_for_step(&mut self) -> Result<ReplayStepAdmission, String> {
        let Some(player) = self.replay_player.as_mut() else {
            return Ok(ReplayStepAdmission::NoActiveReplay);
        };
        let ordinal = player.current_frame();
        if ordinal != self.replay_ordinal.number() {
            return Err(format!(
                "replay player ordinal {} diverged from timeline runtime ordinal {}",
                ordinal,
                self.replay_ordinal.number()
            ));
        }
        if player.is_finished() {
            return Ok(ReplayStepAdmission::Finished {
                ordinal,
                total_frames: player.total_frames(),
            });
        }
        let recorded = player.next_frame().clone();
        if recorded.timeline_before != self.current_frame.number() {
            return Err(format!(
                "replay ordinal {} starts at timeline {}, current timeline is {}",
                self.replay_ordinal.number(),
                recorded.timeline_before,
                self.current_frame.number()
            ));
        }
        self.replay_ordinal.advance();
        Ok(ReplayStepAdmission::Recorded(recorded))
    }

    /// Manual ticks have their own recorder transaction without restarting the
    /// enclosing driver's clock or phase. The caller must first finalize the
    /// ordinary host frame, including its post-refresh contributions.
    pub(super) fn open_manual_frame(
        &mut self,
        engine: &Engine,
        fresh_live_input: bool,
    ) -> MissionFrame {
        self.trace(FrameContractStage::ManualTransactionBegin);
        let mut frame = MissionFrame::new(0);
        frame.bind_timeline(self.current_frame());
        if fresh_live_input {
            frame.external_facts = std::mem::take(&mut self.pending_external_facts);
        }
        frame.recorder_hash = self.replay_recorder.as_ref().and_then(|_| {
            self.replay_ordinal
                .number()
                .is_multiple_of(25)
                .then(|| robin_engine::replay::state_hash(engine))
        });
        frame
    }

    pub(super) fn begin_recording(&mut self, frame: &mut MissionFrame, enabled: bool) {
        assert!(
            matches!(self.recording_validity, RecordingValidity::Linear)
                || self.replay_recorder.is_none(),
            "an invalidated recording cannot own a live recorder"
        );
        let Some(_) = self.replay_recorder.as_ref().filter(|_| enabled) else {
            return;
        };
        frame.open_recording();
    }

    /// A step request may dismiss an already-open modal before its first
    /// tick (including a zero-tick request). Keep that host-only boundary in
    /// the dense stream without inventing an engine tick or consuming facts
    /// intended for the next simulation transaction.
    pub(super) fn record_manual_host_controls(
        &mut self,
        engine: &Engine,
        controls: Vec<PlayerCommand>,
    ) {
        if controls.is_empty()
            || self.replay_recorder.is_none()
            || self.replay_player.is_some()
            || self.frame_number() < self.history.buffer.next_record_frame()
        {
            return;
        }
        self.trace(FrameContractStage::ManualTransactionBegin);
        let mut frame = MissionFrame::new(0);
        frame.bind_timeline(self.current_frame());
        frame.run_hourglass = false;
        frame.simulation_body_allowed = false;
        frame.run_post_initialize = false;
        frame.modal_dismissals = controls;
        frame.recorder_hash = self
            .replay_ordinal
            .number()
            .is_multiple_of(25)
            .then(|| robin_engine::replay::state_hash(engine));
        self.begin_recording(&mut frame, true);
        frame.commit_timeline_after(self.current_frame());
        self.finish_recording(&mut frame);
    }

    /// Attach source evidence observed outside the deterministic command
    /// value (notably HTTP player-command and stepping ingress) to the current
    /// streaming replay ordinal.
    pub(super) fn record_input_taints(
        &mut self,
        taints: impl IntoIterator<Item = robin_engine::replay_rankability::InputTaintKind>,
    ) {
        let Some(recorder) = self.replay_recorder.as_mut() else {
            return;
        };
        let ordinal = self.replay_ordinal.number();
        for kind in taints {
            recorder.record_input_taint(kind, ordinal);
        }
    }

    /// Close the recorder frame opened by [`Self::begin_recording`].
    ///
    /// Normal presentation and emergency modal exits call this same owner
    /// method. Inactive frames (rewind, buffered replay, or no recorder)
    /// do not call `end_frame`, but they still cross this finalization boundary;
    /// closing an already-finished frame is a lifecycle bug.
    pub(super) fn finish_recording(&mut self, frame: &mut MissionFrame) {
        let mut consumed_record = frame.replay_record_consumed;
        if frame.close_recording() {
            let transition = frame.timeline_transition();
            let recorder = self
                .replay_recorder
                .as_mut()
                .expect("open recorder frame lost its recorder owner");
            let input = frame.authoritative_input();
            let host_controls = std::mem::take(&mut frame.modal_dismissals)
                .into_iter()
                .map(|command| match command {
                    PlayerCommand::ModalDismiss { kind, result } => {
                        robin_engine::replay::ReplayHostControl::ModalDismiss {
                            modal: kind,
                            result,
                        }
                    }
                    other => panic!("non-modal host control reached replay recorder: {other:?}"),
                })
                .collect();
            if recorder.write_frame(
                self.replay_ordinal.number(),
                transition.before.number(),
                transition.after.number(),
                input,
                host_controls,
                frame.recorder_hash,
            ) {
                consumed_record = true;
            }
        }
        if consumed_record {
            let transition = frame.timeline_transition();
            if let Some(expected) = frame.replay_timeline_transition {
                assert_eq!(transition, expected, "replay timeline transition diverged");
            }
            tracing::trace!(
                replay_ordinal = self.replay_ordinal.number(),
                timeline_before = transition.before.number(),
                timeline_after = transition.after.number(),
                "replay host record committed"
            );
            self.replay_ordinal.advance();
        }
        self.trace(FrameContractStage::RecorderCommit);
    }

    /// Seal the recorder after the deterministic quit-mission update has
    /// committed. Ranked replay validation requires that terminal command to
    /// be the final replay record; narrative/stat modal dismissals are host UI
    /// and must not extend the canonical simulation artifact.
    pub(super) fn seal_terminal_recording(&mut self, frame: &MissionFrame) -> bool {
        let terminal_count = frame
            .commands
            .commands
            .iter()
            .chain(frame.post_commands.commands.iter())
            .filter(|input| {
                matches!(
                    &input.command,
                    PlayerCommand::ApplyQuitMissionUpdates { .. }
                )
            })
            .count();
        if terminal_count == 0 {
            return false;
        }
        assert_eq!(
            terminal_count, 1,
            "one mission frame cannot contain multiple terminal updates"
        );
        assert_eq!(
            frame.recorder_state,
            RecorderFrameState::Finished,
            "terminal replay must be sealed after recorder finalization"
        );
        if let Some(recorder) = self.replay_recorder.take() {
            self.sealed_replay_header = Some(recorder.into_recording_header());
            tracing::debug!(
                replay_ordinal = self.replay_ordinal.number(),
                "sealed canonical replay at terminal mission record"
            );
        }
        true
    }

    fn transition(&mut self, expected: MissionPhase, next: MissionPhase) {
        transition_phase(&mut self.phase, expected, next);
    }
}

/// Apply the recorded save/load timeline events at the playback cursor's
/// pre-command boundary.
///
/// A save marker pins the complete current runtime save state (verified
/// against the recorded engine hash). A load-back restores it through the
/// normal load path, applies slot-specific post-load synchronization, and
/// resets rewind history, which no longer describes the engine's future.
pub(super) fn apply_replay_timeline_events_at_boundary(
    player: &ReplayPlayer,
    current_timeline: TimelineFrame,
    pinned_saves: &mut BTreeMap<u32, GameRuntimeSnapshot>,
    rewind_buffer: &mut RewindBuffer,
    host: &mut Host,
    game: &mut Game,
    manager: &mut EngineManager,
    assets: &LevelAssets,
) -> Result<Option<TimelineFrame>, String> {
    let frame = player.current_frame();
    let mut adopted_timeline = None;
    if let Some(marker) = player.save_marker_for_frame(frame) {
        if current_timeline.number() != marker.timeline_frame {
            return Err(format!(
                "replay save marker at ordinal {frame} belongs to timeline {}, current timeline is {}",
                marker.timeline_frame,
                current_timeline.number()
            ));
        }
        let actual = robin_engine::replay::state_hash(&manager.engine);
        if actual != marker.state_hash {
            return Err(format!(
                "replay save-marker desync at frame {frame}: \
                 expected {:016x}, got {actual:016x}",
                marker.state_hash
            ));
        }
        pinned_saves.insert(
            frame,
            GameRuntimeSnapshot::capture(&manager.engine, host, game).map_err(|error| {
                format!(
                    "replay save marker at ordinal {frame} could not pin save payload: {error:#}"
                )
            })?,
        );
        tracing::info!(frame, "replay playback: pinned save state");
    }
    if let Some(load_back) = player.load_back_for_frame(frame) {
        let pinned = pinned_saves
            .get(&load_back.to_frame)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "replay load-back at frame {frame} targets frame {}, \
                 but no save state was pinned there (corrupt recording?)",
                    load_back.to_frame
                )
            })?;
        pinned
            .apply_to_with_game(&mut manager.engine, host, game, assets)
            .map_err(|error| {
                format!(
                    "replay load-back at frame {frame} could not restore marker \
                     frame {}: {error}",
                    load_back.to_frame
                )
            })?;
        adopted_timeline = Some(TimelineFrame::from_wire(
            player
                .save_marker_for_frame(load_back.to_frame)
                .expect("validated load-back target marker")
                .timeline_frame,
        ));
        game.apply_post_load_sync(load_back.is_continue);
        game.post_load_resolution_resync();
        if frame == 0
            && load_back.to_frame == 0
            && let Some(expected) = player.hash_for_frame(0)
        {
            let actual = robin_engine::replay::state_hash(&manager.engine);
            if actual != expected {
                return Err(format!(
                    "Replay desync after bootstrap restore: expected {expected:016x}, got {actual:016x}"
                ));
            }
        }
        *rewind_buffer = RewindBuffer::new();
        tracing::info!(
            frame,
            to_frame = load_back.to_frame,
            "replay playback: jumped back to saved state"
        );
    }
    Ok(adopted_timeline)
}

fn transition_phase(phase: &mut MissionPhase, expected: MissionPhase, next: MissionPhase) {
    assert_eq!(
        *phase, expected,
        "invalid mission frame phase transition: {:?} -> {:?}",
        *phase, next
    );
    *phase = next;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_batches_cannot_edit_previously_admitted_inputs() {
        let mut frame = MissionFrame::new(0);
        frame.stage_commands().push(PlayerCommand::CrouchDown);
        {
            let mut batch = frame.stage_commands();
            assert!(
                batch.is_empty(),
                "producers never receive the existing journal"
            );
            batch.push(PlayerCommand::SetLockAlt(false));
            batch.commands.clear();
            batch.push(PlayerCommand::SetLockAlt(true));
        }
        frame
            .stage_post_commands()
            .push(PlayerCommand::QuitMissionRequested);
        drop(frame.stage_commands());
        assert_eq!(frame.commands().len(), 2);
        assert!(matches!(
            frame.commands()[0].command,
            PlayerCommand::CrouchDown
        ));
        assert!(matches!(
            frame.commands()[1].command,
            PlayerCommand::SetLockAlt(true)
        ));
        assert_eq!(frame.post_commands().len(), 1);
        frame.discard_commands();
        assert!(frame.commands().is_empty());
        assert_eq!(frame.post_commands().len(), 1, "discard is phase-specific");
    }

    #[test]
    fn appended_actions_preserve_applied_prefix_and_drain_once() {
        use robin_engine::engine::ExternalAction;
        let action = |ares| ExternalAction::ReplaceCampaign {
            campaign: robin_engine::campaign::Campaign {
                ares,
                ..Default::default()
            },
        };
        let mut frame = MissionFrame::new(0);
        frame.record_applied_post_external_actions(vec![action(1)]);
        {
            let mut batch = frame.stage_post_external_actions();
            batch.push(action(2));
            batch.clear();
            batch.push(action(3));
        }
        assert_eq!(frame.post_external_actions_applied, 1);
        assert_eq!(frame.unapplied_post_external_actions().len(), 1);
        assert!(matches!(frame.unapplied_post_external_actions()[0],
            ExternalAction::ReplaceCampaign { ref campaign } if campaign.ares == 3));
        frame.mark_post_external_actions_applied();
        frame.mark_post_external_actions_applied();
        assert!(frame.unapplied_post_external_actions().is_empty());
        frame.record_applied_post_external_actions(vec![action(4)]);
        assert!(frame.unapplied_post_external_actions().is_empty());
        assert_eq!(frame.authoritative_input().post_external_actions.len(), 3);

        frame.reset_after_terminal_restore(42);
        assert!(frame.authoritative_input().post_external_actions.is_empty());
        frame.stage_post_external_actions().push(action(5));
        assert_eq!(frame.unapplied_post_external_actions().len(), 1);
        assert_eq!(frame.recorder_hash, Some(42));
        assert_eq!(frame.recorder_state, RecorderFrameState::Inactive);
    }

    #[test]
    fn append_authority_is_diagnostic_only() {
        let mut frame = MissionFrame::new(0);
        let value = serde_json::to_value(frame.stage_commands()).unwrap();
        assert!(serde_json::from_value::<FrameCommandBatch<'_>>(value).is_err());
        let value = serde_json::to_value(frame.stage_external_actions()).unwrap();
        assert!(serde_json::from_value::<FrameActionBatch<'_>>(value).is_err());
    }

    #[test]
    fn post_tick_effects_preserve_recorded_order_and_are_not_reapplied() {
        use robin_engine::engine::{ExternalAction, SimulationFrameInput};

        let mut assets = LevelAssets::new();
        let mut engine = Engine::new_for_test_with_level_size(
            1024.0,
            768.0,
            Default::default(),
            &mut assets,
            4096.0,
            4096.0,
        )
        .unwrap();
        let mut host = Host::default();
        let application_context = host.application_context().clone();
        let mut dev = DevState::default();
        let mut http = crate::http_server::SessionIngress::detached_for_test();
        let mut timeline = TimelineRuntime::new(
            super::super::replay_init::ReplayAndRollback {
                recorder: None,
                player: None,
                rollback_checker: None,
                rewind_buffer: RewindBuffer::new(),
                start_paused: false,
            },
            FrameContract::Graphical,
            false,
            true,
        );
        let mut frame = MissionFrame::new(0);
        let replace_campaign = |ares| ExternalAction::ReplaceCampaign {
            campaign: robin_engine::campaign::Campaign {
                ares,
                ..Default::default()
            },
        };
        frame.post_external_actions = vec![replace_campaign(3), replace_campaign(7)];
        drain_post_tick_rpc(
            &mut http,
            &mut timeline,
            &mut host.frontend,
            &mut host.audio,
            &mut host.effects,
            &application_context,
            &host.transport,
            &mut engine,
            &assets,
            &mut dev,
            &mut frame,
        );
        assert_eq!(
            engine.campaign().ares,
            7,
            "recorded actions retain FIFO order"
        );
        assert_eq!(
            dev.noise_display_start_radius, 14,
            "both effect batches run, even when empty"
        );
        assert!(frame.unapplied_post_external_actions().is_empty());
        assert_eq!(
            frame.post_external_actions.len(),
            2,
            "journal keeps each recorded action once"
        );

        engine
            .advance_frame(
                &assets,
                SimulationFrameInput::no_hourglass()
                    .with_post_external_actions(vec![replace_campaign(9)]),
            )
            .unwrap();
        drain_post_tick_rpc(
            &mut http,
            &mut timeline,
            &mut host.frontend,
            &mut host.audio,
            &mut host.effects,
            &application_context,
            &host.transport,
            &mut engine,
            &assets,
            &mut dev,
            &mut frame,
        );
        assert_eq!(
            engine.campaign().ares,
            9,
            "a second drain cannot replay old actions"
        );
        assert_eq!(
            dev.noise_display_start_radius, 14,
            "a second drain cannot replay old effects"
        );
        assert_eq!(frame.post_external_actions.len(), 2);
    }

    #[test]
    fn input_and_presentation_capabilities_preserve_simulation_until_command_admission() {
        let mut assets = LevelAssets::new();
        let engine = Engine::new_for_test_with_level_size(
            1024.0,
            768.0,
            Default::default(),
            &mut assets,
            4096.0,
            4096.0,
        )
        .expect("capability fixture engine");
        let original = robin_engine::replay::state_hash(&engine);
        let mut world = MissionWorld::new(
            Host::scratch(1024.0, 768.0),
            Game::default(),
            EngineManager::new(engine),
            Arc::new(assets),
            DevState::default(),
        );
        let mut frame = MissionFrame::new(17);
        {
            let MissionInputPhase {
                host,
                game,
                engine,
                mut commands,
                ..
            } = world.input_phase(&mut frame);
            super::super::mouse_input::dispatch_corner_button_left_click(
                crate::corner_hud::CornerButton::Sight,
                engine,
                game,
                host,
                &mut commands,
            );
            assert_eq!(robin_engine::replay::state_hash(engine), original);
        }
        assert_eq!(frame.commands.commands.len(), 1);
        assert!(matches!(
            frame.commands.commands[0].command,
            PlayerCommand::SetLockAlt(true)
        ));
        {
            let mut phase = world.post_tick_input_phase(&mut frame);
            phase
                .commands
                .push(PlayerCommand::ClearNpcDoubleStatusBarFlags);
            phase
                .external_actions
                .push(robin_engine::engine::ExternalAction::Native {
                    name: "post-tick cursor fixture".into(),
                    args: Vec::new(),
                    this_actor: None,
                });
            assert_eq!(robin_engine::replay::state_hash(phase.engine), original);
        }
        assert_eq!(frame.commands.commands.len(), 1);
        assert!(frame.external_actions.is_empty());
        assert_eq!(frame.post_commands.commands.len(), 1);
        assert_eq!(frame.post_external_actions.len(), 1);
        {
            let phase = world.audio_phase();
            phase.audio.sound.set_listen_point(
                phase.viewport.sound_listen_point(),
                phase.viewport.zoom_factor,
            );
            assert_eq!(robin_engine::replay::state_hash(phase.engine), original);
        }
        {
            let MissionPresentationPhase {
                host, game, engine, ..
            } = world.presentation_phase();
            host.frontend.draw_order = engine.compute_display_order();
            game.display_message("presentation only".into(), 2);
            assert_eq!(robin_engine::replay::state_hash(engine), original);
        }
        assert_eq!(
            robin_engine::replay::state_hash(&world.view().manager.engine),
            original
        );
    }

    fn test_mission_assets(mission: &str) -> robin_engine::mission_assets::MissionAssetDescriptor {
        robin_engine::mission_assets::MissionAssetDescriptor::built_in(mission, mission, mission)
            .expect("valid built-in test mission descriptor")
    }

    fn timeline_for_trace_test(contract: FrameContract) -> TimelineRuntime {
        TimelineRuntime::new(
            ReplayAndRollback {
                recorder: None,
                player: None,
                rollback_checker: None,
                rewind_buffer: RewindBuffer::new(),
                start_paused: false,
            },
            contract,
            false,
            true,
        )
    }

    fn multiplayer_timeline(local_is_host: bool) -> TimelineRuntime {
        TimelineRuntime::new(
            ReplayAndRollback {
                recorder: None,
                player: None,
                rollback_checker: None,
                rewind_buffer: RewindBuffer::new(),
                start_paused: false,
            },
            FrameContract::Headless,
            true,
            local_is_host,
        )
    }

    #[test]
    fn terminal_apply_quit_is_eof_for_local_and_echoed_multiplayer_admission() {
        for echoed_multiplayer_command in [false, true] {
            let replay = tempfile::NamedTempFile::new().unwrap();
            let path = replay.path().to_string_lossy().into_owned();
            drop(replay);
            let recorder = ReplayRecorder::new(
                &path,
                "H01".to_owned(),
                test_mission_assets("H01"),
                7,
                robin_engine::engine::SimConfig::default(),
                &robin_engine::campaign::Campaign::default(),
            )
            .unwrap();
            let mut timeline = TimelineRuntime::new(
                ReplayAndRollback {
                    recorder: Some(recorder),
                    player: None,
                    rollback_checker: None,
                    rewind_buffer: RewindBuffer::new(),
                    start_paused: false,
                },
                FrameContract::Graphical,
                false,
                true,
            );

            let mut terminal = MissionFrame::new(0);
            terminal.bind_timeline(timeline.current_frame());
            let command = PlayerCommand::ApplyQuitMissionUpdates {
                exit_code: GameCode::LevelSucceeded,
                difficulty: Default::default(),
                completed_at_unix_seconds: Some(1),
                campaign_run_nonce: Some(2),
            };
            if echoed_multiplayer_command {
                terminal.commands.push(command);
            } else {
                terminal.post_commands.push(command);
            }
            timeline.begin_recording(&mut terminal, true);
            let after = timeline.advance_frame();
            terminal.commit_timeline_after(after);
            timeline.finish_recording(&mut terminal);
            assert!(timeline.seal_terminal_recording(&terminal));
            assert!(timeline.replay_recorder.is_none());

            // Narrative UI continues for more outer frames, but sealing makes
            // it impossible for a modal dismissal (or any other host input)
            // to extend the artifact beyond the terminal command.
            timeline.begin_execution_trace(FrameContractStage::NetworkIngress);
            let mut debrief = MissionFrame::new(1);
            debrief.bind_timeline(timeline.current_frame());
            debrief.modal_dismissals.push(PlayerCommand::ModalDismiss {
                kind: robin_engine::player_command::ModalKind::MissionState {
                    kind: robin_engine::player_command::MissionStateModalKind::EndState {
                        won: true,
                    },
                },
                result: robin_engine::player_command::DialogResult::Completed,
            });
            timeline.begin_recording(&mut debrief, true);
            debrief.commit_timeline_after(timeline.current_frame());
            timeline.finish_recording(&mut debrief);
            drop(timeline);

            let replay = robin_engine::replay::ReplayData::from_file(&path).unwrap();
            assert_eq!(replay.frame_count(), 1);
            let final_frame = replay.frame(0).unwrap();
            let terminal_commands = final_frame
                .input
                .commands
                .iter()
                .chain(final_frame.input.post_commands.iter())
                .filter(|command| {
                    matches!(
                        &command.player_input().command,
                        PlayerCommand::ApplyQuitMissionUpdates { .. }
                    )
                })
                .count();
            assert_eq!(terminal_commands, 1);
        }
    }

    #[test]
    fn nonterminal_record_does_not_seal_recorder() {
        let mut timeline = timeline_for_trace_test(FrameContract::Graphical);
        let mut frame = MissionFrame::new(0);
        frame.bind_timeline(timeline.current_frame());
        frame.commit_timeline_after(timeline.current_frame());
        frame.recorder_state = RecorderFrameState::Finished;

        assert!(!timeline.seal_terminal_recording(&frame));
    }

    #[test]
    fn bootstrap_marker_requires_a_completed_restart_save() {
        let directory = tempfile::tempdir().unwrap();
        let mut assets = LevelAssets::new();
        let engine = Engine::new_for_test(
            1024.0,
            768.0,
            robin_engine::campaign::Campaign::default(),
            &mut assets,
        )
        .unwrap();
        let host = Host::scratch(1024.0, 768.0);
        let game = Game::default();
        for restart_save_started in [false, true] {
            let path = directory
                .path()
                .join(format!("{restart_save_started}.rhrec.jsonl"));
            let recorder = ReplayRecorder::new(
                path.to_str().unwrap(),
                "bootstrap".into(),
                test_mission_assets("bootstrap"),
                0,
                robin_engine::engine::SimConfig::default(),
                engine.campaign(),
            )
            .unwrap();
            let mut timeline = timeline_for_trace_test(FrameContract::Graphical);
            timeline.replay_recorder = Some(recorder);
            timeline.register_bootstrap_save(
                restart_save_started
                    .then(|| BootstrapSaveBoundary::capture(&engine, &host, &game, None)),
            );
            if restart_save_started {
                let identity =
                    GameRuntimeSnapshot::identity_of_live(&engine, &host, &game).unwrap();
                assert_eq!(
                    timeline.recorded_save_frames_by_identity.get(&identity),
                    Some(&(ReplayFrameOrdinal::ZERO, TimelineFrame::ZERO)),
                );
            } else {
                assert!(timeline.recorded_save_frames_by_identity.is_empty());
            }
            // Metadata is valid only beside an authoritative recorded frame.
            assert!(timeline.replay_recorder.as_mut().unwrap().write_frame(
                0,
                0,
                1,
                robin_engine::engine::SimulationFrameInput::new(Vec::new()).with_hourglass(true),
                Vec::new(),
                None,
            ));
            drop(timeline);
            let replay =
                robin_engine::replay::ReplayData::from_file(path.to_str().unwrap()).unwrap();
            assert_eq!(
                replay.save_marker_for_frame(0),
                restart_save_started.then(|| robin_engine::replay::ReplaySaveMarker {
                    state_hash: robin_engine::replay::state_hash(&engine),
                    timeline_frame: 0,
                }),
            );
        }
    }

    #[test]
    fn foreign_live_save_retires_recording_until_matching_bootstrap_restores_valid_export() {
        let _serial = crate::replay_service::replay_spool_test_lock();
        let mut assets = LevelAssets::new();
        let mut engine = Engine::new_for_test(
            1024.0,
            768.0,
            robin_engine::campaign::Campaign::default(),
            &mut assets,
        )
        .unwrap();
        let mut host = Host::scratch(1024.0, 768.0);
        let mut game = Game::default();
        let pristine = engine.clone();
        let checkpoint = crate::save_file::PreparedGameSave::capture_session_restart(
            &engine,
            &host,
            &game,
            crate::save_file::SaveHeader::new(
                1,
                test_mission_assets("foreign"),
                "Restart".into(),
                crate::save_file::SaveProvenance::new("Restart".into(), 0, "Player".into())
                    .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let bootstrap_identity = checkpoint.replay_identity().unwrap();
        let mut timeline = timeline_for_trace_test(FrameContract::Graphical);
        timeline.replay_recorder = Some(
            ReplayRecorder::with_writer(
                Box::new(crate::replay_service::process().begin_recording()),
                "foreign".into(),
                test_mission_assets("foreign"),
                0,
                Default::default(),
                engine.campaign(),
            )
            .unwrap(),
        );
        timeline.register_bootstrap_save(Some(BootstrapSaveBoundary::capture(
            &engine,
            &host,
            &game,
            Some(bootstrap_identity),
        )));
        engine
            .advance_frame(
                &assets,
                robin_engine::engine::SimulationFrameInput::new(vec![
                    PlayerCommand::SetFastForward.into(),
                ])
                .with_hourglass(false),
            )
            .unwrap();
        let foreign = crate::save_file::PreparedGameSave::capture_session_restart(
            &engine,
            &host,
            &game,
            checkpoint.header.clone(),
        )
        .unwrap();
        let identity = foreign.replay_identity().unwrap();
        assert_ne!(identity, bootstrap_identity);
        foreign
            .apply_to_with_game(&mut engine, &mut host, &mut game, &assets)
            .unwrap();
        let mut frame = MissionFrame::new(0);
        frame.bind_timeline(timeline.current_frame());
        timeline.begin_recording(&mut frame, true);
        timeline.note_save_load_event(
            crate::main_entry::SaveLoadEvent::LoadApplied {
                identity,
                is_continue: false,
            },
            &mut frame,
            &engine,
            &assets,
        );
        assert!(matches!(
            timeline.recording_validity,
            RecordingValidity::Invalid { .. }
        ));
        assert!(timeline.replay_recorder.is_none());
        assert!(timeline.recorded_save_frames_by_identity.is_empty());
        assert!(
            crate::replay_service::process()
                .snapshot_bytes()
                .unwrap_err()
                .contains("not recorded in this session")
        );
        // Invalidating a recorder during a host frame also retires the open
        // recording transaction; its ordinary finalizer must not panic.
        timeline.finish_recording(&mut frame);
        let mut next = MissionFrame::new(1);
        timeline.begin_recording(&mut next, true);
        assert_eq!(next.recorder_state, RecorderFrameState::Inactive);

        checkpoint
            .apply_to_with_game(&mut engine, &mut host, &mut game, &assets)
            .unwrap();
        next.bind_timeline(timeline.current_frame());
        timeline.note_save_load_event(
            crate::main_entry::SaveLoadEvent::LoadApplied {
                identity: bootstrap_identity,
                is_continue: false,
            },
            &mut next,
            &engine,
            &assets,
        );
        assert_eq!(timeline.recording_validity, RecordingValidity::Linear);
        assert!(timeline.replay_recorder.is_some());
        assert!(timeline.sealed_replay_header.is_none());
        assert_eq!(timeline.replay_ordinal, ReplayFrameOrdinal::ZERO);
        assert_eq!(timeline.current_frame(), TimelineFrame::ZERO);
        assert_eq!(
            timeline
                .recorded_save_frames_by_identity
                .get(&bootstrap_identity),
            Some(&(ReplayFrameOrdinal::ZERO, TimelineFrame::ZERO)),
        );
        timeline.begin_execution_trace(FrameContractStage::TimelineBegin);
        timeline.begin_recording(&mut next, true);
        next.commit_timeline_after(timeline.advance_frame());
        timeline.finish_recording(&mut next);
        let restored_hash = robin_engine::replay::state_hash(&engine);
        let replay = crate::replay_service::process()
            .snapshot()
            .unwrap()
            .parse_sync()
            .unwrap();
        assert_eq!(replay.frame_count(), 1);
        assert_eq!(replay.load_back_for_frame(0).unwrap().to_frame, 0);
        assert_eq!(
            replay.save_marker_for_frame(0).unwrap().state_hash,
            robin_engine::replay::state_hash(&pristine),
        );
        // Validate the new export's actual restore boundary, not merely that
        // clearing the invalid flag made bytes available again.
        let mut playback = timeline_for_trace_test(FrameContract::Headless);
        playback.replay_player = Some(ReplayPlayer::new(replay));
        let mut manager = EngineManager::new(pristine);
        playback
            .apply_playback_timeline_events(
                &mut Host::scratch(1024.0, 768.0),
                &mut Game::default(),
                &mut manager,
                &assets,
            )
            .unwrap();
        assert_eq!(
            robin_engine::replay::state_hash(&manager.engine),
            restored_hash
        );
    }

    #[test]
    fn terminal_restart_exports_new_attempt_and_replays_its_restore_boundary() {
        let _serial = crate::replay_service::replay_spool_test_lock();
        let mut assets = LevelAssets::new();
        let campaign = robin_engine::campaign::Campaign::default();
        let mut engine =
            Engine::new_for_test(1024.0, 768.0, campaign.clone(), &mut assets).unwrap();
        let mut host = Host::scratch(1024.0, 768.0);
        let mut game = Game::default();
        engine
            .advance_frame(
                &assets,
                robin_engine::engine::SimulationFrameInput::new(vec![
                    PlayerCommand::SetFastForward.into(),
                ])
                .with_hourglass(false),
            )
            .unwrap();
        let pristine = engine.clone();
        let checkpoint = crate::save_file::PreparedGameSave::capture_session_restart(
            &engine,
            &host,
            &game,
            crate::save_file::SaveHeader::new(
                1,
                test_mission_assets("restart"),
                "Restart".into(),
                crate::save_file::SaveProvenance::new("Restart".into(), 0, "Player".into())
                    .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let identity = checkpoint.replay_identity().unwrap();
        let recorder = ReplayRecorder::with_writer(
            Box::new(crate::replay_service::process().begin_recording()),
            "restart".into(),
            test_mission_assets("restart"),
            17,
            Default::default(),
            &campaign,
        )
        .unwrap();
        let mut timeline = timeline_for_trace_test(FrameContract::Graphical);
        timeline.replay_recorder = Some(recorder);
        timeline.register_bootstrap_save(Some(BootstrapSaveBoundary::capture(
            &engine,
            &host,
            &game,
            Some(identity),
        )));

        // Complete the first attempt. Its immutable export must remain intact
        // while the active spool switches to each subsequent restored attempt.
        let mut terminal = MissionFrame::new(0);
        terminal.bind_timeline(timeline.current_frame());
        terminal
            .post_commands
            .push(PlayerCommand::ApplyQuitMissionUpdates {
                exit_code: GameCode::LevelFailed,
                difficulty: Default::default(),
                completed_at_unix_seconds: Some(1),
                campaign_run_nonce: Some(2),
            });
        timeline.begin_recording(&mut terminal, true);
        terminal.commit_timeline_after(timeline.advance_frame());
        timeline.finish_recording(&mut terminal);
        timeline.seal_terminal_recording(&terminal);
        let previous = crate::replay_service::process().snapshot().unwrap();
        let original = previous.parse_sync().unwrap();

        for attempt in 0..2 {
            engine.test_set_frame_counter(123);
            checkpoint
                .clone()
                .apply_to_with_game(&mut engine, &mut host, &mut game, &assets)
                .unwrap();
            let restored_hash = robin_engine::replay::state_hash(&engine);
            assert_ne!(restored_hash, robin_engine::replay::state_hash(&pristine));
            let mut frame = MissionFrame::new(0);
            frame.bind_timeline(timeline.current_frame());
            timeline.note_save_load_event(
                crate::main_entry::SaveLoadEvent::LoadApplied {
                    identity,
                    is_continue: false,
                },
                &mut frame,
                &engine,
                &assets,
            );
            assert!(timeline.replay_recorder.is_some());
            assert_eq!(timeline.replay_ordinal, ReplayFrameOrdinal::ZERO);
            assert_eq!(frame.recorder_hash, Some(restored_hash));
            frame
                .post_commands
                .push(PlayerCommand::ApplyQuitMissionUpdates {
                    exit_code: GameCode::LevelFailed,
                    difficulty: Default::default(),
                    completed_at_unix_seconds: Some(10 + attempt),
                    campaign_run_nonce: Some(2),
                });
            timeline.begin_execution_trace(FrameContractStage::TimelineBegin);
            timeline.begin_recording(&mut frame, true);
            frame.commit_timeline_after(timeline.advance_frame());
            timeline.finish_recording(&mut frame);
            timeline.seal_terminal_recording(&frame);
            let restarted = crate::replay_service::process()
                .snapshot()
                .unwrap()
                .parse_sync()
                .unwrap();
            let compact = robin_replay_format::encode_compact(
                &restarted,
                robin_replay_format::ENGINE_VERSION_HASH,
            )
            .unwrap();
            let (_, restarted) = robin_replay_format::decode_compact(&compact).unwrap();
            assert_eq!(restarted.frame_count(), 1);
            assert_eq!(restarted.header().campaign, original.header().campaign);
            assert_eq!(restarted.header().rng_seed, 17);
            assert_eq!(restarted.load_back_for_frame(0).unwrap().to_frame, 0);
            assert!(
                restarted
                    .rankability()
                    .unwrap()
                    .taints()
                    .iter()
                    .any(|taint| taint.kind
                        == robin_engine::replay_rankability::InputTaintKind::StateLoad)
            );
            assert_eq!(
                serde_json::to_value(
                    &previous
                        .parse_sync()
                        .unwrap()
                        .frame(0)
                        .unwrap()
                        .input
                        .post_commands
                )
                .unwrap(),
                serde_json::to_value(&original.frame(0).unwrap().input.post_commands).unwrap(),
            );

            for contract in [FrameContract::Graphical, FrameContract::Headless] {
                let mut playback = timeline_for_trace_test(contract);
                playback.replay_player = Some(ReplayPlayer::new(restarted.clone()));
                let mut manager =
                    robin_engine::engine_manager::EngineManager::new(pristine.clone());
                let mut playback_host = Host::scratch(1024.0, 768.0);
                let mut playback_game = Game::default();
                // Match both drivers: open the frame first, then pin and restore
                // before admitting input. The recorded hash is post-restore.
                playback.begin_frame(0, &manager.engine, &assets);
                playback
                    .apply_playback_timeline_events(
                        &mut playback_host,
                        &mut playback_game,
                        &mut manager,
                        &assets,
                    )
                    .unwrap();
                assert_eq!(
                    robin_engine::replay::state_hash(&manager.engine),
                    restored_hash
                );
            }
            let mut corrupted = restarted.clone();
            corrupted
                .replace_state_hashes(BTreeMap::from([(0, restored_hash ^ 1)]))
                .unwrap();
            let mut playback = timeline_for_trace_test(FrameContract::Headless);
            playback.replay_player = Some(ReplayPlayer::new(corrupted));
            let mut manager = robin_engine::engine_manager::EngineManager::new(pristine.clone());
            assert!(
                playback
                    .apply_playback_timeline_events(
                        &mut Host::scratch(1024.0, 768.0),
                        &mut Game::default(),
                        &mut manager,
                        &assets,
                    )
                    .unwrap_err()
                    .contains("desync after bootstrap restore")
            );
            let mut playback = timeline_for_trace_test(FrameContract::Headless);
            playback.replay_player = Some(ReplayPlayer::new(restarted));
            manager.engine.test_set_frame_counter(999);
            assert!(
                playback
                    .apply_playback_timeline_events(
                        &mut Host::scratch(1024.0, 768.0),
                        &mut Game::default(),
                        &mut manager,
                        &assets,
                    )
                    .unwrap_err()
                    .contains("save-marker desync")
            );
        }
        // A different post-terminal save cannot be reconstructed from the
        // original header; fail the active export instead of advertising stale data.
        let foreign = crate::save_file::PreparedGameSave::capture_session_restart(
            &engine,
            &host,
            &game,
            checkpoint.header.clone(),
        )
        .unwrap();
        let mut frame = MissionFrame::new(0);
        frame.bind_timeline(timeline.current_frame());
        timeline.note_save_load_event(
            crate::main_entry::SaveLoadEvent::LoadApplied {
                identity: foreign.replay_identity().unwrap(),
                is_continue: false,
            },
            &mut frame,
            &engine,
            &assets,
        );
        assert!(timeline.replay_recorder.is_none());
        assert!(
            crate::replay_service::process()
                .snapshot_bytes()
                .unwrap_err()
                .contains("non-bootstrap save")
        );
        assert_eq!(previous.parse_sync().unwrap().frame_count(), 1);
    }

    #[test]
    fn session_restart_records_load_back_to_its_bootstrap_marker() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session-restart.rhrec.jsonl");
        let mut assets = LevelAssets::new();
        let mut engine =
            Engine::new_for_test(1024.0, 768.0, Default::default(), &mut assets).unwrap();
        let mut host = Host::scratch(1024.0, 768.0);
        let mut game = Game::default();
        let header = crate::save_file::SaveHeader::new(
            1,
            test_mission_assets("restart"),
            "Restart".into(),
            crate::save_file::SaveProvenance::new("Restart".into(), 0, "Player".into()).unwrap(),
        )
        .unwrap();
        let checkpoint = crate::save_file::PreparedGameSave::capture_session_restart(
            &engine, &host, &game, header,
        )
        .unwrap();
        let identity = checkpoint.replay_identity().unwrap();
        let marker_hash = robin_engine::replay::state_hash(&engine);
        let recorder = ReplayRecorder::new(
            path.to_str().unwrap(),
            "restart".into(),
            test_mission_assets("restart"),
            0,
            Default::default(),
            engine.campaign(),
        )
        .unwrap();
        let mut timeline = timeline_for_trace_test(FrameContract::Headless);
        timeline.replay_recorder = Some(recorder);
        timeline.register_bootstrap_save(Some(BootstrapSaveBoundary::capture(
            &engine,
            &host,
            &game,
            Some(identity),
        )));
        for _ in 0..3 {
            timeline.begin_execution_trace(FrameContractStage::TimelineBegin);
            let mut frame = MissionFrame::new(0);
            frame.bind_timeline(timeline.current_frame());
            timeline.begin_recording(&mut frame, true);
            frame.commit_timeline_after(timeline.advance_frame());
            timeline.finish_recording(&mut frame);
        }
        engine.test_set_frame_counter(123);
        checkpoint
            .apply_to_with_game(&mut engine, &mut host, &mut game, &assets)
            .unwrap();
        let mut frame = MissionFrame::new(0);
        frame.bind_timeline(timeline.current_frame());
        timeline.note_save_load_event(
            crate::main_entry::SaveLoadEvent::LoadApplied {
                identity,
                is_continue: false,
            },
            &mut frame,
            &engine,
            &assets,
        );
        assert!(timeline.take_state_restored());
        assert!(!timeline.take_state_restored());
        assert_eq!(timeline.current_frame(), TimelineFrame::ZERO);
        timeline.begin_execution_trace(FrameContractStage::TimelineBegin);
        timeline.begin_recording(&mut frame, true);
        frame.commit_timeline_after(timeline.advance_frame());
        timeline.finish_recording(&mut frame);
        drop(timeline);
        let replay = robin_engine::replay::ReplayData::from_file(path.to_str().unwrap()).unwrap();
        assert_eq!(
            replay.save_marker_for_frame(0).unwrap().state_hash,
            marker_hash
        );
        assert_eq!(
            replay.load_back_for_frame(3),
            Some(robin_engine::replay::ReplayLoadBack {
                to_frame: 0,
                is_continue: false
            })
        );
    }

    #[test]
    fn in_mission_save_and_load_record_and_replay_as_one_linear_timeline() {
        let path = std::env::temp_dir()
            .join(format!(
                "robin_runtime_timeline_{}.rhrec.jsonl",
                std::process::id()
            ))
            .to_string_lossy()
            .into_owned();

        let mut assets = LevelAssets::new();
        let mut engine = Engine::new_for_test_with_level_size(
            1024.0,
            768.0,
            robin_engine::campaign::Campaign::default(),
            &mut assets,
            4096.0,
            4096.0,
        )
        .expect("fixture engine");
        let mut host = Host::scratch(1024.0, 768.0);
        let mut game = Game::default();
        host.frontend.input.draw_hidden = true;
        game.persistent.campaign_map_displayed = true;
        game.persistent.campaign_map_active = false;
        engine
            .advance_frame(
                &assets,
                robin_engine::engine::SimulationFrameInput::new(Vec::new()),
            )
            .expect("initialize normal owner-pass scratch");
        assert!(engine.ai_global().primary_target_multiplicity_initialized);
        engine
            .advance_frame(
                &assets,
                robin_engine::engine::SimulationFrameInput::new(vec![
                    PlayerCommand::SetFastForward.into(),
                ])
                .with_hourglass(false),
            )
            .expect("fast-forward command admission");
        assert!(engine.is_fast_forward());
        let marker_engine = engine.clone();
        let marker_hash = robin_engine::replay::state_hash(&engine);
        let save = crate::save_file::GameSaveFile::capture_with_game(
            &engine,
            &host,
            &game,
            1,
            test_mission_assets("timeline"),
            "timeline".into(),
            crate::save_file::SaveProvenance::new("Timeline Test".into(), 0, "Test Player".into())
                .expect("valid test save provenance"),
        )
        .expect("capture timeline save");
        let identity = save.replay_identity().expect("save identity");
        // A live load reads the persisted JSON, not the captured in-memory
        // clone. Exercise the actual persistence boundary in this comparison.
        let save_directory = tempfile::tempdir().expect("save directory");
        let save_path = save_directory.path().join("QuickSave.json");
        save.write_to(&save_path).expect("write live save");
        let save = crate::save_file::GameSaveFile::read_from(&save_path).expect("read live save");

        // ── Live side: save at frame 0, diverge, load back at frame 5. ──
        let recorder = ReplayRecorder::new(
            &path,
            "timeline".into(),
            test_mission_assets("timeline"),
            0,
            robin_engine::engine::SimConfig::default(),
            &robin_engine::campaign::Campaign::default(),
        )
        .expect("recorder");
        let mut live = TimelineRuntime::new(
            ReplayAndRollback {
                recorder: Some(recorder),
                player: None,
                rollback_checker: None,
                rewind_buffer: RewindBuffer::new(),
                start_paused: false,
            },
            FrameContract::Headless,
            false,
            true,
        );
        let mut frame = MissionFrame::new(0);
        frame.bind_timeline(live.current_frame());
        live.note_save_load_event(
            crate::main_entry::SaveLoadEvent::SaveWritten { identity },
            &mut frame,
            &engine,
            &assets,
        );
        for _ in 0..5 {
            live.begin_execution_trace(FrameContractStage::TimelineBegin);
            let mut frame = MissionFrame::new(0);
            frame.bind_timeline(live.current_frame());
            live.begin_recording(&mut frame, true);
            let after = live.advance_frame();
            frame.commit_timeline_after(after);
            live.finish_recording(&mut frame);
        }
        // Exercise the real restore path. Engine post-load fixups intentionally
        // normalize transient state, so its resulting hash differs from the raw
        // payload hash. Identity matching must still find the frame-0 save.
        engine
            .advance_frame(
                &assets,
                robin_engine::engine::SimulationFrameInput::new(vec![
                    PlayerCommand::SetAmountOfSpeaking { amount: 3 }.into(),
                ])
                .with_hourglass(false),
            )
            .expect("speech command admission");
        host.frontend.input.draw_hidden = false;
        game.persistent.campaign_map_displayed = false;
        save.apply_to_with_game(&mut engine, &mut host, &mut game, &assets)
            .expect("restore live save");
        game.apply_post_load_sync(true);
        game.post_load_resolution_resync();
        let restored_hash = robin_engine::replay::state_hash(&engine);
        assert_ne!(restored_hash, marker_hash);
        assert!(!engine.is_fast_forward());
        let mut frame = MissionFrame::new(0);
        frame.bind_timeline(live.current_frame());
        live.note_save_load_event(
            crate::main_entry::SaveLoadEvent::LoadApplied {
                identity,
                is_continue: true,
            },
            &mut frame,
            &engine,
            &assets,
        );
        assert_eq!(live.current_frame(), TimelineFrame::ZERO);
        live.begin_execution_trace(FrameContractStage::TimelineBegin);
        live.begin_recording(&mut frame, true);
        let after = live.advance_frame();
        frame.commit_timeline_after(after);
        live.finish_recording(&mut frame);
        drop(live);

        let data = robin_engine::replay::ReplayData::from_file(&path).expect("recorded replay");
        assert_eq!(
            data.save_marker_for_frame(0),
            Some(robin_engine::replay::ReplaySaveMarker {
                state_hash: marker_hash,
                timeline_frame: 0,
            })
        );
        assert_eq!(
            data.load_back_for_frame(5),
            Some(robin_engine::replay::ReplayLoadBack {
                to_frame: 0,
                is_continue: true,
            })
        );

        // ── Playback side: pin at frame 0, jump back at frame 5. ──
        let mut playback = TimelineRuntime::new(
            ReplayAndRollback {
                recorder: None,
                player: Some(ReplayPlayer::new(data)),
                rollback_checker: None,
                rewind_buffer: RewindBuffer::new(),
                start_paused: false,
            },
            FrameContract::Headless,
            false,
            true,
        );
        let mut manager = robin_engine::engine_manager::EngineManager::new(marker_engine);
        let mut playback_host = Host::scratch(1024.0, 768.0);
        playback_host.frontend.input.draw_hidden = true;
        let mut playback_game = Game::default();
        playback_game.persistent.campaign_map_displayed = true;
        playback
            .apply_playback_timeline_events(
                &mut playback_host,
                &mut playback_game,
                &mut manager,
                &assets,
            )
            .expect("pin replay save");
        assert!(playback.playback_pinned_saves.contains_key(&0));
        for _ in 0..5 {
            let ReplayStepAdmission::Recorded(recorded) = playback
                .consume_replay_frame_for_step()
                .expect("consume replay transaction")
            else {
                panic!("five replay transactions remain");
            };
            assert_eq!(recorded.timeline_before, playback.current_frame().number());
            playback.current_frame = TimelineFrame::from_wire(recorded.timeline_after);
        }
        // Diverge the playback engine, then let the load-back restore it.
        manager
            .engine
            .advance_frame(
                &assets,
                robin_engine::engine::SimulationFrameInput::new(vec![
                    PlayerCommand::SetAmountOfSpeaking { amount: 3 }.into(),
                ])
                .with_hourglass(false),
            )
            .expect("speech command admission");
        playback_host.frontend.input.draw_hidden = false;
        playback_game.persistent.campaign_map_displayed = false;
        playback_game.persistent.campaign_map_active = false;
        assert_ne!(
            robin_engine::replay::state_hash(&manager.engine),
            restored_hash
        );
        playback
            .apply_playback_timeline_events(
                &mut playback_host,
                &mut playback_game,
                &mut manager,
                &assets,
            )
            .expect("apply replay load-back");
        assert_eq!(
            robin_engine::replay::state_hash(&manager.engine),
            restored_hash
        );
        assert!(!manager.engine.is_fast_forward());
        assert!(playback_host.frontend.input.draw_hidden);
        assert!(playback_game.persistent.campaign_map_displayed);
        assert!(playback_game.persistent.campaign_map_active);
        assert!(playback_game.continue_requested);
        assert_eq!(
            manager
                .engine
                .ai_global()
                .primary_target_multiplicity_initialized,
            engine.ai_global().primary_target_multiplicity_initialized,
            "replay must restore the serialized save projection, not clone-only AI scratch"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn replay_save_marker_hash_mismatch_is_rejected_before_pinning() {
        let mut assets = LevelAssets::new();
        let engine = Engine::new_for_test_with_level_size(
            1024.0,
            768.0,
            robin_engine::campaign::Campaign::default(),
            &mut assets,
            4096.0,
            4096.0,
        )
        .expect("fixture engine");
        let actual_hash = robin_engine::replay::state_hash(&engine);
        let data: robin_engine::replay::ReplayData = robin_engine::replay::ReplayFile {
            header: robin_engine::replay::ReplayHeader {
                mission_id: "timeline".into(),
                mission_assets: test_mission_assets("timeline"),
                rng_seed: 0,
                sim_config: robin_engine::engine::SimConfig::default(),
                spellforge_package: None,
                version: robin_engine::replay::REPLAY_SCHEMA_VERSION,
                total_frames: 1,
                rankability: robin_engine::replay_rankability::ReplayRankability::rankable(),
                campaign: bitcode::encode(&robin_engine::campaign::Campaign::default()),
            },
            frames: BTreeMap::from([(
                0,
                robin_engine::replay::ReplayFrame {
                    timeline_before: 0,
                    timeline_after: 1,
                    input: robin_engine::engine::SimulationFrameInput::default(),
                    host_controls: Vec::new(),
                },
            )]),
            hashes: BTreeMap::new(),
            save_markers: BTreeMap::from([(
                0,
                robin_engine::replay::ReplaySaveMarker {
                    state_hash: actual_hash ^ 1,
                    timeline_frame: 0,
                },
            )]),
            load_backs: BTreeMap::new(),
        }
        .try_into()
        .expect("valid replay fixture");
        let player = ReplayPlayer::new(data);
        let mut pinned_saves = BTreeMap::new();
        let mut rewind_buffer = RewindBuffer::new();
        let mut host = Host::scratch(1024.0, 768.0);
        let mut game = Game::default();
        let mut manager = EngineManager::new(engine);

        let error = apply_replay_timeline_events_at_boundary(
            &player,
            TimelineFrame::ZERO,
            &mut pinned_saves,
            &mut rewind_buffer,
            &mut host,
            &mut game,
            &mut manager,
            &assets,
        )
        .expect_err("mismatched marker must fail playback");

        assert!(error.contains("save-marker desync"), "{error}");
        assert!(pinned_saves.is_empty());
    }

    #[test]
    fn whole_state_discontinuity_reopens_the_current_frame_before_commit() {
        let mut assets = LevelAssets::default();
        let engine = Engine::new_for_test(
            640.0,
            480.0,
            robin_engine::campaign::Campaign::default(),
            &mut assets,
        )
        .expect("fixture engine");
        let mut manager = EngineManager::new(engine);
        let mut host = Host::scratch(640.0, 480.0);
        let mut timeline = timeline_for_trace_test(FrameContract::Headless);
        timeline.adopt_frame(TimelineFrame::from_wire(crate::rewind::SNAPSHOT_INTERVAL));
        let mut frame = MissionFrame::new(0);

        timeline.open_frame(&mut frame, &manager.engine, &assets);
        // Save-load/replay adoption replaces the state after open_frame but
        // before this loop iteration's tick and history commit.
        timeline.reset_reconstruction_history(timeline.current_frame(), &manager.engine, &assets);
        timeline.begin_simulation();
        timeline.begin_bookkeeping();
        timeline.commit_simulation_history(
            &mut host,
            &mut manager,
            &frame,
            FrameCommitPolicy {
                store_rewind_commands: true,
            },
        );

        assert_eq!(
            timeline.retained_history().next_record_frame(),
            crate::rewind::SNAPSHOT_INTERVAL + 1
        );
        assert!(
            timeline
                .retained_history()
                .commands_for(crate::rewind::SNAPSHOT_INTERVAL)
                .is_some()
        );
    }

    #[test]
    fn lockstep_frame_and_engine_simulation_tick_are_independent_clocks() {
        let mut assets = LevelAssets::default();
        let engine = Engine::new_for_test(
            640.0,
            480.0,
            robin_engine::campaign::Campaign::default(),
            &mut assets,
        )
        .expect("fixture engine");
        let mut timeline = timeline_for_trace_test(FrameContract::Headless);

        assert_eq!(timeline.frame_number(), 0);
        assert_eq!(engine.simulation_tick().number(), 0);
        timeline.advance_frame();

        assert_eq!(timeline.frame_number(), 1);
        assert_eq!(engine.simulation_tick().number(), 0);
    }

    #[test]
    fn authoritative_frame_adoption_drops_only_stale_typed_inputs() {
        let mut timeline = timeline_for_trace_test(FrameContract::Headless);
        timeline.network.queue_input(
            TimelineFrame::from_wire(3),
            robin_engine::player_command::PlayerInput::host(PlayerCommand::QuitMissionRequested),
        );
        timeline.network.queue_input(
            TimelineFrame::from_wire(7),
            robin_engine::player_command::PlayerInput::host(PlayerCommand::QuitMissionRequested),
        );

        timeline.adopt_frame(TimelineFrame::from_wire(5));

        assert_eq!(timeline.frame_number(), 5);
        assert!(
            timeline
                .network
                .take_inputs(TimelineFrame::from_wire(3))
                .is_empty()
        );
        assert_eq!(
            timeline
                .network
                .take_inputs(TimelineFrame::from_wire(7))
                .len(),
            1
        );
    }

    #[test]
    fn host_waits_for_begin_and_the_synchronized_release_time() {
        let mut timeline = multiplayer_timeline(true);
        assert_eq!(
            timeline.mp_admission,
            MultiplayerAdmission::HostWaitingForBegin
        );
        assert!(timeline.multiplayer_admission_paused(500));

        timeline.apply_multiplayer_admission_events(&[MultiplayerAdmissionEvent::BeginSim {
            frame: 0,
            start_epoch_ms: 1_000,
        }]);

        assert!(timeline.multiplayer_admission_paused(999));
        assert!(!timeline.multiplayer_admission_paused(1_000));
        assert_eq!(timeline.mp_admission, MultiplayerAdmission::Running);
    }

    #[test]
    fn joining_peer_requires_snapshot_then_begin_before_release() {
        let mut timeline = multiplayer_timeline(false);
        assert_eq!(
            timeline.mp_admission,
            MultiplayerAdmission::PeerWaitingForSnapshot
        );

        timeline.apply_multiplayer_admission_events(&[
            MultiplayerAdmissionEvent::InitialSnapshotAdopted { frame: 37 },
            MultiplayerAdmissionEvent::BeginSim {
                frame: 37,
                start_epoch_ms: 2_000,
            },
        ]);

        assert_eq!(
            timeline.mp_admission,
            MultiplayerAdmission::WaitingForStart {
                frame: 37,
                start_epoch_ms: 2_000,
            }
        );
        assert!(timeline.multiplayer_admission_paused(1_999));
        assert!(!timeline.multiplayer_admission_paused(2_000));
    }

    #[test]
    fn host_resync_requires_explicit_adoption_and_matching_begin() {
        let mut timeline = multiplayer_timeline(true);
        timeline.mp_admission = MultiplayerAdmission::Running;
        timeline.adopt_frame(TimelineFrame::from_wire(35));
        timeline.remember_local_mp_hash(25, 7);
        timeline.network.admit_remote_hash(25, 7);
        timeline.begin_synchronized_step_resync();
        assert!(!timeline.has_local_mp_hash(25));
        assert!(timeline.take_due_mp_hash_comparisons().is_empty());
        assert!(timeline.multiplayer_admission_paused(99));
        timeline.apply_multiplayer_admission_events(&[MultiplayerAdmissionEvent::BeginSim {
            frame: 35,
            start_epoch_ms: 100,
        }]);
        assert!(timeline.multiplayer_admission_paused(99));
        assert!(!timeline.multiplayer_admission_paused(100));
    }

    #[test]
    #[should_panic(expected = "invalid multiplayer admission ordering")]
    fn running_host_rejects_unsolicited_begin() {
        let mut timeline = multiplayer_timeline(true);
        timeline.mp_admission = MultiplayerAdmission::Running;
        timeline.apply_multiplayer_admission_events(&[MultiplayerAdmissionEvent::BeginSim {
            frame: 35,
            start_epoch_ms: 100,
        }]);
    }

    #[test]
    #[should_panic(expected = "invalid multiplayer admission ordering")]
    fn resynchronizing_host_rejects_wrong_snapshot_frame() {
        let mut timeline = multiplayer_timeline(true);
        timeline.mp_admission = MultiplayerAdmission::Running;
        timeline.adopt_frame(TimelineFrame::from_wire(35));
        timeline.begin_synchronized_step_resync();
        timeline.apply_multiplayer_admission_events(&[MultiplayerAdmissionEvent::BeginSim {
            frame: 36,
            start_epoch_ms: 100,
        }]);
    }

    #[test]
    fn delayed_hash_comparison_uses_exact_frame_and_invalidates_rollback_future() {
        let mut timeline = multiplayer_timeline(false);
        timeline.remember_local_mp_hash(25, 10);
        timeline.remember_local_mp_hash(50, 20);
        timeline.network.admit_remote_hash(25, 10);
        timeline.network.admit_remote_hash(50, 30);
        timeline.network.admit_remote_hash(100, 40);
        timeline.adopt_frame(TimelineFrame::from_wire(75));
        timeline.invalidate_local_mp_hashes_after(25);
        assert_eq!(
            timeline.take_due_mp_hash_comparisons(),
            vec![(25, 10, Some(10)), (50, 30, None)]
        );
        assert!(timeline.take_due_mp_hash_comparisons().is_empty());
        timeline.adopt_frame(TimelineFrame::from_wire(100));
        assert_eq!(
            timeline.take_due_mp_hash_comparisons(),
            vec![(100, 40, None)]
        );
    }

    #[test]
    fn retained_mp_hashes_are_bounded_and_reconnect_discards_prediction_generation() {
        let mut timeline = multiplayer_timeline(false);
        for frame in 0..300 {
            timeline.remember_local_mp_hash(frame, u64::from(frame));
        }
        assert!(!timeline.has_local_mp_hash(43));
        assert!(timeline.has_local_mp_hash(44));
        timeline.apply_multiplayer_admission_events(&[MultiplayerAdmissionEvent::Disconnected]);
        assert!((0..300).all(|frame| !timeline.has_local_mp_hash(frame)));
    }

    #[test]
    fn delayed_hash_mismatch_is_not_discarded_when_peer_runs_ahead() {
        let mut timeline = multiplayer_timeline(false);
        timeline.remember_local_mp_hash(25, 123);
        timeline.remember_local_mp_hash(25, 999); // repeated paused boundary
        timeline.network.admit_remote_hash(25, 456);
        timeline.adopt_frame(TimelineFrame::from_wire(99));
        assert_eq!(
            timeline.take_due_mp_hash_comparisons(),
            vec![(25, 456, Some(123))]
        );
    }

    #[test]
    fn host_horizon_resync_event_rearms_only_its_exact_snapshot_barrier() {
        let mut timeline = multiplayer_timeline(true);
        timeline.mp_admission = MultiplayerAdmission::Running;
        timeline.apply_multiplayer_admission_events(&[
            MultiplayerAdmissionEvent::HostResynchronizing { frame: 50 },
            MultiplayerAdmissionEvent::BeginSim {
                frame: 50,
                start_epoch_ms: 100,
            },
        ]);
        assert!(timeline.multiplayer_admission_paused(99));
        assert!(!timeline.multiplayer_admission_paused(100));
    }

    #[test]
    fn host_can_rearm_a_released_barrier_before_its_future_start_time() {
        let mut timeline = multiplayer_timeline(true);
        timeline.apply_multiplayer_admission_events(&[
            MultiplayerAdmissionEvent::BeginSim {
                frame: 0,
                start_epoch_ms: 100,
            },
            MultiplayerAdmissionEvent::HostResynchronizing { frame: 0 },
            MultiplayerAdmissionEvent::BeginSim {
                frame: 0,
                start_epoch_ms: 200,
            },
        ]);
        assert!(timeline.multiplayer_admission_paused(100));
        assert!(!timeline.multiplayer_admission_paused(200));
    }

    #[test]
    fn disconnect_returns_running_peer_to_snapshot_admission() {
        let mut timeline = multiplayer_timeline(false);
        timeline.apply_multiplayer_admission_events(&[
            MultiplayerAdmissionEvent::InitialSnapshotAdopted { frame: 0 },
            MultiplayerAdmissionEvent::BeginSim {
                frame: 0,
                start_epoch_ms: 10,
            },
        ]);
        assert!(!timeline.multiplayer_admission_paused(10));

        timeline.apply_multiplayer_admission_events(&[MultiplayerAdmissionEvent::Disconnected]);

        assert_eq!(
            timeline.mp_admission,
            MultiplayerAdmission::PeerWaitingForSnapshot
        );
        assert!(timeline.multiplayer_admission_paused(11));
    }

    #[test]
    #[should_panic(expected = "invalid multiplayer admission ordering")]
    fn joining_peer_rejects_begin_before_snapshot() {
        let mut timeline = multiplayer_timeline(false);
        timeline.apply_multiplayer_admission_events(&[MultiplayerAdmissionEvent::BeginSim {
            frame: 0,
            start_epoch_ms: 10,
        }]);
    }

    #[test]
    fn mission_control_round_trips_without_defaulting_process_state() {
        let control = MissionControl {
            manual_pause: true,
            step_forward_repeat_at_ms: Some(120),
            step_back_repeat_at_ms: Some(240),
            last_shadow_color: 0x1234,
            last_visual_ambiance: robin_engine::engine::Ambiance::Fog,
        };
        let encoded = serde_json::to_string(&control).expect("serialize mission control");
        let decoded: MissionControl =
            serde_json::from_str(&encoded).expect("deserialize mission control");
        assert_eq!(decoded, control);
    }

    #[test]
    fn mission_frame_owns_only_one_iterations_commands() {
        let mut frame = MissionFrame::new(777);
        frame.commands.push(PlayerCommand::QuitMissionRequested);
        frame.recorder_hash = Some(0x55aa);

        let encoded = serde_json::to_string(&frame).expect("serialize mission frame");
        let decoded: MissionFrame =
            serde_json::from_str(&encoded).expect("deserialize mission frame");

        assert_eq!(decoded.started_at_ms, 777);
        assert_eq!(decoded.commands.commands.len(), 1);
        assert!(matches!(
            decoded.commands.commands[0].command,
            PlayerCommand::QuitMissionRequested
        ));
        assert_eq!(decoded.recorder_hash, Some(0x55aa));
        assert!(decoded.modal_dismissals.is_empty());
        assert!(decoded.replay_modal_dismissals.is_empty());
    }

    #[test]
    fn mission_frame_preserves_recorded_transaction_gates() {
        let mut frame = MissionFrame::new(777);
        frame.adopt_authoritative_input(
            robin_engine::engine::SimulationFrameInput::no_hourglass()
                .with_simulation_body_allowed(false)
                .with_post_initialize(false),
        );

        let recorded = frame.authoritative_input();
        assert!(!recorded.run_hourglass);
        assert!(!recorded.simulation_body_allowed);
        assert!(!recorded.run_post_initialize);
    }

    #[test]
    #[should_panic(expected = "cannot cross more than one sound boundary")]
    fn timeline_rejects_a_second_pending_sound_boundary() {
        let mut timeline = timeline_for_trace_test(FrameContract::Graphical);
        timeline.queue_sound_boundary(robin_engine::engine::SoundBoundary::live(Vec::new()));
        timeline.queue_sound_boundary(robin_engine::engine::SoundBoundary::live(Vec::new()));
    }

    #[test]
    fn mission_frame_adapter_preserves_pre_and_post_command_batches() {
        let mut frame = MissionFrame::new(777);
        frame.adopt_authoritative_input(
            robin_engine::engine::SimulationFrameInput::new(vec![
                robin_engine::engine::SimCommand::from(
                    PlayerCommand::SetMenToBlazonConversionMode { on: true },
                ),
            ])
            .with_post_commands(vec![robin_engine::engine::SimCommand::from(
                PlayerCommand::QuitMissionRequested,
            )]),
        );

        let recorded = frame.authoritative_input();
        let pre = recorded.player_inputs();
        let post = recorded.post_player_inputs();
        assert_eq!(pre.len(), 1);
        assert!(matches!(
            pre[0].command,
            PlayerCommand::SetMenToBlazonConversionMode { on: true }
        ));
        assert_eq!(post.len(), 1);
        assert!(matches!(
            post[0].command,
            PlayerCommand::QuitMissionRequested
        ));
    }

    #[test]
    fn graphical_execution_trace_keeps_original_refresh_sound_post_initialize_tail() {
        let mut timeline = timeline_for_trace_test(FrameContract::Graphical);
        timeline.begin_execution_trace(FrameContractStage::NetworkIngress);
        for stage in [
            FrameContractStage::TimelineBegin,
            FrameContractStage::InputAndMenus,
            FrameContractStage::OperationAndSave,
            FrameContractStage::SecondNetworkDrain,
            FrameContractStage::PreTickCommands,
            FrameContractStage::Simulation,
            FrameContractStage::HostRpcAndTimelineCommit,
            FrameContractStage::ModalDrain,
            FrameContractStage::AppEffects,
            FrameContractStage::Audio,
            FrameContractStage::Presentation,
        ] {
            timeline.trace(stage);
        }
        let mut dispatched = false;
        timeline.cross_post_initialize(|| dispatched = true);
        timeline.trace(FrameContractStage::RecorderCommit);
        timeline.trace(FrameContractStage::Pacing);

        assert!(dispatched);
        let stages = timeline.execution_trace();
        assert_eq!(
            stages,
            &[
                FrameContractStage::NetworkIngress,
                FrameContractStage::TimelineBegin,
                FrameContractStage::InputAndMenus,
                FrameContractStage::OperationAndSave,
                FrameContractStage::SecondNetworkDrain,
                FrameContractStage::PreTickCommands,
                FrameContractStage::Simulation,
                FrameContractStage::HostRpcAndTimelineCommit,
                FrameContractStage::ModalDrain,
                FrameContractStage::AppEffects,
                FrameContractStage::Audio,
                FrameContractStage::Presentation,
                FrameContractStage::PostInitialize,
                FrameContractStage::RecorderCommit,
                FrameContractStage::Pacing,
            ]
        );
        let present = stages
            .iter()
            .position(|stage| *stage == FrameContractStage::Presentation)
            .expect("graphical contract requires presentation");
        let post_initialize = stages
            .iter()
            .position(|stage| *stage == FrameContractStage::PostInitialize)
            .expect("graphical contract requires PostInitialize");
        assert!(present < post_initialize);
    }

    #[test]
    fn headless_execution_trace_keeps_post_initialize_before_frame_zero_commit() {
        let mut timeline = timeline_for_trace_test(FrameContract::Headless);
        timeline.begin_execution_trace(FrameContractStage::TimelineBegin);
        for stage in [
            FrameContractStage::PreTickCommands,
            FrameContractStage::Simulation,
            FrameContractStage::HostRpcAndTimelineCommit,
            FrameContractStage::ModalDrain,
        ] {
            timeline.trace(stage);
        }
        let mut dispatched = false;
        timeline.cross_post_initialize(|| dispatched = true);
        let mut frame = MissionFrame::new(0);
        timeline.finish_recording(&mut frame);
        timeline.trace(FrameContractStage::Presentation);
        timeline.trace(FrameContractStage::Pacing);

        assert!(dispatched);
        let stages = timeline.execution_trace();
        assert_eq!(
            stages,
            &[
                FrameContractStage::TimelineBegin,
                FrameContractStage::PreTickCommands,
                FrameContractStage::Simulation,
                FrameContractStage::HostRpcAndTimelineCommit,
                FrameContractStage::ModalDrain,
                FrameContractStage::PostInitialize,
                FrameContractStage::RecorderCommit,
                FrameContractStage::Presentation,
                FrameContractStage::Pacing,
            ]
        );
        let post_initialize = stages
            .iter()
            .position(|stage| *stage == FrameContractStage::PostInitialize)
            .expect("headless contract requires PostInitialize");
        let commit = stages
            .iter()
            .position(|stage| *stage == FrameContractStage::RecorderCommit)
            .expect("headless contract requires recorder commit");
        assert!(post_initialize < commit);
    }

    #[test]
    fn early_restart_trace_stops_before_simulation() {
        let mut trace = FrameExecutionTrace::default();
        trace.begin(FrameContractStage::NetworkIngress);
        trace.emit(FrameContractStage::TimelineBegin);
        trace.emit(FrameContractStage::EarlyRestart);
        assert_eq!(
            trace.stages,
            [
                FrameContractStage::NetworkIngress,
                FrameContractStage::TimelineBegin,
                FrameContractStage::EarlyRestart,
            ]
        );
    }

    #[test]
    fn paused_or_rewind_trace_marks_the_skipped_tick_boundary() {
        let mut trace = FrameExecutionTrace::default();
        trace.begin(FrameContractStage::TimelineBegin);
        trace.emit(FrameContractStage::PreTickCommands);
        trace.emit(FrameContractStage::PausedOrRewind);
        trace.emit(FrameContractStage::Simulation);
        trace.emit(FrameContractStage::ModalDrain);
        trace.emit(FrameContractStage::RecorderCommit);
        let paused = trace
            .stages
            .iter()
            .position(|stage| *stage == FrameContractStage::PausedOrRewind)
            .expect("paused trace requires a skip marker");
        let simulation = trace
            .stages
            .iter()
            .position(|stage| *stage == FrameContractStage::Simulation)
            .expect("paused trace still crosses the simulation boundary");
        assert!(paused < simulation);
    }

    #[test]
    fn terminal_tick_trace_records_exit_before_pacing() {
        let mut trace = FrameExecutionTrace::default();
        trace.begin(FrameContractStage::TimelineBegin);
        trace.emit(FrameContractStage::PreTickCommands);
        trace.emit(FrameContractStage::Simulation);
        trace.emit(FrameContractStage::HostRpcAndTimelineCommit);
        trace.emit(FrameContractStage::ModalDrain);
        trace.emit(FrameContractStage::RecorderCommit);
        trace.emit(FrameContractStage::Exit);
        trace.emit(FrameContractStage::Pacing);
        assert_eq!(
            trace.stages[trace.stages.len() - 2..],
            [FrameContractStage::Exit, FrameContractStage::Pacing]
        );
    }

    #[test]
    fn recorder_frame_finalization_is_exactly_once_when_recording_is_open() {
        let mut frame = MissionFrame::new(0);
        frame.open_recording();
        assert!(frame.close_recording());
        assert_eq!(frame.recorder_state, RecorderFrameState::Finished);
    }

    #[test]
    fn recorder_frame_finalization_is_exactly_once_when_recording_is_skipped() {
        let mut frame = MissionFrame::new(0);
        assert!(!frame.close_recording());
        assert_eq!(frame.recorder_state, RecorderFrameState::Finished);
    }

    #[test]
    fn real_recorder_finalization_seam_emits_the_commit_stage() {
        let mut timeline = timeline_for_trace_test(FrameContract::Graphical);
        timeline.begin_execution_trace(FrameContractStage::ModalDrain);
        let mut frame = MissionFrame::new(0);

        timeline.finish_recording(&mut frame);

        assert_eq!(
            timeline.execution_trace(),
            [
                FrameContractStage::ModalDrain,
                FrameContractStage::RecorderCommit,
            ]
        );
        assert_eq!(frame.recorder_state, RecorderFrameState::Finished);
    }

    #[test]
    #[should_panic(expected = "recorder frame finalized more than once")]
    fn recorder_frame_rejects_a_second_finalization() {
        let mut frame = MissionFrame::new(0);
        assert!(!frame.close_recording());
        frame.close_recording();
    }

    #[test]
    #[should_panic(expected = "recorder frame began more than once")]
    fn recorder_frame_rejects_a_second_begin() {
        let mut frame = MissionFrame::new(0);
        frame.open_recording();
        frame.open_recording();
    }

    #[test]
    fn frame_clock_preserves_original_25_hz_and_slow_motion_cadence() {
        let mut clock = FrameClock::new();
        clock.begin(1_000);
        let normal = FramePacing {
            fast_forward_requested: false,
            headless: false,
            engine_fast_forward: false,
            slow_motion: false,
            host_deadline_ms: None,
        };
        assert_eq!(clock.plan(1_015, normal), 25);
        assert_eq!(
            clock.plan(
                1_015,
                FramePacing {
                    slow_motion: true,
                    ..normal
                }
            ),
            385
        );
    }

    #[test]
    fn fast_paths_and_host_deadline_override_local_pacing() {
        let mut clock = FrameClock::new();
        clock.begin(5_000);
        let normal = FramePacing {
            fast_forward_requested: false,
            headless: false,
            engine_fast_forward: false,
            slow_motion: false,
            host_deadline_ms: None,
        };
        assert_eq!(
            clock.plan(
                5_010,
                FramePacing {
                    headless: true,
                    ..normal
                }
            ),
            0
        );
        assert_eq!(
            clock.plan(
                5_010,
                FramePacing {
                    host_deadline_ms: Some(5_023),
                    ..normal
                }
            ),
            13
        );
    }

    #[test]
    fn explicit_frame_phases_advance_in_order() {
        let mut phase = MissionPhase::Input;
        transition_phase(&mut phase, MissionPhase::Input, MissionPhase::Simulation);
        transition_phase(
            &mut phase,
            MissionPhase::Simulation,
            MissionPhase::Bookkeeping,
        );
        transition_phase(
            &mut phase,
            MissionPhase::Bookkeeping,
            MissionPhase::Presentation,
        );
        assert_eq!(phase, MissionPhase::Presentation);
    }

    #[test]
    #[should_panic(expected = "invalid mission frame phase transition")]
    fn explicit_frame_phases_reject_out_of_order_work() {
        let mut phase = MissionPhase::Input;
        transition_phase(
            &mut phase,
            MissionPhase::Simulation,
            MissionPhase::Bookkeeping,
        );
    }
}
