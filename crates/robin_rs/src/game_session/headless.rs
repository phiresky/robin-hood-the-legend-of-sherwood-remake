//! Complete ownership and policy for a loaded true-headless mission.

use super::modal_state::ActiveModal;
use super::runtime::{
    FrameCommitPolicy, FrameOutcome, FramePacing, MissionRuntime, MissionWorld, TickPolicy,
};
use super::{dismiss_pending_modals, drain_steps, pop_matching_dismissal};
use crate::game_operation::GameCode;
use crate::player_command::{PlayerCommand, PlayerInput};
use serde::{Deserialize, Serialize};

/// Frontend behavior which intentionally differs from interactive play.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct HeadlessPolicy {
    pub(super) auto_dismiss_modals: bool,
    pub(super) exit_when_replay_finishes: bool,
    pub(super) wait_for_multiplayer_start: bool,
}

impl HeadlessPolicy {
    pub(super) const fn replay_runner() -> Self {
        Self {
            auto_dismiss_modals: true,
            exit_when_replay_finishes: true,
            // TODO(parity): Teach the true-headless driver to drain the
            // multiplayer start barrier before enabling this.
            wait_for_multiplayer_start: false,
        }
    }
}

/// Complete process owner returned by true-headless bootstrap.
///
/// It deliberately does not implement serde: timeline writers, network
/// handles, and rollback workers are process resources.
pub(super) struct HeadlessMission {
    pub(super) runtime: MissionRuntime,
    pub(super) policy: HeadlessPolicy,
}

/// Why the headless driver requested an outer-session exit.
///
/// Campaign return ownership remains in `bootstrap::BuiltHeadlessMission`'s
/// private lease; this value only supplies the required restore context
/// without centralizing exit policy in the deterministic runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum HeadlessFrameExit {
    Mission,
    ReplayComplete,
}

impl HeadlessFrameExit {
    pub(super) const fn campaign_restore_context(self) -> &'static str {
        match self {
            Self::Mission => "headless mission exit",
            Self::ReplayComplete => "headless replay completion",
        }
    }
}

/// Result of one complete true-headless host iteration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct HeadlessFrameResult {
    pub(super) outcome: FrameOutcome,
    pub(super) exit: Option<HeadlessFrameExit>,
    pub(super) paused: bool,
}

/// Terminal result selected by the true-headless driver. The exit reason is
/// retained until the outer owner returns the campaign lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct HeadlessMissionOutcome {
    pub(super) code: GameCode,
    pub(super) exit: HeadlessFrameExit,
}

impl HeadlessMission {
    /// Run the complete true-headless mission without constructing graphical,
    /// input-device, menu, or native-audio shims.
    pub(super) async fn run(
        &mut self,
        args: &crate::main_entry::CliArgs,
    ) -> HeadlessMissionOutcome {
        loop {
            let frame_result = self.run_frame(args);
            match frame_result.outcome {
                FrameOutcome::Exit(code) => {
                    return HeadlessMissionOutcome {
                        code,
                        exit: frame_result
                            .exit
                            .expect("runtime exit must have a campaign finalization context"),
                    };
                }
                FrameOutcome::Continue { sleep_ms } if frame_result.paused => {
                    crate::window::sleep_ms(u64::from(sleep_ms.max(10))).await;
                }
                FrameOutcome::Continue { sleep_ms: 0 } => {
                    crate::window::yield_to_runtime().await;
                }
                FrameOutcome::Continue { sleep_ms } => {
                    crate::window::sleep_ms(u64::from(sleep_ms)).await;
                }
            }
        }
    }

    /// Run one complete true-headless frame.
    ///
    /// The method is deliberately a short ordered list of the headless
    /// contract. Modal automation and replay-completion remain explicit
    /// policy here; campaign restoration and async host pacing remain in the
    /// outer driver.
    pub(super) fn run_frame(&mut self, args: &crate::main_entry::CliArgs) -> HeadlessFrameResult {
        let mut frame = self.runtime.begin_frame(crate::window::process_uptime_ms());

        if self
            .runtime
            .timeline
            .replay_player
            .as_ref()
            .is_some_and(|player| !player.is_finished())
        {
            self.runtime.inject_next_replay_frame(&mut frame);
        }

        if self.policy.auto_dismiss_modals {
            let _ = dismiss_pending_modals(&mut self.runtime.world.host);
        }
        let tick_paused = self.runtime.control.manual_pause;
        let tick_exit_code = self.runtime.run_tick(TickPolicy {
            skip_tick: tick_paused,
            paused: false,
        });
        self.runtime.drain_host_rpc();
        self.drain_headless_modals_and_steps(&mut frame);

        // Original ordering differs here: graphical crosses this boundary
        // after presentation, but headless must do so before frame-zero
        // recorder commit. See the frame-contract tests in runtime.rs.
        self.runtime.run_post_initialize();
        let paused = self.runtime.control.manual_pause;
        self.commit_frame(&mut frame, paused);

        let replay_finished = self
            .runtime
            .timeline
            .replay_player
            .as_ref()
            .is_some_and(|player| player.is_finished());
        if replay_finished {
            tracing::info!("headless replay finished");
        }

        self.runtime.timeline.begin_presentation();
        let (exit_code, exit) = if let Some(code) = tick_exit_code {
            (Some(code), Some(HeadlessFrameExit::Mission))
        } else if self.policy.exit_when_replay_finishes && replay_finished {
            (
                Some(GameCode::Quit),
                Some(HeadlessFrameExit::ReplayComplete),
            )
        } else {
            (None, None)
        };
        let outcome = self.runtime.timeline.plan_frame_outcome(
            crate::window::process_uptime_ms(),
            FramePacing {
                fast_forward_requested: args.fast_forward,
                headless: true,
                engine_fast_forward: self.runtime.world.manager.engine.is_fast_forward(),
                slow_motion: self.runtime.world.host.slow_motion,
                host_deadline_ms: None,
            },
            exit_code,
        );

        HeadlessFrameResult {
            outcome,
            exit,
            paused,
        }
    }

    fn drain_headless_modals_and_steps(&mut self, frame: &mut super::runtime::MissionFrame) {
        let MissionRuntime {
            world,
            timeline,
            control,
        } = &mut self.runtime;
        let MissionWorld {
            host,
            game,
            manager,
            assets,
            dev,
        } = world;

        if host.pending_mission_state_popup {
            host.pending_mission_state_popup = false;
            let kind = robin_engine::player_command::ModalKind::MissionState {
                kind: robin_engine::player_command::MissionStateModalKind::LeaveMissionNow,
            };
            let result = pop_matching_dismissal(&mut frame.replay_modal_dismissals, &kind)
                .unwrap_or(robin_engine::player_command::DialogResult::Completed);
            frame
                .modal_dismissals
                .push(PlayerCommand::ModalDismiss { kind, result });
            if result == robin_engine::player_command::DialogResult::Completed {
                let command = PlayerCommand::QuitMissionRequested;
                if let Some(net) = host.net.as_ref() {
                    net.send_input(command.clone());
                } else {
                    manager.engine.apply_local_commands(
                        &mut host.engine_display,
                        &mut host.input,
                        assets,
                        std::slice::from_ref(&command),
                    );
                }
                frame
                    .commands
                    .push(PlayerInput::new(host.local_seat, command));
            }
        }

        let mut active_modal: Option<ActiveModal> = None;
        drain_steps(
            manager,
            host,
            assets,
            dev,
            game,
            &mut timeline.rewind_buffer,
            &mut timeline.rollback_checker,
            &mut timeline.replay_player,
            &mut control.manual_pause,
            &mut active_modal,
        );
        let dismissed = if self.policy.auto_dismiss_modals {
            dismiss_pending_modals(host)
        } else {
            0
        };
        if dismissed > 0 {
            tracing::debug!(dismissed, "headless: auto-dismissed pending modal(s)");
        }
        if !frame.replay_modal_dismissals.is_empty() {
            tracing::debug!(
                "Replay headless: {} recorded ModalDismiss command(s) unused this frame",
                frame.replay_modal_dismissals.len()
            );
        }
    }

    fn commit_frame(&mut self, frame: &mut super::runtime::MissionFrame, paused: bool) {
        if paused {
            return;
        }
        let MissionRuntime {
            world, timeline, ..
        } = &mut self.runtime;
        timeline.commit_simulation_history(
            &mut world.host,
            &mut world.manager,
            frame,
            FrameCommitPolicy {
                store_rewind_commands: true,
            },
        );
        timeline.record_commands(frame.recorder_hash, &frame.commands.commands, true);
        timeline.finish_recording(std::mem::take(&mut frame.modal_dismissals), true);
        world.manager.sim_frame += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::HeadlessPolicy;

    #[test]
    fn replay_runner_policy_keeps_current_headless_contract_explicit() {
        let policy = HeadlessPolicy::replay_runner();

        assert!(policy.auto_dismiss_modals);
        assert!(policy.exit_when_replay_finishes);
        assert!(!policy.wait_for_multiplayer_start);
    }
}
