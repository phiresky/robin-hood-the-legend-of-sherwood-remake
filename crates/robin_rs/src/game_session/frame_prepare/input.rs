//! Input, menu ownership, and first network ingress.

use super::*;
use crate::game_session::event_hud::{
    CollectedFrameInput, EventHudContext, EventHudOutcome, InputModifiers,
    collect_event_and_hud_input,
};
use crate::game_session::flow::MissionExit;
use crate::game_session::interactive::{MissionHud, MissionPresentation};
use crate::game_session::live_gameplay::{
    LiveGameplayContext, LiveGameplayInput, drive_live_gameplay_input,
};
use crate::game_session::runtime::{FrameContractStage, TimelineRuntime};

fn begin_interactive_frame(
    ingress: MissionIngress<'_>,
    runtime: &mut TimelineRuntime,
    hud: &mut MissionHud,
    presentation: &MissionPresentation,
) -> FrameStart {
    let MissionIngress {
        host,
        manager,
        assets,
    } = ingress;
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
    //   the timeline reconciliation owner, drained below alongside the per-25-frame
    //   sampling tick.
    // Publishes the current sim_frame to the server's broadcast
    // pump so peer-input target frames are stamped against a
    // fresh cursor.
    let net_drain = drain_mission_network(runtime, host, manager, assets, true, current_epoch_ms());
    let mp_clock_pause = net_drain.pause_simulation;
    let net_inputs = net_drain.inputs;

    // Enter the shared runtime's input phase after network state correction
    // but before any command for this frame. Current-frame network inputs are
    // commands *to* this pre-tick state and must be applied only after it is
    // captured; otherwise replay starts from a post-command checkpoint and
    // applies the journaled commands twice. The recorder hash samples this
    // same boundary so recording and playback remain in lockstep.
    runtime.open_frame(&mut frame, &manager.engine, assets.as_ref());
    frame.stage_commands().commands.extend(net_inputs);

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
    host.frontend.draw_order = manager.engine.compute_display_order();

    FrameStart {
        frame,
        mp_clock_pause,
    }
}

/// Apply host-only camera controls. These deliberately remain available while
/// deterministic replay or rewind suppresses simulation commands.
fn apply_host_view_input(
    host: &mut Host,
    engine: &Engine,
    hud: &crate::game_session::interactive::MissionHud,
    mouse_position: engine_coordinates::ScreenPoint,
    keyboard_actions: &[GameAction],
    mouse_actions: &[GameAction],
    events: &[GameEvent],
    view_suppressed: bool,
    pan_suppressed: bool,
) {
    let now_ms = crate::window::process_uptime_ms();
    if pan_suppressed || engine.user_locked() {
        host.frontend.viewport.cancel_touch_motion();
    } else {
        host.frontend.viewport.advance_touch_inertia(now_ms);
    }
    if !view_suppressed {
        for action in keyboard_actions.iter().chain(mouse_actions) {
            let scroll_suppressed_by_minimap =
                matches!(
                    action,
                    GameAction::ScrollUp
                        | GameAction::ScrollDown
                        | GameAction::ScrollLeft
                        | GameAction::ScrollRight
                ) && host.frontend.engine_display.minimap().drag_start();
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
                GameAction::ZoomIn => host.frontend.viewport.zoom_by(2.0, Some(mouse_position)),
                GameAction::ZoomOut => host.frontend.viewport.zoom_by(0.5, Some(mouse_position)),
                _ => {}
            }
        }
    }

    if pan_suppressed || engine.user_locked() {
        return;
    }
    for event in events {
        match *event {
            GameEvent::TouchMotionStop => host.frontend.viewport.cancel_touch_motion(),
            GameEvent::ViewportPan { xrel, yrel } => {
                host.frontend.viewport.cancel_touch_motion();
                host.frontend
                    .viewport
                    .scroll_by(robin_engine::coordinates::ScreenVec::new(
                        -(xrel as f32),
                        -(yrel as f32),
                    ));
                host.frontend.input.cancel_multi_selection();
            }
            GameEvent::TouchTransformStart {
                first_x,
                first_y,
                second_x,
                second_y,
            } => {
                let first = engine_coordinates::ScreenPoint::new(first_x, first_y);
                let second = engine_coordinates::ScreenPoint::new(second_x, second_y);
                let accepted = touch_point_is_world(host, hud, first)
                    && touch_point_is_world(host, hud, second);
                host.frontend.viewport.begin_touch_transform(accepted);
                if accepted {
                    host.frontend.input.cancel_multi_selection();
                }
            }
            GameEvent::TouchTransform {
                centroid_x,
                centroid_y,
                pan_x,
                pan_y,
                scale,
                ..
            } => {
                let applied = host.frontend.viewport.apply_touch_transform(
                    engine_coordinates::ScreenPoint::new(centroid_x, centroid_y),
                    engine_coordinates::ScreenVec::new(pan_x, pan_y),
                    scale,
                );
                if applied {
                    host.frontend.input.cancel_multi_selection();
                }
            }
            GameEvent::TouchTransformEnd {
                velocity_x,
                velocity_y,
                cancelled,
            } => host.frontend.viewport.end_touch_transform(
                engine_coordinates::ScreenVec::new(velocity_x, velocity_y),
                cancelled,
                now_ms,
            ),
            _ => {}
        }
    }
}

fn touch_point_is_world(
    host: &Host,
    hud: &crate::game_session::interactive::MissionHud,
    point: engine_coordinates::ScreenPoint,
) -> bool {
    if !point.x.is_finite()
        || !point.y.is_finite()
        || point.x < 0.0
        || point.y < 0.0
        || point.x >= host.frontend.viewport.screen_size.x
        || point.y >= host.frontend.viewport.screen_size.y - engine_api::PANNEL_HEIGHT
        || host.frontend.engine_display.minimap().is_over_widget(point)
    {
        return false;
    }

    let point = crate::gfx_types::Point::new(point.x as i32, point.y as i32);
    let reserved = [
        hud.zoom_layout.zoom_up,
        hud.zoom_layout.zoom_down,
        hud.corner_layout.clock,
        hud.corner_layout.sight,
        hud.corner_layout.quickstart,
        hud.stature_layout.up,
        hud.stature_layout.down,
        hud.sherwood_layout.display_campaign_map,
        hud.sherwood_layout.go_to_exit,
        hud.sherwood_layout.start_mission,
        hud.sherwood_layout.quit_mission,
    ];
    !reserved.iter().any(|rect| rect.contains_point(point))
}

/// Input owns privileged menu/restore work before downgrading to the world's
/// read-only-engine input phase for gameplay command production.
pub(super) async fn collect_input_and_menus(
    world: &mut MissionWorld,
    runtime: &mut TimelineRuntime,
    control: &mut MissionControl,
    frontend: &mut InteractiveFrontend,
    campaign_transition: &mut Option<crate::main_entry::PendingLevelLoad>,
    window: &mut GameWindow,
    callbacks: &mut RustCallbacks,
    profiles: &engine_profiles::ProfileManager,
) -> Result<ControlFlow<FrameControl, InputPrepared>, String> {
    let FrameStart {
        mut frame,
        mp_clock_pause,
    } = begin_interactive_frame(
        world.ingress(),
        runtime,
        &mut frontend.hud,
        &frontend.presentation,
    );
    let mut modal_rendered_this_frame = false;
    let MissionMutation {
        host,
        game,
        manager,
        assets,
        dev,
    } = world.mutation();
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

    if let Some(transition) = host.transport.take_committed_snapshot_transition() {
        let exit_code = transport::apply_committed_transition(
            transition,
            &mut manager.engine,
            &mut game.operation,
            campaign_transition,
        )?;
        runtime.trace(FrameContractStage::Exit);
        return Ok(ControlFlow::Break(FrameControl::Exit(MissionExit::new(
            exit_code,
        ))));
    }
    if let Some((request, id)) = transport::begin_deferred_campaign_exit(
        &mut host.transport,
        &manager.engine,
        runtime.frame_number(),
        callbacks.pending_request().is_none(),
    ) {
        callbacks.queue_operation(request);
        tracing::info!(
            ?id,
            frame = runtime.frame_number(),
            "multiplayer: waiting for peers to validate the campaign-exit snapshot"
        );
    }
    if host.transport.local_seat() == robin_engine::player_command::PlayerId::HOST
        && let Some(net) = host.transport.net()
    {
        let proposals = net
            .take_all_visible_modal_requests()
            .unwrap_or_else(|error| {
                panic!("failed to present multiplayer modal proposals: {error}")
            });
        if !proposals.is_empty() {
            let summary = proposals
                .iter()
                .map(|request| {
                    format!(
                        "Player {} proposes {:?} for {:?}",
                        request.from.0, request.result, request.kind
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            game.display_message(format!("{summary}; host confirmation is required."), 100);
        }
    }

    let client_waiting_for_campaign_host = host.transport.net().is_some()
        && host.transport.local_seat() != robin_engine::player_command::PlayerId::HOST
        && (game.persistent.campaign_map_active || ui.sherwood_campaign_flow.is_some());
    let campaign_ui_presented = !client_waiting_for_campaign_host
        && (game.persistent.campaign_map_active || ui.sherwood_campaign_flow.is_some());
    match handle_sherwood_campaign_map_overlay(
        game,
        manager,
        host,
        callbacks,
        &mut frame,
        assets,
        &mut *window,
        &mut presentation.renderer,
        &mut resources.cursor,
        &mut presentation.sprites.cursor_renderer,
        &mut resources.text,
        &mut ui.campaign_map,
        &mut ui.sherwood_campaign_flow,
        &mut resources.menu,
        &mut hud.sherwood_enable,
    )? {
        HandlerAction::Continue => {
            runtime.trace(FrameContractStage::EarlyRestart);
            return Ok(ControlFlow::Break(FrameControl::RestartIteration));
        }
        HandlerAction::Exit(code) => {
            runtime.trace(FrameContractStage::Exit);
            return Ok(ControlFlow::Break(FrameControl::Exit(MissionExit::new(
                code,
            ))));
        }
        HandlerAction::Proceed => {}
    }
    modal_rendered_this_frame |= campaign_ui_presented;

    let mission_ui_owns_input = ui.terminal_flow_active()
        || game.persistent.campaign_map_active
        || ui.sherwood_campaign_flow.is_some()
        || ui.active_ui_task.is_some()
        || ui
            .lost_sherwood_gate
            .blocks_mission(game.is_sherwood, &manager.engine);
    let collected = if mission_ui_owns_input {
        EventHudOutcome::Ready(CollectedFrameInput {
            events: Vec::new(),
            keyboard_actions: Vec::new(),
            mouse_actions: Vec::new(),
            modifiers: InputModifiers {
                ctrl: false,
                shift: false,
                alt: false,
                plan: false,
            },
            minimap_toggle_pressed: false,
            pause_closed_this_frame: false,
            rewind_active: false,
            step_forward_pressed: false,
            step_back_pressed: false,
        })
    } else {
        collect_event_and_hud_input(EventHudContext {
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
    };
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
            return Ok(ControlFlow::Break(FrameControl::RestartIteration));
        }
        EventHudOutcome::Control(HandlerAction::Exit(code)) => {
            runtime.trace(FrameContractStage::Exit);
            return Ok(ControlFlow::Break(FrameControl::Exit(MissionExit::new(
                code,
            ))));
        }
        EventHudOutcome::Control(HandlerAction::Proceed) => {
            unreachable!("event/HUD collection must return data when it proceeds")
        }
    };
    // Presentation's planned-action cursor and action-bar highlight follow
    // the rebindable planning control, not physical Shift. Physical Shift
    // remains available in `modifiers.shift` for Original behaviours.
    let shift_held = modifiers.plan;

    // Snapshot adoption, rewind, console and campaign transitions above
    // are privileged control work. Gameplay command production below
    // receives no mutable simulation or EngineManager capability.
    let MissionInputPhase {
        host,
        game,
        engine,
        assets,
        dev,
        mut commands,
        mut external_actions,
    } = world.input_phase(&mut frame);

    // ── View-only input (scroll / zoom): always allowed ──
    // These mutate host-side viewport state only — never the sim —
    // so they're safe during replay playback and rewind, when the
    // user wants to pan/zoom around the paused world.  Suppressed
    // only when the console or the pause menu has focus.
    apply_host_view_input(
        host,
        engine,
        hud,
        input.threaded.position(),
        &kb_actions,
        &mouse_actions,
        &events,
        ui.console_overlay.is_visible() || ui.pause_menu.is_some() || pause_closed_this_frame,
        ui.console_overlay.is_visible()
            || ui.pause_menu.is_some()
            || pause_closed_this_frame
            || !host.frontend.preferences().touch_camera_gestures(),
    );

    // ── Skip all sim-affecting input during replay / rewind ──
    // Recorded commands are injected at the tick boundary instead
    // (replay), or suppressed entirely (rewind — live input
    // shouldn't perturb a state reconstructed from the past).
    if runtime.replay_player.is_none() && !rewind_active {
        match drive_live_gameplay_input(
            LiveGameplayContext {
                host,
                engine,
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
                commands: &mut commands,
                external_actions: &mut external_actions,
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
                return Ok(ControlFlow::Break(FrameControl::RestartIteration));
            }
            HandlerAction::Exit(code) => {
                runtime.trace(FrameContractStage::Exit);
                return Ok(ControlFlow::Break(FrameControl::Exit(MissionExit::new(
                    code,
                ))));
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
            engine,
            game,
            profiles,
            &mut *window,
            &mut presentation.renderer,
            &resources.menu,
        )
    {
        ui.active_ui_task = Some(task);
    }

    runtime.trace(FrameContractStage::InputAndMenus);

    drop(commands);
    drop(external_actions);
    Ok(ControlFlow::Continue(InputPrepared(
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
