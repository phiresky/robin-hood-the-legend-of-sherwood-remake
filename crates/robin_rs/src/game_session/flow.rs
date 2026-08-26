//! Interactive mission flow ownership and orchestration.
//!
//! Session services are borrowed for one run and never stored in the loaded
//! mission. This extraction is intentionally mechanical so later focused
//! phase methods cannot disturb the established frame ordering.

use super::frame_prepare::{
    FramePreparation, FramePresentationState, InteractiveFramePreparation, PreparedFrame,
};
use super::runtime::FrameContractStage;
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

impl FrameControl {
    pub(super) const fn exit(code: GameCode) -> Self {
        Self::Exit(MissionExit::new(code))
    }
}
/// Existing mission exit decision propagated to the session wrapper.
///
/// This is deliberately only control flow. Consuming mission finalization
/// returns the engine-owned campaign after the selecting phase completes.
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
                            self.runtime.timeline.frame_number(),
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
        let timeline_frame = runtime.timeline.frame_number();
        let MissionRuntime { world, .. } = runtime;
        let MissionPresentationPhase {
            host,
            game,
            manager,
            assets,
            dev,
        } = world.presentation_phase();
        let InteractiveFrontend {
            input,
            resources,
            ui,
            hud,
            presentation,
            ..
        } = frontend;

        if timeline_frame < args.mission_start_map_frame {
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
                let mut display = std::mem::take(&mut host.engine_display);
                crate::sim_timeline::run_engine_frame_core(
                    host,
                    &mut display,
                    &assets,
                    &mut manager.engine,
                    dev,
                    engine_api::SimulationFrameInput::from_player_inputs(vec![
                        engine_player_command::PlayerCommand::RevealAllBlips.into(),
                    ])
                    .with_hourglass(false),
                );
                host.engine_display = display;
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

            // A full-map export is not an interactive screenshot. Keep the
            // cursor out of its top-left map pixel. Viewport captures retain
            // the ordinary cursor/HUD composition.
            if !args.mission_start_viewport_capture {
                host.input.mouse_opacity = 0;
            }
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
                    hide_ui: !args.mission_start_viewport_capture,
                    full_map: !args.mission_start_viewport_capture,
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
                frame = timeline_frame,
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
        InteractiveFrameFinish {
            mission: self,
            services,
            state,
        }
        .run()
        .await;
    }
}

/// Short-lived owner of the post-modal graphical tail. It keeps application
/// services outside mission state while giving recorder, audio, presentation,
/// PostInitialize, and pacing one explicit orchestration boundary.
struct InteractiveFrameFinish<'mission, 'services, 'app> {
    mission: &'mission mut InteractiveMission,
    services: &'services mut MissionServices<'app>,
    state: FramePresentationState,
}

impl InteractiveFrameFinish<'_, '_, '_> {
    async fn run(self) {
        let Self {
            mission,
            services,
            state,
        } = self;
        let callbacks = &mut *services.callbacks;
        let args = services.args;
        let FramePresentationState {
            mut frame,
            rewind_active,
            consumed_buffered,
            shift_held,
            modal_rendered: modal_rendered_this_frame,
            history_commit_pending,
        } = state;
        let InteractiveMission { runtime, frontend } = mission;
        let MissionRuntime {
            world,
            timeline: runtime,
            ..
        } = runtime;
        let profiling = super::frame_perf::enabled();
        let phase_start = super::frame_perf::start(profiling);
        finish_interactive_audio(runtime, world, frontend, callbacks);
        super::frame_perf::record(super::frame_perf::Phase::Audio, phase_start);

        let phase_start = super::frame_perf::start(profiling);
        let MissionPresentationPhase {
            host,
            game,
            manager,
            assets,
            dev,
        } = world.presentation_phase();
        let input = &mut frontend.input;
        let resources = &mut frontend.resources;
        let ui = &mut frontend.ui;
        let hud = &mut frontend.hud;
        let presentation = &mut frontend.presentation;
        runtime.begin_presentation();
        runtime.trace(FrameContractStage::Presentation);

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
            && runtime.frame_number() <= args.mission_start_map_frame;
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
                &mut frame.post_external_actions,
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
                            manager.engine.campaign(),
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
                runtime.frame_number(),
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
            post_render_engine_cleanup(&mut frame, host);
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
        super::frame_perf::record(super::frame_perf::Phase::Render, phase_start);

        // Original RHgame.cpp ordering is Refresh (including Draw/Flip),
        // then RHSound::Hourglass, then the one-shot engine
        // PostInitialize call.  Rust's sound/render consumers are split in
        // the opposite host order above, so dispatch only after both have
        // completed.  Script mutations and emitted sound/UI effects first
        // become observable on the next frame, matching the original.
        let phase_start = super::frame_perf::start(profiling);
        run_interactive_post_initialize(runtime, host, manager, assets, dev, &mut frame);
        super::frame_perf::record(super::frame_perf::Phase::PostInitialize, phase_start);

        if history_commit_pending {
            runtime.commit_simulation_history(
                host,
                manager,
                &frame,
                FrameCommitPolicy {
                    store_rewind_commands: !consumed_buffered,
                },
            );
        }

        // Finalize only after the authoritative history transition. Full-frame
        // replay records can now persist the explicit cursor before/after
        // pair instead of inferring it from hourglass admission.
        let phase_start = super::frame_perf::start(profiling);
        finalize_interactive_recording(runtime, &mut frame);
        super::frame_perf::record(super::frame_perf::Phase::Recording, phase_start);

        let phase_start = super::frame_perf::start(profiling);
        pace_interactive_frame(runtime, host, manager, &frame, args).await;
        super::frame_perf::record(super::frame_perf::Phase::Pacing, phase_start);
    }
}

impl InteractiveMission {
    /// Run one interactive frame using short-lived borrows of the mission
    /// components and application services.
    async fn run_frame(
        &mut self,
        services: &mut MissionServices<'_>,
    ) -> Result<FrameControl, String> {
        let profiling = super::frame_perf::enabled();
        let total_start = super::frame_perf::start(profiling);
        let phase_start = super::frame_perf::start(profiling);
        let prepared = match InteractiveFramePreparation::new(self, services)
            .run()
            .await?
        {
            FramePreparation::Ready(prepared) => prepared,
            FramePreparation::Control(control) => {
                super::frame_perf::record(super::frame_perf::Phase::Prepare, phase_start);
                super::frame_perf::record(super::frame_perf::Phase::Total, total_start);
                return Ok(control);
            }
        };
        super::frame_perf::record(super::frame_perf::Phase::Prepare, phase_start);
        let PreparedFrame {
            frame,
            rewind_active,
            paused,
            consumed_buffered,
            shift_held,
            modal_rendered,
            step_forward_pressed,
            step_back_pressed,
        } = prepared;
        let phase_start = super::frame_perf::start(profiling);
        let outcome = InteractiveFrameSimulation::new(
            frame,
            FrameSimulationFlags {
                rewind_active,
                paused,
                consumed_buffered,
                shift_held,
                modal_rendered,
                step_forward_pressed,
                step_back_pressed,
            },
        )
        .run(self, services)
        .await?;
        super::frame_perf::record(super::frame_perf::Phase::Simulation, phase_start);
        let control = match outcome {
            FrameSimulationOutcome::Control(control) => control,
            FrameSimulationOutcome::Present(handoff) => {
                self.finish_interactive_frame(
                    services,
                    FramePresentationState {
                        frame: handoff.frame,
                        rewind_active: handoff.rewind_active,
                        consumed_buffered: handoff.consumed_buffered,
                        shift_held: handoff.shift_held,
                        modal_rendered: handoff.modal_rendered,
                        history_commit_pending: handoff.history_commit_pending,
                    },
                )
                .await;
                FrameControl::Continue
            }
        };
        super::frame_perf::record(super::frame_perf::Phase::Total, total_start);
        Ok(control)
    }
}

/// Close the per-frame recorder token after all modal drains have had a chance
/// to append their acknowledgements.
fn finalize_interactive_recording(
    runtime: &mut super::runtime::TimelineRuntime,
    frame: &mut MissionFrame,
) {
    runtime.finish_recording(frame);
    if !frame.replay_modal_dismissals.is_empty() {
        let unused = frame.replay_modal_dismissals.len();
        if frame.replay_modal_dismissals.is_strict_replay() {
            panic!("replay desync: {unused} recorded modal dismissal(s) were unused this frame");
        }
        tracing::warn!("Replay: {unused} recorded ModalDismiss command(s) unused this frame");
    }
}

/// Apply queued process effects, then cross the original sound-hourglass
/// boundary. Keeping these as one focused method makes their required order
/// explicit while retaining separate execution trace checkpoints.
fn finish_interactive_audio(
    runtime: &mut super::runtime::TimelineRuntime,
    world: &mut MissionWorld,
    frontend: &mut InteractiveFrontend,
    callbacks: &mut RustCallbacks,
) {
    let MissionAudioPhase { host, manager } = world.audio_phase();
    execute_app_effects(
        &mut callbacks.app_effects,
        &mut host.audio.sound,
        &mut frontend.input.threaded,
        frontend
            .audio
            .backend
            .as_mut()
            .map(|backend| backend as &mut dyn crate::sound::AudioBackend),
    );
    runtime.trace(FrameContractStage::AppEffects);
    if let Some(boundary) = frontend.audio.tick(manager, host) {
        runtime.queue_sound_boundary(boundary);
    }
    runtime.trace(FrameContractStage::Audio);
}

/// Cross the one-shot post-refresh script boundary and update the initial
/// authoritative multiplayer snapshot from that completed state.
fn run_interactive_post_initialize(
    runtime: &mut super::runtime::TimelineRuntime,
    host: &mut Host,
    manager: &mut robin_engine::engine_manager::EngineManager,
    assets: &std::sync::Arc<robin_engine::engine::LevelAssets>,
    dev: &mut robin_engine::engine::DevState,
    frame: &mut MissionFrame,
) {
    let mut display = std::mem::take(&mut host.engine_display);
    let post_initialized = runtime.cross_post_initialize(|| {
        crate::sim_timeline::run_post_initialize_stage_with_actions(
            host,
            &mut display,
            assets,
            &mut manager.engine,
            dev,
            frame.unapplied_post_external_actions(),
            &frame.post_commands.commands,
            frame.run_post_initialize,
        )
    });
    frame.run_post_initialize = post_initialized;
    host.engine_display = display;
    if post_initialized
        && let Some(net) = host.transport.net.as_ref()
        && host.transport.local_seat == engine_player_command::PlayerId::HOST
    {
        net.set_initial_snapshot(runtime.frame_number(), &manager.engine);
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
    runtime.trace(FrameContractStage::Pacing);
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
    let host_deadline_ms = if host.transport.net.is_some()
        && host.transport.local_seat != engine_player_command::PlayerId::HOST
        && !args.fast_forward
    {
        host_scheduled_frame_deadline_ms(runtime.mp_host_frame_schedule, runtime.frame_number())
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
                local_frame = runtime.frame_number(),
                normal_sleep_ms,
                adjusted_sleep_ms = remaining_sleep_ms,
                correction_ms,
                "multiplayer: adjusted frame sleep to host frame schedule"
            );
        }
    }
    if let Some((hash_frame, hash)) = runtime.pending_mp_state_hash
        && let Some(net) = host.transport.net.as_ref()
        && host.transport.local_seat == engine_player_command::PlayerId::HOST
    {
        net.publish_frame(runtime.frame_number());
        tracing::info!(
            hash_frame,
            clock_frame = runtime.frame_number(),
            elapsed_ms = elapsed,
            target_ms = target,
            remaining_sleep_ms,
            "multiplayer: host sending state hash timing sample"
        );
        net.send_state_hash(hash_frame, hash, runtime.frame_number(), remaining_sleep_ms);
    }
    if remaining_sleep_ms > 0 {
        crate::window::sleep_ms(remaining_sleep_ms as u64).await;
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameControl, MissionExit};
    use robin_engine::game_operation::GameCode;

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
