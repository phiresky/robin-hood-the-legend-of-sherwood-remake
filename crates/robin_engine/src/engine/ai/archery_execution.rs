//! Bow callbacks executed against live actor state.

use super::*;
use crate::ai::{AiState, EmoticonType, Substate};
use crate::sim_rng::SimulationContext;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::AiEntityHandle;
    use crate::engine::test_support::actors::{make_test_ai_soldier, make_test_pc};

    #[test]
    fn proximity_uses_current_target_position_and_live_shield_link() {
        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        let target = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
        let assets = LevelAssets::new();
        for (id, x) in [(owner, 100.0), (target, 1000.0)] {
            engine
                .get_entity_mut(id)
                .unwrap()
                .element_data_mut()
                .set_position(crate::coordinates::WorldPoint3D::new(x, 100.0, 0.0));
        }
        let position = engine.live_ai_position(owner);
        assert!(!engine.ai_archer_is_too_near_to_enemy(&assets, owner, position, target));
        engine
            .get_entity_mut(target)
            .unwrap()
            .element_data_mut()
            .set_position(crate::coordinates::WorldPoint3D::new(125.0, 100.0, 0.0));
        assert!(engine.ai_archer_is_too_near_to_enemy(&assets, owner, position, target));
        engine
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("proximity shield link"))
            .shield_bearer_before_me = Some(AiEntityHandle::new(target.index()));
        assert!(!engine.ai_archer_is_too_near_to_enemy(&assets, owner, position, target));
    }

    #[test]
    fn cover_arrival_completion_focuses_current_target_and_starts_five_frame_timer() {
        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        let target = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
        engine.control.frame_counter = 123;
        let ai = engine
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("cover test setup"));
        ai.current_state = AiState::Attacking;
        ai.current_substate = Substate::AttackingBowRunningBehindShieldBearer;
        ai.primary_target = Some(AiEntityHandle::new(target.index()));
        assert!(engine.execute_ai_archery_expected_event(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            owner,
            StimulusType::EventDone
        ));
        let ai = engine
            .world
            .entities
            .expect_ai_controller(owner, format_args!("cover test timer"));
        assert_eq!(ai.when_does_timer_ring, 128);
        assert!(ai.timer_is_running);
        assert_eq!(
            ai.substate_at_last_timer_launch,
            Substate::AttackingBowRunningBehindShieldBearer
        );
    }
}

impl EngineInner {
    pub(in crate::engine) fn execute_ai_archery_expected_event(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        event: StimulusType,
    ) -> bool {
        let substate = self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("bow callback"))
            .current_substate;
        match (substate, event) {
            (Substate::AttackingBowObservingLoading, StimulusType::EventDone) => {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    Substate::AttackingBowObserving,
                );
                let frame = self.control.frame_counter;
                self.world
                    .entities
                    .expect_ai_controller_mut(owner, format_args!("bow observation timer"))
                    .launch_timer(50, frame);
            }
            (Substate::AttackingBowLoading, StimulusType::EventDone) => {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    Substate::AttackingBowAiming,
                );
                let (_, ability) = self
                    .bow_profile_and_ability(assets, owner)
                    .expect("loaded bow ability");
                let time = ((110 - i32::from(ability as u16)) / 2) as u32;
                let frame = self.control.frame_counter;
                self.world
                    .entities
                    .expect_ai_controller_mut(owner, format_args!("bow aiming timer"))
                    .launch_timer(time, frame);
            }
            (Substate::AttackingBowAiming, StimulusType::EventTimer) => {
                self.world
                    .entities
                    .expect_ai_controller_mut(owner, format_args!("bow emoticon"))
                    .set_emoticon(EmoticonType::None);
                self.drain_direct_ai_owner_boundary(sim, owner, assets);
                let tower = self
                    .world
                    .entities
                    .expect_enemy_ai(owner, format_args!("bow tower guard"))
                    .tower_guard;
                let safe = tower || {
                    let target = self
                        .world
                        .entities
                        .expect_ai_controller(owner, format_args!("bow proximity target"))
                        .primary_target
                        .expect("aiming requires target");
                    let target =
                        self.expect_human_id_for_ai_handle(target.get(), "bow proximity target");
                    !self.ai_archer_is_too_near_to_enemy(
                        assets,
                        owner,
                        self.live_ai_position(owner),
                        target,
                    )
                };
                if safe {
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Attacking,
                        Substate::AttackingBowShooting,
                    );
                    let target = self
                        .world
                        .entities
                        .expect_ai_controller(owner, format_args!("aimed shot target"))
                        .primary_target
                        .expect("shooting requires target");
                    let target =
                        self.expect_human_id_for_ai_handle(target.get(), "aimed shot target");
                    self.world
                        .entities
                        .expect_ai_controller_mut(owner, format_args!("aimed shot stop"))
                        .stop_all();
                    self.drain_direct_ai_owner_boundary(sim, owner, assets);
                    self.shoot_bow_at(assets, owner, target);
                } else {
                    self.world
                        .entities
                        .expect_enemy_ai_mut(owner, format_args!("unsafe aimed shot"))
                        .enemy_seen_below = false;
                    self.execute_battle_decisions(sim, assets, owner);
                }
            }
            (Substate::AttackingBowShooting, StimulusType::EventDone)
            | (Substate::AttackingBowRunningBehindShieldBearer, StimulusType::EventTimer) => {
                self.reinitialize_live_ai_enemies(owner);
                self.execute_battle_decisions(sim, assets, owner);
            }
            (Substate::AttackingBowShooting, StimulusType::CallCoordinate)
            | (Substate::AttackingBowObserving, StimulusType::EventTimer) => {
                if substate == Substate::AttackingBowObserving
                    && self
                        .expect_entity(owner, "bow observer posture")
                        .element_data()
                        .posture()
                        == crate::element::Posture::LeaningOut
                {
                    self.reinitialize_live_ai_enemies(owner);
                } else {
                    self.world
                        .entities
                        .expect_ai_controller_mut(owner, format_args!("bow callback stop"))
                        .stop_all();
                    self.drain_direct_ai_owner_boundary(sim, owner, assets);
                }
                self.execute_battle_decisions(sim, assets, owner);
            }
            (Substate::AttackingBowRunningBehindShieldBearer, StimulusType::EventReachPoint) => {
                if let Some(target) = self
                    .world
                    .entities
                    .expect_ai_controller(owner, format_args!("bow cover facing"))
                    .primary_target
                {
                    let target =
                        self.expect_human_id_for_ai_handle(target.get(), "bow cover facing");
                    let position = self.live_ai_position(target);
                    let elevation = self
                        .expect_entity(target, "bow cover elevation")
                        .position_iface()
                        .get_elevation() as u16;
                    self.duty_face_position_at_elevation(
                        sim,
                        assets,
                        owner,
                        position,
                        f32::from(elevation),
                    );
                }
            }
            (Substate::AttackingBowRunningBehindShieldBearer, StimulusType::EventDone) => {
                let frame = self.control.frame_counter;
                let ai = self
                    .world
                    .entities
                    .expect_ai_controller_mut(owner, format_args!("bow cover focus"));
                ai.launch_timer(5, frame);
                ai.outbox.actor.set_focus(ai.primary_target);
                self.drain_direct_ai_owner_boundary(sim, owner, assets);
            }
            (Substate::AttackingArcherRunOnShootingPath, StimulusType::EventReachPoint) => loop {
                let ai = self
                    .world
                    .entities
                    .expect_enemy_ai_mut(owner, format_args!("archery path cursor"));
                ai.my_archery_point_index = crate::sector::ArcheryPointIdx(
                    u16::from(ai.my_archery_point_index)
                        .wrapping_add_signed(i16::from(ai.my_archery_point_increment)),
                );
                let sector = ai.my_archery_sector.expect("archery path sector");
                let index = u16::from(ai.my_archery_point_index);
                let point = self.ai.global.archery_sectors[usize::from(sector)]
                    .points
                    .get(usize::from(index))
                    .expect("archery path ended without a destination");
                if !point.is_shooting_point {
                    let position = point.position;
                    self.duty_go_to(
                        sim,
                        assets,
                        owner,
                        position,
                        crate::ai::GotoFlags::RUN | crate::ai::GotoFlags::DONT_STOP,
                    );
                    break;
                }
                if point.owner.is_none() || point.owner == Some(owner) {
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Attacking,
                        Substate::AttackingArcherRunOnShootingPathFinalSprint,
                    );
                    self.world
                        .entities
                        .expect_enemy_ai_mut(owner, format_args!("archery path reservation"))
                        .set_my_shooting_point(&mut self.ai.global, Some((sector, index)));
                    let position = self.ai.global.archery_sectors[usize::from(sector)].points
                        [usize::from(index)]
                    .position;
                    self.duty_go_to(sim, assets, owner, position, crate::ai::GotoFlags::RUN);
                    break;
                }
            },
            (
                Substate::AttackingArcherRunOnShootingPathFinalSprint,
                StimulusType::EventReachPoint,
            ) => {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    Substate::AttackingArcherRunOnShootingPathTurn,
                );
                let (sector, index) = self
                    .world
                    .entities
                    .expect_enemy_ai(owner, format_args!("archery path facing"))
                    .my_shooting_point
                    .expect("archery path reserved point");
                let direction = self.ai.global.archery_sectors[usize::from(sector)].points
                    [usize::from(index)]
                .direction;
                self.duty_face_direction(sim, assets, owner, direction);
            }
            (Substate::AttackingArcherRunOnShootingPathTurn, StimulusType::EventDone) => {
                let elevation = self
                    .expect_entity(owner, "archery path elevation")
                    .position_iface()
                    .get_elevation();
                let target_elevation = self
                    .world
                    .entities
                    .expect_enemy_ai(owner, format_args!("archery path target elevation"))
                    .enemy_had_this_elevation;
                if elevation >= f32::from(target_elevation) + 50.0 {
                    self.world
                        .entities
                        .expect_enemy_ai_mut(owner, format_args!("archery path target below"))
                        .enemy_seen_below = true;
                    self.execute_battle_decisions(sim, assets, owner);
                } else {
                    self.execute_ai_get_battle_overview(sim, assets, owner, 0);
                }
            }
            _ => return false,
        }
        true
    }
}
