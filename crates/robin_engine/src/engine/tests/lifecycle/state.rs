use super::*;

#[test]
fn scrolling_table_generation() {
    let bg = BackgroundTransform::default();
    assert_eq!(bg.x_scrolling_values[0], 0.0);
    // First non-zero entry should be DEFAULT_SCROLLING_START (6.0)
    assert_eq!(bg.x_scrolling_values[1], 6.0);
    // Values should be monotonically non-decreasing
    for i in 1..SCROLLING_TABLE_SIZE - 1 {
        assert!(bg.x_scrolling_values[i] <= bg.x_scrolling_values[i + 1]);
    }
    // Last values should be capped at or above DEFAULT_SCROLLING_LIMIT
    assert!(bg.x_scrolling_values[SCROLLING_TABLE_SIZE - 1] >= DEFAULT_SCROLLING_LIMIT);
}

#[test]
fn zoom_state_machine() {
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4096.0, 4096.0);
    engine.feedback.cutscene_camera.display.display_op = DisplayOpCode::NoBackgroundMove;

    assert!(engine.is_zoom_possible());
    assert!(engine.is_zoom_up_possible());
    assert!(engine.is_zoom_down_possible());
    assert!(!engine.is_zooming());

    // Trigger zoom up
    assert!(engine.change_state_with_camera_display(0, EngineStateRequest::ZoomingUp));
    assert!(engine.is_zooming());
    assert!(!engine.is_zoom_possible());
    assert_eq!(
        engine.feedback.cutscene_camera.display.display_op,
        DisplayOpCode::InitZoom
    );
}

#[test]
fn constructor_primed_throwables_receive_exactly_one_appended_live_slot_advance() {
    use crate::coordinates::WorldPoint3D;
    use crate::element::{Entity, Posture};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let actor = engine.add_test_entity(make_test_pc(Posture::Upright));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let start = WorldPoint3D::new(0.0, 0.0, 20.0);
    let end = WorldPoint3D::new(200.0, 0.0, 0.0);
    let mut spawned = Vec::new();
    let mut appended = false;

    engine.tick_actor_animation_action_change_slots_with_hooks(
        &sim,
        &assets,
        |engine, id| {
            if matches!(
                engine.get_entity(id),
                Some(Entity::Projectile(_) | Entity::Net(_))
            ) {
                engine.tick_projectile_or_net_hourglass(&sim, &assets, id);
            }
        },
        |engine, owner| {
            if owner != actor || appended {
                return;
            }
            appended = true;
            for entity in [
                crate::bow_shot::spawn_net(actor, start, end, 0, None),
                crate::bow_shot::spawn_wasp_nest(actor, start, end, 0, None),
                crate::bow_shot::spawn_apple(actor, start, end, None, None, 0, None),
                crate::bow_shot::spawn_stone(actor, start, end, None, None, 0, None),
            ] {
                let id = engine.add_test_entity(entity);
                let frame_count = match engine.get_entity(id).unwrap() {
                    Entity::Projectile(projectile) => projectile.projectile.frame_count,
                    Entity::Net(net) => net.projectile.frame_count,
                    _ => unreachable!(),
                };
                assert_eq!(
                    frame_count, 1,
                    "{id:?} must enter EntitySlots after its primer"
                );
                spawned.push(id);
            }
        },
        |_, _, _, _, _, _, _| {},
        |_, _, _| {},
    );

    assert_eq!(spawned.len(), 4);
    for id in spawned {
        let frame_count = match engine.get_entity(id).unwrap() {
            Entity::Projectile(projectile) => projectile.projectile.frame_count,
            Entity::Net(net) => net.projectile.frame_count,
            _ => unreachable!(),
        };
        assert_eq!(
            frame_count, 2,
            "{id:?} must receive one appended live-slot advance, neither zero nor two"
        );
    }
}

#[test]
fn apple_and_stone_impact_selects_burst_row_then_derived_tail_owns_removal() {
    use crate::element::{
        Animation, ElementData, ElementKind, ElementProjectile, ObjectData, ObjectType,
        TrajectoryPoint,
    };
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    for object_type in [ObjectType::Apple, ObjectType::Stone] {
        let mut engine = EngineInner::new();
        let mut element = {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ObjectProjectile;
            initial_element.active = true;
            initial_element
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[Animation::ObjectFlying as usize] = 0;
        conversion[Animation::ObjectBursting as usize] = 16;
        let row = |animation: Animation, frames: Vec<u32>, delays: Vec<u16>| SpriteScript {
            action_id: animation as u16,
            action_done: frames.len().saturating_sub(1) as u16,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            distances: vec![0; frames.len()],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; frames.len()],
            sound_ids: vec![0; frames.len()],
            frame_ids: frames,
            delays,
        };
        element.sprite.conversion = std::sync::Arc::new(conversion);
        let mut scripts = Vec::with_capacity(32);
        for _ in 0..16 {
            scripts.push(row(Animation::ObjectFlying, vec![10, 11], vec![10, 10]));
        }
        for _ in 0..16 {
            scripts.push(row(
                Animation::ObjectBursting,
                vec![20, 21, 22],
                vec![0, 0, 0],
            ));
        }
        element.sprite.scripts = std::sync::Arc::new(scripts);
        element.sprite.force_animation(Animation::ObjectFlying, 0);
        element.sprite.force_sprite(0, 1);
        // Thrown projectiles get their facing stamped once at spawn from the
        // throw velocity and keep it for the whole flight; a hand-built
        // fixture must stamp it too so the burst-row regression below can
        // prove the burst ignores a nonzero direction.
        element.set_direction_instantly(4);
        let projectile_id = engine.add_test_entity(Entity::Projectile(ElementProjectile {
            element,
            object: ObjectData {
                object_type,
                animation: Animation::ObjectFlying,
                ..Default::default()
            },
            projectile: crate::element::ProjectileData {
                flying: true,
                trajectory: vec![TrajectoryPoint {
                    position: crate::coordinates::WorldPoint3D::new(10.0, 0.0, 0.0),
                    time: 1,
                }],
                ..Default::default()
            },
        }));
        let assets = LevelAssets::new();
        let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
        let tick = |engine: &mut EngineInner| {
            capture_projectile_derived_tails(|| {
                engine.with_simulation_context(|engine, sim| {
                    engine.tick_actor_owner_envelopes(sim, &assets, &positions)
                })
            })
            .1
        };

        let mut impact_tails = Vec::new();
        for _ in 0..3 {
            impact_tails = tick(&mut engine);
            if matches!(
                engine.get_entity(projectile_id),
                Some(Entity::Projectile(projectile))
                    if projectile.object.animation == Animation::ObjectBursting
            ) {
                break;
            }
        }
        assert_eq!(impact_tails, vec![(projectile_id, object_type)]);
        let Entity::Projectile(projectile) = engine.get_entity(projectile_id).unwrap() else {
            unreachable!()
        };
        assert_eq!(projectile.object.animation, Animation::ObjectBursting);
        assert_ne!(
            projectile.element.direction(),
            0,
            "regression requires a nonzero impact direction"
        );
        assert_eq!(projectile.element.sprite.current_row, 16);
        assert_eq!(
            projectile.element.sprite.current_frame, 1,
            "the zero-delay first burst frame must advance in the impact derived tail"
        );

        for _ in 0..8 {
            if !engine.get_entity(projectile_id).unwrap().is_active() {
                break;
            }
            assert_eq!(tick(&mut engine), vec![(projectile_id, object_type)]);
        }
        assert!(!engine.get_entity(projectile_id).unwrap().is_active());
        assert_eq!(
            tick(&mut engine),
            vec![(projectile_id, object_type)],
            "inactive virtual call must still run the derived landed tail"
        );
        // Retirement deactivates in place: the element stays in the array as
        // an inactive tombstone so outstanding references and creation order
        // remain valid; physical removal is reserved for teardown/load.
        assert!(
            engine
                .get_entity(projectile_id)
                .is_some_and(|entity| !entity.is_active()),
            "retired projectile must remain an inactive tombstone slot"
        );
    }
}

#[test]
fn interrupt_corpse_exit_initialization_aligns_body_without_active_drop() {
    use crate::movement::AbilityKind;
    use crate::order::OrderType;

    let (mut engine, carrier, body, _) =
        corpse_exit_initialization_fixture(false, crate::element::Command::WhistleCmd);
    let body_position = engine
        .get_entity(body)
        .unwrap()
        .element_data()
        .position_map();
    assert_eq!(
        engine
            .get_entity(carrier)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .kind,
        None
    );

    engine.tick_actor_animation_action_change_slots(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
    );

    let body_entity = engine.get_entity(body).unwrap();
    assert_eq!(body_entity.element_data().direction(), 9);
    assert_eq!(body_entity.position_iface().get_direction_goal().as_u8(), 9);
    assert_eq!(body_entity.element_data().position_map(), body_position);
    assert_eq!(
        engine.get_entity(carrier).unwrap().sprite().last_action,
        OrderType::TransitionCarryingCorpseWaitingUpright,
        "the no-ability transition must enter the generic action-processing arm"
    );
    assert_ne!(
        engine
            .get_entity(carrier)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .kind,
        Some(AbilityKind::Drop)
    );

    engine
        .get_entity_mut(body)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(3);
    engine.tick_actor_animation_action_change_slots(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
    );
    let body_entity = engine.get_entity(body).unwrap();
    assert_eq!(body_entity.element_data().direction(), 3);
    assert_eq!(body_entity.position_iface().get_direction_goal().as_u8(), 3);
}

#[test]
fn instant_corpse_drop_leaves_the_goal_at_the_carrier_heading() {
    use crate::element::Posture;

    let mut engine = EngineInner::new();
    let body = engine.add_test_entity(make_test_soldier(Posture::Carried));
    let carrier = engine.add_test_entity(make_test_pc(Posture::CarryingCorpse));
    {
        let carrier_entity = engine.get_entity_mut(carrier).unwrap();
        carrier_entity.pc_data_mut().unwrap().carried = Some(body);
        carrier_entity
            .pc_data_mut()
            .unwrap()
            .set_live_carried_posture(Posture::Tied);
        carrier_entity
            .element_data_mut()
            .set_direction_instantly(11);
    }
    {
        let body_entity = engine.get_entity_mut(body).unwrap();
        body_entity.human_data_mut().unwrap().carrier = Some(carrier);
        body_entity.actor_data_mut().unwrap().execution_frozen = true;
        body_entity.element_data_mut().set_direction_instantly(7);
    }

    engine.force_drop_carried_corpse_instant(carrier);

    let body_entity = engine.get_entity(body).unwrap();
    assert_eq!(body_entity.posture(), Posture::Tied);
    assert_eq!(body_entity.human_data().unwrap().carrier, None);
    assert_eq!(
        body_entity.element_data().direction(),
        7,
        "instant direction assignment stamps carrier_direction + 12"
    );
    assert_eq!(
        body_entity.position_iface().get_direction_goal().as_u8(),
        11,
        "carrier removal moves the goal to the carrier's own heading"
    );
}

#[test]
fn interrupted_mid_grab_installs_wait_without_executing_the_dropped_body() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{CascadeFlags, SequenceElement};
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let mut engine = EngineInner::new();
    let body = engine.add_test_entity(make_test_soldier(Posture::Tied));
    let carrier = engine.add_test_entity(make_test_pc(Posture::Upright));
    let carrier_position = crate::coordinates::MapPoint::new(1005.0, 827.0);
    {
        let carrier_entity = engine.get_entity_mut(carrier).unwrap();
        carrier_entity.pc_data_mut().unwrap().carried = Some(body);
        carrier_entity
            .pc_data_mut()
            .unwrap()
            .set_live_carried_posture(Posture::Tied);
        carrier_entity
            .element_data_mut()
            .set_position_map(carrier_position);
        carrier_entity.element_data_mut().set_direction_instantly(3);
    }
    {
        let tied = OrderType::BeingTied;
        let script = SpriteScript {
            action_id: tied as u16,
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
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[tied as usize] = 0;
        let body_entity = engine.get_entity_mut(body).unwrap();
        body_entity.element_data_mut().sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );
        body_entity
            .element_data_mut()
            .set_position_map(crate::coordinates::MapPoint::new(1012.68, 821.24));
        body_entity.position_iface_mut().new_move();
        body_entity
            .element_data_mut()
            .set_position_map(crate::coordinates::MapPoint::new(1000.0, 820.0));
        body_entity.human_data_mut().unwrap().carrier = Some(carrier);
        body_entity.human_data_mut().unwrap().unconscious = true;
        body_entity.actor_data_mut().unwrap().execution_frozen = true;
    }

    let order_id = engine.orders.allocate_order_id();
    let mut take =
        SequenceElement::new_interaction(1, Command::TakeCorpse, Some(carrier), Some(body));
    take.orders.push_back(Order::new(
        OrderType::TransitionWaitingUprightCarryingCorpse,
        0.0,
        0.0,
        order_id,
    ));
    let take_sequence = engine.orders.sequence_manager.launch_element(take);
    engine
        .orders
        .sequence_manager
        .element_in_progress(take_sequence, 0);
    engine
        .orders
        .sequence_manager
        .element_interrupted(take_sequence, 0, CascadeFlags::NEXT_LEVEL);
    let sim = crate::sim_rng::test_context();
    engine.dispatch_condolations(&sim, &LevelAssets::new());

    let body_entity = engine.get_entity(body).unwrap();
    assert_eq!(body_entity.element_data().position_map(), carrier_position);
    assert_eq!(
        body_entity.position_iface().old_map_position(),
        crate::coordinates::MapPoint::new(1012.68, 821.24),
        "dropping a corpse must not execute the body's later movement update"
    );
    assert_ne!(body_entity.sprite().last_action, OrderType::BeingTied);
    assert_eq!(body_entity.sprite().current_frame, 0);
    assert_eq!(body_entity.sprite().frame_count, u16::MAX);
    let selected = engine
        .orders
        .sequence_manager
        .current_element_for_actor(body)
        .and_then(|(sequence, index)| engine.orders.sequence_manager.get_element(sequence, index))
        .expect("DropCorpse must synchronously instruct the body's Wait");
    assert_eq!(selected.command, Command::Wait);
    assert_eq!(
        selected.current_order().map(|order| order.order_type),
        Some(OrderType::BeingTied)
    );
}

#[test]
fn deferred_face_to_generates_live_exit_transition_and_keeps_resolved_direction() {
    use crate::coordinates::MapPoint;
    use crate::element::{ActionState, Posture};
    use crate::order::OrderType;
    use crate::sequence::SequenceState;

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_soldier(Posture::Upright));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::MovingFast;
    let retained_goal = MapPoint::new(321.0, 654.0);

    let sequence = engine.launch_turn_sequence_deferred_no_transitions(
        owner,
        crate::element::Command::TurnFast,
        Some(9),
        0.0,
        0.0,
        Some(retained_goal),
    );
    let deferred = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .unwrap();
    assert_eq!(deferred.state, SequenceState::Todo);
    assert_eq!(deferred.command, crate::element::Command::TurnFast);
    assert_eq!(
        deferred.priority,
        crate::sequence::SequencePriority::NotYetSet,
        "facing priority belongs to the later actor-instruction boundary"
    );
    assert_eq!(deferred.posture_after_transition, Posture::Undefined);
    assert!(
        deferred.orders.is_empty(),
        "deferred facing must remain untranslated until its ordered InstructOwner boundary"
    );

    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    let instructed = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .unwrap();
    assert_eq!(instructed.state, SequenceState::InProgress);
    assert_eq!(instructed.command, crate::element::Command::TurnFast);
    assert_eq!(
        instructed
            .orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        vec![
            OrderType::TransitionRunningUprightWaitingUpright,
            OrderType::Turning,
        ],
        "ordered instruction must sample the live running state and prepend its exit transition"
    );
    assert!(
        !instructed.orders.back().unwrap().compute_direction,
        "turning must retain the direction resolved from the facing request's Direction field"
    );
    let entity = engine.get_entity(owner).unwrap();
    assert_eq!(u8::from(entity.position_iface().get_direction_goal()), 9);
    assert_eq!(entity.position_iface().map_goal(), retained_goal);
}

#[test]
fn explicit_halt_then_goto_keeps_single_stop_transition() {
    use crate::coordinates::MapPoint;
    use crate::element::{ActionState, Command, Posture};
    use crate::movement::ActiveMovement;
    use crate::order::{AiOrderIntent, Order, OrderType};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};
    use std::num::NonZeroU32;

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let mut soldier = make_test_soldier(Posture::Upright);
    let Entity::Soldier(soldier_data) = &mut soldier else {
        unreachable!("make_test_soldier returned a non-soldier")
    };
    soldier_data.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    let owner = engine.add_test_entity(soldier);
    let old_goal = MapPoint::new(1004.836, 1774.2802);

    let mut movement =
        SequenceElement::new_movement(1, Command::MoveOk, Some(owner), OrderType::WalkingUpright);
    movement.priority = SequencePriority::Normal;
    movement.orders.push_back(Order::new(
        OrderType::WalkingUpright,
        old_goal.x,
        old_goal.y,
        NonZeroU32::new(779).unwrap(),
    ));
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        entity.actor_data_mut().unwrap().action_state = ActionState::Moving;
        entity.actor_data_mut().unwrap().active_movement =
            ActiveMovement::new(movement_sequence, 0);
        entity.position_iface_mut().set_map_goal(old_goal);
        let ai = entity.ai_controller_mut().unwrap();
        ai.outbox.actor.halt = true;
        ai.outbox
            .actor
            .orders
            .push(AiOrderIntent::new(OrderType::RunningUpright, 900.0, 1700.0));
    }

    engine.launch_pending_orders_for_npc(&sim, &assets, owner);

    let old = engine
        .orders
        .sequence_manager
        .get_element(movement_sequence, 0)
        .unwrap();
    assert_eq!(old.state, SequenceState::InProgress);
    assert_eq!(
        old.current_order().unwrap().order_type,
        OrderType::TransitionWalkingUprightWaitingUpright,
        "the explicit action stop rewrites one stop transition and movement must not halt it again"
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal(),
        old_goal
    );
    assert_eq!(
        engine.orders.pending_move_requests.len(),
        1,
        "movement remains queued behind the preserved stop transition"
    );
}

#[test]
fn execution_frozen_wait_retains_selected_identity_without_entering_execute_arm() {
    use crate::element::{Command, Posture};
    use crate::order::Order;
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_pc(Posture::Upright));
    let mut element = SequenceElement::new(1, Command::WaitTimer, Some(owner));
    let order = Order::test_new(OrderType::WaitingUpright, 0.0, 0.0);
    let order_id = order.order_id;
    element.orders.push_back(order);
    let sequence = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .execution_frozen = true;

    let (_, outcomes, result) = engine.tick_actor_animation_for(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        owner,
    );
    let result = result.expect("frozen wait still returns the base Execute identity");
    assert_eq!(result.entry_seq_id, sequence);
    assert_eq!(result.entry_elem_idx, 0);
    assert_eq!(result.order_type, OrderType::WaitingUpright);
    assert_eq!(result.motion, crate::sprite::MotionState::InProgress);
    assert!(outcomes.seq_advance.is_empty());
    assert_ne!(
        engine
            .get_entity(owner)
            .unwrap()
            .element_data()
            .sprite
            .last_processed_order_id,
        order_id.get(),
        "per-actor execution freeze returns before the selected sprite call"
    );
}

#[test]
fn active_ability_type_mismatch_is_not_selected_or_allowed_to_suppress_generic_execute() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_pc(Posture::Upright));
    let mut element = SequenceElement::new(1, Command::EatCmd, Some(owner));
    let order = Order::test_new(OrderType::WaitingUpright, 0.0, 0.0);
    let order_id = order.order_id;
    element.orders.push_back(order);
    let seq_id = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(seq_id, 0);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_ability = crate::movement::ActiveAbility {
        kind: Some(crate::movement::AbilityKind::Eat),
        sequence_id: Some(seq_id),
        element_index: 0,
        target: None,
        order_id: Some(order_id),
        done_effect_applied: false,
        strangle_initialized: false,
    };

    let mut observed = None;
    engine.tick_actor_animation_action_change_slots_with_hooks(
        &sim,
        &LevelAssets::new(),
        |_, _| {},
        |_, _| {},
        |_, selected_owner, _, _, _, ability, _| observed = Some((selected_owner, ability)),
        |_, _, _| {},
    );
    assert_eq!(observed, Some((owner, None)));
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .order_id,
        Some(order_id),
        "a stale type mismatch remains latent and does not execute"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
            .unwrap()
            .2
            .order_type,
        OrderType::WaitingUpright,
        "the generic selected order remains authoritative"
    );
}

#[test]
fn injury_postponement_rebuilds_eat_with_a_fresh_ability_identity() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::profiles::Action;
    use crate::sequence::{SequenceElement, SequenceState};

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::default();
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_pc(Posture::Upright));
    attach_test_campaign_identities(&mut engine);
    engine.mission_domain.campaign.characters[0]
        .status
        .set_ammo(Action::Eat, 1);
    engine.mission_domain.campaign.characters[0]
        .status
        .life_points = 70;
    engine
        .get_entity_mut(owner)
        .unwrap()
        .pc_data_mut()
        .unwrap()
        .life_points = 70;

    let eat = engine.launch_element_for_owner(
        &sim,
        &assets,
        SequenceElement::new(1, Command::EatCmd, Some(owner)),
    );
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);
    let first_order = engine
        .orders
        .sequence_manager
        .get_element(eat, 0)
        .unwrap()
        .current_order()
        .expect("initial Eat translation must install an order")
        .order_id;
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(eat, 0)
            .unwrap()
            .state,
        SequenceState::InProgress
    );

    let injury = engine.launch_element_for_owner(
        &sim,
        &assets,
        SequenceElement::new_damage(1, Command::ReceiveArrowDamage, Some(owner), None, 10, 0),
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(eat, 0)
            .unwrap()
            .state,
        SequenceState::Postponed
    );
    assert!(
        !engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .is_active(),
        "postponing the selected Eat deletes its order and must also delete Rust's order mirror"
    );

    engine.hourglass_phase_sequences(&sim, &mut display, &assets);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(injury, 0)
            .unwrap()
            .state,
        SequenceState::InProgress
    );
    engine.orders.sequence_manager.element_terminated(injury, 0);
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    let resumed = engine
        .orders
        .sequence_manager
        .get_element(eat, 0)
        .expect("postponed Eat must remain registered");
    assert_eq!(resumed.state, SequenceState::InProgress);
    let resumed_order = resumed
        .current_order()
        .expect("resumed Eat must be translated again");
    assert_eq!(resumed_order.order_type, OrderType::Eating);
    assert_ne!(resumed_order.order_id, first_order);
    let active = &engine
        .get_entity(owner)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_ability;
    assert_eq!(active.sequence_id, Some(eat));
    assert_eq!(active.element_index, 0);
    assert_eq!(active.order_id, Some(resumed_order.order_id));
}

#[test]
fn aborted_ability_cleanup_is_exact_and_allows_later_selection() {
    use crate::element::{Command, Posture};
    use crate::movement::{AbilityKind, ActiveAbility};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequenceId};

    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_pc(Posture::Upright));
    let original = ActiveAbility {
        kind: Some(AbilityKind::Listen),
        sequence_id: Some(SequenceId(41)),
        element_index: 2,
        target: None,
        order_id: Some(std::num::NonZeroU32::new(9).unwrap()),
        done_effect_applied: false,
        strangle_initialized: false,
    };
    let actor = engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap();
    actor.active_ability = original.clone();
    actor.listen_phase = crate::element::ListenPhase::ExitTransition;
    actor.listen_wait_time = 7;
    engine.cleanup_aborted_ability(
        owner,
        AbilityKind::Listen,
        SequenceId(99),
        2,
        original.order_id,
    );
    let retained = &engine
        .get_entity(owner)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_ability;
    assert_eq!(
        (
            retained.kind,
            retained.sequence_id,
            retained.element_index,
            retained.order_id
        ),
        (
            original.kind,
            original.sequence_id,
            original.element_index,
            original.order_id
        ),
        "stale abort must not clear another selected identity"
    );

    engine.cleanup_aborted_ability(
        owner,
        AbilityKind::Listen,
        SequenceId(41),
        2,
        original.order_id,
    );
    let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
    assert!(!actor.active_ability.is_active());
    assert_eq!(actor.listen_phase, crate::element::ListenPhase::Inactive);
    assert_eq!(actor.listen_wait_time, 0);

    // This is an owner-selection fixture, not an Eat validity fixture.
    let mut element = SequenceElement::new(1, Command::Generic, Some(owner));
    let order = Order::test_new(OrderType::Eating, 0.0, 0.0);
    let order_id = order.order_id;
    element.orders.push_back(order);
    let seq = engine.orders.sequence_manager.launch_element(element);
    engine.orders.sequence_manager.element_in_progress(seq, 0);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_ability = ActiveAbility {
        kind: Some(AbilityKind::Eat),
        sequence_id: Some(seq),
        element_index: 0,
        target: None,
        order_id: Some(order_id),
        done_effect_applied: false,
        strangle_initialized: false,
    };
    let mut selected = None;
    engine.tick_actor_animation_action_change_slots_with_hooks(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        |_, _| {},
        |_, _| {},
        |_, id, _, _, _, ability, _| selected = Some((id, ability)),
        |_, _, _| {},
    );
    assert_eq!(selected, Some((owner, Some((seq, 0, order_id)))));
}

#[test]
fn pay_facing_is_sampled_once_at_first_execute_not_translation() {
    use crate::campaign::CampaignValue;
    use crate::element::{Command, Entity, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceState};
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let pc = engine.add_test_entity(make_test_pc(Posture::Upright));
    let beggar = engine.add_test_entity(make_test_civilian(Posture::Upright));
    let Entity::Civilian(civilian) = engine.get_entity_mut(beggar).unwrap() else {
        unreachable!()
    };
    civilian.civilian.beggar_scroll_sets = Some(vec![vec![]]);
    engine
        .mission_domain
        .campaign
        .set_value(CampaignValue::Ransom, crate::engine::BEGGAR_SALARY);
    engine
        .get_entity_mut(pc)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(1);
    engine
        .get_entity_mut(beggar)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(6);

    let scripts = (0..16)
        .map(|_| SpriteScript {
            action_id: OrderType::Paying as u16,
            action_done: 1,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1, 2, 3],
            delays: vec![10, 10, 10],
            distances: vec![0, 0, 0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
            sound_ids: vec![0; 3],
        })
        .collect();
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[OrderType::Paying as usize] = 0;
    engine.get_entity_mut(pc).unwrap().element_data_mut().sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(scripts),
        std::sync::Arc::new(conversion),
    );
    engine
        .get_entity_mut(pc)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(1);

    let seq = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new_interaction(
            1,
            Command::Pay,
            Some(pc),
            Some(beggar),
        ));
    assert_eq!(
        crate::abilities::begin_pay(
            &mut engine.world.entities,
            &mut engine.orders.sequence_manager,
            pc,
            beggar,
            seq,
            0,
            &mut engine.orders.next_order_id,
        ),
        crate::abilities::BeginResult::Started
    );
    engine.orders.sequence_manager.element_in_progress(seq, 0);
    assert_eq!(
        engine
            .get_entity(pc)
            .unwrap()
            .position_iface()
            .get_direction_goal()
            .as_u8(),
        1,
        "translation must not expose PAYING's facing"
    );
    assert!(
        engine.feedback.sound_sim.pending_exclamations.is_empty(),
        "translation must not emit HERO_GIVE_MONEY"
    );

    let assets = assets_with_test_pc_profile();
    let mut invalid = engine.clone();
    invalid
        .mission_domain
        .campaign
        .set_value(CampaignValue::Ransom, 0);
    invalid
        .get_entity_mut(beggar)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(8);
    invalid
        .get_entity_mut(pc)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .execute_order_initialising = true;
    invalid.tick_ability_for(&sim, &mut CameraDisplayState::default(), &assets, pc);
    assert_eq!(
        invalid
            .get_entity(pc)
            .unwrap()
            .position_iface()
            .get_direction_goal()
            .as_u8(),
        1,
        "failed first-Execute validity must abort before facing"
    );
    assert!(
        invalid.feedback.sound_sim.pending_exclamations.is_empty(),
        "failed first-Execute validity must abort before speech"
    );

    // The target can turn after translation. Original samples its live
    // direction only when PAYING first enters Execute.
    engine
        .get_entity_mut(beggar)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(7);
    engine
        .get_entity_mut(pc)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .execute_order_initialising = true;
    engine.tick_ability_for(&sim, &mut CameraDisplayState::default(), &assets, pc);
    assert_eq!(
        engine
            .get_entity(pc)
            .unwrap()
            .position_iface()
            .get_direction_goal()
            .as_u8(),
        15
    );
    assert_eq!(
        engine
            .feedback
            .sound_sim
            .pending_exclamations
            .iter()
            .filter(|speech| speech.exclamation_id == crate::engine::melee::HERO_GIVE_MONEY)
            .count(),
        1,
        "first valid Execute must emit HERO_GIVE_MONEY exactly once"
    );

    // Capture the invalid-completion branch immediately after first Execute,
    // before another sprite tick can reach PAYING's DONE boundary.
    let mut invalid_completion = engine.clone();
    let mut valid_completion = engine.clone();
    for completion in [&mut invalid_completion, &mut valid_completion] {
        completion
            .get_entity_mut(pc)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .execute_order_initialising = false;
    }
    let invalid_completion_sim = crate::sim_rng::test_context();
    let valid_completion_sim = crate::sim_rng::test_context();
    let invalid_sequence_count = invalid_completion
        .orders
        .sequence_manager
        .sequences_iter()
        .count();
    assert_eq!(
        invalid_completion
            .orders
            .sequence_manager
            .get_element(seq, 0)
            .expect("Pay remains selected before completion invalidation")
            .state,
        SequenceState::InProgress
    );
    assert!(
        !invalid_completion
            .get_entity(pc)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .done_effect_applied
    );
    assert!(
        !invalid_completion
            .get_entity(pc)
            .unwrap()
            .actor_data()
            .unwrap()
            .execute_order_initialising
    );
    assert_eq!(
        invalid_completion
            .mission_domain
            .campaign
            .get_value(CampaignValue::Ransom),
        crate::engine::BEGGAR_SALARY
    );
    assert!(
        !invalid_completion
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(beggar, |command| {
                command == Command::ReceivePurse
            })
    );
    assert!(
        valid_completion.check_sequence_element_validity(
            &assets,
            pc,
            valid_completion
                .orders
                .sequence_manager
                .get_element(seq, 0)
                .expect("valid Pay element remains selected"),
            true,
        )
    );

    engine.control.chorus_timer = 0;
    let Entity::Pc(pc_entity) = engine.get_entity_mut(pc).unwrap() else {
        unreachable!()
    };
    pc_entity.pc.forbidden_expressions.clear();
    engine
        .get_entity_mut(pc)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .execute_order_initialising = false;
    engine
        .get_entity_mut(beggar)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(8);
    engine.tick_ability_for(&sim, &mut CameraDisplayState::default(), &assets, pc);
    assert_eq!(
        engine
            .get_entity(pc)
            .unwrap()
            .position_iface()
            .get_direction_goal()
            .as_u8(),
        15,
        "later Execute frames must not resample the antagonist"
    );
    assert_eq!(
        engine
            .feedback
            .sound_sim
            .pending_exclamations
            .iter()
            .filter(|speech| speech.exclamation_id == crate::engine::melee::HERO_GIVE_MONEY)
            .count(),
        1,
        "later Execute frames must not re-emit HERO_GIVE_MONEY"
    );

    // The original game validates payment again after action completion. A
    // campaign change during the animation aborts before salary deduction or
    // the beggar's ReceivePurse response.
    invalid_completion
        .mission_domain
        .campaign
        .set_value(CampaignValue::Ransom, 0);
    let ((), invalid_cards) = crate::engine::soldier_helpers::capture_condolation_cards(|| {
        for _ in 0..128 {
            invalid_completion.tick_ability_for(
                &invalid_completion_sim,
                &mut CameraDisplayState::default(),
                &assets,
                pc,
            );
            if invalid_completion
                .orders
                .sequence_manager
                .get_element(seq, 0)
                .is_some_and(|element| element.state != SequenceState::InProgress)
                && !invalid_completion
                    .get_entity(pc)
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
    assert!(
        invalid_cards.contains(&(pc, Command::Pay)),
        "completion abort must send the Pay owner's condolation card: {invalid_cards:?}"
    );
    assert_eq!(
        invalid_completion
            .orders
            .sequence_manager
            .get_element(seq, 0)
            .expect("invalid Pay element remains registered")
            .state,
        SequenceState::Impossible,
        "completion validity failure must abort the selected Pay element"
    );
    assert_eq!(
        invalid_completion
            .mission_domain
            .campaign
            .get_value(CampaignValue::Ransom),
        0,
        "completion failure must not deduct another salary"
    );
    assert!(
        !invalid_completion
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(beggar, |command| {
                command == Command::ReceivePurse
            }),
        "completion failure must not launch ReceivePurse"
    );
    assert_eq!(
        invalid_completion
            .orders
            .sequence_manager
            .sequences_iter()
            .count(),
        invalid_sequence_count,
        "completion failure must not launch any response sequence"
    );
    assert!(
        !invalid_completion
            .get_entity(pc)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .is_active()
    );
    // Valid completion still applies the salary exactly once and launches
    // the civilian response.
    for _ in 0..128 {
        valid_completion.tick_ability_for(
            &valid_completion_sim,
            &mut CameraDisplayState::default(),
            &assets,
            pc,
        );
        if valid_completion
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(beggar, |command| command == Command::ReceivePurse)
        {
            break;
        }
    }
    assert_eq!(
        valid_completion
            .mission_domain
            .campaign
            .get_value(CampaignValue::Ransom),
        0,
        "valid completion state={:?}, active_ability={:?}, motion={:?}, pc_pos={:?}, beggar_pos={:?}",
        valid_completion
            .orders
            .sequence_manager
            .get_element(seq, 0)
            .map(|element| element.state),
        valid_completion
            .get_entity(pc)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability,
        valid_completion
            .get_entity(pc)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        valid_completion
            .get_entity(pc)
            .unwrap()
            .element_data()
            .position_map(),
        valid_completion
            .get_entity(beggar)
            .unwrap()
            .element_data()
            .position_map(),
    );
    assert!(
        valid_completion
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(beggar, |command| {
                command == Command::ReceivePurse
            }),
        "valid Pay completion launches the beggar response"
    );
}

#[test]
fn production_selected_beggar_frozen_turns_and_bids_while_execution_frozen_and_fried_skip() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let beggar = engine.add_test_entity(make_test_pc(Posture::SimulatingBeggar));
    let donor = engine.add_test_entity(make_test_civilian(Posture::Upright));
    let donor_actor = engine
        .get_entity_mut(donor)
        .unwrap()
        .actor_data_mut()
        .unwrap();
    donor_actor.action_state = ActionState::Moving;
    let donor_data = engine
        .get_entity_mut(donor)
        .unwrap()
        .npc_data_mut()
        .unwrap();
    donor_data.money = 200;
    engine
        .get_entity_mut(donor)
        .unwrap()
        .position_iface_mut()
        .set_move_box(crate::coordinates::MoveBox::from_corners(
            crate::coordinates::MapVec::new(-5.0, -5.0),
            crate::coordinates::MapVec::new(5.0, 5.0),
        ));
    engine
        .get_entity_mut(beggar)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(1);
    let donor_direction = (0..16)
        .find(|direction| {
            engine
                .get_entity_mut(donor)
                .unwrap()
                .element_data_mut()
                .set_direction_instantly(*direction);
            crate::engine::beggar::can_give_money_to_beggar(&engine, donor, beggar)
        })
        .expect("test geometry has an eligible donor direction");
    engine
        .get_entity_mut(donor)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(donor_direction);
    engine
        .get_entity_mut(beggar)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(0);
    let mut element = SequenceElement::new(1, Command::EnterBeggar, Some(beggar));
    let order = Order::test_new(OrderType::SimulatingBeggar, 0.0, 0.0);
    element.orders.push_back(order);
    let seq = engine.orders.sequence_manager.launch_element(element);
    engine.orders.sequence_manager.element_in_progress(seq, 0);
    engine
        .get_entity_mut(beggar)
        .unwrap()
        .position_iface_mut()
        .set_direction(crate::position_interface::Direction::from_raw(1));
    engine.set_actors_frozen(true);
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    {
        use crate::sprite::Sprite;
        use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};
        use std::sync::Arc;
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[crate::element::Animation::ObjectFlying as usize] = 16;
        let script = SpriteScript {
            action_id: crate::element::Animation::ObjectFlying as u16,
            action_done: 4,
            frame_ids: vec![1, 2, 3, 4, 5],
            delays: vec![0; 5],
            distances: vec![0; 5],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 5],
            sound_ids: vec![0; 5],
            ..Default::default()
        };
        assets.accessory_sprite_prototypes.insert(
            crate::element::ObjectType::Coin,
            Sprite::new(Arc::new(vec![script; 17]), Arc::new(conversion)),
        );
    }
    let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);

    for (name, execution_frozen, fried) in
        [("execution_frozen", true, false), ("fried", false, true)]
    {
        let mut gated = engine.clone();
        let Entity::Pc(pc) = gated.get_entity_mut(beggar).unwrap() else {
            unreachable!()
        };
        pc.actor.execution_frozen = execution_frozen;
        pc.pc.fried_psykokwack = fried;
        gated.tick_actor_owner_envelopes_with_test_owner_hook(
            &crate::sim_rng::test_context(),
            &assets,
            &positions,
            |_, _| {},
        );
        assert!(
            !gated
                .world
                .entities
                .occupied()
                .any(|(_, entity)| entity.object_data().is_some_and(|o| o.belongs_to_beggar)),
            "{name} must suppress selected beggar dispatch"
        );
    }

    engine.tick_actor_owner_envelopes_with_test_owner_hook(
        &crate::sim_rng::test_context(),
        &assets,
        &positions,
        |_, _| {},
    );
    assert_eq!(
        engine
            .get_entity(beggar)
            .unwrap()
            .element_data()
            .direction(),
        1
    );
    let coin = engine
        .world
        .entities
        .occupied()
        .find_map(|(id, entity)| {
            entity
                .object_data()
                .is_some_and(|object| object.belongs_to_beggar)
                .then_some(id)
        })
        .expect("FrozenAll Turn is followed by Bid and a live appended coin");
    assert!(
        coin.index() > donor.index(),
        "coin must occupy a later live creation slot"
    );
    assert!(
        engine
            .get_entity(donor)
            .unwrap()
            .npc_data()
            .unwrap()
            .has_given_money_to_beggar
    );
}

#[test]
fn production_leave_listen_is_postponed_until_enter_chain_naturally_finishes() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceState};
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_pc(Posture::Upright));
    let order_types = [
        OrderType::TransitionWaitingUprightListening,
        OrderType::Listening,
        OrderType::TransitionListeningWaitingUpright,
    ];
    let scripts = order_types
        .iter()
        .map(|order_type| SpriteScript {
            action_id: *order_type as u16,
            action_done: 1,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1, 2, 3],
            delays: vec![0, 0, 0],
            distances: vec![0, 0, 0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
            sound_ids: vec![0; 3],
        })
        .collect::<Vec<_>>();
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    for (row, order_type) in order_types.into_iter().enumerate() {
        conversion[order_type as usize] = row as u16;
    }
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(scripts),
        std::sync::Arc::new(conversion),
    );
    let mut assets = LevelAssets::new();
    let mut profiles = crate::profiles::ProfileManager::new();
    profiles.characters.push(crate::profiles::CharacterProfile {
        actions: [
            crate::profiles::Action::Listen,
            crate::profiles::Action::NoAction,
            crate::profiles::Action::NoAction,
        ],
        ..Default::default()
    });
    assets.profile_manager = std::sync::Arc::new(profiles);
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.players.seats[0].selection.push(owner);
    engine.players.seats[0].selected_action = crate::profiles::Action::Listen;
    let enter_seq =
        engine.launch_element(SequenceElement::new(1, Command::EnterListen, Some(owner)));
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();

    for _ in 0..20 {
        engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
        if engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .listen_phase
            == crate::element::ListenPhase::CountingDown
        {
            break;
        }
    }
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .listen_phase,
        crate::element::ListenPhase::CountingDown
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(enter_seq, 0)
            .unwrap()
            .current_order()
            .unwrap()
            .order_type,
        OrderType::Listening
    );

    let listening_order_id = engine
        .orders
        .sequence_manager
        .get_element(enter_seq, 0)
        .unwrap()
        .current_order()
        .unwrap()
        .order_id;
    let leave_seq =
        engine.launch_element(SequenceElement::new(1, Command::LeaveListen, Some(owner)));
    // Sequence-element launch only registers the element on the manager's
    // to-go queue; the owner instruction boundary that arbitrates it against
    // the non-interruptable EnterListen runs at the next manager hourglass.
    engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
    let leave = engine
        .orders
        .sequence_manager
        .get_element(leave_seq, 0)
        .unwrap();
    assert_eq!(leave.state, SequenceState::Postponed);
    assert!(leave.orders.is_empty());
    let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
    assert_eq!(actor.active_ability.sequence_id, Some(enter_seq));
    assert_eq!(actor.active_ability.element_index, 0);
    assert_eq!(actor.active_ability.order_id, Some(listening_order_id));
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(enter_seq, 0)
            .unwrap()
            .state,
        SequenceState::InProgress,
        "LeaveListen must not replace the non-interruptable EnterListen owner"
    );

    let mut saw_enter_exit = false;
    for _ in 0..80 {
        engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
        saw_enter_exit |= engine
            .orders
            .sequence_manager
            .get_element(enter_seq, 0)
            .and_then(|element| element.current_order())
            .is_some_and(|order| order.order_type == OrderType::TransitionListeningWaitingUpright);
        if !engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .is_active()
        {
            break;
        }
    }
    assert!(
        saw_enter_exit,
        "EnterListen must own its existing exit order"
    );
    assert!(
        !engine
            .get_entity(owner)
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
            .get_element(enter_seq, 0)
            .unwrap()
            .state,
        SequenceState::Terminated
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .listen_phase,
        crate::element::ListenPhase::Inactive
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .action_state,
        crate::element::ActionState::Waiting
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .pc_data()
            .unwrap()
            .current_action,
        crate::profiles::Action::NoAction,
        "selected Listen DONE must synchronously apply MSG_UNSELECT_ACTION"
    );
    let listen_done_waits_before_reselection: Vec<_> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| {
            sequence
                .elements
                .iter()
                .map(move |element| (sequence.id, element.owner, element.state, element.command))
        })
        .filter(|(_, element_owner, _, command)| {
            *element_owner == Some(owner) && *command == Command::Wait
        })
        .filter(|(_, _, state, _)| {
            matches!(
                state,
                SequenceState::Todo | SequenceState::Postponed | SequenceState::InProgress
            )
        })
        .collect();
    assert_eq!(
        listen_done_waits_before_reselection.len(),
        1,
        "Listen DONE must synchronously publish exactly one priority-Wait successor"
    );
    assert!(
        matches!(
            listen_done_waits_before_reselection[0].2,
            SequenceState::Todo | SequenceState::Postponed | SequenceState::InProgress
        ),
        "Listen DONE Wait must remain live before terminal advance"
    );
    engine.select_pc(&assets, 0, owner, false, false);
    let listen_done_waits_after_reselection: Vec<_> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .filter(|element| element.owner == Some(owner) && element.command == Command::Wait)
        .filter(|element| {
            matches!(
                element.state,
                SequenceState::Todo | SequenceState::Postponed | SequenceState::InProgress
            )
        })
        .map(|element| element.state)
        .collect();
    assert_eq!(
        listen_done_waits_after_reselection,
        listen_done_waits_before_reselection
            .iter()
            .map(|(_, _, state, _)| *state)
            .collect::<Vec<_>>(),
        "same-frame SelectPC restitution of cleared NOACTION must not Stop the Listen DONE Wait"
    );
    // The enter chain's terminal condolence releases the postponed leave
    // back through the production to-go queue. Depending on where in the
    // frame the release lands, the same frame's manager drain may already
    // have consumed it (LeaveListen without an active listen resolves
    // Impossible), so accept either boundary here; the loop below settles
    // the terminal state either way.
    let leave_state = engine
        .orders
        .sequence_manager
        .get_element(leave_seq, 0)
        .unwrap()
        .state;
    assert!(
        engine
            .orders
            .sequence_manager
            .is_registered_to_go(leave_seq, 0)
            || leave_state == SequenceState::Impossible,
        "released LeaveListen must be re-dispatched through the production queue \
         (state: {leave_state:?})"
    );
    for _ in 0..20 {
        engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
        if engine
            .orders
            .sequence_manager
            .get_element(leave_seq, 0)
            .unwrap()
            .state
            == SequenceState::Impossible
        {
            break;
        }
    }
    let leave = engine
        .orders
        .sequence_manager
        .get_element(leave_seq, 0)
        .unwrap();
    assert_eq!(leave.state, SequenceState::Impossible);
    assert!(
        !engine
            .orders
            .sequence_manager
            .is_registered_to_go(leave_seq, 0),
        "released LeaveListen action must be consumed exactly once"
    );
    assert!(leave.orders.is_empty());
    assert!(
        !engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .is_active(),
        "released LeaveListen must not install stale ability ownership"
    );
}

#[test]
fn fade_to_black_presents_without_advancing_simulation_timers() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.control.frame_counter = 25;

    let mut campaign = Campaign::new();
    campaign.set_value(CampaignValue::MissionLength, 7);
    engine.mission_domain.campaign = campaign;

    engine
        .feedback
        .sound_sim
        .sources
        .sources_push_some(crate::sound_source::SoundSource {
            source_kind: crate::sound_source::SoundSourceKind::Delayed,
            timer: 9,
            active: true,
            ..Default::default()
        });

    engine.apply_host_commands(
        sim,
        &assets,
        vec![crate::natives::EngineCommand::FadeToBlack { speed: 3 }],
    );

    let fade = engine
        .feedback
        .pending_side_effects
        .fade_to_black
        .take()
        .flatten()
        .expect("fade command should emit a host ramp");
    assert_eq!(fade.frames_remaining, 6);
    assert_eq!(engine.fade_freeze_frames_remaining(), 5);

    // The trigger tick presents the first of six frames. Each of the five
    // subsequent hourglass calls represents one more presentation, but is
    // not a simulation tick in the original game.
    for expected_remaining in (0..5).rev() {
        let side_effects =
            engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
        assert_eq!(side_effects.code, GameCode::LevelInProgress);
        assert!(!side_effects.skip_render);
        assert_eq!(engine.fade_freeze_frames_remaining(), expected_remaining);
        assert_eq!(engine.control.frame_counter, 25);
        assert_eq!(
            engine
                .mission_domain
                .campaign
                .get_value(CampaignValue::MissionLength),
            7
        );
        assert_eq!(engine.feedback.sound_sim.sources.get(0).unwrap().timer, 9);
    }

    // The next call is the first real simulation tick after the blocking
    // fade and resumes every clock from exactly its pre-fade value.
    engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
    assert_eq!(engine.control.frame_counter, 26);
    assert_eq!(engine.feedback.sound_sim.sources.get(0).unwrap().timer, 8);
}

#[test]
fn fade_to_black_host_countdown_advances_once_per_presented_frame() {
    let mut fade = crate::engine::types::FadeToBlack {
        speed: 2,
        frames_remaining: 4,
    };

    for expected_remaining in [3, 2, 1] {
        assert!(fade.advance_presented_frame());
        assert_eq!(fade.frames_remaining, expected_remaining);
    }
    assert!(!fade.advance_presented_frame());
    assert_eq!(fade.frames_remaining, 0);
    assert!(!fade.advance_presented_frame());
}

#[test]
fn enter_helping_climb_on_inactive_pc_terminates_at_init_validity() {
    // A self-ability launched on an inactive (off-map roster) PC still
    // selects its element and reaches the Execute init-time validity
    // check, which terminates it before any animation plays.  The PC
    // stays Upright/Waiting with no selected element afterwards.
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let mut assets = LevelAssets::new();
    let mut profiles = crate::profiles::ProfileManager::new();
    profiles.characters.push(crate::profiles::CharacterProfile {
        actions: [
            crate::profiles::Action::HelpToClimb,
            crate::profiles::Action::NoAction,
            crate::profiles::Action::NoAction,
        ],
        ..Default::default()
    });
    assets.profile_manager = std::sync::Arc::new(profiles);
    let mut engine = EngineInner::new();

    let pc_id = engine.add_test_entity(crate::element::Entity::Pc(crate::element::ActorPc {
        element: {
            let mut initial_element =
                crate::element::ElementData::from_initial_posture(crate::element::Posture::Upright);
            initial_element.kind = crate::element::ElementKind::ActorPc;
            initial_element.active = false;
            initial_element
        },
        actor: crate::element::ActorData {
            action_state: crate::element::ActionState::Waiting,
            ..Default::default()
        },
        human: Default::default(),
        pc: crate::element::PcData {
            life_points: 50,
            // The helping-climb toolbar action is disabled, so the
            // init-time validity check fails and the element must
            // terminate even though the owner is inactive.
            disabled_actions: vec![true, false, false],
            ..Default::default()
        },
    }));

    let elem = crate::sequence::SequenceElement::new(
        1,
        crate::element::Command::EnterHelpingClimb,
        Some(pc_id),
    );
    engine.launch_element(elem);
    complete_test_runtime_fixture(&mut engine, &mut assets);

    // First hourglass selects the element and queues the transition
    // order; the init validity pre-pass must terminate it no later
    // than the next hourglass.
    for _ in 0..2 {
        let result = engine
            .perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev)
            .code;
        assert_eq!(result, GameCode::LevelInProgress);
    }

    let pc = engine.get_entity(pc_id).expect("pc still exists");
    assert_eq!(
        pc.element_data().posture(),
        crate::element::Posture::Upright,
        "terminated enter-helping-climb must not change posture"
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(pc_id)
            .is_none(),
        "inactive PC's invalid helping-climb element must be terminated at init"
    );
}

#[test]
fn fast_forward() {
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.camera_slide = crate::coordinates::MapPoint::new(100.0, 200.0);
    engine.set_fast_forward();
    assert!(engine.is_fast_forward());
    // Camera should have jumped to slide target
    assert_eq!(engine.feedback.cutscene_camera.view_position.x, 100.0);
    assert_eq!(engine.feedback.cutscene_camera.view_position.y, 200.0);
    // Slide should be deactivated
    assert!(!engine.feedback.cutscene_camera.is_sliding());
}

#[test]
fn rollback_clone_stays_in_sync() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let seed = 0xDEAD_BEEF_CAFE_BABE;

    let mut original = EngineInner::new();
    original.restore_rng_from_seed(seed);

    // Warm up a few ticks so the clone is taken from a non-initial state.
    for _ in 0..30 {
        original.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
    }

    // Snapshot now. This is the rollback-from point.
    let snapshot = original.clone();

    // Advance both copies by the same number of ticks.
    let mut replay = snapshot.clone();
    for _ in 0..50 {
        original.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
        replay.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
    }

    assert_eq!(original.control.frame_counter, replay.control.frame_counter);
    assert_eq!(original.rng_seed(), replay.rng_seed());
    assert_eq!(original.control.chorus_timer, replay.control.chorus_timer);
    assert_eq!(
        original.mission_domain.state.mission_won,
        replay.mission_domain.state.mission_won
    );
    assert_eq!(original.scripts.globals, replay.scripts.globals);

    // Double-check: re-cloning the original snapshot and replaying the
    // SAME number of ticks a second time must also match — guarding
    // against state that silently leaks across clones (e.g. an RNG allocation
    // accidentally shared between the snapshot and the live engine).
    let mut second_replay = snapshot;
    for _ in 0..50 {
        second_replay.perform_hourglass(
            &mut display,
            &mut InputState::default(),
            &assets,
            &mut dev,
        );
    }
    assert_eq!(
        second_replay.control.frame_counter,
        original.control.frame_counter
    );
    assert_eq!(second_replay.rng_seed(), original.rng_seed());
}

#[test]
fn post_initialize_waits_for_post_refresh_stage() {
    use crate::scb::{ClassEntry, Function, ScbFile};
    use crate::vm::{Opcode, Quad};

    let begin = Quad {
        operation: Opcode::BeginFunction as u8,
        operands: [0; 8],
    };
    let ret = Quad {
        operation: Opcode::Return as u8,
        operands: [0; 8],
    };
    let startup = ClassEntry {
        source_file: "post_initialize_ordering_test.scs".into(),
        class_name: "StartUp".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![Function {
            name: "PostInitialize".into(),
            address: 0,
            num_parameters: 0,
            size_of_return_value: 0,
            size_of_parameters: 0,
            size_of_volatile: 0,
            size_of_temporary: 0,
        }],
        quads: vec![begin, ret],
    };

    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = crate::campaign::Campaign::default();
    engine.scripts.mission = Some(
        MissionScript::from_scb(ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![startup],
        })
        .expect("synthetic StartUp script"),
    );
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();

    engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
    assert_eq!(
        engine.control.frame_counter, 1,
        "the first simulation frame ran"
    );
    assert!(
        !engine.script_domains.mission_ui.game_post_initialized,
        "PostInitialize must not run before the first host refresh and sound hourglass"
    );

    let rng_seed_before_post_initialize = engine.rng_seed();
    engine.control.sim_config.script_enabled = false;
    engine.control.arrow_refresh_pending = true;
    assert!(
        engine
            .perform_post_initialize(&mut display, &assets)
            .is_none()
    );
    assert!(!engine.script_domains.mission_ui.game_post_initialized);
    assert!(engine.control.arrow_refresh_pending);
    assert_eq!(engine.rng_seed(), rng_seed_before_post_initialize);
    engine.control.sim_config.script_enabled = true;
    let first_post_initialize_effects = engine.perform_post_initialize(&mut display, &assets);
    assert!(first_post_initialize_effects.is_some());
    assert!(!engine.control.arrow_refresh_pending);
    assert_eq!(
        engine.rng_seed(),
        rng_seed_before_post_initialize,
        "an empty PostInitialize must reclaim the unchanged simulation RNG stream"
    );
    assert_eq!(
        engine.control.frame_counter, 1,
        "the post-refresh stage must not advance simulation time"
    );
    assert!(
        engine.script_domains.mission_ui.game_post_initialized,
        "the post-refresh stage must dispatch PostInitialize exactly at the frame-one boundary"
    );

    let second_post_initialize_effects = engine.perform_post_initialize(&mut display, &assets);
    assert!(second_post_initialize_effects.is_none());
    assert_eq!(engine.control.frame_counter, 1);
    assert!(engine.script_domains.mission_ui.game_post_initialized);
}

#[test]
fn evaluate_opponents_maps_legacy_climb_like_original_release() {
    use crate::element::Command;
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceElementData, SequenceState};

    let sim = crate::sim_rng::test_context();
    let assets = assets_with_test_pc_profile();
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_pc(crate::element::Posture::OnWall));
    let mut legacy_movement =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::ClimbingWallUp);
    legacy_movement.state = SequenceState::InProgress;
    let movement = engine
        .orders
        .sequence_manager
        .launch_element(legacy_movement);

    engine.evaluate_opponents(&sim, &assets, owner);

    let movement = engine
        .orders
        .sequence_manager
        .get_element(movement, 0)
        .expect("legacy movement must remain registered for postponement");
    let SequenceElementData::Movement { action, .. } = &movement.data else {
        panic!("legacy movement changed data kind")
    };
    assert_eq!(
        *action,
        OrderType::RunningUpright,
        "every non-walking-sword action maps to running upright"
    );
}
