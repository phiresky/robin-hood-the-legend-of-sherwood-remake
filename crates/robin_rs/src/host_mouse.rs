//! Host-side mouse/cursor/aim state processing.
//!
//! Originally lived as `impl Engine` methods in `robin_engine::engine::input`
//! but these three functions are pure host UI computation — they read engine
//! state and write into `Host.input` / trajectory fields. Moved here during
//! the Host carve-out. Still takes `&mut Engine` for render-only refreshes
//! such as patch door highlights; sim-visible mouse commands are dispatched
//! earlier in the frame so replay / rollback can record them.

use crate::host::Host;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::coordinates::MapPoint;
use robin_engine::element as engine_element;
use robin_engine::element::Entity;
use robin_engine::element::Focus;
use robin_engine::engine::input::{
    BowTarget, MOUSE_BOW_CIVIL_COLOR, MOUSE_BOW_NO_COLOR, MOUSE_BOW_VIP_COLOR,
    MOUSE_OPACITY_DEFAULT, TrajectoryPreview,
};
use robin_engine::engine::{DevState, Engine, LevelAssets};
use robin_engine::fast_find_grid as engine_fast_find_grid;
use robin_engine::profiles as engine_profiles;
use robin_engine::profiles::Action;
use robin_engine::sector as engine_sector;
use robin_engine::weapons as engine_weapons;

/// Apply a `TrajectoryPreview` returned from engine to host's
/// trajectory-preview fields. See `TrajectoryPreview` docs.
fn apply_trajectory_preview(host: &mut Host, preview: TrajectoryPreview) {
    match preview {
        TrajectoryPreview::Invalid => {
            host.trajectory_preview_points.clear();
            host.valid_trajectory = false;
            host.net_crumpled = false;
        }
        TrajectoryPreview::HitNoArc => {
            host.valid_trajectory = true;
            host.trajectory_preview_points.clear();
            host.net_crumpled = false;
        }
        TrajectoryPreview::ShowArc {
            points,
            start,
            crumpled,
            layer,
        } => {
            host.valid_trajectory = true;
            host.trajectory_preview_points = points;
            host.trajectory_preview_start = start;
            host.trajectory_preview_layer = layer;
            host.net_crumpled = crumpled;
        }
    }
}

fn door_click_polygon_at(engine: &Engine, mouse_map: MapPoint) -> Option<u32> {
    engine.mission_script()?;
    engine
        .doors()
        .iter()
        .enumerate()
        .find(|(_, door)| door.is_door() && door.click_polygon_contains(mouse_map.x, mouse_map.y))
        .map(|(idx, _)| idx as u32)
}

//
// The consumer lives in `engine/tick.rs` (the message handler
// populates `host.pc_info_overlay`) and `ui_panel::draw_pc_info_overlay`
// (which reads the PC's sword/bow capacity from the campaign
// `HumanStatus` each frame and renders the pip overlay).
pub fn update_pc_popup_information(
    engine: &Engine,
    host: &mut Host,
    assets: &LevelAssets,
    mouse_map: MapPoint,
) {
    let drag_active = host.input.multi_selection_active || host.input.multi_unselection_active;

    // Both the show and hide message handlers early-out unless we're
    // in Sherwood — the popup is HQ-only.
    let focused_pc = if drag_active || !engine.is_sherwood(&assets.profile_manager) {
        None
    } else {
        engine.find_focusable_pc(assets, mouse_map, engine_element::Focus::Select)
    };

    // Host-side mouse hover writes directly into the host-owned
    // overlay.  The per-frame pip counts and position are recomputed
    // from engine state at render time (see
    // `ui_panel::draw_pc_info_overlay`), so we only need to update
    // the `visible` / `pc_id` pair here.
    match focused_pc {
        Some(pc_id) => {
            host.pc_info_overlay.visible = true;
            host.pc_info_overlay.pc_id = Some(pc_id);
        }
        None => host.pc_info_overlay.hide(),
    }
}

// ─── choose_mouse_pointer_for_no_action ───────────────────────────

/// Choose the mouse pointer when no action is selected.
pub fn choose_mouse_pointer_for_no_action(
    engine: &mut Engine,
    host: &mut Host,
    assets: &LevelAssets,
    mouse_map: MapPoint,
    shift_held: bool,
) -> i32 {
    use robin_engine::resource_ids::*;

    // Door hover UI enabled by default in this function.
    host.input.display_door = true;

    // Dragging while swordfighting → swordfight cursor.
    let local_seat = host.transport.local_seat;
    let selected = engine.seat_selection(local_seat);

    if host.input.left_mouse_down
        && crate::game_input::is_selected_unit_swordfighting(engine, local_seat)
    {
        return RHMOUSE_SWORDFIGHT_YES;
    }

    // Drawing multi-selection box → default.
    if host.input.draw_multi_selection {
        return RHMOUSE_DEFAULT;
    }

    let num_selected = selected.len();

    // Outside of grid → can't go there.
    if !engine.fast_grid().is_inside_grid_point(mouse_map) {
        return RHMOUSE_CANTGOTHERE;
    }

    // No PC selected — only selectable PCs.
    if num_selected == 0 {
        let focused =
            engine.find_focusable_entity(assets, &host.draw_order.ids, mouse_map, Focus::Select);
        if let Some(eid) = focused {
            host.input.focused_entity_id = focused;
            // With no PC selected, the only meaningful branch in the
            // PC mouse-focus dispatch returns RHMOUSE_DEFAULT (the
            // SHORT_LEG override needs a JUMP-capable selected PC,
            // which we don't have here).  Use the helper anyway so the
            // SELECT-branch dispatch lives in a single place.
            return engine.choose_select_cursor(assets, eid, None);
        }
        return RHMOUSE_DEFAULT;
    }

    // Iterate display order checking select/use/sword.
    let is_swordfighting = crate::game_input::is_selected_unit_swordfighting(engine, local_seat);
    let selected_pc = selected.first().copied();
    let recording_macro = engine.is_recording_macro();

    // Clone the id list (cheap) so the iteration doesn't hold an
    // immutable borrow of `host` while the loop body mutates
    // `host.input`.
    let draw_order_ids = host.draw_order.ids.clone();
    for &eid in draw_order_ids.iter().rev() {
        // Borrow entity for read-only checks, then drop the borrow
        // before mutating host.input.
        let (is_pc, is_human, select_ok, use_ok, sword_ok, interact_ok) = {
            let entity = match engine.get_entity(eid) {
                Some(e) => e,
                None => continue,
            };
            (
                entity.is_pc(),
                entity.is_human(),
                engine.is_entity_focusable(
                    assets,
                    eid,
                    entity,
                    mouse_map,
                    Focus::Select,
                    selected_pc,
                ),
                !is_swordfighting
                    && engine.is_entity_focusable(
                        assets,
                        eid,
                        entity,
                        mouse_map,
                        Focus::Use,
                        selected_pc,
                    ),
                engine.is_entity_focusable(
                    assets,
                    eid,
                    entity,
                    mouse_map,
                    Focus::Sword,
                    selected_pc,
                ),
                engine.is_entity_focusable(
                    assets,
                    eid,
                    entity,
                    mouse_map,
                    Focus::Interact,
                    selected_pc,
                ),
            )
        };

        // While recording a QA macro, hovering any focusable human
        // shows the generic INTERRACT cursor in place of the usual
        // select / use / sword paths so the player can chain
        // click-to-interact into the macro.
        //
        // The swordfight variant of this loop has only SELECT and
        // SWORD arms — no INTERACT branch — so the QA interact cursor
        // must not fire while swordfighting.  Gate on
        // `!is_swordfighting`.
        if recording_macro && !is_swordfighting && is_human && interact_ok {
            host.input.focused_entity_id = Some(eid);
            host.input.display_door = false;
            return if is_pc {
                RHMOUSE_INTERRACT_PC
            } else {
                RHMOUSE_INTERRACT_NPC
            };
        }

        // Unselected PC: for an alive, selectable PC the dispatch
        // returns RHMOUSE_DEFAULT (the regular arrow stays put —
        // left-click switches selection); the only override is
        // RHMOUSE_SHORT_LEG when the hovered PC is in the
        // HelpingToClimb posture and the selected PC has the Jump
        // contextual action.
        if select_ok && is_pc && !selected.contains(&eid) {
            host.input.focused_entity_id = Some(eid);
            host.input.display_door = false;
            return engine.choose_select_cursor(assets, eid, selected_pc);
        }

        // Contextual use — virtual NPC/Human dispatch returns
        // different cursors based on target state and selected PC's
        // abilities.
        if use_ok {
            let cursor = engine.choose_use_cursor(assets, eid, selected_pc);
            // The mouse-focus chain calls `Mark()` only on positive-
            // interaction branches.  The "no" cursors —
            // `RHMOUSE_PAY_NO` (beggar with insufficient ransom) and
            // `RHMOUSE_GET_NO` (heavy corpse, unpickupable object) —
            // return the cursor without marking, so the player sees
            // the "no" icon but no selection halo.  The actual outline
            // color is chosen by the renderer from this per-seat focus
            // flag; hover rendering must not mutate engine state.
            let cursor_marks = !matches!(cursor, RHMOUSE_PAY_NO | RHMOUSE_GET_NO);
            if cursor_marks {
                host.input.focused_entity_id = Some(eid);
            }
            host.input.display_door = false;
            return cursor;
        }

        // Sword-targetable enemy.  Suppress the sword cursor while
        // recording a macro so swordfight targeting doesn't hijack
        // the INTERRACT branch above.
        if sword_ok && !recording_macro {
            // The double-status bar latches on whenever a soldier
            // is hovered as a sword target, so the bar blinks on
            // under the cursor for one frame — both in the
            // VIP-not-Robin (CANTGOTHERE) branch and the regular
            // (SWORDFIGHT_YES) branch.  Set unconditionally.
            host.input.double_status_bar_entity_id = Some(eid);
            // Override the sword cursor with `RHMOUSE_CANTGOTHERE`
            // when the target is a VIP and the selected PC isn't
            // Robin — only Robin can fight VIPs.  Only the non-VIP
            // branch outlines the soldier; the CANTGOTHERE branch
            // returns without marking.
            let target_is_vip = engine
                .get_entity(eid)
                .is_some_and(|e| engine.is_entity_vip(assets, e));
            let selected_pc_is_robin = selected_pc
                .and_then(|id| engine.get_entity(id))
                .and_then(|e| e.pc_data())
                .is_some_and(|pc| pc.robin);
            if target_is_vip && !selected_pc_is_robin {
                return RHMOUSE_CANTGOTHERE;
            }
            // The per-seat focus flag drives the ShowSelection pass
            // directly so hover rendering stays outside engine state.
            host.input.focused_entity_id = Some(eid);
            host.input.display_door = false;
            return RHMOUSE_SWORDFIGHT_YES;
        }
    }

    // ── Sector-based cursor ──
    //
    // When PCs are selected and no entity was focused, check the sector
    // under the mouse for doors, lifts (climb/stairs), jumps, etc.
    let pc_id = selected[0]; // at least one selected (checked above)
    let (pc_layer, pc_pos) = engine
        .get_entity(pc_id)
        .map(|e| {
            let elem = e.element_data();
            (elem.layer(), elem.position_map())
        })
        .unwrap_or((0, mouse_map));

    // Look up sector under mouse.
    let mouse_sector_result = engine.fast_grid().get_sector_screen(mouse_map, pc_pos);
    let pc_sector_hit = engine.fast_grid().get_sector(pc_pos, pc_pos, pc_layer);

    if let Some(door_idx) = host.input.hovered_door_idx {
        host.input.increment_cursor_animation = false;
        host.input.selected_layer = mouse_sector_result.layer;
        host.valid_trajectory = false;
        return engine.choose_door_cursor(Some(door_idx), None);
    }

    // If the mouse is over a patch overlay sector, resolve the owning
    // patch and route to `choose_door_cursor`. The patch's first door
    // (if any) picks between door/lockpick variants; a door-less patch
    // falls through to the patch-lock fallback inside
    // `choose_door_cursor`.
    if let Some(patch_sector_idx) = mouse_sector_result.sector_idx {
        let is_patch = engine
            .fast_grid()
            .level
            .sectors
            .get(usize::from(patch_sector_idx))
            .map(|s| s.sector_type.is_patch())
            .unwrap_or(false);
        // `find_patch_for_grid_sector` returns `None` only when no
        // mission script is loaded; in that state we can't evaluate
        // patch doors and fall through to the default cursor logic.
        if is_patch && let Some(patch_idx) = engine.find_patch_for_grid_sector(patch_sector_idx) {
            let first_door = engine
                .mission_script()
                .and_then(|_| engine.patches().get(patch_idx as usize))
                .and_then(|p| p.door_indices.first().copied());
            // Door-cursor pointer freezes the cursor animation.
            host.input.increment_cursor_animation = false;
            host.input.selected_layer = mouse_sector_result.layer;
            return engine.choose_door_cursor(first_door, Some(patch_idx));
        }
    }

    // If mouse sector != PC's sector.
    let mouse_sector_idx = mouse_sector_result.sector_idx;
    let pc_sector_idx = match pc_sector_hit {
        engine_fast_find_grid::SectorHit::Found { sector_idx, .. } => Some(sector_idx),
        _ => None,
    };

    if mouse_sector_idx != pc_sector_idx {
        // Allow the jump-sector branch below to swap `idx` to its
        // underlying motion-area sector; cap at a couple of hops so a
        // misconfigured proto can't spin forever.
        let mut idx_opt = mouse_sector_idx;
        let mut loops = 0;
        while let Some(idx) = idx_opt {
            loops += 1;
            if loops > 4 {
                break;
            }
            if let Some(sector) = engine.fast_grid().level.sectors.get(usize::from(idx)) {
                let st = sector.sector_type;

                // Motion area sector.
                if st.is_motion() && st.is_area() {
                    // Reset trajectory for motion area navigation.
                    host.valid_trajectory = false;
                    if st.is_lift()
                        && let Some(lt) = sector.lift_type
                    {
                        match lt {
                            // Wall → climbing cursor (if PC can climb).
                            engine_sector::LiftType::Wall => {
                                // Gated on every selected PC
                                // having the contextual Climb
                                // action.  Without it the cursor
                                // falls through to CANTGOTHERE
                                // (or shift-variant).
                                if engine.all_selected_pcs_can_climb(assets) {
                                    return RHMOUSE_CLIMBING;
                                } else {
                                    return if shift_held {
                                        RHMOUSE_CANTGOTHERE_OUTLINE
                                    } else {
                                        RHMOUSE_CANTGOTHERE
                                    };
                                }
                            }
                            // Stairs → default (intentional bug,
                            // see "STAIRS CURSOR BUG" in the
                            // original game).
                            engine_sector::LiftType::Stairs => {
                                return if is_swordfighting {
                                    if shift_held {
                                        RHMOUSE_DEFAULT_OUTLINE
                                    } else {
                                        RHMOUSE_SWORDFIGHT_YES
                                    }
                                } else if shift_held {
                                    RHMOUSE_DEFAULT_OUTLINE
                                } else {
                                    RHMOUSE_DEFAULT
                                };
                            }
                            // Other lifts → climbing.
                            _ => {
                                return RHMOUSE_CLIMBING;
                            }
                        }
                    }
                    // If either source or target motion area has 0
                    // gates, the two areas can't possibly be connected
                    // by a door → show can't-go-there cursor. Lift
                    // sectors are handled above because wall/ladder
                    // traversal is authorized by lift type, not by
                    // door-gate adjacency.
                    let target_gates = sector.gate_indices.len();
                    let source_gates = pc_sector_idx
                        .and_then(|i| engine.fast_grid().level.sectors.get(usize::from(i)))
                        .map(|s| s.gate_indices.len())
                        .unwrap_or(0);
                    if target_gates == 0 || source_gates == 0 {
                        return if shift_held {
                            RHMOUSE_CANTGOTHERE_OUTLINE
                        } else {
                            RHMOUSE_CANTGOTHERE
                        };
                    }
                    // Non-lift motion area: normal traversal
                    return if is_swordfighting {
                        if shift_held {
                            RHMOUSE_DEFAULT_OUTLINE
                        } else {
                            RHMOUSE_SWORDFIGHT_YES
                        }
                    } else if shift_held {
                        RHMOUSE_DEFAULT_OUTLINE
                    } else {
                        RHMOUSE_DEFAULT
                    };
                }

                // Door sector.
                if st.is_door() {
                    // Update selected layer for door.
                    host.input.selected_layer = mouse_sector_result.layer;
                    // Door-cursor pointer freezes the cursor animation.
                    host.input.increment_cursor_animation = false;
                    host.valid_trajectory = false;
                    // Snapshot door index before further borrows.
                    let door_idx = sector.door_index;
                    return engine.choose_door_cursor(door_idx, None);
                }

                // Jump sector.
                if st.contains(engine_sector::SectorType::JUMP) {
                    host.valid_trajectory = false;
                    // Walk selected PCs, find the first with the
                    // Jump action, then return the nearest
                    // *reachable* jump line for that PC (or null,
                    // falling through to swordfight/recurse).
                    //
                    // A non-negative height delta selects JUMP_HIGH,
                    // else JUMP_LOW.  The height is derived from the
                    // paired jump-line elevation delta (see
                    // engine/jump.rs).  Return the first selected
                    // PC's result unconditionally — including null —
                    // rather than searching later PCs for a
                    // reachable line.  Use a plain early-return loop
                    // instead of `find_map`.
                    let mut jump_line_idx: Option<u32> = None;
                    let mut jumper_on_shoulders = false;
                    for &pc_id in engine.seat_selection(host.transport.local_seat) {
                        if !engine.selected_pc_has_contextual_action(
                            assets,
                            Some(pc_id),
                            engine_profiles::Action::Jump,
                        ) {
                            continue;
                        }
                        let Some(entity) = engine.get_entity(pc_id) else {
                            tracing::warn!(?pc_id, "jump hover: selected PC is missing");
                            continue;
                        };
                        jumper_on_shoulders =
                            entity.element_data().posture == engine_element::Posture::OnShoulders;
                        let pc_pos_map = entity.element_data().position_map();
                        jump_line_idx = engine.get_nearest_jumpable_jump_line(
                            pc_id,
                            u32::from(idx),
                            pc_pos_map,
                            mouse_map,
                            /* test_posture */ false,
                            None,
                        );
                        break;
                    }

                    let mut height: Option<f32> = None;
                    if let Some(line_idx) = jump_line_idx
                        && let Some(line) =
                            engine.fast_grid().level.jump_lines.get(line_idx as usize)
                        && let Some(assoc_idx) = line.associated_line_index
                        && let Some(dst) =
                            engine.fast_grid().level.jump_lines.get(assoc_idx as usize)
                    {
                        height = Some(dst.z_a - line.z_a);
                        // Jump-line midpoint ghost titbit when the
                        // associated line needs a helper (and PC isn't
                        // already on someone's shoulders).
                        if dst.helper_needed && !jumper_on_shoulders && dst.z_a - line.z_a >= 0.0 {
                            let mid_x = 0.5 * (line.point_a.x + line.point_b.x);
                            let mid_y =
                                0.5 * (line.point_a.y + line.point_b.y + line.z_a + line.z_b);
                            let mid_z = 0.5 * (line.z_a + line.z_b);
                            let dx = line.point_b.x - line.point_a.x;
                            let dy = line.point_b.y - line.point_a.y;
                            // Normal direction as sector0..15 via atan2
                            // of the perpendicular.
                            let angle = dy.atan2(dx) + std::f32::consts::FRAC_PI_2;
                            let sector_dir = (angle / (2.0 * std::f32::consts::PI) * 16.0)
                                .rem_euclid(16.0)
                                as u16;
                            let position = engine_coordinates::WorldPoint3D {
                                x: mid_x,
                                y: mid_y,
                                z: mid_z,
                            };
                            host.host_titbit_preview =
                                Some(crate::host::HostTitbitPreview::JumpHelperGhost {
                                    position,
                                    layer: line.layer,
                                    sector_dir,
                                    display_order: position.y + 0.01,
                                });
                        }
                    }
                    match height {
                        Some(h) => {
                            // Compute the jump-arc ghost once the
                            // mouse has stabilised so the player sees
                            // the path Robin will take over the jump
                            // sector.
                            const TIME_TRAJECTORY_DISPLAY: u32 = 1;
                            if host.time_no_mouse_move > TIME_TRAJECTORY_DISPLAY
                                && !host.valid_trajectory
                                && let Some(line_idx) = jump_line_idx
                            {
                                let preview = engine.compute_jump_preview(line_idx);
                                apply_trajectory_preview(host, preview);
                            }
                            return if h >= 0.0 {
                                RHMOUSE_JUMP_HIGH
                            } else {
                                RHMOUSE_JUMP_LOW
                            };
                        }
                        None => {
                            // No valid jump line.  Swordfighting PCs
                            // get the combat-allowed cursor;
                            // otherwise recurse with the underlying
                            // motion sector the jump polygon overlays.
                            if is_swordfighting {
                                return if shift_held {
                                    RHMOUSE_DEFAULT_OUTLINE
                                } else {
                                    RHMOUSE_SWORDFIGHT_YES
                                };
                            }
                            // The selected sector is mutated in place
                            // before recursing, so any later host
                            // state that reads the selected sector
                            // sees the underlying motion area, not
                            // the overlaying jump polygon.  Overwrite
                            // `host.input.selected_sector_idx` before
                            // the next iteration picks up.
                            idx_opt = sector.underlying_sector;
                            host.input.selected_sector_idx = idx_opt;
                            continue;
                        }
                    }
                }

                // Fell through all sector-type branches with no
                // specific cursor — break out of the fallback loop
                // and drop into the same-sector / can't-go logic.
                break;
            } else {
                // No sector data → can't go there.
                return if shift_held {
                    RHMOUSE_CANTGOTHERE_OUTLINE
                } else {
                    RHMOUSE_CANTGOTHERE
                };
            }
        }
        // Loop exited via `break` (no type branch matched) or by
        // exhausting the fallback chain — fall through to the
        // can't-go-there default.
        if mouse_sector_idx.is_none() {
            // Null sector → can't go there.
            return if shift_held {
                RHMOUSE_CANTGOTHERE_OUTLINE
            } else {
                RHMOUSE_CANTGOTHERE
            };
        }
    }

    // Same sector as PC.
    if shift_held {
        return RHMOUSE_DEFAULT_OUTLINE;
    }

    if is_swordfighting {
        RHMOUSE_SWORDFIGHT_YES
    } else {
        RHMOUSE_DEFAULT
    }
}

/// Compute the mouse cursor ID for the current frame.
///
/// Called each frame with mouse position in map space and modifier
/// key state.  Also sets `host.input.mouse_opacity`,
/// `host.input.mouse_shadow_color`, and
/// `host.input.double_status_bar_entity_id`.
pub fn update_mouse(
    engine: &mut Engine,
    host: &mut Host,
    assets: &LevelAssets,
    dev: &DevState,
    external_actions: &mut Vec<robin_engine::engine::ExternalAction>,
    mouse_map: MapPoint,
    alt_held: bool,
    shift_held: bool,
) -> i32 {
    use robin_engine::resource_ids::*;

    host.host_titbit_preview = None;

    let cursor_action = if shift_held {
        engine.planned_action_for_seat(host.transport.local_seat)
    } else {
        engine.selected_action_for_seat(host.transport.local_seat)
    };
    if host.trajectory_preview_shift_held != shift_held
        || host.trajectory_preview_action != cursor_action
    {
        host.valid_trajectory = false;
        host.trajectory_preview_points.clear();
        host.time_no_mouse_move = 0;
        host.trajectory_preview_shift_held = shift_held;
        host.trajectory_preview_action = cursor_action;
    }

    // Track mouse-no-move timer.  Used to delay trajectory preview
    // display.
    if mouse_map == host.mouse_map_prev {
        host.time_no_mouse_move = host.time_no_mouse_move.saturating_add(1);
    } else {
        host.time_no_mouse_move = 0;
        host.valid_trajectory = false;
        host.trajectory_preview_points.clear();
        host.mouse_map_prev = mouse_map;
    }

    // Per-frame clear of the UI-focus latch — the messenger resets
    // the flag every frame so it only stays true for the frame a
    // widget raised `MSG_UI_HAS_FOCUS`.  The flag is observable in
    // the window between `apply_side_effects` (which OR's it into
    // `host.ui_focus`) and this reset — i.e. during the tick itself,
    // script command handlers, and anything reading host state before
    // `update_mouse`.  The sole reader (the init-zoom display path)
    // is currently unported, so this is dead state today but
    // preserves the cadence.
    host.ui_focus = false;

    // When the cursor hasn't been stable long enough to show a
    // preview, wipe any stale `valid_trajectory` flag from the prior
    // frame before any per-action branch recomputes.
    const TIME_TRAJECTORY_DISPLAY: u32 = 1;
    if host.time_no_mouse_move <= TIME_TRAJECTORY_DISPLAY {
        host.valid_trajectory = false;
        host.trajectory_preview_points.clear();
    }

    // Once-every-10-frames ground-mark drop at the projected impact
    // point.  Run off the previously-applied preview before per-
    // action handlers below refresh it, so the mark follows the shown
    // arc's current destination.  In-flight projectiles deliberately
    // do *not* drop ground marks: the projectile trajectory draw
    // path renders arc dots without touching the ground mark, so the
    // preview-side drop here is the complete parity set.
    if host.valid_trajectory {
        host.trajectory_mark_count = host.trajectory_mark_count.wrapping_add(1);
        if host.trajectory_mark_count.is_multiple_of(10)
            && let Some(dest) = host.trajectory_preview_points.last()
        {
            let layer = host.trajectory_preview_layer;
            let mark = dest.position.to_map();
            host.trajectory_ground_mark.add_mark(mark.x, mark.y, layer);
        }
    } else {
        host.trajectory_mark_count = 0;
    }

    // Clear per-frame focus state.
    host.input.focused_entity_id = None;
    host.input.double_status_bar_entity_id = None;
    host.input.mouse_opacity = MOUSE_OPACITY_DEFAULT;
    host.input.mouse_shadow_color = 0;
    host.input.increment_cursor_animation = true;
    host.input.display_door = false; // set true in choose_mouse_pointer_for_no_action

    let mouse_map_pt = mouse_map;

    // Sector lookup for the selected sector / layer.  Used for door/
    // jump alpha overlays and cursor context.  With shift held, use
    // the "peek under" helper that returns the sector under the
    // topmost hit instead of the regular screen lookup.
    //
    // The reference point is seeded from the first selected PC's map
    // position (falling back to the cursor when no PC is selected).
    // The reference is used by `get_sector` to tie-break overlapping
    // jump sectors (nearest-mid wins).
    let reference = engine
        .seat_selection(host.transport.local_seat)
        .first()
        .and_then(|&id| engine.get_entity(id))
        .map(|e| {
            let p = e.element_data().position_map();
            engine_coordinates::MapPoint::new(p.x, p.y)
        })
        .unwrap_or(mouse_map_pt);
    let sector_hit = if shift_held {
        engine
            .fast_grid()
            .get_sector_screen_hidden(mouse_map_pt, reference)
    } else {
        engine
            .fast_grid()
            .get_sector_screen(mouse_map_pt, reference)
    };

    // Resolve the owning patch once per frame.  The selected patch is
    // reused in cursor + render paths; caching avoids the O(patches)
    // scan that `find_patch_for_grid_sector` performs each cursor
    // call.
    //
    // When the cursor lands on a patch overlay, replace the selected
    // sector and layer with the patch's underlying sector + layer so
    // downstream door/building logic sees the real geometry.
    let (final_sector_idx, final_layer, selected_patch_idx) =
        if let Some(idx) = sector_hit.sector_idx {
            let (is_patch, patch_sector_idx, patch_layer) = engine
                .fast_grid()
                .level
                .sectors
                .get(usize::from(idx))
                .map(|s| (s.sector_type.is_patch(), s.underlying_sector, s.layer))
                .unwrap_or((false, None, 0));
            if is_patch {
                let patch_idx = engine.find_patch_for_grid_sector(idx);
                let (under_idx, under_layer) = patch_sector_idx
                    .map(|u| (Some(u), patch_layer))
                    .unwrap_or((sector_hit.sector_idx, sector_hit.layer));
                (under_idx, under_layer, patch_idx)
            } else {
                (sector_hit.sector_idx, sector_hit.layer, None)
            }
        } else {
            (None, sector_hit.layer, None)
        };
    host.input.selected_map_point = mouse_map_pt;
    host.input.selected_sector_idx = final_sector_idx;
    host.input.selected_layer = final_layer;
    host.input.selected_patch_idx = selected_patch_idx;
    host.input.hovered_door_idx = door_click_polygon_at(engine, mouse_map_pt);

    // `valid_position_for_move` is true when the hovered patch is
    // set, or the selected sector is a motion-area / door / jump
    // sector.  Gate move-command dispatch on this.
    host.input.valid_position_for_move = host.input.selected_patch_idx.is_some()
        || host.input.hovered_door_idx.is_some()
        || sector_hit.sector_idx.is_some_and(|idx| {
            engine
                .fast_grid()
                .level
                .sectors
                .get(usize::from(idx))
                .is_some_and(|s| {
                    let st = s.sector_type;
                    (st.is_motion() && st.is_area())
                        || st.is_door()
                        || st.contains(engine_sector::SectorType::JUMP)
                })
        });

    // Alt → view cursor.
    if alt_held {
        let focus_id = engine
            .find_focusable_npc(assets, mouse_map_pt, Focus::View)
            .or_else(|| {
                if dev.debug.pc_sight {
                    engine.find_focusable_pc(assets, mouse_map_pt, Focus::Select)
                } else {
                    None
                }
            });
        if let Some(id) = focus_id {
            host.input.focused_entity_id = Some(id);
            // EZEKIEL_2517 cheat swallows the gesture and instakills
            // the target instead of highlighting its vision cone.
            // Non-cheat path is pure host UI state —
            // `selected_view_element` lives on Host, not Engine, so
            // the rollback hash doesn't see it.
            if engine.can_ezekiel_instakill(id) {
                external_actions
                    .push(robin_engine::engine::ExternalAction::EzekielInstakill { target: id });
            } else {
                host.selected_view_element = Some(id);
            }
        }
        return RHMOUSE_VIEW;
    }

    // Locker (follow-cam) mode.  When the messenger's locker flag is
    // set, hovering an NPC lets the player pick the follow target;
    // clicking snaps the camera to track it.
    if engine.view_locked() {
        if let Some(id) = engine.find_focusable_npc(assets, mouse_map_pt, Focus::View) {
            host.input.focused_entity_id = Some(id);
        }
        return RHMOUSE_VIEW;
    }

    let selected_action = cursor_action;

    match selected_action {
        // ── NoAction ───────────────
        Action::NoAction => {
            let cursor =
                choose_mouse_pointer_for_no_action(engine, host, assets, mouse_map_pt, shift_held);
            // Dispatches MSG_SHOW_PC_INFORMATION /
            // MSG_HIDE_PC_INFORMATION based on whether the mouse is
            // over a selectable PC.
            update_pc_popup_information(engine, host, assets, mouse_map_pt);
            cursor
        }
        Action::Bow => cursor_for_bow(engine, host, assets, mouse_map_pt, shift_held),
        Action::Hit | Action::HitHard => {
            cursor_for_hit(engine, host, assets, mouse_map_pt, shift_held)
        }
        Action::Apple => cursor_for_apple(engine, host, assets, mouse_map_pt, shift_held),
        Action::Stone => cursor_for_stone(engine, host, assets, mouse_map_pt, shift_held),
        Action::Purse => cursor_for_purse(engine, host, assets, mouse_map_pt, shift_held),
        Action::Heal => cursor_for_heal(engine, host, assets, mouse_map_pt, shift_held),
        Action::WaspNest => cursor_for_wasp_nest(engine, host, assets, mouse_map_pt, shift_held),
        Action::HelpToClimb => {
            cursor_for_help_to_climb(engine, host, assets, mouse_map_pt, shift_held)
        }
        // Self-targeted consumables / whistle always show the OK cursor.
        Action::Eat | Action::Guzzle | Action::Whistle => RHMOUSE_OK,
        Action::Shield | Action::BigShield => {
            cursor_for_shield(engine, host, assets, mouse_map_pt, shift_held)
        }
        Action::Net => cursor_for_net(engine, host, assets, mouse_map_pt, shift_held),
        Action::Lever => cursor_for_lever(engine, host, assets, mouse_map_pt, shift_held),
        Action::Ale => cursor_for_ale(engine, host, assets, mouse_map_pt, shift_held),
        Action::Strangle => cursor_for_strangle(engine, host, assets, mouse_map_pt, shift_held),
        Action::Beggar => cursor_for_beggar(engine, host, assets, mouse_map_pt, shift_held),
        Action::Listen => cursor_for_listen(engine, host, assets, mouse_map_pt, shift_held),
        // Remaining actions.
        _ => RHMOUSE_DEFAULT,
    }
}

// ─── Per-action cursor arms ─────────────────────────────────────────
//
// One function per `update_mouse` action arm. All share the same
// signature: read engine/host/assets and the mouse-map point, mutate
// `host.input` focus/opacity/trajectory fields, and return the cursor
// resource id.

/// Bow cursor arm.
fn cursor_for_bow(
    engine: &mut Engine,
    host: &mut Host,
    assets: &LevelAssets,
    mouse_map_pt: MapPoint,
    shift_held: bool,
) -> i32 {
    use robin_engine::element::Camp;
    use robin_engine::resource_ids::*;
    {
        if engine.seat_selection(host.transport.local_seat).is_empty() {
            return RHMOUSE_BOW_NO;
        }
        let mut cursor = RHMOUSE_BOW_NO;
        let pc_id = engine.seat_selection(host.transport.local_seat)[0];

        // Shift planning previews the shot from the end of the actor's live
        // movement / queued movement chain. It must not require the live bow
        // equip/aim state, because selecting the planned action deliberately
        // leaves the real PC untouched.
        if shift_held {
            host.input.mouse_opacity = 0;
            host.input.mouse_shadow_color = 0;
            if let Some(target_id) =
                engine.find_focusable_entity(assets, &host.draw_order.ids, mouse_map_pt, Focus::Bow)
            {
                host.input.focused_entity_id = Some(target_id);
                const TIME_TRAJECTORY_DISPLAY: u32 = 1;
                if host.time_no_mouse_move > TIME_TRAJECTORY_DISPLAY && !host.valid_trajectory {
                    apply_trajectory_preview(
                        host,
                        engine.compute_planned_bow_trajectory_preview(assets, pc_id, target_id),
                    );
                }
                return RHMOUSE_BOW_YES_LONG;
            }
            host.valid_trajectory = false;
            return RHMOUSE_BOW_NO;
        }

        // When recording a macro, take the shorter path — no
        // range/trajectory checks, just BOW_YES over any
        // focusable element, BOW_NO otherwise.  Opacity/shadow
        // are cleared.
        if engine.is_recording_macro() {
            host.valid_trajectory = false;
            host.input.mouse_opacity = 0;
            host.input.mouse_shadow_color = 0;
            if let Some(target_id) =
                engine.find_focusable_entity(assets, &host.draw_order.ids, mouse_map_pt, Focus::Bow)
            {
                host.input.focused_entity_id = Some(target_id);
                cursor = RHMOUSE_BOW_YES;
            }
            return cursor;
        }

        // Check if PC is in building or wall/ladder lift.
        let in_restricted = engine.is_selected_pc_in_restricted_sector();

        // Opacity/shadow are set per-branch; declare without initializer
        // so clippy doesn't warn about overwritten values.
        let mut opacity: u16;
        let mut shadow_color: u16;

        if !in_restricted {
            // Compute mouse opacity from shooting level.
            opacity = engine
                .calculate_shooting_level(assets, pc_id, mouse_map_pt)
                .max(MOUSE_OPACITY_DEFAULT);
            shadow_color = 0;

            if let Some(target_id) =
                engine.find_focusable_entity(assets, &host.draw_order.ids, mouse_map_pt, Focus::Bow)
            {
                host.input.focused_entity_id = Some(target_id);

                // Get shoot type and bow target.
                let (bow_target, shoot_mode) =
                    engine.can_shoot_with_bow_at(assets, pc_id, target_id);
                let is_long = shoot_mode == engine_weapons::ShootMode::Long;

                // Extract entity data before mutating host.input.
                let target_info = engine.get_entity(target_id).map(|target| {
                    let camp = match target {
                        Entity::Soldier(s) => s.soldier.cached_camp,
                        Entity::Civilian(_) => Camp::Lacklandists,
                        _ => Camp::Error,
                    };
                    let is_npc = target.is_npc();
                    let is_civilian = target.is_civilian();
                    let is_fx_target = target.kind().is_fx_target();
                    let is_vip = engine.is_entity_vip(assets, target);
                    (camp, is_npc, is_civilian, is_fx_target, is_vip)
                });

                if bow_target == BowTarget::Valid {
                    if let Some((camp, is_npc, is_civilian, is_fx_target, is_vip)) = target_info {
                        if is_npc {
                            if is_civilian {
                                cursor = if is_long {
                                    RHMOUSE_BOW_CIVILIAN_LONG
                                } else {
                                    RHMOUSE_BOW_CIVIL
                                };
                                opacity = 50;
                                shadow_color = MOUSE_BOW_CIVIL_COLOR;
                            } else if is_vip {
                                cursor = if is_long {
                                    RHMOUSE_BOW_VIP_LONG
                                } else {
                                    RHMOUSE_BOW_VIP
                                };
                                opacity = 50;
                                shadow_color = MOUSE_BOW_VIP_COLOR;
                            } else if camp != Camp::Royalists {
                                // Enemy target.
                                cursor = if is_long {
                                    RHMOUSE_BOW_YES_LONG
                                } else {
                                    RHMOUSE_BOW_YES
                                };
                            }
                            // same camp → stays BOW_NO

                            if cursor != RHMOUSE_BOW_NO {
                                host.input.double_status_bar_entity_id = Some(target_id);
                            }
                        } else if is_fx_target {
                            cursor = if is_long {
                                RHMOUSE_BOW_YES_LONG
                            } else {
                                RHMOUSE_BOW_YES
                            };
                        }
                    }

                    // Compute trajectory preview for long shots.
                    // Only computes after TIME_TRAJECTORY_DISPLAY
                    // (1) frames of mouse stillness, and only
                    // once per hover (`valid_trajectory` guards
                    // re-computation).
                    const TIME_TRAJECTORY_DISPLAY: u32 = 1;
                    if host.time_no_mouse_move > TIME_TRAJECTORY_DISPLAY && !host.valid_trajectory {
                        apply_trajectory_preview(
                            host,
                            engine.compute_trajectory_preview(assets, pc_id, target_id, shoot_mode),
                        );
                    }
                } else {
                    host.valid_trajectory = false;
                }

                // Out of range overrides everything.
                if bow_target == BowTarget::OutOfRange {
                    cursor = RHMOUSE_BOW_OUT;
                    opacity = 50;
                    shadow_color = MOUSE_BOW_NO_COLOR;
                }
            } else {
                host.valid_trajectory = false;
            }
        } else {
            // In building/wall-ladder: no valid bow shot
            opacity = 50;
            shadow_color = MOUSE_BOW_NO_COLOR;
            host.valid_trajectory = false;
        }

        host.input.mouse_opacity = opacity;
        host.input.mouse_shadow_color = shadow_color;
        cursor
    }
}

/// Hit / HitHard cursor arm.
fn cursor_for_hit(
    engine: &mut Engine,
    host: &mut Host,
    assets: &LevelAssets,
    mouse_map_pt: MapPoint,
    _shift_held: bool,
) -> i32 {
    use robin_engine::resource_ids::*;
    {
        let focused =
            engine.find_focusable_entity(assets, &host.draw_order.ids, mouse_map_pt, Focus::Hit);
        if let Some(eid) = focused {
            host.input.focused_entity_id = Some(eid);
            RHMOUSE_HIT_YES
        } else {
            RHMOUSE_HIT_NO
        }
    }
}

/// Apple cursor arm.
fn cursor_for_apple(
    engine: &mut Engine,
    host: &mut Host,
    assets: &LevelAssets,
    mouse_map_pt: MapPoint,
    shift_held: bool,
) -> i32 {
    use robin_engine::resource_ids::*;
    {
        let mut cursor = RHMOUSE_APPLE_NO;
        let pc_id = engine
            .seat_selection(host.transport.local_seat)
            .first()
            .copied();

        if shift_held || !engine.is_selected_pc_in_restricted_sector() {
            if let Some(target_id) = engine.find_focusable_entity(
                assets,
                &host.draw_order.ids,
                mouse_map_pt,
                Focus::Apple,
            ) {
                // Range check.
                let target_pos = engine
                    .get_entity(target_id)
                    .map(|t| t.element_data().position_map());
                let in_range = shift_held
                    || pc_id.zip(target_pos).is_some_and(|(pid, tpos)| {
                        engine.is_in_range_for_projectile(
                            assets,
                            pid,
                            tpos,
                            Action::Apple,
                            Some(target_id),
                        )
                    });

                if in_range {
                    host.input.focused_entity_id = Some(target_id);
                    cursor = RHMOUSE_APPLE_YES;

                    // Compute the trajectory preview after a brief
                    // hover stillness, mirroring the bow branch.
                    // The arc is drawn only when the throw will
                    // miss the target — see
                    // `compute_trajectory_preview`.
                    const TIME_TRAJECTORY_DISPLAY: u32 = 1;
                    if host.time_no_mouse_move > TIME_TRAJECTORY_DISPLAY
                        && !host.valid_trajectory
                        && let Some(pid) = pc_id
                    {
                        let preview = if shift_held {
                            engine.compute_planned_trajectory_preview(
                                assets,
                                pid,
                                target_id,
                                Action::Apple,
                            )
                        } else {
                            engine.compute_trajectory_preview(
                                assets,
                                pid,
                                target_id,
                                engine_weapons::ShootMode::Long,
                            )
                        };
                        apply_trajectory_preview(host, preview);
                    }
                } else {
                    host.valid_trajectory = false;
                }
            } else {
                host.valid_trajectory = false;
            }
        } else {
            host.valid_trajectory = false;
        }
        cursor
    }
}

/// Stone cursor arm.
fn cursor_for_stone(
    engine: &mut Engine,
    host: &mut Host,
    assets: &LevelAssets,
    mouse_map_pt: MapPoint,
    shift_held: bool,
) -> i32 {
    use robin_engine::resource_ids::*;
    {
        let mut cursor = RHMOUSE_STONE_NO;
        let pc_id = engine
            .seat_selection(host.transport.local_seat)
            .first()
            .copied();

        if shift_held || !engine.is_selected_pc_in_restricted_sector() {
            if let Some(target_id) = engine.find_focusable_entity(
                assets,
                &host.draw_order.ids,
                mouse_map_pt,
                Focus::Stone,
            ) {
                let target_pos = engine
                    .get_entity(target_id)
                    .map(|t| t.element_data().position_map());
                let in_range = shift_held
                    || pc_id.zip(target_pos).is_some_and(|(pid, tpos)| {
                        engine.is_in_range_for_projectile(
                            assets,
                            pid,
                            tpos,
                            Action::Stone,
                            Some(target_id),
                        )
                    });

                if in_range {
                    host.input.focused_entity_id = Some(target_id);
                    let is_npc = engine
                        .get_entity(target_id)
                        .map(|t| t.is_npc())
                        .unwrap_or(false);
                    // Gate the double-status bar latch on
                    // `!is_recording_macro` (the
                    // recording-macro branch suppresses the
                    // double-status overlay).
                    if is_npc && !engine.is_recording_macro() {
                        host.input.double_status_bar_entity_id = Some(target_id);
                    }
                    cursor = RHMOUSE_STONE_YES;

                    const TIME_TRAJECTORY_DISPLAY: u32 = 1;
                    if host.time_no_mouse_move > TIME_TRAJECTORY_DISPLAY
                        && !host.valid_trajectory
                        && let Some(pid) = pc_id
                    {
                        let preview = if shift_held {
                            engine.compute_planned_trajectory_preview(
                                assets,
                                pid,
                                target_id,
                                Action::Stone,
                            )
                        } else {
                            engine.compute_trajectory_preview(
                                assets,
                                pid,
                                target_id,
                                engine_weapons::ShootMode::Long,
                            )
                        };
                        apply_trajectory_preview(host, preview);
                    }
                } else {
                    host.valid_trajectory = false;
                }
            } else {
                host.valid_trajectory = false;
            }
        } else {
            host.valid_trajectory = false;
        }
        cursor
    }
}

/// Purse cursor arm.
fn cursor_for_purse(
    engine: &mut Engine,
    host: &mut Host,
    assets: &LevelAssets,
    mouse_map_pt: MapPoint,
    shift_held: bool,
) -> i32 {
    use robin_engine::resource_ids::*;
    {
        let mut cursor = RHMOUSE_PURSE_NO;
        let mouse_elem = mouse_map_pt;
        let pc_id = engine
            .seat_selection(host.transport.local_seat)
            .first()
            .copied();

        if (shift_held || !engine.is_selected_pc_in_restricted_sector())
            && engine.is_mouse_sector_valid_for_ground_target(mouse_map_pt)
        {
            let in_range = shift_held
                || pc_id.is_some_and(|pid| {
                    engine.is_in_range_for_projectile(assets, pid, mouse_elem, Action::Purse, None)
                });
            if in_range {
                cursor = RHMOUSE_PURSE_YES;

                // Trajectory preview for ground throws.
                const TIME_TRAJECTORY_DISPLAY: u32 = 1;
                if host.time_no_mouse_move > TIME_TRAJECTORY_DISPLAY
                    && !host.valid_trajectory
                    && let Some(pid) = pc_id
                {
                    let preview = if shift_held {
                        engine.compute_planned_trajectory_preview_ground(
                            assets,
                            pid,
                            mouse_elem,
                            Action::Purse,
                        )
                    } else {
                        engine.compute_trajectory_preview_ground(assets, pid, mouse_elem)
                    };
                    apply_trajectory_preview(host, preview);
                }
            } else {
                host.valid_trajectory = false;
            }
        } else {
            host.valid_trajectory = false;
        }
        cursor
    }
}

/// Heal cursor arm.
fn cursor_for_heal(
    engine: &mut Engine,
    host: &mut Host,
    assets: &LevelAssets,
    mouse_map_pt: MapPoint,
    _shift_held: bool,
) -> i32 {
    use robin_engine::resource_ids::*;
    {
        let focused =
            engine.find_focusable_entity(assets, &host.draw_order.ids, mouse_map_pt, Focus::Heal);
        if let Some(eid) = focused {
            host.input.focused_entity_id = Some(eid);
            RHMOUSE_HEAL_YES
        } else {
            RHMOUSE_HEAL_NO
        }
    }
}

/// WaspNest cursor arm.
fn cursor_for_wasp_nest(
    engine: &mut Engine,
    host: &mut Host,
    assets: &LevelAssets,
    mouse_map_pt: MapPoint,
    shift_held: bool,
) -> i32 {
    use robin_engine::resource_ids::*;
    {
        let mut cursor = RHMOUSE_WASP_NEST_NO;
        let mouse_elem = mouse_map_pt;
        let pc_id = engine
            .seat_selection(host.transport.local_seat)
            .first()
            .copied();

        if shift_held || !engine.is_selected_pc_in_restricted_sector() {
            let in_range = shift_held
                || pc_id.is_some_and(|pid| {
                    engine.is_in_range_for_projectile(
                        assets,
                        pid,
                        mouse_elem,
                        Action::WaspNest,
                        None,
                    )
                });
            if in_range {
                cursor = RHMOUSE_WASP_NEST_YES;

                const TIME_TRAJECTORY_DISPLAY: u32 = 1;
                if host.time_no_mouse_move > TIME_TRAJECTORY_DISPLAY
                    && !host.valid_trajectory
                    && let Some(pid) = pc_id
                {
                    let preview = if shift_held {
                        engine.compute_planned_trajectory_preview_ground(
                            assets,
                            pid,
                            mouse_elem,
                            Action::WaspNest,
                        )
                    } else {
                        engine.compute_trajectory_preview_ground(assets, pid, mouse_elem)
                    };
                    apply_trajectory_preview(host, preview);
                }
            } else {
                host.valid_trajectory = false;
            }
        } else {
            host.valid_trajectory = false;
        }
        cursor
    }
}

/// HelpToClimb cursor arm.
fn cursor_for_help_to_climb(
    engine: &mut Engine,
    host: &mut Host,
    assets: &LevelAssets,
    mouse_map_pt: MapPoint,
    shift_held: bool,
) -> i32 {
    use robin_engine::element::Posture;
    use robin_engine::resource_ids::*;
    {
        let posture = engine
            .seat_selection(host.transport.local_seat)
            .first()
            .and_then(|&id| engine.get_entity(id))
            .map(|e| e.element_data().posture)
            .unwrap_or(Posture::Undefined);

        // If not in building/lift AND carrying on shoulders.
        if !engine.is_selected_pc_in_restricted_sector() && posture == Posture::CarryingOnShoulders
        {
            choose_mouse_pointer_for_no_action(engine, host, assets, mouse_map_pt, shift_held)
        } else if posture == Posture::HelpingToClimb {
            // Already helping → NoAction cursor.
            choose_mouse_pointer_for_no_action(engine, host, assets, mouse_map_pt, shift_held)
        } else {
            RHMOUSE_OK
        }
    }
}

/// Shield / BigShield cursor arm.
fn cursor_for_shield(
    engine: &mut Engine,
    host: &mut Host,
    assets: &LevelAssets,
    mouse_map_pt: MapPoint,
    shift_held: bool,
) -> i32 {
    use robin_engine::resource_ids::*;
    {
        let action = if shift_held {
            engine.planned_action_for_seat(host.transport.local_seat)
        } else {
            engine.selected_action_for_seat(host.transport.local_seat)
        };
        let is_big = action == Action::BigShield;

        // Check the shield-protected flag.
        if shift_held || engine.shield().is_protected {
            let focused = engine.find_focusable_pc(assets, mouse_map_pt, Focus::Shield);
            if let Some(eid) = focused {
                host.input.focused_entity_id = Some(eid);
                if is_big {
                    RHMOUSE_BIG_SHIELD_YES
                } else {
                    RHMOUSE_SHIELD_YES
                }
            } else if is_big {
                RHMOUSE_BIG_SHIELD_NO
            } else {
                RHMOUSE_SHIELD_NO
            }
        } else {
            // Not yet protected → point cursor.
            if is_big {
                RHMOUSE_BIG_SHIELD_POINT
            } else {
                RHMOUSE_SHIELD_POINT
            }
        }
    }
}

/// Net cursor arm.
fn cursor_for_net(
    engine: &mut Engine,
    host: &mut Host,
    assets: &LevelAssets,
    mouse_map_pt: MapPoint,
    shift_held: bool,
) -> i32 {
    use robin_engine::resource_ids::*;
    {
        let mut cursor = RHMOUSE_NET_NO;
        let mouse_elem = mouse_map_pt;

        let pc_id = engine
            .seat_selection(host.transport.local_seat)
            .first()
            .copied();
        if (shift_held || !engine.is_selected_pc_in_restricted_sector())
            && engine.is_mouse_sector_valid_for_ground_target(mouse_map_pt)
        {
            let in_range = shift_held
                || pc_id.is_some_and(|pid| {
                    engine.is_in_range_for_projectile(assets, pid, mouse_elem, Action::Net, None)
                });
            if in_range {
                cursor = RHMOUSE_NET_YES;

                // Gate the YES cursor on a valid trajectory and
                // render the arc preview when the mouse has been
                // still long enough.
                const TIME_TRAJECTORY_DISPLAY: u32 = 1;
                if host.time_no_mouse_move > TIME_TRAJECTORY_DISPLAY
                    && !host.valid_trajectory
                    && let Some(pid) = pc_id
                {
                    let preview = if shift_held {
                        engine.compute_planned_trajectory_preview_ground(
                            assets,
                            pid,
                            mouse_elem,
                            Action::Net,
                        )
                    } else {
                        engine.compute_trajectory_preview_ground(assets, pid, mouse_elem)
                    };
                    apply_trajectory_preview(host, preview);
                }
            } else {
                host.valid_trajectory = false;
            }
        } else {
            host.valid_trajectory = false;
        }
        cursor
    }
}

/// Lever cursor arm.
fn cursor_for_lever(
    engine: &mut Engine,
    host: &mut Host,
    assets: &LevelAssets,
    mouse_map_pt: MapPoint,
    _shift_held: bool,
) -> i32 {
    use robin_engine::resource_ids::*;
    {
        let focused =
            engine.find_focusable_entity(assets, &host.draw_order.ids, mouse_map_pt, Focus::Lever);
        if let Some(eid) = focused {
            host.input.focused_entity_id = Some(eid);
            RHMOUSE_LEVER_YES
        } else {
            RHMOUSE_LEVER_NO
        }
    }
}

/// Ale cursor arm.
fn cursor_for_ale(
    engine: &mut Engine,
    _host: &mut Host,
    _assets: &LevelAssets,
    mouse_map_pt: MapPoint,
    _shift_held: bool,
) -> i32 {
    use robin_engine::resource_ids::*;
    {
        // Validate mouse sector (no door, no wall/ladder).
        if engine.is_mouse_sector_valid_for_ground_target(mouse_map_pt) {
            RHMOUSE_ALE_YES
        } else {
            RHMOUSE_ALE_NO
        }
    }
}

/// Strangle cursor arm.
fn cursor_for_strangle(
    engine: &mut Engine,
    host: &mut Host,
    assets: &LevelAssets,
    mouse_map_pt: MapPoint,
    _shift_held: bool,
) -> i32 {
    use robin_engine::resource_ids::*;
    {
        let focused = engine.find_focusable_entity(
            assets,
            &host.draw_order.ids,
            mouse_map_pt,
            Focus::Strangle,
        );
        if let Some(eid) = focused {
            host.input.focused_entity_id = Some(eid);
            RHMOUSE_STRANGLE_YES
        } else {
            RHMOUSE_STRANGLE_NO
        }
    }
}

/// Beggar cursor arm.
fn cursor_for_beggar(
    engine: &mut Engine,
    host: &mut Host,
    assets: &LevelAssets,
    mouse_map_pt: MapPoint,
    shift_held: bool,
) -> i32 {
    use robin_engine::element::Posture;
    use robin_engine::resource_ids::*;
    {
        let posture = engine
            .seat_selection(host.transport.local_seat)
            .first()
            .and_then(|&id| engine.get_entity(id))
            .map(|e| e.element_data().posture)
            .unwrap_or(Posture::Undefined);
        if posture == Posture::SimulatingBeggar {
            choose_mouse_pointer_for_no_action(engine, host, assets, mouse_map_pt, shift_held)
        } else {
            RHMOUSE_OK
        }
    }
}

/// Listen cursor arm.
fn cursor_for_listen(
    engine: &mut Engine,
    host: &mut Host,
    assets: &LevelAssets,
    mouse_map_pt: MapPoint,
    shift_held: bool,
) -> i32 {
    use robin_engine::resource_ids::*;
    {
        let action_state = engine
            .seat_selection(host.transport.local_seat)
            .first()
            .and_then(|&id| engine.get_entity(id))
            .and_then(|e| e.actor_data())
            .map(|a| a.action_state)
            .unwrap_or(engine_element::ActionState::Waiting);
        if action_state == engine_element::ActionState::Listening {
            choose_mouse_pointer_for_no_action(engine, host, assets, mouse_map_pt, shift_held)
        } else {
            RHMOUSE_OK
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::campaign::Campaign;
    use robin_engine::element::{
        ActorData, ActorPc, ElementData, ElementKind, HumanData, PcData, Posture,
    };
    use robin_engine::player_command::PlayerCommand;
    use robin_engine::resource_ids::*;

    fn fixture() -> (Engine, LevelAssets, Host) {
        let mut assets = LevelAssets::new();
        let engine = Engine::new_for_test(800.0, 600.0, Campaign::default(), &mut assets)
            .expect("test engine");
        let host = Host::scratch(800.0, 600.0);
        (engine, assets, host)
    }

    fn add_selected_pc(
        engine: &mut Engine,
        assets: &LevelAssets,
    ) -> robin_engine::element::EntityId {
        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..Default::default()
        };
        element.set_position_map(MapPoint::new(10.0, 10.0));
        element.sprite.position_iface.settle_current_position();
        let pc = engine.test_add_entity(robin_engine::element::Entity::Pc(ActorPc {
            element,
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData {
                life_points: 100,
                playable: true,
                ..Default::default()
            },
        }));
        engine
            .advance_frame(
                assets,
                robin_engine::engine::SimulationFrameInput::new(vec![
                    PlayerCommand::SelectPc {
                        pc_id: pc,
                        append: false,
                    }
                    .into(),
                ])
                .with_hourglass(false),
            )
            .expect("selection command admission");
        pc
    }

    #[test]
    fn hit_cursor_is_no_without_focusable_target() {
        let (mut engine, assets, mut host) = fixture();
        add_selected_pc(&mut engine, &assets);

        let cursor = cursor_for_hit(
            &mut engine,
            &mut host,
            &assets,
            MapPoint::new(300.0, 300.0),
            false,
        );
        assert_eq!(cursor, RHMOUSE_HIT_NO);
        assert_eq!(host.input.focused_entity_id, None);
    }

    #[test]
    fn heal_cursor_is_no_without_focusable_target() {
        let (mut engine, assets, mut host) = fixture();
        add_selected_pc(&mut engine, &assets);

        let cursor = cursor_for_heal(
            &mut engine,
            &mut host,
            &assets,
            MapPoint::new(300.0, 300.0),
            false,
        );
        assert_eq!(cursor, RHMOUSE_HEAL_NO);
    }

    #[test]
    fn ale_cursor_rejects_invalid_ground_sector() {
        // The blank test level has no sectors, so no cursor position is
        // a valid ale-drop target.
        let (mut engine, assets, mut host) = fixture();
        add_selected_pc(&mut engine, &assets);

        let cursor = cursor_for_ale(
            &mut engine,
            &mut host,
            &assets,
            MapPoint::new(300.0, 300.0),
            false,
        );
        assert_eq!(cursor, RHMOUSE_ALE_NO);
    }

    #[test]
    fn shield_cursor_tracks_big_variant_and_protection_state() {
        let (mut engine, assets, mut host) = fixture();
        let pc = add_selected_pc(&mut engine, &assets);

        for (action, cursor_no, cursor_point) in [
            (Action::Shield, RHMOUSE_SHIELD_NO, RHMOUSE_SHIELD_POINT),
            (
                Action::BigShield,
                RHMOUSE_BIG_SHIELD_NO,
                RHMOUSE_BIG_SHIELD_POINT,
            ),
        ] {
            engine
                .advance_frame(
                    &assets,
                    robin_engine::engine::SimulationFrameInput::new(vec![
                        PlayerCommand::SelectResolvedAction { pc_id: pc, action }.into(),
                    ])
                    .with_hourglass(false),
                )
                .expect("action selection admission");
            let cursor = cursor_for_shield(
                &mut engine,
                &mut host,
                &assets,
                MapPoint::new(300.0, 300.0),
                false,
            );
            let expected = if engine.shield().is_protected {
                // Awaiting the protected-PC pick with nothing focusable
                // under the cursor.
                cursor_no
            } else {
                // Awaiting the danger-point pick.
                cursor_point
            };
            assert_eq!(cursor, expected);
        }
    }

    #[test]
    fn strangle_cursor_is_no_without_focusable_target() {
        let (mut engine, assets, mut host) = fixture();
        add_selected_pc(&mut engine, &assets);

        let cursor = cursor_for_strangle(
            &mut engine,
            &mut host,
            &assets,
            MapPoint::new(300.0, 300.0),
            false,
        );
        assert_eq!(cursor, RHMOUSE_STRANGLE_NO);
    }
}
