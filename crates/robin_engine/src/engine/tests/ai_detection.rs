use super::*;

#[test]
fn primary_target_tracking_precedes_view_refresh() {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();

    let soldier_id = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Royalists));
    let pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let soldier_pos = MapPoint::new(100.0, 100.0);
    let target_pos = MapPoint::new(100.0, 200.0);

    if let Some(Entity::Soldier(soldier)) = engine.get_entity_mut(soldier_id) {
        soldier.element.active = true;
        soldier.element.set_position_map(soldier_pos);
        soldier.element.set_direction_instantly(4);
        soldier.npc.direction_old = 4;
        let ai = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("test soldier has enemy AI");
        ai.base.me = soldier_id.index();
        ai.base.primary_target = pc_id.index();
        ai.base.current_state = crate::ai::AiState::Attacking;
        ai.base.current_substate = crate::ai::Substate::AttackingReactiontime;
    }
    if let Some(Entity::Pc(pc)) = engine.get_entity_mut(pc_id) {
        pc.element.active = true;
        pc.element.set_position_map(target_pos);
    }

    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let expected = crate::position_interface::vector_to_sector_0_to_15_iso(
        target_pos.x - soldier_pos.x,
        target_pos.y - soldier_pos.y,
    );
    let Entity::Soldier(soldier) = engine.get_entity(soldier_id).unwrap() else {
        panic!("test soldier changed entity kind");
    };
    assert_eq!(soldier.element.direction(), expected);
    assert_eq!(
        soldier.npc.direction_old, expected,
        "RefreshView must observe the combat tracking direction in the same frame"
    );
}

#[test]
fn npc_hourglass_observes_exact_original_phase_order() {
    use super::tick::{NpcHourglassPhase as Phase, capture_npc_hourglass_phases};

    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();

    let (_, phases) =
        capture_npc_hourglass_phases(|| engine.perform_hourglass(&mut display, &assets, &mut dev));

    assert_eq!(
        phases,
        vec![
            Phase::SoldierPrelude,
            Phase::Patrol,
            Phase::BaseHuman,
            Phase::Broadcasts,
            Phase::View,
            Phase::Detection,
            Phase::Ambush,
            Phase::Busy,
            Phase::Ladder,
            Phase::LockGate,
            Phase::SixteenthFrame,
            Phase::NormalTimer,
            Phase::MacroTimer,
            Phase::QueuedStimuli,
        ]
    );
}

#[test]
fn npc_detection_observes_friend_state_at_creation_order_boundary() {
    use crate::ai::{AiState, Substate};
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};
    use crate::profiles::ProfileRank;

    fn observe(attacker_before_officer: bool) -> (AiState, AiState, Substate) {
        let mut engine = EngineInner::new();
        // Keep the relevant NPCs in slots 1/2 in both arrangements so the
        // swapped oracle is not confounded by a slot-zero special case.
        engine.add_entity(Entity::Target(crate::element::ElementTarget {
            element: ElementData {
                kind: ElementKind::Target,
                ..ElementData::default()
            },
            fx: Default::default(),
            target: Default::default(),
        }));

        let attacker = make_test_ai_soldier(Camp::Lacklandists);
        let officer = make_test_ai_soldier(Camp::Lacklandists);
        let (attacker_id, officer_id) = if attacker_before_officer {
            (engine.add_entity(attacker), engine.add_entity(officer))
        } else {
            let officer_id = engine.add_entity(officer);
            let attacker_id = engine.add_entity(attacker);
            (attacker_id, officer_id)
        };
        let pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

        for (id, x) in [(officer_id, 0.0), (attacker_id, 120.0)] {
            let Entity::Soldier(soldier) = engine
                .get_entity_mut(id)
                .expect("creation-order detection soldier exists")
            else {
                panic!("creation-order detection entity changed kind")
            };
            soldier.element.active = true;
            soldier
                .element
                .set_position(crate::coordinates::WorldPoint3D { x, y: 0.0, z: 0.0 });
            soldier.element.set_position_map(MapPoint::new(x, 0.0));
            soldier.element.set_direction_instantly(4);
            soldier.npc.life_points = 100;
            soldier.npc.view_direction = [1.0, 0.0];
            soldier.npc.view_radius = 135;
            soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
            soldier.npc.eye_status = crate::element::EyeStatus::Stare;
            let ai = soldier
                .npc
                .ai_brain
                .enemy_mut()
                .expect("creation-order detection soldier has enemy AI");
            ai.base.me = id.index();
            ai.soldier_profile_rank = if id == officer_id {
                ProfileRank::Officer
            } else {
                ProfileRank::Soldier
            };
        }

        let Entity::Pc(pc) = engine
            .get_entity_mut(pc_id)
            .expect("creation-order detection PC exists")
        else {
            panic!("creation-order detection target changed kind")
        };
        pc.element.active = true;
        pc.element.set_position(crate::coordinates::WorldPoint3D {
            x: 175.0,
            y: 0.0,
            z: 0.0,
        });
        pc.element.set_position_map(MapPoint::new(175.0, 0.0));
        pc.pc.life_points = 100;

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
            .characters
            .get_mut(0)
            .expect("fixture installs the PC character profile");
        profile.detection_speed_in_city = 100;
        profile.detection_speed_in_forest = 100;

        // Isolate the exact A-EVENT_VIEW → B-FRIEND edge after fixture
        // initialization has installed profiles and AI runtime defaults.
        let Entity::Soldier(attacker) = engine
            .get_entity_mut(attacker_id)
            .expect("attacker exists before detection")
        else {
            panic!("attacker changed kind")
        };
        attacker.npc.detectable_lists[DetectableType::Enemy as usize].clear();
        attacker.npc.detectable_lists[DetectableType::Friend as usize].clear();
        attacker.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
        attacker.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
            element: Some(pc_id),
            detectable_type: DetectableType::Enemy,
            // This oracle starts after the ordinary predetection shadow edge.
            shadow_seen_last_frame: true,
            ..Detectable::default()
        });

        let Entity::Soldier(officer) = engine
            .get_entity_mut(officer_id)
            .expect("officer exists before detection")
        else {
            panic!("officer changed kind")
        };
        officer.npc.detectable_lists[DetectableType::Friend as usize].clear();
        officer.npc.detectable_lists[DetectableType::Friend as usize].push(Detectable {
            element: Some(attacker_id),
            detectable_type: DetectableType::Friend,
            ..Detectable::default()
        });

        crate::sim_rng::with_seed(0xA013, |sim| engine.tick_enemy_ai(sim, &assets));

        let attacker_ai = engine
            .get_entity(attacker_id)
            .and_then(Entity::enemy_ai)
            .expect("attacker remains an enemy AI");
        let officer_ai = engine
            .get_entity(officer_id)
            .and_then(Entity::enemy_ai)
            .expect("officer remains an enemy AI");
        (
            attacker_ai.base.current_state,
            officer_ai.base.current_state,
            officer_ai.base.current_substate,
        )
    }

    let attacker_first = observe(true);
    assert_eq!(attacker_first.0, AiState::Attacking);
    assert_eq!(
        attacker_first.1,
        AiState::Default,
        "later officer must see that the earlier EVENT_VIEW made its friend unable to help"
    );
    assert_ne!(attacker_first.2, Substate::SeekingOfficerCallSoldier);
    assert_eq!(
        observe(false),
        (
            AiState::Attacking,
            AiState::Seeking,
            Substate::SeekingOfficerCallSoldier,
        ),
        "earlier officer must see the still-helpful soldier before that soldier handles EVENT_VIEW"
    );
}

#[test]
fn npc_hearing_thinks_before_same_slot_optical_detection() {
    use crate::ai::AiState;
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    // Keep the NPC out of legacy slot zero and choose the frame so its
    // `(frame + creation_order) % DETECTION_FREQUENCY_SOUNDS` gate is open.
    engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: ElementData {
            kind: ElementKind::Target,
            ..ElementData::default()
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    engine.control.frame_counter = 2;

    let soldier_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let stale_pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Dead));

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("hearing-order soldier exists")
    else {
        panic!("hearing-order entity changed kind")
    };
    soldier.element.active = true;
    soldier
        .element
        .set_position(crate::coordinates::WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
    soldier.element.set_position_map(MapPoint::new(0.0, 0.0));
    soldier.element.set_direction_instantly(4);
    soldier.npc.life_points = 100;
    soldier.npc.view_direction = [1.0, 0.0];
    soldier.npc.view_radius = 135;
    soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    soldier.npc.eye_status = crate::element::EyeStatus::Stare;
    let ai = soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("hearing-order soldier has enemy AI");
    ai.base.me = soldier_id.index();

    let Entity::Pc(pc) = engine
        .get_entity_mut(pc_id)
        .expect("hearing-order PC exists")
    else {
        panic!("hearing-order target changed kind")
    };
    pc.element.active = true;
    pc.element.set_position(crate::coordinates::WorldPoint3D {
        x: 55.0,
        y: 0.0,
        z: 0.0,
    });
    pc.element.set_position_map(MapPoint::new(55.0, 0.0));
    pc.pc.life_points = 100;

    let Entity::Pc(stale_pc) = engine
        .get_entity_mut(stale_pc_id)
        .expect("stale hearing-order PC exists")
    else {
        panic!("stale hearing-order target changed kind")
    };
    stale_pc.element.active = false;
    stale_pc.pc.life_points = 0;

    // RunningUpright on ground produces a 70-volume TAPTAPTAP. At 55 units
    // this becomes the original's 15-volume subjective noise, while keeping
    // EVENT_VIEW outside the unrelated close-combat branch (< 50 units).
    // Install it as the production snapshot builder's current animation
    // instead of injecting a synthetic noise into the detection helper.
    let mut movement = SequenceElement::new_movement(
        1,
        crate::element::Command::Move,
        Some(pc_id),
        OrderType::RunningUpright,
    );
    movement
        .orders
        .push_back(Order::test_new(OrderType::RunningUpright, 0.0, 0.0));
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("fixture installs the PC character profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("soldier exists before hearing-order detection")
    else {
        panic!("hearing-order soldier changed kind")
    };
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    soldier.npc.detection_suspects[DetectableType::Enemy as usize] = 0;
    // Acoustics runs before optical CleanUpDetectables. A just-dead PC can
    // therefore still occupy an earlier enemy-list slot without appearing in
    // the alive-only world snapshot; it must not block the later audible PC.
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
        element: Some(stale_pc_id),
        detectable_type: DetectableType::Enemy,
        ..Detectable::default()
    });
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
        element: Some(pc_id),
        detectable_type: DetectableType::Enemy,
        // Keep the oracle about HEAR → VIEW rather than predetection shadow.
        shadow_seen_last_frame: true,
        ..Detectable::default()
    });

    crate::sim_rng::with_seed(0xA013_0EAD, |sim| engine.tick_enemy_ai(sim, &assets));

    assert_eq!(
        engine
            .get_entity(pc_id)
            .and_then(Entity::actor_data)
            .expect("hearing-order PC remains an actor")
            .last_noise_volume,
        70,
        "production PC snapshot must derive the expected running noise"
    );
    assert!(
        engine
            .get_entity(soldier_id)
            .and_then(Entity::npc_data)
            .expect("hearing-order soldier remains an NPC")
            .detectable_lists[DetectableType::Enemy as usize][0]
            .heard_last_frame,
        "production acoustic pass must reach UpdateHearing's latch"
    );
    let ai = engine
        .get_entity(soldier_id)
        .and_then(Entity::enemy_ai)
        .expect("soldier remains an enemy AI");
    assert_eq!(
        ai.base.current_state,
        AiState::Attacking,
        "synchronous EVENT_HEAR must make optical detection instant in the same RefreshDetection call"
    );
}

#[test]
fn lackland_detection_scans_and_retains_full_fifo_while_ai_locked() {
    use crate::ai::{AiLockFlags, AiState, StimulusInfo, StimulusType, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{
        Camp, Detectable, DetectableType, ElementBonus, ElementData, ElementKind, Entity,
    };
    use crate::element_kinds::ObjectType;
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: ElementData {
            kind: ElementKind::Target,
            ..ElementData::default()
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    engine.control.frame_counter = 2;

    let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let first_visible_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let lost_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let last_visible_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let body_id = engine.add_entity(make_test_pc(crate::element::Posture::Dead));
    let object_id = engine.add_entity(Entity::Bonus(ElementBonus {
        element: ElementData {
            kind: ElementKind::ObjectBonus,
            active: true,
            ..ElementData::default()
        },
        object: crate::element::ObjectData {
            object_type: ObjectType::Coin,
            ..crate::element::ObjectData::default()
        },
    }));
    let friend_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("locked detection observer exists")
    else {
        panic!("locked detection observer changed kind")
    };
    observer.element.active = true;
    observer
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
    observer.element.set_position_map(MapPoint::new(0.0, 0.0));
    observer.element.set_direction_instantly(4);
    observer.npc.life_points = 100;
    observer.npc.view_direction = [1.0, 0.0];
    observer.npc.view_radius = 300;
    observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    observer.npc.eye_status = crate::element::EyeStatus::Stare;

    for (id, x, life_points) in [
        (first_visible_id, 55.0, 100),
        (lost_id, -200.0, 100),
        (last_visible_id, 80.0, 100),
        (body_id, 100.0, 0),
    ] {
        let Entity::Pc(pc) = engine
            .get_entity_mut(id)
            .expect("locked detection PC exists")
        else {
            panic!("locked detection PC changed kind")
        };
        pc.element.active = true;
        pc.element
            .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
        pc.element.set_position_map(MapPoint::new(x, 0.0));
        pc.pc.life_points = life_points;
    }

    let Entity::Bonus(object) = engine
        .get_entity_mut(object_id)
        .expect("locked detection object exists")
    else {
        panic!("locked detection object changed kind")
    };
    object
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(100.0, 0.0, 0.0));
    object.element.set_position_map(MapPoint::new(100.0, 0.0));

    let Entity::Soldier(friend) = engine
        .get_entity_mut(friend_id)
        .expect("locked observer's friend exists")
    else {
        panic!("locked observer's friend changed kind")
    };
    friend.element.active = true;
    friend
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(-20.0, 20.0, 0.0));
    friend.element.set_position_map(MapPoint::new(-20.0, 20.0));
    friend.npc.life_points = 100;
    friend.npc.eye_status = crate::element::EyeStatus::Closed;

    // RunningUpright produces the production 70-volume TAPTAPTAP used by
    // RefreshDetection's acoustic pass. With observer slot 1 and frame 2,
    // its three-frame hearing cadence is open.
    let mut movement = SequenceElement::new_movement(
        1,
        crate::element::Command::Move,
        Some(first_visible_id),
        OrderType::RunningUpright,
    );
    movement
        .orders
        .push_back(Order::test_new(OrderType::RunningUpright, 0.0, 0.0));
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("fixture installs the PC character profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("locked detection observer exists after fixture")
    else {
        panic!("locked detection observer changed kind after fixture")
    };
    let ai = observer
        .npc
        .ai_brain
        .enemy_mut()
        .expect("locked detection observer has enemy AI");
    ai.base.me = observer_id.index();
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultOnPost;
    ai.current_task_priority = task_priority::NONE;
    ai.base.locks_flag_field = AiLockFlags::FREEZE;

    observer.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    observer.npc.detectable_lists[DetectableType::Body as usize].clear();
    observer.npc.detectable_lists[DetectableType::Object as usize].clear();
    observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
    observer.npc.detection_suspects[DetectableType::Body as usize] = 999;
    observer.npc.detection_suspects[DetectableType::Object as usize] = 999;
    for (target_id, seen_last_frame) in [
        (first_visible_id, false),
        (lost_id, true),
        (last_visible_id, false),
    ] {
        observer.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
            element: Some(target_id),
            detectable_type: DetectableType::Enemy,
            seen_last_frame,
            ..Detectable::default()
        });
    }
    observer.npc.detectable_lists[DetectableType::Body as usize].push(Detectable {
        element: Some(body_id),
        detectable_type: DetectableType::Body,
        // Keep this oracle's shadow prefix confined to the Enemy bucket.
        shadow_seen_last_frame: true,
        ..Detectable::default()
    });
    observer.npc.detectable_lists[DetectableType::Object as usize].push(Detectable {
        element: Some(object_id),
        detectable_type: DetectableType::Object,
        ..Detectable::default()
    });

    crate::sim_rng::with_seed(0xA013_0B22, |sim| engine.tick_enemy_ai(sim, &assets));

    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .expect("locked detection observer remains an NPC");
    assert_eq!(
        observer.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .map(|det| (
                det.heard_last_frame,
                det.shadow_seen_last_frame,
                det.seen_last_frame,
            ))
            .collect::<Vec<_>>(),
        vec![
            (true, true, true),
            (false, false, false),
            (false, true, true)
        ],
        "AI lock must not suppress acoustic, predetection, or optical latch updates"
    );
    assert_eq!(
        observer.detection_suspects[DetectableType::Enemy as usize],
        0,
        "locked Enemy detection must still commit and reset suspects"
    );
    assert!(
        observer.detectable_lists[DetectableType::Body as usize].is_empty(),
        "locked non-Enemy buckets must still commit one-shot detectables"
    );
    assert!(
        observer.detectable_lists[DetectableType::Object as usize].is_empty(),
        "locked Object detection must still commit its one-shot detectable"
    );

    let ai = engine
        .get_entity(observer_id)
        .and_then(Entity::enemy_ai)
        .expect("locked detection observer retains enemy AI");
    assert_eq!(
        (ai.base.current_state, ai.base.current_substate),
        (AiState::Default, Substate::DefaultOnPost)
    );
    assert!(ai.base.outbox.detection.stimuli.is_empty());
    assert_eq!(ai.base.last_stimulus_actor, Some(body_id.index()));
    assert_eq!(
        ai.base
            .stimulus_queue
            .iter()
            .map(|stimulus| stimulus.stimulus_type)
            .collect::<Vec<_>>(),
        vec![
            StimulusType::EventHear,
            StimulusType::EventSeesShadow,
            StimulusType::EventSeesShadow,
            StimulusType::EventView,
            StimulusType::EventOutOfView,
            StimulusType::EventView,
            StimulusType::EventSeesBody,
            StimulusType::EventSeesObject,
        ],
        "StartThink must retain the complete HEAR then optical FIFO under AI lock"
    );
    assert!(matches!(
        ai.base.stimulus_queue[0].info,
        StimulusInfo::Noise(_)
    ));
    assert_eq!(
        ai.base
            .stimulus_queue
            .iter()
            .filter_map(|stimulus| match stimulus.info {
                StimulusInfo::Human(target) => Some(target),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            first_visible_id.index(),
            lost_id.index(),
            last_visible_id.index(),
            body_id.index(),
        ]
    );
    assert_eq!(
        ai.base
            .stimulus_queue
            .last()
            .expect("Object event closes the retained detection FIFO")
            .info,
        StimulusInfo::Object(object_id.index()),
        "EVENT_SEES_OBJECT must retain an object payload, not impersonate a human"
    );
    assert_eq!(
        engine
            .get_entity(friend_id)
            .and_then(Entity::npc_data)
            .expect("locked observer's friend remains an NPC")
            .ai_state(),
        AiState::Default,
        "a retained VIEW must not leak through the later out-of-band ally alert"
    );
    assert!(
        !engine
            .get_entity(observer_id)
            .and_then(Entity::npc_data)
            .expect("locked detection observer remains an NPC")
            .alerted,
        "AILOCK_FREEZE must retain VIEW without pre-alerting its observer"
    );

    // Static RHArtificialIntelligence::mbFreeze is a separate mode: the
    // next RefreshDetection still scans and commits its latch, but StartThink
    // discards the resulting VIEW instead of retaining it.
    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("static-freeze detection observer exists")
    else {
        panic!("static-freeze detection observer changed kind")
    };
    let ai = observer
        .npc
        .ai_brain
        .enemy_mut()
        .expect("static-freeze detection observer retains enemy AI");
    ai.base.locks_flag_field = AiLockFlags::empty();
    ai.base.stimulus_queue.clear();
    observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
    observer.npc.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame = false;
    observer.npc.detectable_lists[DetectableType::Enemy as usize][0].shadow_seen_last_frame = true;

    engine.ai.global.freeze = true;
    crate::sim_rng::with_seed(0xA013_0B24, |sim| engine.tick_enemy_ai(sim, &assets));

    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .expect("static-freeze detection observer remains an NPC");
    assert!(
        observer.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame,
        "static AI freeze must not suppress RefreshDetection latch commits"
    );
    let ai = observer
        .ai_brain
        .enemy()
        .expect("static-freeze detection observer retains enemy AI");
    assert!(
        ai.base.stimulus_queue.is_empty(),
        "static AI freeze must discard detection stimuli"
    );
    assert_eq!(ai.base.current_state, AiState::Default);
    assert!(!observer.alerted);
}

#[test]
fn inactive_building_viewer_runs_hearing_then_optics_while_outdoor_viewer_is_a_noop() {
    use crate::ai::{AiLockFlags, AiState, StimulusType, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, Entity};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    for observer_inside in [true, false] {
        let mut engine = EngineInner::new();
        let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
        let indoor_target_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
        let inactive_outdoor_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
        let runner_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
        let building = crate::position_interface::SectorHandle::new(42).unwrap();
        install_test_building_sector(&mut engine, 42);

        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("inactive-viewer observer exists")
        else {
            panic!("inactive-viewer observer changed kind")
        };
        observer.element.active = true;
        observer
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
        observer.element.set_position_map(MapPoint::new(0.0, 0.0));
        observer.element.set_direction_instantly(4);
        observer.npc.life_points = 100;
        observer.npc.view_direction = [1.0, 0.0];
        observer.npc.view_radius = 300;
        observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
        observer.npc.eye_status = crate::element::EyeStatus::Stare;
        if observer_inside {
            observer.element.set_sector(Some(building));
        }

        let Entity::Pc(indoor_target) = engine
            .get_entity_mut(indoor_target_id)
            .expect("inactive same-building target exists")
        else {
            panic!("inactive same-building target changed kind")
        };
        indoor_target.element.active = true;
        indoor_target
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(40.0, 0.0, 0.0));
        indoor_target
            .element
            .set_position_map(MapPoint::new(40.0, 0.0));
        indoor_target.element.set_sector(Some(building));
        indoor_target.pc.life_points = 100;

        let Entity::Pc(inactive_outdoor) = engine
            .get_entity_mut(inactive_outdoor_id)
            .expect("inactive outdoor target exists")
        else {
            panic!("inactive outdoor target changed kind")
        };
        inactive_outdoor.element.active = true;
        inactive_outdoor
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(45.0, 0.0, 0.0));
        inactive_outdoor
            .element
            .set_position_map(MapPoint::new(45.0, 0.0));
        inactive_outdoor.pc.life_points = 100;

        let Entity::Pc(runner) = engine
            .get_entity_mut(runner_id)
            .expect("inactive-viewer runner exists")
        else {
            panic!("inactive-viewer runner changed kind")
        };
        runner.element.active = true;
        runner
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(55.0, 0.0, 0.0));
        runner.element.set_position_map(MapPoint::new(55.0, 0.0));
        runner.pc.life_points = 100;

        let mut movement = SequenceElement::new_movement(
            1,
            crate::element::Command::Move,
            Some(runner_id),
            OrderType::RunningUpright,
        );
        movement
            .orders
            .push_back(Order::test_new(OrderType::RunningUpright, 0.0, 0.0));
        let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(movement_sequence, 0);

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
            .characters
            .get_mut(0)
            .expect("fixture installs the PC character profile");
        profile.detection_speed_in_city = 100;
        profile.detection_speed_in_forest = 100;

        let Entity::Pc(indoor_target) = engine
            .get_entity_mut(indoor_target_id)
            .expect("same-building target exists after fixture")
        else {
            panic!("same-building target changed kind after fixture")
        };
        indoor_target.element.active = false;
        engine
            .get_entity_mut(inactive_outdoor_id)
            .expect("inactive outdoor target exists after fixture")
            .element_data_mut()
            .active = false;

        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("inactive-viewer observer exists after fixture")
        else {
            panic!("inactive-viewer observer changed kind after fixture")
        };
        observer.element.active = false;
        let ai = observer
            .npc
            .ai_brain
            .enemy_mut()
            .expect("inactive-viewer observer has enemy AI");
        ai.base.me = observer_id.index();
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::DefaultOnPost;
        ai.current_task_priority = task_priority::NONE;
        ai.base.locks_flag_field = AiLockFlags::BUSY;
        observer.npc.detectable_lists[DetectableType::Enemy as usize] = vec![
            Detectable {
                element: Some(indoor_target_id),
                detectable_type: DetectableType::Enemy,
                shadow_seen_last_frame: true,
                ..Detectable::default()
            },
            Detectable {
                element: Some(inactive_outdoor_id),
                detectable_type: DetectableType::Enemy,
                seen_last_frame: true,
                shadow_seen_last_frame: true,
                ..Detectable::default()
            },
            Detectable {
                element: Some(runner_id),
                detectable_type: DetectableType::Enemy,
                shadow_seen_last_frame: true,
                ..Detectable::default()
            },
        ];
        observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
        observer.npc.maximal_detection_suspect = 777;

        crate::sim_rng::with_seed(0xA013_1A51, |sim| engine.tick_enemy_ai(sim, &assets));

        let observer = engine
            .get_entity(observer_id)
            .and_then(Entity::npc_data)
            .expect("inactive-viewer observer remains an NPC");
        let ai = observer
            .ai_brain
            .enemy()
            .expect("inactive-viewer observer retains enemy AI");
        assert_eq!(ai.base.current_state, AiState::Default);

        if observer_inside {
            assert_eq!(
                ai.base
                    .stimulus_queue
                    .iter()
                    .map(|stimulus| stimulus.stimulus_type)
                    .collect::<Vec<_>>(),
                vec![
                    StimulusType::EventHear,
                    StimulusType::EventView,
                    StimulusType::EventOutOfView,
                ],
                "an inactive building viewer must retain acoustic-before-optical FIFO order"
            );
            assert!(observer.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame);
            assert!(
                !observer.detectable_lists[DetectableType::Enemy as usize][1].seen_last_frame,
                "a living inactive outdoor PC must stay in the list and produce a falling edge"
            );
            assert!(observer.detectable_lists[DetectableType::Enemy as usize][2].heard_last_frame);
            assert_eq!(
                observer.detection_suspects[DetectableType::Enemy as usize],
                0
            );
            assert_eq!(observer.maximal_detection_suspect, 0);
            assert_eq!(
                engine
                    .get_entity(indoor_target_id)
                    .and_then(Entity::actor_data)
                    .expect("inactive indoor target remains an actor")
                    .last_noise_volume,
                0,
                "retaining an inactive PC for same-building sight must not make it audible"
            );
        } else {
            assert!(
                ai.base.stimulus_queue.is_empty(),
                "the first RefreshDetection gate must make an inactive outdoor viewer a no-op"
            );
            assert_eq!(
                observer.detectable_lists[DetectableType::Enemy as usize].len(),
                3,
                "living inactive targets remain Enemy detectables until they die"
            );
            assert!(!observer.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame);
            assert!(observer.detectable_lists[DetectableType::Enemy as usize][1].seen_last_frame);
            assert!(!observer.detectable_lists[DetectableType::Enemy as usize][2].heard_last_frame);
            assert_eq!(
                observer.detection_suspects[DetectableType::Enemy as usize],
                999
            );
            assert_eq!(observer.maximal_detection_suspect, 777);
        }
    }
}

#[test]
fn inactive_npc_blip_detection_requires_door_or_building_eligibility() {
    use crate::element::{Camp, Entity};

    for observer_inside in [true, false] {
        let mut engine = EngineInner::new();
        let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
        let pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
        install_test_building_sector(&mut engine, 42);

        let Entity::Soldier(observer) = engine
            .get_entity_mut(observer_id)
            .expect("blipped inactive observer exists")
        else {
            panic!("blipped inactive observer changed kind")
        };
        observer.element.active = true;
        observer.element.blipped = true;
        observer.npc.life_points = 100;
        observer
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
        observer.element.set_position_map(MapPoint::new(0.0, 0.0));
        if observer_inside {
            observer.element.set_sector(Some(
                crate::position_interface::SectorHandle::new(42).unwrap(),
            ));
        }

        let Entity::Pc(pc) = engine
            .get_entity_mut(pc_id)
            .expect("blip-viewing PC exists")
        else {
            panic!("blip-viewing PC changed kind")
        };
        pc.element.active = true;
        pc.pc.playable = true;
        pc.pc.life_points = 100;
        pc.element
            .set_position(crate::coordinates::WorldPoint3D::new(20.0, 0.0, 0.0));
        pc.element.set_position_map(MapPoint::new(20.0, 0.0));

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);
        engine
            .get_entity_mut(observer_id)
            .expect("blipped observer exists after fixture")
            .element_data_mut()
            .active = false;

        crate::sim_rng::with_seed(0xA013_B11F, |sim| engine.tick_enemy_ai(sim, &assets));

        assert_eq!(
            engine
                .get_entity(observer_id)
                .expect("blipped observer survives tick")
                .element_data()
                .blipped,
            !observer_inside,
            "inactive building NPCs run blip detection; inactive outdoor NPCs do not"
        );
    }
}

#[test]
fn inactive_door_transit_viewer_runs_blip_and_hearing_then_skips_optics() {
    use crate::ai::{AiLockFlags, AiState, StimulusType, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, Entity};
    use crate::order::{Order, OrderType};
    use crate::position_interface::DoorHandle;
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let runner_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("door-transit observer exists")
    else {
        panic!("door-transit observer changed kind")
    };
    observer.element.active = true;
    observer.element.blipped = true;
    observer.npc.life_points = 100;
    observer
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
    observer.element.set_position_map(MapPoint::new(0.0, 0.0));
    observer.element.set_direction_instantly(4);
    observer.npc.view_direction = [1.0, 0.0];
    observer.npc.view_radius = 300;
    observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    observer.npc.eye_status = crate::element::EyeStatus::Stare;

    let Entity::Pc(runner) = engine
        .get_entity_mut(runner_id)
        .expect("door-transit runner exists")
    else {
        panic!("door-transit runner changed kind")
    };
    runner.element.active = true;
    runner.pc.playable = true;
    runner.pc.life_points = 100;
    runner
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(55.0, 0.0, 0.0));
    runner.element.set_position_map(MapPoint::new(55.0, 0.0));

    let mut movement = SequenceElement::new_movement(
        1,
        crate::element::Command::Move,
        Some(runner_id),
        OrderType::RunningUpright,
    );
    movement
        .orders
        .push_back(Order::test_new(OrderType::RunningUpright, 0.0, 0.0));
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("door-transit observer exists after fixture")
    else {
        panic!("door-transit observer changed kind after fixture")
    };
    observer.element.active = false;
    observer
        .element
        .sprite
        .position_iface
        .set_door_for_test(DoorHandle(0));
    observer.npc.detectable_lists[DetectableType::Enemy as usize] = vec![Detectable {
        element: Some(runner_id),
        detectable_type: DetectableType::Enemy,
        shadow_seen_last_frame: true,
        ..Detectable::default()
    }];
    observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
    observer.npc.maximal_detection_suspect = 777;
    let ai = observer
        .npc
        .ai_brain
        .enemy_mut()
        .expect("door-transit observer has enemy AI");
    ai.base.me = observer_id.index();
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultOnPost;
    ai.current_task_priority = task_priority::NONE;
    ai.base.locks_flag_field = AiLockFlags::BUSY;
    ai.base.max_visibility = 0.75;

    crate::sim_rng::with_seed(0xA013_D00F, |sim| engine.tick_enemy_ai(sim, &assets));

    let Entity::Soldier(observer) = engine
        .get_entity(observer_id)
        .expect("door-transit observer survives tick")
    else {
        panic!("door-transit observer changed kind during tick")
    };
    let ai = observer
        .npc
        .ai_brain
        .enemy()
        .expect("door-transit observer retains enemy AI");
    assert!(
        !observer.element.blipped,
        "door transit passes the blip gate"
    );
    assert_eq!(
        ai.base
            .stimulus_queue
            .iter()
            .map(|stimulus| stimulus.stimulus_type)
            .collect::<Vec<_>>(),
        vec![StimulusType::EventHear],
        "door transit passes acoustics but the sector-only optical gate rejects it"
    );
    assert!(observer.npc.detectable_lists[DetectableType::Enemy as usize][0].heard_last_frame);
    assert!(!observer.npc.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame);
    assert_eq!(
        observer.npc.detection_suspects[DetectableType::Enemy as usize],
        999,
        "the door-only optical return must not scan or decay Enemy suspects"
    );
    assert_eq!(observer.npc.maximal_detection_suspect, 0);
    assert_eq!(ai.base.max_visibility, 0.0);
}

#[test]
fn royalist_blip_auto_reveal_obeys_the_common_sixteen_frame_cadence() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::element::{Camp, Entity};

    let mut engine = EngineInner::new();
    let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Royalists));
    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("Royalist blip observer exists")
    else {
        panic!("Royalist blip observer changed kind")
    };
    observer.element.active = true;
    observer.element.blipped = true;
    observer.npc.life_points = 100;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    engine.control.frame_counter = 1;
    engine.tick_enemy_ai(sim, &assets);
    assert!(
        engine
            .get_entity(observer_id)
            .expect("Royalist blip observer survives closed cadence")
            .element_data()
            .blipped
    );

    engine.control.frame_counter = 16;
    engine.tick_enemy_ai(sim, &assets);
    assert!(
        !engine
            .get_entity(observer_id)
            .expect("Royalist blip observer survives open cadence")
            .element_data()
            .blipped
    );
}

#[test]
fn retained_detection_view_rebuilds_the_live_enemy_scan_on_replay() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::ai::{AiLockFlags, AiState, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, Entity};

    let mut engine = EngineInner::new();
    let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let rising_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let already_seen_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("queued replay observer exists")
    else {
        panic!("queued replay observer changed kind")
    };
    observer.element.active = true;
    observer
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
    observer.element.set_position_map(MapPoint::new(0.0, 0.0));
    observer.element.set_direction_instantly(4);
    observer.npc.life_points = 100;
    observer.npc.view_direction = [1.0, 0.0];
    observer.npc.view_radius = 300;
    observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    observer.npc.eye_status = crate::element::EyeStatus::Stare;

    for (id, x) in [(rising_id, 80.0), (already_seen_id, 120.0)] {
        let Entity::Pc(pc) = engine.get_entity_mut(id).expect("queued replay PC exists") else {
            panic!("queued replay PC changed kind")
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
        .expect("fixture installs the PC character profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("queued replay observer exists after fixture")
    else {
        panic!("queued replay observer changed kind after fixture")
    };
    let ai = observer
        .npc
        .ai_brain
        .enemy_mut()
        .expect("queued replay observer has enemy AI");
    ai.base.me = observer_id.index();
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultOnPost;
    ai.current_task_priority = task_priority::NONE;
    ai.base.locks_flag_field = AiLockFlags::BUSY;
    ai.list_them.clear();

    observer.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
    for (target_id, seen_last_frame) in [(rising_id, false), (already_seen_id, true)] {
        observer.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
            element: Some(target_id),
            detectable_type: DetectableType::Enemy,
            seen_last_frame,
            shadow_seen_last_frame: true,
            ..Detectable::default()
        });
    }

    crate::sim_rng::with_seed(0xA013_0B23, |sim| engine.tick_enemy_ai(sim, &assets));
    let ai = engine
        .get_entity(observer_id)
        .and_then(Entity::enemy_ai)
        .expect("queued replay observer retains enemy AI");
    assert_eq!(ai.base.stimulus_queue.len(), 1);
    assert_eq!(ai.base.current_state, AiState::Default);

    engine
        .get_entity_mut(observer_id)
        .and_then(Entity::ai_controller_mut)
        .expect("queued replay observer retains controller")
        .locks_flag_field = AiLockFlags::empty();
    engine.tick_ai_queued_stimuli(sim, &assets);

    let ai = engine
        .get_entity(observer_id)
        .and_then(Entity::enemy_ai)
        .expect("queued replay observer retains enemy AI after replay");
    assert!(ai.base.stimulus_queue.is_empty());
    assert_eq!(ai.base.current_state, AiState::Attacking);
    assert_eq!(ai.base.primary_target, rising_id.index());
    assert_eq!(
        ai.list_them,
        vec![rising_id.index(), already_seen_id.index()],
        "retained VIEW replay must rebuild all currently latched enemies, not seed only its payload"
    );
    assert!(
        engine
            .get_entity(observer_id)
            .and_then(Entity::npc_data)
            .expect("queued replay observer remains an NPC after replay")
            .alerted,
        "accepted retained VIEW must set the persistent alert marker at dispatch time"
    );
}

#[test]
fn npc_out_of_view_precedes_same_slot_body_fifo() {
    use crate::ai::{AiState, Position, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};

    let mut engine = EngineInner::new();
    engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: ElementData {
            kind: ElementKind::Target,
            ..ElementData::default()
        },
        fx: Default::default(),
        target: Default::default(),
    }));

    let soldier_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let lost_pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let body_id = engine.add_entity(make_test_pc(crate::element::Posture::Dead));

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("out-of-view soldier exists")
    else {
        panic!("out-of-view observer changed kind")
    };
    soldier.element.active = true;
    soldier
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
    soldier.element.set_position_map(MapPoint::new(0.0, 0.0));
    soldier.element.set_direction_instantly(4);
    soldier.npc.life_points = 100;
    soldier.npc.view_direction = [1.0, 0.0];
    soldier.npc.view_radius = 135;
    soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    soldier.npc.eye_status = crate::element::EyeStatus::Stare;
    let ai = soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("out-of-view soldier has enemy AI");
    ai.base.me = soldier_id.index();
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingObserve;
    ai.base.primary_target = lost_pc_id.index();
    ai.base.seek_position = Position {
        x: -200.0,
        y: 0.0,
        ..Position::default()
    };
    ai.current_task_priority = task_priority::ENEMY;

    let Entity::Pc(lost_pc) = engine.get_entity_mut(lost_pc_id).expect("lost PC exists") else {
        panic!("lost target changed kind")
    };
    lost_pc.element.active = true;
    lost_pc
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(-200.0, 0.0, 0.0));
    lost_pc.element.set_position_map(MapPoint::new(-200.0, 0.0));
    lost_pc.pc.life_points = 100;

    let Entity::Pc(body) = engine.get_entity_mut(body_id).expect("body PC exists") else {
        panic!("body target changed kind")
    };
    body.element.active = true;
    body.element
        .set_position(crate::coordinates::WorldPoint3D::new(80.0, 0.0, 0.0));
    body.element.set_position_map(MapPoint::new(80.0, 0.0));
    body.pc.life_points = 0;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("fixture installs the living PC profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("out-of-view soldier exists before detection")
    else {
        panic!("out-of-view soldier changed kind")
    };
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    soldier.npc.detectable_lists[DetectableType::Body as usize].clear();
    soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
        element: Some(lost_pc_id),
        detectable_type: DetectableType::Enemy,
        seen_last_frame: true,
        shadow_seen_last_frame: true,
        ..Detectable::default()
    });
    soldier.npc.detectable_lists[DetectableType::Body as usize].push(Detectable {
        element: Some(body_id),
        detectable_type: DetectableType::Body,
        shadow_seen_last_frame: true,
        ..Detectable::default()
    });

    crate::sim_rng::with_seed(0xA013_0A7, |sim| engine.tick_enemy_ai(sim, &assets));

    let soldier = engine
        .get_entity(soldier_id)
        .and_then(Entity::npc_data)
        .expect("out-of-view soldier remains an NPC");
    assert!(
        !soldier.detectable_lists[DetectableType::Enemy as usize][0].seen_last_frame,
        "lost enemy must clear its seen latch"
    );
    assert!(
        soldier.detectable_lists[DetectableType::Body as usize].is_empty(),
        "visible body must commit and leave its one-shot detectable list"
    );
    let ai = engine
        .get_entity(soldier_id)
        .and_then(Entity::enemy_ai)
        .expect("out-of-view soldier retains enemy AI");
    assert_eq!(
        ai.base.detected_body,
        body_id.index(),
        "OUTOFVIEW must enter Seeking before the later BODY stimulus is handled"
    );
    assert_eq!(ai.base.current_state, AiState::Seeking);
    assert_eq!(ai.base.current_substate, Substate::SeekingBodyReactiontime);
}

#[test]
fn npc_detection_queues_every_rising_enemy_in_detectable_order() {
    use crate::ai::{AiState, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};

    fn observe(far_first: bool) -> (Vec<u32>, Vec<bool>, Vec<u32>) {
        let mut engine = EngineInner::new();
        engine.add_entity(Entity::Target(crate::element::ElementTarget {
            element: ElementData {
                kind: ElementKind::Target,
                ..ElementData::default()
            },
            fx: Default::default(),
            target: Default::default(),
        }));

        let soldier_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
        // A learned-through disguise remains in the Enemy bucket and must
        // emit ordinary EVENT_VIEW, never EVENT_SEES_BEGGAR.
        let far_pc_id = engine.add_entity(make_test_pc(crate::element::Posture::SimulatingBeggar));
        let near_pc_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

        let Entity::Soldier(soldier) = engine
            .get_entity_mut(soldier_id)
            .expect("multi-view soldier exists")
        else {
            panic!("multi-view observer changed kind")
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

        for (pc_id, x) in [(far_pc_id, 120.0), (near_pc_id, 80.0)] {
            let Entity::Pc(pc) = engine.get_entity_mut(pc_id).expect("multi-view PC exists") else {
                panic!("multi-view target changed kind")
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
            .expect("fixture installs the PC character profile");
        profile.detection_speed_in_city = 100;
        profile.detection_speed_in_forest = 100;

        let Entity::Soldier(soldier) = engine
            .get_entity_mut(soldier_id)
            .expect("multi-view soldier exists before detection")
        else {
            panic!("multi-view observer changed kind")
        };
        let ai = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("multi-view soldier has enemy AI");
        ai.base.me = soldier_id.index();
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::DefaultOnPost;
        ai.current_task_priority = task_priority::NONE;
        ai.base.got_the_beggar_trick = true;
        ai.list_them.clear();

        let ordered_targets = if far_first {
            [far_pc_id, near_pc_id]
        } else {
            [near_pc_id, far_pc_id]
        };
        let expected_order: Vec<u32> = ordered_targets.iter().map(|id| id.index()).collect();
        soldier.npc.detectable_lists[DetectableType::Enemy as usize].clear();
        soldier.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
        for target_id in ordered_targets {
            soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
                element: Some(target_id),
                detectable_type: DetectableType::Enemy,
                shadow_seen_last_frame: true,
                ..Detectable::default()
            });
        }

        crate::sim_rng::with_seed(0xA013_0B1E, |sim| engine.tick_enemy_ai(sim, &assets));

        let soldier = engine
            .get_entity(soldier_id)
            .and_then(Entity::npc_data)
            .expect("multi-view soldier remains an NPC");
        let latches = soldier.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .map(|det| det.seen_last_frame)
            .collect();
        assert_eq!(
            soldier.follow_target,
            Some(ordered_targets[0]),
            "the first accepted VIEW must retain focus after the later FIFO entry"
        );
        let ai = engine
            .get_entity(soldier_id)
            .and_then(Entity::enemy_ai)
            .expect("multi-view soldier retains enemy AI");
        assert_eq!(ai.base.current_state, AiState::Attacking);
        assert_eq!(ai.base.current_substate, Substate::AttackingReactiontime);
        assert_eq!(
            ai.base.primary_target, expected_order[0],
            "the first detectable's VIEW must win even when a later target is nearer"
        );
        assert_eq!(
            ai.base.last_stimulus_actor,
            Some(expected_order[1]),
            "the second VIEW must run through its own complete Think boundary"
        );
        (ai.list_them.clone(), latches, expected_order)
    }

    let (far_then_near, far_then_near_latches, far_then_near_expected) = observe(true);
    let (near_then_far, near_then_far_latches, near_then_far_expected) = observe(false);

    assert_eq!(far_then_near_latches, vec![true, true]);
    assert_eq!(near_then_far_latches, vec![true, true]);
    assert_eq!(far_then_near, far_then_near_expected);
    assert_eq!(near_then_far, near_then_far_expected);
}

#[test]
fn npc_detection_view_rebinds_combat_data_to_the_queued_target() {
    use crate::ai::{AiState, Decision, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};

    let mut engine = EngineInner::new();
    engine.add_entity(Entity::Target(crate::element::ElementTarget {
        element: ElementData {
            kind: ElementKind::Target,
            ..ElementData::default()
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    let soldier_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let old_target_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let viewed_target_id = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

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

    for (pc_id, x) in [(old_target_id, -200.0), (viewed_target_id, 40.0)] {
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
    ai.base.primary_target = old_target_id.index();
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
    assert_eq!(
        (
            ai.base.primary_target,
            ai.base.last_stimulus_actor,
            ai.base.current_state,
            ai.base.current_substate,
            ai.forced_next_battle_decision,
        ),
        (
            viewed_target_id.index(),
            Some(viewed_target_id.index()),
            AiState::Attacking,
            Substate::AttackingSwordfight,
            Decision::None,
        )
    );
    assert_eq!(ai.base.current_state, AiState::Attacking);
    assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
}

#[test]
fn royalist_detection_think_opens_a_later_royalists_same_frame_view() {
    use crate::ai::{AiState, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, ElementData, ElementKind, Entity};

    fn observe(source_before_listener: bool) -> (bool, AiState, bool) {
        let mut engine = EngineInner::new();
        engine.add_entity(Entity::Target(crate::element::ElementTarget {
            element: ElementData {
                kind: ElementKind::Target,
                ..ElementData::default()
            },
            fx: Default::default(),
            target: Default::default(),
        }));

        let source = make_test_ai_soldier(Camp::Royalists);
        let listener = make_test_ai_soldier(Camp::Royalists);
        let (source_id, listener_id) = if source_before_listener {
            (engine.add_entity(source), engine.add_entity(listener))
        } else {
            let listener_id = engine.add_entity(listener);
            let source_id = engine.add_entity(source);
            (source_id, listener_id)
        };
        let target_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));

        for (id, x) in [(source_id, 0.0), (listener_id, 20.0), (target_id, 80.0)] {
            let Entity::Soldier(soldier) = engine
                .get_entity_mut(id)
                .expect("Royalist ordering soldier exists")
            else {
                panic!("Royalist ordering actor changed kind")
            };
            soldier.element.active = true;
            soldier
                .element
                .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
            soldier.element.set_position_map(MapPoint::new(x, 0.0));
            soldier.npc.life_points = 100;
            soldier.npc.view_radius = 200;
            soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
            if id == target_id {
                soldier.element.blipped = true;
            }
        }

        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);

        for id in [source_id, listener_id] {
            let Entity::Soldier(soldier) = engine
                .get_entity_mut(id)
                .expect("Royalist observer exists after fixture")
            else {
                panic!("Royalist observer changed kind after fixture")
            };
            let ai = soldier
                .npc
                .ai_brain
                .enemy_mut()
                .expect("Royalist observer has enemy AI");
            ai.base.me = id.index();
            ai.base.current_state = AiState::Default;
            ai.base.current_substate = Substate::DefaultOnPost;
            ai.base.current_music_alert_status = crate::ai::AlertLevel::Green;
            ai.current_task_priority = task_priority::NONE;
            ai.base.primary_target = 0;
            soldier.npc.detectable_lists[DetectableType::Enemy as usize].clear();
            soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
                element: Some(target_id),
                detectable_type: DetectableType::Enemy,
                ..Detectable::default()
            });
        }

        let Entity::Soldier(source) = engine
            .get_entity_mut(source_id)
            .expect("source Royalist exists")
        else {
            panic!("source Royalist changed kind")
        };
        source.element.set_direction_instantly(4);
        source.npc.view_direction = [1.0, 0.0];
        source.npc.eye_status = crate::element::EyeStatus::Stare;

        let Entity::Soldier(listener) = engine
            .get_entity_mut(listener_id)
            .expect("listener Royalist exists")
        else {
            panic!("listener Royalist changed kind")
        };
        listener.element.set_direction_instantly(4);
        listener.npc.view_direction = [1.0, 0.0];
        listener.npc.eye_status = crate::element::EyeStatus::LookForward;

        engine.control.frame_counter = 7;
        assert!(
            !(engine.control.frame_counter + listener_id.index())
                .is_multiple_of(crate::ai_vision::DETECTION_FREQUENCY_ENEMY_NPC),
            "listener fixture must start on a closed Royalist NPC detection gate"
        );
        crate::sim_rng::with_seed(0xA013_0B20, |sim| engine.tick_enemy_ai(sim, &assets));
        assert!(
            !engine
                .get_entity(target_id)
                .expect("Royalist target remains present")
                .element_data()
                .blipped,
            "Royalist HandleDetection must reveal its blipped NPC target at the detecting slot"
        );

        let source = engine
            .get_entity(source_id)
            .and_then(Entity::npc_data)
            .expect("source Royalist remains an NPC");
        let source_latch = source.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .find(|det| det.element == Some(target_id))
            .expect("source retains target detectable")
            .seen_last_frame;
        let source_ai = engine
            .get_entity(source_id)
            .and_then(Entity::enemy_ai)
            .expect("source Royalist retains enemy AI");
        assert_eq!(
            (source_latch, source_ai.base.current_state),
            (true, AiState::Attacking),
            "source Royalist must detect before its alert can test creation ordering"
        );

        let listener = engine
            .get_entity(listener_id)
            .and_then(Entity::npc_data)
            .expect("listener Royalist remains an NPC");
        let latch = listener.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .find(|det| det.element == Some(target_id))
            .expect("listener retains target detectable")
            .seen_last_frame;
        let ai = engine
            .get_entity(listener_id)
            .and_then(Entity::enemy_ai)
            .expect("listener Royalist retains enemy AI");
        (
            latch,
            ai.base.current_state,
            ai.base.primary_target == target_id.index(),
        )
    }

    let source_first = observe(true);
    assert_eq!(
        source_first,
        (true, AiState::Attacking, true),
        "the first Royalist's synchronous VIEW alert must turn and open detection for the later slot"
    );

    let listener_first = observe(false);
    assert_eq!(
        listener_first,
        (false, AiState::Wondering, false),
        "an earlier listener slot must not retroactively rescan after the later source alerts it"
    );
}

#[test]
fn royalist_detection_retains_every_ordered_view_edge_while_ai_locked() {
    use crate::ai::{AiLockFlags, AiState, StimulusInfo, StimulusType, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, Entity};

    let mut engine = EngineInner::new();
    let observer_id = engine.add_entity(make_test_ai_soldier(Camp::Royalists));
    let first_visible_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let lost_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let last_visible_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));

    for (id, x) in [
        (observer_id, 0.0),
        (first_visible_id, 80.0),
        (lost_id, 100.0),
        (last_visible_id, 120.0),
    ] {
        let Entity::Soldier(soldier) = engine
            .get_entity_mut(id)
            .expect("Royalist multi-edge soldier exists")
        else {
            panic!("Royalist multi-edge actor changed kind")
        };
        soldier.element.active = true;
        soldier
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
        soldier.element.set_position_map(MapPoint::new(x, 0.0));
        soldier.npc.life_points = 100;
        soldier.element.blipped = id != observer_id;
    }

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("Royalist multi-edge observer exists")
    else {
        panic!("Royalist multi-edge observer changed kind")
    };
    observer.element.set_direction_instantly(4);
    observer.npc.view_direction = [1.0, 0.0];
    observer.npc.view_radius = 300;
    observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    observer.npc.eye_status = crate::element::EyeStatus::Stare;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let Entity::Soldier(lost) = engine
        .get_entity_mut(lost_id)
        .expect("lost Royalist target exists after fixture")
    else {
        panic!("lost Royalist target changed kind after fixture")
    };
    // Original CleanUpDetectables removes dead enemies, not inactive living
    // ones. An inactive outdoor target remains in the list and emits the
    // falling OUTOFVIEW edge.
    lost.element.active = false;

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("Royalist multi-edge observer exists after fixture")
    else {
        panic!("Royalist multi-edge observer changed kind after fixture")
    };
    let ai = observer
        .npc
        .ai_brain
        .enemy_mut()
        .expect("Royalist multi-edge observer has enemy AI");
    ai.base.me = observer_id.index();
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultOnPost;
    ai.base.current_music_alert_status = crate::ai::AlertLevel::Green;
    ai.current_task_priority = task_priority::NONE;
    ai.base.locks_flag_field = AiLockFlags::BUSY;

    observer.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    for (target_id, seen_last_frame) in [
        (first_visible_id, false),
        (lost_id, true),
        (last_visible_id, false),
    ] {
        observer.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
            element: Some(target_id),
            detectable_type: DetectableType::Enemy,
            seen_last_frame,
            ..Detectable::default()
        });
    }

    crate::sim_rng::with_seed(0xA013_0B21, |sim| engine.tick_enemy_ai(sim, &assets));

    let observer = engine
        .get_entity(observer_id)
        .and_then(Entity::npc_data)
        .expect("Royalist multi-edge observer remains an NPC");
    assert_eq!(
        observer.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .map(|det| det.seen_last_frame)
            .collect::<Vec<_>>(),
        vec![true, false, true],
        "HandleDetection must settle every Royalist Enemy latch before Think"
    );
    assert!(
        !engine
            .get_entity(first_visible_id)
            .expect("first visible Royalist target remains present")
            .element_data()
            .blipped
            && !engine
                .get_entity(last_visible_id)
                .expect("last visible Royalist target remains present")
                .element_data()
                .blipped,
        "every rising Royalist Enemy edge must reveal its target before Think"
    );
    assert!(
        engine
            .get_entity(lost_id)
            .expect("lost Royalist target remains present")
            .element_data()
            .blipped,
        "a falling Royalist Enemy edge must not reveal its target"
    );
    let ai = engine
        .get_entity(observer_id)
        .and_then(Entity::enemy_ai)
        .expect("Royalist multi-edge observer retains enemy AI");
    assert!(ai.base.outbox.detection.stimuli.is_empty());
    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.base.last_stimulus_actor, Some(last_visible_id.index()));
    assert_eq!(
        ai.base
            .stimulus_queue
            .iter()
            .map(|stimulus| {
                let StimulusInfo::Human(target) = stimulus.info else {
                    panic!("Royalist Enemy edge lost its human payload")
                };
                (stimulus.stimulus_type, target)
            })
            .collect::<Vec<_>>(),
        vec![
            (StimulusType::EventView, first_visible_id.index()),
            (StimulusType::EventOutOfView, lost_id.index()),
            (StimulusType::EventView, last_visible_id.index()),
        ],
        "Royalist HandleDetection must retain interleaved edges in detectable-list order"
    );
}
