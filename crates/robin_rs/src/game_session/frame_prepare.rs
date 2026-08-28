//! Interactive frame input and operation preparation.
//!
//! This phase owns the exclusive mission and application-service borrows until
//! it has finalized the deterministic command stream. No presentation borrow
//! escapes the phase or crosses into simulation.

use super::event_hud::{
    CollectedFrameInput, EventHudContext, EventHudOutcome, collect_event_and_hud_input,
};
use super::flow::{FrameControl, MissionExit, MissionServices};
use super::interactive::MissionInput;
use super::live_gameplay::{LiveGameplayContext, LiveGameplayInput, drive_live_gameplay_input};
use super::runtime::FrameContractStage;
use super::*;

/// Values produced by graphical network ingress at the frame boundary.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct FrameStart {
    pub(super) frame: MissionFrame,
    pub(super) mp_clock_pause: bool,
}

/// State handed from modal/recorder bookkeeping to presentation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct FramePresentationState {
    pub(super) frame: MissionFrame,
    pub(super) rewind_active: bool,
    pub(super) consumed_buffered: bool,
    pub(super) shift_held: bool,
    pub(super) modal_rendered: bool,
    pub(super) history_commit_pending: bool,
}

/// Deterministic and presentation flags carried across the tick boundary.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct PreparedFrame {
    pub(super) frame: MissionFrame,
    pub(super) rewind_active: bool,
    pub(super) paused: bool,
    pub(super) consumed_buffered: bool,
    pub(super) shift_held: bool,
    pub(super) modal_rendered: bool,
    pub(super) step_forward_pressed: bool,
    pub(super) step_back_pressed: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) enum FramePreparation {
    Ready(PreparedFrame),
    Control(FrameControl),
}

/// Apply multiplayer ingress and capture the deterministic pre-command
/// snapshot before any interactive input mutates the engine.
fn begin_interactive_frame(mission: &mut InteractiveMission) -> FrameStart {
    let InteractiveMission { runtime, frontend } = mission;
    let MissionRuntime {
        world,
        timeline: runtime,
        control: _,
    } = runtime;
    let MissionIngress {
        host,
        manager,
        assets,
    } = world.ingress();
    let hud = &mut frontend.hud;
    let presentation = &mut frontend.presentation;
    let mut frame = MissionFrame::new(crate::window::process_uptime_ms());
    runtime.begin_execution_trace(FrameContractStage::NetworkIngress);

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
    let net_drain = drain_mission_network(
        runtime,
        host,
        manager,
        assets.as_ref(),
        true,
        current_epoch_ms(),
    );
    let mp_clock_pause = net_drain.pause_simulation;
    let net_inputs = net_drain.inputs;

    // Enter the shared runtime's input phase after network state correction
    // but before any command for this frame. Current-frame network inputs are
    // commands *to* this pre-tick state and must be applied only after it is
    // captured; otherwise replay starts from a post-command checkpoint and
    // applies the journaled commands twice. The recorder hash samples this
    // same boundary so recording and playback remain in lockstep.
    runtime.open_frame(&mut frame, &manager.engine, assets);
    frame.commands.commands.extend(net_inputs);

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
    // and titbit Z flush. This is the interactive-only driver; true
    // headless construction never reaches this presentation stage.
    host.draw_order = manager.engine.compute_display_order();

    FrameStart {
        frame,
        mp_clock_pause,
    }
}

/// Apply process-side input and banner effects produced by save/load I/O.
/// These intentionally remain after operation processing and before replay or
/// multiplayer command injection.
fn apply_post_save_ui_state(
    callbacks: &mut RustCallbacks,
    game: &mut crate::game::Game,
    input: &mut MissionInput,
) {
    if std::mem::take(&mut callbacks.pending_reset_input) {
        input.reset_after_engine_request();
    }
    if let Some(kind) = callbacks.pending_save_banner.take() {
        let text = match kind {
            SaveBannerKind::Saved => "Game saved.",
            SaveBannerKind::Loaded => "Game loaded.",
        };
        // TODO(refactor): replace these literals with MT_MSG_GAME_SAVED and
        // MT_MSG_GAME_LOADED once the localized text ownership is explicit.
        game.display_message(text.to_string(), 100);
    }
}

/// Apply host-only camera controls. These deliberately remain available while
/// deterministic replay or rewind suppresses simulation commands.
fn apply_host_view_input(
    host: &mut Host,
    engine: &Engine,
    mouse_position: engine_coordinates::ScreenPoint,
    keyboard_actions: &[GameAction],
    mouse_actions: &[GameAction],
    events: &[GameEvent],
    view_suppressed: bool,
    pan_suppressed: bool,
) {
    if !view_suppressed {
        for action in keyboard_actions.iter().chain(mouse_actions) {
            let scroll_suppressed_by_minimap = matches!(
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
                GameAction::ScrollUp => apply_local_viewport_scroll(host, ScrollDirection::Up),
                GameAction::ScrollDown => apply_local_viewport_scroll(host, ScrollDirection::Down),
                GameAction::ScrollLeft => apply_local_viewport_scroll(host, ScrollDirection::Left),
                GameAction::ScrollRight => {
                    apply_local_viewport_scroll(host, ScrollDirection::Right)
                }
                GameAction::ZoomIn => host.viewport.zoom_by(2.0, Some(mouse_position)),
                GameAction::ZoomOut => host.viewport.zoom_by(0.5, Some(mouse_position)),
                _ => {}
            }
        }
    }

    if pan_suppressed || engine.user_locked() {
        return;
    }
    for event in events {
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

/// Drain packets which arrived while the frame was processing local input.
/// This is the final network mutation boundary before state hashing and tick.
fn drain_pre_tick_network(
    runtime: &mut super::runtime::TimelineRuntime,
    host: &mut Host,
    manager: &mut robin_engine::engine_manager::EngineManager,
    assets: &robin_engine::engine::LevelAssets,
    frame: &mut MissionFrame,
    mp_clock_pause: &mut bool,
    rewind_active: bool,
) {
    if host.transport.net.is_none() || rewind_active {
        return;
    }

    runtime.trace(FrameContractStage::SecondNetworkDrain);
    let drain = drain_mission_network(runtime, host, manager, assets, false, current_epoch_ms());
    *mp_clock_pause |= drain.pause_simulation;
    frame.commands.commands.extend(drain.inputs);
}

/// Publish or verify the periodic multiplayer state hash after the second
/// network drain has made this frame's command set final.
pub(super) fn process_pre_tick_state_hash(
    runtime: &mut super::runtime::TimelineRuntime,
    host: &Host,
    manager: &robin_engine::engine_manager::EngineManager,
) {
    if host.transport.net.is_none()
        || !runtime
            .frame_number()
            .is_multiple_of(crate::multiplayer::STATE_HASH_INTERVAL)
    {
        return;
    }
    if host.transport.local_seat == engine_player_command::PlayerId::HOST
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
    } else if let Some(&host_hash) = runtime.peer_hashes.get(&runtime.frame_number()) {
        let local_hash = robin_engine::replay::state_hash(&manager.engine);
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
                frame = runtime.frame_number(),
                local = format!("{local_hash:016x}"),
                host = format!("{host_hash:016x}"),
                host_schedule_frame = runtime.mp_host_frame_schedule.map(|(frame, _)| frame),
                pending_input_frames = runtime.pending_inputs.len(),
                last_rollback_path,
                last_rollback_earliest,
                last_rollback_target,
                last_rollback_replayed,
                last_rollback_total_us,
                "multiplayer DESYNC: local engine hash differs from host's"
            );
        } else {
            tracing::debug!(frame = runtime.frame_number(), "multiplayer hash OK");
        }
    }
    let current_frame = runtime.frame_number();
    runtime.peer_hashes.retain(|&f, _| f > current_frame);
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

/// Drop pending load-type requests while a replay is playing back.
///
/// The replay stream owns the deterministic state during playback: recorded
/// loads arrive as load-back records applied at the frame boundary, so
/// re-running the request against on-disk saves (which may differ or be
/// missing on this machine) would corrupt or abort playback.  Save-type
/// requests still flush — writing a save during playback is harmless.
fn suppress_load_requests_during_playback(
    runtime: &super::runtime::TimelineRuntime,
    callbacks: &mut RustCallbacks,
) {
    if runtime.replay_player.is_some()
        && callbacks
            .pending
            .as_ref()
            .is_some_and(|request| !request.writes_save_payload())
    {
        tracing::info!(
            "replay playback: dropping live load request; recorded load-backs own the timeline"
        );
        callbacks.pending = None;
    }
}

/// Admit replay commands and reconcile rewind history after all live/network
/// commands for the frame are known.
fn prepare_pre_tick_timeline(
    runtime: &mut super::runtime::TimelineRuntime,
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
    if runtime.replay_player.is_some() && !replay_cursor_paused {
        // Recorded save markers pin the boundary state and load-back
        // records swap a pinned state in, before this frame's commands.
        runtime.apply_playback_timeline_events(host, game, manager, assets)?;
    }
    if let Some(ref mut player) = runtime.replay_player
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
            frame.inject_replay_input(player);
            frame.assert_replay_timeline_before(runtime.current_frame());
        }
    }

    let mut consumed_buffered = false;
    let current_frame = runtime.frame_number();
    if !rewind_active && !paused && current_frame < runtime.rewind_buffer.next_record_frame() {
        let Some(recorded) = runtime.rewind_buffer.frame_for(current_frame).cloned() else {
            return Err(format!(
                "cannot replay frame {}: rewind command history starts at frame {}",
                current_frame,
                runtime.rewind_buffer.oldest_cmd_frame()
            ));
        };
        if runtime.replay_player.is_some() && frame.external_actions.is_empty() {
            frame.adopt_authoritative_input(recorded);
            consumed_buffered = true;
            tracing::trace!("Replay reused rewind-buffer frame {}", current_frame);
        } else if frame.commands.commands.is_empty() && frame.external_actions.is_empty() {
            frame.adopt_authoritative_input(recorded);
            consumed_buffered = true;
            tracing::trace!("Auto-replay -> frame {}", current_frame);
        } else {
            tracing::trace!(
                "Auto-replay interrupted by live input; truncating buffer at {}",
                current_frame
            );
            runtime.rewind_buffer.truncate_future(current_frame);
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
    runtime: &super::runtime::TimelineRuntime,
    host: &mut Host,
    manager: &mut robin_engine::engine_manager::EngineManager,
    assets: &robin_engine::engine::LevelAssets,
    input: &MissionInput,
    frame: &mut MissionFrame,
    rewind_active: bool,
    paused: bool,
) {
    if runtime.replay_player.is_some() || rewind_active || paused {
        return;
    }
    let Some(mouse_map) = host.viewport.screen_to_map(input.threaded.position()) else {
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
        dispatch_local_command(host, &mut manager.engine, &mut frame.commands, assets, &cmd);
    }

    let bow_armed = manager
        .engine
        .selected_action_for_seat(host.transport.local_seat)
        == engine_profiles::Action::Bow;
    if host.time_no_mouse_move != 0 || bow_armed {
        let cmd = PlayerCommand::PerformOrientation { mouse_map };
        dispatch_local_command(host, &mut manager.engine, &mut frame.commands, assets, &cmd);
    }
}

/// Collect input, drive operation/save flows, and finalize the pre-tick
/// command stream.
pub(super) struct InteractiveFramePreparation<'mission, 'services, 'app> {
    mission: &'mission mut InteractiveMission,
    services: &'services mut MissionServices<'app>,
    state: Option<PreparationPhaseState>,
}

struct PreparationPhaseState {
    frame: MissionFrame,
    mp_clock_pause: bool,
    pause_closed_this_frame: bool,
    rewind_active: bool,
    shift_held: bool,
    step_forward_pressed: bool,
    step_back_pressed: bool,
    modal_rendered_this_frame: bool,
}

impl<'mission, 'services, 'app> InteractiveFramePreparation<'mission, 'services, 'app> {
    pub(super) fn new(
        mission: &'mission mut InteractiveMission,
        services: &'services mut MissionServices<'app>,
    ) -> Self {
        Self {
            mission,
            services,
            state: None,
        }
    }

    /// Collect input, drive operation/save flows, and finalize the pre-tick
    /// command stream.
    pub(super) async fn run(mut self) -> Result<FramePreparation, String> {
        if let Some(control) = self.collect_input_and_menus().await? {
            return Ok(FramePreparation::Control(control));
        }
        if let Some(control) = self.process_operation_and_save().await? {
            return Ok(FramePreparation::Control(control));
        }
        self.finalize_pre_tick()
    }

    async fn collect_input_and_menus(&mut self) -> Result<Option<FrameControl>, String> {
        let mission = &mut *self.mission;
        let services = &mut *self.services;
        let window = &mut *services.window;
        let callbacks = &mut *services.callbacks;
        let profiles = services.profiles;
        let FrameStart {
            mut frame,
            mp_clock_pause,
        } = begin_interactive_frame(mission);
        let modal_rendered_this_frame = false;
        // Preserve the existing statement order while migrating ownership. These
        // are disjoint borrows from the two mission-lifetime roots, not secondary
        // state copies.
        let InteractiveMission { runtime, frontend } = mission;
        let MissionRuntime {
            world,
            timeline: runtime,
            control,
        } = runtime;
        let MissionInputPhase {
            host,
            game,
            manager,
            assets,
            dev,
        } = world.input_phase();
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
            &mut frame,
            assets,
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
                runtime.trace(FrameContractStage::EarlyRestart);
                return Ok(Some(FrameControl::RestartIteration));
            }
            HandlerAction::Exit(code) => {
                runtime.trace(FrameContractStage::Exit);
                return Ok(Some(FrameControl::Exit(MissionExit::new(code))));
            }
            HandlerAction::Proceed => {}
        }

        let collected = collect_event_and_hud_input(EventHudContext {
            host,
            manager,
            game,
            assets: assets.as_ref(),
            dev,
            callbacks,
            window,
            presentation,
            resources,
            input,
            ui,
            hud,
            runtime,
            frame: &mut frame,
            manual_pause,
            step_forward_repeat_at_ms,
            step_back_repeat_at_ms,
        })
        .await;
        let CollectedFrameInput {
            events,
            keyboard_actions: kb_actions,
            mouse_actions,
            modifiers,
            minimap_toggle_pressed,
            mut pause_closed_this_frame,
            rewind_active,
            step_forward_pressed,
            step_back_pressed,
        } = match collected {
            EventHudOutcome::Ready(input) => input,
            EventHudOutcome::Control(HandlerAction::Continue) => {
                runtime.trace(FrameContractStage::EarlyRestart);
                return Ok(Some(FrameControl::RestartIteration));
            }
            EventHudOutcome::Control(HandlerAction::Exit(code)) => {
                runtime.trace(FrameContractStage::Exit);
                return Ok(Some(FrameControl::Exit(MissionExit::new(code))));
            }
            EventHudOutcome::Control(HandlerAction::Proceed) => {
                unreachable!("event/HUD collection must return data when it proceeds")
            }
        };
        let shift_held = modifiers.shift;

        // ── View-only input (scroll / zoom): always allowed ──
        // These mutate host-side viewport state only — never the sim —
        // so they're safe during replay playback and rewind, when the
        // user wants to pan/zoom around the paused world.  Suppressed
        // only when the console or the pause menu has focus.
        apply_host_view_input(
            host,
            &manager.engine,
            input.threaded.position(),
            &kb_actions,
            &mouse_actions,
            &events,
            ui.console_overlay.is_visible() || ui.pause_menu.is_some() || pause_closed_this_frame,
            ui.pause_menu.is_some() || pause_closed_this_frame,
        );

        // ── Skip all sim-affecting input during replay / rewind ──
        // Recorded commands are injected at the tick boundary instead
        // (replay), or suppressed entirely (rewind — live input
        // shouldn't perturb a state reconstructed from the past).
        if runtime.replay_player.is_none() && !rewind_active {
            match drive_live_gameplay_input(
                LiveGameplayContext {
                    host,
                    manager,
                    game,
                    assets: assets.as_ref(),
                    dev,
                    callbacks,
                    window,
                    presentation,
                    resources,
                    audio,
                    input,
                    ui,
                    frame: &mut frame,
                },
                LiveGameplayInput {
                    events: &events,
                    keyboard_actions: &kb_actions,
                    mouse_actions: &mouse_actions,
                    minimap_toggle_pressed,
                    modifiers,
                    pause_closed_this_frame: &mut pause_closed_this_frame,
                },
            )
            .await
            {
                HandlerAction::Continue => {
                    runtime.trace(FrameContractStage::EarlyRestart);
                    return Ok(Some(FrameControl::RestartIteration));
                }
                HandlerAction::Exit(code) => {
                    runtime.trace(FrameContractStage::Exit);
                    return Ok(Some(FrameControl::Exit(MissionExit::new(code))));
                }
                HandlerAction::Proceed => {}
            }
        }
        // ── Cross-mission QuickLoad confirmation task ──
        // Quick-load prompts the
        // player with `MSG_REALLY_LOAD_QUICKSAVE` whenever the quicksave
        // header's mission ID differs from the running mission.  Run
        // the task here, before the save/load drain. It then advances one
        // frame at a time alongside the mission loop.
        if ui.active_ui_task.is_none()
            && let Some(task) = prepare_quickload_cross_mission(
                callbacks,
                &manager.engine,
                profiles,
                &mut *window,
                &mut presentation.renderer,
                &resources.menu,
            )
        {
            ui.active_ui_task = Some(task);
        }

        runtime.trace(FrameContractStage::InputAndMenus);

        self.state = Some(PreparationPhaseState {
            frame,
            mp_clock_pause,
            pause_closed_this_frame,
            rewind_active,
            shift_held,
            step_forward_pressed,
            step_back_pressed,
            modal_rendered_this_frame,
        });
        Ok(None)
    }

    async fn process_operation_and_save(&mut self) -> Result<Option<FrameControl>, String> {
        let services = &mut *self.services;
        let callbacks = &mut *services.callbacks;
        let profiles = services.profiles;
        let PreparationPhaseState {
            mut frame,
            mp_clock_pause,
            pause_closed_this_frame,
            rewind_active,
            shift_held,
            step_forward_pressed,
            step_back_pressed,
            modal_rendered_this_frame,
        } = self.state.take().expect("input phase must complete first");
        let InteractiveMission { runtime, frontend } = &mut *self.mission;
        let MissionRuntime {
            world,
            timeline: runtime,
            ..
        } = runtime;
        let MissionOperationPhase {
            host,
            game,
            manager,
            assets,
            dev,
        } = world.operation_phase();
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
        let exit_code = game.process_operation(manager.engine.campaign(), profiles, callbacks);
        let pending_thumbnail = if callbacks
            .pending
            .as_ref()
            .is_some_and(|request| request.writes_save_payload())
            && !host.skip_render
            && !modal_rendered_this_frame
        {
            pre_render_engine_setup(manager, host, assets.as_ref(), &mut presentation.renderer);
            update_mouse_and_cursor(
                manager,
                host,
                assets,
                dev,
                &mut frame.external_actions,
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
                            manager.engine.campaign(),
                        ),
                },
            );
            capture_save_thumbnail(
                &manager.engine,
                &display_snapshot,
                host,
                assets,
                dev,
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
            let save_load = perform_pending_save_load(
                host,
                game,
                callbacks,
                &mut manager.engine,
                assets.as_ref(),
                profiles,
                pending_thumbnail.clone(),
            );
            if save_load.processed
                && let Some(ref mut checker) = runtime.rollback_checker
            {
                checker.reset();
            }
            if let Some(event) = save_load.event {
                runtime.note_save_load_event(event, &mut frame, &manager.engine, assets.as_ref());
            }
            if callbacks.pending_level_restart {
                callbacks.pending_level_restart = false;
                game.operation.set(GameCode::LevelRestart);
                runtime.trace(FrameContractStage::Exit);
                return Ok(Some(FrameControl::Exit(MissionExit::new(
                    GameCode::LevelRestart,
                ))));
            }
            if let Some(sync) = callbacks.post_load_sync.take() {
                game.apply_post_load_sync(sync.is_continue);
                game.post_load_resolution_resync();
            }
            runtime.trace(FrameContractStage::Exit);
            return Ok(Some(FrameControl::Exit(MissionExit::new(exit_code))));
        }
        suppress_load_requests_during_playback(runtime, callbacks);
        let save_load = perform_pending_save_load(
            host,
            game,
            callbacks,
            &mut manager.engine,
            assets.as_ref(),
            profiles,
            pending_thumbnail,
        );
        if save_load.processed
            && let Some(ref mut checker) = runtime.rollback_checker
        {
            checker.reset();
        }
        if let Some(event) = save_load.event {
            runtime.note_save_load_event(event, &mut frame, &manager.engine, assets.as_ref());
        }

        // A rejected/missing/unappliable Restart payload must leave this
        // mission. Continuing after terminal debriefing reset the operation
        // to LevelInProgress would keep the failed mission alive with mixed
        // lifecycle state. The outer session owns the authoritative restart
        // campaign/RNG/SimConfig checkpoint.
        if callbacks.pending_level_restart {
            callbacks.pending_level_restart = false;
            game.operation.set(GameCode::LevelRestart);
            runtime.trace(FrameContractStage::Exit);
            return Ok(Some(FrameControl::Exit(MissionExit::new(
                GameCode::LevelRestart,
            ))));
        }

        // ── Cross-mission load: bubble up ──
        // `perform_pending_save_load` stashes a `PendingLevelLoad` when the
        // chosen slot targets a different mission than the one running. Force
        // the Game state machine into LevelLoad so `process_operation` exits
        // on the next iteration; the outer session loop will switch missions
        // and re-queue the Load on the fresh engine.
        if callbacks.pending_level_load.is_some() {
            game.operation.set(GameCode::LevelLoad);
            runtime.trace(FrameContractStage::Exit);
            return Ok(Some(FrameControl::Exit(MissionExit::new(
                GameCode::LevelLoad,
            ))));
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

        apply_post_save_ui_state(callbacks, game, input);

        self.state = Some(PreparationPhaseState {
            frame,
            mp_clock_pause,
            pause_closed_this_frame,
            rewind_active,
            shift_held,
            step_forward_pressed,
            step_back_pressed,
            modal_rendered_this_frame,
        });
        Ok(None)
    }

    fn finalize_pre_tick(&mut self) -> Result<FramePreparation, String> {
        let PreparationPhaseState {
            mut frame,
            mut mp_clock_pause,
            pause_closed_this_frame: _,
            rewind_active,
            shift_held,
            step_forward_pressed,
            step_back_pressed,
            modal_rendered_this_frame,
        } = self
            .state
            .take()
            .expect("operation/save phase must complete first");
        let InteractiveMission { runtime, frontend } = &mut *self.mission;
        let MissionRuntime {
            world,
            timeline: runtime,
            control,
        } = runtime;
        let MissionPreTickPhase {
            host,
            game,
            manager,
            assets,
        } = world.pre_tick_phase();
        let manual_pause = &mut control.manual_pause;
        let input = &mut frontend.input;
        let ui = &mut frontend.ui;

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
        drain_pre_tick_network(
            runtime,
            host,
            manager,
            assets.as_ref(),
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
                host.transport.net.is_some(),
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
}

#[cfg(test)]
mod tests {
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
