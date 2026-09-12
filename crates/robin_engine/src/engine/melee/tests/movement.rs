use super::*;

#[test]
fn sector_to_angle_keeps_original_double_intermediate_rounding() {
    let direction_angle = sector_to_angle(9);
    assert_eq!(direction_angle.to_bits(), 0x4068_983d);

    // Profile angle queries likewise narrow 45 degrees to this single-precision value.
    // Three rotation ticks must reach the true-half-circle final angle
    // exactly, allowing true-circle sword-strike execution to resume the
    // sprite animation on the terminal-direction update.
    let quarter_turn = ((45.0_f64 / 360.0) * 2.0 * f64::from(std::f32::consts::PI)) as f32;
    let initial_angle = direction_angle - quarter_turn;
    let final_angle = initial_angle + std::f32::consts::PI;
    let after_three_ticks = direction_angle + quarter_turn + quarter_turn + quarter_turn;

    assert_eq!(after_three_ticks.to_bits(), final_angle.to_bits());
    assert_eq!(final_angle.to_bits(), 0x40bf_b210);
}

#[test]
fn push_warning_and_done_effect_keep_distinct_elevation_and_max_norm_gates() {
    assert!(push_strike_elevation_allows(
        PushStrikePositionSpace::Map,
        0.0,
        80.0,
    ));
    assert!(!push_strike_elevation_allows(
        PushStrikePositionSpace::Ground,
        0.0,
        80.0,
    ));

    // Push-strike execution stores the absolute elevation difference in an unsigned 32-bit value,
    // so the fractional part is discarded before the <= 40.f gate.
    assert!(push_strike_elevation_allows(
        PushStrikePositionSpace::Ground,
        0.0,
        40.75,
    ));
    assert!(!push_strike_elevation_allows(
        PushStrikePositionSpace::Ground,
        0.0,
        41.0,
    ));

    assert!(push_strike_max_norm_allows(
        PushStrikePositionSpace::Map,
        149.999,
        0.0,
    ));
    assert!(!push_strike_max_norm_allows(
        PushStrikePositionSpace::Map,
        150.0,
        0.0,
    ));
    assert!(push_strike_max_norm_allows(
        PushStrikePositionSpace::Ground,
        160.0,
        0.0,
    ));
}

#[test]
fn half_circle_done_seed_combines_3d_range_with_ground_space_sector() {
    let attacker = WorldPoint3D::ZERO;
    let elevated_same_map = WorldPoint3D::new(0.0, 10.0, 10.0);

    assert_eq!(attacker.to_map(), elevated_same_map.to_map());
    assert!(half_circle_strike_seed_allows(
        attacker,
        elevated_same_map,
        0.0,
        20.0,
        8,
        8,
    ));
    assert!(!half_circle_strike_seed_allows(
        attacker,
        elevated_same_map,
        0.0,
        10.0,
        8,
        8,
    ));

    let exact_boundary = WorldPoint3D::new(3.0, 4.0, 12.0);
    assert!(half_circle_strike_seed_allows(
        attacker,
        exact_boundary,
        13.0,
        13.0,
        0,
        15,
    ));
}

#[test]
fn straight_strike_range_uses_stored_world_position() {
    let mut engine = EngineInner::new();
    let attacker = engine.add_test_entity(make_soldier(WorldPoint3D::ZERO, None));
    let target = engine.add_test_entity(make_soldier(WorldPoint3D::ZERO, None));
    engine
        .get_entity_mut(target)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D::new(60.0, 40.0, 40.0));

    // The isometric projection subtracts elevation from world Y, so
    // these actors are only 60 map units apart while Original's
    // Straight sword-strike range checking sees all three components.
    assert_eq!(
        entity_distance(&engine.world.entities, attacker, target),
        60.0
    );
    assert_eq!(
        entity_world_distance(&engine.world.entities, attacker, target),
        (60.0_f32 * 60.0 + 40.0 * 40.0 + 40.0 * 40.0).sqrt()
    );
}

#[test]
fn thrust_a_accepts_an_existing_opponent_during_ordinary_door_transit() {
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(
        WorldPoint3D::default(),
        crate::position_interface::SectorHandle::new(42),
    ));
    let target = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 20.0,
            ..WorldPoint3D::default()
        },
        crate::position_interface::SectorHandle::new(43),
    ));
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(target);
    {
        let target_entity = engine.get_entity_mut(target).unwrap();
        target_entity
            .human_data_mut()
            .unwrap()
            .opponents
            .push(attacker);
        let target_actor = target_entity.actor_data_mut().unwrap();
        target_actor.active_door_pass = Some(crate::element::ActiveDoorPass {
            door_index: crate::gate::DoorIndex::new(7).expect("valid door index"),
            direct: true,
            position_direct: true,
            steps: std::collections::VecDeque::new(),
            preallocated_order_ids: Default::default(),
            triggers_fired: 0,
            current_action: OrderType::WalkingWithSword,
            current_reverse: false,
            saved_action_state: None,
        });
        target_entity.position_iface_mut().set_door_for_test(
            crate::position_interface::DoorHandle::new(7).expect("valid door index"),
        );
    }
    assert!(engine.get_entity(target).unwrap().is_in_door_transit());

    let assets = assets_with_sword_profile(1, 50);
    assert!(can_enter_swordfight_with(
        &engine.world.entities,
        attacker,
        target,
        &assets.profile_manager,
        &engine.world.fast_grid,
    ));

    let strike = crate::sequence::SequenceElement::new_interaction(
        1,
        Command::SwordstrikeThrustA,
        Some(attacker),
        Some(target),
    );
    let sequence = engine.launch_element(strike);
    engine.dispatch_sword_strike(&assets, attacker, target, SwordStrike::A, sequence, 0);

    let element = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .unwrap();
    assert_eq!(element.state, crate::sequence::SequenceState::InProgress);
    let order = element.current_order().unwrap();
    assert_eq!(order.order_type, OrderType::StrikingStraightSword);
    assert_eq!(order.antagonist, Some(target));
}

#[test]
fn circle_tail_retains_candidate_past_final_in_the_same_sector() {
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let pending_victim = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let assets = assets_with_nonstraight_profile(
        SwordStrike::F,
        crate::profiles::WeaponThrustKind::FalseHalfCircle,
    );
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .sweep_state = Some(crate::movement::SweepState {
        pending_victims: vec![pending_victim],
        initial_angle: 0.0,
        current_angle: 0.0,
        final_angle: 0.70,
        rotation_per_frame: 0.75,
        direction: crate::profiles::WeaponThrustDirection::LeftToRight,
        strike: SwordStrike::F,
        attacker_profile_idx: Some(1),
        gesture_quality: crate::player_command::GestureQuality::PERFECT,
        strike_kind: crate::profiles::WeaponThrustKind::FalseHalfCircle,
    });

    engine.tick_sweep_for(&assets, attacker, false);

    let current = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("unreached victim keeps the circle sweep observable")
        .current_angle;
    assert!(
        (current - 0.75).abs() < f32::EPSILON,
        "a candidate past 0.70 in the same final sector must be retained instead of clamped"
    );
}

#[test]
fn domino_propagates_to_actors_in_flight_path() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let hitter = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let flyer = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let mid = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 16.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let far = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 22.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    // 5 frames of +X motion at 1 unit per frame — short enough to
    // stay inside DOMINO_DISTANCE for the front pair.
    give_flight(&mut engine, flyer, hitter, 1.0, 0.0, 5);

    engine.tick_push_flights(sim, &LevelAssets::default());

    assert_eq!(
        count_domino_hits_for(&engine, mid, hitter),
        1,
        "soldier directly in front should take a domino hit"
    );
    assert_eq!(
        count_domino_hits_for(&engine, far, hitter),
        1,
        "soldier further along the flight axis should also take a domino hit"
    );
    assert_eq!(
        count_domino_hits_for(&engine, hitter, hitter),
        0,
        "hitter must never domino itself"
    );
    assert_eq!(
        count_domino_hits_for(&engine, flyer, hitter),
        0,
        "flyer is not its own domino victim"
    );
}
