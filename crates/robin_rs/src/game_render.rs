//! In-game rendering passes for the mission loop.
//!
//! Contains the GPU-phase rendering functions: entity sprites, selection
//! outlines, ground marks, ambiance overlays, and minimap.  The in-game
//! menu rendering lives in [`crate::ingame_menu`] and is driven by
//! [`crate::game_session`].

use crate::Host;
use crate::campaign::CampaignValue;
use crate::element::{
    ElementKind, Entity, GameMaterial, ListenPhase, OutlineColorName, Posture, RenderingProperties,
};
use crate::geo2d::{self, BBox2D};
use crate::gfx_types::Rect;
use crate::hud_text::{self, HudFonts};
use crate::ingame_menu::layout;
use crate::ingame_menu::resources::{IngameMenuResources, MT_STR_AMULETS, MT_STR_RANSOM};
use crate::minimap::UIState;
use crate::player_command::PlayerCommand;
use crate::player_profile::PlayerProfileManager;
use crate::renderer::{BLIT_SOURCE_TRANSPARENT, OUTLINE_PAD, Renderer, rgb565_to_rgb8};
use crate::titbit_renderer::TitbitRenderer;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::coordinates::MapPoint;
use robin_engine::element as engine_element;
use robin_engine::engine as engine_api;
use robin_engine::engine::{Ambiance, DevState, Engine, LevelAssets, MULTI_SELECTION_THRESHOLD};
use robin_engine::geo2d as engine_geo2d;
use robin_engine::markers::GroundMark;
use robin_engine::mask as engine_mask;
use robin_engine::minimap as engine_minimap;
use robin_engine::pathfinder as engine_pathfinder;
use robin_engine::position_interface as engine_position_interface;
use robin_engine::sector as engine_sector;
use robin_engine::sight_obstacle as engine_sight_obstacle;
use robin_engine::sprite::BBox;

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
///     patch (the patch-driven draw happens below via
///     `Patch::display_doors`).
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
    use crate::element::Posture;
    use crate::gate::DoorType;
    use crate::geo2d::pt;
    use crate::profiles::Action;
    use crate::sector::SectorType;

    let Some(game_host) = engine.mission_script().and_then(|m| m.game_host()) else {
        return;
    };

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

    let draw_door = |renderer: &mut Renderer, door: &crate::gate::Door| {
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
                             sector: &crate::fast_find_grid::GridSector,
                             require_active: bool| {
        for &gate_idx in &sector.gate_indices {
            let Some(door) = game_host.doors.get(usize::from(gate_idx)) else {
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

    let sector_by_number = |sector_num: i16| -> Option<&crate::fast_find_grid::GridSector> {
        let &idx = engine
            .fast_grid()
            .level
            .sector_number_map
            .get(&engine_sector::SectorNumber::new(sector_num))?;
        engine.fast_grid().level.sectors.get(idx)
    };

    // ── 1. Selected PCs inside buildings (runs unconditionally) ──
    let local_seat = host.local_seat;
    for &pc_id in engine.seat_selection(local_seat) {
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
        for door in game_host.doors.iter() {
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
        for patch in game_host.patches.iter() {
            for &door_idx in &patch.door_indices {
                if let Some(door) = game_host.doors.get(door_idx as usize) {
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
        .seat_selection(local_seat)
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
        && let Some(door) = game_host.doors.get(door_idx as usize)
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

    if let Some(sector) = selected_sector {
        if host.input.display_door
            && sector.sector_type.is_door()
            && let Some(door_idx) = sector.door_index
            && let Some(door) = game_host.doors.get(door_idx as usize)
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
                        // otherwise the patch-FX path handles it via
                        // `Patch::display_doors`, which is set below.
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
            for &pc_id in engine.seat_selection(local_seat) {
                if !engine.selected_pc_has_contextual_action(assets, Some(pc_id), Action::Jump) {
                    continue;
                }
                let pc_pos = engine
                    .get_entity(pc_id)
                    .map(|e| {
                        let p = e.element_data().position_map();
                        pt(p.x, p.y)
                    })
                    .unwrap_or(pt(0.0, 0.0));
                paint = engine
                    .get_nearest_jumpable_jump_line(
                        pc_id,
                        pc_pos,
                        pt(0.0, 0.0),
                        /* test_posture */ false,
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
    //    `Patch::display_doors` is refreshed each frame in `update_mouse`
    //    (cleared on every patch, set on the hovered one).
    for patch in game_host.patches.iter() {
        if !patch.display_doors {
            continue;
        }

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
            let Some(door) = game_host.doors.get(door_idx as usize) else {
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

// ─── Ambiance screen overlay ───────────────────────────────────────

/// Apply a night or fog color tint to the entire screen surface.
///
/// - Night: darkens scene with a blue-ish tint at 50% intensity
/// - Fog: blends toward white at 60% intensity
/// - Day: no overlay
///
/// The same math used for per-sprite effects in `frame_holder.rs` is applied
/// to the composited screen buffer, giving a consistent visual result even
/// for elements that bypass the sprite variant system (e.g. debug rectangles,
/// selection highlights drawn before this pass, the background map which is
/// already loaded from the ambiance-specific directory).
pub(crate) fn apply_ambiance_overlay(engine: &Engine, renderer: &mut Renderer) {
    use robin_assets::frame_holder::{
        FOG_COLOR, FOG_INTENSITY, NIGHT_FOG_COLOR_16, NIGHT_INTENSITY,
    };

    let (level, fog_color) = match engine.weather().ambiance {
        Ambiance::Night => (NIGHT_INTENSITY, NIGHT_FOG_COLOR_16),
        Ambiance::Fog => (FOG_INTENSITY, FOG_COLOR),
        _ => return, // Day/Storm: no overlay
    };

    // GPU path: draw a fullscreen semi-transparent rectangle.
    // The blend formula `screen = fog * alpha + screen * (1 - alpha)` is
    // equivalent to the per-pixel `apply_fog_effect_viewport` math, with
    // slightly better precision (8-bit vs 5/6/5 channels).
    let (r, g, b) = rgb565_to_rgb8(fog_color);
    let alpha = ((100u16 - level) * 255 / 100) as u8;
    let w = renderer.screen_width() as i32;
    let h = renderer.screen_height() as i32;
    renderer.render_gpu_rect(0, 0, w, h, r, g, b, alpha);
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
            geo2d::pt(pos.x, pos.y),
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
    let view_rect = BBox::new(
        host.viewport.view_position.to_geo(),
        geo2d::pt(
            host.viewport.view_position.x
                + (host.viewport.screen_size.x - 1.0) / host.viewport.zoom_factor,
            host.viewport.view_position.y
                + (host.viewport.screen_size.y - PANNEL_HEIGHT + 1.0) / host.viewport.zoom_factor,
        ),
    );

    let alpha = params.alpha.min(crate::shadow_polygon::alpha_for_ambiance(
        engine.weather().ambiance == Ambiance::Night || engine.weather().ambiance == Ambiance::Fog,
    ));

    let tint = tint.unwrap_or((0, 0, 0));

    // Collect character masks whose world-space bbox intersects the view
    // rect — these building silhouettes clear the tint inside the cone
    // in `render_darken_inside_gpu`'s mask post-pass.
    let view_bbox = BBox2D::from_coords(
        view_rect.min.x,
        view_rect.min.y,
        view_rect.max.x,
        view_rect.max.y,
    );
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
                && m.bbox.intersects_bbox(&view_bbox)
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
    polys: Vec<Vec<crate::geo2d::GeoPoint2D>>,
    viewer: crate::geo2d::GeoPoint2D,
    radius: f32,
    projection_plane: Option<engine_position_interface::PlaneZCoeffs>,
}

fn view_cone_polys_for_render(
    viewer: crate::geo2d::GeoPoint2D,
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

    let active_obstacles: Vec<(usize, &crate::sight_obstacle::SightObstacle)> = obstacles_view
        .iter_indexed()
        .filter_map(|(idx, o)| {
            let idx = idx as usize;
            obstacles_view.is_active(idx).then_some((idx, o))
        })
        .collect();

    let all_obstacles: Vec<&crate::sight_obstacle::SightObstacle> =
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
        let mut bbox = BBox2D::new();
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
        let obstacles: Vec<&crate::sight_obstacle::SightObstacle> = active_obstacles
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
        let occluding_projection_areas: Vec<&crate::sight_obstacle::SightObstacle> =
            active_obstacles
                .iter()
                .filter_map(|(idx, o)| {
                    (*idx != projection_idx
                        && o.is_projection_area()
                        && o.layer >= projection_area.layer
                        && o.box_screen.intersects_bbox(&projection_area.box_screen))
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
    viewer: crate::geo2d::GeoPoint2D,
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

/// Render the developer shadow-polygon sphere debug overlay when the
/// `shadow_polygon_sphere` cheat is active.
///
/// Reduced to the ground-ring slice — the 100-slice vertical stack is
/// omitted because it visualises viewer Z which is always 0 in the
/// current camera model.  A white ground ellipse of radius
/// `params.radius` is sufficient to match the gameplay-relevant debug
/// cue.
pub(crate) fn render_shadow_polygon_sphere_debug(
    host: &Host,
    engine: &Engine,
    selected_view_element: Option<engine_element::EntityId>,
    dev: &engine_api::DevState,
    renderer: &mut Renderer,
) {
    if !dev.debug.shadow_polygon_sphere {
        return;
    }
    let Some((viewer, params, _tint)) = engine.selected_view_cone_params(selected_view_element)
    else {
        return;
    };
    host.draw_manager.draw_ellipse(
        renderer,
        engine_coordinates::MapPoint::new(viewer.x, viewer.y),
        params.radius as u16,
        0xFFFF,
    );
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

    let view_rect = BBox::new(
        host.viewport.view_position.to_geo(),
        geo2d::pt(
            host.viewport.view_position.x
                + (host.viewport.screen_size.x - 1.0) / host.viewport.zoom_factor,
            host.viewport.view_position.y
                + (host.viewport.screen_size.y - PANNEL_HEIGHT + 1.0) / host.viewport.zoom_factor,
        ),
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
            let cone_bbox = BBox::new(
                geo2d::pt(viewer.x - r, viewer.y - z - r),
                geo2d::pt(viewer.x + r, viewer.y + r),
            );
            view_rect.is_intersecting(&cone_bbox)
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
                            let mut bbox = BBox::new(p[0], p[0]);
                            for &point in &p[1..] {
                                bbox.min.x = bbox.min.x.min(point.x);
                                bbox.min.y = bbox.min.y.min(point.y);
                                bbox.max.x = bbox.max.x.max(point.x);
                                bbox.max.y = bbox.max.y.max(point.y);
                            }
                            view_rect_for_filter.is_intersecting(&bbox)
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

        let src_box = BBox::new(geo2d::pt(0.0, 0.0), geo2d::pt(fw as f32, fh as f32));
        let dst_box = BBox::new(
            geo2d::pt(dst_x as f32, dst_y as f32),
            geo2d::pt((dst_x + scaled_w) as f32, (dst_y + scaled_h) as f32),
        );

        renderer.blit_with_shadow(
            surf_id,
            Some(&src_box),
            0,
            Some(&dst_box),
            shadow_color,
            shadow_level,
            BLIT_SOURCE_TRANSPARENT,
        );

        let mark_world_bbox = BBox2D::from_coords(
            mark.x + ox as f32,
            mark.y + oy as f32,
            mark.x + ox as f32 + fw as f32,
            mark.y + oy as f32 + fh as f32,
        );
        let mark_position = crate::geo2d::pt(
            mark.x + ox as f32 + fw as f32 * 0.5,
            mark.y + oy as f32 + fh as f32 * 0.5,
        );
        let mark_rect = Rect::new(dst_x, dst_y, scaled_w as u32, scaled_h as u32);
        render_character_masks_clipped(
            engine,
            renderer,
            mark.layer,
            &mark_world_bbox,
            engine_coordinates::MapPoint::from_geo(mark_position),
            mark_rect,
            view_pos.to_geo(),
            zoom,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn render_character_masks_clipped(
    engine: &Engine,
    renderer: &mut Renderer,
    layer: u16,
    world_bbox: &crate::geo2d::BBox2D,
    position: engine_coordinates::MapPoint,
    clip_rect: Rect,
    view: crate::geo2d::GeoPoint2D,
    zoom: f32,
) {
    for mask_idx in engine
        .fast_grid()
        .get_masks_applied_to_character(layer, world_bbox, position)
    {
        let mask = &engine.fast_grid().level.masks[usize::from(mask_idx)];
        let mask_screen_x = ((mask.bbox.x_min() - view.x) * zoom).round() as i32;
        let mask_screen_y = ((mask.bbox.y_min() - view.y) * zoom).round() as i32;
        let mask_screen_w = (mask.width as f32 * zoom).round() as u32;
        let mask_screen_h = (mask.height as f32 * zoom).round() as u32;
        if mask_screen_w == 0 || mask_screen_h == 0 {
            continue;
        }
        let mask_rect = Rect::new(mask_screen_x, mask_screen_y, mask_screen_w, mask_screen_h);
        renderer.render_cached_mask_clipped(u32::from(mask_idx), mask_rect, clip_rect);
    }
}

// ─── GPU entity rendering ─────────────────────────────────────────

/// Render all entities using cached GPU textures.
///
/// Replaces `render_entities` for the GPU phase.  Each sprite frame is
/// decompressed once and cached as an ARGB8888 GPU texture; subsequent
/// frames with the same `(bank_id, variant, shadow_color)` key reuse the
/// cached texture via `SDL_RenderCopy` (zero CPU decompression work).
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
    let display_anim = PlayerProfileManager::global()
        .as_ref()
        .and_then(|mgr| mgr.get_active())
        .map(|p| p.graphic_config.display_anim)
        .unwrap_or(true);

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
        if entity.fx_data().is_some_and(|fx| fx.patch_index.is_some()) {
            continue;
        }
        // FX entities early-return when `is_to_be_displayed` is false;
        // non-FX kinds always pass.
        if !entity.is_to_be_displayed(display_anim) {
            continue;
        }
        let variant = engine.resolve_render_variant(entity);

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
        let world_x = elem.position_map().x;
        // Airborne actors (mid line-jump) lift off the ground: draw
        // `position_map.y - jump_z_offset` so the sprite moves up the
        // screen as the character clears the gap.  Isometric
        // projection `screenY = mapY - z`.
        let jump_z = entity.actor_data().map(|a| a.jump_z_offset).unwrap_or(0.0);
        let world_y = elem.position_map().y - jump_z;
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
                renderer.render_cached_sprite_alpha(
                    bank_id,
                    variant,
                    shadow_color,
                    shadow_level,
                    ghost_rect,
                    old_alpha,
                );
            }

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
            let sprite_world_bbox = BBox2D::from_coords(
                sprite_x,
                sprite_y,
                sprite_x + sw as f32,
                sprite_y + sh as f32,
            );
            let actor_layer = elem.layer();
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
            let kind = entity.kind();
            // The mask pass is gated on `has_valid_box_for_masking`.
            // FX / target overlays never set the flag, so they render
            // without building-mask occlusion.  Flying humans use the
            // original projectile/flying-human mask path.
            let is_flying_human = elem.posture == Posture::Flying;
            if !kind.has_valid_box_for_masking() && !is_flying_human {
                // Nothing more to do: sprite is drawn, no mask pass.
                continue;
            }
            let use_projectile_path = is_flying_human || kind.is_projectile();
            let projectile_mask_position =
                transition_crenel_climb_up_mask_position(entity, engine, assets)
                    .unwrap_or_else(|| elem.position());
            let mask_indices = if use_projectile_path {
                // Pass the special layer (not the actor layer) for
                // projectile masking, since projectile masks live on
                // the synthetic top-of-all-layers special layer.
                engine.fast_grid().get_masks_applied_to_projectile(
                    engine.fast_grid().level.special_layer,
                    &sprite_world_bbox,
                    projectile_mask_position,
                    is_flying_human, // is_human — bottom-plane test
                    engine.sight_obstacles(assets),
                )
            } else {
                engine.fast_grid().get_masks_applied_to_character(
                    actor_layer,
                    &sprite_world_bbox,
                    actor_position,
                )
            };
            // When `draw_hidden` is on, the original mutates the
            // temporary sprite surface per mask: masked pixels become
            // transparent, except horizontal transparent/body edges
            // become the actor's outline colour. The background mask
            // pass below does the transparency part; the hidden
            // outline pass restores those edge pixels.
            let draw_hidden = host.input.draw_hidden;
            let hidden_outline_rgb = if draw_hidden {
                // Objects/bonuses hardcode the Hidden outline color
                // regardless of any "current outline" state — there is
                // no targeting concept on a ground-lying object.
                // Actors (PCs, NPCs) use the active outline so their
                // target/striking/parrying tints render correctly.
                let color_565 = if matches!(
                    kind,
                    crate::element::ElementKind::ObjectBonus
                        | crate::element::ElementKind::ObjectOther
                        | crate::element::ElementKind::ObjectScroll
                ) {
                    elem.outline_colors[OutlineColorName::Hidden as usize]
                } else {
                    elem.active_outline_color()
                };
                (color_565 != 0).then(|| rgb565_to_rgb8(color_565))
            } else {
                None
            };
            for &mask_idx in &mask_indices {
                let mask = &engine.fast_grid().level.masks[usize::from(mask_idx)];
                let mask_screen_x = ((mask.bbox.x_min() - view.x) * zoom).round() as i32;
                let mask_screen_y = ((mask.bbox.y_min() - view.y) * zoom).round() as i32;
                let mask_screen_w = (mask.width as f32 * zoom).round() as u32;
                let mask_screen_h = (mask.height as f32 * zoom).round() as u32;
                if mask_screen_w == 0 || mask_screen_h == 0 {
                    continue;
                }
                let mask_rect =
                    Rect::new(mask_screen_x, mask_screen_y, mask_screen_w, mask_screen_h);
                renderer.render_cached_mask_clipped(u32::from(mask_idx), mask_rect, dst_rect);

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
    entity: &crate::element::Entity,
    engine: &Engine,
    assets: &LevelAssets,
) -> Option<engine_coordinates::WorldPoint3D> {
    use crate::order::OrderType;

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
    let game_host = engine.mission_script().and_then(|m| m.game_host())?;
    let door = game_host.doors.get(usize::from(door_pass.door_index))?;
    let point_mid = crate::geo2d::pt(door.point_mid.0, door.point_mid.1);
    let point_out = crate::geo2d::pt(door.point_out.0, door.point_out.1);

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
            || !obs.contains_point_screen(point_out)
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
    sprite_world_bbox: &crate::geo2d::BBox2D,
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
    draw_world_bbox_outline(host, renderer, sprite_world_bbox, sprite_color);

    for &mask_idx in mask_indices {
        let mask = &engine.fast_grid().level.masks[usize::from(mask_idx)];
        draw_world_bbox_outline(host, renderer, &mask.bbox, 0xffe0);
    }

    draw_world_cross(host, renderer, actor_position.to_geo(), 0x07e0);
    if use_projectile_path {
        let projectile_test_point = crate::geo2d::pt(position_3d.x, position_3d.y);
        let actor_screen = world_to_screen(host, actor_position.to_geo());
        let projectile_screen = world_to_screen(host, projectile_test_point);
        renderer.draw_line_screen(
            actor_screen.0,
            actor_screen.1,
            projectile_screen.0,
            projectile_screen.1,
            0xfd20,
        );
        draw_world_cross(host, renderer, projectile_test_point, 0xfd20);
    }
}

fn draw_world_bbox_outline(
    host: &Host,
    renderer: &mut Renderer,
    bbox: &crate::geo2d::BBox2D,
    color: u16,
) {
    if !bbox.is_somewhere() {
        return;
    }
    let (x1, y1) = world_to_screen(host, crate::geo2d::pt(bbox.x_min(), bbox.y_min()));
    let (x2, y2) = world_to_screen(host, crate::geo2d::pt(bbox.x_max(), bbox.y_max()));
    renderer.draw_rect_outline_screen(x1, y1, x2, y2, color);
}

fn draw_world_cross(
    host: &Host,
    renderer: &mut Renderer,
    point: crate::geo2d::GeoPoint2D,
    color: u16,
) {
    let (x, y) = world_to_screen(host, point);
    renderer.draw_line_screen(x - 4, y, x + 4, y, color);
    renderer.draw_line_screen(x, y - 4, x, y + 4, color);
}

fn world_to_screen(host: &Host, point: crate::geo2d::GeoPoint2D) -> (i32, i32) {
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
/// For each outlined entity the cached outline mask texture is tinted
/// via `SDL_SetTextureColorMod` and alpha-modulated via
/// `SDL_SetTextureAlphaMod` (for hulk fade animation).
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
        let variant = engine.resolve_render_variant(entity);

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

        let outline_color_565 = if is_focused || is_action_marked {
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
        // to 0-255 for SDL.
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

        // Screen position.  Lift by `jump_z_offset` so selection
        // outlines follow airborne actors mid line-jump.
        let jump_z = entity.actor_data().map(|a| a.jump_z_offset).unwrap_or(0.0);
        let world_x = elem.position_map().x;
        let world_y = elem.position_map().y - jump_z;
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

// ─── Combat status bars (red life / blue stamina) ────────────────

/// Draw the red life / blue stamina bars below characters involved in combat.
///
/// - Red bar at offset_y = 8, width = value * 20, 3 rows tall.
/// - Blue bar at offset_y = 12 (civilians skipped — tiredness only
///   exists on NPCs/PCs that fight).
/// - Row 0: black background, full 20-wide.
/// - Row 0 (top pixel), value-width: bright colour `(r, g, b)`.
/// - Rows 1-2, value-width: darker `(r>>1, g>>1, b>>1)`.
///
/// Targets:
/// - NPC set as `host.input.double_status_bar_entity_id` by the bow /
///   stone mouse-hover handlers in
///   [`robin_engine::engine::input::update_mouse`].
/// - Every selected PC currently swordfighting, plus every opponent on
///   that PC's opponents list.
///
/// The `NpcData::display_double_status_bar` flag (set by the soldier
/// hover path, currently un-ported) is also honoured so the feature is
/// ready once that call site lands.
pub(crate) fn render_combat_status_bars(host: &mut Host, engine: &Engine, renderer: &mut Renderer) {
    use crate::element::{Entity, EntityId, Human};
    use std::collections::HashSet;

    let mut targets: HashSet<EntityId> = HashSet::new();

    // Mouse hover target (bow / stone cursor over an NPC).
    if let Some(id) = host.input.double_status_bar_entity_id {
        targets.insert(id);
    }

    // Each selected PC currently swordfighting — bars for PC + all opponents.
    for &pc_id in engine.seat_selection(host.local_seat) {
        let Some(pc) = engine.get_entity(pc_id) else {
            continue;
        };
        let Some(h) = pc.human_data() else { continue };
        if h.opponents.is_empty() {
            continue;
        }
        targets.insert(pc_id);
        for &opp in &h.opponents {
            targets.insert(opp);
        }
    }

    // NPCs that got `display_double_status_bar` set by other code paths
    // (soldier mouse focus, AI, etc.).  The flag is one-shot:
    // consumers elsewhere clear it after rendering.  We only *read* it
    // here to keep this function `&Engine`; the clearing happens in
    // `clear_display_flags`.
    for &npc_id in engine.npc_ids() {
        let Some(e) = engine.get_entity(npc_id) else {
            continue;
        };
        if e.npc_data().is_some_and(|n| n.display_double_status_bar) {
            targets.insert(npc_id);
        }
    }

    for id in targets {
        let Some(entity) = engine.get_entity(id) else {
            continue;
        };
        if !entity.is_active() {
            continue;
        }

        // Dispatch to the Human trait for life/max/tiredness.  Non-human
        // entities have no bars.
        let (life, max_life, tiredness, is_civilian) = match entity {
            Entity::Pc(p) => (
                Human::life_points(p),
                Human::max_life_points(p),
                Human::tiredness(p),
                false,
            ),
            Entity::Soldier(s) => (
                Human::life_points(s),
                Human::max_life_points(s),
                Human::tiredness(s),
                false,
            ),
            Entity::Civilian(c) => (
                Human::life_points(c),
                Human::max_life_points(c),
                Human::tiredness(c),
                true,
            ),
            _ => continue,
        };

        let pos = &entity.element_data().position_map();

        if max_life > 0 {
            // min(1, lifepoints / maxLifePoints)
            let frac = (life as f32 / max_life as f32).clamp(0.0, 1.0);
            draw_status_bar(host, renderer, pos.x, pos.y, 8.0, frac, 255, 0, 0);
        }
        if !is_civilian {
            // max(0, 0.01 * (100 - tiredness))
            let t = tiredness.min(100) as f32;
            let frac = ((100.0 - t) * 0.01).max(0.0);
            draw_status_bar(host, renderer, pos.x, pos.y, 12.0, frac, 3, 205, 255);
        }
    }
}

/// Draw one status bar at `(x_world, y_world + offset_y)` with the given
/// fill fraction.  Helper for [`render_combat_status_bars`].
///
/// Screen coords are computed manually (matching the other GPU-phase calls
/// in this module) rather than going through
/// [`crate::draw_manager::DrawManager::fill_box`], which has broader
/// gameplay draw-manager semantics than this fixed HUD overlay.
#[allow(clippy::too_many_arguments)]
fn draw_status_bar(
    host: &Host,
    renderer: &mut Renderer,
    x_world: f32,
    y_world: f32,
    offset_y: f32,
    frac: f32,
    r: u8,
    g: u8,
    b: u8,
) {
    let view = host.viewport.view_position;
    let zoom = host.viewport.zoom_factor;

    // Cast the origin to integer — truncation in world space, required
    // for pixel-perfect parity.
    let origin_x_world = (x_world - 10.0).floor();
    let origin_y_world = (y_world + offset_y).floor();

    // World → screen (same transform used by `render_entities_gpu`).
    let sx = ((origin_x_world - view.x) * zoom).round() as i32;
    let sy = ((origin_y_world - view.y) * zoom).round() as i32;

    // The bar is 20 × 3 in world units; scale by zoom.  Round so every
    // pixel row/col is covered and there is never a 1-pixel seam.
    let w_full = (20.0 * zoom).round().max(1.0) as i32;
    let w_val = (20.0 * frac * zoom).round().max(0.0) as i32;
    let h_top = zoom.round().max(1.0) as i32;
    let h_full = (3.0 * zoom).round().max(1.0) as i32;
    let h_body = (h_full - h_top).max(0);

    // Case 0: black background, full width, full height.
    renderer.render_gpu_rect(sx, sy, w_full, h_full, 0, 0, 0, 255);
    // Case 1 + 2: value-width bright top row + darker body.
    if w_val > 0 {
        renderer.render_gpu_rect(sx, sy, w_val, h_top, r, g, b, 255);
        if h_body > 0 {
            renderer.render_gpu_rect(sx, sy + h_top, w_val, h_body, r >> 1, g >> 1, b >> 1, 255);
        }
    }
}

/// Clear the one-shot `display_double_status_bar` flag on every NPC.
///
/// The flag is one-shot and must be reset right after rendering bars.
/// Since the bar renderer is a separate pass and takes `&Engine`, we
/// clear the flag here in a tiny `&mut Engine` post-pass, routed
/// through the single `apply_command` entry point.
pub(crate) fn clear_status_bar_flags(
    engine: &mut Engine,
    display: &mut engine_api::HostDisplayState,
    input: &mut engine_api::InputState,
    assets: &LevelAssets,
) {
    engine.apply_command(
        display,
        input,
        assets,
        &PlayerCommand::ClearNpcDoubleStatusBarFlags,
    );
}

/// Fallback: draw a colored rectangle for entities without sprites.
fn render_entity_fallback(
    renderer: &mut Renderer,
    kind: crate::element::ElementKind,
    screen_x: i32,
    screen_y: i32,
    screen_w: i32,
    screen_h: i32,
) {
    use crate::element::ElementKind;

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

// ─── Minimap rendering ─────────────────────────────────────────────

/// Blits the minimap bitmap at its current position, the viewport
/// indicator rectangle for the current camera view, and a dot per
/// active entity coloured by kind + state.
pub(crate) fn render_minimap(
    host: &mut Host,
    display: &engine_api::HostDisplayState,
    engine: &Engine,
    assets: &LevelAssets,
    renderer: &mut Renderer,
) {
    if host.map_surface == 0 {
        return; // no minimap loaded
    }

    let mm = display.minimap();

    // When map is closed and no transition is active, render the corner
    // button — blit it at the button-box position.
    if !mm.is_displayed() && mm.transition_counter() == 0.0 {
        if mm.button_box().is_somewhere() && !host.minimap_corner_surfaces.is_empty() {
            let state_idx = match mm.ui_state() {
                UIState::Default => 0,
                UIState::Focused => 1,
                UIState::Selected => 2,
            };
            let surface = host
                .minimap_corner_surfaces
                .get(state_idx)
                .or(host.minimap_corner_surfaces.first())
                .copied()
                .unwrap_or(0);
            if surface != 0 {
                let tl = mm.button_box().top_left();
                let br = mm.button_box().bottom_right();
                let src = BBox::new(geo2d::pt(0.0, 0.0), geo2d::pt(br.x - tl.x, br.y - tl.y));
                let dst = BBox::new(geo2d::pt(tl.x, tl.y), geo2d::pt(br.x, br.y));
                renderer.blit_to_screen(surface, Some(&src), Some(&dst), BLIT_SOURCE_TRANSPARENT);
            }
        }
        return;
    }

    if !mm.is_displayed() {
        return; // transitioning — don't draw full map yet
    }

    if !mm.map_box().is_somewhere() {
        return;
    }

    let map_box = mm.map_box();
    let map_tl = map_box.top_left();
    let map_size = mm.map_size();
    let map_w = map_size.x;
    let map_h = map_size.y;

    // Blit the minimap bitmap to the screen
    let src_box = BBox::new(geo2d::pt(0.0, 0.0), geo2d::pt(map_w, map_h));
    let dst_box = BBox::new(
        geo2d::pt(map_tl.x, map_tl.y),
        geo2d::pt(map_tl.x + map_w, map_tl.y + map_h),
    );

    renderer.blit_to_screen(
        host.map_surface,
        Some(&src_box),
        Some(&dst_box),
        BLIT_SOURCE_TRANSPARENT,
    );

    // Draw viewport indicator rectangle.
    let camera_pos = host.viewport.view_position;
    let screen_size = host.viewport.screen_size;
    let zoom = host.viewport.zoom_factor;
    let level_size = host.viewport.level_size;

    // The visible area in world coordinates (accounting for zoom and
    // panel height).  Divide by zoom first, then subtract
    // PANNEL_HEIGHT.  This diverges from the camera-position clamp
    // formula (which subtracts before dividing); the original may
    // itself be a bug, but the parity contract wins.
    let view_br = engine_coordinates::MapPoint::new(
        camera_pos.x + screen_size.x / zoom,
        camera_pos.y + screen_size.y / zoom - 80.0, // PANNEL_HEIGHT = 80
    );

    // Convert camera corners to minimap pixel coordinates
    if let (Some(tl), Some(br)) = (
        mm.real_to_map(camera_pos, level_size),
        mm.real_to_map(view_br, level_size),
    ) {
        let x1 = tl.x.floor() as i32;
        let y1 = tl.y.floor() as i32;
        let x2 = br.x.floor() as i32;
        let y2 = br.y.floor() as i32;

        // Black rectangle outline (color 0x0000).
        renderer.draw_rect_outline_screen(x1, y1, x2, y2, 0x0000);
    }

    // ── Element dots ──
    // Sort for minimap, draw each active non-highlighted element's
    // dot, then draw delayed highlights.
    if host.minimap_dot_surfaces.is_empty() {
        return;
    }

    let widget_box = if mm.map_box().is_somewhere() {
        *mm.map_box()
    } else {
        return;
    };

    let sorted = engine.sort_for_minimap();
    for id in sorted {
        if mm.is_element_highlighted(id.index()) {
            continue;
        }
        let info = match engine.minimap_dot_info(id, assets) {
            Some(i) => i,
            None => continue,
        };
        if !info.is_active {
            continue;
        }
        let dot_type = match engine_minimap::classify_element_dot(&info) {
            Some(d) => d,
            None => continue,
        };
        let entity = match engine.get_entity(id) {
            Some(e) => e,
            None => continue,
        };
        refresh_dot(
            host,
            mm,
            level_size,
            entity.element_data().position_map(),
            dot_type,
            &widget_box,
            renderer,
        );
    }

    // Delayed-reveal highlighted elements (scroll reveal etc.).
    for h in mm.highlighted_elements() {
        if !h.refresh {
            continue;
        }
        let Some(entity_id) = engine.entity_id_for_index(h.element_index) else {
            continue;
        };
        let entity = match engine.get_entity(entity_id) {
            Some(e) => e,
            None => continue,
        };
        refresh_dot(
            host,
            mm,
            level_size,
            entity.element_data().position_map(),
            engine_minimap::DotType::Highlighted,
            &widget_box,
            renderer,
        );
    }
}

/// Blit a single minimap dot sprite centred on a converted world
/// position.
fn refresh_dot(
    host: &Host,
    mm: &engine_minimap::MinimapState,
    level_size: engine_geo2d::Vec2D,
    world_pos: engine_coordinates::MapPoint,
    dot_type: engine_minimap::DotType,
    widget_box: &engine_coordinates::ScreenBBox,
    renderer: &mut Renderer,
) {
    let idx = dot_type as usize;
    let (surface, dot_w, dot_h) = match host.minimap_dot_surfaces.get(idx) {
        Some(&(s, w, h)) if s != 0 => (s, w, h),
        _ => return,
    };

    let map_pos = match mm.real_to_map(world_pos, level_size) {
        Some(p) => p,
        None => return,
    };

    // Centre the sprite on the converted position.
    let top_left = engine_coordinates::ScreenPoint::new(
        map_pos.x - (dot_w as f32) * 0.5,
        map_pos.y - (dot_h as f32) * 0.5,
    );

    // The top-left (already shifted by half-size) must lie inside the
    // full widget box.  Dots that spill out get clipped below; dots
    // whose anchor is entirely outside are skipped.
    if !widget_box.contains_point(top_left) {
        return;
    }

    // Clip the destination rect to the widget box before the final
    // blit.
    let mut dst_x_min = top_left.x;
    let mut dst_y_min = top_left.y;
    let mut dst_x_max = top_left.x + dot_w as f32;
    let mut dst_y_max = top_left.y + dot_h as f32;

    let mut src_x_min = 0.0f32;
    let mut src_y_min = 0.0f32;

    if dst_x_min < widget_box.top_left().x {
        src_x_min += widget_box.top_left().x - dst_x_min;
        dst_x_min = widget_box.top_left().x;
    }
    if dst_y_min < widget_box.top_left().y {
        src_y_min += widget_box.top_left().y - dst_y_min;
        dst_y_min = widget_box.top_left().y;
    }
    if dst_x_max > widget_box.bottom_right().x {
        dst_x_max = widget_box.bottom_right().x;
    }
    if dst_y_max > widget_box.bottom_right().y {
        dst_y_max = widget_box.bottom_right().y;
    }

    if dst_x_max <= dst_x_min || dst_y_max <= dst_y_min {
        return;
    }

    let src_box = BBox::new(
        geo2d::pt(src_x_min, src_y_min),
        geo2d::pt(
            src_x_min + (dst_x_max - dst_x_min),
            src_y_min + (dst_y_max - dst_y_min),
        ),
    );
    let dst_box = BBox::new(
        geo2d::pt(dst_x_min, dst_y_min),
        geo2d::pt(dst_x_max, dst_y_max),
    );

    renderer.blit_to_screen(
        surface,
        Some(&src_box),
        Some(&dst_box),
        BLIT_SOURCE_TRANSPARENT,
    );
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
    if engine.bg_animation_ids().is_empty() {
        return;
    }
    render_fx_entities_gpu(
        engine.bg_animation_ids().iter().copied(),
        engine,
        host,
        renderer,
    );
}

pub(crate) fn render_patch_fx_gpu(
    engine: &Engine,
    host: &Host,
    _assets: &LevelAssets,
    renderer: &mut Renderer,
) {
    let patch_ids: Vec<_> = engine
        .entities_iter_with_id()
        .filter(|(_, entity)| entity.fx_data().is_some_and(|fx| fx.patch_index.is_some()))
        .map(|(id, _)| id)
        .collect();
    if patch_ids.is_empty() {
        return;
    }
    render_fx_entities_gpu(patch_ids, engine, host, renderer);
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
    let display_anim = PlayerProfileManager::global()
        .as_ref()
        .and_then(|mgr| mgr.get_active())
        .map(|p| p.graphic_config.display_anim)
        .unwrap_or(true);

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
        let variant = engine.resolve_render_variant(entity);

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

// ─── Trajectory preview ──────────────────────────────────────────────

/// Distance between trajectory dots in world units.
const TRAJECTORY_DOT_INTERVAL: f32 = 7.0;

/// Draw trajectory preview dots.
///
/// Draws filled 1-pixel squares at regular intervals along the
/// ballistic arc.
pub(crate) fn render_trajectory_preview(host: &mut Host, renderer: &mut Renderer) {
    if !host.valid_trajectory {
        return;
    }

    let view = host.viewport.view_position;
    let zoom = host.viewport.zoom_factor;
    let screen_w = host.viewport.screen_size.x as i32;
    let screen_h = host.viewport.screen_size.y as i32;

    // Trajectory color: cyan (0,231,191) for a normal arc, pink
    // (255,100,150) when the shot is crumpled / will miss
    // (`net_crumpled` is set).
    let (cr, cg, cb) = if host.net_crumpled {
        (255u8, 100u8, 150u8)
    } else {
        (0u8, 231u8, 191u8)
    };

    /// Render dots along a trajectory from `start` through `points`.
    #[allow(clippy::too_many_arguments)]
    fn render_arc(
        start: engine_coordinates::WorldPoint3D,
        points: &[crate::element::TrajectoryPoint],
        view: crate::geo2d::GeoPoint2D,
        zoom: f32,
        screen_w: i32,
        screen_h: i32,
        cr: u8,
        cg: u8,
        cb: u8,
        renderer: &mut Renderer,
    ) {
        if points.is_empty() {
            return;
        }
        let mut last = start;
        let mut carry = 0.0f32;

        for tp in points {
            let current = tp.position;
            let dx = current.x - last.x;
            let dy = current.y - last.y;
            let dz = current.z - last.z;
            let seg_len = (dx * dx + dy * dy + dz * dz).sqrt();

            if seg_len < 0.001 {
                last = current;
                continue;
            }

            let mut dot_distance = TRAJECTORY_DOT_INTERVAL - carry;
            while dot_distance <= seg_len {
                let ratio = dot_distance / seg_len;
                let walk = engine_coordinates::WorldPoint3D {
                    x: last.x + dx * ratio,
                    y: last.y + dy * ratio,
                    z: last.z + dz * ratio,
                };

                // Project 3D → 2D: screen_y uses (y - z) for isometric height
                let map_x = walk.x;
                let map_y = walk.y - walk.z;
                let sx = ((map_x - view.x) * zoom) as i32;
                let sy = ((map_y - view.y) * zoom) as i32;

                if sx >= 0 && sy >= 0 && sx < screen_w && sy < screen_h {
                    renderer.render_gpu_rect(sx, sy, 2, 2, cr, cg, cb, 255);
                }

                dot_distance += TRAJECTORY_DOT_INTERVAL;
            }

            let total = carry + seg_len;
            carry = total - (total / TRAJECTORY_DOT_INTERVAL).floor() * TRAJECTORY_DOT_INTERVAL;
            last = current;
        }
    }

    // Render the hover-preview trajectory (computed by is_valid_trajectory).
    if !host.trajectory_preview_points.is_empty() {
        render_arc(
            host.trajectory_preview_start,
            &host.trajectory_preview_points,
            view.to_geo(),
            zoom,
            screen_w,
            screen_h,
            cr,
            cg,
            cb,
            renderer,
        );
    }
}

// ─── Listen / Whistle ability radar ping ─────────────────────────────

/// Draw the expanding radar-ping circle at a PC's feet during the
/// last `TIME_LISTEN` (5) frames of the Listen / Whistle countdown.
///
/// Draws an ellipse with a radius growing from 0 → `DISTANCE_LISTEN`
/// (Listen) or `NOISE_VOLUME_PFIIIT` (Whistle) over `TIME_LISTEN`
/// frames.
pub(crate) fn render_listen_ping(host: &mut Host, engine: &Engine, renderer: &mut Renderer) {
    const TIME_LISTEN: u32 = 5;
    const DISTANCE_LISTEN: f32 = 750.0;
    const NOISE_VOLUME_PFIIIT: f32 = 400.0;
    const LISTEN_STEP_RADIUS: f32 = DISTANCE_LISTEN / TIME_LISTEN as f32;
    const WHISTLE_STEP_RADIUS: f32 = NOISE_VOLUME_PFIIIT / TIME_LISTEN as f32;

    for entity in engine.entities_iter() {
        // Guard: `wait_time != 0 && anim ∈ {Listening, Whistling}`
        // and `wait_time < TIME_LISTEN`.  Listen and Whistle are
        // tracked on separate fields (`listen_wait_time` /
        // `whistle_wait_time`) — only one ability can be active at a
        // time so they never collide.
        let (position, radius) = match entity {
            Entity::Pc(pc) => {
                let listen_active = pc.actor.listen_phase == ListenPhase::CountingDown
                    && pc.actor.listen_wait_time != 0
                    && pc.actor.listen_wait_time < TIME_LISTEN;
                let whistle_active = matches!(
                    pc.actor.active_ability.kind,
                    Some(crate::movement::AbilityKind::Whistle)
                ) && pc.actor.whistle_wait_time != 0
                    && pc.actor.whistle_wait_time < TIME_LISTEN;

                let (wait_time, step) = if listen_active {
                    (pc.actor.listen_wait_time, LISTEN_STEP_RADIUS)
                } else if whistle_active {
                    (pc.actor.whistle_wait_time, WHISTLE_STEP_RADIUS)
                } else {
                    continue;
                };
                // radius = (TIME_LISTEN - wait_time) * STEP
                let frames_in = TIME_LISTEN - wait_time;
                let radius = (frames_in as f32 * step) as u16;
                (pc.element.position_map(), radius)
            }
            _ => continue,
        };
        host.draw_manager.draw_circle(
            renderer, position, radius, 0xFFFF, // white
        );
    }
}

// ─── Debug overlays ──────────────────────────────────────────────────

/// Render debug door gizmos: gate endpoint markers + connecting lines.
///
/// Dispatched when the `door_display` debug flag is set.  For every
/// gate-that-is-a-door, the pass draws:
///
/// * Green (`0x00FF`) line(s) from `point_out` → (`point_mid` →)
///   `point_in`.
/// * Cyan-ish (`0x0CC0`) 4-px box at `point_out`.
/// * White (`0xFFFF`) 4-px box at `point_mid` (non-default door types
///   only).
/// * Red (`0xFA00`) 4-px box at `point_in`.
///
/// Drawn through GPU helpers, so zoom scaling is applied per-axis
/// (same architectural shift the `draw_status_bar` docstring calls
/// out).
pub(crate) fn render_debug_doors(
    host: &Host,
    engine: &Engine,
    dev: &engine_api::DevState,
    renderer: &mut Renderer,
) {
    use crate::gate::DoorType;

    if !dev.debug.door_display {
        return;
    }
    let Some(game_host) = engine.mission_script().and_then(|m| m.game_host()) else {
        return;
    };

    let view = host.viewport.view_position;
    let zoom = host.viewport.zoom_factor;

    // 4-px endpoint box (`point ± (2,2)`) in world units; scale by
    // zoom so the gizmo stays the same pixel size regardless of zoom.
    let half = (2.0 * zoom).round().max(1.0) as i32;
    let side = (half * 2).max(1);

    let (line_r, line_g, line_b) = rgb565_to_rgb8(0x00FF);
    let (out_r, out_g, out_b) = rgb565_to_rgb8(0x0CC0);
    let (mid_r, mid_g, mid_b) = rgb565_to_rgb8(0xFFFF);
    let (in_r, in_g, in_b) = rgb565_to_rgb8(0xFA00);

    let world_to_screen = |p: (f32, f32)| -> (i32, i32) {
        let sx = ((p.0 - view.x) * zoom).round() as i32;
        let sy = ((p.1 - view.y) * zoom).round() as i32;
        (sx, sy)
    };
    let box_at = |renderer: &mut Renderer, (x, y): (i32, i32), (r, g, b): (u8, u8, u8)| {
        renderer.render_gpu_rect(x - half, y - half, side, side, r, g, b, 255);
    };

    for door in &game_host.doors {
        if !door.is_door() {
            continue;
        }

        let out_screen = world_to_screen(door.point_out);
        let in_screen = world_to_screen(door.point_in);

        if matches!(door.door_type, DoorType::Default) {
            // 2-point branch.
            renderer.render_gpu_line(
                out_screen.0,
                out_screen.1,
                in_screen.0,
                in_screen.1,
                line_r,
                line_g,
                line_b,
            );
            box_at(renderer, out_screen, (out_r, out_g, out_b));
            box_at(renderer, in_screen, (in_r, in_g, in_b));
        } else {
            // 3-point branch.
            let mid_screen = world_to_screen(door.point_mid);
            renderer.render_gpu_line(
                out_screen.0,
                out_screen.1,
                mid_screen.0,
                mid_screen.1,
                line_r,
                line_g,
                line_b,
            );
            renderer.render_gpu_line(
                mid_screen.0,
                mid_screen.1,
                in_screen.0,
                in_screen.1,
                line_r,
                line_g,
                line_b,
            );
            box_at(renderer, out_screen, (out_r, out_g, out_b));
            box_at(renderer, mid_screen, (mid_r, mid_g, mid_b));
            box_at(renderer, in_screen, (in_r, in_g, in_b));
        }
    }
}

/// Render the pathfinder motion-graph debug overlay: graph edges plus
/// per-node corner stubs.
///
/// Dispatched when the motion-graph-display cheat is set, drawing both
/// edges and nodes.  The half-diagonal index is the first PC's
/// pathfinder index, so the rendered overlay reflects what A* would
/// actually consider for that unit's body size.
///
/// Both passes use GPU line draws (the same architectural shift that
/// `render_debug_doors` documents); per-segment clipping is delegated
/// to the GPU framebuffer rather than explicit clipping against the
/// view rect.  World→screen transform is
/// `(point - view_rect.top_left) * zoom`.
pub(crate) fn render_debug_motion_graph(
    host: &Host,
    engine: &Engine,
    assets: &engine_api::LevelAssets,
    dev: &engine_api::DevState,
    renderer: &mut Renderer,
) {
    if !dev.debug.motion_graph_display {
        return;
    }

    let view = host.viewport.view_position;
    let zoom = host.viewport.zoom_factor;
    let screen_size = host.viewport.screen_size;
    if zoom <= 0.0 || screen_size.x <= 0.0 || screen_size.y <= 0.0 {
        return;
    }

    // The bounding box is the camera viewport in world coords: origin
    // at `view_position`, dimensions `screen_size / zoom_factor`.
    let view_rect = engine_geo2d::BBox2D::from_point_size(
        view.to_geo(),
        screen_size.x / zoom,
        screen_size.y / zoom,
    );

    // PC[0] is the first portrait-order player character.
    let half_diagonal_idx = engine
        .pc_ids()
        .first()
        .and_then(|id| engine.get_entity(*id))
        .map(|e| e.sprite().position_iface.get_pathfinder_index())
        .unwrap_or(0);

    let world_to_screen = |p: engine_coordinates::MapPoint| -> (i32, i32) {
        let sx = ((p.x - view.x) * zoom).round() as i32;
        let sy = ((p.y - view.y) * zoom).round() as i32;
        (sx, sy)
    };

    let pathfinder = engine.pathfinder();

    pathfinder.draw_graph(
        assets.pathfinder_graph.as_ref(),
        view_rect,
        half_diagonal_idx,
        |a, b, color| {
            let (r, g, blu) = rgb565_to_rgb8(color);
            let (x1, y1) = world_to_screen(a);
            let (x2, y2) = world_to_screen(b);
            renderer.render_gpu_line(x1, y1, x2, y2, r, g, blu);
        },
    );

    pathfinder.draw_nodes(
        assets.pathfinder_graph.as_ref(),
        view_rect,
        half_diagonal_idx,
        |a, b, color| {
            let (r, g, blu) = rgb565_to_rgb8(color);
            let (x1, y1) = world_to_screen(a);
            let (x2, y2) = world_to_screen(b);
            renderer.render_gpu_line(x1, y1, x2, y2, r, g, blu);
        },
    );
}

// ─── Debug surfaces overlay ──────────────────────────────────────────

/// Hash a `(layer, area)` pair to a stable RGB color.  Uses a Wang-style
/// integer hash to spread adjacent indices across the hue circle so
/// neighbouring areas get visually distinct colors.
fn surface_color(layer: usize, area: usize) -> (u8, u8, u8) {
    let mut h = (layer as u32).wrapping_mul(0x9E3779B1) ^ (area as u32).wrapping_mul(0x85EBCA77);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB352D);
    h ^= h >> 15;
    let hue = (h & 0xFF) as f32 / 255.0;
    // HSV → RGB with fixed S=0.7, V=0.9.
    let (s, v) = (0.7_f32, 0.9_f32);
    let i = (hue * 6.0).floor() as i32;
    let f = hue * 6.0 - i as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match i.rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// Locate the `(layer, area_idx)` of a world-space point across every
/// layer.  Both indices are vec positions in
/// `move_layers[layer][area]` — matches `PathGraph::find_area_at_point`,
/// which is the canonical lookup.
fn locate_surface(
    graph: &engine_pathfinder::PathGraph,
    pt: engine_geo2d::GeoPoint2D,
) -> Option<(usize, usize)> {
    let pt = engine_coordinates::MapPoint::from_geo(pt);
    (0..graph.static_data.move_layers.len())
        .find_map(|l| graph.find_area_at_point(l, pt).map(|a| (l, a)))
}

/// Draw a closed polyline outline on the GPU layer.
fn draw_polygon_outline_world(
    renderer: &mut Renderer,
    verts: &[engine_geo2d::GeoPoint2D],
    world_to_screen: &dyn Fn(engine_geo2d::GeoPoint2D) -> (i32, i32),
    r: u8,
    g: u8,
    b: u8,
) {
    if verts.len() < 2 {
        return;
    }
    for i in 0..verts.len() {
        let a = verts[i];
        let bp = verts[(i + 1) % verts.len()];
        let (x1, y1) = world_to_screen(a);
        let (x2, y2) = world_to_screen(bp);
        renderer.render_gpu_line(x1, y1, x2, y2, r, g, b);
    }
}

/// Fill a (possibly concave) polygon via ear-clipping triangulation.
///
/// Falls back silently on degenerate polygons (earcutr returns an empty
/// index list).  Fan triangulation isn't sufficient here — `MotionArea`
/// boundaries are routinely concave (e.g. ground areas wrapping around
/// buildings), and a fan from vertex 0 produces giant bowtie triangles
/// that fan across empty space.
fn fill_polygon_world(
    renderer: &mut Renderer,
    verts: &[engine_geo2d::GeoPoint2D],
    world_to_screen: &dyn Fn(engine_geo2d::GeoPoint2D) -> (f32, f32),
    r: u8,
    g: u8,
    b: u8,
    a: u8,
) {
    if verts.len() < 3 {
        return;
    }
    let mut flat: Vec<f64> = Vec::with_capacity(verts.len() * 2);
    for v in verts {
        flat.push(v.x as f64);
        flat.push(v.y as f64);
    }
    let indices = match earcutr::earcut(&flat, &[], 2) {
        Ok(ix) => ix,
        Err(_) => return,
    };
    for tri in indices.chunks_exact(3) {
        let p0 = world_to_screen(verts[tri[0]]);
        let p1 = world_to_screen(verts[tri[1]]);
        let p2 = world_to_screen(verts[tri[2]]);
        renderer.render_gpu_triangle([p0, p1, p2], r, g, b, a);
    }
}

/// Find the `(layer, area_idx)` the selected character is standing on,
/// using the canonical `PathGraph::find_area_at_point` lookup.
fn selected_surface(
    host: &Host,
    engine: &Engine,
    graph: &engine_pathfinder::PathGraph,
) -> Option<(usize, usize)> {
    let pc_id = engine.seat_selection(host.local_seat).first().copied()?;
    let entity = engine.get_entity(pc_id)?;
    let ed = entity.element_data();
    let layer = ed.layer() as usize;
    let pos = ed.position_map();
    let area = graph.find_area_at_point(layer, pos)?;
    Some((layer, area))
}

/// Fill pass for the surface debug overlay.  Drawn before sprite
/// rendering so the highlight tint sits *under* characters and
/// non-static obstacle sprites.  Only the selected character's
/// MotionArea is filled — outlining every area is left to the post-
/// sprite pass so the sprite art reads cleanly.
pub(crate) fn render_debug_surfaces_fill(
    host: &Host,
    engine: &Engine,
    assets: &LevelAssets,
    dev: &engine_api::DevState,
    renderer: &mut Renderer,
) {
    if !dev.debug.surface_display {
        return;
    }
    let view = host.viewport.view_position;
    let zoom = host.viewport.zoom_factor;
    let screen_size = host.viewport.screen_size;
    if zoom <= 0.0 || screen_size.x <= 0.0 || screen_size.y <= 0.0 {
        return;
    }
    let to_screen_f = move |p: engine_geo2d::GeoPoint2D| -> (f32, f32) {
        ((p.x - view.x) * zoom, (p.y - view.y) * zoom)
    };
    let graph = assets.pathfinder_graph.as_ref();
    let Some((sel_layer, sel_area)) = selected_surface(host, engine, graph) else {
        return;
    };
    let Some(area) = graph
        .static_data
        .move_layers
        .get(sel_layer)
        .and_then(|areas| areas.get(sel_area))
    else {
        return;
    };
    fill_polygon_world(renderer, &area.polygon, &to_screen_f, 255, 255, 0, 80);
}

/// Outline + path pass for the surface debug overlay.  Drawn after
/// sprite rendering so polygon outlines, obstacle outlines, the
/// highlighted-surface outline, and the committed-path polyline all
/// sit on top of the world and remain readable.
pub(crate) fn render_debug_surfaces_outline(
    host: &Host,
    engine: &Engine,
    assets: &LevelAssets,
    dev: &engine_api::DevState,
    renderer: &mut Renderer,
) {
    if !dev.debug.surface_display {
        return;
    }

    let view = host.viewport.view_position;
    let zoom = host.viewport.zoom_factor;
    let screen_size = host.viewport.screen_size;
    if zoom <= 0.0 || screen_size.x <= 0.0 || screen_size.y <= 0.0 {
        return;
    }

    let to_screen_i = move |p: engine_geo2d::GeoPoint2D| -> (i32, i32) {
        let (sx, sy) = ((p.x - view.x) * zoom, (p.y - view.y) * zoom);
        (sx.round() as i32, sy.round() as i32)
    };

    let graph = assets.pathfinder_graph.as_ref();
    let move_layers = &graph.static_data.move_layers;
    let selected_layer_area = selected_surface(host, engine, graph);
    let selected_id = engine.seat_selection(host.local_seat).first().copied();

    // Pass 1: outline every walkable area, plus active obstacles within.
    for (layer_idx, areas) in move_layers.iter().enumerate() {
        for (area_idx, area) in areas.iter().enumerate() {
            let (r, g, b) = surface_color(layer_idx, area_idx);
            draw_polygon_outline_world(renderer, &area.polygon, &to_screen_i, r, g, b);
            for obstacle in &area.motion_obstacles {
                if !obstacle.active {
                    continue;
                }
                draw_polygon_outline_world(renderer, &obstacle.polygon, &to_screen_i, 200, 40, 40);
            }
        }
    }

    // Pass 2: bright outline on the selected character's surface
    // (the fill is drawn earlier, beneath sprites).
    if let Some((sel_layer, sel_area)) = selected_layer_area
        && let Some(area) = move_layers
            .get(sel_layer)
            .and_then(|areas| areas.get(sel_area))
    {
        draw_polygon_outline_world(renderer, &area.polygon, &to_screen_i, 255, 255, 0);
    }

    // Pass 3: committed path polyline, colored per segment by the
    // destination waypoint's (layer, area).  An X marker at each
    // waypoint highlights surface transitions.
    if let Some(pc_id) = selected_id
        && let Some(waypoints) = engine.actor_path_waypoints(pc_id)
        && !waypoints.is_empty()
    {
        let start = engine
            .get_entity(pc_id)
            .map(|e| {
                let pm = e.element_data().position_map();
                engine_geo2d::pt(pm.x, pm.y)
            })
            .unwrap_or_else(|| waypoints[0].to_geo());
        let mut prev = start;
        for &wp in &waypoints {
            let wp_geo = wp.to_geo();
            let (r, g, b) = match locate_surface(graph, wp_geo) {
                Some((l, a)) => surface_color(l, a),
                None => (255, 255, 255),
            };
            let (x1, y1) = to_screen_i(prev);
            let (x2, y2) = to_screen_i(wp_geo);
            renderer.render_gpu_line(x1, y1, x2, y2, r, g, b);
            const M: i32 = 4;
            renderer.render_gpu_line(x2 - M, y2 - M, x2 + M, y2 + M, r, g, b);
            renderer.render_gpu_line(x2 - M, y2 + M, x2 + M, y2 - M, r, g, b);
            prev = wp_geo;
        }
    }

    // Pass 4: 3D-position anchor for the selected character.
    // Vertical drop from where the sprite renders (iso-projected
    // (x, y - z)) down to the z = 0 ground projection (x, y), plus a
    // small flat ellipse "shadow footprint" at the bottom.  Makes the
    // entity's height immediately visible — useful when debugging
    // movement on rooftops, ladders, or during jumps/falls.
    if let Some(pc_id) = selected_id
        && let Some(entity) = engine.get_entity(pc_id)
    {
        let pos = entity.element_data().position();
        // Top: where the sprite is drawn.  Bottom: same map (x, y)
        // but at z = 0.
        let top_x_w = pos.x;
        let top_y_w = pos.y - pos.z;
        let bot_x_w = pos.x;
        let bot_y_w = pos.y;
        let top = (
            ((top_x_w - view.x) * zoom).round() as i32,
            ((top_y_w - view.y) * zoom).round() as i32,
        );
        let bot = (
            ((bot_x_w - view.x) * zoom).round() as i32,
            ((bot_y_w - view.y) * zoom).round() as i32,
        );
        // Vertical drop line.
        renderer.render_gpu_line(top.0, top.1, bot.0, bot.1, 255, 255, 255);
        // Footprint ellipse: 16 segments around an ellipse with
        // world-unit radii (rx, ry) — flattened to suggest the
        // ground plane.  Drawn in screen space directly.
        const RX_W: f32 = 8.0;
        const RY_W: f32 = 3.0;
        const SEGMENTS: u32 = 16;
        let cx = bot.0 as f32;
        let cy = bot.1 as f32;
        let rx_s = RX_W * zoom;
        let ry_s = RY_W * zoom;
        let mut prev_pt = (cx + rx_s, cy);
        for i in 1..=SEGMENTS {
            let t = (i as f32) * std::f32::consts::TAU / (SEGMENTS as f32);
            let p = (cx + rx_s * t.cos(), cy + ry_s * t.sin());
            renderer.render_gpu_line(
                prev_pt.0.round() as i32,
                prev_pt.1.round() as i32,
                p.0.round() as i32,
                p.1.round() as i32,
                255,
                255,
                255,
            );
            prev_pt = p;
        }
    }
}

/// Render debug animation lines: polylines for all FX entities.
///
/// Active FX are drawn in white (0xFFFF), inactive in dark gray
/// (0xFA00).
/// Render the `noise_display` cheat overlay.
///
/// 0. Outline every sound-sector polygon on the visible map in dark
///    teal (0x00AF).  These are the material sectors that feed
///    footstep material lookups + water/hole detection.
/// 1. For every PC, print the current-floor material name above the
///    PC and draw expanding isometric rings sized by the PC's
///    currently-produced footstep noise volume.  The start radius
///    scrolls via `dev.noise_display_start_radius`, which the engine
///    advances each tick so the rings animate outward.
/// 2. For every punctual noise active in `dev.displayed_noises`
///    (populated by `broadcast_noise`), draw concentric rings from
///    `start_radius` up to the effective volume; those entries retire
///    on the sim side once the ring has outgrown the volume.
/// 3. For the currently view-selected NPC (or the first NPC if none
///    is selected), draw a black ring at its `cover_noise_deafness`
///    radius — the "can't hear inside this circle" envelope.
pub(crate) fn render_noise_display(
    host: &mut Host,
    engine: &Engine,
    assets: &LevelAssets,
    dev: &engine_api::DevState,
    fonts: Option<&HudFonts>,
    selected_view_element: Option<engine_element::EntityId>,
    renderer: &mut Renderer,
) {
    if !dev.debug.noise_display {
        return;
    }

    const CIRCLE_DISTANCE: u16 = 20;
    const HEARING_FACTOR: f32 = 1.0;

    // ── (0) Sound-sector polygon outlines ────────────────────────
    // Iterate material sectors registered as sound sectors and draw
    // each polygon outline in dark teal.
    for sector in &assets.material_sectors.sectors {
        if sector.points.len() < 2 {
            continue;
        }
        // `draw_polyline` draws segments between consecutive points —
        // append the first point so the polygon closes.
        let mut closed = sector.points.clone();
        closed.push(sector.points[0]);
        host.draw_manager.draw_polyline(renderer, &closed, 0x00AF);
    }

    // ── (1) Per-PC footstep rings + material label ────────────────
    let start_radius = dev.noise_display_start_radius;
    for entity in engine.entities_iter() {
        let Entity::Pc(pc) = entity else {
            continue;
        };
        let position = pc.element.position_map();
        let origin = position;

        // Material name text.  Use the PC's cached position-interface
        // material (the same value `pc_noise_volume` reads) so the
        // label stays consistent with the ring size.
        if let Some(fonts) = fonts {
            let label = match pc.element.material() {
                GameMaterial::Ground => "ground",
                GameMaterial::Wood => "wood",
                GameMaterial::Stone => "stone",
                GameMaterial::Grass => "grass",
                GameMaterial::Leaves => "leaves",
                GameMaterial::Water => "water",
                GameMaterial::Bush => "bush",
                GameMaterial::Ice => "ice",
                GameMaterial::Hole => "hole",
                GameMaterial::LightShadow => "shadow",
            };
            // Offset (+10, -40) from the centre, in screen space.
            let screen = host.draw_manager.map_to_screen(origin);
            render_text_with_shadow(
                renderer,
                fonts,
                label,
                screen.x as i32 + 10,
                screen.y as i32 - 40,
            );
        }

        let volume = pc.actor.last_noise_volume;
        if volume == 0 {
            continue;
        }
        let effective = (volume as f32 * HEARING_FACTOR) as u16;
        let mut r = start_radius;
        while r < effective {
            host.draw_manager.draw_ellipse(renderer, origin, r, 0xFFFF);
            r = r.saturating_add(CIRCLE_DISTANCE);
            if r == 0 {
                break;
            }
        }
    }

    // ── (2) Punctual noises ──────────────────────────────────────
    for displayed in &dev.displayed_noises {
        let noise = &displayed.noise;
        let origin = engine_coordinates::MapPoint::new(noise.origin.x, noise.origin.y);
        let effective = (noise.volume as f32 * HEARING_FACTOR) as u16;
        let mut r = displayed.start_radius;
        while r < effective {
            host.draw_manager.draw_ellipse(renderer, origin, r, 0xFFFF);
            r = r.saturating_add(CIRCLE_DISTANCE);
            if r == 0 {
                break;
            }
        }
        // Height slices — stack dim ellipses offset vertically by
        // ±height to hint at 3D noise volume.
        let mut sw_height = effective.saturating_sub(1) as i32;
        let min_h = -(effective as i32) + 1;
        while sw_height > min_h {
            if sw_height <= -(noise.elevation as i32) {
                break;
            }
            let r2 = effective as f32 * effective as f32 - (sw_height * sw_height) as f32;
            if r2 > 0.0 {
                let radius = r2.sqrt() as u16;
                host.draw_manager.draw_ellipse(
                    renderer,
                    engine_coordinates::MapPoint::new(origin.x, origin.y - sw_height as f32),
                    radius,
                    0x000A,
                );
            }
            sw_height -= CIRCLE_DISTANCE as i32;
        }
    }

    // ── (3) Selected NPC deafness ring ───────────────────────────
    // Pick the selected view element if it's an NPC, else the first
    // NPC.
    let picked_npc: Option<engine_element::EntityId> = selected_view_element
        .filter(|id| engine.get_entity(*id).map(|e| e.is_npc()).unwrap_or(false))
        .or_else(|| engine.npc_ids().first().copied());
    if let Some(npc_id) = picked_npc
        && let Some(entity) = engine.get_entity(npc_id)
        && let Some(npc) = entity.npc_data()
    {
        // Read the stored deafness — decay was already applied this
        // tick in the sim path.  `get_deafness` mutates for lazy
        // decay, but during rendering we only need the snapshot.
        let radius = npc.old_cover_noise_deafness;
        if radius > 0 {
            let pos = entity.element_data().position_map();
            host.draw_manager
                .draw_ellipse(renderer, pos, radius, 0x0000);
        }
    }
}

pub(crate) fn render_debug_animation_lines(
    host: &mut Host,
    engine: &Engine,
    dev: &engine_api::DevState,
    renderer: &mut Renderer,
) {
    if !dev.debug.display_animation_lines {
        return;
    }

    for entity in engine.entities_iter() {
        if !entity.is_fx() {
            continue;
        }
        let polyline = entity.display_polyline();
        if polyline.is_empty() {
            continue;
        }
        let color: u16 = if entity.is_active() { 0xFFFF } else { 0xFA00 };

        host.draw_manager.draw_polyline(renderer, polyline, color);
    }
}

/// Render the per-NPC "whatsup" debug overlay: a red suspect bar and a
/// white outline rectangle centred 55 world units above each active NPC.
///
/// Gated on `GlobalOptions::whatsup` so the overlay stays off for
/// normal runs.  The reference visibility bar is commented out in the
/// source and is therefore intentionally not ported.
///
/// The red fill occupies the top half of the outline (y in
/// `[-HALFHEIGHT, 0]`) and scales its width by `max_suspect * 0.001`,
/// i.e. `min(1000, max(maximal_detection_suspect, sorrow_level)) /
/// 1000`.  The outline is always the full 40×4 rectangle centred on
/// `position_map + (0, -55)`.
///
/// Architectural note: drawn through the GPU path (matching
/// `render_debug_doors`, `draw_status_bar`, etc.) so we just
/// world→screen transform and let the framebuffer clip.  Empty rects
/// are skipped.
pub(crate) fn render_debug_whatsup_overlay(host: &Host, engine: &Engine, renderer: &mut Renderer) {
    let enabled = engine_api::GlobalOptions::global()
        .as_ref()
        .is_some_and(|o| o.whatsup);
    if !enabled {
        return;
    }

    let view = host.viewport.view_position;
    let zoom = host.viewport.zoom_factor;
    if zoom <= 0.0 {
        return;
    }

    const HALF_WIDTH: f32 = 20.0;
    const HALF_HEIGHT: f32 = 2.0;

    let to_screen = |wx: f32, wy: f32| -> (i32, i32) {
        let sx = ((wx - view.x) * zoom).round() as i32;
        let sy = ((wy - view.y) * zoom).round() as i32;
        (sx, sy)
    };

    for &npc_id in engine.npc_ids() {
        let Some(entity) = engine.get_entity(npc_id) else {
            continue;
        };
        if !entity.is_active() || entity.is_dead() {
            continue;
        }
        let Some(ai) = entity.ai_controller() else {
            continue;
        };
        let Some(npc) = entity.npc_data() else {
            continue;
        };

        // `max_suspect` = min(1000, max(maximal_detection_suspect,
        // sorrow_level)) — see `alert_colors.rs:61` for the
        // definition.
        let max_suspect = npc.maximal_detection_suspect.max(ai.sorrow_level).min(1000);

        let pos = entity.element_data().position_map();
        // Centre = position_map + (0, -55).
        let cx = pos.x;
        let cy = pos.y - 55.0;

        // ── Red suspect fill bar (b3D=false) ──
        // full_box = (centre - (HALFWIDTH, HALFHEIGHT),
        //            centre + (-HALFWIDTH + 2*HALFWIDTH*filled, 0))
        let filled = max_suspect as f32 * 0.001;
        let full_min_x = cx - HALF_WIDTH;
        let full_min_y = cy - HALF_HEIGHT;
        let full_max_x = cx + (-HALF_WIDTH + 2.0 * HALF_WIDTH * filled);
        let full_max_y = cy;
        if full_max_x > full_min_x && full_max_y > full_min_y {
            let (sx1, sy1) = to_screen(full_min_x, full_min_y);
            let (sx2, sy2) = to_screen(full_max_x, full_max_y);
            fill_box_whatsup(renderer, sx1, sy1, sx2 - sx1, sy2 - sy1, 255, 0, 0, false);
        }

        // ── White outline around the full bar extent (b3D=false, distance=0) ──
        let (ex1, ey1) = to_screen(cx - HALF_WIDTH, cy - HALF_HEIGHT);
        let (ex2, ey2) = to_screen(cx + HALF_WIDTH, cy + HALF_HEIGHT);
        if ex2 > ex1 && ey2 > ey1 {
            // 4-line rectangle — `b3D=false` bounding-box branch.
            renderer.render_gpu_line(ex1, ey1, ex2, ey1, 255, 255, 255);
            renderer.render_gpu_line(ex2, ey1, ex2, ey2, 255, 255, 255);
            renderer.render_gpu_line(ex2, ey2, ex1, ey2, 255, 255, 255);
            renderer.render_gpu_line(ex1, ey2, ex1, ey1, 255, 255, 255);
        }
    }
}

/// Fill a screen-space rectangle and optionally draw a Windows-button
/// bevel around it.
///
/// The base colour is `(r, g, b)` (handled directly by
/// `render_gpu_rect`, so a depth-aware color helper is not needed);
/// the bevel uses `min(255, c * 1.5)` for the top-left highlight and
/// `c * 0.7` for the bottom-right shadow.  The only caller is
/// `render_debug_whatsup_overlay`, which passes `b3D=false` for both
/// of its boxes, so the bevel arm is wired up for completeness but
/// exercised only when a future caller needs it.
#[allow(clippy::too_many_arguments)]
fn fill_box_whatsup(
    renderer: &mut Renderer,
    sx: i32,
    sy: i32,
    w: i32,
    h: i32,
    r: u8,
    g: u8,
    b: u8,
    b3d: bool,
) {
    if w <= 0 || h <= 0 {
        return;
    }
    renderer.render_gpu_rect(sx, sy, w, h, r, g, b, 255);
    if !b3d {
        return;
    }

    // `(c * 1.5)` clamped to 255.
    let br = ((r as u16 * 3) / 2).min(255) as u8;
    let bg = ((g as u16 * 3) / 2).min(255) as u8;
    let bb = ((b as u16 * 3) / 2).min(255) as u8;
    // `(c * 0.7)` — float truncation to integer.
    let dr = (r as f32 * 0.7) as u8;
    let dg = (g as f32 * 0.7) as u8;
    let db = (b as f32 * 0.7) as u8;

    let x1 = sx;
    let y1 = sy;
    let x2 = sx + w;
    let y2 = sy + h;

    // Shadow: bottom-left → bottom-right, bottom-right → top-right.
    renderer.render_gpu_line(x1, y2, x2, y2, dr, dg, db);
    renderer.render_gpu_line(x2, y2, x2, y1, dr, dg, db);
    // Highlight: bottom-left → top-left, top-left → top-right.
    renderer.render_gpu_line(x1, y2, x1, y1, br, bg, bb);
    renderer.render_gpu_line(x1, y1, x2, y1, br, bg, bb);
}

// ─── Ransom/amulet text overlay ──────────────────────────────────────

/// Render the ransom and amulet counters in the top-left corner.
///
/// Renders both values via `render_text_background`, which draws the
/// text with a drop shadow using the shadow font at ±1 offsets, then
/// the main font on top.
pub(crate) fn render_ransom_amulet_overlay(
    engine: &Engine,
    renderer: &mut Renderer,
    fonts: &HudFonts,
    menu_resources: Option<&IngameMenuResources>,
) {
    let campaign = match engine.campaign() {
        Some(c) => c,
        None => return,
    };

    let ransom = campaign.get_value(CampaignValue::Ransom);
    let amulets = campaign.get_value(CampaignValue::Amulets);

    // Use localized menu-text format strings with `%d`.  The demo
    // data's English strings are "Money: £%d" and "Clover: %d"; fall
    // back to hard-coded English "Ransom: %d" / "Amulets: %d" when the
    // table is unavailable.
    let (ransom_tpl, amulet_tpl) = if let Some(res) = menu_resources {
        (
            res.menu_text.get(MT_STR_RANSOM),
            res.menu_text.get(MT_STR_AMULETS),
        )
    } else {
        ("Ransom: %d".into(), "Amulets: %d".into())
    };
    let ransom_text = substitute_int(&ransom_tpl, ransom);
    let amulet_text = substitute_int(&amulet_tpl, amulets);

    // Positions: (0, 0) for ransom, (0, 15) for amulets.  The text
    // renderer's left-anchored point overload insets the glyph anchor
    // by `kerning_margin = 2` on the X axis.
    const KERNING_MARGIN: i32 = 2;
    render_text_with_shadow(renderer, fonts, &ransom_text, KERNING_MARGIN, 0);
    render_text_with_shadow(renderer, fonts, &amulet_text, KERNING_MARGIN, 15);
}

/// Substitute the first `%d` or `%i` token in a C-style format string.
/// C's `swprintf` accepts either for an integer; the stock English demo
/// data uses `%i` ("Money: £%i") while other locales use `%d`.
fn substitute_int(template: &str, value: i32) -> String {
    let d = template.find("%d");
    let i = template.find("%i");
    let pos = match (d, i) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    match pos {
        Some(p) => format!("{}{}{}", &template[..p], value, &template[p + 2..]),
        None => template.to_string(),
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

// ─── Multi-selection rubber-band rectangle ────────────────────────

/// Draw the multi-selection rubber-band box when dragging a selection
/// rectangle on the map.
///
/// Rules:
/// * If any selected PC is swordfighting, cancel both the selection
///   and unselection drags and skip rendering.
/// * Otherwise, once the drag exceeds `MULTI_SELECTION_THRESHOLD`
///   squared distance, latch `draw_multi_selection = true` so
///   subsequent frames paint the box even if the pointer briefly
///   shrinks the rect below the threshold.
/// * When latched, paint the four edges in the select/unselect color.
pub(crate) fn draw_multi_selection_box(host: &mut Host, engine: &Engine, renderer: &mut Renderer) {
    // ── Swordfighting cancel ──
    if engine.is_seat_selection_swordfighting(host.local_seat) {
        host.input.multi_selection_active = false;
        host.input.multi_unselection_active = false;
        return;
    }

    if !host.input.multi_selection_active && !host.input.multi_unselection_active {
        return;
    }

    let p1 = host.input.multi_selection_pt1;
    let p2 = host.input.multi_selection_pt2;

    // ── Latch draw_multi_selection once the drag clears the
    //    threshold.  The square norm is in map units; compared to
    //    `MULTI_SELECTION_THRESHOLD` (1600). ──
    if !host.input.draw_multi_selection {
        let dx = p1.x - p2.x;
        let dy = p1.y - p2.y;
        if dx * dx + dy * dy > MULTI_SELECTION_THRESHOLD {
            host.input.draw_multi_selection = true;
        }
    }

    if !host.input.draw_multi_selection {
        return;
    }

    // ── Colors: 0x737 for select, 0x373 for unselect — written
    //    directly as RGB565 pixel values. ──
    let color: u16 = if host.input.multi_selection_active {
        0x0737
    } else {
        0x0373
    };

    // ── Compute screen-space corners via the unclamped transform;
    //    SDL's line drawer clips off-screen pieces. ──
    let a = host
        .viewport
        .map_to_screen_unclamped(engine_coordinates::MapPoint::new(
            p1.x.min(p2.x),
            p1.y.min(p2.y),
        ));
    let b = host
        .viewport
        .map_to_screen_unclamped(engine_coordinates::MapPoint::new(
            p1.x.max(p2.x),
            p1.y.min(p2.y),
        ));
    let c = host
        .viewport
        .map_to_screen_unclamped(engine_coordinates::MapPoint::new(
            p1.x.max(p2.x),
            p1.y.max(p2.y),
        ));
    let d = host
        .viewport
        .map_to_screen_unclamped(engine_coordinates::MapPoint::new(
            p1.x.min(p2.x),
            p1.y.max(p2.y),
        ));

    renderer.draw_line_screen(a.x as i32, a.y as i32, b.x as i32, b.y as i32, color);
    renderer.draw_line_screen(b.x as i32, b.y as i32, c.x as i32, c.y as i32, color);
    renderer.draw_line_screen(c.x as i32, c.y as i32, d.x as i32, d.y as i32, color);
    renderer.draw_line_screen(d.x as i32, d.y as i32, a.x as i32, a.y as i32, color);
}
