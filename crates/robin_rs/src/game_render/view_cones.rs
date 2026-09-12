//! view_cones presentation pass.
use super::*;

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
    host: &HostDraw<'_>,
    presentation: &FramePresentationInputs,
    engine: &PresentationView<'_>,
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
        render_all_view_cones(host, presentation, engine, assets, renderer);
        return;
    }

    let (viewer, params, tint) = if dev.debug.free_shadow_polygon {
        // Developer cheat: anchor the cone at a stored 3D position,
        // or at the camera centre when nothing has been set yet.
        let pos =
            dev.cheat_free_shadow_polygon_pos
                .unwrap_or_else(|| engine_coordinates::WorldPoint3D {
                    x: host.viewport().view_position.x
                        + (host.viewport().screen_size.x / host.viewport().zoom_factor) * 0.5,
                    y: host.viewport().view_position.y
                        + (host.viewport().screen_size.y / host.viewport().zoom_factor) * 0.5,
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
        host.viewport().view_position.x,
        host.viewport().view_position.y,
        host.viewport().view_position.x
            + (host.viewport().screen_size.x - 1.0) / host.viewport().zoom_factor,
        host.viewport().view_position.y
            + (host.viewport().screen_size.y - PANNEL_HEIGHT + 1.0) / host.viewport().zoom_factor,
    );

    let alpha = params
        .alpha
        .min(crate::shadow_polygon::alpha_for_ambiance(matches!(
            presentation.ambiance,
            Ambiance::Night | Ambiance::Fog
        )));

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
            host.viewport().zoom_factor,
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

pub(super) fn view_cone_polys_for_render(
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
        let ground_polys = crate::shadow_polygon::compute_visibility_polygon_on_surface(
            viewer,
            &slice_params,
            &all_obstacles,
            None,
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
        let projection_attachment = projection_area
            .projection_area_ref()
            .expect("projection-area obstacle is missing its exact topology attachment");
        let occluding_projection_areas: Vec<&robin_engine::sight_obstacle::SightObstacle> =
            active_obstacles
                .iter()
                .filter_map(|(idx, o)| {
                    (*idx != projection_idx
                        && o.is_projection_area()
                        && o.projection_area_ref().is_some_and(|attachment| {
                            attachment.layer >= projection_attachment.layer
                        })
                        && o.box_projection
                            .intersects_bbox(&projection_area.box_projection))
                    .then_some(*o)
                })
                .collect();
        let (polys, viewer) = crate::shadow_polygon::project_and_clip_to_projection_area(
            &crate::shadow_polygon::compute_visibility_polygon_on_surface(
                viewer,
                &slice_params,
                &obstacles,
                Some(projection_plane),
            ),
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
                // in the same projected map space the original game uses.
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

pub(super) fn shadow_polygon_slice_radius(
    params: &crate::shadow_polygon::ViewParameters,
    projection_plane: Option<engine_position_interface::PlaneZCoeffs>,
    viewer: GroundPoint,
) -> Option<f32> {
    const FACTOR_ELLIPSE: f32 = 0.35;
    const INV_SQUARE_FACTOR_ELLIPSE: f32 = 8.163_265;
    const FACTOR_CONE_LEAN_OUT: f32 = 0.8;

    let distance_to_plane = projection_plane
        .map(|plane| {
            let vertical = params.viewer_z - plane.compute_world_z(viewer.x, viewer.y);
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
pub(super) fn render_all_view_cones(
    host: &HostDraw<'_>,
    presentation: &FramePresentationInputs,
    engine: &PresentationView<'_>,
    assets: &LevelAssets,
    renderer: &mut Renderer,
) {
    use robin_engine::engine::{Ambiance, PANNEL_HEIGHT};

    let all_params = engine.all_npc_view_cone_params();
    if all_params.is_empty() {
        return;
    }

    let view_rect = engine_coordinates::MapBBox::from_coords(
        host.viewport().view_position.x,
        host.viewport().view_position.y,
        host.viewport().view_position.x
            + (host.viewport().screen_size.x - 1.0) / host.viewport().zoom_factor,
        host.viewport().view_position.y
            + (host.viewport().screen_size.y - PANNEL_HEIGHT + 1.0) / host.viewport().zoom_factor,
    );

    let obstacles_view = engine.sight_obstacles(assets);

    // Each NPC's visibility polygon may fragment into multiple rings
    // after obstacle subtraction. Each ring becomes its own TintedCone
    // with the NPC's tint — geo's difference guarantees the MultiPolygon
    // parts are disjoint, so same-tint rings never overlap and the GPU
    // path's alpha-blend doesn't double-darken.
    let weather_alpha = crate::shadow_polygon::alpha_for_ambiance(matches!(
        presentation.ambiance,
        Ambiance::Night | Ambiance::Fog
    ));

    let visible_params = all_params.into_iter().filter(|(viewer, params, _)| {
        let r = params.radius;
        let z = params.viewer_z.max(0.0);
        let cone_bbox = engine_coordinates::MapBBox::from_coords(
            viewer.x - r,
            viewer.y - z - r,
            viewer.x + r,
            viewer.y + r,
        );
        view_rect.intersects_bbox(&cone_bbox)
    });

    let cones: Vec<crate::shadow_polygon::TintedCone> = visible_params
        .flat_map(|(viewer, params, tint)| {
            let slices = view_cone_polys_for_render(viewer, &params, &obstacles_view);
            let color = tint.unwrap_or((0, 0, 0));
            let alpha = params.alpha.min(weather_alpha);
            let view_rect_for_filter = view_rect;
            slices.into_iter().flatten().flat_map(move |slice| {
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
                        let bbox =
                            engine_coordinates::MapBBox::from_coords(x_min, y_min, x_max, y_max);
                        view_rect_for_filter.intersects_bbox(&bbox)
                    })
                    .map(move |p| crate::shadow_polygon::TintedCone {
                        polygon: p,
                        tint: color,
                        viewer: slice.viewer,
                        radius,
                        alpha,
                        projection_plane: slice.projection_plane,
                    })
            })
        })
        .collect();

    if cones.is_empty() {
        return;
    }

    crate::shadow_polygon::render_tinted_cones(
        renderer,
        &view_rect,
        host.viewport().zoom_factor,
        &cones,
    );
}
