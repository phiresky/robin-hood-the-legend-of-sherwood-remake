use super::*;

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
fn speech_snapshot_roundtrip_and_hash_cover_fifo_live_identity_and_global_state() {
    use crate::ai::{
        AiOwnerWork, AiSpeechAttempt, ForbiddenRemark, Remark, RemarkTargetFlags, ScreenRemark,
        SpeechFlags,
    };

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
    let ai = engine
        .get_entity_mut(first)
        .unwrap()
        .ai_controller_mut()
        .unwrap();
    ai.current_remark = Remark::Arrow;
    ai.current_remark_flags = SpeechFlags::MYTALK_2.bits();
    ai.outbox.reentrant.owner_work = vec![
        AiOwnerWork::Speech(AiSpeechAttempt {
            remark: Remark::Arrow,
            flags: 0,
        }),
        AiOwnerWork::Speech(AiSpeechAttempt {
            remark: Remark::WaspSting,
            flags: SpeechFlags::ALWAYS.bits(),
        }),
    ];
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

    let mut reordered = engine.clone();
    reordered
        .get_entity_mut(first)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .outbox
        .reentrant
        .owner_work
        .reverse();
    assert_ne!(
        robin_util::state_hash::compute(&reordered),
        robin_util::state_hash::compute(&engine)
    );

    let mut retargeted = engine.clone();
    retargeted.feedback.sound_sim.playing_exclamations[0].actor_id = second.index();
    assert_ne!(
        robin_util::state_hash::compute(&retargeted),
        robin_util::state_hash::compute(&engine)
    );
}

#[test]
fn specialized_ai_continuation_snapshot_roundtrip_and_hash_cover_pending_barrier() {
    use crate::ai::{
        AlertContinuation, AlertSoldiersFailureContinuation, CrossNpcAction, Position,
        StimulusInfo, StimulusType, ThinkResultContinuation,
    };

    let mut engine = EngineInner::new();
    let caller = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let target = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    engine
        .get_entity_mut(caller)
        .and_then(Entity::ai_controller_mut)
        .expect("snapshot caller has AI")
        .outbox
        .reentrant
        .cross_npc_actions
        .extend([
            CrossNpcAction::RequestAlert {
                target: target.index(),
                caller: caller.index(),
                continuation: AlertContinuation::SoldierSawOfficer,
            },
            CrossNpcAction::RequestThinkResult {
                target: target.index(),
                caller: caller.index(),
                stimulus_type: StimulusType::CallAlert,
                info: StimulusInfo::Human(crate::ai::AiEntityHandle::new(target.index())),
                continuation: ThinkResultContinuation::OfficerAlertedSoldier {
                    last: true,
                    use_formation: true,
                    failure: AlertSoldiersFailureContinuation::SeekBody {
                        center: Position {
                            x: 8.0,
                            y: 16.0,
                            ..Default::default()
                        },
                        radius: 160,
                    },
                },
            },
            CrossNpcAction::FinalizeAlertSoldiers {
                caller: caller.index(),
                use_formation: true,
                failure: AlertSoldiersFailureContinuation::SeekBody {
                    center: Position {
                        x: target.index() as f32 + 12.5,
                        y: -7.0,
                        ..Default::default()
                    },
                    radius: 320,
                },
            },
        ]);

    let json = serde_json::to_string(&engine).expect("serialize AI continuation snapshot");
    let restored: EngineInner =
        serde_json::from_str(&json).expect("deserialize AI continuation snapshot");
    assert_eq!(
        serde_json::to_value(&restored).unwrap(),
        serde_json::to_value(&engine).unwrap()
    );
    assert_eq!(
        robin_util::state_hash::compute(&restored),
        robin_util::state_hash::compute(&engine)
    );

    let mut changed_continuation_payload = engine.clone();
    let actions = &mut changed_continuation_payload
        .get_entity_mut(caller)
        .and_then(Entity::ai_controller_mut)
        .expect("snapshot caller retains AI")
        .outbox
        .reentrant
        .cross_npc_actions;
    let CrossNpcAction::RequestThinkResult {
        continuation: ThinkResultContinuation::OfficerAlertedSoldier { last, .. },
        ..
    } = &mut actions[1]
    else {
        panic!("snapshot test lost its result-bearing continuation")
    };
    *last = false;
    assert_ne!(
        robin_util::state_hash::compute(&changed_continuation_payload),
        robin_util::state_hash::compute(&engine),
        "the nested continuation payload must participate in the deterministic hash"
    );
}

#[test]
fn detectable_mutations_preserve_statement_order_through_snapshot_and_drain() {
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
    let mut assets = LevelAssets::new();
    let pc = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let opponent = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let mut shot = crate::sequence::SequenceElement::new_interaction(
        1,
        crate::element::Command::ShootBow,
        Some(pc),
        Some(opponent),
    );
    shot.priority = crate::sequence::SequencePriority::Preference;
    let shot_seq = engine.orders.sequence_manager.launch_element(shot);
    assert!(engine.pc_has_pending_shoot_bow(pc));

    let _ = engine.enter_swordfight(sim, &assets, pc, opponent, false);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(shot_seq, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Interrupted
    );
    assert!(
        !engine.pc_has_pending_shoot_bow(pc),
        "swordfight entry clears the actor's pending shoot list before validity checks"
    );
}

#[test]
fn npc_enter_swordfight_preserves_postponed_bow_sequence() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let initiator = engine.add_entity(make_test_soldier(crate::element::Posture::Upright));
    let opponent = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let mut shot = crate::sequence::SequenceElement::new_interaction(
        1,
        crate::element::Command::ShootBow,
        Some(initiator),
        Some(opponent),
    );
    shot.priority = crate::sequence::SequencePriority::Preference;
    let shot_seq = engine.orders.sequence_manager.launch_element(shot);
    engine.orders.sequence_manager.postpone_element(shot_seq, 0);

    let _ = engine.enter_swordfight(sim, &assets, initiator, opponent, false);

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
        engine
            .orders
            .sequence_manager
            .drain_pending_condolations()
            .is_empty(),
        "clearing an NPC shoot pointer must not invent EventDone"
    );
}

#[test]
fn synchronous_one_shot_noise_is_handled_before_broadcast_returns() {
    use crate::ai::{AiState, NoiseType, Stimulus, StimulusType, Substate};
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
    let listener_id = engine.add_entity(listener);
    engine
        .get_entity_mut(listener_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("test listener has enemy AI")
        .base
        .me = listener_id.index();
    engine
        .get_entity_mut(listener_id)
        .and_then(Entity::ai_controller_mut)
        .expect("test listener has base AI")
        .outbox
        .detection
        .stimuli
        .push(Stimulus::new(StimulusType::EventTimer));
    complete_test_runtime_fixture(&mut engine, &mut assets);

    engine.broadcast_noise_synchronously(
        &sim,
        &assets,
        NoiseType::Bonk,
        MapPoint::new(20.0, 10.0),
        Some(crate::position_interface::Layer::ZERO),
        crate::parameters_ai::NOISE_VOLUME_BONK as u16,
        0,
        None,
    );

    let listener = engine
        .get_entity(listener_id)
        .and_then(Entity::enemy_ai)
        .expect("test listener survives synchronous noise");
    assert_eq!(
        listener
            .base
            .outbox
            .detection
            .stimuli
            .iter()
            .map(|stimulus| stimulus.stimulus_type)
            .collect::<Vec<_>>(),
        vec![StimulusType::EventTimer],
        "direct EVENT_HEAR must not consume an unrelated deferred FIFO"
    );
    assert_eq!(listener.base.current_state, AiState::Wondering);
    assert_eq!(listener.base.current_substate, Substate::WonderingWatching);
}

#[test]
fn one_shot_noise_listener_walk_uses_restored_original_creation_order() {
    use crate::element::Camp;

    let mut engine = EngineInner::new();
    let first = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let second = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let first_order = engine.world.original_creation_order(first);
    let second_order = engine.world.original_creation_order(second);
    engine.world.install_original_creation_orders(
        std::collections::BTreeMap::from([(first, second_order), (second, first_order)]),
        second_order + 1,
    );

    assert_eq!(engine.one_shot_noise_listener_ids(), vec![second, first]);
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
    let listener_id = engine.add_entity(listener);

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
            .get_entity(listener_id)
            .and_then(Entity::npc_data)
            .expect("listener keeps NPC state")
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
            .get_entity(listener_id)
            .and_then(Entity::npc_data)
            .expect("listener keeps NPC state")
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
    let listener_id = engine.add_entity(listener);
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
fn fighter_snapshot_recovers_exact_duplicate_pc_sector_for_combat_routes() {
    use crate::coordinates::{MapBBox, MapPoint};
    use crate::fast_find_grid::{GridSector, SectorIndex};
    use crate::sector::{SectorNumber, SectorType};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let target = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

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
        door_index: None,
        lift_type: None,
        lift_direction: 0,
        force_crouched: false,
        building_index: None,
        low_exit_point: None,
        high_exit_point: None,
        lowest_door_index: None,
        jump_line_indices: Vec::new(),
        gate_indices: Vec::new(),
        underlying_sector: None,
    };
    engine.world.fast_grid_mut().level_mut().sectors =
        vec![square(0.0, 100.0), square(600.0, 800.0)];

    let Entity::Soldier(owner_entity) = engine.get_entity_mut(owner).unwrap() else {
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

    let target_element = engine.get_entity_mut(target).unwrap().element_data_mut();
    target_element.active = true;
    target_element.set_position_map(MapPoint::new(684.0, 745.0));
    target_element.set_layer(2);
    target_element.set_sector(crate::position_interface::SectorHandle::new(88));
    assert_eq!(target_element.sector().unwrap().arena_index(), None);

    let registry = engine.build_full_fighter_registry_for_test(owner, &assets);
    let target_sector = registry
        .iter()
        .find(|fighter| fighter.handle == target.index())
        .and_then(|fighter| fighter.position.sector)
        .expect("target fighter snapshot has a sector");
    assert_eq!(u16::from(target_sector), 88);
    assert_eq!(target_sector.arena_index(), SectorIndex::new(1));
}

#[test]
fn bow_interaction_accepts_a_target_that_died_while_aiming() {
    use crate::profiles::{BowProfile, BowShootMode, CharacterProfile, ProfileManager};

    let mut engine = EngineInner::new();
    let shooter = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let target = engine.add_entity(make_test_pc(crate::element::Posture::Dead));
    let Entity::Pc(dead_target) = engine.get_entity_mut(target).expect("dead target exists") else {
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

    assert!(engine.shoot_bow_at(&assets, shooter, target).is_some());
}

#[test]
fn fighter_snapshot_uses_committed_gate_side_for_door_passing_actor() {
    use crate::coordinates::{MapPoint, WorldPoint3D};
    use crate::gate::{Door, DoorIndex, DoorType};
    use crate::order::OrderType;
    use crate::sector::SectorNumber;
    use crate::sequence::{SequenceElement, SequenceElementData};

    let mut engine = EngineInner::new();
    let self_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let target_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Royalists));

    for (id, x) in [(self_id, 0.0), (target_id, 20.0)] {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).expect("test fighter exists")
        else {
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

    let Entity::Soldier(target) = engine
        .get_entity_mut(target_id)
        .expect("door-passing target exists")
    else {
        panic!("door-passing target changed kind")
    };
    assert!(target.actor.active_door_pass.is_none());
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
    let sequence_id = engine.orders.sequence_manager.launch_element(pass_door);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let positions = engine.boundary_positions_snapshot();
    let (optical_ai_position, optical_point) =
        engine.enemy_optical_geometry_at_owner_for_test(&assets, self_id, &positions, target_id);
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

    let fighters = engine.build_nearby_fighters_for(self_id, &assets);
    let target = fighters
        .iter()
        .find(|fighter| fighter.handle == target_id.index())
        .expect("door-passing target remains inside the fighter radius");
    assert_eq!(target.position.x, 120.0);
    assert_eq!(target.position.y, 5.0);
    assert_eq!(
        target.position.sector,
        crate::position_interface::SectorHandle::new(7)
    );
    assert_eq!(target.position.level, 3);
}

#[test]
fn reconsider_observation_uses_raw_positions_without_changing_shared_door_snapshots() {
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::coordinates::{MapPoint, WorldPoint3D};
    use crate::gate::{Door, DoorIndex, DoorType};
    use crate::order::OrderType;
    use crate::sector::SectorNumber;
    use crate::sequence::{SequenceElement, SequenceElementData};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    // Human handle zero means no entry in original-game AI lists.
    engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::Target;
            initial_element
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    let owner_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let raw_near_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let raw_far_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));

    for (id, x) in [(owner_id, 0.0), (raw_near_id, 20.0), (raw_far_id, 600.0)] {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).expect("test fighter exists")
        else {
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
    let owner = engine
        .get_entity_mut(owner_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("observation owner has enemy AI");
    owner.set_state(AiState::Attacking, Substate::AttackingObserve);
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
            classes: vec![crate::scb::ClassEntry {
                source_file: "reconsider_observation_pass_door_test.scs".into(),
                class_name: "StartUp".into(),
                size_of_member_variables: 0,
                member_variables: Vec::new(),
                functions: Vec::new(),
                quads: Vec::new(),
            }],
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
        let sequence_id = engine.orders.sequence_manager.launch_element(pass_door);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);
    }

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let scratch = engine.build_sim_scratch(&assets);
    let ctx = crate::engine::ai::build_ai_context_from_entity(
        engine
            .get_entity(owner_id)
            .expect("observation owner exists"),
        engine.control.frame_counter,
        None,
        engine.world.weather.is_forest_level,
        engine.world.weather.ambiance,
        engine.ai.standard_view_polygon_radius,
        &scratch.ai_entity_views,
        &scratch.ai_sight_obstacles,
        &engine.world.fast_grid,
        &assets.navigation.hiking_paths,
        &assets.navigation.hiking_waypoint_sectors,
        &engine.ai.global.all_soldier_handles,
        engine.control.sim_config.difficulty,
    );
    let tick = engine.build_npc_tick_data(&sim, owner_id, &assets);

    assert!(
        !tick
            .nearby_fighters
            .iter()
            .any(|fighter| fighter.handle == raw_near_id.index()),
        "generic nearby scan must reject the raw-near fighter at its door-resolved far side"
    );
    assert!(
        tick.nearby_fighters
            .iter()
            .any(|fighter| fighter.handle == raw_far_id.index()),
        "generic nearby scan must retain the raw-far fighter at its door-resolved near side"
    );
    let observation_position = |handle| {
        tick.reconsider_swordfight_observation_fighters
            .iter()
            .find(|fighter| fighter.handle == handle)
            .expect("fighter exists in complete observation registry")
            .raw_world_position
    };
    assert_eq!(observation_position(raw_near_id.index()).x, 20.0);
    assert_eq!(observation_position(raw_far_id.index()).x, 600.0);

    engine.dispatch_think_with_drain(
        &sim,
        owner_id,
        &Stimulus::new(StimulusType::EventTimer),
        &ctx,
        &tick,
        &assets,
    );

    let owner = engine
        .get_entity(owner_id)
        .and_then(Entity::enemy_ai)
        .expect("observation owner retains enemy AI");
    assert_eq!(
        owner.base.list_us,
        vec![owner_id.index(), raw_near_id.index()]
    );
}

#[test]
fn closure_review_alert_soldiers_keeps_inactive_soldier_in_both_camp_snapshots() {
    use crate::ai::{AlertSoldiersFailureContinuation, CrossNpcAction, Position};

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("inactive help recipient exists")
    else {
        panic!("inactive help recipient changed kind")
    };
    soldier.element.active = false;

    let (snapshot_able_to_fight, snapshot_able_to_help) =
        engine.test_soldier_snapshot_abilities(&assets, soldier_id);
    assert!(!snapshot_able_to_fight);
    assert!(
        snapshot_able_to_help,
        "full-tick camp population must use help eligibility's alive/conscious gate"
    );

    let (ctx, tick) = review2_context_and_tick(&engine, &sim, &assets, officer_id);
    let candidate = tick
        .camp_soldiers
        .iter()
        .find(|candidate| candidate.handle == soldier_id.index())
        .expect("inactive soldier remains in the direct-owner camp population");
    assert!(!candidate.is_able_to_fight);
    assert!(candidate.is_able_to_help);

    let global = engine.ai.global.clone();
    assert!(
        engine
            .get_entity_mut(officer_id)
            .and_then(Entity::enemy_ai_mut)
            .expect("inactive-help officer has EnemyAi")
            .alert_soldiers(
                Position::default(),
                0,
                &global,
                None,
                &ctx,
                &tick,
                AlertSoldiersFailureContinuation::None,
            )
    );
    assert!(matches!(
        engine
            .get_entity(officer_id)
            .and_then(Entity::ai_controller)
            .expect("inactive-help officer retains AI")
            .outbox
            .reentrant
            .cross_npc_actions
            .as_slice(),
        [CrossNpcAction::RequestThinkResult { target, .. }] if *target == soldier_id.index()
    ));
}
