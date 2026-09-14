//! Direct actor visibility queries against current geometry and the surface memo.

use super::*;

#[cfg(test)]
mod tests;

impl EngineInner {
    pub(in crate::engine) fn live_ai_detects_180(
        &mut self,
        assets: &LevelAssets,
        viewer_id: EntityId,
        target_id: EntityId,
    ) -> bool {
        let entity = self.expect_entity(viewer_id, "180-degree viewer");
        let in_building = self.entity_data_in_building_sector(entity.element_data());
        if in_building || !entity.is_active() {
            return false;
        }
        let npc = entity
            .ai_actor_data()
            .expect("180-degree viewer requires NPC data");
        let eye = entity
            .compute_eyes_point(None)
            .expect("180-degree viewer requires eyes");
        let viewer = crate::ai_enemy::Viewer180 {
            entity: viewer_id,
            eye_ground: crate::coordinates::GroundPoint::new(eye.x, eye.y),
            eye_z: eye.z,
            direction: entity.element_data().direction() as u16,
            in_building,
            view_radius: npc.view_radius,
            sq_view_radius: f32::from(npc.view_radius) * f32::from(npc.view_radius),
            view_direction: npc.view_direction,
            real_half_aperture: npc.real_half_aperture,
        };
        let entity = self.expect_entity(target_id, "180-degree target");
        assert!(
            entity.human_data().is_some(),
            "180-degree target must be human"
        );
        let element = entity.element_data();
        let target = crate::ai_enemy::Target180 {
            handle: target_id.index(),
            active: entity.is_active(),
            detection_world: crate::stealth::detection_point_world(
                element.position(),
                element.posture(),
                element.direction(),
                entity.soldier_data().is_some_and(|soldier| soldier.rider),
            ),
            obstacle: element.obstacle_index(),
        };
        let obstacles = crate::sight_obstacle::ObstacleList {
            static_obstacles: &assets.environment.static_sight_obstacles,
            dynamic_obstacles: &self.world.dynamic_sight_obstacles,
            static_active: &self.world.static_sight_obstacle_active,
        };
        let frame = self.control.frame_counter;
        let night_or_fog = matches!(
            self.world.weather.ambiance,
            crate::engine::types::Ambiance::Night | crate::engine::types::Ambiance::Fog
        );
        crate::ai_enemy::detects_180_degrees_live(&viewer, &target, obstacles, || {
            if let Some(radius) = self
                .ai
                .view_radius_cache
                .get(target.obstacle, viewer_id, frame)
            {
                return radius;
            }
            let obstacle = target.obstacle.map(|handle| {
                obstacles
                    .get(usize::from(handle))
                    .expect("180-degree target sight obstacle is missing")
            });
            let radius = crate::ai_vision::compute_view_radius(
                eye,
                viewer.view_radius,
                (viewer.view_direction[0], viewer.view_direction[1]),
                viewer.real_half_aperture,
                night_or_fog,
                &self.world.fast_grid,
                obstacles,
                obstacle,
            );
            self.ai
                .view_radius_cache
                .set(target.obstacle, viewer_id, frame, radius);
            radius
        })
    }
}
