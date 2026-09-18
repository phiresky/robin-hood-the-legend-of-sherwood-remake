//! Operations whose callers continue after a completed return to duty.

use super::*;
use crate::ai::{
    AiEntityHandle, AiSpeechAttempt, AiState, DutyFlags, GotoFlags, Remark, ReportType, Substate,
};
use crate::ai_enemy::SeekFlags;
use crate::sim_rng::SimulationContext;

impl EngineInner {
    pub(in crate::engine) fn execute_finish_exhausted_search(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let enemy = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("completed search"));
        if enemy.base.my_reconnaissance_report.report_type <= ReportType::Noise
            && !enemy
                .seek_flags
                .intersects(SeekFlags::REPORT_OFFICER_AFTER | SeekFlags::LOOK_FOR_HELP_AFTER)
        {
            self.execute_ai_speech(
                sim,
                assets,
                owner,
                AiSpeechAttempt {
                    remark: Remark::EndsSearch,
                    flags: 0,
                },
            );
        }
    }

    pub(in crate::engine) fn execute_kill_nearby_sleeping_enemies(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        observer_camp: Camp,
    ) {
        self.execute_ai_unfocus(owner);

        let trainer = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("sleeping enemy search duty gate"))
            .combat_trainer;
        let entity = self.expect_entity(owner, "sleeping enemy search forest gate");
        let forest_foot_soldier = self.is_player_aligned_camp(entity.camp())
            && self.world.weather.is_forest_level
            && !entity.soldier_data().is_some_and(|soldier| soldier.rider);
        if trainer || forest_foot_soldier {
            self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
        }

        self.world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("sleeping enemy list reset"))
            .list_them
            .clear();
        // Registry membership is selected after duty callbacks. No callback occurs
        // inside this scan; candidate properties are read in registration order.
        let fighter_count = self.world.fighter_registry_ids.len();
        for index in 0..fighter_count {
            let target = self.world.fighter_registry_ids[index];
            let entity = self.expect_entity(target, "sleeping enemy candidate");
            if !self.camps_are_hostile(observer_camp, entity.camp())
                || !entity.is_unconscious()
                || entity
                    .human_data()
                    .expect("fighter must be human")
                    .carrier
                    .is_some()
                || !self.patrol_member_visible(assets, owner, target)
                || !self.sleeping_enemy_attack_allowed(owner, target)
            {
                continue;
            }
            self.world
                .entities
                .expect_enemy_ai_mut(owner, format_args!("sleeping enemy list insertion"))
                .list_them
                .push(target.index());
        }

        self.approach_selected_sleeping_enemy(sim, assets, owner);
    }

    pub(in crate::engine) fn execute_approach_sleeping_enemies(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        targets: Vec<crate::ai::HumanHandle>,
    ) {
        let enemy = self
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("retained sleeping enemies"));
        assert!(
            enemy.list_them.is_empty(),
            "retained sleeping enemies require an empty hostile list"
        );
        enemy.list_them = targets;
        self.approach_selected_sleeping_enemy(sim, assets, owner);
    }

    pub(in crate::engine) fn select_nearest_battle_target(
        &self,
        owner: EntityId,
    ) -> Option<EntityId> {
        self.select_live_ai_primary_target(owner, crate::ai_enemy::PrimaryTargetFlags::empty())
            .map(|target| self.expect_human_id_for_ai_handle(target.get(), "nearest battle target"))
    }

    fn approach_selected_sleeping_enemy(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let nearest = self.select_nearest_battle_target(owner);
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("sleeping enemy primary target"))
            .primary_target = nearest.map(|id| AiEntityHandle::new(id.index()));
        if nearest.is_some() {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Attacking,
                Substate::AttackingApproachingSleepingEnemy,
            );
            let target = self
                .world
                .entities
                .expect_ai_controller(
                    owner,
                    format_args!("sleeping enemy primary after state change"),
                )
                .primary_target
                .expect("sleeping enemy approach requires primary target");
            let target =
                self.expect_human_id_for_ai_handle(target.get(), "sleeping enemy approach target");
            let position = self.live_ai_position(target);
            self.duty_go_near(sim, assets, owner, position, 20, GotoFlags::RUN);
        } else {
            self.execute_ai_return_to_duty(sim, assets, owner, DutyFlags::empty());
        }
    }

    pub(in crate::engine) fn sleeping_enemy_attack_allowed(
        &self,
        owner: EntityId,
        target: EntityId,
    ) -> bool {
        let vip = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("sleeping enemy attack authorization"))
            .is_vip;
        let target = self.expect_entity(target, "sleeping enemy authorization target");
        (!vip || matches!(target, Entity::Pc(pc) if pc.pc.robin))
            && (matches!(target, Entity::Pc(_)) || !target.is_vip())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinates::WorldPoint3D;
    use crate::element::Posture;
    use crate::engine::test_support::actors::{make_test_ai_soldier, make_test_pc};

    fn sleeping_pair(
        first: MapPoint,
        second: MapPoint,
    ) -> (EngineInner, LevelAssets, EntityId, [EntityId; 2]) {
        let mut engine = EngineInner::new();
        let (sector, _) = crate::engine::test_support::extra_engine_combat::square_sector_map(
            &mut engine,
            (128, 128),
            (2000.0, 2000.0),
        );
        let owner = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        let targets = [
            engine.add_test_entity(make_test_pc(Posture::Lying)),
            engine.add_test_entity(make_test_pc(Posture::Lying)),
        ];
        for (id, position) in [
            (owner, MapPoint::new(1377.2015, 252.88869)),
            (targets[0], first),
            (targets[1], second),
        ] {
            let entity = engine.ent_mut(id);
            entity.element_data_mut().set_sector(Some(sector));
            entity
                .element_data_mut()
                .set_position(WorldPoint3D::new(position.x, position.y, 0.0));
            entity.actor_data_mut().unwrap().action_state = crate::element::ActionState::Waiting;
            entity.human_data_mut().unwrap().unconscious = id != owner;
        }
        engine
            .ent_mut(owner)
            .ai_actor_data_mut()
            .unwrap()
            .view_radius = 500;
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        let enemy = engine
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("test sleeper observer"));
        enemy.base.initial_position = crate::ai::Position {
            x: 1377.2015,
            y: 252.88869,
            sector: Some(sector),
            level: 0,
        };
        (engine, assets, owner, targets)
    }

    #[test]
    fn sleeping_enemy_visibility_rejects_indoor_targets() {
        let visible = |indoors| {
            patrol_member_visible_from_raw_world(
                WorldPoint3D::new(0.0, 0.0, 0.0),
                false,
                500,
                false,
                WorldPoint3D::new(100.0, 0.0, 0.0),
                Posture::Lying,
                false,
                0,
                indoors,
                crate::sight_obstacle::ObstacleList::empty(),
            )
        };
        assert!(visible(false));
        assert!(!visible(true));
    }

    #[test]
    fn trainer_sleeping_enemy_scan_runs_after_completed_duty() {
        let (mut engine, assets, owner, targets) =
            sleeping_pair(MapPoint::new(1380.0, 252.0), MapPoint::new(1400.0, 252.0));
        let enemy = engine
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("test trainer"));
        enemy.combat_trainer = true;
        enemy.base.current_state = AiState::Attacking;
        enemy.base.current_substate = Substate::AttackingBowObserving;
        enemy.list_them = vec![owner.index()];
        engine.execute_kill_nearby_sleeping_enemies(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            Camp::Lacklandists,
        );
        let enemy = engine
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("test trainer after scan"));
        assert_eq!(
            enemy.base.current_substate,
            Substate::AttackingApproachingSleepingEnemy
        );
        assert_eq!(
            enemy.base.primary_target,
            Some(AiEntityHandle::new(targets[0].index()))
        );
        assert_eq!(enemy.list_them, targets.map(|id| id.index()));
    }

    #[test]
    fn sleeping_enemy_selection_uses_isometric_distance_and_live_positions() {
        let (mut engine, assets, owner, targets) = sleeping_pair(
            MapPoint::new(1394.2125, 328.31696),
            MapPoint::new(1417.7587, 185.4791),
        );
        engine.execute_approach_sleeping_enemies(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            targets.map(|id| id.index()).to_vec(),
        );
        let enemy = engine
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("test selected sleeper"));
        assert_eq!(
            enemy.base.primary_target,
            Some(AiEntityHandle::new(targets[1].index()))
        );
        assert_eq!(
            enemy.base.current_substate,
            Substate::AttackingApproachingSleepingEnemy
        );
        engine.place(targets[0], WorldPoint3D::new(1378.0, 252.0, 0.0));
        assert_eq!(engine.select_nearest_battle_target(owner), Some(targets[0]));
    }

    #[test]
    fn sleeping_enemy_selection_keeps_nearest_and_registration_ties() {
        let (mut engine, _, owner, targets) =
            sleeping_pair(MapPoint::new(1417.0, 250.0), MapPoint::new(1500.0, 400.0));
        engine
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("test sleeper order"))
            .list_them = targets.map(|id| id.index()).to_vec();
        assert_eq!(engine.select_nearest_battle_target(owner), Some(targets[0]));
        let first_position = engine.pos_of(targets[0]);
        engine.place(targets[1], first_position);
        assert_eq!(engine.select_nearest_battle_target(owner), Some(targets[0]));
    }
}
