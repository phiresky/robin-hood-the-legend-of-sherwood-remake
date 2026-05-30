//! Shadow polygon — view cone overlay.
//!
//! **This is NOT a fog-of-war system** — the original game shows the full
//! map at all times.  This module implements the *view cone overlay* drawn
//! for the currently-selected view element: when the player alt-hovers an
//! NPC (or an ally), the map area *outside* that character's vision cone
//! is darkened so the player can see at a glance where the character can
//! and cannot see.
//!
//! The non-debug `Display()` / `DisplaySlice()` variants in the original
//! source are dead code — every call site uses the debug-display path.
//!
//! The original system is a 5000+ line 3D shadow projection engine with
//! stencil buffers, background decompression, and MMX-optimised scanline
//! blitters. This Rust implementation provides the same visual effect
//! using a cleaner 2D approach:
//!
//! 1. Build a **view cone** polygon (fan shape) for the viewer entity
//! 2. For each obstacle in range, **cast shadow edges** to clip the cone
//! 3. Darken pixels outside the visible polygon via alpha blending
//!
//! The rendering uses a scanline rasteriser that walks the visible polygon
//! edges and darkens every pixel that falls outside all visible regions.

use crate::geo2d::GeoPoint2D;
use crate::renderer::Renderer;
use crate::sight_obstacle::SightObstacle;
use geo::{Area, BooleanOps, algorithm::unary_union};
use robin_engine::sprite::BBox;

// Shared constants + types from the engine side.
pub use robin_engine::shadow_polygon::{
    ALPHA_DAY, ALPHA_NIGHT, ASPECT_RATIO, CHARACTER_HEIGHT, NORMAL_HALF_APERTURE, RADIUS_DAY,
    RADIUS_NIGHT, ViewParameters, sector_to_direction,
};

/// A visibility polygon paired with tint/fade metadata (for per-NPC alert coloring).
pub type TintedCone = (
    Vec<GeoPoint2D>,
    (u8, u8, u8),
    GeoPoint2D,
    f32,
    u8,
    Option<robin_engine::position_interface::PlaneZCoeffs>,
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
pub fn compute_view_cone(viewer: GeoPoint2D, params: &ViewParameters) -> Vec<GeoPoint2D> {
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
    poly.push(GeoPoint2D {
        x: viewer.x + left_iso[0] * radius,
        y: viewer.y + left_iso[1] * radius,
    });

    // Arc points from left towards right
    for i in 1..=num_steps {
        let angle = actual_step * i as f32;
        let v = rotate(left_dir, angle);
        let v_iso = iso(v);
        poly.push(GeoPoint2D {
            x: viewer.x + v_iso[0] * radius,
            y: viewer.y + v_iso[1] * radius,
        });
    }

    // Right edge
    let right_iso = iso(right_dir);
    poly.push(GeoPoint2D {
        x: viewer.x + right_iso[0] * radius,
        y: viewer.y + right_iso[1] * radius,
    });

    poly
}

// ── Visibility polygon (view cone clipped by obstacles) ─────────

/// Compute the visible-area polygon(s), clipping the view cone against
/// nearby obstacles.
///
/// For each opaque obstacle in range, a shadow wedge polygon is built:
/// two rays cast from the viewer through the obstacle's two silhouette
/// vertices and extended past the view cone, joined by a far-cap line.
/// The wedge is subtracted from the view cone using polygon boolean
/// difference (geo crate).
///
/// Returns a list of polygons (zero or more) in world coordinates —
/// usually a single non-convex polygon, but obstacle arrangements can
/// produce disjoint visible regions. Polygons are returned with their
/// exterior rings only; holes are flattened as separate rings because
/// the even-odd scanline rasteriser in `render_darken_inside` /
/// `render_tinted_cones` treats every ring uniformly.
pub fn compute_visibility_polygon(
    viewer: GeoPoint2D,
    params: &ViewParameters,
    obstacles: &[&SightObstacle],
) -> Vec<Vec<GeoPoint2D>> {
    let view_cone = compute_view_cone(viewer, params);

    if obstacles.is_empty() {
        return vec![view_cone];
    }

    let radius_sq = params.radius * params.radius;
    // Extend shadow rays well past the view cone so the wedge's far
    // cap lies safely outside the cone after polygon subtraction.
    let far = params.radius * 8.0;

    // Iso-squashed flanking-ray directions. `iso()` applies the same
    // `y *= ASPECT_RATIO` squash as the view-cone polygon itself, so
    // the cone sides, bbox and `is_box_inside_field` test all live in
    // the same reference frame.
    let dir = normalise(params.direction);
    let left_side = iso(rotate(dir, -params.half_aperture));
    let right_side = iso(rotate(dir, params.half_aperture));

    // Build shadow wedge polygons.
    let mut shadows: Vec<geo::Polygon<f32>> = Vec::new();
    for obs in obstacles {
        // Caller filtered to active obstacles already.
        if !obs.is_opaque() {
            continue;
        }
        // Quick distance reject (obstacle centre within 2×radius).
        let cx = (obs.box_ground.x_min() + obs.box_ground.x_max()) * 0.5;
        let cy = (obs.box_ground.y_min() + obs.box_ground.y_max()) * 0.5;
        let dx = cx - viewer.x;
        let dy = cy - viewer.y;
        if dx * dx + dy * dy > radius_sq * 4.0 {
            continue;
        }
        // Cone flanking-ray rejection: if the obstacle's bbox lies
        // entirely outside one of the two cone sides, it can't shadow
        // anything we care about.
        if !is_box_inside_field(viewer, &obs.box_ground, left_side, right_side) {
            continue;
        }
        // Height-based usefulness filter: an obstacle only contributes
        // a shadow when at least one of its silhouette vertices
        // straddles the viewer's horizontal eye-level ray.
        if !is_obstacle_useful(obs, params.viewer_z) {
            continue;
        }
        if let Some(shadow) = compute_shadow_polygon(viewer, params.viewer_z, obs, far) {
            shadows.push(shadow);
        }
    }

    if shadows.is_empty() {
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

    // Union all shadow wedges into a single MultiPolygon, then subtract
    // with one difference op. This replaces N sequential difference calls
    // with 1 union + 1 difference — an order-of-magnitude win in i_overlay
    // when many obstacles contribute shadows. `unary_union` handles
    // overlapping wedges correctly (plain MultiPolygon + EvenOdd would XOR
    // the overlap and leave a false visibility hole).
    let merged_shadows = unary_union(&shadows);
    let cone_mp: geo::MultiPolygon<f32> = geo::MultiPolygon::new(vec![cone_poly]);
    let current = cone_mp.difference(&merged_shadows);

    // Flatten MultiPolygon + holes into a list of rings. The rasteriser
    // applies the even-odd rule across all rings, so outer boundaries
    // and holes both "toggle" inside/outside — matching what we want
    // when a shadow creates a concavity or a hole in the visibility.
    let mut rings: Vec<Vec<GeoPoint2D>> = Vec::new();
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

/// Convert a closed geo::LineString to a `Vec<GeoPoint2D>` (dropping the
/// trailing closing coordinate — the rasteriser treats the edge list
/// as implicitly closed).
fn linestring_to_points(ls: &geo::LineString<f32>) -> Vec<GeoPoint2D> {
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
        .map(|c| GeoPoint2D { x: c.x, y: c.y })
        .collect()
}

fn points_to_polygon(points: &[GeoPoint2D]) -> Option<geo::Polygon<f32>> {
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
/// clip them to that surface's screen-space polygon.
///
/// This mirrors the original `RHShadowPolygon::GetProjectionAreas` /
/// `SetScreenCoords` display path for non-ground slices: points are
/// converted to `(x, y - plane.ComputeZ(x, y))`. C++ then adds the
/// projection area's sector plane as slice iterators in `PrepareSlice`;
/// clipping to `polygon_screen` gives the same surface outline to the GPU
/// polygon path.
pub fn project_and_clip_to_projection_area(
    visible_polygons: &[Vec<GeoPoint2D>],
    viewer: GeoPoint2D,
    projection_plane: robin_engine::position_interface::PlaneZCoeffs,
    projection_area: &SightObstacle,
    occluding_projection_areas: &[&SightObstacle],
) -> (Vec<Vec<GeoPoint2D>>, GeoPoint2D) {
    let project = |p: GeoPoint2D| GeoPoint2D {
        x: p.x,
        y: p.y - projection_plane.compute_z(p.x, p.y),
    };

    if projection_area.polygon_screen.exterior().0.len() < 3 {
        tracing::warn!(
            "projection-area obstacle {} has no screen polygon for view-cone clipping",
            projection_area.id
        );
        return (Vec::new(), project(viewer));
    }

    let mut rings = Vec::new();
    let blockers: Vec<geo::Polygon<f32>> = occluding_projection_areas
        .iter()
        .filter(|obs| obs.polygon_screen.exterior().0.len() >= 3)
        .map(|obs| obs.polygon_screen.clone())
        .collect();
    let blocker_union = (!blockers.is_empty()).then(|| unary_union(&blockers));
    for poly in visible_polygons {
        let projected: Vec<GeoPoint2D> = poly.iter().copied().map(project).collect();
        let Some(projected_poly) = points_to_polygon(&projected) else {
            continue;
        };
        let clipped = projected_poly.intersection(&projection_area.polygon_screen);
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
    viewer: GeoPoint2D,
    box_ground: &robin_engine::geo2d::BBox2D,
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
    if det_r > 0.0 {
        return false;
    }

    true
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

/// Unified usefulness predicate. The reference engine runs multiple
/// projection planes (ground / head / slanted) and has a separate
/// predicate for each; the 2D visibility polygon uses a single eye-level
/// horizontal ray, so they collapse into:
///
///   the obstacle contributes a shadow iff at least one vertex has
///   `z_bottom ≤ viewer_z + CHARACTER_HEIGHT` AND at least one vertex
///   has `z_top ≥ viewer_z`.
///
/// An obstacle failing either half can't straddle the horizontal eye-
/// level ray anywhere along its silhouette, so it doesn't cast a shadow
/// in the view cone the player is visualising.
fn is_obstacle_useful(obs: &SightObstacle, viewer_z: f32) -> bool {
    let eye_ceiling = viewer_z + CHARACTER_HEIGHT;
    let mut has_below_ceiling = false;
    let mut has_above_eye = false;
    for p in &obs.obstacle_points {
        if p.z_bottom <= eye_ceiling {
            has_below_ceiling = true;
        }
        if p.z_top >= viewer_z {
            has_above_eye = true;
        }
        if has_below_ceiling && has_above_eye {
            return true;
        }
    }
    has_below_ceiling && has_above_eye
}

/// Build the shadow wedge polygon cast by one convex obstacle as seen
/// from `viewer`. Returns `None` if the obstacle is degenerate, the
/// viewer is inside, or the silhouette can't be identified.
///
/// For a convex obstacle with the viewer outside it, exactly two
/// silhouette vertices exist: one at the front→back edge-facing
/// transition and one at the back→front transition. The wedge extends
/// rays from the viewer through each silhouette vertex out to `far_dist`
/// — or, when the viewer sits above the obstacle's top, out to where the
/// 3D ray `(viewer, eye_z) → (vertex, z_top)` meets the ground plane.
/// The altitude-aware projection reproduces the top-plane fall-off
/// behaviour without reintroducing the scan-line machinery.
fn compute_shadow_polygon(
    viewer: GeoPoint2D,
    viewer_z: f32,
    obs: &SightObstacle,
    far_dist: f32,
) -> Option<geo::Polygon<f32>> {
    let pts = &obs.obstacle_points;
    let n = pts.len();
    if n < 3 {
        return None;
    }
    // Viewer inside obstacle → no meaningful silhouette. Skip.
    if obs.contains_point(viewer) {
        return None;
    }

    // Determine polygon winding so the outward-normal test is correct
    // regardless of how obstacle_points were wound in the source data.
    let signed_area: f32 = (0..n)
        .map(|i| {
            let a = pts[i].ground_point();
            let b = pts[(i + 1) % n].ground_point();
            a.x * b.y - b.x * a.y
        })
        .sum();
    let winding_sign: f32 = if signed_area >= 0.0 { 1.0 } else { -1.0 };

    // edge_back[i] = edge (pts[i] → pts[(i+1)%n]) faces away from the viewer.
    let mut edge_back: Vec<bool> = Vec::with_capacity(n);
    for i in 0..n {
        let a = pts[i].ground_point();
        let b = pts[(i + 1) % n].ground_point();
        let ex = b.x - a.x;
        let ey = b.y - a.y;
        let nx = ey * winding_sign;
        let ny = -ex * winding_sign;
        let mx = viewer.x - (a.x + b.x) * 0.5;
        let my = viewer.y - (a.y + b.y) * 0.5;
        edge_back.push(nx * mx + ny * my < 0.0);
    }

    // Find silhouette vertices. For a convex obstacle with viewer outside
    // the polygon there is exactly one front→back transition and one
    // back→front transition. The transition vertex is the endpoint shared
    // by edges (i, i+1).
    let mut fb: Option<usize> = None; // front→back: edge_back goes false→true
    let mut bf: Option<usize> = None; // back→front: edge_back goes true→false
    for i in 0..n {
        let next = (i + 1) % n;
        if !edge_back[i] && edge_back[next] {
            fb = Some(next);
        } else if edge_back[i] && !edge_back[next] {
            bf = Some(next);
        }
    }
    let (fb_idx, bf_idx) = match (fb, bf) {
        (Some(f), Some(b)) => (f, b),
        _ => return None, // all edges same facing → no shadow
    };

    let s1 = pts[fb_idx].ground_point(); // first silhouette (enter back-facing run)
    let s2 = pts[bf_idx].ground_point(); // second silhouette (exit back-facing run)
    let s1_z_top = pts[fb_idx].z_top;
    let s2_z_top = pts[bf_idx].z_top;

    // Project silhouette vertices to the ground-plane shadow cap.
    //
    // For a viewer at altitude `viewer_z` looking past the obstacle
    // silhouette at `z_top`, the shadow on the ground plane ends at
    // `t = viewer_z / (viewer_z - z_top)` along the viewer→vertex ray.
    // When `viewer_z ≤ z_top` the ray runs parallel to or above the
    // ground, so we fall back to the 2D `far_dist` extension to keep
    // the wedge a finite polygon that still covers the full view cone.
    let project_far = |s: GeoPoint2D, s_z_top: f32| -> Option<GeoPoint2D> {
        let dx = s.x - viewer.x;
        let dy = s.y - viewer.y;
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-4 {
            return None;
        }
        // Altitude-aware far-cap: only trust the 3D ground intersection
        // when the viewer is strictly above the silhouette top and the
        // resulting ground point is inside the 2D far-cap disc.
        let scale = if viewer_z > s_z_top {
            let t_ground = viewer_z / (viewer_z - s_z_top);
            let ground_dist = len * t_ground;
            if ground_dist > 0.0 && ground_dist < far_dist {
                t_ground
            } else {
                far_dist / len
            }
        } else {
            far_dist / len
        };
        Some(GeoPoint2D {
            x: viewer.x + dx * scale,
            y: viewer.y + dy * scale,
        })
    };
    let s1_far = project_far(s1, s1_z_top)?;
    let s2_far = project_far(s2, s2_z_top)?;

    // Wedge polygon: walk s1 → s1_far → s2_far → s2. With CCW obstacle
    // winding, s1 is the front→back transition (leading silhouette) and
    // s2 is the back→front transition (trailing silhouette); going via
    // the far side yields a polygon that covers every point behind the
    // obstacle from the viewer. The i_overlay engine behind
    // `BooleanOps::difference` is winding-insensitive (EvenOdd fill), so
    // we don't need to normalise the order further.
    let ring = geo::LineString::new(vec![
        geo::Coord { x: s1.x, y: s1.y },
        geo::Coord {
            x: s1_far.x,
            y: s1_far.y,
        },
        geo::Coord {
            x: s2_far.x,
            y: s2_far.y,
        },
        geo::Coord { x: s2.x, y: s2.y },
        geo::Coord { x: s1.x, y: s1.y },
    ]);
    Some(geo::Polygon::new(ring, vec![]))
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
    view_rect: &BBox,
    zoom: f32,
    visible_polygons: &[Vec<GeoPoint2D>],
    tint: (u8, u8, u8),
    alpha: u8,
    viewer: GeoPoint2D,
    radius: f32,
    projection_plane: Option<robin_engine::position_interface::PlaneZCoeffs>,
    masks: &[&robin_engine::mask::RuntimeMask],
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
    view_rect: &BBox,
    zoom: f32,
    visible_polygons: &[Vec<GeoPoint2D>],
    tint: (u8, u8, u8),
    alpha: u8,
    viewer: GeoPoint2D,
    radius: f32,
    projection_plane: Option<robin_engine::position_interface::PlaneZCoeffs>,
    masks: &[&robin_engine::mask::RuntimeMask],
) {
    let w = renderer.screen_width() as i32;
    let h = renderer.screen_height() as i32;
    let inv_zoom = if zoom > 0.0 { 1.0 / zoom } else { 1.0 };

    let project = |p: GeoPoint2D| {
        let z = projection_plane
            .map(|plane| plane.compute_z(p.x, p.y))
            .unwrap_or(0.0);
        let projected_y = p.y - z;
        let sx = (p.x - view_rect.min.x) * zoom;
        let sy = (projected_y - view_rect.min.y) * zoom;
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

    for y in 0..h {
        let yf = y as f32 + 0.5;
        let mut visible_spans: Vec<(i32, i32)> = Vec::new();
        for edges in &edge_tables {
            let mut crossings: Vec<f32> = Vec::new();
            for edge in edges {
                if yf >= edge.y_min && yf < edge.y_max {
                    let x = edge.x_start + (yf - edge.y_min) * edge.dx_per_dy;
                    crossings.push(x);
                }
            }
            crossings.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
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
        let mut spans = merge_spans(&visible_spans);
        if spans.is_empty() {
            continue;
        }

        let mut mask_spans = mask_spans_for_row(masks, view_rect, zoom, inv_zoom, y, w);
        if !mask_spans.is_empty() {
            mask_spans.sort_unstable_by_key(|s| s.0);
            spans = subtract_spans(&spans, &merge_spans(&mask_spans));
        }

        for (start, end) in spans {
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
                crate::gfx_types::Rect::new(start, y, (end - start) as u32, 1),
                tint,
                alpha_left,
                alpha_right,
            );
        }
    }
}

fn mask_spans_for_row(
    masks: &[&robin_engine::mask::RuntimeMask],
    view_rect: &BBox,
    zoom: f32,
    inv_zoom: f32,
    sy: i32,
    screen_w: i32,
) -> Vec<(i32, i32)> {
    let mut spans = Vec::new();
    let world_y = view_rect.min.y + (sy as f32 + 0.5) * inv_zoom;

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

        let sx_min = ((mask_origin_x - view_rect.min.x) * zoom).floor() as i32;
        let sx_max = sx_min + (mw as f32 * zoom).ceil() as i32;
        let sx_from = sx_min.max(0);
        let sx_to = sx_max.min(screen_w);
        if sx_from >= sx_to {
            continue;
        }

        let bitmap_row = by as usize * mw as usize;
        let mut run_start: Option<i32> = None;
        for sx in sx_from..sx_to {
            let world_x = view_rect.min.x + (sx as f32 + 0.5) * inv_zoom;
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

    spans
}

fn subtract_spans(spans: &[(i32, i32)], cuts: &[(i32, i32)]) -> Vec<(i32, i32)> {
    if cuts.is_empty() {
        return spans.to_vec();
    }

    let mut out = Vec::with_capacity(spans.len());
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
    out
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
    view_rect: &BBox,
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
    view_rect: &BBox,
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

/// Merge overlapping or adjacent intervals. Input must be sorted by start.
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
        let cone = compute_view_cone(GeoPoint2D { x: 100.0, y: 100.0 }, &params);

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

        let origin = GeoPoint2D { x: 500.0, y: 500.0 };
        let cone_right = compute_view_cone(origin, &params_right);
        let cone_left = compute_view_cone(origin, &params_left);

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
        let viewer = GeoPoint2D { x: 500.0, y: 500.0 };

        let cone = compute_view_cone(viewer, &params);
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
        use crate::sight_obstacle::{ObstaclePoint, SightObstacle};

        let viewer = GeoPoint2D { x: 0.0, y: 0.0 };
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

        let cone = compute_view_cone(viewer, &params);
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
    fn make_obstacle_with_points(pts: &[(f32, f32)]) -> crate::sight_obstacle::SightObstacle {
        let mut obs = crate::sight_obstacle::SightObstacle::new_default(0);
        obs.obstacle_points = pts
            .iter()
            .map(|&(x, y)| crate::sight_obstacle::ObstaclePoint {
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
        let viewer = GeoPoint2D { x: 0.0, y: 5.0 };
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
        let viewer = GeoPoint2D { x: 0.0, y: 5.0 };
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
        let viewer = GeoPoint2D { x: 0.0, y: 5.0 };
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
        let viewer = GeoPoint2D { x: 0.0, y: 0.0 };
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
        let mut obs = crate::sight_obstacle::SightObstacle::new_default(0);
        obs.obstacle_points = vec![
            crate::sight_obstacle::ObstaclePoint {
                x: -200.0,
                y: -10.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
            crate::sight_obstacle::ObstaclePoint {
                x: -180.0,
                y: -10.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
            crate::sight_obstacle::ObstaclePoint {
                x: -180.0,
                y: 10.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
            crate::sight_obstacle::ObstaclePoint {
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
        let viewer = GeoPoint2D { x: 0.0, y: 0.0 };
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
        let mut obs = crate::sight_obstacle::SightObstacle::new_default(0);
        obs.obstacle_points = vec![
            crate::sight_obstacle::ObstaclePoint {
                x: 100.0,
                y: -10.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
            crate::sight_obstacle::ObstaclePoint {
                x: 120.0,
                y: -10.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
            crate::sight_obstacle::ObstaclePoint {
                x: 120.0,
                y: 10.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
            crate::sight_obstacle::ObstaclePoint {
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
    fn is_obstacle_useful_straddles_eye_level() {
        use crate::sight_obstacle::{ObstaclePoint, SightObstacle};
        let mut obs = SightObstacle::new_default(0);
        obs.obstacle_points = vec![
            ObstaclePoint {
                x: 0.0,
                y: 0.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 10.0,
                y: 0.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 10.0,
                y: 10.0,
                z_top: 50.0,
                z_bottom: 0.0,
            },
        ];
        obs.rebuild_geometry();
        // Viewer at ground level: obstacle top >= 0 and bottom <= CHARACTER_HEIGHT → useful.
        assert!(is_obstacle_useful(&obs, 0.0));
        // Viewer at eye level 20: still straddled.
        assert!(is_obstacle_useful(&obs, 20.0));
    }

    #[test]
    fn is_obstacle_useful_rejects_low_obstacle_for_elevated_viewer() {
        use crate::sight_obstacle::{ObstaclePoint, SightObstacle};
        // A knee-high stump.
        let mut obs = SightObstacle::new_default(0);
        obs.obstacle_points = vec![
            ObstaclePoint {
                x: 0.0,
                y: 0.0,
                z_top: 10.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 10.0,
                y: 0.0,
                z_top: 10.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 10.0,
                y: 10.0,
                z_top: 10.0,
                z_bottom: 0.0,
            },
        ];
        obs.rebuild_geometry();
        // Viewer on a high ledge: the stump can't block horizontal sight.
        // viewer_z = 200 → z_top(10) < viewer_z(200) → not useful.
        assert!(!is_obstacle_useful(&obs, 200.0));
    }

    #[test]
    fn is_obstacle_useful_rejects_skyscraper_below_viewer_feet() {
        use crate::sight_obstacle::{ObstaclePoint, SightObstacle};
        // An elevated slab well above the viewer's head.
        let mut obs = SightObstacle::new_default(0);
        obs.obstacle_points = vec![
            ObstaclePoint {
                x: 0.0,
                y: 0.0,
                z_top: 400.0,
                z_bottom: 300.0,
            },
            ObstaclePoint {
                x: 10.0,
                y: 0.0,
                z_top: 400.0,
                z_bottom: 300.0,
            },
            ObstaclePoint {
                x: 10.0,
                y: 10.0,
                z_top: 400.0,
                z_bottom: 300.0,
            },
        ];
        obs.rebuild_geometry();
        // Viewer at 0: every vertex has z_bottom=300 > CHARACTER_HEIGHT(40)
        // → not useful (floats entirely above viewer's head).
        assert!(!is_obstacle_useful(&obs, 0.0));
        // Viewer at 260: 260+40=300 >= z_bottom(300), so straddles → useful.
        assert!(is_obstacle_useful(&obs, 260.0));
    }

    #[test]
    fn visibility_skips_obstacles_above_viewer_head() {
        // Regression test: a high elevated obstacle shouldn't clip
        // the eye-level visibility polygon.
        use crate::sight_obstacle::{ObstaclePoint, SightObstacle};
        let viewer = GeoPoint2D { x: 0.0, y: 0.0 };
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

        let cone = compute_view_cone(viewer, &params);
        let vis = compute_visibility_polygon(viewer, &params, &[&sky_slab]);
        // Without height filtering the slab would still cast a shadow
        // wedge because it sits inside the cone's ground bbox. The
        // usefulness predicate must reject it so the visibility polygon
        // equals the unclipped cone.
        assert_eq!(vis.len(), 1);
        assert_eq!(cone.len(), vis[0].len());
    }

    #[test]
    fn altitude_aware_shadow_cap_shrinks_for_elevated_viewer() {
        // Viewer on a tall ledge looking down past a short wall: the
        // ground shadow should end well before the 2D far-cap (radius*8)
        // at roughly `horizontal_dist * viewer_z / (viewer_z - z_top)`.
        use crate::sight_obstacle::{ObstaclePoint, SightObstacle};
        let viewer = GeoPoint2D { x: 0.0, y: 0.0 };
        let viewer_z = 200.0;
        let z_top = 20.0;

        let mut wall = SightObstacle::new_default(0);
        // A short wall straddling the viewer's forward direction.
        wall.obstacle_points = vec![
            ObstaclePoint {
                x: 100.0,
                y: -10.0,
                z_top,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 110.0,
                y: -10.0,
                z_top,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 110.0,
                y: 10.0,
                z_top,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 100.0,
                y: 10.0,
                z_top,
                z_bottom: 0.0,
            },
        ];
        wall.rebuild_geometry();

        let far = 800.0;
        let shadow_ground = compute_shadow_polygon(viewer, 0.0, &wall, far).unwrap();
        let shadow_raised = compute_shadow_polygon(viewer, viewer_z, &wall, far).unwrap();

        // Ground-viewer shadow should touch the far-cap envelope
        // (max point distance ≈ far).
        let max_ground = shadow_ground
            .exterior()
            .coords()
            .map(|c| (c.x * c.x + c.y * c.y).sqrt())
            .fold(0.0_f32, f32::max);
        assert!(
            max_ground > far * 0.8,
            "ground-level viewer wedge should extend near far-cap, got {}",
            max_ground
        );

        // Raised-viewer shadow should stop well short of the far-cap.
        // Expected ground extent ≈ 100 * 200 / (200 - 20) ≈ 111 — plenty
        // below `far = 800`.
        let max_raised = shadow_raised
            .exterior()
            .coords()
            .map(|c| (c.x * c.x + c.y * c.y).sqrt())
            .fold(0.0_f32, f32::max);
        assert!(
            max_raised < 200.0,
            "raised-viewer wedge should stop near the obstacle, got {}",
            max_raised
        );
    }

    #[test]
    fn visibility_skips_obstacles_outside_cone_flanks() {
        // Regression test for is_box_inside_field rejection: an
        // obstacle to the side of the view cone shouldn't contribute
        // a shadow wedge.
        use crate::sight_obstacle::{ObstaclePoint, SightObstacle};
        let viewer = GeoPoint2D { x: 0.0, y: 0.0 };
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

        let cone = compute_view_cone(viewer, &params);
        let vis = compute_visibility_polygon(viewer, &params, &[&behind]);
        assert_eq!(vis.len(), 1);
        assert_eq!(cone.len(), vis[0].len());
    }
}
