//! Per-frame tick orchestration: audio tick, pre/post-render engine
//! hooks, command drain + replay/rewind step, and dismiss helpers
//! for pending modals.

use super::modal_state::ActiveModal;
use crate::audio_backend::KiraAudioBackend;
use crate::game::Game;
use crate::host::Host;
use crate::host::{DeferredAudioRequest, HostSignal};
use crate::sound::AlertStatus;
use robin_engine::ai::AlertLevel;
use robin_engine::coordinates::MapBBox;
use robin_engine::engine as engine_api;
use robin_engine::engine_manager as engine_manager_api;
use robin_engine::player_command::{PlayerCommand, PlayerInput};
use robin_engine::sound_cache::SampleLoader;

/// Per-frame audio tick.
///
/// Combat/alert music transitions + sim-emitted sound drains.
/// Handles the music-mode response to alert-status changes plus the
/// resume-all / activate side-effect queues filled by
/// `perform_hourglass`.  The villain-alert recomputation that drives
/// `alert_status` runs inside `perform_hourglass` so it's part of the
/// rollback snapshot.
pub(super) fn tick_audio(
    manager: &mut engine_manager_api::EngineManager,
    host: &mut Host,
    backend: &mut KiraAudioBackend,
    sample_loader: &SampleLoader,
    sound_rng: &mut fastrand::Rng,
    assets: &engine_api::LevelAssets,
) -> Option<engine_api::SoundBoundary> {
    let alert_status = match manager.engine.ai_global().overall_alert_status {
        AlertLevel::Green => AlertStatus::Green,
        AlertLevel::Yellow => AlertStatus::Yellow,
        AlertLevel::Red => AlertStatus::Red,
    };
    let deferred = std::mem::take(&mut host.audio.deferred);
    let mut pending_play_delayed_sources = Vec::new();
    let mut resume_all_sources = false;
    let mut activate_sources = Vec::new();
    let mut stop_exclamations = Vec::new();
    let mut stop_exclamation_channels = Vec::new();
    for request in deferred {
        match request {
            DeferredAudioRequest::PlayDelayedSource(index) => {
                pending_play_delayed_sources.push(index);
            }
            DeferredAudioRequest::ResumeAllSources => resume_all_sources = true,
            DeferredAudioRequest::ActivateSource(index) => activate_sources.push(index),
            DeferredAudioRequest::StopExclamation(actor_id) => {
                stop_exclamations.push(actor_id);
            }
            DeferredAudioRequest::StopExclamationChannel(actor_id) => {
                stop_exclamation_channels.push(actor_id);
            }
        }
    }
    // Drain sim-emitted sound commands that need access to
    // `engine.sound_sim.sources` (stashed on host by `apply_side_effects`).
    host.sync_sound_listener();
    if resume_all_sources {
        host.audio.sound.resume_all_sound_sources(
            &manager.engine.sound_sim().sources,
            host.viewport.sound_listen_point(),
            host.viewport.zoom_factor,
        );
    }
    for idx in activate_sources {
        // Sim already flipped `src.active = true` inside
        // `perform_hourglass`; host only starts the audio channel.
        host.audio
            .sound
            .activate_sound_source(&manager.engine.sound_sim().sources, idx);
    }
    for actor_id in stop_exclamation_channels {
        host.audio
            .sound
            .stop_exclamation_channel_only(actor_id, backend);
    }
    for actor_id in stop_exclamations {
        host.audio.sound.stop_exclamation(actor_id, backend);
    }
    let resolved_exclamations = host.audio.sound.hourglass(
        backend,
        sample_loader,
        &mut |n| sound_rng.u32(0..n),
        alert_status,
        &manager.engine.sound_sim().sources,
        &mut pending_play_delayed_sources,
    );
    let resolved_exclamations: Vec<_> = resolved_exclamations
        .into_iter()
        .map(|resolved| {
            let pending = manager
                .engine
                .sound_sim()
                .pending_exclamations
                .iter()
                .find(|pending| {
                    pending.actor_id == resolved.actor_id
                        && pending.exclamation_id == resolved.exclamation_id
                        && ((pending.profile_id & 0xFFFF_0000)
                            | u32::from(pending.exclamation_id))
                            == resolved.identifier
                })
                .unwrap_or_else(|| {
                    panic!(
                        "host resolved speech ({}, {}, {}) without its authoritative pending request",
                        resolved.actor_id, resolved.identifier, resolved.exclamation_id
                    )
                });
            let duration_frames = assets
                .exclamation_durations
                .get(&(pending.group, pending.profile_id, pending.exclamation_id))
                .copied()
                .unwrap_or_else(|| {
                    panic!(
                        "authoritative speech duration missing for {:?} profile {} exclamation {}",
                        pending.group, pending.profile_id, pending.exclamation_id
                    )
                });
            robin_engine::sound::ResolvedExclamation {
                actor_id: resolved.actor_id,
                identifier: resolved.identifier,
                exclamation_id: resolved.exclamation_id,
                duration_frames,
            }
        })
        .collect();
    // The hourglass drains the queue; whatever it left behind
    // (nothing today, but defensive) goes back on host for next frame.
    host.audio.deferred.extend(
        pending_play_delayed_sources
            .into_iter()
            .map(DeferredAudioRequest::PlayDelayedSource),
    );
    (!resolved_exclamations.is_empty())
        .then_some(engine_api::SoundBoundary::live(resolved_exclamations))
}

/// Apply every pending engine mutation that conceptually belongs with
/// the render pass but must happen *before* `render_frame` so the latter
/// can observe an immutable `&Engine`:
///
/// - Drain deferred `BlitToMap` patch-effect background decal updates.
///
/// The back-to-front draw order (`host.draw_order`) is refreshed at the
/// top of the main loop via `engine.compute_display_order()` — it's host-
/// cache derived state, not sim state, and lives outside the command
/// pipeline.
pub(super) fn pre_render_engine_setup(
    _manager: &mut engine_manager_api::EngineManager,
    host: &mut Host,
    _assets: &engine_api::LevelAssets,
    _renderer: &mut crate::renderer::Renderer,
) {
    let view = host.viewport.view_position;
    let screen = host.viewport.screen_size;
    let zoom = host.viewport.zoom_factor;
    if zoom > 0.0 {
        // C++ `RHEngine::PerformRefreshAllElements` refreshes
        // `RHDrawManager` from the current camera before any world-space
        // overlay uses it.
        host.draw_manager.update_drawing_parameters(
            0,
            MapBBox::from_coords(
                view.x,
                view.y,
                view.x + (screen.x - 1.0) / zoom,
                view.y + (screen.y - engine_api::PANNEL_HEIGHT + 1.0) / zoom,
            ),
            zoom,
        );
    }

    crate::blit_to_map::drain_pending_bg_blits(host);
}

/// Pump any host-side deferred console output into the overlay. Keeps
/// the overlay-owned scrollback as the single display surface for all
/// cheat feedback, regardless of which subsystem originates the message.
pub(super) fn drain_pending_console_output(
    console_overlay: &mut crate::console_overlay::ConsoleOverlay,
    host: &mut Host,
) {
    console_overlay.drain_pending_host_output(host);
}

/// Post-render bookkeeping: clear the one-shot `display_double_status_bar`
/// NPC flag after `render_combat_status_bars` has observed it.
pub(super) fn post_render_engine_cleanup(
    frame: &mut super::runtime::MissionFrame,
    host: &mut Host,
) {
    frame.post_commands.push(PlayerInput::new(
        host.transport.local_seat,
        PlayerCommand::ClearNpcDoubleStatusBarFlags,
    ));
}

/// Process every queued `/step-forward` / `/step-back` HTTP request,
/// replying to each with the post-step frame number.
///
/// Each forward step runs `n` full frame-equivalent ticks (the same
/// bookkeeping the main loop does on a normal unpaused frame: rollback
/// checker, rewind-buffer commit, and timeline-cursor advance). Each back step
/// rewinds `n` frames through the rewind buffer, swapping out the live
/// rollback state with the reconstructed state.
///
/// **Pending modals (dialog / popup-scroll / debriefing / sherwood
/// report / pause-all) are dismissed silently.**  The normal per-frame
/// drain functions show a blocking UI that waits for a mouse click —
/// fine interactively, a deadlock for scripted HTTP drivers (which is
/// the whole point of `--start-paused`).  We just clear the queues
/// both before the first tick and after each subsequent tick so the
/// sim keeps advancing past anything the scripts queue.  The reply
/// includes `modals_dismissed` so callers can see it happened.
///
/// Called once per frame from the main loop, after `drain_global`
/// (which enqueues the requests) and after the normal tick block (so
/// any tick that just ran gets committed to the rewind buffer before
/// we append more frames to it).
#[allow(clippy::too_many_arguments)]
pub(super) fn drain_steps(
    manager: &mut engine_manager_api::EngineManager,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    dev: &mut engine_api::DevState,
    game: &mut Game,
    timeline: &mut super::runtime::TimelineRuntime,
    manual_pause: &mut bool,
    active_modal: &mut Option<ActiveModal>,
) {
    let steps = crate::http_server::take_pending_steps();
    if steps.is_empty() {
        return;
    }

    for step in steps {
        let mut modals_dismissed = dismiss_pending_modals(host);
        if active_modal.take().is_some() {
            modals_dismissed += 1;
            tracing::debug!("HTTP step: dismissed pre-existing active modal");
        }

        match step.kind {
            crate::http_server::StepKind::Forward { n } => {
                let start = timeline.frame_number();
                let result = run_forward_ticks(manager, host, assets, dev, game, timeline, n);
                // Stepping bypasses the checker's begin_frame/end_frame
                // pairing, so its ring buffer is now stale relative to
                // the advanced engine.  Clear it — the checker resumes
                // populating on the next normal frame.
                if let Some(checker) = timeline.rollback_checker.as_mut() {
                    checker.reset();
                }
                match result {
                    Ok((advanced, dismissed_during)) => {
                        modals_dismissed += dismissed_during;
                        step.respond_ok(serde_json::json!({
                            "direction": "forward",
                            "from_frame": start,
                            "frame": timeline.frame_number(),
                            "advanced": advanced,
                            "modals_dismissed": modals_dismissed,
                        }));
                    }
                    Err(error) => step.respond_err(error),
                }
            }
            crate::http_server::StepKind::Back { n } => {
                let Some(target) = timeline.frame_number().checked_sub(n) else {
                    step.respond_err(format!(
                        "n={} exceeds current frame {}",
                        n,
                        timeline.frame_number()
                    ));
                    continue;
                };
                match rewind_to_frame(manager, host, assets, timeline, target) {
                    Ok(from) => step.respond_ok(serde_json::json!({
                        "direction": "back",
                        "from_frame": from,
                        "frame": target,
                        "rewound": from - target,
                    })),
                    Err(e) => step.respond_err(e),
                }
            }
            crate::http_server::StepKind::GoToFrame { target } => {
                let from = timeline.frame_number();
                use std::cmp::Ordering;
                let result: Result<&'static str, String> = match target.cmp(&from) {
                    Ordering::Equal => Ok("noop"),
                    Ordering::Greater => {
                        let delta = target - from;
                        match run_forward_ticks(manager, host, assets, dev, game, timeline, delta) {
                            Ok((advanced, dismissed_during)) => {
                                modals_dismissed += dismissed_during;
                                if advanced < delta {
                                    Err(format!(
                                        "advanced {advanced} of {delta} frames before stepping stopped"
                                    ))
                                } else {
                                    Ok("forward")
                                }
                            }
                            Err(error) => Err(error),
                        }
                    }
                    Ordering::Less => {
                        rewind_to_frame(manager, host, assets, timeline, target).map(|_| "back")
                    }
                };
                // The rollback checker's ring now references a timeline
                // the live engine is no longer on; clear it so the next
                // normal frame starts a fresh window.
                if let Some(checker) = timeline.rollback_checker.as_mut() {
                    checker.reset();
                }
                // Post-rewind / post-forward state may have its own
                // pending modals; keep the same "always dismiss"
                // policy so the next drain_steps call (or normal
                // tick) doesn't hit a blocking UI.
                modals_dismissed += dismiss_pending_modals(host);
                if active_modal.take().is_some() {
                    modals_dismissed += 1;
                }
                match result {
                    Ok(kind) => step.respond_ok(serde_json::json!({
                        "direction": "go-to-frame",
                        "from_frame": from,
                        "frame": timeline.frame_number(),
                        "applied": kind,
                        "modals_dismissed": modals_dismissed,
                    })),
                    Err(e) => step.respond_err(e),
                }
            }
            crate::http_server::StepKind::SetPaused { paused } => {
                *manual_pause = paused;
                step.respond_ok(serde_json::json!({
                    "paused": paused,
                    "frame": timeline.frame_number(),
                }));
            }
        }
    }
}

/// Run up to `n` forward ticks, applying the next recorded commands
/// on each tick when a replay is active.  Returns the number of
/// frames advanced and the count of modals silently dismissed
/// mid-sequence.
///
/// Any modal that becomes pending during the run (dialog,
/// popup-scroll, debriefing, sherwood report, mission-state popup)
/// is dismissed in place and the loop keeps going — the whole point
/// of HTTP stepping is to drive past these without an interactive
/// click.  The keyboard step path in `run_mission` instead refuses
/// to step while a modal is pending; that's a deliberate
/// interactive-vs-scripted divergence.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_forward_ticks(
    manager: &mut engine_manager_api::EngineManager,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    dev: &mut engine_api::DevState,
    game: &mut Game,
    timeline: &mut super::runtime::TimelineRuntime,
    n: u32,
) -> Result<(u32, usize), String> {
    let start = timeline.frame_number();
    let mut dismissed = 0usize;
    for _ in 0..n {
        let frame = timeline.frame_number();
        // Stepping into a save-marker / load-back frame must pin or swap
        // state exactly like the normal playback admission path.
        if timeline
            .replay_player
            .as_ref()
            .is_some_and(|player| !player.is_finished())
        {
            timeline.apply_playback_timeline_events(host, game, manager, assets)?;
        }
        let buffered_frame = if frame < timeline.rewind_buffer.next_record_frame() {
            let Some(recorded) = timeline.rewind_buffer.frame_for(frame).cloned() else {
                return Err(format!(
                    "cannot step frame {frame}: rewind command history starts at frame {}",
                    timeline.rewind_buffer.oldest_cmd_frame()
                ));
            };
            Some(recorded)
        } else {
            None
        };

        let (replay_input, replay_timeline_after) = match timeline
            .consume_replay_frame_for_step()?
        {
            super::runtime::ReplayStepAdmission::NoActiveReplay => (None, None),
            super::runtime::ReplayStepAdmission::Recorded(recorded) => {
                // Host controls are intentionally ignored while scrubbing because
                // presentation modal state may not have the same shape mid-run.
                if recorded.timeline_before != frame {
                    return Err(format!(
                        "replay ordinal admitted at timeline {}, current timeline is {}",
                        recorded.timeline_before, frame
                    ));
                }
                (
                    Some(recorded.input),
                    Some(super::runtime::TimelineFrame::from_wire(
                        recorded.timeline_after,
                    )),
                )
            }
            super::runtime::ReplayStepAdmission::Finished {
                ordinal,
                total_frames,
            } => {
                return Err(format!(
                    "cannot step replay at timeline frame {frame}: replay is finished at ordinal {ordinal} of {total_frames}"
                ));
            }
        };

        // HTTP stepping can advance multiple ticks inside one host frame,
        // so each admitted tick needs its own pre-tick checkpoints. Detect a
        // replay EOF before opening either transaction.
        let engine = &mut manager.engine;
        timeline.rewind_buffer.begin_frame(frame, engine, assets);
        if let Some(checker) = timeline.rollback_checker.as_mut() {
            checker.begin_frame(frame, engine);
        }
        // Force-unpaused tick.  Same as the live-frame path at the
        // top of `run_mission`'s tick block, minus the paused /
        // rewind_active gating — stepping while paused is the whole
        // point of the endpoint.
        let mut display = std::mem::take(&mut host.engine_display);
        let simulation_frame = match (buffered_frame.clone(), replay_input) {
            (Some(buffered), _) => buffered,
            (None, Some(recorded)) => recorded,
            (None, None) => {
                robin_engine::engine::SimulationFrameInput::default().with_post_initialize(true)
            }
        };
        game.run_engine_tick(
            host,
            &mut display,
            assets,
            engine,
            dev,
            simulation_frame.clone(),
            false,
            false,
        );
        host.engine_display = display;
        if let Some(checker) = timeline.rollback_checker.as_mut() {
            checker.end_frame_input(host, simulation_frame.clone(), engine);
        }
        if buffered_frame.is_none() {
            timeline.rewind_buffer.end_frame_input(simulation_frame);
        }
        if let Some(after) = replay_timeline_after {
            timeline.adopt_frame(after);
        } else {
            timeline.advance_frame();
        }

        // If the tick queued any modal, drop it silently and keep
        // going.  Without this the caller's `step N` would stop at
        // the first dialog and the next step request would do the
        // same dance — making `step 1000` advance only as far as
        // the first scripted dialog.
        if modal_state_pending(host) {
            dismissed += dismiss_pending_modals(host);
        }
    }
    Ok((timeline.frame_number() - start, dismissed))
}

/// Rewind to `target`, restoring rollback state from the rewind
/// buffer and syncing the replay cursor if one is active.
/// Returns the frame we rewound from on success.
#[allow(clippy::too_many_arguments)]
pub(super) fn rewind_to_frame(
    manager: &mut engine_manager_api::EngineManager,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    timeline: &mut super::runtime::TimelineRuntime,
    target: u32,
) -> Result<u32, String> {
    let _ = host; // reserved for future hooks (e.g. cursor reset on scrub)
    let Some(oldest) = timeline.rewind_buffer.oldest_reachable_frame() else {
        return Err("rewind buffer empty".into());
    };
    if target < oldest {
        return Err(format!(
            "target frame {target} is older than the oldest retained snapshot ({oldest})"
        ));
    }
    let from = timeline.frame_number();
    timeline.rewind_buffer.begin_session();
    let restored = timeline.restore_retained_frame(
        manager,
        assets,
        super::runtime::TimelineFrame::from_wire(target),
    );
    timeline.rewind_buffer.end_session();
    if !restored {
        return Err("rewind_to failed (no matching snapshot)".into());
    }
    Ok(from)
}

/// True iff the engine has queued a modal dialog / debriefing / scroll
/// / sherwood report that hasn't been shown yet.  Used to gate the
/// interactive step-forward/back hotkeys (they refuse while a modal is
/// pending).  The HTTP stepping path uses `dismiss_pending_modals`
/// instead — scripted drivers want the sim to keep advancing.
pub(super) fn modal_state_pending(host: &Host) -> bool {
    host.effects.dialogue_count() != 0
        || host.effects.popup_text_count() != 0
        || host.effects.debriefing_count() != 0
        || host.effects.has_sherwood_report()
        || host.effects.has_signal(HostSignal::MissionStatePopup)
}

/// Silently drop every queued modal on `host`. Used by non-interactive
/// graphical drivers such as HTTP stepping and mission-map rendering so they
/// never deadlock on the blocking dialog/debriefing/popup UI. Returns the
/// number of modals that were dropped so the step reply can surface
/// it (mostly for debuggability: "why did my scripted driver miss the
/// briefing?" — because it was dismissed, here's the count).
pub(super) fn dismiss_pending_modals(host: &mut Host) -> usize {
    let n = host.effects.dialogue_count()
        + host.effects.popup_text_count()
        + host.effects.debriefing_count()
        + host.effects.has_sherwood_report() as usize
        + host.effects.has_signal(HostSignal::MissionStatePopup) as usize;
    if n > 0 {
        tracing::debug!(
            "non-interactive driver: dismissing {} pending modal(s) \
             (dialogues={}, popups={}, debriefings={}, sherwood_report={}, mission_state={})",
            n,
            host.effects.dialogue_count(),
            host.effects.popup_text_count(),
            host.effects.debriefing_count(),
            host.effects.has_sherwood_report(),
            host.effects.has_signal(HostSignal::MissionStatePopup),
        );
    }
    drop(host.effects.take_dialogues());
    drop(host.effects.take_popup_texts());
    drop(host.effects.take_debriefings());
    host.effects.take_sherwood_report();
    host.effects.take_signal(HostSignal::MissionStatePopup);
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rewind::RewindBuffer;
    use robin_engine::campaign::Campaign;
    use robin_engine::replay::{
        REPLAY_SCHEMA_VERSION, ReplayFile, ReplayFrame, ReplayHeader, ReplayPlayer,
    };
    use std::collections::BTreeMap;

    fn one_frame_replay(input: engine_api::SimulationFrameInput) -> ReplayPlayer {
        let data: robin_engine::replay::ReplayData = ReplayFile {
            header: ReplayHeader {
                mission_id: "step-test".into(),
                rng_seed: 0,
                sim_config: engine_api::SimConfig::default(),
                version: REPLAY_SCHEMA_VERSION,
                total_frames: 1,
                campaign: bitcode::encode(&Campaign::default()),
            },
            frames: BTreeMap::from([(
                0,
                ReplayFrame {
                    timeline_before: 0,
                    timeline_after: 1,
                    input,
                    host_controls: Vec::new(),
                },
            )]),
            hashes: BTreeMap::new(),
            save_markers: BTreeMap::new(),
            load_backs: BTreeMap::new(),
        }
        .into();
        ReplayPlayer::new(data)
    }

    fn stepping_fixture(
        replay_player: Option<ReplayPlayer>,
    ) -> (
        engine_api::LevelAssets,
        engine_manager_api::EngineManager,
        Host,
        engine_api::DevState,
        Game,
        super::super::runtime::TimelineRuntime,
    ) {
        let mut assets = engine_api::LevelAssets::new();
        let engine = engine_api::Engine::new_for_test_with_level_size(
            1024.0,
            768.0,
            Campaign::default(),
            &mut assets,
            4096.0,
            4096.0,
        )
        .expect("fixture engine");
        let timeline = super::super::runtime::TimelineRuntime::new(
            super::super::replay_init::ReplayAndRollback {
                recorder: None,
                player: replay_player,
                rollback_checker: None,
                rewind_buffer: RewindBuffer::new(),
                start_paused: false,
            },
            super::super::runtime::FrameContract::Graphical,
            false,
            true,
        );
        (
            assets,
            engine_manager_api::EngineManager::new(engine),
            Host::default(),
            engine_api::DevState::default(),
            Game::default(),
            timeline,
        )
    }

    #[test]
    fn non_replay_forward_step_keeps_live_debugger_behavior() {
        let (assets, mut manager, mut host, mut dev, mut game, mut timeline) =
            stepping_fixture(None);

        let (advanced, dismissed) = run_forward_ticks(
            &mut manager,
            &mut host,
            &assets,
            &mut dev,
            &mut game,
            &mut timeline,
            1,
        )
        .expect("live debugger step");

        assert_eq!(advanced, 1);
        assert_eq!(dismissed, 0);
        assert_eq!(timeline.frame_number(), 1);
        let input = timeline
            .rewind_buffer
            .frame_for(0)
            .expect("live step recorded in rewind history");
        assert!(input.run_post_initialize);
    }

    #[test]
    fn replay_eof_refuses_to_fabricate_another_step() {
        let recorded_input = engine_api::SimulationFrameInput::default();
        let player = one_frame_replay(recorded_input);
        let (assets, mut manager, mut host, mut dev, mut game, mut timeline) =
            stepping_fixture(Some(player));

        let (advanced, _) = run_forward_ticks(
            &mut manager,
            &mut host,
            &assets,
            &mut dev,
            &mut game,
            &mut timeline,
            1,
        )
        .expect("recorded replay step");
        assert_eq!(advanced, 1);
        assert!(
            !timeline
                .rewind_buffer
                .frame_for(0)
                .expect("recorded replay input")
                .run_post_initialize,
            "the recorded input must remain authoritative"
        );

        let error = run_forward_ticks(
            &mut manager,
            &mut host,
            &assets,
            &mut dev,
            &mut game,
            &mut timeline,
            1,
        )
        .expect_err("replay EOF must refuse a synthetic live frame");

        assert_eq!(
            error,
            "cannot step replay at timeline frame 1: replay is finished at ordinal 1 of 1"
        );
        assert_eq!(timeline.frame_number(), 1);
        assert_eq!(timeline.rewind_buffer.next_record_frame(), 1);
        assert!(timeline.rewind_buffer.frame_for(1).is_none());
        let player = timeline
            .replay_player
            .as_ref()
            .expect("active replay remains");
        assert!(player.is_finished());
        assert_eq!(player.current_frame(), 1);
    }

    #[test]
    fn forward_scrub_reuses_recorded_span_without_appending_an_old_checkpoint() {
        let mut assets = engine_api::LevelAssets::new();
        let engine = engine_api::Engine::new_for_test_with_level_size(
            1024.0,
            768.0,
            Campaign::default(),
            &mut assets,
            4096.0,
            4096.0,
        )
        .expect("fixture engine");
        let mut rewind_buffer = RewindBuffer::new();

        // Model the state that triggered the timeline scrub crash: command
        // history through frame 425, followed by a seek back to frame 250.
        // The engine need not advance here because this test exercises the
        // history ownership contract, not deterministic replay itself.
        for frame in 0..=425 {
            rewind_buffer.begin_frame(frame, &engine, &assets);
            rewind_buffer.end_frame(Vec::new());
        }
        assert_eq!(rewind_buffer.next_record_frame(), 426);

        let mut manager = engine_manager_api::EngineManager::new(engine);
        let mut host = Host::default();
        let mut dev = engine_api::DevState::default();
        let mut game = Game::default();
        let mut timeline = super::super::runtime::TimelineRuntime::new(
            super::super::replay_init::ReplayAndRollback {
                recorder: None,
                player: None,
                rollback_checker: None,
                rewind_buffer,
                start_paused: false,
            },
            super::super::runtime::FrameContract::Graphical,
            false,
            true,
        );
        timeline.adopt_frame(super::super::runtime::TimelineFrame::from_wire(250));

        let (advanced, _) = run_forward_ticks(
            &mut manager,
            &mut host,
            &assets,
            &mut dev,
            &mut game,
            &mut timeline,
            1,
        )
        .expect("forward scrub should reuse frame 250");

        assert_eq!(advanced, 1);
        assert_eq!(timeline.frame_number(), 251);
        assert_eq!(timeline.rewind_buffer.next_record_frame(), 426);
    }
}
