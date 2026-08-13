//! Animation ticking.

use super::*;
use crate::element::{ActionState, Command, Entity, EyeStatus, Posture};
use crate::sprite::{FrameProgression, MotionState};

const WEAKNESS_DISMISH: u16 = 5;

/// Whether the opt-in direction-latch diagnostic admits this actor/frame.
///
/// The master switch deliberately requires both an owner and either an exact
/// frame or a bounded frame range. This keeps an accidentally enabled release
/// runner from printing every actor's `Turn()` calls.
fn turn_provenance_matches(frame: u32, owner: EntityId) -> bool {
    if std::env::var_os("PARITY_DEBUG_TURN_PROVENANCE").is_none() {
        return false;
    }

    let parse_u32 = |name: &str| {
        std::env::var(name).ok().map(|value| {
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid {name}={value:?} for Turn provenance diagnostic: {error}")
            })
        })
    };
    let owner_filter = std::env::var("PARITY_DEBUG_TURN_OWNER").unwrap_or_else(|_| {
        panic!(
            "PARITY_DEBUG_TURN_PROVENANCE requires PARITY_DEBUG_TURN_OWNER=pc|soldier|civilian:INDEX"
        )
    });
    let (kind, index) = owner_filter.split_once(':').unwrap_or_else(|| {
        panic!("PARITY_DEBUG_TURN_OWNER must look like pc|soldier|civilian:INDEX")
    });
    let index = index.parse::<u32>().unwrap_or_else(|error| {
        panic!("invalid PARITY_DEBUG_TURN_OWNER={owner_filter:?}: {error}")
    });
    let owner_matches = match (kind, owner) {
        ("pc", EntityId::Pc(_))
        | ("soldier", EntityId::Soldier(_))
        | ("civilian", EntityId::Civilian(_)) => owner.index() == index,
        ("pc" | "soldier" | "civilian", _) => false,
        _ => panic!("PARITY_DEBUG_TURN_OWNER has unsupported kind {kind:?}"),
    };
    if !owner_matches {
        return false;
    }

    if let Some(exact) = parse_u32("PARITY_DEBUG_TURN_FRAME") {
        return frame == exact;
    }
    let from = parse_u32("PARITY_DEBUG_TURN_FROM").unwrap_or_else(|| {
        panic!(
            "PARITY_DEBUG_TURN_PROVENANCE requires PARITY_DEBUG_TURN_FRAME or PARITY_DEBUG_TURN_FROM"
        )
    });
    let until = parse_u32("PARITY_DEBUG_TURN_UNTIL").unwrap_or(from);
    assert!(
        from <= until,
        "PARITY_DEBUG_TURN_FROM must not exceed PARITY_DEBUG_TURN_UNTIL"
    );
    (from..=until).contains(&frame)
}

/// Emit one direction-latch boundary for the opt-in Turn provenance probe.
///
/// This deliberately shares the existing strict owner/frame gate: the PC181
/// investigation needs to distinguish a genuine `Turn` from direct
/// `SetDirection*` writers and from owner-envelope work, without adding any
/// reads of sprite state when the diagnostic is disabled.
pub(super) fn direction_provenance_snapshot(
    position: &crate::position_interface::PositionInterface,
    owner: EntityId,
    frame: u32,
    call_class: &'static str,
) {
    if !turn_provenance_matches(frame, owner) {
        return;
    }

    let state = position.parity_turn_provenance_state();
    let map = position.map_position();
    let old_map = position.old_map_position();
    let increment = position
        .is_increment_map_computed()
        .then(|| position.get_increment_map());
    eprintln!(
        "DIRPROV frame={frame} owner={owner:?} class={call_class} deviated={} count={} dir={} goal={} map_x={:08x} map_y={:08x} old_x={:08x} old_y={:08x} increment={increment:?}",
        state.0,
        state.1,
        state.2,
        state.3,
        map.x.to_bits(),
        map.y.to_bits(),
        old_map.x.to_bits(),
        old_map.y.to_bits(),
    );
}

/// Execute one actor-owned Turn call and, when explicitly selected, expose
/// the serialized anti-vibration latch on both sides of that exact call.
fn turn_with_provenance(
    entity: &mut Entity,
    owner: EntityId,
    frame: u32,
    call_class: &'static str,
    fast: bool,
) -> bool {
    if !turn_provenance_matches(frame, owner) {
        return if fast {
            entity.position_iface_mut().turn_fast()
        } else {
            entity.position_iface_mut().turn()
        };
    }

    let before = entity.position_iface().parity_turn_provenance_state();
    let result = if fast {
        entity.position_iface_mut().turn_fast()
    } else {
        entity.position_iface_mut().turn()
    };
    let after = entity.position_iface().parity_turn_provenance_state();
    eprintln!(
        "TURNPROV frame={frame} owner={owner:?} class={call_class} fast={fast} result={result} before_deviated={} before_count={} before_dir={} before_goal={} after_deviated={} after_count={} after_dir={} after_goal={}",
        before.0, before.1, before.2, before.3, after.0, after.1, after.2, after.3,
    );
    result
}

/// Return the "alerted" variant of an animation order type when a
/// soldier is attentive.  Each soldier-specific animation handler
/// substitutes the alerted variant at the top of its "attentive"
/// branch before delegating to action/motion playback.  Returns
/// `None` for animation types that have no alerted variant.
pub(super) fn alerted_variant(anim: OrderType) -> Option<OrderType> {
    use OrderType as OT;
    match anim {
        OT::Turning => Some(OT::TurningAlerted),
        OT::WaitingUpright => Some(OT::WaitingAlerted),
        OT::WalkingUpright => Some(OT::WalkingAlerted),
        OT::WalkingStairs => Some(OT::WalkingStairsAlerted),
        OT::LookingLeft => Some(OT::LookingLeftAlerted),
        OT::LookingRight => Some(OT::LookingRightAlerted),
        OT::TransitionWalkingUprightWaitingUpright => {
            Some(OT::TransitionWalkingAlertedWaitingAlerted)
        }
        OT::TransitionRunningUprightWaitingUpright => {
            Some(OT::TransitionRunningAlertedWaitingAlerted)
        }
        OT::TransitionWaitingUprightWalkingUpright => {
            Some(OT::TransitionWaitingAlertedWalkingAlerted)
        }
        OT::TransitionWaitingUprightRunningUpright => {
            Some(OT::TransitionWaitingAlertedRunningAlerted)
        }
        OT::TransitionWalkingUprightRunningUpright => {
            Some(OT::TransitionWalkingAlertedRunningAlerted)
        }
        OT::TransitionRunningUprightWalkingUpright => {
            Some(OT::TransitionRunningAlertedWalkingAlerted)
        }
        _ => None,
    }
}

/// Resolve the concrete sprite animation used by the movement portion of
/// `RHElementActorSoldier::Execute` while leaving the authored order intact.
///
/// The Original's guards are deliberately branch-specific: upright movement
/// transitions and ordinary walking only test `mbAttentive`, while
/// stairs/turning exclude the complete sword-action family. The two sword
/// checks in the ordinary-walking branch are assertions, not release-build
/// control flow.
pub(super) fn soldier_movement_animation(
    anim: OrderType,
    attentive: bool,
    action_state: ActionState,
) -> OrderType {
    use OrderType as OT;

    if !attentive {
        return anim;
    }

    let all_sword_states = matches!(
        action_state,
        ActionState::WaitingSword
            | ActionState::MovingSword
            | ActionState::MovingFastSword
            | ActionState::ParryingSword
            | ActionState::ParryingSwordLow
    );

    match anim {
        OT::TransitionWalkingUprightWaitingUpright
        | OT::TransitionRunningUprightWaitingUpright
        | OT::TransitionWaitingUprightWalkingUpright
        | OT::TransitionWaitingUprightRunningUpright
        | OT::TransitionWalkingUprightRunningUpright
        | OT::TransitionRunningUprightWalkingUpright => alerted_variant(anim).unwrap_or(anim),
        OT::WalkingUpright => OT::WalkingAlerted,
        OT::WalkingStairs | OT::Turning if !all_sword_states => {
            alerted_variant(anim).unwrap_or(anim)
        }
        _ => anim,
    }
}

fn raising_sword_direction(owner: &Entity, opponent: &Entity) -> i16 {
    let (dx, dy) = if matches!(owner, Entity::Soldier(_)) {
        let from = owner.element_data().position_map();
        let to = opponent.element_data().position_map();
        (to.x - from.x, to.y - from.y)
    } else {
        let from = owner.element_data().position();
        let to = opponent.element_data().position();
        (to.x - from.x, to.y - from.y)
    };
    crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy)
}

fn striking_down_sword_direction(owner: &Entity, antagonist: &Entity) -> i16 {
    let from = owner.ground_position();
    let to = antagonist.ground_position();
    crate::position_interface::vector_to_sector_0_to_15_iso(to.x - from.x, to.y - from.y)
}

/// Beggar animation arms whose original PC `Execute` handler calls `Turn()`
/// before advancing the sprite action.
fn pc_beggar_execute_calls_turn(anim: OrderType) -> bool {
    matches!(
        anim,
        OrderType::TransitionWaitingUprightSimulatingBeggar
            | OrderType::TransitionSimulatingBeggarWaitingUpright
            | OrderType::SimulatingBeggar
    )
}

/// Extra per-class conditions guarding a turning animation arm.
///
/// A few arms exist in more than one actor subclass and the overrides disagree
/// about whether they rotate:
///
/// * `TransitionRaisingSword` — the human (and, through its fall-through, the
///   PC) arm turns unconditionally, while the soldier override turns only when
///   the order carries an antagonist.
/// * `ExtractingArrowSword` — turns only while actually swordfighting.
///
/// Everything else in the turning set rotates unconditionally in every
/// subclass that defines it.
fn turn_arm_condition_holds(entity: &Entity, anim: OrderType, has_antagonist: bool) -> bool {
    let is_swordfighting = entity
        .human_data()
        .is_some_and(|human| !human.opponents.is_empty());
    match anim {
        OrderType::TransitionRaisingSword => !entity.is_soldier() || has_antagonist,
        OrderType::ExtractingArrowSword => is_swordfighting,
        _ => true,
    }
}

/// Select the direction row stamped by an actor action that also turns.
///
/// Two arms call the sprite *before* they rotate, so their visible row belongs
/// to the direction at Execute entry even though the position interface has
/// advanced by the end of the same simulation frame: the attentive-soldier
/// `Turning` arm, which plays `TurningAlerted` ahead of its rotation step, and
/// the human `StandingUpSword` arm, which re-aims and turns only after its
/// sprite action.
fn actor_action_row(
    anim_type: OrderType,
    effective_anim: OrderType,
    direction_before_turn: u16,
    direction_after_turn: u16,
) -> u16 {
    if (anim_type == OrderType::Turning && effective_anim == OrderType::TurningAlerted)
        || anim_type == OrderType::StandingUpSword
    {
        direction_before_turn
    } else {
        direction_after_turn
    }
}

/// Complete the human `STANDING_UP_SWORD` arm after sprite playback.
///
/// Original refreshes the goal from the live principal opponent only while
/// swordfighting, but calls `Turn()` unconditionally. Soldiers override this
/// arm and only replay the sprite, so they must not enter this helper.
fn apply_standing_up_sword_post_perform_facing(
    entity: &mut Entity,
    principal_direction: Option<i16>,
    owner: EntityId,
    frame: u32,
) {
    if entity.is_soldier() {
        return;
    }
    if let Some(direction) = principal_direction {
        entity.element_data_mut().set_direction_goal(direction);
    }
    let _ = turn_with_provenance(
        entity,
        owner,
        frame,
        "standing_up_sword_post_perform",
        false,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{
        ActorCivilian, ActorPc, ActorSoldier, ElementData, ElementFx, ElementKind, Entity, FxData,
    };
    use crate::engine::EngineInner;

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
            element: ElementData {
                kind: ElementKind::ActorPc,
                ..Default::default()
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
            element: ElementData {
                kind: ElementKind::ActorPc,
                ..Default::default()
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
            &crate::profiles::ProfileManager::default(),
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
            &crate::profiles::ProfileManager::default(),
        );
        assert_eq!(
            soldier.actor_data().unwrap().action_state,
            ActionState::WaitingSword
        );
    }

    #[test]
    fn standing_up_sword_refreshes_new_principal_after_sprite_and_turns() {
        let mut pc = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            pc: Default::default(),
        });
        pc.element_data_mut().set_direction_instantly(11);

        // No swordfight on the preceding tick: the stale goal is retained,
        // but Original still calls Turn().
        apply_standing_up_sword_post_perform_facing(
            &mut pc,
            None,
            EntityId::Pc(crate::entity_id::PcId(0)),
            0,
        );
        assert_eq!(pc.element_data().direction(), 11);
        assert_eq!(i16::from(pc.position_iface().get_direction_goal()), 11);

        // A reciprocal EnterSwordfight later in that Hourglass makes the
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
            element: ElementData {
                kind: ElementKind::ActorPc,
                ..Default::default()
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
        let actor = engine.add_entity(weak_soldier_at_action_done(0));
        {
            let entity = engine.get_entity_mut(actor).expect("dead actor exists");
            entity
                .npc_data_mut()
                .expect("soldier has NPC data")
                .life_points = 0;
            entity.element_data_mut().posture = Posture::DeadBack;
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

        let (_, _, result) = engine.tick_actor_animation_for(
            &crate::sim_rng::test_context(),
            &crate::engine::types::LevelAssets::new(),
            actor,
        );

        assert_eq!(result.map(|result| result.motion), Some(MotionState::Start));
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
            element: ElementData {
                kind: ElementKind::ActorPc,
                ..Default::default()
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
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture: Posture::Crouched,
                ..Default::default()
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

        assert_eq!(pc.element_data().posture, Posture::Upright);
        assert_eq!(pc.actor_data().unwrap().action_state, ActionState::Waiting);
    }

    #[test]
    fn helping_climb_done_applies_posture_and_toolbar_action() {
        let mut pc = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture: Posture::Upright,
                ..Default::default()
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

        assert_eq!(pc.element_data().posture, Posture::HelpingToClimb);
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
                element: ElementData {
                    kind: ElementKind::ActorPc,
                    posture: Posture::Crouched,
                    ..Default::default()
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
            assert_eq!(pc.element_data().posture, Posture::Upright);
            assert_eq!(pc.actor_data().unwrap().action_state, ActionState::Waiting);

            apply_active_animation_start_state_side_effect(
                &mut pc,
                OrderType::TransitionCrouchingDown,
                motion,
            );
            assert_eq!(pc.element_data().posture, Posture::Crouched);
            assert_eq!(pc.actor_data().unwrap().action_state, ActionState::Waiting);
        }
    }

    fn civilian_actor() -> Entity {
        Entity::Civilian(ActorCivilian {
            element: ElementData {
                kind: ElementKind::ActorCivilian,
                ..Default::default()
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
            element: ElementData {
                kind: ElementKind::Fx,
                active: true,
                sprite,
                ..ElementData::default()
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
        let fx = engine.add_entity(animated_fx(None));
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
        let fx = engine.add_entity(animated_fx(Some(
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
    fn sword_combat_injury_start_sets_waiting_sword() {
        let mut entity = weak_soldier_at_action_done(0);
        entity.actor_data_mut().unwrap().action_state = ActionState::Moving;

        // The sword-injury START states are shared human behavior, so they
        // live in the universal active-animation dispatcher rather than the
        // soldier-only override.
        apply_active_animation_start_state_side_effect(
            &mut entity,
            OrderType::BeingStunnedSword,
            MotionState::Start,
        );

        assert_eq!(entity.element_data().posture, Posture::Upright);
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

            assert_eq!(entity.element_data().posture, posture, "{anim_type:?}");
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

        assert_eq!(entity.element_data().posture, Posture::Crouched);
        assert_eq!(
            entity.actor_data().unwrap().action_state,
            ActionState::Waiting
        );
    }

    #[test]
    fn bow_equip_start_enters_aiming_state() {
        let mut entity = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture: Posture::Upright,
                ..Default::default()
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

        assert_eq!(entity.element_data().posture, Posture::Upright);
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
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture: Posture::Upright,
                ..Default::default()
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
                element: ElementData {
                    kind: ElementKind::ActorPc,
                    posture: Posture::Upright,
                    ..Default::default()
                },
                actor: Default::default(),
                human: Default::default(),
                pc: Default::default(),
            });
            entity.actor_data_mut().unwrap().action_state = ActionState::Moving;

            apply_active_animation_start_state_side_effect(&mut entity, animation, motion);

            assert_eq!(entity.element_data().posture, expected_posture);
            assert_eq!(
                entity.actor_data().unwrap().action_state,
                ActionState::Waiting,
                "{animation:?} must apply RHElementActor::Execute's universal completion state"
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
            "RHElementActorCivilian::Execute returns directly after its coerced sprite call"
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

        assert_eq!(entity.element_data().posture, Posture::Lying);
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

        assert_eq!(entity.element_data().posture, Posture::Upright);
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

        assert_eq!(entity.element_data().posture, Posture::Upright);
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

        apply_standing_up_start_side_effect(
            &mut entity,
            OrderType::StandingUpBow,
            MotionState::Start,
        );

        assert_eq!(entity.element_data().posture, Posture::Upright);
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

            assert_eq!(entity.element_data().posture, Posture::Carried);
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

        assert_eq!(entity.element_data().posture, Posture::Upright);
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

        assert_eq!(entity.element_data().posture, Posture::Upright);
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

        assert_eq!(entity.element_data().posture, Posture::Upright);
        assert_eq!(
            entity.actor_data().unwrap().action_state,
            ActionState::Waiting
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

        assert_eq!(entity.element_data().posture, Posture::Crouched);
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

        assert_eq!(entity.element_data().posture, Posture::Flying);
        assert_eq!(
            entity.actor_data().unwrap().action_state,
            ActionState::Moving
        );

        apply_falling_completion_side_effect(
            &mut entity,
            OrderType::FallingHitWithSword,
            MotionState::Terminated,
        );

        assert_eq!(entity.element_data().posture, Posture::Lying);
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

        assert_eq!(entity.element_data().posture, Posture::Upright);
        assert_eq!(
            entity.actor_data().unwrap().action_state,
            ActionState::Menacing
        );

        apply_falling_completion_side_effect(
            &mut entity,
            OrderType::FallingHitHarderWithSword,
            MotionState::Done,
        );

        assert_eq!(entity.element_data().posture, Posture::Lying);
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

        assert_eq!(entity.element_data().posture, Posture::Lying);
        assert_eq!(
            entity.actor_data().unwrap().action_state,
            ActionState::WaitingSword
        );
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

        assert_eq!(entity.element_data().posture, Posture::Flying);
        assert_eq!(
            entity.actor_data().unwrap().action_state,
            ActionState::WaitingSword
        );

        apply_falling_completion_side_effect(
            &mut entity,
            OrderType::FallingPushedWithBow,
            MotionState::Terminated,
        );

        assert_eq!(entity.element_data().posture, Posture::Lying);
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
        );

        assert_eq!(entity.element_data().posture, Posture::Upright);
        assert_eq!(
            entity.actor_data().unwrap().action_state,
            ActionState::WaitingSword
        );

        apply_smalltalk_start_and_recovery_side_effect(
            &mut entity,
            OrderType::ParryingLeftSmalltalk,
            MotionState::Terminated,
            &profiles,
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

        assert_eq!(entity.element_data().posture, Posture::Upright);
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
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture: Posture::Upright,
                ..Default::default()
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

        let owner = engine.add_entity(pc);
        let mut soldier = weak_soldier_at_action_done(0);
        soldier.element_data_mut().kind = ElementKind::ActorSoldier;
        soldier.element_data_mut().posture = Posture::Lying;
        soldier.human_data_mut().expect("human data").unconscious = true;
        soldier.npc_data_mut().expect("NPC data").life_points = 100;
        let victim = engine.add_entity(soldier);

        let mut selected = SequenceElement::new_interaction(
            1,
            Command::SwordstrikeDown,
            Some(owner),
            Some(victim),
        );
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
    fn striking_down_revalidates_after_done_before_repeating_kill() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, assets, owner, victim, sequence) = striking_down_execute_fixture();

        let (_, first_outcomes, first_result) =
            engine.tick_actor_animation_for(&sim, &assets, owner);
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
        let (_, second_outcomes, second_result) =
            engine.tick_actor_animation_for(&sim, &assets, owner);
        assert_eq!(
            second_result.map(|result| result.motion),
            Some(MotionState::Terminated),
            "the post-PerformAction validity check must retire the stale strike"
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
                    crate::sim_rng::u32(sim, crate::sim_rng::RngSite::BoredAnimationChoice, ..10)
                        != 0
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
            "Original calls NewID only inside the 1-in-10 mutation branch"
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
            let (mut sequence_manager, seq_id) =
                sequence_with_order(OrderType::WaitingUprightBored);
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
                    crate::sim_rng::u32(sim, crate::sim_rng::RngSite::BoredAnimationChoice, ..10)
                        == 0
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
            "RHElementActorCivilian::Execute returns directly from its coerced idle arm"
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

        assert_eq!(entity.element_data().posture, Posture::Lying);
        assert_eq!(
            entity.actor_data().unwrap().action_state,
            ActionState::WaitingSword
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

        apply_under_net_cycle_side_effect(
            sim,
            &mut entity,
            OrderType::LyingStuckUnderNet,
            MotionState::Start,
        );

        assert_eq!(entity.element_data().posture, Posture::StuckUnderNet);
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
            apply_under_net_cycle_side_effect(
                sim,
                &mut entity,
                OrderType::WriggleUnderNet,
                MotionState::Start,
            );
        });

        assert_eq!(entity.element_data().posture, Posture::StuckUnderNet);
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
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut entity = weak_soldier_at_action_done(0);
        if let Entity::Soldier(soldier) = &mut entity {
            soldier.npc.ai_brain =
                crate::element::AiBrain::Enemy(Box::new(crate::ai_enemy::EnemyAi::new(7)));
        }
        entity
            .ai_controller_mut()
            .unwrap()
            .set_emoticon(crate::ai::EmoticonType::Thunderstorm);

        apply_under_net_cycle_side_effect(
            sim,
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
}

/// Whether a soldier is "attentive" for sprite-row purposes.  Reads
/// the soldier's attentive flag, which the completion handler for
/// `TransitionWaitingUprightWaitingAlerted` flips on.  Sword-pose
/// action states suppress the alerted variant to match the explicit
/// gating in the per-case soldier handlers.
///
/// Historic note: this used to also fall back to a
/// `ai.current_state == Attacking` proxy as a workaround for the
/// pre-port bug where the alerted transition never fired and the
/// port "snapped straight into AttackingReactiontime" with the flag
/// stuck at `false`.  The transition now fires correctly via
/// `set_soldier_attentive_mode` → sequence-phase attention dispatch, so
/// the proxy is obsolete — and harmful, because it made
/// sprite-substitution happen one frame *before* the transition
/// animation started (the AI state flips synchronously inside
/// `set_state(Attacking, …)`, a frame before the attention context
/// queues the animation).  That one-frame lead produced a visible
/// pop: `WaitingUprightBored` → `TurningAlerted` (1 frame of
/// "sword-drawn" pose) → `TransitionWaitingUprightBoredWaitingUpright`
/// → the real lean-forward transition.
pub(super) fn soldier_is_attentive(entity: &Entity) -> bool {
    if !matches!(entity, Entity::Soldier(_)) {
        return false;
    }
    let attentive_flag = entity.enemy_ai().map(|e| e.attentive).unwrap_or(false);
    let action_state = entity
        .actor_data()
        .map(|a| a.action_state)
        .unwrap_or(ActionState::Waiting);
    let not_sword = !matches!(
        action_state,
        ActionState::WaitingSword
            | ActionState::MovingSword
            | ActionState::MovingFastSword
            | ActionState::ParryingSword
            | ActionState::ParryingSwordLow
    );
    attentive_flag && not_sword
}

/// Side effects that need to touch entities other than the
/// soldier itself — returned by
/// [`apply_soldier_execute_side_effects`] so the caller (which has
/// `&mut self` on the engine) can apply them.  Covers cross-entity
/// mutations the per-anim handlers need (deactivating the antagonist
/// bottle on DRINKING_ALE, removing a picked-up object on TAKING,
/// etc.).
#[derive(Debug, Clone, Default)]
pub(super) struct ExecuteSideOutcomes {
    /// PCs whose DROPPING_ALE animation reached DONE. Original creates the
    /// bottle and consumes one ale at this action point, before the order's
    /// later TERMINATED edge advances the sequence.
    pub drop_ale_done: Vec<EntityId>,
    /// Antagonist IDs whose `is_active` should be cleared (bottle hide
    /// on DRINKING_ALE DONE).
    pub deactivate_entities: Vec<EntityId>,
    /// `(taker, object)` pairs: the `taker` picks up `object` (purse
    /// or coin); the `object` is removed from the world and the
    /// `taker`'s money grows by the object's value.  Fired on
    /// TAKING DONE.
    pub pickups: Vec<(EntityId, EntityId)>,
    /// Soldiers that should gain `blood_alcohol += profile.beer` on
    /// DRINKING_ALE TERMINATED.
    pub drink_done: Vec<EntityId>,
    /// Entities that should say the wasp-sting remark on
    /// GETTING_FREE_FROM_WASP initialisation.
    pub wasp_sting_remark: Vec<EntityId>,
    /// Entities that should fire the special-action remark
    /// (SPECIAL at start-of-anim).
    pub special_remark: Vec<EntityId>,
    /// Humans whose weak/stunned sword animation just initialized, paired
    /// with the exact wrapper type. Both add the weak-stunned titbit and
    /// notify soldier opponents; only `BeingWeakSword` transfers initiative.
    pub weak_stunned_start: Vec<(EntityId, OrderType)>,
    /// `(thief, victim)` — NPC-on-NPC pickpocket transfer on
    /// SEARCHING DONE: thief gains the victim's money and the victim
    /// is zeroed out.
    pub pickpockets: Vec<(EntityId, EntityId)>,
    /// `(pc, target, activation_cmd)` — PC target interaction animation
    /// reached DONE, so launch the target-side activation sequence.
    /// Covers USING_LEVER / HITTING_TARGET / HANDLING_TARGET /
    /// TAKING_TARGET / SEARCHING.
    pub pc_target_activations: Vec<(EntityId, EntityId, Command)>,
    /// Entities that rolled the LYING_STUCK_UNDER_NET 1/31
    /// cycle-flip this tick and are NPCs.  Each fires
    /// `Say(REMARK_UNDER_NET)` or `Say(CIV_REMARK_UNDER_NET)`
    /// depending on Soldier vs Civilian, plus a `HEEELP` noise at
    /// the entity's position (volume 200).
    pub cry_for_help_under_net: Vec<EntityId>,
    /// `(attacker, antagonist, strike)` triples whose smalltalk sword strike
    /// reached its action-done tag. A back-hit against a swordfighting
    /// antagonist launches real sword damage; a harmless mutually-engaged
    /// swipe only plays its FX.
    pub smalltalk_strikes: Vec<(EntityId, EntityId, crate::weapons::SwordStrike)>,
    /// `(victim, killer)` pairs launched when STRIKING_DOWN_SWORD
    /// reaches its action-done tag.
    pub killed_at_bottom: Vec<(EntityId, EntityId)>,
    /// `(rescuer, target)` pairs fired when WAKING_UP reaches DONE.
    /// The target receives the legacy implementation wake-up side effect in the
    /// post-animation drain where the engine can mutate another human.
    pub waking_up_done: Vec<(EntityId, EntityId)>,
    /// PCs leaving cape/tree disguise remove their Hidden titbit when
    /// the transition reaches DONE.
    pub hidden_titbit_removals: Vec<EntityId>,
    /// PC beggar transitions whose DONE edge toggles nearby ground-coin
    /// eligibility. `true` enters the disguise; `false` leaves it.
    pub beggar_coin_flags: Vec<(EntityId, bool)>,
    /// PCs whose beggar transition reached DONE. Both Original transition
    /// arms call `Wait()` synchronously before returning that DONE result;
    /// `true` distinguishes entry's following SelectAction(Beggar) message.
    pub beggar_wait_handoffs: Vec<(EntityId, bool)>,
    /// PCs whose crouch posture transition reached DONE or TERMINATED.
    /// Original forwards the unparameterized stature-change notification
    /// from both terminal switch arms.
    pub stature_change_end: Vec<EntityId>,
    /// Non-script PC bow-equip transitions that reached START. Original
    /// forwards `MSG_SELECT_ACTION(BOW)` from the Human Execute arm after
    /// entering the aiming state, restoring the PC's remembered action (and
    /// the messenger action when that PC is selected).
    pub pc_bow_equip_action: Vec<EntityId>,
    /// PCs whose helping-to-climb entry transition reached DONE. Original
    /// forwards `MSG_SELECT_ACTION(HELP_TO_CLIMB)` right after setting the
    /// stance, which reselects the already-current action and therefore
    /// still stops a selected PC's group at Normal priority.
    pub pc_helping_climb_action: Vec<EntityId>,
}

fn forwards_pc_bow_action_on_start(
    entity: &Entity,
    anim_type: OrderType,
    motion: MotionState,
    script_driven: bool,
) -> bool {
    entity.is_pc()
        && !script_driven
        && motion == MotionState::Start
        && matches!(
            anim_type,
            OrderType::TransitionEquipBow | OrderType::TransitionEquipBowAnonymous
        )
}

/// Apply the soldier-specific side effects triggered on each motion-state
/// transition (Start, Done, Terminated) of an `active_ai_anim`
/// animation.  Covers the 42 animation cases: action-state/posture
/// transitions, attentive-flag toggling, view-status updates for
/// LOOKING_* anims, sleep/leaning-out/bow-lean transitions,
/// DRINKING_ALE / TAKING / SPECIAL / GETTING_FREE_FROM_WASP
/// antagonist-dependent effects.
///
/// Invoked from `tick_actor_animation_for` after each `perform_action`
/// call on an `active_ai_anim`.  Mutations that can be applied to
/// `entity` directly are; cross-entity mutations (bottle hide,
/// coin pickup) accumulate in the returned [`ExecuteSideOutcomes`]
/// so the caller can process them with `&mut self` after the entity
/// loop.
fn apply_soldier_execute_side_effects(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
    antagonist: Option<EntityId>,
    entity_id: EntityId,
    outcomes: &mut ExecuteSideOutcomes,
    _profile_manager: &crate::profiles::ProfileManager,
) {
    use crate::order::OrderType as OT;
    use crate::sprite::MotionState as MS;

    if !matches!(entity, Entity::Soldier(_)) {
        return;
    }

    let set_states = |e: &mut Entity, posture: Posture, action: ActionState| {
        e.set_posture(posture);
        if let Some(a) = e.actor_data_mut() {
            a.action_state = action;
        }
    };

    match (anim_type, motion) {
        // TRANSITION_WAITING_UPRIGHT_WAITING_ALERTED: attentive = true
        (OT::TransitionWaitingUprightWaitingAlerted, MS::Done | MS::Terminated) => {
            if let Some(e) = entity.enemy_ai_mut() {
                e.attentive = true;
            }
        }
        // TRANSITION_WAITING_ALERTED_WAITING_UPRIGHT (+officer variant):
        // attentive = false
        (
            OT::TransitionWaitingAlertedWaitingUpright
            | OT::TransitionWaitingAlertedWaitingUprightOfficer,
            MS::Done | MS::Terminated,
        ) => {
            if let Some(e) = entity.enemy_ai_mut() {
                e.attentive = false;
            }
        }

        // Movement-transition end states.
        // NB the WAITING_UPRIGHT → WALKING_UPRIGHT transition sets the
        // *end* state to Waiting — the actual walk is launched by the
        // next order in the sequence.
        (
            OT::TransitionWalkingUprightWaitingUpright
            | OT::TransitionRunningUprightWaitingUpright
            | OT::TransitionWaitingUprightWalkingUpright
            | OT::TransitionWalkingAlertedWaitingAlerted
            | OT::TransitionRunningAlertedWaitingAlerted
            | OT::TransitionWaitingAlertedWalkingAlerted,
            MS::Done | MS::Terminated,
        ) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
        }

        // Walking / running animation Start: flip `action_state` to
        // Moving / MovingFast when the walk order becomes the actor's
        // current order.  Without this, a soldier coming out of a
        // WAITING_UPRIGHT_WALKING_UPRIGHT startup transition ends in
        // `Waiting` (per the handler above), the walk order becomes
        // current, but `tick_entity_movement`'s `is_moving()` gate
        // never flips → actor walks-in-place forever.
        // Walking/running Start handlers moved to
        // `apply_npc_execute_side_effects` so civilians get them too
        // (the NPC dispatch covers both soldier and civilian
        // walking/running).
        (
            OT::TransitionWaitingUprightRunningUpright
            | OT::TransitionWalkingUprightRunningUpright
            | OT::TransitionWaitingAlertedRunningAlerted
            | OT::TransitionWalkingAlertedRunningAlerted,
            MS::Done | MS::Terminated,
        ) => {
            set_states(entity, Posture::Upright, ActionState::MovingFast);
        }
        (
            OT::TransitionRunningUprightWalkingUpright | OT::TransitionRunningAlertedWalkingAlerted,
            MS::Done | MS::Terminated,
        ) => {
            set_states(entity, Posture::Upright, ActionState::Moving);
        }

        // BEING_UNCONSCIOUS_* START is handled for every human by
        // `apply_active_animation_start_state_side_effect`.

        // TRANSITION_RAISING_SWORD → WaitingSword on DONE
        (OT::TransitionRaisingSword, MS::Done) => {
            set_states(entity, Posture::Upright, ActionState::WaitingSword);
        }

        // TRANSITION_CHARGING → WaitingSword on DONE (damage resolved
        // separately in melee module).
        (OT::TransitionCharging, MS::Done) => {
            set_states(entity, Posture::Upright, ActionState::WaitingSword);
        }

        // TAKING: TERMINATED → Waiting
        (OT::Taking, MS::Terminated) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
        }

        // TRANSITION_WAITING_SWORD_MENACING: TERMINATED → Menacing
        (OT::TransitionWaitingSwordMenacing, MS::Terminated) => {
            set_states(entity, Posture::Upright, ActionState::Menacing);
        }

        // SLEEPING_UPRIGHT: every tick → (Upright, Sleeping)
        (OT::SleepingUpright, _) => {
            set_states(entity, Posture::Upright, ActionState::Sleeping);
        }

        // TRANSITION_SLEEPING_WAITING_UPRIGHT: TERMINATED → Waiting
        (OT::TransitionSleepingWaitingUpright, MS::Terminated) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
        }

        // TRANSITION_MENACING_WAITING_SWORD: TERMINATED → WaitingSword
        (OT::TransitionMenacingWaitingSword, MS::Terminated) => {
            set_states(entity, Posture::Upright, ActionState::WaitingSword);
        }

        // LEANING_OUT: START → (LeaningOut, Waiting)
        (OT::LeaningOut, MS::Start) => {
            set_states(entity, Posture::LeaningOut, ActionState::Waiting);
        }

        // TRANSITION_WAITING_ALERTED_LEANING_OUT: DONE → (LeaningOut, Waiting)
        (OT::TransitionWaitingAlertedLeaningOut, MS::Done) => {
            set_states(entity, Posture::LeaningOut, ActionState::Waiting);
        }

        // TRANSITION_LEANING_OUT_WAITING_ALERTED: DONE → (Upright, Waiting)
        (OT::TransitionLeaningOutWaitingAlerted, MS::Done) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
        }

        // TRANSITION_LOWERING_BOW_LEANING_OUT: DONE/TERMINATED →
        // (LeaningOut, AimingWithBowDown).
        (OT::TransitionLoweringBowLeaningOut, MS::Done | MS::Terminated) => {
            set_states(entity, Posture::LeaningOut, ActionState::AimingWithBowDown);
        }

        // TRANSITION_RAISING_BOW_LEANING_OUT: DONE/TERMINATED →
        // (Upright, AimingWithBow).
        (OT::TransitionRaisingBowLeaningOut, MS::Done | MS::Terminated) => {
            set_states(entity, Posture::Upright, ActionState::AimingWithBow);
        }

        // SHOOTING_WITH_BOW_LEANING_OUT: DONE → (LeaningOut, AimingWithBow)
        // The actual arrow release side effect is handled in the
        // `bow_shot` module.
        (OT::ShootingWithBowLeaningOut, MS::Done) => {
            set_states(entity, Posture::LeaningOut, ActionState::AimingWithBow);
        }

        // LOOKING_LEFT / LOOKING_LEFT_ALERTED: START → LookToTheLeft,
        // DONE → LookForward.
        (OT::LookingLeft | OT::LookingLeftAlerted, MS::Start) => {
            if let Some(npc) = entity.npc_data_mut() {
                crate::ai_vision::set_view_status(npc, EyeStatus::LookToTheLeft);
            }
        }
        (OT::LookingLeft | OT::LookingLeftAlerted, MS::Done) => {
            if let Some(npc) = entity.npc_data_mut() {
                crate::ai_vision::set_view_status(npc, EyeStatus::LookForward);
            }
        }
        (OT::LookingRight | OT::LookingRightAlerted, MS::Start) => {
            if let Some(npc) = entity.npc_data_mut() {
                crate::ai_vision::set_view_status(npc, EyeStatus::LookToTheRight);
            }
        }
        (OT::LookingRight | OT::LookingRightAlerted, MS::Done) => {
            if let Some(npc) = entity.npc_data_mut() {
                crate::ai_vision::set_view_status(npc, EyeStatus::LookForward);
            }
        }

        // DRINKING_ALE:
        //   START: set states to (Upright, Waiting).
        //   DONE:  deactivate the antagonist (hide the bottle).
        //   TERMINATED: blood_alcohol += profile.beer.
        (OT::DrinkingAle, MS::Start) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
        }
        (OT::DrinkingAle, MS::Done) => {
            if let Some(a) = antagonist {
                outcomes.deactivate_entities.push(a);
            }
        }
        (OT::DrinkingAle, MS::Terminated) => {
            outcomes.drink_done.push(entity_id);
        }

        // TAKING DONE: pick up the antagonist (Purse or Coin) and add
        // its value to the soldier's money.  TERMINATED switches back
        // to (Upright, Waiting) — already handled above (the match arm
        // for `(OT::Taking, MS::Terminated)` fires before this).
        (OT::Taking, MS::Done) => {
            if let Some(a) = antagonist {
                outcomes.pickups.push((entity_id, a));
            }
        }

        // GETTING_FREE_FROM_WASP: on initialisation sets a random
        // rotation offset and says REMARK_WASP_STING; while still
        // turning, plays TURNING_ALERTED instead of the configured
        // animation.  The random-rotation setup is booking-site
        // business (must happen before `active_ai_anim` is set so the
        // target direction is correct); here we just fire the remark
        // once the animation actually starts.
        (OT::GettingFreeFromWasp, MS::Start) => {
            outcomes.wasp_sting_remark.push(entity_id);
        }
        // NB: `wasp_victim = false` is not handled here.  The reset
        // lives in `EngineInner::send_condolation_card`
        // (engine/soldier_helpers.rs), fired by the general
        // sequence-terminated queue when the `ReceiveWaspSting`
        // element finishes.

        // MENACING, WAITING_ALERTED, GATHERING_SOLDIERS,
        // AIMING_WITH_BOW_LEANING_OUT: these cases just play the
        // action and return — no state side effects other than what
        // perform_action already does.
        //
        // TRANSITION_CHARGING DONE damage is applied in `melee.rs`
        // where the charging is booked; we only handle the
        // (Upright, WaitingSword) state change here.
        _ => {}
    }
}

fn special_remark_due_at_sprite_phase(
    speech_id: u32,
    current_frame: u16,
    frame_count: u16,
) -> bool {
    const SPEECH_ID_HELBARDMAN: u32 = 0x4c484453;
    const EAT_FRAMES_HELBARDMAN: u16 = 40;
    frame_count == 0
        && if speech_id == SPEECH_ID_HELBARDMAN {
            current_frame == EAT_FRAMES_HELBARDMAN
        } else {
            current_frame == 0
        }
}

fn special_remark_due_for_execute(
    speech_id: u32,
    before_perform: (u16, u16),
    after_perform: (u16, u16),
) -> bool {
    const SPEECH_ID_HELBARDMAN: u32 = 0x4c484453;
    let (current_frame, frame_count) = if speech_id == SPEECH_ID_HELBARDMAN {
        after_perform
    } else {
        before_perform
    };
    special_remark_due_at_sprite_phase(speech_id, current_frame, frame_count)
}

/// Walk/run animation Start → flip `action_state` to the matching
/// moving variant.  Fires at the start of each walking order's
/// sprite playback for all variants (running, sword, alerted,
/// crouched).
///
/// Applies to **all actor kinds** (PC, soldier, civilian).  Without
/// this, an actor coming out of a
/// `WAITING_UPRIGHT_WALKING_UPRIGHT` startup transition ends in
/// `Waiting` (per the transition-end handler above), the walk order
/// becomes current, but nothing flips `action_state` →
/// `tick_entity_movement`'s `is_moving()` gate never trips and the
/// actor walks-in-place forever.
pub(super) fn apply_actor_walk_start_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
) {
    use crate::order::OrderType as OT;
    use crate::sprite::MotionState as MS;

    if !matches!(motion, MS::Start) {
        return;
    }
    if entity.actor_data().is_none() {
        return;
    }

    let set_states = |e: &mut Entity, posture: Posture, action: ActionState| {
        e.set_posture(posture);
        if let Some(a) = e.actor_data_mut() {
            a.action_state = action;
        }
    };

    match anim_type {
        OT::WalkingUpright | OT::WalkingAlerted => {
            set_states(entity, Posture::Upright, ActionState::Moving);
        }
        // The crouched walk keeps the actor crouched; only the action
        // state moves on. Sharing the upright arm made every crouched
        // walk order stand the actor up on its first frame.
        OT::WalkingCrouched => {
            set_states(entity, Posture::Crouched, ActionState::Moving);
        }
        OT::RunningUpright => {
            set_states(entity, Posture::Upright, ActionState::MovingFast);
        }
        OT::WalkingWithSword => {
            set_states(entity, Posture::Upright, ActionState::MovingSword);
        }
        OT::RunningWithSword => {
            set_states(entity, Posture::Upright, ActionState::MovingFastSword);
        }
        _ => {}
    }
}

/// Apply the per-anim-type side effects from the NPC parent handler
/// — the cases that fall through from the soldier switch into the
/// shared NPC dispatch, plus the civilian-side handlers that reuse
/// them.  Covers SITTING / POINTING / SEARCHING /
/// TRANSITION_SITTING_WAITING_UPRIGHT / TRANSITION_WAITING_UPRIGHT_SITTING /
/// BEGGAR_SHOWING_FACE.
///
/// Applied to any NPC (soldier or civilian) with an
/// `active_ai_anim` — the actual animation is played by the sprite
/// system; this function just runs the post-motion-state dispatch
/// (state transitions, pickpocket money transfer).  Called after
/// `apply_soldier_execute_side_effects` so soldier-specific
/// overrides still take priority.
pub(super) fn apply_npc_execute_side_effects(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
    antagonist: Option<EntityId>,
    entity_id: EntityId,
    outcomes: &mut ExecuteSideOutcomes,
) {
    use crate::order::OrderType as OT;
    use crate::sprite::MotionState as MS;

    // Only NPCs (soldiers, civilians) get the NPC dispatch treatment.
    if !matches!(entity, Entity::Soldier(_) | Entity::Civilian(_)) {
        return;
    }

    let set_states = |e: &mut Entity, posture: Posture, action: ActionState| {
        e.set_posture(posture);
        if let Some(a) = e.actor_data_mut() {
            a.action_state = action;
        }
    };

    match (anim_type, motion) {
        // SITTING: START → (Sitting, Waiting).
        (OT::Sitting, MS::Start) => {
            set_states(entity, Posture::Sitting, ActionState::Waiting);
        }

        // TRANSITION_SITTING_WAITING_UPRIGHT: TERMINATED → (Upright, Waiting).
        (OT::TransitionSittingWaitingUpright, MS::Terminated) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
        }

        // TRANSITION_WAITING_UPRIGHT_SITTING: TERMINATED → (Sitting, Waiting).
        (OT::TransitionWaitingUprightSitting, MS::Terminated) => {
            set_states(entity, Posture::Sitting, ActionState::Waiting);
        }

        // TRANSITION_WAITING_UPRIGHT_SPECIAL (ENTER_LEISURE):
        // DONE/TERMINATED → (Leisure, Waiting).  Both motion states
        // flip posture so the actor stays in Leisure once the
        // transition animation finishes (DONE) or is interrupted
        // (TERMINATED).
        (OT::TransitionWaitingUprightSpecial, MS::Done | MS::Terminated) => {
            set_states(entity, Posture::Leisure, ActionState::Waiting);
        }

        // TRANSITION_SPECIAL_WAITING_UPRIGHT (leave-leisure):
        // DONE/TERMINATED → (Upright, Waiting).
        (OT::TransitionSpecialWaitingUpright, MS::Done | MS::Terminated) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
        }

        // BEGGAR_SHOWING_FACE: TERMINATED → (Upright, Waiting).
        // The sprite-animation selection above falls back to Rolling when
        // the beggar lacks the showing-face animation; dispatch remains
        // keyed on this original order type, matching the Original.
        (OT::BeggarShowingFace, MS::Terminated) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
        }

        // POINTING: TERMINATED → (Upright, Waiting).
        // The booking site already sets the direction field; we only
        // restore the idle state when the point gesture finishes.
        (OT::Pointing, MS::Terminated) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
        }

        // Base actor idle-state changes are applied by
        // `apply_active_animation_start_state_side_effect`. The NPC override
        // only adds the officer eye-status behavior here.
        (OT::WaitingUprightBoredRandom, MS::Start) => {
            // Officer-only: LookToTheRight on START, LookForward on DONE.
            if entity
                .enemy_ai()
                .map(|e| e.soldier_profile_rank == crate::profiles::ProfileRank::Officer)
                .unwrap_or(false)
                && let Some(npc) = entity.npc_data_mut()
            {
                crate::ai_vision::set_view_status(npc, EyeStatus::LookToTheRight);
            }
        }
        (OT::WaitingUprightBoredRandom, MS::Done) => {
            if entity
                .enemy_ai()
                .map(|e| e.soldier_profile_rank == crate::profiles::ProfileRank::Officer)
                .unwrap_or(false)
                && let Some(npc) = entity.npc_data_mut()
            {
                crate::ai_vision::set_view_status(npc, EyeStatus::LookForward);
            }
        }

        // SEARCHING: DONE → (Upright, Waiting) + NPC-on-NPC pickpocket
        // money transfer (thief gains the victim's money and the
        // victim is zeroed out).  The state change fires on DONE
        // (before the switch advances) rather than TERMINATED.
        (OT::Searching, MS::Done) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
            if let Some(victim) = antagonist {
                outcomes.pickpockets.push((entity_id, victim));
            }
        }

        _ => {}
    }
}

/// PC target interactions are two-stage: the PC first plays the
/// visible action animation, then the target receives the activation
/// command on motion Done.
pub(super) fn apply_pc_target_interaction_side_effect(
    entity: &Entity,
    anim_type: OrderType,
    motion: MotionState,
    antagonist: Option<EntityId>,
    entity_id: EntityId,
    outcomes: &mut ExecuteSideOutcomes,
) {
    if !matches!(entity, Entity::Pc(_)) || !matches!(motion, MotionState::Done) {
        return;
    }
    let Some(target) = antagonist else {
        return;
    };
    let activation = match anim_type {
        OrderType::HittingTarget => Command::ActivateSword,
        OrderType::HandlingTarget | OrderType::TakingTarget => Command::ActivateHandle,
        OrderType::UsingLever => Command::ActivateLever,
        OrderType::Searching => Command::ActivateSearch,
        _ => return,
    };
    outcomes
        .pc_target_activations
        .push((entity_id, target, activation));
}

/// Universal `TakingNet` Done handler — fires for any actor (PC or
/// NPC) playing the net-pickup animation.
///
/// When the order's motion completes, the antagonist net is removed
/// from the engine, the net's effect is unapplied (releasing its
/// victims), and (PC-only) the picker's Net-action ammo is
/// incremented.
///
/// We push the `(taker, net)` pair into `outcomes.pickups`; the
/// post-tick handler in `tick.rs` recognises Net antagonists and
/// runs the actual unapply + remove + ammo bump with `&mut self`.
fn apply_taking_net_side_effect(
    anim_type: OrderType,
    motion: MotionState,
    antagonist: Option<EntityId>,
    entity_id: EntityId,
    outcomes: &mut ExecuteSideOutcomes,
) {
    if matches!(anim_type, OrderType::TakingNet)
        && matches!(motion, MotionState::Done)
        && let Some(a) = antagonist
    {
        outcomes.pickups.push((entity_id, a));
    }
}

fn apply_waking_up_done_side_effect(
    anim_type: OrderType,
    motion: MotionState,
    antagonist: Option<EntityId>,
    entity_id: EntityId,
    outcomes: &mut ExecuteSideOutcomes,
) {
    if matches!(anim_type, OrderType::WakingUp)
        && matches!(motion, MotionState::Done)
        && let Some(target) = antagonist
    {
        outcomes.waking_up_done.push((entity_id, target));
    }
}

/// Universal active-animation START state changes shared by PCs and
/// NPCs.  Mirrors the legacy implementation `RHMOTION_START` `SetStates(...)` side
/// effects for active animation arms whose completion logic is handled
/// elsewhere.
fn apply_active_animation_start_state_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
) {
    // RHElementActorCivilian::Execute overrides this entire family, coerces
    // the sprite animation to WAITING_UPRIGHT_BORED, and returns directly.
    // None of RHElementActor::Execute's posture/action-state switch arms run.
    if entity.is_civilian()
        && matches!(
            anim_type,
            OrderType::WaitingUpright
                | OrderType::WaitingUprightBored
                | OrderType::WaitingUprightBoredRandom
                | OrderType::TransitionWaitingUprightBoredWaitingUpright
                | OrderType::TransitionWaitingUprightWaitingUprightBored
        )
    {
        return;
    }

    match (anim_type, motion) {
        // These cases are implemented by RHElementActorHuman::Execute, not
        // the soldier override, so PCs must receive the same START state.
        // Keeping them in the soldier-only side-effect dispatcher left a PC
        // in MovingSword for the first BEING_HIT_SWORD frame.
        (
            OrderType::BeingHitSword
            | OrderType::ExtractingArrowSword
            | OrderType::BeingStunnedSword,
            MotionState::Start,
        ) => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::WaitingSword;
            }
            return;
        }
        // The knock-out hold animations are owned by the human Execute
        // switch, so every human — PC, soldier, civilian — settles into
        // the lying pose on the first frame.  Leaving them soldier-only
        // let a knocked-out PC keep whatever action state it carried
        // into the blow (typically MovingSword or Bored).
        (OrderType::BeingUnconsciousSword, MotionState::Start) if entity.is_human() => {
            entity.set_posture(Posture::Lying);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::WaitingSword;
            }
            return;
        }
        (OrderType::BeingUnconsciousBow, MotionState::Start) if entity.is_human() => {
            entity.set_posture(Posture::Lying);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::AimingWithBow;
            }
            return;
        }
        (OrderType::BeingUnconscious, MotionState::Start) if entity.is_human() => {
            entity.set_posture(Posture::Lying);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        // Corpse-carry idle hold: only a PC ever executes it, and its
        // first frame settles the carrier back into Waiting.  Without
        // it a carrier that stops walking keeps the Moving state the
        // walk order stamped, and every later carry frame — including
        // the drop transition — inherits it.
        (OrderType::WaitingWithCorpse, MotionState::Start) if entity.is_pc() => {
            entity.set_posture(Posture::CarryingCorpse);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        // End of the lift animation: the carrier owns the body and is
        // standing still with it.
        (OrderType::TransitionWaitingUprightCarryingCorpse, MotionState::Done)
            if entity.is_pc() =>
        {
            entity.set_posture(Posture::CarryingCorpse);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        (OrderType::Taking | OrderType::TakingCrouched, MotionState::Terminated)
            if entity.is_pc() =>
        {
            entity.set_posture(if anim_type == OrderType::Taking {
                Posture::Upright
            } else {
                Posture::Crouched
            });
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        // The crouched idle loop is executed only by the PC, and settles
        // the actor into Waiting on its first frame. Without this a PC
        // that stops crouch-walking keeps a stale Moving action state
        // into the following posture transition.
        (OrderType::WaitingCrouched, MotionState::Start) if entity.is_pc() => {
            entity.set_posture(Posture::Crouched);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        // RHElementActorHuman owns this START transition for PCs and other
        // non-soldier humans.  Soldiers override the arm and settle into
        // WaitingSword on DONE instead.
        (OrderType::TransitionRaisingSword, MotionState::Start)
            if !matches!(entity, Entity::Soldier(_)) =>
        {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::WaitingSword;
            }
            return;
        }
        (OrderType::TransitionLoweringSword, MotionState::Start) => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        (OrderType::WaitingUpright, MotionState::Start) => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        (
            OrderType::WaitingUprightBored | OrderType::WaitingUprightBoredRandom,
            MotionState::Start,
        ) => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Bored;
            }
            return;
        }
        (
            OrderType::TransitionWaitingUprightBoredWaitingUpright,
            MotionState::Done | MotionState::Terminated,
        ) => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        (
            OrderType::TransitionWaitingUprightWaitingUprightBored,
            MotionState::Done | MotionState::Terminated,
        ) => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Bored;
            }
            return;
        }
        (
            OrderType::TransitionWalkingUprightWaitingUpright
            | OrderType::TransitionRunningUprightWaitingUpright,
            MotionState::Done | MotionState::Terminated,
        ) => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        (
            OrderType::TransitionWalkingCrouchedWaitingCrouched,
            MotionState::Done | MotionState::Terminated,
        ) => {
            entity.set_posture(Posture::Crouched);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        (OrderType::TransitionWaitingUprightHelpingClimbing, MotionState::Done) => {
            entity.set_posture(Posture::HelpingToClimb);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            if let Some(pc) = entity.pc_data_mut() {
                pc.current_action = crate::profiles::Action::HelpToClimb;
            }
            return;
        }
        (OrderType::TransitionHelpingClimbingWaitingUpright, MotionState::Done) => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            if let Some(pc) = entity.pc_data_mut() {
                pc.current_action = crate::profiles::Action::NoAction;
            }
            return;
        }
        (OrderType::TransitionWaitingUprightSimulatingBeggar, MotionState::Done) => {
            entity.set_posture(Posture::SimulatingBeggar);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            if let Some(pc) = entity.pc_data_mut() {
                pc.current_action = crate::profiles::Action::Beggar;
            }
            return;
        }
        (OrderType::TransitionSimulatingBeggarWaitingUpright, MotionState::Done) => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            if let Some(pc) = entity.pc_data_mut() {
                pc.current_action = crate::profiles::Action::NoAction;
            }
            return;
        }
        (OrderType::TransitionCrouchingUp, MotionState::Done | MotionState::Terminated) => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        (OrderType::TransitionCrouchingDown, MotionState::Done | MotionState::Terminated) => {
            entity.set_posture(Posture::Crouched);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        (
            OrderType::TransitionEquipBow | OrderType::TransitionEquipBowAnonymous,
            MotionState::Start,
        ) => {
            if entity.element_data().posture != Posture::AnonymousArcher {
                entity.set_posture(Posture::Upright);
            }
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::AimingWithBow;
            }
            return;
        }
        (
            OrderType::TransitionUnequipBow | OrderType::TransitionUnequipBowAnonymous,
            MotionState::Start,
        ) => {
            if entity.element_data().posture != Posture::AnonymousArcher {
                entity.set_posture(Posture::Upright);
            }
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        (
            OrderType::TransitionUnloadBow | OrderType::TransitionUnloadBowAnonymous,
            MotionState::Start,
        ) => {
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
            return;
        }
        (
            OrderType::TransitionLoweringBow | OrderType::TransitionLoweringBowAnonymous,
            MotionState::Done | MotionState::Terminated,
        ) => {
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::AimingWithBow;
            }
            return;
        }
        (
            OrderType::TransitionRaisingBow | OrderType::TransitionRaisingBowAnonymous,
            MotionState::Done | MotionState::Terminated,
        ) => {
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::AimingWithBowUp;
            }
            return;
        }
        _ => {}
    }

    if !matches!(motion, MotionState::Start) {
        return;
    }
    let Some(action_state) = (match anim_type {
        OrderType::Provoking => Some(ActionState::WaitingSword),
        OrderType::WaitingShield => Some(ActionState::HoldingShield),
        OrderType::TakingNet => Some(ActionState::Waiting),
        _ => None,
    }) else {
        return;
    };

    entity.set_posture(Posture::Upright);
    if let Some(actor) = entity.actor_data_mut() {
        actor.action_state = action_state;
    }
}

/// PC `Taking` / `TakingCrouched` Done handler — fires when a PC finishes the generic
/// pickup animation for a scroll / bonus / landed projectile.
///
/// Dispatches per `ObjectType` (amulet, purse, coin, relics, scroll,
/// and the default ammo-bonus fallthrough).  The soldier counterpart
/// is handled by `apply_soldier_execute_side_effects`.
///
/// Pushing into `outcomes.pickups` lets the post-tick handler in
/// `tick.rs` run `scroll_is_taken` (for scrolls) or
/// `apply_pc_take_object` (for bonuses/projectiles) with `&mut self`.
fn apply_pc_taking_side_effect(
    entity: &Entity,
    anim_type: OrderType,
    motion: MotionState,
    antagonist: Option<EntityId>,
    entity_id: EntityId,
    outcomes: &mut ExecuteSideOutcomes,
) {
    if matches!(entity, Entity::Pc(_))
        && matches!(anim_type, OrderType::Taking | OrderType::TakingCrouched)
        && matches!(motion, MotionState::Done)
        && let Some(a) = antagonist
    {
        outcomes.pickups.push((entity_id, a));
    }
}

/// Translate an order's `OrderType` (which can be a non-animation
/// dispatch token) into the sprite animation that should actually be
/// played.  Centralises the per-arm "non-animation X → animation Y"
/// mappings, plus the missing-shield-variant fallback cases.
///
/// The order's own `action` field (i.e. `anim_type` at the call site)
/// stays unchanged so side-effect handlers keep matching on the
/// original token — only the value handed to `sprite.perform_action`
/// is substituted.  The `action` field is the dispatch key, while
/// `perform_action` receives an explicit animation argument.
///
/// Source mappings:
///  - `FALLING_HIT_*` / `FALLING_HIT_HARDER_*` / `FALLING_PUSHED_*`
///    → `FALLING_BACK_*` (the falling-hit / falling-pushed dispatch
///    delegates to the `FALLING_BACK_*` sprite animation).
///  - `LOWERING_SHIELD` / `PARRYING_SHIELD` fall back to
///    `TRANSITION_LOWERING_SWORD` / `PARRYING_SWORD` when the sprite
///    lacks the shield variant.
///  - NPC `BEGGAR_SHOWING_FACE` falls back to `ROLLING` when the sprite
///    lacks the showing-face animation.
///  - `TRANSITION_WAITING_SWORD_PARRYING_SWORD_LOW`
///    → `TRANSITION_WAITING_SWORD_PARRYING_SWORD` — the `_LOW`
///    non-animation token re-uses the regular transition sprite anim.
///  - Soldier/NPC `TAKING` → `SEARCHING` (the order remains
///    `TAKING` so pickup side effects still dispatch there).
///  - `WAITING_CAPE_ANONYMOUS_ARCHER` → `WAITING_CAPE`.
///  - `FALLING_LADDER_WALL` → `FALLING_BACK_UPRIGHT`.  The ladder/wall
///    fall is a pure dispatch token with no sprite row of its own; only
///    the flight bookkeeping keys off the original action.
///
/// Identity for everything else (most order types play their own
/// sprite anim).
pub(crate) fn sprite_anim_for_order(
    sprite: &crate::sprite::Sprite,
    effective_anim: OrderType,
    owner_is_pc: bool,
) -> OrderType {
    use OrderType as OT;
    match effective_anim {
        OT::Taking if !owner_is_pc => OT::Searching,
        OT::LoweringShield if !sprite.has_animation(OT::LoweringShield) => {
            OT::TransitionLoweringSword
        }
        OT::ParryingShield if !sprite.has_animation(OT::ParryingShield) => OT::ParryingSword,
        OT::BeggarShowingFace if !sprite.has_animation(OT::BeggarShowingFace) => OT::Rolling,
        OT::FallingHitUpright
        | OT::FallingHitHarderUpright
        | OT::FallingPushedUpright
        | OT::FallingLadderWall => OT::FallingBackUpright,
        OT::FallingHitWithBow | OT::FallingHitHarderWithBow | OT::FallingPushedWithBow => {
            OT::FallingBackBow
        }
        OT::FallingHitWithSword | OT::FallingHitHarderWithSword | OT::FallingPushedWithSword => {
            OT::FallingBackSword
        }
        OT::FallingHitCrouched | OT::FallingHitHarderCrouched | OT::FallingPushedCrouched => {
            OT::FallingBackCrouched
        }
        OT::TransitionWaitingSwordParryingSwordLow => OT::TransitionWaitingSwordParryingSword,
        OT::WaitingCapeAnonymousArcher => OT::WaitingCape,
        other => other,
    }
}

/// Anims whose initialisation lifts the parent sequence element to
/// `NonInterruptable`:
/// - `FALLING_LADDER_WALL`
/// - `ROLLING`
/// - `FALLING_HIT_*`
/// - `FALLING_PUSHED_*`
/// - `FALLING_SHOULDERS` (the priority is set when shoulder damage
///   is translated; we unify it into the start-side-effect set since
///   the runtime assertion would pass either way.)
///
/// These are the anims that unconditionally lift the parent element
/// to non-interruptable so a fresh damage can't preempt the in-flight
/// visual.  Other fall families (`FALLING_BACK_*`, `DYING_*`,
/// `BEING_DEAD_*`) inherit their parent element's priority instead,
/// so they're not in this list.
fn anim_forces_non_interruptable_on_start(anim_type: OrderType) -> bool {
    matches!(
        anim_type,
        OrderType::FallingLadderWall
            | OrderType::Rolling
            | OrderType::FallingHitUpright
            | OrderType::FallingHitWithBow
            | OrderType::FallingHitWithSword
            | OrderType::FallingHitCrouched
            | OrderType::FallingHitHarderUpright
            | OrderType::FallingHitHarderWithBow
            | OrderType::FallingHitHarderWithSword
            | OrderType::FallingHitHarderCrouched
            | OrderType::FallingShoulders
            | OrderType::FallingPushedUpright
            | OrderType::FallingPushedWithBow
            | OrderType::FallingPushedWithSword
            | OrderType::FallingPushedCrouched
    )
}

/// `DYING_SWORD` / `DYING_BOW` / `DYING_UPRIGHT` / `DYING_CROUCHED`
/// on motion Start: set posture to Dead (if already dead) or Lying,
/// then set action_state per family.
fn apply_dying_start_side_effect(entity: &mut Entity, anim_type: OrderType, motion: MotionState) {
    if !matches!(motion, MotionState::Start) {
        return;
    }
    let action = match anim_type {
        OrderType::DyingSword => Some(ActionState::WaitingSword),
        OrderType::DyingBow => Some(ActionState::AimingWithBow),
        OrderType::DyingUpright | OrderType::DyingCrouched => Some(ActionState::Waiting),
        _ => None,
    };
    let action = match action {
        Some(a) => a,
        None => return,
    };
    let posture = if entity.is_dead() {
        Posture::Dead
    } else {
        Posture::Lying
    };
    entity.set_posture(posture);
    if let Some(actor) = entity.actor_data_mut() {
        actor.action_state = action;
    }
}

/// `EXTRACTING_ARROW_UPRIGHT` / `EXTRACTING_ARROW_CROUCHED` /
/// `EXTRACTING_ARROW_BOW` on motion Start restore the same posture and
/// action state as the shared human legacy implementation Execute branch.
fn apply_arrow_extraction_start_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
) {
    if !matches!(motion, MotionState::Start) {
        return;
    }
    let (posture, action) = match anim_type {
        OrderType::ExtractingArrowUpright => (Posture::Upright, ActionState::Waiting),
        OrderType::ExtractingArrowCrouched => (Posture::Crouched, ActionState::Waiting),
        OrderType::ExtractingArrowBow => (Posture::Upright, ActionState::AimingWithBow),
        _ => return,
    };
    entity.set_posture(posture);
    if let Some(actor) = entity.actor_data_mut() {
        actor.action_state = action;
    }
}

/// `STANDING_UP*` on motion Start follows the shared human legacy implementation
/// Execute branch: normal stand-up enters waiting, sword stand-up
/// enters sword waiting, and bow stand-up only restores upright
/// posture while preserving the bow action state.
fn apply_standing_up_start_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
) {
    if !matches!(motion, MotionState::Start) {
        return;
    }
    match anim_type {
        OrderType::StandingUp => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Waiting;
            }
        }
        OrderType::StandingUpSword => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::WaitingSword;
            }
        }
        OrderType::StandingUpBow => {
            entity.set_posture(Posture::Upright);
        }
        _ => {}
    }
}

/// `BEING_CARRIED_LITTLE_JOHN` / `BEING_CARRIED_PEASANT_C` on motion
/// Start enter the carried idle state in the shared human Execute
/// branch.
fn apply_carried_start_side_effect(entity: &mut Entity, anim_type: OrderType, motion: MotionState) {
    if !matches!(
        (anim_type, motion),
        (
            OrderType::BeingCarriedLittleJohn | OrderType::BeingCarriedPeasantC,
            MotionState::Start
        )
    ) {
        return;
    }
    if entity.actor_data().is_none() {
        return;
    }
    entity.set_posture(Posture::Carried);
    if let Some(actor) = entity.actor_data_mut() {
        actor.action_state = ActionState::Waiting;
    }
}

fn endurance_for_smalltalk_recovery(
    entity: &Entity,
    profile_manager: &crate::profiles::ProfileManager,
) -> Option<u16> {
    match entity {
        Entity::Pc(pc) => profile_manager
            .get_character(pc.pc.profile_index)
            .map(|profile| profile.endurance),
        Entity::Soldier(soldier) => profile_manager
            .get_soldier(soldier.soldier.soldier_profile_index)
            .map(|profile| profile.endurance),
        _ => None,
    }
}

/// Smalltalk strike/parry Execute branches set sword-waiting state at
/// animation Start and recover tiredness by `GetEndurance() / 10` at
/// Terminated.
fn apply_smalltalk_start_and_recovery_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
    profile_manager: &crate::profiles::ProfileManager,
) {
    let is_smalltalk = matches!(
        anim_type,
        OrderType::StrikingLeftSmalltalk
            | OrderType::StrikingRightSmalltalk
            | OrderType::StrikingLowLeftSmalltalk
            | OrderType::StrikingLowRightSmalltalk
            | OrderType::ParryingLeftSmalltalk
            | OrderType::ParryingRightSmalltalk
            | OrderType::ParryingLowLeftSmalltalk
            | OrderType::ParryingLowRightSmalltalk
    );
    if !is_smalltalk {
        return;
    }
    match motion {
        MotionState::Start => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::WaitingSword;
            }
        }
        MotionState::Terminated => {
            let Some(endurance) = endurance_for_smalltalk_recovery(entity, profile_manager) else {
                tracing::warn!(
                    ?anim_type,
                    "smalltalk animation terminated but actor profile endurance is unavailable"
                );
                return;
            };
            if let Some(human) = entity.human_data_mut() {
                human.tiredness = human.tiredness.saturating_sub(endurance / 10);
            }
        }
        _ => {}
    }
}

/// STRIKING_DOWN_SWORD refreshes its antagonist-facing goal and sets
/// sword-waiting state at Start, then launches GET_KILLED_AT_BOTTOM on the
/// victim at the action-done tag.
fn apply_striking_down_sword_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
    antagonist: Option<EntityId>,
    antagonist_direction: Option<i16>,
    entity_id: EntityId,
    outcomes: &mut ExecuteSideOutcomes,
) {
    if anim_type != OrderType::StrikingDownSword {
        return;
    }
    match motion {
        MotionState::Start => {
            entity.set_posture(Posture::Upright);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::WaitingSword;
            }
            let direction = antagonist_direction.unwrap_or_else(|| {
                panic!(
                    "actor {entity_id:?} StrikingDownSword started without an antagonist direction"
                )
            });
            entity.position_iface_mut().set_direction(
                crate::position_interface::Direction::from_raw(i32::from(direction)),
            );
        }
        MotionState::Done => {
            let Some(target) = antagonist else {
                tracing::warn!(
                    ?entity_id,
                    "StrikingDownSword reached action-done without an antagonist"
                );
                return;
            };
            outcomes.killed_at_bottom.push((target, entity_id));
        }
        _ => {}
    }
}

/// `BEING_DEAD_*` on motion Start: set states to (Dead, ...).
/// The dispatch returns InProgress always (never Done/Terminated), so
/// the active_ai_anim teardown never fires for these anims and the
/// corpse loops the idle sprite forever.
fn apply_being_dead_start_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
) {
    if !matches!(motion, MotionState::Start) {
        return;
    }
    let action = match anim_type {
        OrderType::BeingDeadSword => Some(ActionState::WaitingSword),
        OrderType::BeingDeadBow => Some(ActionState::AimingWithBow),
        OrderType::BeingDead => Some(ActionState::Waiting),
        OrderType::BeingDeadFallenBackSword => Some(ActionState::WaitingSword),
        OrderType::BeingDeadFallenBackBow => Some(ActionState::AimingWithBow),
        OrderType::BeingDeadFallenBack => Some(ActionState::Waiting),
        _ => None,
    };
    let action = match action {
        Some(a) => a,
        None => return,
    };
    let posture = match anim_type {
        OrderType::BeingDeadFallenBackSword
        | OrderType::BeingDeadFallenBackBow
        | OrderType::BeingDeadFallenBack => Posture::DeadBack,
        _ => Posture::Dead,
    };
    entity.set_posture(posture);
    if let Some(actor) = entity.actor_data_mut() {
        actor.action_state = action;
    }
}

/// Pick the landing `(posture, optional action_state)` for a fall
/// animation that just hit its terminal motion event.  Timing differs
/// by family — see the `fall_motion_for_completion` helper for which
/// `MotionState` triggers the set.
///
/// | Anim | Trigger | Posture | ActionState |
/// |------|---------|---------|-------------|
/// | `FALLING_HIT_*` | TERMINATED | DeadBack/Lying | by weapon/posture variant |
/// | `FALLING_PUSHED_*` | TERMINATED | DeadBack/Lying | by weapon/posture variant |
/// | `FALLING_SHOULDERS` | TERMINATED | DeadBack/Lying | (unchanged) |
/// | `FALLING_BACK_UPRIGHT`/`CROUCHED` | START | DeadBack/Lying | Waiting |
/// | `FALLING_BACK_SWORD` | DONE\|TERMINATED | DeadBack/Lying | WaitingSword |
/// | `FALLING_BACK_BOW` | DONE\|TERMINATED | DeadBack/Lying | AimingWithBow |
/// | `ROLLING` | TERMINATED | Dead/Lying | (unchanged) |
///
/// `FALLING_LADDER_WALL` is absent: its landing states come from the
/// ladder-fall arrival in the flight tick, not from a sprite event.
fn fall_landing_states(
    anim_type: OrderType,
    is_dead: bool,
    _is_unconscious: bool,
) -> Option<(Posture, Option<ActionState>)> {
    let lying_or_dead_back = if is_dead {
        Posture::DeadBack
    } else {
        Posture::Lying
    };
    match anim_type {
        OrderType::FallingHitUpright
        | OrderType::FallingHitHarderUpright
        | OrderType::FallingHitCrouched
        | OrderType::FallingHitHarderCrouched
        | OrderType::FallingPushedUpright
        | OrderType::FallingPushedCrouched => {
            Some((lying_or_dead_back, Some(ActionState::Waiting)))
        }
        OrderType::FallingHitWithBow
        | OrderType::FallingHitHarderWithBow
        | OrderType::FallingPushedWithBow => {
            Some((lying_or_dead_back, Some(ActionState::AimingWithBow)))
        }
        OrderType::FallingHitWithSword
        | OrderType::FallingHitHarderWithSword
        | OrderType::FallingPushedWithSword => {
            Some((lying_or_dead_back, Some(ActionState::WaitingSword)))
        }
        OrderType::FallingShoulders => Some((lying_or_dead_back, None)),
        OrderType::FallingBackUpright | OrderType::FallingBackCrouched => {
            Some((lying_or_dead_back, Some(ActionState::Waiting)))
        }
        OrderType::FallingBackSword => Some((lying_or_dead_back, Some(ActionState::WaitingSword))),
        OrderType::FallingBackBow => Some((lying_or_dead_back, Some(ActionState::AimingWithBow))),
        OrderType::Rolling => Some((
            if is_dead {
                Posture::Dead
            } else {
                Posture::Lying
            },
            None,
        )),
        // FALLING_LADDER_WALL sets no landing state from its sprite
        // event: the ladder-fall arrival (tick countdown reaching zero
        // in the flight tick) applies the lying/dead-back posture, and
        // a sprite that runs out before the countdown leaves the
        // flying posture in place.
        _ => None,
    }
}

/// Which `MotionState` triggers `fall_landing_states` for `anim_type`?
/// State is set at varying points per family:
/// - `FALLING_BACK_UPRIGHT`/`CROUCHED`: Start.
/// - `FALLING_BACK_SWORD`/`BOW`: Done or Terminated.
/// - `FALLING_HIT_HARDER_*`: Done or Terminated.
/// - non-hard `FALLING_HIT_*` / `FALLING_SHOULDERS` / `ROLLING` /
///   `FALLING_LADDER_WALL`: Terminated.
fn fall_state_trigger_matches(anim_type: OrderType, motion: MotionState) -> bool {
    match anim_type {
        OrderType::FallingBackUpright | OrderType::FallingBackCrouched => {
            matches!(motion, MotionState::Start)
        }
        OrderType::FallingBackSword | OrderType::FallingBackBow => {
            matches!(motion, MotionState::Done | MotionState::Terminated)
        }
        OrderType::FallingHitHarderUpright
        | OrderType::FallingHitHarderWithBow
        | OrderType::FallingHitHarderWithSword
        | OrderType::FallingHitHarderCrouched => {
            matches!(motion, MotionState::Done | MotionState::Terminated)
        }
        // Non-hard FallingHit sets only on TERMINATED, not DONE.
        // Rolling, FallingLadderWall, and FallingShoulders do the same.
        _ => matches!(motion, MotionState::Terminated),
    }
}

/// Orders dispatched through `RHSprite::PerformFlight` rather than plain
/// `PerformAction` or `PerformMotion`.
///
/// The four harder-hit variants deliberately do not belong here: Original's
/// `ExecuteFallingHit(..., true)` uses `PerformAction`. `FallingLadderWall`
/// also uses `PerformAction` plus its own wait-time movement loop.
fn uses_perform_flight(anim_type: OrderType) -> bool {
    matches!(
        anim_type,
        OrderType::FallingHitUpright
            | OrderType::FallingHitWithBow
            | OrderType::FallingHitWithSword
            | OrderType::FallingHitCrouched
            | OrderType::FallingPushedUpright
            | OrderType::FallingPushedWithBow
            | OrderType::FallingPushedWithSword
            | OrderType::FallingPushedCrouched
    )
}

/// Non-hard `FALLING_HIT_*` / `FALLING_PUSHED_*` on motion Start.
///
/// legacy implementation `ExecuteFallingHit` enters `(Flying, Moving)`; legacy implementation
/// `ExecuteFallingPushed` enters `(Flying, WaitingSword)` before the
/// wrapper later restores the variant-specific action on termination.
/// The ladder/wall fall enters `(Flying, Moving)` on the same event, so it
/// rides along with the falling-hit group.
/// The `FALLING_HIT_HARDER_*` branch uses `PerformAction`, not flight,
/// and deliberately keeps the current posture/action until landing.
/// Other fall families set state on later motion events — handled by
/// `apply_falling_completion_side_effect`.
fn apply_falling_start_side_effect(entity: &mut Entity, anim_type: OrderType, motion: MotionState) {
    if !matches!(motion, MotionState::Start) {
        return;
    }
    let action_state = if matches!(
        anim_type,
        OrderType::FallingHitUpright
            | OrderType::FallingHitWithBow
            | OrderType::FallingHitWithSword
            | OrderType::FallingHitCrouched
            | OrderType::FallingLadderWall
    ) {
        Some(ActionState::Moving)
    } else if matches!(
        anim_type,
        OrderType::FallingPushedUpright
            | OrderType::FallingPushedWithBow
            | OrderType::FallingPushedWithSword
            | OrderType::FallingPushedCrouched
    ) {
        Some(ActionState::WaitingSword)
    } else {
        None
    };
    let Some(action_state) = action_state else {
        return;
    };
    if uses_perform_flight(anim_type) {
        // RHSprite::PerformFlight disables anti-collision on START. Besides
        // suppressing collision checks this synchronously clears the stale
        // deviation latch used by TurnAntiVibration.
        entity.position_iface_mut().set_anti_collision_on(false);
    }
    entity.set_posture(Posture::Flying);
    if let Some(actor) = entity.actor_data_mut() {
        actor.action_state = action_state;
    }
}

/// Falling-hit / shoulder-fall / falling-back / ladder-wall completion
/// for the active_ai_anim path.  Rolling is intentionally NOT covered
/// here — that's Phase 5 and still flows through the `combat_anim`
/// block.
fn apply_falling_completion_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
) {
    if anim_type == OrderType::Rolling {
        return;
    }
    if !fall_state_trigger_matches(anim_type, motion) {
        return;
    }
    if motion == MotionState::Terminated && uses_perform_flight(anim_type) {
        // RHSprite::PerformFlight restores anti-collision before the wrapper
        // applies its landing posture/action state.
        entity.position_iface_mut().set_anti_collision_on(true);
    }
    let is_dead = entity.is_dead();
    let is_unconscious = entity.human_data().map(|h| h.unconscious).unwrap_or(false);
    if let Some((posture, action_state)) = fall_landing_states(anim_type, is_dead, is_unconscious) {
        entity.set_posture(posture);
        let hard_hit_done = motion == MotionState::Done
            && matches!(
                anim_type,
                OrderType::FallingHitHarderUpright
                    | OrderType::FallingHitHarderWithBow
                    | OrderType::FallingHitHarderWithSword
                    | OrderType::FallingHitHarderCrouched
            );
        // ExecuteFallingHit(..., true) lands on DONE, but the outer
        // FALLING_HIT_HARDER_* wrapper restores its action family only
        // when the sprite later reports TERMINATED.
        if !hard_hit_done
            && let Some(action) = action_state
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.action_state = action;
        }
    }
}

/// Soldier combat-injury anims (`BEING_HIT_SWORD`,
/// `EXTRACTING_ARROW_SWORD`, `BEING_WEAK_SWORD`, `BEING_STUNNED_SWORD`,
/// `STANDING_UP_SWORD`)
/// dispatch `EventAfterCombatInjury` to the AI when they terminate so
/// the soldier can resume the fight.  Pushes onto the caller-owned
/// `combat_injury_terminated` list (which the post-tick loop in
/// `tick.rs` drains with `&mut self`).
fn apply_combat_injury_side_effect(
    entity: &Entity,
    anim_type: OrderType,
    motion: MotionState,
    entity_id: EntityId,
    combat_injury_terminated: &mut Vec<EntityId>,
) {
    if matches!(motion, MotionState::Terminated)
        && matches!(
            anim_type,
            OrderType::BeingHitSword
                | OrderType::ExtractingArrowSword
                | OrderType::BeingWeakSword
                | OrderType::BeingStunnedSword
                | OrderType::StandingUpSword
        )
        && matches!(entity, Entity::Soldier(_))
    {
        combat_injury_terminated.push(entity_id);
    }
}

/// `BEING_WEAK_SWORD` reduces tiredness every tick and, once the
/// action-done frame has been reached, keeps returning `InProgress`
/// until tiredness reaches zero.
fn apply_weak_sword_tiredness_after_perform(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: &mut MotionState,
) {
    if anim_type != OrderType::BeingWeakSword {
        return;
    }
    let Some(human) = entity.human_data_mut() else {
        return;
    };

    human.tiredness = human.tiredness.saturating_sub(WEAKNESS_DISMISH);
    if human.tiredness != 0 && matches!(*motion, MotionState::Done | MotionState::Terminated) {
        *motion = MotionState::InProgress;
    }
}

fn sprite_is_at_action_done(sprite: &crate::sprite::Sprite) -> bool {
    sprite.current_frame == sprite.action_done_frame
        && sprite.frame_count == sprite.action_done_counter
}

fn hold_weak_sword_at_action_done(
    entity: &mut Entity,
    anim_type: OrderType,
) -> Option<MotionState> {
    if anim_type != OrderType::BeingWeakSword {
        return None;
    }
    if !sprite_is_at_action_done(&entity.element_data().sprite) {
        return None;
    }
    let human = entity.human_data_mut()?;
    human.tiredness = human.tiredness.saturating_sub(WEAKNESS_DISMISH);
    if human.tiredness == 0 {
        None
    } else {
        Some(MotionState::InProgress)
    }
}

/// Shield transition completion: set the post-animation `(posture, action_state)`
/// when a shield raise / lower / parry one-shot finishes.  Each
/// arm sets states fully so posture is also coerced to UPRIGHT.
///
/// - `RAISING_SHIELD`  on `MOTION_DONE`       → `(Upright, HoldingShield)`
/// - `LOWERING_SHIELD` on `MOTION_DONE`       → `(Upright, Waiting)`
/// - `PARRYING_SHIELD` on `MOTION_DONE`       → `(Upright, ParryingShield)`
/// - `PARRYING_SHIELD` on `MOTION_TERMINATED` → `(Upright, HoldingShield)`
fn apply_shield_transition_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
) {
    let new_state = match (anim_type, motion) {
        (OrderType::RaisingShield, MotionState::Done | MotionState::Terminated) => {
            Some(ActionState::HoldingShield)
        }
        (OrderType::LoweringShield, MotionState::Done | MotionState::Terminated) => {
            Some(ActionState::Waiting)
        }
        // DONE → ParryingShield: the parry animation running to completion
        // is what puts the actor into the parrying state.
        (OrderType::ParryingShield, MotionState::Done) => Some(ActionState::ParryingShield),
        (OrderType::ParryingShield, MotionState::Terminated) => Some(ActionState::HoldingShield),
        _ => None,
    };
    if let Some(state) = new_state {
        entity.set_posture(Posture::Upright);
        if let Some(actor) = entity.actor_data_mut() {
            actor.action_state = state;
        }
    }
}

/// PC cape/tree disguise exit completion mirrors the PC legacy implementation execute
/// arms: on DONE the actor becomes Upright/Waiting and the Hidden
/// titbit is removed by the engine-side drain.
fn apply_pc_disguise_exit_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
    entity_id: EntityId,
    side_outcomes: &mut ExecuteSideOutcomes,
) {
    if !entity.is_pc() || !matches!(motion, MotionState::Done) {
        return;
    }
    if !matches!(
        anim_type,
        OrderType::TransitionWaitingCapeWaitingUpright
            | OrderType::TransitionWaitingHiddenWaitingUpright
    ) {
        return;
    }
    entity.set_posture(Posture::Upright);
    if let Some(actor) = entity.actor_data_mut() {
        actor.action_state = ActionState::Waiting;
    }
    side_outcomes.hidden_titbit_removals.push(entity_id);
}

/// Sword parry state transitions mirror the legacy implementation execute arms:
/// waiting-to-parry transition seeds normal parry on termination, low
/// parry enters its low state on start and computes its hold counter
/// relative to the opponent's current action-done timing, and
/// parry-to-waiting / low-parry completion return to WaitingSword.
fn apply_sword_parry_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
    principal_frames_from_now: Option<i16>,
) {
    let new_state = match (anim_type, motion) {
        (OrderType::TransitionWaitingSwordParryingSword, MotionState::Terminated) => {
            if let Some(human) = entity.human_data_mut() {
                human.parry_counter = crate::engine::melee::TIME_TO_STAY_IN_PARRY_MODE;
            }
            Some(ActionState::ParryingSword)
        }
        (OrderType::TransitionWaitingSwordParryingSwordLow, MotionState::Start) => {
            Some(ActionState::ParryingSwordLow)
        }
        (OrderType::TransitionWaitingSwordParryingSwordLow, MotionState::Terminated) => {
            let counter = if entity
                .human_data()
                .map(|h| !h.opponents.is_empty())
                .unwrap_or(false)
            {
                let own_start = entity
                    .element_data()
                    .sprite
                    .frames_from_start_till_action_done(OrderType::ParryingLowSword)
                    as i16;
                principal_frames_from_now.unwrap_or(1) - own_start
            } else {
                crate::engine::melee::TIME_TO_STAY_IN_PARRY_MODE as i16
            };
            if let Some(human) = entity.human_data_mut() {
                human.parry_counter = counter.max(1) as u16;
            }
            None
        }
        (OrderType::ParryingSword, MotionState::Start) => Some(ActionState::ParryingSword),
        (OrderType::ParryingLowSword, MotionState::Start) => Some(ActionState::ParryingSwordLow),
        (
            OrderType::TransitionParryingSwordWaitingSword | OrderType::ParryingLowSword,
            MotionState::Terminated,
        ) => Some(ActionState::WaitingSword),
        _ => None,
    };
    if let Some(state) = new_state {
        entity.set_posture(Posture::Upright);
        if let Some(actor) = entity.actor_data_mut() {
            actor.action_state = state;
        }
    }
}

fn apply_under_net_cycle_side_effect(
    sim: &crate::sim_rng::SimulationContext,

    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
) {
    use crate::ai::EmoticonType;
    use crate::order::OrderType as OT;
    use crate::sprite::MotionState as MS;

    if !matches!(
        (anim_type, motion),
        (OT::LyingStuckUnderNet | OT::WriggleUnderNet, MS::Start)
            | (OT::WriggleUnderNet, MS::Terminated)
    ) {
        return;
    }

    if matches!(motion, MS::Start) {
        let is_unconscious = entity.human_data().map(|h| h.unconscious).unwrap_or(false);
        let is_tied = entity.element_data().posture == Posture::Tied;
        if !entity.is_dead() && !is_unconscious && !is_tied {
            entity.set_posture(Posture::StuckUnderNet);
        }
        if let Some(actor) = entity.actor_data_mut() {
            actor.action_state = ActionState::Waiting;
        }
    }

    match (anim_type, motion) {
        (OT::WriggleUnderNet, MS::Start) => {
            match crate::sim_rng::u32(sim, crate::sim_rng::RngSite::WriggleDirection, ..3) {
                0 => {
                    let direction = (entity.element_data().direction() + 1) & 15;
                    entity.element_data_mut().set_direction_instantly(direction);
                }
                1 => {
                    let direction = (entity.element_data().direction() + 15) & 15;
                    entity.element_data_mut().set_direction_instantly(direction);
                }
                _ => {}
            }

            if matches!(entity, Entity::Soldier(_)) {
                if let Some(ai) = entity.ai_controller_mut() {
                    ai.set_emoticon(EmoticonType::Thunderstorm);
                } else {
                    // TODO(net-parity): level-loaded soldiers should always have an AI controller.
                    tracing::warn!(
                        "WriggleUnderNet soldier has no AI controller for thunderstorm emoticon"
                    );
                }
            }
        }
        (OT::WriggleUnderNet, MS::Terminated) => {
            if matches!(entity, Entity::Soldier(_)) {
                if let Some(ai) = entity.ai_controller_mut() {
                    ai.clear_emoticon();
                } else {
                    // TODO(net-parity): level-loaded soldiers should always have an AI controller.
                    tracing::warn!(
                        "WriggleUnderNet soldier has no AI controller to clear emoticon"
                    );
                }
            }
        }
        _ => {}
    }
}

fn safe_frames_from_now_till_action_done(sprite: &crate::sprite::Sprite) -> Option<i16> {
    let script = sprite.scripts.get(sprite.current_row as usize)?;
    let max_frame = sprite.current_frame.max(sprite.action_done_frame) as usize;
    if max_frame >= script.delays.len() {
        return None;
    }
    Some(sprite.frames_from_now_till_action_done())
}

/// Per-arm dispatch result.  Each arm of the per-anim switch returns
/// a motion state that the Hourglass dispatcher then acts on.
///
/// Semantics:
/// - `Forward(TERMINATED)` → advance the sequence element.
/// - `Forward(ABORTED)`    → set the sequence element to Impossible.
/// - `Forward(DONE)`       → no-op (the `bDone` flag was never
///   actually read in the original engine).
/// - `Forward(START/IN_PROGRESS)` → no-op.
/// - `Consumed`            → arm short-circuited the Hourglass
///   dispatch entirely (loop/idle/corpse anims + BORED in-place
///   mutation).
#[derive(Debug, Clone, Copy)]
enum ExecuteOutcome {
    /// Arm consumed the event.  Equivalent to "return InProgress" —
    /// the arm has handled the motion state internally, and the
    /// Hourglass must not advance or terminate the element.
    Consumed,
    /// Forward the motion state to `Hourglass`-style dispatch.
    /// Only TERMINATED (advance) and ABORTED (impossible) have
    /// observable effects; DONE / START / InProgress are no-ops.
    Forward(MotionState),
}

/// Animation arms that always return `InProgress` from their per-anim
/// dispatch — loop/idle/corpse/immobilization arms.  They cycle
/// forever (or until external interruption) and must never advance
/// the owning sequence element even when the sprite reaches
/// TERMINATED.  This matches every always-IN_PROGRESS arm across the
/// five dispatch switches (base actor + Human + NPC + PC + Soldier
/// subclass overrides).
///
/// Arms *not* in this list take the default "forward motion" path —
/// TERMINATED advances, everything else no-ops.  A few arms with
/// conditional IN_PROGRESS branches (e.g. `GETTING_FREE_FROM_WASP`
/// which returns IN_PROGRESS only while still turning, otherwise
/// returns the sprite's motion state) are *not* in this list because
/// their `OrderCompletion` side-channel (e.g. `WaspStruggleCycle`)
/// needs the TERMINATED advance to fire.
fn arm_is_always_consumed(anim_type: OrderType) -> bool {
    use OrderType as OT;
    matches!(
        anim_type,
        // Idle loops whose derived Execute arms explicitly return
        // InProgress. Plain WaitingUpright is intentionally absent:
        // the base actor Execute forwards PerformAction's raw motion so a
        // Wait transition chain can advance when that animation terminates.
        // The PC cape/hidden idle loops are absent for the same reason: their
        // arms play a cyclic PerformAction and return its raw result, so the
        // first tick of a freshly adopted order must still report Start.
        OT::WaitingCrouched
            | OT::WaitingAlerted
            | OT::WaitingSword
            | OT::WaitingShield
            | OT::WaitingOnShoulders
            | OT::WaitingHelpingClimbing
            | OT::WaitingCarryingOnShoulders
            | OT::WaitingWithCorpse
            | OT::WaitingWithPurse
            // Aim loops
            | OT::AimingWithBow
            | OT::AimingWithBowUp
            | OT::AimingWithBowLeaningOut
            | OT::AimingWithBowAnonymous
            | OT::AimingWithBowUpAnonymous
            // Activity loops
            | OT::Sitting
            | OT::Listening
            | OT::Menacing
            | OT::SleepingUpright
            | OT::LeaningOut
            | OT::SimulatingBeggar
            // Parry holds are timer-controlled in the soldier execute
            // arm.  Their sprite may terminate earlier, but legacy implementation keeps
            // returning IN_PROGRESS until `muwParryCounter` expires.
            | OT::ParryingSword
            | OT::ParryingLowSword
            // Corpse / KO loops (freeze-when-terminated progression)
            | OT::BeingDead
            | OT::BeingDeadFallenBack
            | OT::BeingDeadSword
            | OT::BeingDeadBow
            | OT::BeingDeadFallenBackSword
            | OT::BeingDeadFallenBackBow
            // Immobilization / tied
            | OT::WriggleUnderNet
            | OT::BeingTied
            // Non-animation holds
            | OT::Freezing
            | OT::PlayCustomFrozen
            | OT::RefreshingSeek
            // HIDING_BEHIND_SHIELD (PC): always IN_PROGRESS except for
            // a sequence-validity early-out.  The validity check
            // isn't modelled in the dispatcher; mark Consumed — if
            // the shield-holder becomes invalid, the sequence system
            // terminates the element via its own cascade.
            | OT::HidingBehindShield
    )
}

/// Per-arm completion dispatch.  Each arm decides whether to short
/// circuit (Consumed) or forward its motion state to `Hourglass`
/// semantics (Forward).
///
/// Arms covered by explicit in-place mutation:
/// - **`WAITING_UPRIGHT_BORED` / `WAITING_UPRIGHT_BORED_RANDOM`**:
///   Terminated rerolls the animation type + NewID (BORED ↔ RANDOM
///   with 1/10 bias).
/// - **`LYING_STUCK_UNDER_NET`**: Terminated rolls a 1/31 chance to
///   mutate to `WRIGGLE_UNDER_NET` + NewID.
///
/// Everything else: Consumed if in [`arm_is_always_consumed`],
/// otherwise Forward(motion) — the Hourglass dispatcher in the
/// teardown block decides per motion state.
/// Context passed to [`dispatch_arm_completion`].  Bundles the
/// entity / sequence / RNG handles the dispatcher needs so the
/// function signature stays small even as more arms are ported.
struct ArmCtx<'a> {
    entity_id: EntityId,
    is_npc: bool,
    is_unconscious: bool,
    seq_id: crate::sequence::SequenceId,
    elem_idx: usize,
    sequence_manager: &'a mut crate::sequence::SequenceManager,
    next_order_id: &'a mut u32,
    side_outcomes: &'a mut ExecuteSideOutcomes,
}

fn dispatch_arm_completion(
    sim: &crate::sim_rng::SimulationContext,

    anim_type: OrderType,
    motion: MotionState,
    ctx: &mut ArmCtx<'_>,
) -> ExecuteOutcome {
    use crate::order::OrderType as OT;
    use crate::sprite::MotionState as MS;

    // BORED ↔ RANDOM idle cycle — always Consumed. Terminated rerolls
    // the animation type + NewID for every owning command except
    // `WaitTimer`, exactly matching the base Actor Execute guard.
    if matches!(
        anim_type,
        OT::WaitingUprightBored | OT::WaitingUprightBoredRandom
    ) {
        // RHElementActorCivilian::Execute returns the raw coerced
        // PerformAction result for this family, bypassing all base-Actor
        // bored-loop mutation and return-value handling.
        if matches!(ctx.entity_id, EntityId::Civilian(_)) {
            return ExecuteOutcome::Forward(motion);
        }
        if anim_type == OT::WaitingUprightBored && motion == MS::Start {
            // Base Actor preserves the first-entry edge for the ordinary
            // bored loop after applying its Bored action state. Every other
            // result, and every BoredRandom result, is consumed as
            // InProgress.
            return ExecuteOutcome::Forward(MS::Start);
        }
        if matches!(motion, MS::Terminated) {
            let is_wait_timer = ctx
                .sequence_manager
                .get_element(ctx.seq_id, ctx.elem_idx)
                .map(|el| matches!(el.command, crate::element::Command::WaitTimer))
                .unwrap_or_else(|| {
                    panic!(
                        "bored animation owner {:?} lost sequence element {:?}/{}",
                        ctx.entity_id, ctx.seq_id, ctx.elem_idx
                    )
                });
            if !is_wait_timer {
                let next_type = match anim_type {
                    OT::WaitingUprightBored => {
                        tracing::debug!(
                            target: "parity_rng_owner",
                            owner = ?ctx.entity_id,
                            site = ?crate::sim_rng::RngSite::BoredAnimationChoice,
                            "authoritative RNG draw owner"
                        );
                        (crate::sim_rng::u32(
                            sim,
                            crate::sim_rng::RngSite::BoredAnimationChoice,
                            ..10,
                        ) == 0)
                            .then_some(OT::WaitingUprightBoredRandom)
                    }
                    OT::WaitingUprightBoredRandom => Some(OT::WaitingUprightBored),
                    _ => unreachable!(),
                };
                if let Some(next_type) = next_type
                    && let Some(elem) = ctx
                        .sequence_manager
                        .get_element_mut(ctx.seq_id, ctx.elem_idx)
                    && let Some(front) = elem.orders.front_mut()
                {
                    front.order_type = next_type;
                    front.order_id = crate::order::alloc_order_id(ctx.next_order_id);
                }
            }
        }
        return ExecuteOutcome::Consumed;
    }

    // LYING_STUCK_UNDER_NET: every tick, 1/31 chance to mutate to
    // WRIGGLE_UNDER_NET + NewID.  The sprite plays `WRIGGLE_UNDER_NET`
    // frozen on the first frame regardless; the cycle flip is what
    // makes the actor occasionally play the struggle animation.  Roll
    // on every motion state (the roll runs before the motion-state
    // switch), then always return InProgress.
    if matches!(anim_type, OT::LyingStuckUnderNet) {
        if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::NetWriggleGate, ..31) == 0 {
            if let Some(elem) = ctx
                .sequence_manager
                .get_element_mut(ctx.seq_id, ctx.elem_idx)
                && let Some(front) = elem.orders.front_mut()
            {
                front.order_type = OT::WriggleUnderNet;
                front.order_id = crate::order::alloc_order_id(ctx.next_order_id);
            }
            // Cry for help: NPCs (soldier or civilian) say
            // REMARK_UNDER_NET / CIV_REMARK_UNDER_NET and emit a
            // HEEELP noise at their position.  The remark variant is
            // picked at post-tick time based on entity subclass.
            if ctx.is_npc {
                ctx.side_outcomes.cry_for_help_under_net.push(ctx.entity_id);
            }
        }
        return ExecuteOutcome::Consumed;
    }

    // WRIGGLE_UNDER_NET: on sprite termination the current order
    // mutates back to the frozen lying-under-net hold with a fresh id,
    // while the sequence still sees InProgress.
    if matches!(anim_type, OT::WriggleUnderNet) {
        if matches!(motion, MS::Terminated)
            && let Some(elem) = ctx
                .sequence_manager
                .get_element_mut(ctx.seq_id, ctx.elem_idx)
            && let Some(front) = elem.orders.front_mut()
        {
            front.order_type = OT::LyingStuckUnderNet;
            front.order_id = crate::order::alloc_order_id(ctx.next_order_id);
        }
        return ExecuteOutcome::Consumed;
    }

    if matches!(
        anim_type,
        OT::BeingUnconscious | OT::BeingUnconsciousSword | OT::BeingUnconsciousBow
    ) {
        return if ctx.is_unconscious {
            ExecuteOutcome::Consumed
        } else {
            ExecuteOutcome::Forward(MS::Terminated)
        };
    }

    // Loop/idle/corpse arms that always return InProgress.
    if arm_is_always_consumed(anim_type) {
        return ExecuteOutcome::Consumed;
    }

    // Default: forward to `Hourglass` dispatch.  TERMINATED advances;
    // ABORTED sets sequence IMPOSSIBLE; DONE / START / InProgress are
    // no-ops.
    ExecuteOutcome::Forward(motion)
}

/// Side-effects collected by `tick_actor_animation_for` when an order's
/// animation completes.  Matches the non-default variants of
/// [`OrderCompletion`](crate::order::OrderCompletion), plus a generic
/// "advance the owning element via `do_next_order`" bucket and the
/// cross-entity effects fired via the order's antagonist
/// (bottle-hide on DRINKING_ALE, coin pickup on TAKING, etc.).  The
/// caller processes these post-tick because they require `&mut self`
/// on the engine (sequence manager, door table, element removal,
/// speech manager, etc.).
#[derive(Debug, Clone, Default)]
pub(super) struct AnimCompletionOutcomes {
    /// Sequence elements whose priority must become non-interruptable as soon
    /// as their actor's animation enters `Start`.
    pub non_interruptable_lifts: Vec<(crate::sequence::SequenceId, usize)>,
    /// Default path: `do_next_order` pops the just-completed front order
    /// and advances the owning element (or terminates + ensures a wait
    /// element when the queue is empty).
    pub seq_advance: Vec<(crate::sequence::SequenceId, usize)>,
    /// Wasp-last-cycle termination: terminate the element so wasp-victim
    /// cleanup + EVENT_WASP_AWAY fire.
    pub seq_terminate: Vec<(crate::sequence::SequenceId, usize)>,
    /// Sequence elements whose driving animation returned `Aborted`.
    /// The Hourglass dispatch maps Aborted to `Impossible` for the
    /// owning element.
    pub seq_impossible: Vec<(crate::sequence::SequenceId, usize)>,
    /// Wasp struggle-cycle refill: `(seq_id, elem_idx, cycles_remaining)`
    /// — push a fresh `GettingFreeFromWasp` order with the decremented
    /// counter, then advance the element (popping the just-completed
    /// cycle).
    pub wasp_next_cycle: Vec<(crate::sequence::SequenceId, usize, u16)>,
    /// Doors whose lockpick animation reached `MOTION_DONE` this owner slot.
    /// Original clears every live lock/authorization flag at the action point;
    /// order advancement remains tied to the later `MOTION_TERMINATED` edge.
    pub unlock_door_done: Vec<crate::gate::DoorIndex>,
    /// Entities whose door-pass chain should continue.
    pub resume_door_pass: Vec<EntityId>,
    /// PC/Human SELECT action points whose hulk flash starts in this owner
    /// slot. The stored float is the authored door-pass speed.
    pub select_hulk: Vec<(EntityId, f32)>,
    /// Entities whose active jump should advance to the next step.
    pub next_jump_step: Vec<EntityId>,
    /// C++ `PLAY_CUSTOM_FREEZE` launches a follow-up
    /// `PLAY_ANIM_FROZEN` sequence when the animation terminates, so
    /// the actor holds the last frame as a separate in-progress
    /// element.
    pub play_anim_frozen: Vec<(EntityId, u16, OrderType)>,
    /// PCs whose carried-corpse exit transition reached TERMINATED. Original
    /// calls `DropCorpse` inside that exact Execute arm, before Actor::Hourglass
    /// advances to the following order.
    pub corpse_drop_done: Vec<EntityId>,
    /// Soldier-style side effects that touch other entities —
    /// accumulated from `apply_soldier_execute_side_effects`.
    pub execute_sides: ExecuteSideOutcomes,
}

/// The base-Actor completion control that must remain unresolved until every
/// synchronous callback inside the derived Execute arm has drained.
/// TERMINATED targets the owner's then-live sequence element; ABORTED retains
/// the element snapshot taken on entry to Execute.
#[derive(Debug, Clone)]
pub(super) struct ActorExecuteResult {
    pub order_type: OrderType,
    pub entry_seq_id: crate::sequence::SequenceId,
    pub entry_elem_idx: usize,
    pub motion: MotionState,
}

impl EngineInner {
    pub(super) fn finish_patch_transition_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::types::LevelAssets,
        patch_idx: crate::patch::PatchIndex,
    ) {
        let effects = {
            let patch = self
                .script_domains
                .interactables
                .patches
                .get_mut(usize::from(patch_idx))
                .unwrap_or_else(|| {
                    panic!("completed patch animation references missing patch {patch_idx}")
                });
            patch.in_transition = false;
            patch.apply_final(false)
        };

        for door in self.script_domains.interactables.doors.iter_mut() {
            if door.patch_index == Some(patch_idx) {
                door.gate_state.finish_transition();
                tracing::debug!(
                    %patch_idx,
                    new_state = ?door.gate_state,
                    "gate_state advanced on patch transition complete"
                );
            }
        }

        tracing::debug!(
            %patch_idx,
            num_effects = effects.len(),
            "Patch transition animation completed → ApplyFinal"
        );
        self.process_patch_effects(sim, assets, patch_idx, effects);
    }

    /// Run the existing generic actor animation/`Execute` dispatch for one
    /// live legacy creation slot.
    ///
    /// Eligibility remains deliberately narrower than actor `Hourglass`:
    /// movement, melee, and bow actors keep their existing owners. Inactive
    /// actors still execute: Original `RHEngine::Hourglass` visits every
    /// element without an `IsActive()` gate, and actor idles keep advancing
    /// while the actor is hidden from the active world.
    /// Per-actor execution-frozen actors remain skipped unless an installed
    /// WAIT_TIMER/WAIT_FREE_LIFT needs the Original's post-Execute check. A
    /// global FrozenAll does not skip Execute: it suppresses the selected
    /// sprite call while preserving pre/post-sprite arm work. The caller must
    /// still run `ActionChange` for skipped actors.
    pub(super) fn tick_actor_animation_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::types::LevelAssets,
        entity_id: EntityId,
    ) -> (
        Vec<EntityId>,
        AnimCompletionOutcomes,
        Option<ActorExecuteResult>,
    ) {
        let globally_frozen = self.actors_frozen();

        // Production enters through the owner coordinator, which stamps this
        // before choosing a specialized arm. Keep this helper self-contained
        // for focused callers while preserving the same actor-level identity.
        if let Some((_, _, order)) = self
            .orders
            .sequence_manager
            .current_order_for_actor(entity_id)
        {
            let actor = self
                .world
                .entities
                .get_mut(entity_id)
                .and_then(Entity::actor_data_mut)
                .unwrap_or_else(|| panic!("generic Execute owner {entity_id:?} lost actor data"));
            if actor.last_execute_order_id != Some(order.order_id) {
                actor.last_execute_order_id = Some(order.order_id);
                actor.execute_order_initialising = true;
            }
        }

        // RHElementActor::Execute returns IN_PROGRESS immediately for a
        // per-actor execution freeze. Actor::Hourglass still applies its
        // WAIT_TIMER / WAIT_FREE_LIFT modifier to that return value, so retain
        // the selected identity for those two commands without entering any
        // Execute arm or touching the sprite.
        if let Some(actor) = self
            .world
            .entities
            .get(entity_id)
            .and_then(Entity::actor_data)
            && actor.execution_frozen
        {
            let frozen_wait = self
                .orders
                .sequence_manager
                .current_order_for_actor(entity_id)
                .and_then(|(seq_id, elem_idx, order)| {
                    let element = self
                        .orders
                        .sequence_manager
                        .get_element(seq_id, elem_idx)
                        .unwrap_or_else(|| {
                            panic!(
                                "execution-frozen actor {entity_id:?} selected missing element {seq_id:?}/{elem_idx}"
                            )
                        });
                    matches!(element.command, Command::WaitTimer | Command::WaitFreeLift)
                        .then_some(ActorExecuteResult {
                            order_type: order.order_type,
                            entry_seq_id: seq_id,
                            entry_elem_idx: elem_idx,
                            motion: MotionState::InProgress,
                        })
                });
            return (Vec::new(), AnimCompletionOutcomes::default(), frozen_wait);
        }

        let mut combat_injury_terminated: Vec<EntityId> = Vec::new();
        // Sequence elements whose priority should be lifted to
        // `NonInterruptable` because their currently-driven anim hit
        // motion Start and is one of the always-non-interruptable
        // families (see `anim_forces_non_interruptable_on_start`).
        // Applied after the entity loop so we don't double-borrow
        // `self.orders.sequence_manager` while iterating `self.world.entities`.
        let mut completion_outcomes = AnimCompletionOutcomes::default();
        let mut execute_result = None;
        // Original dispatches the selected `mpOrder` through ordinary
        // Execute regardless of the actor's stale action-state enum. Only an
        // order stored in `RHSequenceElementMovement` belongs to the separate
        // movement driver. This distinction covers movement-exit animations,
        // injuries, waits, and interactions without a command allowlist.
        let selected_generic_order = self
            .orders
            .sequence_manager
            .current_order_for_actor(entity_id)
            .and_then(|(seq_id, elem_idx, order)| {
                self.orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .map(|element| {
                        !element.data.is_movement()
                            || order.order_type == OrderType::Select
                            || matches!(element.command, Command::WaitTimer | Command::WaitFreeLift)
                            // A movement element's translated order chain also
                            // holds pure animation steps — the door-pass Turning
                            // and crouch transitions, and the FREEZING token a
                            // pathfinding move carries. Those belong to the
                            // generic Execute switch even though the element
                            // itself is a movement element, so the actor's
                            // stale moving action-state must not suppress them.
                            || super::tick::classify_live_actor_execute_arm(
                                entity_id,
                                order.order_type,
                            ) == Some(super::tick::ExecuteOwnerFamily::GenericAnimation)
                    })
            })
            .unwrap_or(false);
        let (
            principal_frames_from_now,
            drinking_ale_antagonist_active,
            door_pass_crenel_transition_dir,
            validated_antagonist,
            waiting_sword_direction_goal,
            standing_up_sword_direction_goal,
            extracting_arrow_sword_direction_goal,
            striking_down_sword_direction_goal,
            pc_taking_direction_goal,
            pc_target_direction_goal,
        ) = {
            let entity = self.world.entities.get(entity_id).unwrap_or_else(|| {
                panic!(
                    "actor animation creation slot {} lost entity {entity_id:?}",
                    entity_id.index()
                )
            });
            let actor = entity.actor_data().unwrap_or_else(|| {
                panic!(
                    "actor animation creation slot {} resolved non-actor {entity_id:?}",
                    entity_id.index()
                )
            });
            if (actor.action_state.is_moving()
                || matches!(
                    actor.action_state,
                    crate::element::ActionState::MovingSword
                        | crate::element::ActionState::MovingFastSword
                        | crate::element::ActionState::MovingShield
                ))
                && !selected_generic_order
            {
                return (Vec::new(), AnimCompletionOutcomes::default(), None);
            }

            let Some((seq_id, elem_idx, order)) = self
                .orders
                .sequence_manager
                .current_order_for_actor(entity_id)
            else {
                return (Vec::new(), AnimCompletionOutcomes::default(), None);
            };
            let exact_selected_bow = actor.active_shot.is_active()
                && actor.active_shot.sequence_id == Some(seq_id)
                && actor.active_shot.element_index == elem_idx
                && crate::bow_shot::is_active_bow_order(order.order_type);
            if exact_selected_bow {
                return (Vec::new(), AnimCompletionOutcomes::default(), None);
            }
            let anim_type = order.order_type;
            let principal_frames = if anim_type == OrderType::TransitionWaitingSwordParryingSwordLow
            {
                entity
                    .human_data()
                    .and_then(|human| human.opponents.first().copied())
                    .map(|opponent| {
                        let opponent_entity =
                            self.world.entities.get(opponent).unwrap_or_else(|| {
                                panic!(
                                    "actor {entity_id:?} low-parry opponent {opponent:?} is missing at legacy slot {}",
                                    entity_id.index()
                                )
                            });
                        safe_frames_from_now_till_action_done(
                            &opponent_entity.element_data().sprite,
                        )
                    })
                    .flatten()
            } else {
                None
            };

            let anim_uses_antagonist = matches!(
                anim_type,
                OrderType::DrinkingAle
                    | OrderType::Taking
                    | OrderType::TakingCrouched
                    | OrderType::Searching
                    | OrderType::TakingNet
                    | OrderType::WakingUp
                    | OrderType::HittingTarget
                    | OrderType::HandlingTarget
                    | OrderType::TakingTarget
                    | OrderType::UsingLever
                    | OrderType::StrikingDownSword
            );
            let anim_requires_antagonist =
                anim_uses_antagonist && (anim_type != OrderType::Searching || entity.is_pc());
            let validated_antagonist = if anim_requires_antagonist {
                order.antagonist.or_else(|| {
                    panic!(
                        "actor {entity_id:?} {anim_type:?} requires antagonist at legacy slot {}",
                        entity_id.index()
                    )
                })
            } else if anim_uses_antagonist {
                order.antagonist
            } else {
                None
            };
            let antagonist_active = validated_antagonist.map(|antagonist| {
                let antagonist_entity =
                    self.world.entities.get(antagonist).unwrap_or_else(|| {
                        panic!(
                            "actor {entity_id:?} required {anim_type:?} antagonist {antagonist:?} is missing at legacy slot {}",
                            entity_id.index()
                        )
                    });
                if anim_type == OrderType::DrinkingAle {
                    Some(antagonist_entity.is_active())
                } else {
                    None
                }
            }).flatten();

            let striking_down_sword_direction = if anim_type == OrderType::StrikingDownSword {
                let antagonist_id = validated_antagonist
                    .expect("StrikingDownSword requires a validated antagonist");
                let antagonist = self.world.entities.get(antagonist_id).unwrap_or_else(|| {
                        panic!(
                            "actor {entity_id:?} StrikingDownSword antagonist {antagonist_id:?} is missing at legacy slot {}",
                            entity_id.index()
                        )
                    });
                Some(striking_down_sword_direction(entity, antagonist))
            } else {
                None
            };

            let door_direction = if matches!(
                anim_type,
                OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
                    | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
            ) && actor.execute_order_initialising
            {
                let dp = actor.active_door_pass.as_ref().unwrap_or_else(|| {
                    panic!(
                        "actor {entity_id:?} {anim_type:?} lacks required active door pass at legacy slot {}",
                        entity_id.index()
                    )
                });
                let reverse_direction =
                    anim_type == OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel;
                let door = self
                    .script_domains
                    .interactables
                    .doors
                    .get(usize::from(dp.door_index))
                    .unwrap_or_else(|| {
                        panic!(
                            "actor {entity_id:?} crenel transition references missing door {} at legacy slot {}",
                            dp.door_index,
                            entity_id.index()
                        )
                    });
                let sector_number = crate::sector::SectorNumber::new(i16::from(door.sector_in));
                let sector_index = self
                    .world
                    .fast_grid
                    .level
                    .sector_number_map
                    .get(&sector_number)
                    .copied()
                    .unwrap_or_else(|| {
                        panic!(
                            "actor {entity_id:?} crenel door {} references missing sector {sector_number:?} at legacy slot {}",
                            dp.door_index,
                            entity_id.index()
                        )
                    });
                let sector = self
                    .world
                    .fast_grid
                    .level
                    .sectors
                    .get(sector_index)
                    .unwrap_or_else(|| {
                        panic!(
                            "actor {entity_id:?} crenel door {} resolved invalid sector index {sector_index} at legacy slot {}",
                            dp.door_index,
                            entity_id.index()
                        )
                    });
                if sector.lift_type != Some(crate::sector::LiftType::Wall) {
                    panic!(
                        "actor {entity_id:?} crenel door {} requires wall-lift sector {sector_number:?}, found {:?}",
                        dp.door_index, sector.lift_type
                    );
                }
                Some(if reverse_direction {
                    (sector.lift_direction + 8) & 15
                } else {
                    sector.lift_direction
                })
            } else {
                None
            };

            // RHElementActorHuman::Execute refreshes combat-facing goals
            // before Turn() and PerformAction(). WaitingSword, normal parry,
            // and smalltalk parries face the live principal opponent;
            // smalltalk strikes face their order antagonist. Normal low parry
            // does not share either arm. A stale reference is an invariant
            // failure here: the Original dereferences it directly.
            let is_swordfighting = entity
                .human_data()
                .is_some_and(|human| !human.opponents.is_empty());
            let facing_opponent = if matches!(
                anim_type,
                OrderType::WaitingSword
                    | OrderType::ParryingSword
                    | OrderType::ParryingLeftSmalltalk
                    | OrderType::ParryingRightSmalltalk
                    | OrderType::ParryingLowLeftSmalltalk
                    | OrderType::ParryingLowRightSmalltalk
            ) {
                entity
                    .human_data()
                    .and_then(|human| human.opponents.first().copied())
            } else if is_swordfighting
                && matches!(
                    anim_type,
                    OrderType::StrikingLeftSmalltalk
                        | OrderType::StrikingRightSmalltalk
                        | OrderType::StrikingLowLeftSmalltalk
                        | OrderType::StrikingLowRightSmalltalk
                )
            {
                order.antagonist
            } else if anim_type == OrderType::TransitionRaisingSword
                && actor.execute_order_initialising
            {
                order.antagonist
            } else {
                None
            };
            let waiting_sword_direction = facing_opponent.map(|opponent_id| {
                let opponent = self.world.entities.get(opponent_id).unwrap_or_else(|| {
                    panic!(
                        "actor {entity_id:?} {anim_type:?} opponent {opponent_id:?} is missing at legacy slot {}",
                        entity_id.index()
                    )
                });
                if anim_type == OrderType::TransitionRaisingSword {
                    raising_sword_direction(entity, opponent)
                } else {
                    let from = entity.element_data().position();
                    let to = opponent.element_data().position();
                    crate::position_interface::vector_to_sector_0_to_15_iso(
                        to.x - from.x,
                        to.y - from.y,
                    )
                }
            });

            // Unlike the other combat-facing arms above, Original's human
            // STANDING_UP_SWORD arm plays the sprite first and only then
            // refreshes the goal and turns. Compute the live principal here,
            // but defer both mutations until after PerformAction. The soldier
            // override has no corresponding facing or Turn step.
            let standing_up_sword_direction = if anim_type == OrderType::StandingUpSword
                && !entity.is_soldier()
                && is_swordfighting
            {
                let opponent_id = entity
                    .human_data()
                    .and_then(|human| human.opponents.first().copied())
                    .expect("swordfighting stand-up actor has no principal opponent");
                let opponent = self.world.entities.get(opponent_id).unwrap_or_else(|| {
                    panic!(
                        "actor {entity_id:?} standing-up opponent {opponent_id:?} is missing at legacy slot {}",
                        entity_id.index()
                    )
                });
                let from = entity.element_data().position();
                let to = opponent.element_data().position();
                Some(crate::position_interface::vector_to_sector_0_to_15_iso(
                    to.x - from.x,
                    to.y - from.y,
                ))
            } else {
                None
            };

            // RHElementActorHuman::Execute(EXTRACTING_ARROW_SWORD) only
            // refreshes the goal on the order's initialization tick, then
            // calls Turn() on every tick while the actor remains in a
            // swordfight.  The arrow impact may have just snapped the body
            // toward the projectile, so retaining that direction as the goal
            // makes the extraction animation face away from the live duel.
            let extracting_arrow_sword_direction = if actor.execute_order_initialising
                && anim_type == OrderType::ExtractingArrowSword
                && is_swordfighting
            {
                let opponent_id = entity
                    .human_data()
                    .and_then(|human| human.opponents.first().copied())
                    .expect("swordfighting arrow-damage actor has no principal opponent");
                let opponent = self.world.entities.get(opponent_id).unwrap_or_else(|| {
                    panic!(
                        "actor {entity_id:?} extracting-arrow opponent {opponent_id:?} is missing at legacy slot {}",
                        entity_id.index()
                    )
                });
                let from = entity.element_data().position();
                let to = opponent.element_data().position();
                Some(crate::position_interface::vector_to_sector_0_to_15_iso(
                    to.x - from.x,
                    to.y - from.y,
                ))
            } else {
                None
            };

            // RHElementActorPC::Execute initializes TAKING from the live
            // owner/object map positions immediately before its per-tick
            // Turn(). Do not rely on a preceding Seek having left a suitable
            // direction goal: direct interaction sequences take this path too.
            let taking_direction = if entity.is_pc()
                && actor.execute_order_initialising
                && matches!(anim_type, OrderType::Taking | OrderType::TakingCrouched)
            {
                validated_antagonist.map(|antagonist_id| {
                    let antagonist =
                        self.world.entities.get(antagonist_id).unwrap_or_else(|| {
                            panic!(
                                "actor {entity_id:?} {anim_type:?} antagonist {antagonist_id:?} is missing at legacy slot {}",
                                entity_id.index()
                            )
                        });
                    let from = entity.element_data().position_map();
                    let to = antagonist.element_data().position_map();
                    crate::position_interface::vector_to_sector_0_to_15(
                        to.x - from.x,
                        to.y - from.y,
                    )
                })
            } else {
                None
            };
            // HIT_TARGET / HANDLE_TARGET use the target sprite row's live
            // action hotspot, not the target's interaction PositionMap. The
            // original intentionally uses GetSector0to15() without the
            // isometric aspect argument for this screen-space vector.
            let target_direction = if entity.is_pc()
                && actor.execute_order_initialising
                && matches!(
                    anim_type,
                    OrderType::HittingTarget | OrderType::HandlingTarget
                ) {
                validated_antagonist.map(|antagonist_id| {
                    let antagonist = self.world.entities.get(antagonist_id).unwrap_or_else(|| {
                        panic!(
                            "actor {entity_id:?} {anim_type:?} antagonist \
                                 {antagonist_id:?} is missing at legacy slot {}",
                            entity_id.index()
                        )
                    });
                    let from = entity.element_data().position_map();
                    let to = antagonist.cxx_current_point_map().unwrap_or_else(|| {
                        panic!(
                            "actor {entity_id:?} {anim_type:?} target {antagonist_id:?} \
                             has no current sprite hotspot"
                        )
                    });
                    crate::position_interface::vector_to_sector_0_to_15(
                        to.x - from.x,
                        to.y - from.y,
                    )
                })
            } else {
                None
            };

            (
                principal_frames,
                antagonist_active,
                door_direction,
                validated_antagonist,
                waiting_sword_direction,
                standing_up_sword_direction,
                extracting_arrow_sword_direction,
                striking_down_sword_direction,
                taking_direction,
                target_direction,
            )
        };

        // TranslateHitDamage only appends a FALLING_HIT_* order. Original
        // ExecuteFallingHit samples live geometry and calls ReadyForTakeOff
        // under IsInitialisation(), so actors whose creation slot has already
        // passed retain their old facing and no flight state until next frame.
        let initial_hit_flight = self
            .world
            .entities
            .get(entity_id)
            .and_then(Entity::actor_data)
            .is_some_and(|actor| actor.execute_order_initialising)
            .then(|| {
                self.orders
                    .sequence_manager
                    .current_order_for_actor(entity_id)
                    .map(|(_, _, order)| (order.order_type, order.antagonist))
            })
            .flatten()
            .filter(|(anim, _)| {
                matches!(
                    anim,
                    OrderType::FallingHitUpright
                        | OrderType::FallingHitWithBow
                        | OrderType::FallingHitWithSword
                        | OrderType::FallingHitCrouched
                )
            });
        if let Some((anim, antagonist)) = initial_hit_flight {
            self.initialize_hit_flight(assets, entity_id, antagonist, anim);
        }

        // TranslatePushDamage likewise only authors the falling order.
        // ExecuteFallingPushed initializes ReadyForTakeOff from live strike
        // geometry immediately before its first PerformFlight call.
        let initial_push_flight = self
            .world
            .entities
            .get(entity_id)
            .and_then(Entity::actor_data)
            .is_some_and(|actor| actor.execute_order_initialising)
            .then(|| {
                self.orders
                    .sequence_manager
                    .current_order_for_actor(entity_id)
                    .map(|(sequence_id, element_index, order)| {
                        (sequence_id, element_index, order.order_type)
                    })
            })
            .flatten()
            .filter(|(_, _, anim)| {
                matches!(
                    anim,
                    OrderType::FallingPushedUpright
                        | OrderType::FallingPushedWithBow
                        | OrderType::FallingPushedWithSword
                        | OrderType::FallingPushedCrouched
                )
            });
        if let Some((sequence_id, element_index, anim)) = initial_push_flight {
            self.initialize_push_flight(assets, entity_id, (sequence_id, element_index), anim);
        }

        // RHElementActorHuman's eight dying/falling arms run
        // FindPlaceToDie under IsInitialisation() immediately before
        // PerformAction. Do this before borrowing the actor for the generic
        // sprite dispatch so same-stack animation side effects and callbacks
        // observe the relocated position in Original order.
        let find_place_to_die_on_initialisation = self
            .world
            .entities
            .get(entity_id)
            .and_then(Entity::actor_data)
            .is_some_and(|actor| actor.execute_order_initialising)
            && self
                .orders
                .sequence_manager
                .current_order_for_actor(entity_id)
                .is_some_and(|(_, _, order)| {
                    matches!(
                        order.order_type,
                        OrderType::DyingSword
                            | OrderType::DyingBow
                            | OrderType::FallingBackSword
                            | OrderType::FallingBackBow
                            | OrderType::DyingUpright
                            | OrderType::FallingBackUpright
                            | OrderType::FallingBackCrouched
                            | OrderType::DyingCrouched
                    )
                });
        if find_place_to_die_on_initialisation {
            self.find_place_to_die(entity_id);
        }

        // Original `RHElementActorHuman::Execute` advances the
        // STRIKING_DOWN_SWORD sprite and then revalidates the selected
        // SwordstrikeDown element without a distance check. Snapshot the
        // non-sprite operands immediately before the exclusive actor borrow;
        // `PerformAction`/`Turn` cannot mutate any of them. The result is
        // applied below immediately after `perform_action` and before motion
        // side effects, preserving the Original boundary.
        let striking_down_sword_valid_after_perform = self
            .orders
            .sequence_manager
            .current_order_for_actor(entity_id)
            .and_then(|(seq_id, elem_idx, order)| {
                (order.order_type == OrderType::StrikingDownSword).then_some((seq_id, elem_idx))
            })
            .map(|(seq_id, elem_idx)| {
                let element = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .expect("selected StrikingDownSword element disappeared before Execute");
                let antagonist = match element.data {
                    crate::sequence::SequenceElementData::Interaction { antagonist } => {
                        antagonist.expect("selected StrikingDownSword element lost its antagonist")
                    }
                    _ => panic!("selected StrikingDownSword element is not an interaction"),
                };
                let actor = self.world.entities.get(entity_id).unwrap_or_else(|| {
                    panic!("StrikingDownSword owner {entity_id:?} disappeared before Execute")
                });
                let victim = self.world.entities.get(antagonist).unwrap_or_else(|| {
                    panic!("StrikingDownSword victim {antagonist:?} disappeared before Execute")
                });
                super::sequence_validity::striking_down_sword_valid_without_position(
                    actor,
                    victim,
                    self.is_entity_vip(assets, victim),
                )
            });

        'actor: loop {
            let entity = self.world.entities.get_mut(entity_id).unwrap_or_else(|| {
                panic!("actor {entity_id:?} vanished before generic animation dispatch")
            });
            // Actors: animate based on current action state
            if let Some(actor) = entity.actor_data() {
                // Moving actors are animated in tick_entity_movement(sim, ),
                // which computes per-frame combat directional anims
                // (WalkingSword / StrafingRightSword / …) for the
                // sword/shield variants rather than using the Move
                // element's order.action (which is the logical
                // `WalkingWithSword`, unmapped in PC sprite profiles).
                // Keep this gate aligned with tick_entity_movement's
                // own is_moving / sword / shield guard
                // (movement.rs:2210).
                if (actor.action_state.is_moving()
                    || matches!(
                        actor.action_state,
                        crate::element::ActionState::MovingSword
                            | crate::element::ActionState::MovingFastSword
                            | crate::element::ActionState::MovingShield
                    ))
                    && !selected_generic_order
                {
                    break 'actor;
                }

                // Exact selected melee and bow arms are admitted by the live
                // owner coordinator/current-order check. Stale background
                // state must not suppress the actual selected generic arm.
                let direction = entity.element_data().direction() as u16;

                // Read the actor's current in-progress sequence element
                // and its front order.  All animation driving flows off
                // this — dispatch is on the current element's front
                // order.
                //
                // Disjoint-borrow: `self.orders.sequence_manager` is a field of
                // `self` distinct from `self.world.entities`, so the compiler
                // accepts holding `&self.orders.sequence_manager` while iterating
                // mutable occupied entity slots.
                let order_snapshot = self
                    .orders
                    .sequence_manager
                    .current_order_for_actor(entity_id);
                let (
                    order_seq_elem,
                    anim_type,
                    order_id,
                    order_antagonist,
                    order_completion,
                    order_tolerance,
                    order_target,
                    defer_initial_turn_step,
                ) = if let Some((seq_id, elem_idx, order)) = order_snapshot {
                    (
                        Some((seq_id, elem_idx)),
                        order.order_type,
                        Some(order.order_id),
                        order.antagonist,
                        Some(order.completion.clone()),
                        order.tolerance,
                        crate::coordinates::MapPoint::new(order.target_x, order.target_y),
                        order.defer_initial_turn_step,
                    )
                } else {
                    (
                        None,
                        crate::order::OrderType::Invalid,
                        None,
                        None,
                        None,
                        0.0,
                        crate::coordinates::MapPoint::ZERO,
                        false,
                    )
                };
                if let Some((seq_id, elem_idx)) = order_seq_elem
                    && actor.active_shot.is_active()
                    && actor.active_shot.sequence_id == Some(seq_id)
                    && actor.active_shot.element_index == elem_idx
                    && crate::bow_shot::is_active_bow_order(anim_type)
                {
                    break 'actor;
                }
                let antagonist = validated_antagonist.or(order_antagonist);

                // Is the current element a one-shot action (not the
                // actor's `Command::Wait` idle element)?  One-shots
                // drive dead / unconscious actors through DYING_* /
                // FALLING_HIT_* terminates before the corpse settles.
                // The settled BEING_DEAD / BEING_UNCONSCIOUS wait
                // orders still need to execute every tick to keep the
                // sprite row on the corpse/KO hold row used by
                // body-point calculations (e.g. compute-stars-point).
                let cur_command = order_seq_elem.and_then(|(s, e)| {
                    self.orders
                        .sequence_manager
                        .get_element(s, e)
                        .map(|el| el.command)
                });
                let cur_command_level = order_seq_elem.and_then(|(s, e)| {
                    self.orders
                        .sequence_manager
                        .get_element(s, e)
                        .map(|el| el.command_level)
                });
                let current_element_script_driven = order_seq_elem.is_some_and(|(s, e)| {
                    self.orders
                        .sequence_manager
                        .get_element(s, e)
                        .is_some_and(|element| element.script_driven)
                });
                let element_retains_movement_goal = order_seq_elem.is_some_and(|(s, e)| {
                    self.orders
                        .sequence_manager
                        .get_element(s, e)
                        .is_some_and(|element| {
                            element.retained_movement_goal.is_some()
                                || element
                                    .get_property(crate::sequence::Field::RetainedMovementGoal)
                                    .is_some()
                        })
                });
                let turn_resumed_from_legacy_save = order_seq_elem.is_some_and(|(s, e)| {
                    self.orders
                        .sequence_manager
                        .get_element(s, e)
                        .is_some_and(|element| element.legacy_v48.is_some())
                });
                let requested_custom_animation = order_seq_elem.and_then(|(s, e)| {
                    let element = self.orders.sequence_manager.get_element(s, e)?;
                    if !matches!(
                        element.command,
                        Command::PlayAnim
                            | Command::PlayAnimLoop
                            | Command::PlayAnimFreeze
                            | Command::PlayAnimFrozen
                    ) {
                        return None;
                    }
                    match element.get_property(crate::sequence::Field::AnimationId) {
                        Some(crate::sequence::FieldValue::Animation(animation)) => Some(*animation),
                        Some(crate::sequence::FieldValue::Integer(value)) => {
                            OrderType::try_from(*value).ok()
                        }
                        _ => None,
                    }
                });
                let pointing_direction_goal = if cur_command == Some(Command::Point) {
                    let direction = order_seq_elem
                        .and_then(|(s, e)| {
                            self.orders
                                .sequence_manager
                                .get_element(s, e)
                                .and_then(|element| {
                                    element.get_property(crate::sequence::Field::Direction)
                                })
                        })
                        .and_then(|value| match value {
                            crate::sequence::FieldValue::Integer(direction) => {
                                Some(*direction as i16)
                            }
                            _ => None,
                        })
                        .expect("Point sequence is missing its integer Direction property");
                    Some(direction)
                } else {
                    None
                };
                // AI-driven animation (Pointing, RaisingShield, dying,
                // falling-hit, BORED idle cycle, …)?  Drive it via
                // perform_action; on completion, run side-effect
                // helpers and fire the bound `OrderCompletion`.
                if let Some((seq_id, elem_idx)) = order_seq_elem {
                    tracing::trace!(
                        entity = entity_id.index(),
                        ?anim_type,
                        order_id,
                        ?cur_command,
                        "animation: driving order"
                    );
                    // The soldier dispatch swaps many animations for
                    // an "alerted" variant when attentive is set.  For
                    // TURNING specifically the completion flag is
                    // driven by `turn_fast()` / `turn()` rather than
                    // the sprite's `action_done_frame` — the sprite is
                    // just played for the visual, while the body
                    // rotates progressively toward the direction goal.
                    let is_turn = matches!(anim_type, OrderType::Turning);
                    let effective_anim = if soldier_is_attentive(entity) {
                        alerted_variant(anim_type).unwrap_or(anim_type)
                    } else {
                        anim_type
                    };
                    // The civilian dispatch coerces the entire
                    // `WAITING_UPRIGHT` family to
                    // `WAITING_UPRIGHT_BORED` for civilians, so a
                    // civilian never plays the plain upright wait or
                    // its get-up / random variants — they stay in the
                    // bored idle loop.  This sits after the
                    // soldier-attentive remap so it can't be
                    // re-overridden.
                    let effective_anim = if entity.is_civilian()
                        && matches!(
                            effective_anim,
                            OrderType::WaitingUpright
                                | OrderType::WaitingUprightBored
                                | OrderType::WaitingUprightBoredRandom
                                | OrderType::TransitionWaitingUprightBoredWaitingUpright
                                | OrderType::TransitionWaitingUprightWaitingUprightBored,
                        ) {
                        OrderType::WaitingUprightBored
                    } else {
                        effective_anim
                    };
                    // RHNONANIMATION_DROPPING_ALE_CROUCHED is only a
                    // dispatch token. The PC override plays the authored
                    // TAKING_CROUCHED sprite while retaining the drop order
                    // for command/state and DONE-side-effect handling.
                    let effective_anim = if effective_anim == OrderType::DroppingAleCrouched {
                        OrderType::TakingCrouched
                    } else {
                        effective_anim
                    };
                    let mut weak_sword_held = false;
                    let owner_is_pc = entity.is_pc();
                    // Jump steps whose Execute arm calls PerformMotion rather
                    // than PerformAction: they approach an authored map point
                    // while their transition animation plays.
                    let actor_in_jump = entity
                        .actor_data()
                        .is_some_and(|actor| actor.active_jump.is_some());
                    let jump_ground_motion_step =
                        super::jump::jump_step_uses_perform_motion(anim_type) && actor_in_jump;
                    // Airborne segments fly the body themselves and ignore
                    // what their animation reports, so they take their own
                    // Execute path below.
                    let jump_airborne_step = super::jump::jump_step_is_airborne(entity, anim_type);
                    let order_is_initialising = actor.execute_order_initialising;
                    if let Some(direction) = waiting_sword_direction_goal {
                        entity.element_data_mut().set_direction_goal(direction);
                    }
                    if let Some(direction) = extracting_arrow_sword_direction_goal {
                        entity.element_data_mut().set_direction_goal(direction);
                    }
                    if let Some(direction) = pc_taking_direction_goal {
                        entity.element_data_mut().set_direction_goal(direction);
                    }
                    if let Some(direction) = pc_target_direction_goal {
                        entity.element_data_mut().set_direction_goal(direction);
                    }
                    if order_is_initialising
                        && anim_type == OrderType::Pointing
                        && let Some(direction) = pointing_direction_goal
                    {
                        // Original POINT translation only books the order.
                        // Its first Execute tick sets the progressive goal,
                        // then Turn() advances one sector before sprite
                        // playback.
                        entity.element_data_mut().set_direction_goal(direction);
                    }
                    if order_is_initialising
                        && matches!(
                            anim_type,
                            OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
                                | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
                        )
                        && let Some(direction) = door_pass_crenel_transition_dir
                    {
                        entity.element_data_mut().set_direction_instantly(direction);
                        entity.set_posture(crate::element::Posture::Flying);
                    }
                    let drinking_ale_antagonist_inactive =
                        matches!(anim_type, OrderType::DrinkingAle)
                            && antagonist.is_some()
                            && drinking_ale_antagonist_active == Some(false);
                    let special_speech_id = if anim_type == OrderType::Special
                        && entity.is_soldier()
                    {
                        Some(
                                entity
                                .soldier_data()
                                .and_then(|soldier| {
                                    assets
                                    .profile_manager
                                    .get_soldier(soldier.soldier_profile_index)
                            })
                            .map(|profile| profile.exclamation_id)
                            .unwrap_or_else(|| {
                                    panic!(
                                        "Special animation owner {entity_id:?} has no soldier profile"
                                    )
                                }),
                            )
                    } else {
                        None
                    };
                    // RHElementActorSoldier::Execute checks an ordinary
                    // Special animation's IsAtStartOfAnim before
                    // PerformAction. The halberdman exception is deliberately
                    // checked after PerformAction at frame 40 below.
                    let special_sprite_before_perform = special_speech_id.map(|_| {
                        let sprite = entity.sprite();
                        (sprite.current_frame, sprite.frame_count)
                    });
                    if order_is_initialising
                        && matches!(cur_command, Some(Command::Turn | Command::TurnFast))
                        && !element_retains_movement_goal
                        && !turn_resumed_from_legacy_save
                    {
                        // FaceTo can be instructed re-entrantly after a
                        // just-launched GoNear has already written its map
                        // goal. Original leaves that value observable for the
                        // launch frame, then the outgoing movement's
                        // SendCondolationCard clears it before the Turn
                        // element executes its first order. Rust stages those
                        // callbacks, so reproduce the selected-actor boundary
                        // here rather than retaining the superseded movement
                        // destination throughout the turn transition. A
                        // deferred FaceTo carrying RetainedMovementGoal is
                        // different: the outgoing movement's card observed
                        // that Turn as selected, so Original preserves the
                        // goal and Turn::Execute never clears it. A restored
                        // in-progress Turn likewise has no outgoing movement
                        // condolence in this actor slot: its serialized sprite
                        // goal must remain observable while the loaded order
                        // resumes.
                        tracing::trace!(
                            target: "parity_owner_handoff",
                            owner = ?entity_id,
                            ?cur_command,
                            goal = ?entity.position_iface().map_goal(),
                            "turn launch clearing superseded movement goal"
                        );
                        entity
                            .position_iface_mut()
                            .set_map_goal(crate::coordinates::MapPoint::ZERO);
                    }
                    let motion = if is_turn {
                        let direction_before_turn = entity.element_data().direction() as u16;
                        let still_turning = if order_is_initialising && defer_initial_turn_step {
                            true
                        } else {
                            turn_with_provenance(
                                entity,
                                entity_id,
                                self.control.frame_counter,
                                if cur_command == Some(Command::TurnFast) {
                                    "turn_order_fast"
                                } else {
                                    "turn_order"
                                },
                                cur_command == Some(Command::TurnFast),
                            )
                        };
                        if !globally_frozen {
                            let direction_after_turn = entity.element_data().direction() as u16;
                            // Base Actor executes Turn before PerformAction,
                            // but the attentive Soldier override performs
                            // TURNING_ALERTED first. The explicit row keeps
                            // that Original ordering visible while the shared
                            // turn step still updates the position interface
                            // before this block returns.
                            let row = actor_action_row(
                                anim_type,
                                effective_anim,
                                direction_before_turn,
                                direction_after_turn,
                            );
                            let sprite = &mut entity.element_data_mut().sprite;
                            let _ = sprite.perform_action(
                                sim,
                                order_id,
                                effective_anim,
                                row,
                                FrameProgression::Default,
                                false,
                            );
                        }
                        // PI is the single source of truth for direction —
                        // no sync needed now that `ElementData.direction`
                        // is gone.
                        // Swallow `sprite_motion` — its Done/Terminated
                        // is driven by `action_done_frame` in sprite
                        // data, but the control flow here ties
                        // completion to `turn_fast()` instead.
                        if still_turning {
                            Some(MotionState::InProgress)
                        } else {
                            Some(MotionState::Terminated)
                        }
                    } else if anim_type == OrderType::Select {
                        // SELECT is a real non-animation order in the
                        // translated door chain. It starts the Human/PC hulk
                        // effect and terminates in this owner slot without
                        // dispatching a sprite animation.
                        if order_is_initialising {
                            completion_outcomes
                                .select_hulk
                                .push((entity_id, order_tolerance));
                        }
                        Some(MotionState::Terminated)
                    } else if matches!(anim_type, OrderType::DrinkingAle)
                        && order_is_initialising
                        && drinking_ale_antagonist_inactive
                    {
                        Some(MotionState::Terminated)
                    } else {
                        // Many per-anim handlers call `Turn()` each
                        // tick so the body keeps rotating toward the
                        // direction goal *while* the action animation
                        // plays.  Turning is decided strictly per animation
                        // arm, and neighbouring arms of the same family often
                        // disagree: `ParryingSword` turns but `ParryingLowSword`
                        // does not, `StandingUpSword` turns but `StandingUpBow`
                        // does not, and the helping-to-climb entry/exit
                        // transitions do not turn while the
                        // `WaitingHelpingClimbing` idle between them does.
                        // See the per-arm table in `docs/TURN_ARMS.md`.
                        // Step the rotation here, then sync `element.direction`
                        // to match before the sprite picks the row to play.
                        let needs_turn = (matches!(
                            anim_type,
                            OrderType::TransitionRaisingSword
                                | OrderType::TransitionLoweringSword
                                | OrderType::WaitingSword
                                | OrderType::WaitingShield
                                | OrderType::ParryingSword
                                | OrderType::StrikingLowLeftSmalltalk
                                | OrderType::StrikingLowRightSmalltalk
                                | OrderType::StrikingLeftSmalltalk
                                | OrderType::StrikingRightSmalltalk
                                | OrderType::ParryingLeftSmalltalk
                                | OrderType::ParryingRightSmalltalk
                                | OrderType::ParryingLowLeftSmalltalk
                                | OrderType::ParryingLowRightSmalltalk
                                | OrderType::StrikingDownSword
                                | OrderType::ExtractingArrowSword
                                | OrderType::FallingLadderWall
                                | OrderType::RaisingShield
                                | OrderType::Rolling
                                | OrderType::Taking
                                | OrderType::TakingCrouched
                                | OrderType::TakingTarget
                                | OrderType::DroppingAmmo
                                | OrderType::DroppingAmmoCrouched
                                | OrderType::DroppingAle
                                | OrderType::DroppingAleCrouched
                                | OrderType::UsingLever
                                | OrderType::DrinkingAle
                                | OrderType::TakingNet
                                | OrderType::HittingTarget
                                | OrderType::HandlingTarget
                                | OrderType::UnlockingDoor
                                | OrderType::UnlockingTrap
                                | OrderType::SearchingCrouched
                                | OrderType::WaitingHelpingClimbing
                                | OrderType::WaitingCarryingOnShoulders
                                | OrderType::TransitionWaitingCarryingOnShouldersWaitingUpright
                                | OrderType::FallingShoulders
                                | OrderType::TransitionCrouchingUp
                                | OrderType::TransitionCrouchingDown
                                | OrderType::TransitionWaitingUprightClimbingWallUp
                                | OrderType::TransitionClimbingWallUpWaitingCrouched
                                | OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
                                | OrderType::TransitionWaitingCrouchedClimbingWallDown
                                | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
                                | OrderType::TransitionClimbingWallDownWaitingUpright
                                | OrderType::TransitionWaitingUprightClimbingLadderUp
                                | OrderType::TransitionClimbingLadderUpWaitingCrouched
                                | OrderType::TransitionWaitingCrouchedClimbingLadderDown
                                | OrderType::TransitionClimbingLadderDownWaitingUpright
                                | OrderType::ClimbingWallUp
                                | OrderType::ClimbingWallDown
                                | OrderType::ClimbingWallUpFast
                                | OrderType::ClimbingWallDownFast
                                | OrderType::ClimbingLadderUp
                                | OrderType::ClimbingLadderDown
                                | OrderType::ClimbingLadderUpFast
                                | OrderType::ClimbingLadderDownFast
                                // GETTING_FREE_FROM_WASP calls `Turn()` each
                                // tick; the "still turning" branch substitutes
                                // `TURNING_ALERTED` for the sprite — handled
                                // below via `wasp_still_turning`.
                                | OrderType::GettingFreeFromWasp
                                // NPC-only arms that call Turn() per-tick.
                                // POINTING installs its authored direction
                                // goal on the first Execute tick above.
                                // SEARCHING has no direction_goal writer yet,
                                // but the parity slot is required.
                                | OrderType::Pointing
                                | OrderType::Searching
                        ) && turn_arm_condition_holds(
                            entity,
                            anim_type,
                            antagonist.is_some(),
                        )) || (entity.is_pc()
                            && pc_beggar_execute_calls_turn(anim_type));
                        // Capture `Turn()`'s return for the GETTING_FREE_FROM_WASP
                        // still-turning substitution: while still
                        // turning, play TURNING_ALERTED and return
                        // InProgress; otherwise delegate to the
                        // configured animation.
                        let mut wasp_still_turning = false;
                        let mut pc_taking_still_turning = false;
                        let mut pc_target_still_turning = false;
                        let direction_before_turn = entity.element_data().direction() as u16;
                        if order_is_initialising
                            && anim_type == OrderType::RaisingShield
                            && let Some(danger) = entity
                                .actor_data()
                                .and_then(|actor| actor.shield_face_point)
                        {
                            // RHElementActorHuman::Execute initializes
                            // RAISING_SHIELD by applying the generic
                            // SHIELD_DANGER_POINT as the direction goal,
                            // immediately before its first Turn().  Do this
                            // here rather than at command dispatch: an
                            // exit-action transition can leave the order
                            // queued for several frames.
                            let position = entity.element_data().position();
                            let dx = danger.x - position.x;
                            let dy = danger.y - position.y;
                            if dx != 0.0 || dy != 0.0 {
                                let direction =
                                    crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy);
                                entity.position_iface_mut().set_direction(
                                    crate::position_interface::Direction::from_raw(
                                        direction as i32,
                                    ),
                                );
                            }
                        }
                        if owner_is_pc
                            && anim_type == OrderType::WaitingShield
                            && let Some(danger) = entity
                                .actor_data()
                                .and_then(|actor| actor.shield_face_point)
                            && (danger.x != 0.0 || danger.y != 0.0)
                        {
                            // The PC override of WAITING_SHIELD re-aims at the
                            // shield danger point on *every* tick, not just at
                            // initialization, so a danger point that moves
                            // while the shield is up keeps dragging the facing
                            // around. The soldier override has no such step.
                            let position = entity.element_data().position();
                            let dx = danger.x - position.x;
                            let dy = danger.y - position.y;
                            let direction =
                                crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy);
                            entity.position_iface_mut().set_direction(
                                crate::position_interface::Direction::from_raw(direction as i32),
                            );
                        }
                        if needs_turn {
                            if owner_is_pc && anim_type == OrderType::RaisingShield {
                                // The PC override turns and then delegates to
                                // the human arm, which turns again: a PC
                                // raising its shield rotates two steps per
                                // tick, unlike every other shield holder.
                                let _ = turn_with_provenance(
                                    entity,
                                    entity_id,
                                    self.control.frame_counter,
                                    "raising_shield_pc_override",
                                    false,
                                );
                            }
                            let still_turning = turn_with_provenance(
                                entity,
                                entity_id,
                                self.control.frame_counter,
                                if anim_type == OrderType::RaisingShield {
                                    "raising_shield_human"
                                } else if anim_type == OrderType::WaitingShield {
                                    "waiting_shield_human"
                                } else {
                                    "animation_turn_arm"
                                },
                                false,
                            );
                            if anim_type == OrderType::WaitingShield && still_turning {
                                // WaitingShield calls UpdateShield iff Turn()
                                // actually changed the facing direction.
                                crate::bow_shot::refresh_retained_shield_obstacle(
                                    entity,
                                    &assets.profile_manager,
                                );
                            }
                            if matches!(anim_type, OrderType::GettingFreeFromWasp) {
                                wasp_still_turning = still_turning;
                            }
                            if owner_is_pc
                                && matches!(
                                    anim_type,
                                    OrderType::Taking
                                        | OrderType::TakingCrouched
                                        | OrderType::TakingTarget
                                        | OrderType::DroppingAmmo
                                        | OrderType::DroppingAmmoCrouched
                                )
                            {
                                pc_taking_still_turning = still_turning;
                            }
                            if owner_is_pc
                                && matches!(
                                    anim_type,
                                    OrderType::HittingTarget | OrderType::HandlingTarget
                                )
                            {
                                pc_target_still_turning = still_turning;
                            }
                        }
                        let row = actor_action_row(
                            anim_type,
                            effective_anim,
                            direction_before_turn,
                            entity.element_data().direction() as u16,
                        );
                        let held_weak_sword = hold_weak_sword_at_action_done(entity, anim_type);
                        if held_weak_sword.is_some() {
                            weak_sword_held = true;
                        }
                        let sprite_motion = held_weak_sword.or_else(|| {
                            if globally_frozen {
                                // RHEngine::FrozenAll leaves Actor::Execute
                                // live but RHSprite::PerformAction returns
                                // IN_PROGRESS without selecting, stamping, or
                                // advancing the sprite. Actor initialization
                                // was nevertheless consumed at Hourglass
                                // entry, independently of this sprite call.
                                return Some(MotionState::InProgress);
                            }
                            if effective_anim == OrderType::Freezing {
                                // C++ RHElementActor::Execute handles
                                // RHNONANIMATION_FREEZING by returning
                                // RHMOTION_IN_PROGRESS without calling any
                                // RHSprite method.  In particular it must not
                                // stamp WaitingUpright: a pathfinding
                                // MoveWaiting can be inserted between two
                                // instances of the same transition, and
                                // MaybeInitializeFrame then resumes the
                                // transition's existing frame phase.
                                return Some(MotionState::InProgress);
                            }
                            let elem = entity.element_data_mut();
                            let sprite = &mut elem.sprite;
                            // GETTING_FREE_FROM_WASP still-turning: the
                            // arm substitutes `TURNING_ALERTED` while
                            // `Turn()` is still rotating the body
                            // toward the random offset, then switches
                            // to the configured animation once rotation
                            // is done.  Otherwise delegate to
                            // `sprite_anim_for_order` which runs the
                            // per-arm non-animation → animation
                            // substitutions (FALLING_HIT_* →
                            // FALLING_BACK_*, missing-shield-variant
                            // fallbacks, etc.).  The order's `anim_type`
                            // stays unchanged so side-effect handlers
                            // keep matching on the original token.
                            let (played, progression) = if wasp_still_turning {
                                (OrderType::TurningAlerted, FrameProgression::Default)
                            } else if pc_taking_still_turning {
                                (
                                    sprite_anim_for_order(sprite, effective_anim, owner_is_pc),
                                    FrameProgression::FrozenFirstFrame,
                                )
                            } else if pc_target_still_turning {
                                (
                                    sprite_anim_for_order(sprite, effective_anim, owner_is_pc),
                                    FrameProgression::FrozenFirstFrame,
                                )
                            } else if owner_is_pc
                                && matches!(
                                    anim_type,
                                    OrderType::WaitingCape
                                        | OrderType::WaitingCapeAnonymousArcher
                                        | OrderType::WaitingHidden
                                )
                            {
                                // These PC idle arms explicitly use
                                // RHPROGRESSION_CYCLICALLY in the Original.
                                // WaitingCapeAnonymousArcher is a
                                // non-animation control token which plays the
                                // ordinary WaitingCape sprite, but retains the
                                // same non-terminating progression.
                                (
                                    sprite_anim_for_order(sprite, effective_anim, owner_is_pc),
                                    FrameProgression::Cyclically,
                                )
                            } else if let Some(animation) = requested_custom_animation {
                                let progression = match cur_command {
                                    Some(Command::PlayAnimLoop) => FrameProgression::Cyclically,
                                    Some(Command::PlayAnimFrozen) => {
                                        FrameProgression::FrozenLastFrame
                                    }
                                    _ => FrameProgression::Default,
                                };
                                (animation, progression)
                            } else if matches!(cur_command, Some(Command::PlayAnimLoop)) {
                                (
                                    sprite_anim_for_order(sprite, effective_anim, owner_is_pc),
                                    FrameProgression::Cyclically,
                                )
                            } else if matches!(cur_command, Some(Command::PlayAnimFrozen)) {
                                (
                                    sprite_anim_for_order(sprite, effective_anim, owner_is_pc),
                                    FrameProgression::FrozenLastFrame,
                                )
                            } else {
                                (
                                    sprite_anim_for_order(sprite, effective_anim, owner_is_pc),
                                    FrameProgression::Default,
                                )
                            };
                            if jump_ground_motion_step {
                                // This take-off / landing transition runs its
                                // sprite through the shared motion path, which
                                // seeds the goal and its increment itself.
                                let order_id = order_id.unwrap_or_else(|| {
                                    panic!(
                                        "jump transition {anim_type:?} for {entity_id:?} has no order id"
                                    )
                                });
                                let motion_order = crate::sprite::MotionOrderContext {
                                    order_id,
                                    destination: order_target,
                                    reverse: false,
                                    tolerance: order_tolerance,
                                    directional_tolerance: false,
                                    compute_direction: false,
                                    next_destination_same_action: None,
                                    target_element: order_antagonist,
                                };
                                return Some(super::jump::perform_jump_ground_motion(
                                    entity,
                                    sim,
                                    motion_order,
                                    played,
                                    row,
                                ));
                            }
                            if jump_airborne_step {
                                return Some(super::jump::perform_jump_airborne_motion(
                                    entity, sim, order_id, played, row,
                                ));
                            }
                            let elem = entity.element_data_mut();
                            let sprite = &mut elem.sprite;
                            Some(sprite.perform_action(
                                sim,
                                order_id,
                                played,
                                row,
                                progression,
                                false,
                            ))
                        });
                        // While still turning, the arm returns
                        // InProgress regardless of what the
                        // TURNING_ALERTED sprite reports — so the
                        // WaspStruggleCycle completion can't fire early.
                        if wasp_still_turning {
                            Some(MotionState::InProgress)
                        } else {
                            if matches!(anim_type, OrderType::DrinkingAle)
                                && matches!(sprite_motion, Some(MotionState::Done))
                                && drinking_ale_antagonist_inactive
                            {
                                Some(MotionState::Terminated)
                            } else {
                                sprite_motion
                            }
                        }
                    };
                    let motion = motion.map(|mut motion_state| {
                        if anim_type == OrderType::StrikingDownSword
                            && striking_down_sword_valid_after_perform == Some(false)
                        {
                            // RHelementactorhuman.cpp:4083-4092 performs this
                            // check after sprite advancement and returns before
                            // START/DONE side effects when it fails.
                            motion_state = MotionState::Terminated;
                        }
                        if anim_type == OrderType::StandingUpSword {
                            // RHElementActorHuman::Execute performs the
                            // stand-up sprite first, then re-aims at the live
                            // principal opponent (when swordfighting), then
                            // turns. Its soldier override performs none of
                            // this post-sprite facing work.
                            apply_standing_up_sword_post_perform_facing(
                                entity,
                                standing_up_sword_direction_goal,
                                entity_id,
                                self.control.frame_counter,
                            );
                        }
                        if !weak_sword_held {
                            apply_weak_sword_tiredness_after_perform(
                                entity,
                                anim_type,
                                &mut motion_state,
                            );
                        }
                        if order_is_initialising
                            && matches!(
                                anim_type,
                                OrderType::BeingWeakSword | OrderType::BeingStunnedSword
                            )
                        {
                            completion_outcomes
                                .execute_sides
                                .weak_stunned_start
                                .push((entity_id, anim_type));
                        }
                        motion_state
                    });

                    // Apply soldier-side per-anim-type side effects
                    // (posture/action-state transitions, attentive-flag
                    // toggling, view-status updates, bottle-hide /
                    // coin-pickup / remarks).  Runs every tick so
                    // START / DONE / TERMINATED get a chance to fire.
                    // Uses `anim_type` (the original order type) — not
                    // `effective_anim` — because the dispatch switch is
                    // keyed on the order's animation field, not the
                    // substituted one.
                    //
                    // `apply_npc_execute_side_effects` handles the
                    // cases inherited from the NPC parent class
                    // (SITTING / POINTING / SEARCHING pickpocket /
                    // TRANSITION_SITTING / BEGGAR_SHOWING_FACE) — it
                    // applies to both soldier and civilian NPCs.
                    if let Some(motion_state) = motion {
                        if entity.is_pc()
                            && anim_type == OrderType::TransitionCarryingCorpseWaitingUpright
                            && motion_state == MotionState::Terminated
                        {
                            completion_outcomes.corpse_drop_done.push(entity_id);
                        }
                        let special_remark_now = special_speech_id
                            .zip(special_sprite_before_perform)
                            .is_some_and(|(speech_id, before_perform)| {
                                let sprite = entity.sprite();
                                special_remark_due_for_execute(
                                    speech_id,
                                    before_perform,
                                    (sprite.current_frame, sprite.frame_count),
                                )
                            });
                        if special_remark_now {
                            completion_outcomes
                                .execute_sides
                                .special_remark
                                .push(entity_id);
                        }
                        apply_soldier_execute_side_effects(
                            entity,
                            anim_type,
                            motion_state,
                            antagonist,
                            entity_id,
                            &mut completion_outcomes.execute_sides,
                            &assets.profile_manager,
                        );
                        apply_npc_execute_side_effects(
                            entity,
                            anim_type,
                            motion_state,
                            antagonist,
                            entity_id,
                            &mut completion_outcomes.execute_sides,
                        );
                        // Universal walk/run Start handler — applies
                        // to all actor kinds (PC included).  Must run
                        // *before* `tick_entity_movement` on the next
                        // tick consults `is_moving()`, otherwise the
                        // actor walks-in-place.
                        apply_actor_walk_start_side_effect(entity, anim_type, motion_state);
                        super::jump::apply_jump_down_takeoff_drop(entity, anim_type, motion_state);
                        // Universal handlers (run for any actor type).
                        apply_active_animation_start_state_side_effect(
                            entity,
                            anim_type,
                            motion_state,
                        );
                        if forwards_pc_bow_action_on_start(
                            entity,
                            anim_type,
                            motion_state,
                            current_element_script_driven,
                        ) {
                            completion_outcomes
                                .execute_sides
                                .pc_bow_equip_action
                                .push(entity_id);
                        }
                        if entity.is_pc() && motion_state == MotionState::Done {
                            match anim_type {
                                OrderType::TransitionWaitingUprightSimulatingBeggar => {
                                    completion_outcomes
                                        .execute_sides
                                        .beggar_coin_flags
                                        .push((entity_id, true));
                                    completion_outcomes
                                        .execute_sides
                                        .beggar_wait_handoffs
                                        .push((entity_id, true));
                                }
                                OrderType::TransitionSimulatingBeggarWaitingUpright => {
                                    completion_outcomes
                                        .execute_sides
                                        .beggar_coin_flags
                                        .push((entity_id, false));
                                    completion_outcomes
                                        .execute_sides
                                        .beggar_wait_handoffs
                                        .push((entity_id, false));
                                }
                                OrderType::TransitionWaitingUprightHelpingClimbing => {
                                    completion_outcomes
                                        .execute_sides
                                        .pc_helping_climb_action
                                        .push(entity_id);
                                }
                                _ => {}
                            }
                        }
                        if entity.is_pc()
                            && matches!(
                                anim_type,
                                OrderType::TransitionCrouchingUp
                                    | OrderType::TransitionCrouchingDown
                            )
                            && matches!(motion_state, MotionState::Done | MotionState::Terminated)
                        {
                            completion_outcomes
                                .execute_sides
                                .stature_change_end
                                .push(entity_id);
                        }
                        apply_taking_net_side_effect(
                            anim_type,
                            motion_state,
                            antagonist,
                            entity_id,
                            &mut completion_outcomes.execute_sides,
                        );
                        apply_waking_up_done_side_effect(
                            anim_type,
                            motion_state,
                            antagonist,
                            entity_id,
                            &mut completion_outcomes.execute_sides,
                        );
                        apply_pc_taking_side_effect(
                            entity,
                            anim_type,
                            motion_state,
                            antagonist,
                            entity_id,
                            &mut completion_outcomes.execute_sides,
                        );
                        apply_pc_target_interaction_side_effect(
                            entity,
                            anim_type,
                            motion_state,
                            antagonist,
                            entity_id,
                            &mut completion_outcomes.execute_sides,
                        );
                        apply_sword_parry_side_effect(
                            entity,
                            anim_type,
                            motion_state,
                            principal_frames_from_now,
                        );
                        apply_under_net_cycle_side_effect(sim, entity, anim_type, motion_state);
                        apply_smalltalk_start_and_recovery_side_effect(
                            entity,
                            anim_type,
                            motion_state,
                            &assets.profile_manager,
                        );
                        apply_striking_down_sword_side_effect(
                            entity,
                            anim_type,
                            motion_state,
                            antagonist,
                            striking_down_sword_direction_goal,
                            entity_id,
                            &mut completion_outcomes.execute_sides,
                        );
                        apply_arrow_extraction_start_side_effect(entity, anim_type, motion_state);
                        apply_shield_transition_side_effect(entity, anim_type, motion_state);
                        if anim_type == OrderType::RaisingShield
                            && motion_state == MotionState::Done
                        {
                            crate::bow_shot::refresh_retained_shield_obstacle(
                                entity,
                                &assets.profile_manager,
                            );
                        }
                        apply_pc_disguise_exit_side_effect(
                            entity,
                            anim_type,
                            motion_state,
                            entity_id,
                            &mut completion_outcomes.execute_sides,
                        );
                        apply_standing_up_start_side_effect(entity, anim_type, motion_state);
                        apply_carried_start_side_effect(entity, anim_type, motion_state);
                        apply_falling_start_side_effect(entity, anim_type, motion_state);
                        apply_falling_completion_side_effect(entity, anim_type, motion_state);
                        apply_dying_start_side_effect(entity, anim_type, motion_state);
                        apply_being_dead_start_side_effect(entity, anim_type, motion_state);
                        apply_combat_injury_side_effect(
                            entity,
                            anim_type,
                            motion_state,
                            entity_id,
                            &mut combat_injury_terminated,
                        );
                        if matches!(motion_state, MotionState::Done) {
                            let strike = match anim_type {
                                OrderType::StrikingLeftSmalltalk
                                | OrderType::StrikingLowLeftSmalltalk => {
                                    Some(crate::weapons::SwordStrike::SmalltalkLeft)
                                }
                                OrderType::StrikingRightSmalltalk
                                | OrderType::StrikingLowRightSmalltalk => {
                                    Some(crate::weapons::SwordStrike::SmalltalkRight)
                                }
                                _ => None,
                            };
                            if let Some(strike) = strike {
                                completion_outcomes.execute_sides.smalltalk_strikes.push((
                                    entity_id,
                                    antagonist.expect(
                                        "smalltalk strike order must retain its antagonist",
                                    ),
                                    strike,
                                ));
                            }
                        }
                        if matches!(motion_state, MotionState::Done)
                            && let Some(crate::order::OrderCompletion::UnlockDoor { door_id }) =
                                order_completion
                        {
                            completion_outcomes.unlock_door_done.push(door_id);
                        }
                        // Lift sequence-element priority to
                        // NonInterruptable on initialisation for the
                        // always-non-interruptable anim families.
                        // Stage as a deferred sequence-element priority
                        // bump so we don't borrow the sequence manager
                        // mid-entity-loop.
                        if matches!(motion_state, MotionState::Start)
                            && anim_forces_non_interruptable_on_start(anim_type)
                        {
                            completion_outcomes
                                .non_interruptable_lifts
                                .push((seq_id, elem_idx));
                        }
                        if matches!(motion_state, MotionState::Terminated)
                            && cur_command == Some(Command::PlayAnimFreeze)
                        {
                            completion_outcomes.play_anim_frozen.push((
                                entity_id,
                                cur_command_level.unwrap_or(1),
                                requested_custom_animation.unwrap_or_else(|| {
                                    panic!(
                                        "PlayAnimFreeze for {entity_id:?} has no AnimationId property"
                                    )
                                }),
                            ));
                        }
                    }
                    // Dispatch the per-arm return decision, but retain the
                    // resulting base-Actor motion until the coordinator has
                    // drained synchronous derived-Execute callbacks. Those
                    // callbacks may replace this owner's live sequence
                    // element; Original TERMINATED uses that live pointer,
                    // while ABORTED uses the entry snapshot.
                    let is_npc = matches!(entity, Entity::Soldier(_) | Entity::Civilian(_));
                    let is_unconscious =
                        entity.human_data().map(|h| h.unconscious).unwrap_or(false);
                    let mut arm_ctx = ArmCtx {
                        entity_id,
                        is_npc,
                        is_unconscious,
                        seq_id,
                        elem_idx,
                        sequence_manager: &mut self.orders.sequence_manager,
                        next_order_id: &mut self.orders.next_order_id,
                        side_outcomes: &mut completion_outcomes.execute_sides,
                    };
                    let outcome =
                        motion.map(|m| dispatch_arm_completion(sim, anim_type, m, &mut arm_ctx));
                    // These Execute arms mutate the live RHOrder in place and
                    // call NewID rather than selecting another order. Mirror
                    // the changed object after the arm runs; manager
                    // selection alone cannot update the explicit mpOrder
                    // snapshot.
                    let mutated_installed_order = matches!(
                        anim_type,
                        OrderType::WaitingUprightBored
                            | OrderType::WaitingUprightBoredRandom
                            | OrderType::LyingStuckUnderNet
                            | OrderType::WriggleUnderNet
                    )
                    .then(|| {
                        arm_ctx
                            .sequence_manager
                            .get_element(seq_id, elem_idx)
                            .and_then(|element| element.current_order())
                            .map(|order| crate::element::InstalledActorOrder {
                                order_id: order.order_id,
                                order_type: order.order_type,
                            })
                    })
                    .flatten();
                    drop(arm_ctx);
                    if let Some(installed_order) = mutated_installed_order {
                        entity
                            .actor_data_mut()
                            .expect("in-place order mutation owner lost actor data")
                            .installed_order = Some(installed_order);
                    }
                    let effective_motion = match outcome.unwrap_or_else(|| {
                        panic!(
                            "actor {entity_id:?} {anim_type:?} produced no Execute motion at {seq_id:?}/{elem_idx}"
                        )
                    }) {
                        ExecuteOutcome::Forward(motion) => motion,
                        ExecuteOutcome::Consumed => MotionState::InProgress,
                    };
                    execute_result = Some(ActorExecuteResult {
                        order_type: anim_type,
                        entry_seq_id: seq_id,
                        entry_elem_idx: elem_idx,
                        motion: effective_motion,
                    });
                    break 'actor;
                }

                // No current order on the actor.  Dispatch only ever
                // runs from a present front order — when there is
                // none, the next tick's `Wait()` lazy-init creates one
                // and dispatch resumes.  Nothing to play this tick;
                // let `tick_melee_strikes` drive an active sweep if
                // any, and the newly-launched wait element will take
                // over next hourglass.
                let _ = direction;
                break 'actor;
            }
        }

        (
            combat_injury_terminated,
            completion_outcomes,
            execute_result,
        )
    }

    /// Dispatch per-frame animation sound triggers.
    ///
    /// Every animated element type runs a current-sound-id check
    /// during its tick: when the sprite's current frame has a
    /// non-zero sound ID, an FX sound is queued at the entity's
    /// position (with material for actors and projectiles, without
    /// for scenic FX/objects).
    ///
    /// Called once per tick, after [`EngineInner::tick_entity_movement`], the
    /// live actor-slot coordinator, and nonactor animation have advanced all
    /// sprite frames.
    pub(super) fn dispatch_frame_sounds(&mut self) {
        use crate::element::GameMaterial;
        use crate::sound_cache::Material;

        // Collect triggers during iteration so the sound manager mutation
        // isn't interleaved with mutable entity iteration.
        let mut triggers: Vec<(u32, crate::coordinates::MapPoint, Option<Material>)> = Vec::new();

        for (_, entity) in self.world.entities.occupied_mut() {
            if !entity.is_active() {
                continue;
            }

            // Actors (PC/NPC) and projectiles pass material; other
            // elements (FX/objects/ale/bonus/scroll/net) pass None.
            let wants_material = entity.actor_data().is_some() || entity.is_projectile();

            let elem = entity.element_data_mut();
            let sprite = &mut elem.sprite;

            let sound_id = sprite.current_sound_id();
            if sound_id == 0 {
                continue;
            }

            let material = if wants_material {
                // GameMaterial::LightShadow (10) has no sound-material
                // counterpart (Material enum only covers Ground..=Hole),
                // so treat it as material-less.
                match elem.material() {
                    GameMaterial::Ground => Some(Material::Ground),
                    GameMaterial::Wood => Some(Material::Wood),
                    GameMaterial::Stone => Some(Material::Stone),
                    GameMaterial::Grass => Some(Material::Grass),
                    GameMaterial::Leaves => Some(Material::Leaves),
                    GameMaterial::Water => Some(Material::Water),
                    GameMaterial::Bush => Some(Material::Bush),
                    GameMaterial::Ice => Some(Material::Ice),
                    GameMaterial::Hole => Some(Material::Hole),
                    GameMaterial::LightShadow => None,
                }
            } else {
                None
            };

            triggers.push((sound_id as u32, elem.position_map(), material));
        }

        for (fx_id, position, material) in triggers {
            self.feedback
                .pending_side_effects
                .sounds
                .push(super::SoundCommand::Fx {
                    fx_id,
                    position,
                    material,
                });
        }
    }
}

#[cfg(test)]
mod soldier_take_drink_parity_tests {
    use super::*;

    #[test]
    fn attentive_movement_uses_the_original_alerted_animation_family() {
        use OrderType as OT;

        let substitutions = [
            (OT::WalkingUpright, OT::WalkingAlerted),
            (
                OT::TransitionWalkingUprightWaitingUpright,
                OT::TransitionWalkingAlertedWaitingAlerted,
            ),
            (
                OT::TransitionRunningUprightWaitingUpright,
                OT::TransitionRunningAlertedWaitingAlerted,
            ),
            (
                OT::TransitionWaitingUprightWalkingUpright,
                OT::TransitionWaitingAlertedWalkingAlerted,
            ),
            (
                OT::TransitionWaitingUprightRunningUpright,
                OT::TransitionWaitingAlertedRunningAlerted,
            ),
            (
                OT::TransitionWalkingUprightRunningUpright,
                OT::TransitionWalkingAlertedRunningAlerted,
            ),
            (
                OT::TransitionRunningUprightWalkingUpright,
                OT::TransitionRunningAlertedWalkingAlerted,
            ),
            (OT::WalkingStairs, OT::WalkingStairsAlerted),
            (OT::Turning, OT::TurningAlerted),
        ];

        for (authored, effective) in substitutions {
            assert_eq!(
                soldier_movement_animation(authored, true, ActionState::Waiting),
                effective
            );
        }
        assert_eq!(
            soldier_movement_animation(OT::WalkingUpright, true, ActionState::MovingSword),
            OT::WalkingAlerted,
            "Original's sword-state check is only an assertion after entering the attentive branch"
        );
        assert_eq!(
            soldier_movement_animation(OT::WalkingStairs, true, ActionState::MovingFastSword),
            OT::WalkingStairs
        );
        assert_eq!(
            soldier_movement_animation(
                OT::TransitionWaitingUprightWalkingUpright,
                true,
                ActionState::MovingSword,
            ),
            OT::TransitionWaitingAlertedWalkingAlerted,
            "upright transition substitutions only test mbAttentive"
        );
        assert_eq!(
            soldier_movement_animation(OT::WalkingUpright, false, ActionState::Waiting),
            OT::WalkingUpright
        );
    }

    #[test]
    fn npc_taking_plays_searching_sprite_row_without_changing_pc_taking() {
        let sprite = crate::sprite::Sprite::default();

        assert_eq!(
            sprite_anim_for_order(&sprite, OrderType::Taking, false),
            OrderType::Searching
        );
        assert_eq!(
            sprite_anim_for_order(&sprite, OrderType::Taking, true),
            OrderType::Taking
        );
    }

    #[test]
    fn beggar_show_face_falls_back_to_rolling_only_when_the_animation_is_missing() {
        use crate::sprite_script::UNMAPPED;

        let missing = crate::sprite::Sprite::default();
        assert_eq!(
            sprite_anim_for_order(&missing, OrderType::BeggarShowingFace, false),
            OrderType::Rolling
        );

        let mut available = crate::sprite::Sprite::default();
        let mut conversion = vec![UNMAPPED; OrderType::BeggarShowingFace as usize + 1];
        conversion[OrderType::BeggarShowingFace as usize] = 0;
        available.conversion = std::sync::Arc::new(conversion);
        assert_eq!(
            sprite_anim_for_order(&available, OrderType::BeggarShowingFace, false),
            OrderType::BeggarShowingFace
        );
    }
}
