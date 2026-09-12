use super::*;
use crate::element::{
    ActorCivilian, ActorPc, ActorSoldier, ElementData, ElementFx, ElementKind, Entity, FxData,
};
use crate::engine::EngineInner;

#[test]
fn reversed_cape_transition_enters_cloaked_and_honors_switch_off_at_completion() {
    let mut pc = Entity::Pc(ActorPc {
        element: {
            let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
            initial_element.kind = ElementKind::ActorPc;
            initial_element
        },
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });
    let id = EntityId::Pc(crate::entity_id::PcId(7));
    let mut outcomes = ExecuteSideOutcomes::default();

    apply_pc_disguise_exit_side_effect(
        &mut pc,
        OrderType::TransitionWaitingCapeWaitingUpright,
        MotionState::Done,
        Some(Command::EnterCloak),
        true,
        id,
        &mut outcomes,
    );

    assert_eq!(pc.posture(), Posture::Cloaked);
    assert!(outcomes.hidden_titbit_removals.is_empty());

    apply_pc_disguise_exit_side_effect(
        &mut pc,
        OrderType::TransitionWaitingCapeWaitingUpright,
        MotionState::Done,
        Some(Command::LeaveSpy),
        true,
        id,
        &mut outcomes,
    );
    assert_eq!(pc.posture(), Posture::Upright);
    assert_eq!(outcomes.hidden_titbit_removals, vec![id]);

    outcomes.hidden_titbit_removals.clear();
    apply_pc_disguise_exit_side_effect(
        &mut pc,
        OrderType::TransitionWaitingCapeWaitingUpright,
        MotionState::Done,
        Some(Command::EnterCloak),
        false,
        id,
        &mut outcomes,
    );
    assert_eq!(pc.posture(), Posture::Upright);
    assert_eq!(outcomes.hidden_titbit_removals, vec![id]);
}

#[test]
fn pc_beggar_execute_turns_during_both_transitions_and_idle() {
    assert!(pc_beggar_execute_calls_turn(
        OrderType::TransitionWaitingUprightSimulatingBeggar
    ));
    assert!(pc_beggar_execute_calls_turn(
        OrderType::TransitionSimulatingBeggarWaitingUpright
    ));
    assert!(pc_beggar_execute_calls_turn(OrderType::SimulatingBeggar));
    assert!(!pc_beggar_execute_calls_turn(OrderType::Whistling));
}

#[test]
fn leaving_beggar_state_effect_does_not_overwrite_a_newer_pc_action() {
    let mut entity = Entity::Pc(ActorPc {
        element: {
            let mut initial_element =
                ElementData::from_initial_posture(crate::element::Posture::SimulatingBeggar);
            initial_element.kind = ElementKind::ActorPc;
            initial_element
        },
        actor: Default::default(),
        human: Default::default(),
        pc: crate::element::PcData {
            current_action: crate::profiles::Action::Net,
            ..Default::default()
        },
    });

    apply_active_animation_start_state_side_effect(
        &mut entity,
        OrderType::TransitionSimulatingBeggarWaitingUpright,
        MotionState::Done,
    );

    assert_eq!(
        entity.pc_data().unwrap().current_action,
        crate::profiles::Action::Net,
        "state assignment on the exit edge precedes a separately gated beggar-action deselection"
    );
    assert_eq!(
        entity.element_data().posture(),
        crate::element::Posture::Upright
    );
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        crate::element::ActionState::Waiting
    );
}

#[test]
fn attentive_turning_stamps_the_pre_turn_direction_row() {
    assert_eq!(
        actor_action_row(OrderType::Turning, OrderType::TurningAlerted, 13, 12),
        13
    );
    assert_eq!(
        actor_action_row(OrderType::Turning, OrderType::Turning, 13, 12),
        12,
        "the special ordering belongs only to the attentive soldier arm"
    );
}

fn weak_soldier_at_action_done(tiredness: u16) -> Entity {
    let mut entity = Entity::Soldier(ActorSoldier {
        element: ElementData::default(),
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    });
    let sprite = &mut entity.element_data_mut().sprite;
    sprite.current_frame = 4;
    sprite.frame_count = 2;
    sprite.action_done_frame = 4;
    sprite.action_done_counter = 2;
    entity.human_data_mut().unwrap().tiredness = tiredness;
    entity
}

#[test]
fn special_remark_uses_pre_perform_sprite_phase() {
    const HELBARDMAN: u32 = 0x4c484453;

    assert!(!special_remark_due_for_execute(7, (0, u16::MAX), (0, 0)));
    assert!(special_remark_due_for_execute(7, (0, 0), (0, 1)));
    assert!(!special_remark_due_for_execute(7, (0, 1), (0, 0)));

    assert!(!special_remark_due_for_execute(
        HELBARDMAN,
        (40, 0),
        (39, 0)
    ));
    assert!(special_remark_due_for_execute(HELBARDMAN, (39, 0), (40, 0)));
    assert!(!special_remark_due_for_execute(
        HELBARDMAN,
        (39, 0),
        (40, 1)
    ));
}

#[test]
fn raising_sword_preserves_soldier_map_vs_human_ground_facing() {
    let mut soldier = weak_soldier_at_action_done(0);
    let mut pc = Entity::Pc(ActorPc {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ActorPc;
            initial_element
        },
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });
    let mut opponent = weak_soldier_at_action_done(0);
    for owner in [&mut soldier, &mut pc] {
        owner
            .element_data_mut()
            .set_position(crate::coordinates::WorldPoint3D::ZERO);
        owner
            .element_data_mut()
            .set_position_map_preserving_3d(crate::coordinates::MapPoint::new(0.0, 0.0));
    }
    opponent
        .element_data_mut()
        .set_position(crate::coordinates::WorldPoint3D {
            x: 10.0,
            y: 0.0,
            z: 0.0,
        });
    opponent
        .element_data_mut()
        .set_position_map_preserving_3d(crate::coordinates::MapPoint::new(0.0, 10.0));

    assert_eq!(
        raising_sword_direction(&soldier, &opponent),
        crate::position_interface::vector_to_sector_0_to_15_iso(0.0, 10.0)
    );
    assert_eq!(
        raising_sword_direction(&pc, &opponent),
        crate::position_interface::vector_to_sector_0_to_15_iso(10.0, 0.0)
    );
}

#[test]
fn raising_sword_state_changes_follow_human_start_and_soldier_done() {
    let mut pc = Entity::Pc(ActorPc {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ActorPc;
            initial_element
        },
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });
    apply_active_animation_start_state_side_effect(
        &mut pc,
        OrderType::TransitionRaisingSword,
        MotionState::Start,
    );
    assert_eq!(
        pc.actor_data().unwrap().action_state,
        ActionState::WaitingSword
    );

    let mut soldier = weak_soldier_at_action_done(0);
    apply_soldier_execute_side_effects(
        &mut soldier,
        OrderType::TransitionRaisingSword,
        MotionState::Start,
        None,
        EntityId::Soldier(crate::entity_id::SoldierId(1)),
        &mut ExecuteSideOutcomes::default(),
    );
    assert_eq!(
        soldier.actor_data().unwrap().action_state,
        ActionState::Waiting
    );
    apply_soldier_execute_side_effects(
        &mut soldier,
        OrderType::TransitionRaisingSword,
        MotionState::Done,
        None,
        EntityId::Soldier(crate::entity_id::SoldierId(1)),
        &mut ExecuteSideOutcomes::default(),
    );
    assert_eq!(
        soldier.actor_data().unwrap().action_state,
        ActionState::WaitingSword
    );
}

#[test]
fn standing_up_sword_refreshes_new_principal_after_sprite_and_turns() {
    let mut pc = Entity::Pc(ActorPc {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ActorPc;
            initial_element
        },
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });
    pc.element_data_mut().set_direction_instantly(11);

    // No swordfight on the preceding tick: the stale goal is retained,
    // but the original game still initiates a turn.
    apply_standing_up_sword_post_perform_facing(
        &mut pc,
        None,
        EntityId::Pc(crate::entity_id::PcId(0)),
        0,
    );
    assert_eq!(pc.element_data().direction(), 11);
    assert_eq!(i16::from(pc.position_iface().get_direction_goal()), 11);

    // A reciprocal EnterSwordfight later in that update makes the
    // principal visible on the next tick. StandingUpSword must refresh
    // the goal then take exactly one slow-turn step toward it.
    apply_standing_up_sword_post_perform_facing(
        &mut pc,
        Some(3),
        EntityId::Pc(crate::entity_id::PcId(0)),
        0,
    );
    assert_eq!(i16::from(pc.position_iface().get_direction_goal()), 3);
    assert_eq!(pc.element_data().direction(), 10);

    // The soldier Execute override only replays the sprite.
    let mut soldier = weak_soldier_at_action_done(0);
    soldier.element_data_mut().kind = ElementKind::ActorSoldier;
    soldier.element_data_mut().set_direction_instantly(11);
    apply_standing_up_sword_post_perform_facing(
        &mut soldier,
        Some(3),
        EntityId::Soldier(crate::entity_id::SoldierId(0)),
        0,
    );
    assert_eq!(i16::from(soldier.position_iface().get_direction_goal()), 11);
    assert_eq!(soldier.element_data().direction(), 11);
}

#[test]
fn perform_flight_toggles_anti_collision_and_clears_deviation_before_stand_up_turn() {
    let mut pc = Entity::Pc(ActorPc {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ActorPc;
            initial_element
        },
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });
    let position = pc.position_iface_mut();
    position.set_direction_instantly(crate::position_interface::Direction::from_raw(1));
    position.set_direction(crate::position_interface::Direction::from_raw(0));
    let mut serialized = position.v48_serialized_state();
    serialized.anti_collision_on = true;
    serialized.deviated = true;
    serialized.direction_count = 0;
    position.restore_v48_serialized_state(serialized);

    apply_falling_start_side_effect(
        &mut pc,
        OrderType::FallingPushedWithSword,
        MotionState::Start,
    );
    assert!(!pc.position_iface().is_anti_collision_on());
    assert!(!pc.position_iface().is_deviated());

    apply_falling_completion_side_effect(
        &mut pc,
        OrderType::FallingPushedWithSword,
        MotionState::Terminated,
    );
    assert!(pc.position_iface().is_anti_collision_on());

    apply_standing_up_sword_post_perform_facing(
        &mut pc,
        Some(0),
        EntityId::Pc(crate::entity_id::PcId(0)),
        0,
    );
    assert_eq!(pc.element_data().direction(), 0);
}

#[test]
fn perform_flight_order_set_excludes_action_and_ladder_falls() {
    for anim in [
        OrderType::FallingHitUpright,
        OrderType::FallingHitWithBow,
        OrderType::FallingHitWithSword,
        OrderType::FallingHitCrouched,
        OrderType::FallingPushedUpright,
        OrderType::FallingPushedWithBow,
        OrderType::FallingPushedWithSword,
        OrderType::FallingPushedCrouched,
    ] {
        assert!(uses_perform_flight(anim), "{anim:?}");
    }
    for anim in [
        OrderType::FallingHitHarderUpright,
        OrderType::FallingHitHarderWithBow,
        OrderType::FallingHitHarderWithSword,
        OrderType::FallingHitHarderCrouched,
        OrderType::FallingLadderWall,
        OrderType::Rolling,
    ] {
        assert!(!uses_perform_flight(anim), "{anim:?}");
    }
}

#[test]
fn dead_actor_executes_its_selected_ordinary_animation() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::Order;
    use crate::sequence::SequenceElement;
    use crate::sprite::MotionState;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let mut engine = EngineInner::new();
    let actor = engine.add_test_entity(weak_soldier_at_action_done(0));
    {
        let entity = engine.get_entity_mut(actor).expect("dead actor exists");
        entity.element_data_mut().kind = ElementKind::ActorSoldier;
        entity
            .npc_data_mut()
            .expect("soldier has NPC data")
            .life_points = 0;
        entity
            .element_data_mut()
            .publish_order_posture(Posture::DeadBack);
        entity
            .actor_data_mut()
            .expect("soldier has actor data")
            .action_state = ActionState::Waiting;

        let action = OrderType::WaitingUprightBored;
        let script = SpriteScript {
            action_id: action as u16,
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
        conversion[action as usize] = 0;
        entity.element_data_mut().sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script]),
            std::sync::Arc::new(conversion),
        );
    }

    let mut selected = SequenceElement::new(1, Command::Wait, Some(actor));
    selected
        .orders
        .push_back(Order::test_new(OrderType::WaitingUprightBored, 0.0, 0.0));
    let sequence = engine.orders.sequence_manager.launch_element(selected);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    let (_, outcomes, result) = engine.tick_actor_animation_for(
        &crate::sim_rng::test_context(),
        &crate::engine::types::LevelAssets::new(),
        actor,
    );

    assert_eq!(result.map(|result| result.motion), Some(MotionState::Start));
    assert_eq!(
        outcomes.execute_sides.rejected_dead_idle_posture_requests,
        vec![actor],
        "the production Execute START boundary must preserve the rejected Upright request"
    );
    let entity = engine.get_entity(actor).expect("dead actor remains live");
    assert_eq!(
        entity.element_data().sprite.last_action,
        OrderType::WaitingUprightBored
    );
    assert_eq!(
        entity
            .actor_data()
            .expect("soldier has actor data")
            .action_state,
        ActionState::Bored
    );
}

#[test]
fn standing_up_sword_turns_toward_existing_goal_outside_swordfight() {
    let mut pc = Entity::Pc(ActorPc {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ActorPc;
            initial_element
        },
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });
    pc.element_data_mut().set_direction_instantly(11);
    pc.element_data_mut().set_direction_goal(3);

    apply_standing_up_sword_post_perform_facing(
        &mut pc,
        None,
        EntityId::Pc(crate::entity_id::PcId(0)),
        0,
    );

    assert_eq!(i16::from(pc.position_iface().get_direction_goal()), 3);
    assert_eq!(pc.element_data().direction(), 10);
}

#[test]
fn lowering_sword_start_restores_upright_waiting_state() {
    let mut pc = Entity::Pc(ActorPc {
        element: {
            let mut initial_element = ElementData::from_initial_posture(Posture::Crouched);
            initial_element.kind = ElementKind::ActorPc;
            initial_element
        },
        actor: crate::element::ActorData {
            action_state: ActionState::WaitingSword,
            ..Default::default()
        },
        human: Default::default(),
        pc: Default::default(),
    });

    apply_active_animation_start_state_side_effect(
        &mut pc,
        OrderType::TransitionLoweringSword,
        MotionState::Start,
    );

    assert_eq!(pc.element_data().posture(), Posture::Upright);
    assert_eq!(pc.actor_data().unwrap().action_state, ActionState::Waiting);
}

#[test]
fn helping_climb_done_applies_posture_and_toolbar_action() {
    let mut pc = Entity::Pc(ActorPc {
        element: {
            let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
            initial_element.kind = ElementKind::ActorPc;
            initial_element
        },
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });

    apply_active_animation_start_state_side_effect(
        &mut pc,
        OrderType::TransitionWaitingUprightHelpingClimbing,
        MotionState::Done,
    );

    assert_eq!(pc.element_data().posture(), Posture::HelpingToClimb);
    assert_eq!(pc.actor_data().unwrap().action_state, ActionState::Waiting);
    assert_eq!(
        pc.pc_data().unwrap().current_action,
        crate::profiles::Action::HelpToClimb
    );
}

#[test]
fn generic_crouch_transitions_apply_state_at_done_and_terminated() {
    for motion in [MotionState::Done, MotionState::Terminated] {
        let mut pc = Entity::Pc(ActorPc {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Crouched);
                initial_element.kind = ElementKind::ActorPc;
                initial_element
            },
            actor: crate::element::ActorData {
                action_state: ActionState::Waiting,
                ..Default::default()
            },
            human: Default::default(),
            pc: Default::default(),
        });

        apply_active_animation_start_state_side_effect(
            &mut pc,
            OrderType::TransitionCrouchingUp,
            motion,
        );
        assert_eq!(pc.element_data().posture(), Posture::Upright);
        assert_eq!(pc.actor_data().unwrap().action_state, ActionState::Waiting);

        apply_active_animation_start_state_side_effect(
            &mut pc,
            OrderType::TransitionCrouchingDown,
            motion,
        );
        assert_eq!(pc.element_data().posture(), Posture::Crouched);
        assert_eq!(pc.actor_data().unwrap().action_state, ActionState::Waiting);
    }
}

fn civilian_actor() -> Entity {
    Entity::Civilian(ActorCivilian {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ActorCivilian;
            initial_element
        },
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        civilian: Default::default(),
    })
}

fn animated_fx(patch_index: Option<crate::patch::PatchIndex>) -> Entity {
    let script = crate::sprite_script::SpriteScript {
        action_id: 0,
        action_done: 2,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2, 3],
        delays: vec![0, 0, 0],
        distances: vec![0, 0, 0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0, 0, 0],
    };
    let mut sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script]),
        std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]),
    );
    sprite.current_row = 0;
    sprite.current_frame = 0;
    sprite.frame_count = 0;
    Entity::Fx(ElementFx {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::Fx;
            initial_element.active = true;
            initial_element.sprite = sprite;
            initial_element
        },
        fx: FxData {
            patch_index,
            ..FxData::default()
        },
    })
}

#[test]
fn weak_sword_holds_action_done_while_tiredness_remains() {
    let mut entity = weak_soldier_at_action_done(10);

    let motion = hold_weak_sword_at_action_done(&mut entity, OrderType::BeingWeakSword);

    assert_eq!(motion, Some(MotionState::InProgress));
    assert_eq!(entity.human_data().unwrap().tiredness, 5);
    let sprite = &entity.element_data().sprite;
    assert_eq!(sprite.current_frame, 4);
    assert_eq!(sprite.frame_count, 2);
}

#[test]
fn weak_sword_resumes_when_tiredness_reaches_zero() {
    let mut entity = weak_soldier_at_action_done(5);

    let motion = hold_weak_sword_at_action_done(&mut entity, OrderType::BeingWeakSword);

    assert_eq!(motion, None);
    assert_eq!(entity.human_data().unwrap().tiredness, 0);
}

#[test]
fn weak_sword_first_arrival_at_action_done_preserves_done() {
    use crate::order::Order;
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let sim = crate::sim_rng::test_context();
    let assets = crate::engine::types::LevelAssets::new();
    let mut engine = EngineInner::new();
    let mut entity = weak_soldier_at_action_done(100);
    entity.element_data_mut().kind = ElementKind::ActorSoldier;
    let action = OrderType::BeingWeakSword;
    let script = SpriteScript {
        action_id: action as u16,
        action_done: 1,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2, 3],
        delays: vec![1; 3],
        distances: vec![0; 3],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0; 3],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[action as usize] = 0;
    entity.element_data_mut().sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script]),
        std::sync::Arc::new(conversion),
    );
    let actor = engine.add_test_entity(entity);
    let mut selected = SequenceElement::new(1, Command::Wait, Some(actor));
    selected.orders.push_back(Order::test_new(action, 0.0, 0.0));
    let sequence = engine.orders.sequence_manager.launch_element(selected);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    let (_, _, start) = engine.tick_actor_animation_for(&sim, &assets, actor);
    assert_eq!(start.expect("weak-sword START").motion, MotionState::Start);
    for _ in 0..10 {
        let before = engine
            .get_entity(actor)
            .unwrap()
            .human_data()
            .unwrap()
            .tiredness;
        let (_, _, result) = engine.tick_actor_animation_for(&sim, &assets, actor);
        let entity = engine.get_entity(actor).unwrap();
        assert_eq!(
            entity.human_data().unwrap().tiredness,
            before - WEAKNESS_DISMISH
        );
        if !sprite_is_at_action_done(entity.sprite()) {
            continue;
        }
        assert_eq!(
            result.expect("first action-done tick").motion,
            MotionState::Done
        );
        let frame = (entity.sprite().current_frame, entity.sprite().frame_count);
        let (_, _, held) = engine.tick_actor_animation_for(&sim, &assets, actor);
        assert_eq!(
            held.expect("following held tick").motion,
            MotionState::InProgress
        );
        let entity = engine.get_entity(actor).unwrap();
        assert_eq!(
            (entity.sprite().current_frame, entity.sprite().frame_count),
            frame
        );
        assert_eq!(
            entity.human_data().unwrap().tiredness,
            before - 2 * WEAKNESS_DISMISH
        );
        return;
    }
    panic!("weak-sword fixture never reached its action-done frame");
}

#[test]
fn falling_landing_depends_on_death_not_unconsciousness() {
    for dead in [false, true] {
        for unconscious in [false, true] {
            let mut entity = weak_soldier_at_action_done(0);
            entity.npc_data_mut().unwrap().life_points = if dead { 0 } else { 100 };
            entity.human_data_mut().unwrap().unconscious = unconscious;
            entity.set_posture(Posture::Flying);
            apply_falling_completion_side_effect(
                &mut entity,
                OrderType::FallingHitWithSword,
                MotionState::Terminated,
            );
            assert_eq!(
                entity.element_data().posture(),
                if dead {
                    Posture::DeadBack
                } else {
                    Posture::Lying
                }
            );
            assert_eq!(
                entity.actor_data().unwrap().action_state,
                ActionState::WaitingSword
            );
            assert_eq!(entity.human_data().unwrap().unconscious, unconscious);
        }
    }
}

#[test]
fn combat_injury_event_waits_for_terminated() {
    let entity = weak_soldier_at_action_done(0);
    let mut terminated = Vec::new();

    apply_combat_injury_side_effect(
        &entity,
        OrderType::BeingHitSword,
        MotionState::Done,
        EntityId::Pc(crate::entity_id::PcId(7)),
        &mut terminated,
    );
    assert!(terminated.is_empty());

    apply_combat_injury_side_effect(
        &entity,
        OrderType::BeingHitSword,
        MotionState::Terminated,
        EntityId::Pc(crate::entity_id::PcId(7)),
        &mut terminated,
    );
    assert_eq!(terminated, vec![EntityId::Pc(crate::entity_id::PcId(7))]);

    terminated.clear();
    apply_combat_injury_side_effect(
        &entity,
        OrderType::StandingUpSword,
        MotionState::Terminated,
        EntityId::Pc(crate::entity_id::PcId(7)),
        &mut terminated,
    );
    assert_eq!(terminated, vec![EntityId::Pc(crate::entity_id::PcId(7))]);
}

#[test]
fn global_actor_freeze_also_stops_nonactor_animation() {
    let mut engine = EngineInner::new();
    let fx = engine.add_test_entity(animated_fx(None));
    let assets = crate::engine::types::LevelAssets::new();

    engine.set_actors_frozen(true);
    engine.tick_static_entity_hourglass_for(&crate::sim_rng::test_context(), &assets, fx);
    assert_eq!(
        engine
            .world
            .entities
            .get(fx)
            .expect("frozen FX remains installed")
            .element_data()
            .sprite
            .current_frame,
        0
    );

    engine.set_actors_frozen(false);
    engine.tick_static_entity_hourglass_for(&crate::sim_rng::test_context(), &assets, fx);
    assert_eq!(
        engine
            .world
            .entities
            .get(fx)
            .expect("unfrozen FX remains installed")
            .element_data()
            .sprite
            .current_frame,
        1
    );
}

#[test]
fn patch_fx_without_mission_vm_uses_default_progression_without_finalization() {
    let mut engine = EngineInner::new();
    assert!(engine.scripts.mission.is_none());
    let fx = engine.add_test_entity(animated_fx(Some(
        crate::patch::PatchIndex::new(0).expect("zero is a valid patch index"),
    )));

    engine.tick_static_entity_hourglass_for(
        &crate::sim_rng::test_context(),
        &crate::engine::types::LevelAssets::new(),
        fx,
    );

    assert_eq!(
        engine
            .world
            .entities
            .get(fx)
            .expect("no-script patch FX remains installed")
            .element_data()
            .sprite
            .current_frame,
        1,
        "no-script patch FX retains the legacy default frame progression"
    );
    assert!(
        engine.script_domains.interactables.patches.is_empty(),
        "the no-VM compatibility path must not invent or finalize a patch"
    );
}

#[test]
fn sword_combat_injury_callback_retains_pre_perform_action_state() {
    let mut entity = weak_soldier_at_action_done(0);
    entity.actor_data_mut().unwrap().action_state = ActionState::Moving;

    let callback_action =
        weak_stunned_start_action_before_perform(&entity, OrderType::BeingStunnedSword, true);

    // The sword-injury START states are shared human behavior, so they
    // live in the universal active-animation dispatcher rather than the
    // soldier-only override.
    apply_active_animation_start_state_side_effect(
        &mut entity,
        OrderType::BeingStunnedSword,
        MotionState::Start,
    );

    assert_eq!(callback_action, Some(ActionState::Moving));
    assert_eq!(entity.element_data().posture(), Posture::Upright);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::WaitingSword
    );
}

#[test]
fn arrow_extraction_start_restores_original_posture_and_action_states() {
    let cases = [
        (
            OrderType::ExtractingArrowUpright,
            Posture::Upright,
            ActionState::Waiting,
        ),
        (
            OrderType::ExtractingArrowCrouched,
            Posture::Crouched,
            ActionState::Waiting,
        ),
        (
            OrderType::ExtractingArrowBow,
            Posture::Upright,
            ActionState::AimingWithBow,
        ),
    ];

    for (anim_type, posture, action_state) in cases {
        let mut entity = weak_soldier_at_action_done(0);
        entity.set_posture(Posture::Lying);
        entity.actor_data_mut().unwrap().action_state = ActionState::MovingFast;

        apply_arrow_extraction_start_side_effect(&mut entity, anim_type, MotionState::Start);

        assert_eq!(entity.element_data().posture(), posture, "{anim_type:?}");
        assert_eq!(
            entity.actor_data().unwrap().action_state,
            action_state,
            "{anim_type:?}"
        );
    }
}

#[test]
fn arrow_extraction_start_is_universal_for_civilians() {
    let mut entity = civilian_actor();
    entity.set_posture(Posture::Lying);
    entity.actor_data_mut().unwrap().action_state = ActionState::Moving;

    apply_arrow_extraction_start_side_effect(
        &mut entity,
        OrderType::ExtractingArrowCrouched,
        MotionState::Start,
    );

    assert_eq!(entity.element_data().posture(), Posture::Crouched);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::Waiting
    );
}

#[test]
fn bow_equip_start_enters_aiming_state() {
    let mut entity = Entity::Pc(ActorPc {
        element: {
            let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
            initial_element.kind = ElementKind::ActorPc;
            initial_element
        },
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });

    apply_active_animation_start_state_side_effect(
        &mut entity,
        OrderType::TransitionEquipBow,
        MotionState::Start,
    );

    assert_eq!(entity.element_data().posture(), Posture::Upright);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::AimingWithBow
    );
    assert!(forwards_pc_bow_action_on_start(
        &entity,
        OrderType::TransitionEquipBow,
        MotionState::Start,
        false,
    ));
    assert!(
        !forwards_pc_bow_action_on_start(
            &entity,
            OrderType::TransitionEquipBow,
            MotionState::Start,
            true,
        ),
        "script-driven bow equips must not select the player action"
    );
}

#[test]
fn bored_exit_completion_changes_pc_to_waiting() {
    let mut entity = Entity::Pc(ActorPc {
        element: {
            let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
            initial_element.kind = ElementKind::ActorPc;
            initial_element
        },
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });
    entity.actor_data_mut().unwrap().action_state = ActionState::Bored;

    apply_active_animation_start_state_side_effect(
        &mut entity,
        OrderType::TransitionWaitingUprightBoredWaitingUpright,
        MotionState::Done,
    );

    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::Waiting
    );
}

#[test]
fn nonmovement_upright_exit_completion_changes_pc_to_waiting() {
    for (animation, motion, expected_posture) in [
        (
            OrderType::TransitionWalkingUprightWaitingUpright,
            MotionState::Done,
            Posture::Upright,
        ),
        (
            OrderType::TransitionRunningUprightWaitingUpright,
            MotionState::Terminated,
            Posture::Upright,
        ),
        (
            OrderType::TransitionWalkingCrouchedWaitingCrouched,
            MotionState::Done,
            Posture::Crouched,
        ),
    ] {
        let mut entity = Entity::Pc(ActorPc {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
                initial_element.kind = ElementKind::ActorPc;
                initial_element
            },
            actor: Default::default(),
            human: Default::default(),
            pc: Default::default(),
        });
        entity.actor_data_mut().unwrap().action_state = ActionState::Moving;

        apply_active_animation_start_state_side_effect(&mut entity, animation, motion);

        assert_eq!(entity.element_data().posture(), expected_posture);
        assert_eq!(
            entity.actor_data().unwrap().action_state,
            ActionState::Waiting,
            "{animation:?} must apply the actor's universal completion state"
        );
    }
}

#[test]
fn civilian_idle_override_does_not_run_base_actor_state_changes() {
    let mut entity = civilian_actor();
    entity.actor_data_mut().unwrap().action_state = ActionState::Waiting;

    apply_active_animation_start_state_side_effect(
        &mut entity,
        OrderType::TransitionWaitingUprightWaitingUprightBored,
        MotionState::Done,
    );

    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::Waiting,
        "civilian execution returns directly after its coerced sprite call"
    );
}

#[test]
fn arrow_extraction_side_effect_only_runs_on_start() {
    let mut entity = weak_soldier_at_action_done(0);
    entity.set_posture(Posture::Lying);
    entity.actor_data_mut().unwrap().action_state = ActionState::Moving;

    apply_arrow_extraction_start_side_effect(
        &mut entity,
        OrderType::ExtractingArrowBow,
        MotionState::Terminated,
    );

    assert_eq!(entity.element_data().posture(), Posture::Lying);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::Moving
    );
}

#[test]
fn standing_up_start_sets_upright_waiting() {
    let mut entity = weak_soldier_at_action_done(0);
    entity.set_posture(Posture::Lying);
    entity.actor_data_mut().unwrap().action_state = ActionState::Moving;

    apply_standing_up_start_side_effect(&mut entity, OrderType::StandingUp, MotionState::Start);

    assert_eq!(entity.element_data().posture(), Posture::Upright);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::Waiting
    );
}

#[test]
fn standing_up_sword_start_sets_upright_waiting_sword() {
    let mut entity = weak_soldier_at_action_done(0);
    entity.set_posture(Posture::Lying);
    entity.actor_data_mut().unwrap().action_state = ActionState::ParryingSword;

    apply_standing_up_start_side_effect(
        &mut entity,
        OrderType::StandingUpSword,
        MotionState::Start,
    );

    assert_eq!(entity.element_data().posture(), Posture::Upright);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::WaitingSword
    );
}

#[test]
fn standing_up_bow_start_preserves_bow_action() {
    let mut entity = weak_soldier_at_action_done(0);
    entity.set_posture(Posture::Lying);
    entity.actor_data_mut().unwrap().action_state = ActionState::AimingWithBowUp;

    apply_standing_up_start_side_effect(&mut entity, OrderType::StandingUpBow, MotionState::Start);

    assert_eq!(entity.element_data().posture(), Posture::Upright);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::AimingWithBowUp
    );
}

#[test]
fn carried_body_start_sets_carried_waiting_for_human_actors() {
    for anim in [
        OrderType::BeingCarriedLittleJohn,
        OrderType::BeingCarriedPeasantC,
    ] {
        let mut entity = civilian_actor();
        entity.set_posture(Posture::Upright);
        entity.actor_data_mut().unwrap().action_state = ActionState::Moving;

        apply_carried_start_side_effect(&mut entity, anim, MotionState::Start);

        assert_eq!(entity.element_data().posture(), Posture::Carried);
        assert_eq!(
            entity.actor_data().unwrap().action_state,
            ActionState::Waiting
        );
    }
}

#[test]
fn active_provoking_start_sets_upright_waiting_sword() {
    let mut entity = weak_soldier_at_action_done(0);
    entity.set_posture(Posture::Crouched);
    entity.actor_data_mut().unwrap().action_state = ActionState::Moving;

    apply_active_animation_start_state_side_effect(
        &mut entity,
        OrderType::Provoking,
        MotionState::Start,
    );

    assert_eq!(entity.element_data().posture(), Posture::Upright);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::WaitingSword
    );
}

#[test]
fn active_waiting_shield_start_sets_upright_holding_shield() {
    let mut entity = weak_soldier_at_action_done(0);
    entity.set_posture(Posture::Crouched);
    entity.actor_data_mut().unwrap().action_state = ActionState::Moving;

    apply_active_animation_start_state_side_effect(
        &mut entity,
        OrderType::WaitingShield,
        MotionState::Start,
    );

    assert_eq!(entity.element_data().posture(), Posture::Upright);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::HoldingShield
    );
}

#[test]
fn active_taking_net_start_sets_upright_waiting_for_pc() {
    let mut entity = Entity::Pc(ActorPc {
        element: ElementData::default(),
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });
    entity.set_posture(Posture::Crouched);
    entity.actor_data_mut().unwrap().action_state = ActionState::MovingFast;

    apply_active_animation_start_state_side_effect(
        &mut entity,
        OrderType::TakingNet,
        MotionState::Start,
    );

    assert_eq!(entity.element_data().posture(), Posture::Upright);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::Waiting
    );
}

#[test]
fn taking_net_uses_generic_taking_row_when_profile_lacks_dedicated_animation() {
    let sprite = crate::sprite::Sprite::default();
    assert_eq!(
        sprite_anim_for_order(&sprite, OrderType::TakingNet, true),
        OrderType::Taking
    );
}

#[test]
fn active_start_state_side_effect_ignores_non_start_motion() {
    let mut entity = weak_soldier_at_action_done(0);
    entity.set_posture(Posture::Crouched);
    entity.actor_data_mut().unwrap().action_state = ActionState::Moving;

    apply_active_animation_start_state_side_effect(
        &mut entity,
        OrderType::WaitingShield,
        MotionState::Done,
    );

    assert_eq!(entity.element_data().posture(), Posture::Crouched);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::Moving
    );
}

#[test]
fn falling_hit_sword_start_and_termination_restore_original_states() {
    let mut entity = weak_soldier_at_action_done(0);
    entity.set_posture(Posture::Upright);
    entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;

    apply_falling_start_side_effect(
        &mut entity,
        OrderType::FallingHitWithSword,
        MotionState::Start,
    );

    assert_eq!(entity.element_data().posture(), Posture::Flying);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::Moving
    );

    apply_falling_completion_side_effect(
        &mut entity,
        OrderType::FallingHitWithSword,
        MotionState::Terminated,
    );

    assert_eq!(entity.element_data().posture(), Posture::Lying);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::WaitingSword
    );
}

#[test]
fn harder_falling_hit_preserves_pose_until_action_lands() {
    let mut entity = weak_soldier_at_action_done(0);
    entity.set_posture(Posture::Upright);
    entity.actor_data_mut().unwrap().action_state = ActionState::Menacing;

    apply_falling_start_side_effect(
        &mut entity,
        OrderType::FallingHitHarderWithSword,
        MotionState::Start,
    );

    assert_eq!(entity.element_data().posture(), Posture::Upright);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::Menacing
    );

    apply_falling_completion_side_effect(
        &mut entity,
        OrderType::FallingHitHarderWithSword,
        MotionState::Done,
    );

    assert_eq!(entity.element_data().posture(), Posture::Lying);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::Menacing,
        "hard-hit DONE lands but does not restore the wrapper's action family"
    );

    apply_falling_completion_side_effect(
        &mut entity,
        OrderType::FallingHitHarderWithSword,
        MotionState::Terminated,
    );

    assert_eq!(entity.element_data().posture(), Posture::Lying);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::WaitingSword
    );
}

#[test]
fn harder_falling_hits_retain_injury_priority() {
    // Human falling-hit execution sets
    // non-interruptible priority only in the softer arm. A harder
    // hit must therefore remain interruptible by a later injury, as in
    // Save024 replay023 where an incoming sword strike replaces the
    // still-playing harder hit and consumes its protection draws.
    for ordinary in [
        OrderType::FallingHitUpright,
        OrderType::FallingHitWithBow,
        OrderType::FallingHitWithSword,
        OrderType::FallingHitCrouched,
    ] {
        assert!(anim_forces_non_interruptable_on_start(ordinary));
    }
    for harder in [
        OrderType::FallingHitHarderUpright,
        OrderType::FallingHitHarderWithBow,
        OrderType::FallingHitHarderWithSword,
        OrderType::FallingHitHarderCrouched,
    ] {
        assert!(!anim_forces_non_interruptable_on_start(harder));
    }
}

#[test]
fn falling_pushed_bow_start_and_termination_restore_original_states() {
    let mut entity = weak_soldier_at_action_done(0);
    entity.set_posture(Posture::Upright);
    entity.actor_data_mut().unwrap().action_state = ActionState::AimingWithBow;

    apply_falling_start_side_effect(
        &mut entity,
        OrderType::FallingPushedWithBow,
        MotionState::Start,
    );

    assert_eq!(entity.element_data().posture(), Posture::Flying);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::WaitingSword
    );

    apply_falling_completion_side_effect(
        &mut entity,
        OrderType::FallingPushedWithBow,
        MotionState::Terminated,
    );

    assert_eq!(entity.element_data().posture(), Posture::Lying);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::AimingWithBow
    );
}

#[test]
fn smalltalk_start_sets_waiting_sword_and_termination_recovers_tiredness() {
    let mut entity = weak_soldier_at_action_done(0);
    entity.actor_data_mut().unwrap().action_state = ActionState::Moving;
    entity.human_data_mut().unwrap().tiredness = 27;
    let mut profiles = crate::profiles::ProfileManager::default();
    profiles.soldiers.push(crate::profiles::SoldierProfile {
        endurance: 80,
        ..Default::default()
    });

    apply_smalltalk_start_and_recovery_side_effect(
        &mut entity,
        OrderType::ParryingLeftSmalltalk,
        MotionState::Start,
        &profiles,
        None,
    );

    assert_eq!(entity.element_data().posture(), Posture::Upright);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::WaitingSword
    );

    apply_smalltalk_start_and_recovery_side_effect(
        &mut entity,
        OrderType::ParryingLeftSmalltalk,
        MotionState::Terminated,
        &profiles,
        None,
    );

    assert_eq!(entity.human_data().unwrap().tiredness, 19);
}

#[test]
fn striking_down_sword_start_and_done_match_original_side_effects() {
    let mut entity = weak_soldier_at_action_done(0);
    entity.actor_data_mut().unwrap().action_state = ActionState::Moving;
    entity.element_data_mut().set_direction_instantly(14);
    let mut outcomes = ExecuteSideOutcomes::default();

    apply_striking_down_sword_side_effect(
        &mut entity,
        OrderType::StrikingDownSword,
        MotionState::Start,
        Some(EntityId::Pc(crate::entity_id::PcId(9))),
        Some(15),
        EntityId::Pc(crate::entity_id::PcId(7)),
        &mut outcomes,
    );

    assert_eq!(entity.element_data().posture(), Posture::Upright);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::WaitingSword
    );
    assert_eq!(entity.element_data().direction(), 14);
    assert_eq!(entity.position_iface().get_direction_goal().as_u8(), 15);
    assert!(outcomes.killed_at_bottom.is_empty());

    apply_striking_down_sword_side_effect(
        &mut entity,
        OrderType::StrikingDownSword,
        MotionState::Done,
        Some(EntityId::Pc(crate::entity_id::PcId(9))),
        Some(15),
        EntityId::Pc(crate::entity_id::PcId(7)),
        &mut outcomes,
    );

    assert_eq!(
        outcomes.killed_at_bottom,
        vec![(
            EntityId::Pc(crate::entity_id::PcId(9)),
            EntityId::Pc(crate::entity_id::PcId(7))
        )]
    );
}

fn striking_down_execute_fixture() -> (
    EngineInner,
    crate::engine::types::LevelAssets,
    EntityId,
    EntityId,
    crate::sequence::SequenceId,
) {
    use crate::order::Order;
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let mut engine = EngineInner::new();
    let mut pc = Entity::Pc(ActorPc {
        element: {
            let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
            initial_element.kind = ElementKind::ActorPc;
            initial_element
        },
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });
    pc.pc_data_mut().expect("PC data").life_points = 100;
    pc.actor_data_mut().expect("actor data").action_state = ActionState::WaitingSword;

    let action = OrderType::StrikingDownSword;
    let script = SpriteScript {
        action_id: action as u16,
        action_done: 1,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2, 3],
        delays: vec![0, 0, 1],
        distances: vec![0; 3],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0; 3],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[action as usize] = 0;
    pc.element_data_mut().sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script]),
        std::sync::Arc::new(conversion),
    );

    let owner = engine.add_test_entity(pc);
    let mut soldier = weak_soldier_at_action_done(0);
    soldier.element_data_mut().kind = ElementKind::ActorSoldier;
    soldier
        .element_data_mut()
        .publish_order_posture(Posture::Lying);
    soldier.human_data_mut().expect("human data").unconscious = true;
    soldier.npc_data_mut().expect("NPC data").life_points = 100;
    let victim = engine.add_test_entity(soldier);

    let mut selected =
        SequenceElement::new_interaction(1, Command::SwordstrikeDown, Some(owner), Some(victim));
    let order = Order::test_new(action, 0.0, 0.0).with_antagonist(victim);
    selected.orders.push_back(order);
    let sequence = engine.orders.sequence_manager.launch_element(selected);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    let order_id = engine
        .orders
        .sequence_manager
        .current_order_for_actor(owner)
        .expect("selected strike order")
        .2
        .order_id;

    let sprite = &mut engine
        .get_entity_mut(owner)
        .expect("owner")
        .element_data_mut()
        .sprite;
    sprite.last_processed_order_id = order_id.get();
    sprite.last_action = action;
    sprite.current_row = 0;
    sprite.current_frame = 0;
    sprite.frame_count = 0;
    sprite.action_done_frame = 1;
    sprite.action_done_counter = 0;

    assert!(
        super::super::sequence_validity::striking_down_sword_valid_without_position(
            engine.get_entity(owner).expect("owner"),
            engine.get_entity(victim).expect("victim"),
            false,
        ),
        "fixture must begin with a valid unconscious live soldier target"
    );

    (
        engine,
        crate::engine::types::LevelAssets::new(),
        owner,
        victim,
        sequence,
    )
}

#[test]
fn striking_down_force_initialization_preserves_walk_caches_for_start_tick() {
    let sim = crate::sim_rng::test_context();
    let (mut engine, assets, owner, _victim, _sequence) = striking_down_execute_fixture();
    let order_id = engine
        .orders
        .sequence_manager
        .current_order_for_actor(owner)
        .expect("selected strike order")
        .2
        .order_id;

    let sprite = &mut engine
        .get_entity_mut(owner)
        .expect("owner")
        .element_data_mut()
        .sprite;
    sprite.last_processed_order_id = 0;
    sprite.last_action = OrderType::WalkingUpright;
    sprite.current_frame = 2;
    sprite.frame_count = 7;
    sprite
        .position_iface
        .set_map_position(crate::coordinates::MapPoint::new(0.0, 0.0));
    sprite
        .position_iface
        .set_map_goal(crate::coordinates::MapPoint::new(10.0, -1.0));
    sprite.position_iface.compute_increment_all(false);
    sprite.position_iface.update_forecasted_movement(3.0, 1);
    let walk_forecast = sprite.position_iface.get_forecasted_movement();
    assert_ne!(walk_forecast, crate::coordinates::WorldVec3D::ZERO);

    let (_, _, first_result) = engine.tick_actor_animation_for(&sim, &assets, owner);
    assert_eq!(
        first_result.map(|result| result.motion),
        Some(MotionState::Start)
    );
    let sprite = engine.get_entity(owner).expect("owner").sprite();
    assert_eq!(sprite.last_processed_order_id, order_id.get());
    assert_eq!(sprite.current_frame, 0);
    assert_eq!(sprite.frame_count, u16::MAX);
    assert_eq!(sprite.last_action, OrderType::WalkingUpright);
    assert_eq!(
        sprite.position_iface.get_forecasted_movement(),
        walk_forecast,
        "forced strike initialization preserves Original's prior walk forecast"
    );

    let _ = engine.tick_actor_animation_for(&sim, &assets, owner);
    let sprite = engine.get_entity(owner).expect("owner").sprite();
    assert_eq!(sprite.last_action, OrderType::StrikingDownSword);
    assert_eq!(
        sprite.position_iface.get_forecasted_movement(),
        crate::coordinates::WorldVec3D::ZERO,
        "ordinary second-tick frame initialization clears the stale forecast"
    );
}

#[test]
fn striking_down_revalidates_after_done_before_repeating_kill() {
    let sim = crate::sim_rng::test_context();
    let (mut engine, assets, owner, victim, sequence) = striking_down_execute_fixture();

    let (_, first_outcomes, first_result) = engine.tick_actor_animation_for(&sim, &assets, owner);
    assert_eq!(
        first_result.map(|result| result.motion),
        Some(MotionState::Done)
    );
    assert_eq!(
        first_outcomes.execute_sides.killed_at_bottom,
        vec![(victim, owner)],
        "the valid action point must still launch exactly one bottom kill"
    );

    engine
        .get_entity_mut(victim)
        .expect("victim")
        .npc_data_mut()
        .expect("NPC data")
        .life_points = 0;
    let (_, second_outcomes, second_result) = engine.tick_actor_animation_for(&sim, &assets, owner);
    assert_eq!(
        second_result.map(|result| result.motion),
        Some(MotionState::Terminated),
        "the post-action validity check must retire the stale strike"
    );
    assert!(
        second_outcomes.execute_sides.killed_at_bottom.is_empty(),
        "an invalid selected strike must return before DONE side effects"
    );

    let mut completion = AnimCompletionOutcomes::default();
    completion.seq_advance.push((sequence, 0));
    engine.process_anim_completion_outcomes(&sim, completion, &assets);
    assert!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(owner)
            .is_none(),
        "the exhausted strike leaves the Original actor order null for the rest of its slot"
    );
    engine.ensure_wait_element(owner);
    engine
        .drain_script_synchronous_actions(&sim, &assets, &mut Vec::new())
        .expect("fallback Wait launch");
    let (next_sequence, next_index) = engine
        .orders
        .sequence_manager
        .current_element_for_actor(owner)
        .expect("terminal strike must expose the fallback Wait");
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(next_sequence, next_index)
            .expect("fallback Wait element")
            .command,
        Command::Wait,
        "the terminal result must promote the fallback Wait owner"
    );
}

#[test]
fn striking_down_keeps_running_while_unconscious_victim_is_alive() {
    let sim = crate::sim_rng::test_context();
    let (mut engine, assets, owner, _victim, _sequence) = striking_down_execute_fixture();

    let _ = engine.tick_actor_animation_for(&sim, &assets, owner);
    let (_, outcomes, result) = engine.tick_actor_animation_for(&sim, &assets, owner);

    assert_eq!(
        result.map(|result| result.motion),
        Some(MotionState::InProgress)
    );
    assert!(outcomes.execute_sides.killed_at_bottom.is_empty());
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
            .map(|(_, _, order)| order.order_type),
        Some(OrderType::StrikingDownSword)
    );
}

#[test]
fn striking_down_sword_start_facing_uses_stretched_ground_positions() {
    let mut owner = weak_soldier_at_action_done(0);
    let mut antagonist = weak_soldier_at_action_done(0);
    owner
        .element_data_mut()
        .set_position(crate::coordinates::WorldPoint3D::ZERO);
    owner
        .element_data_mut()
        .set_position_map_preserving_3d(crate::coordinates::MapPoint::new(100.0, 100.0));
    antagonist
        .element_data_mut()
        .set_position(crate::coordinates::WorldPoint3D {
            x: -17.0,
            y: -16.0,
            z: 20.0,
        });
    antagonist
        .element_data_mut()
        .set_position_map_preserving_3d(crate::coordinates::MapPoint::new(-100.0, -100.0));

    assert_eq!(
        striking_down_sword_direction(&owner, &antagonist),
        crate::position_interface::vector_to_sector_0_to_15_iso(-17.0, -16.0)
    );
    assert_ne!(
        striking_down_sword_direction(&owner, &antagonist),
        crate::position_interface::vector_to_sector_0_to_15(-17.0, -16.0),
        "strike START must not use bare map-space binning"
    );
}

#[test]
fn unconscious_hold_terminates_after_wakeup() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut sequence_manager = crate::sequence::SequenceManager::new();
    let mut next_order_id = 1;
    let mut side_outcomes = ExecuteSideOutcomes::default();
    let mut ctx = ArmCtx {
        entity_id: EntityId::Pc(crate::entity_id::PcId(7)),
        is_npc: true,
        is_unconscious: false,
        seq_id: crate::sequence::SequenceId(1),
        elem_idx: 0,
        sequence_manager: &mut sequence_manager,
        next_order_id: &mut next_order_id,
        side_outcomes: &mut side_outcomes,
    };

    let outcome = dispatch_arm_completion(
        sim,
        OrderType::BeingUnconsciousSword,
        MotionState::InProgress,
        &mut ctx,
    );

    assert!(matches!(
        outcome,
        ExecuteOutcome::Forward(MotionState::Terminated)
    ));
}

#[test]
fn unconscious_hold_consumes_while_unconscious() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut sequence_manager = crate::sequence::SequenceManager::new();
    let mut next_order_id = 1;
    let mut side_outcomes = ExecuteSideOutcomes::default();
    let mut ctx = ArmCtx {
        entity_id: EntityId::Pc(crate::entity_id::PcId(7)),
        is_npc: true,
        is_unconscious: true,
        seq_id: crate::sequence::SequenceId(1),
        elem_idx: 0,
        sequence_manager: &mut sequence_manager,
        next_order_id: &mut next_order_id,
        side_outcomes: &mut side_outcomes,
    };

    let outcome = dispatch_arm_completion(
        sim,
        OrderType::BeingUnconsciousSword,
        MotionState::Terminated,
        &mut ctx,
    );

    assert!(matches!(outcome, ExecuteOutcome::Consumed));
}

#[test]
fn bored_cycle_keeps_order_id_when_random_variant_is_not_selected() {
    let seed = (0..1000)
        .find(|seed| {
            crate::sim_rng::with_seed(*seed, |sim| {
                crate::sim_rng::u32(sim, crate::sim_rng::RngSite::BoredAnimationChoice, ..10) != 0
            })
        })
        .expect("test should find a nonzero bored-animation roll");
    let (mut sequence_manager, seq_id) = sequence_with_order(OrderType::WaitingUprightBored);
    let original_id = sequence_manager
        .get_element(seq_id, 0)
        .unwrap()
        .orders
        .front()
        .unwrap()
        .order_id;
    let mut next_order_id = 9;
    let mut side_outcomes = ExecuteSideOutcomes::default();
    let mut ctx = ArmCtx {
        entity_id: EntityId::Pc(crate::entity_id::PcId(7)),
        is_npc: false,
        is_unconscious: false,
        seq_id,
        elem_idx: 0,
        sequence_manager: &mut sequence_manager,
        next_order_id: &mut next_order_id,
        side_outcomes: &mut side_outcomes,
    };

    let outcome = crate::sim_rng::with_seed(seed, |sim| {
        dispatch_arm_completion(
            sim,
            OrderType::WaitingUprightBored,
            MotionState::Terminated,
            &mut ctx,
        )
    });

    let order = sequence_manager
        .get_element(seq_id, 0)
        .unwrap()
        .orders
        .front()
        .unwrap();
    assert!(matches!(outcome, ExecuteOutcome::Consumed));
    assert_eq!(order.order_type, OrderType::WaitingUprightBored);
    assert_eq!(
        order.order_id, original_id,
        "the original game generates a new id only inside the 1-in-10 mutation branch"
    );
}

#[test]
fn ordinary_bored_cycle_forwards_only_its_start_edge() {
    let sim = crate::sim_rng::test_context();
    let (mut sequence_manager, seq_id) = sequence_with_order(OrderType::WaitingUprightBored);
    let mut next_order_id = 9;
    let mut side_outcomes = ExecuteSideOutcomes::default();
    let mut ctx = ArmCtx {
        entity_id: EntityId::Soldier(crate::entity_id::SoldierId(7)),
        is_npc: true,
        is_unconscious: false,
        seq_id,
        elem_idx: 0,
        sequence_manager: &mut sequence_manager,
        next_order_id: &mut next_order_id,
        side_outcomes: &mut side_outcomes,
    };

    assert!(matches!(
        dispatch_arm_completion(
            &sim,
            OrderType::WaitingUprightBored,
            MotionState::Start,
            &mut ctx,
        ),
        ExecuteOutcome::Forward(MotionState::Start)
    ));
    assert!(matches!(
        dispatch_arm_completion(
            &sim,
            OrderType::WaitingUprightBoredRandom,
            MotionState::Start,
            &mut ctx,
        ),
        ExecuteOutcome::Consumed
    ));
}

#[test]
fn bored_cycle_rolls_for_non_timer_commands_and_skips_wait_timer() {
    fn consumes_draw(command: Command) -> bool {
        let sim = crate::sim_rng::test_context();
        let seed_before = sim.seed();
        let (mut sequence_manager, seq_id) = sequence_with_order(OrderType::WaitingUprightBored);
        sequence_manager.get_element_mut(seq_id, 0).unwrap().command = command;
        let mut next_order_id = 9;
        let mut side_outcomes = ExecuteSideOutcomes::default();
        let mut ctx = ArmCtx {
            entity_id: EntityId::Pc(crate::entity_id::PcId(7)),
            is_npc: false,
            is_unconscious: false,
            seq_id,
            elem_idx: 0,
            sequence_manager: &mut sequence_manager,
            next_order_id: &mut next_order_id,
            side_outcomes: &mut side_outcomes,
        };

        let _ = dispatch_arm_completion(
            &sim,
            OrderType::WaitingUprightBored,
            MotionState::Terminated,
            &mut ctx,
        );
        sim.seed() != seed_before
    }

    assert!(consumes_draw(Command::Wait));
    assert!(consumes_draw(Command::WaitFreeLift));
    assert!(!consumes_draw(Command::WaitTimer));
}

#[test]
fn civilian_bored_cycle_does_not_run_base_actor_random_choice() {
    let seed = (0..1000)
        .find(|seed| {
            crate::sim_rng::with_seed(*seed, |sim| {
                crate::sim_rng::u32(sim, crate::sim_rng::RngSite::BoredAnimationChoice, ..10) == 0
            })
        })
        .expect("test should find a zero bored-animation roll");
    let (mut sequence_manager, seq_id) = sequence_with_order(OrderType::WaitingUprightBored);
    let original_id = sequence_manager
        .get_element(seq_id, 0)
        .unwrap()
        .orders
        .front()
        .unwrap()
        .order_id;
    let mut next_order_id = 9;
    let mut side_outcomes = ExecuteSideOutcomes::default();
    let mut ctx = ArmCtx {
        entity_id: EntityId::Civilian(crate::entity_id::CivilianId(7)),
        is_npc: true,
        is_unconscious: false,
        seq_id,
        elem_idx: 0,
        sequence_manager: &mut sequence_manager,
        next_order_id: &mut next_order_id,
        side_outcomes: &mut side_outcomes,
    };

    let outcome = crate::sim_rng::with_seed(seed, |sim| {
        dispatch_arm_completion(
            sim,
            OrderType::WaitingUprightBored,
            MotionState::Terminated,
            &mut ctx,
        )
    });

    let order = sequence_manager
        .get_element(seq_id, 0)
        .unwrap()
        .orders
        .front()
        .unwrap();
    assert!(matches!(
        outcome,
        ExecuteOutcome::Forward(MotionState::Terminated)
    ));
    assert_eq!(order.order_type, OrderType::WaitingUprightBored);
    assert_eq!(
        order.order_id, original_id,
        "civilian execution returns directly from its coerced idle arm"
    );
}

#[test]
fn unconscious_sword_start_sets_lying_waiting_sword() {
    let mut entity = weak_soldier_at_action_done(0);
    // The knock-out START arm is human-gated; the shared fixture leaves
    // the element kind unset, so stamp the real soldier kind.
    entity.element_data_mut().kind = crate::element::ElementKind::ActorSoldier;
    entity.actor_data_mut().unwrap().action_state = ActionState::Moving;

    // The knock-out hold START states are owned by the human-level
    // dispatcher (they apply to PCs and soldiers alike), not the
    // soldier-only side-effect switch.
    apply_active_animation_start_state_side_effect(
        &mut entity,
        OrderType::BeingUnconsciousSword,
        MotionState::Start,
    );

    assert_eq!(entity.element_data().posture(), Posture::Lying);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::WaitingSword
    );
}

#[test]
fn dead_random_bored_start_reports_rejected_nonlying_posture_request() {
    let mut entity = weak_soldier_at_action_done(0);
    entity.element_data_mut().kind = ElementKind::ActorSoldier;
    entity.set_posture(Posture::DeadBack);

    assert!(rejected_dead_idle_posture_callback_required(
        &entity,
        OrderType::WaitingUprightBoredRandom,
        MotionState::Start,
    ));
    assert!(rejected_dead_idle_posture_callback_required(
        &entity,
        OrderType::WaitingUpright,
        MotionState::Start,
    ));
    assert!(rejected_dead_idle_posture_callback_required(
        &entity,
        OrderType::WaitingUprightBored,
        MotionState::Start,
    ));
    assert!(!rejected_dead_idle_posture_callback_required(
        &entity,
        OrderType::WaitingUprightBoredRandom,
        MotionState::InProgress,
    ));

    apply_active_animation_start_state_side_effect(
        &mut entity,
        OrderType::WaitingUprightBoredRandom,
        MotionState::Start,
    );
    assert_eq!(
        entity.element_data().posture(),
        Posture::DeadBack,
        "the callback is driven by the rejected Upright request, not an actual posture change"
    );
}

fn sequence_with_order(
    order_type: OrderType,
) -> (
    crate::sequence::SequenceManager,
    crate::sequence::SequenceId,
) {
    let mut sequence_manager = crate::sequence::SequenceManager::new();
    let mut sequence = crate::sequence::Sequence::new();
    let mut elem = crate::sequence::SequenceElement::new_generic(
        1,
        Command::Wait,
        Some(EntityId::Pc(crate::entity_id::PcId(7))),
    );
    elem.push_order(crate::order::Order::test_new(order_type, 0.0, 0.0));
    sequence.append_element(elem);
    let seq_id = sequence_manager.launch_sequence(sequence);
    (sequence_manager, seq_id)
}

#[test]
fn lying_stuck_under_net_start_sets_original_states_for_alive_free_actor() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut entity = weak_soldier_at_action_done(0);
    entity.set_posture(Posture::Upright);
    entity.actor_data_mut().unwrap().action_state = ActionState::Moving;

    apply_under_net_initialization_side_effect(sim, &mut entity, OrderType::LyingStuckUnderNet);

    assert_eq!(entity.element_data().posture(), Posture::StuckUnderNet);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::Waiting
    );
}

#[test]
fn wriggle_under_net_start_sets_state_direction_and_soldier_emoticon() {
    let mut entity = weak_soldier_at_action_done(0);
    entity.set_posture(Posture::Upright);
    entity.actor_data_mut().unwrap().action_state = ActionState::Moving;
    entity.element_data_mut().set_direction_instantly(8);
    if let Entity::Soldier(soldier) = &mut entity {
        soldier.npc.ai_brain =
            crate::element::AiBrain::Enemy(Box::new(crate::ai_enemy::EnemyAi::new(7)));
    }

    crate::sim_rng::with_seed(1, |sim| {
        apply_under_net_initialization_side_effect(sim, &mut entity, OrderType::WriggleUnderNet);
    });

    assert_eq!(entity.element_data().posture(), Posture::StuckUnderNet);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::Waiting
    );
    assert!(matches!(entity.element_data().direction(), 7..=9));
    assert_eq!(
        entity.ai_controller().unwrap().current_emoticon_type,
        crate::ai::EmoticonType::Thunderstorm
    );
}

#[test]
fn wriggle_under_net_terminated_clears_soldier_emoticon() {
    let mut entity = weak_soldier_at_action_done(0);
    if let Entity::Soldier(soldier) = &mut entity {
        soldier.npc.ai_brain =
            crate::element::AiBrain::Enemy(Box::new(crate::ai_enemy::EnemyAi::new(7)));
    }
    entity
        .ai_controller_mut()
        .unwrap()
        .set_emoticon(crate::ai::EmoticonType::Thunderstorm);

    apply_under_net_termination_side_effect(
        &mut entity,
        OrderType::WriggleUnderNet,
        MotionState::Terminated,
    );

    assert_eq!(
        entity.ai_controller().unwrap().current_emoticon_type,
        crate::ai::EmoticonType::None
    );
}

#[test]
fn wriggle_under_net_terminated_mutates_back_and_consumes() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let (mut sequence_manager, seq_id) = sequence_with_order(OrderType::WriggleUnderNet);
    let original_id = sequence_manager
        .get_element(seq_id, 0)
        .unwrap()
        .orders
        .front()
        .unwrap()
        .order_id;
    let mut next_order_id = 9;
    let mut side_outcomes = ExecuteSideOutcomes::default();
    let mut ctx = ArmCtx {
        entity_id: EntityId::Pc(crate::entity_id::PcId(7)),
        is_npc: true,
        is_unconscious: false,
        seq_id,
        elem_idx: 0,
        sequence_manager: &mut sequence_manager,
        next_order_id: &mut next_order_id,
        side_outcomes: &mut side_outcomes,
    };

    let outcome = dispatch_arm_completion(
        sim,
        OrderType::WriggleUnderNet,
        MotionState::Terminated,
        &mut ctx,
    );

    let order = sequence_manager
        .get_element(seq_id, 0)
        .unwrap()
        .orders
        .front()
        .unwrap();
    assert!(matches!(outcome, ExecuteOutcome::Consumed));
    assert_eq!(order.order_type, OrderType::LyingStuckUnderNet);
    assert_ne!(order.order_id, original_id);
}

#[test]
fn lying_stuck_under_net_can_mutate_to_wriggle_and_consumes() {
    let seed = (0..1000)
        .find(|seed| {
            crate::sim_rng::with_seed(*seed, |sim| {
                crate::sim_rng::u32(sim, crate::sim_rng::RngSite::NetWriggleGate, ..31) == 0
            })
        })
        .expect("test should find a 1/31 roll seed");
    let (mut sequence_manager, seq_id) = sequence_with_order(OrderType::LyingStuckUnderNet);
    let mut next_order_id = 9;
    let mut side_outcomes = ExecuteSideOutcomes::default();
    let mut ctx = ArmCtx {
        entity_id: EntityId::Pc(crate::entity_id::PcId(7)),
        is_npc: true,
        is_unconscious: false,
        seq_id,
        elem_idx: 0,
        sequence_manager: &mut sequence_manager,
        next_order_id: &mut next_order_id,
        side_outcomes: &mut side_outcomes,
    };

    let outcome = crate::sim_rng::with_seed(seed, |sim| {
        dispatch_arm_completion(
            sim,
            OrderType::LyingStuckUnderNet,
            MotionState::InProgress,
            &mut ctx,
        )
    });

    let order = sequence_manager
        .get_element(seq_id, 0)
        .unwrap()
        .orders
        .front()
        .unwrap();
    assert!(matches!(outcome, ExecuteOutcome::Consumed));
    assert_eq!(order.order_type, OrderType::WriggleUnderNet);
    assert_eq!(
        side_outcomes.cry_for_help_under_net,
        vec![EntityId::Pc(crate::entity_id::PcId(7))]
    );
}
