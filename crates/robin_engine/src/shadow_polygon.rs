//! Renderer-independent view geometry, state, and constants.
//!
//! This module owns the Original-compatible 3D obstacle projection so both
//! the host-side overlay rasteriser and simulation-side fog can consume the
//! same visibility regions.

use geo::{
    Area, BooleanOps, ConvexHull, Coord, MultiPoint, MultiPolygon, Point, Polygon,
    algorithm::unary_union,
    orient::{Direction, Orient},
};
use serde::{Deserialize, Serialize};

use crate::position_interface::PlaneZCoeffs;
use crate::sight_obstacle::{ObstaclePoint, SightObstacle};

// ── Constants ─────────────────────────────────────────────────────
/// Re-export of [`crate::position_interface::ASPECT_RATIO`].
pub use crate::position_interface::ASPECT_RATIO;
pub const RADIUS_DAY: f32 = 400.0;
pub const RADIUS_NIGHT: f32 = 300.0;
pub const ALPHA_DAY: u8 = 192;
pub const ALPHA_NIGHT: u8 = 120;
pub const NORMAL_HALF_APERTURE: f32 = 0.5;
/// Eye-level offset used by the obstacle-usefulness filter.
/// This is the offset from a character's feet to the eye plane — not the
/// full stature — and is used to decide which obstacles can contribute to
/// the visibility polygon given the viewer's Z.
pub const CHARACTER_HEIGHT: f32 = 40.0;

/// A surface on which visibility is evaluated.
///
/// `height` is measured vertically above `plane` (or above world Z=0 when
/// `plane` is `None`).  Passing both height 0 and [`CHARACTER_HEIGHT`] matches
/// the Original's feet/head shadow-polygon passes.  The type deliberately has
/// no renderer concepts so the same solver can feed view overlays, fog masks,
/// or gameplay diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VisibilitySurface {
    pub plane: Option<PlaneZCoeffs>,
    pub height: f32,
}

impl VisibilitySurface {
    #[inline]
    fn z(self, x: f32, y: f32) -> f32 {
        self.plane
            .map(|plane| plane.compute_world_z(x, y))
            .unwrap_or(0.0)
            + self.height
    }
}

#[derive(Debug, Clone, Copy)]
struct WorldVertex {
    x: f32,
    y: f32,
    z: f32,
}

/// Compute the part of `domain` visible from `viewer` on any of `surfaces`.
///
/// This is the geometric equivalent of the original game's feet-shadow polygon
/// and head iterator counters: each obstacle volume is centrally projected
/// onto every requested surface, shadows are unioned per surface, and a point
/// is hidden only if it is shadowed on *all* surfaces.  Consequently two
/// surfaces at offsets 0 and [`CHARACTER_HEIGHT`] consider a standing target
/// visible when either its feet or its head can be seen.
///
/// `far_distance` must enclose `domain` around the viewer.  It replaces the
/// Original's projection-area screen box when a shadow reaches the projective
/// horizon; callers normally use several times their view radius.
pub fn visible_region_on_surfaces(
    domain: &Polygon<f32>,
    viewer: [f32; 3],
    surfaces: &[VisibilitySurface],
    obstacles: &[&SightObstacle],
    far_distance: f32,
) -> MultiPolygon<f32> {
    if surfaces.is_empty() {
        panic!("visibility projection requires at least one target surface");
    }
    if !far_distance.is_finite() || far_distance <= 0.0 {
        panic!("visibility projection far distance must be finite and positive");
    }

    let mut hidden_on_every_surface: Option<MultiPolygon<f32>> = None;
    for &surface in surfaces {
        let mut shadows = Vec::new();
        for obstacle in obstacles
            .iter()
            .copied()
            .filter(|obstacle| obstacle.is_opaque())
        {
            shadows.extend(project_obstacle_shadow(
                viewer,
                surface,
                obstacle,
                far_distance,
            ));
        }

        let surface_shadow = if shadows.is_empty() {
            MultiPolygon::new(Vec::new())
        } else {
            unary_union(&shadows)
        };
        hidden_on_every_surface = Some(match hidden_on_every_surface {
            None => surface_shadow,
            Some(hidden) => hidden.intersection(&surface_shadow),
        });
    }

    let domain = MultiPolygon::new(vec![domain.orient(Direction::Default)]);
    match hidden_on_every_surface {
        Some(hidden) if !hidden.0.is_empty() => domain.difference(&hidden),
        _ => domain,
    }
}

fn project_obstacle_shadow(
    viewer: [f32; 3],
    surface: VisibilitySurface,
    obstacle: &SightObstacle,
    far_distance: f32,
) -> Vec<Polygon<f32>> {
    let points = &obstacle.obstacle_points;
    if points.len() < 3 {
        tracing::warn!(
            obstacle_id = obstacle.id,
            points = points.len(),
            "opaque sight obstacle has too few points for visibility projection"
        );
        return Vec::new();
    }

    let viewer_height = viewer[2] - surface.z(viewer[0], viewer[1]);
    if viewer_height.abs() < 0.001 {
        return project_eye_plane_shadow(viewer, surface, points, far_distance);
    }

    let mut faces: Vec<Vec<WorldVertex>> = Vec::with_capacity(points.len() + 2);
    faces.push(
        points
            .iter()
            .map(|p| WorldVertex {
                x: p.x,
                y: p.y,
                z: p.z_top,
            })
            .collect(),
    );
    faces.push(
        points
            .iter()
            .rev()
            .map(|p| WorldVertex {
                x: p.x,
                y: p.y,
                z: p.z_bottom,
            })
            .collect(),
    );
    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        faces.push(vec![
            WorldVertex {
                x: a.x,
                y: a.y,
                z: a.z_bottom,
            },
            WorldVertex {
                x: b.x,
                y: b.y,
                z: b.z_bottom,
            },
            WorldVertex {
                x: b.x,
                y: b.y,
                z: b.z_top,
            },
            WorldVertex {
                x: a.x,
                y: a.y,
                z: a.z_top,
            },
        ]);
    }

    let projected_points: Vec<Point<f32>> = faces
        .into_iter()
        .flat_map(|face| {
            let face = clip_face(
                &face,
                |point| (point.z - surface.z(point.x, point.y)) / viewer_height,
                0.0,
                true,
            );
            let face = clip_face(
                &face,
                |point| (point.z - surface.z(point.x, point.y)) / viewer_height,
                1.0,
                false,
            );
            face.into_iter().map(|point| {
                Point::from(project_vertex(
                    point,
                    viewer,
                    surface,
                    viewer_height,
                    far_distance,
                ))
            })
        })
        .collect();
    if projected_points.len() < 3 {
        return Vec::new();
    }
    let shadow = MultiPoint::new(projected_points).convex_hull();
    if shadow.unsigned_area() > 0.001 {
        vec![shadow]
    } else {
        Vec::new()
    }
}

fn clip_face(
    input: &[WorldVertex],
    value: impl Fn(WorldVertex) -> f32,
    threshold: f32,
    keep_above: bool,
) -> Vec<WorldVertex> {
    let Some(mut previous) = input.last().copied() else {
        return Vec::new();
    };
    let mut previous_value = value(previous);
    let mut previous_inside = if keep_above {
        previous_value >= threshold
    } else {
        previous_value <= threshold
    };
    let mut output = Vec::new();

    for &current in input {
        let current_value = value(current);
        let current_inside = if keep_above {
            current_value >= threshold
        } else {
            current_value <= threshold
        };
        if current_inside != previous_inside {
            let denominator = current_value - previous_value;
            if denominator.abs() > f32::EPSILON {
                let t = (threshold - previous_value) / denominator;
                output.push(WorldVertex {
                    x: previous.x + (current.x - previous.x) * t,
                    y: previous.y + (current.y - previous.y) * t,
                    z: previous.z + (current.z - previous.z) * t,
                });
            }
        }
        if current_inside {
            output.push(current);
        }
        previous = current;
        previous_value = current_value;
        previous_inside = current_inside;
    }
    output
}

fn project_vertex(
    point: WorldVertex,
    viewer: [f32; 3],
    surface: VisibilitySurface,
    viewer_height: f32,
    far_distance: f32,
) -> Coord<f32> {
    let relative_height = point.z - surface.z(point.x, point.y);
    let fraction = relative_height / viewer_height;
    let dx = point.x - viewer[0];
    let dy = point.y - viewer[1];
    let distance = dx.hypot(dy);
    let horizon_scale = if distance > 0.001 {
        far_distance / distance
    } else {
        1.0
    };
    let scale = if 1.0 - fraction <= 0.000_01 {
        horizon_scale
    } else {
        (1.0 / (1.0 - fraction)).min(horizon_scale)
    };
    Coord {
        x: viewer[0] + dx * scale,
        y: viewer[1] + dy * scale,
    }
}

#[derive(Debug, Clone, Copy)]
struct EyePlaneVertex {
    x: f32,
    y: f32,
    top: f32,
    bottom: f32,
}

fn project_eye_plane_shadow(
    viewer: [f32; 3],
    surface: VisibilitySurface,
    points: &[ObstaclePoint],
    far_distance: f32,
) -> Vec<Polygon<f32>> {
    let cross_section: Vec<EyePlaneVertex> = points
        .iter()
        .map(|point| EyePlaneVertex {
            x: point.x,
            y: point.y,
            top: point.z_top - surface.z(point.x, point.y),
            bottom: point.z_bottom - surface.z(point.x, point.y),
        })
        .collect();
    let cross_section = clip_eye_plane(&cross_section, |point| point.top, 0.0, true);
    let cross_section = clip_eye_plane(&cross_section, |point| point.bottom, 0.0, false);
    if cross_section.len() < 3 {
        return Vec::new();
    }

    // For a convex cross-section, its radial extrusion away from the viewer
    // is convex as well. Project all silhouette candidates to the horizon and
    // take one hull; within `far_distance` this is the same unbounded shadow
    // the Original closes against its projection-area screen box.
    let projected_points: Vec<Point<f32>> = cross_section
        .iter()
        .flat_map(|&point| {
            let dx = point.x - viewer[0];
            let dy = point.y - viewer[1];
            let distance = dx.hypot(dy);
            let scale = if distance > 0.001 {
                far_distance / distance
            } else {
                1.0
            };
            let far = Coord {
                x: viewer[0] + dx * scale,
                y: viewer[1] + dy * scale,
            };
            [Point::new(point.x, point.y), Point::from(far)]
        })
        .collect();
    let shadow = MultiPoint::new(projected_points).convex_hull();
    if shadow.unsigned_area() > 0.001 {
        vec![shadow]
    } else {
        Vec::new()
    }
}

fn clip_eye_plane(
    input: &[EyePlaneVertex],
    value: impl Fn(EyePlaneVertex) -> f32,
    threshold: f32,
    keep_above: bool,
) -> Vec<EyePlaneVertex> {
    let Some(mut previous) = input.last().copied() else {
        return Vec::new();
    };
    let mut previous_value = value(previous);
    let mut previous_inside = if keep_above {
        previous_value >= threshold
    } else {
        previous_value <= threshold
    };
    let mut output = Vec::new();
    for &current in input {
        let current_value = value(current);
        let current_inside = if keep_above {
            current_value >= threshold
        } else {
            current_value <= threshold
        };
        if current_inside != previous_inside {
            let denominator = current_value - previous_value;
            if denominator.abs() > f32::EPSILON {
                let t = (threshold - previous_value) / denominator;
                output.push(EyePlaneVertex {
                    x: previous.x + (current.x - previous.x) * t,
                    y: previous.y + (current.y - previous.y) * t,
                    top: previous.top + (current.top - previous.top) * t,
                    bottom: previous.bottom + (current.bottom - previous.bottom) * t,
                });
            }
        }
        if current_inside {
            output.push(current);
        }
        previous = current;
        previous_value = current_value;
        previous_inside = current_inside;
    }
    output
}

// ── ViewParameters ────────────────────────────────────────────────
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ViewParameters {
    pub direction: [f32; 2],
    pub half_aperture: f32,
    pub radius: f32,
    pub alpha: u8,
    pub lean_out: bool,
    /// Viewer eye altitude. Used by the obstacle-usefulness filter in
    /// `compute_visibility_polygon`.
    pub viewer_z: f32,
    /// Projection plane for debug rendering. The original
    /// Original-game shadow projection computes rendered polygon
    /// vertices with `screen_y = y - plane_height(x, y)`.
    #[serde(default)]
    pub projection_plane: Option<crate::position_interface::PlaneZCoeffs>,
    /// Current projection-area obstacle used by the display path. The
    /// original game renders one shadow slice per projection area
    /// and clips the slice to that area's projected polygon.
    #[serde(skip)]
    #[bitcode(skip)]
    pub projection_obstacle: Option<crate::position_interface::ObstacleHandle>,
}

impl Default for ViewParameters {
    fn default() -> Self {
        Self {
            direction: [1.0, 0.0],
            half_aperture: NORMAL_HALF_APERTURE,
            radius: RADIUS_DAY,
            alpha: ALPHA_DAY,
            lean_out: false,
            viewer_z: 0.0,
            projection_plane: None,
            projection_obstacle: None,
        }
    }
}

/// Convert a 16-sector cardinal direction index to a unit (x, y) vector.
/// Sector 0 = north = -Y; sectors increase clockwise.
#[allow(clippy::approx_constant, clippy::excessive_precision)]
pub fn sector_to_direction(sector: i16) -> [f32; 2] {
    // Keep the game's literal vector table. Re-evaluating sin/cos
    // produces values a few ULPs away from the original-game constants; patrol
    // formation multiplies these offsets by 20, and the resulting error can
    // flip the exact dot-product test for reaching the goal.
    const SIN_PI_EIGHTH: f32 = 0.382_683_43;
    const COS_PI_EIGHTH: f32 = 0.923_879_5;
    const HALF_SQRT_TWO: f32 = 0.707_106_77;
    const X: [f32; 16] = [
        0.0,
        SIN_PI_EIGHTH,
        HALF_SQRT_TWO,
        COS_PI_EIGHTH,
        1.0,
        COS_PI_EIGHTH,
        HALF_SQRT_TWO,
        SIN_PI_EIGHTH,
        0.0,
        -SIN_PI_EIGHTH,
        -HALF_SQRT_TWO,
        -COS_PI_EIGHTH,
        -1.0,
        -COS_PI_EIGHTH,
        -HALF_SQRT_TWO,
        -SIN_PI_EIGHTH,
    ];
    const Y: [f32; 16] = [
        -1.0,
        -COS_PI_EIGHTH,
        -HALF_SQRT_TWO,
        -SIN_PI_EIGHTH,
        0.0,
        SIN_PI_EIGHTH,
        HALF_SQRT_TWO,
        COS_PI_EIGHTH,
        1.0,
        COS_PI_EIGHTH,
        HALF_SQRT_TWO,
        SIN_PI_EIGHTH,
        0.0,
        -SIN_PI_EIGHTH,
        -HALF_SQRT_TWO,
        -COS_PI_EIGHTH,
    ];
    let index = sector.rem_euclid(16) as usize;
    [X[index], Y[index]]
}

#[cfg(test)]
mod tests {
    use geo::{Contains, point, polygon};

    use super::{
        CHARACTER_HEIGHT, VisibilitySurface, project_obstacle_shadow, sector_to_direction,
        visible_region_on_surfaces,
    };
    use crate::sight_obstacle::{ObstaclePoint, SightObstacle};

    fn wall(z_bottom: f32, z_top: f32) -> SightObstacle {
        let mut obstacle = SightObstacle::new_default(1);
        obstacle.obstacle_points = [(100.0, -10.0), (110.0, -10.0), (110.0, 10.0), (100.0, 10.0)]
            .into_iter()
            .map(|(x, y)| ObstaclePoint {
                x,
                y,
                z_top,
                z_bottom,
            })
            .collect();
        obstacle.rebuild_geometry();
        obstacle
    }

    fn domain() -> geo::Polygon<f32> {
        polygon![
            (x: 0.0, y: -100.0),
            (x: 300.0, y: -100.0),
            (x: 300.0, y: 100.0),
            (x: 0.0, y: 100.0),
        ]
    }

    fn standing_surfaces() -> [VisibilitySurface; 2] {
        [
            VisibilitySurface {
                plane: None,
                height: 0.0,
            },
            VisibilitySurface {
                plane: None,
                height: CHARACTER_HEIGHT,
            },
        ]
    }

    #[test]
    fn sector_directions_match_original_literal_table_bits() {
        let expected_x = [
            0x0000_0000,
            0x3ec3_ef15,
            0x3f35_04f3,
            0x3f6c_835e,
            0x3f80_0000,
            0x3f6c_835e,
            0x3f35_04f3,
            0x3ec3_ef15,
            0x0000_0000,
            0xbec3_ef15,
            0xbf35_04f3,
            0xbf6c_835e,
            0xbf80_0000,
            0xbf6c_835e,
            0xbf35_04f3,
            0xbec3_ef15,
        ];
        let expected_y = [
            0xbf80_0000,
            0xbf6c_835e,
            0xbf35_04f3,
            0xbec3_ef15,
            0x0000_0000,
            0x3ec3_ef15,
            0x3f35_04f3,
            0x3f6c_835e,
            0x3f80_0000,
            0x3f6c_835e,
            0x3f35_04f3,
            0x3ec3_ef15,
            0x0000_0000,
            0xbec3_ef15,
            0xbf35_04f3,
            0xbf6c_835e,
        ];

        for sector in 0..16 {
            let [x, y] = sector_to_direction(sector);
            assert_eq!(x.to_bits(), expected_x[sector as usize]);
            assert_eq!(y.to_bits(), expected_y[sector as usize]);
        }
        assert_eq!(sector_to_direction(-1), sector_to_direction(15));
        assert_eq!(sector_to_direction(16), sector_to_direction(0));
    }

    #[test]
    fn full_height_wall_hides_feet_and_head() {
        let wall = wall(0.0, 60.0);
        for surface in standing_surfaces() {
            let shadows =
                project_obstacle_shadow([0.0, 0.0, CHARACTER_HEIGHT], surface, &wall, 1_000.0);
            assert!(
                shadows
                    .iter()
                    .any(|shadow| shadow.contains(&point!(x: 200.0, y: 0.0))),
                "surface at height {} produced no containing primitive among {} shadows",
                surface.height,
                shadows.len()
            );
            let one_surface = visible_region_on_surfaces(
                &domain(),
                [0.0, 0.0, CHARACTER_HEIGHT],
                &[surface],
                &[&wall],
                1_000.0,
            );
            assert!(
                !one_surface.contains(&point!(x: 200.0, y: 0.0)),
                "surface at height {} did not project the wall shadow",
                surface.height
            );
        }
        let visible = visible_region_on_surfaces(
            &domain(),
            [0.0, 0.0, CHARACTER_HEIGHT],
            &standing_surfaces(),
            &[&wall],
            1_000.0,
        );

        assert!(!visible.contains(&point!(x: 200.0, y: 0.0)));
        assert!(visible.contains(&point!(x: 200.0, y: 50.0)));
    }

    #[test]
    fn low_wall_leaves_head_visible() {
        let wall = wall(0.0, 20.0);
        let visible = visible_region_on_surfaces(
            &domain(),
            [0.0, 0.0, CHARACTER_HEIGHT],
            &standing_surfaces(),
            &[&wall],
            1_000.0,
        );

        assert!(visible.contains(&point!(x: 200.0, y: 0.0)));
    }

    #[test]
    fn elevated_viewer_gets_finite_low_wall_shadow() {
        let wall = wall(0.0, 20.0);
        let visible = visible_region_on_surfaces(
            &domain(),
            [0.0, 0.0, 200.0],
            &standing_surfaces(),
            &[&wall],
            1_000.0,
        );

        assert!(visible.contains(&point!(x: 150.0, y: 0.0)));
    }

    #[test]
    fn sloped_projection_surface_uses_world_plane_heights() {
        let plane = crate::position_interface::PlaneZCoeffs {
            az: 0.2,
            bz: 0.0,
            dz: 0.0,
        };
        let mut wall = wall(0.0, 0.0);
        for point in &mut wall.obstacle_points {
            let surface_z = plane.compute_world_z(point.x, point.y);
            point.z_bottom = surface_z;
            point.z_top = surface_z + 60.0;
        }
        wall.rebuild_geometry();
        let surfaces = [
            VisibilitySurface {
                plane: Some(plane),
                height: 0.0,
            },
            VisibilitySurface {
                plane: Some(plane),
                height: CHARACTER_HEIGHT,
            },
        ];
        let visible = visible_region_on_surfaces(
            &domain(),
            [0.0, 0.0, CHARACTER_HEIGHT],
            &surfaces,
            &[&wall],
            1_000.0,
        );

        assert!(!visible.contains(&point!(x: 200.0, y: 0.0)));
    }
}
