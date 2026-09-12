use super::*;

#[test]
fn phalanx_them_list_snapshot_follows_inactive_linked_member() {
    use crate::element::{Camp, Entity};

    let mut engine = EngineInner::new();
    engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let chief = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let linked = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));

    for id in [chief, linked] {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).unwrap() else {
            unreachable!("test phalanx member changed kind")
        };
        soldier.npc.ai_brain.base_mut().unwrap().me = id.index();
    }
    let chief_ai = engine
        .get_entity_mut(chief)
        .and_then(Entity::enemy_ai_mut)
        .unwrap();
    chief_ai.right_combat_neighbour = Some(crate::ai::AiEntityHandle::new(linked.index()));

    let Entity::Soldier(linked_soldier) = engine.get_entity_mut(linked).unwrap() else {
        unreachable!()
    };
    linked_soldier.element.active = false;
    linked_soldier.human.unconscious = true;
    linked_soldier.npc.life_points = 0;

    let snapshots = engine.build_phalanx_member_them_lists(chief);
    assert_eq!(
        snapshots
            .iter()
            .map(|member| member.handle)
            .collect::<Vec<_>>(),
        [chief.index(), linked.index()],
        "Original follows the installed right-neighbour link without active, consciousness, or life guards"
    );
}

#[test]
fn pc_noise_refresh_invalidates_an_earlier_npc_tactical_snapshot() {
    use crate::element::{Camp, Detectable, DetectableType};

    let mut engine = EngineInner::new();
    let earlier_npc = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let pc = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let later_npc = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));

    // The earlier NPC builds the lazy tactical snapshot before the PC's live
    // human-update slot. The later NPC's cadence is open at frame zero:
    // (0 + 31 hidden creations + slot 2) % 3 == 0.
    engine.control.frame_counter = 0;
    for npc_id in [earlier_npc, later_npc] {
        let Entity::Soldier(npc) = engine.get_entity_mut(npc_id).expect("listener exists") else {
            panic!("listener changed kind")
        };
        npc.element.active = true;
        npc.element.set_position_map(MapPoint::new(0.0, 0.0));
        npc.npc.life_points = 100;
        npc.npc
            .ai_brain
            .enemy_mut()
            .expect("listener has enemy AI")
            .base
            .me = npc_id.index();
    }
    let Entity::Pc(pc_entity) = engine.get_entity_mut(pc).expect("noise PC exists") else {
        panic!("noise PC changed kind")
    };
    pc_entity.element.active = true;
    pc_entity.element.set_position_map(MapPoint::new(55.0, 0.0));
    pc_entity.pc.life_points = 100;
    pc_entity.actor.last_noise_volume = 200;
    pc_entity.actor.produced_noise = Some(crate::ai::Noise {
        origin: crate::ai::NoiseOrigin::from_position(crate::ai::Position {
            x: 55.0,
            y: 0.0,
            sector: None,
            level: 0,
        }),
        noise_type: crate::ai::NoiseType::TapTapTap,
        volume: 200,
        elevation: 0,
        element_id: u16::try_from(pc.index()).expect("test PC id fits noise record"),
    });
    pc_entity.actor.hear_noise_box =
        crate::coordinates::MapBBox::from_coords(-245.0, -220.0, 355.0, 220.0);

    let Entity::Soldier(later) = engine.get_entity_mut(later_npc).unwrap() else {
        unreachable!()
    };
    later.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
        element: Some(pc),
        detectable_type: DetectableType::Enemy,
        heard_last_frame: true,
        ..Detectable::default()
    });

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let positions = engine.boundary_positions_snapshot();
    crate::sim_rng::with_seed(0xA013_0016, |sim| {
        engine.tick_actor_owner_envelopes(sim, &assets, &positions)
    });

    let actor = engine
        .get_entity(pc)
        .and_then(Entity::actor_data)
        .expect("noise PC remains an actor");
    assert_eq!(
        actor
            .produced_noise
            .expect("noise remains initialized")
            .volume,
        15
    );
    let later = engine
        .get_entity(later_npc)
        .and_then(Entity::npc_data)
        .expect("later listener remains an NPC");
    assert!(
        !later.detectable_lists[DetectableType::Enemy as usize][0].heard_last_frame,
        "later NPC must observe the PC's creation-ordered quiet refresh"
    );
}

#[test]
fn arrow_reaction_with_null_interesting_object_clears_stale_look_there_focus() {
    use crate::ai::{AiState, CrossNpcAction, Position, StimulusInfo, StimulusType, Substate};
    use crate::element::{Camp, Entity, EyeStatus};

    let mut engine = EngineInner::new();
    let source_id = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));
    let receiver_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    for id in [source_id, receiver_id] {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).unwrap() else {
            panic!("arrow-focus test NPC changed kind")
        };
        soldier.element.active = true;
        soldier.npc.life_points = 100;
    }
    {
        let receiver = engine.get_entity_mut(receiver_id).unwrap();
        receiver.npc_data_mut().unwrap().eye_status = EyeStatus::Stare;
        assert_eq!(receiver.enemy_ai().unwrap().base.interesting_object, None);
    }

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine
        .get_entity_mut(source_id)
        .and_then(Entity::ai_controller_mut)
        .unwrap()
        .outbox
        .reentrant
        .cross_npc_actions
        .push(CrossNpcAction::SendStimulus {
            target: receiver_id.index(),
            stimulus_type: StimulusType::EventGetArrow,
            info: StimulusInfo::Position(Position::default()),
            fallback_to_sender: None,
            to_whole_patrol: false,
        });

    crate::sim_rng::with_seed(0xA013_1091, |sim| {
        engine.process_synchronous_reentrant_actions_for(sim, source_id, &assets);
    });

    let receiver_ai = engine.get_entity(receiver_id).unwrap().enemy_ai().unwrap();
    assert_eq!(receiver_ai.base.current_state, AiState::Seeking);
    assert_eq!(
        receiver_ai.base.current_substate,
        Substate::SeekingArrowReactiontime
    );
    assert_eq!(
        engine
            .get_entity(receiver_id)
            .and_then(Entity::npc_data)
            .unwrap()
            .eye_status,
        EyeStatus::LookForward,
        "clearing focus must unfocus the stale look-there point stare"
    );
}
