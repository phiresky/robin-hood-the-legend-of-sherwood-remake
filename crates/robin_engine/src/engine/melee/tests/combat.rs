use super::*;

#[test]
fn sword_strike_range_rejects_nan_like_original_positive_comparisons() {
    assert!(!sword_strike_distance_is_in_range(f32::NAN, 0.0, 65.0));
    assert!(sword_strike_distance_is_in_range(0.0, 0.0, 65.0));
    assert!(sword_strike_distance_is_in_range(65.0, 0.0, 65.0));
}

#[test]
fn sweep_state_uses_angles_returned_by_original_sword_getters() {
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(WorldPoint3D::ZERO, None));
    let victim = engine.add_test_entity(make_soldier(WorldPoint3D::new(10.0, 0.0, 0.0), None));
    let mut assets =
        assets_with_nonstraight_profile(SwordStrike::D, crate::profiles::WeaponThrustKind::Lateral);
    let thrust = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
        [SwordStrike::D as usize];
    thrust.initial_angle = 5;
    thrust.final_angle = 5;
    thrust.rotation_angle = 5;

    engine.initialize_sweep(
        &assets,
        attacker,
        SwordStrike::D,
        Some(1),
        crate::profiles::WeaponThrustKind::Lateral,
        vec![victim],
        crate::player_command::GestureQuality::PERFECT,
    );
    let direction_angle = sector_to_angle(
        engine
            .get_entity(attacker)
            .unwrap()
            .element_data()
            .direction(),
    );
    let sweep = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .unwrap();
    let five_degrees = f32::from_bits(0x3db2_b8c3);
    assert_eq!(
        sweep.initial_angle.to_bits(),
        (direction_angle - five_degrees).to_bits()
    );
    assert_eq!(
        sweep.final_angle.to_bits(),
        (direction_angle + five_degrees).to_bits()
    );
    assert_eq!(sweep.rotation_per_frame.to_bits(), five_degrees.to_bits());

    install_test_melee_order(&mut engine, attacker, victim, SwordStrike::D, true);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .sweep_state = None;
    engine.rebind_retained_sweep_to_active_strike(&assets, attacker);
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .as_ref()
            .unwrap()
            .rotation_per_frame
            .to_bits(),
        five_degrees.to_bits(),
        "loaded sweep reconstruction uses the same sword-profile conversion"
    );

    // Keep a common authored angle as a control alongside the one-bit 5° case.
    assert_eq!(strike_profile_angle(45).to_bits(), 0x3f49_0fdb);
}

#[test]
fn circle_warning_tolerance_uses_radians_returned_by_sword_profile() {
    // The profile stores 180 degrees, but original-game strike rotation
    // returns PI radians. At relative sector 8 Original therefore extends
    // the warning range by 15 units: 10 + (8 * 5 * PI) / (8 * PI).
    let tolerance = circle_warning_walking_tolerance(8, 180);
    assert_eq!(tolerance, 15);

    let base_max_distance = 60_u16;
    let walking_target_distance = 74.0;
    assert!(walking_target_distance <= f32::from(base_max_distance + tolerance));

    // Dividing by the raw profile degrees, as the old port did, would
    // reject this moving defender and suppress its strike-warning callback.
    let raw_degrees_tolerance = 10.0 + (8.0 * 5.0 * std::f32::consts::PI) / (8.0 * 180.0);
    assert!(walking_target_distance > f32::from(base_max_distance) + raw_degrees_tolerance);
    assert!(walking_target_distance > f32::from(base_max_distance));

    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_soldier(WorldPoint3D::ZERO, None));
    let target = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: walking_target_distance,
            ..WorldPoint3D::ZERO
        },
        None,
    ));
    {
        let actor = engine
            .get_entity_mut(target)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.action_state = ActionState::MovingSword;
        actor.installed_order = Some(crate::element::InstalledActorOrder {
            order_id: std::num::NonZeroU32::new(1).unwrap(),
            order_type: OrderType::WalkingWithSword,
        });
    }
    let assets = assets_with_sword_profile(0, base_max_distance);
    let collect = |engine: &EngineInner, max_distance| {
        collect_circle_warn_victims(
            &engine.world.entities,
            attacker,
            (0.0, 0.0),
            0,
            max_distance,
            180,
            |target_id| engine.live_actor_animation(target_id) == Some(OrderType::WalkingWithSword),
            &assets.profile_manager,
            &engine.world.fast_grid,
            crate::sight_obstacle::ObstacleList {
                static_obstacles: assets.environment.static_sight_obstacles.as_slice(),
                dynamic_obstacles: &engine.world.dynamic_sight_obstacles,
                static_active: &engine.world.static_sight_obstacle_active,
            },
        )
    };
    assert_eq!(collect(&engine, base_max_distance), vec![target]);

    // Running shares the port's coarse MovingSword state with walking,
    // but Original's exact animation predicate does not extend it.
    engine
        .get_entity_mut(target)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .installed_order
        .as_mut()
        .unwrap()
        .order_type = OrderType::RunningWithSword;
    assert!(collect(&engine, base_max_distance).is_empty());

    engine
        .get_entity_mut(target)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .installed_order
        .as_mut()
        .unwrap()
        .order_type = OrderType::WaitingSword;
    assert!(collect(&engine, base_max_distance).is_empty());

    // The ordinary case above admits at 60 + 15 = 75. The 16-bit compound
    // assignment instead wraps 65530 + 15 to 9, excluding the same
    // target rather than comparing against an unbounded float sum.
    engine
        .get_entity_mut(target)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .installed_order
        .as_mut()
        .unwrap()
        .order_type = OrderType::WalkingWithSword;
    assert!(collect(&engine, u16::MAX - 5).is_empty());
}

#[test]
fn swordfight_range_uses_stored_world_position_across_elevation() {
    let mut engine = EngineInner::new();
    let initiator = engine.add_test_entity(make_soldier(WorldPoint3D::ZERO, None));
    let opponent = engine.add_test_entity(make_soldier(WorldPoint3D::ZERO, None));
    engine
        .get_entity_mut(initiator)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D::new(1028.4918, 2063.3013, 22.8174));
    engine
        .get_entity_mut(opponent)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D::new(1032.8688, 1992.2421, 122.2636));

    // This is the replay geometry from S043 r004 f8369. Isometric map
    // projection puts the actors more than 150 units apart, while the
    // 3D position norm used when the original game enters a swordfight is in range.
    assert!(entity_distance(&engine.world.entities, initiator, opponent) > 150.0);
    assert!(entity_world_distance(&engine.world.entities, initiator, opponent) < 150.0);
}

#[test]
fn kill_experience_uses_exact_campaign_description_not_profile_number() {
    let mut engine = make_engine();
    engine.mission_domain.campaign.characters[0].character_profile_idx =
        Some(crate::profiles::CharacterProfileIdx(1));

    let mut attacker = make_pc(WorldPoint3D::ZERO, None);
    attacker.pc_data_mut().unwrap().profile_index = crate::profiles::CharacterProfileIdx(1);
    let attacker = engine.add_test_entity(attacker);
    let victim = engine.add_test_entity(make_soldier(WorldPoint3D::new(10.0, 0.0, 0.0), None));

    // The original-game player actor updates the character description reached through its
    // description/status reference. Profile number 1 living in campaign slot
    // 0 is valid and occurs in archived interactive replays.
    engine.award_bow_kill_xp(attacker);
    engine.award_sword_kill_xp(&LevelAssets::default(), attacker, victim);

    let status = &engine.mission_domain.campaign.characters[0]
        .status
        .human_status;
    assert_eq!(
        status.bow.experience,
        crate::bow_shot::BOW_KILL_EXPERIENCE_POINTS
    );
    assert_eq!(
        status.hand_to_hand.experience,
        crate::combat::SWORD_KILL_EXPERIENCE_POINTS
    );
}

#[test]
fn autonomous_vip_combatant_death_does_not_latch_party_failure() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let victim = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let Entity::Pc(pc) = engine.get_entity_mut(victim).unwrap() else {
        unreachable!()
    };
    pc.pc.mission_role = crate::human_control::MissionRole::Combatant;

    let mut profiles = crate::profiles::ProfileManager::new();
    profiles.characters.push(crate::profiles::CharacterProfile {
        vip: true,
        ..Default::default()
    });
    let assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::new()
    };

    engine.apply_pc_kill_cascade(&sim, &assets, victim);

    assert!(engine.mission_domain.dead_pc.is_none());
}

#[test]
fn player_party_vip_death_still_latches_party_failure() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let victim = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    let mut profiles = crate::profiles::ProfileManager::new();
    profiles.characters.push(crate::profiles::CharacterProfile {
        vip: true,
        ..Default::default()
    });
    let assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::new()
    };

    engine.apply_pc_kill_cascade(&sim, &assets, victim);

    assert_eq!(engine.mission_domain.dead_pc, Some(victim));
}

#[test]
fn damage_dispatcher_disables_direction_on_live_reaction_orders() {
    for (command, expected) in [
        (Command::ReceiveDamage, OrderType::FallingBackUpright),
        (Command::ReceiveMobileDamage, OrderType::FallingBackUpright),
        (
            Command::ReceiveArrowDamage,
            OrderType::ExtractingArrowUpright,
        ),
        (Command::ReceiveStoneDamage, OrderType::FallingBackUpright),
    ] {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_test_entity(make_soldier(WorldPoint3D::ZERO, None));
        let victim = engine.add_test_entity(make_pc(
            WorldPoint3D {
                x: 10.0,
                ..WorldPoint3D::ZERO
            },
            None,
        ));
        let assets = action_test_assets([crate::profiles::Action::NoAction; 3]);
        let mut damage = crate::sequence::SequenceElement::new_damage(
            1,
            command,
            Some(victim),
            Some(attacker),
            1,
            0,
        );
        engine.resolve_element_priority(&mut damage);
        let sequence = engine.orders.sequence_manager.launch_element(damage);
        let mut display = crate::engine::HostDisplayState::default();
        engine.hourglass_phase_sequences(&sim, &mut display, &assets);

        let element = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("translated damage command remains registered");
        assert_eq!(element.command, command);
        assert!(
            element
                .orders
                .iter()
                .any(|order| order.order_type == expected),
            "{command:?} must author {expected:?}"
        );
        assert!(
            !element
                .orders
                .iter()
                .find(|order| order.order_type == expected)
                .unwrap()
                .compute_direction
        );
    }
}

#[test]
fn hit_translation_defers_flight_facing_until_first_execute() {
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
            x: 30.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.element_data_mut().set_layer(4);
        let position = victim_entity.position_iface_mut();
        position.set_direction_instantly(crate::position_interface::Direction::from_raw(5));
        position.set_move_box(crate::coordinates::MoveBox::from_coords(
            -5.0, -5.0, 5.0, 5.0,
        ));
    }

    let element = crate::sequence::SequenceElement::new(1, Command::ReceiveHitDamage, Some(victim));
    let seq_id = engine.launch_element(element);
    engine.dispatch_hit_fall_animation(
        &LevelAssets::default(),
        victim,
        Some(attacker),
        false,
        (seq_id, 0),
    );

    let queued = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap()
        .orders
        .back()
        .unwrap();
    assert_eq!(queued.order_type, OrderType::FallingHitUpright);
    assert_eq!(queued.antagonist, Some(attacker));
    assert!(!queued.compute_direction);
    let queued_type = queued.order_type;
    let victim_entity = engine.get_entity(victim).unwrap();
    assert_eq!(victim_entity.element_data().direction(), 5);
    assert_eq!(victim_entity.position_iface().layer_goal().get(), 0);
    assert!(victim_entity.actor_data().unwrap().active_flight.is_none());

    engine.initialize_hit_flight(&LevelAssets::default(), victim, Some(attacker), queued_type);

    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .position_iface()
            .layer_goal()
            .get(),
        4,
        "takeoff preparation publishes its authored goal layer immediately"
    );
    assert_ne!(
        engine
            .get_entity(victim)
            .unwrap()
            .element_data()
            .direction(),
        5
    );
}

#[test]
fn hit_translation_without_animation_terminates_despite_retained_transition_order() {
    let mut engine = make_engine();
    let victim = engine.add_test_entity(make_soldier(WorldPoint3D::default(), None));
    engine
        .get_entity_mut(victim)
        .expect("hit victim exists")
        .set_posture(Posture::Flying);

    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveHitDamage, Some(victim));
    damage.orders.push_back(crate::order::Order::new(
        OrderType::NonanimationEnd,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    ));
    damage.initialize_transition_orders();
    let sequence = engine.launch_element(damage);

    engine.dispatch_hit_fall_animation(&LevelAssets::default(), victim, None, false, (sequence, 0));

    let damage = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .expect("terminated hit element remains inspectable");
    assert_eq!(damage.state, crate::sequence::SequenceState::Terminated);
    assert_eq!(
        damage.orders.len(),
        1,
        "termination retains the pre-translation order for diagnostics"
    );
}

#[test]
fn charging_rider_falling_hit_normalizes_non_cardinal_sector_vector() {
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_soldier(WorldPoint3D::ZERO, None));
    let victim = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 32.0,
            y: 1.0,
            z: 0.0,
        },
        None,
    ));
    authorize_test_hit_flight(&mut engine, victim);
    {
        let Entity::Soldier(attacker) = engine.get_entity_mut(attacker).unwrap() else {
            unreachable!()
        };
        attacker.soldier.rider = true;
        attacker.actor.active_rider_charge = Some(crate::element::ActiveRiderCharge {
            pending_victims: vec![victim],
        });
        attacker.element.set_direction_instantly(11);
    }

    engine.initialize_hit_flight(
        &LevelAssets::new(),
        victim,
        Some(attacker),
        OrderType::FallingHitUpright,
    );

    let delta = initialized_hit_flight_delta(&engine, victim);
    assert_eq!(delta.x.to_bits(), 0xc1e9_801b);
    assert_eq!(delta.y.to_bits(), 0x40dd_e72e);
    assert!(
        delta.x < 0.0 && delta.y > 0.0,
        "direction 11 flies southwest"
    );
}

#[test]
fn antagonistless_falling_hit_normalizes_opposite_non_cardinal_sector_vector() {
    let mut engine = make_engine();
    let victim = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 32.0,
            y: 1.0,
            z: 0.0,
        },
        None,
    ));
    authorize_test_hit_flight(&mut engine, victim);
    engine
        .get_entity_mut(victim)
        .unwrap()
        .position_iface_mut()
        .set_direction_instantly(crate::position_interface::Direction::from_raw(3));

    engine.initialize_hit_flight(
        &LevelAssets::new(),
        victim,
        None,
        OrderType::FallingHitUpright,
    );

    let delta = initialized_hit_flight_delta(&engine, victim);
    assert_eq!(delta.x.to_bits(), 0xc1e9_801b);
    assert_eq!(delta.y.to_bits(), 0x40dd_e72e);
    assert!(
        delta.x < 0.0 && delta.y > 0.0,
        "opposite direction 11 flies southwest"
    );
}

#[test]
fn positioned_antagonist_falling_hit_keeps_radial_normalization() {
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: -2.0,
            y: -4.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 1.0,
            y: 1.0,
            z: 0.0,
        },
        None,
    ));
    authorize_test_hit_flight(&mut engine, victim);

    engine.initialize_hit_flight(
        &LevelAssets::new(),
        victim,
        Some(attacker),
        OrderType::FallingHitUpright,
    );

    let delta = initialized_hit_flight_delta(&engine, victim);
    // Adding the exact source component 0x4176_f53d to x=1 and
    // subtracting the origin rounds the observable displacement once;
    // the old per-component normalization instead produced 0x4176_f53e.
    assert_eq!(delta.x.to_bits(), 0x4176_f53c);
    assert_eq!(delta.y.to_bits(), 0x41cd_cc5e);
}

#[test]
fn pc_hit_translation_inherits_silent_human_say_ouch() {
    let mut engine = make_engine();
    let victim = engine.add_test_entity(make_pc(WorldPoint3D::default(), None));
    let damage = crate::sequence::SequenceElement::new(1, Command::ReceiveHitDamage, Some(victim));
    let sequence_id = engine.launch_element(damage);

    engine.apply_hit_damage(
        &crate::sim_rng::test_context(),
        &LevelAssets::default(),
        victim,
        None,
        1,
        false,
        (sequence_id, 0),
    );

    assert!(
        engine.feedback.sound_sim.pending_exclamations.is_empty(),
        "PC inherits the original game's no-op reaction to hit damage"
    );
}

#[test]
fn scroll_civilian_hit_keeps_immunity_but_still_translates_reaction() {
    let mut engine = make_engine();
    let _null_slot = engine.add_test_entity(make_pc(WorldPoint3D::default(), None));
    let attacker = engine.add_test_entity(make_pc(WorldPoint3D::default(), None));
    let victim = engine.add_test_entity(make_civilian(WorldPoint3D {
        x: 20.0,
        ..WorldPoint3D::default()
    }));
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        let civilian = match victim_entity {
            Entity::Civilian(civilian) => civilian,
            other => panic!("test civilian changed kind to {:?}", other.kind()),
        };
        civilian.npc.attached_scroll = Some(crate::entity_id::EntityId::Scroll(
            crate::entity_id::ScrollId(u32::MAX),
        ));
        civilian.npc.ai_brain = crate::element::AiBrain::Friendly(Box::new(
            crate::ai_friendly::FriendlyAi::new(victim.index()),
        ));
    }
    let damage = crate::sequence::SequenceElement::new_damage(
        1,
        Command::ReceiveHitDamage,
        Some(victim),
        Some(attacker),
        0,
        3,
    );
    let sequence = engine.launch_element(damage);
    let mut assets = LevelAssets::default();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .civilians
        .push(crate::profiles::CivilianProfile::default());

    engine.dispatch_receive_damage(
        &crate::sim_rng::test_context(),
        &assets,
        victim,
        sequence,
        0,
    );

    let victim_entity = engine.get_entity(victim).unwrap();
    assert_eq!(
        victim_entity.human_data().unwrap().concussion_of_the_brain,
        0,
        "the attached-scroll civilian override must still suppress concussion"
    );
    let damage = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .unwrap();
    assert_eq!(damage.state, crate::sequence::SequenceState::InProgress);
    assert!(
        damage
            .orders
            .iter()
            .any(|order| order.order_type == OrderType::FallingHitUpright),
        "hit-damage translation must still author the hit reaction after the no-op concussion response"
    );
}

#[test]
fn conscious_hit_applies_ai_eye_status_synchronously() {
    let mut engine = make_engine();
    let null_slot = engine.add_test_entity(make_soldier(WorldPoint3D::default(), None));
    engine
        .get_entity_mut(null_slot)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .hth_weapon_id = 1;
    let attacker = engine.add_test_entity(make_pc(WorldPoint3D::default(), None));
    let victim = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 20.0,
            y: 0.0,
            z: 0.0,
        },
        None,
    ));
    engine
        .get_entity_mut(victim)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .hth_weapon_id = 1;
    let damage = crate::sequence::SequenceElement::new(1, Command::ReceiveHitDamage, Some(victim));
    let seq_id = engine.launch_element(damage);
    let assets = assets_with_sword_profile(1, 50);

    engine.apply_hit_damage(
        &crate::sim_rng::test_context(),
        &assets,
        victim,
        Some(attacker),
        1,
        true,
        (seq_id, 0),
    );

    // EVENT_GOTHIT first stops actions (which queues Unfocus) and only
    // then sets EYES_DIE_OR_GET_UNCONSCIOUS. Exercise the complete
    // fixed-point drain: applying the tail eye write through the earlier
    // recovery channel made this pass immediately after Translate but
    // regress to LookForward once the queued Unfocus was drained.
    engine.drain_pending_for_npc(&crate::sim_rng::test_context(), victim, &assets);

    let victim_entity = engine.get_entity(victim).unwrap();
    assert_eq!(
        victim_entity.npc_data().unwrap().eye_status,
        EyeStatus::DieOrGetUnconscious
    );
    assert_eq!(
        victim_entity
            .ai_controller()
            .unwrap()
            .outbox
            .recovery
            .set_eye_status,
        None,
        "the synchronous EVENT_GOTHIT write must not wait for the next owner slot"
    );
}

#[test]
fn conscious_lying_hit_applies_concussion_and_got_hit_before_terminating() {
    let mut engine = make_engine();
    let null_slot = engine.add_test_entity(make_soldier(WorldPoint3D::default(), None));
    engine
        .get_entity_mut(null_slot)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .hth_weapon_id = 1;
    let attacker = engine.add_test_entity(make_pc(WorldPoint3D::default(), None));
    let victim = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 20.0,
            y: 0.0,
            z: 0.0,
        },
        None,
    ));
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.set_posture(Posture::Lying);
        victim_entity.npc_data_mut().unwrap().eye_status = EyeStatus::Closed;
        victim_entity.enemy_ai_mut().unwrap().hth_weapon_id = 1;
    }
    let damage = crate::sequence::SequenceElement::new_damage(
        1,
        Command::ReceiveHitDamage,
        Some(victim),
        Some(attacker),
        0,
        3,
    );
    let seq_id = engine.launch_element(damage);
    let assets = assets_with_sword_profile(1, 50);

    engine.dispatch_receive_damage(&crate::sim_rng::test_context(), &assets, victim, seq_id, 0);

    let victim_entity = engine.get_entity(victim).unwrap();
    assert_eq!(
        victim_entity.human_data().unwrap().concussion_of_the_brain,
        6,
        "concussion scales the incoming 3 by 100 / 50 life before the lying early exit"
    );
    assert_eq!(
        victim_entity.npc_data().unwrap().eye_status,
        EyeStatus::DieOrGetUnconscious,
        "EVENT_GOTHIT runs before the lying early exit"
    );
    let damage = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap();
    assert_eq!(damage.state, crate::sequence::SequenceState::Terminated);
    assert!(
        damage.orders.is_empty(),
        "an already-lying victim must not receive another fall order"
    );
}

#[test]
fn enemy_ai_hero_consumes_enemy_sword_strike_proposal() {
    let mut engine = make_engine();
    let (attacker, _) = make_enemy_ai_hero_strike_pair(&mut engine);
    let mut assets = assets_with_sword_profile(7, 30);
    std::sync::Arc::make_mut(&mut assets.profile_manager).characters[0].fighting = 100;
    engine.control.rng = SimulationRng::with_original_replay(vec![0]);

    engine.with_simulation_context(|engine, sim| {
        engine.consume_pending_enemy_sword_attack_for(sim, &assets, attacker);
    });

    let ai = engine
        .get_entity(attacker)
        .and_then(Entity::enemy_ai)
        .expect("AI-controlled hero must retain its Enemy AI");
    assert!(ai.pending_special_strike);
    assert!(
        engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(attacker, Command::is_swordstrike),
        "the authorized AI-controlled hero proposal must launch a real strike"
    );
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .element_data()
            .current_outline,
        crate::element::OutlineColorName::Default,
        "attacking another AI-controlled hero must not use the player-warning hulk delay"
    );
}

#[test]
fn entering_attacking_swordfight_without_reconsideration_does_not_propose() {
    let mut engine = make_engine();
    let (attacker, _) = make_enemy_strike_pair(&mut engine, false);
    let assets = assets_with_sword_profile(7, 30);
    engine.control.rng = SimulationRng::with_original_replay(Vec::new());

    engine.with_simulation_context(|engine, sim| {
        engine.tick_enemy_sword_attacks(sim, &assets);
    });

    assert_eq!(engine.control.rng.original_replay_cursor(), Some(0));
    assert!(
        !engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(attacker, Command::is_swordstrike),
        "entering AttackingSwordfight alone must not propose a strike"
    );
}

#[test]
fn sword_strike_honour_reads_live_animation_not_action_change_history() {
    let mut engine = make_engine();
    let (attacker, target) = make_enemy_strike_pair(&mut engine, true);
    let assets = assets_with_sword_profile(7, 30);
    engine.control.rng = SimulationRng::with_original_replay(Vec::new());
    {
        let target = engine.get_entity_mut(target).unwrap();
        let actor = target.actor_data_mut().unwrap();
        actor.old_action = OrderType::Invalid;
        // The live animation is the installed order (the Original's
        // actor order), not the action-change history in `old_action`.
        actor.installed_order = Some(crate::element::InstalledActorOrder {
            order_id: std::num::NonZeroU32::new(1).unwrap(),
            order_type: OrderType::BeingHitSword,
        });
        target.element_data_mut().sprite.last_action = OrderType::BeingHitSword;
    }

    engine.with_simulation_context(|engine, sim| {
        engine.tick_enemy_sword_attacks(sim, &assets);
    });

    assert_eq!(
        engine.control.rng.original_replay_cursor(),
        Some(0),
        "animation-based recovery rejection must precede strike selection"
    );
    assert!(
        !engine
            .get_entity(attacker)
            .and_then(Entity::enemy_ai)
            .unwrap()
            .pending_sword_strike_consideration,
        "the rejected reconsideration remains a one-shot event"
    );
}

#[test]
fn owner_scoped_sword_consideration_precedes_later_owner_rng() {
    let mut engine = make_engine();
    let (attacker, _) = make_enemy_strike_pair(&mut engine, true);
    let assets = assets_with_sword_profile(7, 30);
    {
        let sprite = &mut engine
            .get_entity_mut(attacker)
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
    engine.control.rng = SimulationRng::with_original_replay(vec![85, 36]);

    let later_roll = engine.with_simulation_context(|engine, sim| {
        engine.consume_pending_enemy_sword_attack_for(sim, &assets, attacker);
        crate::sim_rng::u32(sim, crate::sim_rng::RngSite::ScriptRand, 0..100)
    });

    assert_eq!(
        later_roll, 36,
        "the reconsidering owner must consume its strike roll before a later owner's script"
    );
    assert_eq!(engine.control.rng.original_replay_cursor(), Some(2));
    assert!(
        !engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(attacker, Command::is_swordstrike),
        "the first roll rejects the strike and must not borrow the later owner's lower roll"
    );
}

#[test]
fn deferred_combat_insult_depends_on_inline_strike_result() {
    fn install_minimal_sprite(engine: &mut EngineInner, attacker: EntityId) {
        let sprite = &mut engine
            .get_entity_mut(attacker)
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

    let assets = assets_with_sword_profile(7, 30);

    // A failed proposal leaves Original in ordinary Swordfight and the
    // caller's following statement says CombatInsult.
    let mut rejected = make_engine();
    let (rejected_attacker, _) = make_enemy_strike_pair(&mut rejected, true);
    install_minimal_sprite(&mut rejected, rejected_attacker);
    rejected
        .get_entity_mut(rejected_attacker)
        .and_then(Entity::enemy_ai_mut)
        .unwrap()
        .pending_combat_insult_after_strike_consideration = true;
    rejected.control.rng = SimulationRng::with_original_replay(vec![85]);
    rejected.with_simulation_context(|engine, sim| {
        engine.consume_pending_enemy_sword_attack_for(sim, &assets, rejected_attacker);
    });
    let rejected_ai = rejected
        .get_entity(rejected_attacker)
        .and_then(Entity::enemy_ai)
        .unwrap();
    assert!(
        rejected_ai
            .base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .any(|work| matches!(
                work,
                crate::ai::AiOwnerWork::Speech(attempt)
                    if attempt.remark == crate::ai::Remark::CombatInsult
            ))
    );

    // A successful proposal changes Original to SpecialStrike before the
    // same following statement tests the substate, suppressing the bark.
    let mut accepted = make_engine();
    let (accepted_attacker, _) = make_enemy_strike_pair(&mut accepted, true);
    install_minimal_sprite(&mut accepted, accepted_attacker);
    accepted
        .get_entity_mut(accepted_attacker)
        .and_then(Entity::enemy_ai_mut)
        .unwrap()
        .pending_combat_insult_after_strike_consideration = true;
    accepted.control.rng = SimulationRng::with_original_replay(vec![0]);
    accepted.with_simulation_context(|engine, sim| {
        engine.consume_pending_enemy_sword_attack_for(sim, &assets, accepted_attacker);
    });
    let accepted_ai = accepted
        .get_entity(accepted_attacker)
        .and_then(Entity::enemy_ai)
        .unwrap();
    assert!(accepted_ai.pending_special_strike);
    assert!(
        !accepted_ai
            .base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .any(|work| matches!(
                work,
                crate::ai::AiOwnerWork::Speech(attempt)
                    if attempt.remark == crate::ai::Remark::CombatInsult
            ))
    );
}

#[test]
fn civilian_health_counts_toward_round_strike_and_warcry() {
    let mut engine = make_engine();
    let (attacker, _) = make_enemy_strike_pair(&mut engine, true);
    {
        let sprite = &mut engine
            .get_entity_mut(attacker)
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
    engine.add_test_entity(make_civilian(WorldPoint3D {
        x: 15.0,
        y: 100.0,
        z: 0.0,
    }));

    let mut assets = assets_with_nonstraight_profile(
        SwordStrike::H,
        crate::profiles::WeaponThrustKind::TrueCircle,
    );
    std::sync::Arc::make_mut(&mut assets.profile_manager).soldiers[0].fighting = 100;
    engine.control.rng = SimulationRng::with_original_replay(vec![0]);

    engine.with_simulation_context(|engine, sim| {
        engine.consume_pending_enemy_sword_attack_for(sim, &assets, attacker);
    });

    assert!(
        engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(attacker, |command| {
                command == Command::SwordstrikeThrustH
            }),
        "the PC and civilian are two live round-strike victims"
    );
    assert_eq!(
        engine
            .get_entity(attacker)
            .and_then(Entity::enemy_ai)
            .expect("fixture attacker keeps Enemy AI")
            .base
            .current_remark,
        crate::ai::Remark::Warcry,
        "Original says REMARK_WARCRY when selecting thrust H"
    );
}

#[test]
fn completed_missed_sword_strike_adds_tiredness_once() {
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
            x: 500.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let assets = assets_with_sword_profile(7, 30);

    install_test_melee_order(&mut engine, attacker, target, SwordStrike::A, true);

    engine.tick_melee_strikes(sim, &assets);

    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .tiredness,
        7,
        "out-of-range strikes still cost tiredness when the active strike terminates"
    );
}

#[test]
fn circle_done_initialization_advances_without_rotating_or_hitting() {
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
            y: 90.0,
            z: 0.0,
        },
        None,
    ));
    let assets = assets_with_nonstraight_profile(
        SwordStrike::F,
        crate::profiles::WeaponThrustKind::TrueHalfCircle,
    );

    engine.initialize_sweep(
        &assets,
        attacker,
        SwordStrike::F,
        Some(1),
        crate::profiles::WeaponThrustKind::TrueHalfCircle,
        vec![victim],
        crate::player_command::GestureQuality::PERFECT,
    );
    let initial_angle = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .unwrap()
        .current_angle;

    engine.tick_sweep_for(&assets, attacker, true);

    let attacker_entity = engine.get_entity(attacker).unwrap();
    let sweep = attacker_entity
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("true half-circle must retain its initialized sweep");
    assert!(
        (sweep.current_angle - (initial_angle + std::f32::consts::FRAC_PI_2)).abs() < f32::EPSILON,
        "circle sword-strike execution advances its internal angle at the DONE-call tail"
    );
    assert_eq!(
        attacker_entity.element_data().direction(),
        0,
        "the DONE call must not rotate the true-circle sprite"
    );
    assert_eq!(
        soldier_life(&engine, victim),
        50,
        "the DONE effect branch only initializes victims and cannot hit"
    );
}

#[test]
fn lateral_done_initialization_does_not_advance_or_hit() {
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
            y: 90.0,
            z: 0.0,
        },
        None,
    ));
    let assets =
        assets_with_nonstraight_profile(SwordStrike::D, crate::profiles::WeaponThrustKind::Lateral);
    let selected = install_test_melee_order(&mut engine, attacker, victim, SwordStrike::D, false);

    let phase = engine.tick_nonstraight_melee_for(sim, &assets, attacker, selected);
    assert!(
        phase == strikes::SweepTickPhase::Initialized,
        "the lateral DONE branch must initialize a sweep"
    );
    let initial_current = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .unwrap()
        .current_angle;
    engine.tick_sweep_for(&assets, attacker, true);

    let current = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("lateral victim must remain pending after DONE")
        .current_angle;
    assert_eq!(
        current, initial_current,
        "lateral sword-strike execution uses exclusive branches, so DONE cannot also run its in-progress advance"
    );
    assert_eq!(
        soldier_life(&engine, victim),
        50,
        "lateral initialization cannot hit until a later update"
    );
}

#[test]
fn push_victims_queue_damage_in_creation_fifo() {
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
    let first_victim = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 80.0,
            z: 0.0,
        },
        None,
    ));
    let second_victim = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 60.0,
            z: 0.0,
        },
        None,
    ));
    for victim in [first_victim, second_victim] {
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
    }
    let assets = assets_with_nonstraight_profile(
        SwordStrike::D,
        crate::profiles::WeaponThrustKind::PushAside,
    );
    let selected =
        install_test_melee_order(&mut engine, attacker, first_victim, SwordStrike::D, false);

    assert_eq!(
        engine.tick_nonstraight_melee_for(sim, &assets, attacker, selected),
        strikes::SweepTickPhase::InProgress
    );

    let first_life = soldier_life(&engine, first_victim);
    let second_life = soldier_life(&engine, second_victim);
    let damage_fifo: Vec<EntityId> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .filter(|element| element.command == Command::ReceiveSwordDamage)
        .filter_map(|element| element.owner)
        .collect();
    assert_eq!(
        damage_fifo,
        vec![first_victim, second_victim],
        "push damage launches must retain the original actor-list victim FIFO; lives were {first_life}/{second_life}"
    );
}

#[test]
fn launching_sword_damage_does_not_add_attacker_tiredness() {
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
        .get_entity_mut(attacker)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .tiredness = 11;

    engine.queue_sword_damage(victim, attacker, SwordStrike::A, 1);

    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .tiredness,
        11,
        "damage application is victim-count dependent and must not charge strike energy"
    );
}

#[test]
fn helping_climb_shoulder_damage_keeps_posture_until_fall_executes() {
    let sim = crate::sim_rng::SimulationContext::with_seed(0x183);
    let mut engine = make_engine();
    let victim = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    engine
        .get_entity_mut(victim)
        .expect("test victim must exist")
        .set_posture(Posture::HelpingToClimb);

    let mut sequence = crate::sequence::Sequence::new();
    sequence.append_element(crate::sequence::SequenceElement::new(
        1,
        Command::ReceiveSwordDamage,
        Some(victim),
    ));
    let sequence_id = engine.orders.sequence_manager.launch_sequence(sequence);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);

    let assets = action_test_assets([crate::profiles::Action::NoAction; 3]);
    engine.translate_shoulder_damage(&sim, &assets, victim, (sequence_id, 0));

    assert_eq!(
        engine
            .get_entity(victim)
            .expect("test victim must remain live")
            .element_data()
            .posture(),
        Posture::HelpingToClimb,
        "shoulder-damage translation only queues FallingBackUpright; execution start changes posture on the actor's next slot"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 0)
            .expect("damage element must remain registered")
            .orders
            .back()
            .expect("shoulder damage must queue a fall order")
            .order_type,
        OrderType::FallingBackUpright
    );
}

#[test]
fn shoulder_damage_dispatches_partner_fall_without_direction_recompute() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_soldier(WorldPoint3D::ZERO, None));
    let carrier = engine.add_test_entity(make_pc(WorldPoint3D::ZERO, None));
    let carried = engine.add_test_entity(make_pc(WorldPoint3D::ZERO, None));
    engine
        .get_entity_mut(carrier)
        .unwrap()
        .set_posture(Posture::HelpingToClimb);
    engine
        .get_entity_mut(carrier)
        .unwrap()
        .pc_data_mut()
        .unwrap()
        .carried = Some(carried);
    engine
        .get_entity_mut(carried)
        .unwrap()
        .set_posture(Posture::OnShoulders);
    engine
        .get_entity_mut(carried)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .carrier = Some(carrier);

    let assets = action_test_assets([crate::profiles::Action::NoAction; 3]);
    let mut damage = crate::sequence::SequenceElement::new_damage(
        1,
        Command::ReceiveDamage,
        Some(carrier),
        Some(attacker),
        1,
        0,
    );
    engine.resolve_element_priority(&mut damage);
    engine.orders.sequence_manager.launch_element(damage);
    let mut display = crate::engine::HostDisplayState::default();
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    let partner_fall = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .find(|element| element.command == Command::Fall && element.owner == Some(carried))
        .expect("shoulder damage must dispatch Fall to the carried partner");
    let order = partner_fall
        .orders
        .iter()
        .find(|order| order.order_type == OrderType::FallingShoulders)
        .expect("partner Fall command must translate to FallingShoulders");
    assert!(!order.compute_direction);
}

#[test]
fn parried_damage_still_learns_attackers_live_strike() {
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
    let assets = assets_with_sword_profile(7, 30);

    let mut attacker_sequence = crate::sequence::Sequence::new();
    attacker_sequence.append_element(crate::sequence::SequenceElement::new(
        1,
        Command::SwordstrikeThrustE,
        Some(attacker),
    ));
    let attacker_sequence_id = engine
        .orders
        .sequence_manager
        .launch_sequence(attacker_sequence);
    engine
        .orders
        .sequence_manager
        .element_in_progress(attacker_sequence_id, 0);

    let mut damage_sequence = crate::sequence::Sequence::new();
    let mut damage_element =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage_element.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::E, 1);
    damage_sequence.append_element(damage_element);
    let damage_sequence_id = engine
        .orders
        .sequence_manager
        .launch_sequence(damage_sequence);
    engine
        .orders
        .sequence_manager
        .element_in_progress(damage_sequence_id, 0);

    let Entity::Soldier(soldier) = engine.get_entity_mut(victim).unwrap() else {
        unreachable!()
    };
    soldier.actor.action_state = ActionState::ParryingSword;
    let crate::element::AiBrain::Enemy(ai) = &mut soldier.npc.ai_brain else {
        unreachable!()
    };
    ai.known_enemy_strike_1 = Some(SwordStrike::D);

    engine.apply_sword_damage(
        &sim,
        &assets,
        victim,
        Some(attacker),
        Some(SwordStrike::E),
        Some(1),
        (damage_sequence_id, 0),
    );

    let Entity::Soldier(soldier) = engine.get_entity(victim).unwrap() else {
        unreachable!()
    };
    let crate::element::AiBrain::Enemy(ai) = &soldier.npc.ai_brain else {
        unreachable!()
    };
    assert_eq!(ai.known_enemy_strike_1, Some(SwordStrike::E));
    assert_eq!(
        ai.known_enemy_strike_2, None,
        "a low-skill guard forgets its previous strike when the parried live strike is learned"
    );
}

#[test]
fn push_damage_virtual_say_ouch_is_silent_for_pc() {
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
    let assets = assets_with_nonstraight_profile(
        SwordStrike::H,
        crate::profiles::WeaponThrustKind::TrueCircle,
    );
    let damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    let sequence_id = engine.orders.sequence_manager.launch_element(damage);

    assert!(engine.apply_push_effect(
        &sim,
        &assets,
        victim,
        attacker,
        &PushStrikeInfo { repulsion: 100 },
        crate::combat::SwordDamageResult::NO_DAMAGE_PARRIED,
        (sequence_id, 0),
        false,
    ));
    assert!(
        engine.feedback.sound_sim.pending_exclamations.is_empty(),
        "PC inherits the original game's no-op reaction to push damage"
    );
}

#[test]
fn push_damage_command_disables_direction_on_fall_and_successors() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_soldier(WorldPoint3D::ZERO, None));
    let victim = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::ZERO
        },
        None,
    ));
    {
        let victim = engine
            .get_entity_mut(victim)
            .expect("push victim remains live");
        victim.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        victim.human_data_mut().unwrap().concussion_of_the_brain = STUNNING_THRESHOLD + 1;
        victim.enemy_ai_mut().unwrap().hth_weapon_id = 1;
    }
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .hth_weapon_id = 1;
    let assets = assets_with_nonstraight_profile(
        SwordStrike::H,
        crate::profiles::WeaponThrustKind::TrueCircle,
    );
    let damage = crate::sequence::SequenceElement::new_damage(
        1,
        Command::ReceiveSwordDamage,
        Some(victim),
        Some(attacker),
        1,
        0,
    );
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

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

    let element = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .expect("translated push damage remains registered");
    assert_eq!(
        element
            .orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        vec![
            OrderType::FallingPushedWithSword,
            OrderType::StandingUpSword,
            OrderType::BeingStunnedSword,
        ]
    );
    assert!(
        element
            .orders
            .iter()
            .filter(|order| order.order_type != OrderType::Rolling)
            .all(|order| !order.compute_direction),
        "push damage disables direction computation on the fall, stand-up, and stunned orders"
    );
}

#[test]
fn pc_hurt_speech_uses_applied_life_loss_not_attempted_damage() {
    let mut engine = make_engine();
    let victim = engine.add_test_entity(make_pc(WorldPoint3D::default(), None));
    let assets = action_test_assets([crate::profiles::Action::NoAction; 3]);

    // The protected task-188 control attempts more than twenty points of
    // damage, but only 18 LP are ultimately stored (82 -> 64).
    let attempted_damage = 25;
    assert!(attempted_damage > 20);
    engine.pc_life_points_speech(&assets, victim, 82, 64);
    assert!(
        engine.feedback.sound_sim.pending_exclamations.is_empty(),
        "original-game player life updates compare the applied LP delta"
    );

    engine.pc_life_points_speech(&assets, victim, 82, 61);
    assert_eq!(
        engine
            .feedback
            .sound_sim
            .pending_exclamations
            .iter()
            .map(|pending| pending.exclamation_id)
            .collect::<Vec<_>>(),
        vec![HERO_HURT]
    );
}

#[test]
fn pc_shoulder_sword_damage_skips_good_strike_but_keeps_fall_translation() {
    use crate::ai::{AiState, LogLineType, StimulusType, Substate};
    use crate::sequence::SequencePriority;

    for posture in [
        Posture::HelpingToClimb,
        Posture::CarryingOnShoulders,
        Posture::OnShoulders,
    ] {
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
        let partner = engine.add_test_entity(make_pc(WorldPoint3D::default(), None));
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
        engine.get_entity_mut(victim).unwrap().set_posture(posture);
        if posture == Posture::OnShoulders {
            engine
                .get_entity_mut(victim)
                .unwrap()
                .human_data_mut()
                .unwrap()
                .carrier = Some(partner);
            let partner_entity = engine.get_entity_mut(partner).unwrap();
            partner_entity.set_posture(Posture::CarryingOnShoulders);
            partner_entity.pc_data_mut().unwrap().carried = Some(victim);
        } else {
            engine
                .get_entity_mut(victim)
                .unwrap()
                .pc_data_mut()
                .unwrap()
                .carried = Some(partner);
            let partner_entity = engine.get_entity_mut(partner).unwrap();
            partner_entity.set_posture(Posture::OnShoulders);
            partner_entity.human_data_mut().unwrap().carrier = Some(victim);
        }

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
        assert!(
            !attacker_ai.ai_log.iter().any(|entry| {
                entry.line_type == LogLineType::Event
                    && entry.info == StimulusType::EventGoodStrike as u16
            }),
            "PC posture {posture:?} must use the PC shoulder override without EventGoodStrike"
        );
        assert_eq!(
            attacker_ai.current_substate,
            Substate::AttackingSwordfightSpecialStrike,
            "suppressed EventGoodStrike must not advance the attacker AI"
        );

        let damage = engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 0)
            .expect("shoulder damage command remains registered");
        let expected_fall = if posture == Posture::OnShoulders {
            assert_eq!(damage.priority, SequencePriority::NonInterruptable);
            OrderType::FallingShoulders
        } else {
            OrderType::FallingBackUpright
        };
        assert!(
            damage
                .orders
                .iter()
                .any(|order| order.order_type == expected_fall),
            "PC posture {posture:?} must retain its shoulder fall translation"
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .flat_map(|sequence| sequence.elements.iter())
                .any(|element| element.command == Command::Fall && element.owner == Some(partner)),
            "PC posture {posture:?} must still dispatch Fall to its shoulder partner"
        );
    }
}

#[test]
fn surviving_sword_knockout_quits_before_good_strike_and_fall_translation() {
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

    let mut assets = assets_with_sword_profile_effects(1, 50, 4, 100);
    let mut obstacle = crate::sight_obstacle::SightObstacle::new_default(0);
    obstacle.top_plane_points = [[0.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 1.0, 0.0]];
    assets.environment.static_sight_obstacles = std::sync::Arc::new(vec![obstacle]);
    let victim_entity = engine.get_entity_mut(victim).unwrap();
    victim_entity.element_data_mut().set_obstacle_index(
        crate::position_interface::ObstacleHandle::new(0),
        Some(crate::position_interface::PlaneZCoeffs {
            az: 1.0,
            bz: 0.0,
            dz: 0.0,
        }),
    );
    victim_entity
        .position_iface_mut()
        .set_move_box(crate::coordinates::MoveBox::from_corners(
            crate::coordinates::MapVec::new(-5.0, -5.0),
            crate::coordinates::MapVec::new(5.0, 5.0),
        ));
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

    let victim_entity = engine.get_entity(victim).unwrap();
    assert!(victim_entity.human_data().unwrap().unconscious);
    assert!(
        victim_entity.pc_data().unwrap().life_points > 0,
        "fixture must exercise the surviving-knockout arm"
    );
    assert!(victim_entity.human_data().unwrap().opponents.is_empty());
    let attacker_entity = engine.get_entity(attacker).unwrap();
    assert!(attacker_entity.human_data().unwrap().opponents.is_empty());
    let ai = attacker_entity.ai_controller().unwrap();
    assert_eq!(ai.current_substate, Substate::AttackingQuittingSwordfight);
    let good_strike_index = ai
        .ai_log
        .iter()
        .position(|entry| {
            entry.line_type == LogLineType::Event
                && entry.info == StimulusType::EventGoodStrike as u16
        })
        .expect("soldier origin must receive EVENT_GOOD_STRIKE");
    let quit_index = ai
        .ai_log
        .iter()
        .position(|entry| {
            entry.line_type == LogLineType::ChangeState
                && entry.info == Substate::AttackingQuittingSwordfight as u16
        })
        .expect("reciprocal unlink must synchronously enter the quitting substate");
    assert!(
        quit_index < good_strike_index,
        "concussion handling quits before sword-damage translation informs the hitter"
    );
    let translated_orders = &engine
        .orders
        .sequence_manager
        .get_element(sequence_id, 0)
        .expect("knockout damage element remains registered")
        .orders;
    assert_eq!(
        translated_orders.front().map(|order| order.order_type),
        Some(OrderType::FallingBackUpright),
        "sword-damage translation's second quit remains before its knockout fall"
    );
    assert!(
        translated_orders
            .iter()
            .any(|order| order.order_type == OrderType::Rolling),
        "the real surviving-KO translation must still append Roll"
    );
}

#[test]
fn preexisting_unconscious_smalltalk_hit_preserves_closed_eyes_and_plain_quit() {
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
        victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        victim_entity.human_data_mut().unwrap().unconscious = true;
        victim_entity.npc_data_mut().unwrap().eye_status = EyeStatus::Closed;
        victim_entity.enemy_ai_mut().unwrap().hth_weapon_id = 1;
        victim_entity
            .human_data_mut()
            .unwrap()
            .opponents
            .push(attacker);
    }
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(victim);

    let assets = assets_with_sword_profile(1, 50);

    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data = crate::sequence::SequenceElementData::new_sword_damage(
        attacker,
        SwordStrike::SmalltalkRight,
        1,
    );
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    engine.dispatch_receive_damage(&sim, &assets, victim, sequence, 0);

    let victim_entity = engine.get_entity(victim).unwrap();
    assert!(victim_entity.human_data().unwrap().unconscious);
    assert_eq!(
        victim_entity.npc_data().unwrap().eye_status,
        EyeStatus::Closed
    );
    assert!(victim_entity.human_data().unwrap().opponents.is_empty());
    assert!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents
            .is_empty(),
        "sword-damage translation's plain quit removes the reciprocal opponent"
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
        "the pre-existing-unconscious translation owns exactly one plain quit"
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
        "the hit must not replay concussion handling's KO callback"
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
        "the hit must not recreate the existing unconscious star"
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .orders
            .iter()
            .any(|order| order.order_type == OrderType::FallingBackSword),
        "upright WaitingSword translation still queues FallingBackSword"
    );
}

#[test]
fn protected_preexisting_unconscious_smalltalk_hit_has_no_translation() {
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
        victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        victim_entity.human_data_mut().unwrap().unconscious = true;
        victim_entity.npc_data_mut().unwrap().eye_status = EyeStatus::Closed;
        victim_entity.enemy_ai_mut().unwrap().hth_weapon_id = 1;
        victim_entity
            .human_data_mut()
            .unwrap()
            .opponents
            .push(attacker);
    }
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(victim);

    let mut assets = assets_with_sword_profile(1, 50);
    std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0]
        .protection_by_localization = [99; 5];

    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data = crate::sequence::SequenceElementData::new_sword_damage(
        attacker,
        SwordStrike::SmalltalkRight,
        1,
    );
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    engine.dispatch_receive_damage(&sim, &assets, victim, sequence, 0);

    let victim_entity = engine.get_entity(victim).unwrap();
    assert_eq!(
        victim_entity.human_data().unwrap().opponents,
        vec![attacker],
        "NO_DAMAGE must not enter sword-damage translation's plain-quit path"
    );
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![victim]
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .orders
            .is_empty(),
        "NO_DAMAGE must not translate a FallingBack/Roll order"
    );
    assert!(
        !victim_entity
            .ai_controller()
            .unwrap()
            .ai_log
            .iter()
            .any(|entry| {
                entry.line_type == LogLineType::Event
                    && matches!(
                        entry.info,
                        value if value == StimulusType::EventQuitSwordfight as u16
                            || value == StimulusType::EventLoseConsciousness as u16
                    )
            }),
        "NO_DAMAGE must neither quit nor replay the knockout callback"
    );
}

#[test]
fn grounded_preexisting_unconscious_smalltalk_hit_terminates_without_quit() {
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
            .publish_order_posture(Posture::Lying);
        victim_entity.human_data_mut().unwrap().unconscious = true;
        victim_entity.npc_data_mut().unwrap().eye_status = EyeStatus::Closed;
        victim_entity.enemy_ai_mut().unwrap().hth_weapon_id = 1;
        victim_entity
            .human_data_mut()
            .unwrap()
            .opponents
            .push(attacker);
    }
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(victim);

    let assets = assets_with_sword_profile(1, 50);
    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data = crate::sequence::SequenceElementData::new_sword_damage(
        attacker,
        SwordStrike::SmalltalkRight,
        1,
    );
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    engine.dispatch_receive_damage(&sim, &assets, victim, sequence, 0);

    let victim_entity = engine.get_entity(victim).unwrap();
    assert_eq!(
        victim_entity.npc_data().unwrap().eye_status,
        EyeStatus::Closed
    );
    assert_eq!(
        victim_entity.human_data().unwrap().opponents,
        vec![attacker]
    );
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![victim]
    );
    let damage = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .unwrap();
    assert_eq!(damage.state, crate::sequence::SequenceState::Terminated);
    assert!(damage.orders.is_empty());
    assert!(
        !victim_entity
            .ai_controller()
            .unwrap()
            .ai_log
            .iter()
            .any(|entry| {
                entry.line_type == LogLineType::Event
                    && matches!(
                        entry.info,
                        value if value == StimulusType::EventQuitSwordfight as u16
                            || value == StimulusType::EventLoseConsciousness as u16
                    )
            })
    );
}

#[test]
fn lethal_sword_hit_kills_unconscious_npc_before_say_ouch_translation() {
    use crate::ai::{AiState, LogLineType, Remark, Substate};

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
        victim_entity.npc_data_mut().unwrap().life_points = 15;
        victim_entity.human_data_mut().unwrap().unconscious = true;
        victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        let ai = victim_entity.enemy_ai_mut().unwrap();
        ai.hth_weapon_id = 1;
        ai.base.current_state = AiState::Sleeping;
        ai.base.current_substate = Substate::SleepingUnconscious;
    }

    let assets = assets_with_sword_profile_effects(1, 50, 100, 0);
    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    engine.apply_sword_damage(
        &sim,
        &assets,
        victim,
        Some(attacker),
        Some(SwordStrike::A),
        Some(1),
        (sequence, 0),
    );

    let victim_entity = engine.get_entity(victim).unwrap();
    assert!(victim_entity.is_dead());
    assert!(!victim_entity.human_data().unwrap().unconscious);
    let ai = victim_entity.ai_controller().unwrap();
    assert_eq!(ai.current_substate, Substate::SleepingForever);
    assert!(ai.ai_log.iter().any(|entry| {
        entry.line_type == LogLineType::Speak && entry.info == Remark::Dies as u16
    }));
    assert!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .orders
            .iter()
            .any(|order| order.order_type == OrderType::DyingSword),
        "sword-damage translation must retain ownership of the dying visual after synchronous death processing"
    );
}

#[test]
fn nonlethal_sword_hit_keeps_unconscious_npc_silent() {
    use crate::ai::{AiState, LogLineType, Substate};

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
        victim_entity.human_data_mut().unwrap().unconscious = true;
        victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        let ai = victim_entity.enemy_ai_mut().unwrap();
        ai.hth_weapon_id = 1;
        ai.base.current_state = AiState::Sleeping;
        ai.base.current_substate = Substate::SleepingUnconscious;
    }

    let assets = assets_with_sword_profile_effects(1, 50, 1, 0);
    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    engine.apply_sword_damage(
        &sim,
        &assets,
        victim,
        Some(attacker),
        Some(SwordStrike::A),
        Some(1),
        (sequence, 0),
    );

    let victim_entity = engine.get_entity(victim).unwrap();
    assert_eq!(victim_entity.npc_data().unwrap().life_points, 49);
    assert!(victim_entity.human_data().unwrap().unconscious);
    assert!(
        !victim_entity
            .ai_controller()
            .unwrap()
            .ai_log
            .iter()
            .any(|entry| entry.line_type == LogLineType::Speak),
        "the ordinary unconscious hurt-speech early return must remain intact for survivors"
    );
}

#[test]
fn killing_seeking_enemy_clears_only_its_beggar_detectables() {
    use crate::ai::{AiState, Substate};
    use crate::element::{Detectable, DetectableType};

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
        let ai = victim_entity.enemy_ai_mut().unwrap();
        ai.hth_weapon_id = 1;
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingArrowReactiontime;
        let npc = victim_entity.npc_data_mut().unwrap();
        for detectable_type in [DetectableType::Enemy, DetectableType::Beggar] {
            npc.detectable_lists[detectable_type as usize].push(Detectable {
                element: Some(attacker),
                detectable_type,
                ..Detectable::default()
            });
        }
    }
    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    engine.apply_nonvisual_death_cascade(
        &sim,
        &assets_with_sword_profile_effects(1, 50, 100, 0),
        victim,
        (sequence, 0),
        true,
    );

    let victim_entity = engine.get_entity(victim).unwrap();
    let npc = victim_entity.npc_data().unwrap();
    assert!(npc.detectable_lists[DetectableType::Beggar as usize].is_empty());
    assert_eq!(
        npc.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .map(|detectable| detectable.element)
            .collect::<Vec<_>>(),
        vec![Some(attacker)],
        "enemy state change only deletes the beggar bucket when leaving seeking"
    );
    let ai = victim_entity.ai_controller().unwrap();
    assert_eq!(ai.current_state, AiState::Sleeping);
    assert_eq!(ai.current_substate, Substate::SleepingForever);
}

#[test]
fn lethal_push_runs_npc_kill_cascade_before_owning_the_fall() {
    use crate::ai::{AiState, AlertLevel, Substate};
    use crate::element::{Detectable, DetectableType};

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
    let observer = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 20.0,
            ..WorldPoint3D::ZERO
        },
        None,
    ));
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.npc_data_mut().unwrap().life_points = 1;
        victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        victim_entity
            .human_data_mut()
            .unwrap()
            .opponents
            .push(attacker);
        let ai = victim_entity.enemy_ai_mut().unwrap();
        ai.hth_weapon_id = 1;
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfight;
        ai.base.current_music_alert_status = AlertLevel::Red;
        ai.base.view_alert_status = AlertLevel::Red;
    }
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(victim);
    {
        let observer_npc = engine
            .get_entity_mut(observer)
            .unwrap()
            .npc_data_mut()
            .unwrap();
        observer_npc.ai_brain.enemy_mut().unwrap().hth_weapon_id = 1;
        observer_npc.detectable_lists[DetectableType::Friend as usize].extend([
            Detectable {
                element: Some(victim),
                detectable_type: DetectableType::Friend,
                ..Detectable::default()
            },
            Detectable {
                element: Some(victim),
                detectable_type: DetectableType::Friend,
                ..Detectable::default()
            },
        ]);
        observer_npc.detectable_lists[DetectableType::MissedFriend as usize].push(Detectable {
            element: Some(victim),
            detectable_type: DetectableType::MissedFriend,
            ..Detectable::default()
        });
    }

    let mut assets = assets_with_sword_profile_effects(1, 50, 100, 0);
    let thrust = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
        [SwordStrike::A as usize];
    thrust.kind = crate::profiles::WeaponThrustKind::PushAside;
    thrust.repulsion = 100;
    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    let score_before = engine
        .mission_domain
        .campaign
        .get_value(crate::campaign::CampaignValue::Score);
    let killed_allied_before = engine.mission_domain.mission_stat.killed_allied_count;

    engine.apply_sword_damage(
        &sim,
        &assets,
        victim,
        Some(attacker),
        Some(SwordStrike::A),
        Some(1),
        (sequence, 0),
    );

    let victim_entity = engine.get_entity(victim).unwrap();
    let victim_ai = victim_entity.ai_controller().unwrap();
    assert!(victim_entity.is_dead());
    assert_eq!(victim_ai.current_state, AiState::Sleeping);
    assert_eq!(victim_ai.current_substate, Substate::SleepingForever);
    assert_eq!(victim_ai.current_music_alert_status, AlertLevel::Green);
    assert_eq!(victim_ai.view_alert_status, AlertLevel::Green);
    assert!(victim_entity.human_data().unwrap().opponents.is_empty());
    assert!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents
            .is_empty()
    );
    let observer_npc = engine.get_entity(observer).unwrap().npc_data().unwrap();
    let remaining_friends = &observer_npc.detectable_lists[DetectableType::Friend as usize];
    assert_eq!(
        remaining_friends.len(),
        1,
        "Original death fan-out deletes only the first duplicate Friend entry"
    );
    assert_eq!(remaining_friends[0].element, Some(victim));
    assert!(
        observer_npc.detectable_lists[DetectableType::MissedFriend as usize].is_empty(),
        "the ordinary unique MissedFriend entry is still removed"
    );
    let damage = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .expect("push damage remains the visual owner");
    assert_eq!(
        damage
            .orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        vec![OrderType::FallingPushedWithSword]
    );
    assert_eq!(
        engine
            .mission_domain
            .campaign
            .get_value(crate::campaign::CampaignValue::Score),
        score_before + 50,
        "the Lacklandist lethal push applies the Kill score exactly once"
    );
    assert_eq!(
        engine.mission_domain.mission_stat.killed_allied_count, killed_allied_before,
        "an enemy death must not enter the allied-death statistic arm"
    );
}

#[test]
fn surviving_push_does_not_run_npc_kill_cascade() {
    use crate::ai::{AiState, AlertLevel, Substate};

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
        victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        victim_entity
            .human_data_mut()
            .unwrap()
            .opponents
            .push(attacker);
        let ai = victim_entity.enemy_ai_mut().unwrap();
        ai.hth_weapon_id = 1;
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfight;
        ai.base.current_music_alert_status = AlertLevel::Red;
        ai.base.view_alert_status = AlertLevel::Red;
    }
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(victim);

    let mut assets = assets_with_sword_profile_effects(1, 50, 4, 0);
    let thrust = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
        [SwordStrike::A as usize];
    thrust.kind = crate::profiles::WeaponThrustKind::PushAside;
    thrust.repulsion = 100;
    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    engine.apply_sword_damage(
        &sim,
        &assets,
        victim,
        Some(attacker),
        Some(SwordStrike::A),
        Some(1),
        (sequence, 0),
    );

    let victim_entity = engine.get_entity(victim).unwrap();
    let victim_ai = victim_entity.ai_controller().unwrap();
    assert!(get_life_points(victim_entity) > 0);
    assert_eq!(victim_ai.current_state, AiState::Attacking);
    assert_eq!(victim_ai.current_substate, Substate::AttackingSwordfight);
    assert_eq!(victim_ai.current_music_alert_status, AlertLevel::Red);
    assert_eq!(
        victim_entity.human_data().unwrap().opponents,
        vec![attacker]
    );
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![victim]
    );
}

#[test]
fn surviving_push_sword_knockout_applies_one_ko_callback_and_star() {
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
    engine
        .get_entity_mut(victim)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .hth_weapon_id = 1;
    let mut assets = assets_with_sword_profile_effects(1, 50, 4, 100);
    let thrust = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
        [SwordStrike::A as usize];
    thrust.kind = crate::profiles::WeaponThrustKind::PushAside;
    thrust.repulsion = 100;

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

    let victim_entity = engine.get_entity(victim).unwrap();
    assert!(victim_entity.human_data().unwrap().unconscious);
    assert!(
        get_life_points(victim_entity) > 0,
        "fixture must exercise a surviving push knockout"
    );
    let lose_consciousness_callbacks = victim_entity
        .ai_controller()
        .unwrap()
        .ai_log
        .iter()
        .filter(|entry| {
            entry.line_type == LogLineType::Event
                && entry.info == StimulusType::EventLoseConsciousness as u16
        })
        .count();
    assert_eq!(
        lose_consciousness_callbacks, 1,
        "push-damage translation must not repeat concussion handling's synchronous callback"
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
        2,
        "fresh animated push owns concussion handling's first quit and push-damage translation's second quit"
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
        1,
        "a fresh push knockout creates one unconscious-star visual"
    );
}

#[test]
fn hit_flight_starts_from_cached_takeoff_elevation_after_installing_goal_plane() {
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
    // Deliberately make the public-number map select the wrong duplicate.
    // Falling-hit handling must reconstruct the original game's exact spatial identity.
    level.sector_number_map.insert(sector_number, 1);

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
    let mut assets = LevelAssets::new();
    assets.environment.static_sight_obstacles = std::sync::Arc::new(vec![obstacle]);

    let mut damage = crate::sequence::SequenceElement::new_damage(
        1,
        Command::ReceiveHitDamage,
        Some(victim),
        Some(attacker),
        1,
        0,
    );
    let mut fall = crate::order::Order::new(
        OrderType::FallingHitUpright,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    );
    fall.antagonist = Some(attacker);
    damage.orders.push_back(fall);
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    engine
        .get_entity_mut(victim)
        .unwrap()
        .set_posture(Posture::Flying);

    engine.initialize_hit_flight(
        &assets,
        victim,
        Some(attacker),
        OrderType::FallingHitUpright,
    );
    let flight = engine
        .get_entity(victim)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_flight
        .expect("elevated landing plane must author a hit flight");
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
        "hit-induced falling must retain takeoff preparation's cached starting 3D point"
    );

    engine.tick_push_flights(&sim, &assets);
    let position = engine
        .get_entity(victim)
        .unwrap()
        .position_iface()
        .get_position();
    assert_eq!(position.z.to_bits(), flight.increment_z.to_bits());
    assert_eq!(
        position.y.to_bits(),
        (100.0_f32 + flight.increment_y).to_bits(),
        "FallingHit accumulates the authored world-space Y increment"
    );
}

#[test]
fn damage_to_already_dead_pc_does_not_repeat_virtual_kill() {
    let sim = crate::sim_rng::SimulationContext::with_seed(0x181);
    let mut engine = make_engine();
    let victim = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let Entity::Pc(pc) = engine.get_entity_mut(victim).unwrap() else {
        unreachable!()
    };
    pc.pc.life_points = 0;
    pc.pc.trumpet_enabled = false;

    let seed_before = sim.seed();
    engine.handle_post_damage(
        &sim,
        &LevelAssets::new(),
        victim,
        0,
        false,
        None,
        false,
        (crate::sequence::SequenceId(999), 0),
        None,
    );

    assert_eq!(
        sim.seed(),
        seed_before,
        "life-point assignment returns before the repeated death cascade can select a replacement peasant"
    );
    let Entity::Pc(pc) = engine.get_entity(victim).unwrap() else {
        unreachable!()
    };
    assert!(!pc.pc.trumpet_enabled);
}

#[test]
fn charge_hit_on_already_dead_pc_does_not_repeat_virtual_kill_rng() {
    let sim = crate::sim_rng::SimulationContext::with_seed(0x182);
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_soldier(WorldPoint3D::ZERO, None));
    let victim = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .hth_weapon_id = 1;
    {
        let victim = engine.get_entity_mut(victim).unwrap();
        victim.pc_data_mut().unwrap().life_points = 0;
        victim.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
    }

    // Keep an eligible replacement in the gang so replaying the PC Kill
    // cascade would observably consume CampaignReinforcementPeasant.
    engine.mission_domain.campaign.characters = vec![
        crate::campaign::PcDescription {
            character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
            instanced: true,
            ..Default::default()
        },
        crate::campaign::PcDescription {
            character_profile_idx: Some(crate::profiles::CharacterProfileIdx(1)),
            ..Default::default()
        },
    ];
    engine.mission_domain.campaign.gang_indices = vec![0, 1];

    let mut assets = assets_with_nonstraight_profile(
        SwordStrike::Charge,
        crate::profiles::WeaponThrustKind::Straight,
    );
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .push(crate::profiles::CharacterProfile::default());
    let damage = crate::sequence::SequenceElement::new_damage(
        1,
        Command::ReceiveSwordDamage,
        Some(victim),
        Some(attacker),
        1,
        0,
    );
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    let (_, draws) = crate::sim_rng::with_draw_trace(|| {
        engine.apply_sword_damage(
            &sim,
            &assets,
            victim,
            Some(attacker),
            Some(SwordStrike::Charge),
            Some(1),
            (sequence, 0),
        );
    });

    assert_eq!(
        draws,
        vec![
            crate::sim_rng::RngSite::SwordDamageProtection,
            crate::sim_rng::RngSite::SwordDamageProtection,
            crate::sim_rng::RngSite::MeleeProvoke,
        ],
        "life-point assignment returns before death handling, while push-damage translation only owns the visual response"
    );
    assert_eq!(engine.mission_domain.campaign.gang_indices, vec![0, 1]);
    assert!(
        !engine
            .get_entity(victim)
            .unwrap()
            .pc_data()
            .unwrap()
            .trumpet_enabled,
        "an already-dead PC must not be offered another replacement"
    );
}

#[test]
fn lethal_sword_hit_preserves_queued_second_damage_fifo() {
    let sim = crate::sim_rng::SimulationContext::with_seed(0x38);
    let mut engine = make_engine();
    let attacker_a = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let attacker_b = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 20.0,
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
    for attacker in [attacker_a, attacker_b] {
        let Entity::Soldier(attacker_entity) = engine.get_entity_mut(attacker).unwrap() else {
            unreachable!()
        };
        let crate::element::AiBrain::Enemy(attacker_ai) = &mut attacker_entity.npc.ai_brain else {
            unreachable!()
        };
        attacker_ai.hth_weapon_id = 1;
    }
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.pc_data_mut().unwrap().life_points = 1;
        victim_entity.actor_data_mut().unwrap().action_state =
            crate::element::ActionState::WaitingSword;
    }

    let queue_damage = |engine: &mut EngineInner, attacker| {
        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data =
            crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
        engine.resolve_element_priority(&mut damage);
        engine.orders.sequence_manager.launch_element(damage)
    };
    let first_damage = queue_damage(&mut engine, attacker_a);
    let second_damage = queue_damage(&mut engine, attacker_b);

    let mut unrelated = crate::sequence::SequenceElement::new(1, Command::WaitTimer, Some(victim));
    engine.resolve_element_priority(&mut unrelated);
    let unrelated = engine.orders.sequence_manager.launch_element(unrelated);

    let assets = assets_with_sword_profile(200, 30);
    let (_, draws) = crate::sim_rng::with_draw_trace(|| {
        engine.hourglass_phase_sequences(
            &sim,
            &mut crate::engine::HostDisplayState::default(),
            &assets,
        );
    });

    assert_eq!(
        draws,
        vec![
            crate::sim_rng::RngSite::SwordDamageProtection,
            crate::sim_rng::RngSite::SwordDamageProtection,
            crate::sim_rng::RngSite::MeleeProvoke,
            crate::sim_rng::RngSite::SwordDamageProtection,
            crate::sim_rng::RngSite::SwordDamageProtection,
            crate::sim_rng::RngSite::MeleeProvoke,
        ],
        "both simultaneous sword hits must execute their exact damage RNG sites"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(second_damage, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::InProgress,
        "the already-dead second hit must translate into its own live dying order"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(unrelated, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Interrupted,
        "death cleanup must still discard unrelated queued owner work"
    );
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .pc_data()
            .unwrap()
            .life_points,
        0
    );
    assert_ne!(
        engine
            .orders
            .sequence_manager
            .get_element(first_damage, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Todo
    );
    assert_eq!(engine.actor_command(victim), Command::ReceiveSwordDamage);
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .actor_data()
            .unwrap()
            .installed_order
            .map(|order| order.order_type),
        Some(crate::order::OrderType::DyingSword),
        "the second damage card replaces the first while retaining Original's dying-sword lifecycle"
    );
}

#[test]
fn sword_damage_on_dying_pc_preserves_the_fresh_sprite_start() {
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
    let Entity::Soldier(attacker_entity) = engine.get_entity_mut(attacker).unwrap() else {
        unreachable!()
    };
    let crate::element::AiBrain::Enemy(attacker_ai) = &mut attacker_entity.npc.ai_brain else {
        unreachable!()
    };
    attacker_ai.hth_weapon_id = 1;
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.pc_data_mut().unwrap().life_points = 0;
        victim_entity.set_posture(Posture::Dead);
        let actor = victim_entity.actor_data_mut().unwrap();
        actor.action_state = crate::element::ActionState::WaitingSword;
        actor.continuation.motion_state = crate::sprite::MotionState::Start;
    }

    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    engine.resolve_element_priority(&mut damage);
    let damage_sequence = engine.orders.sequence_manager.launch_element(damage);

    let mut display = crate::engine::HostDisplayState::default();
    engine.hourglass_phase_sequences(&sim, &mut display, &assets_with_sword_profile(200, 30));

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(damage_sequence, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Terminated
    );
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        crate::sprite::MotionState::Start,
        "sword-damage translation changes the selection before actor instruction can stamp InProgress"
    );
}

#[test]
fn lethal_sword_damage_to_grounded_non_rider_publishes_dead_before_terminating() {
    for initial_posture in [
        Posture::Lying,
        Posture::StuckUnderNet,
        Posture::Flying,
        Posture::Carried,
        Posture::OnShoulders,
        Posture::Tied,
    ] {
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
                .publish_order_posture(initial_posture);
            victim_entity.npc_data_mut().unwrap().life_points = 1;
            victim_entity.enemy_ai_mut().unwrap().hth_weapon_id = 1;
        }
        let assets = assets_with_sword_profile_effects(200, 50, 100, 0);
        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data =
            crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
        let sequence = engine.orders.sequence_manager.launch_element(damage);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);

        engine.apply_sword_damage(
            &sim,
            &assets,
            victim,
            Some(attacker),
            Some(SwordStrike::A),
            Some(1),
            (sequence, 0),
        );

        let element = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("grounded sword damage remains registered");
        assert_eq!(
            engine.get_entity(victim).unwrap().element_data().posture(),
            Posture::Dead,
            "sword-damage translation must publish Dead for lethal {initial_posture:?} non-riders"
        );
        assert_eq!(element.state, crate::sequence::SequenceState::Terminated);
        assert!(
            element.orders.is_empty(),
            "grounded lethal {initial_posture:?} must not author a replacement animation"
        );
    }
}

#[test]
fn grounded_sword_damage_preserves_living_and_dead_rider_posture_controls() {
    for (life_points, rider, expected_state) in [
        (50, false, crate::sequence::SequenceState::Terminated),
        (1, true, crate::sequence::SequenceState::InProgress),
    ] {
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
            let Entity::Soldier(victim_entity) = engine.get_entity_mut(victim).unwrap() else {
                unreachable!()
            };
            victim_entity.element.publish_order_posture(Posture::Lying);
            victim_entity.npc.life_points = life_points;
            victim_entity.soldier.rider = rider;
            victim_entity
                .npc
                .ai_brain
                .enemy_mut()
                .unwrap()
                .hth_weapon_id = 1;
        }
        let assets = if rider {
            assets_with_sword_profile_effects(200, 50, 100, 0)
        } else {
            assets_with_sword_profile_effects(1, 50, 1, 0)
        };
        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data =
            crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
        let sequence = engine.orders.sequence_manager.launch_element(damage);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);

        engine.apply_sword_damage(
            &sim,
            &assets,
            victim,
            Some(attacker),
            Some(SwordStrike::A),
            Some(1),
            (sequence, 0),
        );

        assert_eq!(
            engine.get_entity(victim).unwrap().element_data().posture(),
            Posture::Lying,
            "living grounded actors and lethal riders bypass the Dead rewrite"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .state,
            expected_state,
            "dead riders fall through while living grounded non-riders terminate"
        );
    }
}

#[test]
fn grounded_sword_damage_resumes_same_sequence_successor_synchronously() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_soldier(WorldPoint3D::ZERO, None));
    let victim = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::ZERO
        },
        None,
    ));
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.set_posture(Posture::Lying);
        victim_entity.pc_data_mut().unwrap().life_points = 1_000;
        victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
    }
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .hth_weapon_id = 1;

    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    let mut successor =
        crate::sequence::SequenceElement::new_generic(2, Command::Generic, Some(victim));
    successor.orders.push_back(crate::order::Order::new(
        OrderType::StandingUpSword,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    ));
    successor.posture_after_transition = Posture::Upright;
    successor.action_state_after_transition = ActionState::WaitingSword;

    let mut sequence = crate::sequence::Sequence::new();
    sequence.append_element(damage);
    sequence.append_element(successor);
    let sequence_id = engine.orders.sequence_manager.launch_sequence(sequence);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);

    engine.apply_sword_damage(
        &sim,
        &assets_with_sword_profile_effects(200, 50, 100, 0),
        victim,
        Some(attacker),
        Some(SwordStrike::A),
        Some(1),
        (sequence_id, 0),
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 1)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Todo,
        "the successor remains pending until the manager pops its newly registered FIFO entry"
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .is_registered_to_go(sequence_id, 1),
        "sword-damage translation's termination must synchronously ready the successor into the manager FIFO"
    );
}

#[test]
fn sword_damage_amulet_coma_preserves_carried_body_and_terminates_during_translation() {
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
    let carried = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let Entity::Soldier(attacker_entity) = engine.get_entity_mut(attacker).unwrap() else {
        unreachable!()
    };
    let crate::element::AiBrain::Enemy(attacker_ai) = &mut attacker_entity.npc.ai_brain else {
        unreachable!()
    };
    attacker_ai.hth_weapon_id = 1;
    let sprite_script = crate::sprite_script::SpriteScript {
        action_id: crate::order::OrderType::WaitingUpright as u16,
        action_done: 0,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1],
        delays: vec![1],
        distances: vec![0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
        sound_ids: vec![0],
    };
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![sprite_script]),
        std::sync::Arc::new(vec![0]),
    );
    let mut assets = assets_with_sword_profile(200, 30);
    std::sync::Arc::make_mut(&mut assets.profile_manager).characters[0].vip = true;
    engine.mission_domain.campaign.values[crate::campaign::CampaignValue::Amulets] = 1;

    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.pc_data_mut().unwrap().life_points = 1;
        victim_entity.set_posture(Posture::CarryingCorpse);
        victim_entity.pc_data_mut().unwrap().carried = Some(carried);
        victim_entity
            .pc_data_mut()
            .unwrap()
            .set_live_carried_posture(Posture::Tied);
        victim_entity.actor_data_mut().unwrap().action_state = ActionState::Moving;
        victim_entity
            .position_iface_mut()
            .set_map_goal(crate::coordinates::MapPoint::new(25.0, 100.0));
        victim_entity
            .actor_data_mut()
            .unwrap()
            .continuation
            .motion_state = crate::sprite::MotionState::Start;
    }
    {
        let carried_entity = engine.get_entity_mut(carried).unwrap();
        carried_entity.set_posture(Posture::Carried);
        carried_entity.human_data_mut().unwrap().carrier = Some(victim);
        carried_entity.actor_data_mut().unwrap().execution_frozen = true;
    }

    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    engine.resolve_element_priority(&mut damage);
    engine.orders.sequence_manager.launch_element(damage);

    let mut display = crate::engine::HostDisplayState::default();
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    let victim_entity = engine.get_entity(victim).unwrap();
    assert!(engine.mission_domain.campaign.characters[0].status.in_coma);
    assert_eq!(victim_entity.element_data().posture(), Posture::Lying);
    assert_eq!(victim_entity.pc_data().unwrap().carried, Some(carried));
    assert_eq!(
        victim_entity.actor_data().unwrap().action_state,
        ActionState::Moving,
        "the coma posture change bypasses PC sword-damage translation's CarryingCorpse case"
    );
    let carried_entity = engine.get_entity(carried).unwrap();
    assert_eq!(carried_entity.element_data().posture(), Posture::Carried);
    assert_eq!(carried_entity.human_data().unwrap().carrier, Some(victim));
    assert!(carried_entity.actor_data().unwrap().execution_frozen);
    assert_eq!(
        carried_entity.actor_data().unwrap().installed_order,
        None,
        "the bypassed DropCorpse must not launch the carried body's Wait singleton"
    );
    assert_eq!(
        victim_entity.position_iface().map_goal(),
        crate::coordinates::MapPoint::ZERO,
        "translation-time termination must clear the interrupted movement goal"
    );
    assert_eq!(
        victim_entity
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        crate::sprite::MotionState::Start,
        "actor instruction must preserve the motion produced before damage translation"
    );
}

#[test]
fn melee_direction_uses_original_aspect_ratio_classifier() {
    let mut engine = make_engine();
    let attacker = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 663.552_37,
            y: 1_755.932_5,
            z: 0.0,
        },
        None,
    ));
    let target = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 726.867_3,
            y: 1_763.275_3,
            z: 0.0,
        },
        None,
    ));

    assert_eq!(direction_to(&engine.world.entities, attacker, target), 5);
}

#[test]
fn enter_swordfight_instruct_queues_transition_without_execute_side_effects() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let owner = engine.add_test_entity(make_pc(
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
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .set_direction_goal(7);

    let mut element =
        crate::sequence::SequenceElement::new_generic(1, Command::EnterSwordfight, Some(owner));
    element.set_property(
        crate::sequence::Field::Opponent,
        crate::sequence::FieldValue::Element(opponent),
    );
    let mut sequence = crate::sequence::Sequence::new();
    sequence.append_element(element);
    let seq_id = engine.launch_sequence(sequence);

    engine.dispatch_enter_swordfight(
        sim,
        &LevelAssets::default(),
        owner,
        Some(opponent),
        seq_id,
        0,
    );

    let owner_entity = engine.get_entity(owner).unwrap();
    assert_eq!(
        owner_entity.actor_data().unwrap().action_state,
        ActionState::Waiting,
        "Instruct must not apply the raising-sword Execute state"
    );
    assert_eq!(
        i16::from(owner_entity.position_iface().get_direction_goal()),
        7,
        "Instruct must not apply the raising-sword Execute facing"
    );
    let element = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap();
    assert_eq!(element.state, crate::sequence::SequenceState::InProgress);
    let order = element.current_order().unwrap();
    assert_eq!(
        order.order_type,
        crate::order::OrderType::TransitionRaisingSword
    );
    assert_eq!(order.antagonist, Some(opponent));
    assert!(
        owner_entity
            .human_data()
            .unwrap()
            .opponents
            .contains(&opponent),
        "relationship changes still belong to Instruct"
    );
}

#[test]
fn failed_enter_swordfight_retires_matching_postponed_thrust_a() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let owner = engine.add_test_entity(make_pc(WorldPoint3D::default(), None));
    let opponent = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::default()
        },
        None,
    ));

    let postponed = engine.launch_element(crate::sequence::SequenceElement::new_interaction(
        1,
        Command::SwordstrikeThrustA,
        Some(owner),
        Some(opponent),
    ));
    let mut enter =
        crate::sequence::SequenceElement::new_generic(1, Command::EnterSwordfight, Some(owner));
    enter.set_property(
        crate::sequence::Field::Opponent,
        crate::sequence::FieldValue::Element(opponent),
    );
    let admission = engine.launch_element(enter);
    engine
        .orders
        .sequence_manager
        .set_cross_postponed_link((admission, 0), Some((postponed, 0)));

    let Entity::Pc(opponent_entity) = engine.get_entity_mut(opponent).unwrap() else {
        unreachable!("test opponent must remain a PC")
    };
    opponent_entity.pc.life_points = 0;

    engine.dispatch_enter_swordfight(
        &sim,
        &LevelAssets::default(),
        owner,
        Some(opponent),
        admission,
        0,
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(admission, 0)
            .unwrap()
            .cross_postponed,
        None,
        "failed admission must sever the restart edge before terminal callbacks"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(postponed, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Impossible,
        "the matching THRUST_A prerequisite must not recreate the failed admission"
    );
}

#[test]
fn failed_enter_swordfight_leaves_mismatched_postponed_work_untouched() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let owner = engine.add_test_entity(make_pc(WorldPoint3D::default(), None));
    let opponent = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::default()
        },
        None,
    ));
    let postponed = engine.launch_element(crate::sequence::SequenceElement::new_interaction(
        1,
        Command::SwordstrikeThrustB,
        Some(owner),
        Some(opponent),
    ));
    let mut enter =
        crate::sequence::SequenceElement::new_generic(1, Command::EnterSwordfight, Some(owner));
    enter.set_property(
        crate::sequence::Field::Opponent,
        crate::sequence::FieldValue::Element(opponent),
    );
    let admission = engine.launch_element(enter);
    engine
        .orders
        .sequence_manager
        .set_cross_postponed_link((admission, 0), Some((postponed, 0)));
    let Entity::Pc(opponent_entity) = engine.get_entity_mut(opponent).unwrap() else {
        unreachable!("test opponent must remain a PC")
    };
    opponent_entity.pc.life_points = 0;

    engine.dispatch_enter_swordfight(
        &sim,
        &LevelAssets::default(),
        owner,
        Some(opponent),
        admission,
        0,
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(admission, 0)
            .unwrap()
            .cross_postponed,
        Some((postponed, 0)),
        "failure cleanup is specific to the THRUST_A admission prerequisite"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(postponed, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Todo
    );
}

#[test]
fn successful_enter_swordfight_retains_postponed_thrust_a() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let owner = engine.add_test_entity(make_pc(WorldPoint3D::default(), None));
    let opponent = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::default()
        },
        None,
    ));
    let postponed = engine.launch_element(crate::sequence::SequenceElement::new_interaction(
        1,
        Command::SwordstrikeThrustA,
        Some(owner),
        Some(opponent),
    ));
    let mut enter =
        crate::sequence::SequenceElement::new_generic(1, Command::EnterSwordfight, Some(owner));
    enter.set_property(
        crate::sequence::Field::Opponent,
        crate::sequence::FieldValue::Element(opponent),
    );
    let admission = engine.launch_element(enter);
    engine
        .orders
        .sequence_manager
        .set_cross_postponed_link((admission, 0), Some((postponed, 0)));

    engine.dispatch_enter_swordfight(
        &sim,
        &LevelAssets::default(),
        owner,
        Some(opponent),
        admission,
        0,
    );

    assert!(
        engine
            .get_entity(owner)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents
            .contains(&opponent),
        "control admission must succeed"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(admission, 0)
            .unwrap()
            .cross_postponed,
        Some((postponed, 0)),
        "successful admission retains the normal prerequisite chain"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(postponed, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Todo
    );
}

#[test]
fn crowded_cross_sector_swordfight_interrupts_without_a_jump_line() {
    let (engine, owner, opponent, seq_id) = dispatch_crowded_cross_sector_swordfight(3);

    let element = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .expect("interrupted element must survive the dispatch");
    // marking the sequence element interrupted — the default
    // cascade is `CASCADE_NEXT_LEVEL`. This
    // must not be Impossible/Terminated: Interrupted abandons the
    // postponed successor instead of resuming it.
    assert_eq!(element.state, crate::sequence::SequenceState::Interrupted);
    assert!(
        element.current_order().is_none(),
        "the interrupt returns before order append"
    );

    let owner_entity = engine.get_entity(owner).unwrap();
    assert_eq!(
        owner_entity.actor_data().unwrap().action_state,
        ActionState::Waiting,
        "the crowded-out PC keeps waiting instead of raising its sword"
    );
    assert!(
        !owner_entity
            .human_data()
            .unwrap()
            .opponents
            .contains(&opponent),
        "the interrupt returns before swordfight entry, so no relationship forms"
    );
}

#[test]
fn uncrowded_cross_sector_swordfight_enters_without_a_jump_line() {
    let (engine, owner, opponent, seq_id) = dispatch_crowded_cross_sector_swordfight(2);

    let element = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .expect("in-progress element must survive the dispatch");
    assert_eq!(element.state, crate::sequence::SequenceState::InProgress);
    assert_eq!(
        element.current_order().unwrap().order_type,
        crate::order::OrderType::TransitionRaisingSword
    );
    assert!(
        engine
            .get_entity(owner)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents
            .contains(&opponent),
    );
}

#[test]
fn enter_swordfight_instruct_preserves_live_sprite_destination() {
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
    let retained_goal = crate::coordinates::MapPoint::new(768.0, 1796.0);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .position_iface_mut()
        .set_map_goal(retained_goal);

    let mut sequence = crate::sequence::Sequence::new();
    sequence.append_element(crate::sequence::SequenceElement::new_generic(
        1,
        Command::EnterSwordfight,
        Some(owner),
    ));
    let seq_id = engine.launch_sequence(sequence);
    engine.dispatch_enter_swordfight(&sim, &LevelAssets::default(), owner, None, seq_id, 0);

    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal(),
        retained_goal,
        "translation must not apply TransitionRaisingSword's zero destination before Execute"
    );
}

#[test]
fn satisfied_enter_swordfight_skips_outer_instruct_epilogue() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let owner = engine.add_test_entity(make_pc(
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
    if let Some(actor) = engine.get_entity_mut(owner).unwrap().actor_data_mut() {
        actor.action_state = ActionState::WaitingSword;
    }
    if let Some(human) = engine.get_entity_mut(owner).unwrap().human_data_mut() {
        human.opponents = vec![opponent].into();
    }
    if let Some(human) = engine.get_entity_mut(opponent).unwrap().human_data_mut() {
        human.opponents = vec![owner].into();
    }

    let mut element =
        crate::sequence::SequenceElement::new_generic(1, Command::EnterSwordfight, Some(owner));
    element.set_property(
        crate::sequence::Field::Opponent,
        crate::sequence::FieldValue::Element(opponent),
    );
    let mut sequence = crate::sequence::Sequence::new();
    sequence.append_element(element);
    let seq_id = engine.launch_sequence(sequence);

    let barrier = engine.dispatch_enter_swordfight(
        &sim,
        &LevelAssets::default(),
        owner,
        Some(opponent),
        seq_id,
        0,
    );

    assert_eq!(
        barrier,
        crate::engine::sequence_runtime::OwnerActionBarrier::Skip,
        "terminal translation changes the selected element before the actor instruction's epilogue"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Terminated
    );
}

#[test]
fn got_hit_direct_entry_authors_reciprocal_enter_on_attacker() {
    use crate::ai::EnterSwordfightRequest;

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let victim = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let existing_opponent = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: -10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let attacker = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let Entity::Soldier(attacker_soldier) = engine.get_entity_mut(attacker).unwrap() else {
        unreachable!()
    };
    attacker_soldier.soldier.cached_camp = crate::element::Camp::Royalists;

    if let Some(human) = engine.get_entity_mut(victim).unwrap().human_data_mut() {
        human.opponents = vec![existing_opponent].into();
    }
    if let Some(human) = engine
        .get_entity_mut(existing_opponent)
        .unwrap()
        .human_data_mut()
    {
        human.opponents = vec![victim].into();
    }

    let mut strike_element =
        crate::sequence::SequenceElement::new(1, Command::SwordstrikeThrustA, Some(attacker));
    strike_element.priority = crate::sequence::SequencePriority::Preference;
    let mut strike = crate::sequence::Sequence::new();
    strike.append_element(strike_element);
    let strike_id = engine.launch_sequence(strike);
    let strike_order_id = engine.orders.allocate_order_id();
    let mut strike_order = crate::order::Order::new(
        crate::order::OrderType::StrikingStraightSword,
        0.0,
        0.0,
        strike_order_id,
    );
    strike_order.antagonist = Some(victim);
    engine
        .orders
        .sequence_manager
        .push_order_on(strike_id, 0, strike_order);
    engine
        .orders
        .sequence_manager
        .element_in_progress(strike_id, 0);

    let attacker_handle = (0..3)
        .find(|slot| engine.world.entities.id_at_legacy_slot(*slot) == Some(attacker))
        .expect("attacker must occupy a legacy entity slot");
    let Entity::Soldier(soldier) = engine.get_entity_mut(victim).unwrap() else {
        unreachable!()
    };
    soldier
        .npc
        .ai_brain
        .enemy_mut()
        .unwrap()
        .base
        .outbox
        .actor
        .enter_swordfight = Some(EnterSwordfightRequest::Direct(AiEntityHandle::new(
        attacker_handle,
    )));

    engine.drain_pending_for_npc(&sim, victim, &LevelAssets::default());

    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![attacker, existing_opponent],
        "opponent insertion installs the new attacker as principal"
    );
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![victim],
        "direct entry synchronously installs the reciprocal relationship"
    );
    let (enter_sequence, enter_index) = engine
        .orders
        .sequence_manager
        .pending_elements_for_owner(attacker)
        .into_iter()
        .find(|(sequence, index)| {
            engine
                .orders
                .sequence_manager
                .get_element(*sequence, *index)
                .is_some_and(|element| element.command == Command::EnterSwordfight)
        })
        .expect("the reciprocal ENTER_SWORDFIGHT must be attacker-owned");
    let enter = engine
        .orders
        .sequence_manager
        .get_element(enter_sequence, enter_index)
        .unwrap();
    assert_eq!(enter.owner, Some(attacker));
    assert!(matches!(
        enter.get_property(crate::sequence::Field::Opponent),
        Some(crate::sequence::FieldValue::Element(opponent)) if *opponent == victim
    ));
    assert!(
        !engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched(victim, Command::EnterSwordfight),
        "EVENT_GOTHIT must not defer a self-owned Engage command"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(strike_id, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::InProgress,
        "the direct call bypasses swordfight-entry preparation; interruption belongs to the reciprocal command scheduler"
    );

    let mut display = crate::engine::HostDisplayState::default();
    engine.hourglass_phase_sequences(&sim, &mut display, &LevelAssets::default());
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(enter_sequence, enter_index)
            .unwrap()
            .state,
        crate::sequence::SequenceState::InProgress,
        "the reciprocal high-priority ENTER becomes attacker-current"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(strike_id, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Postponed,
        "the reciprocal ENTER displaces the attacker's Preference strike"
    );
}

#[test]
fn direct_enter_swordfight_accepts_typed_slot_zero_opponent() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let opponent = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let initiator = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    assert_eq!(opponent.index(), 0, "control requires typed slot zero");

    assert!(engine.direct_enter_swordfight(&sim, &LevelAssets::default(), initiator, opponent,));
    assert_eq!(
        engine
            .get_entity(initiator)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![opponent]
    );
    assert_eq!(
        engine
            .get_entity(opponent)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![initiator]
    );
}

#[test]
fn direct_enter_swordfight_does_not_reject_same_camp_soldiers() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let initiator = engine.add_test_entity(make_soldier(WorldPoint3D::default(), None));
    let opponent = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::default()
        },
        None,
    ));

    assert_eq!(
        engine.get_entity(initiator).unwrap().camp(),
        crate::element::Camp::Lacklandists
    );
    assert_eq!(
        engine.get_entity(opponent).unwrap().camp(),
        crate::element::Camp::Lacklandists
    );
    assert!(engine.direct_enter_swordfight(&sim, &LevelAssets::default(), initiator, opponent,));
    assert_eq!(
        engine
            .get_entity(initiator)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![opponent]
    );
    assert_eq!(
        engine
            .get_entity(opponent)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![initiator]
    );
}

#[test]
fn enter_swordfight_los_uses_retained_raw_eye_points() {
    // schema16 seed4 Savegame_Nescafe/Profile_002/Continue replay-030,
    // frame 486: PC 342's retained world point differs by a few ULPs from
    // projection through its live plane. The shipped game preserves the
    // retained world's exact position bytes for the LOS endpoint.
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let raw_initiator = WorldPoint3D {
        x: 1108.7906,
        y: 1767.0588,
        z: 34.86987,
    };
    let raw_opponent = WorldPoint3D {
        x: 1171.0991,
        y: 1_784.021,
        z: 18.865936,
    };
    let initiator = engine.add_test_entity(make_pc(raw_initiator, None));
    let opponent = engine.add_test_entity(make_pc(raw_opponent, None));
    for (id, raw) in [(initiator, raw_initiator), (opponent, raw_opponent)] {
        let position = engine.get_entity_mut(id).unwrap().position_iface_mut();
        position.set_map_position(crate::coordinates::MapPoint::new(
            raw.x,
            raw.y - raw.z - 0.25,
        ));
        let mut state = position.v48_serialized_state();
        state.position = raw;
        state
            .computed_position
            .remove(crate::position_interface::PositionComputed::THREE_D);
        position.restore_v48_serialized_state(state);
        assert_eq!(position.get_position(), raw);
    }

    crate::sight_obstacle::begin_parity_visibility_capture();
    assert!(engine.direct_enter_swordfight(&sim, &LevelAssets::default(), initiator, opponent,));
    let queries = crate::sight_obstacle::take_parity_visibility_capture();

    assert_eq!(queries.len(), 1);
    assert_eq!(
        queries[0].origin.map(f32::to_bits),
        [
            raw_initiator.x.to_bits(),
            raw_initiator.y.to_bits(),
            (raw_initiator.z + 45.0).to_bits(),
        ]
    );
    assert_eq!(
        queries[0].destination.map(f32::to_bits),
        [
            raw_opponent.x.to_bits(),
            raw_opponent.y.to_bits(),
            (raw_opponent.z + 45.0).to_bits(),
        ]
    );
}

#[test]
fn selected_pc_entering_swordfight_does_not_restore_armed_action_on_quit() {
    use crate::profiles::Action;

    let sim = crate::sim_rng::test_context();
    let assets = action_test_assets([Action::Bow, Action::Apple, Action::Purse]);
    let mut engine = make_engine();
    let pc = engine.add_test_entity(make_pc(
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
    engine.players.seats[0].selection.push(pc);
    {
        let pc_data = engine.get_entity_mut(pc).unwrap().pc_data_mut().unwrap();
        pc_data.current_action = Action::Purse;
        pc_data.disabled_actions = vec![false; 3];
        pc_data.disabled_actions_temp = vec![false; 3];
    }

    assert!(engine.enter_swordfight(&sim, &assets, pc, opponent, false,));
    {
        let pc_data = engine.get_entity(pc).unwrap().pc_data().unwrap();
        assert_eq!(pc_data.current_action, Action::NoAction);
        assert_eq!(pc_data.saved_action, Action::NoAction);
        assert_eq!(pc_data.disabled_actions_temp, vec![true; 3]);
    }

    engine.quit_swordfight(&sim, &assets, pc);
    let pc_data = engine.get_entity(pc).unwrap().pc_data().unwrap();
    assert_eq!(pc_data.current_action, Action::NoAction);
    assert_eq!(pc_data.disabled_actions_temp, vec![false; 3]);
}

#[test]
fn unselected_pc_entering_swordfight_saves_targeted_no_action() {
    use crate::profiles::Action;

    let sim = crate::sim_rng::test_context();
    let assets = action_test_assets([Action::Bow, Action::Apple, Action::Purse]);
    let mut engine = make_engine();
    let pc = engine.add_test_entity(make_pc(
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
    {
        let pc_data = engine.get_entity_mut(pc).unwrap().pc_data_mut().unwrap();
        pc_data.current_action = Action::Bow;
        pc_data.disabled_actions = vec![false; 3];
        pc_data.disabled_actions_temp = vec![false; 3];
    }

    assert!(engine.enter_swordfight(&sim, &assets, pc, opponent, false,));
    {
        let pc_data = engine.get_entity(pc).unwrap().pc_data().unwrap();
        assert_eq!(pc_data.current_action, Action::NoAction);
        assert_eq!(pc_data.saved_action, Action::NoAction);
        assert_eq!(pc_data.disabled_actions_temp, vec![true; 3]);
    }

    engine.quit_swordfight(&sim, &assets, pc);
    let pc_data = engine.get_entity(pc).unwrap().pc_data().unwrap();
    assert_eq!(pc_data.current_action, Action::NoAction);
    assert_eq!(pc_data.disabled_actions_temp, vec![false; 3]);
    assert!(
        !engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched(pc, Command::EquipBow),
        "quitting must not restore the action that was armed before entry"
    );
}

#[test]
fn quit_swordfight_resets_moving_survivor_smalltalk_initiative() {
    use crate::profiles::Action;

    let sim = crate::sim_rng::test_context();
    let assets = action_test_assets([Action::NoAction; 3]);
    let mut engine = make_engine();
    let quitter = engine.add_test_entity(make_pc(WorldPoint3D::default(), None));
    let survivor = engine.add_test_entity(make_pc(WorldPoint3D::default(), None));
    let principal = engine.add_test_entity(make_pc(WorldPoint3D::default(), None));

    {
        let human = engine
            .get_entity_mut(quitter)
            .unwrap()
            .human_data_mut()
            .unwrap();
        human.opponents = vec![survivor].into();
    }
    {
        let survivor_entity = engine.get_entity_mut(survivor).unwrap();
        survivor_entity.actor_data_mut().unwrap().action_state = ActionState::Moving;
        let human = survivor_entity.human_data_mut().unwrap();
        human.opponents = vec![quitter, principal].into();
        human.smalltalk_initiative = false;
        human.received_smalltalk_initiative = false;
    }
    {
        let human = engine
            .get_entity_mut(principal)
            .unwrap()
            .human_data_mut()
            .unwrap();
        human.opponents = vec![survivor].into();
        human.smalltalk_initiative = true;
    }

    engine.quit_swordfight(&sim, &assets, quitter);

    let survivor_human = engine.get_entity(survivor).unwrap().human_data().unwrap();
    assert_eq!(survivor_human.opponents, vec![principal]);
    assert!(survivor_human.smalltalk_initiative);
    assert!(survivor_human.received_smalltalk_initiative);
    assert!(
        !engine
            .get_entity(principal)
            .unwrap()
            .human_data()
            .unwrap()
            .smalltalk_initiative,
        "mutual principal must lose initiative even while the survivor is Moving"
    );
}

#[test]
fn quit_swordfight_does_not_reset_initiative_without_surviving_opponents() {
    use crate::profiles::Action;

    let sim = crate::sim_rng::test_context();
    let assets = action_test_assets([Action::NoAction; 3]);
    let mut engine = make_engine();
    let quitter = engine.add_test_entity(make_pc(WorldPoint3D::default(), None));
    let survivor = engine.add_test_entity(make_pc(WorldPoint3D::default(), None));

    {
        let human = engine
            .get_entity_mut(quitter)
            .unwrap()
            .human_data_mut()
            .unwrap();
        human.opponents = vec![survivor].into();
    }
    {
        let survivor_entity = engine.get_entity_mut(survivor).unwrap();
        survivor_entity.actor_data_mut().unwrap().action_state = ActionState::Moving;
        let human = survivor_entity.human_data_mut().unwrap();
        human.opponents = vec![quitter].into();
        human.smalltalk_initiative = false;
        human.received_smalltalk_initiative = false;
    }

    engine.quit_swordfight(&sim, &assets, quitter);

    let survivor_human = engine.get_entity(survivor).unwrap().human_data().unwrap();
    assert!(survivor_human.opponents.is_empty());
    assert!(!survivor_human.smalltalk_initiative);
    assert!(!survivor_human.received_smalltalk_initiative);
}

#[test]
fn preparing_swordfight_orders_done_enter_then_queues_reciprocal() {
    use crate::ai::{AiState, LogLineType, StimulusType, Substate};
    use crate::profiles::{CharacterProfile, HtHWeaponProfile, ProfileManager, SoldierProfile};

    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let initiator = engine.add_test_entity(make_soldier(
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
    {
        let Entity::Soldier(soldier) = engine.get_entity_mut(initiator).unwrap() else {
            unreachable!()
        };
        soldier.soldier.cached_camp = crate::element::Camp::Royalists;
        let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
        ai.base.me = initiator.index();
        ai.hth_weapon_id = 1;
    }
    {
        let Entity::Soldier(soldier) = engine.get_entity_mut(opponent).unwrap() else {
            unreachable!()
        };
        let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
        ai.base.me = opponent.index();
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingOfficerGivingOrders;
        ai.hth_weapon_id = 1;
    }

    // Give the opponent a selected command for swordfight-entry preparation's
    // Stop(PREFERENCE) to interrupt. Its condolence sends EventDone.
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

    let mut profiles = ProfileManager::new();
    profiles.hth_weapons.push(HtHWeaponProfile {
        distance: [30, 50, 60, 70],
        ..HtHWeaponProfile::default()
    });
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

    assert!(engine.enter_swordfight(sim, &assets, initiator, opponent, false));

    assert!(
        engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched_or_postponed_by_current(
                opponent,
                Command::EnterSwordfight,
            ),
        "non-Wait reciprocal entry remains on the manager FIFO after swordfight entry returns"
    );
    let ai = engine
        .get_entity(opponent)
        .unwrap()
        .ai_controller()
        .unwrap();
    let events: Vec<_> = ai
        .ai_log
        .iter()
        .filter(|entry| entry.line_type == LogLineType::Event)
        .map(|entry| entry.info)
        .collect();
    assert_eq!(
        events,
        vec![
            StimulusType::EventDone as u16,
            StimulusType::EventEnterSwordfight as u16
        ],
        "the interrupted command must complete in the old substate before swordfight entry"
    );
    assert_eq!(ai.current_substate, Substate::AttackingSwordfight);
}

#[test]
fn swordfight_distance_keeps_original_strict_minimum_boundary() {
    use super::super::evaluate::{
        SwordfightDistanceAdjustment as Adjustment, swordfight_distance_adjustment,
    };

    // Savegame_008/replay-012 reaches this representable distance after
    // one ordinary 12-unit swordfight correction. Original compares it
    // directly with the 45-unit MINIMAL range and requests another move.
    assert_eq!(
        swordfight_distance_adjustment(44.999_71, 45.0, 65.0, 65.0, false),
        Adjustment::Farther,
    );
    assert_eq!(
        swordfight_distance_adjustment(45.0, 45.0, 65.0, 65.0, false),
        Adjustment::None,
    );
}

#[test]
fn swordfight_distance_keeps_original_strict_maximum_and_step_back_guards() {
    use super::super::evaluate::{
        SwordfightDistanceAdjustment as Adjustment, swordfight_distance_adjustment,
    };

    assert_eq!(
        swordfight_distance_adjustment(65.000_01, 45.0, 65.0, 60.0, false),
        Adjustment::Closer,
    );
    assert_eq!(
        swordfight_distance_adjustment(65.0, 45.0, 65.0, 60.0, false),
        Adjustment::None,
    );
    assert_eq!(
        swordfight_distance_adjustment(65.000_01, 45.0, 65.0, 60.0, true),
        Adjustment::None,
    );
}
