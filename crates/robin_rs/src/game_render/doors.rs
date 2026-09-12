//! doors presentation pass.
use super::*;

// ─── Door / jump zone alpha overlays ──────────────────────────────────

const COLOR_DOOR: u32 = 0x0060D0; // Royal blue
const ALPHA_DOOR: u32 = 96;
const COLOR_JUMPZONE: u32 = 0xA5FF50; // Lime green
const ALPHA_JUMPZONE: u32 = 64;

/// Render all door- and jump-zone alpha overlays for the current frame.
///
/// Order of operations:
///   * For every selected PC whose sector is a building, draw all that
///     building's door polygons. Runs unconditionally (outside the
///     shift/gating block).
///   * If shift is held: walk every gate (skipping `LiftLow`/`LiftHigh`),
///     every patch's doors, and every active jump sector. Early-return
///     afterwards.
///   * Otherwise, gate on `!draw_multi_selection && !is_dragging &&
///     (action == NoAction || HelpToClimb-with-climb-posture ||
///     Beggar-with-beggar-posture)`.
///   * Hovered-door branch: when `display_door` is set on the cursor and
///     the hovered sector is a door, either stack up to the connected
///     building (Building / BuildingTrap door-types) or paint the single
///     door polygon. In both cases, skip when the door is controlled by a
///     patch (the patch-driven draw happens below from the host's selected
///     patch index).
///   * Hovered-jump branch: jump-type sector gets the jumpzone alpha.
///   * Hovered-patch branch: draw the patch's mouse sector polygon, each
///     of its door polygons, and each opposite-side motion-area's own door
///     polygons.
pub(crate) fn render_door_overlays(
    host: &HostDraw<'_>,
    engine: &PresentationView<'_>,
    assets: &LevelAssets,
    renderer: &mut Renderer,
    shift_held: bool,
) {
    use robin_engine::element::Posture;
    use robin_engine::gate::DoorType;
    use robin_engine::profiles::Action;
    use robin_engine::sector::SectorType;

    if !engine.has_mission_geometry() {
        return;
    }

    let draw_geo_polygon = |renderer: &mut Renderer, pts: &[MapPoint], color: u32, alpha: u32| {
        if pts.len() < 3 {
            return;
        }
        host.draw_manager()
            .draw_alpha_polygon(renderer, pts, color, alpha);
    };

    let draw_map_polygon =
        |renderer: &mut Renderer, pts: &[engine_coordinates::MapPoint], color: u32, alpha: u32| {
            if pts.len() < 3 {
                return;
            }
            host.draw_manager()
                .draw_alpha_polygon(renderer, pts, color, alpha);
        };

    let draw_door = |renderer: &mut Renderer, door: &robin_engine::gate::Door| {
        if door.click_polygon.len() < 3 {
            return;
        }
        let pts: Vec<MapPoint> = door
            .click_polygon
            .iter()
            .map(|&(x, y)| MapPoint::new(x, y))
            .collect();
        draw_geo_polygon(renderer, &pts, COLOR_DOOR, ALPHA_DOOR);
    };

    // Walk a motion-area / building sector's gate list and paint each door.
    // Building sectors paint unconditionally; motion-area sectors require
    // the door to be `active`.
    let draw_sector_doors = |renderer: &mut Renderer,
                             sector: &robin_engine::fast_find_grid::GridSector,
                             require_active: bool| {
        for &gate_idx in &sector.gate_indices {
            let Some(door) = engine.doors().get(usize::from(gate_idx)) else {
                continue;
            };
            if !door.is_door() {
                continue;
            }
            if require_active && !door.active {
                continue;
            }
            draw_door(renderer, door);
        }
    };

    let sector_by_number = |sector_num: i16| -> Option<&robin_engine::fast_find_grid::GridSector> {
        let &idx = engine
            .fast_grid()
            .level
            .sector_number_map
            .get(&engine_sector::SectorNumber::new(sector_num))?;
        engine.fast_grid().level.sectors.get(idx)
    };

    // ── 1. Selected PCs inside buildings (runs unconditionally) ──
    let local_seat = host.local_seat;
    for &pc_id in engine.hero_selection(local_seat) {
        let Some(entity) = engine.get_entity(pc_id) else {
            continue;
        };
        if !entity.is_active() {
            continue;
        }
        let Some(sector_num) = entity.element_data().sector() else {
            continue;
        };
        let Some(sector) = sector_by_number(i16::from(sector_num)) else {
            continue;
        };
        if sector.sector_type.is_building() {
            // Building override skips the `door.active` gate.
            draw_sector_doors(renderer, sector, false);
        }
    }

    // ── 2. Shift-held: display all doors and jump zones ──
    if shift_held {
        // All gates, except lift entry/exit doors.
        for door in engine.doors().iter() {
            if !door.is_door() {
                continue;
            }
            if matches!(door.door_type, DoorType::LiftLow | DoorType::LiftHigh) {
                continue;
            }
            draw_door(renderer, door);
        }

        // Every patch's own doors.  We inline the draw here since the
        // patch-FX consumer isn't plumbed into the renderer.
        for patch in engine.patches().iter() {
            for &door_idx in &patch.door_indices {
                if let Some(door) = engine.doors().get(door_idx as usize) {
                    draw_door(renderer, door);
                }
            }
        }

        // Every active jump sector.
        for (idx, sector) in engine.fast_grid().level.sectors.iter().enumerate() {
            if !engine.fast_grid().is_sector_active(idx as u32) {
                continue;
            }
            if !sector.sector_type.contains(SectorType::JUMP) {
                continue;
            }
            draw_map_polygon(renderer, &sector.points, COLOR_JUMPZONE, ALPHA_JUMPZONE);
        }
        return;
    }

    // ── 3. Gating ──
    if host.frontend.input.draw_multi_selection() || host.frontend.input.is_dragging() {
        return;
    }
    let first_selected_posture = engine
        .hero_selection(local_seat)
        .first()
        .and_then(|&id| engine.get_entity(id))
        .map(|e| e.element_data().posture());
    let action_ok = match engine.selected_action_for_seat(local_seat) {
        Action::NoAction => true,
        Action::HelpToClimb => matches!(
            first_selected_posture,
            Some(Posture::HelpingToClimb | Posture::CarryingOnShoulders)
        ),
        Action::Beggar => matches!(first_selected_posture, Some(Posture::SimulatingBeggar)),
        _ => false,
    };
    if !action_ok {
        return;
    }

    // ── 4. Hovered-door branch ──
    let selected_grid_idx = host
        .frontend
        .input
        .spatial_hit()
        .selected_sector_idx
        .map(usize::from);
    let selected_sector = selected_grid_idx.and_then(|i| engine.fast_grid().level.sectors.get(i));
    let selected_sector_num = selected_sector.map(|s| i16::from(s.sector_number));
    let selected_sector_active = selected_grid_idx
        .map(|i| engine.fast_grid().is_sector_active(i as u32))
        .unwrap_or(false);

    if host.frontend.input.feedback.display_door
        && let Some(door_idx) = host.frontend.input.spatial_hit().hovered_door_idx
        && let Some(door) = engine.doors().get(door_idx as usize)
    {
        match door.door_type {
            DoorType::Building | DoorType::BuildingTrap => {
                let building_sector = sector_by_number(i16::from(door.sector_in))
                    .filter(|s| s.sector_type.is_building())
                    .or_else(|| {
                        sector_by_number(i16::from(door.sector_out))
                            .filter(|s| s.sector_type.is_building())
                    });
                if let Some(building) = building_sector {
                    draw_sector_doors(renderer, building, false);
                } else {
                    draw_door(renderer, door);
                }
            }
            _ => {
                draw_door(renderer, door);
            }
        }
    }

    if let Some((sector_index, sector)) = selected_grid_idx.zip(selected_sector) {
        if host.frontend.input.feedback.display_door
            && sector.sector_type.is_door()
            && let Some(door_idx) = sector.door_index
            && let Some(door) = engine.doors().get(door_idx as usize)
        {
            // Defer to the patch-FX path on either side of the door↔patch
            // wiring: door_triggered (door.patch_index set) or
            // triggers_door (door listed in patch.door_indices).  Mirrors
            // the original game's door-patch lookup.
            let owning_patch = engine.find_patch_for_door(door_idx);
            match door.door_type {
                // Building / BuildingTrap: stack up to the connected
                // building's doors.
                DoorType::Building | DoorType::BuildingTrap => {
                    // Pick whichever side is the building.
                    let building_sector = sector_by_number(i16::from(door.sector_in))
                        .filter(|s| s.sector_type.is_building())
                        .or_else(|| {
                            sector_by_number(i16::from(door.sector_out))
                                .filter(|s| s.sector_type.is_building())
                        });
                    if let Some(building) = building_sector {
                        // Only draw inline when no patch owns the door;
                        // otherwise the selected-patch path handles it below.
                        if owning_patch.is_none() {
                            draw_sector_doors(renderer, building, false);
                        }
                    }
                }
                // Non-building door: paint the single door polygon
                // unless a patch owns it.
                _ => {
                    if owning_patch.is_none() && !sector.points.is_empty() && selected_sector_active
                    {
                        draw_map_polygon(renderer, &sector.points, COLOR_DOOR, ALPHA_DOOR);
                    }
                }
            }
        }

        // ── 5. Hovered-jump branch ──
        // Iterate selected PCs and, on the FIRST PC that has the Jump
        // contextual action, take the result of
        // [`PresentationView::get_nearest_jumpable_jump_line`] unconditionally —
        // including None.  Subsequent selected PCs are NOT consulted:
        // an early-return loop (not a combinator) so multi-PC
        // selections where the first jumper cannot reach the sector
        // suppress the overlay instead of painting it from a later
        // jumper.  The lookup respects sector-match and gate-
        // authorization, so unreachable jump lines (wrong sector,
        // helper-needed destinations without a shoulder ride) don't
        // trigger the jump-highlight.
        if sector.sector_type.contains(SectorType::JUMP)
            && selected_sector_active
            && !sector.points.is_empty()
        {
            let mut paint = false;
            for &pc_id in engine.hero_selection(local_seat) {
                if !engine.selected_pc_has_contextual_action(assets, Some(pc_id), Action::Jump) {
                    continue;
                }
                let pc_pos = engine
                    .get_entity(pc_id)
                    .map(|e| e.element_data().position_map())
                    .unwrap_or(MapPoint::ZERO);
                paint = engine
                    .get_nearest_jumpable_jump_line(
                        pc_id,
                        sector_index as u32,
                        pc_pos,
                        host.frontend.input.spatial_hit().selected_map_point,
                        /* test_posture */ false,
                        None,
                    )
                    .is_some();
                break;
            }
            if paint {
                draw_map_polygon(renderer, &sector.points, COLOR_JUMPZONE, ALPHA_JUMPZONE);
            }
        }
    }

    // ── 6. Hovered-patch branch ──
    // Local cursor selection is host presentation state. Read it directly
    // instead of mutating a render cache inside the authoritative engine.
    if let Some(patch) = host
        .frontend
        .input
        .spatial_hit()
        .selected_patch_idx
        .and_then(|index| engine.patches().get(index as usize))
    {
        // Paint the patch's active mouse sector.
        if !patch.in_transition {
            let mouse_sector_list = if patch.applied {
                &patch.new_sector_indices
            } else {
                &patch.old_sector_indices
            };
            for &grid_idx in mouse_sector_list {
                let Some(s) = engine.fast_grid().level.sectors.get(grid_idx as usize) else {
                    continue;
                };
                if s.sector_type.is_patch() && engine.fast_grid().is_sector_active(grid_idx) {
                    draw_map_polygon(renderer, &s.points, COLOR_DOOR, ALPHA_DOOR);
                    break;
                }
            }
        }

        for &door_idx in &patch.door_indices {
            let Some(door) = engine.doors().get(door_idx as usize) else {
                continue;
            };

            // Draw each patch door's own polygon.
            draw_door(renderer, door);

            // Draw the opposite-side motion area's doors.  Opposite-side
            // is the side whose `sector_number` isn't the hovered
            // sector's.
            let other_sector_num = if Some(i16::from(door.sector_in)) == selected_sector_num {
                door.sector_out
            } else {
                door.sector_in
            };
            let Some(other_sector) = sector_by_number(i16::from(other_sector_num)) else {
                continue;
            };
            if other_sector.sector_type.is_motion() {
                draw_sector_doors(renderer, other_sector, true);
            }
        }
    }
}
