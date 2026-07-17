//! Interactive mission flow ownership and orchestration.
//!
//! Session services are borrowed for one run and never stored in the loaded
//! mission. This extraction is intentionally mechanical so later focused
//! phase methods cannot disturb the established frame ordering.

use super::*;

/// Application/session services borrowed by a loaded interactive mission.
///
/// These references are process resources and deliberately do not implement
/// serde. They remain outside deterministic mission ownership.
pub(super) struct MissionServices<'a> {
    pub(super) window: &'a mut GameWindow,
    pub(super) callbacks: &'a mut RustCallbacks,
    pub(super) campaign: &'a mut Campaign,
    pub(super) profiles: &'a engine_profiles::ProfileManager,
    pub(super) args: &'a crate::main_entry::CliArgs,
}

/// Control returned by one interactive host-frame iteration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum FrameControl {
    Continue,
    RestartIteration,
    Exit(MissionExit),
}

/// Values produced by graphical network ingress at the frame boundary.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct FrameStart {
    frame: MissionFrame,
    mp_clock_pause: bool,
}

/// State handed from modal/recorder bookkeeping to presentation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct FramePresentationState {
    frame: MissionFrame,
    rewind_active: bool,
    consumed_buffered: bool,
    shift_held: bool,
    modal_rendered: bool,
}

/// Deterministic and presentation flags carried across the tick boundary.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PreparedFrame {
    frame: MissionFrame,
    rewind_active: bool,
    paused: bool,
    consumed_buffered: bool,
    shift_held: bool,
    modal_rendered: bool,
    step_forward_pressed: bool,
    step_back_pressed: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
enum FramePreparation {
    Ready(PreparedFrame),
    Control(FrameControl),
}

/// Existing mission exit decision propagated to the session wrapper.
///
/// Campaign restoration intentionally remains at the established exit sites
/// until campaign ownership moves by value in a later wave.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct MissionExit {
    code: GameCode,
}

impl MissionExit {
    const fn new(code: GameCode) -> Self {
        Self { code }
    }

    const fn into_game_code(self) -> GameCode {
        self.code
    }
}

impl InteractiveMission {
    /// Run the already-loaded mission until an existing exit path fires.
    pub(super) async fn run(
        &mut self,
        services: &mut MissionServices<'_>,
    ) -> Result<GameCode, String> {
        if let Some(code) = self.run_startup_action(services).await? {
            return Ok(code);
        }
        loop {
            match self.run_frame(services).await? {
                FrameControl::Continue | FrameControl::RestartIteration => {}
                FrameControl::Exit(exit) => return Ok(exit.into_game_code()),
            }
        }
    }

    /// Handle the one-shot pre-frame tooling path without retaining sibling
    /// frontend borrows across the first frame await.
    async fn run_startup_action(
        &mut self,
        services: &mut MissionServices<'_>,
    ) -> Result<Option<GameCode>, String> {
        let campaign_ref = &mut *services.campaign;
        let args = services.args;
        // Preserve the existing statement order while migrating ownership. These
        // are disjoint borrows from the two mission-lifetime roots, not secondary
        // state copies.
        let InteractiveMission { runtime, frontend } = self;
        let MissionRuntime { world, .. } = runtime;
        let MissionWorld {
            host,
            game,
            manager,
            assets,
            dev,
        } = world;
        let InteractiveFrontend {
            input,
            resources,
            ui,
            hud,
            presentation,
            ..
        } = frontend;

        // One-shot tooling path: render the pristine mission after Initialize
        // and camera setup, but before the first simulation hourglass or
        // PostInitialize call.  This matches the original startup boundary in
        // `original-code/RHgame.cpp` (Initialize around line 1449; the deferred
        // PostInitialize dispatch around lines 1835-1841).
        if let Some(output_path) = args.mission_start_map_output.as_deref() {
            if args.mission_start_reveal_all {
                manager.engine.apply_commands(
                    &mut host.engine_display,
                    &mut host.input,
                    &assets,
                    &[engine_player_command::PlayerCommand::RevealAllBlips.into()],
                );
                tracing::info!("Mission-start map: revealed all blipped NPCs");
            }
            host.draw_order = manager.engine.compute_display_order();
            hud.corner_layout = CornerHudLayout::for_resolution(
                presentation.renderer.screen_width() as u32,
                presentation.renderer.screen_height() as u32,
                &hud.corner_sprites,
            );
            hud.stature_layout = StatureHudLayout::for_resolution(
                presentation.renderer.screen_width() as u32,
                presentation.renderer.screen_height() as u32,
                &hud.stature_sprites,
            );
            pre_render_engine_setup(manager, host, assets.as_ref(), &mut presentation.renderer);

            // A map export is not an interactive screenshot. Keep the cursor out
            // of the top-left map pixel while retaining the normal render path for
            // terrain, decals, ambiance, sprites, masks, and overlays.
            host.input.mouse_opacity = 0;
            let display_snapshot = host.engine_display.clone();
            let capture_result = {
                let mut render_ctx = presentation.render_context(
                    resources,
                    hud,
                    input,
                    ui,
                    game,
                    RenderViewState {
                        shift_held: false,
                        rewind_active: false,
                        display_info_elapsed_secs: 0,
                    },
                );
                capture_wide_map_to_path(
                    &manager.engine,
                    &display_snapshot,
                    host,
                    &assets,
                    &dev,
                    &mut render_ctx,
                    output_path,
                )
            };

            restore_engine_campaign(
                campaign_ref,
                &mut manager.engine,
                "mission-start map capture exit",
            );
            capture_result.map_err(|err| {
                format!(
                    "failed to render mission-start map to {}: {err}",
                    output_path.display()
                )
            })?;
            tracing::info!("Mission-start map → {}", output_path.display());
            return Ok(Some(GameCode::Quit));
        }

        Ok(None)
    }

    /// Apply multiplayer ingress and capture the deterministic pre-command
    /// snapshot before any interactive input mutates the engine.
    fn begin_interactive_frame(&mut self, args: &crate::main_entry::CliArgs) -> FrameStart {
        let InteractiveMission { runtime, frontend } = self;
        let MissionRuntime {
            world,
            timeline: runtime,
            control,
        } = runtime;
        let MissionWorld {
            host,
            manager,
            assets,
            ..
        } = world;
        let manual_pause = &mut control.manual_pause;
        let hud = &mut frontend.hud;
        let presentation = &mut frontend.presentation;
        let mut frame = MissionFrame::new(crate::window::process_uptime_ms());
        if let Some(start_at) = runtime.mp_start_gate {
            if current_epoch_ms() >= start_at {
                runtime.mp_start_gate = None;
                if !runtime.start_paused {
                    *manual_pause = false;
                }
                tracing::info!("multiplayer: synchronized lobby start gate opened");
            } else {
                *manual_pause = true;
            }
        }

        // ── Multiplayer: drain incoming wire events ───────────────
        // - Future inputs queue in `pending_inputs[target_frame]`.
        // - Late inputs (target < sim_frame) splice into the rewind
        //   buffer and trigger a rollback to reconstruct the engine
        //   state with the late input woven in.  `drain_net_inputs`
        //   replaces the live rollback state when that fires.
        // - Inputs scheduled for `sim_frame` come back in the return
        //   value; we apply them and append to `frame.commands`.
        // - Authoritative state hashes from the host land in
        //   `runtime.peer_hashes`, drained below alongside the per-25-frame
        //   sampling tick.
        // Publishes the current sim_frame to the server's broadcast
        // pump so peer-input target frames are stamped against a
        // fresh cursor.
        if let Some(net) = host.net.as_ref() {
            net.publish_frame(manager.sim_frame);
        }
        let net_drain = drain_net_inputs(
            host,
            manager,
            assets.as_ref(),
            &mut runtime.rewind_buffer,
            &mut runtime.peer_hashes,
            &mut runtime.recent_timeline_history,
        );
        if net_drain.rewrote_sim_state
            && let Some(ref mut checker) = runtime.rollback_checker
        {
            checker.reset();
        }
        if let Some(rollback) = net_drain.rollback.clone() {
            runtime.last_mp_rollback = Some(rollback);
        }
        if let Some((_frame, start_epoch_ms)) = net_drain.begin_sim {
            runtime.mp_waiting_for_begin_sim = false;
            runtime.mp_start_gate = Some(start_epoch_ms);
            *manual_pause = true;
        }
        if runtime.mp_waiting_for_initial_snapshot && net_drain.received_initial_snapshot {
            runtime.mp_waiting_for_initial_snapshot = false;
            tracing::info!(
                "multiplayer: initial snapshot received; client ready for start barrier"
            );
        }
        if runtime.mp_waiting_for_initial_snapshot || runtime.mp_waiting_for_begin_sim {
            *manual_pause = true;
        }
        if host.net.is_some()
            && host.local_seat != engine_player_command::PlayerId::HOST
            && let Some((clock_frame, ms_until_next_frame)) = net_drain.latest_host_clock_sample
        {
            accept_host_frame_schedule(
                &mut runtime.mp_host_frame_schedule,
                clock_frame,
                ms_until_next_frame,
                manager.sim_frame,
            );
        }
        let mut mp_clock_pause = false;
        if host.net.is_some()
            && host.local_seat != engine_player_command::PlayerId::HOST
            && !runtime.mp_waiting_for_initial_snapshot
            && !runtime.mp_waiting_for_begin_sim
            && runtime.mp_start_gate.is_none()
        {
            if let Some(deadline_ms) =
                host_scheduled_frame_deadline_ms(runtime.mp_host_frame_schedule, manager.sim_frame)
            {
                let now_ms = crate::window::process_uptime_ms();
                let until_frame_ms = deadline_ms - i64::from(now_ms);
                if until_frame_ms > 0 {
                    mp_clock_pause = true;
                    if now_ms.saturating_sub(runtime.last_mp_clock_ahead_log_ms) >= 1000 {
                        runtime.last_mp_clock_ahead_log_ms = now_ms;
                        tracing::info!(
                            scheduled_frame =
                                runtime.mp_host_frame_schedule.map(|(frame, _)| frame),
                            local_frame = manager.sim_frame,
                            until_frame_ms,
                            "multiplayer: local frame is ahead of host schedule; holding sim"
                        );
                    }
                }
            } else {
                mp_clock_pause = true;
            }
        }
        let net_inputs = net_drain.inputs;
        if host.net.is_some() {
            runtime
                .recent_timeline_history
                .checkpoint(manager.sim_frame, &manager.engine);
        }
        if !net_inputs.is_empty() {
            manager.engine.apply_commands(
                &mut host.engine_display,
                &mut host.input,
                &assets,
                &net_inputs,
            );
            for inp in net_inputs {
                frame.commands.commands.push(inp);
            }
        }

        // Re-derive the corner HUD layout every frame so resolution
        // changes triggered from nested menus (options modal, Sherwood
        // flow, etc.) take effect without needing every call site to
        // plumb a mutable layout ref.  Cheap — just a few rect
        // arithmetic operations.
        hud.corner_layout = CornerHudLayout::for_resolution(
            presentation.renderer.screen_width() as u32,
            presentation.renderer.screen_height() as u32,
            &hud.corner_sprites,
        );
        hud.stature_layout = StatureHudLayout::for_resolution(
            presentation.renderer.screen_width() as u32,
            presentation.renderer.screen_height() as u32,
            &hud.stature_sprites,
        );

        // Refresh the host-cached back-to-front entity draw order from
        // the current engine state.  Consumed by this frame's input
        // handlers (hit-test via `find_focusable_entity`), render loop,
        // and titbit Z flush.  Headless replay has no hit-test/render
        // consumer, so skip the sort there.
        if !args.headless {
            host.draw_order = manager.engine.compute_display_order();
        }

        // Enter the shared runtime's input phase. This captures the
        // rollback/rewind snapshots and replay state hash at the "start of
        // frame N — after N-1's tick, before N's commands" point,
        // before any event-loop handler (Resized → inline
        // `MinimapResize`, live input → local viewport edits, …)
        // mutates the engine.  The recorder write and the player
        // check MUST sample here in lockstep; if the recording
        // captures post-input state while the replay checks
        // pre-input state, every hash-carrying frame spuriously
        // desyncs.  The actual write is deferred to the recorder
        // block further down so the existing
        // `!rewind_active && !consumed_buffered` gating stays in
        // one place.
        runtime.open_frame(&mut frame, manager.sim_frame, &manager.engine, &assets);
        FrameStart {
            frame,
            mp_clock_pause,
        }
    }

    /// Finalize recorder/audio state, present, cross PostInitialize, and pace.
    ///
    /// This view is created only after async modal work completes, so no
    /// RenderContext or sibling frontend borrow survives an await.
    async fn finish_interactive_frame(
        &mut self,
        services: &mut MissionServices<'_>,
        state: FramePresentationState,
    ) {
        let callbacks = &mut *services.callbacks;
        let campaign_ref = &mut *services.campaign;
        let args = services.args;
        let InteractiveMission { runtime, frontend } = self;
        let MissionRuntime {
            world,
            timeline: runtime,
            ..
        } = runtime;
        let MissionWorld {
            host,
            game,
            manager,
            assets,
            dev,
        } = world;
        let input = &mut frontend.input;
        let audio = &mut frontend.audio;
        let resources = &mut frontend.resources;
        let ui = &mut frontend.ui;
        let hud = &mut frontend.hud;
        let presentation = &mut frontend.presentation;
        let FramePresentationState {
            mut frame,
            rewind_active,
            consumed_buffered,
            shift_held,
            modal_rendered: modal_rendered_this_frame,
        } = state;

        // ── Commit the recorder frame ──
        // Deferred from the record block above so every modal drain,
        // including final mission-state/debriefing popups, can append
        // `ModalDismiss` entries to the same frame as the engine tick
        // that queued them.
        runtime.finish_recording(
            std::mem::take(&mut frame.modal_dismissals),
            !rewind_active && !consumed_buffered,
        );
        // Warn if any recorded dismissals went unused — this should not
        // happen for a clean replay; if it does, the replay commands
        // have drifted out of sync with the engine's modal output.
        if !frame.replay_modal_dismissals.is_empty() {
            tracing::warn!(
                "Replay: {} recorded ModalDismiss command(s) unused this frame",
                frame.replay_modal_dismissals.len()
            );
        }

        // Execute any sound-mode / jingle / mouse intents queued by the
        // state machine (`game.process_operation`), the pause-menu input
        // handler, or script-triggered menus. Must run before the sound
        // hourglass so a fresh `set_mode(Mission)` immediately tees up
        // `load_music = true` before the tick.
        execute_app_effects(
            &mut callbacks.app_effects,
            &mut host.sound,
            &mut input.threaded,
            audio
                .backend
                .as_mut()
                .map(|b| b as &mut dyn crate::sound::AudioBackend),
        );

        // ── Sound tick ──
        // Combat/alert music transitions + sim-emitted sound drains.
        // See `tick_audio` for the breakdown.
        audio.tick(manager, host);

        runtime.begin_presentation();

        // ── Render dispatch ──
        // The display-state machine (display_op transitions, scrolling
        // deceleration, zoom interpolation, minimap transition) now runs
        // inside `perform_hourglass` so rollback replay re-runs the
        // same mutations. `last_skip_render` carries the
        // fast-forward "skip this frame" decision back to the host.
        // `--headless` forces the render block off for the entire
        // mission — same gate as the in-game fast-forward
        // `host.skip_render` toggle, just sticky.
        let draw_result = if host.skip_render || args.headless || modal_rendered_this_frame {
            1
        } else {
            0
        };

        if draw_result == 0 {
            pre_render_engine_setup(manager, host, assets.as_ref(), &mut presentation.renderer);
            update_mouse_and_cursor(
                manager,
                host,
                &assets,
                &dev,
                &mut presentation.renderer,
                &mut resources.cursor,
                &mut presentation.sprites.cursor_renderer,
                &input.threaded,
                &presentation.sprites.portrait_cache,
                shift_held,
                &mut hud.last_cursor_id,
            );

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
                            campaign_ref,
                        ),
                },
            );

            // Pending `/screenshot` requests: each renders a dedicated
            // throwaway frame with its own overridden dev flags into
            // the offscreen target, reads the pixels back, and clears
            // the target for the next render.  Runs BEFORE the live
            // frame so `present()` still blits the real frame last.
            let display_snapshot = host.engine_display.clone();
            drain_screenshots(
                &manager.engine,
                &display_snapshot,
                host,
                &assets,
                &dev,
                &mut render_ctx,
            );

            if host.pending_print_screen == Some(PrintScreenRequest::WideSnapshot) {
                host.pending_print_screen = None;
                let display_snapshot = host.engine_display.clone();
                if !drain_wide_print_screen(
                    &manager.engine,
                    &display_snapshot,
                    host,
                    &assets,
                    &dev,
                    &mut render_ctx,
                ) {
                    host.pending_print_screen = Some(PrintScreenRequest::Plain);
                }
            }

            let display_snapshot = host.engine_display.clone();
            render_frame(
                &manager.engine,
                &display_snapshot,
                host,
                &assets,
                &dev,
                &mut render_ctx,
            );

            // PrintScreen keybind — capture the composited frame
            // after `render_frame` completes but before `present()`
            // resets the target.  Writes to
            // `<save-root>/screen%03u.png`, picking the first free
            // slot in `000..1000`.
            if let Some(request) = host.pending_print_screen.take() {
                drain_print_screen_request(render_ctx.renderer, request);
            }

            render_ctx.present();
            if let Some(mut fade) = host.fade_to_black {
                host.fade_to_black = fade.advance_presented_frame().then_some(fade);
            }
            post_render_engine_cleanup(manager, host, &assets);
        } // end if draw_result == 0 (skip render in fast-forward)

        // Transient-message countdown: the render pass drew the
        // message for this frame if `message_delay` was non-zero;
        // tick down now so next frame sees one less frame remaining,
        // and drop the text when the counter reaches zero.  Runs
        // outside the render block so `ctx.game: &game` is out of
        // scope and we can mutably re-borrow `game`.
        if game.message_delay > 0 {
            game.message_delay -= 1;
            if game.message_delay == 0 {
                game.message_text.clear();
            }
        }

        // Original RHgame.cpp ordering is Refresh (including Draw/Flip),
        // then RHSound::Hourglass, then the one-shot engine
        // PostInitialize call.  Rust's sound/render consumers are split in
        // the opposite host order above, so dispatch only after both have
        // completed.  Script mutations and emitted sound/UI effects first
        // become observable on the next frame, matching the original.
        let mut display = std::mem::take(&mut host.engine_display);
        let post_initialized = crate::sim_timeline::run_post_initialize_stage(
            host,
            &mut display,
            &assets,
            &mut manager.engine,
            dev,
        );
        host.engine_display = display;
        if post_initialized
            && let Some(net) = host.net.as_ref()
            && host.local_seat == engine_player_command::PlayerId::HOST
        {
            // The initial authoritative snapshot is published once before
            // presentation bookkeeping. Replace it with the completed
            // post-refresh state so joiners start from the same frame-one
            // boundary as live and rollback replay.
            net.set_initial_snapshot(manager.sim_frame, &manager.engine);
        }

        pace_interactive_frame(runtime, host, manager, &frame, args).await;
    }

    /// Run one interactive frame using short-lived borrows of the mission
    /// components and application services.
    async fn run_frame(
        &mut self,
        services: &mut MissionServices<'_>,
    ) -> Result<FrameControl, String> {
        let prepared = match self.prepare_interactive_frame(services).await? {
            FramePreparation::Ready(prepared) => prepared,
            FramePreparation::Control(control) => return Ok(control),
        };
        self.simulate_interactive_frame(services, prepared).await
    }

    /// Collect input, drive operation/save flows, and finalize the pre-tick
    /// command stream.
    async fn prepare_interactive_frame(
        &mut self,
        services: &mut MissionServices<'_>,
    ) -> Result<FramePreparation, String> {
        let window = &mut *services.window;
        let callbacks = &mut *services.callbacks;
        let campaign_ref = &mut *services.campaign;
        let profiles = services.profiles;
        let args = services.args;
        let FrameStart {
            mut frame,
            mut mp_clock_pause,
        } = self.begin_interactive_frame(args);
        let modal_rendered_this_frame = false;
        // Preserve the existing statement order while migrating ownership. These
        // are disjoint borrows from the two mission-lifetime roots, not secondary
        // state copies.
        let InteractiveMission { runtime, frontend } = self;
        let MissionRuntime {
            world,
            timeline: runtime,
            control,
        } = runtime;
        let MissionWorld {
            host,
            game,
            manager,
            assets,
            dev,
        } = world;
        let MissionControl {
            manual_pause,
            step_forward_repeat_at_ms,
            step_back_repeat_at_ms,
            ..
        } = control;
        let input = &mut frontend.input;
        let audio = &mut frontend.audio;
        let resources = &mut frontend.resources;
        let ui = &mut frontend.ui;
        let hud = &mut frontend.hud;
        let presentation = &mut frontend.presentation;

        match handle_sherwood_campaign_map_overlay(
            game,
            manager,
            host,
            &mut frame.commands,
            &assets,
            campaign_ref,
            &mut *window,
            &mut presentation.renderer,
            &mut resources.cursor,
            &mut presentation.sprites.cursor_renderer,
            &mut resources.text,
            &mut ui.campaign_map,
            &mut resources.menu,
            &mut hud.sherwood_enable,
        )
        .await?
        {
            HandlerAction::Continue => {
                return Ok(FramePreparation::Control(FrameControl::RestartIteration));
            }
            HandlerAction::Exit(code) => {
                return Ok(FramePreparation::Control(FrameControl::Exit(
                    MissionExit::new(code),
                )));
            }
            HandlerAction::Proceed => {}
        }

        // Tracks whether the pause menu closed at any point this
        // frame.  Used to flush queued input so actions that would
        // have run while the menu was up don't leak into the resumed
        // game.
        let mut pause_closed_this_frame = false;

        // ── Input ──
        // Flattened gameplay modals still own their own one-frame
        // `poll_events()` call.  If the main loop drains the window
        // first, modal widgets only receive occasional synthetic state,
        // while global gameplay shortcuts (notably Escape/DisplayMenu)
        // can fire underneath the modal.  Leave the event queue intact
        // whenever a modal is active or queued so `tick_active_modal`
        // gets first chance at the raw input.
        let modal_input_active = ui.active_modal.is_some() || modal_state_pending(&host);
        if modal_input_active && ui.close_pause(input, presentation) {
            pause_closed_this_frame = true;
            callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Mission));
        }
        // Disjoint-borrow event poll: `event_pump`/`width`/`height` are
        // separate fields from `canvas`, which the presentation.renderer owns mutably.
        let mut events = if modal_input_active {
            Vec::new()
        } else {
            window.poll_events()
        };
        input.threaded.feed_sdl_events(&events);

        let rewind_active = handle_hold_to_rewind(
            manager,
            assets.as_ref(),
            &input.threaded,
            &mut runtime.rewind_buffer,
            &mut runtime.rollback_checker,
            &mut runtime.replay_player,
        );

        // Field-disjoint access to keep `presentation.renderer` (holding &mut *window)
        // alive through the event loop.  Skip gamepad command dispatch
        // during replay/rewind — see input_suppressed comment below.
        if runtime.replay_player.is_none() && !rewind_active {
            handle_gamepad_events(
                host,
                manager,
                &assets,
                &mut input.threaded,
                &mut frame.commands,
                &events,
                &mut window.active_gamepad,
            );
        }
        events.extend(input.threaded.drain_synthetic_events());

        // ── Handle window resize ──
        // Window-size changes don't change the game's logical render
        // resolution any more — `present()` letterboxes the fixed-size
        // offscreen RT into whatever shape the WM hands the swapchain.
        // The `host.viewport.set_screen_size` + `presentation.renderer.resize` below
        // are kept for the graphics-options menu's resolution change
        // path, which fakes a Resized event with the user-picked
        // logical size; under the new arch we should separate those,
        // but for now: only fire the full
        // logical-resize cascade if the new size matches one of the
        // menu's supported resolutions. Pure WM resizes drop through
        // and only the swapchain reconfigures.
        for event in &events {
            if let GameEvent::Resized(new_w, new_h) = *event {
                presentation.renderer.configure_surface_size(new_w, new_h);
                let is_logical_resize =
                    matches!((new_w, new_h), (640, 480) | (800, 600) | (1024, 768));
                if !is_logical_resize {
                    continue;
                }
                let w = new_w as f32;
                let h = new_h as f32;
                window.set_logical_size(new_w, new_h);
                host.viewport.set_screen_size(w, h);
                presentation.renderer.resize(new_w as u16, new_h as u16);
                input.resize(new_w, new_h, &host.key_config);
                // Reposition minimap.
                if host.minimap_corner_size.x > 0.0 {
                    let cmd = PlayerCommand::MinimapResize {
                        base: engine_coordinates::ScreenPoint::new(w - 83.0, 38.0),
                        corner_size: host.minimap_corner_size,
                    };
                    dispatch_local_command(
                        host,
                        &mut manager.engine,
                        &mut frame.commands,
                        &assets,
                        &cmd,
                    );
                }
                hud.resize(new_w, new_h);
            }
        }

        if input.threaded.is_ended() {
            restore_engine_campaign(
                campaign_ref,
                &mut manager.engine,
                "window-close mission exit",
            );
            return Ok(FramePreparation::Control(FrameControl::Exit(
                MissionExit::new(GameCode::Quit),
            )));
        }

        match handle_sherwood_hud_buttons(
            game,
            manager,
            host,
            &mut frame.commands,
            &assets,
            callbacks,
            campaign_ref,
            &mut *window,
            &mut presentation.renderer,
            &mut resources.cursor,
            &mut presentation.sprites.cursor_renderer,
            &resources.menu,
            &events,
            &hud.sherwood_layout,
            &mut hud.sherwood_enable,
            args.headless,
        )
        .await
        {
            HandlerAction::Continue => {
                return Ok(FramePreparation::Control(FrameControl::RestartIteration));
            }
            HandlerAction::Exit(code) => {
                return Ok(FramePreparation::Control(FrameControl::Exit(
                    MissionExit::new(code),
                )));
            }
            HandlerAction::Proceed => {}
        }

        // Suppress all mouse-driven HUD widget clicks while a replay
        // is playing back (recorded commands re-enter at the tick
        // boundary) or a rewind hold is active (live clicks
        // shouldn't perturb a reconstructed past state).  Without
        // this the user could click HUD buttons mid-replay and steer
        // the run.
        let input_suppressed = runtime.replay_player.is_some() || rewind_active;

        // Zoom HUD buttons (ZoomUp / ZoomDown).  Maps to
        // `EngineStateRequest::ZoomingUp/Down` — same path the
        // mouse wheel + keyboard bindings use.
        if !input_suppressed {
            let zoom_enable = ZoomButtonEnable::from_engine(&manager.engine, &host.engine_display);
            let mut zoom_btn_hit = None;
            for event in &events {
                if let GameEvent::MouseDown(mx, my, 1 /* left */, _) = *event
                    && let Some(btn) = hud.zoom_layout.hit_test(mx, my, zoom_enable)
                {
                    zoom_btn_hit = Some((btn, mx, my));
                    break;
                }
            }
            if let Some((btn, mx, my)) = zoom_btn_hit {
                let factor = match btn {
                    ZoomButton::ZoomUp => 2.0,
                    ZoomButton::ZoomDown => 0.5,
                };
                host.viewport.zoom_by(
                    factor,
                    Some(engine_coordinates::ScreenPoint::new(mx as f32, my as f32)),
                );
            }
        }

        // Corner HUD buttons (Clock / Sight / QuickStart).  Only
        // active on non-Sherwood missions.
        //
        // Left-click dispatches the activation message (record /
        // lock-alt / launch-all).  Right-click unlocks / deletes
        // macros.
        if !game.is_sherwood && !input_suppressed {
            let corner_enable = CornerButtonEnable::from_engine(&manager.engine);
            for event in &events {
                match *event {
                    GameEvent::MouseDown(mx, my, 1 /* left */, _) => {
                        let Some(btn) = hud.corner_layout.hit_test(mx, my, corner_enable) else {
                            continue;
                        };
                        dispatch_corner_button_left_click(
                            btn,
                            manager,
                            game,
                            host,
                            &assets,
                            &mut frame.commands,
                        );
                    }
                    GameEvent::MouseDown(mx, my, 3 /* right */, _) => {
                        let Some(btn) = hud.corner_layout.hit_test_geometric(mx, my) else {
                            continue;
                        };
                        dispatch_corner_button_right_click(
                            btn,
                            manager,
                            host,
                            &assets,
                            &mut frame.commands,
                        );
                    }
                    _ => {}
                }
            }

            // Stature up/down-arrow click dispatch.  Emits the same
            // PlayerCommand the keyboard path uses.  Clicking either
            // arrow also primes the focus-latch so the arrow stays
            // visually pressed while the sim runs the posture
            // transition.  Auto-clears when the aggregate stature
            // shifts.
            let stature = manager.engine.retrieve_stature(None);
            game.stature_focus.maybe_clear(stature);
            let stature_enable =
                StatureEnable::from_stature(stature).with_focus_latch(game.stature_focus);
            for event in &events {
                if let GameEvent::MouseDown(mx, my, 1 /* left */, _) = *event
                    && let Some(btn) = hud.stature_layout.hit_test(mx, my, stature_enable)
                {
                    let cmd = btn.as_command();
                    dispatch_local_command(
                        host,
                        &mut manager.engine,
                        &mut frame.commands,
                        &assets,
                        &cmd,
                    );
                    match btn {
                        StatureButton::Up => {
                            game.stature_focus.latch_stand_up(stature);
                        }
                        StatureButton::Down => {
                            game.stature_focus.latch_crouch_down(stature);
                        }
                    }
                }
            }
        }

        // Edge-check the minimap accelerator key BEFORE
        // `translate_keyboard` advances the translator's prev-key
        // buffer.  The widget holds the accelerator itself and
        // toggles on release.
        let minimap_toggle_pressed = {
            host.minimap_fast_key.is_some_and(|fast_key| {
                input
                    .translator
                    .was_key_released(fast_key, &input.threaded.keyboard_state().keys)
            })
        };

        // Step-debug keys: `.` (forward), `,` / Backspace (back), Enter
        // (unpause).  `.` and `,` step immediately on the press edge,
        // then repeat after a short hold delay so a normal key tap
        // advances exactly one frame but holding still scrubs. Backspace
        // keeps its held-state rewind scrub behavior.  Enter uses the
        // release edge so a held Enter doesn't spam-resume.  All checks
        // read physical keys rather than the bindable `GameAction` map.
        const STEP_REPEAT_INITIAL_DELAY_MS: u32 = 160;
        const STEP_REPEAT_INTERVAL_MS: u32 = 40;
        use winit::keyboard::KeyCode;
        let keys = &input.threaded.keyboard_state().keys;
        let step_forward_held = keys.contains(&KeyCode::Period);
        let step_back_comma_held = keys.contains(&KeyCode::Comma);
        let step_backspace_held = keys.contains(&KeyCode::Backspace);
        let step_forward_hit = input.translator.was_key_pressed(KeyCode::Period, keys);
        let step_back_comma_hit = input.translator.was_key_pressed(KeyCode::Comma, keys);
        let repeat_step_key = |held: bool, hit: bool, repeat_at_ms: &mut Option<u32>| -> bool {
            if !held {
                *repeat_at_ms = None;
                return false;
            }
            if hit {
                *repeat_at_ms = Some(
                    frame
                        .started_at_ms
                        .saturating_add(STEP_REPEAT_INITIAL_DELAY_MS),
                );
                return true;
            }
            if let Some(next_ms) = *repeat_at_ms
                && frame.started_at_ms >= next_ms
            {
                *repeat_at_ms = Some(frame.started_at_ms.saturating_add(STEP_REPEAT_INTERVAL_MS));
                return true;
            }
            false
        };
        let step_forward_pressed = repeat_step_key(
            step_forward_held,
            step_forward_hit,
            step_forward_repeat_at_ms,
        );
        let step_back_pressed = repeat_step_key(
            step_back_comma_held,
            step_back_comma_hit,
            step_back_repeat_at_ms,
        ) || step_backspace_held;
        let step_unpause_pressed = input.translator.was_key_released(KeyCode::Enter, keys);
        // Suppress these shortcuts when any modal input sink has focus
        // so `.` / `,` / Enter typed into the console, pause menu, or
        // text input don't accidentally freeze/step the sim.
        let step_keys_gated =
            ui.console_overlay.is_visible() || ui.pause_menu.is_some() || modal_input_active;
        if !step_keys_gated {
            if step_forward_pressed || step_back_pressed {
                *manual_pause = true;
            }
            if step_unpause_pressed {
                *manual_pause = false;
            }
        }
        let step_forward_pressed = step_forward_pressed && !step_keys_gated;
        let step_back_pressed = step_back_pressed && !step_keys_gated;

        // Translate to game actions
        let mut kb_actions = input
            .translator
            .translate_keyboard(&input.threaded.keyboard_state().keys, TranslationFlags::ALL);
        if events
            .iter()
            .any(|event| matches!(event, GameEvent::MenuToggleRequested))
            || (ui.pause_menu.is_none()
                && !modal_input_active
                && events
                    .iter()
                    .any(|event| matches!(event, GameEvent::PauseRequested)))
        {
            kb_actions.push(GameAction::DisplayMenu);
        }
        let mouse_actions = if input.threaded.has_position() {
            input.translator.translate_mouse(
                input.threaded.position().x,
                input.threaded.position().y,
                input.threaded.wheel_delta(),
            )
        } else {
            Vec::new()
        };

        // Helper: check if Ctrl is held via keyboard state
        let ctrl_held = {
            let ks = &input.threaded.keyboard_state().keys;
            ks.contains(&KeyCode::ControlLeft) || ks.contains(&KeyCode::ControlRight)
        };
        let shift_held = {
            let ks = &input.threaded.keyboard_state().keys;
            ks.contains(&KeyCode::ShiftLeft) || ks.contains(&KeyCode::ShiftRight)
        };
        let alt_held = {
            let ks = &input.threaded.keyboard_state().keys;
            ks.contains(&KeyCode::AltLeft) || ks.contains(&KeyCode::AltRight)
        };
        // Persist the alt state on `InputState` so subsystems that
        // don't otherwise see the SDL modifier mask can read it.
        host.input.is_alt = alt_held;

        handle_console_overlay_events(
            &mut ui.console_overlay,
            &mut manager.engine,
            &assets,
            host,
            dev,
            &events,
            &kb_actions,
            &mut input.translator,
        );

        // ── View-only input (scroll / zoom): always allowed ──
        // These mutate host-side viewport state only — never the sim —
        // so they're safe during replay playback and rewind, when the
        // user wants to pan/zoom around the paused world.  Suppressed
        // only when the console or the pause menu has focus.
        {
            let view_suppressed = ui.console_overlay.is_visible()
                || ui.pause_menu.is_some()
                || pause_closed_this_frame;
            if !view_suppressed {
                for action in kb_actions.iter().chain(mouse_actions.iter()) {
                    let scroll_suppressed_by_minimap =
                        matches!(
                            action,
                            GameAction::ScrollUp
                                | GameAction::ScrollDown
                                | GameAction::ScrollLeft
                                | GameAction::ScrollRight
                        ) && host.engine_display.minimap().drag_start();
                    if scroll_suppressed_by_minimap {
                        continue;
                    }
                    match action {
                        GameAction::ScrollUp => {
                            apply_local_viewport_scroll(host, ScrollDirection::Up);
                        }
                        GameAction::ScrollDown => {
                            apply_local_viewport_scroll(host, ScrollDirection::Down);
                        }
                        GameAction::ScrollLeft => {
                            apply_local_viewport_scroll(host, ScrollDirection::Left);
                        }
                        GameAction::ScrollRight => {
                            apply_local_viewport_scroll(host, ScrollDirection::Right);
                        }
                        GameAction::ZoomIn => {
                            let mp = input.threaded.position();
                            host.viewport.zoom_by(
                                2.0,
                                Some(engine_coordinates::ScreenPoint::new(mp.x, mp.y)),
                            );
                        }
                        GameAction::ZoomOut => {
                            let mp = input.threaded.position();
                            host.viewport.zoom_by(
                                0.5,
                                Some(engine_coordinates::ScreenPoint::new(mp.x, mp.y)),
                            );
                        }
                        _ => {}
                    }
                }
            }
        }

        // ── Mouse middle-drag viewport pan: always allowed ──
        // Same reasoning as the keyboard scroll/zoom block above —
        // ViewportPan is pure host-side viewport state.  Apply it here
        // before `handle_mouse_input` (which is gated by replay state)
        // can swallow it.
        if ui.pause_menu.is_none() && !pause_closed_this_frame && !manager.engine.user_locked() {
            for event in &events {
                if let GameEvent::ViewportPan { xrel, yrel } = *event {
                    host.viewport
                        .scroll_by(robin_engine::coordinates::ScreenVec::new(
                            -(xrel as f32),
                            -(yrel as f32),
                        ));
                    host.input.cancel_multi_selection();
                }
            }
        }

        // ── Skip all sim-affecting input during replay / rewind ──
        // Recorded commands are injected at the tick boundary instead
        // (replay), or suppressed entirely (rewind — live input
        // shouldn't perturb a state reconstructed from the past).
        if runtime.replay_player.is_none() && !rewind_active {
            // Minimap accelerator key.
            // Suppressed while the console or pause menu has focus so the
            // toggle can't fire underneath modal UI.
            if minimap_toggle_pressed && !ui.console_overlay.is_visible() && ui.pause_menu.is_none()
            {
                let cmd = PlayerCommand::MinimapToggle;
                dispatch_local_command(
                    host,
                    &mut manager.engine,
                    &mut frame.commands,
                    &assets,
                    &cmd,
                );
            }

            for action in kb_actions.iter().chain(mouse_actions.iter()) {
                // Console captures every other action while it has focus.
                if ui.console_overlay.is_visible() {
                    continue;
                }
                match action {
                    GameAction::DisplayConsole => {
                        // Already handled above — swallow so we don't
                        // hit the catch-all below.
                    }
                    GameAction::DisplayInfo => {
                        // Toggle the host flag — the per-frame debug
                        // overlay presentation.renderer polls `host.info_displayed`
                        // to decide whether to draw FPS / mission
                        // clock / music-mode bars.
                        host.info_displayed = !host.info_displayed;
                        tracing::debug!("DisplayInfo toggled: {}", host.info_displayed);
                    }
                    GameAction::DisplayMenu => {
                        if ui.pause_menu.is_some() {
                            debug_assert!(ui.close_pause(input, presentation));
                            pause_closed_this_frame = true;
                            callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Mission));
                            // Resume play-time recording after the
                            // modal closes.
                            callbacks.start_play_time();
                        } else {
                            // Suspend play-time recording before
                            // opening the modal so `MissionLength`
                            // doesn't count wall-clock spent in the
                            // pause menu.
                            callbacks.suspend_play_time();
                            if let Some(resources) = resources.menu.as_ref() {
                                ui.pause_menu = Some(PauseMenu::new(resources, ui.restart_allowed));
                            } else {
                                // Retry the resource load in case a transient presentation.renderer state
                                // prevented mission-start initialization. A pause menu still
                                // requires the real resources after this retry.
                                let fallback = IngameMenuResources::new(
                                    &mut presentation.renderer,
                                    host.shipping.as_deref(),
                                );
                                let res = required_menu_resources(
                                    &fallback,
                                    "opening the pause menu after resource reload",
                                );
                                ui.pause_menu = Some(PauseMenu::new(res, ui.restart_allowed));
                                resources.menu = fallback;
                            }
                            if ui.pause_menu.is_some() {
                                // Freeze the current screen so the
                                // pause-menu backdrop composites over
                                // a still frame instead of the live
                                // engine output.  Idempotent; the
                                // symmetric close-branch above calls
                                // `clear_frozen_scene`.
                                presentation.renderer.freeze_scene_for_modal();
                                callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Menu));
                            }
                        }
                    }
                    _ if ui.pause_menu.is_some() || pause_closed_this_frame => {
                        // Skip all other game actions while paused
                        // and for the remainder of the frame if pause
                        // was toggled off this frame, so actions
                        // queued during pause don't fire the instant
                        // the game resumes.
                    }
                    _ => {
                        match action {
                            GameAction::SlowMotion => {
                                // Toggle the slow-motion pacing flag.
                                // The frame-pacing block multiplies
                                // the 40 ms frame target by 10 when
                                // set.  Pure host-side, not sim state.
                                host.slow_motion = !host.slow_motion;
                            }
                            GameAction::SwitchMaskedDisplay => {
                                // Toggle the "draw hidden" debug view.
                                // This is per-seat presentation state;
                                // script-visible outline display
                                // changes still come from sim-side
                                // `SetOutlineDisplay` commands.
                                host.input.draw_hidden = !host.input.draw_hidden;
                            }
                            // Scroll{Up,Down,Left,Right} and Zoom{In,Out}
                            // are handled by the always-on view-only
                            // input pass at the top of the frame so
                            // they work during replay/rewind.
                            GameAction::ScrollUp
                            | GameAction::ScrollDown
                            | GameAction::ScrollLeft
                            | GameAction::ScrollRight
                            | GameAction::ZoomIn
                            | GameAction::ZoomOut => {}
                            GameAction::SelectAll => {
                                let cmd = PlayerCommand::SelectAllPcs;
                                dispatch_local_command(
                                    host,
                                    &mut manager.engine,
                                    &mut frame.commands,
                                    &assets,
                                    &cmd,
                                );
                            }
                            GameAction::UnselectAll => {
                                let cmd = PlayerCommand::UnselectAllPcs;
                                dispatch_local_command(
                                    host,
                                    &mut manager.engine,
                                    &mut frame.commands,
                                    &assets,
                                    &cmd,
                                );
                            }
                            GameAction::SelectAction { index } => {
                                let selected = manager.engine.seat_selection(host.local_seat);
                                if selected.len() == 1 {
                                    let pc_id = selected[0];
                                    let cmd = PlayerCommand::SelectAction {
                                        pc_id,
                                        action_index: *index as u32,
                                    };
                                    dispatch_local_command(
                                        host,
                                        &mut manager.engine,
                                        &mut frame.commands,
                                        &assets,
                                        &cmd,
                                    );
                                }
                            }
                            GameAction::SelectCharacter { portrait_index } => {
                                let idx = *portrait_index as usize;
                                let cmd = if ctrl_held {
                                    PlayerCommand::AssignQuickGroup { index: idx as u8 }
                                } else {
                                    let has_group = idx < 9
                                        && !manager.engine.quick_select_group(idx).is_empty();
                                    if has_group {
                                        PlayerCommand::RecallQuickGroup { index: idx as u8 }
                                    } else {
                                        PlayerCommand::SelectByPortrait {
                                            portrait_index: *portrait_index as u32,
                                            append: false,
                                        }
                                    }
                                };
                                dispatch_local_command(
                                    host,
                                    &mut manager.engine,
                                    &mut frame.commands,
                                    &assets,
                                    &cmd,
                                );
                            }
                            GameAction::QuickSave => {
                                // F9 (default binding).  The quick-
                                // save request rotates the previous
                                // QuickSave to ExQuickSave before
                                // writing — distinct from the generic
                                // `LevelSave` state-machine path.
                                //
                                // Defer the save until any active zoom
                                // finishes so the mid-zoom background
                                // isn't captured.
                                if !manager.engine.is_zoom_possible(&host.engine_display) {
                                    game.quick_save_after_zoom = true;
                                } else {
                                    let campaign = manager
                                        .engine
                                        .campaign()
                                        .expect("QuickSave requires the engine campaign");
                                    let mission_id =
                                        current_mission_id(campaign, &assets.profile_manager);
                                    callbacks.pending =
                                        Some(SaveLoadRequest::QuickSave { mission_id });
                                }
                            }
                            GameAction::QuickLoad => {
                                // F12 (default binding).  Loads the
                                // quick-save slot into the current
                                // engine, with a zoom-defer gate and a
                                // Shift+F12 → backup (ExQuickSave)
                                // shortcut.  The cross-mission
                                // confirmation modal is handled by
                                // `confirm_quickload_cross_mission`
                                // running before the per-frame
                                // `perform_pending_save_load` flush —
                                // it either drops the queued request
                                // (No) or rewrites it to
                                // `SaveLoadRequest::Load` so the
                                // cross-mission `PendingLevelLoad`
                                // routing performs the mission swap
                                // (Yes).
                                if !manager.engine.is_zoom_possible(&host.engine_display) {
                                    game.quick_load_after_zoom = true;
                                } else {
                                    callbacks.pending = Some(SaveLoadRequest::QuickLoad {
                                        use_backup: shift_held,
                                    });
                                }
                            }
                            GameAction::CrouchDown => {
                                // Prime the crouch-down focus latch
                                // before issuing the command so the
                                // down-arrow "pressed" overlay
                                // appears for the full transition.
                                // Snapshot the pre-command stature so
                                // the latch clears the first frame
                                // posture shifts.
                                let pre = manager.engine.retrieve_stature(None);
                                let cmd = PlayerCommand::CrouchDown;
                                dispatch_local_command(
                                    host,
                                    &mut manager.engine,
                                    &mut frame.commands,
                                    &assets,
                                    &cmd,
                                );
                                game.stature_focus.latch_crouch_down(pre);
                            }
                            GameAction::StandUp => {
                                // Companion of CrouchDown above —
                                // primes the stand-up focus latch so
                                // the up-arrow holds pressed while
                                // the sim runs the stand-up animation.
                                let pre = manager.engine.retrieve_stature(None);
                                let cmd = PlayerCommand::StandUp;
                                dispatch_local_command(
                                    host,
                                    &mut manager.engine,
                                    &mut frame.commands,
                                    &assets,
                                    &cmd,
                                );
                                game.stature_focus.latch_stand_up(pre);
                            }
                            GameAction::KeyControl => {
                                // Save the current action on every
                                // selected PC.  Used by the
                                // "move during action" modifier so
                                // ctrl-release can restore the action.
                                let cmd = PlayerCommand::KeyControl;
                                dispatch_local_command(
                                    host,
                                    &mut manager.engine,
                                    &mut frame.commands,
                                    &assets,
                                    &cmd,
                                );
                            }
                            GameAction::KeyReleaseControl => {
                                // Restore each selected PC's saved
                                // action on ctrl-up.  The handler
                                // honours the macOS carve-out via
                                // `cfg(target_os = "macos")`.
                                let cmd = PlayerCommand::KeyReleaseControl;
                                dispatch_local_command(
                                    host,
                                    &mut manager.engine,
                                    &mut frame.commands,
                                    &assets,
                                    &cmd,
                                );
                            }
                            GameAction::SwitchTask => {
                                // Emit a reset-input so held-key edges
                                // caught during an Alt+Tab / Ctrl+Esc
                                // task switch don't re-fire in-game
                                // when focus returns.  Route through
                                // the engine messenger so the drain
                                // handler applies the reset
                                // symmetrically with the hide-console
                                // path.
                                manager.engine.send_simple_message(
                                    engine_messenger::SimpleMessage::SwitchTask,
                                );
                            }
                            GameAction::Teleport => {
                                // F7 cheat — teleport every selected
                                // PC to the current mouse map point.
                                let mouse_screen = input.threaded.position();
                                if let Some(mouse_map) = host.viewport.screen_to_map(mouse_screen) {
                                    if !manager.engine.seat_selection(host.local_seat).is_empty() {
                                        // Resolve destination sector/layer
                                        // via `get_sector_screen_accessible`
                                        // and bail when it returns None.
                                        // Doors / motion obstacles / empty
                                        // cells are rejected up front rather
                                        // than going through as the topmost
                                        // hit.
                                        let accessible = manager
                                            .engine
                                            .fast_grid()
                                            .get_sector_screen_accessible(mouse_map);
                                        if let Some(sector_idx) = accessible.sector_idx {
                                            let cmd = PlayerCommand::TeleportSelectedToPoint {
                                                dest: mouse_map,
                                                layer: accessible.layer,
                                                sector: u16::try_from(u32::from(sector_idx))
                                                    .ok()
                                                    .and_then(
                                                    engine_position_interface::SectorHandle::new,
                                                ),
                                            };
                                            dispatch_local_command(
                                                host,
                                                &mut manager.engine,
                                                &mut frame.commands,
                                                &assets,
                                                &cmd,
                                            );
                                        }
                                    } else if dev.debug.free_shadow_polygon {
                                        // With no PCs selected and the
                                        // shadow-polygon dev cheat on,
                                        // reposition the free-floating
                                        // shadow-polygon viewer at the
                                        // mouse map point, 45 units
                                        // above the impact surface.
                                        // Non-sim dev state, handled
                                        // host-side outside the replay
                                        // pipeline.
                                        let p3d = manager.engine.fast_grid().convert_2d_to_3d(
                                            mouse_map,
                                            engine_sight_obstacle::SIGHTOBSTACLE_MOUSE,
                                            manager.engine.sight_obstacles(&assets),
                                        );
                                        dev.cheat_free_shadow_polygon_pos =
                                            Some(engine_coordinates::WorldPoint3D {
                                                x: p3d.x,
                                                y: p3d.y,
                                                z: p3d.z + 45.0,
                                            });
                                    }
                                }
                            }
                            GameAction::RecordQa => {
                                // F5 (default binding) — replay the
                                // corner-clock left-click behaviour:
                                // start / cycle the macro slot for
                                // the currently-selected PC(s).
                                if !game.is_sherwood {
                                    dispatch_corner_button_left_click(
                                        CornerButton::Clock,
                                        manager,
                                        game,
                                        host,
                                        &assets,
                                        &mut frame.commands,
                                    );
                                }
                            }
                            GameAction::PrintScreen => {
                                // Defer to the post-render drain so we
                                // capture the fully-composited frame
                                // rather than an incomplete in-progress draw
                                // queue. Ctrl matches the historical wide
                                // snapshot branch; Shift applies the 3x3
                                // median filter branch.
                                host.pending_print_screen = Some(
                                    print_screen_request_from_modifiers(ctrl_held, shift_held),
                                );
                            }
                            _ => {
                                tracing::trace!("Game action: {:?}", action);
                            }
                        }
                    }
                }
            }

            match handle_pause_menu_events(
                &mut ui.pause_menu,
                &mut pause_closed_this_frame,
                host,
                manager,
                game,
                &assets,
                callbacks,
                campaign_ref,
                &mut *window,
                &mut presentation.renderer,
                &mut resources.cursor,
                &mut presentation.sprites.cursor_renderer,
                &resources.menu,
                &mut audio.backend,
                &audio.sample_loader,
                &mut input.threaded,
                &mut input.translator,
                &mut hud.sherwood_layout,
                &mut hud.zoom_layout,
                &hud.zoom_sprites,
                &mut frame.commands,
                &events,
            )
            .await
            {
                HandlerAction::Continue => {
                    return Ok(FramePreparation::Control(FrameControl::RestartIteration));
                }
                HandlerAction::Exit(code) => {
                    execute_app_effects(
                        &mut callbacks.app_effects,
                        &mut host.sound,
                        &mut input.threaded,
                        audio
                            .backend
                            .as_mut()
                            .map(|backend| backend as &mut dyn crate::sound::AudioBackend),
                    );
                    return Ok(FramePreparation::Control(FrameControl::Exit(
                        MissionExit::new(code),
                    )));
                }
                HandlerAction::Proceed => {}
            }

            handle_mouse_input(
                manager,
                host,
                &assets,
                &presentation.renderer,
                &presentation.sprites.portrait_cache,
                &mut frame.commands,
                &events,
                ui.pause_menu.as_ref(),
                pause_closed_this_frame,
                shift_held,
                ctrl_held,
            );
        } // if runtime.replay_player.is_none()

        // ── Cross-mission QuickLoad confirmation modal ──
        // Quick-load prompts the
        // player with `MSG_REALLY_LOAD_QUICKSAVE` whenever the quicksave
        // header's mission ID differs from the running mission.  Run
        // the modal here, before the thumbnail capture and state-machine
        // drain — the helper either drops the pending request (No) or
        // rewrites it into a `Load` so the existing cross-mission
        // routing performs the mission swap (Yes).
        confirm_quickload_cross_mission(
            callbacks,
            &manager.engine,
            profiles,
            &host,
            &mut *window,
            &mut presentation.renderer,
            &mut resources.cursor,
            &mut presentation.sprites.cursor_renderer,
            &resources.menu,
        )
        .await;

        // ── Process game operations (save/load/quit/win/lose) ──
        //
        // The Game state machine queues save/load intents on the
        // callbacks; `perform_pending_save_load` then flushes them to
        // disk with live engine access.
        let exit_code = manager
            .engine
            .campaign()
            .and_then(|c| game.process_operation(c, profiles, callbacks));
        let pending_thumbnail = if callbacks
            .pending
            .as_ref()
            .is_some_and(|request| request.writes_save_payload())
            && !host.skip_render
            && !args.headless
            && !modal_rendered_this_frame
        {
            pre_render_engine_setup(manager, host, assets.as_ref(), &mut presentation.renderer);
            update_mouse_and_cursor(
                manager,
                host,
                &assets,
                &dev,
                &mut presentation.renderer,
                &mut resources.cursor,
                &mut presentation.sprites.cursor_renderer,
                &input.threaded,
                &presentation.sprites.portrait_cache,
                shift_held,
                &mut hud.last_cursor_id,
            );
            let display_snapshot = host.engine_display.clone();
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
                            campaign_ref,
                        ),
                },
            );
            capture_save_thumbnail(
                &manager.engine,
                &display_snapshot,
                host,
                &assets,
                &dev,
                &mut render_ctx,
            )
        } else {
            None
        };
        if let Some(exit_code) = exit_code {
            // `RHGame::GameLoop` applies transition sound/input changes
            // before returning its terminal code. Execute them here so
            // the mission-local Host is not dropped with queued effects.
            execute_app_effects(
                &mut callbacks.app_effects,
                &mut host.sound,
                &mut input.threaded,
                audio
                    .backend
                    .as_mut()
                    .map(|backend| backend as &mut dyn crate::sound::AudioBackend),
            );
            tracing::info!("Game exited with: {:?}", exit_code);
            // Flush any pending save before returning (e.g. the
            // quit-time continue save).
            let save_load_processed = perform_pending_save_load(
                host,
                game,
                callbacks,
                &mut manager.engine,
                assets.as_ref(),
                profiles,
                pending_thumbnail.clone(),
            );
            if save_load_processed && let Some(ref mut checker) = runtime.rollback_checker {
                checker.reset();
            }
            if let Some(sync) = callbacks.post_load_sync.take() {
                game.apply_post_load_sync(sync.is_continue);
                game.post_load_resolution_resync();
            }
            restore_engine_campaign(campaign_ref, &mut manager.engine, "completed mission exit");
            return Ok(FramePreparation::Control(FrameControl::Exit(
                MissionExit::new(exit_code),
            )));
        }
        let save_load_processed = perform_pending_save_load(
            host,
            game,
            callbacks,
            &mut manager.engine,
            assets.as_ref(),
            profiles,
            pending_thumbnail,
        );
        if save_load_processed && let Some(ref mut checker) = runtime.rollback_checker {
            checker.reset();
        }

        // ── Cross-mission load: bubble up ──
        // `perform_pending_save_load` stashes a `PendingLevelLoad` when the
        // chosen slot targets a different mission than the one running. Force
        // the Game state machine into LevelLoad so `process_operation` exits
        // on the next iteration; the outer session loop will switch missions
        // and re-queue the Load on the fresh engine.
        if callbacks.pending_level_load.is_some() {
            game.operation.set(GameCode::LevelLoad);
            restore_engine_campaign(campaign_ref, &mut manager.engine, "cross-mission load exit");
            return Ok(FramePreparation::Control(FrameControl::Exit(
                MissionExit::new(GameCode::LevelLoad),
            )));
        }

        // ── Post-load slot-type sync ──
        // Sync the continue-save flag and re-arm the campaign-map
        // overlay if the loaded save had it open.  `post_load_sync`
        // is armed by `perform_pending_save_load` after any Load
        // variant succeeds, threading the slot type back out of the
        // save-I/O layer.
        if let Some(sync) = callbacks.post_load_sync.take() {
            game.apply_post_load_sync(sync.is_continue);
            game.post_load_resolution_resync();
        }

        // ── Reset input state after load ──
        // Clear the translator's key-edge state so half-pressed keys
        // at save time don't emit stale edge-detection events on the
        // next frame.  Host-side `InputState` is already wiped by
        // `Host::post_load_reset` during `apply_to`; this clears the
        // mirror that lives on the input translator itself.
        if std::mem::take(&mut callbacks.pending_reset_input) {
            input.reset_after_engine_request();
        }

        // ── Save/load banner ──
        // `perform_pending_save_load` queues GAME_SAVED/GAME_LOADED
        // on every successful non-Restart/non-Sherwood save or load.
        // Threaded onto `game.message_text` / `message_delay` through
        // `Game::display_message`.
        if let Some(kind) = callbacks.pending_save_banner.take() {
            let text = match kind {
                SaveBannerKind::Saved => "Game saved.",
                SaveBannerKind::Loaded => "Game loaded.",
            };
            // 100 ticks — `display_message` is a fire-and-forget
            // delay that the presentation.renderer polls (the hook lives in
            // `render_frame` and calls
            // `hud_text::render_transient_message`).  IDs
            // `MT_MSG_GAME_SAVED` / `MT_MSG_GAME_LOADED` should be
            // wired for localisation later.
            game.display_message(text.to_string(), 100);
        }

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
            .is_some_and(|modal| !modal.is_empty());

        // Drain once more at the last deterministic pre-tick boundary.
        // Packets can arrive after the top-of-loop drain while this
        // frame handles UI, local input, and modal work.  Applying due
        // inputs here keeps them on the same `sim_frame` without
        // mutating sim state at arbitrary points in the frame.
        if host.net.is_some() && !rewind_active {
            if let Some(net) = host.net.as_ref() {
                net.publish_frame(manager.sim_frame);
            }
            let pre_tick_net_drain = drain_net_inputs(
                host,
                manager,
                assets.as_ref(),
                &mut runtime.rewind_buffer,
                &mut runtime.peer_hashes,
                &mut runtime.recent_timeline_history,
            );
            if pre_tick_net_drain.rewrote_sim_state
                && let Some(ref mut checker) = runtime.rollback_checker
            {
                checker.reset();
            }
            if let Some(rollback) = pre_tick_net_drain.rollback.clone() {
                runtime.last_mp_rollback = Some(rollback);
            }
            if let Some((_frame, start_epoch_ms)) = pre_tick_net_drain.begin_sim {
                runtime.mp_waiting_for_begin_sim = false;
                runtime.mp_start_gate = Some(start_epoch_ms);
                *manual_pause = true;
            }
            if runtime.mp_waiting_for_initial_snapshot
                && pre_tick_net_drain.received_initial_snapshot
            {
                runtime.mp_waiting_for_initial_snapshot = false;
                tracing::info!(
                    "multiplayer: initial snapshot received; client ready for start barrier"
                );
            }
            if runtime.mp_waiting_for_initial_snapshot || runtime.mp_waiting_for_begin_sim {
                *manual_pause = true;
            }
            if host.net.is_some()
                && host.local_seat != engine_player_command::PlayerId::HOST
                && let Some((clock_frame, ms_until_next_frame)) =
                    pre_tick_net_drain.latest_host_clock_sample
            {
                accept_host_frame_schedule(
                    &mut runtime.mp_host_frame_schedule,
                    clock_frame,
                    ms_until_next_frame,
                    manager.sim_frame,
                );
            }
            if host.net.is_some()
                && host.local_seat != engine_player_command::PlayerId::HOST
                && !runtime.mp_waiting_for_initial_snapshot
                && !runtime.mp_waiting_for_begin_sim
                && runtime.mp_start_gate.is_none()
            {
                if let Some(deadline_ms) = host_scheduled_frame_deadline_ms(
                    runtime.mp_host_frame_schedule,
                    manager.sim_frame,
                ) {
                    let now_ms = crate::window::process_uptime_ms();
                    let until_frame_ms = deadline_ms - i64::from(now_ms);
                    if until_frame_ms > 0 {
                        mp_clock_pause = true;
                        if now_ms.saturating_sub(runtime.last_mp_clock_ahead_log_ms) >= 1000 {
                            runtime.last_mp_clock_ahead_log_ms = now_ms;
                            tracing::info!(
                                scheduled_frame =
                                    runtime.mp_host_frame_schedule.map(|(frame, _)| frame),
                                local_frame = manager.sim_frame,
                                until_frame_ms,
                                "multiplayer: local frame is ahead of host schedule; holding sim"
                            );
                        }
                    }
                } else {
                    mp_clock_pause = true;
                }
            }
            if pre_tick_net_drain.rewrote_sim_state && host.net.is_some() {
                runtime
                    .recent_timeline_history
                    .checkpoint(manager.sim_frame, &manager.engine);
            }
            if !pre_tick_net_drain.inputs.is_empty() {
                manager.engine.apply_commands(
                    &mut host.engine_display,
                    &mut host.input,
                    &assets,
                    &pre_tick_net_drain.inputs,
                );
                frame.commands.commands.extend(pre_tick_net_drain.inputs);
            }
        }

        // ── Multiplayer: state hash broadcast / verify ──
        // Sample after the final deterministic pre-tick network drain.
        // Inputs can arrive between the top-of-loop drain and this
        // boundary; hashing earlier can compare two machines that will
        // tick the same commands but sampled before/after a current-frame
        // input that just arrived.
        if host.net.is_some()
            && manager
                .sim_frame
                .is_multiple_of(crate::multiplayer::STATE_HASH_INTERVAL)
        {
            if host.local_seat == engine_player_command::PlayerId::HOST
                && runtime.last_mp_state_hash_frame != Some(manager.sim_frame)
            {
                runtime.last_mp_state_hash_frame = Some(manager.sim_frame);
                let mp_hash_start = web_time::Instant::now();
                let live_hash_start = web_time::Instant::now();
                let local_hash = crate::replay::state_hash(&manager.engine);
                let live_hash_us = live_hash_start.elapsed().as_micros();
                runtime.pending_mp_state_hash = Some((manager.sim_frame, local_hash));

                let total_us = mp_hash_start.elapsed().as_micros();
                tracing::debug!(
                    frame = manager.sim_frame,
                    total_us,
                    live_hash_us,
                    "multiplayer hash frame timing"
                );
            } else if let Some(&host_hash) = runtime.peer_hashes.get(&manager.sim_frame) {
                let local_hash = crate::replay::state_hash(&manager.engine);
                if local_hash != host_hash {
                    let last_rollback_path =
                        runtime.last_mp_rollback.as_ref().map_or("none", |r| r.path);
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
                    tracing::error!(
                        frame = manager.sim_frame,
                        local = format!("{local_hash:016x}"),
                        host = format!("{host_hash:016x}"),
                        host_schedule_frame =
                            runtime.mp_host_frame_schedule.map(|(frame, _)| frame),
                        pending_input_frames = manager.pending_inputs.len(),
                        last_rollback_path,
                        last_rollback_earliest,
                        last_rollback_target,
                        last_rollback_replayed,
                        last_rollback_total_us,
                        "multiplayer DESYNC: local engine hash differs from host's"
                    );
                } else {
                    tracing::debug!(frame = manager.sim_frame, "multiplayer hash OK");
                }
            }
            // Stale entries: drop everything strictly older than
            // sim_frame so the map doesn't grow unbounded if the
            // host sends ahead of our verification.
            runtime.peer_hashes.retain(|&f, _| f > manager.sim_frame);
        }

        let mut paused = ui.pause_menu.is_some() || *manual_pause || mp_clock_pause || modal_pause;

        if let Some(ref mut player) = runtime.replay_player
            && !paused
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
                // Hash check for this frame was already done at the top of
                // the loop (see the record/check block after begin_frame),
                // so the check and the recorder write share the same
                // engine-state sampling point and can't drift.
                frame.inject_replay_commands(player, host, manager, &assets);
                // Discard any live input commands during replay, then stash
                // the commands we actually applied so the rewind buffer's
                // per-frame command log captures them — otherwise a later
                // step-back during replay has nothing to walk forward from
                // its snapshots.  Recording is still a no-op (the recorder
                // gate below short-circuits when `runtime.replay_recorder` is None,
                // which it always is in replay mode).
            }
        }

        // ── Post-rewind auto-replay ──
        // When the player releases the rewind key, `sim_frame` ends up
        // inside the buffer's recorded range and the original
        // `[sim_frame .. next_record_frame)` commands are still
        // buffered.  Keep replaying that future forward one frame at a
        // time until a live input fires — at which point the player
        // has chosen to diverge, so truncate the now-orphaned future
        // out of the buffer and record fresh commands from here on.
        //
        // Paused frames don't tick, so they'd consume the same
        // buffered slot repeatedly; skipped here for the same reason
        // the tick below is.  Replay playback (`--replay`) has its
        // own command stream and stays unaffected (guarded below by
        // the separate `runtime.replay_player.is_none()` check).
        let mut consumed_buffered = false;
        if !rewind_active
            && !paused
            && runtime.replay_player.is_none()
            && manager.sim_frame < runtime.rewind_buffer.next_record_frame()
        {
            if frame.commands.commands.is_empty() {
                if let Some(recorded) = runtime.rewind_buffer.commands_for(manager.sim_frame) {
                    let recorded: Vec<PlayerInput> = recorded.to_vec();
                    manager.engine.apply_commands(
                        &mut host.engine_display,
                        &mut host.input,
                        &assets,
                        &recorded,
                    );
                    frame.commands.commands = recorded;
                    consumed_buffered = true;
                    tracing::trace!("Auto-replay → frame {}", manager.sim_frame);
                }
            } else {
                tracing::trace!(
                    "Auto-replay interrupted by live input; truncating buffer at {}",
                    manager.sim_frame
                );
                runtime.rewind_buffer.truncate_future(manager.sim_frame);
            }
        }

        // ── Locker follow hover ──
        // `SelectFollowElement` mutates sim-visible seat/camera state.
        // Keep it in the recorded pre-tick command stream; doing this
        // from the render cursor pass applies it after rollback/rewind
        // have committed the frame and leaves no command to replay.
        if runtime.replay_player.is_none()
            && !rewind_active
            && !paused
            && manager.engine.locker_active()
            && let Some(mouse_map) = host.viewport.screen_to_map(input.threaded.position())
            && let Some(id) =
                manager
                    .engine
                    .find_focusable_npc(&assets, mouse_map, engine_element::Focus::View)
        {
            let cmd = PlayerCommand::SelectFollowElement {
                entity_id: Some(id),
            };
            dispatch_local_command(
                host,
                &mut manager.engine,
                &mut frame.commands,
                &assets,
                &cmd,
            );
        }

        // ── Per-frame aim orientation ──
        // This is sim state (direction/current animation row and
        // bow raise/lower command launch), so it must be recorded in
        // the same frame command log as clicks and keys.  Do not run
        // it from `host_mouse::update_mouse`: render happens after
        // rollback has committed the frame, and live-only mutation
        // there desynchronizes replay/rollback.
        if runtime.replay_player.is_none()
            && !rewind_active
            && !paused
            && let Some(mouse_map) = host.viewport.screen_to_map(input.threaded.position())
        {
            let bow_armed = manager.engine.selected_action_for_seat(host.local_seat)
                == engine_profiles::Action::Bow;
            if host.time_no_mouse_move != 0 || bow_armed {
                let cmd = PlayerCommand::PerformOrientation { mouse_map };
                dispatch_local_command(
                    host,
                    &mut manager.engine,
                    &mut frame.commands,
                    &assets,
                    &cmd,
                );
            }
        }

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

    /// Run the deterministic tick, timeline/step bookkeeping, and scripted
    /// modal flow before handing the completed frame to presentation.
    async fn simulate_interactive_frame(
        &mut self,
        services: &mut MissionServices<'_>,
        prepared: PreparedFrame,
    ) -> Result<FrameControl, String> {
        let window = &mut *services.window;
        let callbacks = &mut *services.callbacks;
        let campaign_ref = &mut *services.campaign;
        let profiles = services.profiles;
        let args = services.args;
        let InteractiveMission { runtime, frontend } = self;
        let MissionRuntime {
            world,
            timeline: runtime,
            control,
        } = runtime;
        let MissionWorld {
            host,
            game,
            manager,
            assets,
            dev,
        } = world;
        let MissionControl {
            manual_pause,
            last_shadow_color,
            ..
        } = control;
        let input = &mut frontend.input;
        let audio = &mut frontend.audio;
        let resources = &mut frontend.resources;
        let ui = &mut frontend.ui;
        let presentation = &mut frontend.presentation;
        let PreparedFrame {
            mut frame,
            rewind_active,
            paused,
            consumed_buffered,
            shift_held,
            modal_rendered: mut modal_rendered_this_frame,
            step_forward_pressed,
            step_back_pressed,
        } = prepared;

        // ── Record frame commands + periodic state hash ──
        // The matching `recorder.end_frame()` runs after the modal
        // drain block so `ModalDismiss` entries land in the same
        // frame as the modal that produced them.  Skipped while
        // rewinding (no tick is running) and while consuming buffered
        // commands (they were already written to disk on the original
        // pass). The hash itself was computed at the top of the
        // frame into `frame.recorder_hash` — writing it here
        // keeps the gating in one place.
        runtime.record_commands(
            frame.recorder_hash,
            &frame.commands.commands,
            !rewind_active && !consumed_buffered,
        );

        // ── Engine tick ──
        // The pause menu freezes the simulation by skipping the
        // hourglass while the menu is shown.  Rewind also freezes
        // the tick: the engine state was just replaced with a
        // reconstruction of an earlier frame and must not be
        // advanced this frame.
        let tick_exit_code = runtime.run_simulation(|| {
            if rewind_active {
                return None;
            }
            let mut display = std::mem::take(&mut host.engine_display);
            let result = game.run_engine_tick(
                host,
                &mut display,
                assets.as_ref(),
                &mut manager.engine,
                dev,
                false,
                paused,
            );
            host.engine_display = display;
            result
        });

        // ── Drain pending script-RPC requests ──
        // External tools (HTTP /native, /command, /console, /state, …)
        // queue invocations on the server thread; we run them here so
        // any side-effect commands (camera, dialog, sequences, sound,
        // PlayerCommand applies) land on the same frame as the tick
        // that just finished.  No-op when the HTTP server is disabled
        // or the mission isn't loaded yet (each handler returns an
        // `Err` that's relayed back).
        let net = host.net.take();
        crate::http_server::drain_global(manager, host, &assets, net.as_ref());
        host.net = net;

        // ── Rollback check + rewind buffer commit ──
        // Both are post-tick bookkeeping.  Skipped on paused frames
        // (no tick ran) and rewind frames (tick was suppressed).  The
        // rewind buffer also skips commits while consuming its own
        // log — the slot is already populated and would duplicate.
        if !paused && !rewind_active {
            runtime.commit_simulation_history(
                host,
                manager,
                &frame,
                FrameCommitPolicy {
                    store_rewind_commands: !consumed_buffered,
                },
            );
            manager.sim_frame += 1;
            if let Some(net) = host.net.as_ref()
                && host.local_seat == engine_player_command::PlayerId::HOST
            {
                net.set_initial_snapshot(manager.sim_frame, &manager.engine);
            }
        }

        // ── Pending `/step-forward` / `/step-back` requests ──
        // Run each queued step synchronously with its own tick +
        // bookkeeping (forward) or rewind-buffer seek (back).  These
        // requests intentionally bypass the `paused` gate — their whole
        // purpose is to drive the sim from a paused state — but still
        // refuse if a modal dialog is queued so the user doesn't step
        // past it.
        drain_steps(
            manager,
            host,
            assets.as_ref(),
            dev,
            game,
            &mut runtime.rewind_buffer,
            &mut runtime.rollback_checker,
            &mut runtime.replay_player,
            manual_pause,
            &mut ui.active_modal,
        );

        // Publish replay-playback status for the script-RPC `state`
        // endpoint so JS timelines can render a playhead.  `None`
        // when we're not replaying — the state response will carry
        // `null` for `replay`, the JS UI's "hide me" signal.
        crate::http_server::set_replay_status(runtime.replay_player.as_ref().map(|p| {
            crate::http_server::ReplayStatus {
                frame: p.current_frame(),
                total: p.total_frames(),
                paused: *manual_pause,
            }
        }));

        // ── Keyboard-driven single-frame step (`.` / `,`) ──
        // Same bookkeeping as the HTTP `/step-forward` / `/step-back`
        // requests handled in `drain_steps`, but driven by the local
        // keybindings and without a network reply.  Refused while a
        // modal is pending for the same reason (stepping past a queued
        // dialog would skip it).
        //
        // During replay, the main per-frame replay advance is skipped
        // (gated on `!paused`) so the step handlers drive the replay
        // cursor themselves: forward pulls the next recorded commands
        // and applies them before the tick; back seeks the cursor to
        // the rewound frame so playback resumes from there.
        if step_forward_pressed && !modal_state_pending(&host) {
            let mut step_frame_cmds: Vec<PlayerInput> = Vec::new();
            if let Some(ref mut player) = runtime.replay_player
                && !player.is_finished()
            {
                let replay_cmds = player.next_frame();
                for cmd in replay_cmds {
                    if matches!(cmd.command, PlayerCommand::ModalDismiss { .. }) {
                        tracing::debug!(
                            "step-forward: dropping recorded ModalDismiss at frame {}",
                            manager.sim_frame
                        );
                        continue;
                    }
                    step_frame_cmds.push(cmd.clone());
                }
                manager.engine.apply_commands(
                    &mut host.engine_display,
                    &mut host.input,
                    &assets,
                    &step_frame_cmds,
                );
            }
            let mut display = std::mem::take(&mut host.engine_display);
            game.run_engine_tick(
                host,
                &mut display,
                assets.as_ref(),
                &mut manager.engine,
                dev,
                false,
                false,
            );
            crate::sim_timeline::run_post_initialize_stage(
                host,
                &mut display,
                &assets,
                &mut manager.engine,
                dev,
            );
            host.engine_display = display;
            runtime.rewind_buffer.end_frame(step_frame_cmds);
            manager.sim_frame += 1;
            // Stepping bypasses the checker's begin_frame/end_frame
            // pairing, so its ring buffer is now stale relative to the
            // advanced engine.  Clear it — the checker resumes
            // populating on the next normal frame.
            if let Some(ref mut checker) = runtime.rollback_checker {
                checker.reset();
            }
        } else if step_back_pressed && !modal_state_pending(&host) {
            if let Some(target) = manager.sim_frame.checked_sub(1)
                && let Some(oldest) = runtime.rewind_buffer.oldest_reachable_frame()
                && target >= oldest
            {
                runtime.rewind_buffer.begin_session();
                let rewound = runtime.rewind_buffer.rewind_to(&assets, target);
                runtime.rewind_buffer.end_session();
                if let Some(new_engine) = rewound {
                    manager.engine = new_engine;
                    manager.sim_frame = target;
                    // Keep the replay cursor in sync with the rewound
                    // sim frame so resuming playback re-applies the
                    // right commands.
                    if let Some(ref mut player) = runtime.replay_player {
                        player.seek(target);
                    }
                    // The rollback checker's ring now references a
                    // timeline the live engine is no longer on; clear
                    // it so the next normal frame starts a fresh
                    // window.
                    if let Some(ref mut checker) = runtime.rollback_checker {
                        checker.reset();
                    }
                } else {
                    tracing::warn!("step-back: rewind_to({target}) failed");
                }
            } else {
                tracing::debug!("step-back: already at oldest retained frame");
            }
        }

        // ── Rebind shadow key on ambience change ──
        // The shadow key is baked into frame dictionaries at load
        // time and never re-run, which would leave loaded sprites
        // with a stale shadow key if the ambience ever changed
        // (day→fog, day→night). Poll the engine state's
        // `night_color` and re-run the shadow-dependent host
        // renderers on change. No current code path mutates
        // `weather.ambiance` post-load, so this is dormant until a
        // future weather/scripting feature wires a trigger.
        let current_shadow_color = manager.engine.weather().night_color;
        if current_shadow_color != *last_shadow_color {
            tracing::info!(
                "Ambience shadow-key changed {:#06x} → {:#06x}; rebinding sprite caches",
                last_shadow_color,
                current_shadow_color,
            );
            presentation.rebind_shadow_key(resources, host, &window.gpu, current_shadow_color);
            // Frame counts don't change on a shadow-key rebind — same
            // resource rows reloaded with a different shadow colour —
            // so the engine's `titbit_row_frame_counts` stays valid.
            *last_shadow_color = current_shadow_color;
        }

        // ── Expand DisplayAll cheats ──
        // Console `LEVEL TEXT D/DB/PT` sets `dev.debug.all_*` bools.
        // The engine tick can't expand them because level descriptors
        // live host-side; we do the expansion here using the same
        // typed IDs the drain code below already understands.
        if dev.debug.all_dialogues {
            dev.debug.all_dialogues = false;
            if let Some(descriptors) = &resources.level_descriptors {
                let count = descriptors.dialogues.len();
                host.pending_dialogues.extend((0..count).map(|i| i as i32));
            } else {
                tracing::warn!("cheat all_dialogues: level descriptors unavailable");
            }
        }
        if dev.debug.all_popup_texts {
            dev.debug.all_popup_texts = false;
            if let Some(descriptors) = &resources.level_descriptors {
                let count = descriptors.popup_text.picture_ids.len();
                host.pending_popup_texts
                    .extend((0..count).map(|i| i as i32));
            } else {
                tracing::warn!("cheat all_popup_texts: level descriptors unavailable");
            }
        }
        if dev.debug.all_debriefings {
            dev.debug.all_debriefings = false;
            if let Some(descriptors) = &resources.level_descriptors {
                let lose = descriptors.debriefing.lose_count as usize;
                let win = descriptors.debriefing.win_count as usize;
                host.pending_debriefings.extend(
                    (0..lose).map(|index| engine_player_command::DebriefingTextId::Lose { index }),
                );
                host.pending_debriefings.extend(
                    (0..win).map(|index| engine_player_command::DebriefingTextId::Win { index }),
                );
            } else {
                tracing::warn!("cheat all_debriefings: level descriptors unavailable");
            }
        }

        if args.headless {
            drain_pending_dialogues(
                host,
                &mut *window,
                &mut presentation.renderer,
                &mut resources.cursor,
                &mut presentation.sprites.cursor_renderer,
                &mut audio.backend,
                &mut resources.text,
                &game,
                &resources.level_descriptors,
                &mut resources.menu,
                &mut runtime.replay_recorder,
                &mut frame.replay_modal_dismissals,
                true,
            )
            .await;
        } else {
            if ui.active_modal.is_none()
                && let Some(batch) = start_active_dialogue_batch(
                    host,
                    &mut resources.text,
                    &game,
                    &resources.level_descriptors,
                    &mut frame.replay_modal_dismissals,
                )
            {
                ui.active_modal = Some(ActiveModal::Dialogue(Box::new(batch)));
            }
            if ui.active_modal.is_some() {
                let outcome = tick_active_modal(
                    &mut ui.active_modal,
                    host,
                    &mut *window,
                    &mut presentation.renderer,
                    &mut resources.cursor,
                    &mut presentation.sprites.cursor_renderer,
                    &mut audio.backend,
                    &audio.sample_loader,
                    &mut resources.menu,
                    &mut runtime.replay_recorder,
                );
                debug_assert_eq!(outcome, ActiveModalOutcome::None);
                modal_rendered_this_frame = true;
            }
        }

        if !modal_rendered_this_frame && args.headless {
            drain_pending_popup_scroll(
                host,
                &mut *window,
                &mut presentation.renderer,
                &mut resources.cursor,
                &mut presentation.sprites.cursor_renderer,
                &mut audio.backend,
                &audio.sample_loader,
                &mut resources.text,
                &resources.level_descriptors,
                &mut resources.menu,
                &mut runtime.replay_recorder,
                &mut frame.replay_modal_dismissals,
                manager.engine.frame_counter(),
            )
            .await;
            drain_pending_sherwood_stat(
                host,
                &mut *window,
                &mut presentation.renderer,
                &mut resources.cursor,
                &mut presentation.sprites.cursor_renderer,
                &manager.engine,
                profiles,
                &mut audio.backend,
                &audio.sample_loader,
                &mut resources.menu,
                &mut runtime.replay_recorder,
                &mut frame.replay_modal_dismissals,
            )
            .await;
        } else if !modal_rendered_this_frame {
            if ui.active_modal.is_none()
                && let Some(batch) = start_active_popup_scroll_batch(
                    host,
                    &mut presentation.renderer,
                    &mut resources.text,
                    &resources.level_descriptors,
                    &mut resources.menu,
                    &mut frame.replay_modal_dismissals,
                    manager.engine.frame_counter(),
                )
            {
                ui.active_modal = Some(ActiveModal::PopupScroll(Box::new(batch)));
            }
            if ui.active_modal.is_none()
                && let Some(batch) = start_active_sherwood_report(
                    host,
                    &manager.engine,
                    profiles,
                    &mut resources.menu,
                    &mut frame.replay_modal_dismissals,
                )
            {
                ui.active_modal = Some(ActiveModal::PopupScroll(Box::new(batch)));
            }
            if ui.active_modal.is_some() {
                let outcome = tick_active_modal(
                    &mut ui.active_modal,
                    host,
                    &mut *window,
                    &mut presentation.renderer,
                    &mut resources.cursor,
                    &mut presentation.sprites.cursor_renderer,
                    &mut audio.backend,
                    &audio.sample_loader,
                    &mut resources.menu,
                    &mut runtime.replay_recorder,
                );
                debug_assert_eq!(outcome, ActiveModalOutcome::None);
                modal_rendered_this_frame = true;
            }
        }

        if !modal_rendered_this_frame && args.headless {
            drain_pending_debriefings(
                host,
                &mut *window,
                &mut presentation.renderer,
                &mut resources.cursor,
                &mut presentation.sprites.cursor_renderer,
                &mut resources.text,
                &resources.level_descriptors,
                &resources.menu,
                &mut runtime.replay_recorder,
                &mut frame.replay_modal_dismissals,
            )
            .await;
        } else if !modal_rendered_this_frame {
            if ui.active_modal.is_none()
                && let Some(batch) = start_active_debriefing_batch(
                    host,
                    &mut resources.text,
                    &resources.level_descriptors,
                    &resources.menu,
                    &mut frame.replay_modal_dismissals,
                )
            {
                ui.active_modal = Some(ActiveModal::Debriefing(Box::new(batch)));
            }
            if ui.active_modal.is_some() {
                let outcome = tick_active_modal(
                    &mut ui.active_modal,
                    host,
                    &mut *window,
                    &mut presentation.renderer,
                    &mut resources.cursor,
                    &mut presentation.sprites.cursor_renderer,
                    &mut audio.backend,
                    &audio.sample_loader,
                    &mut resources.menu,
                    &mut runtime.replay_recorder,
                );
                debug_assert_eq!(outcome, ActiveModalOutcome::None);
                modal_rendered_this_frame = true;
            }
        }

        drain_pending_console_display(host, &mut ui.console_overlay);

        // First-time mission-won "leave mission now" banner
        // Blocks the main loop briefly to
        // show the popup; if the player confirms we kick the normal
        // quit-mission flow by queuing `SimpleMessage::QuitMission`
        // (the same path the quit-mission widget would have driven
        // before it was disabled by `Game::perform_hourglass_*`).
        if !modal_rendered_this_frame
            && (host.pending_mission_state_popup || ui.active_modal.is_some())
        {
            if host.pending_mission_state_popup {
                host.pending_mission_state_popup = false;
                if args.headless {
                    let cmd = PlayerCommand::QuitMissionRequested;
                    dispatch_local_command(
                        host,
                        &mut manager.engine,
                        &mut frame.commands,
                        &assets,
                        &cmd,
                    );
                    frame.commands.push(cmd);
                } else if let Some(resources) = resources.menu.as_ref() {
                    let kind = engine_player_command::ModalKind::MissionState {
                        kind: engine_player_command::MissionStateModalKind::LeaveMissionNow,
                    };
                    let replay_result =
                        pop_matching_dismissal(&mut frame.replay_modal_dismissals, &kind);
                    let message = resources.menu_text.get(MT_MSG_LEAVE_MISSION_NOW);
                    let message_str = if message.is_empty() {
                        "You may leave the mission now.".to_string()
                    } else {
                        message
                    };
                    ui.active_modal = Some(ActiveModal::MissionState {
                        kind,
                        state: MissionStatePopupState::new(
                            &presentation.renderer,
                            resources,
                            message_str,
                            true,
                            None,
                        ),
                        replay_result,
                    });
                }
            }

            if ui.active_modal.is_some() {
                let outcome = tick_active_modal(
                    &mut ui.active_modal,
                    host,
                    &mut *window,
                    &mut presentation.renderer,
                    &mut resources.cursor,
                    &mut presentation.sprites.cursor_renderer,
                    &mut audio.backend,
                    &audio.sample_loader,
                    &mut resources.menu,
                    &mut runtime.replay_recorder,
                );
                modal_rendered_this_frame = true;
                if outcome == ActiveModalOutcome::QuitMissionRequested {
                    // Route through the command pipeline so replay /
                    // rollback reproduce the quit deterministically.
                    // The command sets `quit_won` when the mission
                    // is already marked won (our first-time-mission-
                    // won path) so the next tick returns
                    // `LevelSucceeded`.
                    let cmd = PlayerCommand::QuitMissionRequested;
                    dispatch_local_command(
                        host,
                        &mut manager.engine,
                        &mut frame.commands,
                        &assets,
                        &cmd,
                    );
                }
            }
        }

        // Drain zoom-deferred QuickSave / QuickLoad: pressing F9/F12
        // during an in-flight zoom is held until the transition
        // settles so we don't snapshot or overwrite a mid-zoom
        // engine.  Once `is_zoom_possible()` reports clear, enqueue
        // the same request the live key would have produced.
        if manager.engine.is_zoom_possible(&host.engine_display) {
            if game.quick_save_after_zoom {
                game.quick_save_after_zoom = false;
                let campaign = manager
                    .engine
                    .campaign()
                    .expect("deferred QuickSave requires the engine campaign");
                let mission_id = current_mission_id(campaign, &assets.profile_manager);
                callbacks.pending = Some(SaveLoadRequest::QuickSave { mission_id });
            }
            if game.quick_load_after_zoom {
                game.quick_load_after_zoom = false;
                // Shift state at drain time differs from press time;
                // re-read it on the deferred fire to keep the
                // shift-modifier semantics intact.
                callbacks.pending = Some(SaveLoadRequest::QuickLoad {
                    use_backup: shift_held,
                });
            }
        }

        // Drain `host.pending_reset_input` — set when the engine
        // consumed `SimpleMessage::ResetInput` during this tick,
        // which fires after a modal dialogue / popup / Sherwood
        // report closes.  Zeroes mouse/keyboard state so held-key
        // edges from the modal don't re-fire as gameplay actions.
        // Re-syncs the host cursor latches and clears the
        // InputTranslator's edge-detection buffer too so the next
        // `translate_keyboard` pass sees fresh edges.
        if host.pending_reset_input {
            host.pending_reset_input = false;
            input.reset_after_engine_request();
            host.input.left_mouse_down = false;
            host.input.right_mouse_down = false;
            host.input.is_dragging = false;
            host.input.multi_selection_active = false;
            host.input.multi_unselection_active = false;
            host.input.draw_multi_selection = false;
        }

        if let Some(exit_code) = tick_exit_code {
            tracing::info!("Engine tick returned: {:?}", exit_code);

            // Apply quit-mission updates (stat sync, coma reset,
            // score bonuses, warcrime recruitment, blazon
            // consumption) before showing the debriefing so it
            // displays correct stats.  The engine internally
            // takes/restores its owned campaign.
            dispatch_local_command(
                host,
                &mut manager.engine,
                &mut frame.commands,
                &assets,
                &PlayerCommand::ApplyQuitMissionUpdates { exit_code },
            );

            // Show the mission state popup + debriefing synchronously
            // now, while the presentation.renderer and menu resources are still
            // alive.  `show_debriefing` blocks the loop until the
            // player dismisses it.
            if let (Some((popup_title, _popup_body)), Some(menu_resources)) = (
                crate::ingame_menu::mission_state_text(exit_code),
                &resources.menu,
            ) {
                let won = exit_code == GameCode::LevelSucceeded;
                let mission_state_kind = engine_player_command::ModalKind::MissionState {
                    kind: engine_player_command::MissionStateModalKind::EndState { won },
                };
                let mission_state_result = match pop_matching_dismissal(
                    &mut frame.replay_modal_dismissals,
                    &mission_state_kind,
                ) {
                    Some(
                        result @ (engine_player_command::DialogResult::Completed
                        | engine_player_command::DialogResult::Aborted),
                    ) => result,
                    Some(result) => {
                        tracing::warn!(
                            ?result,
                            "final mission-state replay result is only yes/no; treating as aborted"
                        );
                        engine_player_command::DialogResult::Aborted
                    }
                    None => {
                        let cursor = Some(default_modal_cursor(
                            &mut presentation.sprites.cursor_renderer,
                            &mut resources.cursor,
                            &mut presentation.renderer,
                        ));
                        let confirmed = crate::ingame_menu::show_mission_state_popup(
                            &mut *window,
                            &mut presentation.renderer,
                            menu_resources,
                            cursor,
                            popup_title,
                            won,
                            None,
                        )
                        .await;
                        if confirmed {
                            engine_player_command::DialogResult::Completed
                        } else {
                            engine_player_command::DialogResult::Aborted
                        }
                    }
                };
                if let Some(recorder) = runtime.replay_recorder.as_mut() {
                    recorder.push(engine_player_command::PlayerCommand::ModalDismiss {
                        kind: mission_state_kind,
                        result: mission_state_result,
                    });
                }
                // Resolve the per-mission debriefing prose from the
                // level's text resource table: pick win or lose
                // table_id depending on `won`, then look up the
                // string at `victory_defeat_id` (set by the
                // script-side `ChooseVictoryDefeatText`).  On any
                // failure, fall back to a placeholder so the body is
                // never empty.
                let debriefing_index = manager.engine.mission().victory_defeat_id as usize;
                let debriefing_kind = engine_player_command::ModalKind::FinalDebriefing {
                    text_id: engine_player_command::DebriefingTextId::from_outcome(
                        won,
                        debriefing_index,
                    ),
                };
                let debriefing_body =
                    if let Some(descriptors) = resources.level_descriptors.as_ref() {
                        let table_id = if won {
                            descriptors.debriefing.win_text_table_id
                        } else {
                            descriptors.debriefing.lose_text_table_id
                        };
                        match resources.text.get_string(table_id, debriefing_index) {
                            Ok(s) => s.to_string(),
                            Err(e) => {
                                tracing::warn!(
                                    "Debriefing text lookup failed (table={table_id}, \
                                     index={debriefing_index}): {e}"
                                );
                                "Invalid debriefing ID...".to_string()
                            }
                        }
                    } else {
                        tracing::warn!("Debriefing text lookup: level descriptors unavailable");
                        "No dynamic resources for this level...".to_string()
                    };
                // Feed the mission-stat panel through the mission-clock
                // abstraction. The current implementation returns the
                // deterministic campaign counter, which advances from
                // completed sim seconds.
                let mission_length = manager
                    .engine
                    .campaign()
                    .map(|c| {
                        <RustCallbacks as crate::game::GameCallbacks>::get_current_playing_time(
                            callbacks, c,
                        )
                    })
                    .unwrap_or(0);
                // When restart is allowed, the debriefing accepts a
                // QuickLoad keypress to short-circuit into a load.
                // Pull the configured `QuickLoad1` key out of
                // the input translator so the modal can fire on that
                // key.
                let quick_load_key = input.translator.get_binding(GameKey::QuickLoad1);
                // Restart only fires when a restart snapshot exists.
                // When missing, the body window closes and the stat
                // panel still shows.  Probe the save manager up
                // front so the modal can short-circuit a no-snapshot
                // Restart click to "skip body, show stat".
                let restart_snapshot_exists =
                    ui.restart_allowed && callbacks.save_manager.has_restart_save();
                let campaign = manager
                    .engine
                    .campaign()
                    .expect("mission debriefing requires the engine campaign");
                let mission_id = current_mission_id(campaign, &assets.profile_manager);

                // Re-entry loop for the Load button: clicking Load
                // chains into the save-load picker; if the picker is
                // cancelled, the current debriefing page stays
                // visible and the player can continue interacting
                // with it.  We model that by re-entering
                // `show_debriefing` with the page text the player
                // was viewing when they clicked Load.
                let post_load_outcome = if let Some(result) =
                    pop_matching_dismissal(&mut frame.replay_modal_dismissals, &debriefing_kind)
                {
                    final_debriefing_outcome_from_replay(result)
                } else {
                    let mut current_body = debriefing_body.clone();
                    let mut start_at_stat = false;
                    loop {
                        let cursor = Some(default_modal_cursor(
                            &mut presentation.sprites.cursor_renderer,
                            &mut resources.cursor,
                            &mut presentation.renderer,
                        ));
                        let outcome = crate::ingame_menu::show_debriefing(
                            &mut *window,
                            &mut presentation.renderer,
                            menu_resources,
                            cursor,
                            &current_body,
                            Some(manager.engine.mission_stat()),
                            mission_length,
                            won,
                            ui.restart_allowed,
                            quick_load_key,
                            restart_snapshot_exists,
                            start_at_stat,
                        )
                        .await;
                        match outcome {
                            DebriefingOutcome::LoadAttempt {
                                body_remaining,
                                was_on_stat,
                            } => {
                                // Run the save-load picker.  If a slot is
                                // selected we'll re-enter the loop with a
                                // synthetic outcome below; otherwise we
                                // re-show the same page (body or stat).
                                let cursor = Some(default_modal_cursor(
                                    &mut presentation.sprites.cursor_renderer,
                                    &mut resources.cursor,
                                    &mut presentation.renderer,
                                ));
                                let picker_outcome = crate::ingame_menu::show_save_load(
                                    &mut *window,
                                    &mut presentation.renderer,
                                    menu_resources,
                                    cursor,
                                    &mut callbacks.save_manager,
                                    mission_id,
                                    Some(&assets.profile_manager),
                                    SaveLoadMode::Load,
                                    Some(&mut host.sound),
                                    audio
                                        .backend
                                        .as_mut()
                                        .map(|b| b as &mut dyn crate::sound::AudioBackend),
                                    Some(&audio.sample_loader),
                                )
                                .await;
                                match picker_outcome {
                                    SaveLoadOutcome::Slot(slot) => {
                                        // Synthesise a Load-resolved outcome and
                                        // exit the re-entry loop.  Stored in
                                        // `post_load_outcome` so the match
                                        // below processes it uniformly.
                                        break SettledDebriefingOutcome::Load { slot };
                                    }
                                    SaveLoadOutcome::Cancel => {
                                        // Picker cancelled — re-enter the
                                        // debriefing on the same page.
                                        current_body = body_remaining;
                                        start_at_stat = was_on_stat;
                                        continue;
                                    }
                                }
                            }
                            DebriefingOutcome::Ok { .. } => {
                                break SettledDebriefingOutcome::Ok;
                            }
                            DebriefingOutcome::Restart => {
                                break SettledDebriefingOutcome::Restart;
                            }
                            DebriefingOutcome::EmergencyEnd => {
                                break SettledDebriefingOutcome::EmergencyEnd;
                            }
                        }
                    }
                };
                if let Some(recorder) = runtime.replay_recorder.as_mut() {
                    recorder.push(engine_player_command::PlayerCommand::ModalDismiss {
                        kind: debriefing_kind,
                        result: final_debriefing_result(&post_load_outcome),
                    });
                }

                // Wire the Load/Restart outcomes back into the game
                // state machine.  Both funnel through the engine's
                // save-game slot machinery rather than re-running
                // the mission cold.
                match post_load_outcome {
                    SettledDebriefingOutcome::Ok => {
                        // Normal dismissal — let the exit_code flow
                        // through the Game state machine on the next
                        // frame's `process_operation`.
                    }
                    SettledDebriefingOutcome::Restart => {
                        // We've already verified the restart snapshot
                        // exists via `restart_snapshot_exists`; queue
                        // `SaveLoadRequest::LoadRestart` and reset
                        // `game.operation` so the next frame's
                        // `perform_pending_save_load` applies it in
                        // place.
                        callbacks.pending = Some(SaveLoadRequest::LoadRestart);
                        game.operation.set(GameCode::LevelInProgress);
                    }
                    SettledDebriefingOutcome::Load { slot } => {
                        // The Load button chains into the save-load
                        // picker (run inline above) and queues a
                        // level load.
                        callbacks.pending = Some(SaveLoadRequest::Load {
                            slot: Some(slot),
                            mission_id,
                        });
                        game.operation.set(GameCode::LevelInProgress);
                    }
                    SettledDebriefingOutcome::EmergencyEnd => {
                        // External force-close (window close / Alt-
                        // F4) propagates as `GameCode::Quit` so
                        // `handle_quit` writes the continue-save and
                        // the outer session returns to the main
                        // menu.
                        if let Some(ref mut recorder) = runtime.replay_recorder
                            && !rewind_active
                            && !consumed_buffered
                        {
                            recorder.end_frame();
                        }
                        restore_engine_campaign(
                            campaign_ref,
                            &mut manager.engine,
                            "emergency debriefing exit",
                        );
                        return Ok(FrameControl::Exit(MissionExit::new(GameCode::Quit)));
                    }
                }
            }
        }

        let presentation_state = FramePresentationState {
            frame,
            rewind_active,
            consumed_buffered,
            shift_held,
            modal_rendered: modal_rendered_this_frame,
        };
        self.finish_interactive_frame(services, presentation_state)
            .await;
        Ok(FrameControl::Continue)
    }
}

/// Apply graphical cadence, host-clock correction, and authoritative hash
/// publication after presentation and PostInitialize complete.
async fn pace_interactive_frame(
    runtime: &mut super::runtime::TimelineRuntime,
    host: &mut Host,
    manager: &mut robin_engine::engine_manager::EngineManager,
    frame: &MissionFrame,
    args: &crate::main_entry::CliArgs,
) {
    // ── Frame timing (25 fps) ──
    // `--fast-forward` CLI flag skips the pacing sleep entirely so
    // the loop runs at full host speed (tests / profiling).  The
    // in-game fast-forward engine flag uses a 1 ms floor instead so
    // other host timers don't starve.
    let frame_end_ms = crate::window::process_uptime_ms();
    let elapsed = frame_end_ms.saturating_sub(frame.started_at_ms);
    let target = if args.fast_forward || args.headless {
        0
    } else if manager.engine.is_fast_forward() {
        1
    } else if host.slow_motion {
        // While SlowMotion is on (and neither console nor engine
        // fast-forward are active), each frame waits 40 * 10 ms.
        engine_api::FRAME_TIME_MS * 10
    } else {
        engine_api::FRAME_TIME_MS
    };
    let normal_sleep_ms = target.saturating_sub(elapsed);
    let host_deadline_ms = if host.net.is_some()
        && host.local_seat != engine_player_command::PlayerId::HOST
        && !args.fast_forward
        && !args.headless
    {
        host_scheduled_frame_deadline_ms(runtime.mp_host_frame_schedule, manager.sim_frame)
    } else {
        None
    };
    let outcome = runtime.plan_frame_outcome(
        frame_end_ms,
        FramePacing {
            fast_forward_requested: args.fast_forward,
            headless: args.headless,
            engine_fast_forward: manager.engine.is_fast_forward(),
            slow_motion: host.slow_motion,
            host_deadline_ms,
        },
        None,
    );
    let FrameOutcome::Continue {
        sleep_ms: remaining_sleep_ms,
    } = outcome
    else {
        unreachable!("native frame pacing cannot request mission exit")
    };
    if host_deadline_ms.is_some() {
        let correction_ms = i64::from(remaining_sleep_ms) - i64::from(normal_sleep_ms);
        if correction_ms != 0
            && frame_end_ms.saturating_sub(runtime.last_mp_sleep_correction_log_ms) >= 1000
        {
            runtime.last_mp_sleep_correction_log_ms = frame_end_ms;
            tracing::info!(
                scheduled_frame = runtime.mp_host_frame_schedule.map(|(frame, _)| frame),
                local_frame = manager.sim_frame,
                normal_sleep_ms,
                adjusted_sleep_ms = remaining_sleep_ms,
                correction_ms,
                "multiplayer: adjusted frame sleep to host frame schedule"
            );
        }
    }
    if let Some((hash_frame, hash)) = runtime.pending_mp_state_hash
        && let Some(net) = host.net.as_ref()
        && host.local_seat == engine_player_command::PlayerId::HOST
    {
        net.publish_frame(manager.sim_frame);
        tracing::info!(
            hash_frame,
            clock_frame = manager.sim_frame,
            elapsed_ms = elapsed,
            target_ms = target,
            remaining_sleep_ms,
            "multiplayer: host sending state hash timing sample"
        );
        net.send_state_hash(hash_frame, hash, manager.sim_frame, remaining_sleep_ms);
    }
    if remaining_sleep_ms > 0 {
        crate::window::sleep_ms(remaining_sleep_ms as u64).await;
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameControl, MissionExit};
    use crate::game_operation::GameCode;

    #[test]
    fn frame_control_keeps_restart_and_exit_distinct() {
        let controls = [
            FrameControl::Continue,
            FrameControl::RestartIteration,
            FrameControl::Exit(MissionExit::new(GameCode::LevelLoad)),
        ];

        let encoded = serde_json::to_string(&controls).expect("serialize frame controls");
        let decoded: Vec<FrameControl> =
            serde_json::from_str(&encoded).expect("deserialize frame controls");

        assert_eq!(decoded, controls);
    }
}
