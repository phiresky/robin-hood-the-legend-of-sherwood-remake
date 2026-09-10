//! Shadow polygon — view cone overlay.
//!
//! **This is NOT a fog-of-war system** — the original game shows the full
//! map at all times.  This module implements the *view cone overlay* drawn
//! for the currently-selected view element: when the player alt-hovers an
//! NPC (or an ally), the map area *outside* that character's vision cone
//! is darkened so the player can see at a glance where the character can
//! and cannot see.
//!
//! The visibility geometry follows the original game's shadow-polygon result:
//! obstacle volumes are centrally projected onto each ground/platform slice,
//! once at feet height and once at head height. A point is hidden only when
//! both projections occlude it. The old 16-bit/MMX scanline blitter is replaced
//! by GPU spans, but it consumes the same renderer-independent regions.
//!
//! The rendering uses a scanline rasteriser that walks the visible polygon
//! edges and darkens every pixel that falls outside all visible regions.
//!
//! Coordinate note: private `GroundPoint` values in this module are ground-space
//! computational geometry points. Public entry points use `GroundPoint`; the
//! `geo` crate boundary is kept internal for polygon clipping/boolean ops.

use crate::gfx_types::Rect;
use crate::renderer::Renderer;
use geo::{Area, BooleanOps, algorithm::unary_union};
use robin_engine::coordinates::{GroundBBox, GroundPoint, MapBBox};
use robin_engine::mask as engine_mask;
use robin_engine::position_interface as engine_position_interface;
use robin_engine::sight_obstacle::SightObstacle;

// Shared constants + types from the engine side.
#[cfg(test)]
use robin_engine::shadow_polygon::NORMAL_HALF_APERTURE;
pub(crate) use robin_engine::shadow_polygon::{
    ALPHA_DAY, ALPHA_NIGHT, ASPECT_RATIO, CHARACTER_HEIGHT, ViewParameters, sector_to_direction,
};

/// A visibility polygon paired with tint/fade metadata (for per-NPC alert coloring).
pub type TintedCone = (
    Vec<GroundPoint>,
    (u8, u8, u8),
    GroundPoint,
    f32,
    u8,
    Option<engine_position_interface::PlaneZCoeffs>,
);

// Constants/structs imported from robin_engine::shadow_polygon (see top of file).

/// Angular step reference for arc polygon construction.
const APERTURE_STEP: f32 = 0.45;
const RADIUS_REFERENCE: f32 = 200.0;
const ALPHA_END: u8 = 4;

// Free-standing helpers (the sim-state struct lives in robin_engine).

// ── View-cone construction ──────────────────────────────────────

/// Build the view-cone polygon for a viewer.
///
/// Returns a list of points forming a fan-shaped polygon in **world
/// coordinates**: viewer → left edge → arc → right edge.
pub fn compute_view_cone(viewer: GroundPoint, params: &ViewParameters) -> Vec<GroundPoint> {
    compute_view_cone_geo(viewer, params)
}

fn compute_view_cone_geo(viewer: GroundPoint, params: &ViewParameters) -> Vec<GroundPoint> {
    let dir = normalise(params.direction);
    let radius = params.radius;
    let aperture = 2.0 * params.half_aperture;

    // Number of arc segments — radius-dependent so smaller cones don't
    // get over-tessellated.
    let step_ref = RADIUS_REFERENCE * APERTURE_STEP / radius;
    let num_steps = aperture / step_ref;
    let num_steps = num_steps.floor().max(1.0) as u32;
    let actual_step = aperture / (num_steps + 1) as f32;

    // Left / right edges of the cone
    let left_dir = rotate(dir, -params.half_aperture);
    let right_dir = rotate(dir, params.half_aperture);

    // Capacity: center + left + num_steps arc + right
    let mut poly = Vec::with_capacity(num_steps as usize + 3);

    // Center
    poly.push(viewer);

    // Left edge (with isometric Y squash)
    let left_iso = iso(left_dir);
    poly.push(GroundPoint {
        x: viewer.x + left_iso[0] * radius,
        y: viewer.y + left_iso[1] * radius,
    });

    // Arc points from left towards right
    for i in 1..=num_steps {
        let angle = actual_step * i as f32;
        let v = rotate(left_dir, angle);
        let v_iso = iso(v);
        poly.push(GroundPoint {
            x: viewer.x + v_iso[0] * radius,
            y: viewer.y + v_iso[1] * radius,
        });
    }

    // Right edge
    let right_iso = iso(right_dir);
    poly.push(GroundPoint {
        x: viewer.x + right_iso[0] * radius,
        y: viewer.y + right_iso[1] * radius,
    });

    poly
}

// ── Visibility polygon (view cone clipped by obstacles) ─────────

/// Compute the visible-area polygon(s), clipping the view cone against
/// nearby obstacles.
///
/// Opaque obstacle volumes are projected onto the requested surface at feet
/// and head height. Their common occlusion is subtracted from the cone, which
/// matches the Original's rule that either visible endpoint reveals a standing
/// target.
///
/// Returns a list of polygons (zero or more) in world coordinates —
/// usually a single non-convex polygon, but obstacle arrangements can
/// produce disjoint visible regions. Polygons are returned with their
/// exterior rings only; holes are flattened as separate rings because
/// the even-odd scanline rasteriser in `render_darken_inside` /
/// `render_tinted_cones` treats every ring uniformly.
pub fn compute_visibility_polygon(
    viewer: GroundPoint,
    params: &ViewParameters,
    obstacles: &[&SightObstacle],
) -> Vec<Vec<GroundPoint>> {
    compute_visibility_polygon_on_surface(viewer, params, obstacles, params.projection_plane)
}

/// Compute visibility directly on one world-space ground/platform plane.
///
/// Unlike the former implementation, this projects every 3D obstacle onto
/// the requested slice before clipping. Fog of war can call this same entry
/// point with a circular domain/sampling adapter instead of duplicating LOS
/// rules.
pub fn compute_visibility_polygon_on_surface(
    viewer: GroundPoint,
    params: &ViewParameters,
    obstacles: &[&SightObstacle],
    projection_plane: Option<engine_position_interface::PlaneZCoeffs>,
) -> Vec<Vec<GroundPoint>> {
    let view_cone = compute_view_cone_geo(viewer, params);

    if obstacles.is_empty() {
        return vec![view_cone];
    }

    let radius_sq = params.radius * params.radius;
    // Iso-squashed flanking-ray directions. `iso()` applies the same
    // `y *= ASPECT_RATIO` squash as the view-cone polygon itself, so
    // the cone sides, bbox and `is_box_inside_field` test all live in
    // the same reference frame.
    let dir = normalise(params.direction);
    let left_side = iso(rotate(dir, -params.half_aperture));
    let right_side = iso(rotate(dir, params.half_aperture));

    let relevant_obstacles: Vec<&SightObstacle> = obstacles
        .iter()
        .copied()
        .filter(|obs| {
            // Caller filtered to active obstacles already.
            if !obs.is_opaque() {
                return false;
            }
            if !obs.box_ground.is_somewhere() {
                tracing::warn!(
                    obstacle_id = obs.id,
                    "opaque sight obstacle has no ground bounding box"
                );
                return false;
            }
            // Reject only when the nearest bbox point is outside the domain.
            // A centre-distance shortcut loses large walls whose centre lies far
            // away while an edge crosses the cone.
            let nearest_x = viewer
                .x
                .clamp(obs.box_ground.x_min(), obs.box_ground.x_max());
            let nearest_y = viewer
                .y
                .clamp(obs.box_ground.y_min(), obs.box_ground.y_max());
            let dx = nearest_x - viewer.x;
            let dy = nearest_y - viewer.y;
            if dx * dx + dy * dy > radius_sq {
                return false;
            }
            // Cone flanking-ray rejection: if the obstacle's bbox lies
            // entirely outside one of the two cone sides, it can't shadow
            // anything we care about.
            if !is_box_inside_field(viewer, &obs.box_ground, left_side, right_side) {
                return false;
            }
            true
        })
        .collect();

    if relevant_obstacles.is_empty() {
        return vec![view_cone];
    }

    // Convert view cone to geo::Polygon.
    let cone_poly = {
        let mut coords: Vec<geo::Coord<f32>> = view_cone
            .iter()
            .map(|p| geo::Coord { x: p.x, y: p.y })
            .collect();
        // geo::LineString::from auto-closes but let's ensure exterior is closed.
        if let (Some(first), Some(last)) = (coords.first().copied(), coords.last().copied())
            && first != last
        {
            coords.push(first);
        }
        geo::Polygon::new(geo::LineString::new(coords), vec![])
    };

    let surfaces = [
        robin_engine::shadow_polygon::VisibilitySurface {
            plane: projection_plane,
            height: 0.0,
        },
        robin_engine::shadow_polygon::VisibilitySurface {
            plane: projection_plane,
            height: CHARACTER_HEIGHT,
        },
    ];
    let current = robin_engine::shadow_polygon::visible_region_on_surfaces(
        &cone_poly,
        [viewer.x, viewer.y, params.viewer_z],
        &surfaces,
        &relevant_obstacles,
        params.radius * 8.0,
    );

    // Flatten MultiPolygon + holes into a list of rings. The rasteriser
    // applies the even-odd rule across all rings, so outer boundaries
    // and holes both "toggle" inside/outside — matching what we want
    // when a shadow creates a concavity or a hole in the visibility.
    let mut rings: Vec<Vec<GroundPoint>> = Vec::new();
    for poly in current.0.iter() {
        if poly.unsigned_area() < 0.1 {
            continue;
        }
        rings.push(linestring_to_points(poly.exterior()));
        for hole in poly.interiors() {
            rings.push(linestring_to_points(hole));
        }
    }
    rings
}

/// Convert a closed geo::LineString to a `Vec<GroundPoint>` (dropping the
/// trailing closing coordinate — the rasteriser treats the edge list
/// as implicitly closed).
fn linestring_to_points(ls: &geo::LineString<f32>) -> Vec<GroundPoint> {
    let coords: Vec<_> = ls.coords().collect();
    let n = coords.len();
    // A closed ring has the last == first; drop the duplicate.
    let end = if n >= 2 && coords[0] == coords[n - 1] {
        n - 1
    } else {
        n
    };
    coords[..end]
        .iter()
        .map(|c| GroundPoint { x: c.x, y: c.y })
        .collect()
}

fn points_to_polygon(points: &[GroundPoint]) -> Option<geo::Polygon<f32>> {
    if points.len() < 3 {
        return None;
    }
    let mut coords: Vec<geo::Coord<f32>> = points
        .iter()
        .map(|p| geo::Coord { x: p.x, y: p.y })
        .collect();
    if let (Some(first), Some(last)) = (coords.first().copied(), coords.last().copied())
        && first != last
    {
        coords.push(first);
    }
    Some(geo::Polygon::new(geo::LineString::new(coords), vec![]))
}

/// Project computed visibility polygons onto a projection-area plane and
/// clip them to that surface's projected polygon.
///
/// This mirrors the original game's projection-area lookup and
/// screen-coordinate display path for non-ground slices: points are
/// converted to `(x, y - plane_height(x, y))`. The original game then adds the
/// projection area's sector plane as slice iterators during slice preparation;
/// clipping to `polygon_projection` gives the same surface outline to the GPU
/// polygon path.
pub fn project_and_clip_to_projection_area(
    visible_polygons: &[Vec<GroundPoint>],
    viewer: GroundPoint,
    projection_plane: engine_position_interface::PlaneZCoeffs,
    projection_area: &SightObstacle,
    occluding_projection_areas: &[&SightObstacle],
) -> (Vec<Vec<GroundPoint>>, GroundPoint) {
    let project = |p: GroundPoint| GroundPoint {
        x: p.x,
        y: p.y - projection_plane.compute_world_z(p.x, p.y),
    };

    if projection_area
        .polygon_projection
        .as_geo()
        .exterior()
        .0
        .len()
        < 3
    {
        tracing::warn!(
            "projection-area obstacle {} has no projected polygon for view-cone clipping",
            projection_area.id
        );
        return (Vec::new(), project(viewer));
    }

    let mut rings = Vec::new();
    let blockers: Vec<geo::Polygon<f32>> = occluding_projection_areas
        .iter()
        .filter(|obs| obs.polygon_projection.as_geo().exterior().0.len() >= 3)
        .map(|obs| obs.polygon_projection.as_geo().clone())
        .collect();
    let blocker_union = (!blockers.is_empty()).then(|| unary_union(&blockers));
    for poly in visible_polygons {
        let projected: Vec<GroundPoint> = poly.iter().copied().map(project).collect();
        let Some(projected_poly) = points_to_polygon(&projected) else {
            continue;
        };
        let clipped = projected_poly.intersection(projection_area.polygon_projection.as_geo());
        let clipped = if let Some(blocker_union) = &blocker_union {
            clipped.difference(blocker_union)
        } else {
            clipped
        };
        for clipped_poly in clipped.0.iter() {
            if clipped_poly.unsigned_area() < 0.1 {
                continue;
            }
            rings.push(linestring_to_points(clipped_poly.exterior()));
            for hole in clipped_poly.interiors() {
                rings.push(linestring_to_points(hole));
            }
        }
    }

    (rings, project(viewer))
}

/// Returns `true` if the obstacle bbox *might* overlap the view cone —
/// i.e. it isn't entirely on the outside of one of the cone's flanking
/// rays.
///
/// The test picks the single bbox corner most likely to be on the inside
/// side of each flanking ray (based on the ray direction's component
/// signs). If that best-case corner is still on the outside, every other
/// corner must be too, so the box is outside and we reject.
///
/// `left_side` / `right_side` are the iso-squashed flanking-ray
/// directions (not scaled by radius).
fn is_box_inside_field(
    viewer: GroundPoint,
    box_ground: &GroundBBox,
    left_side: [f32; 2],
    right_side: [f32; 2],
) -> bool {
    if !box_ground.is_somewhere() {
        return false;
    }
    let x_min = box_ground.x_min();
    let x_max = box_ground.x_max();
    let y_min = box_ground.y_min();
    let y_max = box_ground.y_max();

    // Pick the corner whose signed area against the flanking ray is
    // maximal (i.e. most likely to sit on the inside side). Reject only
    // when even this best-case corner is on the outside.
    //
    // `det(v, p - viewer)` = v.x * (p.y - viewer.y) - v.y * (p.x - viewer.x).
    // For the LEFT ray, "inside" means det >= 0; "outside" means det < 0.
    // For the RIGHT ray, "inside" means det <= 0; "outside" means det > 0.
    let (lx, ly) = (
        x_for_left(left_side, x_min, x_max),
        y_for_left(left_side, y_min, y_max),
    );
    let det_l = left_side[0] * (ly - viewer.y) - left_side[1] * (lx - viewer.x);
    if det_l < 0.0 {
        return false;
    }

    let (rx, ry) = (
        x_for_right(right_side, x_min, x_max),
        y_for_right(right_side, y_min, y_max),
    );
    let det_r = right_side[0] * (ry - viewer.y) - right_side[1] * (rx - viewer.x);
    det_r <= 0.0
}

// Corner-picking logic: for the LEFT ray the inside-side corner is the
// one that makes `det(left, corner - viewer)` largest. The sign of each
// component of `left_side` determines which corner that is.
#[inline]
fn x_for_left(v: [f32; 2], x_min: f32, x_max: f32) -> f32 {
    // left.x > 0 & left.y > 0 → x_min (top-left x, bottom y)
    // left.x > 0 & left.y < 0 → x_max (bottom-right corner)
    // left.x < 0 & left.y > 0 → x_min (top-left corner)
    // left.x < 0 & left.y < 0 → x_max (bottom-right x, top y)
    if v[1] >= 0.0 { x_min } else { x_max }
}
#[inline]
fn y_for_left(v: [f32; 2], y_min: f32, y_max: f32) -> f32 {
    if v[0] >= 0.0 { y_max } else { y_min }
}
#[inline]
fn x_for_right(v: [f32; 2], x_min: f32, x_max: f32) -> f32 {
    // Symmetric to the left case but with inverted inequality: the
    // "outside" of the right ray is det > 0, so the best-case corner is
    // the one that minimises det.
    if v[1] >= 0.0 { x_max } else { x_min }
}
#[inline]
fn y_for_right(v: [f32; 2], y_min: f32, y_max: f32) -> f32 {
    if v[0] >= 0.0 { y_min } else { y_max }
}

// ── Darkening pass ──────────────────────────────────────────────

/// Tint every pixel that lies **inside** any of the given visible
/// polygons.  Used to render the view-cone overlay for the selected
/// view element: the area the viewer can see is highlighted with the
/// alert colour, while the rest of the map renders untouched.
///
/// Uses a scanline rasteriser that builds an edge table from the polygon
/// edges, then for each screen row determines inside intervals and
/// alpha-blends the tint over them.
///
/// `visible_polygons` — one per viewer.  Typically a single polygon
/// (the selected element's cone); multiple are supported so debug /
/// cheat modes can tint the union of several cones.
/// `view_rect` — world-space bounding box of the current camera view.
/// `zoom` — current zoom factor.
/// `tint` — RGB 0..255 to blend pixels towards.  For the PC overlay
/// this is effectively black; for NPCs it uses the alert-status colour.
/// `alpha` — tint strength 0 (invisible) .. 255 (opaque).
#[allow(clippy::too_many_arguments)]
pub fn render_darken_inside(
    renderer: &mut Renderer,
    view_rect: &MapBBox,
    zoom: f32,
    visible_polygons: &[Vec<GroundPoint>],
    tint: (u8, u8, u8),
    alpha: u8,
    viewer: GroundPoint,
    radius: f32,
    projection_plane: Option<engine_position_interface::PlaneZCoeffs>,
    masks: &[&engine_mask::RuntimeMask],
) {
    if alpha == 0 || visible_polygons.is_empty() {
        return;
    }

    if renderer.is_gpu_phase() {
        render_darken_inside_gpu_spans(
            renderer,
            view_rect,
            zoom,
            visible_polygons,
            tint,
            alpha,
            viewer,
            radius,
            projection_plane,
            masks,
        );
        return;
    }

    panic!("render_darken_inside called before flush_base_layer/GPU phase");
}

/// GPU path for `render_darken_inside`: the CPU builds scanline span geometry
/// and mask exclusions; the actual tint/fade blend is done by GPU quads.
#[allow(clippy::too_many_arguments)]
fn render_darken_inside_gpu_spans(
    renderer: &mut Renderer,
    view_rect: &MapBBox,
    zoom: f32,
    visible_polygons: &[Vec<GroundPoint>],
    tint: (u8, u8, u8),
    alpha: u8,
    viewer: GroundPoint,
    radius: f32,
    projection_plane: Option<engine_position_interface::PlaneZCoeffs>,
    masks: &[&engine_mask::RuntimeMask],
) {
    let w = renderer.screen_width() as i32;
    let h = renderer.screen_height() as i32;
    let inv_zoom = if zoom > 0.0 { 1.0 / zoom } else { 1.0 };

    let project = |p: GroundPoint| {
        let z = projection_plane
            .map(|plane| plane.compute_world_z(p.x, p.y))
            .unwrap_or(0.0);
        let projected_y = p.y - z;
        let sx = (p.x - view_rect.x_min()) * zoom;
        let sy = (projected_y - view_rect.y_min()) * zoom;
        [sx, sy]
    };

    // Convert all polygons from world → screen coordinates.
    let screen_polys: Vec<Vec<[f32; 2]>> = visible_polygons
        .iter()
        .map(|poly| poly.iter().map(|p| project(*p)).collect())
        .collect();
    let viewer_screen = project(viewer);

    let edge_tables: Vec<Vec<ScanEdge>> = screen_polys
        .iter()
        .map(|poly| build_edge_table(poly))
        .collect();

    // Only walk the rows actually covered by the polygons instead of the
    // full screen height, and reuse the per-row scratch buffers.
    let Some((y_start, y_end)) = edge_tables_y_extent(&edge_tables, h) else {
        return;
    };
    let mut visible_spans: Vec<(i32, i32)> = Vec::new();
    let mut crossings: Vec<f32> = Vec::new();
    let mut mask_spans = Vec::new();
    let mut unmasked_spans = Vec::new();
    for y in y_start..y_end {
        let yf = y as f32 + 0.5;
        visible_spans.clear();
        for edges in &edge_tables {
            crossings.clear();
            for edge in edges {
                if yf >= edge.y_min && yf < edge.y_max {
                    let x = edge.x_start + (yf - edge.y_min) * edge.dx_per_dy;
                    crossings.push(x);
                }
            }
            crossings.sort_unstable_by(f32::total_cmp);
            let mut i = 0;
            while i + 1 < crossings.len() {
                let x0 = (crossings[i].ceil() as i32).max(0);
                let x1 = (crossings[i + 1].floor() as i32 + 1).min(w);
                if x0 < x1 {
                    visible_spans.push((x0, x1));
                }
                i += 2;
            }
        }

        visible_spans.sort_unstable_by_key(|s| s.0);
        merge_spans_in_place(&mut visible_spans);
        if visible_spans.is_empty() {
            continue;
        }

        mask_spans_for_row_into(masks, view_rect, zoom, inv_zoom, y, w, &mut mask_spans);
        if !mask_spans.is_empty() {
            mask_spans.sort_unstable_by_key(|s| s.0);
            merge_spans_in_place(&mut mask_spans);
            subtract_spans_into(&visible_spans, &mask_spans, &mut unmasked_spans);
            std::mem::swap(&mut visible_spans, &mut unmasked_spans);
        }

        for &(start, end) in &visible_spans {
            let alpha_left =
                cone_alpha_at_screen(start as f32 + 0.5, yf, viewer_screen, zoom, radius, alpha);
            let alpha_right = cone_alpha_at_screen(
                (end - 1) as f32 + 0.5,
                yf,
                viewer_screen,
                zoom,
                radius,
                alpha,
            );
            renderer.render_view_cone_span(
                Rect::new(start, y, (end - start) as u32, 1),
                tint,
                alpha_left,
                alpha_right,
            );
        }
    }
}

fn mask_spans_for_row_into(
    masks: &[&engine_mask::RuntimeMask],
    view_rect: &MapBBox,
    zoom: f32,
    inv_zoom: f32,
    sy: i32,
    screen_w: i32,
    spans: &mut Vec<(i32, i32)>,
) {
    spans.clear();
    let world_y = view_rect.y_min() + (sy as f32 + 0.5) * inv_zoom;

    for mask in masks {
        if !mask.is_character() {
            continue;
        }
        let mw = mask.width as i32;
        let mh = mask.height as i32;
        if mw <= 0 || mh <= 0 {
            continue;
        }
        let mask_origin_x = mask.bbox.x_min();
        let mask_origin_y = mask.bbox.y_min();
        let by = (world_y - mask_origin_y).floor() as i32;
        if by < 0 || by >= mh {
            continue;
        }

        let sx_min = ((mask_origin_x - view_rect.x_min()) * zoom).floor() as i32;
        let sx_max = sx_min + (mw as f32 * zoom).ceil() as i32;
        let sx_from = sx_min.max(0);
        let sx_to = sx_max.min(screen_w);
        if sx_from >= sx_to {
            continue;
        }

        let bitmap_row = by as usize * mw as usize;
        let mut run_start: Option<i32> = None;
        for sx in sx_from..sx_to {
            let world_x = view_rect.x_min() + (sx as f32 + 0.5) * inv_zoom;
            let bx = (world_x - mask_origin_x).floor() as i32;
            let covered = bx >= 0 && bx < mw && mask.bitmap[bitmap_row + bx as usize] != 0;
            match (run_start, covered) {
                (None, true) => run_start = Some(sx),
                (Some(start), false) => {
                    spans.push((start, sx));
                    run_start = None;
                }
                _ => {}
            }
        }
        if let Some(start) = run_start {
            spans.push((start, sx_to));
        }
    }
}

fn subtract_spans_into(spans: &[(i32, i32)], cuts: &[(i32, i32)], out: &mut Vec<(i32, i32)>) {
    out.clear();
    if cuts.is_empty() {
        out.extend_from_slice(spans);
        return;
    }

    let mut cut_idx = 0;
    for &(span_start, span_end) in spans {
        let mut cursor = span_start;
        while cut_idx < cuts.len() && cuts[cut_idx].1 <= span_start {
            cut_idx += 1;
        }
        let mut idx = cut_idx;
        while idx < cuts.len() && cuts[idx].0 < span_end {
            let (cut_start, cut_end) = cuts[idx];
            if cut_start > cursor {
                out.push((cursor, cut_start.min(span_end)));
            }
            cursor = cursor.max(cut_end);
            if cursor >= span_end {
                break;
            }
            idx += 1;
        }
        if cursor < span_end {
            out.push((cursor, span_end));
        }
    }
}

fn cone_alpha_at_screen(
    screen_x: f32,
    screen_y: f32,
    viewer_screen: [f32; 2],
    zoom: f32,
    radius: f32,
    alpha_start: u8,
) -> u8 {
    if radius <= f32::EPSILON {
        return alpha_start;
    }
    let inv_zoom = if zoom > 0.0 { 1.0 / zoom } else { 1.0 };
    let dx = (screen_x - viewer_screen[0]) * inv_zoom;
    let dy = ((screen_y - viewer_screen[1]) * inv_zoom) / ASPECT_RATIO;
    let dist = (dx * dx + dy * dy).sqrt();
    let t = (1.0 - dist / radius).clamp(0.0, 1.0);
    let alpha = ALPHA_END as f32 + (alpha_start.saturating_sub(ALPHA_END) as f32 * t);
    alpha.round().clamp(ALPHA_END as f32, alpha_start as f32) as u8
}

/// GPU path for `render_tinted_cones`: fills INSIDE each polygon with its
/// own tint and distance fade, so overlapping cones blend naturally.
fn render_tinted_cones_gpu(
    renderer: &mut Renderer,
    view_rect: &MapBBox,
    zoom: f32,
    cones: &[TintedCone],
) {
    for (poly, tint, viewer, radius, alpha, projection_plane) in cones {
        render_darken_inside_gpu_spans(
            renderer,
            view_rect,
            zoom,
            std::slice::from_ref(poly),
            *tint,
            *alpha,
            *viewer,
            *radius,
            *projection_plane,
            &[],
        );
    }
}

/// Return the darken alpha level for the current ambiance.
pub fn alpha_for_ambiance(is_night_or_fog: bool) -> u8 {
    if is_night_or_fog {
        ALPHA_NIGHT
    } else {
        ALPHA_DAY
    }
}

/// Render each cone filled with its own tint colour (for `--view-cones`).
pub fn render_tinted_cones(
    renderer: &mut Renderer,
    view_rect: &MapBBox,
    zoom: f32,
    cones: &[TintedCone],
) {
    if cones.is_empty() {
        return;
    }
    if renderer.is_gpu_phase() {
        render_tinted_cones_gpu(renderer, view_rect, zoom, cones);
    }
}

// ── Geometry helpers ────────────────────────────────────────────────────

/// Normalise a 2D direction vector.
fn normalise(d: [f32; 2]) -> [f32; 2] {
    let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
    if len > 1e-6 {
        [d[0] / len, d[1] / len]
    } else {
        [1.0, 0.0]
    }
}

/// Rotate a 2D vector by `angle` radians (CCW).
fn rotate(v: [f32; 2], angle: f32) -> [f32; 2] {
    let (sin_a, cos_a) = angle.sin_cos();
    [v[0] * cos_a - v[1] * sin_a, v[0] * sin_a + v[1] * cos_a]
}

/// Apply isometric Y compression to a direction vector.
fn iso(v: [f32; 2]) -> [f32; 2] {
    [v[0], v[1] * ASPECT_RATIO]
}

// ── Scanline rasteriser helpers ─────────────────────────────────────────

/// A polygon edge in the edge table, parameterised for scanline intersection.
struct ScanEdge {
    y_min: f32,
    y_max: f32,
    x_start: f32,   // X at y_min
    dx_per_dy: f32, // change in X per unit Y
}

/// Build an edge table from a screen-space polygon.
fn build_edge_table(poly: &[[f32; 2]]) -> Vec<ScanEdge> {
    let n = poly.len();
    if n < 3 {
        return Vec::new();
    }
    let mut edges = Vec::with_capacity(n);
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        let dy = b[1] - a[1];
        if dy.abs() < 0.001 {
            continue; // skip horizontal edges
        }
        let (y_min, y_max, x_start);
        if a[1] < b[1] {
            y_min = a[1];
            y_max = b[1];
            x_start = a[0];
        } else {
            y_min = b[1];
            y_max = a[1];
            x_start = b[0];
        }
        let dx_per_dy = (b[0] - a[0]) / dy;
        edges.push(ScanEdge {
            y_min,
            y_max,
            x_start,
            dx_per_dy,
        });
    }
    edges
}

/// Vertical screen extent (half-open row range) covered by the edge
/// tables, clamped to `0..h`. `None` if no edges cover any on-screen row.
fn edge_tables_y_extent(edge_tables: &[Vec<ScanEdge>], h: i32) -> Option<(i32, i32)> {
    let mut ext_min = f32::INFINITY;
    let mut ext_max = f32::NEG_INFINITY;
    for edges in edge_tables {
        for edge in edges {
            ext_min = ext_min.min(edge.y_min);
            ext_max = ext_max.max(edge.y_max);
        }
    }
    if !(ext_min < ext_max) {
        return None;
    }
    let y_start = (ext_min.floor() as i32).max(0);
    let y_end = (ext_max.ceil() as i32).min(h);
    if y_start >= y_end {
        return None;
    }
    Some((y_start, y_end))
}

/// Merge overlapping or adjacent intervals in place. Input must be sorted
/// by start. Allocation-free variant of `merge_spans` for hot loops.
fn merge_spans_in_place(spans: &mut Vec<(i32, i32)>) {
    if spans.is_empty() {
        return;
    }
    let mut write = 0;
    for read in 1..spans.len() {
        let (start, end) = spans[read];
        if start <= spans[write].1 {
            spans[write].1 = spans[write].1.max(end);
        } else {
            write += 1;
            spans[write] = (start, end);
        }
    }
    spans.truncate(write + 1);
}

/// Merge overlapping or adjacent intervals. Input must be sorted by start.
#[cfg(test)]
fn merge_spans(spans: &[(i32, i32)]) -> Vec<(i32, i32)> {
    if spans.is_empty() {
        return Vec::new();
    }
    let mut merged = Vec::with_capacity(spans.len());
    let mut current = spans[0];
    for &(start, end) in &spans[1..] {
        if start <= current.1 {
            current.1 = current.1.max(end);
        } else {
            merged.push(current);
            current = (start, end);
        }
    }
    merged.push(current);
    merged
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_cone_basic_shape() {
        let params = ViewParameters {
            direction: [1.0, 0.0],
            half_aperture: NORMAL_HALF_APERTURE,
            radius: 200.0,
            alpha: ALPHA_DAY,
            lean_out: false,
            viewer_z: 0.0,
            projection_plane: None,
            projection_obstacle: None,
        };
        let cone = compute_view_cone_geo(GroundPoint { x: 100.0, y: 100.0 }, &params);

        // First point is the viewer
        assert_eq!(cone[0].x, 100.0);
        assert_eq!(cone[0].y, 100.0);

        // Should have center + left + arc + right = at least 4 points
        assert!(cone.len() >= 4, "cone has {} points", cone.len());

        // All arc points should be roughly `radius` from viewer
        for p in &cone[1..] {
            let dx = p.x - 100.0;
            let _dy = p.y - 100.0;
            // With aspect ratio, the Y is squashed, so raw distance
            // won't exactly equal radius — but X extent should be ≤ radius
            assert!(dx.abs() <= 201.0, "point x={} too far from viewer", p.x);
        }
    }

    #[test]
    fn view_cone_direction_affects_shape() {
        let params_right = ViewParameters {
            direction: [1.0, 0.0],
            half_aperture: NORMAL_HALF_APERTURE,
            radius: 200.0,
            alpha: ALPHA_DAY,
            lean_out: false,
            viewer_z: 0.0,
            projection_plane: None,
            projection_obstacle: None,
        };
        let params_left = ViewParameters {
            direction: [-1.0, 0.0],
            ..params_right.clone()
        };

        let origin = GroundPoint { x: 500.0, y: 500.0 };
        let cone_right = compute_view_cone_geo(origin, &params_right);
        let cone_left = compute_view_cone_geo(origin, &params_left);

        // Average X of arc points should be > viewer for right, < for left
        let avg_x_right: f32 =
            cone_right[1..].iter().map(|p| p.x).sum::<f32>() / (cone_right.len() - 1) as f32;
        let avg_x_left: f32 =
            cone_left[1..].iter().map(|p| p.x).sum::<f32>() / (cone_left.len() - 1) as f32;

        assert!(avg_x_right > 500.0, "right cone should extend right");
        assert!(avg_x_left < 500.0, "left cone should extend left");
    }

    #[test]
    fn visibility_no_obstacles_equals_cone() {
        let params = ViewParameters::default();
        let viewer = GroundPoint { x: 500.0, y: 500.0 };

        let cone = compute_view_cone_geo(viewer, &params);
        let vis = compute_visibility_polygon(viewer, &params, &[]);

        assert_eq!(vis.len(), 1, "no obstacles → single visibility polygon");
        let vis0 = &vis[0];
        assert_eq!(cone.len(), vis0.len());
        for (a, b) in cone.iter().zip(vis0.iter()) {
            assert!((a.x - b.x).abs() < 0.001);
            assert!((a.y - b.y).abs() < 0.001);
        }
    }

    #[test]
    fn visibility_clips_against_obstacle() {
        use robin_engine::sight_obstacle::{ObstaclePoint, SightObstacle};

        let viewer = GroundPoint { x: 0.0, y: 0.0 };
        let params = ViewParameters {
            direction: [1.0, 0.0],
            half_aperture: std::f32::consts::FRAC_PI_2, // 90° half → 180° total
            radius: 300.0,
            alpha: ALPHA_DAY,
            lean_out: false,
            viewer_z: 0.0,
            projection_plane: None,
            projection_obstacle: None,
        };

        // Place a small obstacle directly ahead
        let mut obs = SightObstacle::new_default(0);
        obs.obstacle_points = vec![
            ObstaclePoint {
                x: 100.0,
                y: -20.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 120.0,
                y: -20.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 120.0,
                y: 20.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 100.0,
                y: 20.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
        ];
        obs.rebuild_geometry();

        let cone = compute_view_cone_geo(viewer, &params);
        let vis = compute_visibility_polygon(viewer, &params, &[&obs]);

        // The clipped result should carve a concavity behind the obstacle.
        // The full cone is a simple fan; after subtraction we expect a
        // non-convex polygon with MORE vertices than the original cone
        // (the wedge intersections add 2-4 new vertices).
        assert!(
            !vis.is_empty() && vis[0].len() >= cone.len(),
            "expected clipped polygon to gain vertices from shadow wedge; cone={} vis={:?}",
            cone.len(),
            vis.iter().map(Vec::len).collect::<Vec<_>>()
        );
    }

    #[test]
    fn merge_spans_basic() {
        assert_eq!(
            merge_spans(&[(0, 5), (3, 8), (10, 15)]),
            vec![(0, 8), (10, 15)]
        );
        assert_eq!(merge_spans(&[(0, 10)]), vec![(0, 10)]);
        assert_eq!(merge_spans(&[]), Vec::<(i32, i32)>::new());
    }

    #[test]
    fn merge_spans_in_place_matches_merge_spans() {
        for input in [
            vec![(0, 5), (3, 8), (10, 15)],
            vec![(0, 10)],
            vec![],
            vec![(0, 2), (2, 4), (5, 6), (5, 9)],
        ] {
            let mut in_place = input.clone();
            merge_spans_in_place(&mut in_place);
            assert_eq!(in_place, merge_spans(&input));
        }
    }

    #[test]
    fn edge_tables_y_extent_bounds_rows() {
        let tri = vec![[0.0, 10.3], [100.0, 10.3], [50.0, 90.7]];
        let tables = vec![build_edge_table(&tri)];
        // Extent covers floor(min)..ceil(max), clamped to screen height.
        assert_eq!(edge_tables_y_extent(&tables, 1080), Some((10, 91)));
        assert_eq!(edge_tables_y_extent(&tables, 50), Some((10, 50)));
        // Entirely above the screen → nothing to draw.
        let above = vec![[0.0, -50.0], [100.0, -50.0], [50.0, -10.0]];
        let tables = vec![build_edge_table(&above)];
        assert_eq!(edge_tables_y_extent(&tables, 1080), None);
        // No edges at all.
        assert_eq!(edge_tables_y_extent(&[Vec::new()], 1080), None);
    }

    #[test]
    fn edge_table_triangle() {
        let tri = vec![[0.0, 0.0], [100.0, 0.0], [50.0, 100.0]];
        let edges = build_edge_table(&tri);
        // 3 edges minus any horizontal; the bottom edge y=0→y=0 is horizontal → skipped
        assert_eq!(
            edges.len(),
            2,
            "triangle should have 2 non-horizontal edges"
        );
    }

    #[test]
    fn view_parameters_serde_roundtrip() {
        let vp = ViewParameters::default();
        let json = serde_json::to_string(&vp).unwrap();
        let back: ViewParameters = serde_json::from_str(&json).unwrap();
        assert!((back.half_aperture - NORMAL_HALF_APERTURE).abs() < 0.001);
    }

    /// Helper to build a SightObstacle with given ground points.
    fn make_obstacle_with_points(
        pts: &[(f32, f32)],
    ) -> robin_engine::sight_obstacle::SightObstacle {
        let mut obs = SightObstacle::new_default(0);
        obs.obstacle_points = pts
            .iter()
            .map(|&(x, y)| robin_engine::sight_obstacle::ObstaclePoint {
                x,
                y,
                z_top: 5.0,
                z_bottom: 0.0,
            })
            .collect();
        obs.rebuild_geometry();
        obs
    }

    #[test]
    fn view_cone_winding_ccw_obstacle_clips_correctly() {
        // CCW square obstacle at (50,0)..(60,10) — should clip a
        // narrow shadow from the right side of a view cone centered
        // at the origin looking right.
        let obs =
            make_obstacle_with_points(&[(50.0, 0.0), (60.0, 0.0), (60.0, 10.0), (50.0, 10.0)]);
        let params = ViewParameters {
            direction: [1.0, 0.0], // looking right
            radius: 200.0,
            half_aperture: 45.0_f32.to_radians(),
            alpha: 128,
            lean_out: false,
            viewer_z: 0.0,
            projection_plane: None,
            projection_obstacle: None,
        };
        let viewer = GroundPoint { x: 0.0, y: 5.0 };
        let result = compute_visibility_polygon(viewer, &params, &[&obs]);
        // Expect a single non-convex polygon with a wedge bite. It
        // must not degenerate to a sliver (< 4 vertices total across
        // all rings was the bug symptom).
        let total: usize = result.iter().map(Vec::len).sum();
        assert!(
            total >= 4,
            "CCW obstacle produced {} total vertices across {:?} rings (degenerate)",
            total,
            result.iter().map(Vec::len).collect::<Vec<_>>()
        );
    }

    #[test]
    fn view_cone_winding_cw_obstacle_clips_correctly() {
        // CW square obstacle (reversed winding) — the winding detection
        // should still produce correct silhouette edges.
        let obs =
            make_obstacle_with_points(&[(50.0, 10.0), (60.0, 10.0), (60.0, 0.0), (50.0, 0.0)]);
        let params = ViewParameters {
            direction: [1.0, 0.0],
            radius: 200.0,
            half_aperture: 45.0_f32.to_radians(),
            alpha: 128,
            lean_out: false,
            viewer_z: 0.0,
            projection_plane: None,
            projection_obstacle: None,
        };
        let viewer = GroundPoint { x: 0.0, y: 5.0 };
        let result = compute_visibility_polygon(viewer, &params, &[&obs]);
        let total: usize = result.iter().map(Vec::len).sum();
        assert!(
            total >= 4,
            "CW obstacle produced {} total vertices across {:?} rings (degenerate)",
            total,
            result.iter().map(Vec::len).collect::<Vec<_>>()
        );
    }

    #[test]
    fn view_cone_winding_both_produce_similar_results() {
        // CCW and CW versions of the same obstacle should produce
        // visibility polygons with the same total vertex count.
        let ccw =
            make_obstacle_with_points(&[(50.0, 0.0), (60.0, 0.0), (60.0, 10.0), (50.0, 10.0)]);
        let cw = make_obstacle_with_points(&[(50.0, 10.0), (60.0, 10.0), (60.0, 0.0), (50.0, 0.0)]);
        let params = ViewParameters {
            direction: [1.0, 0.0], // looking right
            radius: 200.0,
            half_aperture: 45.0_f32.to_radians(),
            alpha: 128,
            lean_out: false,
            viewer_z: 0.0,
            projection_plane: None,
            projection_obstacle: None,
        };
        let viewer = GroundPoint { x: 0.0, y: 5.0 };
        let ccw_result = compute_visibility_polygon(viewer, &params, &[&ccw]);
        let cw_result = compute_visibility_polygon(viewer, &params, &[&cw]);
        let ccw_total: usize = ccw_result.iter().map(Vec::len).sum();
        let cw_total: usize = cw_result.iter().map(Vec::len).sum();
        assert_eq!(
            ccw_total, cw_total,
            "CCW ({}) and CW ({}) should produce same total vertex count",
            ccw_total, cw_total
        );
    }

    // ── flanking-ray rejection / usefulness predicate tests ───────

    #[test]
    fn box_inside_field_rejects_obstacle_outside_left_flank() {
        // Cone looks right (+x) with ±30° half-aperture. An obstacle
        // far to the left rear should be rejected by the flanking-ray
        // test before any shadow computation.
        let viewer = GroundPoint { x: 0.0, y: 0.0 };
        let params = ViewParameters {
            direction: [1.0, 0.0],
            half_aperture: 30.0_f32.to_radians(),
            radius: 400.0,
            ..ViewParameters::default()
        };
        let dir = normalise(params.direction);
        let left_side = iso(rotate(dir, -params.half_aperture));
        let right_side = iso(rotate(dir, params.half_aperture));

        // Behind-left obstacle: (-200..-180, -10..10)
        let mut obs = SightObstacle::new_default(0);
        obs.obstacle_points = vec![
            robin_engine::sight_obstacle::ObstaclePoint {
                x: -200.0,
                y: -10.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
            robin_engine::sight_obstacle::ObstaclePoint {
                x: -180.0,
                y: -10.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
            robin_engine::sight_obstacle::ObstaclePoint {
                x: -180.0,
                y: 10.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
            robin_engine::sight_obstacle::ObstaclePoint {
                x: -200.0,
                y: 10.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
        ];
        obs.rebuild_geometry();

        assert!(
            !is_box_inside_field(viewer, &obs.box_ground, left_side, right_side),
            "obstacle entirely behind/left of the cone should be rejected"
        );
    }

    #[test]
    fn box_inside_field_accepts_obstacle_inside_cone() {
        let viewer = GroundPoint { x: 0.0, y: 0.0 };
        let params = ViewParameters {
            direction: [1.0, 0.0],
            half_aperture: 30.0_f32.to_radians(),
            radius: 400.0,
            ..ViewParameters::default()
        };
        let dir = normalise(params.direction);
        let left_side = iso(rotate(dir, -params.half_aperture));
        let right_side = iso(rotate(dir, params.half_aperture));

        // Obstacle directly ahead along +x.
        let mut obs = SightObstacle::new_default(0);
        obs.obstacle_points = vec![
            robin_engine::sight_obstacle::ObstaclePoint {
                x: 100.0,
                y: -10.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
            robin_engine::sight_obstacle::ObstaclePoint {
                x: 120.0,
                y: -10.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
            robin_engine::sight_obstacle::ObstaclePoint {
                x: 120.0,
                y: 10.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
            robin_engine::sight_obstacle::ObstaclePoint {
                x: 100.0,
                y: 10.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
        ];
        obs.rebuild_geometry();

        assert!(
            is_box_inside_field(viewer, &obs.box_ground, left_side, right_side),
            "obstacle inside the cone should be kept"
        );
    }

    #[test]
    fn visibility_skips_obstacles_above_viewer_head() {
        // Regression test: a high elevated obstacle shouldn't clip
        // the eye-level visibility polygon.
        use robin_engine::sight_obstacle::{ObstaclePoint, SightObstacle};
        let viewer = GroundPoint { x: 0.0, y: 0.0 };
        let params = ViewParameters {
            direction: [1.0, 0.0],
            half_aperture: 45.0_f32.to_radians(),
            radius: 400.0,
            alpha: ALPHA_DAY,
            lean_out: false,
            viewer_z: 0.0,
            projection_plane: None,
            projection_obstacle: None,
        };

        let mut sky_slab = SightObstacle::new_default(0);
        sky_slab.obstacle_points = vec![
            ObstaclePoint {
                x: 100.0,
                y: -20.0,
                z_top: 500.0,
                z_bottom: 400.0,
            },
            ObstaclePoint {
                x: 120.0,
                y: -20.0,
                z_top: 500.0,
                z_bottom: 400.0,
            },
            ObstaclePoint {
                x: 120.0,
                y: 20.0,
                z_top: 500.0,
                z_bottom: 400.0,
            },
            ObstaclePoint {
                x: 100.0,
                y: 20.0,
                z_top: 500.0,
                z_bottom: 400.0,
            },
        ];
        sky_slab.rebuild_geometry();

        let cone = compute_view_cone_geo(viewer, &params);
        let vis = compute_visibility_polygon(viewer, &params, &[&sky_slab]);
        // Without height filtering the slab would still cast a shadow
        // wedge because it sits inside the cone's ground bbox. The
        // usefulness predicate must reject it so the visibility polygon
        // equals the unclipped cone.
        assert_eq!(vis.len(), 1);
        assert_eq!(cone.len(), vis[0].len());
    }

    #[test]
    fn visibility_skips_obstacles_outside_cone_flanks() {
        // Regression test for is_box_inside_field rejection: an
        // obstacle to the side of the view cone shouldn't contribute
        // a shadow wedge.
        use robin_engine::sight_obstacle::{ObstaclePoint, SightObstacle};
        let viewer = GroundPoint { x: 0.0, y: 0.0 };
        let params = ViewParameters {
            direction: [1.0, 0.0],
            // Narrow 10° half-aperture so only a thin strip in +x is visible.
            half_aperture: 10.0_f32.to_radians(),
            radius: 400.0,
            alpha: ALPHA_DAY,
            lean_out: false,
            viewer_z: 0.0,
            projection_plane: None,
            projection_obstacle: None,
        };

        // Obstacle behind the viewer — fully outside the cone.
        let mut behind = SightObstacle::new_default(0);
        behind.obstacle_points = vec![
            ObstaclePoint {
                x: -120.0,
                y: -10.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: -100.0,
                y: -10.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: -100.0,
                y: 10.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: -120.0,
                y: 10.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
        ];
        behind.rebuild_geometry();

        let cone = compute_view_cone_geo(viewer, &params);
        let vis = compute_visibility_polygon(viewer, &params, &[&behind]);
        assert_eq!(vis.len(), 1);
        assert_eq!(cone.len(), vis[0].len());
    }
}

#[test]
fn span_subtraction_matches_pixel_difference_and_reuses_output() {
    let spans_for_bits = |bits: u32| {
        let mut spans = (0..6)
            .filter(|x| bits & (1 << x) != 0)
            .map(|x| (x, x + 1))
            .collect::<Vec<_>>();
        merge_spans_in_place(&mut spans);
        spans
    };
    let mut output = Vec::with_capacity(12);
    let pointer = output.as_ptr();
    for visible in 0..64 {
        for hidden in 0..64 {
            output.push((-100, 100));
            subtract_spans_into(
                &spans_for_bits(visible),
                &spans_for_bits(hidden),
                &mut output,
            );
            assert_eq!(
                output,
                spans_for_bits(visible & !hidden),
                "visible={visible}, hidden={hidden}"
            );
            assert_eq!(output.as_ptr(), pointer);
        }
    }
}
