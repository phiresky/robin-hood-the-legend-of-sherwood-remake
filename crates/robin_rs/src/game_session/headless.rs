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
        // Network ingress may adopt/rewind whole state, but due commands are
        // inputs to the resulting frame boundary. Capture before applying
        // them so rollback replay does not start post-command and apply the
        // journaled inputs a second time.
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
        let tick_exit_code = self.runtime.run_tick(
            TickPolicy {
                skip_tick: tick_paused,
                paused: false,
            },
            &mut frame,
        );
        super::frame_perf::record(super::frame_perf::Phase::Simulation, simulation_start);
        self.runtime.drain_host_rpc(&mut frame);
        self.drain_headless_modals(&mut frame);
        self.runtime.timeline.trace(FrameContractStage::ModalDrain);

        // Original ordering differs here: graphical crosses this boundary
        // after presentation, but headless must do so before frame-zero
        // recorder commit and before a debugger step can advance frame one.
        // See the frame-contract tests in runtime.rs.
        self.runtime.run_post_initialize(&frame);
        let paused = tick_paused;
        let timeline_advances = frame.timeline_advances(!paused);
        self.commit_simulation_history(&frame, timeline_advances);
        self.drain_headless_steps(!network_paused);
        self.finish_frame_recording(&mut frame, timeline_advances);

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

    fn drain_headless_modals(&mut self, frame: &mut super::runtime::MissionFrame) {
        let MissionRuntime { world, .. } = &mut self.runtime;
        let MissionWorld { host, .. } = world;

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
                }
                frame
                    .post_commands
                    .push(PlayerInput::new(host.transport.local_seat, command));
            }
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

    /// Commit the outer frame before any debugger-driven forward ticks reuse
    /// the rewind/checker begin/end lifecycle. Graphical already has this
    /// ordering; headless must not leave its pending frame open across steps.
    fn commit_simulation_history(
        &mut self,
        frame: &super::runtime::MissionFrame,
        timeline_advances: bool,
    ) {
        if !timeline_advances {
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
        world.manager.sim_frame += 1;
        if let Some(net) = world.host.transport.net.as_ref()
            && world.host.transport.local_seat == robin_engine::player_command::PlayerId::HOST
        {
            net.set_initial_snapshot(world.manager.sim_frame, &world.manager.engine);
        }
    }

    fn drain_headless_steps(&mut self, allow_timeline_steps: bool) {
        if !allow_timeline_steps {
            return;
        }
        let MissionRuntime {
            world,
            timeline,
            control,
        } = &mut self.runtime;
        let mut active_modal: Option<ActiveModal> = None;
        drain_steps(
            &mut world.manager,
            &mut world.host,
            &world.assets,
            &mut world.dev,
            &mut world.game,
            &mut timeline.rewind_buffer,
            &mut timeline.rollback_checker,
            &mut timeline.replay_player,
            &mut timeline.playback_pinned_saves,
            &mut control.manual_pause,
            &mut active_modal,
        );
    }

    fn finish_frame_recording(
        &mut self,
        frame: &mut super::runtime::MissionFrame,
        timeline_advances: bool,
    ) {
        if self.runtime.timeline.replay_recorder.is_some() {
            let timeline_before = self
                .runtime
                .world
                .manager
                .sim_frame
                .saturating_sub(u32::from(timeline_advances));
            self.runtime
                .timeline
                .begin_recording(frame, true, timeline_before);
        }
        self.runtime
            .timeline
            .finish_recording(frame, timeline_advances);
    }
}

#[cfg(test)]
mod tests {
    use super::{HeadlessMission, HeadlessPolicy};
    use crate::game::Game;
    use crate::game_session::replay_init::ReplayAndRollback;
    use crate::game_session::runtime::{
        FrameContract, MissionControl, MissionRuntime, MissionWorld, TimelineRuntime,
    };
    use crate::host::Host;
    use crate::multiplayer::{NetChannels, NetEvent};
    use crate::rewind::RewindBuffer;
    use robin_engine::campaign::Campaign;
    use robin_engine::engine::{DevState, Engine, LevelAssets};
    use robin_engine::engine_manager::EngineManager;
    use robin_engine::player_command::{PlayerCommand, PlayerId, PlayerInput};
    use std::sync::Arc;

    #[test]
    fn replay_runner_policy_keeps_current_headless_contract_explicit() {
        let policy = HeadlessPolicy::replay_runner();

        assert!(policy.auto_dismiss_modals);
        assert!(policy.exit_when_replay_finishes);
    }

    #[test]
    fn due_network_commands_are_applied_after_the_headless_checkpoint() {
        let mut level_assets = LevelAssets::default();
        let engine = Engine::new_for_test(640.0, 480.0, Campaign::default(), &mut level_assets)
            .expect("fixture engine");
        let initial_hash = robin_engine::replay::state_hash(&engine);
        let assets = Arc::new(level_assets);
        let (channels, incoming, _outgoing, _, _) = NetChannels::new();
        let mut host = Host::scratch(640.0, 480.0);
        host.transport.local_seat = PlayerId::HOST;
        host.transport.net = Some(channels);
        incoming
            .send(NetEvent::Input {
                server_frame: 0,
                origin_frame: 0,
                target_frame: 0,
                input: PlayerInput::new(
                    PlayerId(2),
                    PlayerCommand::SetAmountOfSpeaking { amount: 9 },
                ),
            })
            .expect("queue current-frame network command");
        let manager = EngineManager::new(engine, PlayerId::HOST);
        let timeline = TimelineRuntime::new(
            ReplayAndRollback {
                recorder: None,
                player: None,
                rollback_checker: None,
                rewind_buffer: RewindBuffer::new(),
                start_paused: false,
            },
            FrameContract::Headless,
            false,
            true,
        );
        let control = MissionControl::new(false, manager.engine.weather().night_color);
        let runtime = MissionRuntime::new(
            MissionWorld::new(
                host,
                Game::default(),
                manager,
                Arc::clone(&assets),
                DevState::default(),
            ),
            timeline,
            control,
        );
        let mut mission = HeadlessMission {
            runtime,
            policy: HeadlessPolicy::replay_runner(),
        };

        mission.run_frame(&crate::main_entry::CliArgs::default());

        let checkpoint = mission
            .runtime
            .timeline
            .rewind_buffer
            .rewind_to(&assets, 0)
            .expect("frame-0 pre-command checkpoint");
        assert_eq!(robin_engine::replay::state_hash(&checkpoint), initial_hash);
        assert_eq!(
            mission
                .runtime
                .timeline
                .rewind_buffer
                .commands_for(0)
                .map(|commands| commands.len()),
            Some(1)
        );
    }

    #[test]
    fn headless_outer_frame_commits_before_forward_step_reuses_timeline_lifecycle() {
        let mut level_assets = LevelAssets::default();
        let engine = Engine::new_for_test(640.0, 480.0, Campaign::default(), &mut level_assets)
            .expect("fixture engine");
        let assets = Arc::new(level_assets);
        let host = Host::scratch(640.0, 480.0);
        let manager = EngineManager::new(engine, PlayerId::HOST);
        let timeline = TimelineRuntime::new(
            ReplayAndRollback {
                recorder: None,
                player: None,
                rollback_checker: None,
                rewind_buffer: RewindBuffer::new(),
                start_paused: false,
            },
            FrameContract::Headless,
            false,
            true,
        );
        let control = MissionControl::new(false, manager.engine.weather().night_color);
        let runtime = MissionRuntime::new(
            MissionWorld::new(
                host,
                Game::default(),
                manager,
                Arc::clone(&assets),
                DevState::default(),
            ),
            timeline,
            control,
        );
        let mut mission = HeadlessMission {
            runtime,
            policy: HeadlessPolicy::replay_runner(),
        };
        let mut frame = mission.runtime.begin_frame(0);
        mission.runtime.run_tick(
            super::TickPolicy {
                skip_tick: false,
                paused: false,
            },
            &mut frame,
        );

        // This is the ordering enforced by run_frame: publish the normal
        // frame before run_forward_ticks opens and commits its own frame.
        mission.commit_simulation_history(&frame, false);
        let advanced = {
            let MissionRuntime {
                world, timeline, ..
            } = &mut mission.runtime;
            crate::game_session::tick::run_forward_ticks(
                &mut world.manager,
                &mut world.host,
                &world.assets,
                &mut world.dev,
                &mut world.game,
                &mut timeline.rewind_buffer,
                &mut timeline.rollback_checker,
                &mut timeline.replay_player,
                &mut timeline.playback_pinned_saves,
                1,
            )
            .expect("forward step after outer commit")
            .0
        };

        assert_eq!(advanced, 1);
        assert_eq!(mission.runtime.world.manager.sim_frame, 2);
        assert!(
            mission
                .runtime
                .timeline
                .rewind_buffer
                .commands_for(0)
                .is_some()
        );
        assert!(
            mission
                .runtime
                .timeline
                .rewind_buffer
                .commands_for(1)
                .is_some()
        );
    }
}
