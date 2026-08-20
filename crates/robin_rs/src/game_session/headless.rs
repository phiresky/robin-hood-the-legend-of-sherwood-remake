//! Complete ownership and policy for a loaded true-headless mission.

use super::modal_state::ActiveModal;
use super::multiplayer::{drain_mission_network, host_scheduled_frame_deadline_ms};
use super::runtime::{
    FrameCommitPolicy, FrameContractStage, FrameOutcome, FramePacing, MissionRuntime, MissionWorld,
    TickPolicy,
};
use super::{dismiss_pending_modals, drain_steps, pop_matching_dismissal};
use crate::multiplayer::matchmaking::current_epoch_ms;
use robin_engine::game_operation::GameCode;
use robin_engine::player_command::{PlayerCommand, PlayerInput};
use serde::{Deserialize, Serialize};

/// Frontend behavior which intentionally differs from interactive play.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct HeadlessPolicy {
    pub(super) auto_dismiss_modals: bool,
    pub(super) exit_when_replay_finishes: bool,
}

impl HeadlessPolicy {
    pub(super) const fn replay_runner() -> Self {
        Self {
            auto_dismiss_modals: true,
            exit_when_replay_finishes: true,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum HeadlessFrameExit {
    Mission,
    ReplayComplete,
}

/// Result of one complete true-headless host iteration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct HeadlessFrameResult {
    pub(super) outcome: FrameOutcome,
    pub(super) exit: Option<HeadlessFrameExit>,
    pub(super) paused: bool,
}

/// Terminal result selected by the true-headless driver.
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
    /// policy here; consuming campaign return and async host pacing remain in
    /// the outer driver.
    pub(super) fn run_frame(&mut self, args: &crate::main_entry::CliArgs) -> HeadlessFrameResult {
        let profiling = super::frame_perf::enabled();
        let total_start = super::frame_perf::start(profiling);
        let frame_started_at_ms = crate::window::process_uptime_ms();
        let net_drain = {
            let MissionRuntime {
                world, timeline, ..
            } = &mut self.runtime;
            drain_mission_network(
                timeline,
                &mut world.host,
                &mut world.manager,
                &world.assets,
                true,
                current_epoch_ms(),
            )
        };
        let network_paused = net_drain.pause_simulation;
        let tick_paused = self.runtime.control.manual_pause || network_paused;
        let net_inputs = net_drain.inputs;
        if !net_inputs.is_empty() {
            self.runtime.world.manager.engine.apply_commands(
                &mut self.runtime.world.host.frontend.engine_display,
                &mut self.runtime.world.host.frontend.input,
                &self.runtime.world.assets,
                &net_inputs,
            );
        }
        let mut frame = self.runtime.begin_frame(frame_started_at_ms);
        frame.commands.commands.extend(net_inputs);

        if !tick_paused
            && self
                .runtime
                .timeline
                .replay_player
                .as_ref()
                .is_some_and(|player| !player.is_finished())
        {
            self.runtime
                .inject_next_replay_frame(&mut frame)
                .unwrap_or_else(|error| panic!("headless replay boundary failed: {error}"));
        }

        if self.policy.auto_dismiss_modals {
            let _ = dismiss_pending_modals(&mut self.runtime.world.host);
        }
        self.runtime
            .timeline
            .trace(FrameContractStage::PreTickCommands);
        super::frame_prepare::process_pre_tick_state_hash(
            &mut self.runtime.timeline,
            &self.runtime.world.host,
            &self.runtime.world.manager,
        );
        let simulation_start = super::frame_perf::start(profiling);
        let tick_exit_code = self.runtime.run_tick(TickPolicy {
            skip_tick: tick_paused,
            paused: false,
        });
        super::frame_perf::record(super::frame_perf::Phase::Simulation, simulation_start);
        self.runtime.drain_host_rpc();
        self.drain_headless_modals_and_steps(&mut frame, !network_paused);
        self.runtime.timeline.trace(FrameContractStage::ModalDrain);

        // Original ordering differs here: graphical crosses this boundary
        // after presentation, but headless must do so before frame-zero
        // recorder commit. See the frame-contract tests in runtime.rs.
        self.runtime.run_post_initialize();
        let paused = tick_paused;
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
        self.runtime
            .timeline
            .trace(FrameContractStage::Presentation);
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
        if exit_code.is_some() {
            self.runtime.timeline.trace(FrameContractStage::Exit);
        }
        self.runtime.timeline.trace(FrameContractStage::Pacing);
        let host_deadline_ms = if self.runtime.world.host.transport.net.is_some()
            && self.runtime.world.host.transport.local_seat
                != robin_engine::player_command::PlayerId::HOST
        {
            host_scheduled_frame_deadline_ms(
                self.runtime.timeline.mp_host_frame_schedule,
                self.runtime.world.manager.sim_frame,
            )
        } else {
            None
        };
        let outcome = self.runtime.timeline.plan_frame_outcome(
            crate::window::process_uptime_ms(),
            FramePacing {
                fast_forward_requested: args.fast_forward,
                headless: true,
                engine_fast_forward: self.runtime.world.manager.engine.is_fast_forward(),
                slow_motion: self.runtime.world.host.slow_motion,
                host_deadline_ms,
            },
            exit_code,
        );
        if let FrameOutcome::Continue { sleep_ms } = outcome
            && let Some((hash_frame, hash)) = self.runtime.timeline.pending_mp_state_hash
            && let Some(net) = self.runtime.world.host.transport.net.as_ref()
            && self.runtime.world.host.transport.local_seat
                == robin_engine::player_command::PlayerId::HOST
        {
            net.publish_frame(self.runtime.world.manager.sim_frame);
            net.send_state_hash(
                hash_frame,
                hash,
                self.runtime.world.manager.sim_frame,
                sleep_ms,
            );
        }

        let result = HeadlessFrameResult {
            outcome,
            exit,
            paused,
        };
        super::frame_perf::record(super::frame_perf::Phase::Total, total_start);
        result
    }

    fn drain_headless_modals_and_steps(
        &mut self,
        frame: &mut super::runtime::MissionFrame,
        allow_timeline_steps: bool,
    ) {
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

        if host
            .effects
            .take_signal(crate::host::HostSignal::MissionStatePopup)
        {
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
                if let Some(net) = host.transport.net.as_ref() {
                    net.send_input(command.clone());
                } else {
                    manager.engine.apply_local_commands(
                        &mut host.frontend.engine_display,
                        &mut host.frontend.input,
                        assets,
                        std::slice::from_ref(&command),
                    );
                }
                frame
                    .commands
                    .push(PlayerInput::new(host.transport.local_seat, command));
            }
        }

        let mut active_modal: Option<ActiveModal> = None;
        if allow_timeline_steps {
            drain_steps(
                manager,
                host,
                assets,
                dev,
                game,
                &mut timeline.rewind_buffer,
                &mut timeline.rollback_checker,
                &mut timeline.replay_player,
                &mut timeline.playback_pinned_saves,
                &mut control.manual_pause,
                &mut active_modal,
            );
        }
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
            self.runtime.timeline.finish_recording(frame);
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
        timeline.record_commands(frame, true);
        timeline.finish_recording(frame);
        world.manager.sim_frame += 1;
        if let Some(net) = world.host.transport.net.as_ref()
            && world.host.transport.local_seat == robin_engine::player_command::PlayerId::HOST
        {
            net.set_initial_snapshot(world.manager.sim_frame, &world.manager.engine);
        }
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
    }
}
