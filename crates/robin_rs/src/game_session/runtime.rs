//! Owning state and phase boundaries for one loaded mission.
//!
//! The native and headless loops still differ in input, modal, and
//! presentation work, but they share the same deterministic frame
//! bookkeeping through [`TimelineRuntime`].

use super::multiplayer::MultiplayerRollbackTelemetry;
use super::replay_init::ReplayAndRollback;
use crate::Host;
use crate::game::Game;
use crate::game_operation::GameCode;
use crate::player_command::{FrameCommands, PlayerCommand};
use crate::replay::{ReplayPlayer, ReplayRecorder};
use crate::rewind::RewindBuffer;
use crate::rollback_checker::RollbackChecker;
use crate::sim_timeline::{
    CheckpointPolicy, RECENT_TIMELINE_HISTORY_FRAMES, RetentionPolicy, SnapshotHistory,
};
use robin_engine::engine::{DevState, Engine, LevelAssets};
use robin_engine::engine_manager::EngineManager;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

/// Common owned state for one loaded mission.
///
/// This deliberately stops at the simulation/host boundary. Renderer, input,
/// audio backend, and modal resources are owned by the graphical driver's
/// `InteractiveFrontend`. None of these process resources is serializable;
/// deterministic persistence remains the Engine snapshot.
pub(super) struct MissionWorld {
    // TODO(refactor): make these fields private once frame methods move onto
    // their focused owners. Wave 1 borrows them directly to keep loop order
    // and behavior unchanged.
    pub(super) host: Host,
    pub(super) game: Game,
    pub(super) manager: EngineManager,
    pub(super) assets: Arc<LevelAssets>,
    pub(super) dev: DevState,
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
}

/// Mission-lifetime host controls which are neither deterministic Engine state
/// nor timeline resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct MissionControl {
    pub(super) manual_pause: bool,
    pub(super) step_forward_repeat_at_ms: Option<u32>,
    pub(super) step_back_repeat_at_ms: Option<u32>,
    pub(super) last_shadow_color: u16,
}

impl MissionControl {
    pub(super) fn new(manual_pause: bool, last_shadow_color: u16) -> Self {
        Self {
            manual_pause,
            step_forward_repeat_at_ms: None,
            step_back_repeat_at_ms: None,
            last_shadow_color,
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
    pub(super) commands: FrameCommands,
    pub(super) modal_dismissals: Vec<PlayerCommand>,
    pub(super) replay_modal_dismissals: VecDeque<PlayerCommand>,
    pub(super) recorder_hash: Option<u64>,
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

impl MissionFrame {
    pub(super) fn new(started_at_ms: u32) -> Self {
        Self {
            started_at_ms,
            commands: FrameCommands::new(),
            modal_dismissals: Vec::new(),
            replay_modal_dismissals: VecDeque::new(),
            recorder_hash: None,
            recorder_state: RecorderFrameState::Inactive,
        }
    }

    /// Split one admitted replay frame into simulation commands and modal
    /// acknowledgements, then apply only the simulation commands.
    pub(super) fn inject_replay_commands(
        &mut self,
        player: &mut ReplayPlayer,
        host: &mut Host,
        manager: &mut EngineManager,
        assets: &LevelAssets,
    ) {
        assert!(
            !player.is_finished(),
            "replay injection requested after the replay finished"
        );
        let replay_commands = player.next_frame();
        let mut simulation_commands = Vec::with_capacity(replay_commands.len());
        for command in replay_commands {
            if matches!(command.command, PlayerCommand::ModalDismiss { .. }) {
                self.replay_modal_dismissals
                    .push_back(command.command.clone());
            } else {
                simulation_commands.push(command.clone());
            }
        }
        manager.engine.apply_commands(
            &mut host.engine_display,
            &mut host.input,
            assets,
            &simulation_commands,
        );
        self.commands = FrameCommands::new();
        self.commands.commands = simulation_commands;
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
    pub(super) world: MissionWorld,
    pub(super) timeline: TimelineRuntime,
    pub(super) control: MissionControl,
}

impl MissionRuntime {
    pub(super) fn new(
        world: MissionWorld,
        timeline: TimelineRuntime,
        control: MissionControl,
    ) -> Self {
        Self {
            world,
            timeline,
            control,
        }
    }

    /// Open one host frame at the deterministic pre-command boundary.
    ///
    /// Network ingress remains a driver concern and must run before this
    /// method. That ordering is observable for late multiplayer inputs.
    pub(super) fn begin_frame(&mut self, now_ms: u32) -> MissionFrame {
        self.timeline.reset_execution_trace();
        let mut frame = MissionFrame::new(now_ms);
        self.timeline.open_frame(
            &mut frame,
            self.world.manager.sim_frame,
            &self.world.manager.engine,
            &self.world.assets,
        );
        frame
    }

    /// Apply the next replay frame, separating modal acknowledgements from
    /// deterministic engine commands.
    ///
    /// Callers decide whether playback is currently allowed (for example,
    /// the graphical driver freezes playback while paused). Once admitted,
    /// both drivers use this exact command injection contract.
    pub(super) fn inject_next_replay_frame(&mut self, frame: &mut MissionFrame) {
        let Some(player) = self.timeline.replay_player.as_mut() else {
            return;
        };
        frame.inject_replay_commands(
            player,
            &mut self.world.host,
            &mut self.world.manager,
            &self.world.assets,
        );
    }

    /// Advance the common simulation phase while preserving each driver's
    /// explicit pause/rewind policy.
    pub(super) fn run_tick(&mut self, policy: TickPolicy) -> Option<GameCode> {
        if policy.skip_tick || policy.paused {
            self.timeline.trace(FrameContractStage::PausedOrRewind);
        }
        self.timeline.trace(FrameContractStage::Simulation);
        self.timeline.run_simulation(|| {
            if policy.skip_tick {
                return None;
            }
            let mut display = std::mem::take(&mut self.world.host.engine_display);
            let result = self.world.game.run_engine_tick(
                &mut self.world.host,
                &mut display,
                self.world.assets.as_ref(),
                &mut self.world.manager.engine,
                &mut self.world.dev,
                false,
                policy.paused,
            );
            self.world.host.engine_display = display;
            result
        })
    }

    /// Drain host RPC requests at the shared post-tick boundary.
    pub(super) fn drain_host_rpc(&mut self) {
        let net = self.world.host.net.take();
        crate::http_server::drain_global(
            &mut self.world.manager,
            &mut self.world.host,
            &self.world.assets,
            net.as_ref(),
        );
        self.world.host.net = net;
        self.timeline
            .trace(FrameContractStage::HostRpcAndTimelineCommit);
    }

    /// Cross the deferred Original `PostInitialize` boundary.
    ///
    /// Drivers intentionally choose when to call this: headless does so
    /// before frame-zero recorder commit, graphical does so after refresh.
    pub(super) fn run_post_initialize(&mut self) -> bool {
        let mut display = std::mem::take(&mut self.world.host.engine_display);
        let initialized = crate::sim_timeline::run_post_initialize_stage(
            &mut self.world.host,
            &mut display,
            &self.world.assets,
            &mut self.world.manager.engine,
            &mut self.world.dev,
        );
        self.world.host.engine_display = display;
        self.timeline.trace(FrameContractStage::PostInitialize);
        initialized
    }
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
            // Original provenance: original-code/RHgame.cpp,
            // RHGame::GameLoop waits for `40 * 10` while the messenger
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

/// Mission-lifetime replay, rollback, network, and frame-clock state.
///
/// This is deliberately not serializable: recorder writers, rollback
/// workers, and live network diagnostics are process resources, not game
/// state. Persisting them would create a fake/default runtime on restore.
pub(super) struct TimelineRuntime {
    contract: FrameContract,
    phase: MissionPhase,
    clock: FrameClock,
    execution_trace: FrameExecutionTrace,

    pub(super) replay_recorder: Option<ReplayRecorder>,
    pub(super) replay_player: Option<ReplayPlayer>,
    pub(super) rollback_checker: Option<RollbackChecker>,
    pub(super) rewind_buffer: RewindBuffer,
    pub(super) start_paused: bool,
    pub(super) replay_finished_logged: bool,

    pub(super) peer_hashes: BTreeMap<u32, u64>,
    pub(super) recent_timeline_history: SnapshotHistory,
    pub(super) mp_start_gate: Option<u64>,
    pub(super) mp_waiting_for_initial_snapshot: bool,
    pub(super) mp_waiting_for_begin_sim: bool,
    pub(super) mp_host_frame_schedule: Option<(u32, u32)>,
    pub(super) last_mp_rollback: Option<MultiplayerRollbackTelemetry>,
    pub(super) last_mp_clock_ahead_log_ms: u32,
    pub(super) last_mp_sleep_correction_log_ms: u32,
    pub(super) last_mp_state_hash_frame: Option<u32>,
    pub(super) pending_mp_state_hash: Option<(u32, u64)>,
}

impl TimelineRuntime {
    pub(super) fn new(
        replay: ReplayAndRollback,
        contract: FrameContract,
        wait_for_multiplayer_start: bool,
        local_is_host: bool,
    ) -> Self {
        Self {
            contract,
            phase: MissionPhase::Presentation,
            clock: FrameClock::new(),
            execution_trace: FrameExecutionTrace::default(),
            replay_recorder: replay.recorder,
            replay_player: replay.player,
            rollback_checker: replay.rollback_checker,
            rewind_buffer: replay.rewind_buffer,
            start_paused: replay.start_paused,
            replay_finished_logged: false,
            peer_hashes: BTreeMap::new(),
            recent_timeline_history: SnapshotHistory::new(
                CheckpointPolicy::EveryFrame,
                RetentionPolicy::Latest {
                    capacity: RECENT_TIMELINE_HISTORY_FRAMES,
                },
            ),
            mp_start_gate: None,
            mp_waiting_for_initial_snapshot: wait_for_multiplayer_start && !local_is_host,
            mp_waiting_for_begin_sim: wait_for_multiplayer_start,
            mp_host_frame_schedule: None,
            last_mp_rollback: None,
            last_mp_clock_ahead_log_ms: 0,
            last_mp_sleep_correction_log_ms: 0,
            last_mp_state_hash_frame: None,
            pending_mp_state_hash: None,
        }
    }

    pub(super) fn initially_paused(&self) -> bool {
        self.start_paused || self.mp_waiting_for_initial_snapshot || self.mp_waiting_for_begin_sim
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
        sim_frame: u32,
        engine: &Engine,
        assets: &LevelAssets,
    ) {
        frame.recorder_hash = self.begin_frame(frame.started_at_ms, sim_frame, engine, assets);
        self.trace(FrameContractStage::TimelineBegin);
    }

    /// Start the input phase and capture the shared pre-command snapshots.
    ///
    /// `begin_frame` intentionally permits replacing any previous phase:
    /// native event handlers can restart the outer loop before simulation,
    /// abandoning that host frame exactly as the old loop did.
    pub(super) fn begin_frame(
        &mut self,
        now_ms: u32,
        sim_frame: u32,
        engine: &Engine,
        assets: &LevelAssets,
    ) -> Option<u64> {
        self.phase = MissionPhase::Input;
        self.clock.begin(now_ms);
        self.pending_mp_state_hash = None;
        self.rewind_buffer.begin_frame(sim_frame, engine, assets);
        if let Some(checker) = self.rollback_checker.as_mut() {
            checker.begin_frame(sim_frame, engine);
        }

        let recorder_hash = self.replay_recorder.as_ref().and_then(|recorder| {
            recorder
                .frame_number()
                .is_multiple_of(25)
                .then(|| crate::replay::state_hash(engine))
        });
        if let Some(player) = self.replay_player.as_ref()
            && !player.is_finished()
        {
            let frame = player.current_frame();
            let is_terminal_frame = frame + 1 >= player.total_frames();
            if !is_terminal_frame && let Some(expected) = player.hash_for_frame(frame) {
                let actual = crate::replay::state_hash(engine);
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

    /// Commit rollback and rewind history for a frame which advanced.
    ///
    /// Pause/rewind admission and recorder ordering remain driver concerns.
    /// Once admitted, rollback verification and rewind history are one
    /// timeline step. Frame-number advancement intentionally stays in each
    /// driver because headless advances after recorder commit while graphical
    /// advances before modal/presentation work.
    pub(super) fn commit_simulation_history(
        &mut self,
        host: &mut Host,
        manager: &mut EngineManager,
        frame: &MissionFrame,
        policy: FrameCommitPolicy,
    ) {
        assert_eq!(
            self.phase,
            MissionPhase::Bookkeeping,
            "simulation commit requested outside bookkeeping phase"
        );
        if let Some(checker) = self.rollback_checker.as_mut() {
            checker.end_frame(host, frame.commands.commands.clone(), &manager.engine);
        }
        if policy.store_rewind_commands {
            self.rewind_buffer
                .end_frame(frame.commands.commands.clone());
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

    pub(super) fn record_commands(&mut self, frame: &mut MissionFrame, enabled: bool) {
        let Some(recorder) = self.replay_recorder.as_mut().filter(|_| enabled) else {
            return;
        };
        frame.open_recording();
        if let Some(hash) = frame.recorder_hash {
            recorder.write_hash(recorder.frame_number(), hash);
        }
        for command in &frame.commands.commands {
            recorder.push(command.clone());
        }
    }

    /// Close the recorder frame opened by [`Self::record_commands`].
    ///
    /// Normal presentation and emergency modal exits call this same owner
    /// method. Inactive frames (rewind, buffered replay, or no recorder)
    /// do not call `end_frame`, but they still cross this finalization boundary;
    /// closing an already-finished frame is a lifecycle bug.
    pub(super) fn finish_recording(&mut self, frame: &mut MissionFrame) {
        if frame.close_recording() {
            let recorder = self
                .replay_recorder
                .as_mut()
                .expect("open recorder frame lost its recorder owner");
            for command in std::mem::take(&mut frame.modal_dismissals) {
                recorder.push(command);
            }
            recorder.end_frame();
        }
        self.trace(FrameContractStage::RecorderCommit);
    }

    fn transition(&mut self, expected: MissionPhase, next: MissionPhase) {
        transition_phase(&mut self.phase, expected, next);
    }
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

    #[test]
    fn mission_control_round_trips_without_defaulting_process_state() {
        let control = MissionControl {
            manual_pause: true,
            step_forward_repeat_at_ms: Some(120),
            step_back_repeat_at_ms: Some(240),
            last_shadow_color: 0x1234,
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
    fn graphical_execution_trace_keeps_original_refresh_sound_post_initialize_tail() {
        let mut trace = FrameExecutionTrace::default();
        trace.begin(FrameContractStage::NetworkIngress);
        for stage in [
            FrameContractStage::TimelineBegin,
            FrameContractStage::InputAndMenus,
            FrameContractStage::OperationAndSave,
            FrameContractStage::SecondNetworkDrain,
            FrameContractStage::PreTickCommands,
            FrameContractStage::Simulation,
            FrameContractStage::HostRpcAndTimelineCommit,
            FrameContractStage::ModalDrain,
            FrameContractStage::RecorderCommit,
            FrameContractStage::AppEffects,
            FrameContractStage::Audio,
            FrameContractStage::Presentation,
            FrameContractStage::PostInitialize,
            FrameContractStage::Pacing,
        ] {
            trace.emit(stage);
        }
        let stages = &trace.stages;
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
                FrameContractStage::RecorderCommit,
                FrameContractStage::AppEffects,
                FrameContractStage::Audio,
                FrameContractStage::Presentation,
                FrameContractStage::PostInitialize,
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
        let mut trace = FrameExecutionTrace::default();
        trace.begin(FrameContractStage::TimelineBegin);
        for stage in [
            FrameContractStage::PreTickCommands,
            FrameContractStage::Simulation,
            FrameContractStage::HostRpcAndTimelineCommit,
            FrameContractStage::ModalDrain,
            FrameContractStage::PostInitialize,
            FrameContractStage::RecorderCommit,
            FrameContractStage::Presentation,
            FrameContractStage::Pacing,
        ] {
            trace.emit(stage);
        }
        let stages = &trace.stages;
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
