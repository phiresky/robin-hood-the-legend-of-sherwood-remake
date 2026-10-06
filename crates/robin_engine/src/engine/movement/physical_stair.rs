//! Physical stair execution through the ordinary actor animation/order loop.

use super::*;
use crate::coordinates::{WorldPoint3D, WorldVec3D};

impl EngineInner {
    pub(super) fn commit_physical_stair_step(
        &mut self,
        tcx: TickCtx<'_>,
        owner: EntityId,
        selected: SelectedMovementOrder,
        tolerance: FinalTol,
        speed: f32,
        fallback: MotionState,
        wait_for_animation: bool,
    ) -> MotionState {
        let sector = selected
            .physical_stair
            .expect("physical order lost its stair");
        let stair = tcx
            .assets
            .navigation
            .physical_stairs
            .get(&sector)
            .expect("physical order references an unloaded stair");
        let entity = self
            .world
            .entities
            .get(owner)
            .expect("physical stair owner disappeared");
        assert_eq!(
            entity.element_data().sector().map(|sector| sector.get()),
            Some(sector),
            "physical order must execute in its owning stair sector"
        );
        let position = entity.position_iface().get_position();
        let goal = selected.physical_goal;
        for point in [position, goal] {
            assert!(
                stair.contains_runtime_position([point.x, point.y, point.z]),
                "physical stair motion must start and end on its floor"
            );
        }
        if position == goal {
            if wait_for_animation {
                let entity = self.world.entities.get_mut(owner).unwrap();
                entity.position_iface_mut().zero_all_increments();
                if speed > 0.0 {
                    refresh_motion_forecast(entity.sprite_mut(), speed);
                    Self::emit_movement_water(entity, speed, &mut self.feedback.titbit_manager);
                }
                return fallback;
            }
            return self.settle_movement_waypoint(
                tcx,
                tolerance,
                selected,
                owner,
                MovementArrivalBoundary {
                    tolerance_arrival: false,
                    point_seek_post_arrival: false,
                    arrived_after_committed_step: false,
                    live_seek_target: None,
                },
            );
        }
        if speed == 0.0 {
            return fallback;
        }
        assert!(
            speed.is_finite() && speed > 0.0,
            "physical stair speed must be finite and nonnegative"
        );

        let half = entity.position_iface().get_half_diagonal();
        let mut bounds = MapBBox::new();
        for point in &stair.definition.boundary {
            bounds.expand_point(MapPoint::new(point[0] - half.x, point[1] - half.y));
            bounds.expand_point(MapPoint::new(point[0] + half.x, point[1] + half.y));
        }
        let mut mover = super::super::anti_collision::CollisionMover::new(owner, entity);
        mover.position_map = MapPoint::new(position.x, position.y);
        let dynamic = if entity.position_iface().is_anti_collision_on() && mover.active {
            let (_, neighbours) = self
                .world
                .entities
                .split_owner(owner)
                .expect("physical owner disappeared");
            super::super::anti_collision::gather_physical_stair_neighbours(
                &mover,
                super::super::anti_collision::CollisionWorld {
                    neighbours,
                    profiles: &tcx.assets.profile_manager,
                },
                &bounds,
                &|other| {
                    let elem = other.element_data();
                    let (Some(layer), Some(sector)) = (elem.optional_layer(), elem.sector()) else {
                        return false;
                    };
                    let position = elem.position();
                    stair.supports_landing_neighbour(
                        layer.get(),
                        sector.get(),
                        [position.x, position.y, position.z],
                    )
                },
            )
            .into_iter()
            .map(|point| {
                // Circumscribe the hard personal-space radius; the mover's own
                // footprint is applied by the physical corridor query.
                let radius = point.radius / (std::f32::consts::PI / 16.0).cos();
                (0..16)
                    .map(|index| {
                        let angle = index as f32 * std::f32::consts::TAU / 16.0;
                        [
                            point.position.x + angle.cos() * radius,
                            point.position.y + angle.sin() * radius,
                        ]
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        // Re-evaluate controls and neighbours before every step. A route accepted
        // earlier is not authority to cross a barrier that has since closed.
        let route = stair
            .route_with_obstacles(
                &self.world.pathfinder,
                [position.x, position.y],
                [goal.x, goal.y],
                half,
                &dynamic,
            )
            .expect("invalid physical stair collision geometry");
        let Some(route) = route else {
            let pi = self
                .world
                .entities
                .get_mut(owner)
                .unwrap()
                .position_iface_mut();
            pi.update_forecasted_movement(0.0, 1);
            pi.update_box_blocked(MapPoint::new(position.x, position.y));
            if pi.is_blocked() {
                pi.reset_box_blocked();
                return MotionState::Aborted;
            }
            return fallback;
        };
        let target = route
            .iter()
            .skip(1)
            .copied()
            .find(|point| *point != [position.x, position.y])
            .expect("nonzero physical route lost its next waypoint");
        let step = stair
            .advance([position.x, position.y], target, f64::from(speed))
            .expect("invalid physical stair step");
        let next = if step.reached && target == [goal.x, goal.y] {
            goal
        } else {
            WorldPoint3D::new(
                step.world[0] as f32,
                step.world[1] as f32,
                step.world[2] as f32,
            )
        };
        let entity = self.world.entities.get_mut(owner).unwrap();
        let pi = entity.position_iface_mut();
        if !stair.definition.floor_patches.is_empty() {
            let [a, b, c] = stair
                .plane_at([next.x, next.y])
                .expect("physical step lost its floor");
            pi.set_obstacle_at_ground_position(
                None,
                Some(crate::position_interface::PlaneZCoeffs {
                    az: a as f32,
                    bz: b as f32,
                    dz: c as f32,
                }),
                crate::coordinates::GroundPoint::new(next.x, next.y),
            )
            .expect("invalid joined stair receiver");
        }
        pi.set_position(next);
        pi.reset_box_blocked();
        pi.set_physical_step_increment(
            WorldVec3D {
                x: next.x - position.x,
                y: next.y - position.y,
                z: next.z - position.z,
            },
            selected.order_compute_direction,
        );
        let travelled = if stair.definition.floor_patches.is_empty() {
            (next.x - position.x)
                .hypot(next.y - position.y)
                .hypot(next.z - position.z)
        } else {
            stair
                .route_distance([position.x, position.y], [next.x, next.y])
                .expect("physical step lost its supported route") as f32
        };
        refresh_motion_forecast(entity.sprite_mut(), travelled);
        Self::emit_movement_water(entity, speed, &mut self.feedback.titbit_manager);
        // TODO: share soft repulsion with ordinary movement.
        if next == goal {
            if wait_for_animation {
                entity.position_iface_mut().zero_all_increments();
                entity.sprite_mut().compute_display_depth();
                return fallback;
            }
            self.settle_movement_waypoint(
                tcx,
                tolerance,
                selected,
                owner,
                MovementArrivalBoundary {
                    tolerance_arrival: false,
                    point_seek_post_arrival: false,
                    arrived_after_committed_step: true,
                    live_seek_target: None,
                },
            )
        } else {
            fallback
        }
    }
}
