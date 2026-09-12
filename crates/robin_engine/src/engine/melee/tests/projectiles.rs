use super::*;

#[test]
fn lying_arrow_victim_speaks_before_posture_termination() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(WorldPoint3D::default(), None));
    let lying = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 20.0,
            ..WorldPoint3D::default()
        },
        None,
    ));
    let upright = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 40.0,
            ..WorldPoint3D::default()
        },
        None,
    ));
    engine
        .get_entity_mut(lying)
        .unwrap()
        .set_posture(Posture::Lying);
    for victim in [lying, upright] {
        engine
            .get_entity_mut(victim)
            .unwrap()
            .enemy_ai_mut()
            .unwrap()
            .hth_weapon_id = 1;
    }

    let mut assets = assets_with_sword_profile(1, 50);
    std::sync::Arc::make_mut(&mut assets.profile_manager).soldiers[0].exclamation_id = 0x5744_0000;

    for victim in [lying, upright] {
        let damage = crate::sequence::SequenceElement::new_damage(
            1,
            Command::ReceiveArrowDamage,
            Some(victim),
            Some(attacker),
            1,
            0,
        );
        let sequence = engine.launch_element(damage);
        engine.dispatch_receive_damage(&sim, &assets, victim, sequence, 0);
    }

    assert_eq!(
        engine
            .feedback
            .sound_sim
            .pending_exclamations
            .iter()
            .map(|pending| (pending.actor_id, pending.exclamation_id))
            .collect::<Vec<_>>(),
        vec![(lying.index(), crate::ai::Remark::Wounded as u16)],
        "the lying actor speaks first and its type-wide Wounded forbid rejects the later actor"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(lying),
        None,
        "the lying damage element still terminates after hurt speech"
    );
}

#[test]
fn sword_strike_consideration_latch_is_one_shot_when_honour_rejects() {
    let mut engine = make_engine();
    let (attacker, target) = make_enemy_strike_pair(&mut engine, true);
    let assets = assets_with_sword_profile(7, 30);
    engine.control.rng = SimulationRng::with_original_replay(Vec::new());
    engine
        .get_entity_mut(target)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::Waiting;

    engine.with_simulation_context(|engine, sim| {
        engine.tick_enemy_sword_attacks(sim, &assets);
    });
    let cursor_after_first = engine.control.rng.original_replay_cursor().unwrap();
    assert_eq!(cursor_after_first, 0, "honour rejection precedes proposal");
    let pending_after_first = engine
        .get_entity(attacker)
        .and_then(Entity::enemy_ai)
        .unwrap()
        .pending_sword_strike_consideration;
    assert!(!pending_after_first, "the authorization must be one-shot");

    engine.with_simulation_context(|engine, sim| {
        engine.tick_enemy_sword_attacks(sim, &assets);
    });
    assert_eq!(
        engine.control.rng.original_replay_cursor(),
        Some(cursor_after_first),
        "the rejected, consumed latch must not retry next frame"
    );
}

#[test]
fn consecutive_lethal_arrow_damage_preserves_new_amulet_coma() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
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
        victim_entity.pc_data_mut().unwrap().life_points = 10;
        victim_entity
            .position_iface_mut()
            .set_map_goal(crate::coordinates::MapPoint::new(25.0, 100.0));
        victim_entity
            .actor_data_mut()
            .unwrap()
            .continuation
            .motion_state = crate::sprite::MotionState::Start;
    }

    let mut damage = crate::sequence::SequenceElement::new_damage(
        1,
        Command::ReceiveArrowDamage,
        Some(victim),
        Some(attacker),
        10,
        0,
    );
    engine.resolve_element_priority(&mut damage);
    engine.orders.sequence_manager.launch_element(damage);

    let mut display = crate::engine::HostDisplayState::default();
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    {
        let victim_entity = engine.get_entity(victim).unwrap();
        assert!(engine.mission_domain.campaign.characters[0].status.in_coma);
        assert_eq!(victim_entity.pc_data().unwrap().life_points, 5);
        assert_eq!(victim_entity.element_data().posture(), Posture::Lying);
        assert_eq!(
            victim_entity.position_iface().map_goal(),
            crate::coordinates::MapPoint::ZERO,
            "post-damage Lying translation must terminate and clear the movement goal"
        );
        assert_eq!(
            victim_entity
                .actor_data()
                .unwrap()
                .continuation
                .motion_state,
            crate::sprite::MotionState::Start,
            "terminal arrow translation must preserve the pre-damage motion state"
        );
    }
    assert_eq!(
        engine.mission_domain.campaign.values[crate::campaign::CampaignValue::Amulets],
        0,
        "the first lethal arrow must establish coma and consume one amulet"
    );

    let mut second_damage = crate::sequence::SequenceElement::new_damage(
        1,
        Command::ReceiveArrowDamage,
        Some(victim),
        Some(attacker),
        10,
        0,
    );
    engine.resolve_element_priority(&mut second_damage);
    engine.orders.sequence_manager.launch_element(second_damage);
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    let victim_entity = engine.get_entity(victim).unwrap();
    assert_eq!(victim_entity.pc_data().unwrap().life_points, 5);
    assert!(!victim_entity.is_dead());
    assert!(engine.mission_domain.campaign.characters[0].status.in_coma);
    assert_eq!(
        engine.mission_domain.campaign.values[crate::campaign::CampaignValue::Amulets],
        0,
        "the second lethal arrow must not consume another amulet"
    );
}

#[test]
fn sherwood_lethal_arrow_still_consumes_amulet_without_hurting_pc() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
    let victim = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
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
    let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
    profiles.characters[0].vip = true;
    profiles.missions.push(crate::profiles::MissionProfile {
        location: crate::profiles::MissionLocation::Sherwood,
        ..Default::default()
    });
    engine
        .mission_domain
        .campaign
        .missions
        .push(crate::mission::Mission {
            profile_idx: Some(0),
            ..Default::default()
        });
    engine.mission_domain.campaign.current_mission_idx = Some(0);
    engine.mission_domain.campaign.values[crate::campaign::CampaignValue::Amulets] = 1;

    {
        let victim = engine.get_entity_mut(victim).unwrap();
        victim.pc_data_mut().unwrap().life_points = 100;
        victim.human_data_mut().unwrap().concussion_of_the_brain = 0;
        victim.human_data_mut().unwrap().unconscious = false;
    }

    let mut damage = crate::sequence::SequenceElement::new_damage(
        1,
        Command::ReceiveArrowDamage,
        Some(victim),
        Some(attacker),
        100,
        20,
    );
    engine.resolve_element_priority(&mut damage);
    engine.orders.sequence_manager.launch_element(damage);

    let mut display = crate::engine::HostDisplayState::default();
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    let victim = engine.get_entity(victim).unwrap();
    assert_eq!(victim.pc_data().unwrap().life_points, 100);
    assert_eq!(victim.human_data().unwrap().concussion_of_the_brain, 0);
    assert!(!victim.human_data().unwrap().unconscious);
    assert_eq!(victim.element_data().posture(), Posture::Lying);
    assert!(engine.mission_domain.campaign.characters[0].status.in_coma);
    assert_eq!(
        engine.mission_domain.campaign.values[crate::campaign::CampaignValue::Amulets],
        0
    );
}

#[test]
fn same_frame_arrow_after_death_replaces_dying_order_and_then_rolls() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
    let victim = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    engine
        .get_entity_mut(victim)
        .unwrap()
        .pc_data_mut()
        .unwrap()
        .life_points = 1;

    let mut obstacle = crate::sight_obstacle::SightObstacle::new_default(0);
    obstacle.top_plane_points = [[0.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 1.0, 0.0]];
    let mut assets = action_test_assets([crate::profiles::Action::NoAction; 3]);
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

    let mut launched = Vec::new();
    for _ in 0..2 {
        let mut damage = crate::sequence::SequenceElement::new_damage(
            1,
            Command::ReceiveArrowDamage,
            Some(victim),
            Some(attacker),
            1,
            0,
        );
        engine.resolve_element_priority(&mut damage);
        launched.push(engine.orders.sequence_manager.launch_element(damage));
    }

    let mut display = crate::engine::HostDisplayState::default();
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(victim),
        Some((launched[1], 0)),
        "the second injury must replace the first dying element"
    );
    let second = engine
        .orders
        .sequence_manager
        .get_element(launched[1], 0)
        .expect("second arrow damage remains registered");
    assert_eq!(second.state, crate::sequence::SequenceState::InProgress);
    assert_eq!(
        second
            .orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        vec![OrderType::DyingUpright, OrderType::Rolling],
        "arrow-damage translation must author DyingUpright before roll translation"
    );
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .actor_data()
            .unwrap()
            .installed_order
            .as_ref()
            .map(|order| order.order_type),
        Some(OrderType::DyingUpright)
    );
}

#[test]
fn arrow_damage_to_dead_grounded_actor_sets_dead_and_terminates_without_orders() {
    for (initial_posture, use_pc) in [
        (Posture::Lying, true),
        (Posture::StuckUnderNet, true),
        (Posture::Flying, true),
        (Posture::Carried, true),
        // PC dispatch intercepts OnShoulders. A Soldier reaches
        // the human actor's literal OnShoulders fallthrough.
        (Posture::OnShoulders, false),
        (Posture::Tied, true),
    ] {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
        let victim = engine.add_entity(if use_pc {
            make_pc(WorldPoint3D::ZERO, None)
        } else {
            make_soldier(WorldPoint3D::ZERO, None)
        });
        {
            let victim = engine.get_entity_mut(victim).unwrap();
            let (_, life_points) = victim
                .human_and_life_points_mut()
                .expect("grounded test victim must be human");
            *life_points = 0;
            victim
                .element_data_mut()
                .publish_order_posture(initial_posture);
        }
        let assets = action_test_assets([crate::profiles::Action::NoAction; 3]);
        let mut damage = crate::sequence::SequenceElement::new_damage(
            1,
            Command::ReceiveArrowDamage,
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
            .expect("dead-body arrow damage remains registered");
        assert_eq!(
            element.state,
            crate::sequence::SequenceState::Terminated,
            "{initial_posture:?} must enter the terminating fallthrough"
        );
        assert!(element.orders.is_empty());
        assert_eq!(
            engine.get_entity(victim).unwrap().element_data().posture(),
            Posture::Dead,
            "arrow-damage translation changes dead {initial_posture:?} non-riders to Dead"
        );
    }
}

#[test]
fn arrow_damage_to_pc_on_shoulders_uses_virtual_shoulder_translation() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
    let carrier = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    let victim = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    engine
        .get_entity_mut(carrier)
        .unwrap()
        .pc_data_mut()
        .unwrap()
        .carried = Some(victim);
    {
        let victim = engine.get_entity_mut(victim).unwrap();
        victim
            .element_data_mut()
            .publish_order_posture(Posture::OnShoulders);
        victim.human_data_mut().unwrap().carrier = Some(carrier);
    }

    let assets = action_test_assets([crate::profiles::Action::NoAction; 3]);
    let mut damage = crate::sequence::SequenceElement::new_damage(
        1,
        Command::ReceiveArrowDamage,
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
        .expect("shoulder arrow damage remains registered");
    assert_ne!(element.state, crate::sequence::SequenceState::Terminated);
    assert_eq!(
        element.orders.front().map(|order| order.order_type),
        Some(OrderType::FallingShoulders),
        "PC arrow-damage translation must dispatch shoulder-damage translation"
    );
    assert_ne!(
        engine.get_entity(victim).unwrap().element_data().posture(),
        Posture::Dead,
        "PC OnShoulders must not enter Human's dead-grounded fallthrough"
    );
}
