//! Direct actor visibility queries against current geometry and the surface memo.
use super::*;
use crate::ai_enemy::{HalfPlane180, half_plane_180};

#[cfg(test)]
mod tests;

impl EngineInner {
    pub(in crate::engine) fn live_ai_detects_180(
        &mut self,
        assets: &LevelAssets,
        viewer_id: EntityId,
        target_id: EntityId,
    ) -> bool {
        let viewer = self
            .entities()
            .expect_entity(viewer_id, format_args!("180-degree viewer"));
        if self.entity_data_in_building_sector(viewer.element_data()) || !viewer.is_active() {
            return false;
        }
        let target = self
            .entities()
            .expect_entity(target_id, format_args!("180-degree target"));
        assert!(
            target.human_data().is_some(),
            "180-degree target must be human"
        );
        if !target.is_active() {
            return false;
        }
        let npc = viewer
            .ai_actor_data()
            .expect("180-degree viewer requires NPC data");
        let eye = viewer
            .compute_eyes_point(None)
            .expect("180-degree viewer requires eyes");
        let element = target.element_data();
        let point = crate::stealth::detection_point_world(
            element.position(),
            element.posture(),
            element.direction(),
            target.soldier_data().is_some_and(|soldier| soldier.rider),
        );
        let dx = point.x - eye.x;
        let dy = (point.y - eye.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
        let square_distance = dx * dx + dy * dy;
        let radius = f32::from(npc.view_radius);
        if square_distance > radius * radius {
            return false;
        }
        match half_plane_180(
            dx,
            dy,
            square_distance,
            viewer.element_data().direction() as u16,
        ) {
            HalfPlane180::Beside => return true,
            HalfPlane180::NotBeside { forward_dot } if forward_dot < 0.0 => return false,
            _ => {}
        }
        let obstacles = crate::sight_obstacle::ObstacleList {
            static_obstacles: &assets.environment.static_sight_obstacles,
            dynamic_obstacles: &self.world.dynamic_sight_obstacles,
            static_active: &self.world.static_sight_obstacle_active,
        };
        let surface = element.obstacle_index();
        let frame = self.control.frame_counter;
        let radius = if let Some(radius) = self.ai.view_radius_cache.get(surface, viewer_id, frame)
        {
            radius
        } else {
            let radius = crate::ai_vision::compute_view_radius(
                eye,
                npc.view_radius,
                (npc.view_direction[0], npc.view_direction[1]),
                npc.real_half_aperture,
                matches!(
                    self.world.weather.ambiance,
                    crate::engine::types::Ambiance::Night | crate::engine::types::Ambiance::Fog
                ),
                &self.world.fast_grid,
                obstacles,
                surface.map(|handle| {
                    obstacles
                        .get(usize::from(handle))
                        .expect("180-degree target sight obstacle is missing")
                }),
            );
            self.ai
                .view_radius_cache
                .set(surface, viewer_id, frame, radius);
            radius
        };
        if square_distance > radius * radius {
            return false;
        }
        crate::sight_obstacle::is_reachable_3d(
            obstacles,
            [eye.x, eye.y, eye.z],
            [point.x, point.y, point.z],
            crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
        )
    }

    /// Query current actors while retaining only the per-surface radius memo.
    pub(crate) fn npc_is_detecting_human(
        &mut self,
        assets: &LevelAssets,
        viewer_id: EntityId,
        target_id: EntityId,
        universal_frame: u32,
    ) -> bool {
        self.live_human_visibility_for_detection(
            assets,
            viewer_id,
            target_id,
            universal_frame,
            false,
        ) > 0.0
    }

    pub(super) fn live_human_visibility_for_detection(
        &mut self,
        assets: &LevelAssets,
        viewer_id: EntityId,
        target_id: EntityId,
        universal_frame: u32,
        seen_last_frame: bool,
    ) -> f32 {
        let viewer = self
            .world
            .entities
            .expect_entity(viewer_id, format_args!("human visibility viewer"));
        let Some(npc) = viewer.ai_actor_data() else {
            return 0.0;
        };
        let viewer_building = self.entity_building_sector(viewer.element_data().sector());
        if (!viewer.is_active() && viewer_building.is_none())
            || viewer.is_dead()
            || viewer.is_unconscious()
            || viewer.element_data().posture() == crate::element::Posture::Tied
        {
            return 0.0;
        }
        let ai = match viewer {
            Entity::Pc(_) | Entity::Soldier(_) => {
                &npc.ai_brain
                    .enemy()
                    .expect("eligible human visibility viewer requires EnemyAi")
                    .base
            }
            Entity::Civilian(_) => {
                &npc.ai_brain
                    .friendly()
                    .expect("eligible civilian visibility viewer requires FriendlyAi")
                    .base
            }
            _ => unreachable!("non-AI entity passed visibility viewer gate"),
        };
        let target = self
            .world
            .entities
            .expect_entity(target_id, format_args!("human visibility target"));
        let human = target
            .human_data()
            .expect("human visibility target requires human data");
        let target_element = target.element_data();
        let actor = target
            .actor_data()
            .expect("human visibility target requires actor data");
        let target_building = self.entity_building_sector(target_element.sector());
        let (eye, eye_world) = super::detection::human_eye_point_for_visibility(viewer);
        let posture = target_element.posture();
        let direction = target_element.direction();
        let target_world = crate::stealth::detection_point_world(
            target_element.position(),
            posture,
            direction,
            target.soldier_data().is_some_and(|soldier| soldier.rider),
        );
        let obstacles = crate::sight_obstacle::ObstacleList {
            static_obstacles: &assets.environment.static_sight_obstacles,
            dynamic_obstacles: &self.world.dynamic_sight_obstacles,
            static_active: &self.world.static_sight_obstacle_active,
        };
        let surface = target_element.obstacle_index();
        let target_obstacle = surface.map(|handle| {
            obstacles
                .get(usize::from(handle))
                .expect("human visibility target sight obstacle is missing")
        });
        let query = crate::ai_vision::VisibilityQuery {
            viewer_los: eye,
            viewer_world: eye_world,
            viewer_direction: viewer.element_data().direction(),
            view_forward: (npc.view_direction[0], npc.view_direction[1]),
            view_radius: npc.view_radius,
            viewer_eye_status: npc.eye_status,
            real_half_aperture: npc.real_half_aperture,
            viewer_in_building: viewer_building.is_some(),
            target_in_same_building: viewer_building.is_some()
                && viewer_building == target_building,
            forest_180_degree_view:
                super::detection::forest_180_degree_view_enabled_with_relationship(
                    self.world.weather.is_forest_level,
                    self.is_player_aligned_camp(viewer.camp()),
                ),
            golden_eye_mode: self.ai.global.golden_eye_mode,
            effective_view_radius: f32::from(npc.view_radius),
            target_is_active_and_outside_building: target.is_active() && target_building.is_none(),
            target_los: crate::stealth::detection_point_xy(
                target_element.position_map(),
                posture,
                direction,
            ),
            target_world,
            target_posture: posture,
            target_action_state: actor.action_state,
            target_is_pc: matches!(target, Entity::Pc(_)),
            cloak_deception_applies: posture == crate::element::Posture::Cloaked
                && viewer.camp().is_hostile_to(target.camp()),
            cloak_remembers_target: seen_last_frame
                || ai.primary_target == Some(crate::ai::AiEntityHandle::new(target_id.index()))
                || viewer
                    .enemy_ai()
                    .is_some_and(|enemy| enemy.list_them.contains(&target_id.index())),
            cloak_authored_detector: crate::cloak::SHIPPED_AUTHORED_DETECTOR,
            sight_obstacles: obstacles,
            fast_grid: &self.world.fast_grid,
            layer: viewer.element_data().layer(),
            target_dead: target.is_dead(),
            target_unconscious: human.unconscious,
            target_passing_door: selected_actor_is_passing_door(
                &self.world.entities,
                &self.orders.sequence_manager,
                target_id,
            ),
        };
        crate::ai_vision::compute_visibility_with_effective_radius(&query, || {
            if let Some(radius) = self
                .ai
                .view_radius_cache
                .get(surface, viewer_id, universal_frame)
            {
                return radius;
            }
            let radius = crate::ai_vision::compute_view_radius(
                eye_world,
                npc.view_radius,
                query.view_forward,
                npc.real_half_aperture,
                matches!(
                    self.world.weather.ambiance,
                    crate::engine::types::Ambiance::Night | crate::engine::types::Ambiance::Fog
                ),
                &self.world.fast_grid,
                obstacles,
                target_obstacle,
            );
            self.ai
                .view_radius_cache
                .set(surface, viewer_id, universal_frame, radius);
            radius
        })
    }
}
