//! Owning state and phase boundaries for one loaded mission.
//!
//! The native and headless loops still differ in input, modal, and
//! presentation work, but they share the same deterministic frame
//! bookkeeping through [`MissionRuntime`].

use super::multiplayer::MultiplayerRollbackTelemetry;
use super::replay_init::ReplayAndRollback;
use crate::game_operation::GameCode;
use crate::player_command::PlayerInput;
use crate::replay::{ReplayPlayer, ReplayRecorder};
use crate::rewind::RewindBuffer;
use crate::rollback_checker::RollbackChecker;
use crate::sim_timeline::{
    CheckpointPolicy, RECENT_TIMELINE_HISTORY_FRAMES, RetentionPolicy, SnapshotHistory,
};
use robin_engine::engine::{Engine, LevelAssets};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Coarse stages that both mission-loop implementations pass through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum MissionPhase {
    Input,
    Simulation,
    Bookkeeping,
    Presentation,
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
pub(super) struct MissionRuntime {
    phase: MissionPhase,
    clock: FrameClock,

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

impl MissionRuntime {
    pub(super) fn new(
        replay: ReplayAndRollback,
        wait_for_multiplayer_start: bool,
        local_is_host: bool,
    ) -> Self {
        Self {
            phase: MissionPhase::Presentation,
            clock: FrameClock::new(),
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

    pub(super) fn record_commands(
        &mut self,
        recorder_hash: Option<u64>,
        commands: &[PlayerInput],
        enabled: bool,
    ) {
        let Some(recorder) = self.replay_recorder.as_mut().filter(|_| enabled) else {
            return;
        };
        if let Some(hash) = recorder_hash {
            recorder.write_hash(recorder.frame_number(), hash);
        }
        for command in commands {
            recorder.push(command.clone());
        }
    }

    pub(super) fn finish_recording<I, T>(&mut self, commands: I, enabled: bool)
    where
        I: IntoIterator<Item = T>,
        T: Into<PlayerInput>,
    {
        let Some(recorder) = self.replay_recorder.as_mut().filter(|_| enabled) else {
            return;
        };
        for command in commands {
            recorder.push(command.into());
        }
        recorder.end_frame();
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
