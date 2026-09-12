use super::*;

#[test]
fn lethal_piercing_damage_quits_swordfight_from_a_flying_posture() {
    use crate::element::{Command, Posture};
    use crate::sequence::SequenceElement;

    let mut assets = assets_with_test_pc_profile();
    let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
    profiles.characters[0].hth_weapon_id = 1;
    profiles.soldiers.push(crate::profiles::SoldierProfile {
        hth_weapon_id: 1,
        ..Default::default()
    });
    profiles.hth_weapons.push(Default::default());
    let mut engine = EngineInner::new();
    let victim = engine.add_test_entity(make_test_pc(Posture::Flying));
    let opponent = engine.add_test_entity(super::super::scenarios::make_test_ai_soldier(
        crate::element::Camp::Lacklandists,
    ));
    attach_test_campaign_identities(&mut engine);
    engine
        .get_entity_mut(opponent)
        .and_then(Entity::enemy_ai_mut)
        .expect("test soldier has EnemyAi")
        .hth_weapon_id = 1;
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        *victim_entity.human_and_life_points_mut().unwrap().1 = 20;
        victim_entity.human_data_mut().unwrap().opponents = vec![opponent].into();
    }
    engine
        .get_entity_mut(opponent)
        .and_then(Entity::human_data_mut)
        .unwrap()
        .opponents = vec![victim].into();

    let damage = SequenceElement::new_damage(
        1,
        Command::ReceiveArrowDamage,
        Some(victim),
        Some(opponent),
        20,
        0,
    );
    let sequence = engine.orders.sequence_manager.launch_element(damage);

    engine.dispatch_receive_damage(
        &crate::sim_rng::test_context(),
        &assets,
        victim,
        sequence,
        0,
    );

    assert_eq!(
        engine.get_entity(victim).unwrap().human_life_points(),
        0,
        "the arrow is lethal"
    );
    // Human life updating runs death handling inside piercing damage, so
    // Arrow-damage translation's flying arm terminates its element on a corpse
    // that already left every opponent list.
    assert!(
        engine
            .get_entity(victim)
            .and_then(Entity::human_data)
            .unwrap()
            .opponents
            .is_empty(),
        "the killed victim leaves its own opponent list"
    );
    assert!(
        engine
            .get_entity(opponent)
            .and_then(Entity::human_data)
            .unwrap()
            .opponents
            .is_empty(),
        "the killed victim is removed from its opponent's list"
    );
}

#[test]
fn piercing_damage_on_ladder_applies_damage_before_fall_translation() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let victim = engine.add_test_entity(make_test_pc(Posture::OnLadder));
    attach_test_campaign_identities(&mut engine);

    let damage =
        SequenceElement::new_damage(1, Command::ReceiveArrowDamage, Some(victim), None, 20, 0);
    let sequence = engine.orders.sequence_manager.launch_element(damage);

    engine.dispatch_receive_damage(
        &crate::sim_rng::test_context(),
        &LevelAssets::default(),
        victim,
        sequence,
        0,
    );

    assert_eq!(engine.get_entity(victim).unwrap().human_life_points(), 80);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .orders
            .front()
            .map(|order| order.order_type),
        Some(OrderType::FallingLadderWall),
        "the piercing hit still translates to the ladder-fall reaction"
    );
}

#[test]
fn enter_swordfight_corpse_exit_registers_then_drops_on_first_execute() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;

    let (mut engine, carrier, body, _) =
        corpse_exit_initialization_fixture(false, Command::EnterSwordfight);

    assert_eq!(
        engine.get_entity(carrier).unwrap().posture(),
        Posture::CarryingCorpse
    );
    assert_eq!(engine.get_entity(body).unwrap().posture(), Posture::Carried);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(carrier)
            .map(|(_, _, order)| order.order_type),
        Some(OrderType::TransitionCarryingCorpseWaitingUpright),
        "translation frame must retain the registered corpse-exit owner"
    );

    engine.tick_actor_animation_action_change_slots(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
    );

    let carrier_entity = engine.get_entity(carrier).unwrap();
    assert_eq!(carrier_entity.posture(), Posture::Upright);
    assert_eq!(carrier_entity.pc_data().unwrap().carried, None);
    let body_entity = engine.get_entity(body).unwrap();
    assert_eq!(body_entity.posture(), Posture::Lying);
    assert_eq!(body_entity.human_data().unwrap().carrier, None);
    assert!(!body_entity.actor_data().unwrap().execution_frozen);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(carrier)
            .map(|(_, _, order)| order.order_type),
        Some(OrderType::TransitionRaisingSword),
        "first Execute must terminate the corpse-exit prefix and expose the sword order"
    );
    assert_eq!(
        carrier_entity
            .actor_data()
            .unwrap()
            .installed_order
            .map(|order| order.order_type),
        Some(OrderType::TransitionRaisingSword),
        "order advancement must publish the successor in the same owner boundary"
    );
}

#[test]
fn heal_done_revalidates_before_effect_and_ammo_consumption() {
    use crate::coordinates::MapPoint;
    use crate::element::{Command, ElementData, ElementFx, ElementKind, Entity, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceState};

    enum TargetKind {
        Human(f32),
        Fx(f32),
        SelfHeal,
    }

    for (target_kind, expect_effect) in [
        (TargetKind::Human(40.0), false),
        (TargetKind::Human(39.999), true),
        (TargetKind::Fx(80.0), true),
        (TargetKind::SelfHeal, true),
    ] {
        let mut engine = EngineInner::new();
        let healer = engine.add_test_entity(make_test_pc(Posture::Upright));
        let healer_entity = engine.get_entity_mut(healer).unwrap();
        healer_entity.pc_data_mut().unwrap().life_points = 100;
        healer_entity
            .element_data_mut()
            .set_position_map(MapPoint::ZERO);

        let target = match target_kind {
            TargetKind::Human(distance) => {
                let target = engine.add_test_entity(make_test_pc(Posture::Upright));
                let entity = engine.get_entity_mut(target).unwrap();
                entity.pc_data_mut().unwrap().life_points = 50;
                entity
                    .element_data_mut()
                    .set_position_map(MapPoint::new(distance, 0.0));
                target
            }
            TargetKind::Fx(distance) => {
                let mut element = {
                    let mut initial_element = ElementData::default();
                    initial_element.kind = ElementKind::Target;
                    initial_element.active = true;
                    initial_element
                };
                element.set_position_map(MapPoint::new(distance, 0.0));
                engine.add_test_entity(Entity::Fx(ElementFx {
                    element,
                    fx: Default::default(),
                }))
            }
            TargetKind::SelfHeal => {
                engine
                    .get_entity_mut(healer)
                    .unwrap()
                    .pc_data_mut()
                    .unwrap()
                    .life_points = 50;
                healer
            }
        };

        attach_test_campaign_identities(&mut engine);
        let healer_description = engine
            .get_entity(healer)
            .and_then(Entity::pc_data)
            .and_then(|pc| pc.campaign_description_index)
            .unwrap() as usize;
        engine.mission_domain.campaign.characters[healer_description]
            .status
            .set_ammo(crate::profiles::Action::Heal, 2);

        let sprite_action = if target == healer {
            OrderType::Eating
        } else {
            OrderType::Healing
        };
        // Use a multi-frame action whose DONE point is distinct from its
        // terminal frame. The one-frame action-point helper terminates on its
        // first tick and never exercises HealDone's second validity check.
        let script = crate::sprite_script::SpriteScript {
            action_id: sprite_action as u16,
            action_done: 1,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1, 2, 3],
            delays: vec![0, 0, 0],
            distances: vec![0, 0, 0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
            sound_ids: vec![0; 3],
        };
        let mut conversion =
            vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
        conversion[sprite_action as usize] = 0;
        let element = engine.get_entity_mut(healer).unwrap().element_data_mut();
        let position = element.position_map();
        let direction = element.direction();
        element.sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script]),
            std::sync::Arc::new(conversion),
        );
        element.set_position_map(position);
        element.set_direction_instantly(direction);

        let element =
            SequenceElement::new_interaction(1, Command::HealCmd, Some(healer), Some(target));
        let sequence = engine.orders.sequence_manager.launch_element(element);
        assert!(engine.get_entity(healer).unwrap().is_pc());
        assert!(!engine.get_entity(healer).unwrap().is_dead());
        assert!(
            engine.get_entity(target).unwrap().kind().is_fx_target()
                || engine
                    .get_entity(target)
                    .and_then(Entity::pc_data)
                    .is_some_and(|pc| pc.life_points > 0 && pc.life_points < 100)
        );
        assert_eq!(
            crate::abilities::begin_heal(
                &mut engine.world.entities,
                &mut engine.orders.sequence_manager,
                healer,
                target,
                sequence,
                0,
                &mut engine.orders.next_order_id,
            ),
            crate::abilities::BeginResult::Started
        );
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);

        let target_life_before = engine
            .get_entity(target)
            .and_then(Entity::pc_data)
            .map(|pc| pc.life_points);
        let assets = assets_with_test_pc_profile();
        let sim = crate::sim_rng::test_context();
        let mut display = CameraDisplayState::default();
        for _ in 0..4 {
            engine.tick_ability_for(&sim, &mut display, &assets, healer);
            let ammo = engine.mission_domain.campaign.characters[healer_description]
                .status
                .get_ammo(crate::profiles::Action::Heal);
            if ammo < 2
                || engine
                    .orders
                    .sequence_manager
                    .get_element(sequence, 0)
                    .is_some_and(|element| element.state == SequenceState::Terminated)
            {
                break;
            }
        }

        let ammo = engine.mission_domain.campaign.characters[healer_description]
            .status
            .get_ammo(crate::profiles::Action::Heal);
        if expect_effect {
            assert_eq!(ammo, 1);
            if let Some(life_before) = target_life_before {
                assert!(
                    engine
                        .get_entity(target)
                        .and_then(Entity::pc_data)
                        .unwrap()
                        .life_points
                        > life_before
                );
            }
        } else {
            assert_eq!(ammo, 2, "invalid Heal DONE must not consume a plant");
            assert_eq!(
                engine
                    .get_entity(target)
                    .and_then(Entity::pc_data)
                    .map(|pc| pc.life_points),
                target_life_before,
                "invalid Heal DONE must not apply its effect"
            );
            assert_eq!(
                engine
                    .orders
                    .sequence_manager
                    .get_element(sequence, 0)
                    .unwrap()
                    .state,
                SequenceState::Terminated
            );
            assert_eq!(
                engine
                    .get_entity(healer)
                    .and_then(Entity::actor_data)
                    .unwrap()
                    .continuation
                    .motion_state,
                crate::sprite::MotionState::Terminated
            );
            let healer_actor = engine
                .get_entity(healer)
                .and_then(Entity::actor_data)
                .unwrap();
            assert!(
                !healer_actor.active_ability.is_active(),
                "the synchronous owner condolence must clear the terminated Heal mirror"
            );
            assert!(
                engine
                    .orders
                    .sequence_manager
                    .current_order_for_actor(healer)
                    .is_none(),
                "the invalid Healing order must no longer be selected"
            );
            assert_eq!(
                engine.actor_order_type(healer),
                Some(OrderType::NonanimationEnd),
                "the owner condolence must detach the installed Healing order"
            );
        }
    }
}

#[test]
fn moving_strangle_victim_event_stop_precedes_next_owner_live_initialization() {
    use crate::element::{ActionState, Command, Posture};
    use crate::sequence::{SequenceElement, SequenceState};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let _null_handle_slot = engine.add_test_entity(make_test_pc(Posture::Upright));
    let attacker = engine.add_test_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_test_entity(make_test_soldier(Posture::Upright));
    let crate::element::Entity::Soldier(victim_soldier) = engine.get_entity_mut(victim).unwrap()
    else {
        unreachable!()
    };
    victim_soldier.soldier.cached_camp = crate::element::Camp::Lacklandists;
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_position_map(crate::coordinates::MapPoint::new(0.0, 0.0));
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .set_position_map(crate::coordinates::MapPoint::new(20.0, 0.0));
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(8);
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(8);
    engine
        .get_entity_mut(victim)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::Moving;
    engine.dispatch_ai_stimulus(
        victim,
        crate::ai::Stimulus::new(crate::ai::StimulusType::EventTimer),
    );
    let seq = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new_interaction(
            1,
            Command::StrangleCmd,
            Some(attacker),
            Some(victim),
        ));
    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    let element = engine
        .orders
        .sequence_manager
        .get_element(seq, 0)
        .expect("strangle interaction identity must survive synchronous EventStop effects");
    assert_eq!(element.state, SequenceState::InProgress);
    let order = element
        .current_order()
        .expect("strangle order must remain selected");
    let active = &engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_ability;
    assert_eq!(active.sequence_id, Some(seq));
    assert_eq!(active.element_index, 0);
    assert_eq!(active.target, Some(victim));
    assert_eq!(active.order_id, Some(order.order_id));

    let victim_ai = engine.get_entity(victim).unwrap().ai_controller().unwrap();
    assert!(
        !victim_ai
            .locks_flag_field
            .contains(crate::ai::AiLockFlags::FREEZE)
    );
    assert_eq!(
        victim_ai
            .ai_log
            .iter()
            .filter(|line| {
                line.line_type == crate::ai::LogLineType::Event
                    && line.info == crate::ai::StimulusType::EventStop as u16
            })
            .count(),
        1,
        "EventStop Think must complete while FREEZE is still absent",
    );
    assert_eq!(victim_ai.current_state, crate::ai::AiState::Seeking);
    assert_eq!(
        victim_ai.current_substate,
        crate::ai::Substate::SeekingGotStopEvent,
        "EventStop's re-entrant state/timer effects must be applied before FREEZE",
    );
    assert!(victim_ai.timer_is_running);
    assert_eq!(
        victim_ai
            .outbox
            .detection
            .stimuli
            .iter()
            .map(|stimulus| stimulus.stimulus_type)
            .collect::<Vec<_>>(),
        vec![crate::ai::StimulusType::EventTimer],
        "synchronous EventStop and its re-entrant effects must preserve the older FIFO",
    );
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .element_data()
            .direction(),
        8,
        "translation must not change the attacker direction",
    );
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .element_data()
            .direction(),
        8,
        "translation must not change the victim direction",
    );
    assert_eq!(
        i16::from(
            engine
                .get_entity(attacker)
                .unwrap()
                .position_iface()
                .get_direction_goal()
        ),
        8,
        "translation must not eagerly set the attacker goal",
    );
    assert_eq!(
        i16::from(
            engine
                .get_entity(victim)
                .unwrap()
                .position_iface()
                .get_direction_goal()
        ),
        8,
        "translation must not eagerly set the victim goal",
    );

    let mut invalid = engine.clone();
    invalid
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .set_position_map(crate::coordinates::MapPoint::new(100.0, 100.0));
    invalid.tick_ability_for(&sim, &mut CameraDisplayState::default(), &assets, attacker);
    assert_eq!(
        invalid
            .orders
            .sequence_manager
            .get_element(seq, 0)
            .unwrap()
            .state,
        SequenceState::Impossible,
        "first owner Execute must recheck live Strangle validity",
    );
    assert!(
        !invalid
            .get_entity(victim)
            .unwrap()
            .ai_controller()
            .unwrap()
            .locks_flag_field
            .contains(crate::ai::AiLockFlags::FREEZE)
    );
    assert!(
        !invalid
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .is_active()
    );

    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .set_position_map(crate::coordinates::MapPoint::new(0.0, 20.0));
    let live_facing = crate::position_interface::vector_to_sector_0_to_15_iso(0.0, 20.0);
    let mut camera_display = CameraDisplayState::default();
    engine.tick_ability_for(&sim, &mut camera_display, &assets, attacker);

    let victim_ai = engine.get_entity(victim).unwrap().ai_controller().unwrap();
    assert!(
        victim_ai
            .locks_flag_field
            .contains(crate::ai::AiLockFlags::FREEZE)
    );
    assert_eq!(
        i16::from(
            engine
                .get_entity(attacker)
                .unwrap()
                .position_iface()
                .get_direction_goal()
        ),
        live_facing,
        "first owner Execute must compute the attacker goal from live positions",
    );
    assert_eq!(
        i16::from(
            engine
                .get_entity(victim)
                .unwrap()
                .position_iface()
                .get_direction_goal()
        ),
        live_facing,
        "first owner Execute must compute the victim goal from live positions",
    );
}

#[test]
fn moving_hit_victim_receives_synchronous_event_stop_and_blinks_enemy() {
    use crate::element::{ActionState, Command, Detectable, DetectableType, Posture};
    use crate::sequence::{SequenceElement, SequenceState};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let attacker = engine.add_test_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_test_entity(make_test_soldier(Posture::Upright));
    let crate::element::Entity::Soldier(victim_soldier) = engine.get_entity_mut(victim).unwrap()
    else {
        unreachable!()
    };
    victim_soldier.soldier.cached_camp = crate::element::Camp::Lacklandists;
    victim_soldier.actor.action_state = ActionState::MovingFast;
    victim_soldier.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
        element: Some(attacker),
        detectable_type: DetectableType::Enemy,
        seen_now: true,
        seen_last_frame: true,
        ..Detectable::default()
    });

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let seq = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new_interaction(
            1,
            Command::HitCmd,
            Some(attacker),
            Some(victim),
        ));
    engine.hourglass_phase_sequences(&sim, &mut HostDisplayState::default(), &assets);

    let hit = engine
        .orders
        .sequence_manager
        .get_element(seq, 0)
        .expect("Hit remains selected after the victim's re-entrant EventStop");
    assert_eq!(hit.state, SequenceState::InProgress);
    assert_eq!(
        hit.current_order().unwrap().order_type,
        crate::order::OrderType::Hitting
    );
    let victim = engine.get_entity(victim).unwrap();
    let ai = victim.ai_controller().unwrap();
    assert_eq!(ai.current_state, crate::ai::AiState::Seeking);
    assert_eq!(
        ai.current_substate,
        crate::ai::Substate::SeekingGotStopEvent
    );
    assert_eq!(ai.view_alert_status, crate::ai::AlertLevel::Yellow);
    assert_eq!(ai.current_music_alert_status, crate::ai::AlertLevel::Yellow);
    assert_eq!(
        ai.ai_log
            .iter()
            .filter(|line| {
                line.line_type == crate::ai::LogLineType::Event
                    && line.info == crate::ai::StimulusType::EventStop as u16
            })
            .count(),
        1
    );
    let enemy = &victim.npc_data().unwrap().detectable_lists[DetectableType::Enemy as usize][0];
    assert!(!enemy.seen_now);
    assert!(!enemy.seen_last_frame);
}

#[test]
fn hit_done_rechecks_live_target_distance_before_launching_damage() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceState};
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    fn bind_hitting(engine: &mut EngineInner, attacker: EntityId) {
        let script = SpriteScript {
            action_id: OrderType::Hitting as u16,
            action_done: 2,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1, 2, 3, 4],
            delays: vec![0; 4],
            distances: vec![0; 4],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 4],
            sound_ids: vec![0; 4],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[OrderType::Hitting as usize] = 0;
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            .sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );
    }

    fn receive_hit_damage_count(engine: &EngineInner, victim: EntityId) -> usize {
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .filter(|element| {
                element.command == Command::ReceiveHitDamage && element.owner == Some(victim)
            })
            .count()
    }

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let attacker = engine.add_test_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_test_entity(make_test_soldier(Posture::Upright));
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .active = true;
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.element_data_mut().active = true;
        victim_entity
            .element_data_mut()
            .set_position_map(crate::coordinates::MapPoint::new(20.0, 0.0));
        let crate::element::Entity::Soldier(victim_soldier) = victim_entity else {
            unreachable!()
        };
        victim_soldier.soldier.cached_camp = crate::element::Camp::Lacklandists;
    }
    bind_hitting(&mut engine, attacker);
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let seq = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new_interaction(
            1,
            Command::HitCmd,
            Some(attacker),
            Some(victim),
        ));
    assert_eq!(
        crate::abilities::begin_hit(
            &mut engine.world.entities,
            &mut engine.orders.sequence_manager,
            attacker,
            victim,
            seq,
            0,
            &mut engine.orders.next_order_id,
        ),
        crate::abilities::BeginResult::Started
    );
    engine.orders.sequence_manager.element_in_progress(seq, 0);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .execute_order_initialising = true;

    let mut display = CameraDisplayState::default();
    engine.tick_ability_for(&sim, &mut display, &assets, attacker);
    assert!(
        !engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .done_effect_applied,
        "the first valid Execute must leave a later terminal boundary to recheck"
    );
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .execute_order_initialising = false;

    let mut in_range = engine.clone();
    let mut out_of_range = engine;
    in_range
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .set_position_map(crate::coordinates::MapPoint::new(39.0, 0.0));
    out_of_range
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .set_position_map(crate::coordinates::MapPoint::new(41.0, 0.0));

    for branch in [&mut in_range, &mut out_of_range] {
        for _ in 0..10 {
            branch.tick_ability_for(&sim, &mut CameraDisplayState::default(), &assets, attacker);
            if branch
                .get_entity(attacker)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_ability
                .done_effect_applied
            {
                break;
            }
        }
        assert!(
            branch
                .get_entity(attacker)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_ability
                .done_effect_applied,
            "both branches must reach the real Hitting Done boundary"
        );
    }

    let in_range_sprite = &in_range.get_entity(attacker).unwrap().element_data().sprite;
    let out_of_range_sprite = &out_of_range
        .get_entity(attacker)
        .unwrap()
        .element_data()
        .sprite;
    assert_eq!(
        out_of_range_sprite.current_frame,
        in_range_sprite.current_frame + 1,
        "terminal invalidity must perform Original's extra virgin frame increment"
    );
    assert_eq!(
        out_of_range_sprite.frame_count, in_range_sprite.frame_count,
        "the zero-delay fixture makes the virgin increment advance exactly one frame"
    );
    assert_eq!(
        out_of_range_sprite.last_motion_state,
        Some(crate::sprite::MotionState::Done),
        "the virgin increment must not replace Hitting's terminal Done edge"
    );
    assert_eq!(receive_hit_damage_count(&in_range, victim), 1);
    assert_eq!(receive_hit_damage_count(&out_of_range, victim), 0);
    assert_eq!(
        out_of_range
            .orders
            .sequence_manager
            .get_element(seq, 0)
            .expect("missed Hit remains selected until normal animation termination")
            .state,
        SequenceState::InProgress,
        "terminal invalidity suppresses damage without aborting or making Hit Impossible"
    );
}

#[test]
fn strangle_authorized_placement_failure_cleans_exact_owner_before_post_authorization_effects() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceState};
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let _null_handle_slot = engine.add_test_entity(make_test_pc(Posture::Upright));
    let attacker = engine.add_test_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_test_entity(make_test_soldier(Posture::Upright));
    assert_ne!(
        attacker.index(),
        0,
        "the attacker must have a non-null legacy AI handle so EventGotHit observation is meaningful"
    );
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .active = true;
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .active = true;
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    // Keep the synthetic strangle hotspot within the victim's effective
    // sword range after the failed placement. The original game's enemy attack truncates
    // the raw norm to 16 bits and compares it with standard range + 10; the old
    // (7, 9) offset truncates to 11 and correctly authors an approach Move,
    // obscuring the synchronous EnterSwordfight behavior this test covers.
    let hotspot = crate::coordinates::SpriteLocalPoint::new(6.0, 7.0);
    let script = SpriteScript {
        action_id: OrderType::Strangling as u16,
        action_done: 1,
        average_speed: 0.0,
        hotspot,
        sum_distance: 0,
        frame_ids: vec![1, 2, 3],
        delays: vec![0, 0, 0],
        distances: vec![0, 0, 0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0; 3],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[OrderType::Strangling as usize] = 0;
    let attacker_sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .sprite = attacker_sprite;
    {
        let element = engine.get_entity_mut(attacker).unwrap().element_data_mut();
        element.sprite.current_row = 0;
        element.set_position_map(crate::coordinates::MapPoint::new(100.0, 120.0));
        element.set_layer(3);
        element.set_sector(crate::position_interface::SectorHandle::new(2));
    }
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        let element = victim_entity.element_data_mut();
        element.set_layer(8);
        element.set_sector(crate::position_interface::SectorHandle::new(5));
        element.set_direction_instantly(6);
        victim_entity.npc_data_mut().unwrap().eye_status = crate::element::EyeStatus::LookToTheLeft;
    }
    // EventGotHit synchronously enters the enemy retaliation path, which
    // resolves the attacker's real sector while checking for a lift approach.
    // Production actors cannot carry a non-null sector that is absent from
    // the level grid; install the ordinary sector used by this synthetic
    // attacker rather than weakening that runtime invariant.
    {
        let sector_number = crate::sector::SectorNumber::new(2);
        let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
        level
            .sector_number_map
            .insert(sector_number, level.sectors.len());
        level.sectors.push(crate::fast_find_grid::GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type: crate::sector::SectorType::empty(),
            layer: 3,
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
        });
    }
    let expected_action_point = {
        let attacker = engine.get_entity(attacker).unwrap();
        let sprite_pos = attacker.gameplay_sprite_position();
        crate::coordinates::MapPoint::new(sprite_pos.x + hotspot.x, sprite_pos.y + hotspot.y)
    };
    let victim_frame_before = engine
        .get_entity(victim)
        .unwrap()
        .element_data()
        .sprite
        .current_frame;
    let sequence_count_before = engine.orders.sequence_manager.sequences_iter().count();
    let seq = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new_interaction(
            1,
            Command::StrangleCmd,
            Some(attacker),
            Some(victim),
        ));
    assert_eq!(
        crate::abilities::begin_strangle(
            &mut engine.world.entities,
            &mut engine.orders.sequence_manager,
            attacker,
            victim,
            seq,
            0,
            &mut engine.orders.next_order_id,
        ),
        crate::abilities::BeginResult::Started
    );
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_ability
        .strangle_initialized = true;
    let attacker_topology = {
        let element = engine.get_entity(attacker).unwrap().element_data();
        (
            element.layer(),
            element.sector(),
            element.obstacle_index(),
            element.direction(),
        )
    };
    engine.orders.sequence_manager.element_in_progress(seq, 0);
    let mut display = CameraDisplayState::default();

    let (_, condolation_order) =
        crate::engine::soldier_helpers::capture_strangle_condolation_order(|| {
            for _ in 0..10 {
                engine.tick_ability_for(&sim, &mut display, &assets, attacker);
                if !engine
                    .get_entity(attacker)
                    .unwrap()
                    .actor_data()
                    .unwrap()
                    .active_ability
                    .is_active()
                {
                    break;
                }
            }
        });
    assert_eq!(
        condolation_order,
        ["Wait", "Unlock", "EventGotHit", "LookForward"]
    );

    assert!(
        !engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .is_active()
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq, 0)
            .unwrap()
            .state,
        SequenceState::Impossible
    );
    let victim_entity = engine.get_entity(victim).unwrap();
    assert_eq!(
        victim_entity.element_data().position_map(),
        expected_action_point
    );
    assert_eq!(victim_entity.element_data().layer(), 3);
    assert_eq!(
        (
            victim_entity.element_data().layer(),
            victim_entity.element_data().sector(),
            victim_entity.element_data().obstacle_index(),
            victim_entity.element_data().direction(),
        ),
        attacker_topology,
        "failed authorization retains the topology copied before the search"
    );
    assert!(!victim_entity.actor_data().unwrap().execution_frozen);
    assert_ne!(
        victim_entity.element_data().sprite.last_action,
        OrderType::BeingStrangled
    );
    assert_eq!(
        victim_entity.element_data().sprite.current_frame,
        victim_frame_before,
        "failed setup must not virgin-increment the victim"
    );
    assert_eq!(
        engine.orders.sequence_manager.sequences_iter().count(),
        sequence_count_before + 4,
        "failed setup appends Strangle, victim Wait, and faithful hit-retaliation orders"
    );
    let victim_commands: Vec<_> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .filter(|element| element.owner == Some(victim))
        .map(|element| element.command)
        .collect();
    assert_eq!(
        victim_commands,
        [
            Command::Wait,
            Command::EnterSwordfight,
            Command::EnterAttentiveMode
        ],
        "Original EVENT_GOTHIT retaliation remains synchronous after the failed Strangle setup"
    );
    let victim_waits: Vec<_> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| {
            sequence
                .elements
                .iter()
                .map(move |element| (sequence.id, element))
        })
        .filter(|(_, element)| element.owner == Some(victim) && element.command == Command::Wait)
        .collect();
    assert_eq!(victim_waits.len(), 1);
    assert!(
        victim_waits[0].0 > seq,
        "condolation Wait must be appended synchronously after its Interaction owner"
    );
    assert_eq!(
        victim_entity.npc_data().unwrap().eye_status,
        crate::element::EyeStatus::LookForward
    );
    assert!(
        !victim_entity.ai_controller().unwrap().ai_is_locked(),
        "owner-boundary condolation must unlock the failed victim before returning"
    );
    assert!(
        victim_entity
            .ai_controller()
            .expect("soldier fixture requires an AI controller")
            .outbox
            .reentrant
            .owner_work
            .is_empty(),
        "failed authorization must not enqueue emergency speech owner work"
    );
    let victim_ai = victim_entity.ai_controller().unwrap();
    assert!(
        victim_ai.outbox.detection.stimuli.is_empty(),
        "synchronous EventGotHit Think must finish before tick_ability_for returns"
    );
    assert_eq!(
        victim_ai.primary_target,
        Some(crate::ai::AiEntityHandle::new(attacker.index())),
        "the victim's EventGotHit handler must observe the attacker at the owner boundary"
    );

    let snapshot = (
        victim_entity.element_data().position_map(),
        victim_entity.element_data().sprite.current_frame,
        engine.orders.sequence_manager.sequences_iter().count(),
    );
    engine.tick_ability_for(&sim, &mut display, &assets, attacker);
    let victim_entity = engine.get_entity(victim).unwrap();
    assert_eq!(
        (
            victim_entity.element_data().position_map(),
            victim_entity.element_data().sprite.current_frame,
            engine.orders.sequence_manager.sequences_iter().count(),
        ),
        snapshot,
        "a later owner tick must not repeat failed setup effects"
    );
}

#[test]
#[should_panic(expected = "requires Interaction data")]
fn strangle_condolation_rejects_non_interaction_owner_data() {
    use crate::element::{Command, Posture};
    use crate::sequence::SequenceElement;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_pc(Posture::Upright));
    let seq = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::StrangleCmd, Some(owner)));
    engine.orders.sequence_manager.element_impossible(seq, 0);
    engine.dispatch_condolations_for_owner_boundary(&sim, owner, &LevelAssets::new());
}

#[test]
fn straight_strike_damage_interrupts_only_later_creation_slots() {
    // Straight strikes register their damage during the envelope pass and
    // the manager phase applies it afterwards, so both creation orders end
    // the frame the same way: the chained attacker dies to the interrupter
    // while its already-registered strike still reaches the final target.
    // The creation-slot distinction survives as the damage registration
    // order (checked inside the shared drain helper).
    for interrupter_first in [true, false] {
        assert!(
            chained_straight_strike_target_life(interrupter_first) < 50,
            "the chained attacker's registered strike must land at the manager phase \
             (interrupter_first={interrupter_first})"
        );
    }
}

#[test]
fn hourglass_nonstraight_damage_interrupts_only_later_creation_slots() {
    // Nonstraight strikes register their damage during the envelope pass and
    // the manager phase applies it afterwards, so both creation orders end
    // the frame the same way: the chained attacker dies to the interrupter
    // while its own already-registered strike still reaches the final
    // target. The creation-slot distinction survives as the damage
    // registration order (checked inside the shared drain helper).
    for (interrupt, label) in [
        (NonstraightInterrupt::Lateral, "lateral"),
        (NonstraightInterrupt::Push, "push"),
    ] {
        for interrupter_first in [true, false] {
            let (final_life, interrupted_life) =
                chained_nonstraight_strike_lives(interrupt, interrupter_first);
            assert!(
                interrupted_life <= 0,
                "the {label} must land on the chained victim at the manager phase \
                 (interrupter_first={interrupter_first})"
            );
            assert!(
                final_life < 50,
                "the chained attacker's registered strike must still hit past the {label} \
                 (interrupter_first={interrupter_first})"
            );
        }
    }
}

#[test]
fn lethal_swordfight_cleanup_only_unlinks_the_survivor() {
    let sim = crate::sim_rng::test_context();
    let assets = assets_with_test_pc_profile();
    let mut engine = EngineInner::new();
    let survivor = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let victim = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));

    {
        let survivor_entity = engine.get_entity_mut(survivor).unwrap();
        survivor_entity.actor_data_mut().unwrap().action_state =
            crate::element::ActionState::WaitingSword;
        survivor_entity.human_data_mut().unwrap().opponents = vec![victim].into();
        survivor_entity.pc_data_mut().unwrap().melee_target = Some(victim);
    }
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.actor_data_mut().unwrap().action_state =
            crate::element::ActionState::WaitingSword;
        victim_entity.human_data_mut().unwrap().opponents = vec![survivor].into();
        *victim_entity.human_and_life_points_mut().unwrap().1 = 0;
    }

    engine.quit_swordfight(&sim, &assets, victim);

    let survivor_entity = engine.get_entity(survivor).unwrap();
    assert!(survivor_entity.human_data().unwrap().opponents.is_empty());
    assert_eq!(
        survivor_entity.actor_data().unwrap().action_state,
        crate::element::ActionState::WaitingSword,
        "death cleanup must not lower the surviving opponent's sword"
    );
    assert_eq!(survivor_entity.pc_data().unwrap().melee_target, None);
    assert_eq!(
        engine.orders.sequence_manager.sequences_iter().count(),
        0,
        "relationship-only cleanup must not synthesize a QuitSwordfight command"
    );
}

#[test]
fn explicit_quit_dispatch_preserves_cross_postponed_sword_movement_action() {
    use crate::element::{ActionState, Command};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceElementData, SequenceState};

    let sim = crate::sim_rng::test_context();
    let assets = assets_with_test_pc_profile();
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::WaitingSword;

    let movement = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new_movement(
            1,
            Command::Move,
            Some(owner),
            OrderType::RunningWithSword,
        ));
    engine
        .orders
        .sequence_manager
        .get_element_mut(movement, 0)
        .unwrap()
        .state = SequenceState::Postponed;

    let quit = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(
            1,
            Command::QuitSwordfight,
            Some(owner),
        ));
    engine
        .orders
        .sequence_manager
        .get_element_mut(quit, 0)
        .unwrap()
        .cross_postponed = Some((movement, 0));

    engine.dispatch_quit_swordfight(&sim, &assets, owner, quit, 0);

    let movement = engine
        .orders
        .sequence_manager
        .get_element(movement, 0)
        .expect("cross-postponed movement must remain registered");
    let SequenceElementData::Movement { action, .. } = &movement.data else {
        panic!("cross-postponed movement changed data kind")
    };
    assert_eq!(
        *action,
        OrderType::RunningWithSword,
        "QUIT_SWORDFIGHT translation must not apply opponent evaluation's live-movement rewrite"
    );
}

#[test]
fn lethal_sword_damage_pins_forced_attentive_view_and_hands_corpse_to_wait() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceElementData, SequenceState};
    use crate::weapons::SwordStrike;

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let attacker = engine.add_test_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_test_entity(super::super::scenarios::make_test_ai_soldier(
        crate::element::Camp::Lacklandists,
    ));
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        *victim_entity.human_and_life_points_mut().unwrap().1 = 0;
        let enemy = victim_entity
            .enemy_ai_mut()
            .expect("test soldier has EnemyAi");
        enemy.forced_attentive = true;
        enemy.base.current_music_alert_status = crate::ai::AlertLevel::Red;
        enemy.base.view_alert_status = crate::ai::AlertLevel::Red;
    }

    let mut damage = SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data = SequenceElementData::new_sword_damage(attacker, SwordStrike::E, 0);
    let damage_sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(damage_sequence, 0);

    engine.handle_death_with_damage_element(&sim, &assets, victim, (damage_sequence, 0), None);

    let enemy = engine
        .get_entity(victim)
        .and_then(crate::element::Entity::enemy_ai)
        .expect("dead soldier retains EnemyAi");
    assert_eq!(
        enemy.base.current_music_alert_status,
        crate::ai::AlertLevel::Green,
        "death lowers the music alert to Green"
    );
    assert_eq!(
        enemy.base.view_alert_status,
        crate::ai::AlertLevel::Yellow,
        "Original pins a forced-attentive soldier's view alert at Yellow"
    );

    let damage = engine
        .orders
        .sequence_manager
        .get_element(damage_sequence, 0)
        .expect("lethal damage element remains inspectable");
    assert_eq!(
        damage
            .orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        vec![OrderType::DyingSword],
        "ReceiveSwordDamage owns only the one-shot death animation"
    );

    // Model DYING_SWORD's START side effect before its eventual
    // TERMINATED result advances the damage element.
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.set_posture(Posture::Dead);
        victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
    }
    engine.do_next_order(damage_sequence, 0);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(damage_sequence, 0)
            .unwrap()
            .state,
        SequenceState::Terminated
    );
    assert_eq!(engine.actor_command(victim), Command::Wait);

    engine.ensure_wait_element(victim);
    let wait_sequence = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .find_map(|sequence| {
            sequence
                .elements
                .first()
                .filter(|element| element.owner == Some(victim) && element.command == Command::Wait)
                .map(|_| sequence.id)
        })
        .expect("dead actor receives its ordinary Wait element");
    // Instruction handling stamps the owner's current posture / action state onto the
    // element before Translate; the dead-hold animation choice reads the
    // stamped action-state-after-transition, not the live actor field.
    engine.stamp_element_transition_state(victim, wait_sequence, 0);
    crate::engine::sequence_runtime::WaitCommandContext {
        entities: &mut engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
        next_order_id: &mut engine.orders.next_order_id,
        profiles: &assets.profile_manager,
    }
    .dispatch(victim, Command::Wait, wait_sequence, 0);

    let wait = engine
        .orders
        .sequence_manager
        .get_element(wait_sequence, 0)
        .unwrap();
    assert_eq!(wait.state, SequenceState::InProgress);
    assert_eq!(
        wait.current_order().map(|order| order.order_type),
        Some(OrderType::BeingDeadSword)
    );
    assert_eq!(engine.actor_command(victim), Command::Wait);
}
