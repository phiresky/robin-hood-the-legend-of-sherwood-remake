//! Per-frame mouse + corner-HUD input dispatch.
//!
//! Hosts `handle_mouse_input` (the big mouse-event walker that translates
//! polled events into engine commands), the corner-HUD button click
//! dispatchers, and `choose_recording_place` (the empty-slot picker for
//! the macro recorder).

use super::sherwood_flow::{SherwoodCampaignFlow, SherwoodConfirmationAction};
use super::ui_task_state::{ActiveUiTask, OptionsTaskState, SaveLoadTaskState};
use super::{
    HandlerAction, MissionFrame, center_on_reselected_allied_portrait,
    center_on_reselected_portrait_pc, dispatch_local_command, dispatch_local_commands,
    request_sherwood_trading_panel, required_menu_resources, sherwood_trading_access,
};
use crate::app_effect::{AppEffect, SoundMode};
use crate::audio_backend::KiraAudioBackend;
use crate::campaign_map::{self, CampaignMapChoice};
use crate::corner_hud::CornerButton;
use crate::cursor::CursorRenderer;
use crate::game::{Game, GameCallbacks};
use crate::gfx_types::GameEvent;
use crate::host::{Host, TacticalTargetMode};
use crate::ingame_menu::widget_bridge::default_modal_cursor;
use crate::ingame_menu::{
    self, IngameMenuResources, PauseMenu, PauseMenuOutcome, SaveLoadMode, mission_description,
    resources,
};
use crate::input::ThreadedInput;
use crate::input_translator::InputTranslator;
use crate::main_entry::{RustCallbacks, SaveLoadRequest, current_mission_id};
use crate::menu::CampaignMapState;
use crate::renderer::Renderer;
use crate::sherwood_hud::{SherwoodButton, SherwoodButtonEnable, SherwoodHudLayout};
use crate::ui_panel::{self, PortraitCache, PortraitHitArea, PortraitTarget};
use crate::ui_screens::MissionChoice;
use crate::window::GameWindow;
use robin_assets::res_descr as assets_res_descr;
use robin_assets::resource_manager::ResourceManager;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::element::{Command, Posture};
use robin_engine::engine as engine_api;
use robin_engine::engine::Engine;
use robin_engine::engine_manager as engine_manager_api;
use robin_engine::game_operation::GameCode;
use robin_engine::mission as engine_mission;
use robin_engine::player_command as engine_player_command;
use robin_engine::player_command::{FrameCommands, PlayerCommand};
use robin_engine::profiles as engine_profiles;
use robin_engine::profiles::Action;
use robin_engine::sherwood_stat as engine_sherwood_stat;
use robin_engine::sound_cache::SampleLoader;
use robin_engine::tactical_control::{CombatStance, TacticalFormation};

/// Per-frame mouse-input dispatch.
///
/// Consumes the frame's polled `events`, walks each mouse event, and
/// either applies a `PlayerCommand` directly via `engine.apply_command`
/// or pushes it onto `frame_cmds` for replay recording.  The helper is
/// a straight transplant of the inline block from `run_mission`; no
/// outer-loop control flow (return / break) is involved — the only
/// `continue` statements inside are scoped to the `for event in events`
/// loop and transplant cleanly.
///
/// Button mapping (matching the original game):
///   Left click  = move / interact / select PC (context-sensitive)
///   Left drag   = green selection box
///   Left dblclk = run to location / interact
///   Right click = cancel / stop / deselect-box completion
///   Right drag  = red deselection box
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_mouse_input(
    manager: &mut engine_manager_api::EngineManager,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    screen_width: u16,
    screen_height: u16,
    portrait_cache: &PortraitCache,
    frame_cmds: &mut FrameCommands,
    events: &[GameEvent],
    pause_menu: Option<&PauseMenu>,
    pause_closed_this_frame: bool,
    shift_held: bool,
    ctrl_held: bool,
) {
    let engine = &mut manager.engine;
    // ── Portrait action countdown ──
    // Decrements once per frame. MakeFast fires on double-click within window.
    if host.input.portrait_action_countdown > 0 {
        host.input.portrait_action_countdown -= 1;
        if host.input.portrait_action_countdown == 0 {
            host.input.portrait_action_pc = None;
        }
    }

    // Reset `has_focus = true` once per draw frame so the next
    // frame's input dispatch starts un-suppressed.  A widget that
    // needs the mouse (minimap drag, future modal overlays) flips it
    // back to false; any mouse events processed after that point in
    // the same frame skip the engine-level dispatch (see the
    // `has_focus` guards on the LMB/RMB arms below).  Resetting at
    // the top of `handle_mouse_input` lets the very first mouse
    // event each frame land normally.
    host.input.has_focus = true;

    if pause_menu.is_none() && !pause_closed_this_frame {
        for event in events {
            // When `user_locked` is set (by Command::LockUser, which
            // cutscenes and forced dialogues dispatch), MOUSE_MOVED
            // and MOUSE_BUTTON are dropped.  Filter all mouse events
            // here at the top of the dispatch loop.
            if engine.user_locked()
                && matches!(
                    *event,
                    GameEvent::MouseDown(..)
                        | GameEvent::MouseUp(..)
                        | GameEvent::MouseMove { .. }
                        | GameEvent::ViewportPan { .. }
                        | GameEvent::PointerCancel
                        | GameEvent::TouchMotionStop
                        | GameEvent::TouchTransformStart { .. }
                        | GameEvent::TouchTransform { .. }
                        | GameEvent::TouchTransformEnd { .. }
                )
            {
                continue;
            }
            match *event {
                // ViewportPan is applied unconditionally in
                // `run_mission`'s always-on view-input pass so middle-
                // drag panning works during replay; nothing to do here.
                GameEvent::ViewportPan { .. } => {}
                GameEvent::TouchMotionStop => {}
                GameEvent::PointerCancel => {
                    cancel_left_pointer(engine, host, assets, frame_cmds);
                }
                GameEvent::MouseDown(mx, my, 1, clicks) => {
                    on_left_mouse_down(
                        engine, host, assets, frame_cmds, mx, my, clicks, shift_held,
                    );
                }
                GameEvent::MouseDown(mx, my, 3, clicks) => {
                    on_right_mouse_down(engine, host, mx, my, clicks, shift_held);
                }
                GameEvent::MouseMove { x, y, .. } => {
                    on_mouse_move(engine, host, assets, frame_cmds, x, y, shift_held);
                }
                GameEvent::MouseUp(mx, my, 1) => {
                    on_left_mouse_up(
                        engine,
                        host,
                        assets,
                        portrait_cache,
                        frame_cmds,
                        screen_width,
                        screen_height,
                        mx,
                        my,
                        shift_held,
                        ctrl_held,
                    );
                }
                GameEvent::MouseUp(mx, my, 3) => {
                    on_right_mouse_up(
                        engine,
                        host,
                        assets,
                        portrait_cache,
                        frame_cmds,
                        screen_width,
                        screen_height,
                        mx,
                        my,
                        shift_held,
                    );
                }
                _ => {}
            }
        }
    }
}

/// Tear down a touch-originated left drag without running any release action.
/// In particular this must not box-select, perform a sword gesture, center the
/// minimap, or dispatch a world click when a second finger takes over.
fn cancel_left_pointer(
    engine: &mut Engine,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    frame_cmds: &mut FrameCommands,
) {
    if host.engine_display.minimap().drag_start() {
        dispatch_local_command(
            host,
            engine,
            frame_cmds,
            assets,
            &PlayerCommand::MinimapMouseUp { on_minimap: false },
        );
    }
    host.input.left_mouse_down = false;
    host.input.is_dragging = false;
    host.input.target_drag = None;
    host.input.left_double_click_pending = false;
    host.input.next_left_double_is_simple = false;
    host.input.ignore_next_drag = false;
    host.input.ignore_next_left_click = false;
    host.input.cancel_multi_selection();
    host.input.cancel_multi_unselection();
    host.mouse_way.clear();
}

// ─── Per-event handlers ─────────────────────────────────────────────

/// Left-mouse-down: begin drags (multi-selection box, swordfight
/// gesture polyline, per-action drag) and route minimap presses.
fn on_left_mouse_down(
    engine: &mut Engine,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    frame_cmds: &mut FrameCommands,
    mx: i32,
    my: i32,
    clicks: u8,
    shift_held: bool,
) {
    let local_seat = host.transport.local_seat;
    {
        host.input.left_mouse_down = true;
        host.input.is_dragging = true;
        host.input.left_mouse_start_screen =
            engine_coordinates::ScreenPoint::new(mx as f32, my as f32);
        // When `next_left_double_is_simple` is set, the
        // next left-click is demoted to simple even if the window
        // reports a double-click.  Set by the multi-select
        // path so a box-select doesn't accidentally chain
        // into the double-click repeat path.
        if host.input.next_left_double_is_simple {
            host.input.left_double_click_pending = false;
            host.input.next_left_double_is_simple = false;
        } else {
            host.input.left_double_click_pending = clicks >= 2;
        }

        // Clear the swordfight mouse-way polyline at the
        // start of every left-drag.
        host.mouse_way.clear();

        let click_pt = engine_coordinates::ScreenPoint::new(mx as f32, my as f32);
        let on_minimap = host.engine_display.minimap().is_over_widget(click_pt);

        if on_minimap {
            // Minimap click — start drag if map is deployed.
            // In the event-driven model, MouseDown on the
            // minimap is inherently "entered nicely".
            let cmd = PlayerCommand::MinimapMouseDown {
                click_pt,
                continuing_drag: host.engine_display.minimap().drag_start(),
            };
            dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
            // Don't start multi-selection when clicking minimap
        } else if !host.input.ignore_next_drag
            && host.input.has_focus
            && let Some(map_pt) = host.viewport.screen_to_map(click_pt)
        {
            // Left-drag dispatch:
            //   - `ignore_next_drag` → entire body skipped.
            //   - `has_focus == false` (UI widget grabbed
            //     focus earlier this frame) → skip engine-
            //     level mouse dispatch.
            //   - NoAction / HelpToClimb (with posture
            //     HelpingToClimb) → start multi-selection.
            //   - NoAction additionally bails on alt or
            //     locker.
            //   - Apple / Stone / Hit / HitHard / Heal /
            //     Lever / Strangle → fire the matching
            //     drag action (see `resolve_action_drag`).
            let selected_action = if shift_held {
                engine.planned_action_for_seat(local_seat)
            } else {
                engine.selected_action_for_seat(local_seat)
            };
            let is_swordfighting =
                crate::game_input::is_selected_unit_swordfighting(engine, local_seat);
            match selected_action {
                Action::HelpToClimb => {
                    let posture_ok = engine
                        .hero_selection(local_seat)
                        .first()
                        .and_then(|&id| engine.get_entity(id))
                        .map(|e| e.element_data().posture)
                        == Some(Posture::HelpingToClimb);
                    if posture_ok && !is_swordfighting {
                        host.input.start_multi_selection(map_pt);
                    }
                }
                Action::NoAction
                    if !host.input.is_alt && !engine.view_locked() && !is_swordfighting =>
                {
                    host.input.start_multi_selection(map_pt);
                }
                Action::Apple
                | Action::Stone
                | Action::Hit
                | Action::HitHard
                | Action::Heal
                | Action::Lever
                | Action::Strangle
                    if !shift_held =>
                {
                    let cmds = crate::game_input::resolve_action_drag(host, engine, assets, map_pt);
                    dispatch_local_commands(host, engine, frame_cmds, assets, &cmds);
                }
                _ => {
                    // Other actions (Bow, Net, Purse,
                    // WaspNest, Shield/BigShield, Ale,
                    // Beggar, Listen, Whistle, Eat, Guzzle)
                    // have no drag arm — drag is a no-op
                    // while they're armed.
                }
            }
        }
    }
}

/// Right-mouse-down: start the deselection drag.  Only `NoAction`
/// enables it, and only when not in swordfight, not Alt-held, and not
/// Locker-latched — missing any of these guards caused right-drag to
/// deselect PCs during swordfight, while an action was armed, etc.
fn on_right_mouse_down(
    engine: &mut Engine,
    host: &mut Host,
    _mx: i32,
    _my: i32,
    clicks: u8,
    shift_held: bool,
) {
    let local_seat = host.transport.local_seat;
    {
        host.input.right_mouse_down = true;
        host.right_double_click_pending = clicks >= 2;

        // `has_focus` gate: a UI widget that grabbed
        // focus this frame blocks the deselection-drag
        // from starting.
        let cancelling_planned_action =
            shift_held && engine.planned_action_for_seat(local_seat) != Action::NoAction;
        let guard_ok = !cancelling_planned_action
            && !crate::game_input::is_selected_unit_swordfighting(engine, local_seat)
            && engine.selected_action_for_seat(local_seat) == engine_profiles::Action::NoAction
            && !host.input.is_alt
            && !engine.view_locked()
            && host.input.has_focus;
        if guard_ok
            && let Some(map_pt) = host
                .viewport
                .screen_to_map(engine_coordinates::ScreenPoint::new(_mx as f32, _my as f32))
        {
            host.input.start_multi_unselection(map_pt);
        }
    }
}

/// Mouse-move: swordfight gesture polyline, minimap hover/drag,
/// selection-box updates, and the per-frame action-drag dispatch.
fn on_mouse_move(
    engine: &mut Engine,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    frame_cmds: &mut FrameCommands,
    x: i32,
    y: i32,
    shift_held: bool,
) {
    let local_seat = host.transport.local_seat;
    {
        let mouse_pt = engine_coordinates::ScreenPoint::new(x as f32, y as f32);

        // While a left drag is in progress and the player
        // has a swordfighting PC selected (and isn't
        // holding alt or in another action mode), append
        // every mouse move to the swordfight gesture
        // polyline.  Gated on `is_dragging` (not
        // `left_mouse_down`) so a portrait re-arm on a
        // double-click stops the append path.
        if host.input.is_dragging
            && !host.input.is_alt
            && engine.selected_action_for_seat(local_seat) == Action::NoAction
            && crate::game_input::is_selected_unit_swordfighting(engine, local_seat)
        {
            host.mouse_way.add_point(mouse_pt);
        }

        // ── Minimap hover / drag update ──
        // Single command handles ui_state, entered_nicely,
        // capture, and drag continuation.
        let cmd = PlayerCommand::MinimapMouseMove {
            mouse_pt,
            left_mouse_down: host.input.left_mouse_down,
            continuing_drag: host.input.left_mouse_down
                && host.engine_display.minimap().drag_start(),
        };
        dispatch_local_command(host, engine, frame_cmds, assets, &cmd);

        // Multi-selection box drag (only when not minimap-dragging).
        // Skip the entire drag body while
        // `ignore_next_drag` is latched — the drag never
        // started (guarded at MouseDown), so nothing to
        // update either way; keep the guard for safety.
        if host.input.left_mouse_down
            && !host.engine_display.minimap().drag_start()
            && host.input.multi_selection_active
            && !host.input.ignore_next_drag
            && let Some(map_pt) = host.viewport.screen_to_map(mouse_pt)
        {
            host.input.update_multi_selection(map_pt);
        }
        if host.input.right_mouse_down
            && host.input.multi_unselection_active
            && let Some(map_pt) = host.viewport.screen_to_map(mouse_pt)
        {
            host.input.update_multi_selection(map_pt);
        }

        // ── Action-drag dispatch ──
        // Fire the armed action on every mouse-move frame
        // while the left button is held: when an action
        // like Hit / Apple / Strangle is armed, the moment
        // the cursor crosses over a focusable target the
        // command launches immediately (not at MouseUp).
        //
        // Skip when dragging over the minimap — the
        // minimap captures the drag — and when
        // `ignore_next_drag` has suppressed this drag
        // cycle.
        if !shift_held
            && host.input.left_mouse_down
            && !host.engine_display.minimap().drag_start()
            && !host.input.ignore_next_drag
            && let Some(map_pt) = host.viewport.screen_to_map(mouse_pt)
        {
            let selected_action = engine.selected_action_for_seat(local_seat);
            if matches!(
                selected_action,
                robin_engine::profiles::Action::Apple
                    | robin_engine::profiles::Action::Stone
                    | robin_engine::profiles::Action::Hit
                    | robin_engine::profiles::Action::HitHard
                    | robin_engine::profiles::Action::Heal
                    | robin_engine::profiles::Action::Lever
                    | robin_engine::profiles::Action::Strangle
            ) {
                let cmds = crate::game_input::resolve_action_drag(host, engine, assets, map_pt);
                dispatch_local_commands(host, engine, frame_cmds, assets, &cmds);
            }
        }
    }
}

/// Left-mouse-up: minimap release, box-select completion, portrait
/// clicks, or the world left-click resolver.
#[allow(clippy::too_many_arguments)]
fn on_left_mouse_up(
    engine: &mut Engine,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    portrait_cache: &PortraitCache,
    frame_cmds: &mut FrameCommands,
    screen_width: u16,
    screen_height: u16,
    mx: i32,
    my: i32,
    shift_held: bool,
    ctrl_held: bool,
) {
    let local_seat = host.transport.local_seat;
    {
        host.input.left_mouse_down = false;
        // Drop the dragging flag on button release.
        host.input.is_dragging = false;
        // Clear the drag target on release so the next
        // drag starts fresh.
        host.input.target_drag = None;
        // Clear `ignore_next_drag` at the top of the click
        // handler so a one-shot drag suppression doesn't
        // persist past the button release.
        host.input.ignore_next_drag = false;
        let is_double = host.input.left_double_click_pending;
        host.input.left_double_click_pending = false;

        // ── Minimap click / drag-end handling ──
        // Checks dragged flag, dead zone, and dispatches
        // to open or center-on-click.  Also handles drag
        // release outside the minimap (cleans up drag
        // state so it doesn't linger).
        let click_pt = engine_coordinates::ScreenPoint::new(mx as f32, my as f32);
        let on_minimap = host.engine_display.minimap().is_over_widget(click_pt);
        let minimap_handled = on_minimap || host.engine_display.minimap().drag_start();
        if minimap_handled {
            let center_on = host.engine_display.resolve_minimap_center(
                click_pt,
                on_minimap,
                host.viewport.level_size,
            );
            let cmd = PlayerCommand::MinimapMouseUp { on_minimap };
            dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
            if let Some(point) = center_on {
                dispatch_local_command(
                    host,
                    engine,
                    frame_cmds,
                    assets,
                    &PlayerCommand::CenterCameraOnPoint { point },
                );
            }
            host.input.cancel_multi_selection();
        }

        if minimap_handled {
            // Consumed by minimap — skip normal picking
        } else if !host.input.has_focus {
            // When a UI widget grabbed focus earlier this
            // frame, the engine-level left-click is
            // silently dropped. The active multi-selection
            // drag (if any) is still cleaned up below so
            // the next frame starts clean.
        } else if host.input.multi_selection_active && host.input.draw_multi_selection {
            // Drag was large enough — box-select all PCs in the area.
            // Shift adds to existing selection.
            let cmd = PlayerCommand::BoxSelect {
                pt1: host.input.multi_selection_pt1,
                pt2: host.input.multi_selection_pt2,
                shift: shift_held,
            };
            dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
            if host.control_tactical_units {
                let tactical_cmd = PlayerCommand::BoxSelectTacticalUnits {
                    pt1: host.input.multi_selection_pt1,
                    pt2: host.input.multi_selection_pt2,
                    shift: shift_held,
                };
                dispatch_local_command(host, engine, frame_cmds, assets, &tactical_cmd);
            }
            tracing::info!(
                "Box-select: {} PCs and {} allied soldiers selected",
                engine.hero_selection(local_seat).len(),
                engine.tactical_selection(local_seat).len(),
            );
        } else {
            // Single click (drag too small or no drag started — e.g. panel clicks
            // where screen_to_map returns None so multi_selection never started).
            host.input.cancel_multi_selection();

            // If a swordfight gesture drag was being recorded, the LMB-up
            // commits that gesture — skip portrait hit-testing so a release
            // over a portrait doesn't accidentally select that PC.
            let swordfight_drag =
                crate::game_input::is_selected_unit_swordfighting(engine, local_seat)
                    && !host.mouse_way.is_empty();

            // Check portrait panel first (detailed sub-area hit-test).
            let portrait_hit = if swordfight_drag {
                None
            } else {
                ui_panel::hit_test_portrait_detailed(
                    engine,
                    local_seat,
                    portrait_cache,
                    screen_width,
                    screen_height,
                    mx as f32,
                    my as f32,
                )
            };

            if let Some(hit) = portrait_hit {
                if on_portrait_click(
                    engine, host, assets, frame_cmds, &hit, is_double, shift_held, ctrl_held,
                ) {
                    // `true` mirrors the original `continue`:
                    // the click was fully consumed, skip the
                    // trailing multi-selection cleanup.
                    return;
                }
            } else {
                on_world_click(
                    engine, host, assets, frame_cmds, mx, my, shift_held, ctrl_held, is_double,
                );
            }
        }

        // Clean up multi-selection state at the end of the
        // left-click handler.
        host.input.multi_selection_active = false;
        host.input.multi_unselection_active = false;
        host.input.draw_multi_selection = false;
    }
}

/// Left-mouse-up on a portrait: quick-action slots, macro commit,
/// Shield/Heal portrait targeting, burned-portrait widgets, and
/// portrait (re)selection.
///
/// Returns `true` when the click was fully consumed and the caller
/// must skip its trailing multi-selection cleanup (the paths that were
/// `continue` statements before extraction).
fn on_portrait_click(
    engine: &mut Engine,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    frame_cmds: &mut FrameCommands,
    hit: &ui_panel::PortraitHit,
    is_double: bool,
    shift_held: bool,
    ctrl_held: bool,
) -> bool {
    let local_seat = host.transport.local_seat;

    if matches!(
        hit.area,
        PortraitHitArea::PageLeft | PortraitHitArea::PageRight
    ) {
        let delta = if hit.area == PortraitHitArea::PageLeft {
            -1
        } else {
            1
        };
        let cmd = PlayerCommand::PageTacticalPortraits { delta };
        dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
        return true;
    }

    if !matches!(hit.target, PortraitTarget::Pc(_)) {
        let members = match hit.target {
            PortraitTarget::AlliedSelection => engine.tactical_selection(local_seat).to_vec(),
            PortraitTarget::AlliedGroup(group_id) => engine
                .tactical_pinned_groups(local_seat)
                .iter()
                .find(|group| group.id == group_id)
                .unwrap_or_else(|| panic!("portrait references missing allied group {group_id}"))
                .members
                .clone(),
            PortraitTarget::Pc(_) => unreachable!(),
        };
        match hit.area {
            PortraitHitArea::Pin => {
                let cmd = match hit.target {
                    PortraitTarget::AlliedSelection => PlayerCommand::PinTacticalSelection,
                    PortraitTarget::AlliedGroup(group_id) => {
                        PlayerCommand::UnpinTacticalGroup { group_id }
                    }
                    PortraitTarget::Pc(_) => unreachable!(),
                };
                dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
            }
            PortraitHitArea::AlliedAction(0) => {
                let stance = members
                    .first()
                    .and_then(|id| engine.tactical_order(*id))
                    .map_or(CombatStance::Defensive, |order| order.stance)
                    .next();
                let cmd = PlayerCommand::SetCombatStance {
                    soldiers: members,
                    stance,
                };
                dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
            }
            PortraitHitArea::AlliedAction(1) => {
                let formation = members
                    .first()
                    .and_then(|id| engine.tactical_order(*id))
                    .map_or(TacticalFormation::Line, |order| order.formation);
                host.tactical_target_mode = Some(TacticalTargetMode::Patrol {
                    soldiers: members,
                    formation,
                });
            }
            PortraitHitArea::AlliedAction(2) => {
                let formation = members
                    .first()
                    .and_then(|id| engine.tactical_order(*id))
                    .map_or(TacticalFormation::Line, |order| order.formation)
                    .next();
                let cmd = PlayerCommand::SetTacticalFormation {
                    soldiers: members,
                    formation,
                };
                dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
            }
            PortraitHitArea::TopScroll
            | PortraitHitArea::BottomScroll
            | PortraitHitArea::Visage => {
                center_on_reselected_allied_portrait(
                    host,
                    engine,
                    local_seat,
                    &members,
                    shift_held || ctrl_held,
                    hit.area,
                );
                if let PortraitTarget::AlliedGroup(group_id) = hit.target {
                    let cmd = PlayerCommand::SelectTacticalGroup {
                        group_id,
                        append: shift_held || ctrl_held,
                    };
                    dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
                }
            }
            _ => {}
        }
        return true;
    }
    {
        let pc_id = hit.pc_id;

        if let PortraitHitArea::QuickAction(slot) = hit.area {
            let has_macro = engine.has_quick_action(pc_id, slot);
            let is_recording_slot = engine.is_qa_recording_for(pc_id);
            if engine.is_recording_macro() && is_recording_slot {
                let cmd = PlayerCommand::ChangeQaMemory { slot };
                dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
            } else if has_macro {
                let cmd = PlayerCommand::StartMacro {
                    pc: Some(pc_id),
                    slot,
                };
                dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
            } else {
                return true;
            }
            host.input.multi_selection_active = false;
            host.input.multi_unselection_active = false;
            host.input.draw_multi_selection = false;
            return true;
        }

        // ── Portrait click while recording: stop & commit ──
        // Clicking the portrait of the PC currently
        // being recorded dispatches a
        // stop-recording-macro and swallows the
        // click.  Scoped to visage/scroll areas
        // (non-action-button, non-burned) so the
        // portrait body acts as the "commit macro"
        // button during recording.
        let macro_stop_handled = !hit.is_burned
            && !is_double
            && engine.is_qa_recording_for(pc_id)
            && matches!(
                hit.area,
                PortraitHitArea::TopScroll
                    | PortraitHitArea::BottomScroll
                    | PortraitHitArea::Visage
            );
        if macro_stop_handled {
            let cmd = PlayerCommand::StopRecordingMacro;
            dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
            tracing::info!("Portrait click: stop recording macro on slot {}", hit.slot);
            // Swallow the click.
            host.input.multi_selection_active = false;
            host.input.multi_unselection_active = false;
            host.input.draw_multi_selection = false;
            return true;
        }

        // ── Shield/Heal portrait targeting ──
        // When a Shield/BigShield/Heal action is
        // pending, clicking a non-burned portrait
        // commits that action on the portrait's PC.
        let mut portrait_action_handled = macro_stop_handled;
        if !hit.is_burned && !is_double && !macro_stop_handled {
            let selected_action = if shift_held {
                engine.planned_action_for_seat(local_seat)
            } else {
                engine.selected_action_for_seat(local_seat)
            };
            portrait_action_handled = match selected_action {
                Action::Heal => {
                    // Target must be alive and injured (life < 100).
                    let can_heal = engine
                        .get_entity(pc_id)
                        .and_then(|e| e.pc_data())
                        .is_some_and(|pc| pc.life_points > 0 && pc.life_points < 100);
                    if can_heal {
                        if let Some(&healer_id) = engine.hero_selection(local_seat).first() {
                            let cmds = vec![
                                PlayerCommand::LaunchInteraction {
                                    actor: healer_id,
                                    target: pc_id,
                                    command: Command::HealCmd,
                                    running: false,
                                },
                                PlayerCommand::CancelAction { pc_id: healer_id },
                            ];
                            let cmds = crate::game_input::queue_shift_click_commands(
                                cmds,
                                selected_action,
                                shift_held,
                            );
                            dispatch_local_commands(host, engine, frame_cmds, assets, &cmds);
                            tracing::info!("Portrait heal: {:?} → heal {:?}", healer_id, pc_id);
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                }
                Action::Shield | Action::BigShield => {
                    // While the engine is mid-prompt
                    // for the shield's protected
                    // target, the same-click commit
                    // shortcut is suppressed and the
                    // click falls through to the
                    // world protectee-selection path.
                    // Gated on
                    // `!engine.shield().is_protected`.
                    let mid_prompt = engine.shield().is_protected;
                    // Target must be alive and active.
                    let can_shield = !mid_prompt
                        && engine
                            .get_entity(pc_id)
                            .and_then(|e| e.pc_data())
                            .is_some_and(|pc| pc.life_points > 0);
                    if can_shield {
                        if let Some(&shielder_id) = engine.hero_selection(local_seat).first() {
                            let cmds = vec![
                                PlayerCommand::LaunchInteraction {
                                    actor: shielder_id,
                                    target: pc_id,
                                    command: Command::RaiseShield,
                                    running: false,
                                },
                                PlayerCommand::CancelAction { pc_id: shielder_id },
                            ];
                            let cmds = crate::game_input::queue_shift_click_commands(
                                cmds,
                                selected_action,
                                shift_held,
                            );
                            dispatch_local_commands(host, engine, frame_cmds, assets, &cmds);
                            tracing::info!(
                                "Portrait shield: {:?} → protect {:?}",
                                shielder_id,
                                pc_id
                            );
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                }
                _ => false,
            };
        }

        if portrait_action_handled {
            // Click consumed by portrait action targeting
        } else if hit.is_burned {
            // ── Burned portrait clicks ──
            match hit.area {
                PortraitHitArea::Amulet => {
                    // Amulet click revives from coma.
                    tracing::info!(
                        "Portrait amulet click: slot {}, reviving from coma",
                        hit.slot
                    );
                    let cmd = PlayerCommand::ResetComa { pc_id };
                    dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
                }
                PortraitHitArea::Guard => {
                    // Guard click centers on the guard.
                    if let Some(guard_pos) = engine.get_guard_position(pc_id) {
                        tracing::info!("Portrait guard click: centering on guard");
                        host.viewport.center_on_point(guard_pos);
                    }
                }
                PortraitHitArea::Trumpet => {
                    // Burned-branch trumpet click
                    // dispatches `SendReinforcement`,
                    // which clears `trumpet_enabled`
                    // (so the player can't queue a
                    // second replacement while the
                    // first is in flight), posts the
                    // PC message, arms
                    // `time_till_reinforcement`, and
                    // plays the new-peasant jingle.
                    tracing::info!(
                        "Portrait trumpet click: slot {}, requesting reinforcement",
                        hit.slot
                    );
                    let cmd = PlayerCommand::SendReinforcement { pc_id };
                    dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
                }
                _ => {
                    // Other burned areas: no action
                    // (double-click is a no-op in
                    // burned state).
                }
            }
        } else if is_double {
            // ── Double-click on non-burned portrait ──
            // If the action countdown is active, a
            // double-click accelerates the
            // last-dispatched action (MakeFast).
            if host.input.portrait_action_countdown > 0 {
                if let Some(fast_pc) = host.input.portrait_action_pc {
                    let cmd = PlayerCommand::MakePcFast { pc_id: fast_pc };
                    dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
                }
                host.input.portrait_action_countdown = 0;
                host.input.portrait_action_pc = None;
            } else if engine.is_pc_selectable(assets, pc_id) {
                let cmd = PlayerCommand::SelectPc {
                    pc_id,
                    append: false,
                };
                dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
                tracing::info!("Portrait double-click: selected slot {}", hit.slot);
            } else if let Some(ent) = engine.get_entity(pc_id) {
                host.viewport
                    .center_on_point(ent.position_iface().map_position());
                tracing::info!("Portrait double-click: centering on non-selectable PC");
            }
        } else {
            // ── Normal click on non-burned portrait ──
            match hit.area {
                PortraitHitArea::ActionButton(btn_idx) => {
                    // A left click always selects the action. Ammo dropping is
                    // exclusively a right-click gesture in the original UI.
                    let Some(profile_action) = engine
                        .get_entity(pc_id)
                        .and_then(|entity| entity.pc_data())
                        .and_then(|pc| {
                            assets
                                .profile_manager
                                .get_character(pc.profile_index)
                                .and_then(|profile| profile.actions.get(btn_idx as usize))
                        })
                        .copied()
                    else {
                        tracing::warn!(
                            "Portrait left-click ignored: missing action {} for {:?}",
                            btn_idx,
                            pc_id
                        );
                        return false;
                    };
                    let dispatched = portrait_action_dispatchable(
                        shift_held,
                        profile_action,
                        engine.can_dispatch_pc_action(assets, pc_id, btn_idx),
                    );
                    if dispatched {
                        let planned_action = profile_action;
                        let cmd = if shift_held {
                            PlayerCommand::SelectPlannedAction {
                                pc_id,
                                action: planned_action,
                            }
                        } else {
                            PlayerCommand::SelectAction {
                                pc_id,
                                action_index: btn_idx as u32,
                            }
                        };
                        dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
                        host.input.portrait_action_countdown = 5;
                        host.input.portrait_action_pc =
                            engine.hero_selection(local_seat).first().copied();

                        // Action-button click only arms
                        // the action; the fire-on-target step
                        // happens on the second click of the
                        // two-click flow.  The armed-then-fire
                        // branch lives in the
                        // `portrait_action_handled` path
                        // above, which pulls the actor from
                        // the seat selection, uses the
                        // clicked portrait's PC as the
                        // target, and emits a trailing
                        // `CancelAction`.
                        //
                        // Shield/BigShield additionally have a
                        // two-click danger-point + protected
                        // state machine that a same-click
                        // shortcut cannot cover; sticking to
                        // the two-click flow keeps the path
                        // consistent.

                        tracing::info!(
                            "Portrait action button {}: dispatched on slot {}",
                            btn_idx,
                            hit.slot
                        );
                    } else {
                        let cmd2 = PlayerCommand::SelectPc {
                            pc_id,
                            append: ctrl_held,
                        };
                        dispatch_local_command(host, engine, frame_cmds, assets, &cmd2);
                        tracing::info!(
                            "Portrait action button {} disabled on slot {}; selecting PC",
                            btn_idx,
                            hit.slot
                        );
                    }
                }
                PortraitHitArea::TopScroll
                | PortraitHitArea::BottomScroll
                | PortraitHitArea::Visage => {
                    center_on_reselected_portrait_pc(
                        host, engine, local_seat, pc_id, ctrl_held, hit.area,
                    );
                    let cmd = PlayerCommand::SelectPc {
                        pc_id,
                        append: ctrl_held,
                    };
                    dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
                    tracing::info!("Portrait select: slot {}, area {:?}", hit.slot, hit.area);
                }
                PortraitHitArea::QuickAction(_) => {
                    let cmd = PlayerCommand::SelectPc {
                        pc_id,
                        append: ctrl_held,
                    };
                    dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
                    tracing::info!("Portrait select: slot {}, area {:?}", hit.slot, hit.area);
                }
                // Amulet / Guard / Trumpet only matter on burned portraits,
                // which branch earlier; on a non-burned portrait these
                // areas don't exist, but if the hit-tester returns them
                // we fall back to a plain select rather than dropping
                // the click.
                PortraitHitArea::Amulet | PortraitHitArea::Guard | PortraitHitArea::Trumpet => {
                    let cmd = PlayerCommand::SelectPc {
                        pc_id,
                        append: ctrl_held,
                    };
                    dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
                }
                PortraitHitArea::AlliedAction(_)
                | PortraitHitArea::Pin
                | PortraitHitArea::PageLeft
                | PortraitHitArea::PageRight => {
                    unreachable!("allied portrait hit reached PC portrait handling")
                }
            }
        }
    }
    false
}

fn portrait_action_dispatchable(
    shift_held: bool,
    profile_action: Action,
    live_action_enabled: bool,
) -> bool {
    if shift_held {
        profile_action != Action::NoAction
    } else {
        live_action_enabled
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PortraitActionRightClick {
    Cancel,
    DropAmmo(u32),
}

fn portrait_action_right_click(
    clicked_action: Action,
    selected_action: Action,
    max_ammo: u16,
    is_double: bool,
) -> PortraitActionRightClick {
    // RHWidgetRadioButton sends UnSelect for a selected button. For an
    // already-unselected finite-ammo button it sends DropSingleAmmo, or
    // DropSeveralAmmo for the repeated/double right-click event.
    if clicked_action == selected_action || max_ammo == 0 {
        PortraitActionRightClick::Cancel
    } else {
        PortraitActionRightClick::DropAmmo(if is_double { 5 } else { 1 })
    }
}

/// Left-mouse-up on the world (no portrait hit): swordfight-gesture
/// commit or the regular left-click resolver.
fn on_world_click(
    engine: &mut Engine,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    frame_cmds: &mut FrameCommands,
    mx: i32,
    my: i32,
    shift_held: bool,
    ctrl_held: bool,
    is_double: bool,
) {
    // ── Engine-level LMB release ──
    // Prologue:
    //   ignore_next_drag = false;
    //   if (ignore_next_left_click) {
    //       ignore_next_left_click = false;
    //       cancel multi-selection state;
    //       if (!ctrl_held) return;
    //       target_drag = None;
    //   }
    //   next_left_double_is_simple = false;
    // Clearing `ignore_next_drag` on every LMB
    // release lets a subsequent drag fire again
    // once the button is re-pressed.
    host.input.ignore_next_drag = false;
    let mut swallow_click = false;
    if host.input.ignore_next_left_click {
        host.input.ignore_next_left_click = false;
        // The next platform double-click is
        // already demoted at MouseDown via the
        // `next_left_double_is_simple` flag, so
        // there's nothing extra to do here.
        host.input.multi_selection_active = false;
        host.input.multi_unselection_active = false;
        host.input.draw_multi_selection = false;
        if !ctrl_held {
            swallow_click = true;
        } else {
            host.input.target_drag = None;
        }
    }
    host.input.next_left_double_is_simple = false;

    if !swallow_click
        && let Some(map_pt) = host
            .viewport
            .screen_to_map(engine_coordinates::ScreenPoint::new(mx as f32, my as f32))
    {
        if let Some(TacticalTargetMode::Patrol {
            soldiers,
            formation,
        }) = host.tactical_target_mode.take()
        {
            let cmd = PlayerCommand::SetTacticalPatrol {
                soldiers,
                destination: map_pt,
                formation,
            };
            dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
            return;
        }
        // Resolve swordfight first, then regular click
        let mut cmds = crate::game_input::resolve_swordfight(host, engine, assets, map_pt, true);
        if cmds.is_empty() {
            cmds = crate::game_input::resolve_left_click(
                host, engine, assets, map_pt, shift_held, ctrl_held, is_double,
            );
        }
        let queued_action = if shift_held {
            engine.planned_action_for_seat(host.transport.local_seat)
        } else {
            Action::NoAction
        };
        cmds = crate::game_input::queue_shift_click_commands(cmds, queued_action, shift_held);
        dispatch_local_commands(host, engine, frame_cmds, assets, &cmds);
    }
}

/// Right-mouse-up: deselection box completion, macro-recording commit,
/// alt view-cone clear, minimap close, portrait right-click handling,
/// or the map right-click resolver.
fn on_right_mouse_up(
    engine: &mut Engine,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    portrait_cache: &PortraitCache,
    frame_cmds: &mut FrameCommands,
    screen_width: u16,
    screen_height: u16,
    mx: i32,
    my: i32,
    shift_held: bool,
) {
    let local_seat = host.transport.local_seat;
    let right_double_click = std::mem::take(&mut host.right_double_click_pending);
    {
        host.input.right_mouse_down = false;

        if shift_held && engine.planned_action_for_seat(local_seat) != Action::NoAction {
            let cmd = PlayerCommand::CancelPlannedAction;
            dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
            host.input.cancel_multi_unselection();
            host.input.accept_mouse_event(true, true);
            host.input.next_left_double_is_simple = false;
            host.input.multi_unselection_active = false;
            host.input.multi_selection_active = false;
            host.input.draw_multi_selection = false;
            return;
        }

        if host.tactical_target_mode.take().is_some() {
            host.input.cancel_multi_unselection();
            return;
        }

        // When a UI widget grabbed focus earlier this
        // frame, the engine-level right-click is dropped.
        if !host.input.has_focus {
            host.input.cancel_multi_unselection();
            return;
        }

        // While a macro is recording, right-click commits
        // (stop-recording-macro) and swallows the click.
        // Box-unselect, alt-view-cone clear, and the
        // map/portrait right-click resolver all wait for
        // the next right-click.
        if engine.is_recording_macro() {
            let cmd = PlayerCommand::StopRecordingMacro;
            dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
            host.input.cancel_multi_unselection();
            host.input.ignore_next_drag = false;
            host.input.ignore_next_left_click = false;
            host.input.next_left_double_is_simple = false;
            host.input.multi_selection_active = false;
            host.input.multi_unselection_active = false;
            host.input.draw_multi_selection = false;
            return;
        }

        if host.input.multi_unselection_active && host.input.draw_multi_selection {
            // Red deselection box was drawn — deselect PCs in area
            let cmd = PlayerCommand::BoxUnselect {
                pt1: host.input.multi_selection_pt1,
                pt2: host.input.multi_selection_pt2,
            };
            dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
            tracing::info!(
                "Box-deselect: {} PCs remain selected",
                engine.selected_hero_ids().len()
            );
        } else if engine.is_alt_effective(&host.input) && host.selected_view_element.is_some() {
            // Alt+right-click while the view cone overlay
            // is active swallows the click:
            //   - permanent alt (lock on): unlocks alt
            //     without clearing the selected view
            //     element.
            //   - momentary alt: clears the selected view
            //     element.
            if engine.is_lock_alt() {
                let cmd = PlayerCommand::SetLockAlt(false);
                dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
            } else {
                host.selected_view_element = None;
            }
            host.input.cancel_multi_unselection();
        } else {
            host.input.cancel_multi_unselection();

            // Right-click on minimap closes it.
            if host.engine_display.minimap().is_displayed()
                && host
                    .engine_display
                    .minimap()
                    .is_over_widget(engine_coordinates::ScreenPoint::new(mx as f32, my as f32))
            {
                let cmd = PlayerCommand::MinimapRightClick;
                dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
            } else if let Some(hit) = ui_panel::hit_test_portrait_detailed(
                engine,
                local_seat,
                portrait_cache,
                screen_width,
                screen_height,
                mx as f32,
                my as f32,
            ) {
                if !matches!(hit.target, PortraitTarget::Pc(_)) {
                    let cmd = match hit.target {
                        PortraitTarget::AlliedGroup(group_id) => {
                            PlayerCommand::UnpinTacticalGroup { group_id }
                        }
                        PortraitTarget::AlliedSelection => PlayerCommand::ClearTacticalSelection,
                        PortraitTarget::Pc(_) => unreachable!(),
                    };
                    dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
                    return;
                }
                if let PortraitHitArea::QuickAction(slot) = hit.area {
                    let pc_id = hit.pc_id;
                    let cmd = PlayerCommand::DeleteMacro {
                        pc: Some(pc_id),
                        slot,
                    };
                    dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
                    return;
                }
                // Original radio-button parity: right-clicking the selected
                // action unselects it. Right-clicking an already-unselected
                // finite-ammo action drops one; its repeated/double event
                // requests five. Shift is unrelated to ammo amount.
                let pc_id = hit.pc_id;
                let armed_action = engine.selected_action_for_seat(local_seat);
                let action_armed = matches!(
                    armed_action,
                    robin_engine::profiles::Action::Heal
                        | robin_engine::profiles::Action::Shield
                        | robin_engine::profiles::Action::BigShield
                );
                if !hit.is_burned
                    && let PortraitHitArea::ActionButton(btn_idx) = hit.area
                {
                    let Some((clicked_action, max_ammo)) = engine
                        .get_entity(pc_id)
                        .and_then(|entity| entity.pc_data())
                        .and_then(|pc| assets.profile_manager.get_character(pc.profile_index))
                        .and_then(|profile| {
                            Some((
                                *profile.actions.get(btn_idx as usize)?,
                                *profile.action_max_ammo.get(btn_idx as usize)?,
                            ))
                        })
                    else {
                        tracing::warn!(
                            "Portrait right-click ignored: missing action {} for {:?}",
                            btn_idx,
                            pc_id
                        );
                        host.input.cancel_multi_unselection();
                        return;
                    };

                    match portrait_action_right_click(
                        clicked_action,
                        armed_action,
                        max_ammo,
                        right_double_click,
                    ) {
                        PortraitActionRightClick::Cancel => {
                            let cmd = PlayerCommand::CancelAction { pc_id };
                            dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
                            tracing::info!(
                                "Portrait right-click: cancel action on slot {}",
                                hit.slot
                            );
                        }
                        PortraitActionRightClick::DropAmmo(amount) => {
                            let cmd = PlayerCommand::DropAmmo {
                                pc_id,
                                action_id: clicked_action as u32,
                                amount,
                            };
                            dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
                            tracing::info!(
                                "Portrait right-click: drop {:?} x{} on slot {}",
                                clicked_action,
                                amount,
                                hit.slot
                            );
                        }
                    }
                } else if !hit.is_burned && action_armed {
                    // When the pointer is inside a non-burned
                    // portrait and a Heal/Shield/BigShield
                    // action is armed, right-click cancels the
                    // action regardless of which sub-widget
                    // (visage / scroll / etc.) is under the
                    // pointer.  Emit CancelAction for any
                    // non-`ActionButton` area while an action
                    // is armed.
                    if let Some(&actor_id) = engine.hero_selection(local_seat).first() {
                        let cmd = PlayerCommand::CancelAction { pc_id: actor_id };
                        dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
                        tracing::info!(
                            "Portrait right-click while {:?} armed: cancel on slot {}",
                            armed_action,
                            hit.slot
                        );
                    }
                } else if !hit.is_burned
                    && engine.hero_selection(local_seat).contains(&pc_id)
                    && matches!(
                        hit.area,
                        PortraitHitArea::TopScroll
                            | PortraitHitArea::BottomScroll
                            | PortraitHitArea::Visage
                    )
                {
                    // A right-click on lower/upper/visage of
                    // an open (selected) non-burned portrait
                    // unselects the PC.  Use
                    // `TogglePcSelection` since we already
                    // verified the PC is in the selection.
                    let cmd = PlayerCommand::TogglePcSelection { pc_id };
                    dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
                    tracing::info!("Portrait right-click: unselect PC on slot {}", hit.slot);
                }
                // Other portrait right-click areas: swallow
                // the click (don't fall through to map).
            } else {
                let cmds = crate::game_input::resolve_right_click(engine, local_seat);
                dispatch_local_commands(host, engine, frame_cmds, assets, &cmds);
            }
        }

        // Clean up: wipe the `IgnoreMouseEvent` flags and
        // the multi-selection state so the next frame
        // starts with a clean slate.  The macro-recording
        // short-circuit above already clears these;
        // duplicating the clears on the non-recording path
        // keeps the "flags are zero at end of RMB release"
        // invariant even when `resolve_right_click` ran.
        host.input.accept_mouse_event(true, true);
        host.input.next_left_double_is_simple = false;
        host.input.multi_unselection_active = false;
        host.input.multi_selection_active = false;
        host.input.draw_multi_selection = false;
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn handle_pause_menu_events(
    pause_menu: &mut Option<PauseMenu>,
    active_ui_task: &mut Option<ActiveUiTask>,
    pause_closed_this_frame: &mut bool,
    host: &mut Host,
    manager: &mut engine_manager_api::EngineManager,
    assets: &engine_api::LevelAssets,
    callbacks: &mut RustCallbacks,
    event_pump: &mut GameWindow,
    renderer: &mut Renderer,
    menu_resources: &Option<IngameMenuResources>,
    audio_backend: &mut Option<KiraAudioBackend>,
    sample_loader: &SampleLoader,
    threaded_input: &mut ThreadedInput,
    input_translator: &mut InputTranslator,
    events: &[GameEvent],
) -> HandlerAction {
    if active_ui_task.is_some() {
        return HandlerAction::Proceed;
    }
    let engine = &mut manager.engine;
    // ── Pause menu event handling ──
    // The menu state machine owns all keyboard/mouse input while the
    // game is paused. We feed it the same events the game loop sees
    // and react to its outcome.
    let mut pause_outcome: Option<PauseMenuOutcome> = None;
    if let Some(menu) = pause_menu.as_mut() {
        let authoritative_transition_enabled =
            host.transport.authoritative_transition_actions_enabled();
        menu.set_authoritative_transition_actions_enabled(authoritative_transition_enabled);
        let screen_w = renderer.screen_width() as i32;
        let screen_h = renderer.screen_height() as i32;
        for event in events {
            let backend = audio_backend
                .as_mut()
                .map(|b| b as &mut dyn crate::sound::AudioBackend);
            match menu.handle_event_with_audio(
                event,
                screen_w,
                screen_h,
                Some(&mut host.audio.sound),
                backend,
                Some(sample_loader),
            ) {
                PauseMenuOutcome::Pending => {}
                other => {
                    pause_outcome = Some(other);
                    break;
                }
            }
        }
    }
    if let Some(outcome) = pause_outcome {
        match outcome {
            PauseMenuOutcome::Pending => {}
            PauseMenuOutcome::Continue => {
                *pause_menu = None;
                *pause_closed_this_frame = true;
                renderer.clear_frozen_scene();
                threaded_input.reset_input_state();
                input_translator.reset_state();
                callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Mission));
                if host.transport.net.is_none() {
                    callbacks.start_play_time();
                }
                // Forward a MSG_MOUSE_MOVED at the current cursor
                // position so HUD hover state is re-evaluated on the
                // first frame after the menu closes.
                threaded_input.queue_mouse_motion_resync();
            }
            PauseMenuOutcome::OpenSherwoodTrading => {
                match request_sherwood_trading_panel(host, engine, &assets.profile_manager) {
                    Ok(()) => {
                        *pause_menu = None;
                        *pause_closed_this_frame = true;
                        renderer.clear_frozen_scene();
                        threaded_input.reset_input_state();
                        input_translator.reset_state();
                        callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Mission));
                    }
                    Err(reason) => {
                        // The row was admitted when the menu opened, but the
                        // host/rule/location can change before activation.
                        // Revalidation keeps the stale button fail-closed.
                        tracing::warn!(?reason, "pause-menu Sherwood trading request rejected");
                        if let Some(menu) = pause_menu.as_mut() {
                            menu.reset_after_side_menu();
                        }
                    }
                }
            }
            PauseMenuOutcome::OpenOptions => {
                // RHMenuIngame::OnOptions → RHMenuOptions::Display
                let resources = required_menu_resources(menu_resources, "pause-menu Options");
                let profile = host
                    .application_context
                    .active_profile_snapshot()
                    .unwrap_or_else(|error| {
                        panic!("in-game Options requires an active profile: {error}")
                    });
                // Replay headers and multiplayer snapshots own deterministic
                // simulation toggles. Seed those rows from the active mission
                // rather than a potentially stale local profile.
                let mut gameplay_config = profile.gameplay_config;
                gameplay_config.enable_unbinding = engine.sim_config().enable_unbinding;
                gameplay_config.clean_hands_npc_kills_invalidate =
                    engine.sim_config().clean_hands_npc_kills_invalidate;
                gameplay_config.reusable_cloaks = engine.sim_config().reusable_cloaks;
                gameplay_config.sherwood_trading = engine.sim_config().sherwood_trading;
                *active_ui_task = Some(ActiveUiTask::Options(OptionsTaskState::new(
                    event_pump,
                    renderer,
                    resources,
                    profile.id,
                    profile.graphic_config,
                    gameplay_config,
                    profile.sound_config,
                    host.frontend.key_config.clone(),
                    host.frontend.custom_key_config.clone(),
                    host.audio.sound.can_3d_sound(),
                    host.transport.local_seat == engine_player_command::PlayerId::HOST,
                )));
            }
            PauseMenuOutcome::OpenLoad | PauseMenuOutcome::OpenSave => {
                let mode = if outcome == PauseMenuOutcome::OpenLoad {
                    SaveLoadMode::Load
                } else {
                    SaveLoadMode::Save
                };
                let resources =
                    required_menu_resources(menu_resources, "pause-menu save/load picker");
                let campaign = engine.campaign();
                let mission_id = current_mission_id(campaign, &assets.profile_manager);
                *active_ui_task = Some(ActiveUiTask::SaveLoad(SaveLoadTaskState::new(
                    event_pump,
                    renderer,
                    resources,
                    &mut callbacks.save_manager,
                    mission_id,
                    mode,
                    host.transport.net.is_some(),
                )));
            }
            PauseMenuOutcome::Restart => {
                callbacks.emit_app_effect(AppEffect::SetSoundMode(SoundMode::Mission));
                if host.transport.net.is_some() {
                    assert_eq!(
                        host.transport.local_seat,
                        robin_engine::player_command::PlayerId::HOST,
                        "multiplayer client activated disabled Restart button"
                    );
                    callbacks.pending = Some(SaveLoadRequest::LoadRestart);
                    *pause_menu = None;
                    *pause_closed_this_frame = true;
                    renderer.clear_frozen_scene();
                    threaded_input.reset_input_state();
                    input_translator.reset_state();
                    return HandlerAction::Proceed;
                }
                // Single-player reloads the same mission directly.
                return HandlerAction::Exit(GameCode::LevelRestart);
            }
            PauseMenuOutcome::Quit => {
                // Show the "really quit?" Yes/No prompt.
                let resources =
                    required_menu_resources(menu_resources, "pause-menu Quit confirmation");
                let msg = resources.menu_text.get(resources::MT_MSG_REALLY_QUIT);
                *active_ui_task = Some(ActiveUiTask::Quit(ingame_menu::YesNoModalState::new(
                    event_pump, renderer, resources, msg,
                )));
            }
        }
    }

    HandlerAction::Proceed
}

/// Dispatch a left-click on one of the three corner HUD buttons.
///
/// * Clock — gated on an active PC selection.  If not recording, pick
///   an empty slot with `choose_recording_place` and arm recording;
///   if already recording, rotate to the next slot.
/// * Sight — lock the alt-held flag so the view-cone overlay stays up.
/// * QuickStart — disabled during recording; otherwise launch all PCs'
///   slot-0 macros.
pub(super) fn dispatch_corner_button_left_click(
    btn: crate::corner_hud::CornerButton,
    manager: &mut engine_manager_api::EngineManager,
    game: &mut Game,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    frame_cmds: &mut FrameCommands,
) {
    let local_seat = host.transport.local_seat;
    match btn {
        CornerButton::Clock => {
            if manager.engine.hero_selection(local_seat).is_empty() {
                return;
            }
            if !manager.engine.is_recording_macro() {
                // Pick the first slot where no selected PC already
                // has a macro recorded.
                let slot = choose_recording_place(&manager.engine, local_seat);
                game.level_of_qa = slot as u16;
                let cmd = PlayerCommand::StartRecordingMacro { pc: None, slot };
                dispatch_local_command(host, &mut manager.engine, frame_cmds, assets, &cmd);
            } else {
                let next = ((game.level_of_qa as usize + 1)
                    % robin_engine::macro_store::NUMBER_OF_QA_MEMORY)
                    as u8;
                game.level_of_qa = next as u16;
                let cmd = PlayerCommand::ChangeQaMemory { slot: next };
                dispatch_local_command(host, &mut manager.engine, frame_cmds, assets, &cmd);
            }
        }
        CornerButton::Sight => {
            let cmd = PlayerCommand::SetLockAlt(true);
            dispatch_local_command(host, &mut manager.engine, frame_cmds, assets, &cmd);
        }
        CornerButton::QuickStart => {
            if manager.engine.is_recording_macro() {
                return;
            }
            let cmd = PlayerCommand::StartMacro { pc: None, slot: 0 };
            dispatch_local_command(host, &mut manager.engine, frame_cmds, assets, &cmd);
        }
    }
}

/// Dispatch a right-click on one of the three corner HUD buttons.
///
/// * Clock — drop all slot-0 macros.
/// * Sight — clear the alt-lock and the selected view element.
/// * QuickStart — drop all slot-0 macros (same as Clock).
pub(super) fn dispatch_corner_button_right_click(
    btn: crate::corner_hud::CornerButton,
    manager: &mut engine_manager_api::EngineManager,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    frame_cmds: &mut FrameCommands,
) {
    match btn {
        CornerButton::Clock | CornerButton::QuickStart => {
            // `apply_delete_macro` calls `stop_recording_macro` first
            // (commands.rs), so the engine's `qa_recording_for` is
            // cleared inline — no host-side flag to twiddle.
            let cmd = PlayerCommand::DeleteMacro { pc: None, slot: 0 };
            dispatch_local_command(host, &mut manager.engine, frame_cmds, assets, &cmd);
        }
        CornerButton::Sight => {
            let unlock = PlayerCommand::SetLockAlt(false);
            dispatch_local_command(host, &mut manager.engine, frame_cmds, assets, &unlock);
            // `selected_view_element` is host-side UI state — clear
            // locally, no PlayerCommand needed.
            host.selected_view_element = None;
        }
    }
}

/// Pick the first QA memory slot that *no* currently-selected PC has
/// already populated.  Defaults to slot 0 when every slot is taken.
pub(super) fn choose_recording_place(
    engine: &Engine,
    local_seat: engine_player_command::PlayerId,
) -> u8 {
    let selected = engine.hero_selection(local_seat);
    for slot in 0..robin_engine::macro_store::NUMBER_OF_QA_MEMORY as u8 {
        let taken = selected.iter().any(|&pc| engine.has_quick_action(pc, slot));
        if !taken {
            return slot;
        }
    }
    0
}

fn defer_multiplayer_campaign_exit(
    host: &Host,
    callbacks: &mut RustCallbacks,
    mission_id: u32,
) -> bool {
    let Some(net) = host.transport.net.as_ref() else {
        return false;
    };
    assert_eq!(
        host.transport.local_seat,
        robin_engine::player_command::PlayerId::HOST,
        "only the multiplayer host can launch the selected campaign mission"
    );
    assert!(
        callbacks.pending_multiplayer_campaign_exit.is_none(),
        "a Sherwood campaign transition is already deferred"
    );
    let origin_frame = net.frame_cursor.load(std::sync::atomic::Ordering::Relaxed);
    callbacks.pending_multiplayer_campaign_exit =
        Some(crate::main_entry::PendingMultiplayerCampaignExit {
            not_before_frame: origin_frame
                .saturating_add(robin_engine::multiplayer::INPUT_DELAY_FRAMES)
                .saturating_add(1),
            mission_id,
        });
    true
}

/// Handle the Sherwood-only HUD buttons
/// (DisplayCampaignMap / GoToExit / StartMission / QuitMission).
///
/// Returns `HandlerAction::Continue` if the caller should restart the
/// outer-loop iteration (button consumed input), `Exit(code)` if the
/// caller should return that `GameCode` from `run_mission`
/// (StartMission), or `Proceed` to continue with the rest of the
/// frame.
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_sherwood_hud_buttons(
    game: &mut Game,
    manager: &mut engine_manager_api::EngineManager,
    host: &mut Host,
    frame_cmds: &mut FrameCommands,
    assets: &engine_api::LevelAssets,
    callbacks: &mut RustCallbacks,
    event_pump: &mut GameWindow,
    renderer: &mut Renderer,
    menu_resources: &Option<IngameMenuResources>,
    sherwood_flow: &mut Option<SherwoodCampaignFlow>,
    events: &[GameEvent],
    sherwood_layout: &SherwoodHudLayout,
    sherwood_enable: &mut SherwoodButtonEnable,
) -> HandlerAction {
    if host.transport.net.is_some()
        && host.transport.local_seat != robin_engine::player_command::PlayerId::HOST
    {
        // Sherwood campaign state is host-authored. Clients keep simulating
        // and receive the eventual authoritative mission transition.
        return HandlerAction::Proceed;
    }
    let engine = &mut manager.engine;
    sherwood_enable.sherwood_trading = game.is_sherwood
        && sherwood_trading_access(host, engine, &assets.profile_manager)
            .validate()
            .is_ok();
    // ── Sherwood HUD buttons ──
    //
    // Hit-test the Sherwood-only DisplayCampaignMap / GoToExit /
    // StartMission / QuitMission rects.  The enable mask decides
    // which buttons are live — see
    // `SherwoodButtonEnable::{pre_commit,post_commit}`.
    //
    // Live-refresh the `start_mission` gate every frame when a
    // mission has been committed — driven by portrait-bar changes,
    // reshuffled mission team, etc.
    if game.is_sherwood && !game.persistent.campaign_map_active {
        let men_to_blazon = game.is_men_to_blazon_conversion();
        // Snapshot everything we need before we re-borrow the engine
        // for `are_selected_pc_in_mission_team`, which also walks the
        // campaign through `engine.campaign()`.
        let (has_next_mission, requirements_met, can_convert_merry_men, next_beam_mes) = {
            let campaign = engine.campaign();
            let has_next = campaign.next_mission_idx.is_some();
            let requirements_met = campaign.mission_requirements_met(&assets.profile_manager);
            // Pass the *next* mission (not the blazon mission) to
            // the merry-men-to-blazons check.
            let can_convert = campaign
                .next_mission_idx
                .map(|idx| campaign.can_convert_merry_men_to_blazons(idx, &assets.profile_manager))
                .unwrap_or(false);
            let beam_mes = campaign
                .next_mission_idx
                .and_then(|idx| campaign.missions.get(idx))
                .map(|m| m.profile(&assets.profile_manager).number_of_beam_mes)
                .unwrap_or(0);
            (has_next, requirements_met, can_convert, beam_mes)
        };
        // The men-to-blazon arm runs unconditionally; the non-
        // men-to-blazon arm is gated on having a next mission armed.
        // Run the button-state refresh whenever we're in
        // men-to-blazon conversion or have a next mission armed.
        if men_to_blazon || has_next_mission {
            // Propagate the temp-disable flags into the Sherwood HUD
            // enable mask each frame so the PC-guarded hourglass
            // transient suppression (set by `disable_*_mission_temp`
            // in `Game::perform_hourglass_inner`) actually disables
            // Start / Quit visually.
            let start_disabled_temp = game.start_mission_disabled_temp();
            let quit_disabled_temp = game.quit_mission_disabled_temp();
            sherwood_enable.apply_update_mission_team(
                men_to_blazon,
                can_convert_merry_men,
                requirements_met,
                start_disabled_temp,
                quit_disabled_temp,
            );
            // Sherwood branch of the delayed portraits refresh.
            // Runs every frame so GoToExit tracks portrait-bar
            // changes (reinforcements, deaths) and mission-team
            // commits without waiting for a commit-level transition.
            let portrait_count = engine.pc_ids().len();
            let selected_pc_in_mission_team = engine.are_selected_pc_in_mission_team();
            sherwood_enable.apply_update_portraits_delayed(
                has_next_mission,
                portrait_count,
                next_beam_mes,
                men_to_blazon,
                selected_pc_in_mission_team,
            );
        }
    }

    if game.is_sherwood && !game.persistent.campaign_map_active {
        let mut sherwood_btn_hit = None;
        for event in events {
            if let GameEvent::MouseDown(mx, my, 1 /* left */, _) = *event
                && let Some(btn) = sherwood_layout.hit_test(mx, my, *sherwood_enable)
            {
                sherwood_btn_hit = Some(btn);
                break;
            }
        }
        if let Some(btn) = sherwood_btn_hit {
            match btn {
                SherwoodButton::DisplayCampaignMap => {
                    // Raise the map again so the player can change
                    // their selection.  Only set `campaign_map_active`
                    // here; `campaign_map_displayed` flips when the
                    // overlay actually opens (see
                    // `handle_sherwood_campaign_map_overlay`).
                    game.show_campaign_map();
                    return HandlerAction::Continue;
                }
                SherwoodButton::QuitMission => {
                    // QuitMission in Sherwood mode prompts
                    // REALLY_RETURN_TO_MAP, then on Yes re-raises the
                    // campaign map without leaving Sherwood.
                    if let Some(resources) = menu_resources.as_ref() {
                        let msg = resources
                            .menu_text
                            .get(resources::MT_MSG_REALLY_RETURN_TO_MAP);
                        *sherwood_flow = Some(SherwoodCampaignFlow::Confirmation {
                            state: ingame_menu::YesNoModalState::new(
                                event_pump, renderer, resources, msg,
                            ),
                            action: SherwoodConfirmationAction::ReturnToMap,
                        });
                        return HandlerAction::Continue;
                    }
                    dispatch_local_command(
                        host,
                        engine,
                        frame_cmds,
                        assets,
                        &PlayerCommand::CampaignSelectNextMission { mission_idx: None },
                    );
                    *sherwood_enable = SherwoodButtonEnable::pre_commit();
                    game.show_campaign_map();
                    return HandlerAction::Continue;
                }
                SherwoodButton::StartMission => {
                    // StartMission in Sherwood mode prompts
                    // REALLY_START_MISSION (or REALLY_CONVERT_PEASANTS
                    // in men-to-blazon mode), then serializes Sherwood
                    // and exits to the picked mission.
                    let prompt_id = if game.is_men_to_blazon_conversion() {
                        resources::MT_MSG_REALLY_CONVERT_PEASANTS
                    } else {
                        resources::MT_MSG_REALLY_START_MISSION
                    };
                    if let Some(resources) = menu_resources.as_ref() {
                        let msg = resources.menu_text.get(prompt_id);
                        *sherwood_flow = Some(SherwoodCampaignFlow::Confirmation {
                            state: ingame_menu::YesNoModalState::new(
                                event_pump, renderer, resources, msg,
                            ),
                            action: SherwoodConfirmationAction::StartMission {
                                men_to_blazon: game.is_men_to_blazon_conversion(),
                            },
                        });
                        return HandlerAction::Continue;
                    }
                    if game.is_men_to_blazon_conversion() {
                        // Men-to-blazon branch: unselect everyone, run
                        // the peasants-to-blazons conversion, then
                        // re-open the campaign map and stay in
                        // Sherwood (no mission launch, no Sherwood
                        // serialise).
                        dispatch_local_command(
                            host,
                            engine,
                            frame_cmds,
                            assets,
                            &PlayerCommand::UnselectAllPcs,
                        );
                        dispatch_local_command(
                            host,
                            engine,
                            frame_cmds,
                            assets,
                            &PlayerCommand::CampaignConvertSelectedPeasantsToBlazons,
                        );
                        // The next frame re-opens the Sherwood
                        // campaign map overlay so the player can pick
                        // another mission or exit.
                        game.persistent.campaign_map_active = true;
                        game.persistent.campaign_map_displayed = true;
                        // Clear the conversion flag.  Our persistent
                        // flag survives the overlay round-trip; reset
                        // so a follow-up StartMission click launches a
                        // real mission instead of attempting another
                        // (now empty) conversion pass.
                        game.set_men_to_blazon_conversion(false);
                        dispatch_local_command(
                            host,
                            engine,
                            frame_cmds,
                            assets,
                            &PlayerCommand::SetMenToBlazonConversionMode { on: false },
                        );
                        *sherwood_enable = SherwoodButtonEnable::pre_commit();
                        return HandlerAction::Continue;
                    }
                    let mission_id = current_mission_id(engine.campaign(), &assets.profile_manager);
                    // Harvest Sherwood's production-sector state into
                    // the campaign before exiting.  Executed with the
                    // Sherwood engine still live so current bonus
                    // counts + PC occupants are captured.
                    dispatch_local_command(
                        host,
                        engine,
                        frame_cmds,
                        assets,
                        &PlayerCommand::CampaignHarvestProductionSectorState,
                    );
                    if defer_multiplayer_campaign_exit(host, callbacks, mission_id) {
                        return HandlerAction::Proceed;
                    }
                    callbacks.pending = Some(SaveLoadRequest::Sherwood { mission_id });
                    return HandlerAction::Exit(GameCode::LevelInterrupted);
                }
                SherwoodButton::GoToExit => {
                    // GoToExit dispatches engine message 1000 to the
                    // StartUp script.  The Sherwood StartUp handler
                    // centres the camera on the exit gate tied to the
                    // selected next mission, so the cross-mission
                    // element lookup lives script-side and no Rust-
                    // side registry is needed.
                    dispatch_local_command(
                        host,
                        engine,
                        frame_cmds,
                        assets,
                        &PlayerCommand::DispatchStartupMessage {
                            msg: 1000,
                            arg1: 0,
                            arg2: 0,
                        },
                    );
                    return HandlerAction::Continue;
                }
                SherwoodButton::SherwoodTrading => {
                    if let Err(reason) =
                        request_sherwood_trading_panel(host, engine, &assets.profile_manager)
                    {
                        tracing::warn!(?reason, "live Sherwood trading button request rejected");
                    }
                    return HandlerAction::Continue;
                }
            }
        }
    }

    HandlerAction::Proceed
}

/// Handle the Sherwood campaign-map overlay modal.
///
/// Returns `HandlerAction::Exit(GameCode::Quit)` when the player
/// escapes out of the map (emergency quit-game path).  Returns
/// `Proceed` otherwise.
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_sherwood_campaign_map_overlay(
    game: &mut Game,
    manager: &mut engine_manager_api::EngineManager,
    host: &mut Host,
    callbacks: &mut RustCallbacks,
    frame: &mut MissionFrame,
    assets: &engine_api::LevelAssets,
    event_pump: &mut GameWindow,
    renderer: &mut Renderer,
    cursor_res: &mut ResourceManager,
    cursor_renderer: &mut CursorRenderer,
    text_res: &mut ResourceManager,
    sherwood_campaign_map: &mut CampaignMapState,
    sherwood_flow: &mut Option<SherwoodCampaignFlow>,
    menu_resources: &mut Option<IngameMenuResources>,
    sherwood_enable: &mut SherwoodButtonEnable,
) -> Result<HandlerAction, String> {
    let engine = &mut manager.engine;
    if host.transport.net.is_some()
        && host.transport.local_seat != robin_engine::player_command::PlayerId::HOST
        && (game.persistent.campaign_map_active || sherwood_flow.is_some())
    {
        // Mission-description ticks can spend blazons through synchronous
        // external actions, so clients must not even advance their local
        // campaign modal state. They keep simulating until the host publishes
        // the exact campaign-exit snapshot.
        game.display_message(
            "The multiplayer host is choosing the next campaign action.".to_string(),
            100,
        );
        return Ok(HandlerAction::Proceed);
    }
    if let Some(active_flow) = sherwood_flow.take() {
        match active_flow {
            SherwoodCampaignFlow::Map(state) => {
                *sherwood_flow = Some(SherwoodCampaignFlow::Map(state));
            }
            SherwoodCampaignFlow::PseudoDebrief { mut state } => {
                let outcome = if let Some(resources) = menu_resources.as_ref() {
                    let cursor = Some(default_modal_cursor(cursor_renderer, cursor_res, renderer));
                    state.tick(event_pump, renderer, resources, cursor)
                } else {
                    tracing::warn!(
                        "Pseudo-mission debriefing resources disappeared — acknowledging it"
                    );
                    Some(ingame_menu::DebriefingOutcome::Ok {
                        text_remaining: String::new(),
                    })
                };
                if outcome.is_none() {
                    *sherwood_flow = Some(SherwoodCampaignFlow::PseudoDebrief { state });
                    return Ok(HandlerAction::Proceed);
                }

                let action = engine_api::ExternalAction::AcknowledgePseudoMissionDebrief;
                let result = mission_description::admit_paused_campaign_action(
                    engine,
                    assets,
                    action.clone(),
                );
                assert!(matches!(
                    result,
                    engine_api::ExternalActionResult::AcknowledgePseudoMissionDebrief
                ));
                frame.record_applied_external_action(action);
                if engine.campaign().get_ares() == 0 {
                    return Ok(HandlerAction::Exit(GameCode::Quit));
                }
                game.show_campaign_map();
                return Ok(HandlerAction::Proceed);
            }
            SherwoodCampaignFlow::MissionDescription {
                mission_index,
                mut state,
                mut admitted_actions,
            } => {
                let Some(resources) = menu_resources.as_mut() else {
                    panic!("mission-description resources disappeared while the modal was open")
                };
                let cursor = Some(default_modal_cursor(cursor_renderer, cursor_res, renderer));
                let mut frame_actions = Vec::new();
                let outcome = state.tick(
                    event_pump,
                    renderer,
                    resources,
                    cursor,
                    engine,
                    assets,
                    &mut frame_actions,
                    &assets.profile_manager,
                );
                admitted_actions.extend(frame_actions);
                let Some((choice, men_to_blazon)) = outcome else {
                    *sherwood_flow = Some(SherwoodCampaignFlow::MissionDescription {
                        mission_index,
                        state,
                        admitted_actions,
                    });
                    return Ok(HandlerAction::Proceed);
                };
                for action in admitted_actions {
                    frame.record_applied_external_action(action);
                }
                match choice {
                    MissionChoice::StartMission => {
                        dispatch_local_command(
                            host,
                            engine,
                            &mut frame.commands,
                            assets,
                            &PlayerCommand::CampaignSelectNextMission {
                                mission_idx: Some(mission_index),
                            },
                        );
                        game.set_men_to_blazon_conversion(men_to_blazon);
                        dispatch_local_command(
                            host,
                            engine,
                            &mut frame.commands,
                            assets,
                            &PlayerCommand::SetMenToBlazonConversionMode { on: men_to_blazon },
                        );
                        *sherwood_enable = SherwoodButtonEnable::post_commit();
                    }
                    MissionChoice::ShowPendingMissions => {
                        dispatch_local_command(
                            host,
                            engine,
                            &mut frame.commands,
                            assets,
                            &PlayerCommand::CampaignSwapPendingToAccessibleMissions,
                        );
                        game.show_campaign_map();
                    }
                    MissionChoice::None => game.show_campaign_map(),
                }
                game.persistent.campaign_map_displayed = false;
                return Ok(HandlerAction::Proceed);
            }
            SherwoodCampaignFlow::Confirmation { mut state, action } => {
                let resources =
                    required_menu_resources(menu_resources, "Sherwood mission confirmation");
                let cursor = default_modal_cursor(cursor_renderer, cursor_res, renderer);
                let Some(confirmed) = state.tick(event_pump, renderer, resources, Some(&cursor))
                else {
                    *sherwood_flow = Some(SherwoodCampaignFlow::Confirmation { state, action });
                    return Ok(HandlerAction::Proceed);
                };
                if !confirmed {
                    return Ok(HandlerAction::Proceed);
                }
                match action {
                    SherwoodConfirmationAction::ReturnToMap => {
                        dispatch_local_command(
                            host,
                            engine,
                            &mut frame.commands,
                            assets,
                            &PlayerCommand::CampaignSelectNextMission { mission_idx: None },
                        );
                        *sherwood_enable = SherwoodButtonEnable::pre_commit();
                        game.show_campaign_map();
                        return Ok(HandlerAction::Proceed);
                    }
                    SherwoodConfirmationAction::StartMission {
                        men_to_blazon: true,
                    } => {
                        dispatch_local_command(
                            host,
                            engine,
                            &mut frame.commands,
                            assets,
                            &PlayerCommand::UnselectAllPcs,
                        );
                        dispatch_local_command(
                            host,
                            engine,
                            &mut frame.commands,
                            assets,
                            &PlayerCommand::CampaignConvertSelectedPeasantsToBlazons,
                        );
                        game.persistent.campaign_map_active = true;
                        game.persistent.campaign_map_displayed = true;
                        game.set_men_to_blazon_conversion(false);
                        dispatch_local_command(
                            host,
                            engine,
                            &mut frame.commands,
                            assets,
                            &PlayerCommand::SetMenToBlazonConversionMode { on: false },
                        );
                        *sherwood_enable = SherwoodButtonEnable::pre_commit();
                        return Ok(HandlerAction::Proceed);
                    }
                    SherwoodConfirmationAction::StartMission {
                        men_to_blazon: false,
                    } => {
                        let mission_id =
                            current_mission_id(engine.campaign(), &assets.profile_manager);
                        dispatch_local_command(
                            host,
                            engine,
                            &mut frame.commands,
                            assets,
                            &PlayerCommand::CampaignHarvestProductionSectorState,
                        );
                        if defer_multiplayer_campaign_exit(host, callbacks, mission_id) {
                            return Ok(HandlerAction::Proceed);
                        }
                        callbacks.pending = Some(SaveLoadRequest::Sherwood { mission_id });
                        return Ok(HandlerAction::Exit(GameCode::LevelInterrupted));
                    }
                }
            }
        }
    }
    // ── Sherwood campaign-map overlay ──
    // Open the campaign-map overlay whenever `campaign_map_active`
    // is set (player entered Sherwood, or the DisplayCampaignMap
    // widget fired). The persistent modal below advances exactly one
    // presentation frame per outer mission iteration.
    if game.persistent.campaign_map_active {
        // The `campaign_map_displayed` flag flips here, once the
        // overlay is actually about to open — NOT
        // in `ShowCampaignMap` the way the pre-refactor Rust code did.
        // The split keeps the save/load invariant: a save taken in the
        // "requested but not yet opened" window reloads without the
        // overlay flagged as displayed.
        // Pseudo-mission debriefing is triggered from inside the map
        // modal after its 500 ms timer.
        let pseudo_status = engine.campaign().get_last_pseudo_mission_status();
        let pseudo_debrief_pending = pseudo_status != engine_mission::MissionStatus::Available;
        if sherwood_flow.is_none() {
            let campaign_profile = host
                .application_context
                .active_profile_snapshot()
                .unwrap_or_else(|error| {
                    panic!("campaign presentation requires an active profile: {error}")
                });
            let campaign_view_config = campaign_profile.gameplay_config;
            game.mark_campaign_map_displayed();
            game.take_campaign_map_redisplay();
            let campaign = engine.campaign();
            sherwood_campaign_map.update_all(campaign, &assets.profile_manager);
            let default_menu_text = resources::MenuText::default();
            let menu_text: &dyn engine_sherwood_stat::MenuTextLookup = match menu_resources.as_ref()
            {
                Some(resources) => &resources.menu_text,
                None => &default_menu_text,
            };
            sherwood_campaign_map.update_war_crime_text(campaign, menu_text);
            *sherwood_flow = Some(SherwoodCampaignFlow::Map(
                campaign_map::CampaignMapModalState::new(
                    &host.application_context,
                    renderer,
                    campaign,
                    &assets.profile_manager,
                    sherwood_campaign_map,
                    menu_resources.as_mut(),
                    text_res,
                    host.shipping.as_deref(),
                    pseudo_debrief_pending,
                    campaign_view_config.campaign_presentation,
                    campaign_view_config.show_achievement_badges,
                    &campaign_profile.campaign_history,
                ),
            ));
        }

        let cursor = Some(default_modal_cursor(cursor_renderer, cursor_res, renderer));
        let choice = match sherwood_flow.as_mut() {
            Some(SherwoodCampaignFlow::Map(state)) => state.tick(
                event_pump,
                renderer,
                game,
                engine.campaign(),
                &assets.profile_manager,
                sherwood_campaign_map,
                menu_resources.as_ref(),
                cursor,
            ),
            Some(_) => unreachable!("non-map Sherwood flow reached campaign-map driver"),
            None => unreachable!("campaign-map state was not initialized"),
        };
        let Some(choice) = choice else {
            return Ok(HandlerAction::Proceed);
        };
        *sherwood_flow = None;

        // Handle the redisplay re-entry path before clearing
        // `campaign_map_active`.  `show_campaign_map` returns
        // `Redisplay` when it observed `take_campaign_map_redisplay()
        // == true` at the top of one of its loop iterations — leave
        // `campaign_map_active` set and `Proceed` so the next frame
        // re-enters this handler at the new resolution.
        if matches!(choice, crate::campaign_map::CampaignMapChoice::Redisplay) {
            if game.operation.get_current() != GameCode::LevelInProgress {
                // Exit the redisplay loop when the game operation
                // has changed away from LEVEL_IN_PROGRESS, even if a
                // redisplay was requested.  Clear the overlay flag
                // and fall through to the standard post-modal
                // cleanup below (treated as a Quit-style close —
                // the ARES check + emergency-exit gate fire through
                // the Quit branch).
                game.persistent.campaign_map_active = false;
            } else {
                return Ok(HandlerAction::Proceed);
            }
        }

        // If a redisplay was requested via `take_campaign_map_redisplay`
        // *outside* of the modal's loop poll (legacy path; today the
        // modal consumes the flag itself and returns `Redisplay`), keep
        // `campaign_map_active` set so we re-enter on the next frame.
        let redisplay_requested = game.take_campaign_map_redisplay()
            && game.operation.get_current() == GameCode::LevelInProgress;
        if !redisplay_requested {
            game.persistent.campaign_map_active = false;
        }
        // Defer clearing `campaign_map_displayed` until we know the
        // match arm below didn't take the Quit (emergency-end)
        // branch.  The Quit branch preserves the flag so a save-on-
        // emergency-exit restores the overlay.  Clear eagerly for
        // the non-Quit path; the Quit arm early-returns before we'd
        // reach that clear.
        match choice {
            CampaignMapChoice::PseudoDebriefTimer => {
                let won = pseudo_status == engine_mission::MissionStatus::Won;
                if let Some(resources) = menu_resources.as_ref() {
                    // Try the per-mission win/lose text first, fall
                    // back to the generic strategical-mission text
                    // only if the resource lookup fails.
                    let last_id = engine.campaign().last_pseudo_mission_id;
                    let pseudo_red = {
                        let filename = assets_res_descr::red_filename(last_id);
                        host.shipping
                            .as_deref()
                            .and_then(|dd| dd.localized_level_descriptors(&filename).cloned())
                            .or_else(|| {
                                let path = format!("Data/Text/{filename}");
                                assets_res_descr::load(&path)
                                    .map_err(|e| {
                                        tracing::warn!(
                                            "Pseudo-mission debriefing: failed to load .red {path}: {e}"
                                        );
                                        e
                                    })
                                    .ok()
                            })
                    };
                    let per_mission_text = pseudo_red.as_ref().and_then(|desc| {
                        let table_id = if won {
                            desc.debriefing.win_text_table_id
                        } else {
                            desc.debriefing.lose_text_table_id
                        };
                        if !text_res.has_text_resource(table_id) {
                            return None;
                        }
                        match text_res.get_string(table_id, 0) {
                            Ok(s) => Some(s.to_string()),
                            Err(e) => {
                                tracing::warn!(
                                    "Pseudo-mission debriefing: text {table_id}/0 not found: {e}"
                                );
                                None
                            }
                        }
                    });
                    let text = per_mission_text.unwrap_or_else(|| {
                        let id = if won {
                            resources::MT_MSG_STRATEGICAL_MISSION_WON
                        } else {
                            resources::MT_MSG_STRATEGICAL_MISSION_LOST
                        };
                        resources.menu_text.get(id)
                    });
                    *sherwood_flow = Some(SherwoodCampaignFlow::PseudoDebrief {
                        state: ingame_menu::DebriefingModalState::new(
                            resources, text, None, 0, won, false, None, false, false,
                        ),
                    });
                    return Ok(HandlerAction::Proceed);
                } else {
                    tracing::warn!(
                        "Pseudo-mission debriefing: menu resources unavailable — dropping dialog"
                    );
                }
                let action = engine_api::ExternalAction::AcknowledgePseudoMissionDebrief;
                let result = mission_description::admit_paused_campaign_action(
                    engine,
                    assets,
                    action.clone(),
                );
                assert!(matches!(
                    result,
                    engine_api::ExternalActionResult::AcknowledgePseudoMissionDebrief
                ));
                frame.record_applied_external_action(action);
                let ares_after = engine.campaign().get_ares();
                if ares_after == 0 {
                    return Ok(HandlerAction::Exit(GameCode::Quit));
                }
                game.show_campaign_map();
                return Ok(HandlerAction::Proceed);
            }
            CampaignMapChoice::SelectMission(idx) => {
                // Open the pre-mission description dialog: clicking a
                // location does *not* commit the mission on its own;
                // it pops the mission-description modal first and only
                // commits on `StartMission`.  On
                // `ShowPendingMissions` the accessible list is rebuilt
                // from the pending list; otherwise the campaign map
                // is re-shown.
                if let Some(resources) = menu_resources.as_mut() {
                    let mission_descriptors = {
                        let campaign = engine.campaign();
                        let mission = &campaign.missions[idx];
                        let mission_id = mission.profile(&assets.profile_manager).id;
                        let filename = assets_res_descr::red_filename(mission_id);
                        host.shipping
                            .as_deref()
                            .and_then(|dd| dd.localized_level_descriptors(&filename).cloned())
                            .or_else(|| {
                                let path = format!("Data/Text/{filename}");
                                assets_res_descr::load(&path).ok()
                            })
                    };
                    *sherwood_flow = Some(SherwoodCampaignFlow::MissionDescription {
                        mission_index: idx,
                        state: mission_description::MissionDescriptionModalState::new(
                            event_pump,
                            renderer,
                            resources,
                            idx,
                            engine,
                            &assets.profile_manager,
                            mission_descriptors.as_ref(),
                            text_res,
                        ),
                        admitted_actions: Vec::new(),
                    });
                    return Ok(HandlerAction::Proceed);
                }

                tracing::warn!(
                    "menu_resources unavailable — skipping mission description dialog \
                     and auto-committing mission {idx}"
                );
                dispatch_local_command(
                    host,
                    engine,
                    &mut frame.commands,
                    assets,
                    &PlayerCommand::CampaignSelectNextMission {
                        mission_idx: Some(idx),
                    },
                );
                *sherwood_enable = SherwoodButtonEnable::post_commit();
            }
            CampaignMapChoice::Quit => {
                // Escape / window close from the overlay with no
                // mission committed: exit Sherwood to the main menu.
                // We deliberately leave `campaign_map_displayed` set
                // so a save-on-exit would restore the overlay.
                return Ok(HandlerAction::Exit(GameCode::Quit));
            }
            CampaignMapChoice::Redisplay => {
                // Reached only when a redisplay was requested but
                // `game.operation` was no longer LevelInProgress (the
                // LevelInProgress arm took the `return Ok(Proceed)`
                // path above).  Fall through to the tail cleanup.
            }
        }

        // Non-Quit tail: clear `campaign_map_displayed` (the Quit
        // arm above early-returned so we only hit this on the
        // non-emergency-exit path). Information bars are immediate-mode and
        // read campaign state directly; there is no Engine mutation to queue.
        if !redisplay_requested {
            game.persistent.campaign_map_displayed = false;
        }
    }

    Ok(HandlerAction::Proceed)
}

#[cfg(test)]
mod shift_planning_tests {
    use super::*;

    #[test]
    fn hypothetical_action_can_be_selected_when_live_ammo_disables_it() {
        assert!(portrait_action_dispatchable(true, Action::Bow, false));
        assert!(!portrait_action_dispatchable(false, Action::Bow, false));
        assert!(!portrait_action_dispatchable(true, Action::NoAction, false));
    }

    #[test]
    fn right_click_on_selected_action_only_cancels_it() {
        assert_eq!(
            portrait_action_right_click(Action::Bow, Action::Bow, 15, false),
            PortraitActionRightClick::Cancel
        );
    }

    #[test]
    fn right_click_on_unselected_ammo_action_drops_one_without_shift() {
        assert_eq!(
            portrait_action_right_click(Action::Bow, Action::NoAction, 15, false),
            PortraitActionRightClick::DropAmmo(1)
        );
    }

    #[test]
    fn repeated_right_click_on_unselected_ammo_action_drops_several() {
        assert_eq!(
            portrait_action_right_click(Action::Bow, Action::NoAction, 15, true),
            PortraitActionRightClick::DropAmmo(5)
        );
    }

    #[test]
    fn right_click_on_unselected_unlimited_action_still_cancels() {
        assert_eq!(
            portrait_action_right_click(Action::Hit, Action::NoAction, 0, true),
            PortraitActionRightClick::Cancel
        );
    }
}
