use super::*;

#[test]
fn look_there_broadcast_skips_attacking_chief_and_reacts_on_eligible_member() {
    use crate::ai::{AiState, CrossNpcAction, LookThereContinuation, Position, Substate};
    use crate::element::{Camp, Entity};

    let mut engine = EngineInner::new();
    let source_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let chief_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let member_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    for id in [source_id, chief_id, member_id] {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).unwrap() else {
            panic!("LOOKTHERE broadcast test NPC changed kind")
        };
        soldier.element.active = true;
        soldier.npc.life_points = 100;
        soldier.npc.view_radius = 400;
    }
    {
        let chief = engine
            .get_entity_mut(chief_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap();
        chief.current_state = AiState::Attacking;
        chief.current_substate = Substate::AttackingReactiontime;
    }
    engine
        .get_entity_mut(member_id)
        .and_then(Entity::ai_controller_mut)
        .unwrap()
        .patrol_chief = Some(chief_id);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine
        .get_entity_mut(source_id)
        .and_then(Entity::ai_controller_mut)
        .unwrap()
        .outbox
        .reentrant
        .cross_npc_actions
        .push(CrossNpcAction::BroadcastLookThere {
            caller: source_id.index(),
            position: Position::default(),
            radius: 100,
            continuation: LookThereContinuation::SeekingArrowReactiontime,
        });

    crate::sim_rng::with_seed(0xA013_1090, |sim| {
        engine.process_synchronous_reentrant_actions_for(sim, source_id, &assets);
    });

    let chief = engine
        .get_entity(chief_id)
        .unwrap()
        .ai_controller()
        .unwrap();
    assert_eq!(chief.current_state, AiState::Attacking);
    assert_eq!(chief.current_substate, Substate::AttackingReactiontime);
    let member = engine
        .get_entity(member_id)
        .unwrap()
        .ai_controller()
        .unwrap();
    assert_eq!(member.current_state, AiState::Wondering);
    assert_eq!(member.current_substate, Substate::WonderingWatching);
}

#[test]
fn npc_detection_view_rebinds_combat_data_to_the_queued_target() {
    use crate::ai::{AiState, Decision, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};

    let mut engine = EngineInner::new();
    engine.add_test_entity(Entity::Target(crate::element::ElementTarget {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::Target;
            initial_element
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    let soldier_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let old_target_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let viewed_target_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("target-rebind soldier exists")
    else {
        panic!("target-rebind observer changed kind")
    };
    soldier.element.active = true;
    soldier
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
    soldier.element.set_position_map(MapPoint::new(0.0, 0.0));
    soldier.element.set_direction_instantly(4);
    soldier.npc.life_points = 100;
    soldier.npc.view_direction = [1.0, 0.0];
    soldier.npc.view_radius = 300;
    soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    soldier.npc.eye_status = crate::element::EyeStatus::Stare;

    for (pc_id, x) in [(old_target_id, -200.0), (viewed_target_id, 5.0)] {
        let Entity::Pc(pc) = engine
            .get_entity_mut(pc_id)
            .expect("target-rebind PC exists")
        else {
            panic!("target-rebind target changed kind")
        };
        pc.element.active = true;
        pc.element
            .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
        pc.element.set_position_map(MapPoint::new(x, 0.0));
        pc.pc.life_points = 100;
    }

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("fixture installs the target-rebind PC character profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("target-rebind soldier exists before detection")
    else {
        panic!("target-rebind observer changed kind")
    };
    let ai = soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("target-rebind soldier has enemy AI");
    ai.base.me = soldier_id.index();
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingJustWatching;
    ai.current_task_priority = task_priority::SEEKING;
    ai.base.primary_target = Some(crate::ai::AiEntityHandle::new(old_target_id.index()));
    ai.base.seek_position = crate::ai::Position {
        x: -200.0,
        y: 0.0,
        ..crate::ai::Position::default()
    };
    ai.forced_next_battle_decision = Decision::Fight;

    soldier.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    soldier.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
        element: Some(viewed_target_id),
        detectable_type: DetectableType::Enemy,
        shadow_seen_last_frame: true,
        ..Detectable::default()
    });

    crate::sim_rng::with_seed(0xA013_0B1F, |sim| engine.tick_enemy_ai(sim, &assets));

    let ai = engine
        .get_entity(soldier_id)
        .and_then(Entity::enemy_ai)
        .expect("target-rebind soldier retains enemy AI");
    // Original-game battle decisions do not clear the forced
    // decision after using it. The serialized reset flag is never consulted.
    assert_eq!(
        (
            ai.base.primary_target,
            ai.base.last_stimulus_actor,
            ai.base.current_state,
            ai.base.current_substate,
            ai.forced_next_battle_decision,
        ),
        (
            Some(crate::ai::AiEntityHandle::new(viewed_target_id.index())),
            Some(crate::ai::AiEntityHandle::new(viewed_target_id.index())),
            AiState::Attacking,
            Substate::AttackingSwordfight,
            Decision::Fight,
        )
    );
    assert_eq!(ai.base.current_state, AiState::Attacking);
    assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
}
