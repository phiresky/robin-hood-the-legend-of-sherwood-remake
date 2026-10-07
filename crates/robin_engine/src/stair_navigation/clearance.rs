//! Climbing clearance retains actor dimensions along the supporting plane.

use super::*;
use geo::{ConvexHull, MultiPoint};

pub(super) fn rectangle_offsets(half: MoveBoxHalfDiagonal, inset: f64) -> Vec<[f64; 2]> {
    let [x, y] = [f64::from(half.x) - inset, f64::from(half.y) - inset];
    vec![[-x, -y], [x, -y], [x, y], [-x, y]]
}

fn plane_offsets(offsets: &[[f64; 2]], plane: [f64; 3]) -> Vec<[f64; 2]> {
    let [a, b, _] = plane;
    let gradient = a.hypot(b);
    if gradient == 0.0 {
        return offsets.to_vec();
    }
    assert!(gradient.is_finite(), "climbing gradient must be finite");
    let [nx, ny] = [a / gradient, b / gradient];
    let contraction = 1.0 - 1.0 / gradient.hypot(1.0);
    // Inverse positive square root of I + gradient*gradient^T. Lifting these
    // offsets onto the plane preserves the original lengths and angles in 3D.
    offsets
        .iter()
        .map(|&[x, y]| {
            let along = (x * nx + y * ny) * contraction;
            [x - along * nx, y - along * ny]
        })
        .collect()
}

impl BoundPhysicalStair {
    pub(crate) fn clearance_bounds_half(&self, half: MoveBoxHalfDiagonal) -> [f32; 2] {
        if !self.climbing {
            return [half.x, half.y];
        }
        let extent = self
            .clearance_offsets(half, 0.0)
            .iter()
            .fold([0.0_f64; 2], |extent, p| {
                [extent[0].max(p[0].abs()), extent[1].max(p[1].abs())]
            });
        extent.map(|value| (value as f32).next_up())
    }

    pub(super) fn clearance_offsets(&self, half: MoveBoxHalfDiagonal, inset: f64) -> Vec<[f64; 2]> {
        let rectangle = rectangle_offsets(half, inset);
        if !self.climbing {
            return rectangle;
        }
        if self.definition.floor_patches.is_empty() {
            return plane_offsets(&rectangle, self.definition.plane);
        }
        // A single query can cross several patches. Use the convex envelope of
        // their footprints so it cannot under-check clearance at a floor seam.
        let points = self.definition.floor_patches.iter().flat_map(|patch| {
            plane_offsets(&rectangle, patch.plane)
                .into_iter()
                .map(Point::from)
        });
        let hull = MultiPoint::from_iter(points).convex_hull();
        hull.exterior()
            .points()
            .map(|point| [point.x(), point.y()])
            .collect()
    }

    pub(super) fn clearance_polygon(
        &self,
        position: [f32; 2],
        half: MoveBoxHalfDiagonal,
    ) -> Polygon<f64> {
        if !self.climbing {
            return actor_footprint(position, half).map_coords(|point| geo::Coord {
                x: f64::from(point.x),
                y: f64::from(point.y),
            });
        }
        polygon(
            &self
                .clearance_offsets(half, 0.0)
                .iter()
                .map(|offset| {
                    [
                        f64::from(position[0]) + offset[0],
                        f64::from(position[1]) + offset[1],
                    ]
                })
                .collect::<Vec<_>>(),
        )
        .expect("valid physical actor clearance")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bound(plane: [f64; 3]) -> BoundPhysicalStair {
        BoundPhysicalStair {
            climbing: true,
            floor: None,
            layer: 0,
            area: 0,
            obstacle_states: vec![0],
            landings: vec![],
            definition: robin_level_data::physical_stair::PhysicalStairNavigation {
                plane,
                floor_patches: vec![],
                boundary: vec![[90., 90.], [110., 90.], [110., 110.], [90., 110.]],
                doors: vec![],
                obstacles: vec![],
            },
        }
    }

    #[test]
    fn closing_barrier_and_neighbour_bounds_use_the_climbing_footprint() {
        let mut stair = bound([0., 20., 0.]);
        stair
            .definition
            .obstacles
            .push(robin_level_data::physical_stair::PhysicalStairObstacle {
                motion_obstacle: 0,
                polygon: vec![[99., 101.], [101., 101.], [101., 102.], [99., 102.]],
            });
        let half = MoveBoxHalfDiagonal::new(6., 3.);
        assert!(!stair.obstacle_intersects_actor(0, [100., 100.], half));
        stair.definition.obstacles[0].polygon =
            vec![[99., 100.1], [101., 100.1], [101., 100.2], [99., 100.2]];
        assert!(stair.obstacle_intersects_actor(0, [100., 100.], half));
        stair.definition.plane = [-9., -9., 0.];
        let bounds = stair.clearance_bounds_half(half);
        for point in stair.clearance_offsets(half, 0.0) {
            assert!(point[0].abs() <= f64::from(bounds[0]));
            assert!(point[1].abs() <= f64::from(bounds[1]));
        }
        assert!(
            bounds[1] > half.y,
            "slope-aligned corners can extend beyond the old Y bounds"
        );
    }

    #[test]
    fn compound_clearance_contains_each_patch_footprint() {
        let mut stair = bound([0., 20., 0.]);
        stair.definition.floor_patches = [[0., 20., 0.], [20., 0., 0.]]
            .map(
                |plane| robin_level_data::stair_navigation_floor::StairFloorPatch {
                    plane,
                    boundary: vec![],
                },
            )
            .to_vec();
        let half = MoveBoxHalfDiagonal::new(6., 3.);
        let envelope = polygon(&stair.clearance_offsets(half, 1.0)).unwrap();
        for patch in &stair.definition.floor_patches {
            for point in plane_offsets(&rectangle_offsets(half, 1.0), patch.plane) {
                assert!(envelope.intersects(&Point::from(point)));
            }
        }
    }

    #[test]
    fn invalid_climbing_footprints_return_an_error_before_geometry_work() {
        let stair = bound([0., 20., 0.]);
        for half in [
            MoveBoxHalfDiagonal::new(f32::NAN, 3.),
            MoveBoxHalfDiagonal::new(6., f32::INFINITY),
            MoveBoxHalfDiagonal::new(1., 3.),
        ] {
            assert!(
                stair
                    .route(&PathFinder::new(), [100., 100.], [100., 101.], half)
                    .is_err()
            );
        }
    }

    #[test]
    fn planar_climbing_offsets_preserve_the_actor_box_on_the_surface() {
        let rectangle = rectangle_offsets(MoveBoxHalfDiagonal::new(6.0, 3.0), 1.0);
        for [a, b] in [[0., 0.], [2., -5.], [0., 20.], [-9., -9.]] {
            let offsets = plane_offsets(&rectangle, [a, b, 0.]);
            for i in 0..4 {
                for j in 0..4 {
                    let [dx, dy] = [offsets[i][0] - offsets[j][0], offsets[i][1] - offsets[j][1]];
                    let world_distance = dx.hypot(dy).hypot(a * dx + b * dy);
                    let profile_distance = (rectangle[i][0] - rectangle[j][0])
                        .hypot(rectangle[i][1] - rectangle[j][1]);
                    assert!((world_distance - profile_distance).abs() < 1e-10);
                }
            }
        }
    }

    #[test]
    fn steep_contact_uses_surface_clearance_but_still_blocks_real_barriers() {
        let half = MoveBoxHalfDiagonal::new(6., 3.);
        let offsets = plane_offsets(&rectangle_offsets(half, 1.0), [0., 20., 0.]);
        let mut geometry = StairRouteGeometry {
            boundary: vec![[90., 99.], [110., 99.], [110., 101.], [90., 101.]],
            obstacles: vec![],
        };
        let from = [100., 99.3];
        let to = [100., 100.7];
        assert!(
            geometry
                .route_with_landing_regions(from, to, half, &[], &[])
                .unwrap()
                .is_none()
        );
        assert!(
            geometry
                .route_with_clearance_regions(from, to, half, &offsets, &[], &[])
                .unwrap()
                .is_some()
        );
        geometry.obstacles.push(vec![
            [90., 99.95],
            [110., 99.95],
            [110., 100.05],
            [90., 100.05],
        ]);
        assert!(
            geometry
                .route_with_clearance_regions(from, to, half, &offsets, &[], &[])
                .unwrap()
                .is_none()
        );
        geometry.obstacles.clear();
        geometry.boundary = vec![[96., 99.], [104., 99.], [104., 101.], [96., 101.]];
        assert!(
            geometry
                .route_with_clearance_regions(from, to, half, &offsets, &[], &[])
                .unwrap()
                .is_none()
        );
    }
}
