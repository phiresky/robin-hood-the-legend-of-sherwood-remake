//! Bow callbacks executed against live actor state.

use super::*;
use crate::ai::{AiState, EmoticonType, Substate};
use crate::sim_rng::SimulationContext;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::AiEntityHandle;
    use crate::engine::test_support::actors::{make_test_ai_soldier, make_test_pc};

    fn beggar_fixture(civilian: bool) -> (EngineInner, LevelAssets, EntityId, EntityId) {
        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(128, 128);
        engine.world.fast_grid_mut().allocate_layers(1);
        let index = engine.world.fast_grid_mut().add_sector(
            crate::engine::test_support::square_sector(
                1,
                0,
                crate::coordinates::MapPoint::new(0.0, 0.0),
                crate::coordinates::MapPoint::new(2000.0, 2000.0),
            ),
            0,
        );
        let sector = crate::ai::SectorHandle::new(1)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(index).unwrap());
        let owner = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
        let beggar = engine.add_test_entity(if civilian {
            crate::engine::test_support::actors::make_test_civilian(
                crate::element::Posture::Upright,
            )
        } else {
            make_test_pc(crate::element::Posture::SimulatingBeggar)
        });
        for (id, x) in [(owner, 100.0), (beggar, 140.0)] {
            engine
                .get_entity_mut(id)
                .unwrap()
                .element_data_mut()
                .set_position(crate::coordinates::WorldPoint3D::new(x, 100.0, 0.0));
            engine
                .get_entity_mut(id)
                .unwrap()
                .element_data_mut()
                .set_sector(Some(sector));
        }
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        std::sync::Arc::make_mut(&mut assets.profile_manager)
            .civilians
            .push(crate::profiles::CivilianProfile::default());
        engine.scripts.mission = Some(crate::engine::test_support::asm::empty_mission_script(
            "beggar_test.scs",
        ));
        let ai = engine
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("beggar fixture"));
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingSeekpointIdentifyingBeggar1;
        ai.beggar_to_examine = Some(AiEntityHandle::new(beggar.index()));
        engine.enter_ai_think_frame(owner);
        (engine, assets, owner, beggar)
    }

    #[test]
    fn live_beggar_arrival_registers_one_turn_then_response_sequence() {
        for (archer, command, timer) in [
            (false, crate::element::Command::StartMenace, 30),
            (true, crate::element::Command::EquipBow, 100),
        ] {
            let (mut engine, assets, owner, _) = beggar_fixture(false);
            let ai = engine
                .world
                .entities
                .expect_enemy_ai_mut(owner, format_args!("beggar arrival setup"));
            ai.base.current_substate = Substate::SeekingSeekpointApproachingBeggar;
            ai.is_archer_unit = archer;
            assert!(engine.execute_ai_archery_expected_event(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                StimulusType::EventReachPoint
            ));
            let sequence = engine
                .orders
                .sequence_manager
                .sequences_iter()
                .find(|sequence| {
                    sequence.elements.len() == 2
                        && sequence.elements[0].command == crate::element::Command::TurnFast
                })
                .expect("beggar response sequence");
            assert_eq!(sequence.elements[0].command_level, 1);
            assert_eq!(sequence.elements[1].command_level, 2);
            assert_eq!(sequence.elements[1].command, command);
            assert_eq!(
                engine
                    .world
                    .entities
                    .expect_ai_controller(owner, format_args!("beggar response timer"))
                    .when_does_timer_ring,
                timer
            );
        }
    }

    #[test]
    fn live_npc_beggar_identification_launches_show_face_and_waits() {
        let (mut engine, assets, owner, beggar) = beggar_fixture(true);
        assert!(engine.execute_ai_archery_expected_event(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            StimulusType::EventTimer
        ));
        assert!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .any(|sequence| sequence
                    .elements
                    .iter()
                    .any(|element| element.owner == Some(beggar)
                        && element.command == crate::element::Command::BeggarShowFace))
        );
        let ai = engine
            .world
            .entities
            .expect_ai_controller(owner, format_args!("identified beggar result"));
        assert_eq!(
            ai.current_substate,
            Substate::SeekingSeekpointIdentifyingBeggar2
        );
        assert_eq!(ai.when_does_timer_ring, 50);
    }

    #[test]
    fn live_false_beggar_archer_identification_selects_target_and_registers_departure() {
        let (mut engine, mut assets, owner, beggar) = beggar_fixture(false);
        let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
        if profiles.bows.is_empty() {
            profiles.bows.push(crate::profiles::BowProfile::default());
        }
        profiles.bows[0].normal_shoot.range = 1000;
        for profile in &mut profiles.soldiers {
            profile.shooting_weapon_id = 1;
        }
        engine
            .world
            .entities
            .expect_ai_actor_data_mut(owner, format_args!("false beggar arrows"))
            .number_of_arrows = 10;
        engine
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("false beggar archer"))
            .is_archer_unit = true;
        assert!(engine.execute_ai_archery_expected_event(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            StimulusType::EventTimer
        ));
        let ai = engine
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("false beggar result"));
        assert_eq!(
            ai.base.primary_target,
            Some(AiEntityHandle::new(beggar.index()))
        );
        assert_eq!(ai.list_them, vec![beggar.index()]);
        assert_eq!(ai.base.current_substate, Substate::AttackingBowShooting);
        assert!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .any(|sequence| sequence
                    .elements
                    .iter()
                    .any(|element| element.owner == Some(beggar)
                        && element.command == crate::element::Command::LeaveBeggar))
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .any(|sequence| sequence
                    .elements
                    .iter()
                    .any(|element| element.owner == Some(owner)
                        && element.command == crate::element::Command::ShootBow))
        );
    }

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
            (Substate::SeekingSeekpointApproachingBeggar, StimulusType::EventReachPoint) => {
                let beggar = self.live_beggar_to_examine(owner);
                let there = self
                    .expect_entity(beggar, "beggar arrival distance")
                    .element_data()
                    .position();
                let here = self
                    .expect_entity(owner, "beggar inspector distance")
                    .element_data()
                    .position();
                let distance = (there.x - here.x)
                    .abs()
                    .max(
                        ((there.y - here.y) * crate::position_interface::INVERSE_ASPECT_RATIO)
                            .abs(),
                    )
                    .max((there.z - here.z).abs());
                if distance >= 100.0 {
                    self.execute_ai_seek_next_point(sim, assets, owner);
                } else {
                    self.stop_ai_owner(sim, assets, owner);
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Seeking,
                        Substate::SeekingSeekpointIdentifyingBeggar1,
                    );
                    self.execute_ai_speech(
                        sim,
                        assets,
                        owner,
                        crate::ai::AiSpeechAttempt {
                            remark: crate::ai::Remark::ControlsBeggar,
                            flags: 0,
                        },
                    );
                    let beggar = self.live_beggar_to_examine(owner);
                    let there = self
                        .expect_entity(beggar, "beggar inspection turn")
                        .element_data()
                        .position();
                    let here = self
                        .expect_entity(owner, "beggar inspector turn")
                        .element_data()
                        .position();
                    let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
                        there.x - here.x,
                        there.y - here.y,
                    );
                    use crate::sequence::{Field, FieldValue, Sequence, SequenceElement};
                    let mut sequence = Sequence::new();
                    let mut turn = SequenceElement::new_generic(
                        1,
                        crate::element::Command::TurnFast,
                        Some(owner),
                    );
                    turn.set_property(Field::Direction, FieldValue::Integer(direction as u32));
                    sequence.append_element(turn);
                    let archer = self
                        .world
                        .entities
                        .expect_enemy_ai(owner, format_args!("beggar inspector weapon"))
                        .is_archer();
                    sequence.append_element(SequenceElement::new(
                        2,
                        if archer {
                            crate::element::Command::EquipBow
                        } else {
                            crate::element::Command::StartMenace
                        },
                        Some(owner),
                    ));
                    let time = if archer {
                        if matches!(beggar, EntityId::Civilian(_) | EntityId::Soldier(_)) {
                            50
                        } else {
                            100
                        }
                    } else {
                        30
                    };
                    let frame = self.control.frame_counter;
                    self.world
                        .entities
                        .expect_ai_controller_mut(owner, format_args!("beggar inspection timer"))
                        .launch_timer(time, frame);
                    self.launch_sequence(sim, assets, sequence);
                }
            }
            (Substate::SeekingSeekpointIdentifyingBeggar1, StimulusType::EventTimer) => {
                let beggar = self.live_beggar_to_examine(owner);
                if matches!(beggar, EntityId::Civilian(_) | EntityId::Soldier(_)) {
                    self.launch_element(
                        sim,
                        assets,
                        crate::sequence::SequenceElement::new(
                            1,
                            crate::element::Command::BeggarShowFace,
                            Some(beggar),
                        ),
                    );
                    let beggar = self.live_beggar_to_examine(owner);
                    self.execute_ai_speech(
                        sim,
                        assets,
                        beggar,
                        crate::ai::AiSpeechAttempt {
                            remark: crate::ai::Remark::CivBeggarIdentifiesHimself,
                            flags: 0,
                        },
                    );
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Seeking,
                        Substate::SeekingSeekpointIdentifyingBeggar2,
                    );
                    let frame = self.control.frame_counter;
                    self.world
                        .entities
                        .expect_ai_controller_mut(owner, format_args!("identified beggar timer"))
                        .launch_timer(50, frame);
                } else {
                    let ai = self
                        .world
                        .entities
                        .expect_enemy_ai_mut(owner, format_args!("false beggar target"));
                    ai.base.primary_target = ai.beggar_to_examine;
                    ai.list_them.clear();
                    ai.list_them.push(beggar.index());
                    if ai.is_archer() {
                        self.launch_element(
                            sim,
                            assets,
                            crate::sequence::SequenceElement::new(
                                1,
                                crate::element::Command::LeaveBeggar,
                                Some(beggar),
                            ),
                        );
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
                            .expect_ai_controller(owner, format_args!("false beggar shot target"))
                            .primary_target
                            .expect("false beggar shot requires target");
                        let target = self.expect_human_id_for_ai_handle(
                            target.get(),
                            "false beggar shot target",
                        );
                        self.stop_ai_owner(sim, assets, owner);
                        self.shoot_bow_at(sim, assets, owner, target);
                    } else {
                        self.execute_ai_begin_swordfight(sim, assets, owner);
                    }
                }
            }
            (Substate::SeekingSeekpointIdentifyingBeggar2, StimulusType::EventTimer) => {
                self.execute_ai_seek_next_point(sim, assets, owner);
            }
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
                    self.stop_ai_owner(sim, assets, owner);
                    self.shoot_bow_at(sim, assets, owner, target);
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
                    self.stop_ai_owner(sim, assets, owner);
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
                let target = ai.primary_target;
                self.execute_ai_focus(owner, target);
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

    fn live_beggar_to_examine(&self, owner: EntityId) -> EntityId {
        let handle = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("beggar inspection target"))
            .beggar_to_examine
            .expect("beggar inspection requires a target");
        self.expect_human_id_for_ai_handle(handle.get(), "beggar inspection target")
    }
}
