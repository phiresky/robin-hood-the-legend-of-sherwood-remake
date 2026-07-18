//! Interactive mission flow ownership and orchestration.
//!
//! Session services are borrowed for one run and never stored in the loaded
//! mission. This extraction is intentionally mechanical so later focused
//! phase methods cannot disturb the established frame ordering.

use super::frame_prepare::{
    FramePreparation, FramePresentationState, InteractiveFramePreparation, PreparedFrame,
};
use super::*;

/// Application/session services borrowed by a loaded interactive mission.
///
/// These references are process resources and deliberately do not implement
/// serde. They remain outside deterministic mission ownership.
pub(super) struct MissionServices<'a> {
    pub(super) window: &'a mut GameWindow,
    pub(super) callbacks: &'a mut RustCallbacks,
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

/// Existing mission exit decision propagated to the session wrapper.
///
/// This is deliberately only control flow. The outer campaign lease finalizes
/// every exit after the frame phase which selected it has completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct MissionExit {
    code: GameCode,
}

impl MissionExit {
    pub(super) const fn new(code: GameCode) -> Self {
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
        if let Some(code) = self.capture_requested_screenshot_if_ready(services).await? {
            return Ok(code);
        }
        loop {
            let control = self.run_frame(services).await?;
            if let Some(code) = self.capture_requested_screenshot_if_ready(services).await? {
                return Ok(code);
            }
            match control {
                FrameControl::Continue | FrameControl::RestartIteration => {}
                FrameControl::Exit(exit) => {
                    if let Some(output) = services.args.mission_start_map_output.as_deref() {
                        return Err(format!(
                            "mission exited at simulation frame {} before screenshot frame {} could be written to {}",
                            self.runtime.world.manager.sim_frame,
                            services.args.mission_start_map_frame,
                            output.display()
                        ));
                    }
                    return Ok(exit.into_game_code());
                }
            }
        }
    }

    /// Fulfil the example's file-backed screenshot through the same request
    /// and render implementation as the HTTP screenshot endpoint.
    async fn capture_requested_screenshot_if_ready(
        &mut self,
        services: &mut MissionServices<'_>,
    ) -> Result<Option<GameCode>, String> {
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

        if manager.sim_frame < args.mission_start_map_frame {
            return Ok(None);
        }

        // Frame zero is the pristine mission after Initialize and camera setup,
        // before the first simulation hourglass or PostInitialize call. Positive
        // targets arrive here after that many complete normal mission frames.
        // This matches the original startup boundary in
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
                let screenshot = crate::http_server::ScreenshotRequest {
                    frame: Some(args.mission_start_map_frame),
                    hide_ui: true,
                    full_map: true,
                    ..Default::default()
                };
                capture_screenshot_to_path(
                    &manager.engine,
                    &display_snapshot,
                    host,
                    &assets,
                    &dev,
                    &mut render_ctx,
                    &screenshot,
                    output_path,
                )
            };

            capture_result.map_err(|err| {
                format!(
                    "failed to render mission-start map to {}: {err}",
                    output_path.display()
                )
            })?;
            tracing::info!(
                frame = manager.sim_frame,
                "Mission map screenshot → {}",
                output_path.display()
            );
            return Ok(Some(GameCode::Quit));
        }

        Ok(None)
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
        // File-backed map exports need normal simulation/PostInitialize frames,
        // not intermediate window presentation. Their requested full-map
        // screenshot is rendered once immediately after the target frame.
        let warming_up_map_export = args.mission_start_map_output.is_some()
            && manager.sim_frame <= args.mission_start_map_frame;
        let draw_result = if host.skip_render || modal_rendered_this_frame || warming_up_map_export
        {
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
                            manager
                                .engine
                                .campaign()
                                .expect("interactive mission time requires engine campaign"),
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
                manager.sim_frame,
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
        let prepared = match InteractiveFramePreparation::new(self, services)
            .run()
            .await?
        {
            FramePreparation::Ready(prepared) => prepared,
            FramePreparation::Control(control) => return Ok(control),
        };
        self.simulate_interactive_frame(services, prepared).await
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
        let profiles = services.profiles;
        let args = services.args;
        // File-backed screenshot runs have no player to dismiss a dialogue
        // which appears before their requested frame. Use the established
        // headless auto-dismiss path while retaining normal graphical ticks.
        let auto_dismiss_modals = args.mission_start_map_output.is_some();
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
            let step_frame = manager.sim_frame;
            let buffered_cmds = if step_frame < runtime.rewind_buffer.next_record_frame() {
                runtime
                    .rewind_buffer
                    .commands_for(step_frame)
                    .map(<[PlayerInput]>::to_vec)
            } else {
                Some(Vec::new())
            };
            if let Some(buffered_cmds) = buffered_cmds {
                let reusing_recorded_frame = step_frame < runtime.rewind_buffer.next_record_frame();
                runtime
                    .rewind_buffer
                    .begin_frame(step_frame, &manager.engine, &assets);

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
                } else if reusing_recorded_frame {
                    step_frame_cmds = buffered_cmds;
                }
                manager.engine.apply_commands(
                    &mut host.engine_display,
                    &mut host.input,
                    &assets,
                    &step_frame_cmds,
                );

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
                if !reusing_recorded_frame {
                    runtime.rewind_buffer.end_frame(step_frame_cmds);
                }
                manager.sim_frame += 1;
                // Stepping bypasses the checker's begin_frame/end_frame
                // pairing, so its ring buffer is now stale relative to the
                // advanced engine.  Clear it — the checker resumes
                // populating on the next normal frame.
                if let Some(ref mut checker) = runtime.rollback_checker {
                    checker.reset();
                }
            } else {
                tracing::warn!(
                    frame = step_frame,
                    oldest_command_frame = runtime.rewind_buffer.oldest_cmd_frame(),
                    "step-forward: frame lies inside recorded history but its commands are missing"
                );
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

        if auto_dismiss_modals {
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

        if !modal_rendered_this_frame && auto_dismiss_modals {
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

        if !modal_rendered_this_frame && auto_dismiss_modals {
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
                if auto_dismiss_modals {
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
                &PlayerCommand::ApplyQuitMissionUpdates {
                    exit_code,
                    difficulty: game.global_options.sim_config().difficulty,
                },
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
    let target = if args.fast_forward {
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
    {
        host_scheduled_frame_deadline_ms(runtime.mp_host_frame_schedule, manager.sim_frame)
    } else {
        None
    };
    let outcome = runtime.plan_frame_outcome(
        frame_end_ms,
        FramePacing {
            fast_forward_requested: args.fast_forward,
            headless: false,
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
