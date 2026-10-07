//! Ordinary walking keeps its world position and actual receiving-floor identity.

use super::*;
use crate::coordinates::{WorldPoint3D, WorldVec3D};
use geo::Intersects;
use robin_level_data::stair_navigation::StairNavigationPlane;

impl EngineInner {
    fn physical_walking_geometry(
        &self,
        assets: &LevelAssets,
        index: u32,
    ) -> crate::stair_navigation::walking_surface::PhysicalWalkingSurface {
        let floor = &assets.navigation.physical_walking[index as usize];
        let mut geometry = floor.snapshot(&self.world.pathfinder);
        for stair in assets.navigation.physical_stairs.values() {
            geometry.support.extend(stair.walking_support(
                &self.world.pathfinder,
                floor.layer,
                floor.sector,
                geometry.plane,
            ));
        }
        geometry
    }

    pub(super) fn current_physical_walking_floor(
        &self,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> Option<u32> {
        let entity = self.world.entities.get(owner)?;
        let pi = entity.position_iface();
        let receiver = pi.get_obstacle()?.get();
        let sector = entity.element_data().sector()?.get();
        let layer = entity.element_data().layer();
        let position = pi.get_position();
        assets
            .navigation
            .physical_walking
            .iter()
            .position(|floor| {
                floor.layer == layer
                    && floor.sector == sector
                    && floor.receivers.contains(&receiver)
                    && floor.contains_world_position([position.x, position.y, position.z])
            })
            .map(|index| u32::try_from(index).expect("too many physical walking floors"))
    }

    fn commit_walking_position(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        floor: u32,
        point: WorldPoint3D,
    ) -> bool {
        let binding = &assets.navigation.physical_walking[floor as usize];
        let receivers = self.sight_obstacles(assets);
        let receiver = binding
            .receivers
            .iter()
            .copied()
            .find(|index| {
                receivers.get(*index as usize).is_some_and(|receiver| {
                    receiver
                        .polygon
                        .as_geo()
                        .intersects(&geo::Point::new(point.x, point.y))
                })
            })
            .and_then(crate::sight_obstacle::SightObstacleIndex::new);
        let Some(receiver) = receiver else {
            tracing::warn!(
                ?owner,
                floor,
                ?point,
                "physical walk lost its receiving geometry"
            );
            return false;
        };
        self.set_obstacle_and_material(assets, owner, Some(receiver));
        let entity = self
            .world
            .entities
            .get_mut(owner)
            .expect("physical walking owner disappeared");
        entity.position_iface_mut().set_position(point);
        entity.element_data_mut().update_grid_cell();
        true
    }

    pub(super) fn extract_physical_walking_source(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        floor: u32,
    ) -> bool {
        let entity = self
            .world
            .entities
            .get(owner)
            .expect("walking source owner");
        let pi = entity.position_iface();
        let point = pi.get_position();
        let half = pi.get_half_diagonal();
        // Recovery must stay local to the actor's footprint; an unsupported
        // floor must not teleport its occupant across the map.
        let limit = f64::from(half.x.hypot(half.y)) + 0.5;
        let geometry = self.physical_walking_geometry(assets, floor);
        let recovered = geometry
            .recover_source([point.x, point.y, point.z], half, limit)
            .expect("invalid bound walking source geometry");
        let Some(recovered) = recovered else {
            // Failed source extraction does not itself cancel a Move. A narrow
            // floor can support its normal movement footprint without room for
            // the expanded recovery box. Continue only with proven movement
            // support; every following step still checks its full swept route.
            return geometry
                .route(
                    [point.x, point.y, point.z],
                    [point.x, point.y, point.z],
                    half,
                )
                .expect("invalid physical walking source clearance")
                .is_some();
        };
        self.commit_walking_position(
            assets,
            owner,
            floor,
            WorldPoint3D::new(
                recovered[0] as f32,
                recovered[1] as f32,
                recovered[2] as f32,
            ),
        )
    }

    pub(super) fn commit_physical_floor_step(
        &mut self,
        tcx: TickCtx<'_>,
        owner: EntityId,
        selected: SelectedMovementOrder,
        tolerance: FinalTol,
        speed: f32,
        fallback: MotionState,
        wait_for_animation: bool,
    ) -> MotionState {
        let Some(index) = selected.physical_walking else {
            return self.commit_physical_stair_step(
                tcx,
                owner,
                selected,
                tolerance,
                speed,
                fallback,
                wait_for_animation,
            );
        };
        assert!(
            selected.physical_stair.is_none(),
            "movement order has two physical floors"
        );
        let floor = &tcx.assets.navigation.physical_walking[index as usize];
        let entity = self
            .world
            .entities
            .get(owner)
            .expect("physical walking owner");
        assert_eq!(
            entity.element_data().sector().map(|sector| sector.get()),
            Some(floor.sector)
        );
        assert_eq!(entity.element_data().layer(), floor.layer);
        let position = entity.position_iface().get_position();
        let goal = selected.physical_goal;
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
            "invalid physical walking speed"
        );
        let half = entity.position_iface().get_half_diagonal();
        let mut geometry = self.physical_walking_geometry(tcx.assets, index);
        let plane = StairNavigationPlane::new(geometry.plane).expect("bound walking plane");
        let mut bounds = MapBBox::new();
        for point in &geometry.boundary {
            bounds.expand_point(MapPoint::new(
                point[0] as f32 - half.x,
                point[1] as f32 - half.y,
            ));
            bounds.expand_point(MapPoint::new(
                point[0] as f32 + half.x,
                point[1] as f32 + half.y,
            ));
        }
        let mut mover = super::super::anti_collision::CollisionMover::new(owner, entity);
        mover.position_map = MapPoint::new(position.x, position.y);
        if entity.position_iface().is_anti_collision_on() && mover.active {
            let (_, neighbours) = self
                .world
                .entities
                .split_owner(owner)
                .expect("physical walking owner");
            let obstacles = super::super::anti_collision::gather_physical_walking_neighbours(
                &mover,
                super::super::anti_collision::CollisionWorld {
                    neighbours,
                    profiles: &tcx.assets.profile_manager,
                },
                &bounds,
                &|other| {
                    let elem = other.element_data();
                    let p = elem.position();
                    elem.optional_layer()
                        .is_some_and(|layer| layer.get() == floor.layer)
                        && plane.contains_runtime_position([p.x, p.y, p.z])
                },
            );
            geometry
                .obstacles
                .extend(obstacles.into_iter().map(|point| {
                    let radius = f64::from(point.radius) / (std::f64::consts::PI / 16.0).cos();
                    (0..16)
                        .map(|index| {
                            let angle = f64::from(index) * std::f64::consts::TAU / 16.0;
                            [
                                f64::from(point.position.x) + angle.cos() * radius,
                                f64::from(point.position.y) + angle.sin() * radius,
                            ]
                        })
                        .collect()
                }));
        }
        let route = geometry
            .route(
                [position.x, position.y, position.z],
                [goal.x, goal.y, goal.z],
                half,
            )
            .expect("invalid physical walking collision");
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
            .find(|p| *p != [position.x, position.y])
            .unwrap_or_else(|| {
                // A precise door and a recomputed receiver can round Z
                // differently at identical XY. The route query validated both
                // heights; seat that same-position endpoint without inventing
                // a horizontal segment.
                assert_eq!(
                    [position.x, position.y],
                    [goal.x, goal.y],
                    "nonzero walking route needs a waypoint"
                );
                [goal.x, goal.y]
            });
        let step = plane
            .advance(
                [f64::from(position.x), f64::from(position.y)],
                target.map(f64::from),
                f64::from(speed),
            )
            .expect("physical walking step");
        let next = if step.reached && target == [goal.x, goal.y] {
            goal
        } else {
            WorldPoint3D::new(
                step.world[0] as f32,
                step.world[1] as f32,
                step.world[2] as f32,
            )
        };
        if !self.commit_walking_position(tcx.assets, owner, index, next) {
            return MotionState::Aborted;
        }
        let entity = self.world.entities.get_mut(owner).unwrap();
        let pi = entity.position_iface_mut();
        pi.reset_box_blocked();
        pi.set_physical_step_increment(
            WorldVec3D {
                x: next.x - position.x,
                y: next.y - position.y,
                z: next.z - position.z,
            },
            selected.order_compute_direction,
        );
        let distance = (next.x - position.x)
            .hypot(next.y - position.y)
            .hypot(next.z - position.z);
        refresh_motion_forecast(entity.sprite_mut(), distance);
        Self::emit_movement_water(entity, speed, &mut self.feedback.titbit_manager);
        // TODO: Share ordinary soft repulsion with physical walking corridors.
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
