//! Bind ordinary walking to actual receivers without stair-specific clipping.

use super::*;
use walking_surface::PhysicalWalkingSurface;

mod neighbours;

/// Fit physical navigation from the receiver anchors without rounding the
/// coefficients to f32. Rounding the intercept before inverse projection can
/// separate an otherwise shared edge by several coordinate ULPs after placement.
pub(crate) fn receiver_plane(points: &[[f32; 3]; 3]) -> Result<[f64; 3], &'static str> {
    let [p, q, r] = points.map(|point| point.map(f64::from));
    let u = std::array::from_fn::<_, 3, _>(|i| q[i] - p[i]);
    let v = std::array::from_fn::<_, 3, _>(|i| r[i] - p[i]);
    let normal = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    if normal[2].abs() < 1e-9 {
        return Err("physical receiver anchors do not define a height plane");
    }
    let a = -normal[0] / normal[2];
    let b = -normal[1] / normal[2];
    let plane = [a, b, p[2] - a * p[0] - b * p[1]];
    if plane.iter().any(|value| !value.is_finite()) {
        return Err("physical receiver plane must be finite");
    }
    Ok(plane)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundPhysicalWalkingSurface {
    pub layer: u16,
    pub sector: u16,
    pub receivers: Vec<u32>,
    area: usize,
    geometry: PhysicalWalkingSurface,
    obstacle_states: Vec<u32>,
    #[serde(default)]
    neighbours: Vec<neighbours::WalkingNeighbour>,
}

impl BoundPhysicalWalkingSurface {
    pub fn contains_world_position(&self, point: [f32; 3]) -> bool {
        use robin_level_data::stair_navigation::StairNavigationPlane;
        if !StairNavigationPlane::new(self.geometry.plane)
            .expect("bound walking plane")
            .contains_runtime_position(point)
        {
            return false;
        }
        let point = Point::new(f64::from(point[0]), f64::from(point[1]));
        polygon(&self.geometry.boundary)
            .expect("bound walking boundary")
            .intersects(&point)
            && !self.geometry.holes.iter().any(|hole| {
                polygon(hole)
                    .expect("bound walking hole")
                    .intersects(&point)
            })
    }

    pub fn world_point_from_screen(&self, point: MapPoint) -> Option<[f32; 3]> {
        let [a, b, c] = self.geometry.plane;
        let x = f64::from(point.x);
        let y = (f64::from(point.y) + a * x + c) / (1.0 - b);
        let result = [x as f32, y as f32, (a * x + b * y + c) as f32];
        result.iter().all(|v| v.is_finite()).then_some(result)
    }

    /// Bind a coplanar receiver group to its motion region. Different heights
    /// and disconnected components remain separate physical floors.
    pub fn bind(
        motion: &crate::level_data::RawMotionArea,
        layer: u16,
        area: usize,
        sector: u16,
        plane: [f64; 3],
        receivers: Vec<u32>,
        coverage: &geo::MultiPolygon<f32>,
    ) -> Result<Vec<Self>, String> {
        if motion.is_lift {
            return Err("ordinary walking cannot bind a lift motion region".into());
        }
        let [a, b, c] = plane;
        if plane.iter().any(|v| !v.is_finite()) || (1.0 - b).abs() < 1e-8 {
            return Err(
                "walking collision needs an invertible projection or explicit world geometry"
                    .into(),
            );
        }
        if receivers.is_empty() || coverage.0.is_empty() {
            return Err("physical walking requires an actual receiving floor".into());
        }
        let unproject = |grid: &crate::level_data::SectorPolygon, precise: &[[f64; 2]]| {
            let points = if precise.is_empty() {
                grid.points
                    .iter()
                    .map(|&(x, y)| [f64::from(x), f64::from(y)])
                    .collect::<Vec<_>>()
            } else {
                precise.to_vec()
            };
            polygon(
                &points
                    .into_iter()
                    .map(|[x, y]| [x, (y + a * x + c) / (1.0 - b)])
                    .collect::<Vec<_>>(),
            )
        };
        let floor = unproject(&motion.polygon, &motion.precise_polygon)?;
        let mut support = geo::MultiPolygon::new(vec![]);
        for receiver in coverage {
            if !receiver.is_valid() || receiver.unsigned_area() <= 0.0 {
                return Err("invalid physical walking receiver".into());
            }
            let precise = receiver.map_coords(|p| geo::Coord {
                x: f64::from(p.x),
                y: f64::from(p.y),
            });
            if motion.precise_polygon.is_empty()
                && super::landing_binding::receiver_matches_motion(receiver, motion, plane)
            {
                support = support.union(&precise);
            } else {
                support = support.union(&floor.intersection(&precise));
            }
        }
        let obstacles = motion
            .obstacles
            .iter()
            .map(|obstacle| {
                obstacle.validate_precise_polygon()?;
                Ok(unproject(&obstacle.polygon, &obstacle.precise_polygon)?
                    .exterior()
                    .points()
                    .map(|p| [p.x(), p.y()])
                    .collect())
            })
            .collect::<Result<Vec<Vec<[f64; 2]>>, String>>()?;
        let ring = |line: &LineString<f64>| line.points().map(|p| [p.x(), p.y()]).collect();
        support
            .into_iter()
            .map(|floor| {
                if !floor.is_valid() {
                    return Err("walking receiver intersection is invalid".into());
                }
                Ok(Self {
                    layer,
                    sector,
                    area,
                    receivers: receivers.clone(),
                    geometry: PhysicalWalkingSurface {
                        boundary: ring(floor.exterior()),
                        holes: floor.interiors().iter().map(ring).collect(),
                        plane,
                        obstacles: obstacles.clone(),
                        support: vec![],
                    },
                    obstacle_states: motion.obstacles.iter().map(|o| o.state_id).collect(),
                    neighbours: vec![],
                })
            })
            .collect()
    }

    /// Refresh collision from the owning motion area, including restored state.
    pub fn snapshot(&self, pathfinder: &PathFinder) -> PhysicalWalkingSurface {
        assert_eq!(
            self.geometry.obstacles.len(),
            self.obstacle_states.len(),
            "walking obstacle state identities changed"
        );
        PhysicalWalkingSurface {
            obstacles: self
                .geometry
                .obstacles
                .iter()
                .zip(&self.obstacle_states)
                .filter(|(_, state)| {
                    pathfinder.is_motion_obstacle_active(
                        usize::from(self.layer),
                        self.area,
                        **state,
                    )
                })
                .map(|(polygon, _)| polygon.clone())
                .collect(),
            boundary: self.geometry.boundary.clone(),
            holes: self.geometry.holes.clone(),
            plane: self.geometry.plane,
            support: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn motion() -> crate::level_data::RawMotionArea {
        serde_json::from_value(serde_json::json!({
            "is_lift": false, "state_id": 0, "flags": 0, "skeleton_segments": [],
            "polygon": {"points": [[0,0],[100,0],[100,100],[0,100]]},
            "obstacles": [{"state_id": 2, "polygon": {"points": [[45,0],[55,0],[55,100],[45,100]]}}]
        }))
        .unwrap()
    }

    #[test]
    fn whole_receiver_routes_read_live_state_and_survive_restore() {
        let raw = motion();
        let coverage = geo::MultiPolygon::from(vec![
            polygon(&[[0., 0.], [100., 0.], [100., 100.], [0., 100.]]).unwrap(),
        ]);
        let floors =
            BoundPhysicalWalkingSurface::bind(&raw, 0, 0, 0, [0.; 3], vec![7], &coverage).unwrap();
        let encoded = serde_json::to_string(&floors[0]).unwrap();
        let bound: BoundPhysicalWalkingSurface = serde_json::from_str(&encoded).unwrap();
        let mut pathfinder = PathFinder::new();
        pathfinder.states = vec![vec![0]];
        let route = |pathfinder: &PathFinder| {
            bound
                .snapshot(pathfinder)
                .route(
                    [20., 50., 0.],
                    [80., 50., 0.],
                    MoveBoxHalfDiagonal::new(6., 3.),
                )
                .unwrap()
        };
        assert!(route(&pathfinder).is_some());
        pathfinder.states[0][0] = 2;
        assert!(route(&pathfinder).is_none());
        pathfinder.states[0][0] = 0;
        assert!(route(&pathfinder).is_some());
        assert_eq!(bound.receivers, vec![7]);
    }

    #[test]
    fn disconnected_receivers_never_become_a_single_walkable_floor() {
        let mut raw = motion();
        raw.obstacles.clear();
        let coverage = geo::MultiPolygon::from(vec![
            polygon(&[[0., 0.], [40., 0.], [40., 100.], [0., 100.]]).unwrap(),
            polygon(&[[60., 0.], [100., 0.], [100., 100.], [60., 100.]]).unwrap(),
        ]);
        let floors =
            BoundPhysicalWalkingSurface::bind(&raw, 0, 0, 0, [0.; 3], vec![7, 8], &coverage)
                .unwrap();
        assert_eq!(floors.len(), 2);
        let mut pf = PathFinder::new();
        pf.states = vec![vec![0]];
        assert!(floors.iter().all(|floor| {
            floor
                .snapshot(&pf)
                .route(
                    [20., 50., 0.],
                    [80., 50., 0.],
                    MoveBoxHalfDiagonal::new(6., 3.),
                )
                .unwrap()
                .is_none()
        }));
    }

    #[test]
    fn receiver_holes_and_pre_grid_boundaries_are_retained() {
        let mut raw = motion();
        raw.obstacles.clear();
        let exterior =
            polygon(&[[0.25, 0.25], [99.75, 0.25], [99.75, 99.75], [0.25, 99.75]]).unwrap();
        let hole = polygon(&[[40., 40.], [60., 40.], [60., 60.], [40., 60.]]).unwrap();
        let coverage = geo::MultiPolygon::from(vec![Polygon::new(
            exterior.exterior().clone(),
            vec![hole.exterior().clone()],
        )]);
        let floors =
            BoundPhysicalWalkingSurface::bind(&raw, 0, 0, 0, [0.; 3], vec![7], &coverage).unwrap();
        assert_eq!(floors[0].geometry.holes.len(), 1);
        assert!(
            floors[0]
                .geometry
                .boundary
                .iter()
                .all(|p| p.iter().all(|v| *v >= 0.25 && *v <= 99.75))
        );
        let mut pf = PathFinder::new();
        pf.states = vec![vec![0]];
        assert!(
            floors[0]
                .snapshot(&pf)
                .route(
                    [50., 50., 0.],
                    [50., 50., 0.],
                    MoveBoxHalfDiagonal::new(6., 3.)
                )
                .unwrap()
                .is_none()
        );
    }
}
