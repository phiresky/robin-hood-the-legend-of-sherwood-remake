//! Landing geometry comes from the current motion area and its actual receiver.

use super::*;
use geo::{BooleanOps, BoundingRect, MapCoords};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct BoundLanding {
    pub boundary: Vec<[f32; 2]>,
    pub holes: Vec<Vec<[f32; 2]>>,
    pub layer: usize,
    pub area: usize,
    pub sector: u16,
    pub plane: [f64; 3],
    pub obstacles: Vec<BoundLandingObstacle>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct BoundLandingObstacle {
    pub motion_obstacle: u16,
    pub state: u32,
    pub polygon: Vec<[f32; 2]>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seam_roundoff_cleanup_preserves_real_and_standalone_thin_obstacles() {
        let stair = polygon(&[
            [2100., 1800.],
            [2140., 1800.],
            [2140., 1840.],
            [2100., 1840.],
        ])
        .unwrap();
        let solid = polygon(&[
            [2090., 1800.],
            [2150., 1800.],
            [2150., 1830.],
            [2090., 1830.],
        ])
        .unwrap();
        let strip = |width: f32| {
            polygon(&[
                [2090., 1800.],
                [2150., 1800.],
                [2150., 1800. - width],
                [2090., 1800. - width],
            ])
            .unwrap()
        };
        let noise = strip(0.000244140625);
        assert!(rounded_seam_sliver(&noise, &solid, &stair, 0.0005));
        assert!(!rounded_seam_sliver(&strip(0.01), &solid, &stair, 0.0005));
        assert!(!rounded_seam_sliver(&noise, &noise, &stair, 0.0005));
        let landing_wall = polygon(&[
            [2090., 1800.],
            [2150., 1800.],
            [2150., 1790.],
            [2090., 1790.],
        ])
        .unwrap();
        assert!(!rounded_seam_sliver(&noise, &landing_wall, &stair, 0.0005));
        let unrelated = polygon(&[
            [2100., 1700.],
            [2140., 1700.],
            [2140., 1740.],
            [2100., 1740.],
        ])
        .unwrap();
        assert!(!rounded_seam_sliver(&noise, &solid, &unrelated, 0.0005));
    }

    #[test]
    fn receiver_can_cover_part_of_a_joined_motion_region() {
        let motion = serde_json::from_value(serde_json::json!({
            "is_lift":false, "state_id":0, "flags":0, "skeleton_segments":[], "obstacles":[],
            "polygon":{"points":[[0,0],[100,0],[100,100],[50,100],[50,120],[0,120]]}
        }))
        .unwrap();
        let receiver = polygon(&[[0., 0.], [100., 0.], [100., 100.25], [0., 100.25]]).unwrap();
        assert!(receiver_matches_motion(&receiver, &motion, [0.; 3]));
        // Being assigned to the same region does not authorize an overhang
        // whose rounded footprint extends beyond that region.
        let overhang = polygon(&[[0., 0.], [100., 0.], [100., 101.], [0., 101.]]).unwrap();
        assert!(!receiver_matches_motion(&overhang, &motion, [0.; 3]));
    }

    #[test]
    fn receiver_identity_ignores_rounded_vertices_inserted_on_straight_edges() {
        let motion = serde_json::from_value(serde_json::json!({
            "is_lift":false, "state_id":0, "flags":0, "skeleton_segments":[], "obstacles":[],
            "polygon":{"points":[[0,0],[100,0],[100,100],[0,10]]}
        }))
        .unwrap();
        let receiver = polygon(&[
            [0., 0.],
            [100., 0.],
            [100., 100.25],
            [50.2, 55.43],
            [0., 10.25],
        ])
        .unwrap();
        assert!(receiver_matches_motion(&receiver, &motion, [0.; 3]));
        let unrelated = polygon(&[
            [0., 0.],
            [100., 0.],
            [100., 100.25],
            [50.2, 56.43],
            [0., 10.25],
        ])
        .unwrap();
        assert!(!receiver_matches_motion(&unrelated, &motion, [0.; 3]));
    }

    #[test]
    fn landing_binding_preserves_matching_pre_grid_receiver() {
        let mut stair = BoundPhysicalStair {
            definition: robin_level_data::physical_stair::PhysicalStairNavigation {
                plane: [0., 1., -200.25],
                boundary: vec![
                    [380., 300.25],
                    [420., 300.25],
                    [420., 400.25],
                    [380., 400.25],
                ],
                obstacles: vec![],
                doors: vec![robin_level_data::physical_stair::PhysicalStairDoor {
                    inside: [400., 310.25, 110.],
                    middle: [400., 300.25, 100.],
                    outside: [400., 290., 100.],
                }],
            },
            layer: 2,
            area: 0,
            obstacle_states: vec![],
            landings: vec![],
        };
        let motion = serde_json::from_value(serde_json::json!({
            "is_lift":false, "state_id":0, "flags":0, "skeleton_segments":[], "obstacles":[],
            "polygon":{"points":[[380,170],[420,170],[420,200],[380,200]]}
        }))
        .unwrap();
        for end in [299.75, 301.] {
            let receiver =
                polygon(&[[380., 270.], [420., 270.], [420., end], [380., end]]).unwrap();
            assert!(
                stair
                    .bind_landing(0, &motion, 0, 0, 0, [0., 0., 100.], Some(&receiver))
                    .is_err(),
                "a short or unrelated receiver must not bridge a missing landing"
            );
        }
        assert!(stair.landings.is_empty());
        let receiver =
            polygon(&[[380., 270.], [420., 270.], [420., 300.25], [380., 300.25]]).unwrap();
        stair
            .bind_landing(0, &motion, 0, 0, 0, [0., 0., 100.], Some(&receiver))
            .unwrap();
        assert!(stair.supports_landing_neighbour(0, 0, [400., 300.125, 100.]));
        assert!(!stair.supports_landing_neighbour(0, 0, [400., 300.5, 100.]));
        let mut with_hole = motion.clone();
        with_hole.obstacles.push(
            serde_json::from_value(serde_json::json!({
                "state_id": 2,
                "polygon": {"points": [[390,200],[410,200],[410,210],[390,210]]}
            }))
            .unwrap(),
        );
        let mut bound = stair.clone();
        bound.landings.clear();
        bound
            .bind_landing(0, &with_hole, 0, 0, 0, [0., 0., 100.], Some(&receiver))
            .unwrap();
        assert_eq!(
            bound.landings[0].obstacles.len(),
            1,
            "rounded hole overlaps the seam"
        );
        with_hole.obstacles[0].precise_polygon = vec![
            [390., 200.25],
            [410., 200.25],
            [410., 210.25],
            [390., 210.25],
        ];
        bound.landings.clear();
        bound
            .bind_landing(0, &with_hole, 0, 0, 0, [0., 0., 100.], Some(&receiver))
            .unwrap();
        assert!(
            bound.landings[0].obstacles.is_empty(),
            "exact hole starts beyond the seam"
        );
        // Real landing collision keeps its index and live state after clipping.
        for point in &mut with_hole.obstacles[0].polygon.points {
            point.1 -= 10;
        }
        for point in &mut with_hole.obstacles[0].precise_polygon {
            point[1] -= 10.;
        }
        bound.landings.clear();
        bound
            .bind_landing(0, &with_hole, 0, 0, 0, [0., 0., 100.], Some(&receiver))
            .unwrap();
        assert_eq!(bound.landings[0].obstacles.len(), 1);
        assert_eq!(bound.landings[0].obstacles[0].motion_obstacle, 0);
        assert_eq!(bound.landings[0].obstacles[0].state, 2);
        for degrees in [37.0_f64, 90., 180., 270.] {
            let (sin, cos) = degrees.to_radians().sin_cos();
            let transform = |[x, y]: [f32; 2]| {
                [
                    (2000. + cos * f64::from(x) - sin * f64::from(y)) as f32,
                    (1700. + sin * f64::from(x) + cos * f64::from(y)) as f32,
                ]
            };
            for gap in [0.0, 0.01] {
                let mut placed = stair.clone();
                placed.landings.clear();
                placed.definition.plane = [-sin, cos, -200.25 + sin * 2000. - cos * 1700.];
                placed.definition.boundary = stair
                    .definition
                    .boundary
                    .iter()
                    .copied()
                    .map(transform)
                    .collect();
                for door in &mut placed.definition.doors {
                    for point in [&mut door.inside, &mut door.middle, &mut door.outside] {
                        let [x, y] = transform([point[0], point[1]]);
                        point[0] = x;
                        point[1] = y;
                    }
                }
                let ring = [
                    [370., 270.],
                    [430., 270.],
                    [430., 300.25 - gap],
                    [370., 300.25 - gap],
                ]
                .map(transform);
                let mut placed_motion = motion.clone();
                placed_motion.polygon.points = ring
                    .iter()
                    .map(|p| {
                        (
                            (p[0] + 0.5).floor() as i16,
                            (p[1] - 100. + 0.5).floor() as i16,
                        )
                    })
                    .collect();
                let result = placed.bind_landing(
                    0,
                    &placed_motion,
                    0,
                    0,
                    0,
                    [0., 0., 100.],
                    Some(&polygon(&ring).unwrap()),
                );
                assert_eq!(
                    result.is_ok(),
                    gap == 0.,
                    "rotation {degrees}, gap {gap}: {result:?}"
                );
                if gap == 0. {
                    let door = &placed.definition.doors[0];
                    let geometry = StairRouteGeometry {
                        boundary: placed.definition.boundary.clone(),
                        obstacles: vec![],
                    };
                    assert!(
                        geometry
                            .route_with_landing_support(
                                [door.middle[0], door.middle[1]],
                                [door.inside[0], door.inside[1]],
                                MoveBoxHalfDiagonal::new(6., 3.),
                                &[placed.landings[0].boundary.clone()]
                            )
                            .unwrap()
                            .is_some(),
                        "rotation {degrees}: bound landing must support entry"
                    );
                }
            }
        }
    }

    #[test]
    fn landing_binding_rejects_wrong_heights_and_incomplete_receivers() {
        let mut stair = BoundPhysicalStair {
            definition: robin_level_data::physical_stair::PhysicalStairNavigation {
                plane: [0., 1., -200.],
                boundary: vec![[380., 300.], [420., 300.], [420., 400.], [380., 400.]],
                obstacles: vec![],
                doors: vec![robin_level_data::physical_stair::PhysicalStairDoor {
                    inside: [400., 310., 110.],
                    middle: [400., 300., 100.],
                    outside: [400., 290., 100.],
                }],
            },
            layer: 2,
            area: 0,
            obstacle_states: vec![],
            landings: vec![],
        };
        let motion = serde_json::from_value(serde_json::json!({
            "is_lift":false, "state_id":0, "flags":0, "skeleton_segments":[], "obstacles":[],
            "polygon":{"points":[[380,170],[420,170],[420,200],[380,200]]}
        }))
        .unwrap();
        assert!(
            stair
                .bind_landing(0, &motion, 0, 0, 0, [0., 0., 101.], None)
                .is_err()
        );
        assert!(
            stair
                .bind_landing(0, &motion, 0, 0, 0, [0.2, 0., 20.], None)
                .is_err()
        );
        let short_receiver =
            polygon(&[[380., 270.], [420., 270.], [420., 299.], [380., 299.]]).unwrap();
        assert!(
            stair
                .bind_landing(0, &motion, 0, 0, 0, [0., 0., 100.], Some(&short_receiver))
                .is_err()
        );
        assert!(
            stair.landings.is_empty(),
            "rejected geometry must not provide foot support"
        );
        let receiver = polygon(&[[380., 270.], [420., 270.], [420., 300.], [380., 300.]]).unwrap();
        stair
            .bind_landing(0, &motion, 0, 0, 0, [0., 0., 100.], Some(&receiver))
            .unwrap();
        assert_eq!(stair.landings.len(), 1);
        assert!(stair.supports_landing_neighbour(0, 0, [400., 299., 100.]));
        assert!(!stair.supports_landing_neighbour(1, 0, [400., 299., 100.]));
        assert!(!stair.supports_landing_neighbour(0, 1, [400., 299., 100.]));
        assert!(!stair.supports_landing_neighbour(0, 0, [400., 299., 200.]));
        assert!(!stair.supports_landing_neighbour(0, 0, [400., 301., 100.]));
        let mut overlapping_motion = motion.clone();
        for point in &mut overlapping_motion.polygon.points {
            if point.1 == 200 {
                point.1 = 201;
            }
        }
        let overlapping_receiver =
            polygon(&[[380., 270.], [420., 270.], [420., 301.], [380., 301.]]).unwrap();
        stair
            .bind_landing(
                0,
                &overlapping_motion,
                0,
                0,
                0,
                [0., 0., 100.],
                Some(&overlapping_receiver),
            )
            .unwrap();
        assert!(
            stair
                .landings
                .last()
                .unwrap()
                .boundary
                .iter()
                .all(|point| point[1] <= 300.)
        );
        assert!(!stair.supports_landing_neighbour(0, 0, [400., 300.5, 100.]));
    }
}

impl BoundPhysicalStair {
    pub(crate) fn has_landing(&self, layer: u16, sector: u16) -> bool {
        self.landings
            .iter()
            .any(|landing| landing.layer == usize::from(layer) && landing.sector == sector)
    }

    /// A newly closed landing obstacle can overlap a stair actor's supported footprint.
    pub(crate) fn landing_obstacle_intersects_actor(
        &self,
        layer: u16,
        sector: u16,
        motion_obstacle: u16,
        position: [f32; 2],
        half: MoveBoxHalfDiagonal,
    ) -> bool {
        let actor = actor_footprint(position, half);
        self.landings
            .iter()
            .filter(|landing| landing.layer == usize::from(layer) && landing.sector == sector)
            .flat_map(|landing| &landing.obstacles)
            .filter(|obstacle| obstacle.motion_obstacle == motion_obstacle)
            .any(|obstacle| {
                polygon(&obstacle.polygon)
                    .expect("bound landing obstacle is invalid")
                    .intersects(&actor)
            })
    }

    /// Only actors on an explicitly bound adjoining floor can disturb this stair.
    pub(crate) fn supports_landing_neighbour(
        &self,
        layer: u16,
        sector: u16,
        position: [f32; 3],
    ) -> bool {
        self.landings.iter().any(|landing| {
            if landing.layer != usize::from(layer) || landing.sector != sector {
                return false;
            }
            let [a, b, c] = landing.plane;
            if (a * f64::from(position[0]) + b * f64::from(position[1]) + c
                - f64::from(position[2]))
            .abs()
                > 0.001
            {
                return false;
            }
            let point = Point::new(position[0], position[1]);
            polygon(&landing.boundary)
                .expect("bound landing boundary is invalid")
                .intersects(&point)
                && !landing.holes.iter().any(|hole| {
                    polygon(hole)
                        .expect("bound landing hole is invalid")
                        .intersects(&point)
                })
        })
    }

    /// Bind only the connected receiving patch at a door. Never extrapolate a
    /// receiver plane over the rest of a multi-height motion area.
    pub(crate) fn bind_landing(
        &mut self,
        door: usize,
        motion: &crate::level_data::RawMotionArea,
        layer: usize,
        area: usize,
        sector: u16,
        plane: [f64; 3],
        receiver: Option<&Polygon<f32>>,
    ) -> Result<(), String> {
        let physical = self
            .definition
            .doors
            .get(door)
            .ok_or("missing physical landing door")?;
        let [a, b, c] = plane;
        if plane.iter().any(|v| !v.is_finite()) || (1.0 - b).abs() < 1e-8 {
            return Err("landing receiver needs nonsingular navigation coordinates".into());
        }
        for [x, y, z] in [physical.middle, physical.outside] {
            if (a * f64::from(x) + b * f64::from(y) + c - f64::from(z)).abs() > 0.001 {
                return Err("landing height does not match the physical door".into());
            }
        }
        let unproject = |points: &[(i16, i16)]| -> Result<Polygon<f32>, String> {
            let ring = points
                .iter()
                .map(|&(x, y)| {
                    [
                        f32::from(x),
                        ((f64::from(y) + a * f64::from(x) + c) / (1.0 - b)) as f32,
                    ]
                })
                .collect::<Vec<_>>();
            polygon(&ring)
        };
        let floor = unproject(&motion.polygon.points)?;
        let mut support = if let Some(receiver) = receiver {
            // Exact receiving geometry may encode all or part of a motion
            // region before integer-grid rounding. Joined regions can include
            // other receivers at different heights. Preserve this receiver only
            // when its rounded footprint stays inside the assigned region.
            if receiver_matches_motion(receiver, motion, plane) {
                geo::MultiPolygon::from(vec![receiver.clone()])
            } else {
                floor.intersection(receiver)
            }
        } else {
            geo::MultiPolygon::from(vec![floor])
        };
        let [sa, sb, sc] = self.definition.plane;
        let difference = |x: f64, y: f64| (sa - a) * x + (sb - b) * y + sc - c;
        let outside_side = difference(
            f64::from(physical.outside[0]),
            f64::from(physical.outside[1]),
        );
        if outside_side.abs() > 0.001 {
            // Only the side of the equal-height seam containing this landing
            // can support a stair footprint. Rounded corners on the opposite
            // side must not create support beside a different stair height.
            let bounds = support.bounding_rect().ok_or("empty landing support")?;
            let corners = [
                [bounds.min().x, bounds.min().y],
                [bounds.max().x, bounds.min().y],
                [bounds.max().x, bounds.max().y],
                [bounds.min().x, bounds.max().y],
            ];
            let mut mask = Vec::new();
            for (index, from) in corners.iter().enumerate() {
                let to = corners[(index + 1) % corners.len()];
                let d0 = difference(f64::from(from[0]), f64::from(from[1])) * outside_side.signum();
                let d1 = difference(f64::from(to[0]), f64::from(to[1])) * outside_side.signum();
                if d0 >= 0. {
                    mask.push(*from);
                }
                if (d0 >= 0.) != (d1 >= 0.) {
                    let t = d0 / (d0 - d1);
                    mask.push([
                        (f64::from(from[0]) + t * (f64::from(to[0]) - f64::from(from[0]))) as f32,
                        (f64::from(from[1]) + t * (f64::from(to[1]) - f64::from(from[1]))) as f32,
                    ]);
                }
            }
            support = support.intersection(&polygon(&mask)?);
        }
        let stair = polygon(&self.definition.boundary)?;
        // Quantized landing contours can extend slightly into a physical stair.
        // Remove that overlap before binding; never extend a floor across a gap.
        let support = support.difference(&stair);
        let tolerance = physical.middle[..2]
            .iter()
            .fold(1.0_f64, |scale, value| scale.max(f64::from(value.abs())))
            * f64::from(f32::EPSILON)
            * 2.0;
        let Some(support) = support.iter().find(|patch| {
            [physical.middle, physical.outside].iter().all(|point| {
                patch.intersects(&Point::new(point[0], point[1]))
                    || patch.exterior().lines().any(|edge| {
                        point_edge_distance(
                            [f64::from(point[0]), f64::from(point[1])],
                            edge.map_coords(|c| geo::Coord {
                                x: f64::from(c.x),
                                y: f64::from(c.y),
                            }),
                        ) <= tolerance
                    })
            })
        }) else {
            return Err("landing receiver does not reach its physical door".into());
        };
        let mut door_seam = false;
        for stair_edge in stair.exterior().lines() {
            for landing_edge in support.exterior().lines() {
                if let Some(intersection) = rounded_shared_edge(stair_edge, landing_edge, tolerance)
                {
                    if intersection.start == intersection.end {
                        continue;
                    }
                    for p in [intersection.start, intersection.end] {
                        let difference = (sa - a) * p.x + (sb - b) * p.y + sc - c;
                        if difference.abs() > 0.001 {
                            return Err(
                                "landing and stair heights disagree along their shared edge".into(),
                            );
                        }
                    }
                    door_seam |= point_edge_distance(
                        [f64::from(physical.middle[0]), f64::from(physical.middle[1])],
                        intersection,
                    ) <= tolerance;
                }
            }
        }
        if !door_seam {
            return Err("landing and stair have no shared edge at the door".into());
        }
        let ring =
            |line: &LineString<f32>| line.points().map(|p| [p.x(), p.y()]).collect::<Vec<_>>();
        let mut obstacles = Vec::new();
        for (index, obstacle) in motion.obstacles.iter().enumerate() {
            obstacle.validate_precise_polygon()?;
            let collision = if obstacle.precise_polygon.is_empty() {
                unproject(&obstacle.polygon.points)?
            } else {
                polygon(
                    &obstacle
                        .precise_polygon
                        .iter()
                        .map(|&[x, y]| [x as f32, ((y + a * x + c) / (1.0 - b)) as f32])
                        .collect::<Vec<_>>(),
                )?
            };
            for clipped in collision.intersection(support) {
                if !obstacle.precise_polygon.is_empty()
                    && rounded_seam_sliver(&clipped, &collision, &stair, tolerance)
                {
                    continue;
                }
                if !clipped.interiors().is_empty() {
                    return Err(
                        "landing collision clipping produced an unsupported holed solid".into(),
                    );
                }
                obstacles.push(BoundLandingObstacle {
                    motion_obstacle: u16::try_from(index)
                        .map_err(|_| "too many landing motion obstacles")?,
                    state: obstacle.state_id,
                    polygon: ring(clipped.exterior()),
                });
            }
        }
        self.landings.push(BoundLanding {
            boundary: ring(support.exterior()),
            holes: support.interiors().iter().map(ring).collect(),
            layer,
            area,
            sector,
            plane,
            obstacles,
        });
        Ok(())
    }
}

/// Clipping independently encoded f32 contours can leave a strip on their shared
/// edge. Only discard that strip when the original solid extends away from it;
/// a genuinely thin standalone obstacle must retain its collision.
fn rounded_seam_sliver(
    clipped: &Polygon<f32>,
    collision: &Polygon<f32>,
    stair: &Polygon<f32>,
    tolerance: f64,
) -> bool {
    if !clipped.interiors().is_empty() {
        return false;
    }
    // A real wall extending into the landing is not a stair-hole rounding
    // artifact. The solid must overlap more stair area than this clipped strip.
    if collision.intersection(stair).unsigned_area() <= clipped.unsigned_area() {
        return false;
    }
    collision.exterior().lines().any(|edge| {
        let precise_edge = edge.map_coords(|p| geo::Coord {
            x: f64::from(p.x),
            y: f64::from(p.y),
        });
        let distance = |p: geo::Point<f32>| {
            point_edge_distance([f64::from(p.x()), f64::from(p.y())], precise_edge)
        };
        stair.exterior().lines().any(|other| {
            rounded_shared_edge(edge, other, tolerance)
                .is_some_and(|shared| shared.start != shared.end)
        }) && clipped
            .exterior()
            .points()
            .all(|p| distance(p) <= tolerance)
            && collision
                .exterior()
                .points()
                .any(|p| distance(p) > tolerance)
    })
}

fn point_edge_distance(point: [f64; 2], edge: geo::Line<f64>) -> f64 {
    let dx = edge.end.x - edge.start.x;
    let dy = edge.end.y - edge.start.y;
    let length = dx * dx + dy * dy;
    let t = if length > 0. {
        ((point[0] - edge.start.x) * dx + (point[1] - edge.start.y) * dy) / length
    } else {
        0.
    };
    let t = t.clamp(0., 1.);
    (point[0] - edge.start.x - t * dx).hypot(point[1] - edge.start.y - t * dy)
}

/// Encoded boundary vertices may disagree by a few ULPs along the same seam.
/// Require a nonzero overlapping segment within that precision, not a nearby
/// corner or an extension across a real gap.
fn rounded_shared_edge(
    first: geo::Line<f32>,
    second: geo::Line<f32>,
    tolerance: f64,
) -> Option<geo::Line<f64>> {
    let convert = |c: geo::Coord<f32>| geo::Coord {
        x: f64::from(c.x),
        y: f64::from(c.y),
    };
    let first = first.map_coords(convert);
    let second = second.map_coords(convert);
    let dx = first.end.x - first.start.x;
    let dy = first.end.y - first.start.y;
    let length = dx * dx + dy * dy;
    if length == 0. {
        return None;
    }
    let project =
        |p: geo::Coord<f64>| ((p.x - first.start.x) * dx + (p.y - first.start.y) * dy) / length;
    let a = project(second.start);
    let b = project(second.end);
    let low = a.min(b).max(0.);
    let high = a.max(b).min(1.);
    if (high - low) * length.sqrt() <= tolerance {
        return None;
    }
    let at = |t| geo::Coord {
        x: first.start.x + t * dx,
        y: first.start.y + t * dy,
    };
    let overlap = geo::Line::new(at(low), at(high));
    [overlap.start, overlap.end]
        .iter()
        .all(|p| point_edge_distance([p.x, p.y], second) <= tolerance)
        .then_some(overlap)
}

fn receiver_matches_motion(
    receiver: &Polygon<f32>,
    motion: &crate::level_data::RawMotionArea,
    plane: [f64; 3],
) -> bool {
    let [a, b, c] = plane;
    let project = |line: &LineString<f32>| {
        let mut points = line
            .points()
            .map(|p| {
                let x = f64::from(p.x());
                let y = f64::from(p.y());
                (x, y - (a * x + b * y + c))
            })
            .collect::<Vec<_>>();
        // Material clipping may insert points along an existing edge. Rounding
        // those independently introduces kinks absent from the motion contour.
        // Remove only collinearity noise within the encoded f32 precision.
        let tolerance = points
            .iter()
            .fold(1.0_f64, |scale, &(x, y)| scale.max(x.abs()).max(y.abs()))
            * f64::from(f32::EPSILON)
            * 2.0;
        simplify_receiver_ring(&mut points, tolerance);
        for (x, y) in &mut points {
            // Match the exporter's nearest-integer ties toward positive infinity.
            *x = (*x + 0.5).floor();
            *y = (*y + 0.5).floor();
        }
        simplify_receiver_ring(&mut points, 0.0);
        LineString::from(points)
    };
    let rounded = Polygon::new(
        project(receiver.exterior()),
        receiver.interiors().iter().map(project).collect(),
    );
    if !rounded.is_valid() {
        return false;
    }
    let raw = Polygon::new(
        LineString::from(
            motion
                .polygon
                .points
                .iter()
                .map(|&(x, y)| (f64::from(x), f64::from(y)))
                .collect::<Vec<_>>(),
        ),
        Vec::new(),
    );
    rounded.difference(&raw).unsigned_area() < 1e-6
}

fn simplify_receiver_ring(points: &mut Vec<(f64, f64)>, tolerance: f64) {
    if points.first() == points.last() {
        points.pop();
    }
    loop {
        if points.len() < 3 {
            return;
        }
        let redundant = (0..points.len()).find(|&i| {
            let a = points[(i + points.len() - 1) % points.len()];
            let b = points[i];
            let c = points[(i + 1) % points.len()];
            let cross = ((b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0)).abs();
            let length = (c.0 - a.0)
                .hypot(c.1 - a.1)
                .max((b.0 - a.0).hypot(b.1 - a.1))
                .max((c.0 - b.0).hypot(c.1 - b.1));
            cross <= tolerance * length
        });
        let Some(index) = redundant else {
            break;
        };
        points.remove(index);
    }
}
