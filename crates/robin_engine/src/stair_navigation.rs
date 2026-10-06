//! Actor-sized route queries in physical stair coordinates.
//!
//! The caller supplies a current collision snapshot, including connected landing
//! support. Screen projection is deliberately absent from pathfinding.

use geo::{Area, Contains, Intersects, LineString, Point, Polygon, Validation};
use serde::{Deserialize, Serialize};

use crate::coordinates::{MapBBox, MapPoint, MoveBoxHalfDiagonal};
use crate::fast_find_grid::{FastFindGrid, GridLine};
use crate::pathfinder::{MotionArea, MotionObstacle, PathFinder, PathGraph};

mod landing_binding;
mod landing_support;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StairRouteGeometry {
    /// One connected physical walking boundary, including authored landing support.
    pub boundary: Vec<[f32; 2]>,
    /// Currently active holes and movement solids. Rebuild after a control changes.
    pub obstacles: Vec<Vec<[f32; 2]>>,
}

/// Immutable physical geometry bound to the normal pathfinder's live state.
/// Keeping obstacle identities avoids a second, independently toggled state table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundPhysicalStair {
    pub definition: robin_level_data::physical_stair::PhysicalStairNavigation,
    layer: usize,
    area: usize,
    obstacle_states: Vec<u32>,
    #[serde(default)]
    landings: Vec<landing_binding::BoundLanding>,
    #[serde(default)]
    floor: Option<robin_level_data::stair_navigation_floor::StairNavigationFloor>,
}

impl BoundPhysicalStair {
    /// Resolve a screen point only when this floor has a unique inverse.
    /// Edge-on floors require an explicit world endpoint instead.
    pub fn world_point_from_screen(&self, point: MapPoint) -> Option<[f32; 3]> {
        if let Some(floor) = &self.floor {
            return floor.world_point_from_screen([point.x, point.y]);
        }
        let [a, b, c] = self.definition.plane;
        let determinant = 1.0 - b;
        if determinant.abs() < 1e-8 {
            return None;
        }
        let x = f64::from(point.x);
        let y = (f64::from(point.y) + a * x + c) / determinant;
        let world = [x as f32, y as f32, (a * x + b * y + c) as f32];
        world.iter().all(|value| value.is_finite()).then_some(world)
    }

    pub fn plane_at(&self, point: [f32; 2]) -> Result<[f64; 3], &'static str> {
        self.floor
            .as_ref()
            .map_or(Ok(self.definition.plane), |floor| {
                floor.runtime_plane_at(point)
            })
    }

    pub fn contains_runtime_position(&self, point: [f32; 3]) -> bool {
        self.plane_at([point[0], point[1]])
            .ok()
            .and_then(|coefficients| {
                robin_level_data::stair_navigation::StairNavigationPlane::new(coefficients).ok()
            })
            .is_some_and(|plane| plane.contains_runtime_position(point))
    }

    pub fn world_position(&self, point: [f32; 2]) -> Result<[f64; 3], &'static str> {
        robin_level_data::stair_navigation::StairNavigationPlane::new(self.plane_at(point)?)?
            .world_position(point.map(f64::from))
    }

    pub fn advance(
        &self,
        from: [f32; 2],
        to: [f32; 2],
        distance: f64,
    ) -> Result<robin_level_data::stair_navigation::StairRouteStep, &'static str> {
        if let Some(floor) = &self.floor {
            return floor.advance_runtime(from, to, distance);
        }
        robin_level_data::stair_navigation::StairNavigationPlane::new(self.definition.plane)?
            .advance(from.map(f64::from), to.map(f64::from), distance)
    }

    pub fn route_distance(&self, from: [f32; 2], to: [f32; 2]) -> Result<f64, &'static str> {
        if let Some(floor) = &self.floor {
            return floor.route_distance_runtime(from, to);
        }
        robin_level_data::stair_navigation::StairNavigationPlane::new(self.definition.plane)?
            .route_distance(from.map(f64::from), to.map(f64::from))
    }

    /// Test crushing against the actual floor footprint, not its potentially
    /// collapsed screen projection. Several pieces can share a motion identity.
    pub fn obstacle_intersects_actor(
        &self,
        motion_obstacle: u16,
        position: [f32; 2],
        half: MoveBoxHalfDiagonal,
    ) -> bool {
        let actor = actor_footprint(position, half);
        self.definition
            .obstacles
            .iter()
            .filter(|obstacle| obstacle.motion_obstacle == motion_obstacle)
            .any(|obstacle| {
                polygon(&obstacle.polygon)
                    .expect("validated physical obstacle")
                    .intersects(&actor)
            })
    }

    pub fn bind(
        lift: &crate::level_data::RawLift,
        motion_area: &crate::level_data::RawMotionArea,
        layer: usize,
        area: usize,
    ) -> Result<Self, String> {
        let definition = lift
            .physical_navigation
            .as_ref()
            .ok_or("lift has no physical stair navigation")?;
        definition.validate(lift, motion_area)?;
        polygon(&definition.boundary)?;
        for obstacle in &definition.obstacles {
            polygon(&obstacle.polygon)?;
        }
        Ok(Self {
            definition: definition.clone(),
            layer,
            area,
            obstacle_states: motion_area
                .obstacles
                .iter()
                .map(|obstacle| obstacle.state_id)
                .collect(),
            landings: Vec::new(),
            floor: if definition.floor_patches.is_empty() {
                None
            } else {
                Some(
                    robin_level_data::stair_navigation_floor::StairNavigationFloor::new(
                        definition.floor_patches.clone(),
                    )?,
                )
            },
        })
    }

    /// Rebuild collision from the current state, including after rollback or an
    /// already-issued route's barrier changes. No cached route implies clearance.
    pub fn route(
        &self,
        pathfinder: &PathFinder,
        source: [f32; 2],
        goal: [f32; 2],
        half_diagonal: MoveBoxHalfDiagonal,
    ) -> Result<Option<Vec<[f32; 2]>>, String> {
        self.route_with_obstacles(pathfinder, source, goal, half_diagonal, &[])
    }

    pub fn route_with_obstacles(
        &self,
        pathfinder: &PathFinder,
        source: [f32; 2],
        goal: [f32; 2],
        half_diagonal: MoveBoxHalfDiagonal,
        extra_obstacles: &[Vec<[f32; 2]>],
    ) -> Result<Option<Vec<[f32; 2]>>, String> {
        let geometry = StairRouteGeometry {
            boundary: self.definition.boundary.clone(),
            obstacles: self
                .definition
                .obstacles
                .iter()
                .filter(|obstacle| {
                    pathfinder.is_motion_obstacle_active(
                        self.layer,
                        self.area,
                        self.obstacle_states[usize::from(obstacle.motion_obstacle)],
                    )
                })
                .map(|obstacle| obstacle.polygon.clone())
                .chain(extra_obstacles.iter().cloned())
                .collect(),
        };
        if self.landings.is_empty() {
            return geometry.route(source, goal, half_diagonal);
        }
        let support = self
            .landings
            .iter()
            .map(|landing| landing.boundary.clone())
            .collect::<Vec<_>>();
        let mut precise_obstacles = Vec::new();
        for landing in &self.landings {
            precise_obstacles.extend(landing.holes.iter().cloned());
            precise_obstacles.extend(
                landing
                    .obstacles
                    .iter()
                    .filter(|obstacle| {
                        pathfinder.is_motion_obstacle_active(
                            landing.layer,
                            landing.area,
                            obstacle.state,
                        )
                    })
                    .map(|obstacle| obstacle.polygon.clone()),
            );
        }
        geometry.route_with_precise_landing_support(
            source,
            goal,
            half_diagonal,
            &support,
            &precise_obstacles,
        )
    }
}

fn actor_footprint(position: [f32; 2], half: MoveBoxHalfDiagonal) -> Polygon<f32> {
    geo::Rect::new(
        geo::Coord {
            x: position[0] - half.x,
            y: position[1] - half.y,
        },
        geo::Coord {
            x: position[0] + half.x,
            y: position[1] + half.y,
        },
    )
    .to_polygon()
}

fn polygon<T: geo::GeoFloat>(points: &[[T; 2]]) -> Result<Polygon<T>, String> {
    if points.len() < 3 || points.iter().flatten().any(|x| !x.is_finite()) {
        return Err("physical stair polygon requires at least three finite points".into());
    }
    let polygon = Polygon::new(
        LineString::from(points.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>()),
        Vec::new(),
    );
    if !polygon.is_valid() || polygon.unsigned_area() <= T::zero() {
        return Err("physical stair polygon is degenerate or self-intersecting".into());
    }
    Ok(polygon)
}

impl StairRouteGeometry {
    /// Uses the normal engine footprint and visibility routing rules, in ground
    /// XY. Returns source through destination, or `None` for a valid but blocked
    /// request. Invalid geometry is an error, never an empty successful route.
    pub fn route(
        &self,
        source: [f32; 2],
        goal: [f32; 2],
        half_diagonal: MoveBoxHalfDiagonal,
    ) -> Result<Option<Vec<[f32; 2]>>, String> {
        if source.iter().chain(&goal).any(|x| !x.is_finite())
            || !half_diagonal.x.is_finite()
            || !half_diagonal.y.is_finite()
            || half_diagonal.x <= 1.0
            || half_diagonal.y <= 1.0
        {
            return Err(
                "physical stair query requires finite points and a positive effective footprint"
                    .into(),
            );
        }
        let boundary = polygon(&self.boundary)?;
        let obstacles = self
            .obstacles
            .iter()
            .map(|ring| polygon(ring))
            .collect::<Result<Vec<_>, _>>()?;
        let supported = |p: [f32; 2]| {
            let point = Point::new(p[0], p[1]);
            boundary.contains(&point) && !obstacles.iter().any(|obstacle| obstacle.contains(&point))
        };
        if !supported(source) || !supported(goal) {
            return Ok(None);
        }
        // Use a local positive grid to support assets placed at negative world
        // coordinates without changing footprint shape or clipping geometry.
        let mut minimum = [f32::INFINITY; 2];
        let mut maximum = [f32::NEG_INFINITY; 2];
        for p in self.boundary.iter().chain(self.obstacles.iter().flatten()) {
            for axis in 0..2 {
                minimum[axis] = minimum[axis].min(p[axis]);
                maximum[axis] = maximum[axis].max(p[axis]);
            }
        }
        let span = [
            maximum[0] - minimum[0] + 128.0,
            maximum[1] - minimum[1] + 128.0,
        ];
        if span
            .iter()
            .any(|value| !value.is_finite() || *value > 32700.0)
        {
            return Err("physical stair geometry exceeds local navigation grid bounds".into());
        }
        let local = |p: [f32; 2]| MapPoint::new(p[0] - minimum[0] + 64.0, p[1] - minimum[1] + 64.0);
        let mut grid = FastFindGrid::new();
        grid.size_map(
            (span[0] / 64.0).ceil() as u16,
            (span[1] / 64.0).ceil() as u16,
        );
        grid.allocate_layers(1);
        grid.add_move_box_half_diagonal(half_diagonal);
        let mut register = |ring: &[[f32; 2]], is_area: bool| {
            let mut points = ring.iter().map(|p| local(*p)).collect::<Vec<_>>();
            if points.first() == points.last() {
                let _ = points.pop();
            }
            let winding: f32 = points
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let q = points[(i + 1) % points.len()];
                    p.x * q.y - q.x * p.y
                })
                .sum();
            if winding < 0.0 {
                points.reverse();
            }
            let lines = (0..points.len())
                .map(|i| {
                    let mut line = GridLine::new(
                        points[(i + points.len() - 1) % points.len()],
                        points[i],
                        true,
                    );
                    line.initialize_motion_normal(is_area);
                    grid.add_line(line, 0)
                })
                .collect::<Vec<_>>();
            (points, lines)
        };
        let (floor, _) = register(&self.boundary, true);
        let mut motion_obstacles = Vec::new();
        for ring in &self.obstacles {
            let (points, lines) = register(ring, false);
            let mut bounds = MapBBox::new();
            for point in &points {
                bounds.expand_point(*point);
            }
            motion_obstacles.push(MotionObstacle {
                state_id: 0,
                active: true,
                bounding_box: bounds,
                polygon: points,
                grid_sector_index: None,
                grid_line_indices: lines,
            });
        }
        let mut graph = PathGraph::new();
        graph.static_mut().move_layers = vec![vec![MotionArea {
            skeleton: Vec::new(),
            polygon: floor,
            motion_obstacles,
        }]];
        graph.static_mut().half_diagonals.push(half_diagonal);
        graph.layers = vec![vec![vec![Vec::new(); self.obstacles.len()]]];
        graph.alternative_layers = graph.layers.clone();
        graph.states = vec![vec![0]];
        graph.build_sector_conversion();
        let mut finder = PathFinder::new();
        finder.initialize_from_graph(&graph, &mut grid);
        let start = local(source);
        let end = local(goal);
        // Source containment is a separate precondition; the normal pathfinder
        // can repair unauthorized sources, which this physical route must not do.
        let effective = [half_diagonal.x - 1.0, half_diagonal.y - 1.0];
        let authorized = |p: MapPoint| {
            grid.is_position_authorized(
                &MapBBox::from_coords(
                    p.x - effective[0],
                    p.y - effective[1],
                    p.x + effective[0],
                    p.y + effective[1],
                ),
                0,
            )
        };
        if !authorized(start) || !authorized(end) {
            return Ok(None);
        }
        let Some(path) = finder.find_path(&graph, &grid, 0, 0, 0, start, end, false) else {
            return Ok(None);
        };
        if path.first() != Some(&start)
            || path.last() != Some(&end)
            || path.iter().any(|p| !authorized(*p))
            || path
                .windows(2)
                .any(|pair| !grid.is_reachable_thick(pair[0], pair[1], 0, half_diagonal))
        {
            return Err("pathfinder returned an invalid physical stair route".into());
        }
        let mut route = path
            .iter()
            .map(|p| [p.x + minimum[0] - 64.0, p.y + minimum[1] - 64.0])
            .collect::<Vec<_>>();
        // Preserve exact caller endpoints after the local-grid translation.
        route[0] = source;
        *route.last_mut().expect("path endpoints checked above") = goal;
        Ok(Some(route))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::Intersects;
    use robin_level_data::stair_navigation::StairNavigationPlane;

    fn rectangle(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<[f32; 2]> {
        vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
    }

    #[test]
    fn edge_on_route_detours_around_actual_collision() {
        let geometry = StairRouteGeometry {
            boundary: rectangle(-50.0, -50.0, 50.0, 150.0),
            obstacles: vec![rectangle(-20.0, 35.0, 20.0, 65.0)],
        };
        let route = geometry
            .route([0.0, 0.0], [0.0, 100.0], MoveBoxHalfDiagonal::new(6.0, 3.0))
            .unwrap()
            .unwrap();
        assert!(
            route.len() > 2,
            "route must not pass through the central block"
        );
        assert!(route.iter().any(|p| p[0].abs() > 20.0));
        let plane = StairNavigationPlane::new([0.0, 1.0, 40.0]).unwrap();
        assert_eq!(
            plane.screen_position([0.0, 0.0]),
            plane.screen_position([0.0, 100.0])
        );
        for pair in route.windows(2) {
            assert!(
                plane
                    .route_distance(pair[0].map(f64::from), pair[1].map(f64::from))
                    .unwrap()
                    > 0.0
            );
            // Independent sampled footprint oracle: use polygons, not grid lines.
            for step in 0..=100 {
                let t = step as f32 / 100.0;
                let x = pair[0][0] * (1.0 - t) + pair[1][0] * t;
                let y = pair[0][1] * (1.0 - t) + pair[1][1] * t;
                let footprint = polygon(&rectangle(x - 5.0, y - 2.0, x + 5.0, y + 2.0)).unwrap();
                assert!(polygon(&geometry.boundary).unwrap().contains(&footprint));
                assert!(
                    !polygon(&geometry.obstacles[0])
                        .unwrap()
                        .intersects(&footprint)
                );
            }
        }
    }

    #[test]
    fn walls_and_unsupported_endpoints_do_not_produce_routes() {
        let mut geometry = StairRouteGeometry {
            boundary: rectangle(0.0, 0.0, 100.0, 200.0),
            obstacles: vec![rectangle(0.0, 95.0, 100.0, 105.0)],
        };
        let footprint = MoveBoxHalfDiagonal::new(6.0, 3.0);
        assert!(
            geometry
                .route([50.0, 30.0], [50.0, 170.0], footprint)
                .unwrap()
                .is_none()
        );
        geometry.obstacles.clear();
        assert!(
            geometry
                .route([50.0, 30.0], [50.0, 170.0], footprint)
                .unwrap()
                .is_some()
        );
        assert!(
            geometry
                .route([-10.0, 30.0], [50.0, 170.0], footprint)
                .unwrap()
                .is_none()
        );
        assert!(
            geometry
                .route([1.0, 30.0], [50.0, 170.0], footprint)
                .unwrap()
                .is_none()
        );
        geometry.boundary[1] = [f32::NAN, 0.0];
        assert!(
            geometry
                .route([50.0, 30.0], [50.0, 170.0], footprint)
                .is_err()
        );
    }

    #[test]
    fn small_block_inside_the_swept_footprint_requires_a_detour() {
        let geometry = StairRouteGeometry {
            boundary: rectangle(0.0, 0.0, 100.0, 200.0),
            obstacles: vec![rectangle(49.0, 99.0, 51.0, 101.0)],
        };
        let route = geometry
            .route(
                [50.0, 30.0],
                [50.0, 170.0],
                MoveBoxHalfDiagonal::new(6.0, 3.0),
            )
            .unwrap()
            .unwrap();
        assert!(route.len() > 2);
    }

    #[test]
    fn rotated_and_translated_physical_routes_preserve_blockers() {
        for angle in [0.0_f32, 37.0, 90.0, 180.0, 270.0] {
            let (sine, cosine) = angle.to_radians().sin_cos();
            let place = |p: [f32; 2]| {
                [
                    p[0] * cosine - p[1] * sine - 300.25,
                    p[0] * sine + p[1] * cosine + 450.125,
                ]
            };
            let boundary = rectangle(-50.0, -50.0, 50.0, 150.0)
                .into_iter()
                .map(place)
                .collect();
            let block = rectangle(-20.0, 35.0, 20.0, 65.0)
                .into_iter()
                .map(place)
                .collect();
            let geometry = StairRouteGeometry {
                boundary,
                obstacles: vec![block],
            };
            for (source, goal) in [
                (place([0.0, 0.0]), place([0.0, 100.0])),
                (place([0.0, 100.0]), place([0.0, 0.0])),
            ] {
                let route = geometry
                    .route(source, goal, MoveBoxHalfDiagonal::new(6.0, 3.0))
                    .unwrap()
                    .unwrap();
                assert_eq!(route.first(), Some(&source));
                assert_eq!(route.last(), Some(&goal));
                assert!(route.len() > 2);
            }
        }
    }

    #[test]
    fn short_stair_requires_real_connected_landing_support() {
        let mut geometry = StairRouteGeometry {
            boundary: rectangle(0.0, 48.0, 100.0, 52.0),
            obstacles: Vec::new(),
        };
        let footprint = MoveBoxHalfDiagonal::new(6.0, 3.0);
        assert!(
            geometry
                .route([30.0, 50.0], [70.0, 50.0], footprint)
                .unwrap()
                .is_none()
        );
        // The caller supplies the union with actual landings; the router itself
        // never widens an unsupported stair floor to make a path fit.
        geometry.boundary = rectangle(0.0, 20.0, 100.0, 80.0);
        assert!(
            geometry
                .route([30.0, 50.0], [70.0, 50.0], footprint)
                .unwrap()
                .is_some()
        );
    }
}
