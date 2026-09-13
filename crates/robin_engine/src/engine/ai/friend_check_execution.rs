//! Waypoint friend checks read the partner's live brain and path.

use super::*;
use crate::ai::{AiController, AiEntityHandle, AiState, LookDirection, PathId, Position, Substate};

fn synchronize_index(current: u16, encoded: u16) -> u16 {
    if encoded > 500 {
        (current as i32 + encoded as i16 as i32 - 1000) as u16
    } else {
        encoded
    }
}

fn look_count(frames: u16, interval: u16) -> u8 {
    (frames / interval + 1) as u8
}

fn path_status(ai: &AiController) -> (u16, u16, bool, Option<PathId>) {
    if let Some(path) = &ai.patrol_path {
        (
            path.current_waypoint_index as u16,
            path.last_waypoint_index as u16,
            path.forward,
            Some(path.hiking_path_index),
        )
    } else {
        let path = &ai.detached_patrol_path_status;
        (
            path.current_waypoint_index as u16,
            path.last_waypoint_index as u16,
            path.forward,
            path.hiking_path_index,
        )
    }
}

impl EngineInner {
    pub(in crate::engine) fn initialize_ai_friend_check(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        friend_id: u16,
        frames: u16,
        index: u16,
    ) {
        // The civilian role has no friend-check implementation.
        if self
            .world
            .entities
            .get(owner)
            .and_then(Entity::enemy_ai)
            .is_none()
        {
            return;
        }
        let target_handle = *self
            .ai
            .global
            .all_soldier_handles
            .get(friend_id as usize)
            .expect("friend-check soldier index is out of range");
        let target = self.expect_human_id_for_ai_handle(target_handle, "friend-check partner");
        assert!(
            self.expect_entity(target, "friend-check partner")
                .npc_data()
                .is_some(),
            "friend-check partner must be an NPC"
        );
        self.friend_check_owner_mut(owner)
            .set_checkpoint_charly(Some(AiEntityHandle::new(target_handle)));
        assert_ne!(
            owner, target,
            "friend-check partner cannot be the checking actor"
        );
        self.drain_direct_ai_owner_boundary(sim, owner, assets);

        let ai = self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("friend-check owner"));
        if ai.missed_in_action.contains(&target_handle)
            || (ai.frame_when_enemy_detected > 0
                && self
                    .control
                    .frame_counter
                    .wrapping_sub(ai.frame_when_enemy_detected)
                    < crate::parameters_ai::NO_CHECK_FOR_AFTER_CHARLY_ALERT_TIME)
        {
            self.friend_check_owner_mut(owner)
                .set_checkpoint_charly(None);
            self.resume_after_friend_check(sim, assets, owner);
            return;
        }

        if frames == 0 && index != u16::MAX {
            let current = path_status(
                self.world
                    .entities
                    .expect_ai_controller(owner, format_args!("synchronizing owner")),
            )
            .0;
            let destination = synchronize_index(current, index);
            let ai = self.friend_check_owner_mut(owner);
            ai.synchronize_index = destination;
            ai.synchronize_charly = Some(AiEntityHandle::new(target_handle));
            ai.set_checkpoint_charly(None);
            assert!(
                ai.macro_in_progress,
                "pure friend synchronization requires a running macro"
            );
            self.drain_direct_ai_owner_boundary(sim, owner, assets);

            let entity = self.expect_entity(target, "synchronizing partner");
            let partner = entity
                .ai_controller()
                .expect("synchronizing partner needs AI");
            if partner.current_state != AiState::Default || entity.is_dead() {
                self.resume_after_friend_check(sim, assets, owner);
                return;
            }
            let (current, last, partner_forward, _) = path_status(partner);
            let (_, _, forward, _) = path_status(
                self.world
                    .entities
                    .expect_ai_controller(owner, format_args!("synchronizing owner")),
            );
            let waypoint = if partner.macro_in_progress {
                Some(current)
            } else if partner.current_substate == Substate::DefaultEnroute {
                Some(last)
            } else {
                None
            };
            let already_there = waypoint.is_some_and(|waypoint| {
                if index < 500 {
                    waypoint == destination
                } else if partner_forward != forward {
                    forward
                } else if forward {
                    waypoint >= destination
                } else {
                    waypoint <= destination
                }
            });
            if already_there {
                self.resume_after_friend_check(sim, assets, owner);
            } else {
                self.world
                    .entities
                    .expect_ai_controller_mut(target, format_args!("synchronizing partner"))
                    .synchronizing_actors
                    .push(owner.index());
                self.friend_check_state(sim, assets, owner, Substate::DefaultSynchronizing);
            }
            return;
        }

        let partner = self
            .world
            .entities
            .expect_ai_controller(target, format_args!("friend-check partner"));
        if !partner.has_patrol_path {
            let post = partner.initial_position;
            let mut point = self.friend_check_world_point(assets, post);
            if !self.friend_check_detects_point(owner, assets, point) {
                point.z += 15.0;
                assert!(
                    self.friend_check_detects_point(owner, assets, point),
                    "friend-check partner's post is not visible"
                );
            }
            assert_eq!(
                index,
                u16::MAX,
                "cannot synchronize with a partner without a path"
            );
        } else {
            let path = path_status(partner)
                .3
                .expect("friend-check partner has no authored path");
            let waypoints = &assets.navigation.hiking_paths[path.get() as usize].waypoints;
            let count = waypoints.len() as u16;
            let mut visible = false;
            for waypoint_index in 0..count {
                let waypoint = &waypoints[waypoint_index as usize];
                let position = Position {
                    x: waypoint.x as f32,
                    y: waypoint.y as f32,
                    level: waypoint.level,
                    sector: assets.navigation.hiking_waypoint_sector(
                        path.get() as usize,
                        waypoint_index as usize,
                        waypoint.sector,
                    ),
                };
                let mut point = self.friend_check_world_point(assets, position);
                point.z += 15.0;
                if self.friend_check_detects_point(owner, assets, point) {
                    visible = true;
                    break;
                }
            }
            if !visible {
                tracing::warn!(
                    ?owner,
                    ?target,
                    "no waypoint of friend-check partner is visible"
                );
                self.resume_after_friend_check(sim, assets, owner);
                return;
            }
        }

        let ai = self.friend_check_owner_mut(owner);
        if index == u16::MAX {
            ai.synchronize_charly = None;
            ai.synchronize_index = u16::MAX;
        } else {
            ai.synchronize_charly = Some(AiEntityHandle::new(target_handle));
            ai.synchronize_index = synchronize_index(path_status(ai).0, index);
        }
        ai.number_of_looks = look_count(
            frames,
            crate::parameters_ai::AI_CHECKFOR_TIME_INTERVAL as u16,
        );
        ai.delta_sorrow_level = 1000 / ai.number_of_looks as u16;
        self.friend_check_state(
            sim,
            assets,
            owner,
            Substate::DefaultLookingSidewardsForCharly,
        );
        let direction = if crate::sim_rng::u32(
            sim,
            crate::sim_rng::RngSite::CheckForLookDirection,
            0..2,
        ) != 0
        {
            LookDirection::LeftRight
        } else {
            LookDirection::RightLeft
        };
        self.friend_check_owner_mut(owner)
            .outbox
            .actor
            .look_sidewards = Some(direction);
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
    }

    fn friend_check_owner_mut(&mut self, owner: EntityId) -> &mut AiController {
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("friend-check owner"))
    }

    fn friend_check_state(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        substate: Substate,
    ) {
        self.world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("friend-check state"))
            .set_state(AiState::Default, substate);
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
    }

    fn resume_after_friend_check(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        self.friend_check_state(sim, assets, owner, Substate::DefaultInMacro);
        self.run_ai_macro(sim, assets, owner);
    }

    fn friend_check_world_point(
        &self,
        assets: &LevelAssets,
        position: Position,
    ) -> crate::coordinates::WorldPoint3D {
        crate::ai::ai_position_to_point_3d(
            &self.world.fast_grid,
            crate::sight_obstacle::ObstacleList {
                static_obstacles: &assets.environment.static_sight_obstacles,
                dynamic_obstacles: &self.world.dynamic_sight_obstacles,
                static_active: &self.world.static_sight_obstacle_active,
            },
            position,
        )
    }

    fn friend_check_detects_point(
        &self,
        owner: EntityId,
        assets: &LevelAssets,
        point: crate::coordinates::WorldPoint3D,
    ) -> bool {
        let entity = self.expect_entity(owner, "friend-check observer");
        if self
            .entity_building_sector(entity.element_data().sector())
            .is_some()
        {
            return false;
        }
        let eye = entity
            .compute_eyes_point(None)
            .expect("friend-check observer requires eyes");
        let radius = entity
            .ai_actor_data()
            .expect("friend-check observer requires AI")
            .view_radius as f32;
        let dx = point.x - eye.x;
        let dy = (point.y - eye.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
        let dz = point.z - eye.z;
        if dx * dx + dy * dy + dz * dz > radius * radius {
            return false;
        }
        crate::sight_obstacle::is_reachable_3d(
            crate::sight_obstacle::ObstacleList {
                static_obstacles: &assets.environment.static_sight_obstacles,
                dynamic_obstacles: &self.world.dynamic_sight_obstacles,
                static_active: &self.world.static_sight_obstacle_active,
            },
            [eye.x, eye.y, eye.z],
            [point.x, point.y, point.z],
            crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{DetachedPatrolPathStatus, MacroOpcode};
    use crate::engine::test_support::{actors::make_test_ai_soldier, square_sector};
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    fn checking_pair() -> (EngineInner, LevelAssets, EntityId, EntityId) {
        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(16, 16);
        engine.world.fast_grid_mut().allocate_layers(1);
        let sector_index = engine.world.fast_grid_mut().add_sector(
            square_sector(1, 0, MapPoint::new(0.0, 0.0), MapPoint::new(1000.0, 1000.0)),
            0,
        );
        let sector = crate::position_interface::SectorHandle::new(1)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(sector_index).unwrap());
        let owner = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));
        let target = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));
        for id in [owner, target] {
            let entity = engine.world.entities.get_mut(id).unwrap();
            entity.element_data_mut().set_sector(Some(sector));
            entity
                .element_data_mut()
                .set_position_map(MapPoint::new(10.0, 10.0));
            let npc = entity.npc_data_mut().unwrap();
            npc.life_points = 100;
            npc.view_radius = 1000;
        }
        engine.ai.global.all_soldier_handles = std::sync::Arc::new(vec![target.index()]);
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        assets.navigation.hiking_paths = std::sync::Arc::new(vec![RawHikingPath {
            waypoints: vec![RawWaypoint {
                x: 20,
                y: 10,
                sector: 1,
                level: 0,
                command: WaypointCommand::None,
            }],
        }]);
        assets.navigation.hiking_waypoint_sectors = Some(std::sync::Arc::new(vec![vec![sector]]));
        let ai = engine.friend_check_owner_mut(owner);
        ai.current_state = AiState::Default;
        ai.current_substate = Substate::DefaultInMacro;
        ai.macro_in_progress = true;
        ai.macro_command = vec![0; 20];
        ai.macro_command[17] = MacroOpcode::Wait as u8;
        ai.macro_command[18..20].copy_from_slice(&75_u16.to_le_bytes());
        ai.macro_command_offset = 17;
        ai.number_of_remaining_macro_bytes = 3;
        let ai = engine.friend_check_owner_mut(target);
        ai.current_state = AiState::Default;
        ai.current_substate = Substate::DefaultInMacro;
        ai.has_patrol_path = true;
        ai.detached_patrol_path_status = DetachedPatrolPathStatus {
            hiking_path_index: PathId::new(0),
            forward: true,
            ..Default::default()
        };
        (engine, assets, owner, target)
    }

    #[test]
    fn detached_partner_path_is_visible_without_consuming_following_wait() {
        let (mut engine, assets, owner, target) = checking_pair();
        engine.initialize_ai_friend_check(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            0,
            225,
            u16::MAX,
        );
        let ai = engine.friend_check_owner_mut(owner);
        assert_eq!(
            ai.current_substate,
            Substate::DefaultLookingSidewardsForCharly
        );
        assert_eq!(
            ai.checkpoint_charly,
            Some(AiEntityHandle::new(target.index()))
        );
        assert_eq!(ai.macro_command_offset, 17);
        assert_eq!(ai.number_of_remaining_macro_bytes, 3);
        assert!(!ai.macro_timer_is_running);
    }

    #[test]
    fn already_synchronized_partner_resumes_macro_on_the_same_stack() {
        let (mut engine, assets, owner, target) = checking_pair();
        let partner = engine.friend_check_owner_mut(target);
        partner.macro_in_progress = true;
        partner.detached_patrol_path_status.current_waypoint_index = 5;
        engine.initialize_ai_friend_check(&crate::sim_rng::test_context(), &assets, owner, 0, 0, 5);
        let ai = engine.friend_check_owner_mut(owner);
        assert_eq!(ai.current_substate, Substate::DefaultInMacro);
        assert!(ai.macro_timer_is_running);
        assert_eq!(ai.number_of_remaining_macro_bytes, 0);
        assert!(
            engine
                .friend_check_owner_mut(target)
                .synchronizing_actors
                .is_empty()
        );
    }

    #[test]
    fn pure_sync_registers_on_live_partner_before_waiting() {
        let (mut engine, assets, owner, target) = checking_pair();
        engine.initialize_ai_friend_check(&crate::sim_rng::test_context(), &assets, owner, 0, 0, 5);
        assert_eq!(
            engine.friend_check_owner_mut(owner).current_substate,
            Substate::DefaultSynchronizing
        );
        assert_eq!(
            engine.friend_check_owner_mut(target).synchronizing_actors,
            vec![owner.index()]
        );
        assert!(
            engine
                .friend_check_owner_mut(owner)
                .outbox
                .reentrant
                .cross_npc_actions
                .is_empty()
        );
    }

    #[test]
    fn point_visibility_uses_live_leaning_eyes_and_radius() {
        let (mut engine, assets, owner, _) = checking_pair();
        let entity = engine.world.entities.get_mut(owner).unwrap();
        entity
            .element_data_mut()
            .set_position_map(MapPoint::new(0.0, 0.0));
        entity.element_data_mut().set_direction_instantly(4);
        entity
            .element_data_mut()
            .set_posture(crate::element::Posture::LeaningOut);
        entity.npc_data_mut().unwrap().view_radius = 11;
        let point = crate::coordinates::WorldPoint3D {
            x: 50.0,
            y: 0.0,
            z: 45.0,
        };
        assert!(engine.friend_check_detects_point(owner, &assets, point));
        engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .npc_data_mut()
            .unwrap()
            .view_radius = 9;
        assert!(!engine.friend_check_detects_point(owner, &assets, point));
    }

    #[test]
    fn relative_indices_and_look_counts_preserve_narrowing() {
        assert_eq!(synchronize_index(7, 500), 500);
        assert_eq!(synchronize_index(3, 1002), 5);
        assert_eq!(synchronize_index(1, 998), u16::MAX);
        assert_eq!(synchronize_index(0, u16::MAX), 64_535);
        assert_eq!(look_count(254, 1), 255);
        assert_eq!(look_count(255, 1), 0);
    }
}
