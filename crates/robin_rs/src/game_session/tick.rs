//! Per-frame tick orchestration: audio tick, pre/post-render engine
//! hooks, command drain + replay/rewind step, and dismiss helpers
//! for pending modals.

use super::modal_state::ActiveModal;
use crate::Host;
use crate::ai::AlertLevel;
use crate::game::Game;
use crate::game_render::clear_status_bar_flags;
use crate::player_command::{PlayerCommand, PlayerInput};
use crate::replay::ReplayPlayer;
use crate::rewind::RewindBuffer;
use crate::rollback_checker::RollbackChecker;
use crate::sdl_audio::SdlMixerBackend;
use crate::sound::AlertStatus;
use crate::sound_cache::SampleLoader;
use robin_engine::coordinates::MapBBox;
use robin_engine::engine as engine_api;
use robin_engine::engine_manager as engine_manager_api;

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
    backend: &mut SdlMixerBackend,
    sample_loader: &SampleLoader,
    sound_rng: &mut fastrand::Rng,
) {
    let alert_status = match manager.engine.ai_global().overall_alert_status {
        AlertLevel::Green => AlertStatus::Green,
        AlertLevel::Yellow => AlertStatus::Yellow,
        AlertLevel::Red => AlertStatus::Red,
    };
    // Disjoint-borrow split: `host.sound` and the side-effect queues
    // on host are separate fields.
    let mut pending_play_delayed_sources = std::mem::take(&mut host.pending_play_delayed_sources);
    // Drain sim-emitted sound commands that need access to
    // `engine.sound_sim.sources` (stashed on host by `apply_side_effects`).
    host.sync_sound_listener();
    if host.pending_resume_all_sources.take().is_some() {
        host.sound.resume_all_sound_sources(
            &manager.engine.sound_sim().sources,
            host.viewport.sound_listen_point(),
            host.viewport.zoom_factor,
        );
    }
    for idx in std::mem::take(&mut host.pending_activate_sources) {
        // Sim already flipped `src.active = true` inside
        // `perform_hourglass`; host only starts the audio channel.
        host.sound
            .activate_sound_source(&manager.engine.sound_sim().sources, idx);
    }
    for actor_id in std::mem::take(&mut host.pending_stop_exclamation_channels) {
        host.sound.stop_exclamation_channel_only(actor_id, backend);
    }
    for actor_id in std::mem::take(&mut host.pending_stop_exclamations) {
        host.sound.stop_exclamation(actor_id, backend);
    }
    host.sound.hourglass(
        backend,
        sample_loader,
        &mut |n| sound_rng.u32(0..n),
        alert_status,
        &manager.engine.sound_sim().sources,
        &mut pending_play_delayed_sources,
    );
    // The hourglass drains the queue; whatever it left behind
    // (nothing today, but defensive) goes back on host for next frame.
    host.pending_play_delayed_sources = pending_play_delayed_sources;
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
    manager: &mut engine_manager_api::EngineManager,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
) {
    clear_status_bar_flags(
        &mut manager.engine,
        &mut host.engine_display,
        &mut host.input,
        assets,
    );
}

/// Process every queued `/step-forward` / `/step-back` HTTP request,
/// replying to each with the post-step frame number.
///
/// Each forward step runs `n` full frame-equivalent ticks (the same
/// bookkeeping the main loop does on a normal unpaused frame: rollback
/// checker, rewind-buffer commit, `sim_frame += 1`).  Each back step
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
    rewind_buffer: &mut RewindBuffer,
    rollback_checker: &mut Option<RollbackChecker>,
    replay_player: &mut Option<ReplayPlayer>,
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
                let start = manager.sim_frame;
                let (advanced, dismissed_during) = run_forward_ticks(
                    manager,
                    host,
                    assets,
                    dev,
                    game,
                    rewind_buffer,
                    rollback_checker,
                    replay_player,
                    n,
                );
                modals_dismissed += dismissed_during;
                // Stepping bypasses the checker's begin_frame/end_frame
                // pairing, so its ring buffer is now stale relative to
                // the advanced engine.  Clear it — the checker resumes
                // populating on the next normal frame.
                if let Some(checker) = rollback_checker.as_mut() {
                    checker.reset();
                }
                step.respond_ok(serde_json::json!({
                    "direction": "forward",
                    "from_frame": start,
                    "frame": manager.sim_frame,
                    "advanced": advanced,
                    "modals_dismissed": modals_dismissed,
                }));
            }
            crate::http_server::StepKind::Back { n } => {
                let Some(target) = manager.sim_frame.checked_sub(n) else {
                    step.respond_err(format!(
                        "n={} exceeds current frame {}",
                        n, manager.sim_frame
                    ));
                    continue;
                };
                match rewind_to_frame(manager, host, assets, rewind_buffer, replay_player, target) {
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
                let from = manager.sim_frame;
                use std::cmp::Ordering;
                let result: Result<&'static str, String> = match target.cmp(&from) {
                    Ordering::Equal => Ok("noop"),
                    Ordering::Greater => {
                        let delta = target - from;
                        let (advanced, dismissed_during) = run_forward_ticks(
                            manager,
                            host,
                            assets,
                            dev,
                            game,
                            rewind_buffer,
                            rollback_checker,
                            replay_player,
                            delta,
                        );
                        modals_dismissed += dismissed_during;
                        if advanced < delta {
                            Err(format!(
                                "advanced {advanced} of {delta} frames before stepping stopped"
                            ))
                        } else {
                            Ok("forward")
                        }
                    }
                    Ordering::Less => {
                        rewind_to_frame(manager, host, assets, rewind_buffer, replay_player, target)
                            .map(|_| "back")
                    }
                };
                // The rollback checker's ring now references a timeline
                // the live engine is no longer on; clear it so the next
                // normal frame starts a fresh window.
                if let Some(checker) = rollback_checker.as_mut() {
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
                        "frame": manager.sim_frame,
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
                    "frame": manager.sim_frame,
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
    rewind_buffer: &mut RewindBuffer,
    rollback_checker: &mut Option<RollbackChecker>,
    replay_player: &mut Option<ReplayPlayer>,
    n: u32,
) -> (u32, usize) {
    let engine = &mut manager.engine;
    let sim_frame = &mut manager.sim_frame;
    let start = *sim_frame;
    let mut dismissed = 0usize;
    for _ in 0..n {
        let mut frame_cmds: Vec<PlayerInput> = Vec::new();
        if let Some(player) = replay_player.as_mut()
            && !player.is_finished()
        {
            for cmd in player.next_frame() {
                // `ModalDismiss` is recorded when the player clicked
                // through a dialog during the original session; we
                // drop it here because the engine's modal state may
                // not be in the same shape mid-scrub.
                if matches!(cmd.command, PlayerCommand::ModalDismiss { .. }) {
                    continue;
                }
                frame_cmds.push(cmd.clone());
            }
            engine.apply_commands(
                &mut host.engine_display,
                &mut host.input,
                assets,
                &frame_cmds,
            );
        }
        // Force-unpaused tick.  Same as the live-frame path at the
        // top of `run_mission`'s tick block, minus the paused /
        // rewind_active gating — stepping while paused is the whole
        // point of the endpoint.
        let mut display = std::mem::take(&mut host.engine_display);
        game.run_engine_tick(host, &mut display, assets, engine, dev, false, false);
        crate::sim_timeline::run_post_initialize_stage(host, &mut display, assets, engine, dev);
        host.engine_display = display;
        if let Some(checker) = rollback_checker.as_mut() {
            checker.end_frame(host, frame_cmds.clone(), engine);
        }
        rewind_buffer.end_frame(frame_cmds);
        *sim_frame += 1;

        // If the tick queued any modal, drop it silently and keep
        // going.  Without this the caller's `step N` would stop at
        // the first dialog and the next step request would do the
        // same dance — making `step 1000` advance only as far as
        // the first scripted dialog.
        if modal_state_pending(host) {
            dismissed += dismiss_pending_modals(host);
        }
    }
    (*sim_frame - start, dismissed)
}

/// Rewind to `target`, restoring rollback state from the rewind
/// buffer and syncing the replay cursor if one is active.
/// Returns the frame we rewound from on success.
#[allow(clippy::too_many_arguments)]
pub(super) fn rewind_to_frame(
    manager: &mut engine_manager_api::EngineManager,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    rewind_buffer: &mut RewindBuffer,
    replay_player: &mut Option<ReplayPlayer>,
    target: u32,
) -> Result<u32, String> {
    let _ = host; // reserved for future hooks (e.g. cursor reset on scrub)
    let Some(oldest) = rewind_buffer.oldest_reachable_frame() else {
        return Err("rewind buffer empty".into());
    };
    if target < oldest {
        return Err(format!(
            "target frame {target} is older than the oldest retained snapshot ({oldest})"
        ));
    }
    rewind_buffer.begin_session();
    let rewound = rewind_buffer.rewind_to(assets, target);
    rewind_buffer.end_session();
    let Some(new_engine) = rewound else {
        return Err("rewind_to failed (no matching snapshot)".into());
    };
    manager.engine = new_engine;
    let from = manager.sim_frame;
    manager.sim_frame = target;
    // Keep the replay cursor in sync with the rewound sim frame so
    // resuming playback re-applies the right commands.
    if let Some(player) = replay_player.as_mut() {
        player.seek(target);
    }
    Ok(from)
}

/// True iff the engine has queued a modal dialog / debriefing / scroll
/// / sherwood report that hasn't been shown yet.  Used to gate the
/// interactive step-forward/back hotkeys (they refuse while a modal is
/// pending).  The HTTP stepping path uses `dismiss_pending_modals`
/// instead — scripted drivers want the sim to keep advancing.
pub(super) fn modal_state_pending(host: &Host) -> bool {
    !host.pending_dialogues.is_empty()
        || !host.pending_popup_texts.is_empty()
        || !host.pending_debriefings.is_empty()
        || host.pending_sherwood_report
        || host.pending_mission_state_popup
}

/// Silently drop every queued modal on `host`.  Used by the HTTP
/// `/step-forward` and `/step-back` handlers so a scripted driver
/// never deadlocks on the blocking dialog/debriefing/popup UI — the
/// user explicitly asked for stepping to skip dialogs.  Returns the
/// number of modals that were dropped so the step reply can surface
/// it (mostly for debuggability: "why did my scripted driver miss the
/// briefing?" — because it was dismissed, here's the count).
pub(super) fn dismiss_pending_modals(host: &mut Host) -> usize {
    let n = host.pending_dialogues.len()
        + host.pending_popup_texts.len()
        + host.pending_debriefings.len()
        + host.pending_sherwood_report as usize
        + host.pending_mission_state_popup as usize;
    if n > 0 {
        tracing::debug!(
            "HTTP step: dismissing {} pending modal(s) \
             (dialogues={}, popups={}, debriefings={}, sherwood_report={}, mission_state={})",
            n,
            host.pending_dialogues.len(),
            host.pending_popup_texts.len(),
            host.pending_debriefings.len(),
            host.pending_sherwood_report,
            host.pending_mission_state_popup,
        );
    }
    host.pending_dialogues.clear();
    host.pending_popup_texts.clear();
    host.pending_debriefings.clear();
    host.pending_sherwood_report = false;
    host.pending_mission_state_popup = false;
    n
}
