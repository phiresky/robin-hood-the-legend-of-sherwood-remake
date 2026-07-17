//! Per-frame mouse + corner-HUD input dispatch.
//!
//! Hosts `handle_mouse_input` (the big mouse-event walker that translates
//! polled events into engine commands), the corner-HUD button click
//! dispatchers, and `choose_recording_place` (the empty-slot picker for
//! the macro recorder).

use super::{
    HandlerAction, center_on_reselected_portrait_pc, dispatch_local_command,
    dispatch_local_commands, required_menu_resources, restore_required_campaign,
};
use crate::Host;
use crate::campaign::Campaign;
use crate::campaign_map::{self, CampaignMapChoice};
use crate::corner_hud::CornerButton;
use crate::cursor::CursorRenderer;
use crate::element::{Command, Posture};
use crate::game::{Game, GameCallbacks, SoundMode};
use crate::game_operation::GameCode;
use crate::gfx_types::GameEvent;
use crate::graphic_config::GraphicConfig;
use crate::ingame_menu::widget_bridge::default_modal_cursor;
use crate::ingame_menu::{
    self, IngameMenuResources, PauseMenu, PauseMenuOutcome, SaveLoadMode, SaveLoadOutcome,
    mission_description, resources,
};
use crate::input::ThreadedInput;
use crate::input_translator::{GameKey, InputTranslator};
use crate::key_config_store::KeyConfigStore;
use crate::main_entry::{RustCallbacks, SaveLoadRequest, current_mission_id};
use crate::menu::CampaignMapState;
use crate::player_command::{FrameCommands, PlayerCommand};
use crate::player_profile::PlayerProfileManager;
use crate::profiles::Action;
use crate::renderer::Renderer;
use crate::resource_manager::ResourceManager;
use crate::sdl_audio::SdlMixerBackend;
use crate::sherwood_hud::{
    SherwoodButton, SherwoodButtonEnable, SherwoodButtonSprites, SherwoodHudLayout,
};
use crate::sound_cache::SampleLoader;
use crate::sound_config::SoundConfig;
use crate::ui_panel::{self, PortraitCache, PortraitHitArea};
use crate::ui_screens::MissionChoice;
use crate::window::GameWindow;
use crate::zoom_hud::{ZoomButtonSprites, ZoomHudLayout};
use robin_assets::keyconfig as assets_keyconfig;
use robin_assets::res_descr as assets_res_descr;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::engine as engine_api;
use robin_engine::engine::Engine;
use robin_engine::engine_manager as engine_manager_api;
use robin_engine::mission as engine_mission;
use robin_engine::player_command as engine_player_command;
use robin_engine::profiles as engine_profiles;
use robin_engine::sherwood_stat as engine_sherwood_stat;

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
    renderer: &Renderer,
    portrait_cache: &PortraitCache,
    frame_cmds: &mut FrameCommands,
    events: &[GameEvent],
    pause_menu: Option<&PauseMenu>,
    pause_closed_this_frame: bool,
    shift_held: bool,
    ctrl_held: bool,
) {
    let engine = &mut manager.engine;
    let local_seat = host.local_seat;
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
                )
            {
                continue;
            }
            match *event {
                // ViewportPan is applied unconditionally in
                // `run_mission`'s always-on view-input pass so middle-
                // drag panning works during replay; nothing to do here.
                GameEvent::ViewportPan { .. } => {}
                // ── Left mouse down ──
                // LEFTDOWN starts a multi-selection drag.
                GameEvent::MouseDown(mx, my, 1, clicks) => {
                    host.input.left_mouse_down = true;
                    host.input.is_dragging = true;
                    host.input.left_mouse_start_screen =
                        engine_coordinates::ScreenPoint::new(mx as f32, my as f32);
                    // When `next_left_double_is_simple` is set, the
                    // next left-click is demoted to simple even if SDL
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
                        let cmd = PlayerCommand::MinimapMouseDown { click_pt };
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
                        let selected_action = engine.selected_action_for_seat(local_seat);
                        let is_swordfighting = engine.is_seat_selection_swordfighting(local_seat);
                        match selected_action {
                            Action::HelpToClimb => {
                                let posture_ok = engine
                                    .seat_selection(local_seat)
                                    .first()
                                    .and_then(|&id| engine.get_entity(id))
                                    .map(|e| e.element_data().posture)
                                    == Some(Posture::HelpingToClimb);
                                if posture_ok && !is_swordfighting {
                                    host.input.start_multi_selection(map_pt);
                                }
                            }
                            Action::NoAction
                                if !host.input.is_alt
                                    && !engine.locker_active()
                                    && !is_swordfighting =>
                            {
                                host.input.start_multi_selection(map_pt);
                            }
                            Action::Apple
                            | Action::Stone
                            | Action::Hit
                            | Action::HitHard
                            | Action::Heal
                            | Action::Lever
                            | Action::Strangle => {
                                let cmds = crate::game_input::resolve_action_drag(
                                    host, engine, assets, map_pt,
                                );
                                for cmd in &cmds {
                                    frame_cmds.push(cmd.clone());
                                }
                                dispatch_local_commands(host, engine, assets, &cmds);
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

                // ── Right mouse down: start deselection drag ──
                // RIGHTDOWN starts a multi-unselection drag.  Only
                // `NoAction` enables it, and only when not in
                // swordfight, not Alt-held, and not Locker-latched —
                // missing any of these guards caused right-drag to
                // deselect PCs during swordfight, while an action was
                // armed, etc.
                GameEvent::MouseDown(_mx, _my, 3, _) => {
                    host.input.right_mouse_down = true;

                    // `has_focus` gate: a UI widget that grabbed
                    // focus this frame blocks the deselection-drag
                    // from starting.
                    let guard_ok = !engine.is_seat_selection_swordfighting(local_seat)
                        && engine.selected_action_for_seat(local_seat)
                            == engine_profiles::Action::NoAction
                        && !host.input.is_alt
                        && !engine.locker_active()
                        && host.input.has_focus;
                    if guard_ok
                        && let Some(map_pt) =
                            host.viewport
                                .screen_to_map(engine_coordinates::ScreenPoint::new(
                                    _mx as f32, _my as f32,
                                ))
                    {
                        host.input.start_multi_unselection(map_pt);
                    }
                }

                // Mouse move: update minimap drag or multi-select box
                GameEvent::MouseMove { x, y, .. } => {
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
                        && engine.is_seat_selection_swordfighting(local_seat)
                    {
                        host.mouse_way.add_point(mouse_pt);
                    }

                    // ── Minimap hover / drag update ──
                    // Single command handles ui_state, entered_nicely,
                    // capture, and drag continuation.
                    let cmd = PlayerCommand::MinimapMouseMove {
                        mouse_pt,
                        left_mouse_down: host.input.left_mouse_down,
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
                    if host.input.left_mouse_down
                        && !host.engine_display.minimap().drag_start()
                        && !host.input.ignore_next_drag
                        && let Some(map_pt) = host.viewport.screen_to_map(mouse_pt)
                    {
                        let selected_action = engine.selected_action_for_seat(local_seat);
                        if matches!(
                            selected_action,
                            crate::profiles::Action::Apple
                                | crate::profiles::Action::Stone
                                | crate::profiles::Action::Hit
                                | crate::profiles::Action::HitHard
                                | crate::profiles::Action::Heal
                                | crate::profiles::Action::Lever
                                | crate::profiles::Action::Strangle
                        ) {
                            let cmds = crate::game_input::resolve_action_drag(
                                host, engine, assets, map_pt,
                            );
                            for cmd in &cmds {
                                frame_cmds.push(cmd.clone());
                            }
                            dispatch_local_commands(host, engine, assets, &cmds);
                        }
                    }
                }

                // ── Left mouse up: click action or box-select ──
                GameEvent::MouseUp(mx, my, 1) => {
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
                        let cmd = PlayerCommand::MinimapMouseUp {
                            click_pt,
                            on_minimap,
                        };
                        dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
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
                        tracing::info!(
                            "Box-select: {} PCs selected",
                            engine.seat_selection(local_seat).len()
                        );
                    } else {
                        // Single click (drag too small or no drag started — e.g. panel clicks
                        // where screen_to_map returns None so multi_selection never started).
                        host.input.cancel_multi_selection();

                        // If a swordfight gesture drag was being recorded, the LMB-up
                        // commits that gesture — skip portrait hit-testing so a release
                        // over a portrait doesn't accidentally select that PC.
                        let swordfight_drag = engine.is_seat_selection_swordfighting(local_seat)
                            && !host.mouse_way.is_empty();

                        // Check portrait panel first (detailed sub-area hit-test).
                        let portrait_hit = if swordfight_drag {
                            None
                        } else {
                            ui_panel::hit_test_portrait_detailed(
                                engine,
                                local_seat,
                                portrait_cache,
                                renderer.screen_width(),
                                renderer.screen_height(),
                                mx as f32,
                                my as f32,
                            )
                        };

                        if let Some(hit) = portrait_hit {
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
                                    continue;
                                }
                                host.input.multi_selection_active = false;
                                host.input.multi_unselection_active = false;
                                host.input.draw_multi_selection = false;
                                continue;
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
                                tracing::info!(
                                    "Portrait click: stop recording macro on slot {}",
                                    hit.slot
                                );
                                // Swallow the click.
                                host.input.multi_selection_active = false;
                                host.input.multi_unselection_active = false;
                                host.input.draw_multi_selection = false;
                                continue;
                            }

                            // ── Shield/Heal portrait targeting ──
                            // When a Shield/BigShield/Heal action is
                            // pending, clicking a non-burned portrait
                            // commits that action on the portrait's PC.
                            let mut portrait_action_handled = macro_stop_handled;
                            if !hit.is_burned && !is_double && !macro_stop_handled {
                                let selected_action = engine.selected_action_for_seat(local_seat);
                                portrait_action_handled = match selected_action {
                                    Action::Heal => {
                                        // Target must be alive and injured (life < 100).
                                        let can_heal = engine
                                            .get_entity(pc_id)
                                            .and_then(|e| e.pc_data())
                                            .is_some_and(|pc| {
                                                pc.life_points > 0 && pc.life_points < 100
                                            });
                                        if can_heal {
                                            if let Some(&healer_id) =
                                                engine.seat_selection(local_seat).first()
                                            {
                                                let cmd = PlayerCommand::LaunchInteraction {
                                                    actor: healer_id,
                                                    target: pc_id,
                                                    command: Command::HealCmd,
                                                    running: false,
                                                };
                                                dispatch_local_command(
                                                    host, engine, frame_cmds, assets, &cmd,
                                                );
                                                let cancel = PlayerCommand::CancelAction {
                                                    pc_id: healer_id,
                                                };
                                                dispatch_local_command(
                                                    host, engine, frame_cmds, assets, &cancel,
                                                );
                                                tracing::info!(
                                                    "Portrait heal: {:?} → heal {:?}",
                                                    healer_id,
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
                                            if let Some(&shielder_id) =
                                                engine.seat_selection(local_seat).first()
                                            {
                                                let cmd = PlayerCommand::LaunchInteraction {
                                                    actor: shielder_id,
                                                    target: pc_id,
                                                    command: Command::RaiseShield,
                                                    running: false,
                                                };
                                                dispatch_local_command(
                                                    host, engine, frame_cmds, assets, &cmd,
                                                );
                                                let cancel = PlayerCommand::CancelAction {
                                                    pc_id: shielder_id,
                                                };
                                                dispatch_local_command(
                                                    host, engine, frame_cmds, assets, &cancel,
                                                );
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
                                        dispatch_local_command(
                                            host, engine, frame_cmds, assets, &cmd,
                                        );
                                    }
                                    PortraitHitArea::Guard => {
                                        // Guard click centers on the guard.
                                        if let Some(guard_pos) = engine.get_guard_position(pc_id) {
                                            tracing::info!(
                                                "Portrait guard click: centering on guard"
                                            );
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
                                        dispatch_local_command(
                                            host, engine, frame_cmds, assets, &cmd,
                                        );
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
                                        dispatch_local_command(
                                            host, engine, frame_cmds, assets, &cmd,
                                        );
                                    }
                                    host.input.portrait_action_countdown = 0;
                                    host.input.portrait_action_pc = None;
                                } else if engine.is_pc_selectable(assets, pc_id) {
                                    let cmd = PlayerCommand::SelectPc {
                                        pc_id,
                                        append: false,
                                    };
                                    dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
                                    tracing::info!(
                                        "Portrait double-click: selected slot {}",
                                        hit.slot
                                    );
                                } else if let Some(ent) = engine.get_entity(pc_id) {
                                    host.viewport
                                        .center_on_point(ent.position_iface().map_position());
                                    tracing::info!(
                                        "Portrait double-click: centering on non-selectable PC"
                                    );
                                }
                            } else {
                                // ── Normal click on non-burned portrait ──
                                match hit.area {
                                    PortraitHitArea::ActionButton(btn_idx) => {
                                        // After a right-click cancel
                                        // (all buttons deselected),
                                        // clicking a button drops ammo
                                        // instead of arming. Single =
                                        // 1, Several (shift) = 5.
                                        if host.input.portrait_drop_ammo_armed
                                            && engine.selected_action_for_seat(local_seat)
                                                == Action::NoAction
                                        {
                                            // Look up the action for this button index
                                            let btn_action = engine
                                                .get_entity(pc_id)
                                                .and_then(|e| e.pc_data())
                                                .and_then(|pc| {
                                                    assets
                                                        .profile_manager
                                                        .get_character(pc.profile_index)
                                                        .and_then(|p| {
                                                            p.actions.get(btn_idx as usize).copied()
                                                        })
                                                })
                                                .unwrap_or(Action::NoAction);
                                            if btn_action != Action::NoAction {
                                                let amount: u32 = if shift_held { 5 } else { 1 };
                                                let cmd = PlayerCommand::DropAmmo {
                                                    pc_id,
                                                    action_id: btn_action as u32,
                                                    amount,
                                                };
                                                dispatch_local_command(
                                                    host, engine, frame_cmds, assets, &cmd,
                                                );
                                                tracing::info!(
                                                    "Portrait drop ammo: slot {}, action {:?}, amount {}",
                                                    hit.slot,
                                                    btn_action,
                                                    amount
                                                );
                                            }
                                        } else {
                                            // Normal action select path. Resolve the
                                            // dispatch decision read-only before emitting
                                            // the command, so the live and replay paths
                                            // agree on the fallback-select branch.
                                            let dispatched = engine
                                                .can_dispatch_pc_action(assets, pc_id, btn_idx);
                                            if dispatched {
                                                let cmd = PlayerCommand::SelectAction {
                                                    pc_id,
                                                    action_index: btn_idx as u32,
                                                };
                                                dispatch_local_command(
                                                    host, engine, frame_cmds, assets, &cmd,
                                                );
                                                host.input.portrait_drop_ammo_armed = false;
                                                host.input.portrait_action_countdown = 5;
                                                host.input.portrait_action_pc = engine
                                                    .seat_selection(local_seat)
                                                    .first()
                                                    .copied();

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
                                                let cmd2 = PlayerCommand::SelectByPortrait {
                                                    portrait_index: hit.slot as u32,
                                                    append: ctrl_held,
                                                };
                                                dispatch_local_command(
                                                    host, engine, frame_cmds, assets, &cmd2,
                                                );
                                                tracing::info!(
                                                    "Portrait action button {} disabled on slot {}; selecting PC",
                                                    btn_idx,
                                                    hit.slot
                                                );
                                            }
                                        }
                                    }
                                    PortraitHitArea::TopScroll
                                    | PortraitHitArea::BottomScroll
                                    | PortraitHitArea::Visage => {
                                        center_on_reselected_portrait_pc(
                                            host, engine, local_seat, pc_id, ctrl_held, hit.area,
                                        );
                                        let cmd = PlayerCommand::SelectByPortrait {
                                            portrait_index: hit.slot as u32,
                                            append: ctrl_held,
                                        };
                                        dispatch_local_command(
                                            host, engine, frame_cmds, assets, &cmd,
                                        );
                                        tracing::info!(
                                            "Portrait select: slot {}, area {:?}",
                                            hit.slot,
                                            hit.area
                                        );
                                    }
                                    PortraitHitArea::QuickAction(_) => {
                                        let cmd = PlayerCommand::SelectByPortrait {
                                            portrait_index: hit.slot as u32,
                                            append: ctrl_held,
                                        };
                                        dispatch_local_command(
                                            host, engine, frame_cmds, assets, &cmd,
                                        );
                                        tracing::info!(
                                            "Portrait select: slot {}, area {:?}",
                                            hit.slot,
                                            hit.area
                                        );
                                    }
                                    // Amulet / Guard / Trumpet only matter on burned portraits,
                                    // which branch earlier; on a non-burned portrait these
                                    // areas don't exist, but if the hit-tester returns them
                                    // we fall back to a plain select rather than dropping
                                    // the click.
                                    PortraitHitArea::Amulet
                                    | PortraitHitArea::Guard
                                    | PortraitHitArea::Trumpet => {
                                        let cmd = PlayerCommand::SelectByPortrait {
                                            portrait_index: hit.slot as u32,
                                            append: ctrl_held,
                                        };
                                        dispatch_local_command(
                                            host, engine, frame_cmds, assets, &cmd,
                                        );
                                    }
                                }
                            }
                        } else {
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
                                // The next SDL double-click is
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
                                && let Some(map_pt) = host.viewport.screen_to_map(
                                    engine_coordinates::ScreenPoint::new(mx as f32, my as f32),
                                )
                            {
                                // Resolve swordfight first, then regular click
                                let mut cmds = crate::game_input::resolve_swordfight(
                                    host, engine, assets, map_pt, true,
                                );
                                if cmds.is_empty() {
                                    cmds = crate::game_input::resolve_left_click(
                                        host, engine, assets, map_pt, shift_held, ctrl_held,
                                        is_double,
                                    );
                                }
                                for cmd in &cmds {
                                    frame_cmds.push(cmd.clone());
                                }
                                dispatch_local_commands(host, engine, assets, &cmds);
                            }
                        }
                    }

                    // Clean up multi-selection state at the end of the
                    // left-click handler.
                    host.input.multi_selection_active = false;
                    host.input.multi_unselection_active = false;
                    host.input.draw_multi_selection = false;
                }

                // ── Right mouse up: deselection box or right-click action ──
                GameEvent::MouseUp(mx, my, 3) => {
                    host.input.right_mouse_down = false;

                    // When a UI widget grabbed focus earlier this
                    // frame, the engine-level right-click is dropped.
                    if !host.input.has_focus {
                        host.input.cancel_multi_unselection();
                        continue;
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
                        continue;
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
                            engine.selected_pc_ids().len()
                        );
                    } else if engine.is_alt_effective(&host.input)
                        && host.selected_view_element.is_some()
                    {
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
                            && host.engine_display.minimap().is_over_widget(
                                engine_coordinates::ScreenPoint::new(mx as f32, my as f32),
                            )
                        {
                            let cmd = PlayerCommand::MinimapRightClick;
                            dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
                        } else if let Some(hit) = ui_panel::hit_test_portrait_detailed(
                            engine,
                            local_seat,
                            portrait_cache,
                            renderer.screen_width(),
                            renderer.screen_height(),
                            mx as f32,
                            my as f32,
                        ) {
                            if let PortraitHitArea::QuickAction(slot) = hit.area {
                                let pc_id = hit.pc_id;
                                let cmd = PlayerCommand::DeleteMacro {
                                    pc: Some(pc_id),
                                    slot,
                                };
                                dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
                                continue;
                            }
                            // Right-click on portrait action button → cancel action.
                            let pc_id = hit.pc_id;
                            let armed_action = engine.selected_action_for_seat(local_seat);
                            let action_armed = matches!(
                                armed_action,
                                crate::profiles::Action::Heal
                                    | crate::profiles::Action::Shield
                                    | crate::profiles::Action::BigShield
                            );
                            if !hit.is_burned
                                && let PortraitHitArea::ActionButton(_) = hit.area
                            {
                                let cmd = PlayerCommand::CancelAction { pc_id };
                                dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
                                host.input.portrait_drop_ammo_armed = true;
                                tracing::info!(
                                    "Portrait right-click: cancel action on slot {}",
                                    hit.slot
                                );
                            } else if !hit.is_burned && action_armed {
                                // When the pointer is inside a non-burned
                                // portrait and a Heal/Shield/BigShield
                                // action is armed, right-click cancels the
                                // action regardless of which sub-widget
                                // (visage / scroll / etc.) is under the
                                // pointer.  Emit CancelAction for any
                                // non-`ActionButton` area while an action
                                // is armed.
                                if let Some(&actor_id) = engine.seat_selection(local_seat).first() {
                                    let cmd = PlayerCommand::CancelAction { pc_id: actor_id };
                                    dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
                                    tracing::info!(
                                        "Portrait right-click while {:?} armed: cancel on slot {}",
                                        armed_action,
                                        hit.slot
                                    );
                                }
                            } else if !hit.is_burned
                                && engine.seat_selection(local_seat).contains(&pc_id)
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
                                tracing::info!(
                                    "Portrait right-click: unselect PC on slot {}",
                                    hit.slot
                                );
                            }
                            // Other portrait right-click areas: swallow
                            // the click (don't fall through to map).
                        } else {
                            let cmds = crate::game_input::resolve_right_click(engine, local_seat);
                            for cmd in &cmds {
                                frame_cmds.push(cmd.clone());
                            }
                            dispatch_local_commands(host, engine, assets, &cmds);
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

                _ => {}
            }
        }
    }
}

// Holds `PlayerProfileManager::global()` mutex across the
// options-modal `await` — safe under the single-threaded runtime.
#[allow(clippy::too_many_arguments, clippy::await_holding_lock)]
pub(super) async fn handle_pause_menu_events(
    pause_menu: &mut Option<PauseMenu>,
    pause_closed_this_frame: &mut bool,
    host: &mut Host,
    manager: &mut engine_manager_api::EngineManager,
    game: &mut Game,
    assets: &engine_api::LevelAssets,
    callbacks: &mut RustCallbacks,
    campaign_ref: &mut Campaign,
    event_pump: &mut GameWindow,
    renderer: &mut Renderer,
    cursor_res: &mut ResourceManager,
    cursor_renderer: &mut CursorRenderer,
    menu_resources: &Option<IngameMenuResources>,
    audio_backend: &mut Option<SdlMixerBackend>,
    sample_loader: &SampleLoader,
    threaded_input: &mut ThreadedInput,
    input_translator: &mut InputTranslator,
    sherwood_layout: &mut SherwoodHudLayout,
    zoom_layout: &mut ZoomHudLayout,
    zoom_sprites: &ZoomButtonSprites,
    frame_cmds: &mut FrameCommands,
    events: &[GameEvent],
) -> HandlerAction {
    let engine = &mut manager.engine;
    // ── Pause menu event handling ──
    // The menu state machine owns all keyboard/mouse input while the
    // game is paused. We feed it the same events the game loop sees
    // and react to its outcome.
    let mut pause_outcome: Option<PauseMenuOutcome> = None;
    if let Some(menu) = pause_menu.as_mut() {
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
                Some(&mut host.sound),
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
                callbacks.set_sound_mode(SoundMode::Mission);
                // Forward a MSG_MOUSE_MOVED at the current cursor
                // position so HUD hover state is re-evaluated on the
                // first frame after the menu closes.
                threaded_input.queue_mouse_motion_resync();
            }
            PauseMenuOutcome::OpenOptions => {
                // RHMenuIngame::OnOptions → RHMenuOptions::Display
                if let Some(resources) = menu_resources.as_ref() {
                    // Edit the active player profile's configs in
                    // place so changes persist across sessions.
                    let mut guard = PlayerProfileManager::global();
                    let (options_outcome, new_resolution) = if let Some(profile) =
                        guard.as_mut().and_then(|mgr| mgr.get_active_mut())
                    {
                        let cursor =
                            Some(default_modal_cursor(cursor_renderer, cursor_res, renderer));
                        let outcome = ingame_menu::show_options(
                            event_pump,
                            renderer,
                            resources,
                            cursor,
                            &mut profile.graphic_config,
                            &mut profile.sound_config,
                            &mut host.key_config,
                            &mut host.custom_key_config,
                            Some(&mut host.sound),
                            audio_backend
                                .as_mut()
                                .map(|b| b as &mut dyn crate::sound::AudioBackend),
                            Some(sample_loader),
                        )
                        .await;
                        let new_res = outcome.resolution_changed.then_some((
                            profile.graphic_config.resolution_x,
                            profile.graphic_config.resolution_y,
                        ));
                        (outcome, new_res)
                    } else {
                        // No active profile — edit defaults so the UI is
                        // still exercisable in tooling / headless runs.
                        let mut graphic = GraphicConfig::default();
                        let mut sound_cfg = SoundConfig::default();
                        let mut key_cfg = assets_keyconfig::KeyConfig::default_preset();
                        let mut custom_key_cfg = key_cfg.clone();
                        let cursor =
                            Some(default_modal_cursor(cursor_renderer, cursor_res, renderer));
                        let outcome = ingame_menu::show_options(
                            event_pump,
                            renderer,
                            resources,
                            cursor,
                            &mut graphic,
                            &mut sound_cfg,
                            &mut key_cfg,
                            &mut custom_key_cfg,
                            Some(&mut host.sound),
                            audio_backend
                                .as_mut()
                                .map(|b| b as &mut dyn crate::sound::AudioBackend),
                            Some(sample_loader),
                        )
                        .await;
                        let new_res = outcome
                            .resolution_changed
                            .then_some((graphic.resolution_x, graphic.resolution_y));
                        (outcome, new_res)
                    };

                    // On resolution change, switch the draw surface,
                    // update input clipping, and resize the engine.
                    // We skip the close + re-open dance and just let
                    // the pause menu re-render at the new resolution
                    // next frame.
                    if let Some((new_w, new_h)) = new_resolution {
                        let w = new_w;
                        let h = new_h;
                        let w_u16 = w.round() as u16;
                        let h_u16 = h.round() as u16;
                        event_pump.set_logical_size(w_u16 as u32, h_u16 as u32);
                        host.viewport.set_screen_size(w, h);
                        renderer.resize(w_u16, h_u16);
                        threaded_input.set_clipping(
                            robin_engine::coordinates::ScreenBBox::from_coords(0.0, 0.0, w, h),
                        );
                        *input_translator = InputTranslator::new(w, h);
                        // Re-install HUD-adjacent dead zones at the
                        // new resolution.
                        input_translator.install_hud_dead_zones();
                        if host.minimap_corner_size.x > 0.0 {
                            let cmd = PlayerCommand::MinimapResize {
                                base: engine_coordinates::ScreenPoint::new(w - 83.0, 38.0),
                                corner_size: host.minimap_corner_size,
                            };
                            dispatch_local_command(host, engine, frame_cmds, assets, &cmd);
                        }
                        *sherwood_layout = SherwoodHudLayout::for_resolution(
                            w_u16 as u32,
                            h_u16 as u32,
                            &SherwoodButtonSprites::default(),
                        );
                        *zoom_layout =
                            ZoomHudLayout::for_resolution(w_u16 as u32, h_u16 as u32, zoom_sprites);
                        // Re-show the campaign map overlay if it was
                        // active.  No-op when it isn't; when it is
                        // (e.g. a save taken with `campaign_map_active
                        // = true` restored at a different resolution),
                        // arms the redisplay flag so the campaign-map
                        // handler rebuilds the modal at the new size
                        // on the next frame.
                        game.reshow_campaign_map();
                    }

                    // Push the (possibly updated) GraphicConfig
                    // through the shadow polygon and per-element shadow
                    // caches.  Today a near-no-op (see method doc) but
                    // kept here so the ordering is in place.
                    engine.change_detail_level();

                    // Persist profile manager whenever any graphic/sound/key
                    // setting was edited — otherwise in-game option changes
                    // (e.g. scaling mode) survive the session but are lost
                    // when the game exits.
                    if options_outcome.changed
                        && let Some(mgr) = guard.as_ref()
                        && let Err(err) = mgr.save()
                    {
                        tracing::error!("Options: failed to save profile manager: {err:#}");
                    }
                    // Post-OK pipeline for the shortcuts modal:
                    // persist key-config store and refresh the input
                    // translator + minimap accelerator from the new
                    // bindings.  The shortcuts modal already wrote into
                    // host.{key_config, custom_key_config}; sync those
                    // back to the store.
                    if options_outcome.key_config_changed {
                        if let Some(profile_id) =
                            guard.as_ref().and_then(|m| m.get_active().map(|p| p.id))
                        {
                            let mut store_guard = KeyConfigStore::global();
                            if let Some(store) = store_guard.as_mut() {
                                let entry = store.entry_or_default(profile_id);
                                entry.active = host.key_config.clone();
                                entry.custom = host.custom_key_config.clone();
                                if let Err(err) = store.save() {
                                    tracing::error!(
                                        "Options: failed to save key configs after change: {err:#}"
                                    );
                                }
                            }
                        }
                        input_translator.load_bindings_from_keyconfig(&host.key_config);
                        host.minimap_fast_key = input_translator.get_binding(GameKey::DisplayMap);
                    }
                }
                if let Some(menu) = pause_menu.as_mut() {
                    menu.reset_after_side_menu();
                    let sw = renderer.screen_width() as i32;
                    let sh = renderer.screen_height() as i32;
                    menu.seed_mouse_from_sdl(event_pump, sw, sh);
                }
            }
            PauseMenuOutcome::OpenLoad | PauseMenuOutcome::OpenSave => {
                let mode = if outcome == PauseMenuOutcome::OpenLoad {
                    SaveLoadMode::Load
                } else {
                    SaveLoadMode::Save
                };
                let mut close_pause_menu = false;
                let resources =
                    required_menu_resources(menu_resources, "pause-menu save/load picker");
                let campaign = engine
                    .campaign()
                    .expect("pause-menu save/load picker requires the engine campaign");
                let mission_id = current_mission_id(campaign, &assets.profile_manager);
                let cursor = Some(default_modal_cursor(cursor_renderer, cursor_res, renderer));
                let picker_outcome = ingame_menu::show_save_load(
                    event_pump,
                    renderer,
                    resources,
                    cursor,
                    &mut callbacks.save_manager,
                    mission_id,
                    Some(&assets.profile_manager),
                    mode,
                    Some(&mut host.sound),
                    audio_backend
                        .as_mut()
                        .map(|b| b as &mut dyn crate::sound::AudioBackend),
                    Some(sample_loader),
                )
                .await;
                if let SaveLoadOutcome::Slot(slot) = picker_outcome {
                    callbacks.pending = Some(match mode {
                        SaveLoadMode::Save => SaveLoadRequest::Save {
                            slot: Some(slot),
                            mission_id,
                        },
                        SaveLoadMode::Load => SaveLoadRequest::Load {
                            slot: Some(slot),
                            mission_id,
                        },
                    });
                    // When the picker returns a slot, close the
                    // pause-menu modal so the outer game loop
                    // processes the save/load and resumes.  Only
                    // the cancel branch falls through to restore
                    // the menu.
                    close_pause_menu = true;
                }
                if close_pause_menu {
                    *pause_menu = None;
                    *pause_closed_this_frame = true;
                    renderer.clear_frozen_scene();
                    threaded_input.reset_input_state();
                    input_translator.reset_state();
                    callbacks.set_sound_mode(SoundMode::Mission);
                } else if let Some(menu) = pause_menu.as_mut() {
                    menu.reset_after_side_menu();
                    let sw = renderer.screen_width() as i32;
                    let sh = renderer.screen_height() as i32;
                    menu.seed_mouse_from_sdl(event_pump, sw, sh);
                }
            }
            PauseMenuOutcome::Restart => {
                // Reload the same mission.
                callbacks.set_sound_mode(SoundMode::Mission);
                restore_required_campaign(
                    campaign_ref,
                    engine.take_campaign(),
                    "pause-menu restart",
                );
                return HandlerAction::Exit(GameCode::LevelRestart);
            }
            PauseMenuOutcome::Quit => {
                // Show the "really quit?" Yes/No prompt.
                let resources =
                    required_menu_resources(menu_resources, "pause-menu Quit confirmation");
                let msg = resources.menu_text.get(resources::MT_MSG_REALLY_QUIT);
                let cursor = Some(default_modal_cursor(cursor_renderer, cursor_res, renderer));
                let confirmed =
                    ingame_menu::show_yesno(event_pump, renderer, resources, cursor, &msg).await;
                if confirmed {
                    callbacks.set_sound_mode(SoundMode::Mission);
                    restore_required_campaign(
                        campaign_ref,
                        engine.take_campaign(),
                        "confirmed pause-menu Quit",
                    );
                    return HandlerAction::Exit(GameCode::Quit);
                }
                if let Some(menu) = pause_menu.as_mut() {
                    menu.reset_after_side_menu();
                    let sw = renderer.screen_width() as i32;
                    let sh = renderer.screen_height() as i32;
                    menu.seed_mouse_from_sdl(event_pump, sw, sh);
                }
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
    let local_seat = host.local_seat;
    match btn {
        CornerButton::Clock => {
            if manager.engine.seat_selection(local_seat).is_empty() {
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
                    % crate::macro_store::NUMBER_OF_QA_MEMORY) as u8;
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
    let selected = engine.seat_selection(local_seat);
    for slot in 0..crate::macro_store::NUMBER_OF_QA_MEMORY as u8 {
        let taken = selected.iter().any(|&pc| engine.has_quick_action(pc, slot));
        if !taken {
            return slot;
        }
    }
    0
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
pub(super) async fn handle_sherwood_hud_buttons(
    game: &mut Game,
    manager: &mut engine_manager_api::EngineManager,
    host: &mut Host,
    frame_cmds: &mut FrameCommands,
    assets: &engine_api::LevelAssets,
    callbacks: &mut RustCallbacks,
    campaign_ref: &mut Campaign,
    event_pump: &mut GameWindow,
    renderer: &mut Renderer,
    cursor_res: &mut ResourceManager,
    cursor_renderer: &mut CursorRenderer,
    menu_resources: &Option<IngameMenuResources>,
    events: &[GameEvent],
    sherwood_layout: &SherwoodHudLayout,
    sherwood_enable: &mut SherwoodButtonEnable,
    headless: bool,
) -> HandlerAction {
    let engine = &mut manager.engine;
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
            let campaign = engine.campaign().expect("campaign");
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
                    let confirmed = if headless {
                        true
                    } else if let Some(resources) = menu_resources.as_ref() {
                        let msg = resources
                            .menu_text
                            .get(resources::MT_MSG_REALLY_RETURN_TO_MAP);
                        let cursor =
                            Some(default_modal_cursor(cursor_renderer, cursor_res, renderer));
                        ingame_menu::show_yesno(event_pump, renderer, resources, cursor, &msg).await
                    } else {
                        true
                    };
                    if confirmed {
                        dispatch_local_command(
                            host,
                            engine,
                            frame_cmds,
                            assets,
                            &PlayerCommand::CampaignSelectNextMission { mission_idx: None },
                        );
                        *sherwood_enable = SherwoodButtonEnable::pre_commit();
                        // See the `ShowCampaignMap` note elsewhere: only
                        // the active flag gets set here; the displayed
                        // flag flips when the overlay opens.
                        game.show_campaign_map();
                    }
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
                    let confirmed = if headless {
                        true
                    } else if let Some(resources) = menu_resources.as_ref() {
                        let msg = resources.menu_text.get(prompt_id);
                        let cursor =
                            Some(default_modal_cursor(cursor_renderer, cursor_res, renderer));
                        ingame_menu::show_yesno(event_pump, renderer, resources, cursor, &msg).await
                    } else {
                        true
                    };
                    if !confirmed {
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
                    let mission_id = current_mission_id(
                        engine.campaign().expect("campaign"),
                        &assets.profile_manager,
                    );
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
                    callbacks.pending = Some(SaveLoadRequest::Sherwood { mission_id });
                    restore_required_campaign(
                        campaign_ref,
                        engine.take_campaign(),
                        "Sherwood mission launch",
                    );
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
pub(super) async fn handle_sherwood_campaign_map_overlay(
    game: &mut Game,
    manager: &mut engine_manager_api::EngineManager,
    host: &mut Host,
    frame_cmds: &mut FrameCommands,
    assets: &engine_api::LevelAssets,
    campaign_ref: &mut Campaign,
    event_pump: &mut GameWindow,
    renderer: &mut Renderer,
    cursor_res: &mut ResourceManager,
    cursor_renderer: &mut CursorRenderer,
    text_res: &mut ResourceManager,
    sherwood_campaign_map: &mut CampaignMapState,
    menu_resources: &mut Option<IngameMenuResources>,
    sherwood_enable: &mut SherwoodButtonEnable,
) -> Result<HandlerAction, String> {
    let engine = &mut manager.engine;
    // ── Sherwood campaign-map overlay ──
    // Open the campaign-map overlay whenever `campaign_map_active`
    // is set (player entered Sherwood, or the DisplayCampaignMap
    // widget fired).  It's a blocking modal — `show_campaign_map`
    // polls events inline until the player selects a mission or
    // dismisses the map.
    if game.persistent.campaign_map_active {
        // The `campaign_map_displayed` flag flips here, once the
        // overlay is actually about to open — NOT
        // in `ShowCampaignMap` the way the pre-refactor Rust code did.
        // The split keeps the save/load invariant: a save taken in the
        // "requested but not yet opened" window reloads without the
        // overlay flagged as displayed.
        game.mark_campaign_map_displayed();

        // Clear any prior `ReshowCampaignMap` request so a stale flag
        // from a previous frame doesn't trick us into looping forever.
        game.take_campaign_map_redisplay();

        // Pseudo-mission debriefing is triggered from inside the map
        // modal after its 500 ms timer.
        let pseudo_status = engine
            .campaign()
            .expect("campaign")
            .get_last_pseudo_mission_status();
        let pseudo_debrief_pending = pseudo_status != engine_mission::MissionStatus::Available;

        let campaign = engine.campaign().expect("campaign");
        sherwood_campaign_map.update_all(campaign, &assets.profile_manager);
        // `menu_resources` is `None` only if `DEFAULT.RES` failed to
        // load — rare dev-only case.  Default `MenuText` returns an
        // empty string for every id, so the status bar just shows
        // the raw numbers.
        let default_menu_text = resources::MenuText::default();
        let menu_text: &dyn engine_sherwood_stat::MenuTextLookup = match menu_resources.as_ref() {
            Some(r) => &r.menu_text,
            None => &default_menu_text,
        };
        sherwood_campaign_map.update_war_crime_text(campaign, menu_text);

        let cursor = Some(default_modal_cursor(cursor_renderer, cursor_res, renderer));
        let choice = campaign_map::show_campaign_map(
            event_pump,
            renderer,
            game,
            campaign,
            &assets.profile_manager,
            sherwood_campaign_map,
            menu_resources.as_mut(),
            text_res,
            host.shipping.as_deref(),
            cursor,
            pseudo_debrief_pending,
        )
        .await?;

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
                    let last_id = engine.campaign().expect("campaign").last_pseudo_mission_id;
                    let pseudo_red = {
                        let filename = assets_res_descr::red_filename(last_id);
                        host.shipping
                            .as_deref()
                            .and_then(|dd| dd.red_files.get(&filename).cloned())
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
                    let cursor = Some(default_modal_cursor(cursor_renderer, cursor_res, renderer));
                    let _outcome = ingame_menu::show_debriefing(
                        event_pump, renderer, resources, cursor, &text, None, 0, won, false, None,
                        false, false,
                    )
                    .await;
                } else {
                    tracing::warn!(
                        "Pseudo-mission debriefing: menu resources unavailable — dropping dialog"
                    );
                }
                engine.campaign_reset_last_pseudo_mission_status();
                let ares_after = engine.campaign().expect("campaign").get_ares();
                if ares_after == 0 {
                    restore_required_campaign(
                        campaign_ref,
                        engine.take_campaign(),
                        "lost pseudo-mission exit",
                    );
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
                let desc_outcome = if let Some(resources) = menu_resources.as_mut() {
                    let mission_descriptors = {
                        let campaign = engine.campaign().expect("campaign");
                        let mission = &campaign.missions[idx];
                        let mission_id = mission.profile(&assets.profile_manager).id;
                        let filename = assets_res_descr::red_filename(mission_id);
                        host.shipping
                            .as_deref()
                            .and_then(|dd| dd.red_files.get(&filename).cloned())
                            .or_else(|| {
                                let path = format!("Data/Text/{filename}");
                                assets_res_descr::load(&path).ok()
                            })
                    };
                    let cursor = Some(default_modal_cursor(cursor_renderer, cursor_res, renderer));
                    let (choice, men_to_blazon) = mission_description::show_mission_description(
                        event_pump,
                        renderer,
                        resources,
                        cursor,
                        idx,
                        engine,
                        &assets.profile_manager,
                        mission_descriptors.as_ref(),
                        text_res,
                    )
                    .await;
                    Some((choice, men_to_blazon))
                } else {
                    tracing::warn!(
                        "menu_resources unavailable — skipping mission description dialog \
                         and auto-committing mission {idx}"
                    );
                    None
                };

                match desc_outcome {
                    // Menu resources missing (dev path without
                    // DEFAULT.RES) — preserve the old direct-commit
                    // behaviour so the game still progresses.
                    None => {
                        dispatch_local_command(
                            host,
                            engine,
                            frame_cmds,
                            assets,
                            &PlayerCommand::CampaignSelectNextMission {
                                mission_idx: Some(idx),
                            },
                        );
                        *sherwood_enable = SherwoodButtonEnable::post_commit();
                    }
                    Some((MissionChoice::StartMission, men_to_blazon)) => {
                        // Set the next mission + toggle the
                        // men-to-blazon conversion flag, then close.
                        // The HUD commit path (StartMission button)
                        // runs afterwards.
                        dispatch_local_command(
                            host,
                            engine,
                            frame_cmds,
                            assets,
                            &PlayerCommand::CampaignSelectNextMission {
                                mission_idx: Some(idx),
                            },
                        );
                        game.set_men_to_blazon_conversion(men_to_blazon);
                        dispatch_local_command(
                            host,
                            engine,
                            frame_cmds,
                            assets,
                            &PlayerCommand::SetMenToBlazonConversionMode { on: men_to_blazon },
                        );
                        *sherwood_enable = SherwoodButtonEnable::post_commit();
                    }
                    Some((MissionChoice::ShowPendingMissions, _)) => {
                        // Swap pending missions into the accessible
                        // list and re-open the campaign map next
                        // frame.
                        dispatch_local_command(
                            host,
                            engine,
                            frame_cmds,
                            assets,
                            &PlayerCommand::CampaignSwapPendingToAccessibleMissions,
                        );
                        // See the `ShowCampaignMap` note elsewhere: only
                        // the active flag gets set here; the displayed
                        // flag flips when the overlay opens.
                        game.show_campaign_map();
                    }
                    Some((MissionChoice::None, _)) => {
                        // Cancel from the description dialog —
                        // restore the campaign-map overlay so the
                        // player can pick a different mission.
                        // Only the active flag is set here; the
                        // displayed flag flips when the overlay
                        // opens.
                        game.show_campaign_map();
                    }
                }
            }
            CampaignMapChoice::Quit => {
                // Escape / window close from the overlay with no
                // mission committed: exit Sherwood to the main menu.
                // We deliberately leave `campaign_map_displayed` set
                // so a save-on-exit would restore the overlay.
                restore_required_campaign(
                    campaign_ref,
                    engine.take_campaign(),
                    "campaign-map Quit",
                );
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
        // non-emergency-exit path), and re-queue the HUD info-bar
        // refresh so the script side picks up the newly-selected
        // mission's requirements/blazons.  The live Sherwood HUD's
        // mission-team refresh re-runs at the top of the next frame
        // via `handle_sherwood_buttons` — leaving a known
        // low-severity one-frame lag.
        if !redisplay_requested {
            game.persistent.campaign_map_displayed = false;
            engine.queue_update_information_bars();
        }
    }

    Ok(HandlerAction::Proceed)
}
