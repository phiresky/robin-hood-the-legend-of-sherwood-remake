//! Final network correction, replay/rewind admission, and pointer commands.

use super::*;
use crate::game_session::interactive::{MissionInput, MissionUi};
use crate::game_session::runtime::{FrameContractStage, TimelineRuntime};

/// Drain packets which arrived while the frame was processing local input.
/// This is the final network mutation boundary before state hashing and tick.
fn drain_pre_tick_network(
    runtime: &mut crate::game_session::runtime::TimelineRuntime,
    host: &mut Host,
    manager: &mut robin_engine::engine_manager::EngineManager,
    assets: &mut std::sync::Arc<robin_engine::engine::LevelAssets>,
    frame: &mut MissionFrame,
    mp_clock_pause: &mut bool,
    rewind_active: bool,
) {
    if host.transport.net().is_none() || rewind_active {
        return;
    }

    runtime.trace(FrameContractStage::SecondNetworkDrain);
    let drain = drain_mission_network(runtime, host, manager, assets, false, current_epoch_ms());
    if drain.rollback.is_some() {
        // Late input invalidates the capture opened before local input/UI.
        // Reconstruction returns to this same pre-tick frame; retain its
        // queued commands/facts but capture their corrected starting state.
        runtime.reopen_after_pre_tick_network_rollback(frame, &manager.engine, assets);
    }
    *mp_clock_pause |= drain.pause_simulation;
    frame.stage_commands().commands.extend(drain.inputs);
    if host.transport.local_seat() == engine_player_command::PlayerId::HOST
        && host.transport.reconnecting()
    {
        discard_abandoned_host_frame_inputs(frame);
    }
}

fn discard_abandoned_host_frame_inputs(frame: &mut MissionFrame) {
    if !frame.commands().is_empty() {
        tracing::warn!(
            count = frame.commands().len(),
            "multiplayer: discarded accumulated frame inputs after host snapshot resynchronization"
        );
        frame.discard_commands();
    }
}

/// Publish or verify the periodic multiplayer state hash after the second
/// network drain has made this frame's command set final.
pub(in crate::game_session) fn process_pre_tick_state_hash(
    runtime: &mut crate::game_session::runtime::TimelineRuntime,
    host: &Host,
    manager: &robin_engine::engine_manager::EngineManager,
) {
    if host.transport.net().is_none() {
        return;
    }
    let local_is_host = host.transport.local_seat() == engine_player_command::PlayerId::HOST;
    let hash_boundary = runtime
        .frame_number()
        .is_multiple_of(crate::multiplayer::STATE_HASH_INTERVAL);
    if local_is_host
        && hash_boundary
        && runtime.last_mp_state_hash_frame != Some(runtime.frame_number())
    {
        runtime.last_mp_state_hash_frame = Some(runtime.frame_number());
        let mp_hash_start = web_time::Instant::now();
        let live_hash_start = web_time::Instant::now();
        let local_hash = robin_engine::replay::state_hash(&manager.engine);
        let live_hash_us = live_hash_start.elapsed().as_micros();
        runtime.pending_mp_state_hash = Some((runtime.frame_number(), local_hash));
        tracing::debug!(
            frame = runtime.frame_number(),
            total_us = mp_hash_start.elapsed().as_micros(),
            live_hash_us,
            "multiplayer hash frame timing"
        );
    }
    if local_is_host {
        return;
    }
    if hash_boundary && !runtime.has_local_mp_hash(runtime.frame_number()) {
        runtime.remember_local_mp_hash(
            runtime.frame_number(),
            robin_engine::replay::state_hash(&manager.engine),
        );
    }
    // Network ingress (including rollback) precedes this boundary. Compare
    // delayed host hashes against their exact retained pre-tick frame, never
    // against the newer live engine merely because it is available.
    for (frame, host_hash, local_hash) in runtime.take_due_mp_hash_comparisons() {
        let Some(local_hash) = local_hash else {
            tracing::warn!(
                frame,
                local_frame = runtime.frame_number(),
                "multiplayer hash comparison missed: historical state expired or was invalidated by rollback"
            );
            continue;
        };
        if local_hash != host_hash {
            let last_rollback_path = runtime.last_mp_rollback.as_ref().map_or("none", |r| r.path);
            let last_rollback_earliest = runtime
                .last_mp_rollback
                .as_ref()
                .map_or(0, |r| r.earliest_frame);
            let last_rollback_target = runtime
                .last_mp_rollback
                .as_ref()
                .map_or(0, |r| r.target_frame);
            let last_rollback_replayed = runtime
                .last_mp_rollback
                .as_ref()
                .map_or(0, |r| r.replayed_frames);
            let last_rollback_total_us =
                runtime.last_mp_rollback.as_ref().map_or(0, |r| r.total_us);
            tracing::warn!(
                frame,
                local = format!("{local_hash:016x}"),
                host = format!("{host_hash:016x}"),
                host_schedule_frame = runtime.mp_host_frame_schedule.map(|(frame, _)| frame),
                pending_input_frames = runtime.pending_input_frame_count(),
                last_rollback_path,
                last_rollback_earliest,
                last_rollback_target,
                last_rollback_replayed,
                last_rollback_total_us,
                "multiplayer DESYNC: local engine hash differs from host's"
            );
        } else {
            tracing::debug!(frame, "multiplayer hash OK");
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct PreTickPauseSources {
    pause_menu: bool,
    manual: bool,
    multiplayer_clock: bool,
    modal: bool,
}

fn pre_tick_is_paused(sources: PreTickPauseSources) -> bool {
    sources.pause_menu || sources.manual || sources.multiplayer_clock || sources.modal
}

fn local_pause_stops_timeline(menu_open: bool, multiplayer: bool) -> bool {
    menu_open && !multiplayer
}

/// A modal freezes the authoritative timeline but not the dense replay host
/// record cursor. Recording emits stationary records while the modal remains
/// open, including the later record carrying its dismissal. Explicit user or
/// network pauses still freeze playback entirely.
fn replay_cursor_is_paused(sources: PreTickPauseSources) -> bool {
    sources.pause_menu || sources.manual || sources.multiplayer_clock
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct PreTickTimelineOutput {
    paused: bool,
    consumed_buffered: bool,
}

/// Admit replay commands and reconcile rewind history after all live/network
/// commands for the frame are known.
fn prepare_pre_tick_timeline(
    runtime: &mut crate::game_session::runtime::TimelineRuntime,
    host: &mut Host,
    game: &mut crate::game::Game,
    manager: &mut robin_engine::engine_manager::EngineManager,
    assets: &robin_engine::engine::LevelAssets,
    frame: &mut MissionFrame,
    manual_pause: &mut bool,
    rewind_active: bool,
    mut paused: bool,
    replay_cursor_paused: bool,
) -> Result<PreTickTimelineOutput, String> {
    if runtime.playback().is_some() && !replay_cursor_paused {
        // Recorded save markers pin the boundary state and load-back
        // records swap a pinned state in, before this frame's commands.
        runtime.apply_playback_timeline_events(host, game, manager, assets)?;
    }
    if let Some(player) = runtime.playback()
        && !replay_cursor_paused
    {
        if player.is_finished() {
            if !runtime.replay_finished_logged {
                tracing::info!("Replay finished after {} frames", player.current_frame());
                runtime.replay_finished_logged = true;
            }
            *manual_pause = true;
            paused = true;
        } else {
            runtime.replay_finished_logged = false;
            runtime.inject_replay_input(frame);
            frame.assert_replay_timeline_before(runtime.current_frame());
        }
    }

    let mut consumed_buffered = false;
    let current_frame = runtime.frame_number();
    if !rewind_active && !paused && current_frame < runtime.retained_history().next_record_frame() {
        let Some(recorded) = runtime.retained_history().frame_for(current_frame).cloned() else {
            return Err(format!(
                "cannot replay frame {}: rewind command history starts at frame {}",
                current_frame,
                runtime.retained_history().oldest_cmd_frame()
            ));
        };
        if runtime.playback().is_some() && frame.external_actions().is_empty() {
            frame.adopt_authoritative_input(recorded);
            consumed_buffered = true;
            tracing::trace!("Replay reused rewind-buffer frame {}", current_frame);
        } else if frame.commands().is_empty() && frame.external_actions().is_empty() {
            frame.adopt_authoritative_input(recorded);
            consumed_buffered = true;
            tracing::trace!("Auto-replay -> frame {}", current_frame);
        } else {
            tracing::trace!(
                "Auto-replay interrupted by live input; truncating buffer at {}",
                current_frame
            );
            runtime.branch_history_at(current_frame);
        }
    }
    Ok(PreTickTimelineOutput {
        paused,
        consumed_buffered,
    })
}

/// Emit pointer-derived simulation commands only after replay/rewind/pause
/// admission is final, so they enter the same deterministic frame log.
fn dispatch_pre_tick_pointer_commands(
    runtime: &crate::game_session::runtime::TimelineRuntime,
    host: &mut Host,
    manager: &mut robin_engine::engine_manager::EngineManager,
    assets: &robin_engine::engine::LevelAssets,
    input: &MissionInput,
    frame: &mut MissionFrame,
    rewind_active: bool,
    paused: bool,
) {
    if runtime.playback().is_some() || rewind_active || paused {
        return;
    }
    let Some(mouse_map) = host
        .frontend
        .viewport
        .screen_to_map(input.threaded.position())
    else {
        return;
    };

    if manager.engine.view_locked()
        && let Some(id) =
            manager
                .engine
                .find_focusable_npc(assets, mouse_map, engine_element::Focus::View)
    {
        let cmd = PlayerCommand::SelectFollowElement {
            entity_id: Some(id),
        };
        dispatch_local_command(&host.transport, &mut frame.stage_commands(), &cmd);
    }

    let bow_armed = manager
        .engine
        .selected_action_for_seat(host.transport.local_seat())
        == engine_profiles::Action::Bow;
    if host.frontend.trajectory_preview().hover_ticks() != 0 || bow_armed {
        let cmd = PlayerCommand::PerformOrientation { mouse_map };
        dispatch_local_command(&host.transport, &mut frame.stage_commands(), &cmd);
    }
}

/// The final deterministic boundary can observe input/UI pause state but
/// cannot consume events, render, play audio, or enqueue save operations.
pub(super) fn finalize_pre_tick(
    phase: MissionPreTickPhase<'_>,
    runtime: &mut TimelineRuntime,
    manual_pause: &mut bool,
    leaderboard: &mut Option<crate::game_session::leaderboard_runtime::MissionLeaderboardRuntime>,
    input: &MissionInput,
    ui: &MissionUi,
    saves: SavesPrepared,
) -> Result<FramePreparation, String> {
    let PreparationPhaseState {
        mut frame,
        mut mp_clock_pause,
        pause_closed_this_frame: _,
        rewind_active,
        shift_held,
        step_forward_pressed,
        step_back_pressed,
        modal_rendered_this_frame,
    } = saves.0;
    let MissionPreTickPhase {
        host,
        game,
        manager,
        assets,
    } = phase;
    // ── Replay: inject recorded commands + desync check ──
    // `ModalDismiss` commands are split out of the recorded stream
    // here and handed to the modal drain step further down, so the
    // interactive dialog / popup event loops are skipped during
    // playback. All other commands are sim-affecting and applied
    // immediately.
    // Freeze every sim-advancing step (replay playback, engine
    // tick, rewind-buffer commit, sim-frame increment) whenever the
    // user has asked to pause.  Under `--replay`, this means the
    // player's cursor on the recorded command stream stops too —
    // otherwise `--start-paused --replay` would still race through
    // the replay even though the tick was suppressed.
    let modal_pause = ui
        .active_modal
        .as_ref()
        .is_some_and(|modal| modal.pauses_simulation(host.transport.net().is_some()))
        || ui.terminal_flow_active()
        || (ui.sherwood_campaign_flow.is_some() && host.transport.net().is_none())
        || ui
            .lost_sherwood_gate
            .blocks_mission(game.is_sherwood, &manager.engine);

    // Drain once more at the last deterministic pre-tick boundary.
    // Packets can arrive after the top-of-loop drain while this
    // frame handles UI, local input, and modal work.  Applying due
    // inputs here keeps them on the same `sim_frame` without
    // mutating sim state at arbitrary points in the frame.
    drain_pre_tick_network(
        runtime,
        host,
        manager,
        assets,
        &mut frame,
        &mut mp_clock_pause,
        rewind_active,
    );

    // ── Multiplayer: state hash broadcast / verify ──
    // Sample after the final deterministic pre-tick network drain.
    // Inputs can arrive between the top-of-loop drain and this
    // boundary; hashing earlier can compare two machines that will
    // tick the same commands but sampled before/after a current-frame
    // input that just arrived.
    process_pre_tick_state_hash(runtime, host, manager);

    let pause_sources = PreTickPauseSources {
        // A local menu cannot stop an authoritative multiplayer clock.
        // It still owns local input and presentation, but peers and the
        // local simulation continue underneath it.
        pause_menu: local_pause_stops_timeline(
            ui.pause_menu.is_some() || ui.active_ui_task.is_some(),
            host.transport.net().is_some(),
        ),
        manual: *manual_pause,
        multiplayer_clock: mp_clock_pause,
        modal: modal_pause,
    };
    let paused = pre_tick_is_paused(pause_sources);
    let replay_cursor_paused = replay_cursor_is_paused(pause_sources);
    let PreTickTimelineOutput {
        paused,
        consumed_buffered,
    } = prepare_pre_tick_timeline(
        runtime,
        host,
        game,
        manager,
        assets.as_ref(),
        &mut frame,
        manual_pause,
        rewind_active,
        paused,
        replay_cursor_paused,
    )?;

    if runtime.take_state_restored()
        && let Some(leaderboard) = leaderboard.as_mut()
    {
        leaderboard.after_state_restore(manager.engine.campaign());
    }

    dispatch_pre_tick_pointer_commands(
        runtime,
        host,
        manager,
        assets.as_ref(),
        input,
        &mut frame,
        rewind_active,
        paused,
    );

    if paused || rewind_active {
        runtime.trace(FrameContractStage::PausedOrRewind);
    }
    runtime.trace(FrameContractStage::PreTickCommands);
    Ok(FramePreparation::Ready(PreparedFrame {
        frame,
        rewind_active,
        paused,
        consumed_buffered,
        shift_held,
        modal_rendered: modal_rendered_this_frame,
        step_forward_pressed,
        step_back_pressed,
    }))
}
#[cfg(test)]
mod tests {
    #[test]
    fn final_boundary_preserves_handoff_without_advancing_simulation() {
        use super::*;
        use crate::game_session::replay_init::ReplayAndRollback;
        use crate::game_session::runtime::FrameContract;
        use robin_engine::engine::LevelAssets;
        use robin_engine::engine_manager::EngineManager;
        use robin_engine::replay::state_hash;
        use std::sync::Arc;

        // Exercise the production boundary without a window, renderer, audio,
        // or callback store: none is part of its capability set anymore.
        for (manual, network, rewind_active) in [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (false, false, true),
            (true, true, true),
        ] {
            let mut assets = LevelAssets::new();
            let mut manager = EngineManager::new(
                Engine::new_for_test(640.0, 480.0, Default::default(), &mut assets).unwrap(),
            );
            let mut assets = Arc::new(assets);
            let mut host = Host::scratch(640.0, 480.0);
            let mut game = crate::game::Game::new(engine_profiles::MissionLocation::Lincoln);
            let mut timeline = TimelineRuntime::new(
                ReplayAndRollback {
                    recording_control:
                        std::sync::Arc::<crate::replay_service::ReplayService>::default()
                            .recording(),
                    recorder: None,
                    player: None,
                    rollback_checker: None,
                    rewind_buffer: crate::rewind::RewindBuffer::new(),
                    start_paused: false,
                },
                FrameContract::Graphical,
                false,
                true,
            );
            let mut frame = MissionFrame::new(123);
            timeline.open_frame(&mut frame, &manager.engine, &assets);
            frame
                .stage_commands()
                .commands
                .push(robin_engine::player_command::PlayerInput::host(
                    PlayerCommand::CrouchDown,
                ));
            let hash_before = state_hash(&manager.engine);
            let mut manual_pause = manual;
            let input = MissionInput::new(
                crate::input::ThreadedInput::new(),
                crate::input_translator::InputTranslator::new(640.0, 480.0),
            );
            let prepared = finalize_pre_tick(
                MissionPreTickPhase {
                    host: &mut host,
                    game: &mut game,
                    manager: &mut manager,
                    assets: &mut assets,
                },
                &mut timeline,
                &mut manual_pause,
                &mut None,
                &input,
                &MissionUi::new(true),
                SavesPrepared(PreparationPhaseState {
                    frame,
                    mp_clock_pause: network,
                    pause_closed_this_frame: true,
                    rewind_active,
                    shift_held: true,
                    step_forward_pressed: true,
                    step_back_pressed: true,
                    modal_rendered_this_frame: true,
                }),
            )
            .unwrap();
            let FramePreparation::Ready(prepared) = prepared else {
                panic!("pre-tick boundary unexpectedly requested mission control");
            };
            assert_eq!(prepared.paused, manual || network);
            assert_eq!(prepared.rewind_active, rewind_active);
            assert!(prepared.shift_held);
            assert!(prepared.step_forward_pressed);
            assert!(prepared.step_back_pressed);
            assert!(prepared.modal_rendered);
            assert!(!prepared.consumed_buffered);
            assert_eq!(prepared.frame.started_at_ms, 123);
            assert_eq!(prepared.frame.commands().len(), 1);
            assert_eq!(manual_pause, manual);
            assert_eq!(timeline.frame_number(), 0);
            assert_eq!(state_hash(&manager.engine), hash_before);
        }
    }

    fn second_drain_rollback_reopens_current_frame(use_recent_history: bool) {
        use crate::game_session::replay_init::ReplayAndRollback;
        use crate::game_session::runtime::{
            FrameCommitPolicy, FrameContract, MissionFrame, TimelineRuntime,
        };
        use crate::host::Host;
        use crate::multiplayer::{NetChannels, NetEvent};
        use crate::rewind::RewindBuffer;
        use robin_engine::engine::{Engine, LevelAssets, SimulationFrameInput};
        use robin_engine::engine_manager::EngineManager;
        use robin_engine::player_command::{PlayerCommand, PlayerId, PlayerInput};
        use robin_engine::replay::{ReplayRecorder, state_hash};
        use std::sync::Arc;

        let mut assets = LevelAssets::new();
        let mut manager = EngineManager::new(
            Engine::new_for_test(640.0, 480.0, Default::default(), &mut assets).unwrap(),
        );
        let mut assets = Arc::new(assets);
        let directory = tempfile::tempdir().unwrap();
        let recording_path = directory.path().join("second-drain.rhrec.jsonl");
        let recorder = ReplayRecorder::new(
            recording_path.to_str().unwrap(),
            "boundary".into(),
            robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                "boundary", "boundary", "boundary",
            )
            .unwrap(),
            0,
            Default::default(),
            &Default::default(),
        )
        .unwrap();
        let mut timeline = TimelineRuntime::new(
            ReplayAndRollback {
                recording_control: std::sync::Arc::<crate::replay_service::ReplayService>::default(
                )
                .recording(),
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
        // Commit two ordinary historical frames before opening this host
        // iteration, exactly as the graphical driver does before local UI.
        for number in 0..2 {
            timeline.begin_history_frame(number, &manager.engine, &assets);
            let input = SimulationFrameInput::default()
                .with_hourglass(false)
                .with_post_initialize(false);
            manager
                .engine
                .advance_frame(&assets, input.clone())
                .unwrap();
            timeline.append_history_fixture(input);
            timeline.advance_frame();
        }
        if !use_recent_history {
            timeline.clear_recent_history_fixture();
        }
        let (channels, incoming, _outgoing, _, _) = NetChannels::new();
        let mut host = Host::scratch(640.0, 480.0);
        host.transport = crate::host::HostTransport::test_session(channels, PlayerId::HOST);
        let mut frame = MissionFrame::new(17);
        frame.run_hourglass = false;
        frame.run_post_initialize = false;
        timeline.open_frame(&mut frame, &manager.engine, &assets);
        let original_hash = frame.recorder_hash.unwrap();
        let local = PlayerInput::host(PlayerCommand::SetUnbindingEnabled { enabled: false });
        let due = PlayerInput::host(PlayerCommand::SetAmountOfSpeaking { amount: 7 });
        frame.stage_commands().commands.push(local.clone());
        frame.adopt_authoritative_input(
            frame.authoritative_input().with_external_facts(
                robin_engine::engine::ExternalFacts::default()
                    .with_sound_boundary(robin_engine::engine::SoundBoundary::live(Vec::new())),
            ),
        );
        let facts_before =
            serde_json::to_value(&frame.authoritative_input().external_facts).unwrap();
        for (target_frame, input) in [
            (
                0,
                PlayerInput::host(PlayerCommand::SetAmountOfSpeaking { amount: 9 }),
            ),
            (2, due.clone()),
        ] {
            incoming
                .send(NetEvent::Input {
                    server_frame: 2,
                    origin_frame: target_frame,
                    target_frame,
                    input,
                })
                .unwrap();
        }

        let mut paused = false;
        // This is the actual production second-drain adapter, not a direct
        // call to a repair helper or a replacement mock network path.
        super::drain_pre_tick_network(
            &mut timeline,
            &mut host,
            &mut manager,
            &mut assets,
            &mut frame,
            &mut paused,
            false,
        );
        assert!(!paused);
        assert_eq!(
            timeline.last_mp_rollback.as_ref().unwrap().path,
            if use_recent_history {
                "recent-timeline-history"
            } else {
                "rewind-buffer"
            }
        );
        assert_eq!(
            serde_json::to_value(frame.commands()).unwrap(),
            serde_json::to_value(vec![local, due]).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&frame.authoritative_input().external_facts).unwrap(),
            facts_before
        );
        assert_eq!(frame.started_at_ms, 17);
        assert_eq!(timeline.frame_number(), 2);
        let corrected_pre_tick_hash = state_hash(&manager.engine);
        assert_ne!(corrected_pre_tick_hash, original_hash);

        timeline.begin_simulation();
        manager
            .engine
            .advance_frame(&assets, frame.authoritative_input())
            .unwrap();
        timeline.begin_bookkeeping();
        timeline.commit_simulation_history(
            &mut host,
            &mut manager,
            &frame,
            FrameCommitPolicy {
                store_rewind_commands: true,
            },
        );
        assert_eq!(timeline.retained_history().next_record_frame(), 3);
        assert_eq!(frame.recorder_hash, Some(corrected_pre_tick_hash));
        assert_eq!(
            timeline.retained_history().commands_for(2).unwrap().len(),
            2
        );
        let checkpoint = timeline
            .retained_history()
            .restore_recent(2, robin_engine::sim_timeline::RestorePolicy::Exact)
            .unwrap();
        assert_eq!(state_hash(&checkpoint.engine), corrected_pre_tick_hash);

        // The telemetry intentionally survives into the next host iteration.
        // Its empty second drain must not re-open/re-sample based on stale
        // last_mp_rollback. A sentinel makes an unwanted re-sample observable.
        timeline.advance_frame();
        let mut next_frame = MissionFrame::new(31);
        timeline.open_frame(&mut next_frame, &manager.engine, &assets);
        next_frame.recorder_hash = Some(0x55aa);
        super::drain_pre_tick_network(
            &mut timeline,
            &mut host,
            &mut manager,
            &mut assets,
            &mut next_frame,
            &mut paused,
            false,
        );
        assert_eq!(next_frame.recorder_hash, Some(0x55aa));
    }

    #[test]
    fn second_network_drain_recent_rollback_reopens_pre_tick_boundary() {
        second_drain_rollback_reopens_current_frame(true);
    }

    #[test]
    fn second_network_drain_sparse_rollback_reopens_pre_tick_boundary() {
        second_drain_rollback_reopens_current_frame(false);
    }

    #[test]
    fn host_snapshot_reset_discards_inputs_accumulated_before_second_drain() {
        let mut frame = super::MissionFrame::new(0);
        frame
            .stage_commands()
            .commands
            .push(robin_engine::player_command::PlayerInput::host(
                robin_engine::player_command::PlayerCommand::CrouchDown,
            ));
        super::discard_abandoned_host_frame_inputs(&mut frame);
        assert!(frame.commands().is_empty());
    }

    use super::{
        PreTickPauseSources, local_pause_stops_timeline, pre_tick_is_paused,
        replay_cursor_is_paused,
    };

    #[test]
    fn local_pause_only_stops_single_player_timeline() {
        assert!(local_pause_stops_timeline(true, false));
        assert!(!local_pause_stops_timeline(true, true));
        assert!(!local_pause_stops_timeline(false, false));
    }

    #[test]
    fn pre_tick_pause_combines_all_graphical_pause_sources() {
        let clear = PreTickPauseSources {
            pause_menu: false,
            manual: false,
            multiplayer_clock: false,
            modal: false,
        };
        assert!(!pre_tick_is_paused(clear));

        for paused in [
            PreTickPauseSources {
                pause_menu: true,
                ..clear
            },
            PreTickPauseSources {
                manual: true,
                ..clear
            },
            PreTickPauseSources {
                multiplayer_clock: true,
                ..clear
            },
            PreTickPauseSources {
                modal: true,
                ..clear
            },
        ] {
            assert!(pre_tick_is_paused(paused));
        }
    }

    #[test]
    fn modal_pause_keeps_replay_host_records_moving() {
        let modal_only = PreTickPauseSources {
            pause_menu: false,
            manual: false,
            multiplayer_clock: false,
            modal: true,
        };
        assert!(pre_tick_is_paused(modal_only));
        assert!(!replay_cursor_is_paused(modal_only));

        for explicit_pause in [
            PreTickPauseSources {
                pause_menu: true,
                ..modal_only
            },
            PreTickPauseSources {
                manual: true,
                ..modal_only
            },
            PreTickPauseSources {
                multiplayer_clock: true,
                ..modal_only
            },
        ] {
            assert!(replay_cursor_is_paused(explicit_pause));
        }
    }
}
