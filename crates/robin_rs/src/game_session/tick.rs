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
use robin_engine::player_command::{PlayerCommand, PlayerId, PlayerInput};
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
    engine: &engine_api::Engine,
    audio: &mut crate::host::HostAudio,
    viewport: &crate::host::ViewportState,
    backend: &mut KiraAudioBackend,
    sample_loader: &SampleLoader,
    sound_rng: &mut fastrand::Rng,
    assets: &engine_api::LevelAssets,
) -> Option<engine_api::SoundBoundary> {
    let alert_status = match engine.ai_global().overall_alert_status {
        AlertLevel::Green => AlertStatus::Green,
        AlertLevel::Yellow => AlertStatus::Yellow,
        AlertLevel::Red => AlertStatus::Red,
    };
    let deferred = std::mem::take(&mut audio.deferred);
    let mut pending_play_delayed_sources = Vec::new();
    let mut resume_all_sources = false;
    let mut activate_sources = Vec::new();
    let mut refresh_ambience_sources = false;
    let mut stop_exclamations = Vec::new();
    let mut stop_exclamation_channels = Vec::new();
    for request in deferred {
        match request {
            DeferredAudioRequest::PlayDelayedSource(index) => {
                pending_play_delayed_sources.push(index);
            }
            DeferredAudioRequest::ResumeAllSources => resume_all_sources = true,
            DeferredAudioRequest::ActivateSource(index) => activate_sources.push(index),
            DeferredAudioRequest::RefreshAmbienceSources => refresh_ambience_sources = true,
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
    audio
        .sound
        .set_listen_point(viewport.sound_listen_point(), viewport.zoom_factor);
    if resume_all_sources {
        audio.sound.resume_all_sound_sources(
            &engine.sound_sim().sources,
            viewport.sound_listen_point(),
            viewport.zoom_factor,
        );
    }
    if refresh_ambience_sources {
        audio
            .sound
            .sync_ambience_sources(&engine.sound_sim().sources, backend);
    }
    for idx in activate_sources {
        // Sim already flipped `src.active = true` inside
        // `perform_hourglass`; host only starts the audio channel.
        audio
            .sound
            .activate_sound_source(&engine.sound_sim().sources, idx);
    }
    for actor_id in stop_exclamation_channels {
        audio.sound.stop_exclamation_channel_only(actor_id, backend);
    }
    for actor_id in stop_exclamations {
        audio.sound.stop_exclamation(actor_id, backend);
    }
    let resolved_exclamations = audio.sound.hourglass(
        backend,
        sample_loader,
        &mut |n| sound_rng.u32(0..n),
        alert_status,
        &engine.sound_sim().sources,
        &mut pending_play_delayed_sources,
    );
    let resolved_exclamations: Vec<_> = resolved_exclamations
        .into_iter()
        .map(|resolved| {
            let pending = engine
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
                .audio.exclamation_durations()
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
    audio.deferred.extend(
        pending_play_delayed_sources
            .into_iter()
            .map(DeferredAudioRequest::PlayDelayedSource),
    );
    (!resolved_exclamations.is_empty())
        .then_some(engine_api::SoundBoundary::live(resolved_exclamations))
}

/// Apply pending host presentation updates before `render_frame`.
/// This setup does not require simulation mutation:
///
/// - Drain deferred patch-effect background decal updates.
///
/// The back-to-front draw order (`host.frontend.draw_order`) is refreshed at the
/// top of the main loop via `engine.compute_display_order()` — it's host-
/// cache derived state, not sim state, and lives outside the command
/// pipeline.
pub(super) fn pre_render_engine_setup(host: &mut Host) {
    sync_render_camera(&mut host.frontend);
    crate::blit_to_map::drain_pending_bg_blits(&mut host.frontend, &mut host.effects);
}

/// Refresh camera-derived draw parameters without consuming any fixed-tick
/// side-effect queues. Native-refresh interpolation calls this for each
/// sampled camera pose.
pub(super) fn sync_render_camera(frontend: &mut crate::host::HostFrontend) {
    let view = frontend.viewport.view_position;
    let screen = frontend.viewport.screen_size;
    let zoom = frontend.viewport.zoom_factor;
    if zoom > 0.0 {
        // The original game's refresh updates
        // the draw manager from the current camera before any world-space
        // overlay uses it.
        frontend.draw_manager.update_drawing_parameters(
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
}

/// Post-render bookkeeping: clear the one-shot `display_double_status_bar`
/// NPC flag after `render_combat_status_bars` has observed it.
pub(super) fn post_render_engine_cleanup(
    frame: &mut super::runtime::MissionFrame,
    local_seat: robin_engine::player_command::PlayerId,
) {
    frame.post_commands.push(PlayerInput::new(
        local_seat,
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
/// Pending gameplay modals are resolved under the request's typed modal
/// policy. Automation defaults to a conservative auto-dismiss outcome; strict
/// drivers can disable it and provide one-shot `(ModalKind, DialogResult)`
/// answers. An unanswered modal blocks without mutating its queue. Replies
/// include every accepted typed outcome.
///
/// Called once per frame from the main loop, after the session RPC drain
/// (which enqueues the requests) and after the normal tick block (so
/// any tick that just ran gets committed to the rewind buffer before
/// we append more frames to it).
#[allow(clippy::too_many_arguments)]
pub(super) fn drain_steps(
    steps: Vec<crate::http_server::PendingStep>,
    manager: &mut engine_manager_api::EngineManager,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    dev: &mut engine_api::DevState,
    game: &mut Game,
    timeline: &mut super::runtime::TimelineRuntime,
    manual_pause: &mut bool,
    active_modal: &mut Option<ActiveModal>,
    mut terminal_debriefing: Option<&mut super::terminal_debriefing::TerminalDebriefingState>,
    terminal_save_manager: Option<&crate::savegame::SaveGameManager>,
    mission_ui_block_reason: Option<&str>,
    mut session_modals: Option<&mut super::session_policy::SessionModalScheduler>,
    mut resolve_local_ui: impl FnMut(&crate::http_server::StepModalPolicy) -> Result<(), String>,
) {
    if steps.is_empty() {
        return;
    }

    for step in steps {
        // Keep the response handle intact: every branch below consumes the
        // complete PendingStep when it replies.
        let kind = step.kind.clone();
        let mut modal_policy = match &kind {
            crate::http_server::StepKind::Forward { modal_policy, .. }
            | crate::http_server::StepKind::Back { modal_policy, .. }
            | crate::http_server::StepKind::GoToFrame { modal_policy, .. } => {
                Some(modal_policy.clone())
            }
            crate::http_server::StepKind::SetPaused { .. } => None,
        };
        if let Err(error) = validate_multiplayer_step_request(host, timeline.mp_admission, &kind) {
            step.respond_err(error);
            continue;
        }
        if let Some(policy) = modal_policy.as_ref()
            && let Err(error) = resolve_local_ui(policy)
        {
            step.respond_err(error);
            continue;
        }
        if modal_policy.is_some()
            && let Some(reason) = mission_ui_block_reason
        {
            step.respond_err(format!(
                "blocked by {reason}; dismiss it in the game before stepping"
            ));
            continue;
        }
        if let (Some(policy), Some(terminal)) =
            (modal_policy.as_mut(), terminal_debriefing.as_deref_mut())
        {
            let Some(modal_kind) = terminal.current_kind() else {
                step.respond_err(
                    "blocked by mission-end leaderboard; dismiss it in the game before stepping"
                        .to_owned(),
                );
                continue;
            };
            let explicit = policy
                .dismissals
                .iter()
                .position(|dismissal| dismissal.kind == modal_kind)
                .map(|index| policy.dismissals.remove(index).result);
            let result = match explicit.or_else(|| {
                policy
                    .auto_dismiss
                    .then(|| default_http_modal_result(&modal_kind))
            }) {
                Some(result) => result,
                None => {
                    step.respond_err(format!(
                        "blocked by modal {}; retry with auto_dismiss=true or a matching typed dismissal",
                        serde_json::to_string(&modal_kind).expect("ModalKind serializes")
                    ));
                    continue;
                }
            };
            let terminal_dismissal = crate::http_server::HttpModalDismissal {
                kind: modal_kind.clone(),
                result,
            };
            if let Err(error) = validate_http_modal_result(&modal_kind, result)
                .and_then(|()| authorize_http_modal_dismissals(host, &[terminal_dismissal]))
                .and_then(|()| {
                    terminal.queue_http_result(modal_kind.clone(), result, terminal_save_manager)
                })
            {
                step.respond_err(error);
                continue;
            }
            step.respond_err(format!(
                "dismissed terminal modal {}; retry the step after the outer frame applies it",
                serde_json::to_string(&modal_kind).expect("ModalKind serializes")
            ));
            continue;
        }
        let strict_session_replay = session_modals.is_some() && timeline.replay_player.is_some();
        let mut accepted_dismissals = if strict_session_replay {
            Vec::new()
        } else if let Some(policy) = modal_policy.as_mut() {
            match resolve_http_step_modals(host, Some(active_modal), policy) {
                Ok(dismissals) => dismissals,
                Err(error) => {
                    step.respond_err(error);
                    continue;
                }
            }
        } else {
            Vec::new()
        };
        timeline.record_manual_host_controls(
            &manager.engine,
            accepted_dismissals
                .iter()
                .map(|dismissal| PlayerCommand::ModalDismiss {
                    kind: dismissal.kind.clone(),
                    result: dismissal.result,
                })
                .collect(),
        );

        match kind {
            crate::http_server::StepKind::Forward { n, .. } => {
                let start = timeline.frame_number();
                let result = run_forward_ticks_with_session_modals(
                    manager,
                    host,
                    assets,
                    dev,
                    game,
                    timeline,
                    n,
                    modal_policy
                        .as_mut()
                        .expect("forward steps always carry a modal policy"),
                    session_modals.as_deref_mut(),
                );
                match result {
                    Ok((advanced, dismissed_during)) => {
                        accepted_dismissals.extend(dismissed_during);
                        if let Err(error) =
                            begin_synchronized_step_resync(host, timeline, &manager.engine)
                        {
                            step.respond_err(error);
                            continue;
                        }
                        step.respond_ok(serde_json::json!({
                            "direction": "forward",
                            "from_frame": start,
                            "frame": timeline.frame_number(),
                            "advanced": advanced,
                            "modals_dismissed": accepted_dismissals.len(),
                            "modal_dismissals": accepted_dismissals,
                        }));
                    }
                    Err(error) => step.respond_err(error),
                }
            }
            crate::http_server::StepKind::Back { n, .. } => {
                let Some(target) = timeline.frame_number().checked_sub(n) else {
                    step.respond_err(format!(
                        "n={} exceeds current frame {}",
                        n,
                        timeline.frame_number()
                    ));
                    continue;
                };
                match rewind_with_session_modals(
                    manager,
                    host,
                    assets,
                    timeline,
                    target,
                    session_modals.as_deref_mut(),
                ) {
                    Ok(from) => {
                        if let Err(error) =
                            begin_synchronized_step_resync(host, timeline, &manager.engine)
                        {
                            step.respond_err(error);
                            continue;
                        }
                        step.respond_ok(serde_json::json!({
                            "direction": "back",
                            "from_frame": from,
                            "frame": target,
                            "rewound": from - target,
                            "modals_dismissed": accepted_dismissals.len(),
                            "modal_dismissals": accepted_dismissals,
                        }))
                    }
                    Err(e) => step.respond_err(e),
                }
            }
            crate::http_server::StepKind::GoToFrame { target, .. } => {
                let from = timeline.frame_number();
                use std::cmp::Ordering;
                let mut result: Result<&'static str, String> = match target.cmp(&from) {
                    Ordering::Equal => Ok("noop"),
                    Ordering::Greater => {
                        let delta = target - from;
                        match run_forward_ticks_with_session_modals(
                            manager,
                            host,
                            assets,
                            dev,
                            game,
                            timeline,
                            delta,
                            modal_policy
                                .as_mut()
                                .expect("go-to-frame steps always carry a modal policy"),
                            session_modals.as_deref_mut(),
                        ) {
                            Ok((advanced, dismissed_during)) => {
                                accepted_dismissals.extend(dismissed_during);
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
                    Ordering::Less => rewind_with_session_modals(
                        manager,
                        host,
                        assets,
                        timeline,
                        target,
                        session_modals.as_deref_mut(),
                    )
                    .map(|_| "back"),
                };
                match if strict_session_replay {
                    Ok(Vec::new())
                } else {
                    resolve_http_step_modals(
                        host,
                        Some(active_modal),
                        modal_policy
                            .as_mut()
                            .expect("go-to-frame steps always carry a modal policy"),
                    )
                } {
                    Ok(dismissed) => accepted_dismissals.extend(dismissed),
                    Err(error) if result.is_ok() => result = Err(error),
                    Err(_) => {}
                }
                match result {
                    Ok(kind) => {
                        if let Err(error) =
                            begin_synchronized_step_resync(host, timeline, &manager.engine)
                        {
                            step.respond_err(error);
                            continue;
                        }
                        step.respond_ok(serde_json::json!({
                            "direction": "go-to-frame",
                            "from_frame": from,
                            "frame": timeline.frame_number(),
                            "applied": kind,
                            "modals_dismissed": accepted_dismissals.len(),
                            "modal_dismissals": accepted_dismissals,
                        }))
                    }
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

fn validate_multiplayer_step_request(
    host: &Host,
    admission: super::runtime::MultiplayerAdmission,
    kind: &crate::http_server::StepKind,
) -> Result<(), String> {
    if host.transport.net().is_none() {
        return Ok(());
    }
    let policy = match kind {
        crate::http_server::StepKind::SetPaused { .. } => {
            return Err(
                "manual pause is disabled in multiplayer; use explicit synchronized host timeline movement"
                    .to_string(),
            );
        }
        crate::http_server::StepKind::Forward { modal_policy, .. }
        | crate::http_server::StepKind::Back { modal_policy, .. }
        | crate::http_server::StepKind::GoToFrame { modal_policy, .. } => modal_policy,
    };
    if host.transport.local_seat() != PlayerId::HOST {
        return Err(
            "manual stepping is disabled for multiplayer clients; only explicit synchronized host automation is allowed"
                .to_string(),
        );
    }
    if !policy.synchronized_multiplayer {
        return Err(
            "manual stepping is disabled in multiplayer; retry on the host with synchronized_multiplayer=true"
                .to_string(),
        );
    }
    if host.transport.reconnecting() || admission != super::runtime::MultiplayerAdmission::Running {
        return Err(
            "multiplayer snapshot synchronization is still in progress; wait for the ready barrier"
                .to_string(),
        );
    }
    Ok(())
}

fn begin_synchronized_step_resync(
    host: &mut Host,
    timeline: &mut super::runtime::TimelineRuntime,
    engine: &engine_api::Engine,
) -> Result<(), String> {
    let Some(net) = host.transport.net() else {
        return Ok(());
    };
    if host.transport.local_seat() != PlayerId::HOST {
        return Err("only the multiplayer host can synchronize manual stepping".to_string());
    }
    let frame = timeline.frame_number();
    timeline.begin_synchronized_step_resync();
    net.set_initial_snapshot(frame, engine);
    net.reconnect_all_for_snapshot(format!(
        "host synchronized automation adopted timeline frame {frame}"
    ))?;
    net.send_ready_to_sim(frame)?;
    host.transport.await_authoritative_snapshot();
    Ok(())
}

/// Run up to `n` forward ticks, applying the next recorded commands
/// on each tick when a replay is active.  Returns the number of
/// frames advanced and the typed modals resolved mid-sequence.
///
/// Any modal that becomes pending during the run (dialog, popup-scroll,
/// debriefing, sherwood report, mission-state popup) is resolved by that same
/// request policy. The keyboard step path instead refuses to step while a
/// modal is pending; that's a deliberate interactive-vs-scripted divergence.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_forward_ticks(
    manager: &mut engine_manager_api::EngineManager,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    dev: &mut engine_api::DevState,
    game: &mut Game,
    timeline: &mut super::runtime::TimelineRuntime,
    n: u32,
    modal_policy: &mut crate::http_server::StepModalPolicy,
) -> Result<(u32, Vec<crate::http_server::HttpModalDismissal>), String> {
    run_forward_ticks_with_session_modals(
        manager,
        host,
        assets,
        dev,
        game,
        timeline,
        n,
        modal_policy,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_forward_ticks_with_session_modals(
    manager: &mut engine_manager_api::EngineManager,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    dev: &mut engine_api::DevState,
    game: &mut Game,
    timeline: &mut super::runtime::TimelineRuntime,
    n: u32,
    modal_policy: &mut crate::http_server::StepModalPolicy,
    mut session_modals: Option<&mut super::session_policy::SessionModalScheduler>,
) -> Result<(u32, Vec<crate::http_server::HttpModalDismissal>), String> {
    let mut advanced = 0;
    let mut dismissed = Vec::new();
    for _ in 0..n {
        // Stepping into a save-marker / load-back frame must pin or swap
        // state exactly like the normal playback admission path.
        if timeline
            .replay_player
            .as_ref()
            .is_some_and(|player| !player.is_finished())
        {
            let player = timeline.replay_player.as_ref().expect("active replay");
            let ordinal = player.current_frame();
            let loads_state = player.load_back_for_frame(ordinal).is_some();
            if let Some(scheduler) = session_modals.as_deref_mut() {
                scheduler.checkpoint(ordinal, &host.effects);
            }
            timeline.apply_playback_timeline_events(host, game, manager, assets)?;
            if loads_state && let Some(scheduler) = session_modals.as_deref_mut() {
                scheduler.after_load_back();
            }
        }
        let frame = timeline.frame_number();
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

        let mut recorded_modals = super::session_policy::ReplayModalDismissals::default();
        let (replay_input, replay_timeline_after) = match timeline
            .consume_replay_frame_for_step()?
        {
            super::runtime::ReplayStepAdmission::NoActiveReplay => (None, None),
            super::runtime::ReplayStepAdmission::Recorded(recorded) => {
                // A session adapter retains active batch ownership during
                // recorded forward steps. Graphical debugger scrubbing keeps
                // its explicit presentation-discontinuity policy without it.
                if session_modals.is_some() {
                    recorded_modals.begin_replay_frame();
                    for control in recorded.host_controls {
                        match control {
                            robin_engine::replay::ReplayHostControl::ModalDismiss {
                                modal,
                                result,
                            } => recorded_modals.push_back(PlayerCommand::ModalDismiss {
                                kind: modal,
                                result,
                            }),
                        }
                    }
                }
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
        let mut transaction =
            timeline.open_manual_frame(engine, buffered_frame.is_none() && replay_input.is_none());
        transaction.run_hourglass &= game.should_run_hourglass(
            false,
            !game
                .operation
                .is(robin_engine::game_operation::GameCode::LevelInProgress),
            false,
        );
        timeline.rewind_buffer.begin_frame(frame, engine, assets);
        // Force-unpaused tick.  Same as the live-frame path at the
        // top of `run_mission`'s tick block, minus the paused /
        // rewind_active gating — stepping while paused is the whole
        // point of the endpoint.
        let mut display = std::mem::take(&mut host.frontend.engine_display);
        let simulation_frame = match (buffered_frame.clone(), replay_input) {
            (_, Some(recorded)) => recorded,
            (Some(buffered), None) => buffered,
            (None, None) => transaction.authoritative_input(),
        };
        transaction.adopt_authoritative_input(simulation_frame.clone());
        // Buffered scrubbing is not a new live record. The linear recorder
        // stays at its frontier until that retained future has been replayed.
        // TODO: recording a new branch while rewound requires an explicit
        // raw-checkpoint transition, not the save/load projection protocol.
        timeline.begin_recording(
            &mut transaction,
            timeline.replay_player.is_none() && buffered_frame.is_none(),
        );
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
        host.frontend.engine_display = display;
        let after = replay_timeline_after.unwrap_or_else(|| timeline.current_frame().next());
        if buffered_frame.is_none() && after.number() > frame {
            timeline.rewind_buffer.end_frame_input(simulation_frame);
            if let Some(checker) = timeline.rollback_checker.as_mut() {
                checker.check_after_commit(host, &timeline.rewind_buffer, engine);
            }
        }
        advanced += after
            .number()
            .checked_sub(frame)
            .expect("admitted step must not move backwards within its transaction");
        if replay_timeline_after.is_some() {
            timeline.adopt_frame(after);
        } else {
            timeline.advance_frame();
        }
        transaction.commit_timeline_after(timeline.current_frame());
        refresh_authoritative_multiplayer_state(host, timeline.frame_number(), engine);

        // If the tick queued any modal, drop it silently and keep
        // going.  Without this the caller's `step N` would stop at
        // the first dialog and the next step request would do the
        // same dance — making `step 1000` advance only as far as
        // the first scripted dialog.
        let modal_result = if replay_timeline_after.is_some() && session_modals.is_some() {
            let scheduler = session_modals
                .as_deref_mut()
                .expect("session adapter checked");
            let mut accepted = Vec::new();
            while let Some(PlayerCommand::ModalDismiss { kind, result }) = scheduler.advance(
                &mut host.effects,
                &mut recorded_modals,
                super::session_policy::ModalDecisionSource::Recorded,
            ) {
                accepted.push(crate::http_server::HttpModalDismissal { kind, result });
                if recorded_modals.is_empty() {
                    break;
                }
            }
            recorded_modals.assert_consumed();
            Ok(accepted)
        } else if modal_state_pending(host) {
            resolve_http_step_modals(host, None, modal_policy)
        } else {
            Ok(Vec::new())
        };
        if let Ok(accepted) = &modal_result {
            transaction
                .modal_dismissals
                .extend(
                    accepted
                        .iter()
                        .map(|dismissal| PlayerCommand::ModalDismiss {
                            kind: dismissal.kind.clone(),
                            result: dismissal.result,
                        }),
                );
        }
        // Even a strict modal policy which stops this request has already
        // executed the tick. Persist it before returning that error.
        timeline.finish_recording(&mut transaction);
        dismissed.extend(modal_result?);
        if let (Some(scheduler), Some(player)) = (
            session_modals.as_deref_mut(),
            timeline.replay_player.as_ref(),
        ) {
            scheduler.checkpoint(player.current_frame(), &host.effects);
        }
    }
    Ok((advanced, dismissed))
}

/// Rewind to `target`, restoring rollback state from the rewind
/// buffer and syncing the replay cursor if one is active.
/// Returns the frame we rewound from on success.
fn rewind_with_session_modals(
    manager: &mut engine_manager_api::EngineManager,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    timeline: &mut super::runtime::TimelineRuntime,
    target: u32,
    session_modals: Option<&mut super::session_policy::SessionModalScheduler>,
) -> Result<u32, String> {
    // Resolve the player's exact dense ordinal before changing Engine or Host.
    // seek_timeline_frame itself has no side effects beyond its cursor, which
    // is restored immediately even when checkpoint validation fails.
    let restore_ordinal = if let (Some(scheduler), Some(player)) =
        (session_modals.as_ref(), timeline.replay_player.as_mut())
    {
        let original = player.current_frame();
        let resolved = player.seek_timeline_frame(super::runtime::TimelineFrame::from_wire(target));
        player.seek_ordinal(super::runtime::ReplayFrameOrdinal::from_wire(original));
        let ordinal = resolved?.number();
        scheduler.validate_restore(ordinal)?;
        Some(ordinal)
    } else {
        None
    };
    let from = rewind_to_frame(manager, host, assets, timeline, target)?;
    if let (Some(ordinal), Some(scheduler)) = (restore_ordinal, session_modals) {
        assert_eq!(
            timeline
                .replay_player
                .as_ref()
                .expect("seek replay")
                .current_frame(),
            ordinal
        );
        scheduler.restore(ordinal, &mut host.effects);
    }
    Ok(from)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn rewind_to_frame(
    manager: &mut engine_manager_api::EngineManager,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    timeline: &mut super::runtime::TimelineRuntime,
    target: u32,
) -> Result<u32, String> {
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
    refresh_authoritative_multiplayer_state(host, timeline.frame_number(), &manager.engine);
    Ok(from)
}

/// Keep reconnect admission and input stamping aligned with debugger-driven
/// timeline movement. Manual HTTP steps bypass the normal outer-frame commit,
/// which otherwise refreshes both pieces of host-authoritative network state.
fn refresh_authoritative_multiplayer_state(host: &Host, frame: u32, engine: &engine_api::Engine) {
    if let Some(net) = host.transport.net()
        && host.transport.local_seat() == PlayerId::HOST
    {
        net.publish_frame(frame);
        net.set_initial_snapshot(frame, engine);
    }
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

fn default_http_modal_result(
    kind: &robin_engine::player_command::ModalKind,
) -> robin_engine::player_command::DialogResult {
    use robin_engine::player_command::DialogResult;
    match kind {
        // Automation's historical behavior was to continue informational
        // screens and decline/abort choice screens.
        robin_engine::player_command::ModalKind::Dialog { .. }
        | robin_engine::player_command::ModalKind::PopupText { .. }
        | robin_engine::player_command::ModalKind::SherwoodReport
        | robin_engine::player_command::ModalKind::Debriefing { .. }
        | robin_engine::player_command::ModalKind::FinalDebriefing { .. } => {
            DialogResult::Completed
        }
        robin_engine::player_command::ModalKind::MissionState { .. } => DialogResult::Aborted,
    }
}

use super::session_policy::validate_modal_result as validate_http_modal_result;

/// Apply multiplayer authority to HTTP-supplied modal outcomes before any
/// local presentation state is changed. A host publishes the same decision
/// its own UI would publish; a client can only submit an advisory proposal and
/// must keep the modal open until that decision comes back from the host.
fn authorize_http_modal_dismissals(
    host: &Host,
    dismissals: &[crate::http_server::HttpModalDismissal],
) -> Result<(), String> {
    let Some(net) = host.transport.net() else {
        return Ok(());
    };

    if host.transport.local_seat() == PlayerId::HOST {
        for dismissal in dismissals {
            let instance = net.open_modal_instance(&dismissal.kind)?;
            net.decide_modal_dismiss(instance, dismissal.kind.clone(), dismissal.result)?;
            net.complete_modal_instance(&dismissal.kind, instance)?;
        }
        Ok(())
    } else {
        for dismissal in dismissals {
            let instance = net.open_modal_instance(&dismissal.kind)?;
            net.propose_modal_dismiss(instance, dismissal.kind.clone(), dismissal.result)?;
        }
        Err(format!(
            "blocked by host-authoritative multiplayer modal; submitted {} proposal(s) and left local modal state unchanged",
            dismissals.len()
        ))
    }
}

fn resolve_http_step_modals(
    host: &mut Host,
    active_modal: Option<&mut Option<ActiveModal>>,
    policy: &mut crate::http_server::StepModalPolicy,
) -> Result<Vec<crate::http_server::HttpModalDismissal>, String> {
    use robin_engine::player_command::{MissionStateModalKind, ModalKind};

    let mut pending = host.effects.pending_modal_kinds();
    if host.effects.has_signal(HostSignal::MissionStatePopup) {
        pending.push(ModalKind::MissionState {
            kind: MissionStateModalKind::LeaveMissionNow,
        });
    }
    if let Some(active) = active_modal.as_deref()
        && let Some(kind) = active.as_ref().and_then(ActiveModal::kind)
    {
        pending.push(kind);
    }
    if pending.is_empty() {
        return Ok(Vec::new());
    }

    let mut accepted = Vec::with_capacity(pending.len());
    for kind in pending {
        let explicit = policy
            .dismissals
            .iter()
            .position(|dismissal| dismissal.kind == kind)
            .map(|index| policy.dismissals.remove(index).result);
        let result = explicit
            .or_else(|| policy.auto_dismiss.then(|| default_http_modal_result(&kind)))
            .ok_or_else(|| {
                format!(
                    "blocked by modal {}; retry with auto_dismiss=true or a matching typed dismissal",
                    serde_json::to_string(&kind).expect("ModalKind serializes")
                )
            })?;
        validate_http_modal_result(&kind, result)?;
        accepted.push(crate::http_server::HttpModalDismissal { kind, result });
    }

    authorize_http_modal_dismissals(host, &accepted)?;
    dismiss_pending_modals(host);
    if let Some(active) = active_modal {
        active.take();
    }
    Ok(accepted)
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
        ReplayPlayer::new(
            one_frame_replay_file(input)
                .try_into()
                .expect("valid replay fixture"),
        )
    }

    fn one_frame_replay_file(input: engine_api::SimulationFrameInput) -> ReplayFile {
        ReplayFile {
            header: ReplayHeader {
                mission_id: "step-test".into(),
                mission_assets: robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                    "step-test",
                    "step-test",
                    "step-test",
                )
                .unwrap(),
                rng_seed: 0,
                sim_config: engine_api::SimConfig::default(),
                spellforge_package: None,
                version: REPLAY_SCHEMA_VERSION,
                total_frames: 1,
                rankability: robin_engine::replay_rankability::ReplayRankability::rankable(),
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
    fn complete_normal_frame_then_manual_ticks_are_live_recorded_and_reconstructible() {
        use super::super::runtime::{FrameCommitPolicy, MissionFrame};
        use robin_engine::replay::{ReplayData, ReplayRecorder, state_hash};

        let (assets, mut manager, mut host, mut dev, mut game, mut timeline) =
            stepping_fixture(None);
        let initial = manager.engine.clone();
        let file = tempfile::NamedTempFile::new().unwrap();
        let path = file.path().to_string_lossy().into_owned();
        timeline.replay_recorder = Some(
            ReplayRecorder::new(
                &path,
                "step-test".into(),
                robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                    "step-test",
                    "step-test",
                    "step-test",
                )
                .unwrap(),
                0,
                engine_api::SimConfig::default(),
                &Campaign::default(),
            )
            .unwrap(),
        );

        // Match the interactive boundary: capture BEFORE input, execute the
        // normal tick, include its late post-refresh command, then commit
        // history AND its live recorder token before servicing the HTTP step.
        let mut normal = MissionFrame::new(0);
        timeline.open_frame(&mut normal, &manager.engine, &assets);
        timeline.begin_recording(&mut normal, true);
        normal.post_commands.push(PlayerInput::new(
            PlayerId::HOST,
            PlayerCommand::SetLockAlt(true),
        ));
        let mut display = std::mem::take(&mut host.frontend.engine_display);
        timeline.run_simulation(|| {
            game.run_engine_tick(
                &mut host,
                &mut display,
                &assets,
                &mut manager.engine,
                &mut dev,
                normal.authoritative_input(),
                false,
                false,
            )
        });
        host.frontend.engine_display = display;
        timeline.advance_frame();
        normal.commit_timeline_after(timeline.current_frame());
        timeline.commit_simulation_history(
            &mut host,
            &mut manager,
            &normal,
            FrameCommitPolicy {
                store_rewind_commands: true,
            },
        );
        timeline.finish_recording(&mut normal);
        host.effects.extend_dialogues([7]);
        let accepted = resolve_http_step_modals(&mut host, None, &mut Default::default()).unwrap();
        timeline.record_manual_host_controls(
            &manager.engine,
            accepted
                .iter()
                .map(|dismissal| PlayerCommand::ModalDismiss {
                    kind: dismissal.kind.clone(),
                    result: dismissal.result,
                })
                .collect(),
        );
        run_forward_ticks(
            &mut manager,
            &mut host,
            &assets,
            &mut dev,
            &mut game,
            &mut timeline,
            4,
            &mut Default::default(),
        )
        .unwrap();
        assert_eq!(timeline.frame_number(), 5);
        assert_eq!(timeline.rewind_buffer.next_record_frame(), 5);
        let expected = state_hash(&manager.engine);
        // Existing backwards controls remain available during recording.
        // Replaying the retained future must not append old timeline frames
        // to the recorder's already-committed dense ordinal frontier.
        rewind_to_frame(&mut manager, &mut host, &assets, &mut timeline, 4).unwrap();
        host.effects.extend_dialogues([8]);
        let accepted = resolve_http_step_modals(&mut host, None, &mut Default::default()).unwrap();
        timeline.record_manual_host_controls(
            &manager.engine,
            accepted
                .iter()
                .map(|dismissal| PlayerCommand::ModalDismiss {
                    kind: dismissal.kind.clone(),
                    result: dismissal.result,
                })
                .collect(),
        );
        // A zero-tick request can dismiss UI while scrubbing, but cannot
        // append a backwards stationary record into the live stream.
        run_forward_ticks(
            &mut manager,
            &mut host,
            &assets,
            &mut dev,
            &mut game,
            &mut timeline,
            0,
            &mut Default::default(),
        )
        .unwrap();
        assert_eq!(host.effects.dialogue_count(), 0);
        run_forward_ticks(
            &mut manager,
            &mut host,
            &assets,
            &mut dev,
            &mut game,
            &mut timeline,
            1,
            &mut Default::default(),
        )
        .unwrap();
        assert_eq!(state_hash(&manager.engine), expected);
        drop(timeline.replay_recorder.take());
        let replay = ReplayData::from_file(&path).unwrap();
        assert_eq!(replay.frame_count(), 6);
        assert!(!replay.frame(0).unwrap().input.post_commands.is_empty());
        let modal_boundary = replay.frame(1).unwrap();
        assert_eq!(
            (
                modal_boundary.timeline_before,
                modal_boundary.timeline_after
            ),
            (1, 1)
        );
        assert_eq!(modal_boundary.host_controls.len(), 1);
        assert!(!modal_boundary.input.run_hourglass);
        assert!(!modal_boundary.input.run_post_initialize);
        for ordinal in [0, 2, 3, 4, 5] {
            let frame = replay.frame(ordinal).unwrap();
            let tick = if ordinal == 0 { 0 } else { ordinal - 1 };
            assert_eq!(
                (frame.timeline_before, frame.timeline_after),
                (tick, tick + 1)
            );
            assert_eq!(
                serde_json::to_value(&frame.input).unwrap(),
                serde_json::to_value(timeline.rewind_buffer.frame_for(tick).unwrap()).unwrap()
            );
        }

        let (
            _,
            mut playback_manager,
            mut playback_host,
            mut playback_dev,
            mut playback_game,
            mut playback,
        ) = stepping_fixture(Some(ReplayPlayer::new(replay)));
        playback_manager.engine = initial;
        run_forward_ticks(
            &mut playback_manager,
            &mut playback_host,
            &assets,
            &mut playback_dev,
            &mut playback_game,
            &mut playback,
            6,
            &mut Default::default(),
        )
        .unwrap();
        assert_eq!(state_hash(&playback_manager.engine), expected);
        rewind_to_frame(&mut manager, &mut host, &assets, &mut timeline, 4).unwrap();
        run_forward_ticks(
            &mut manager,
            &mut host,
            &assets,
            &mut dev,
            &mut game,
            &mut timeline,
            1,
            &mut Default::default(),
        )
        .unwrap();
        assert_eq!(state_hash(&manager.engine), expected);
    }

    #[test]
    fn strict_modal_stop_keeps_the_executed_live_tick_in_the_recorder() {
        use robin_engine::replay::{ReplayData, ReplayRecorder};
        let (assets, mut manager, mut host, mut dev, mut game, mut timeline) =
            stepping_fixture(None);
        let file = tempfile::NamedTempFile::new().unwrap();
        let path = file.path().to_string_lossy().into_owned();
        timeline.replay_recorder = Some(
            ReplayRecorder::new(
                &path,
                "step-test".into(),
                robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                    "step-test",
                    "step-test",
                    "step-test",
                )
                .unwrap(),
                0,
                engine_api::SimConfig::default(),
                &Campaign::default(),
            )
            .unwrap(),
        );
        // Model a modal encountered at the tick's post-dispatch boundary.
        // The strict policy leaves it pending, but cannot unexecute the tick.
        host.effects.extend_dialogues([7]);
        let mut policy = crate::http_server::StepModalPolicy {
            auto_dismiss: false,
            ..Default::default()
        };
        let error = run_forward_ticks(
            &mut manager,
            &mut host,
            &assets,
            &mut dev,
            &mut game,
            &mut timeline,
            4,
            &mut policy,
        )
        .unwrap_err();
        assert!(error.contains("blocked by modal"));
        assert_eq!(timeline.frame_number(), 1);
        assert_eq!(host.effects.dialogue_count(), 1);
        policy.auto_dismiss = true;
        run_forward_ticks(
            &mut manager,
            &mut host,
            &assets,
            &mut dev,
            &mut game,
            &mut timeline,
            1,
            &mut policy,
        )
        .unwrap();
        drop(timeline.replay_recorder.take());
        let replay = ReplayData::from_file(&path).unwrap();
        assert_eq!(replay.frame_count(), 2);
        assert!(replay.frame(0).unwrap().host_controls.is_empty());
        assert_eq!(replay.frame(1).unwrap().host_controls.len(), 1);
        assert_eq!(replay.frame(1).unwrap().timeline_after, 2);
    }

    #[test]
    fn recorded_forward_step_uses_existing_session_batch_across_stationary_dismissal() {
        use super::super::session_policy::{
            ModalDecisionSource, ReplayModalDismissals, SessionModalScheduler,
        };
        use robin_engine::player_command::{DialogResult, ModalKind};
        let mut file = one_frame_replay_file(engine_api::SimulationFrameInput::default());
        file.header.total_frames = 2;
        file.frames.insert(
            0,
            ReplayFrame {
                timeline_before: 0,
                timeline_after: 0,
                input: engine_api::SimulationFrameInput::default()
                    .with_hourglass(false)
                    .with_post_initialize(false),
                host_controls: vec![robin_engine::replay::ReplayHostControl::ModalDismiss {
                    modal: ModalKind::PopupText { text_id: 7 },
                    result: DialogResult::Completed,
                }],
            },
        );
        file.frames.insert(
            1,
            ReplayFrame {
                timeline_before: 0,
                timeline_after: 1,
                input: engine_api::SimulationFrameInput::default().with_post_initialize(true),
                host_controls: Vec::new(),
            },
        );
        let player = ReplayPlayer::new(file.try_into().unwrap());
        let (assets, mut manager, mut host, mut dev, mut game, mut timeline) =
            stepping_fixture(Some(player));
        let mut scheduler = SessionModalScheduler::default();
        host.effects.extend_popup_texts([7]);
        scheduler.advance(
            &mut host.effects,
            &mut ReplayModalDismissals::default(),
            ModalDecisionSource::Recorded,
        );
        assert!(scheduler.is_active());
        let before_tick = manager.engine.simulation_tick().number();
        let mut strict = crate::http_server::StepModalPolicy {
            auto_dismiss: false,
            dismissals: Vec::new(),
            ..Default::default()
        };
        let stationary = run_forward_ticks_with_session_modals(
            &mut manager,
            &mut host,
            &assets,
            &mut dev,
            &mut game,
            &mut timeline,
            1,
            &mut strict,
            Some(&mut scheduler),
        )
        .unwrap();
        assert_eq!(stationary.0, 0);
        assert_eq!(stationary.1.len(), 1);
        assert_eq!(manager.engine.simulation_tick().number(), before_tick);
        assert_eq!(timeline.frame_number(), 0);
        assert!(!scheduler.is_active());
        let forward = run_forward_ticks_with_session_modals(
            &mut manager,
            &mut host,
            &assets,
            &mut dev,
            &mut game,
            &mut timeline,
            1,
            &mut strict,
            Some(&mut scheduler),
        )
        .unwrap();
        assert_eq!(forward.0, 1);
        assert!(forward.1.is_empty());
        assert_eq!(timeline.replay_player.as_ref().unwrap().current_frame(), 2);
        let mut missing = SessionModalScheduler::default();
        let tick_before_failed_seek = manager.engine.simulation_tick();
        assert!(
            rewind_with_session_modals(
                &mut manager,
                &mut host,
                &assets,
                &mut timeline,
                0,
                Some(&mut missing)
            )
            .unwrap_err()
            .contains("checkpoint")
        );
        assert_eq!(timeline.frame_number(), 1);
        assert_eq!(manager.engine.simulation_tick(), tick_before_failed_seek);
        assert_eq!(timeline.replay_player.as_ref().unwrap().current_frame(), 2);

        // A future captured same-ID popup must not replace the earlier batch,
        // and a newly queued future dialogue must not survive the seek.
        host.effects.extend_popup_texts([7]);
        scheduler.advance(
            &mut host.effects,
            &mut ReplayModalDismissals::default(),
            ModalDecisionSource::Recorded,
        );
        host.effects.extend_dialogues([99]);
        rewind_with_session_modals(
            &mut manager,
            &mut host,
            &assets,
            &mut timeline,
            0,
            Some(&mut scheduler),
        )
        .unwrap();
        assert_eq!(timeline.replay_player.as_ref().unwrap().current_frame(), 0);
        assert!(scheduler.is_active());
        assert!(host.effects.pending_modal_kinds().is_empty());
        let repeated = run_forward_ticks_with_session_modals(
            &mut manager,
            &mut host,
            &assets,
            &mut dev,
            &mut game,
            &mut timeline,
            2,
            &mut strict,
            Some(&mut scheduler),
        )
        .unwrap();
        assert_eq!(repeated.0, 1);
        assert_eq!(repeated.1.len(), 1);
        assert!(!scheduler.is_active());
    }

    #[test]
    fn paused_replay_record_does_not_replace_or_append_a_simulation_tick() {
        let mut file = one_frame_replay_file(engine_api::SimulationFrameInput::default());
        file.header.total_frames = 2;
        file.frames.insert(
            0,
            ReplayFrame {
                timeline_before: 0,
                timeline_after: 0,
                input: engine_api::SimulationFrameInput::default()
                    .with_hourglass(false)
                    .with_post_initialize(false),
                host_controls: Vec::new(),
            },
        );
        file.frames.insert(
            1,
            ReplayFrame {
                timeline_before: 0,
                timeline_after: 1,
                input: engine_api::SimulationFrameInput::default().with_post_initialize(true),
                host_controls: Vec::new(),
            },
        );
        let player = ReplayPlayer::new(file.try_into().unwrap());
        let (assets, mut manager, mut host, mut dev, mut game, mut timeline) =
            stepping_fixture(Some(player));
        let result = run_forward_ticks(
            &mut manager,
            &mut host,
            &assets,
            &mut dev,
            &mut game,
            &mut timeline,
            2,
            &mut Default::default(),
        )
        .unwrap();
        assert_eq!(result.0, 1);
        assert_eq!(timeline.frame_number(), 1);
        assert_eq!(timeline.rewind_buffer.next_record_frame(), 1);
        assert_eq!(timeline.replay_player.as_ref().unwrap().current_frame(), 2);
        assert!(timeline.rewind_buffer.frame_for(0).unwrap().run_hourglass);
    }

    #[test]
    fn non_replay_forward_step_keeps_live_debugger_behavior() {
        let (assets, mut manager, mut host, mut dev, mut game, mut timeline) =
            stepping_fixture(None);
        let mut modal_policy = crate::http_server::StepModalPolicy::default();

        let (advanced, dismissed) = run_forward_ticks(
            &mut manager,
            &mut host,
            &assets,
            &mut dev,
            &mut game,
            &mut timeline,
            1,
            &mut modal_policy,
        )
        .expect("live debugger step");

        assert_eq!(advanced, 1);
        assert!(dismissed.is_empty());
        assert_eq!(timeline.frame_number(), 1);
        let input = timeline
            .rewind_buffer
            .frame_for(0)
            .expect("live step recorded in rewind history");
        assert!(input.run_post_initialize);
    }

    #[test]
    fn multiplayer_host_manual_steps_refresh_reconnect_state() {
        let (assets, mut manager, mut host, mut dev, mut game, mut timeline) =
            stepping_fixture(None);
        let (net, _incoming, _outgoing, frame_cursor, initial_snapshot) =
            crate::multiplayer::NetChannels::new();
        host.transport = crate::host::HostTransport::test_session(net, PlayerId::HOST);
        let mut modal_policy = crate::http_server::StepModalPolicy::default();

        run_forward_ticks(
            &mut manager,
            &mut host,
            &assets,
            &mut dev,
            &mut game,
            &mut timeline,
            1,
            &mut modal_policy,
        )
        .expect("multiplayer host forward step");

        assert_eq!(frame_cursor.load(std::sync::atomic::Ordering::Relaxed), 1);
        {
            let snapshot = initial_snapshot.lock().expect("initial snapshot lock");
            let (frame, engine_bytes) = snapshot.as_ref().expect("forward-step snapshot");
            assert_eq!(*frame, 1);
            assert_eq!(
                engine_bytes.as_slice(),
                manager.engine.encode_native_snapshot().as_slice()
            );
        }

        rewind_to_frame(&mut manager, &mut host, &assets, &mut timeline, 0)
            .expect("multiplayer host rewind");

        assert_eq!(frame_cursor.load(std::sync::atomic::Ordering::Relaxed), 0);
        let snapshot = initial_snapshot.lock().expect("initial snapshot lock");
        let (frame, engine_bytes) = snapshot.as_ref().expect("rewind snapshot");
        assert_eq!(*frame, 0);
        assert_eq!(
            engine_bytes.as_slice(),
            manager.engine.encode_native_snapshot().as_slice()
        );
    }

    #[test]
    fn multiplayer_rejects_local_pause_and_requires_explicit_host_sync() {
        use super::super::runtime::MultiplayerAdmission;
        let (net, _incoming, _outgoing, _cursor, _snapshot) =
            crate::multiplayer::NetChannels::new();
        let mut host = Host::default();
        host.transport = crate::host::HostTransport::test_session(net, PlayerId::HOST);

        let pause_error = validate_multiplayer_step_request(
            &host,
            MultiplayerAdmission::Running,
            &crate::http_server::StepKind::SetPaused { paused: true },
        )
        .expect_err("one peer must not pause a multiplayer timeline");
        assert!(pause_error.contains("manual pause is disabled"));

        let ordinary = crate::http_server::StepKind::Forward {
            n: 1,
            modal_policy: crate::http_server::StepModalPolicy::default(),
        };
        let ordinary_error =
            validate_multiplayer_step_request(&host, MultiplayerAdmission::Running, &ordinary)
                .expect_err("ordinary multiplayer stepping must be rejected");
        assert!(ordinary_error.contains("synchronized_multiplayer=true"));

        let synchronized = crate::http_server::StepKind::Forward {
            n: 1,
            modal_policy: crate::http_server::StepModalPolicy {
                synchronized_multiplayer: true,
                ..Default::default()
            },
        };
        validate_multiplayer_step_request(&host, MultiplayerAdmission::Running, &synchronized)
            .expect("the host may explicitly synchronize automation");
        for admission in [
            MultiplayerAdmission::HostWaitingForBegin,
            MultiplayerAdmission::WaitingForStart {
                frame: 0,
                start_epoch_ms: 1,
            },
            MultiplayerAdmission::HostWaitingForResyncBegin { snapshot_frame: 4 },
        ] {
            assert!(
                validate_multiplayer_step_request(&host, admission, &synchronized)
                    .unwrap_err()
                    .contains("wait for the ready barrier")
            );
        }

        host.transport.test_local_seat(PlayerId(1));
        let client_error =
            validate_multiplayer_step_request(&host, MultiplayerAdmission::Running, &synchronized)
                .expect_err("a client must never own timeline movement");
        assert!(client_error.contains("multiplayer clients"));
    }

    #[test]
    fn http_step_without_auto_dismiss_reports_typed_modal_blocker() {
        let mut host = Host::default();
        host.effects.extend_dialogues([7]);
        let mut policy = crate::http_server::StepModalPolicy {
            auto_dismiss: false,
            dismissals: Vec::new(),
            synchronized_multiplayer: false,
        };
        let error = resolve_http_step_modals(&mut host, None, &mut policy)
            .expect_err("unanswered modal must block");
        assert!(error.contains("blocked by modal"));
        assert!(error.contains("dialog_id"));
        assert_eq!(host.effects.dialogue_count(), 1);
    }

    #[test]
    fn http_step_accepts_matching_typed_modal_result() {
        use robin_engine::player_command::{DialogResult, ModalKind};

        let mut host = Host::default();
        host.effects.extend_dialogues([7]);
        let expected = crate::http_server::HttpModalDismissal {
            kind: ModalKind::Dialog { dialog_id: 7 },
            result: DialogResult::Aborted,
        };
        let mut policy = crate::http_server::StepModalPolicy {
            auto_dismiss: false,
            dismissals: vec![expected.clone()],
            synchronized_multiplayer: false,
        };
        let accepted = resolve_http_step_modals(&mut host, None, &mut policy)
            .expect("matching typed dismissal");
        assert_eq!(accepted, vec![expected]);
        assert!(policy.dismissals.is_empty(), "typed outcomes are one-shot");
        assert_eq!(host.effects.dialogue_count(), 0);
    }

    #[test]
    fn multiplayer_client_http_step_proposes_but_cannot_dismiss_modal() {
        use crate::multiplayer::{NetChannels, NetOutbound};
        use robin_engine::player_command::{DialogResult, ModalKind, PlayerId};

        let (net, _incoming, outgoing, _cursor, _snapshot) = NetChannels::new();
        net.install_session_id(crate::multiplayer::MultiplayerSessionId([1; 32]))
            .unwrap();
        let mut host = Host::default();
        host.transport = crate::host::HostTransport::test_session(net, PlayerId(1));
        host.effects.extend_dialogues([7]);
        let expected = crate::http_server::HttpModalDismissal {
            kind: ModalKind::Dialog { dialog_id: 7 },
            result: DialogResult::Completed,
        };
        let mut policy = crate::http_server::StepModalPolicy::default();

        let error = resolve_http_step_modals(&mut host, None, &mut policy)
            .expect_err("client HTTP endpoint is not modal authority");

        assert!(error.contains("host-authoritative multiplayer modal"));
        assert_eq!(host.effects.dialogue_count(), 1);
        assert!(matches!(
            outgoing.try_recv().expect("advisory proposal"),
            NetOutbound::ModalProposal { kind, result, .. }
                if kind == expected.kind && result == expected.result
        ));
    }

    #[test]
    fn multiplayer_host_http_step_broadcasts_decision_before_dismissal() {
        use crate::multiplayer::{NetChannels, NetOutbound};
        use robin_engine::player_command::{DialogResult, ModalKind, PlayerId};

        let (net, _incoming, outgoing, _cursor, _snapshot) = NetChannels::new();
        net.install_session_id(crate::multiplayer::MultiplayerSessionId([2; 32]))
            .unwrap();
        let mut host = Host::default();
        host.transport = crate::host::HostTransport::test_session(net, PlayerId::HOST);
        host.effects.extend_popup_texts([9]);
        let expected = crate::http_server::HttpModalDismissal {
            kind: ModalKind::PopupText { text_id: 9 },
            result: DialogResult::Completed,
        };
        let mut policy = crate::http_server::StepModalPolicy::default();

        let accepted = resolve_http_step_modals(&mut host, None, &mut policy)
            .expect("host HTTP endpoint has modal authority");

        assert_eq!(accepted, vec![expected.clone()]);
        assert_eq!(host.effects.popup_text_count(), 0);
        assert!(matches!(
            outgoing.try_recv().expect("authoritative decision"),
            NetOutbound::ModalDecision { kind, result, .. }
                if kind == expected.kind && result == expected.result
        ));
    }

    #[test]
    fn http_step_rejects_a_result_invalid_for_the_modal_kind() {
        use robin_engine::player_command::{DialogResult, ModalKind};

        let mut host = Host::default();
        host.effects.extend_popup_texts([9]);
        let mut policy = crate::http_server::StepModalPolicy {
            auto_dismiss: false,
            dismissals: vec![crate::http_server::HttpModalDismissal {
                kind: ModalKind::PopupText { text_id: 9 },
                result: DialogResult::Restart,
            }],
            synchronized_multiplayer: false,
        };
        let error = resolve_http_step_modals(&mut host, None, &mut policy)
            .expect_err("single-button popup cannot restart a mission");
        assert!(error.contains("cannot accept result"));
        assert_eq!(host.effects.popup_text_count(), 1);
    }

    #[test]
    fn replay_eof_refuses_to_fabricate_another_step() {
        let recorded_input = engine_api::SimulationFrameInput::default();
        let player = one_frame_replay(recorded_input);
        let (assets, mut manager, mut host, mut dev, mut game, mut timeline) =
            stepping_fixture(Some(player));
        let mut modal_policy = crate::http_server::StepModalPolicy::default();

        let (advanced, _) = run_forward_ticks(
            &mut manager,
            &mut host,
            &assets,
            &mut dev,
            &mut game,
            &mut timeline,
            1,
            &mut modal_policy,
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
            &mut modal_policy,
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
            rewind_buffer.end_frame_input(robin_engine::engine::SimulationFrameInput::default());
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
        let mut modal_policy = crate::http_server::StepModalPolicy::default();

        let (advanced, _) = run_forward_ticks(
            &mut manager,
            &mut host,
            &assets,
            &mut dev,
            &mut game,
            &mut timeline,
            1,
            &mut modal_policy,
        )
        .expect("forward scrub should reuse frame 250");

        assert_eq!(advanced, 1);
        assert_eq!(timeline.frame_number(), 251);
        assert_eq!(timeline.rewind_buffer.next_record_frame(), 426);
    }
}
