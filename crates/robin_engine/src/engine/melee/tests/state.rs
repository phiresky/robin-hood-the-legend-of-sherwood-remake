use super::*;

#[test]
fn strike_collector_angles_and_push_width_keep_original_conversions() {
    let expected_angle = ((7.0_f64 / 360.0) * 2.0 * f64::from(std::f32::consts::PI)) as f32;
    assert_eq!(strike_profile_angle(7).to_bits(), expected_angle.to_bits());
    assert_eq!(angle_to_sector(-std::f32::consts::PI / 8.0), 14);
    assert_eq!(push_strike_half_width(5), 2.0);
    assert_eq!(push_strike_half_width(6), 3.0);
}

#[test]
fn strike_estimation_collects_inactive_principal_only() {
    let attacker = EntityId::Pc(crate::element::PcId(1));
    let principal = EntityId::Soldier(crate::element::SoldierId(2));
    let bystander = EntityId::Soldier(crate::element::SoldierId(3));

    assert!(should_collect_strike_estimation_human(
        principal,
        attacker,
        Some(principal),
        false,
    ));
    assert!(!should_collect_strike_estimation_human(
        bystander,
        attacker,
        Some(principal),
        false,
    ));
    assert!(should_collect_strike_estimation_human(
        bystander,
        attacker,
        Some(principal),
        true,
    ));
    assert!(!should_collect_strike_estimation_human(
        attacker,
        attacker,
        Some(attacker),
        true,
    ));
}

#[test]
fn full_circle_done_seed_uses_inclusive_unprojected_3d_range() {
    let attacker = WorldPoint3D::ZERO;
    let elevated_same_map = WorldPoint3D::new(0.0, 50.0, 50.0);

    assert_eq!(attacker.to_map(), elevated_same_map.to_map());
    assert!(!full_circle_strike_distance_is_in_range(
        attacker,
        elevated_same_map,
        0.0,
        60.0,
    ));

    let ordinary = WorldPoint3D::new(3.0, 4.0, 12.0);
    assert_eq!(full_circle_strike_distance(attacker, ordinary), 13.0);
    assert!(full_circle_strike_distance_is_in_range(
        attacker, ordinary, 13.0, 13.0,
    ));
    assert!(!full_circle_strike_distance_is_in_range(
        attacker, ordinary, 13.01, 20.0,
    ));
}

#[test]
#[should_panic(expected = "straight-strike distance references missing victim Soldier")]
fn straight_strike_range_rejects_a_missing_victim() {
    let mut engine = EngineInner::new();
    let attacker = engine.add_test_entity(make_soldier(WorldPoint3D::ZERO, None));
    let missing = EntityId::Soldier(crate::entity_id::SoldierId(u32::MAX));

    let _ = entity_world_distance(&engine.world.entities, attacker, missing);
}

#[test]
fn fresh_selected_strike_uses_captured_stale_impossible_row_residue() {
    let mut engine = make_engine();
    let target = engine.add_test_entity(make_soldier(WorldPoint3D::ZERO, None));
    let selected_row = crate::sprite_script::SpriteScript {
        action_done: 2,
        frame_ids: vec![0, 1, 2],
        delays: vec![2, 2, 2],
        distances: vec![0; 3],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0; 3],
        ..Default::default()
    };
    let stale_row = crate::sprite_script::SpriteScript {
        action_done: u16::MAX,
        frame_ids: vec![0, 1],
        delays: vec![2, 2],
        distances: vec![0; 2],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 2],
        sound_ids: vec![0; 2],
        ..Default::default()
    };
    {
        let sprite = &mut engine
            .get_entity_mut(target)
            .unwrap()
            .element_data_mut()
            .sprite;
        let mut conversion =
            vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
        conversion[OrderType::StrikingRightSword as usize] = 0;
        sprite.scripts = std::sync::Arc::new(vec![selected_row, stale_row]);
        sprite.conversion = std::sync::Arc::new(conversion);
        sprite.current_row = 1;
        sprite.current_frame = 1;
        sprite.action_done_frame = u16::MAX;
        sprite.last_processed_order_id = 41;
    }

    let mut element =
        crate::sequence::SequenceElement::new(1, Command::SwordstrikeThrustE, Some(target));
    element.priority = crate::sequence::SequencePriority::Preference;
    let selected_order_id = engine.orders.allocate_order_id();
    element.orders.push_back(crate::order::Order::new(
        OrderType::StrikingRightSword,
        0.0,
        0.0,
        selected_order_id,
    ));
    let sequence_id = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);
    engine.publish_selected_order_as_installed(target);

    assert_eq!(
        engine.opponent_sword_strike_time_limit_for_actor(target, target),
        Some(i16::MIN),
        "replays without captured allocator residue use the reviewed strict impossible deadline"
    );
    assert_eq!(
        engine.enemy_reconsider_sword_strike_time_limit_for_actor(target, target),
        Some(1000),
        "historical enemy reconsideration treats its observed -1 as an unavailable action point"
    );

    let target_creation_order = engine.world.original_creation_order(target);
    let other_proposer = engine.add_test_entity(make_soldier(WorldPoint3D::ZERO, None));
    let other_creation_order = engine.world.original_creation_order(other_proposer);
    engine
        .control
        .original_impossible_action_done_deadlines
        .insert(
            (target_creation_order, target_creation_order),
            std::collections::VecDeque::from([-22478, 12]),
        );
    engine
        .control
        .original_impossible_action_done_deadlines
        .insert(
            (other_creation_order, target_creation_order),
            std::collections::VecDeque::from([30]),
        );
    assert_eq!(
        engine.opponent_sword_strike_time_limit_for_actor(target, target),
        Some(-22478),
        "schema-16 can carry the Original allocator-dependent wrapped SWORD"
    );
    assert_eq!(
        engine.opponent_sword_strike_time_limit_for_actor(target, target),
        Some(12),
        "repeated proposals against one target consume captured deadlines in invocation order"
    );
    assert_eq!(
        engine.opponent_sword_strike_time_limit_for_actor(other_proposer, target),
        Some(30),
        "different proposers targeting the same actor keep independent occurrence queues"
    );

    let sprite = &mut engine
        .get_entity_mut(target)
        .unwrap()
        .element_data_mut()
        .sprite;
    sprite.last_processed_order_id = selected_order_id.get();
    sprite.action_done_frame = u16::MAX;
    engine
        .control
        .original_impossible_action_done_deadlines
        .insert(
            (target_creation_order, target_creation_order),
            std::collections::VecDeque::from([17]),
        );
    assert_eq!(
        engine.opponent_sword_strike_time_limit_for_actor(target, target),
        Some(17),
        "a captured deadline also overrides the current row's impossible marker"
    );
    assert_eq!(
        engine.opponent_sword_strike_time_limit_for_actor(target, target),
        Some(i16::MIN),
        "the current sprite's impossible marker retains the strict S075 behavior"
    );
}

#[test]
fn ladder_fall_translation_retains_layer_goal_and_authors_landing_target() {
    let mut engine = make_engine();
    engine.scripts.mission = Some(empty_mission_script());

    let lift_sector = crate::sector::SectorNumber::new(42);
    let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
    level.sector_number_map.insert(lift_sector, 0);
    level.sectors.push(crate::fast_find_grid::GridSector {
        points: Vec::new(),
        bounding_box: crate::coordinates::MapBBox::new(),
        sector_type: crate::sector::SectorType::LIFT,
        layer: 0,
        sector_number: lift_sector,
        door_index: None,
        lift_type: Some(crate::sector::LiftType::Ladder),
        lift_direction: 0,
        force_crouched: false,
        building_index: None,
        low_exit_point: None,
        high_exit_point: None,
        lowest_door_index: Some(0),
        jump_line_indices: Vec::new(),
        gate_indices: Vec::new(),
        underlying_sector: None,
    });
    engine
        .script_domains
        .interactables
        .doors
        .push(crate::gate::Door {
            point_out: crate::coordinates::MapPoint::new(30.0, 0.0),
            layer_out: 3,
            sector_out: crate::sector::SectorNumber::new(7),
            sector_out_index: crate::fast_find_grid::SectorIndex::new(7),
            ..crate::gate::Door::default()
        });

    let victim = engine.add_test_entity(make_pc(
        WorldPoint3D::default(),
        crate::position_interface::SectorHandle::new(42),
    ));
    let damage = crate::sequence::SequenceElement::new(1, Command::ReceiveHitDamage, Some(victim));
    let sequence = engine.launch_element(damage);

    engine.translate_ladder_wall_fall(&LevelAssets::default(), victim, (sequence, 0));

    let victim_entity = engine.get_entity(victim).unwrap();
    assert_eq!(
        victim_entity.position_iface().layer_goal().get(),
        0,
        "translation must not publish the destination layer before arrival"
    );
    let flight = victim_entity
        .actor_data()
        .unwrap()
        .active_flight
        .as_ref()
        .expect("a non-trivial ladder fall installs a flight");
    assert_eq!(flight.goal_layer, 3);
    assert_eq!(
        flight.goal_sector,
        crate::position_interface::SectorHandle::new(7)
    );
    assert!(flight.ladder_fall);
}

#[test]
fn purse_brawl_knocks_out_allied_soldier_independently_of_diplomacy() {
    for (diplomacy_enabled, faction_wars) in
        [(false, false), (false, true), (true, false), (true, true)]
    {
        let mut engine = make_engine();
        engine
            .mission_domain
            .diplomacy
            .set_enabled(diplomacy_enabled);
        engine
            .mission_domain
            .diplomacy
            .set_npc_faction_wars(faction_wars);
        let null_slot = engine.add_test_entity(make_soldier(WorldPoint3D::ZERO, None));
        let attacker = engine.add_test_entity(make_soldier(WorldPoint3D::ZERO, None));
        let victim = engine.add_test_entity(make_soldier(WorldPoint3D::new(20.0, 0.0, 0.0), None));
        let assets = assets_with_sword_profile(1, 50);
        for id in [null_slot, attacker, victim] {
            engine
                .get_entity_mut(id)
                .unwrap()
                .enemy_ai_mut()
                .unwrap()
                .hth_weapon_id = 1;
        }
        engine
            .get_entity_mut(victim)
            .unwrap()
            .npc_data_mut()
            .unwrap()
            .life_points = 100;
        // Two original NPC punches cross the 70-concussion knockout threshold.
        for _ in 0..2 {
            let damage = crate::sequence::SequenceElement::new_damage(
                1,
                Command::ReceiveHitDamage,
                Some(victim),
                Some(attacker),
                0,
                40,
            );
            let sequence = engine.launch_element(damage);
            engine.apply_hit_damage(
                &crate::sim_rng::test_context(),
                &assets,
                victim,
                Some(attacker),
                40,
                false,
                (sequence, 0),
            );
        }
        let victim = engine.get_entity(victim).unwrap();
        assert!(victim.human_data().unwrap().unconscious);
        assert!(victim.ai_controller().unwrap().knocked_out_in_money_fight);
    }
}

#[test]
fn postponed_non_entry_strike_translates_after_antagonist_dies() {
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_soldier(WorldPoint3D::default(), None));
    let target = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 20.0,
            ..WorldPoint3D::default()
        },
        None,
    ));
    match engine.get_entity_mut(target).unwrap() {
        Entity::Pc(pc) => pc.pc.life_points = 0,
        _ => unreachable!("test target must remain a PC"),
    }

    for (command, strike, expected_order) in [
        (
            Command::SwordstrikeThrustB,
            SwordStrike::B,
            OrderType::StrikingStraightStrongSword,
        ),
        (
            Command::SwordstrikeThrustC,
            SwordStrike::C,
            OrderType::ExecutingSword,
        ),
    ] {
        let element = crate::sequence::SequenceElement::new_interaction(
            1,
            command,
            Some(attacker),
            Some(target),
        );
        let sequence = engine.launch_element(element);
        engine.dispatch_sword_strike(
            &LevelAssets::default(),
            attacker,
            target,
            strike,
            sequence,
            0,
        );

        let element = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap();
        assert_eq!(element.state, crate::sequence::SequenceState::InProgress);
        let order = element.current_order().unwrap();
        assert_eq!(order.order_type, expected_order);
        assert_eq!(order.antagonist, Some(target));
    }

    let thrust_a = crate::sequence::SequenceElement::new_interaction(
        1,
        Command::SwordstrikeThrustA,
        Some(attacker),
        Some(target),
    );
    let sequence = engine.launch_element(thrust_a);
    engine.dispatch_sword_strike(
        &LevelAssets::default(),
        attacker,
        target,
        SwordStrike::A,
        sequence,
        0,
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Impossible,
        "Thrust A must retain the swordfight-entry dead-target admission check"
    );
}

#[test]
fn special_strike_cancellation_closes_its_set_state_callback_boundary() {
    let mut engine = make_engine();
    let (attacker, _) = make_enemy_strike_pair(&mut engine, false);
    let assets = assets_with_sword_profile(7, 30);
    {
        let ai = engine
            .get_entity_mut(attacker)
            .and_then(Entity::enemy_ai_mut)
            .unwrap();
        ai.begin_special_strike();
        ai.base.outbox.reentrant.owner_work.clear();
    }

    engine.with_simulation_context(|engine, sim| {
        engine.tick_enemy_sword_attacks(sim, &assets);
    });

    let ai = engine
        .get_entity(attacker)
        .and_then(Entity::enemy_ai)
        .unwrap();
    assert!(!ai.pending_special_strike);
    assert_eq!(
        ai.base.current_substate,
        crate::ai::Substate::AttackingSwordfight
    );
    assert!(
        ai.base.outbox.reentrant.owner_work.is_empty(),
        "the cancellation state-change callback must run synchronously"
    );
}

#[test]
fn event_authorized_parade_reconsideration_reaches_strike_proposal() {
    let mut engine = make_engine();
    let (attacker, _) = make_enemy_strike_pair(&mut engine, true);
    let assets = assets_with_sword_profile(7, 30);
    {
        let Entity::Soldier(soldier) = engine.get_entity_mut(attacker).unwrap() else {
            unreachable!()
        };
        soldier.npc.ai_brain.base_mut().unwrap().current_substate =
            crate::ai::Substate::AttackingSwordfightParade;
        soldier.human.tiredness = TIREDNESS_WEAK_THRESHOLD;
        let crate::element::AiBrain::Enemy(ai) = &mut soldier.npc.ai_brain else {
            unreachable!()
        };
        ai.next_sword_strike_frame = u32::MAX;
        soldier.element.sprite.scripts =
            std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
                action_done: 0,
                frame_ids: vec![0],
                delays: vec![1],
                distances: vec![0],
                offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
                sound_ids: vec![0],
                ..Default::default()
            }]);
        soldier.element.sprite.conversion =
            std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
    }
    engine.control.rng = SimulationRng::with_original_replay(vec![85]);

    engine.with_simulation_context(|engine, sim| {
        engine.consume_pending_enemy_sword_attack_for(sim, &assets, attacker);
    });

    assert_eq!(
        engine.control.rng.original_replay_cursor(),
        Some(1),
        "swordfight reconsideration already passed the state, cooldown, and tiredness checks"
    );
    assert!(
        !engine
            .get_entity(attacker)
            .and_then(Entity::enemy_ai)
            .unwrap()
            .pending_sword_strike_consideration
    );
}

#[test]
fn reactive_counterstrike_uses_difficulty_modified_soldier_fighting_ability() {
    let mut engine = make_engine();
    engine.control.sim_config.difficulty = crate::player_profile::DifficultyLevel::Hard;
    let (victim, attacker) = make_enemy_strike_pair(&mut engine, false);
    for actor in [victim, attacker] {
        let sprite = &mut engine
            .get_entity_mut(actor)
            .unwrap()
            .element_data_mut()
            .sprite;
        sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
            action_done: 0,
            frame_ids: vec![0],
            delays: vec![1],
            distances: vec![0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
            ..Default::default()
        }]);
        sprite.conversion = std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
    }
    let mut assets = assets_with_sword_profile(7, 30);
    std::sync::Arc::get_mut(&mut assets.profile_manager)
        .unwrap()
        .soldiers[0]
        .fighting = 40;
    {
        let ai = engine
            .get_entity_mut(victim)
            .and_then(Entity::enemy_ai_mut)
            .unwrap();
        ai.known_enemy_strike_1 = Some(SwordStrike::A);
    }

    // The replay victim is still performing a selected smalltalk parry
    // when the reactive counterstrike replaces it. Stopping actions must mark the
    // interrupted element's condolence as coming from Halt; otherwise its
    // later EventDone immediately leaves the new SpecialStrike substate.
    let old_parry =
        engine
            .orders
            .sequence_manager
            .launch_element(crate::sequence::SequenceElement::new(
                1,
                Command::ParrySmalltalkLeft,
                Some(victim),
            ));
    engine
        .orders
        .sequence_manager
        .element_in_progress(old_parry, 0);

    // 65 rejects raw fighting 40 and produces a parade, but Hard's
    // Lacklandist modifier raises it to 80, allowing the counterstrike.
    engine.control.rng = SimulationRng::with_original_replay(vec![65]);
    engine.with_simulation_context(|engine, sim| {
        engine.consider_to_begin_parade(
            sim,
            &assets,
            victim,
            attacker,
            Some(SwordStrike::A),
            SwordStrike::A,
        );
        engine.dispatch_condolations(sim, &assets);
    });

    let ai = engine
        .get_entity(victim)
        .and_then(Entity::enemy_ai)
        .unwrap();
    assert!(ai.pending_special_strike);
    assert_eq!(
        ai.base.current_substate,
        crate::ai::Substate::AttackingSwordfightSpecialStrike
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(victim, Command::is_swordstrike)
    );
}

#[test]
fn reactive_zero_distance_step_back_completes_before_returning() {
    for rider in [false, true] {
        let mut engine = make_engine();
        let (victim, attacker) = make_enemy_strike_pair(&mut engine, false);
        let Entity::Soldier(victim_soldier) = engine.get_entity_mut(victim).unwrap() else {
            unreachable!()
        };
        victim_soldier.soldier.rider = rider;
        engine.world.fast_grid_mut().size_map(4, 4);
        engine.world.fast_grid_mut().allocate_layers(1);
        let sector_points = vec![
            crate::coordinates::MapPoint::new(0.0, 0.0),
            crate::coordinates::MapPoint::new(256.0, 0.0),
            crate::coordinates::MapPoint::new(256.0, 256.0),
            crate::coordinates::MapPoint::new(0.0, 256.0),
        ];
        engine.world.fast_grid_mut().add_sector(
            crate::fast_find_grid::GridSector {
                points: sector_points,
                bounding_box: crate::coordinates::MapBBox::from_coords(0.0, 0.0, 256.0, 256.0),
                sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
                layer: 0,
                sector_number: crate::sector::SectorNumber::new(0),
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
            },
            0,
        );
        {
            let victim_element = engine.get_entity_mut(victim).unwrap().element_data_mut();
            victim_element.set_position(WorldPoint3D::new(100.0, 100.0, 0.0));
            victim_element.set_sector(crate::position_interface::SectorHandle::new(0));
            victim_element.sprite.position_iface.set_move_box(
                crate::coordinates::MoveBox::from_coords(-5.0, -5.0, 5.0, 5.0),
            );
        }
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            .set_position(WorldPoint3D::new(180.0, 100.0, 0.0));
        for actor in [victim, attacker] {
            let sprite = &mut engine
                .get_entity_mut(actor)
                .unwrap()
                .element_data_mut()
                .sprite;
            sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
                action_done: 0,
                frame_ids: vec![0],
                delays: vec![1],
                distances: vec![0],
                offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
                sound_ids: vec![0],
                ..Default::default()
            }]);
            sprite.conversion =
                std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
        }

        // The replay victim is already moving with a sword. Stopping actions
        // stops movement, preserving only ordinary upright or
        // crouched locomotion; WalkingWithSword falls through to
        // INTERRUPTED and clears the actor order. The immediately following movement
        // therefore reads NONANIMATION_END. Since the attacker is already
        // beyond the desired separation, the proposed step-back goal is
        // the victim's current point and movement must take its synchronous
        // already-at-destination exit without publishing a replacement
        // movement.
        let mut old_movement = crate::sequence::SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(victim),
            OrderType::WalkingWithSword,
        );
        old_movement.priority = crate::sequence::SequencePriority::Normal;
        let old_sequence = engine.launch_element(old_movement);
        let old_order =
            engine.push_new_order(old_sequence, 0, OrderType::WalkingWithSword, 90.0, 100.0);
        engine
            .orders
            .sequence_manager
            .element_in_progress(old_sequence, 0);
        {
            let actor = engine
                .get_entity_mut(victim)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.action_state = ActionState::MovingSword;
            actor.installed_order = Some(crate::element::InstalledActorOrder {
                order_id: old_order,
                order_type: OrderType::WalkingWithSword,
            });
            actor.active_movement = crate::movement::ActiveMovement::new(old_sequence, 0);
        }
        let mut assets = assets_with_sword_profile(7, 30);
        let profiles = std::sync::Arc::get_mut(&mut assets.profile_manager).unwrap();
        profiles.soldiers[0].fighting = 50;
        let incoming_thrust = &mut profiles.hth_weapons[0].thrusts[SwordStrike::A as usize];
        incoming_thrust.kind = crate::profiles::WeaponThrustKind::PushAside;
        incoming_thrust.maximal_distance = 30;
        {
            let ai = engine
                .get_entity_mut(victim)
                .and_then(Entity::enemy_ai_mut)
                .unwrap();
            ai.known_enemy_strike_1 = Some(SwordStrike::A);
        }

        // 65 rejects an offensive response at fighting ability 50, selecting
        // the parade path. A push-aside strike turns that parade into a
        // step-back movement, which the original game launches synchronously before this
        // callback returns.
        engine.control.rng = SimulationRng::with_original_replay(vec![65]);
        engine.with_simulation_context(|engine, sim| {
            engine.consider_to_begin_parade(
                sim,
                &assets,
                victim,
                attacker,
                Some(SwordStrike::A),
                SwordStrike::A,
            );
        });

        let ai = engine
            .get_entity(victim)
            .and_then(Entity::enemy_ai)
            .unwrap();
        assert_eq!(
            ai.base.current_substate,
            crate::ai::Substate::AttackingSwordfight,
            "an already-at-goal step-back synchronously handles EVENT_REACHPOINT"
        );
        assert!(ai.base.timer_is_running);
        assert_eq!(ai.base.when_does_timer_ring, 20);
        assert!(
            ai.base
                .last_goto_flags
                .contains(crate::ai::GotoFlags::SWORD)
        );
        assert_eq!(
            ai.base.last_goto_flags.contains(crate::ai::GotoFlags::RUN),
            !rider,
            "Original's rider step-back omits GOTO_RUN"
        );
        let owned_elements: Vec<_> = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .filter(|element| element.owner == Some(victim))
            .map(|element| {
                (
                    element.command,
                    element.state,
                    element.current_order().map(|order| order.order_type),
                )
            })
            .collect();
        assert!(
            !owned_elements
                .iter()
                .any(|(command, _, _)| *command == Command::EnterSwordfight),
            "a soldier already in WaitingSword must not receive a spurious raise-sword prefix: {owned_elements:?}"
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .live_element_for_actor_matching(victim, |element| {
                    matches!(element.command, Command::Move | Command::MoveOk)
                })
                .is_none(),
            "the already-at-goal request must not leave stale movement that can displace a same-frame smalltalk parry: {owned_elements:?}"
        );
    }
}

#[test]
fn empty_true_circle_sweep_advances_until_rotation_complete() {
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    if let Some(actor) = engine.get_entity_mut(attacker).unwrap().actor_data_mut() {
        actor.sweep_state = Some(crate::movement::SweepState {
            pending_victims: Vec::new(),
            current_angle: 0.0,
            final_angle: std::f32::consts::PI * 2.0,
            rotation_per_frame: std::f32::consts::PI,
            direction: crate::profiles::WeaponThrustDirection::LeftToRight,
            strike: SwordStrike::H,
            strike_kind: crate::profiles::WeaponThrustKind::TrueCircle,
            ..Default::default()
        });
    }

    engine.tick_sweep_for(&LevelAssets::default(), attacker, false);
    assert!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .is_some(),
        "true-circle sweep with no victims must still rotate instead of clearing immediately"
    );

    engine.tick_sweep_for(&LevelAssets::default(), attacker, false);
    assert!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .is_some(),
        "the tick that reaches the final angle must retain it for the terminal Execute call"
    );

    engine.tick_sweep_for(&LevelAssets::default(), attacker, false);
    assert!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .is_none(),
        "empty true-circle sweep should clear after presenting its terminal angle"
    );
}

#[test]
fn lateral_seed_uses_ground_direction_instead_of_map_direction() {
    let mut engine = make_engine();
    // Both actors have the same ground Y, so the victim is due west
    // (sector 12).  Its lower elevation projects six units south in map
    // space, which moves the same vector into sector 11.
    let attacker = engine.add_test_entity(make_pc(WorldPoint3D::default(), None));
    let victim = engine.add_test_entity(make_soldier(WorldPoint3D::default(), None));
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 100.0,
        });
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D {
            x: -15.0,
            y: 100.0,
            z: 94.0,
        });
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(9);

    let mut assets =
        assets_with_nonstraight_profile(SwordStrike::E, crate::profiles::WeaponThrustKind::Lateral);
    let thrust = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
        [SwordStrike::E as usize];
    thrust.direction = crate::profiles::WeaponThrustDirection::RightToLeft;
    thrust.initial_angle = 45;
    thrust.final_angle = 90;

    let attacker_map = engine
        .get_entity(attacker)
        .unwrap()
        .element_data()
        .position_map();
    let victim_map = engine
        .get_entity(victim)
        .unwrap()
        .element_data()
        .position_map();
    let attacker_ground = engine
        .get_entity(attacker)
        .unwrap()
        .element_data()
        .position();
    let victim_ground = engine.get_entity(victim).unwrap().element_data().position();
    assert_eq!(
        crate::position_interface::vector_to_sector_0_to_15(
            victim_ground.x - attacker_ground.x,
            victim_ground.y - attacker_ground.y,
        ),
        12,
    );
    assert_eq!(
        crate::position_interface::vector_to_sector_0_to_15(
            victim_map.x - attacker_map.x,
            victim_map.y - attacker_map.y,
        ),
        11,
        "the old map-space seed would admit this victim at the arc boundary"
    );

    let victims = engine.execute_multi_target_strike(&assets, attacker, SwordStrike::E, Some(1));
    assert!(
        victims.is_empty(),
        "Original seeds lateral victims from ground-space direction, where this actor is sector 12 and outside sectors 5..=11"
    );
}

#[test]
fn interrupted_lateral_sweep_is_retained_and_rebound_by_next_strike() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let unreached_victim = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: -10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    let mut profile_manager = crate::profiles::ProfileManager::new();
    let mut weapon = crate::profiles::HtHWeaponProfile::default();
    let retained = &mut weapon.thrusts[SwordStrike::D as usize];
    retained.kind = crate::profiles::WeaponThrustKind::Lateral;
    retained.direction = crate::profiles::WeaponThrustDirection::RightToLeft;
    retained.minimal_distance = 0;
    retained.maximal_distance = 100;
    retained.rotation_angle = 5;
    retained.cutting = 1;
    let replacement = &mut weapon.thrusts[SwordStrike::E as usize];
    replacement.kind = crate::profiles::WeaponThrustKind::Lateral;
    replacement.direction = crate::profiles::WeaponThrustDirection::LeftToRight;
    replacement.minimal_distance = 0;
    replacement.maximal_distance = 100;
    replacement.rotation_angle = 90;
    replacement.cutting = 100;
    profile_manager.hth_weapons.push(weapon);
    profile_manager
        .characters
        .push(crate::profiles::CharacterProfile {
            hth_weapon_id: 1,
            ..crate::profiles::CharacterProfile::default()
        });
    profile_manager
        .soldiers
        .push(crate::profiles::SoldierProfile {
            hth_weapon_id: 1,
            ..crate::profiles::SoldierProfile::default()
        });
    let assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profile_manager),
        ..LevelAssets::default()
    };

    let retained_selection =
        install_test_melee_order(&mut engine, attacker, victim, SwordStrike::D, true);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .sweep_state = Some(crate::movement::SweepState {
        pending_victims: vec![victim, unreached_victim],
        initial_angle: 0.0,
        current_angle: 0.0,
        final_angle: -std::f32::consts::PI,
        rotation_per_frame: -5.0_f32.to_radians(),
        direction: crate::profiles::WeaponThrustDirection::RightToLeft,
        strike: SwordStrike::D,
        attacker_profile_idx: Some(1),
        gesture_quality: crate::player_command::GestureQuality::PERFECT,
        strike_kind: crate::profiles::WeaponThrustKind::Lateral,
    });

    engine.stop_owner_active_mechanics(attacker);
    let retained_after_interrupt = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("interrupting the D sequence must retain its human-owned sweep");
    assert_eq!(retained_after_interrupt.strike, SwordStrike::D);
    assert_eq!(
        retained_after_interrupt.pending_victims,
        vec![victim, unreached_victim]
    );

    let replacement_order_id = engine.orders.allocate_order_id();
    let replacement_element = engine
        .orders
        .sequence_manager
        .get_element_mut(retained_selection.seq_id, retained_selection.elem_idx)
        .expect("retained strike element exists");
    replacement_element.command = SwordStrike::E.to_command();
    let replacement_order = replacement_element
        .orders
        .front_mut()
        .expect("retained strike order exists");
    replacement_order.order_type = strike_to_animation(SwordStrike::E);
    replacement_order.antagonist = Some(victim);
    replacement_order.reseed_id(replacement_order_id);
    // A live replacement strike is published as the actor's installed
    // order during instruction handling; execution's Start path resolves the strike from
    // that installed animation, not from the sequence element.
    engine.publish_selected_order_as_installed(attacker);
    {
        let entity = engine.get_entity_mut(attacker).unwrap();
        let sprite = &mut entity.element_data_mut().sprite;
        sprite.scripts = std::sync::Arc::new(vec![
            crate::sprite_script::SpriteScript {
                action_done: 3,
                frame_ids: vec![0, 1, 2, 3],
                delays: vec![1, 1, 1, 1],
                distances: vec![0, 0, 0, 0],
                offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 4],
                sound_ids: vec![0; 4],
                ..Default::default()
            };
            16
        ]);
        sprite.conversion = std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
    }

    engine.tick_melee_strikes(sim, &assets);
    let retained_on_start = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("the replacement strike START must not consume the retained sweep");
    let replacement_direction_angle = sector_to_angle(
        engine
            .get_entity(attacker)
            .unwrap()
            .element_data()
            .direction(),
    );
    assert_eq!(retained_on_start.strike, SwordStrike::E);
    assert_eq!(
        retained_on_start.pending_victims,
        vec![victim, unreached_victim],
        "the START warning forecast rebases geometry but keeps the interrupted victim FIFO"
    );
    assert_eq!(retained_on_start.initial_angle, replacement_direction_angle);
    assert_eq!(retained_on_start.current_angle, replacement_direction_angle);
    assert_eq!(retained_on_start.final_angle, replacement_direction_angle);
    assert_eq!(soldier_life(&engine, victim), 50);

    engine.rebind_retained_sweep_to_active_strike(&assets, attacker);
    engine.tick_sweep_for(&assets, attacker, false);

    let retained_after_hit = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("a lateral sweep remains allocated until its animation terminates");
    assert_eq!(retained_after_hit.strike, SwordStrike::E);
    assert_eq!(
        retained_after_hit.direction,
        crate::profiles::WeaponThrustDirection::LeftToRight
    );
    assert!(
        (retained_after_hit.rotation_per_frame - std::f32::consts::FRAC_PI_2).abs() < f32::EPSILON,
        "the retained geometry must advance using E's rotation, not D's"
    );
}

#[test]
fn interrupted_push_victims_are_rebound_by_replacement_lateral_start() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    engine
        .get_entity_mut(victim)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .unconscious = true;
    engine
        .get_entity_mut(victim)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .hth_weapon_id = 1;
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .pending_push_swordfight = vec![victim];

    engine.stop_owner_active_mechanics(attacker);
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .pending_push_swordfight,
        vec![victim],
        "interrupting PushAside preserves Original's human-owned victim list"
    );

    let assets =
        assets_with_nonstraight_profile(SwordStrike::D, crate::profiles::WeaponThrustKind::Lateral);
    let selected = install_test_melee_order(&mut engine, attacker, victim, SwordStrike::D, false);
    engine.publish_selected_order_as_installed(attacker);
    {
        let sprite = &mut engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            .sprite;
        sprite.last_processed_order_id = u32::MAX;
        sprite.last_action = crate::order::OrderType::WaitingSword;
        sprite.current_frame = 0;
        sprite.frame_count = 0;
    }

    engine.tick_selected_melee_owner(&sim, &assets, attacker, selected);

    let actor = engine.get_entity(attacker).unwrap().actor_data().unwrap();
    assert_eq!(
        actor
            .sweep_state
            .as_ref()
            .expect("replacement lateral owns the retained push list")
            .pending_victims,
        vec![victim],
    );
    assert!(
        actor.pending_push_swordfight.is_empty(),
        "Original has one shared victim list, not duplicate push/sweep ownership"
    );
}

#[test]
fn interrupted_h_circle_runs_replacement_i_effect_without_advancing_geometry() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: -10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let unreached_victim = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 90.0,
            z: 0.0,
        },
        None,
    ));
    for target in [victim, unreached_victim] {
        engine
            .get_entity_mut(target)
            .and_then(Entity::enemy_ai_mut)
            .unwrap()
            .hth_weapon_id = 1;
    }

    let mut profile_manager = crate::profiles::ProfileManager::new();
    let mut weapon = crate::profiles::HtHWeaponProfile::default();
    for strike in [SwordStrike::H, SwordStrike::I] {
        let thrust = &mut weapon.thrusts[strike as usize];
        thrust.kind = crate::profiles::WeaponThrustKind::TrueCircle;
        thrust.minimal_distance = 0;
        thrust.maximal_distance = 100;
        thrust.initial_angle = 0;
        thrust.final_angle = 360;
        thrust.rotation_angle = 22;
    }
    weapon.thrusts[SwordStrike::H as usize].direction =
        crate::profiles::WeaponThrustDirection::LeftToRight;
    weapon.thrusts[SwordStrike::I as usize].direction =
        crate::profiles::WeaponThrustDirection::RightToLeft;
    profile_manager.hth_weapons.push(weapon);
    profile_manager
        .characters
        .push(crate::profiles::CharacterProfile {
            hth_weapon_id: 1,
            ..crate::profiles::CharacterProfile::default()
        });
    profile_manager
        .soldiers
        .push(crate::profiles::SoldierProfile {
            hth_weapon_id: 1,
            ..crate::profiles::SoldierProfile::default()
        });
    let assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profile_manager),
        ..LevelAssets::default()
    };

    let retained_selection =
        install_test_melee_order(&mut engine, attacker, victim, SwordStrike::H, true);
    let retained_initial_angle = 0.1;
    let retained_current_angle = 1.251_917_2;
    let retained_final_angle = std::f32::consts::TAU + retained_initial_angle;
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(7);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .sweep_state = Some(crate::movement::SweepState {
        pending_victims: vec![victim, unreached_victim],
        initial_angle: retained_initial_angle,
        current_angle: retained_current_angle,
        final_angle: retained_final_angle,
        rotation_per_frame: 0.383_972_44,
        direction: crate::profiles::WeaponThrustDirection::LeftToRight,
        strike: SwordStrike::H,
        attacker_profile_idx: Some(1),
        gesture_quality: crate::player_command::GestureQuality::PERFECT,
        strike_kind: crate::profiles::WeaponThrustKind::TrueCircle,
    });

    let replacement_order_id = engine.orders.allocate_order_id();
    let replacement_element = engine
        .orders
        .sequence_manager
        .get_element_mut(retained_selection.seq_id, retained_selection.elem_idx)
        .expect("retained strike element exists");
    replacement_element.command = SwordStrike::I.to_command();
    let replacement_order = replacement_element
        .orders
        .front_mut()
        .expect("retained strike order exists");
    replacement_order.order_type = strike_to_animation(SwordStrike::I);
    replacement_order.antagonist = Some(victim);
    replacement_order.reseed_id(replacement_order_id);
    engine.publish_selected_order_as_installed(attacker);
    {
        let entity = engine.get_entity_mut(attacker).unwrap();
        let sprite = &mut entity.element_data_mut().sprite;
        sprite.use_alternate_profile = false;
        sprite.scripts = std::sync::Arc::new(vec![
            crate::sprite_script::SpriteScript {
                action_done: 5,
                frame_ids: vec![0, 1, 2, 3, 4, 5, 6],
                delays: vec![1; 7],
                distances: vec![0; 7],
                offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 7],
                sound_ids: vec![0; 7],
                ..Default::default()
            };
            16
        ]);
        sprite.conversion = std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
    }

    engine.tick_melee_strikes(sim, &assets);
    {
        let sprite = &mut engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            .sprite;
        assert_eq!(sprite.action_done_frame, 5);
        sprite.current_frame = 2;
        sprite.frame_count = 0;
    }
    engine.tick_melee_strikes(sim, &assets);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .filter(|element| {
                element.command == Command::ReceiveSwordDamage && element.owner == Some(victim)
            })
            .count(),
        1,
        "the first replacement pre-action effect consumes the reached retained victim"
    );
    engine.tick_melee_strikes(sim, &assets);
    {
        let sprite = &mut engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            .sprite;
        sprite.current_frame = 4;
        sprite.frame_count = 1;
        assert_eq!(
            sprite.frames_from_now_till_action_done(),
            0,
            "the forecast can reach zero one tick before the exact action point"
        );
        assert_ne!(sprite.current_frame, sprite.action_done_frame);
    }
    engine.tick_selected_sweep_phase(&assets, attacker, strikes::SweepTickPhase::InProgress);

    let queued_damage: Vec<&crate::sequence::SequenceElement> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .filter(|element| {
            element.command == Command::ReceiveSwordDamage && element.owner == Some(victim)
        })
        .collect();
    assert_eq!(
        queued_damage.len(),
        1,
        "replacement I's right-to-left effect must reach the retained H victim before I's action point"
    );
    assert!(matches!(
        &queued_damage[0].data,
        crate::sequence::SequenceElementData::Damage {
            sword_strike: Some(SwordStrike::I),
            ..
        }
    ));

    let attacker_entity = engine.get_entity(attacker).unwrap();
    let retained = &attacker_entity.human_data().unwrap().sword_sweep;
    assert_eq!(retained.victims, vec![unreached_victim]);
    assert_eq!(retained.initial_angle, retained_initial_angle);
    assert_eq!(retained.current_angle, retained_current_angle);
    assert_eq!(retained.final_angle, retained_final_angle);
    let executable = attacker_entity
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("the unreached victim keeps retained geometry executable");
    assert_eq!(executable.strike, SwordStrike::I);
    assert_eq!(executable.current_angle, retained_current_angle);
    assert_eq!(
        attacker_entity
            .element_data()
            .sprite
            .frames_from_now_till_action_done(),
        0,
        "the zero-forecast pre-action effect must not advance the replacement sprite"
    );
    assert_eq!(
        attacker_entity.element_data().direction(),
        7,
        "the pre-action effect must not rotate the replacement sprite"
    );
}

#[test]
fn replacement_half_circle_start_rebases_retained_angles_before_effect() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let retained_victim = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: -10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let replacement_target = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    for target in [retained_victim, replacement_target] {
        engine
            .get_entity_mut(target)
            .and_then(Entity::enemy_ai_mut)
            .unwrap()
            .hth_weapon_id = 1;
    }
    let mut assets = assets_with_nonstraight_profile(
        SwordStrike::G,
        crate::profiles::WeaponThrustKind::TrueHalfCircle,
    );
    std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
        [SwordStrike::G as usize]
        .direction = crate::profiles::WeaponThrustDirection::RightToLeft;
    let selected = install_test_melee_order(
        &mut engine,
        attacker,
        replacement_target,
        SwordStrike::G,
        false,
    );
    // Actor instruction handling publishes the selected G order before
    // Execute reaches its START warning boundary. The low-level fixture
    // installs the sequence order directly, so mirror that publication.
    engine.publish_selected_order_as_installed(attacker);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(0);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .sweep_state = Some(crate::movement::SweepState {
        pending_victims: vec![retained_victim],
        // Stale left-to-right H geometry already spans the victim's
        // sector. Without the START warning-query rebase, G's first
        // IN_PROGRESS effect would queue a second sword hit.
        initial_angle: 0.0,
        current_angle: std::f32::consts::PI,
        final_angle: std::f32::consts::TAU,
        rotation_per_frame: std::f32::consts::FRAC_PI_2,
        direction: crate::profiles::WeaponThrustDirection::LeftToRight,
        strike: SwordStrike::H,
        attacker_profile_idx: Some(1),
        gesture_quality: crate::player_command::GestureQuality::PERFECT,
        strike_kind: crate::profiles::WeaponThrustKind::TrueCircle,
    });
    {
        // The shared order fixture advances to the action point for most
        // sweep tests. Rewind only its sprite identity so the established
        // selected-owner dispatcher observes G's real START boundary.
        let sprite = &mut engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            .sprite;
        sprite.last_processed_order_id = u32::MAX;
        sprite.last_action = crate::order::OrderType::WaitingSword;
        sprite.current_frame = 0;
        sprite.frame_count = 0;
    }

    // Human action execution warns for the strike on MotionState::Start. Its
    // half-circle victim query mutates the shared angles even though the
    // retained victim FIFO belongs to the interrupted H strike.
    engine.tick_selected_melee_owner(&sim, &assets, attacker, selected);
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .element_data()
            .sprite
            .last_motion_state,
        Some(crate::sprite::MotionState::Start),
        "the first selected-owner tick must exercise G's START warning boundary"
    );
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .element_data()
            .direction(),
        0,
        "replacement G must turn right while the stale H victim remains on the left"
    );

    let sweep = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("replacement G keeps the interrupted victim FIFO");
    assert_eq!(sweep.pending_victims, vec![retained_victim]);
    assert_eq!(sweep.strike, SwordStrike::G);
    assert_eq!(
        sweep.direction,
        crate::profiles::WeaponThrustDirection::RightToLeft
    );
    let replacement_direction_angle = sector_to_angle(0);
    assert!((sweep.initial_angle - replacement_direction_angle).abs() < f32::EPSILON);
    assert!((sweep.current_angle - replacement_direction_angle).abs() < f32::EPSILON);
    assert!(
        (sweep.final_angle - (replacement_direction_angle - std::f32::consts::PI)).abs()
            < f32::EPSILON
    );

    engine.tick_selected_melee_owner(&sim, &assets, attacker, selected);
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .element_data()
            .sprite
            .last_motion_state,
        Some(crate::sprite::MotionState::InProgress),
        "the second selected-owner tick must exercise retained G geometry before DONE"
    );
    assert!(
        !engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| {
                element.command == Command::ReceiveSwordDamage
                    && element.owner == Some(retained_victim)
            }),
        "G's first effect must use the START-rebased half-circle angles"
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .is_some(),
        "the focused effect check keeps the replacement strike selected"
    );
}

#[test]
fn terminal_true_circle_direction_is_presented_before_done_progresses() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let assets = assets_with_nonstraight_profile(
        SwordStrike::G,
        crate::profiles::WeaponThrustKind::TrueHalfCircle,
    );
    let selected = install_test_melee_order(&mut engine, attacker, victim, SwordStrike::G, true);

    let terminal_angle = sector_to_angle(13);
    {
        let entity = engine.get_entity_mut(attacker).unwrap();
        entity.element_data_mut().set_direction_instantly(15);
        let sprite = &mut entity.element_data_mut().sprite;
        assert_eq!(sprite.current_frame, sprite.action_done_frame);
        assert_eq!(sprite.frame_count, sprite.action_done_counter);
        entity.actor_data_mut().unwrap().sweep_state = Some(crate::movement::SweepState {
            pending_victims: Vec::new(),
            initial_angle: terminal_angle + std::f32::consts::PI,
            current_angle: terminal_angle,
            final_angle: terminal_angle,
            rotation_per_frame: -std::f32::consts::FRAC_PI_2,
            // Deliberately stale retained metadata: Original dispatches
            // terminal action semantics from the current G call.
            direction: crate::profiles::WeaponThrustDirection::RightToLeft,
            strike: SwordStrike::F,
            attacker_profile_idx: Some(1),
            gesture_quality: crate::player_command::GestureQuality::PERFECT,
            strike_kind: crate::profiles::WeaponThrustKind::FalseHalfCircle,
        });
    }

    engine.tick_selected_melee_owner(sim, &assets, attacker, selected);

    let attacker_entity = engine.get_entity(attacker).unwrap();
    assert_eq!(
        attacker_entity.element_data().direction(),
        13,
        "Original presents the terminal true-circle angle before the exact action-done call advances the sprite"
    );
    let sprite = &attacker_entity.element_data().sprite;
    let current_g_row = sprite
        .row_for_action(strike_to_animation(SwordStrike::G))
        .expect("current G animation remains mapped");
    assert_eq!(
        sprite.current_row,
        current_g_row + 13,
        "terminal presentation must force the current G animation row, not retained F"
    );
    assert_eq!(
        sprite.current_frame,
        sprite.action_done_frame + 1,
        "the zero-delay fixture advances one frame after presenting the terminal angle"
    );
    assert_eq!(
        sprite.frame_count, 0,
        "the zero-delay next frame begins at counter zero"
    );
    assert!(
        attacker_entity.actor_data().unwrap().sweep_state.is_none(),
        "the terminal presentation call clears an exhausted sweep mirror"
    );
}

#[test]
fn saved_empty_true_circle_sweep_is_rehydrated_and_rotates() {
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let target = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let mut assets = assets_with_nonstraight_profile(
        SwordStrike::H,
        crate::profiles::WeaponThrustKind::TrueCircle,
    );
    std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
        [SwordStrike::H as usize]
        .direction = crate::profiles::WeaponThrustDirection::RightToLeft;
    install_test_melee_order(&mut engine, attacker, target, SwordStrike::H, true);

    let current_angle = sector_to_angle(3);
    {
        let entity = engine.get_entity_mut(attacker).unwrap();
        entity.element_data_mut().set_direction_instantly(10);
        entity.human_data_mut().unwrap().sword_sweep = crate::element::HumanSwordSweepState {
            victims: Vec::new(),
            initial_angle: current_angle + std::f32::consts::FRAC_PI_2,
            current_angle,
            final_angle: current_angle - std::f32::consts::PI,
        };
        assert!(entity.actor_data().unwrap().sweep_state.is_none());
    }

    engine.rebind_retained_sweep_to_active_strike(&assets, attacker);

    let sweep = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("an empty loaded true-circle still owns executable angle state");
    assert!(sweep.pending_victims.is_empty());
    assert_eq!(sweep.current_angle.to_bits(), current_angle.to_bits());

    engine.tick_sweep_for(&assets, attacker, false);
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .element_data()
            .direction(),
        3,
        "true-circle sword-strike execution presents its saved angle even with no victims"
    );
}

#[test]
fn lateral_start_rebases_retained_serialized_human_victims() {
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 1_000.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let assets =
        assets_with_nonstraight_profile(SwordStrike::D, crate::profiles::WeaponThrustKind::Lateral);
    let selected = install_test_melee_order(&mut engine, attacker, victim, SwordStrike::D, false);
    engine.publish_selected_order_as_installed(attacker);
    {
        let entity = engine.get_entity_mut(attacker).unwrap();
        entity.human_data_mut().unwrap().sword_sweep = crate::element::HumanSwordSweepState {
            victims: vec![victim],
            initial_angle: 0.1,
            current_angle: 0.1,
            final_angle: 0.1 - std::f32::consts::PI,
        };
        assert!(entity.actor_data().unwrap().sweep_state.is_none());
        let sprite = &mut entity.element_data_mut().sprite;
        sprite.last_processed_order_id = u32::MAX;
        sprite.last_action = crate::order::OrderType::WaitingSword;
        sprite.current_frame = 0;
        sprite.frame_count = 0;
    }

    engine.tick_selected_melee_owner(&crate::sim_rng::test_context(), &assets, attacker, selected);

    let attacker = engine.get_entity(attacker).unwrap();
    assert_eq!(
        attacker.element_data().sprite.last_motion_state,
        Some(crate::sprite::MotionState::Start)
    );
    let direction_angle = sector_to_angle(attacker.element_data().direction());
    let sweep = attacker
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("the START warning must make the retained rider-charge list executable");
    assert_eq!(sweep.pending_victims, vec![victim]);
    assert_eq!(sweep.strike, SwordStrike::D);
    assert_eq!(sweep.initial_angle, direction_angle);
    assert_eq!(sweep.current_angle, direction_angle);
    assert_eq!(sweep.final_angle, direction_angle + std::f32::consts::PI);
    assert_eq!(
        attacker.human_data().unwrap().sword_sweep.victims,
        vec![victim],
        "rebasing geometry must preserve Original's shared victim FIFO"
    );
}

#[test]
fn terminated_lateral_sweep_cannot_rehydrate_into_a_fresh_strike() {
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let assets =
        assets_with_nonstraight_profile(SwordStrike::D, crate::profiles::WeaponThrustKind::Lateral);

    engine.initialize_sweep(
        &assets,
        attacker,
        SwordStrike::D,
        Some(1),
        crate::profiles::WeaponThrustKind::Lateral,
        vec![victim],
        crate::player_command::GestureQuality::PERFECT,
    );
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .sword_sweep
            .victims,
        vec![victim]
    );

    engine.complete_melee_strike(&assets, attacker, None, 0, SwordStrike::D, Some(1));

    let attacker_entity = engine.get_entity(attacker).unwrap();
    assert!(
        attacker_entity.actor_data().unwrap().sweep_state.is_none(),
        "termination clears the executable sweep"
    );
    assert!(
        attacker_entity
            .human_data()
            .unwrap()
            .sword_sweep
            .victims
            .is_empty(),
        "Original deletes the human-owned victim list on RHMOTION_TERMINATED"
    );

    install_test_melee_order(&mut engine, attacker, victim, SwordStrike::D, true);
    engine.rebind_retained_sweep_to_active_strike(&assets, attacker);
    assert!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .is_none(),
        "a fresh lateral strike must wait for its own action-done initialization"
    );
}

#[test]
fn later_circle_frame_tests_existing_angle_before_tail_advance() {
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_test_entity(make_soldier(
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
        pending_victims: vec![victim],
        initial_angle: 0.0,
        current_angle: 0.0,
        final_angle: std::f32::consts::PI,
        rotation_per_frame: std::f32::consts::FRAC_PI_2,
        direction: crate::profiles::WeaponThrustDirection::LeftToRight,
        strike: SwordStrike::F,
        attacker_profile_idx: Some(1),
        gesture_quality: crate::player_command::GestureQuality::PERFECT,
        strike_kind: crate::profiles::WeaponThrustKind::FalseHalfCircle,
    });

    let queued_damage_count = |engine: &EngineInner| {
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .filter(|element| {
                element.command == Command::ReceiveSwordDamage && element.owner == Some(victim)
            })
            .count()
    };

    engine.tick_sweep_for(&assets, attacker, false);
    assert_eq!(
        queued_damage_count(&engine),
        0,
        "the victim in the newly reached sector cannot be tested before the circle tail advance"
    );
    let sweep = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("pending final-sector victim must keep the sweep alive");
    assert!((sweep.current_angle - std::f32::consts::FRAC_PI_2).abs() < f32::EPSILON);

    engine.tick_sweep_for(&assets, attacker, false);
    assert_eq!(
        queued_damage_count(&engine),
        1,
        "the next IN_PROGRESS effect must test the angle reached by the prior tail advance"
    );
}

#[test]
fn lateral_advance_is_raw_and_does_not_use_circle_final_clamping() {
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
    let assets =
        assets_with_nonstraight_profile(SwordStrike::D, crate::profiles::WeaponThrustKind::Lateral);
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
        rotation_per_frame: 1.20,
        direction: crate::profiles::WeaponThrustDirection::LeftToRight,
        strike: SwordStrike::D,
        attacker_profile_idx: Some(1),
        gesture_quality: crate::player_command::GestureQuality::PERFECT,
        strike_kind: crate::profiles::WeaponThrustKind::Lateral,
    });

    engine.tick_sweep_for(&assets, attacker, false);

    let current = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("unreached victim keeps the lateral sweep observable")
        .current_angle;
    assert!(
        (current - 1.20).abs() < f32::EPSILON,
        "lateral Execute applies its signed rotation directly even past final_angle"
    );
}

#[test]
fn push_replacement_executes_without_advancing_retained_circle_sweep() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 140.0,
            z: 0.0,
        },
        None,
    ));
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .sprite
        .position_iface
        .set_move_box(crate::coordinates::MoveBox::from_corners(
            crate::coordinates::MapVec::new(-5.0, -5.0),
            crate::coordinates::MapVec::new(5.0, 5.0),
        ));
    let assets = assets_with_nonstraight_profile(
        SwordStrike::A,
        crate::profiles::WeaponThrustKind::PushAside,
    );
    let selected = install_test_melee_order(&mut engine, attacker, victim, SwordStrike::A, false);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(8);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .sweep_state = Some(crate::movement::SweepState {
        pending_victims: vec![victim],
        initial_angle: 2.0,
        current_angle: 5.5,
        final_angle: 5.5,
        rotation_per_frame: std::f32::consts::FRAC_PI_4,
        direction: crate::profiles::WeaponThrustDirection::LeftToRight,
        strike: SwordStrike::F,
        attacker_profile_idx: Some(1),
        gesture_quality: crate::player_command::GestureQuality::PERFECT,
        strike_kind: crate::profiles::WeaponThrustKind::TrueHalfCircle,
    });

    engine.tick_selected_melee_owner(sim, &assets, attacker, selected);

    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .element_data()
            .direction(),
        8,
        "PushAside must not present the retained F sweep's terminal direction"
    );
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .as_ref()
            .expect("PushAside leaves interrupted sweep storage dormant")
            .strike,
        SwordStrike::F,
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| {
                element.command == Command::ReceiveSwordDamage && element.owner == Some(victim)
            }),
        "the replacement PushAside must still execute and queue its damage"
    );
}

#[test]
fn push_strike_does_not_recover_antagonist_outside_rectangle() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let target = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 11.0,
            y: 80.0,
            z: 0.0,
        },
        None,
    ));
    let mut assets = assets_with_nonstraight_profile(
        SwordStrike::A,
        crate::profiles::WeaponThrustKind::PushAside,
    );
    std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
        [SwordStrike::A as usize]
        .repulsion = 20;
    let selected = install_test_melee_order(&mut engine, attacker, target, SwordStrike::A, false);

    assert_eq!(
        engine.tick_nonstraight_melee_for(sim, &assets, attacker, selected),
        strikes::SweepTickPhase::InProgress
    );

    assert!(
        !engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| {
                element.command == Command::ReceiveSwordDamage && element.owner == Some(target)
            }),
        "Original's PushAside scan rejects side projection 11 outside half-width 10 even when the actor is the interaction antagonist"
    );
    assert_eq!(soldier_life(&engine, target), 50);
}

#[test]
fn push_strike_does_not_inform_soldier_of_good_strike() {
    use crate::ai::{AiState, LogLineType, StimulusType, Substate};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    {
        let Entity::Soldier(soldier) = engine.get_entity_mut(attacker).unwrap() else {
            unreachable!()
        };
        let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
        ai.base.me = attacker.index();
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfightSpecialStrike;
        ai.hth_weapon_id = 1;
    }
    let assets = assets_with_nonstraight_profile(
        SwordStrike::H,
        crate::profiles::WeaponThrustKind::TrueCircle,
    );
    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::H, 1);
    let sequence_id = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);

    engine.apply_sword_damage(
        &sim,
        &assets,
        victim,
        Some(attacker),
        Some(SwordStrike::H),
        Some(1),
        (sequence_id, 0),
    );

    let ai = engine
        .get_entity(attacker)
        .unwrap()
        .ai_controller()
        .unwrap();
    assert!(
        !ai.ai_log.iter().any(|entry| {
            entry.line_type == LogLineType::Event
                && entry.info == StimulusType::EventGoodStrike as u16
        }),
        "sword-damage translation, and therefore EVENT_GOOD_STRIKE, is skipped for push strikes"
    );
    assert_eq!(
        ai.current_substate,
        Substate::AttackingSwordfightSpecialStrike
    );
}

#[test]
fn ordinary_cutting_strike_still_informs_soldier_of_good_strike() {
    use crate::ai::{AiState, LogLineType, StimulusType, Substate};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_soldier(WorldPoint3D::default(), None));
    let victim = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::default()
        },
        None,
    ));
    {
        let Entity::Soldier(soldier) = engine.get_entity_mut(attacker).unwrap() else {
            unreachable!()
        };
        soldier.human.opponents.push(victim);
        let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
        ai.base.me = attacker.index();
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfightSpecialStrike;
        ai.hth_weapon_id = 1;
    }
    engine
        .get_entity_mut(victim)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(attacker);
    let assets = assets_with_sword_profile(1, 50);
    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    let sequence_id = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);

    engine.apply_sword_damage(
        &sim,
        &assets,
        victim,
        Some(attacker),
        Some(SwordStrike::A),
        Some(1),
        (sequence_id, 0),
    );

    let ai = engine
        .get_entity(attacker)
        .unwrap()
        .ai_controller()
        .unwrap();
    assert!(ai.ai_log.iter().any(|entry| {
        entry.line_type == LogLineType::Event && entry.info == StimulusType::EventGoodStrike as u16
    }));
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![victim],
        "a conscious surviving victim must retain the swordfight"
    );
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![attacker]
    );
    let damage = engine
        .orders
        .sequence_manager
        .get_element(sequence_id, 0)
        .expect("cutting damage command remains registered");
    assert!(
        damage
            .orders
            .iter()
            .filter(|order| order.order_type != OrderType::Rolling)
            .all(|order| !order.compute_direction),
        "sword-damage translation disables direction computation on its cutting-hit order"
    );
}

#[test]
fn non_pc_helping_to_climb_still_informs_soldier_of_good_strike() {
    use crate::ai::{AiState, LogLineType, StimulusType, Substate};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_soldier(WorldPoint3D::default(), None));
    let victim = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::default()
        },
        None,
    ));
    {
        let Entity::Soldier(soldier) = engine.get_entity_mut(attacker).unwrap() else {
            unreachable!()
        };
        let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
        ai.base.me = attacker.index();
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfightSpecialStrike;
        ai.hth_weapon_id = 1;
    }
    engine
        .get_entity_mut(victim)
        .unwrap()
        .set_posture(Posture::HelpingToClimb);
    engine
        .get_entity_mut(victim)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .hth_weapon_id = 1;
    let assets = assets_with_sword_profile(1, 50);
    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    let sequence_id = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);

    engine.apply_sword_damage(
        &sim,
        &assets,
        victim,
        Some(attacker),
        Some(SwordStrike::A),
        Some(1),
        (sequence_id, 0),
    );

    let attacker_ai = engine
        .get_entity(attacker)
        .unwrap()
        .ai_controller()
        .unwrap();
    assert!(attacker_ai.ai_log.iter().any(|entry| {
        entry.line_type == LogLineType::Event && entry.info == StimulusType::EventGoodStrike as u16
    }));
}

#[test]
fn preexisting_unconscious_push_preserves_closed_eyes_without_replaying_ko() {
    use crate::ai::{LogLineType, StimulusType};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(WorldPoint3D::ZERO, None));
    let victim = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::ZERO
        },
        None,
    ));
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity
            .element_data_mut()
            .publish_order_posture(Posture::Upright);
        victim_entity.human_data_mut().unwrap().unconscious = true;
        victim_entity.npc_data_mut().unwrap().eye_status = EyeStatus::Closed;
        victim_entity.enemy_ai_mut().unwrap().hth_weapon_id = 1;
        victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
    }
    engine
        .get_entity_mut(victim)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(attacker);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(victim);
    let assets = assets_with_sword_profile(1, 50);
    let damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    let sequence = engine.orders.sequence_manager.launch_element(damage);

    assert!(engine.apply_push_effect(
        &sim,
        &assets,
        victim,
        attacker,
        &PushStrikeInfo { repulsion: 100 },
        crate::combat::SwordDamageResult::STUNNING_DAMAGE,
        (sequence, 0),
        false,
    ));

    let victim_entity = engine.get_entity(victim).unwrap();
    assert!(victim_entity.human_data().unwrap().unconscious);
    assert_eq!(
        victim_entity.npc_data().unwrap().eye_status,
        EyeStatus::Closed
    );
    assert_eq!(
        victim_entity
            .ai_controller()
            .unwrap()
            .ai_log
            .iter()
            .filter(|entry| {
                entry.line_type == LogLineType::Event
                    && entry.info == StimulusType::EventLoseConsciousness as u16
            })
            .count(),
        0,
        "push-damage translation must not replay concussion handling's conscious-to-unconscious callback"
    );
    assert_eq!(
        engine
            .feedback
            .titbit_manager
            .titbits()
            .iter()
            .filter(|titbit| {
                titbit.kind == crate::titbit::TitbitKind::UnconsciousStar
                    && titbit
                        .element_supplier
                        .is_some_and(|supplier| supplier.0 == victim.index())
            })
            .count(),
        0,
        "push damage must not recreate the existing unconscious star"
    );
    assert!(
        victim_entity.human_data().unwrap().opponents.is_empty(),
        "the animated translation removes the victim's opponent"
    );
    assert!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents
            .is_empty(),
        "the animated translation removes the reciprocal opponent"
    );
    assert_eq!(
        victim_entity
            .ai_controller()
            .unwrap()
            .ai_log
            .iter()
            .filter(|entry| {
                entry.line_type == LogLineType::Event
                    && entry.info == StimulusType::EventQuitSwordfight as u16
            })
            .count(),
        1,
        "pre-existing unconscious animated translation owns exactly one plain quit"
    );
    let orders = &engine
        .orders
        .sequence_manager
        .get_sequence(sequence)
        .unwrap()
        .elements[0]
        .orders;
    assert!(
        orders
            .iter()
            .any(|order| order.order_type == OrderType::FallingPushedWithSword),
        "the already-unconscious victim still receives the authored push animation"
    );
}

#[test]
fn parried_true_circle_still_queues_push_fall() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let assets = assets_with_nonstraight_profile(
        SwordStrike::H,
        crate::profiles::WeaponThrustKind::TrueCircle,
    );

    let mut damage_sequence = crate::sequence::Sequence::new();
    let mut damage_element =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage_element.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::H, 1);
    damage_sequence.append_element(damage_element);
    let damage_sequence_id = engine
        .orders
        .sequence_manager
        .launch_sequence(damage_sequence);
    engine
        .orders
        .sequence_manager
        .element_in_progress(damage_sequence_id, 0);
    engine
        .get_entity_mut(victim)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::ParryingSword;
    let victim_position_before = engine
        .get_entity(victim)
        .unwrap()
        .element_data()
        .position_map();
    let victim_moving_before = engine
        .get_entity(victim)
        .unwrap()
        .position_iface()
        .is_moving_map();

    engine.apply_sword_damage(
        &sim,
        &assets,
        victim,
        Some(attacker),
        Some(SwordStrike::H),
        Some(1),
        (damage_sequence_id, 0),
    );

    let damage = engine
        .orders
        .sequence_manager
        .get_element(damage_sequence_id, 0)
        .expect("parried push damage must retain its sequence element");
    assert_eq!(
        damage
            .orders
            .back()
            .expect("push damage queues a fall even when the hit is parried")
            .order_type,
        OrderType::FallingPushedWithSword
    );
    assert!(
        damage
            .orders
            .iter()
            .filter(|order| order.order_type != OrderType::Rolling)
            .all(|order| !order.compute_direction),
        "push damage disables direction computation on the falling-pushed order"
    );
    let victim_after_translation = engine.get_entity(victim).unwrap();
    assert_eq!(
        victim_after_translation.element_data().position_map(),
        victim_position_before,
        "push-damage translation only queues the falling order; pushed-fall execution owns movement"
    );
    assert_eq!(
        victim_after_translation.position_iface().is_moving_map(),
        victim_moving_before,
        "translation must not introduce movement before the falling order executes"
    );

    // Model the replay boundary: the damage element has authored a push
    // fall, but the victim's still-selected order is its postponed parry.
    // Takeoff must not initialize until pushed falling with a sword
    // becomes current and reports Start.
    engine
        .orders
        .sequence_manager
        .postpone_element(damage_sequence_id, 0);
    let mut parry_sequence = crate::sequence::Sequence::new();
    let mut parry_element =
        crate::sequence::SequenceElement::new(1, Command::ParrySword, Some(victim));
    parry_element.orders.push_back(crate::order::Order::new(
        OrderType::ParryingSword,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    ));
    parry_sequence.append_element(parry_element);
    let parry_sequence_id = engine
        .orders
        .sequence_manager
        .launch_sequence(parry_sequence);
    engine
        .orders
        .sequence_manager
        .element_in_progress(parry_sequence_id, 0);
    assert!(
        engine
            .get_entity(victim)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_flight
            .is_none(),
        "push-damage translation must not prepare takeoff eagerly"
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(victim)
            .map(|(_, _, order)| order.order_type),
        Some(OrderType::ParryingSword)
    );
    engine.tick_push_flights(&sim, &assets);
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .element_data()
            .position_map(),
        victim_position_before,
        "prepared push flight must wait behind the still-selected parry order"
    );

    engine
        .orders
        .sequence_manager
        .element_terminated(parry_sequence_id, 0);
    engine
        .orders
        .sequence_manager
        .element_in_progress(damage_sequence_id, 0);
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.set_posture(Posture::Flying);
        victim_entity
            .actor_data_mut()
            .unwrap()
            .execute_order_initialising = true;
    }
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(victim)
            .map(|(_, _, order)| order.order_type),
        Some(OrderType::FallingPushedWithSword)
    );
    let fall_script = crate::sprite_script::SpriteScript {
        action_id: OrderType::FallingBackSword as u16,
        action_done: 1,
        frame_ids: vec![1, 2],
        delays: vec![0, 0],
        distances: vec![0, 0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 2],
        sound_ids: vec![0, 0],
        ..Default::default()
    };
    let mut fall_conversion =
        vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
    fall_conversion[OrderType::FallingBackSword as usize] = 0;
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        let position_iface = victim_entity.element_data().sprite.position_iface.clone();
        let mut sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![fall_script; 16]),
            std::sync::Arc::new(fall_conversion),
        );
        sprite.position_iface = position_iface;
        victim_entity.element_data_mut().sprite = sprite;
    }
    let material_before_takeoff = engine.get_entity(victim).unwrap().element_data().material();
    engine.initialize_push_flight(
        &assets,
        victim,
        (damage_sequence_id, 0),
        OrderType::FallingPushedWithSword,
    );
    assert_eq!(
        engine.get_entity(victim).unwrap().element_data().material(),
        material_before_takeoff,
        "takeoff preparation installs only the goal obstacle/plane, not its material"
    );
    let rejected_flight = engine
        .get_entity(victim)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_flight
        .expect("takeoff preparation must retain a fully rejected flight");
    assert_eq!(rejected_flight.increment_x, 0.0);
    assert_eq!(rejected_flight.increment_y, 0.0);
    assert_eq!(rejected_flight.increment_z, 0.0);
    let accepted_increment = 1.0;
    engine
        .get_entity_mut(victim)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_flight = Some(ActiveFlight {
        increment_x: accepted_increment,
        goal_x: victim_position_before.x + 8.0,
        goal_y: victim_position_before.y,
        frames_remaining: 8,
        antagonist: Some(attacker),
        ..Default::default()
    });
    engine.tick_push_flights(&sim, &assets);
    let victim_after_fall_start = engine.get_entity(victim).unwrap();
    assert_eq!(
        victim_after_fall_start.element_data().posture(),
        Posture::Flying
    );
    assert_eq!(
        victim_after_fall_start.element_data().position_map(),
        crate::coordinates::MapPoint::new(
            victim_position_before.x + accepted_increment,
            victim_position_before.y
        ),
        "flight processing applies its first increment on the starting execution"
    );
    assert_eq!(
        victim_after_fall_start
            .actor_data()
            .unwrap()
            .active_flight
            .unwrap()
            .frames_remaining,
        7
    );

    engine
        .get_entity_mut(victim)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .execute_order_initialising = false;
    engine.tick_push_flights(&sim, &assets);
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .element_data()
            .position_map(),
        crate::coordinates::MapPoint::new(
            victim_position_before.x + 2.0 * accepted_increment,
            victim_position_before.y
        ),
        "the following Execute applies the second push-flight increment"
    );
    assert!(
        !engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(attacker, |command| {
                command == Command::Provoke
            }),
        "parried push strikes still skip the later provoke branch"
    );
}

#[test]
fn pushed_flight_starts_from_cached_takeoff_elevation_after_installing_goal_plane() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        crate::position_interface::SectorHandle::new(0),
    ));

    let sector_number = crate::sector::SectorNumber::new(0);
    let make_sector = |min: f32, max: f32| {
        let points = vec![
            crate::coordinates::MapPoint::new(min, min),
            crate::coordinates::MapPoint::new(max, min),
            crate::coordinates::MapPoint::new(max, max),
            crate::coordinates::MapPoint::new(min, max),
        ];
        crate::fast_find_grid::GridSector {
            bounding_box: crate::coordinates::MapBBox::from_coords(min, min, max, max),
            points,
            sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
            layer: 0,
            sector_number,
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
        }
    };
    let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
    level.sectors.push(make_sector(-1000.0, 1000.0));
    level.sectors.push(make_sector(2000.0, 3000.0));
    // The public-number map deliberately names the other duplicate. Original
    // carries the exact sector reference from the victim's position.
    level.sector_number_map.insert(sector_number, 1);

    // The landing projection is ten units above the takeoff point. An
    // empty test grid rejects the horizontal push, which isolates the
    // vertical takeoff behavior: installing this goal plane must
    // not eagerly lift the actor before flight processing's first increment.
    let mut obstacle = crate::sight_obstacle::SightObstacle::new(
        0,
        crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA,
    );
    obstacle.set_projection_area_ref(
        crate::position_interface::Layer::ZERO,
        crate::fast_find_grid::SectorIndex::new(0).unwrap(),
    );
    obstacle.obstacle_points = vec![
        crate::sight_obstacle::ObstaclePoint {
            x: -1000.0,
            y: -1000.0,
            z_top: 10.0,
            z_bottom: 0.0,
        },
        crate::sight_obstacle::ObstaclePoint {
            x: 1000.0,
            y: -1000.0,
            z_top: 10.0,
            z_bottom: 0.0,
        },
        crate::sight_obstacle::ObstaclePoint {
            x: 1000.0,
            y: 1000.0,
            z_top: 10.0,
            z_bottom: 0.0,
        },
        crate::sight_obstacle::ObstaclePoint {
            x: -1000.0,
            y: 1000.0,
            z_top: 10.0,
            z_bottom: 0.0,
        },
    ];
    obstacle.top_plane_points = [
        [-1000.0, -1000.0, 10.0],
        [1000.0, -1000.0, 10.0],
        [-1000.0, 1000.0, 10.0],
    ];
    obstacle.rebuild_geometry();

    let mut assets = assets_with_nonstraight_profile(
        SwordStrike::H,
        crate::profiles::WeaponThrustKind::TrueCircle,
    );
    assets.environment.static_sight_obstacles = std::sync::Arc::new(vec![obstacle]);

    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::H, 1);
    damage.orders.push_back(crate::order::Order::new(
        OrderType::FallingPushedWithSword,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    ));
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    engine
        .get_entity_mut(victim)
        .unwrap()
        .set_posture(Posture::Flying);

    engine.initialize_push_flight(
        &assets,
        victim,
        (sequence, 0),
        OrderType::FallingPushedWithSword,
    );
    let flight = engine
        .get_entity(victim)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_flight
        .expect("elevated landing plane must author a flight");
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .sprite
        .scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
        frame_ids: vec![0, 1],
        ..Default::default()
    }]);
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .position_iface()
            .get_elevation()
            .to_bits(),
        0.0_f32.to_bits(),
        "obstacle assignment must preserve takeoff preparation's cached starting 3D point"
    );

    engine.tick_push_flights(&sim, &assets);
    let position = engine
        .get_entity(victim)
        .unwrap()
        .position_iface()
        .get_position();
    assert_eq!(
        position.z.to_bits(),
        flight.increment_z.to_bits(),
        "the first flight tick advances from takeoff Z, not the landing plane"
    );
    assert_eq!(
        position.y.to_bits(),
        (100.0_f32 + flight.increment_y).to_bits(),
        "flight processing accumulates the authored world-space Y increment before re-projecting map Y"
    );
}

#[test]
fn thrust_a_promotes_clicked_secondary_opponent() {
    let mut engine = make_engine();
    let pc = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let current = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let clicked = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 20.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    if let Some(human) = engine.get_entity_mut(pc).unwrap().human_data_mut() {
        human.opponents = vec![current, clicked].into();
    }
    if let Some(human) = engine.get_entity_mut(clicked).unwrap().human_data_mut() {
        human.opponents = vec![current, pc].into();
    }
    engine
        .get_entity_mut(pc)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(8);
    let direction_before_dispatch = engine.get_entity(pc).unwrap().element_data().direction();
    let direction_goal_before_dispatch = engine
        .get_entity(pc)
        .unwrap()
        .position_iface()
        .get_direction_goal();

    let mut sequence = crate::sequence::Sequence::new();
    sequence.append_element(crate::sequence::SequenceElement::new_interaction(
        1,
        Command::SwordstrikeThrustA,
        Some(pc),
        Some(clicked),
    ));
    let seq_id = engine.launch_sequence(sequence);
    let action_state_before_dispatch = engine
        .get_entity(pc)
        .unwrap()
        .actor_data()
        .unwrap()
        .action_state;

    engine.dispatch_sword_strike(
        &LevelAssets::default(),
        pc,
        clicked,
        SwordStrike::A,
        seq_id,
        0,
    );
    assert_eq!(
        engine
            .get_entity(pc)
            .unwrap()
            .actor_data()
            .unwrap()
            .action_state,
        action_state_before_dispatch,
        "Instruct must not apply the Execute MotionState::Start WaitingSword transition"
    );
    assert_eq!(
        engine.get_entity(pc).unwrap().element_data().direction(),
        direction_before_dispatch,
        "strike translation must leave facing to the following Execute call"
    );
    assert_eq!(
        engine
            .get_entity(pc)
            .unwrap()
            .position_iface()
            .get_direction_goal(),
        direction_goal_before_dispatch,
        "strike translation must not install the Execute-time facing goal"
    );

    assert_eq!(
        engine
            .get_entity(pc)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![clicked, current],
        "thrust-A against an existing secondary opponent must make it principal"
    );
    assert_eq!(
        engine
            .get_entity(clicked)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![pc, current],
        "the attacker is also promoted as the target's principal opponent"
    );
}

#[test]
fn reconsider_rebalance_rejection_preserves_opponent_and_ai_primary_target() {
    use crate::ai::EnterSwordfightRequest;

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let owner = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let old_primary = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let replacement = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 20.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    if let Some(human) = engine.get_entity_mut(owner).unwrap().human_data_mut() {
        human.opponents = vec![old_primary].into();
    }
    if let Some(human) = engine.get_entity_mut(replacement).unwrap().human_data_mut() {
        human.unconscious = true;
    }
    let old_primary_handle = (0..3)
        .find(|slot| engine.world.entities.id_at_legacy_slot(*slot) == Some(old_primary))
        .expect("old primary PC must occupy a legacy entity slot");
    let replacement_handle = (0..3)
        .find(|slot| engine.world.entities.id_at_legacy_slot(*slot) == Some(replacement))
        .expect("replacement PC must occupy a legacy entity slot");
    let Entity::Soldier(soldier) = engine.get_entity_mut(owner).unwrap() else {
        unreachable!()
    };
    let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
    ai.base.primary_target = Some(AiEntityHandle::new(old_primary_handle));
    ai.base.outbox.actor.enter_swordfight = Some(EnterSwordfightRequest::Rebalance(
        AiEntityHandle::new(replacement_handle),
    ));

    engine.drain_pending_for_npc(&sim, owner, &LevelAssets::default());

    let Entity::Soldier(soldier) = engine.get_entity(owner).unwrap() else {
        unreachable!()
    };
    assert_eq!(soldier.human.opponents, vec![old_primary]);
    assert_eq!(
        soldier.npc.ai_brain.enemy().unwrap().base.primary_target,
        Some(AiEntityHandle::new(old_primary_handle)),
        "failed swordfight entry must preserve the old AI primary target"
    );
}

#[test]
fn reconsider_direct_entry_does_not_prepare_or_stop_opponent() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let initiator = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let opponent = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    let mut selected = crate::sequence::Sequence::new();
    selected.append_element(crate::sequence::SequenceElement::new(
        1,
        Command::Point,
        Some(opponent),
    ));
    let selected_id = engine.launch_sequence(selected);
    engine
        .orders
        .sequence_manager
        .element_in_progress(selected_id, 0);
    engine
        .get_entity_mut(opponent)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .continuation
        .motion_state = crate::sprite::MotionState::InProgress;

    assert!(engine.direct_enter_swordfight(&sim, &LevelAssets::default(), initiator, opponent,));

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(selected_id, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::InProgress,
        "direct swordfight entry must not run the preparation's stop action"
    );
    assert_eq!(
        engine
            .get_entity(opponent)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        crate::sprite::MotionState::InProgress
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched(opponent, Command::EnterSwordfight),
        "direct entry still queues the reciprocal Enter command"
    );
}

#[test]
fn deleting_final_opponent_synchronously_quits_soldier_ai() {
    use crate::ai::{AiState, LogLineType, StimulusType, Substate};
    use crate::profiles::{CharacterProfile, HtHWeaponProfile, ProfileManager, SoldierProfile};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let soldier = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let opponent = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    if let Some(human) = engine.get_entity_mut(soldier).unwrap().human_data_mut() {
        human.opponents = vec![opponent].into();
    }
    let Entity::Soldier(soldier_entity) = engine.get_entity_mut(soldier).unwrap() else {
        unreachable!()
    };
    let ai = soldier_entity.npc.ai_brain.enemy_mut().unwrap();
    ai.base.me = soldier.index();
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingSwordfightSpecialStrike;
    ai.hth_weapon_id = 1;
    let Entity::Soldier(opponent_entity) = engine.get_entity_mut(opponent).unwrap() else {
        unreachable!()
    };
    let opponent_ai = opponent_entity.npc.ai_brain.enemy_mut().unwrap();
    opponent_ai.base.me = opponent.index();
    opponent_ai.hth_weapon_id = 1;

    let mut profiles = ProfileManager::new();
    profiles.hth_weapons.push(HtHWeaponProfile::default());
    profiles.characters.push(CharacterProfile {
        hth_weapon_id: 1,
        ..CharacterProfile::default()
    });
    profiles.soldiers.push(SoldierProfile {
        hth_weapon_id: 1,
        hostile: true,
        ..SoldierProfile::default()
    });
    let assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::new()
    };

    assert!(engine.delete_opponent(&sim, &assets, soldier, opponent));

    let ai = engine.get_entity(soldier).unwrap().ai_controller().unwrap();
    assert_eq!(
        ai.current_substate,
        Substate::AttackingQuittingSwordfight,
        "opponent removal must synchronously deliver the final-opponent quit event"
    );
    assert!(ai.ai_log.iter().any(|entry| {
        entry.line_type == LogLineType::Event
            && entry.info == StimulusType::EventQuitSwordfight as u16
    }));
}

#[test]
fn elevated_domino_uses_world_ground_xy_not_projected_map_y() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let hitter = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 110.0,
            z: 0.0,
        },
        None,
    ));
    let flyer = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 10.0,
        },
        None,
    ));
    let victim = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 90.0,
            z: 18.0,
        },
        None,
    ));

    // The generic actor fixture finishes by authoring a map point, which
    // intentionally flattens actors without a level plane. Restore the
    // literal 3D positions needed by this elevated-flight boundary.
    engine
        .get_entity_mut(flyer)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 10.0,
        });
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D {
            x: 0.0,
            y: 90.0,
            z: 18.0,
        });

    give_flight(&mut engine, flyer, hitter, 0.0, -1.0, 5);
    let flight = engine
        .get_entity_mut(flyer)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_flight
        .as_mut()
        .expect("test flight remains active");
    flight.geometry = crate::element::FlightGeometry::World3d;
    flight.increment_z = 1.0;

    engine.tick_push_flights(sim, &LevelAssets::default());

    let flyer_element = engine.get_entity(flyer).unwrap().element_data();
    let victim_element = engine.get_entity(victim).unwrap().element_data();
    let world_y_delta = victim_element.position().y - flyer_element.position().y;
    let map_y_delta = victim_element.position_map().y - flyer_element.position_map().y;
    assert_eq!(world_y_delta, -9.0);
    assert_eq!(map_y_delta, -16.0);

    assert_eq!(
        count_domino_hits_for(&engine, victim, hitter),
        1,
        "world ground delta is 9 units after the flight step; projected map Y would incorrectly measure 16"
    );
}

#[test]
fn domino_skips_actors_behind_flight_direction() {
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
    // Sits "behind" the flyer relative to its +X motion.
    let behind = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 5.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    give_flight(&mut engine, flyer, hitter, 1.0, 0.0, 5);
    engine.tick_push_flights(sim, &LevelAssets::default());

    assert_eq!(
        count_domino_hits_for(&engine, behind, hitter),
        0,
        "actor behind the flyer should not be domino-hit (negative dot product)"
    );
}

#[test]
fn domino_respects_distance_radius() {
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
    // 16 map units away on the X axis — outside DOMINO_DISTANCE = 15.
    let far = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 26.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    give_flight(&mut engine, flyer, hitter, 1.0, 0.0, 5);
    engine.tick_push_flights(sim, &LevelAssets::default());

    assert_eq!(
        count_domino_hits_for(&engine, far, hitter),
        0,
        "actor outside DOMINO_DISTANCE must not be domino-hit"
    );
}

#[test]
fn domino_skips_non_upright_actors() {
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
    let mut lying_entity = make_soldier(
        WorldPoint3D {
            x: 16.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    );
    lying_entity.set_posture(Posture::Lying);
    let lying = engine.add_test_entity(lying_entity);

    give_flight(&mut engine, flyer, hitter, 1.0, 0.0, 5);
    engine.tick_push_flights(sim, &LevelAssets::default());

    assert_eq!(
        count_domino_hits_for(&engine, lying, hitter),
        0,
        "lying actor must not be domino-hit (filtered by Posture::Upright)"
    );
}

#[test]
fn no_domino_when_flight_has_no_antagonist() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let _hitter = engine.add_test_entity(make_pc(
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
    let candidate = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 16.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    // No antagonist — mirrors the rolling / ladder-wall fall path.
    let flyer_pos = engine
        .get_entity(flyer)
        .unwrap()
        .element_data()
        .position_map();
    if let Some(entity) = engine.world.entities.get_mut(flyer)
        && let Some(actor) = entity.actor_data_mut()
    {
        actor.active_flight = Some(ActiveFlight {
            increment_x: 1.0,
            increment_y: 0.0,
            goal_x: flyer_pos.x + 5.0,
            goal_y: flyer_pos.y,
            frames_remaining: 5,
            antagonist: None,
            ..Default::default()
        });
    }

    engine.tick_push_flights(sim, &LevelAssets::default());

    let any_hit = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|s| s.elements.iter())
        .any(|e| e.command == Command::ReceiveHitDamage && e.owner == Some(candidate));
    assert!(
        !any_hit,
        "antagonist=None flight (rolling / ladder-fall) must not domino"
    );
}

#[test]
fn apply_concussion_uses_pc_profile_wake_up() {
    use crate::engine::LevelAssets;
    use crate::profiles::{CharacterProfile, CharacterProfileIdx, ProfileManager};

    const PC_WAKE_UP: u16 = 555;

    let mut engine = make_engine();
    // Forest of Barnsdale/Charnwood/Ashby missions use the forest proto
    // flag too, but only the Sherwood HQ mission grants PC immunity.
    engine.world.weather.is_forest_level = true;

    // PC with profile_index 0 — `make_pc` defaults to that.
    let pc_id = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        None,
    ));
    // Sanity: the helper does default to index 0.
    assert_eq!(
        engine
            .get_entity(pc_id)
            .unwrap()
            .pc_data()
            .unwrap()
            .profile_index,
        CharacterProfileIdx(0)
    );

    // Build a `LevelAssets` whose `ProfileManager` has a single PC
    // profile at index 0 with a distinctive `wake_up`.
    let mut profile_manager = ProfileManager::new();
    profile_manager.characters.push(CharacterProfile {
        wake_up: PC_WAKE_UP,
        ..CharacterProfile::default()
    });
    let assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profile_manager),
        ..LevelAssets::default()
    };

    // Drive the cheat-equivalent call: 100 concussion → KO →
    // healing-timeout init.
    let outcome =
        engine.apply_concussion(&crate::sim_rng::test_context(), &assets, pc_id, 100, false);
    assert_eq!(outcome, crate::combat::ConcussionOutcome::WentUnconscious);

    let timeout = engine
        .get_entity(pc_id)
        .unwrap()
        .human_data()
        .unwrap()
        .concussion_healing_timeout;
    assert_eq!(
        timeout, PC_WAKE_UP,
        "cheat-driven KO on a PC must seed `concussion_healing_timeout` with \
             the PC profile's `wake_up`, not the soldier fallback constant \
             ({SOLDIER_CONCUSSION_HEALING_SPEED})"
    );
}

#[test]
fn concussion_context_uses_campaign_description_identity_not_ui_list_index() {
    let mut engine = make_engine();
    engine.mission_domain.campaign.characters = vec![
        crate::campaign::PcDescription {
            character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
            status: crate::pc_status::PcStatus {
                in_coma: true,
                ..Default::default()
            },
            ..Default::default()
        },
        crate::campaign::PcDescription {
            character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
            status: crate::pc_status::PcStatus {
                in_coma: false,
                ..Default::default()
            },
            ..Default::default()
        },
    ];

    let mut pc = make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        None,
    );
    let pc_data = pc.pc_data_mut().unwrap();
    pc_data.list_index = 0;
    pc_data.campaign_description_index = Some(1);

    let ctx = concussion_ctx_full(
        &pc,
        false,
        Some(&engine.mission_domain.campaign),
        engine.control.sim_config.difficulty,
    );
    assert!(
        !ctx.is_in_coma,
        "the UI list index must not borrow another same-profile PC's coma status"
    );

    engine.mission_domain.campaign.characters[1].status.in_coma = true;
    let ctx = concussion_ctx_full(
        &pc,
        false,
        Some(&engine.mission_domain.campaign),
        engine.control.sim_config.difficulty,
    );
    assert!(ctx.is_in_coma);
}

#[test]
fn evaluated_step_back_aborted_before_motion_terminal_preserves_history() {
    let mut engine = make_engine();
    let owner = engine.add_test_entity(make_pc(WorldPoint3D::default(), None));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .last_motion_was_step_back_in_combat = false;

    engine.launch_evaluated_step_back(owner, crate::coordinates::MapPoint::new(12.0, 0.0), 0);
    let (sequence_id, element_index) = engine
        .orders
        .sequence_manager
        .live_element_for_actor_matching(owner, |element| {
            element.movement_flags_for_test().is_some_and(|flags| {
                flags.contains(crate::sequence::MoveFlags::STEP_BACK_IN_COMBAT)
            })
        })
        .expect("evaluated step-back movement must be registered");

    engine
        .orders
        .sequence_manager
        .element_impossible(sequence_id, element_index);
    assert!(
        !engine
            .get_entity(owner)
            .unwrap()
            .human_data()
            .unwrap()
            .last_motion_was_step_back_in_combat,
        "requesting and then aborting a step-back before RHMOTION_TERMINATED must not publish completed-step history"
    );
}

#[test]
fn unselected_rescue_pc_proposes_strike_before_step_back() {
    let mut engine = make_engine();
    let owner = engine.add_test_entity(make_pc(
        WorldPoint3D::new(0.0, 100.0, 0.0),
        crate::position_interface::SectorHandle::new(0),
    ));
    let opponent = engine.add_test_entity(make_soldier(
        WorldPoint3D::new(10.0, 100.0, 0.0),
        crate::position_interface::SectorHandle::new(0),
    ));

    {
        let Entity::Pc(pc) = engine.get_entity_mut(owner).unwrap() else {
            unreachable!()
        };
        pc.actor.action_state = ActionState::WaitingSword;
        pc.human.opponents.push(opponent);
        pc.human.smalltalk_initiative = true;
        pc.human.received_smalltalk_initiative = true;
        pc.pc.command_interface = crate::human_control::CommandInterface::None;
        pc.pc.mission_role = crate::human_control::MissionRole::RescueTarget;
        pc.element.sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
            action_done: 0,
            frame_ids: vec![0],
            delays: vec![1],
            distances: vec![0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
            ..Default::default()
        }]);
        pc.element.sprite.conversion =
            std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
    }
    {
        let Entity::Soldier(soldier) = engine.get_entity_mut(opponent).unwrap() else {
            unreachable!()
        };
        soldier.actor.action_state = ActionState::WaitingSword;
        soldier.human.opponents.push(owner);
        soldier.npc.ai_brain.enemy_mut().unwrap().hth_weapon_id = 1;
        soldier.element.sprite.scripts =
            std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
                action_done: 0,
                frame_ids: vec![0],
                delays: vec![1],
                distances: vec![0],
                offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
                sound_ids: vec![0],
                ..Default::default()
            }]);
        soldier.element.sprite.conversion =
            std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
    }

    let mut assets = assets_with_sword_profile(7, 30);
    let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
    profiles.characters[0].fighting = 1;
    profiles.soldiers[0].fighting = 100;
    profiles.hth_weapons[0].distance[crate::weapons::WeaponDistance::Uber as usize] = 30;

    // The first draw accepts strike A. The second reaches the step-back
    // decision and rejects the one-point friendly strength against the
    // opponent's 100 points. Original proposes the strike first even though
    // this rescue PC owns no player command interface.
    engine.control.rng = SimulationRng::with_original_replay(vec![0, 99, 0]);
    engine.with_simulation_context(|engine, sim| {
        engine.tick_waiting_sword_execute_for(sim, &assets, owner);
    });
    let draws = engine
        .control
        .rng
        .original_replay_sites(0..3)
        .expect("the deterministic replay RNG must retain its call-site history");

    assert_eq!(
        draws,
        vec![
            crate::sim_rng::RngSite::SwordStrikeSelection,
            crate::sim_rng::RngSite::MeleeStepBack,
            crate::sim_rng::RngSite::SmalltalkStrikeSide,
        ],
        "the unselected autonomous PC must propose its strike before evaluating step-back"
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(owner, Command::is_swordstrike),
        "the accepted proposal must launch a strike for a RescueTarget PC without hero commands"
    );
}
