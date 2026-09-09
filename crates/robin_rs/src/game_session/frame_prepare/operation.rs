//! Operation/save processing between live input and timeline admission.

use super::*;
use crate::game_session::flow::MissionExit;
use crate::game_session::interactive::MissionInput;
use crate::game_session::runtime::{FrameContractStage, TimelineRuntime};

/// Browser startup may defer only the nonurgent initial autosave thumbnail.
#[cfg(any(target_arch = "wasm32", test))]
fn can_defer_initial_autosave(
    frame: u32,
    reason: Option<crate::autosave::AutosaveReason>,
    pending_save_load: bool,
    backgrounded: bool,
    exiting: bool,
) -> bool {
    frame == 0
        && reason == Some(crate::autosave::AutosaveReason::MissionTransition)
        && !pending_save_load
        && !backgrounded
        && !exiting
}

/// Apply process-side input and banner effects produced by save/load I/O.
/// These intentionally remain after operation processing and before replay or
/// multiplayer command injection.
fn apply_post_save_ui_state(
    host: &mut Host,
    notices: &mut crate::main_entry::AutosaveNotices,
    game: &mut crate::game::Game,
    input: &mut MissionInput,
    outcome: &crate::main_entry::OperationOutcome,
) {
    if outcome.reset_input() {
        input.reset_after_engine_request(host);
    }
    if let Some(kind) = notices.select_banner(outcome.banner) {
        let text = match kind {
            SaveBannerKind::Saved => "Game saved.",
            SaveBannerKind::Loaded => "Game loaded.",
            SaveBannerKind::Autosaved => "Game autosaved.",
            SaveBannerKind::AutosaveFailed => "Autosave failed - check the log.",
            SaveBannerKind::SaveFailed => "Save failed - check the log before retrying.",
        };
        // TODO(refactor): replace these literals with MT_MSG_GAME_SAVED and
        // MT_MSG_GAME_LOADED once the localized text ownership is explicit.
        game.display_message(text.to_string(), 100);
    }
}

/// Drop pending load-type requests while a replay is playing back.
///
/// The replay stream owns the deterministic state during playback: recorded
/// loads arrive as load-back records applied at the frame boundary, so
/// re-running the request against on-disk saves (which may differ or be
/// missing on this machine) would corrupt or abort playback.  Save-type
/// requests still flush — writing a save during playback is harmless.
fn suppress_load_requests_during_playback(
    runtime: &crate::game_session::runtime::TimelineRuntime,
    callbacks: &mut RustCallbacks,
) {
    if runtime.playback().is_some()
        && callbacks
            .pending_request()
            .is_some_and(|request| !request.writes_save_payload())
    {
        tracing::info!(
            "replay playback: dropping live load request; recorded load-backs own the timeline"
        );
        callbacks.clear_operation();
    }
}

/// Save/load may replace engine state and render a thumbnail, but receives no
/// pause/stepping control or leaderboard ownership and no process service bag.
pub(super) async fn process_operation_and_save(
    mutation: MissionMutation<'_>,
    runtime: &mut TimelineRuntime,
    frontend: &mut InteractiveFrontend,
    campaign_transition: &mut Option<crate::main_entry::PendingLevelLoad>,
    window: &mut GameWindow,
    callbacks: &mut RustCallbacks,
    profiles: &engine_profiles::ProfileManager,
    args: &crate::main_entry::CliArgs,
    prepared: InputPrepared,
) -> Result<ControlFlow<FrameControl, SavesPrepared>, String> {
    let PreparationPhaseState {
        mut frame,
        mp_clock_pause,
        pause_closed_this_frame,
        rewind_active,
        shift_held,
        step_forward_pressed,
        step_back_pressed,
        modal_rendered_this_frame,
    } = prepared.0;
    let MissionMutation {
        host,
        game,
        manager,
        assets,
        dev,
    } = mutation;
    let input = &mut frontend.input;
    let audio = &mut frontend.audio;
    let resources = &mut frontend.resources;
    let ui = &mut frontend.ui;
    let hud = &mut frontend.hud;
    let presentation = &mut frontend.presentation;

    // ── Process game operations (save/load/quit/win/lose) ──
    runtime.trace(FrameContractStage::OperationAndSave);
    //
    // The Game state machine queues save/load intents on the
    // callbacks; `perform_pending_save_load` then flushes them to
    // disk with live engine access.
    let autosave_completions = callbacks.poll_autosaves();
    callbacks.autosave_notices.observe(autosave_completions);
    let exit_code = game.process_operation(manager.engine.campaign(), profiles, callbacks);
    let lifecycle_autosave = window.lifecycle_autosave_requested();
    let mission_id = current_mission_id(manager.engine.campaign(), profiles);
    let autosave_allowed = crate::autosave::session_allows_autosave(
        callbacks.autosave_enabled(),
        host.transport.net().is_some(),
        runtime.playback().is_some(),
        args.headless,
    );
    let snapshot_available = callbacks
        .pending_request()
        .is_none_or(SaveLoadRequest::writes_save_payload);
    if lifecycle_autosave && !autosave_allowed {
        // This session is excluded by policy, rather than temporarily
        // unable to capture. Do not retain a stale browser lifecycle edge
        // that could fire after returning to an eligible session.
        window.acknowledge_lifecycle_autosave_request();
    }
    let autosave_reason = callbacks.plan_autosave(
        autosave_allowed,
        snapshot_available,
        mission_id,
        u64::from(runtime.frame_number()),
        lifecycle_autosave,
        exit_code.is_some(),
    );
    #[cfg(target_arch = "wasm32")]
    let defer_initial_thumbnail = can_defer_initial_autosave(
        runtime.frame_number(),
        autosave_reason,
        callbacks.pending_request().is_some(),
        lifecycle_autosave,
        exit_code.is_some(),
    ) && {
        let search = web_sys::window()
            .expect("browser window")
            .location()
            .search()
            .expect("startup save query");
        let query =
            web_sys::UrlSearchParams::new_with_str(&search).expect("startup save query parameters");
        query.get("startup-save").as_deref() != Some("blocking")
    };
    #[cfg(target_arch = "wasm32")]
    let mut deferred_thumbnail = None;
    let pending_thumbnail = if (callbacks
        .pending_request()
        .is_some_and(|request| request.writes_save_payload())
        || autosave_reason.is_some())
        && !host.frontend.presentation.skip_render
        && !modal_rendered_this_frame
    {
        pre_render_engine_setup(host);
        update_mouse_and_cursor(
            &manager.engine,
            host,
            assets,
            dev,
            &mut frame.stage_external_actions(),
            &mut presentation.renderer,
            &mut resources.cursor,
            &mut presentation.sprites.cursor_renderer,
            &input.threaded,
            &presentation.sprites.portrait_cache,
            shift_held,
            &mut hud.last_cursor_id,
        );
        let display_snapshot = host.frontend.presentation.engine_display.clone();
        presentation.prepare_zoom(&manager.engine, &host.presentation(), hud, input);
        let mut render_ctx = presentation.render_context(
            resources,
            hud,
            input,
            ui,
            game,
            RenderViewState {
                shift_held,
                rewind_active,
                display_info_elapsed_secs:
                    <RustCallbacks as crate::game::GameCallbacks>::get_current_playing_time(
                        callbacks,
                        manager.engine.campaign(),
                    ),
            },
        );
        let thumbnail = crate::game_session::render::begin_save_thumbnail(
            &manager.engine.presentation_view(),
            &display_snapshot,
            &mut host.presentation(),
            assets,
            dev,
            &mut render_ctx,
        );
        #[cfg(target_arch = "wasm32")]
        if defer_initial_thumbnail {
            deferred_thumbnail = Some(thumbnail);
            None
        } else {
            thumbnail.await
        }
        #[cfg(not(target_arch = "wasm32"))]
        thumbnail.await
    } else {
        None
    };
    if let Some(reason) = autosave_reason {
        #[cfg(target_arch = "wasm32")]
        let accepted = if let Some(thumbnail) = deferred_thumbnail {
            callbacks.enqueue_initial_autosave_with_thumbnail(
                host,
                game,
                &manager.engine,
                mission_id,
                profiles,
                thumbnail,
            )
        } else {
            callbacks.enqueue_autosave(
                host,
                game,
                &manager.engine,
                mission_id,
                profiles,
                pending_thumbnail.clone(),
                reason,
            )
        };
        #[cfg(not(target_arch = "wasm32"))]
        let accepted = callbacks.enqueue_autosave(
            host,
            game,
            &manager.engine,
            mission_id,
            profiles,
            pending_thumbnail.clone(),
            reason,
        );
        match accepted {
            Ok(()) if lifecycle_autosave => window.acknowledge_lifecycle_autosave_request(),
            Ok(()) => {}
            Err(error) => {
                tracing::error!(?reason, "Autosave could not be queued: {error}");
                callbacks.autosave_notices.enqueue_failed();
            }
        }
    }
    if let Some(exit_code) = exit_code {
        // The original game loop applies transition sound/input changes
        // before returning its terminal code. Execute them here so
        // the mission-local Host is not dropped with queued effects.
        execute_app_effects(
            &mut callbacks.app_effects,
            &mut host.audio.sound,
            &mut input.threaded,
            audio
                .backend
                .as_mut()
                .map(|backend| backend as &mut dyn crate::sound::AudioBackend),
        );
        tracing::info!("Game exited with: {:?}", exit_code);
        // Flush any pending save before returning (e.g. the
        // quit-time continue save).
        suppress_load_requests_during_playback(runtime, callbacks);
        let mut save_load = perform_pending_save_load(
            host,
            game,
            callbacks,
            &mut manager.engine,
            assets.as_ref(),
            profiles,
            pending_thumbnail.clone(),
        );
        if save_load.processed() {
            runtime.reset_rollback_checker();
        }
        runtime.synchronize_save_boundary(&mut frame, &manager.engine);
        if let Some(event) = save_load.event.take() {
            runtime.note_save_load_event(event, &mut frame, &manager.engine, assets.as_ref());
        }
        if let Some(transition) = save_load.take_transition() {
            *campaign_transition = Some(transition);
            game.operation.set(GameCode::LevelLoad);
            runtime.trace(FrameContractStage::Exit);
            return Ok(ControlFlow::Break(FrameControl::exit(GameCode::LevelLoad)));
        }
        if save_load.restart_requested() {
            game.operation.set(GameCode::LevelRestart);
            runtime.trace(FrameContractStage::Exit);
            return Ok(ControlFlow::Break(FrameControl::Exit(MissionExit::new(
                GameCode::LevelRestart,
            ))));
        }
        if let Some(sync) = save_load.restore() {
            runtime.note_state_restored();
            game.apply_post_load_sync(sync.is_continue);
            game.post_load_resolution_resync();
        }
        runtime.trace(FrameContractStage::Exit);
        return Ok(ControlFlow::Break(FrameControl::Exit(MissionExit::new(
            exit_code,
        ))));
    }
    suppress_load_requests_during_playback(runtime, callbacks);
    let mut save_load = perform_pending_save_load(
        host,
        game,
        callbacks,
        &mut manager.engine,
        assets.as_ref(),
        profiles,
        pending_thumbnail,
    );
    if save_load.processed() {
        runtime.reset_rollback_checker();
    }
    runtime.synchronize_save_boundary(&mut frame, &manager.engine);
    if let Some(event) = save_load.event.take() {
        runtime.note_save_load_event(event, &mut frame, &manager.engine, assets.as_ref());
    }

    // A rejected/missing/unappliable Restart payload must leave this
    // mission. Continuing after terminal debriefing reset the operation
    // to LevelInProgress would keep the failed mission alive with mixed
    // lifecycle state. The outer session owns the authoritative restart
    // campaign/RNG/SimConfig checkpoint.
    if save_load.restart_requested() {
        game.operation.set(GameCode::LevelRestart);
        runtime.trace(FrameContractStage::Exit);
        return Ok(ControlFlow::Break(FrameControl::Exit(MissionExit::new(
            GameCode::LevelRestart,
        ))));
    }

    // ── Cross-mission load: bubble up ──
    // `perform_pending_save_load` returns a `PendingLevelLoad` when the
    // chosen slot targets a different mission than the one running. Force
    // the Game state machine into LevelLoad so `process_operation` exits
    // on the next iteration; the outer session loop will switch missions
    // and re-queue the Load on the fresh engine.
    if let Some(transition) = save_load.take_transition() {
        *campaign_transition = Some(transition);
        game.operation.set(GameCode::LevelLoad);
        runtime.trace(FrameContractStage::Exit);
        return Ok(ControlFlow::Break(FrameControl::Exit(MissionExit::new(
            GameCode::LevelLoad,
        ))));
    }

    // ── Post-load slot-type sync ──
    // Sync the continue-save flag and re-arm the campaign-map
    // overlay if the loaded save had it open.  `post_load_sync`
    // is armed by `perform_pending_save_load` after any Load
    // variant succeeds, threading the slot type back out of the
    // save-I/O layer.
    if let Some(sync) = save_load.restore() {
        runtime.note_state_restored();
        game.apply_post_load_sync(sync.is_continue);
        game.post_load_resolution_resync();
    }

    apply_post_save_ui_state(
        host,
        &mut callbacks.autosave_notices,
        game,
        input,
        &save_load,
    );

    Ok(ControlFlow::Continue(SavesPrepared(
        PreparationPhaseState {
            frame,
            mp_clock_pause,
            pause_closed_this_frame,
            rewind_active,
            shift_held,
            step_forward_pressed,
            step_back_pressed,
            modal_rendered_this_frame,
        },
    )))
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_initial_nonurgent_autosave_can_defer_thumbnail_completion() {
        use super::can_defer_initial_autosave;
        use crate::autosave::AutosaveReason;
        assert!(can_defer_initial_autosave(
            0,
            Some(AutosaveReason::MissionTransition),
            false,
            false,
            false
        ));
        for reason in [
            None,
            Some(AutosaveReason::Periodic),
            Some(AutosaveReason::Backgrounded),
        ] {
            assert!(!can_defer_initial_autosave(0, reason, false, false, false));
        }
        assert!(!can_defer_initial_autosave(
            1,
            Some(AutosaveReason::MissionTransition),
            false,
            false,
            false
        ));
        for (pending, backgrounded, exiting) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            assert!(!can_defer_initial_autosave(
                0,
                Some(AutosaveReason::MissionTransition),
                pending,
                backgrounded,
                exiting
            ));
        }
    }
}
