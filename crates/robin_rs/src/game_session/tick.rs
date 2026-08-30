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
    manager: &mut engine_manager_api::EngineManager,
    host: &mut Host,
    backend: &mut KiraAudioBackend,
    sample_loader: &SampleLoader,
    sound_rng: &mut fastrand::Rng,
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
        .map(|resolved| robin_engine::sound::ResolvedExclamation {
            actor_id: resolved.actor_id,
            identifier: resolved.identifier,
            exclamation_id: resolved.exclamation_id,
            duration_frames: resolved.length_ms.saturating_add(39) / 40,
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
    sync_render_camera(host);
    crate::blit_to_map::drain_pending_bg_blits(host);
}

/// Refresh camera-derived draw parameters without consuming any fixed-tick
/// side-effect queues. Native-refresh interpolation calls this for each
/// sampled camera pose.
pub(super) fn sync_render_camera(host: &mut Host) {
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
/// Pending gameplay modals are resolved under the request's typed modal
/// policy. Automation defaults to a conservative auto-dismiss outcome; strict
/// drivers can disable it and provide one-shot `(ModalKind, DialogResult)`
/// answers. An unanswered modal blocks without mutating its queue. Replies
/// include every accepted typed outcome.
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
    mut terminal_debriefing: Option<&mut super::terminal_debriefing::TerminalDebriefingState>,
    mission_ui_block_reason: Option<&str>,
    mut resolve_local_ui: impl FnMut(&crate::http_server::StepModalPolicy) -> Result<(), String>,
) {
    let steps = crate::http_server::take_pending_steps();
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
        if let Err(error) = validate_multiplayer_step_request(host, &kind) {
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
            let modal_kind = terminal.current_kind();
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
                .and_then(|()| terminal.queue_http_result(modal_kind.clone(), result))
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
        let mut accepted_dismissals = if let Some(policy) = modal_policy.as_mut() {
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

        match kind {
            crate::http_server::StepKind::Forward { n, .. } => {
                let start = timeline.frame_number();
                let result = run_forward_ticks(
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
                );
                match result {
                    Ok((advanced, dismissed_during)) => {
                        accepted_dismissals.extend(dismissed_during);
                        if let Err(error) = begin_synchronized_step_resync(
                            host,
                            timeline.frame_number(),
                            &manager.engine,
                        ) {
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
                match rewind_to_frame(manager, host, assets, timeline, target) {
                    Ok(from) => {
                        if let Err(error) = begin_synchronized_step_resync(
                            host,
                            timeline.frame_number(),
                            &manager.engine,
                        ) {
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
                        match run_forward_ticks(
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
                    Ordering::Less => {
                        rewind_to_frame(manager, host, assets, timeline, target).map(|_| "back")
                    }
                };
                match resolve_http_step_modals(
                    host,
                    Some(active_modal),
                    modal_policy
                        .as_mut()
                        .expect("go-to-frame steps always carry a modal policy"),
                ) {
                    Ok(dismissed) => accepted_dismissals.extend(dismissed),
                    Err(error) if result.is_ok() => result = Err(error),
                    Err(_) => {}
                }
                match result {
                    Ok(kind) => {
                        if let Err(error) = begin_synchronized_step_resync(
                            host,
                            timeline.frame_number(),
                            &manager.engine,
                        ) {
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
    kind: &crate::http_server::StepKind,
) -> Result<(), String> {
    if host.transport.net.is_none() {
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
    if host.transport.local_seat != PlayerId::HOST {
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
    if host.transport.reconnecting {
        return Err(
            "multiplayer snapshot synchronization is still in progress; wait for the ready barrier"
                .to_string(),
        );
    }
    Ok(())
}

fn begin_synchronized_step_resync(
    host: &mut Host,
    frame: u32,
    engine: &engine_api::Engine,
) -> Result<(), String> {
    let Some(net) = host.transport.net.as_ref() else {
        return Ok(());
    };
    if host.transport.local_seat != PlayerId::HOST {
        return Err("only the multiplayer host can synchronize manual stepping".to_string());
    }
    net.set_initial_snapshot(frame, engine);
    net.reconnect_all_for_snapshot(format!(
        "host synchronized automation adopted timeline frame {frame}"
    ))?;
    net.send_ready_to_sim(frame);
    host.transport.reconnecting = true;
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
    let start = timeline.frame_number();
    let mut dismissed = Vec::new();
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
        if buffered_frame.is_none() {
            timeline.rewind_buffer.end_frame_input(simulation_frame);
            if let Some(checker) = timeline.rollback_checker.as_mut() {
                checker.check_after_commit(host, &timeline.rewind_buffer, engine);
            }
        }
        if let Some(after) = replay_timeline_after {
            timeline.adopt_frame(after);
        } else {
            timeline.advance_frame();
        }
        refresh_authoritative_multiplayer_state(host, timeline.frame_number(), engine);

        // If the tick queued any modal, drop it silently and keep
        // going.  Without this the caller's `step N` would stop at
        // the first dialog and the next step request would do the
        // same dance — making `step 1000` advance only as far as
        // the first scripted dialog.
        if modal_state_pending(host) {
            dismissed.extend(resolve_http_step_modals(host, None, modal_policy)?);
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
    if let Some(net) = host.transport.net.as_ref()
        && host.transport.local_seat == PlayerId::HOST
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

fn validate_http_modal_result(
    kind: &robin_engine::player_command::ModalKind,
    result: robin_engine::player_command::DialogResult,
) -> Result<(), String> {
    use robin_engine::player_command::{DialogResult, ModalKind};

    let valid = match kind {
        ModalKind::Dialog { .. }
        | ModalKind::Debriefing { .. }
        | ModalKind::MissionState { .. } => {
            matches!(result, DialogResult::Completed | DialogResult::Aborted)
        }
        ModalKind::PopupText { .. } | ModalKind::SherwoodReport => {
            result == DialogResult::Completed
        }
        ModalKind::FinalDebriefing { .. } => true,
    };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "modal {} cannot accept result {}",
            serde_json::to_string(kind).expect("ModalKind serializes"),
            serde_json::to_string(&result).expect("DialogResult serializes")
        ))
    }
}

/// Apply multiplayer authority to HTTP-supplied modal outcomes before any
/// local presentation state is changed. A host publishes the same decision
/// its own UI would publish; a client can only submit an advisory proposal and
/// must keep the modal open until that decision comes back from the host.
fn authorize_http_modal_dismissals(
    host: &Host,
    dismissals: &[crate::http_server::HttpModalDismissal],
) -> Result<(), String> {
    let Some(net) = host.transport.net.as_ref() else {
        return Ok(());
    };

    if host.transport.local_seat == PlayerId::HOST {
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
        host.transport.local_seat = PlayerId::HOST;
        host.transport.net = Some(net);
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
        let (net, _incoming, _outgoing, _cursor, _snapshot) =
            crate::multiplayer::NetChannels::new();
        let mut host = Host::default();
        host.transport.local_seat = PlayerId::HOST;
        host.transport.net = Some(net);

        let pause_error = validate_multiplayer_step_request(
            &host,
            &crate::http_server::StepKind::SetPaused { paused: true },
        )
        .expect_err("one peer must not pause a multiplayer timeline");
        assert!(pause_error.contains("manual pause is disabled"));

        let ordinary = crate::http_server::StepKind::Forward {
            n: 1,
            modal_policy: crate::http_server::StepModalPolicy::default(),
        };
        let ordinary_error = validate_multiplayer_step_request(&host, &ordinary)
            .expect_err("ordinary multiplayer stepping must be rejected");
        assert!(ordinary_error.contains("synchronized_multiplayer=true"));

        let synchronized = crate::http_server::StepKind::Forward {
            n: 1,
            modal_policy: crate::http_server::StepModalPolicy {
                synchronized_multiplayer: true,
                ..Default::default()
            },
        };
        validate_multiplayer_step_request(&host, &synchronized)
            .expect("the host may explicitly synchronize automation");

        host.transport.local_seat = PlayerId(1);
        let client_error = validate_multiplayer_step_request(&host, &synchronized)
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
        net.install_session_id(crate::multiplayer::MultiplayerSessionId([1; 16]))
            .unwrap();
        let mut host = Host::default();
        host.transport.local_seat = PlayerId(1);
        host.transport.net = Some(net);
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
        net.install_session_id(crate::multiplayer::MultiplayerSessionId([2; 16]))
            .unwrap();
        let mut host = Host::default();
        host.transport.local_seat = PlayerId::HOST;
        host.transport.net = Some(net);
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
