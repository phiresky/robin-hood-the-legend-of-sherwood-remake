use super::*;
use crate::engine::TickCtx;

#[test]
fn this_guy_forbid_preserves_original_uword_narrowing_and_ulong_comparison() {
    use std::collections::BTreeMap;

    use crate::ai::{ForbiddenRemark, Remark, RemarkTargetFlags, SpeechFlags};

    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let owner = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: false },
        613,
    );
    let creation_order = u32::from(u16::MAX) + 2;
    engine.world.install_original_creation_orders(
        BTreeMap::from([(owner, creation_order)]),
        creation_order + 1,
    );
    engine.ai.global.forbidden_remarks.push(ForbiddenRemark {
        remark: Remark::Drunken,
        flags: RemarkTargetFlags::THIS_GUY.bits(),
        speech_id: 0,
        guy_index: creation_order as u16,
        bad_guy: true,
        forbidden_till_frame: engine.control.frame_counter,
    });

    queue_and_settle_speech(
        &mut engine,
        &assets,
        owner,
        Remark::Drunken,
        SpeechFlags::empty(),
    );

    assert_ne!(last_speech_impossible(&engine, owner), Some(2));
    let personal = engine
        .ai
        .global
        .forbidden_remarks
        .last()
        .expect("accepted Drunken speech adds its personal forbid");
    assert_eq!(personal.flags, RemarkTargetFlags::THIS_GUY.bits());
    assert_eq!(personal.guy_index, creation_order as u16);
}

#[test]
fn speech_state_roundtrip_and_hash_cover_live_identity_and_global_state() {
    use crate::ai::{ForbiddenRemark, Remark, RemarkTargetFlags, ScreenRemark, SpeechFlags};

    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let first = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: false },
        701,
    );
    let second = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: false },
        702,
    );
    let first_creation_order = engine.world.original_creation_order(first);
    let ai = engine.ai_ctrl_mut(first);
    ai.current_remark = Remark::Arrow;
    ai.current_remark_flags = SpeechFlags::MYTALK_2.bits();
    engine
        .feedback
        .sound_sim
        .playing_exclamations
        .push(crate::sound::PlayingExclamation {
            actor_id: first.index(),
            exclamation_id: Remark::Arrow as u32,
            finish_frame: 77,
        });
    engine.ai.global.current_speech_variant = 2;
    engine.ai.global.screen_remarks.push(ScreenRemark {
        timer: 100,
        prefix: "snapshot".into(),
        remark: Remark::Arrow,
    });
    engine.ai.global.forbidden_remarks.push(ForbiddenRemark {
        remark: Remark::Arrow,
        flags: RemarkTargetFlags::THIS_GUY.bits(),
        speech_id: 701,
        guy_index: first_creation_order as u16,
        bad_guy: true,
        forbidden_till_frame: 88,
    });

    let json = serde_json::to_string(&engine).expect("serialize speech snapshot");
    let restored: EngineInner = serde_json::from_str(&json).expect("deserialize speech snapshot");
    assert_eq!(
        robin_util::state_hash::compute(&restored),
        robin_util::state_hash::compute(&engine)
    );
    assert_eq!(
        serde_json::to_value(&restored).unwrap(),
        serde_json::to_value(&engine).unwrap()
    );

    let mut retargeted = engine.clone();
    retargeted.feedback.sound_sim.playing_exclamations[0].actor_id = second.index();
    assert_ne!(
        robin_util::state_hash::compute(&retargeted),
        robin_util::state_hash::compute(&engine)
    );
}

#[test]
fn live_detectable_mutations_preserve_statement_order_through_snapshot() {
    // Native actor decoding needs more than libtest's 2 MiB thread stack for
    // this complete three-actor snapshot. Keep the adjustment local to this
    // codec matrix rather than changing production or the full test runner.
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(check_detectable_snapshot_and_drain_matrix)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn enter_swordfight_clears_pending_bow_shot_list() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    let pc = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let opponent = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    let assets = engine.test_runtime_assets();

    let mut shot = crate::sequence::SequenceElement::new_interaction(
        1,
        crate::element::Command::ShootBow,
        Some(pc),
        Some(opponent),
    );
    shot.priority = crate::sequence::SequencePriority::Preference;
    let shot_seq = engine.launch_element(TickCtx::new(sim, &assets), shot);
    engine.queue_pc_shoot_bow(pc, crate::sequence::SequenceElementRef::new(shot_seq, 0));
    assert_eq!(engine.human(pc).pending_shoots.len(), 1);
    assert!(engine.pc_has_pending_shoot_bow(pc));

    let _ = engine.enter_swordfight(TickCtx::new(sim, &assets), pc, opponent, false);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(shot_seq, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Todo,
        "clearing the shoot FIFO leaves the registered sequence unchanged"
    );
    assert!(
        engine.human(pc).pending_shoots.is_empty(),
        "swordfight entry clears the retained human shoot FIFO before validity checks"
    );
    assert!(engine.pc_has_pending_shoot_bow(pc));
}

#[test]
fn npc_enter_swordfight_preserves_postponed_bow_sequence() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    let initiator = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    let opponent = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let assets = engine.test_runtime_assets();

    let mut shot = crate::sequence::SequenceElement::new_interaction(
        1,
        crate::element::Command::ShootBow,
        Some(initiator),
        Some(opponent),
    );
    shot.priority = crate::sequence::SequencePriority::Preference;
    let shot_seq = engine.launch_element(TickCtx::new(sim, &assets), shot);
    engine.postpone_element(TickCtx::new(sim, &assets), &mut Vec::new(), shot_seq, 0);

    let (_, stimuli) = crate::engine::soldier_helpers::capture_condolation_stimuli(|| {
        engine.enter_swordfight(TickCtx::new(sim, &assets), initiator, opponent, false)
    });

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(shot_seq, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Postponed,
        "Original ClearShootList removes the retained NPC pointer without interrupting its sequence"
    );
    assert!(
        !stimuli
            .iter()
            .any(|(owner, event)| *owner == initiator
                && *event == crate::ai::StimulusType::EventDone),
        "clearing an NPC shoot pointer must not invent EventDone"
    );
}

#[test]
fn synchronous_one_shot_noise_is_handled_before_broadcast_returns() {
    use crate::ai::{AiState, NoiseType, Substate};
    use crate::coordinates::{MapPoint, WorldPoint3D};
    use crate::element::Camp;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .soldiers
        .push(crate::profiles::SoldierProfile::default());

    let mut listener = make_test_ai_soldier(Camp::Lacklandists);
    let Entity::Soldier(soldier) = &mut listener else {
        unreachable!("make_test_ai_soldier returned non-soldier")
    };
    soldier.element.active = true;
    soldier
        .element
        .set_position(WorldPoint3D::new(10.0, 10.0, 0.0));
    soldier.element.set_position_map(MapPoint::new(10.0, 10.0));
    let listener_id = engine.add_test_entity(listener);
    engine.enemy_mut(listener_id).base.me = listener_id.index();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    engine.broadcast_noise_synchronously(
        TickCtx::new(&sim, &assets),
        NoiseType::Bonk,
        MapPoint::new(20.0, 10.0),
        Some(crate::position_interface::Layer::ZERO),
        crate::parameters_ai::NOISE_VOLUME_BONK as u16,
        0,
        None,
    );

    let listener = engine.enemy(listener_id);
    assert_eq!(listener.base.current_state, AiState::Wondering);
    assert_eq!(listener.base.current_substate, Substate::WonderingWatching);
}

#[test]
fn one_shot_noise_listener_walk_uses_restored_original_creation_order() {
    use crate::element::Camp;

    let mut engine = EngineInner::new();
    let first = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let second = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let first_order = engine.world.original_creation_order(first);
    let second_order = engine.world.original_creation_order(second);
    engine.world.install_original_creation_orders(
        std::collections::BTreeMap::from([(first, second_order), (second, first_order)]),
        second_order + 1,
    );

    assert_eq!(engine.world.npc_registry_ids, vec![second, first]);
}

#[test]
fn one_shot_hearing_defers_listener_state_filtering_but_rejects_its_source_point() {
    use crate::ai::NoiseType;
    use crate::coordinates::{MapPoint, WorldPoint3D};
    use crate::element::Camp;

    let mut engine = EngineInner::new();
    engine.control.frame_counter = 5;
    let mut listener = make_test_ai_soldier(Camp::Lacklandists);
    let Entity::Soldier(soldier) = &mut listener else {
        unreachable!("make_test_ai_soldier returned non-soldier")
    };
    soldier.element.active = false;
    soldier.human.unconscious = true;
    soldier
        .element
        .set_position(WorldPoint3D::new(10.0, 10.0, 0.0));
    soldier.element.set_position_map(MapPoint::new(10.0, 10.0));
    let listener_id = engine.add_test_entity(listener);

    let audible = engine.one_shot_noise(
        NoiseType::Bonk,
        MapPoint::new(20.0, 10.0),
        Some(crate::position_interface::Layer::ZERO),
        crate::parameters_ai::NOISE_VOLUME_BONK as u16,
        0,
        None,
    );
    assert!(
        engine
            .subjective_one_shot_noise_for(listener_id, audible)
            .is_some(),
        "inactive/unconscious state belongs to decision-tick admission, after heard-volume calculation"
    );
    assert_eq!(
        engine
            .npc(listener_id)
            .old_cover_noise_deafness_frame_counter,
        5,
        "heard-volume calculation must refresh deafness before decision-tick admission refuses the event"
    );

    let same_point = engine.one_shot_noise(
        NoiseType::Aaargh,
        MapPoint::new(10.0, 10.0),
        Some(crate::position_interface::Layer::ZERO),
        crate::parameters_ai::NOISE_VOLUME_AAARGH as u16,
        0,
        Some(listener_id),
    );
    assert!(
        engine
            .subjective_one_shot_noise_for(listener_id, same_point)
            .is_none(),
        "the actor at the exact full-3D source point must not hear its own cry"
    );

    engine.control.frame_counter = 6;
    let max_norm_only = engine.one_shot_noise(
        NoiseType::Bonk,
        MapPoint::new(18.0, 14.0),
        Some(crate::position_interface::Layer::ZERO),
        10,
        0,
        None,
    );
    assert!(
        engine
            .subjective_one_shot_noise_for(listener_id, max_norm_only)
            .is_none(),
        "a source inside the max-norm box can still have no positive Euclidean remainder"
    );
    assert_eq!(
        engine
            .npc(listener_id)
            .old_cover_noise_deafness_frame_counter,
        5,
        "heard-volume calculation must not refresh deafness until subjective volume is positive"
    );
}

#[test]
fn one_shot_hearing_uses_authoritative_world_y_at_uword_volume_boundary() {
    use crate::ai::NoiseType;
    use crate::coordinates::{MapPoint, WorldPoint3D};
    use crate::element::Camp;
    use crate::position_interface::INVERSE_ASPECT_RATIO;

    // Find the adjacent f32 world-Y values whose aspect-stretched distances
    // straddle 499. The original game truncates `500 - distance` to 16 bits, so the
    // lower reconstructed value is audible at volume 1 while the upper
    // authoritative value is inaudible at volume 0.
    let boundary = 499.0_f32 / INVERSE_ASPECT_RATIO;
    let mut reconstructed_y = boundary;
    while reconstructed_y * INVERSE_ASPECT_RATIO >= 499.0 {
        reconstructed_y = f32::from_bits(reconstructed_y.to_bits() - 1);
    }
    let mut authoritative_y = boundary;
    while authoritative_y * INVERSE_ASPECT_RATIO <= 499.0 {
        authoritative_y = f32::from_bits(authoritative_y.to_bits() + 1);
    }
    assert_eq!((500.0 - reconstructed_y * INVERSE_ASPECT_RATIO) as u16, 1);
    assert_eq!((500.0 - authoritative_y * INVERSE_ASPECT_RATIO) as u16, 0);

    let mut engine = EngineInner::new();
    let mut listener = make_test_ai_soldier(Camp::Lacklandists);
    let Entity::Soldier(soldier) = &mut listener else {
        unreachable!("make_test_ai_soldier returned non-soldier")
    };
    soldier
        .element
        .set_position(WorldPoint3D::new(0.0, authoritative_y, 0.0));
    // Projection roundoff can make map Y reconstruct to the adjacent lower
    // float even though stored world Y remains authoritative.
    soldier
        .element
        .set_position_map_preserving_3d(MapPoint::new(0.0, reconstructed_y));
    let listener_id = engine.add_test_entity(listener);
    let drawbridge = engine.one_shot_noise(
        NoiseType::Drawbridge,
        MapPoint::new(0.0, 0.0),
        Some(crate::position_interface::Layer::ZERO),
        crate::parameters_ai::NOISE_VOLUME_DRAWBRIDGE as u16,
        0,
        None,
    );

    assert!(
        engine
            .subjective_one_shot_noise_for(listener_id, drawbridge)
            .is_none(),
        "heard-volume calculation must use stored 3D Y; reconstructing map Y would create a spurious volume-1 listener"
    );
}

#[test]
fn live_combat_position_recovers_exact_duplicate_pc_sector() {
    use crate::coordinates::{MapBBox, MapPoint};
    use crate::fast_find_grid::{GridSector, SectorIndex};
    use crate::sector::{SectorNumber, SectorType};

    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let target = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    engine.test_runtime_assets();

    let square = |min: f32, max: f32| GridSector {
        points: vec![
            MapPoint::new(min, min),
            MapPoint::new(max, min),
            MapPoint::new(max, max),
            MapPoint::new(min, max),
        ],
        bounding_box: MapBBox::from_coords(min, min, max, max),
        sector_number: SectorNumber::new(88),
        layer: 2,
        sector_type: SectorType::MOTION,
        ..Default::default()
    };
    engine.world.fast_grid_mut().level_mut().sectors =
        vec![square(0.0, 100.0), square(600.0, 800.0)];

    let Entity::Soldier(owner_entity) = engine.ent_mut(owner) else {
        panic!("fighter owner changed kind")
    };
    owner_entity.element.active = true;
    owner_entity.npc.life_points = 100;
    owner_entity
        .npc
        .ai_brain
        .enemy_mut()
        .expect("fighter owner has Enemy AI")
        .base
        .me = owner.index();

    let target_element = engine.elem_mut(target);
    target_element.active = true;
    target_element.set_position_map(MapPoint::new(684.0, 745.0));
    target_element.set_layer(2);
    target_element.set_sector(crate::position_interface::SectorHandle::new(88));
    assert_eq!(target_element.sector().unwrap().arena_index(), None);

    let target_sector = engine
        .live_ai_position(target)
        .sector
        .expect("live combat target has a sector");
    assert_eq!(u16::from(target_sector), 88);
    assert_eq!(target_sector.arena_index(), SectorIndex::new(1));
}

#[test]
fn bow_interaction_accepts_a_target_that_died_while_aiming() {
    use crate::profiles::{BowProfile, BowShootMode, CharacterProfile, ProfileManager};

    let mut engine = EngineInner::new();
    let shooter = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let target = engine.add_test_entity(make_test_pc(crate::element::Posture::Dead));
    let Entity::Pc(dead_target) = engine.ent_mut(target) else {
        panic!("dead target changed kind")
    };
    dead_target.element.active = true;
    dead_target.pc.life_points = 0;

    let mut profiles = ProfileManager::new();
    profiles.characters.push(CharacterProfile {
        shooting_weapon_id: 1,
        shooting: 100,
        ..CharacterProfile::default()
    });
    profiles.bows.push(BowProfile {
        normal_shoot: BowShootMode {
            range: 2000,
            ..BowShootMode::default()
        },
        ..BowProfile::default()
    });
    let mut assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::new()
    };
    complete_test_runtime_fixture(&mut engine, &mut assets);

    assert!(
        engine
            .shoot_bow_at(
                TickCtx::new(&crate::sim_rng::test_context(), &assets),
                shooter,
                target
            )
            .is_some()
    );
}

#[test]
fn live_combat_position_uses_committed_gate_side_for_door_passing_actor() {
    let assets = LevelAssets::new();
    use crate::coordinates::{MapPoint, WorldPoint3D};
    use crate::gate::{Door, DoorIndex, DoorType};
    use crate::order::OrderType;
    use crate::sector::SectorNumber;
    use crate::sequence::{SequenceElement, SequenceElementData};

    let mut engine = EngineInner::new();
    // Live AI views expose the installed door registry only for a mission.
    engine.scripts.mission = Some(crate::engine::test_support::asm::empty_mission_script(
        "fighter_gate_position_test.scs",
    ));
    let self_id = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let target_id = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Royalists));

    for (id, x) in [(self_id, 0.0), (target_id, 20.0)] {
        let Entity::Soldier(soldier) = engine.ent_mut(id) else {
            panic!("test fighter changed kind")
        };
        soldier.element.active = true;
        soldier.npc.life_points = 100;
        soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("test fighter has enemy AI")
            .base
            .me = id.index();
        soldier.element.set_position(WorldPoint3D::new(x, 0.0, 0.0));
        soldier.element.set_position_map(MapPoint::new(x, 0.0));
    }

    let Entity::Soldier(target) = engine.ent_mut(target_id) else {
        panic!("door-passing target changed kind")
    };
    let exact_target_world = WorldPoint3D::new(20.123_457, 9.876_543, 7.654_321);
    target.element.set_position_map(MapPoint::from_world_xyz(
        exact_target_world.x,
        exact_target_world.y,
        exact_target_world.z,
    ));
    target.element.set_position(exact_target_world);
    let expected_optical_point = crate::stealth::detection_point_world(
        exact_target_world,
        target.element.posture(),
        target.element.direction(),
        target.soldier.rider,
    );

    engine.script_domains.interactables.doors = vec![Door {
        door_type: DoorType::Default,
        sector_out: SectorNumber::new(7),
        sector_in: SectorNumber::new(8),
        sector_out_index: crate::fast_find_grid::SectorIndex::new(7),
        sector_in_index: crate::fast_find_grid::SectorIndex::new(8),
        layer_out: 3,
        layer_in: 4,
        point_out: MapPoint::new(120.0, 5.0),
        point_in: MapPoint::new(100.0, 5.0),
        ..Door::default()
    }];
    let mut pass_door = SequenceElement::new_movement(
        1,
        crate::element::Command::PassDoor,
        Some(target_id),
        OrderType::WalkingWithSword,
    );
    let SequenceElementData::Movement {
        gate_id, direction, ..
    } = &mut pass_door.data
    else {
        panic!("PassDoor test element changed kind")
    };
    *gate_id = Some(DoorIndex::new(0).expect("valid door index"));
    *direction = 0;
    let sequence_id = engine.orders.sequence_manager.insert_element(pass_door);
    engine
        .orders
        .sequence_manager
        .start_sequence_level(sequence_id);
    engine.select_sequence_element(target_id, Some((sequence_id, 0)));
    engine.t_element_in_progress(&assets, sequence_id, 0);

    let assets = engine.test_runtime_assets();

    let (optical_ai_position, optical_point) =
        engine.enemy_optical_geometry_for_test(&assets, target_id);
    assert_eq!(optical_ai_position.x, 120.0);
    assert_eq!(optical_ai_position.y, 5.0);
    assert_eq!(optical_ai_position.level, 3);
    assert_eq!(
        optical_point.x.to_bits(),
        expected_optical_point.x.to_bits()
    );
    assert_eq!(
        optical_point.y.to_bits(),
        expected_optical_point.y.to_bits()
    );
    assert_eq!(
        optical_point.z.to_bits(),
        expected_optical_point.z.to_bits()
    );

    let target = engine.live_ai_position(target_id);
    assert_eq!(target.x, 120.0);
    assert_eq!(target.y, 5.0);
    assert_eq!(
        target.sector,
        crate::position_interface::SectorHandle::new(7)
    );
    assert_eq!(target.level, 3);
}

#[test]
fn reconsider_observation_uses_raw_positions_across_committed_gate_sides() {
    let assets = LevelAssets::new();
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::coordinates::{MapPoint, WorldPoint3D};
    use crate::gate::{Door, DoorIndex, DoorType};
    use crate::order::OrderType;
    use crate::sector::SectorNumber;
    use crate::sequence::{SequenceElement, SequenceElementData};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    // Human handle zero means no entry in original-game AI lists.
    engine.add_test_entity(Entity::Target(crate::element::ElementTarget {
        element: {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::Target;
            initial_element
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    let owner_id = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let raw_near_id =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let raw_far_id =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));

    for (id, x) in [(owner_id, 0.0), (raw_near_id, 20.0), (raw_far_id, 600.0)] {
        let Entity::Soldier(soldier) = engine.ent_mut(id) else {
            panic!("test fighter changed kind")
        };
        soldier.element.active = true;
        soldier.npc.life_points = 100;
        soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("test fighter has enemy AI")
            .base
            .me = id.index();
        let world = WorldPoint3D::new(x, 0.0, 0.0);
        soldier.element.set_position(world);
        soldier
            .element
            .set_position_map(MapPoint::from_world_xyz(world.x, world.y, world.z));
    }
    let frame = engine.control.frame_counter;
    let owner = engine.enemy_mut(owner_id);
    owner.base.current_state = AiState::Attacking;
    owner.base.current_substate = Substate::AttackingObserve;
    owner.base.launch_timer(0, frame);
    // The engine clears the running latch when the due timer is emitted; the
    // launch substate remains as the stale-event guard consumed by Think.
    owner.base.timer_is_running = false;

    engine.script_domains.interactables.doors = vec![
        Door {
            door_type: DoorType::Default,
            sector_out: SectorNumber::new(7),
            sector_in: SectorNumber::new(8),
            sector_out_index: crate::fast_find_grid::SectorIndex::new(7),
            sector_in_index: crate::fast_find_grid::SectorIndex::new(8),
            point_out: MapPoint::new(600.0, 0.0),
            point_in: MapPoint::new(580.0, 0.0),
            ..Door::default()
        },
        Door {
            door_type: DoorType::Default,
            sector_out: SectorNumber::new(9),
            sector_in: SectorNumber::new(10),
            sector_out_index: crate::fast_find_grid::SectorIndex::new(9),
            sector_in_index: crate::fast_find_grid::SectorIndex::new(10),
            point_out: MapPoint::new(20.0, 0.0),
            point_in: MapPoint::new(40.0, 0.0),
            ..Door::default()
        },
    ];
    engine.scripts.mission = Some(
        crate::engine::MissionScript::from_scb(crate::scb::ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![crate::engine::test_support::asm::empty_startup_class(
                "reconsider_observation_pass_door_test.scs".into(),
            )],
        })
        .expect("minimal mission exposes the installed test doors"),
    );
    for (id, door_index) in [(raw_near_id, 0), (raw_far_id, 1)] {
        let mut pass_door = SequenceElement::new_movement(
            1,
            crate::element::Command::PassDoor,
            Some(id),
            OrderType::WalkingWithSword,
        );
        let SequenceElementData::Movement {
            gate_id, direction, ..
        } = &mut pass_door.data
        else {
            panic!("PassDoor test element changed kind")
        };
        *gate_id = Some(DoorIndex::new(door_index).expect("valid door index"));
        *direction = 0;
        let sequence_id = engine.orders.sequence_manager.insert_element(pass_door);
        engine
            .orders
            .sequence_manager
            .start_sequence_level(sequence_id);
        engine.select_sequence_element(id, Some((sequence_id, 0)));
        engine.t_element_in_progress(&assets, sequence_id, 0);
    }

    let assets = engine.test_runtime_assets();
    assert_eq!(engine.live_ai_position(raw_near_id).x, 600.0);
    assert_eq!(engine.live_ai_position(raw_far_id).x, 20.0);

    engine.dispatch_think_with_drain(
        TickCtx::new(&sim, &assets),
        owner_id,
        &Stimulus::new(StimulusType::EventTimer),
    );

    let owner = engine.enemy(owner_id);
    assert_eq!(
        owner.base.list_us,
        vec![owner_id.index(), raw_near_id.index()]
    );
}

#[test]
fn closure_review_alert_soldiers_keeps_inactive_soldier_in_live_camp_scan() {
    use crate::ai::Position;

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
    let Entity::Soldier(soldier) = engine.ent_mut(soldier_id) else {
        panic!("inactive help recipient changed kind")
    };
    soldier.element.active = false;

    assert!(
        engine
            .ai_ctx(&sim, &assets, officer_id)
            .execute_ai_alert_soldiers(Position::default(), 0)
    );
    assert_eq!(
        engine
            .world
            .entities
            .expect_enemy_ai(officer_id, format_args!("inactive-help caller"))
            .alerted_us,
        vec![soldier_id.index()]
    );
}
