//! Landing geometry comes from the current motion area and its actual receiver.

use super::*;
use geo::BooleanOps;

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
        let support = if let Some(receiver) = receiver {
            floor.intersection(receiver)
        } else {
            geo::MultiPolygon::from(vec![floor])
        };
        let Some(support) = support.iter().find(|patch| {
            [physical.middle, physical.outside]
                .iter()
                .all(|point| patch.intersects(&Point::new(point[0], point[1])))
        }) else {
            return Err("landing receiver does not reach its physical door".into());
        };
        let stair = polygon(&self.definition.boundary)?;
        if stair.intersection(support).unsigned_area() > 0.001 {
            return Err("landing receiver overlaps the stair in ground space".into());
        }
        let [sa, sb, sc] = self.definition.plane;
        let mut door_seam = false;
        for stair_edge in stair.exterior().lines() {
            for landing_edge in support.exterior().lines() {
                if let Some(geo::line_intersection::LineIntersection::Collinear { intersection }) =
                    geo::line_intersection::line_intersection(stair_edge, landing_edge)
                {
                    if intersection.start == intersection.end {
                        continue;
                    }
                    for p in [intersection.start, intersection.end] {
                        let difference =
                            (sa - a) * f64::from(p.x) + (sb - b) * f64::from(p.y) + sc - c;
                        if difference.abs() > 0.001 {
                            return Err(
                                "landing and stair heights disagree along their shared edge".into(),
                            );
                        }
                    }
                    door_seam |= intersection
                        .intersects(&Point::new(physical.middle[0], physical.middle[1]));
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
            for clipped in unproject(&obstacle.polygon.points)?.intersection(support) {
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
