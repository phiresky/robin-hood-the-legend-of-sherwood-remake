//! In-game rendering passes for the mission loop.
//!
//! Contains the GPU-phase rendering functions: entity sprites, selection
//! outlines, ground marks, and minimap.  The in-game
//! menu rendering lives in [`crate::ingame_menu`] and is driven by
//! [`crate::game_session`].

use crate::gfx_types::Rect;
use crate::host::Host;
use crate::hud_text::{self, HudFonts};
use crate::ingame_menu::layout;
use crate::renderer::{BLIT_SOURCE_TRANSPARENT, OUTLINE_PAD, Renderer, rgb565_to_rgb8};
use crate::titbit_renderer::TitbitRenderer;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::coordinates::{GroundPoint, MapPoint};
use robin_engine::element as engine_element;
use robin_engine::element::{ElementKind, Entity, OutlineColorName, Posture, RenderingProperties};
use robin_engine::engine as engine_api;
use robin_engine::engine::{DevState, Engine, LevelAssets};
use robin_engine::markers::GroundMark;
use robin_engine::mask as engine_mask;
use robin_engine::position_interface as engine_position_interface;
use robin_engine::sector as engine_sector;
use robin_engine::sight_obstacle as engine_sight_obstacle;
use robin_engine::sprite::BBox;

mod debug;
mod hud;
mod minimap;

pub(crate) use debug::{
    render_debug_animation_lines, render_debug_doors, render_debug_motion_graph,
    render_debug_surfaces_fill, render_debug_surfaces_outline, render_debug_whatsup_overlay,
    render_noise_display, render_shadow_polygon_sphere_debug,
};
pub(crate) use hud::{
    draw_multi_selection_box, render_combat_status_bars, render_item_effect_preview,
    render_listen_ping, render_ransom_amulet_overlay, render_trajectory_preview,
};
pub(crate) use minimap::render_minimap;

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
    host: &mut Host,
    engine: &Engine,
    assets: &LevelAssets,
    renderer: &mut Renderer,
    shift_held: bool,
) {
    use robin_engine::element::Posture;
    use robin_engine::gate::DoorType;
    use robin_engine::profiles::Action;
    use robin_engine::sector::SectorType;

    if engine.mission_script().is_none() {
        return;
    }

    let draw_geo_polygon = |renderer: &mut Renderer, pts: &[MapPoint], color: u32, alpha: u32| {
        if pts.len() < 3 {
            return;
        }
        host.draw_manager
            .draw_alpha_polygon(renderer, pts, color, alpha);
    };

    let draw_map_polygon =
        |renderer: &mut Renderer, pts: &[engine_coordinates::MapPoint], color: u32, alpha: u32| {
            if pts.len() < 3 {
                return;
            }
            host.draw_manager
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
    let local_seat = host.transport.local_seat;
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
    if host.input.draw_multi_selection || host.input.is_dragging {
        return;
    }
    let first_selected_posture = engine
        .hero_selection(local_seat)
        .first()
        .and_then(|&id| engine.get_entity(id))
        .map(|e| e.element_data().posture);
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
    let selected_grid_idx = host.input.selected_sector_idx.map(usize::from);
    let selected_sector = selected_grid_idx.and_then(|i| engine.fast_grid().level.sectors.get(i));
    let selected_sector_num = selected_sector.map(|s| i16::from(s.sector_number));
    let selected_sector_active = selected_grid_idx
        .map(|i| engine.fast_grid().is_sector_active(i as u32))
        .unwrap_or(false);

    if host.input.display_door
        && let Some(door_idx) = host.input.hovered_door_idx
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
        if host.input.display_door
            && sector.sector_type.is_door()
            && let Some(door_idx) = sector.door_index
            && let Some(door) = engine.doors().get(door_idx as usize)
        {
            // Defer to the patch-FX path on either side of the door↔patch
            // wiring: door_triggered (door.patch_index set) or
            // triggers_door (door listed in patch.door_indices).  Mirrors
            // C++ `RHFastFindGrid::GetPatch(pDoor)`.
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
        // [`Engine::get_nearest_jumpable_jump_line`] unconditionally —
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
                        host.input.selected_map_point,
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
    // instead of mutating a render cache inside the authoritative Engine.
    if let Some(patch) = host
        .input
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

// ─── View cone overlay ────────────────────────────────────────────

/// Darken the map outside the vision cone of the currently-selected view
/// element, if any.
///
/// Computes a view cone for the selected view element, clips it against
/// nearby opaque sight obstacles, and darkens the complement.  The call
/// is a no-op when no entity is selected as the view element (i.e. the
/// player isn't holding Alt over an NPC).
///
/// Renders the darkening overlay as a blended GPU texture so that
/// entities drawn later via GPU sprite textures appear at full
/// brightness on top of the darkened base — the overlay must run before
/// the entity refresh loop.
pub(crate) fn render_view_cone_overlay(
    host: &Host,
    engine: &Engine,
    assets: &LevelAssets,
    selected_view_element: Option<engine_element::EntityId>,
    dev: &engine_api::DevState,
    renderer: &mut Renderer,
) {
    use robin_engine::engine::{Ambiance, PANNEL_HEIGHT};

    // Priority order:
    //   1. `--view-cones` CLI flag: show ALL NPC cones at once
    //   2. `free_shadow_polygon`: developer cheat with stored position
    //   3. `selected_view_element`: Alt-hover single cone
    if dev.debug.all_view_cones {
        render_all_view_cones(host, engine, assets, renderer);
        return;
    }

    let (viewer, params, tint) = if dev.debug.free_shadow_polygon {
        // Developer cheat: anchor the cone at a stored 3D position,
        // or at the camera centre when nothing has been set yet.
        let pos =
            dev.cheat_free_shadow_polygon_pos
                .unwrap_or_else(|| engine_coordinates::WorldPoint3D {
                    x: host.viewport.view_position.x
                        + (host.viewport.screen_size.x / host.viewport.zoom_factor) * 0.5,
                    y: host.viewport.view_position.y
                        + (host.viewport.screen_size.y / host.viewport.zoom_factor) * 0.5,
                    z: 0.0,
                });
        (
            GroundPoint::new(pos.x, pos.y),
            dev.cheat_free_shadow_polygon_params.clone(),
            None,
        )
    } else {
        let Some(triple) = engine.selected_view_cone_params(selected_view_element) else {
            return;
        };
        triple
    };

    // compute_visibility_polygon only cares about active obstacles —
    // pre-filter so the callee doesn't need to consult the parallel
    // active flag (which lives on the engine, not the obstacle itself).
    let obstacles_view = engine.sight_obstacles(assets);
    let Some(render_slices) = view_cone_polys_for_render(viewer, &params, &obstacles_view) else {
        return;
    };
    if !render_slices
        .iter()
        .any(|slice| slice.polys.iter().any(|p| p.len() >= 3))
    {
        return;
    }

    // World-space view rectangle, matching `update_draw_manager_params`
    // (engine/render.rs) — the UI panel at the bottom is excluded so the
    // overlay leaves the panel alone.
    let view_rect = engine_coordinates::MapBBox::from_coords(
        host.viewport.view_position.x,
        host.viewport.view_position.y,
        host.viewport.view_position.x
            + (host.viewport.screen_size.x - 1.0) / host.viewport.zoom_factor,
        host.viewport.view_position.y
            + (host.viewport.screen_size.y - PANNEL_HEIGHT + 1.0) / host.viewport.zoom_factor,
    );

    let alpha = params.alpha.min(crate::shadow_polygon::alpha_for_ambiance(
        engine.weather().ambiance == Ambiance::Night || engine.weather().ambiance == Ambiance::Fog,
    ));

    let tint = tint.unwrap_or((0, 0, 0));

    // Collect character masks whose world-space bbox intersects the view
    // rect — these building silhouettes clear the tint inside the cone
    // in `render_darken_inside_gpu`'s mask post-pass.
    let cone_masks: Vec<&engine_mask::RuntimeMask> = engine
        .fast_grid()
        .level
        .masks
        .iter()
        .enumerate()
        .filter(|(idx, m)| {
            // Only masks with a valid (non-max) index participate in the
            // active toggle; enumerate() yields usize so wrap through new().
            engine_mask::MaskIndex::new(*idx as u32)
                .is_some_and(|mi| engine.fast_grid().is_mask_active(mi))
                && m.is_character()
                && m.bbox.intersects_bbox(&view_rect)
        })
        .map(|(_, m)| m)
        .collect();

    for slice in render_slices {
        crate::shadow_polygon::render_darken_inside(
            renderer,
            &view_rect,
            host.viewport.zoom_factor,
            &slice.polys,
            tint,
            alpha,
            slice.viewer,
            slice.radius,
            slice.projection_plane,
            &cone_masks,
        );
    }
}

struct ViewConeRenderSlice {
    polys: Vec<Vec<GroundPoint>>,
    viewer: GroundPoint,
    radius: f32,
    projection_plane: Option<engine_position_interface::PlaneZCoeffs>,
}

fn view_cone_polys_for_render(
    viewer: GroundPoint,
    params: &crate::shadow_polygon::ViewParameters,
    obstacles_view: &engine_sight_obstacle::ObstacleList<'_>,
) -> Option<Vec<ViewConeRenderSlice>> {
    if let Some(obstacle_handle) = params.projection_obstacle {
        let idx = usize::from(obstacle_handle);
        let Some(current_area) = obstacles_view.get(idx) else {
            tracing::warn!(
                "view-cone projection obstacle {} is missing from the sight-obstacle list",
                obstacle_handle
            );
            return None;
        };
        if !current_area.is_projection_area() {
            tracing::warn!(
                "view-cone projection obstacle {} is not a projection area",
                obstacle_handle
            );
            return None;
        }
    }

    let active_obstacles: Vec<(usize, &robin_engine::sight_obstacle::SightObstacle)> =
        obstacles_view
            .iter_indexed()
            .filter_map(|(idx, o)| {
                let idx = idx as usize;
                obstacles_view.is_active(idx).then_some((idx, o))
            })
            .collect();

    let all_obstacles: Vec<&robin_engine::sight_obstacle::SightObstacle> =
        active_obstacles.iter().map(|(_, o)| *o).collect();
    let mut slices = Vec::new();
    if let Some(radius) = shadow_polygon_slice_radius(params, None, viewer) {
        let mut slice_params = params.clone();
        slice_params.radius = radius;
        let ground_polys = crate::shadow_polygon::compute_visibility_polygon(
            viewer,
            &slice_params,
            &all_obstacles,
        );
        slices.push(ViewConeRenderSlice {
            polys: ground_polys,
            viewer,
            radius,
            projection_plane: None,
        });
    }

    let cone_bbox = {
        let cone = crate::shadow_polygon::compute_view_cone(viewer, params);
        let mut bbox = engine_coordinates::GroundBBox::new();
        for p in cone {
            bbox.expand_point(p);
        }
        bbox
    };

    for (projection_idx, projection_area) in active_obstacles.iter().copied().filter(|(_, o)| {
        o.is_projection_area()
            && o.is_showing_shadow_polygon()
            && o.box_ground.intersects_bbox(&cone_bbox)
    }) {
        let obstacles: Vec<&robin_engine::sight_obstacle::SightObstacle> = active_obstacles
            .iter()
            .filter(|(idx, _)| *idx != projection_idx)
            .map(|(_, o)| *o)
            .collect();
        let projection_plane = engine_position_interface::PlaneZCoeffs::from_plane_points(
            &projection_area.top_plane_points,
        );
        let Some(radius) = shadow_polygon_slice_radius(params, Some(projection_plane), viewer)
        else {
            continue;
        };
        let mut slice_params = params.clone();
        slice_params.radius = radius;
        let occluding_projection_areas: Vec<&robin_engine::sight_obstacle::SightObstacle> =
            active_obstacles
                .iter()
                .filter_map(|(idx, o)| {
                    (*idx != projection_idx
                        && o.is_projection_area()
                        && o.layer >= projection_area.layer
                        && o.box_projection
                            .intersects_bbox(&projection_area.box_projection))
                    .then_some(*o)
                })
                .collect();
        let (polys, viewer) = crate::shadow_polygon::project_and_clip_to_projection_area(
            &crate::shadow_polygon::compute_visibility_polygon(viewer, &slice_params, &obstacles),
            viewer,
            projection_plane,
            projection_area,
            &occluding_projection_areas,
        );
        if polys.iter().any(|p| p.len() >= 3) {
            slices.push(ViewConeRenderSlice {
                polys,
                viewer,
                radius,
                // `project_and_clip_to_projection_area` returns coordinates
                // in the same projected map space C++ blits the slice from.
                // Passing the plane again here would subtract the elevation a
                // second time.
                projection_plane: None,
            });
        }
    }

    if slices.iter().any(|s| s.polys.iter().any(|p| p.len() >= 3)) {
        Some(slices)
    } else {
        None
    }
}

fn shadow_polygon_slice_radius(
    params: &crate::shadow_polygon::ViewParameters,
    projection_plane: Option<engine_position_interface::PlaneZCoeffs>,
    viewer: GroundPoint,
) -> Option<f32> {
    const FACTOR_ELLIPSE: f32 = 0.35;
    const INV_SQUARE_FACTOR_ELLIPSE: f32 = 8.163_265;
    const FACTOR_CONE_LEAN_OUT: f32 = 0.8;

    let distance_to_plane = projection_plane
        .map(|plane| {
            let vertical = params.viewer_z - plane.compute_z(viewer.x, viewer.y);
            let normal_len = (plane.az * plane.az + plane.bz * plane.bz + 1.0).sqrt();
            vertical / normal_len
        })
        .unwrap_or(params.viewer_z);

    let radius = params.radius;
    if params.lean_out {
        if projection_plane.is_none() {
            if distance_to_plane > radius {
                return None;
            }
            return Some(FACTOR_CONE_LEAN_OUT * distance_to_plane);
        }
        if distance_to_plane >= radius || distance_to_plane <= 0.0 {
            return None;
        }
        return Some(FACTOR_CONE_LEAN_OUT * distance_to_plane);
    }

    if distance_to_plane.abs() >= FACTOR_ELLIPSE * radius {
        return None;
    }
    let radius_sq =
        radius * radius - INV_SQUARE_FACTOR_ELLIPSE * distance_to_plane * distance_to_plane;
    (radius_sq > 0.0).then(|| radius_sq.sqrt())
}

/// Render view cones for ALL NPCs with per-NPC alert tinting (`--view-cones`).
fn render_all_view_cones(
    host: &Host,
    engine: &Engine,
    assets: &LevelAssets,
    renderer: &mut Renderer,
) {
    use robin_engine::engine::{Ambiance, PANNEL_HEIGHT};

    let all_params = engine.all_npc_view_cone_params();
    if all_params.is_empty() {
        return;
    }

    let view_rect = engine_coordinates::MapBBox::from_coords(
        host.viewport.view_position.x,
        host.viewport.view_position.y,
        host.viewport.view_position.x
            + (host.viewport.screen_size.x - 1.0) / host.viewport.zoom_factor,
        host.viewport.view_position.y
            + (host.viewport.screen_size.y - PANNEL_HEIGHT + 1.0) / host.viewport.zoom_factor,
    );

    let obstacles_view = engine.sight_obstacles(assets);

    // Each NPC's visibility polygon may fragment into multiple rings
    // after obstacle subtraction. Each ring becomes its own TintedCone
    // with the NPC's tint — geo's difference guarantees the MultiPolygon
    // parts are disjoint, so same-tint rings never overlap and the GPU
    // path's alpha-blend doesn't double-darken.
    let weather_alpha = crate::shadow_polygon::alpha_for_ambiance(
        engine.weather().ambiance == Ambiance::Night || engine.weather().ambiance == Ambiance::Fog,
    );

    let visible_params: Vec<_> = all_params
        .into_iter()
        .filter(|(viewer, params, _)| {
            let r = params.radius;
            let z = params.viewer_z.max(0.0);
            let cone_bbox = engine_coordinates::MapBBox::from_coords(
                viewer.x - r,
                viewer.y - z - r,
                viewer.x + r,
                viewer.y + r,
            );
            view_rect.intersects_bbox(&cone_bbox)
        })
        .collect();
    if visible_params.is_empty() {
        return;
    }

    let cones: Vec<crate::shadow_polygon::TintedCone> = visible_params
        .into_iter()
        .flat_map(|(viewer, params, tint)| {
            let Some(slices) = view_cone_polys_for_render(viewer, &params, &obstacles_view) else {
                return Vec::new();
            };
            let color = tint.unwrap_or((0, 0, 0));
            let alpha = params.alpha.min(weather_alpha);
            let view_rect_for_filter = view_rect;
            slices
                .into_iter()
                .flat_map(move |slice| {
                    let view_rect_for_filter = view_rect_for_filter;
                    let radius = slice.radius;
                    slice
                        .polys
                        .into_iter()
                        .filter(|p| p.len() >= 3)
                        .filter(move |p| {
                            let mut x_min = p[0].x;
                            let mut y_min = p[0].y;
                            let mut x_max = p[0].x;
                            let mut y_max = p[0].y;
                            for &point in &p[1..] {
                                x_min = x_min.min(point.x);
                                y_min = y_min.min(point.y);
                                x_max = x_max.max(point.x);
                                y_max = y_max.max(point.y);
                            }
                            let bbox = engine_coordinates::MapBBox::from_coords(
                                x_min, y_min, x_max, y_max,
                            );
                            view_rect_for_filter.intersects_bbox(&bbox)
                        })
                        .map(move |p| {
                            (
                                p,
                                color,
                                slice.viewer,
                                radius,
                                alpha,
                                slice.projection_plane,
                            )
                        })
                })
                .collect::<Vec<_>>()
        })
        .collect();

    if cones.is_empty() {
        return;
    }

    crate::shadow_polygon::render_tinted_cones(
        renderer,
        &view_rect,
        host.viewport.zoom_factor,
        &cones,
    );
}

// ─── Ground marks ──────────────────────────────────────────────────

/// Render every active destination marker.
///
/// For each active mark, check on-screen and blit. Engine-owned command
/// marks advance inside `perform_hourglass`; host-owned trajectory-
/// preview marks advance on the same hourglass cadence without entering
/// sim state.
pub(crate) fn render_ground_marks(
    host: &mut Host,
    engine: &Engine,
    _assets: &LevelAssets,
    renderer: &mut Renderer,
) {
    if host.ground_mark_surfaces.is_empty() {
        return;
    }

    render_ground_mark_set(host, engine.ground_mark(), engine, renderer);
    render_ground_mark_set(host, &host.trajectory_ground_mark, engine, renderer);
}

fn render_ground_mark_set(
    host: &Host,
    ground_mark: &GroundMark,
    engine: &Engine,
    renderer: &mut Renderer,
) {
    if ground_mark.is_empty() {
        return;
    }
    let zoom = host.viewport.zoom_factor;
    let screen_w = host.viewport.screen_size.x as i32;
    let screen_h = host.viewport.screen_size.y as i32;

    // The same shadow rendering used for entity shadows.
    let shadow_color = engine.weather().night_color;
    let shadow_level = host.frame_holder.global_shadow();

    let view_pos = host.viewport.view_position;

    let per_frame_offsets = ground_mark.per_frame_offsets();

    for mark in &ground_mark.marks {
        // Read `render_frame` (snapshot taken at the start of `tick`
        // before advancing) so we draw the pre-retire frame on the tick
        // where the animation ends.
        let frame_idx = mark.render_frame as usize;
        let (surf_id, fw, fh) = match host.ground_mark_surfaces.get(frame_idx) {
            Some(&entry) => entry,
            None => continue,
        };

        // World→screen. `mark.x`/`mark.y` is already the sprite top-left
        // (half-diagonal was subtracted at `add_mark` time), so the
        // on-screen destination is the direct affine transform — no
        // additional half-width offset.
        let screen_x = (mark.x - view_pos.x) * zoom;
        let screen_y = (mark.y - view_pos.y) * zoom;

        let scaled_w = (fw as f32 * zoom).round() as i32;
        let scaled_h = (fh as f32 * zoom).round() as i32;
        if scaled_w <= 0 || scaled_h <= 0 {
            continue;
        }

        let dst_x = screen_x.round() as i32;
        let dst_y = screen_y.round() as i32;

        // Per-frame offset — added to the sprite top-left before
        // computing the cull AABB.  We still blit the uncropped surface
        // (transparent border absorbs the offset visually), but the
        // cull tracks the offset when it's non-zero.
        let (ox, oy) = per_frame_offsets.get(frame_idx).copied().unwrap_or((0, 0));
        let cull_x = dst_x + (ox as f32 * zoom).round() as i32;
        let cull_y = dst_y + (oy as f32 * zoom).round() as i32;

        let on_screen = cull_x + scaled_w > 0
            && cull_y + scaled_h > 0
            && cull_x < screen_w
            && cull_y < screen_h;
        if !on_screen {
            continue;
        }

        let src_box = BBox::from_coords(0.0, 0.0, fw as f32, fh as f32);
        let dst_box = BBox::from_coords(
            dst_x as f32,
            dst_y as f32,
            (dst_x + scaled_w) as f32,
            (dst_y + scaled_h) as f32,
        );

        let draw_checkpoint = renderer.draw_queue_checkpoint();
        renderer.blit_with_shadow(
            surf_id,
            Some(&src_box),
            0,
            Some(&dst_box),
            shadow_color,
            shadow_level,
            BLIT_SOURCE_TRANSPARENT,
        );

        let mark_world_bbox = engine_coordinates::MapBBox::from_coords(
            mark.x + ox as f32,
            mark.y + oy as f32,
            mark.x + ox as f32 + fw as f32,
            mark.y + oy as f32 + fh as f32,
        );
        let mark_position = MapPoint::new(
            mark.x + ox as f32 + fw as f32 * 0.5,
            mark.y + oy as f32 + fh as f32 * 0.5,
        );
        let mark_rect = Rect::new(dst_x, dst_y, scaled_w as u32, scaled_h as u32);
        render_character_masks_clipped(
            engine,
            renderer,
            mark.layer,
            &mark_world_bbox,
            mark_position,
            mark_rect,
            draw_checkpoint,
            view_pos,
            zoom,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn render_character_masks_clipped(
    engine: &Engine,
    renderer: &mut Renderer,
    layer: u16,
    world_bbox: &engine_coordinates::MapBBox,
    position: engine_coordinates::MapPoint,
    clip_rect: Rect,
    draw_checkpoint: usize,
    view: engine_coordinates::MapPoint,
    zoom: f32,
) {
    let mask_indices = engine
        .fast_grid()
        .get_masks_applied_to_character(layer, world_bbox, position);
    if mask_indices.is_empty() {
        return;
    }
    let screen_masks = sprite_screen_masks(engine, &mask_indices, view, zoom);
    renderer.mask_queued_draws(draw_checkpoint, &screen_masks, clip_rect);
}

fn sprite_screen_masks(
    engine: &Engine,
    mask_indices: &[engine_mask::MaskIndex],
    view: engine_coordinates::MapPoint,
    zoom: f32,
) -> Vec<(u32, Rect)> {
    let mut screen_masks = Vec::with_capacity(mask_indices.len());
    for &mask_idx in mask_indices {
        let mask = &engine.fast_grid().level.masks[usize::from(mask_idx)];
        let mask_screen_x = ((mask.bbox.x_min() - view.x) * zoom).round() as i32;
        let mask_screen_y = ((mask.bbox.y_min() - view.y) * zoom).round() as i32;
        let mask_screen_w = (mask.width as f32 * zoom).round() as u32;
        let mask_screen_h = (mask.height as f32 * zoom).round() as u32;
        if mask_screen_w == 0 || mask_screen_h == 0 {
            continue;
        }
        let mask_rect = Rect::new(mask_screen_x, mask_screen_y, mask_screen_w, mask_screen_h);
        screen_masks.push((u32::from(mask_idx), mask_rect));
    }
    screen_masks
}

#[allow(clippy::too_many_arguments)]
fn applicable_sprite_masks(
    engine: &Engine,
    assets: &LevelAssets,
    actor_layer: u16,
    sprite_world_bbox: &engine_coordinates::MapBBox,
    actor_position: engine_coordinates::MapPoint,
    projectile_position: engine_coordinates::WorldPoint3D,
    use_projectile_path: bool,
    is_flying_human: bool,
) -> Vec<engine_mask::MaskIndex> {
    if use_projectile_path {
        engine.fast_grid().get_masks_applied_to_projectile(
            engine.fast_grid().level.special_layer,
            sprite_world_bbox,
            projectile_position,
            is_flying_human,
            engine.sight_obstacles(assets),
        )
    } else {
        engine.fast_grid().get_masks_applied_to_character(
            actor_layer,
            sprite_world_bbox,
            actor_position,
        )
    }
}

// ─── GPU entity rendering ─────────────────────────────────────────

fn entity_visual_map_position(entity: &Entity) -> MapPoint {
    entity.sprite_visual_map_position()
}

/// Render all entities using cached GPU textures.
///
/// Replaces `render_entities` for the GPU phase.  Each sprite frame is
/// decompressed once and cached as an ARGB8888 GPU texture; subsequent
/// frames with the same `(bank_id, variant, shadow_color)` key reuse the
/// cached texture in a queued GPU draw (zero CPU decompression work).
pub(crate) fn render_entities_gpu(
    host: &mut Host,
    engine: &Engine,
    assets: &LevelAssets,
    dev: &DevState,
    renderer: &mut Renderer,
    titbit_renderer: &mut TitbitRenderer,
) {
    let view = host.viewport.view_position;
    let zoom = host.viewport.zoom_factor;
    let screen_w = host.viewport.screen_size.x as i32;
    let screen_h = host.viewport.screen_size.y as i32;
    let shadow_color = engine.weather().night_color;
    let global_shadow = host.frame_holder.global_shadow();
    let blip_shadow = host.frame_holder.global_blip_shadow();
    // When the player has disabled "Display Animations" in the graphics
    // options, unforced non-patched non-elevated non-masked FX should
    // not render.  The flag defaults to `true` so the live datadir is
    // unaffected; it only bites when the user toggles it off in the
    // options menu.
    let graphic_config = host
        .application_context
        .active_profile_snapshot()
        .unwrap_or_else(|error| panic!("entity rendering requires an active profile: {error}"))
        .graphic_config;
    let display_anim = graphic_config.display_anim;
    let apply_fog_to_all_sprites = graphic_config.apply_fog_to_all_sprites;

    // Clone ids (cheap: `Vec<EntityId>` of u32s) so the iteration borrow
    // doesn't conflict with the `&mut host` we hand to `render_up_to`.
    let draw_order_ids = host.draw_order.ids.clone();
    for &entity_id in &draw_order_ids {
        let entity = match engine.get_entity(entity_id) {
            Some(e) => e,
            None => continue,
        };
        if !entity.is_active() || entity.element_data().hidden_in_building {
            continue;
        }
        // FX entities early-return when `is_to_be_displayed` is false;
        // non-FX kinds always pass.
        if !entity.is_to_be_displayed(display_anim) {
            continue;
        }
        let variant = engine.resolve_render_variant(entity, apply_fog_to_all_sprites);

        // ── Interleave titbits that belong behind this entity ─────
        // Immediately before drawing each human entity, flush any
        // pending titbits whose depth falls behind this entity's so
        // they render back-to-front with the entity list (projectile /
        // dust / stars sit between actors at the correct depth instead
        // of piled on top at the end).
        if entity.is_human()
            && let Some(entity_depth) = host.draw_order.depth(entity_id)
        {
            titbit_renderer.render_up_to(host, engine, assets, renderer, entity_depth);
        }

        let elem = entity.element_data();
        let visual_pos = entity_visual_map_position(entity);
        let world_x = visual_pos.x;
        let world_y = visual_pos.y;
        if world_x == 0.0 && world_y == 0.0 {
            continue;
        }

        let screen_x = ((world_x - view.x) * zoom) as i32;
        let screen_y = ((world_y - view.y) * zoom) as i32;

        let margin = 256;
        if screen_x < -margin
            || screen_y < -margin
            || screen_x > screen_w + margin
            || screen_y > screen_h + margin
        {
            continue;
        }

        // Try GPU sprite rendering.
        //
        // Using `current_scripts_opt` (not a direct `.scripts` field
        // read) is essential for blipped NPCs: `load_frame_info` stores
        // the normal character as primary + `blip00` as alternate, and
        // flips `use_alternate_profile` so the blip silhouette is the
        // active profile until reveal flips it back.  A direct
        // field read would show the revealed character even while it
        // should still be a shadow.  Always go through the active-
        // profile pointer.
        let sprite = &elem.sprite;
        let scripts = match sprite.current_scripts_opt() {
            Some(s) => s,
            None => {
                render_entity_fallback(
                    renderer,
                    entity.kind(),
                    screen_x,
                    screen_y,
                    screen_w,
                    screen_h,
                );
                continue;
            }
        };

        let row = sprite.current_row;
        let frame = sprite.current_frame;
        if row as usize >= scripts.len() {
            render_entity_fallback(
                renderer,
                entity.kind(),
                screen_x,
                screen_y,
                screen_w,
                screen_h,
            );
            continue;
        }
        let script = &scripts[row as usize];
        if frame as usize >= script.frame_ids.len() {
            render_entity_fallback(
                renderer,
                entity.kind(),
                screen_x,
                screen_y,
                screen_w,
                screen_h,
            );
            continue;
        }
        let bank_id = script.frame_ids[frame as usize];

        // Blipped (undiscovered) NPCs render from the `blip00`
        // alternate profile as a silhouette sprite; the alpha-keying
        // pass uses the global blip shadow (60) for this branch vs the
        // global shadow (40) for normal characters.
        let mut shadow_level = if sprite.use_alternate_profile {
            blip_shadow
        } else {
            global_shadow
        };
        // FX entities switch on `rendering_properties`: `NeedShadow`
        // composites a shadow, `Blocky` doesn't.  Zero `shadow_level`
        // for `Blocky` FX so the cached sprite key drops the shadow
        // tint.
        if matches!(entity.kind(), ElementKind::Fx)
            && let Some(fx) = entity.fx_data()
            && fx.rendering_properties == RenderingProperties::Blocky
        {
            shadow_level = 0;
        }

        if let Some((sw, sh)) = renderer.ensure_sprite_cached(
            &host.frame_holder,
            bank_id,
            variant,
            shadow_color,
            shadow_level,
        ) {
            // Sprite screen position:
            //   sprite_pos  = floor(position_map - sprite.center)
            //   blit_origin = sprite_pos + script_offset
            //   screen_xy   = (blit_origin - view) * zoom
            // The floor() in world space (before zoom) is critical for
            // pixel-perfect alignment.
            let center = &sprite.center;
            let offset = script.offsets[frame as usize];
            let sprite_x = (world_x - center.x).floor() + offset.x;
            let sprite_y = (world_y - center.y).floor() + offset.y;
            let dst_x = ((sprite_x - view.x) * zoom) as i32;
            let dst_y = ((sprite_y - view.y) * zoom) as i32;

            let dst_rect = Rect::new(dst_x, dst_y, sw as u32, sh as u32);
            let kind = entity.kind();
            let actor_layer = elem.layer();
            let is_flying_human = elem.posture == Posture::Flying;
            let hidden_outline_rgb = if host.input.draw_hidden {
                // Ground objects always use Hidden; actors retain their active
                // targeting/parrying outline just like the original path.
                let color_565 = if matches!(
                    kind,
                    robin_engine::element::ElementKind::ObjectBonus
                        | robin_engine::element::ElementKind::ObjectOther
                        | robin_engine::element::ElementKind::ObjectScroll
                ) {
                    elem.outline_colors[OutlineColorName::Hidden as usize]
                } else {
                    elem.active_outline_color()
                };
                (color_565 != 0).then(|| rgb565_to_rgb8(color_565))
            } else {
                None
            };

            // Cheat-teleport hulk-rebuild fade.  When
            // `teleport_counter > 0`, the PC is rendered TWICE: first
            // at `position_before_teleport` with alpha
            // `100 * counter / max_counter` (the vanishing ghost),
            // then at the current position with alpha
            // `100 - 100 * counter / max_counter` (the appearing
            // sprite).  As the counter ticks down 20→0 the ghost
            // fades out and the new sprite fades in.  The per-frame
            // decrement is done in `pre_render_engine_setup` via
            // `EngineInner::tick_pc_teleport_fades`.
            let teleport_fade = entity.pc_data().and_then(|pc| {
                if pc.teleport_counter > 0 && pc.max_teleport_counter > 0 {
                    let ratio = pc.teleport_counter as f32 / pc.max_teleport_counter as f32;
                    let old_alpha_255 = (ratio * 255.0).round().clamp(0.0, 255.0) as u8;
                    let new_alpha_255 = ((1.0 - ratio) * 255.0).round().clamp(0.0, 255.0) as u8;
                    Some((pc.position_before_teleport, old_alpha_255, new_alpha_255))
                } else {
                    None
                }
            });

            if let Some((before, old_alpha, _new_alpha)) = teleport_fade {
                // Render the vanishing ghost at the pre-teleport
                // position first, so the appearing sprite stacks on
                // top.
                let ghost_x = (before.x - center.x).floor() + offset.x;
                let ghost_y = (before.y - center.y).floor() + offset.y;
                let ghost_dst_x = ((ghost_x - view.x) * zoom) as i32;
                let ghost_dst_y = ((ghost_y - view.y) * zoom) as i32;
                let ghost_rect = Rect::new(ghost_dst_x, ghost_dst_y, sw as u32, sh as u32);
                let ghost_draw_checkpoint = renderer.draw_queue_checkpoint();
                renderer.render_cached_sprite_alpha(
                    bank_id,
                    variant,
                    shadow_color,
                    shadow_level,
                    ghost_rect,
                    old_alpha,
                );
                let ghost_world_bbox = engine_coordinates::MapBBox::from_coords(
                    ghost_x,
                    ghost_y,
                    ghost_x + sw as f32,
                    ghost_y + sh as f32,
                );
                let current_world = elem.position();
                let ghost_world = engine_coordinates::WorldPoint3D::new(
                    before.x,
                    before.y + current_world.z,
                    current_world.z,
                );
                let ghost_mask_indices = applicable_sprite_masks(
                    engine,
                    assets,
                    actor_layer,
                    &ghost_world_bbox,
                    before,
                    ghost_world,
                    is_flying_human,
                    is_flying_human,
                );
                let ghost_screen_masks =
                    sprite_screen_masks(engine, &ghost_mask_indices, view, zoom);
                renderer.mask_queued_draws(ghost_draw_checkpoint, &ghost_screen_masks, ghost_rect);
                if let Some(rgb) = hidden_outline_rgb {
                    for &(mask_idx, mask_rect) in &ghost_screen_masks {
                        let mask = &engine.fast_grid().level.masks[mask_idx as usize];
                        renderer.render_hidden_mask_outline(
                            &host.frame_holder,
                            bank_id,
                            variant,
                            shadow_color,
                            &mask.bitmap,
                            mask.width,
                            mask.height,
                            mask_rect,
                            ghost_rect,
                            rgb,
                        );
                    }
                }
            }

            // The teleport ghost above is masked independently at its old
            // position; this checkpoint applies current-position masks only
            // to the appearing sprite.
            let sprite_draw_checkpoint = renderer.draw_queue_checkpoint();

            // When the GoldenEye cheat is on, every PC sprite is
            // composited at 50% alpha (~128/255 in 8-bit).  Teleport
            // fade takes precedence — these are `else if` siblings.
            if let Some((_, _, new_alpha)) = teleport_fade {
                renderer.render_cached_sprite_alpha(
                    bank_id,
                    variant,
                    shadow_color,
                    shadow_level,
                    dst_rect,
                    new_alpha,
                );
            } else if entity.is_pc() && engine.get_golden_eye_mode() {
                renderer.render_cached_sprite_alpha(
                    bank_id,
                    variant,
                    shadow_color,
                    shadow_level,
                    dst_rect,
                    128,
                );
            } else {
                renderer.render_cached_sprite(
                    bank_id,
                    variant,
                    shadow_color,
                    shadow_level,
                    dst_rect,
                );
            }

            // ── Sprite occlusion masks ──
            //
            // After drawing the sprite, ask the grid for any building
            // masks that apply to this actor's position + layer, then
            // blit each mask's pre-composed background texture on top
            // of the sprite.  Where the mask is set the building
            // pixels reappear in front of the actor; elsewhere the
            // texture is transparent and the sprite stays visible.
            let sprite_world_bbox = engine_coordinates::MapBBox::from_coords(
                sprite_x,
                sprite_y,
                sprite_x + sw as f32,
                sprite_y + sh as f32,
            );
            let actor_position = engine_coordinates::MapPoint::new(world_x, world_y);
            // The mask lookup switches between
            // `get_masks_applied_to_character` and
            // `get_masks_applied_to_projectile` based on the masking
            // category.  PCs override to flying-human masking when
            // their posture is `Flying` so a PC mid-jump no longer
            // gets clipped by the building it's soaring over.  Arrows,
            // thrown bonuses and nets (`ElementKind::ObjectProjectile`
            // / `ObjectNet`) use the projectile masking category so
            // they route through the projectile polyline + 3D
            // altitude test, not the character polyline.
            // The mask pass is gated on `has_valid_box_for_masking`.
            // FX / target overlays never set the flag, so they render
            // without building-mask occlusion.  Flying humans use the
            // original projectile/flying-human mask path.
            if !kind.has_valid_box_for_masking() && !is_flying_human {
                // Nothing more to do: sprite is drawn, no mask pass.
                continue;
            }
            let use_projectile_path = is_flying_human || kind.is_projectile();
            let projectile_mask_position =
                transition_crenel_climb_up_mask_position(entity, engine, assets)
                    .unwrap_or_else(|| elem.position());
            let mask_indices = applicable_sprite_masks(
                engine,
                assets,
                actor_layer,
                &sprite_world_bbox,
                actor_position,
                projectile_mask_position,
                use_projectile_path,
                is_flying_human,
            );
            // When `draw_hidden` is on, the original mutates the
            // temporary sprite surface per mask: masked pixels become
            // transparent, except horizontal transparent/body edges
            // become the actor's outline colour. Stencil rejection does the
            // transparency part; the hidden outline pass restores those edge
            // pixels.
            let screen_masks = sprite_screen_masks(engine, &mask_indices, view, zoom);
            if use_projectile_path {
                renderer.mask_queued_draws(sprite_draw_checkpoint, &screen_masks, dst_rect);
            } else {
                renderer.mask_queued_draws_with_depth(
                    sprite_draw_checkpoint,
                    &screen_masks,
                    dst_rect,
                    view.x,
                    view.y,
                    zoom,
                    projectile_mask_position.y,
                );
            }

            for &(mask_idx, mask_rect) in &screen_masks {
                let mask = &engine.fast_grid().level.masks[mask_idx as usize];
                if let Some(rgb) = hidden_outline_rgb {
                    renderer.render_hidden_mask_outline(
                        &host.frame_holder,
                        bank_id,
                        variant,
                        shadow_color,
                        &mask.bitmap,
                        mask.width,
                        mask.height,
                        mask_rect,
                        dst_rect,
                        rgb,
                    );
                }
            }
            if dev.debug.sprite_masks_display {
                render_sprite_mask_debug_overlay(
                    host,
                    engine,
                    renderer,
                    &sprite_world_bbox,
                    actor_position,
                    projectile_mask_position,
                    use_projectile_path,
                    &mask_indices,
                );
            }
        } else {
            render_entity_fallback(
                renderer,
                entity.kind(),
                screen_x,
                screen_y,
                screen_w,
                screen_h,
            );
        }
    }
}

fn transition_crenel_climb_up_mask_position(
    entity: &robin_engine::element::Entity,
    engine: &Engine,
    assets: &LevelAssets,
) -> Option<engine_coordinates::WorldPoint3D> {
    use robin_engine::order::OrderType;

    let elem = entity.element_data();
    if elem.sprite.last_action != OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
        || elem.sprite.current_frame != 0
    {
        return None;
    }
    let actor = entity.actor_data()?;
    let door_pass = actor.active_door_pass.as_ref()?;
    if door_pass.current_action != OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel {
        return None;
    }
    engine.mission_script()?;
    let door = engine.doors().get(usize::from(door_pass.door_index))?;
    let point_mid = door.point_mid;
    let point_out = door.point_out;

    // C++ applies the high-crenel transition projection at action-done:
    // SetPositionMap(point_mid), SetObstacleAndMaterial(point_out projection
    // area), SetOldPositionMap(point_out), then ComputePositionAll().
    // Frame 0 is visually still anchored at the pre-snap map point so its
    // offset lines up with frame 1, but the flying-human mask decision must
    // already use the far-side projection or the wall projectile masks erase
    // the whole frame.
    let mut best_z: Option<f32> = None;
    for obs in engine.sight_obstacles(assets).iter() {
        if !obs.is_projection_area()
            || obs.layer != door.layer_out
            || obs.sector != u16::from(door.sector_out)
            || !obs.contains_point_projection(point_out)
        {
            continue;
        }
        let z = obs.compute_top_z(point_mid.x, point_mid.y);
        best_z = Some(best_z.map_or(z, |old| old.max(z)));
    }
    let z = best_z?;
    Some(engine_coordinates::WorldPoint3D {
        x: point_mid.x,
        y: point_mid.y + z,
        z,
    })
}

#[allow(clippy::too_many_arguments)]
fn render_sprite_mask_debug_overlay(
    host: &Host,
    engine: &Engine,
    renderer: &mut Renderer,
    sprite_world_bbox: &engine_coordinates::MapBBox,
    actor_position: engine_coordinates::MapPoint,
    position_3d: engine_coordinates::WorldPoint3D,
    use_projectile_path: bool,
    mask_indices: &[engine_mask::MaskIndex],
) {
    if mask_indices.is_empty() && !use_projectile_path {
        return;
    }

    let sprite_color = if mask_indices.is_empty() {
        0x07ff
    } else {
        0xf81f
    };
    draw_map_bbox_outline(host, renderer, sprite_world_bbox, sprite_color);

    for &mask_idx in mask_indices {
        let mask = &engine.fast_grid().level.masks[usize::from(mask_idx)];
        draw_map_bbox_outline(host, renderer, &mask.bbox, 0xffe0);
    }

    draw_map_cross(host, renderer, actor_position, 0x07e0);
    if use_projectile_path {
        let projectile_test_point = position_3d.to_map();
        let actor_screen = map_to_screen(host, actor_position);
        let projectile_screen = map_to_screen(host, projectile_test_point);
        renderer.draw_line_screen(
            actor_screen.0,
            actor_screen.1,
            projectile_screen.0,
            projectile_screen.1,
            0xfd20,
        );
        draw_map_cross(host, renderer, projectile_test_point, 0xfd20);
    }
}

fn draw_map_bbox_outline(
    host: &Host,
    renderer: &mut Renderer,
    bbox: &engine_coordinates::MapBBox,
    color: u16,
) {
    if !bbox.is_somewhere() {
        return;
    }
    let (x1, y1) = map_to_screen(
        host,
        engine_coordinates::MapPoint::new(bbox.x_min(), bbox.y_min()),
    );
    let (x2, y2) = map_to_screen(
        host,
        engine_coordinates::MapPoint::new(bbox.x_max(), bbox.y_max()),
    );
    renderer.draw_rect_outline_screen(x1, y1, x2, y2, color);
}

fn draw_map_cross(
    host: &Host,
    renderer: &mut Renderer,
    point: engine_coordinates::MapPoint,
    color: u16,
) {
    let (x, y) = map_to_screen(host, point);
    renderer.draw_line_screen(x - 4, y, x + 4, y, color);
    renderer.draw_line_screen(x, y - 4, x, y + 4, color);
}

fn map_to_screen(host: &Host, point: engine_coordinates::MapPoint) -> (i32, i32) {
    let view = host.viewport.view_position;
    let zoom = host.viewport.zoom_factor;
    (
        ((point.x - view.x) * zoom).round() as i32,
        ((point.y - view.y) * zoom).round() as i32,
    )
}

// ─── GPU selection outline pass ──────────────────────────────────

/// Render coloured outlines for selected PCs and the hovered entity.
///
/// The selection-outline pass runs after all entity sprites are drawn
/// so the outline is drawn ON TOP of entities and is never occluded.
///
/// For each outlined entity, the cached outline mask texture is tinted and
/// alpha-modulated by the GPU pipeline (for hulk fade animation).
pub(crate) fn render_selection_outlines_gpu(
    host: &mut Host,
    engine: &Engine,
    _assets: &LevelAssets,
    renderer: &mut Renderer,
) {
    let view = host.viewport.view_position;
    let zoom = host.viewport.zoom_factor;
    let screen_w = host.viewport.screen_size.x as i32;
    let screen_h = host.viewport.screen_size.y as i32;
    let shadow_color = engine.weather().night_color;
    let shadow_level = host.frame_holder.global_shadow();
    let apply_fog_to_all_sprites = host
        .application_context
        .active_profile_snapshot()
        .unwrap_or_else(|error| panic!("outline rendering requires an active profile: {error}"))
        .graphic_config
        .apply_fog_to_all_sprites;

    // Clone ids (cheap) to sidestep borrow conflict with `&mut host`.
    let draw_order_ids = host.draw_order.ids.clone();
    for &entity_id in &draw_order_ids {
        let entity = match engine.get_entity(entity_id) {
            Some(e) => e,
            None => continue,
        };
        if !entity.is_active() || entity.element_data().hidden_in_building {
            continue;
        }
        let variant = engine.resolve_render_variant(entity, apply_fog_to_all_sprites);

        let elem = entity.element_data();

        // The outline is blitted only when the PC is mouse-marked or
        // its `running_hulk` is positive.  The selection set on its
        // own does NOT draw the outline — `refresh_pc_selection_hulk`
        // seeds `running_hulk` on the first frame of selection and
        // decrements it each tick, so the glow naturally fades from
        // 100 down to 40 over `HULK_LENGTH` frames and then vanishes.
        //
        // `is_focused` stands in for the mouse-hover mark.
        // `is_action_marked` covers the requirement-bar action flag,
        // which marks every PC matching the hovered action.  Either
        // forces `hulk_level = 100` for one frame.
        let is_focused = host.input.focused_entity_id == Some(entity_id);
        let is_action_marked = host.input.marked_pc_ids.contains(&entity_id);
        let hulk_running = entity.human_data().is_some_and(|h| h.running_hulk > 0);

        if !is_focused && !is_action_marked && !hulk_running {
            continue;
        }

        let is_selected_tactical = host.control_tactical_units
            && engine
                .tactical_selection(host.transport.local_seat)
                .contains(&entity_id);
        let outline_color_565 = if is_selected_tactical && hulk_running && !is_focused {
            robin_engine::element_kinds::outline_colors::pc_default()
        } else if is_focused || is_action_marked {
            elem.outline_colors[OutlineColorName::Default as usize]
        } else {
            elem.active_outline_color()
        };
        if outline_color_565 == 0 {
            continue;
        }

        // Alpha: focused/marked/action-marked force 100 (override any
        // in-flight fade); otherwise use `hulk_level` (40..=100) from
        // the fade state machine. The percentage (0-100) is converted
        // to the renderer's 0-255 alpha range.
        let alpha_pct = if is_focused || is_action_marked {
            100u16
        } else {
            entity.human_data().map(|h| h.hulk_level).unwrap_or(100)
        };
        let alpha_255 = ((alpha_pct as u32) * 255 / 100).min(255) as u8;

        // Resolve sprite frame (same calculation as render_entities_gpu).
        // See note there about `current_scripts_opt` vs direct field read.
        let sprite = &elem.sprite;
        let scripts = match sprite.current_scripts_opt() {
            Some(s) => s,
            None => continue,
        };
        let row = sprite.current_row;
        let frame = sprite.current_frame;
        if row as usize >= scripts.len() {
            continue;
        }
        let script = &scripts[row as usize];
        if frame as usize >= script.frame_ids.len() {
            continue;
        }
        let bank_id = script.frame_ids[frame as usize];

        // Screen position. Use the same visual anchor as sprite rendering
        // so hover outlines stay aligned with targets and airborne actors.
        let visual_pos = entity_visual_map_position(entity);
        let world_x = visual_pos.x;
        let world_y = visual_pos.y;
        let screen_x = ((world_x - view.x) * zoom) as i32;
        let screen_y = ((world_y - view.y) * zoom) as i32;
        let margin = 256;
        if screen_x < -margin
            || screen_y < -margin
            || screen_x > screen_w + margin
            || screen_y > screen_h + margin
        {
            continue;
        }

        // PositionSprite calculation (same as render_entities_gpu).
        let center = &sprite.center;
        let offset = script.offsets[frame as usize];
        let sprite_x = (world_x - center.x).floor() + offset.x;
        let sprite_y = (world_y - center.y).floor() + offset.y;
        let dst_x = ((sprite_x - view.x) * zoom) as i32;
        let dst_y = ((sprite_y - view.y) * zoom) as i32;

        if let Some((ow, oh)) = renderer.ensure_outline_cached(
            &host.frame_holder,
            bank_id,
            variant,
            shadow_color,
            shadow_level,
        ) {
            let rgb = rgb565_to_rgb8(outline_color_565);
            let outline_x = dst_x - OUTLINE_PAD as i32;
            let outline_y = dst_y;
            let outline_rect = Rect::new(outline_x, outline_y, ow as u32, oh as u32);
            renderer.render_cached_outline(
                bank_id,
                variant,
                shadow_color,
                shadow_level,
                outline_rect,
                rgb,
                alpha_255,
            );
        }
    }
}

/// Fallback: draw a colored rectangle for entities without sprites.
fn render_entity_fallback(
    renderer: &mut Renderer,
    kind: robin_engine::element::ElementKind,
    screen_x: i32,
    screen_y: i32,
    screen_w: i32,
    screen_h: i32,
) {
    use robin_engine::element::ElementKind;

    let (r, g, b): (u8, u8, u8) = match kind {
        ElementKind::ActorPc => (0, 255, 0),
        ElementKind::ActorSoldier => (255, 0, 0),
        ElementKind::ActorCivilian => (0, 0, 255),
        ElementKind::Fx => (255, 224, 0),
        ElementKind::Target => (255, 0, 255),
        ElementKind::ObjectBonus => (0, 255, 255),
        _ => (255, 255, 255),
    };

    let half = 4;
    let x = (screen_x - half).max(0);
    let y = (screen_y - half).max(0);
    let w = ((screen_x + half).min(screen_w) - x).max(0);
    let h = ((screen_y + half).min(screen_h) - y).max(0);
    if w > 0 && h > 0 {
        renderer.render_gpu_rect(x, y, w, h, r, g, b, 255);
    }
}

// ─── Background animation rendering ──────────────────────────────────

/// Render background animations (elevation-0 FX) as GPU sprites.
///
/// Iterates the background-animations list and renders them BEFORE
/// the main entity loop.  Background animations are excluded from
/// `display_order` by `sort_for_display`, so we render them in a
/// dedicated pass here.
///
/// Must be called after `flush_base_layer` (GPU phase active) and before
/// `render_entities_gpu`.
pub(crate) fn render_bg_animations_gpu(
    engine: &Engine,
    host: &Host,
    _assets: &LevelAssets,
    renderer: &mut Renderer,
) {
    let bg_animation_ids = engine.bg_animation_ids();
    if bg_animation_ids.is_empty() {
        return;
    }
    render_fx_entities_gpu(bg_animation_ids.iter().copied(), engine, host, renderer);
}

fn render_fx_entities_gpu<I>(entity_ids: I, engine: &Engine, host: &Host, renderer: &mut Renderer)
where
    I: IntoIterator<Item = engine_element::EntityId>,
{
    let view = host.viewport.view_position;
    let zoom = host.viewport.zoom_factor;
    let screen_w = host.viewport.screen_size.x as i32;
    let screen_h = host.viewport.screen_size.y as i32;
    let shadow_color = engine.weather().night_color;
    let global_shadow = host.frame_holder.global_shadow();

    // Bg animations are unforced ground-level non-masked FX, so they
    // are suppressed when the player has disabled "Display Animations"
    // unless `force_display` or `patch_index` overrides.  See
    // `render_entities_gpu` for the full gate; identical logic via
    // `Entity::is_to_be_displayed`.
    let graphic_config = host
        .application_context
        .active_profile_snapshot()
        .unwrap_or_else(|error| {
            panic!("background animation rendering requires an active profile: {error}")
        })
        .graphic_config;
    let display_anim = graphic_config.display_anim;
    let apply_fog_to_all_sprites = graphic_config.apply_fog_to_all_sprites;

    for entity_id in entity_ids {
        let entity = match engine.get_entity(entity_id) {
            Some(e) => e,
            None => continue,
        };
        if !entity.is_active() {
            continue;
        }
        if !entity.is_to_be_displayed(display_anim) {
            continue;
        }
        let variant = engine.resolve_render_variant(entity, apply_fog_to_all_sprites);

        let elem = entity.element_data();
        let sprite = &elem.sprite;
        let scripts = match sprite.current_scripts_opt() {
            Some(s) => s,
            None => continue,
        };

        let row = sprite.current_row;
        let frame = sprite.current_frame;
        if row as usize >= scripts.len() {
            continue;
        }
        let script = &scripts[row as usize];
        if frame as usize >= script.frame_ids.len() {
            continue;
        }
        let bank_id = script.frame_ids[frame as usize];

        let world_x = elem.position_map().x;
        let world_y = elem.position_map().y;
        if world_x == 0.0 && world_y == 0.0 {
            continue;
        }

        let margin = 256;
        let screen_x = ((world_x - view.x) * zoom) as i32;
        let screen_y = ((world_y - view.y) * zoom) as i32;
        if screen_x < -margin
            || screen_y < -margin
            || screen_x > screen_w + margin
            || screen_y > screen_h + margin
        {
            continue;
        }

        // FX entities composite a shadow when `rendering_properties`
        // is `NeedShadow`, and skip it for `Blocky`.  Zero
        // `shadow_level` for `Blocky` FX.
        let shadow_level = match entity.fx_data() {
            Some(fx) if fx.rendering_properties == RenderingProperties::Blocky => 0,
            _ => global_shadow,
        };

        if let Some((sw, sh)) = renderer.ensure_sprite_cached(
            &host.frame_holder,
            bank_id,
            variant,
            shadow_color,
            shadow_level,
        ) {
            let center = &sprite.center;
            let offset = script.offsets[frame as usize];
            let sprite_x = (world_x - center.x).floor() + offset.x;
            let sprite_y = (world_y - center.y).floor() + offset.y;
            let dst_x = ((sprite_x - view.x) * zoom) as i32;
            let dst_y = ((sprite_y - view.y) * zoom) as i32;

            let dst_rect = Rect::new(dst_x, dst_y, sw as u32, sh as u32);
            renderer.render_cached_sprite(bank_id, variant, shadow_color, shadow_level, dst_rect);
        }
    }
}

/// Renderer-path wrapper around [`crate::hud_text::render_text_background`]
/// for the ransom/amulet overlay and dev noise labels.  Routes the
/// shadow+foreground pass through `Renderer::render_text_argb` instead of
/// the old HUD surface-raster path.
fn render_text_with_shadow(renderer: &mut Renderer, fonts: &HudFonts, text: &str, x: i32, y: i32) {
    hud_text::render_text_background(
        &fonts.tooltip_font,
        fonts.shadow_font.as_ref(),
        text,
        x,
        y,
        |f, t, fx, fy| {
            layout::render_text_screen(renderer, f, t, fx, fy);
        },
    );
}
