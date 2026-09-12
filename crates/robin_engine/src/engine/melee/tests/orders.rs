use super::*;

#[test]
fn reactive_strike_recognition_uses_command_not_replacement_animation() {
    fn run(remembered: SwordStrike, busy: bool) -> (usize, crate::ai::Substate, usize) {
        let mut engine = make_engine();
        let (victim, attacker) = make_enemy_strike_pair(&mut engine, false);
        engine.world.fast_grid_mut().size_map(4, 4);
        engine.world.fast_grid_mut().allocate_layers(1);
        engine.world.fast_grid_mut().add_sector(
            crate::fast_find_grid::GridSector {
                points: vec![
                    crate::coordinates::MapPoint::new(0.0, 0.0),
                    crate::coordinates::MapPoint::new(256.0, 0.0),
                    crate::coordinates::MapPoint::new(256.0, 256.0),
                    crate::coordinates::MapPoint::new(0.0, 256.0),
                ],
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
            // Keep the attacker inside the H thrust's 50-unit desired
            // push-back distance.  At 80 units Original returns the
            // victim's current point as a successful step-back goal;
            // synchronous movement completion then restores ordinary
            // swordfight state before strike-warning processing returns, so that
            // fixture cannot distinguish H's PushAside geometry.
            .set_position(WorldPoint3D::new(130.0, 100.0, 0.0));
        for actor in [victim, attacker] {
            let sprite = &mut engine
                .get_entity_mut(actor)
                .unwrap()
                .element_data_mut()
                .sprite;
            let mut scripts = vec![
                crate::sprite_script::SpriteScript {
                    action_done: 10,
                    frame_ids: (0..16).collect(),
                    delays: vec![1; 16],
                    distances: vec![0; 16],
                    offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 16],
                    sound_ids: vec![0; 16],
                    ..Default::default()
                };
                16
            ];
            // The incoming H animation has ten frames left, while the
            // victim's authored parry transition starts in three. This
            // satisfies Original's strict startup deadline and lets the
            // test observe the later PushAside geometry branch.
            scripts[1].action_done = 3;
            sprite.scripts = std::sync::Arc::new(scripts);
            let mut conversion = vec![0; crate::sprite_script::NONANIMATION_END];
            conversion[OrderType::TransitionWaitingSwordParryingSword as usize] = 1;
            sprite.conversion = std::sync::Arc::new(conversion);
        }

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

        // The selected request remains F while its installed replacement
        // row is H, exactly separating the original game's command selection from
        // animation queries at the strike-warning boundary.
        let strike_element = crate::sequence::SequenceElement::new_interaction(
            1,
            Command::SwordstrikeThrustF,
            Some(attacker),
            Some(victim),
        );
        let strike_sequence = engine.launch_element(strike_element);
        let strike_order = engine.push_new_order(
            strike_sequence,
            0,
            OrderType::StrikingRoundLeftSword,
            100.0,
            100.0,
        );
        engine
            .orders
            .sequence_manager
            .element_in_progress(strike_sequence, 0);
        {
            let attacker_entity = engine.get_entity_mut(attacker).unwrap();
            let actor = attacker_entity.actor_data_mut().unwrap();
            actor.action_state = ActionState::WaitingSword;
            actor.installed_order = Some(crate::element::InstalledActorOrder {
                order_id: strike_order,
                order_type: OrderType::StrikingRoundLeftSword,
            });
            let sprite = &mut attacker_entity.element_data_mut().sprite;
            sprite.current_row = 0;
            sprite.current_frame = 0;
            sprite.frame_count = 0;
            sprite.action_done_frame = 10;
            sprite.action_done_counter = 1;
            sprite.last_action = OrderType::StrikingRoundLeftSword;
        }

        let mut assets = assets_with_sword_profile(7, 30);
        let profiles = std::sync::Arc::get_mut(&mut assets.profile_manager).unwrap();
        profiles.soldiers[0].fighting = 50;
        profiles.hth_weapons[0].thrusts[SwordStrike::H as usize].kind =
            crate::profiles::WeaponThrustKind::PushAside;
        profiles.hth_weapons[0].thrusts[SwordStrike::H as usize].maximal_distance = 30;
        let ai = engine
            .get_entity_mut(victim)
            .and_then(Entity::enemy_ai_mut)
            .unwrap();
        ai.known_enemy_strike_1 = Some(remembered);
        if busy {
            ai.base.locks_flag_field = crate::ai::AiLockFlags::BUSY;
        }

        // 65 selects parade at ability 50. Only the H animation's
        // PushAside geometry can turn that parade into a step-back.
        engine.control.rng = SimulationRng::with_original_replay(vec![85]);
        engine.with_simulation_context(|engine, sim| {
            engine.warn_for_strike(sim, &assets, attacker, &[victim], SwordStrike::H);
        });
        let ai = engine
            .get_entity(victim)
            .and_then(Entity::enemy_ai)
            .unwrap();
        if busy {
            assert_eq!(
                ai.base.stimulus_queue[0].stimulus_type,
                crate::ai::StimulusType::EventSwordStrike,
            );
            assert_eq!(
                ai.base.stimulus_queue[0].info,
                crate::ai::StimulusInfo::Human(crate::ai::AiEntityHandle::new(attacker.index())),
                "queued EVENT_SWORDSTRIKE must retain the attacking human"
            );
        }
        (
            engine.control.rng.original_replay_cursor().unwrap(),
            ai.base.current_substate,
            ai.base.stimulus_queue.len(),
        )
    }

    assert_eq!(
        run(SwordStrike::F, false),
        (1, crate::ai::Substate::AttackingSwordfightStepBack, 0,),
        "command F must admit the proposal while animation H supplies PushAside geometry"
    );
    assert_eq!(
        run(SwordStrike::H, false),
        (0, crate::ai::Substate::AttackingSwordfight, 0),
        "remembering only replacement H must not admit selected command F"
    );
    let locked = run(SwordStrike::F, true);
    assert_eq!(
        locked.0, 0,
        "BUSY decision entry must not reach the proposal RNG"
    );
    assert_eq!(
        locked.1,
        crate::ai::Substate::AttackingSwordfight,
        "BUSY warning must not launch a parade or counter-strike"
    );
    assert_eq!(locked.2, 1);
}

#[test]
fn lateral_done_keeps_actor_scan_order_and_does_not_recover_out_of_arc_antagonist() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    // Facing south (sector 8), thrust D covers sectors 4..=9. Keep the
    // valid victims on either side of the out-of-arc antagonist in actor
    // creation order so the assertion also guards the collector FIFO.
    let first_in_arc = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 20.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let antagonist = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: -20.0,
            y: 120.0,
            z: 0.0,
        },
        None,
    ));
    let second_in_arc = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 120.0,
            z: 0.0,
        },
        None,
    ));
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(8);

    let mut assets =
        assets_with_nonstraight_profile(SwordStrike::D, crate::profiles::WeaponThrustKind::Lateral);
    let thrust = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
        [SwordStrike::D as usize];
    thrust.initial_angle = 90;
    thrust.final_angle = 22;
    thrust.rotation_angle = 45;

    assert_eq!(
        crate::position_interface::vector_to_sector_0_to_15(-20.0, 20.0),
        10,
        "the interaction antagonist must be outside thrust D's actor-scan arc"
    );
    let selected =
        install_test_melee_order(&mut engine, attacker, antagonist, SwordStrike::D, false);

    assert_eq!(
        engine.tick_nonstraight_melee_for(sim, &assets, attacker, selected),
        strikes::SweepTickPhase::Initialized
    );
    let pending = &engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("the in-arc actors initialize the lateral sweep")
        .pending_victims;
    assert_eq!(pending, &[first_in_arc, second_in_arc]);
    assert!(!pending.contains(&antagonist));
}

#[test]
fn interrupted_circle_sweep_preserves_geometry_before_replacement_action_point() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    engine
        .get_entity_mut(victim)
        .and_then(Entity::enemy_ai_mut)
        .unwrap()
        .hth_weapon_id = 1;

    let mut profile_manager = crate::profiles::ProfileManager::new();
    let mut weapon = crate::profiles::HtHWeaponProfile::default();
    for strike in [SwordStrike::I, SwordStrike::F] {
        let thrust = &mut weapon.thrusts[strike as usize];
        thrust.kind = crate::profiles::WeaponThrustKind::TrueCircle;
        thrust.direction = crate::profiles::WeaponThrustDirection::LeftToRight;
        thrust.minimal_distance = 0;
        thrust.maximal_distance = 100;
        thrust.initial_angle = 0;
        thrust.final_angle = 360;
        thrust.rotation_angle = 45;
    }
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
        install_test_melee_order(&mut engine, attacker, victim, SwordStrike::I, true);
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
        pending_victims: vec![victim],
        initial_angle: 0.0,
        current_angle: 0.0,
        final_angle: std::f32::consts::TAU,
        rotation_per_frame: std::f32::consts::FRAC_PI_4,
        direction: crate::profiles::WeaponThrustDirection::LeftToRight,
        strike: SwordStrike::I,
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
    replacement_element.command = SwordStrike::F.to_command();
    let replacement_order = replacement_element
        .orders
        .front_mut()
        .expect("retained strike order exists");
    replacement_order.order_type = strike_to_animation(SwordStrike::F);
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
        assert_eq!(sprite.action_done_counter, 0);
        sprite.current_frame = 3;
        sprite.frame_count = 0;
    }
    engine.tick_melee_strikes(sim, &assets);

    let attacker_entity = engine.get_entity(attacker).unwrap();
    let retained_before_action = attacker_entity
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("replacement pre-action frames retain the interrupted circle sweep");
    assert_eq!(
        retained_before_action.strike,
        SwordStrike::F,
        "Original reads replacement F's effect parameters before its action point"
    );
    assert_eq!(retained_before_action.current_angle, 0.0);
    assert_eq!(
        attacker_entity.element_data().direction(),
        7,
        "the interrupted circle geometry must not rotate replacement strike F before its action point"
    );

    engine.tick_melee_strikes(sim, &assets);
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .as_ref()
            .expect("replacement circle initializes its own sweep at action done")
            .strike,
        SwordStrike::F,
    );
}

#[test]
fn replacement_true_circle_uses_current_direction_at_action_done() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
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

    let current_angle = sector_to_angle(13);
    {
        let entity = engine.get_entity_mut(attacker).unwrap();
        entity.element_data_mut().set_direction_instantly(15);
        entity.actor_data_mut().unwrap().sweep_state = Some(crate::movement::SweepState {
            pending_victims: Vec::new(),
            initial_angle: current_angle,
            current_angle,
            final_angle: current_angle + std::f32::consts::FRAC_PI_2,
            rotation_per_frame: std::f32::consts::FRAC_PI_2,
            // Stale retained right-to-left/false-F metadata says the
            // sweep is complete. Current G is a left-to-right true
            // circle and must keep rotating without progressing sprite.
            direction: crate::profiles::WeaponThrustDirection::RightToLeft,
            strike: SwordStrike::F,
            attacker_profile_idx: Some(1),
            gesture_quality: crate::player_command::GestureQuality::PERFECT,
            strike_kind: crate::profiles::WeaponThrustKind::FalseHalfCircle,
        });
    }

    engine.tick_selected_melee_owner(sim, &assets, attacker, selected);

    let attacker_entity = engine.get_entity(attacker).unwrap();
    let sprite = &attacker_entity.element_data().sprite;
    assert_eq!(attacker_entity.element_data().direction(), 13);
    assert_eq!(sprite.current_frame, sprite.action_done_frame);
    assert_eq!(sprite.frame_count, sprite.action_done_counter);
    assert!(
        attacker_entity.actor_data().unwrap().sweep_state.is_some(),
        "current G's left-to-right direction keeps the retained geometry rotating"
    );
}

#[test]
fn saved_human_sweep_is_rehydrated_for_the_live_strike_order() {
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let assets =
        assets_with_nonstraight_profile(SwordStrike::E, crate::profiles::WeaponThrustKind::Lateral);
    install_test_melee_order(&mut engine, attacker, victim, SwordStrike::E, true);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .sword_sweep = crate::element::HumanSwordSweepState {
        victims: vec![victim],
        initial_angle: 0.0,
        current_angle: 0.0,
        final_angle: std::f32::consts::PI,
    };
    assert!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .is_none()
    );

    engine.rebind_retained_sweep_to_active_strike(&assets, attacker);

    let sweep = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("serialized human sweep must regain its executable mirror");
    assert_eq!(sweep.pending_victims, vec![victim]);
    assert_eq!(sweep.initial_angle, 0.0);
    assert_eq!(sweep.current_angle, 0.0);
    assert_eq!(sweep.final_angle, std::f32::consts::PI);
    assert_eq!(sweep.strike, SwordStrike::E);
    assert_eq!(
        sweep.strike_kind,
        crate::profiles::WeaponThrustKind::Lateral
    );

    engine.tick_sweep_for(&assets, attacker, false);
    assert!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .sword_sweep
            .victims
            .is_empty(),
        "consuming the executable victim must consume the serialized human mirror too"
    );
    engine.rebind_retained_sweep_to_active_strike(&assets, attacker);
    assert!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .is_none(),
        "the consumed save victim must not be rehydrated and hit again next frame"
    );
}

#[test]
fn slope_translate_roll_order_keeps_its_source_authored_direction_recompute() {
    let mut engine = make_engine();
    let victim = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    let mut obstacle = crate::sight_obstacle::SightObstacle::new_default(0);
    obstacle.top_plane_points = [[0.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 1.0, 0.0]];
    let mut assets = LevelAssets::new();
    assets.environment.static_sight_obstacles = std::sync::Arc::new(vec![obstacle]);
    {
        let victim = engine.get_entity_mut(victim).unwrap();
        victim.element_data_mut().set_obstacle_index(
            crate::position_interface::ObstacleHandle::new(0),
            Some(crate::position_interface::PlaneZCoeffs {
                az: 1.0,
                bz: 0.0,
                dz: 0.0,
            }),
        );
        victim
            .position_iface_mut()
            .set_move_box(crate::coordinates::MoveBox::from_corners(
                crate::coordinates::MapVec::new(-5.0, -5.0),
                crate::coordinates::MapVec::new(5.0, 5.0),
            ));
    }
    let damage = crate::sequence::SequenceElement::new(1, Command::ReceiveDamage, Some(victim));
    let sequence = engine.orders.sequence_manager.launch_element(damage);

    engine.try_queue_roll(&assets, victim, (sequence, 0));

    let rolling = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .unwrap()
        .orders
        .iter()
        .find(|order| order.order_type == OrderType::Rolling)
        .expect("roll translation must append its Rolling order");
    assert!(rolling.compute_direction);
}

#[test]
fn lateral_done_processes_victims_in_original_actor_order_before_good_strike() {
    use crate::ai::{AiState, LogLineType, Remark, StimulusType, Substate};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(WorldPoint3D::default(), None));
    // Allocate the survivor first so typed entity iteration disagrees with
    // Original's actor registry below. This is the Save016 shape: the
    // later-ID victim must knock out and unlink the attacker first.
    let survivor = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 20.0,
            ..WorldPoint3D::default()
        },
        None,
    ));
    let knockout = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::default()
        },
        None,
    ));
    engine.world.install_original_creation_orders(
        [(attacker, 0), (knockout, 1), (survivor, 2)]
            .into_iter()
            .collect(),
        3,
    );

    {
        let Entity::Soldier(soldier) = engine.get_entity_mut(attacker).unwrap() else {
            unreachable!()
        };
        // Only the first Original-order victim is an opponent. Its KO
        // therefore synchronously sends EventQuitSwordfight to the
        // attacker before damage reaches the later survivor.
        soldier.human.opponents.push(knockout);
        let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
        ai.base.me = attacker.index();
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfightSpecialStrike;
        ai.hth_weapon_id = 1;
    }
    engine
        .get_entity_mut(knockout)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(attacker);
    // Keep the later victim conscious while retaining real cutting damage,
    // so it would emit GoodStrike if processed before the KO callback.
    engine
        .get_entity_mut(survivor)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .invulnerable = true;

    let mut assets = assets_with_sword_profile_effects(1, 100, 4, 100);
    let thrust = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
        [SwordStrike::A as usize];
    thrust.kind = crate::profiles::WeaponThrustKind::Lateral;
    thrust.direction = crate::profiles::WeaponThrustDirection::LeftToRight;
    thrust.initial_angle = 0;
    thrust.final_angle = 180;
    thrust.rotation_angle = 90;

    let victims = engine.execute_multi_target_strike(&assets, attacker, SwordStrike::A, Some(1));
    assert_eq!(
        victims,
        [knockout, survivor],
        "DONE membership is unchanged, but follows Original actor FIFO rather than typed IDs"
    );

    for victim in victims {
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
    }

    assert!(
        engine
            .get_entity(knockout)
            .unwrap()
            .human_data()
            .unwrap()
            .unconscious,
        "first victim must exercise the synchronous knockout/quit arm"
    );
    assert!(
        !engine
            .get_entity(survivor)
            .unwrap()
            .human_data()
            .unwrap()
            .unconscious,
        "later cutting victim must remain a genuine surviving control"
    );
    let ai = engine
        .get_entity(attacker)
        .unwrap()
        .ai_controller()
        .unwrap();
    assert_eq!(ai.current_substate, Substate::AttackingQuittingSwordfight);
    assert!(
        ai.ai_log.iter().any(|entry| {
            entry.line_type == LogLineType::Event
                && entry.info == StimulusType::EventGoodStrike as u16
        }),
        "later survivor must deliver a real GoodStrike after the first victim quits"
    );
    assert!(
        !ai.ai_log.iter().any(|entry| {
            entry.line_type == LogLineType::Speak && entry.info == Remark::GoodStrikeCombat as u16
        }),
        "later GoodStrike is delivered after quit and must not start speech"
    );
    assert!(
        engine.feedback.sound_sim.pending_exclamations.is_empty(),
        "ignored later GoodStrike must not leave a pending combat exclamation"
    );
}

#[test]
fn no_animation_fresh_push_knockout_does_not_repeat_ko_side_effects() {
    use crate::ai::{LogLineType, StimulusType};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    let victim = engine.add_entity(make_soldier(
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
            .publish_order_posture(Posture::Carried);
        victim_entity.human_data_mut().unwrap().unconscious = true;
        victim_entity.enemy_ai_mut().unwrap().hth_weapon_id = 1;
    }
    let assets = assets_with_sword_profile(1, 50);
    // Model concussion handling's already-completed fresh-KO prefix,
    // then translate push damage without an animation.
    engine.apply_knockout_side_effects(&sim, &assets, victim, true, false);
    let damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .set_translating_element(Some((
            victim,
            crate::sequence::SequenceElementRef::new(sequence, 0),
        )));

    assert!(engine.apply_push_effect(
        &sim,
        &assets,
        victim,
        attacker,
        &PushStrikeInfo { repulsion: 100 },
        crate::combat::SwordDamageResult::STUNNING_DAMAGE,
        (sequence, 0),
        true,
    ));
    engine.orders.sequence_manager.set_translating_element(None);

    let victim_entity = engine.get_entity(victim).unwrap();
    assert!(victim_entity.human_data().unwrap().unconscious);
    assert_eq!(victim_entity.element_data().posture(), Posture::Lying);
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
        1,
        "push damage without an animation must not repeat a fresh KO callback"
    );
}

#[test]
fn reconsider_rebalance_updates_opponents_without_recursive_enter_command() {
    use crate::ai::EnterSwordfightRequest;

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let owner = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let old_primary = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let replacement = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 20.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    if let Some(human) = engine.get_entity_mut(owner).unwrap().human_data_mut() {
        human.opponents = vec![old_primary, replacement].into();
    }
    if let Some(human) = engine.get_entity_mut(replacement).unwrap().human_data_mut() {
        human.opponents = vec![owner].into();
    }
    let replacement_handle = (0..3)
        .find(|slot| engine.world.entities.id_at_legacy_slot(*slot) == Some(replacement))
        .expect("replacement PC must occupy a legacy entity slot");
    let Entity::Soldier(soldier) = engine.get_entity_mut(owner).unwrap() else {
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
        .enter_swordfight = Some(EnterSwordfightRequest::Rebalance(AiEntityHandle::new(
        replacement_handle,
    )));

    engine.drain_pending_for_npc(&sim, owner, &LevelAssets::default());

    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents
            .first(),
        Some(&replacement),
        "direct swordfight entry must promote the replacement opponent"
    );
    let Entity::Soldier(soldier) = engine.get_entity(owner).unwrap() else {
        unreachable!()
    };
    assert_eq!(
        soldier.npc.ai_brain.enemy().unwrap().base.primary_target,
        Some(AiEntityHandle::new(replacement_handle)),
        "successful rebalance must promote the AI primary target"
    );
    assert!(
        !engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched(owner, Command::EnterSwordfight),
        "swordfight reconsideration's direct call must not author another Enter command"
    );
}

#[test]
fn enabling_temp_actions_restores_matching_slot_after_targeted_selection_collapse() {
    use crate::profiles::Action;

    let assets = action_test_assets([Action::Bow, Action::Apple, Action::Purse]);
    let mut engine = make_engine();
    let pc = engine.add_entity(make_pc(WorldPoint3D::default(), None));
    let companion = engine.add_entity(make_pc(WorldPoint3D::default(), None));
    engine.players.seats[0].selection = vec![pc, companion];
    {
        let pc_data = engine.get_entity_mut(pc).unwrap().pc_data_mut().unwrap();
        pc_data.current_action = Action::NoAction;
        pc_data.saved_action = Action::Purse;
        pc_data.disabled_actions = vec![false; 3];
        pc_data.disabled_actions_temp = vec![true; 3];
    }
    engine
        .get_entity_mut(companion)
        .unwrap()
        .pc_data_mut()
        .unwrap()
        .current_action = Action::Bow;

    engine.enable_pc_actions_temp(&assets, 0, pc);

    let pc_data = engine.get_entity(pc).unwrap().pc_data().unwrap();
    assert_eq!(pc_data.current_action, Action::Purse);
    assert_eq!(pc_data.disabled_actions_temp, vec![false; 3]);
    assert_eq!(
        engine
            .get_entity(companion)
            .unwrap()
            .pc_data()
            .unwrap()
            .current_action,
        Action::Bow,
        "the messenger removes the companion before fanning out the targeted restored action"
    );
    assert_eq!(engine.players.seats[0].selection, vec![pc]);
    assert_eq!(engine.players.seats[0].selected_action, Action::Purse);
    assert!(
        engine
            .feedback
            .pending_side_effects
            .invalidate_trajectory_preview
    );
}

#[test]
fn enabling_temp_actions_does_not_restore_action_absent_from_profile_slots() {
    use crate::profiles::Action;

    let assets = action_test_assets([Action::Bow, Action::Apple, Action::Purse]);
    let mut engine = make_engine();
    let pc = engine.add_entity(make_pc(WorldPoint3D::default(), None));
    engine.players.seats[0].selection.push(pc);
    {
        let pc_data = engine.get_entity_mut(pc).unwrap().pc_data_mut().unwrap();
        pc_data.current_action = Action::NoAction;
        pc_data.saved_action = Action::Stone;
        pc_data.disabled_actions = vec![false; 3];
        pc_data.disabled_actions_temp = vec![true; 3];
    }

    engine.enable_pc_actions_temp(&assets, 0, pc);

    let pc_data = engine.get_entity(pc).unwrap().pc_data().unwrap();
    assert_eq!(pc_data.current_action, Action::NoAction);
    assert_eq!(pc_data.disabled_actions_temp, vec![false; 3]);
    assert_eq!(engine.players.seats[0].selected_action, Action::NoAction);
    assert!(
        !engine
            .feedback
            .pending_side_effects
            .invalidate_trajectory_preview
    );
}
