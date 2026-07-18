//! Animation ticking.

use super::*;
use crate::element::{ActionState, Command, Entity, EyeStatus, Posture};
use crate::entities::EntitySlots;
use crate::order::OrderCompletion;
use crate::sprite::{FrameProgression, MotionState};

const WEAKNESS_DISMISH: u16 = 5;

/// Return the "alerted" variant of an animation order type when a
/// soldier is attentive.  Each soldier-specific animation handler
/// substitutes the alerted variant at the top of its "attentive"
/// branch before delegating to action/motion playback.  Returns
/// `None` for animation types that have no alerted variant.
fn alerted_variant(anim: OrderType) -> Option<OrderType> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{ActorCivilian, ActorPc, ActorSoldier, ElementData, ElementKind, Entity};

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
    }

    #[test]
    fn sword_combat_injury_start_sets_waiting_sword() {
        let mut entity = weak_soldier_at_action_done(0);
        entity.actor_data_mut().unwrap().action_state = ActionState::Moving;

        apply_soldier_execute_side_effects(
            &mut entity,
            OrderType::BeingStunnedSword,
            MotionState::Start,
            None,
            EntityId::Pc(crate::entity_id::PcId(7)),
            &mut ExecuteSideOutcomes::default(),
            &crate::profiles::ProfileManager::default(),
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
        let mut outcomes = ExecuteSideOutcomes::default();

        apply_striking_down_sword_side_effect(
            &mut entity,
            OrderType::StrikingDownSword,
            MotionState::Start,
            Some(EntityId::Pc(crate::entity_id::PcId(9))),
            EntityId::Pc(crate::entity_id::PcId(7)),
            &mut outcomes,
        );

        assert_eq!(entity.element_data().posture, Posture::Upright);
        assert_eq!(
            entity.actor_data().unwrap().action_state,
            ActionState::WaitingSword
        );
        assert!(outcomes.killed_at_bottom.is_empty());

        apply_striking_down_sword_side_effect(
            &mut entity,
            OrderType::StrikingDownSword,
            MotionState::Done,
            Some(EntityId::Pc(crate::entity_id::PcId(9))),
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

    #[test]
    fn unconscious_hold_terminates_after_wakeup() {
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
            OrderType::BeingUnconsciousSword,
            MotionState::Terminated,
            &mut ctx,
        );

        assert!(matches!(outcome, ExecuteOutcome::Consumed));
    }

    #[test]
    fn unconscious_sword_start_sets_lying_waiting_sword() {
        let mut entity = weak_soldier_at_action_done(0);
        entity.actor_data_mut().unwrap().action_state = ActionState::Moving;

        apply_soldier_execute_side_effects(
            &mut entity,
            OrderType::BeingUnconsciousSword,
            MotionState::Start,
            None,
            EntityId::Pc(crate::entity_id::PcId(7)),
            &mut ExecuteSideOutcomes::default(),
            &crate::profiles::ProfileManager::default(),
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
        let mut entity = weak_soldier_at_action_done(0);
        entity.set_posture(Posture::Upright);
        entity.actor_data_mut().unwrap().action_state = ActionState::Moving;

        apply_under_net_cycle_side_effect(
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

        crate::sim_rng::with_seed(1, || {
            apply_under_net_cycle_side_effect(
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
                crate::sim_rng::with_seed(*seed, || {
                    crate::sim_rng::u32(crate::sim_rng::RngSite::NetWriggleGate, ..31) == 0
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

        let outcome = crate::sim_rng::with_seed(seed, || {
            dispatch_arm_completion(
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
fn soldier_is_attentive(entity: &Entity) -> bool {
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
    /// Humans whose weak/stunned sword animation just initialized.
    /// Adds the weak-stunned tidbit, transfers smalltalk initiative,
    /// and notifies soldier opponents.
    pub weak_stunned_start: Vec<EntityId>,
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
    /// Actors whose smalltalk sword strike reached its action-done tag.
    /// Plays a STRIKE_SWIPE FX when principal opponents are still
    /// mutually engaged.
    pub smalltalk_swipes: Vec<EntityId>,
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
}

/// Apply the soldier-specific side effects triggered on each motion-state
/// transition (Start, Done, Terminated) of an `active_ai_anim`
/// animation.  Covers the 42 animation cases: action-state/posture
/// transitions, attentive-flag toggling, view-status updates for
/// LOOKING_* anims, sleep/leaning-out/bow-lean transitions,
/// DRINKING_ALE / TAKING / SPECIAL / GETTING_FREE_FROM_WASP
/// antagonist-dependent effects.
///
/// Invoked from `tick_entity_animations` after each `perform_action`
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
    profile_manager: &crate::profiles::ProfileManager,
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

        // Sword combat-injury arms set sword waiting state on START.
        (OT::BeingHitSword | OT::ExtractingArrowSword | OT::BeingStunnedSword, MS::Start) => {
            set_states(entity, Posture::Upright, ActionState::WaitingSword);
        }

        // BEING_UNCONSCIOUS_*: START sets the settled lying state.
        // The dispatch arm itself keeps the order alive only while
        // `human.unconscious` is still true.
        (OT::BeingUnconsciousSword, MS::Start) => {
            set_states(entity, Posture::Lying, ActionState::WaitingSword);
        }
        (OT::BeingUnconsciousBow, MS::Start) => {
            set_states(entity, Posture::Lying, ActionState::AimingWithBow);
        }
        (OT::BeingUnconscious, MS::Start) => {
            set_states(entity, Posture::Lying, ActionState::Waiting);
        }

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

        // SPECIAL: most speakers fire the special-action remark on the
        // first frame of the animation.  The Halberdman variant fires
        // it instead at a specific eat-frame mid-animation.
        (OT::Special, _) => {
            const SPEECH_ID_HELBARDMAN: u32 = 0x4c484453;
            const EAT_FRAMES_HELBARDMAN: u16 = 40;
            let speech_id = entity
                .soldier_data()
                .and_then(|s| profile_manager.get_soldier(s.soldier_profile_index))
                .map(|p| p.exclamation_id)
                .unwrap_or(0);
            if speech_id == SPEECH_ID_HELBARDMAN {
                if entity.sprite().is_at_frame_number(EAT_FRAMES_HELBARDMAN) {
                    outcomes.special_remark.push(entity_id);
                }
            } else if matches!(motion, MS::Start) {
                outcomes.special_remark.push(entity_id);
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
        OT::WalkingUpright | OT::WalkingAlerted | OT::WalkingCrouched => {
            set_states(entity, Posture::Upright, ActionState::Moving);
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
        // Rolling fallback when the beggar doesn't have the
        // showing-face animation is checked at booking time (see
        // beggar ability), so nothing to do here.
        (OT::BeggarShowingFace, MS::Terminated) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
        }

        // POINTING: TERMINATED → (Upright, Waiting).
        // The booking site already sets the direction field; we only
        // restore the idle state when the point gesture finishes.
        (OT::Pointing, MS::Terminated) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
        }

        // WAITING_UPRIGHT: START → (Upright, Waiting).
        (OT::WaitingUpright, MS::Start) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
        }

        // WAITING_UPRIGHT_BORED: START → (Upright, Bored).
        // The BORED ↔ BORED_RANDOM cycle is handled at do_next_order
        // time (engine/mod.rs).
        (OT::WaitingUprightBored, MS::Start) => {
            set_states(entity, Posture::Upright, ActionState::Bored);
        }

        // TRANSITION_WAITING_UPRIGHT_BORED_WAITING_UPRIGHT:
        // DONE / TERMINATED → (Upright, Waiting).
        // This fires the moment a bored NPC gets interrupted
        // (an attentive-mode trigger arrives) so the subsequent WAIT
        // translate sees `action_state=Waiting` and picks
        // `WAITING_ALERTED`.  Without this, the actor stayed in
        // `action_state=Bored`, so the attentive-idle check in
        // tick.rs Command::Wait fell through to
        // `WAITING_UPRIGHT_BORED` — the soldier played the normal
        // idle pose during reactiontime instead of the alerted
        // lean-forward pose.
        (OT::TransitionWaitingUprightBoredWaitingUpright, MS::Done | MS::Terminated) => {
            set_states(entity, Posture::Upright, ActionState::Waiting);
        }

        // TRANSITION_WAITING_UPRIGHT_WAITING_UPRIGHT_BORED:
        // DONE / TERMINATED → (Upright, Bored).
        // Symmetrical to the arm above, but usually reached via the
        // idle random-bored-cycle path rather than an external
        // interrupt.
        (OT::TransitionWaitingUprightWaitingUprightBored, MS::Done | MS::Terminated) => {
            set_states(entity, Posture::Upright, ActionState::Bored);
        }

        // WAITING_UPRIGHT_BORED_RANDOM: START → (Upright, Bored),
        // plus officer-only eye-status.
        (OT::WaitingUprightBoredRandom, MS::Start) => {
            set_states(entity, Posture::Upright, ActionState::Bored);
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
    match (anim_type, motion) {
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

/// PC `Taking` Done handler — fires when a PC finishes the generic
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
        && matches!(anim_type, OrderType::Taking)
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
///  - `TRANSITION_WAITING_SWORD_PARRYING_SWORD_LOW`
///    → `TRANSITION_WAITING_SWORD_PARRYING_SWORD` — the `_LOW`
///    non-animation token re-uses the regular transition sprite anim.
///  - Soldier/NPC `TAKING` → `SEARCHING` (the order remains
///    `TAKING` so pickup side effects still dispatch there).
///  - `WAITING_CAPE_ANONYMOUS_ARCHER` → `WAITING_CAPE`.
///
/// Identity for everything else (most order types play their own
/// sprite anim).
fn sprite_anim_for_order(
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
        OT::FallingHitUpright | OT::FallingHitHarderUpright | OT::FallingPushedUpright => {
            OT::FallingBackUpright
        }
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

fn is_sword_movement_nonanimation(order: OrderType) -> bool {
    matches!(
        order,
        OrderType::WalkingWithSword | OrderType::RunningWithSword
    )
}

fn is_wall_anim(order: OrderType) -> bool {
    matches!(
        order,
        OrderType::TransitionWaitingUprightClimbingWallUp
            | OrderType::ClimbingWallUp
            | OrderType::ClimbingWallDown
            | OrderType::TransitionClimbingWallUpWaitingCrouched
            | OrderType::TransitionClimbingWallDownWaitingUpright
            | OrderType::TransitionWaitingCrouchedClimbingWallDown
            | OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
            | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
            | OrderType::ClimbingWallUpFast
            | OrderType::ClimbingWallDownFast
    )
}

fn is_ladder_anim(order: OrderType) -> bool {
    matches!(
        order,
        OrderType::ClimbingLadderUp
            | OrderType::ClimbingLadderDown
            | OrderType::ClimbingLadderUpFast
            | OrderType::ClimbingLadderDownFast
            | OrderType::TransitionWaitingUprightClimbingLadderUp
            | OrderType::TransitionClimbingLadderUpWaitingCrouched
            | OrderType::TransitionWaitingCrouchedClimbingLadderDown
            | OrderType::TransitionClimbingLadderDownWaitingUpright
            | OrderType::TransitionWaitingUprightClimbingLadderUpAlerted
            | OrderType::ClimbingLadderUpAlerted
            | OrderType::TransitionClimbingLadderUpWaitingUprightAlerted
            | OrderType::TransitionWaitingUprightClimbingLadderDownAlerted
            | OrderType::ClimbingLadderDownAlerted
            | OrderType::TransitionClimbingLadderDownWaitingUprightAlerted
    )
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

/// STRIKING_DOWN_SWORD sets sword-waiting state at Start and launches
/// GET_KILLED_AT_BOTTOM on the victim at the action-done tag.
fn apply_striking_down_sword_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
    antagonist: Option<EntityId>,
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

/// `ROLLING` on motion Start.
/// Reads `actor.pending_roll` (set by `try_queue_roll`) and installs an
/// `active_flight` toward the destination — equivalent to a walk-style
/// motion to the destination point.  The Rolling order itself doesn't
/// carry the destination through `Order`'s flight fields, so
/// `pending_roll` is the side channel — cleared once the flight is set
/// up so a second Rolling start can't reuse stale data.
fn apply_rolling_start_side_effect(
    entity: &mut Entity,
    anim_type: OrderType,
    motion: MotionState,
    next_order_id: &mut u32,
) {
    if anim_type != OrderType::Rolling || !matches!(motion, MotionState::Start) {
        return;
    }
    let dest = match entity.actor_data_mut().and_then(|a| a.pending_roll.take()) {
        Some(d) => d,
        None => return,
    };
    let cur_pos = entity.element_data().position_map();
    let dx = dest.x - cur_pos.x;
    let dy = dest.y - cur_pos.y;
    let facing = crate::position_interface::vector_to_sector_0_to_15(dx, dy);
    entity.element_data_mut().set_direction_instantly(facing);
    let rolling_ticks = entity
        .element_data()
        .sprite
        .total_ticks_for_anim(OrderType::Rolling);
    let frames = if rolling_ticks > 1 { rolling_ticks } else { 8 };
    let _ = next_order_id; // reserved for future use (per-tick id tracking)
    if let Some(actor) = entity.actor_data_mut() {
        actor.active_flight = Some(crate::element::ActiveFlight {
            increment_x: dx / frames as f32,
            increment_y: dy / frames as f32,
            goal_x: dest.x,
            goal_y: dest.y,
            frames_remaining: frames,
            // The rolling dispatch does not apply a domino-knockdown
            // effect to other actors.
            antagonist: None,
            ..Default::default()
        });
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
/// | `FALLING_LADDER_WALL` | TERMINATED | Dead/Lying/Upright | (unchanged) |
fn fall_landing_states(
    anim_type: OrderType,
    is_dead: bool,
    is_unconscious: bool,
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
        OrderType::FallingLadderWall => Some((
            if is_dead {
                Posture::Dead
            } else if is_unconscious {
                Posture::Lying
            } else {
                Posture::Upright
            },
            None,
        )),
        _ => None,
    }
}

/// Which `MotionState` triggers `fall_landing_states` for `anim_type`?
/// State is set at varying points per family:
/// - `FALLING_BACK_UPRIGHT`/`CROUCHED`: Start.
/// - `FALLING_BACK_SWORD`/`BOW`: Done or Terminated.
/// - `FALLING_HIT_*` / `FALLING_SHOULDERS` / `ROLLING` / `FALLING_LADDER_WALL`:
///   Terminated.
fn fall_state_trigger_matches(anim_type: OrderType, motion: MotionState) -> bool {
    match anim_type {
        OrderType::FallingBackUpright | OrderType::FallingBackCrouched => {
            matches!(motion, MotionState::Start)
        }
        OrderType::FallingBackSword | OrderType::FallingBackBow => {
            matches!(motion, MotionState::Done | MotionState::Terminated)
        }
        // FallingHit sets only on TERMINATED, not DONE.
        // Rolling, FallingLadderWall, FallingShoulders all the same.
        _ => matches!(motion, MotionState::Terminated),
    }
}

/// `FALLING_HIT_*` / `FALLING_PUSHED_*` on motion Start.
///
/// legacy implementation `ExecuteFallingHit` enters `(Flying, Moving)`; legacy implementation
/// `ExecuteFallingPushed` enters `(Flying, WaitingSword)` before the
/// wrapper later restores the variant-specific action on termination.
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
            | OrderType::FallingHitHarderUpright
            | OrderType::FallingHitHarderWithBow
            | OrderType::FallingHitHarderWithSword
            | OrderType::FallingHitHarderCrouched
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
    let is_dead = entity.is_dead();
    let is_unconscious = entity.human_data().map(|h| h.unconscious).unwrap_or(false);
    if let Some((posture, action_state)) = fall_landing_states(anim_type, is_dead, is_unconscious) {
        entity.set_posture(posture);
        if let Some(action) = action_state
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.action_state = action;
        }
    }
}

/// Soldier combat-injury anims (`BEING_HIT_SWORD`,
/// `EXTRACTING_ARROW_SWORD`, `BEING_WEAK_SWORD`, `BEING_STUNNED_SWORD`)
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
        // DONE → ParryingShield (re-asserts the state set at dispatch
        // time so other systems see it consistently).
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
            match crate::sim_rng::u32(crate::sim_rng::RngSite::WriggleDirection, ..3) {
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
        // Idle loops — always-InProgress WAITING_* arms.
        OT::WaitingUpright
            | OT::WaitingCrouched
            | OT::WaitingAlerted
            | OT::WaitingSword
            | OT::WaitingShield
            | OT::WaitingOnShoulders
            | OT::WaitingCape
            | OT::WaitingHidden
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
    anim_type: OrderType,
    motion: MotionState,
    ctx: &mut ArmCtx<'_>,
) -> ExecuteOutcome {
    use crate::order::OrderType as OT;
    use crate::sprite::MotionState as MS;

    // BORED ↔ RANDOM idle cycle — always Consumed.  Terminated
    // rerolls the animation type + NewID on `Command::Wait` only
    // (not `WaitTimer`).
    if matches!(
        anim_type,
        OT::WaitingUprightBored | OT::WaitingUprightBoredRandom
    ) {
        if matches!(motion, MS::Terminated) {
            let is_wait_cmd = ctx
                .sequence_manager
                .get_element(ctx.seq_id, ctx.elem_idx)
                .map(|el| matches!(el.command, crate::element::Command::Wait))
                .unwrap_or(false);
            if is_wait_cmd {
                let next_type = match anim_type {
                    OT::WaitingUprightBored => {
                        if crate::sim_rng::u32(crate::sim_rng::RngSite::BoredAnimationChoice, ..10)
                            == 0
                        {
                            OT::WaitingUprightBoredRandom
                        } else {
                            OT::WaitingUprightBored
                        }
                    }
                    OT::WaitingUprightBoredRandom => OT::WaitingUprightBored,
                    _ => unreachable!(),
                };
                if let Some(elem) = ctx
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
        if crate::sim_rng::u32(crate::sim_rng::RngSite::NetWriggleGate, ..31) == 0 {
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

/// Side-effects collected by `tick_entity_animations` when an order's
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
    /// `(door_id, seq_id, elem_idx)` — flip `locked_pc` then terminate.
    pub unlock_door: Vec<(crate::gate::DoorIndex, crate::sequence::SequenceId, usize)>,
    /// Entities whose door-pass chain should continue.
    pub resume_door_pass: Vec<EntityId>,
    /// Entities whose active jump should advance to the next step.
    pub next_jump_step: Vec<EntityId>,
    /// C++ `PLAY_CUSTOM_FREEZE` launches a follow-up
    /// `PLAY_ANIM_FROZEN` sequence when the animation terminates, so
    /// the actor holds the last frame as a separate in-progress
    /// element.
    pub play_anim_frozen: Vec<(EntityId, u16, OrderType)>,
    /// Soldier-style side effects that touch other entities —
    /// accumulated from `apply_soldier_execute_side_effects`.
    pub execute_sides: ExecuteSideOutcomes,
}

impl EngineInner {
    /// Tick animations.
    ///
    /// Returns two lists:
    /// - `ai_anim_done`: entities whose `active_ai_anim` completed this
    ///   frame (need EventDone dispatch).
    /// - `combat_injury_terminated`: entities whose combat-hit / extracting-
    ///   arrow / being-weak / being-stunned `combat_anim` just terminated
    ///   (need EventAfterCombatInjury dispatch for soldiers).
    pub(super) fn tick_entity_animations(
        &mut self,
        assets: &crate::engine::types::LevelAssets,
    ) -> (Vec<EntityId>, Vec<EntityId>, AnimCompletionOutcomes) {
        if self.actors_frozen() {
            return (Vec::new(), Vec::new(), AnimCompletionOutcomes::default());
        }

        let ai_anim_done: Vec<EntityId> = Vec::new();
        let mut combat_injury_terminated: Vec<EntityId> = Vec::new();
        // Sequence elements whose priority should be lifted to
        // `NonInterruptable` because their currently-driven anim hit
        // motion Start and is one of the always-non-interruptable
        // families (see `anim_forces_non_interruptable_on_start`).
        // Applied after the entity loop so we don't double-borrow
        // `self.orders.sequence_manager` while iterating `self.world.entities`.
        let mut non_interruptable_lifts: Vec<(crate::sequence::SequenceId, usize)> = Vec::new();
        let mut completion_outcomes = AnimCompletionOutcomes::default();

        // Snapshot patch (applied, in_transition) states before entity
        // iteration. We need this to decide Reversed vs Default progression
        // for patch FX entities, but cannot borrow the canonical patch domain
        // during the mutable entity loop.
        let patch_states: Vec<(bool, bool)> = self
            .scripts
            .mission
            .as_ref()
            .map(|_| {
                self.script_domains
                    .interactables
                    .patches
                    .iter()
                    .map(|p| (p.applied, p.in_transition))
                    .collect()
            })
            .unwrap_or_default();
        let mut frames_from_now_till_action_done =
            EntitySlots::filled(self.world.entities.len(), None);
        let mut active_entity_flags = EntitySlots::filled(self.world.entities.len(), false);
        let mut door_pass_crenel_transition_dirs =
            EntitySlots::filled(self.world.entities.len(), None);
        for (entity_id, entity) in self.world.entities.occupied() {
            let sprite = &entity.element_data().sprite;
            frames_from_now_till_action_done[entity_id] =
                safe_frames_from_now_till_action_done(sprite);
            active_entity_flags[entity_id] = entity.is_active();
            door_pass_crenel_transition_dirs[entity_id] = (|| {
                let dp = entity.actor_data()?.active_door_pass.as_ref()?;
                let reverse_direction = match dp.current_action {
                    OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel => false,
                    OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel => true,
                    _ => return None,
                };
                let sector_in = self
                    .scripts
                    .mission
                    .as_ref()
                    .and_then(|_| {
                        self.script_domains
                            .interactables
                            .doors
                            .get(usize::from(dp.door_index))
                    })
                    .map(|door| door.sector_in)?;
                let direction = self
                    .world
                    .fast_grid
                    .level
                    .sector_number_map
                    .get(&crate::sector::SectorNumber::new(i16::from(sector_in)))
                    .and_then(|&idx| self.world.fast_grid.level.sectors.get(idx))
                    .and_then(|sector| {
                        if sector.lift_type == Some(crate::sector::LiftType::Wall) {
                            Some(sector.lift_direction)
                        } else {
                            None
                        }
                    })?;
                Some(if reverse_direction {
                    (direction + 8) & 15
                } else {
                    direction
                })
            })();
        }

        // Collect patch indices whose transitions completed this tick.
        // Processed after the entity loop to avoid borrowing conflicts.
        let mut completed_patch_transitions: Vec<crate::patch::PatchIndex> = Vec::new();

        for (entity_id, entity) in self.world.entities.occupied_mut() {
            if !entity.is_active() {
                continue;
            }

            // Actors: animate based on current action state
            if let Some(actor) = entity.actor_data() {
                // Moving actors are animated in tick_entity_movement(),
                // which computes per-frame combat directional anims
                // (WalkingSword / StrafingRightSword / …) for the
                // sword/shield variants rather than using the Move
                // element's order.action (which is the logical
                // `WalkingWithSword`, unmapped in PC sprite profiles).
                // Keep this gate aligned with tick_entity_movement's
                // own is_moving / sword / shield guard
                // (movement.rs:2210).
                if actor.action_state.is_moving()
                    || matches!(
                        actor.action_state,
                        crate::element::ActionState::MovingSword
                            | crate::element::ActionState::MovingFastSword
                            | crate::element::ActionState::MovingShield
                    )
                {
                    continue;
                }

                if actor.execution_frozen {
                    continue;
                }

                // Active melee strike: `tick_melee_strikes` drives the
                // strike animation + hit timing itself via
                // `sprite.perform_action`, reading the stamped id from
                // `active_melee.order_id`.  If this generic animation
                // driver *also* calls `perform_action` against the
                // same sprite on the same tick with the Order-side id,
                // the sprite's `last_processed_order_id` thrashes
                // between the two drivers and the swing wedges at
                // `MotionState::Start` forever — which is exactly the
                // strike-animation-stuck bug from the replay.
                if actor.active_melee.is_active() {
                    continue;
                }
                if actor.active_shot.is_active() {
                    continue;
                }

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
                let (order_seq_elem, anim_type, order_id, antagonist, completion) =
                    if let Some((seq_id, elem_idx, order)) = order_snapshot {
                        (
                            Some((seq_id, elem_idx)),
                            order.order_type,
                            Some(order.order_id),
                            order.antagonist,
                            order.completion.clone(),
                        )
                    } else {
                        (
                            None,
                            crate::order::OrderType::Invalid,
                            None,
                            None,
                            OrderCompletion::AdvanceElement,
                        )
                    };

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
                let driving_one_shot = matches!(cur_command, Some(cmd) if cmd != Command::Wait && cmd != Command::WaitTimer);
                let settled_dead_or_ko_hold = matches!(
                    anim_type,
                    OrderType::BeingDead
                        | OrderType::BeingDeadFallenBack
                        | OrderType::BeingDeadSword
                        | OrderType::BeingDeadBow
                        | OrderType::BeingDeadFallenBackSword
                        | OrderType::BeingDeadFallenBackBow
                        | OrderType::BeingUnconscious
                        | OrderType::BeingUnconsciousSword
                        | OrderType::BeingUnconsciousBow
                );

                // Dead entities: freeze on last frame unless a
                // combat one-shot (DYING_*) is still playing.  The
                // dispatch keeps driving DYING_SWORD even after
                // `is_dead` is true — that's where `set_posture(Dead)`
                // runs, and the animation needs to play to visible
                // completion before the corpse settles.  Skipping too
                // early here meant killed enemies snapped to the
                // ground / held their last pose without ever falling.
                if entity.is_dead() && !driving_one_shot && !settled_dead_or_ko_hold {
                    continue;
                }

                // Unconscious entities: freeze on last frame unless a
                // combat one-shot (falling-hit / KO fall) is still
                // playing.  The dispatch keeps driving FALLING_HIT_*
                // even while `unconscious` is set; once the fall
                // terminates, the entity lies still.  Skipping too
                // early here meant KO'd NPCs snapped to the ground
                // without ever playing the fall animation.
                if entity.human_data().map(|h| h.unconscious).unwrap_or(false)
                    && !driving_one_shot
                    && !settled_dead_or_ko_hold
                {
                    continue;
                }

                // AI-driven animation (Pointing, RaisingShield, dying,
                // falling-hit, BORED idle cycle, …)?  Drive it via
                // perform_action; on completion, run side-effect
                // helpers and fire the bound `OrderCompletion`.
                if let Some((seq_id, elem_idx)) = order_seq_elem {
                    if matches!(
                        cur_command,
                        Some(Command::Move | Command::MoveOk | Command::Seek | Command::PassDoor)
                    ) && is_sword_movement_nonanimation(anim_type)
                    {
                        // Movement elements are owned by
                        // tick_entity_movement.  WALKING_WITH_SWORD /
                        // RUNNING_WITH_SWORD are driven through the
                        // motion path (via face-opponent), not through
                        // the generic action-animation path.  If an
                        // in-progress movement element is still
                        // visible here while the actor state has
                        // already left Moving, do not try to play its
                        // logical movement token as a sprite row.
                        continue;
                    }
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
                    let mut weak_sword_held = false;
                    let owner_is_pc = entity.is_pc();
                    let order_is_initialising = order_id.is_some_and(|oid| {
                        entity.element_data().sprite.last_processed_order_id != oid.get()
                    });
                    if order_is_initialising
                        && matches!(
                            anim_type,
                            OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
                                | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
                        )
                        && let Some(direction) = door_pass_crenel_transition_dirs
                            .get(entity_id)
                            .copied()
                            .flatten()
                    {
                        entity.element_data_mut().set_direction_instantly(direction);
                        entity.set_posture(crate::element::Posture::Flying);
                    }
                    let drinking_ale_antagonist_inactive =
                        matches!(anim_type, OrderType::DrinkingAle)
                            && antagonist.is_some_and(|a| {
                                !active_entity_flags.get(a).copied().unwrap_or(false)
                            });
                    let motion = if is_turn {
                        // Play the turn sprite animation (alerted
                        // variant for attentive soldiers) at the
                        // current direction row, then step the body
                        // rotation via `turn_fast()`.  Resulting motion
                        // state is Terminated iff the rotation reached
                        // its goal this tick.
                        let sprite_motion = {
                            let sprite = &mut entity.element_data_mut().sprite;
                            sprite.perform_action(
                                order_id,
                                effective_anim,
                                direction,
                                FrameProgression::Default,
                                false,
                            )
                        };
                        let still_turning = entity.position_iface_mut().turn_fast();
                        // PI is the single source of truth for direction —
                        // no sync needed now that `ElementData.direction`
                        // is gone.
                        // Swallow `sprite_motion` — its Done/Terminated
                        // is driven by `action_done_frame` in sprite
                        // data, but the control flow here ties
                        // completion to `turn_fast()` instead.
                        let _ = sprite_motion;
                        if still_turning {
                            Some(MotionState::InProgress)
                        } else {
                            Some(MotionState::Terminated)
                        }
                    } else if matches!(anim_type, OrderType::DrinkingAle)
                        && order_is_initialising
                        && drinking_ale_antagonist_inactive
                    {
                        Some(MotionState::Terminated)
                    } else {
                        // Many per-anim handlers call `Turn()` each
                        // tick so the body keeps rotating toward the
                        // direction goal *while* the action animation
                        // plays.  Anims with explicit `Turn()` calls:
                        //   TRANSITION_RAISING_SWORD, TRANSITION_LOWERING_SWORD,
                        //   WAITING_SWORD, PARRYING_SWORD,
                        //   STRIKING_LOW_LEFT_SMALLTALK and the matching
                        //   strike/parry smalltalk family,
                        //   STRIKING_DOWN_SWORD,
                        //   STANDING_UP_SWORD/BOW,
                        //   EXTRACTING_ARROW_SWORD, RAISING_SHIELD,
                        //   ROLLING, TAKING_NET,
                        //   TAKING, DRINKING_ALE,
                        //   BEING_UNCONSCIOUS_SWORD,
                        //   TRANSITION_CARRYING_CORPSE_WAITING_UPRIGHT.
                        // Step the rotation here, then sync `element.direction`
                        // to match before the sprite picks the row to play.
                        let needs_turn = matches!(
                            anim_type,
                            OrderType::TransitionRaisingSword
                                | OrderType::TransitionLoweringSword
                                | OrderType::WaitingSword
                                | OrderType::ParryingSword
                                | OrderType::ParryingLowSword
                                | OrderType::StrikingLowLeftSmalltalk
                                | OrderType::StrikingLowRightSmalltalk
                                | OrderType::StrikingLeftSmalltalk
                                | OrderType::StrikingRightSmalltalk
                                | OrderType::ParryingLeftSmalltalk
                                | OrderType::ParryingRightSmalltalk
                                | OrderType::ParryingLowLeftSmalltalk
                                | OrderType::ParryingLowRightSmalltalk
                                | OrderType::StrikingDownSword
                                | OrderType::StandingUpSword
                                | OrderType::StandingUpBow
                                | OrderType::ExtractingArrowSword
                                | OrderType::RaisingShield
                                | OrderType::Rolling
                                | OrderType::Taking
                                | OrderType::DrinkingAle
                                | OrderType::TakingNet
                                | OrderType::BeingUnconsciousSword
                                | OrderType::TransitionCarryingCorpseWaitingUpright
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
                                // POINTING is booked with
                                // set_direction_instantly so turn() is a no-op;
                                // SEARCHING has no direction_goal writer yet
                                // but the parity slot is required.
                                | OrderType::Pointing
                                | OrderType::Searching
                        );
                        // Capture `Turn()`'s return for the GETTING_FREE_FROM_WASP
                        // still-turning substitution: while still
                        // turning, play TURNING_ALERTED and return
                        // InProgress; otherwise delegate to the
                        // configured animation.
                        let mut wasp_still_turning = false;
                        if needs_turn {
                            let still_turning = entity.position_iface_mut().turn();
                            if matches!(anim_type, OrderType::GettingFreeFromWasp) {
                                wasp_still_turning = still_turning;
                            }
                        }
                        let row = entity.element_data().direction() as u16;
                        let held_weak_sword = hold_weak_sword_at_action_done(entity, anim_type);
                        if held_weak_sword.is_some() {
                            weak_sword_held = true;
                        }
                        let door_pass_action = entity
                            .actor_data()
                            .and_then(|actor| actor.active_door_pass.as_ref())
                            .map(|dp| dp.current_action);
                        let sprite_motion = held_weak_sword.or_else(|| {
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
                            let posture = elem.posture;
                            let (played, progression) = if wasp_still_turning {
                                (OrderType::TurningAlerted, FrameProgression::Default)
                            } else if effective_anim == OrderType::Freezing {
                                match (posture, door_pass_action) {
                                    (crate::element::Posture::OnWall, _) => {
                                        (OrderType::ClimbingWallUp, FrameProgression::Frozen)
                                    }
                                    (crate::element::Posture::OnLadder, _) => {
                                        (OrderType::ClimbingLadderUp, FrameProgression::Frozen)
                                    }
                                    (_, Some(action)) if is_wall_anim(action) => {
                                        (OrderType::ClimbingWallUp, FrameProgression::Frozen)
                                    }
                                    (_, Some(action)) if is_ladder_anim(action) => {
                                        (OrderType::ClimbingLadderUp, FrameProgression::Frozen)
                                    }
                                    _ => (
                                        OrderType::WaitingUpright,
                                        FrameProgression::FrozenFirstFrame,
                                    ),
                                }
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
                            Some(sprite.perform_action(order_id, played, row, progression, false))
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
                        if !weak_sword_held {
                            apply_weak_sword_tiredness_after_perform(
                                entity,
                                anim_type,
                                &mut motion_state,
                            );
                        }
                        if matches!(motion_state, MotionState::Start)
                            && matches!(
                                anim_type,
                                OrderType::BeingWeakSword | OrderType::BeingStunnedSword
                            )
                        {
                            completion_outcomes
                                .execute_sides
                                .weak_stunned_start
                                .push(entity_id);
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
                        let principal_frames_from_now = entity
                            .human_data()
                            .and_then(|h| h.opponents.first().copied())
                            .and_then(|opponent| {
                                frames_from_now_till_action_done
                                    .get(opponent)
                                    .copied()
                                    .flatten()
                            });
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
                        // Universal handlers (run for any actor type).
                        apply_active_animation_start_state_side_effect(
                            entity,
                            anim_type,
                            motion_state,
                        );
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
                        apply_under_net_cycle_side_effect(entity, anim_type, motion_state);
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
                            entity_id,
                            &mut completion_outcomes.execute_sides,
                        );
                        apply_arrow_extraction_start_side_effect(entity, anim_type, motion_state);
                        apply_shield_transition_side_effect(entity, anim_type, motion_state);
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
                        apply_rolling_start_side_effect(
                            entity,
                            anim_type,
                            motion_state,
                            &mut self.orders.next_order_id,
                        );
                        apply_combat_injury_side_effect(
                            entity,
                            anim_type,
                            motion_state,
                            entity_id,
                            &mut combat_injury_terminated,
                        );
                        if matches!(motion_state, MotionState::Done)
                            && matches!(
                                anim_type,
                                OrderType::StrikingLeftSmalltalk
                                    | OrderType::StrikingRightSmalltalk
                                    | OrderType::StrikingLowLeftSmalltalk
                                    | OrderType::StrikingLowRightSmalltalk
                            )
                        {
                            completion_outcomes
                                .execute_sides
                                .smalltalk_swipes
                                .push(entity_id);
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
                            non_interruptable_lifts.push((seq_id, elem_idx));
                        }
                        if matches!(motion_state, MotionState::Terminated)
                            && cur_command == Some(Command::PlayAnimFreeze)
                        {
                            completion_outcomes.play_anim_frozen.push((
                                entity_id,
                                cur_command_level.unwrap_or(1),
                                anim_type,
                            ));
                        }
                    }
                    // Dispatch the per-arm return decision, then apply
                    // `Hourglass`-style semantics to the forwarded
                    // motion state (TERMINATED → advance, ABORTED →
                    // set Impossible, DONE / START / InProgress →
                    // no-op).
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
                        motion.map(|m| dispatch_arm_completion(anim_type, m, &mut arm_ctx));
                    let forwarded = match outcome {
                        Some(ExecuteOutcome::Forward(m)) => Some(m),
                        Some(ExecuteOutcome::Consumed) | None => None,
                    };
                    // ABORTED: set sequence element to Impossible.
                    // The Hourglass dispatch flips state to Impossible
                    // with a non-interruptable-priority assertion —
                    // matched by `element_impossible`'s own guard.
                    if matches!(forwarded, Some(MotionState::Aborted)) {
                        completion_outcomes.seq_impossible.push((seq_id, elem_idx));
                    }
                    // TERMINATED: advance via OrderCompletion.
                    let advance = matches!(forwarded, Some(MotionState::Terminated));
                    if advance {
                        match completion {
                            OrderCompletion::AdvanceElement => {
                                // Default path: `do_next_order` pops this
                                // order and advances the owning element
                                // (or terminates on empty, auto-relaunching
                                // a wait element).
                                completion_outcomes.seq_advance.push((seq_id, elem_idx));
                            }
                            OrderCompletion::UnlockDoor { door_id } => {
                                completion_outcomes
                                    .unlock_door
                                    .push((door_id, seq_id, elem_idx));
                            }
                            OrderCompletion::ResumeDoorPass => {
                                completion_outcomes.resume_door_pass.push(entity_id);
                            }
                            OrderCompletion::NextJumpStep => {
                                completion_outcomes.next_jump_step.push(entity_id);
                            }
                            OrderCompletion::WaspStruggleCycle { cycles_remaining } => {
                                if cycles_remaining <= 1 {
                                    // Last cycle just finished — terminate
                                    // the sequence element so wasp-victim
                                    // cleanup and EVENT_WASP_AWAY fire.
                                    completion_outcomes.seq_terminate.push((seq_id, elem_idx));
                                } else {
                                    // Re-arm the next struggle cycle.
                                    // The original engine inserts
                                    // bee_time discrete orders; we
                                    // model that by re-pushing a new
                                    // order with a decremented counter
                                    // after the loop finishes.
                                    completion_outcomes.wasp_next_cycle.push((
                                        seq_id,
                                        elem_idx,
                                        cycles_remaining - 1,
                                    ));
                                }
                            }
                        }
                    }
                    continue;
                }

                // No current order on the actor.  Dispatch only ever
                // runs from a present front order — when there is
                // none, the next tick's `Wait()` lazy-init creates one
                // and dispatch resumes.  Nothing to play this tick;
                // let `tick_melee_strikes` drive an active sweep if
                // any, and the newly-launched wait element will take
                // over next hourglass.
                let _ = direction;
                continue;
            }

            // FX entities: frame advance. Mobile children freeze with their
            // stopped master (RHElementFXMasked::Hourglass checks
            // IsStopped before advancing). Patch FX entities need
            // special handling: reversed playback during unapply
            // transitions, and final patch effects on completion.
            if entity.is_fx() {
                if let Entity::Fx(fx) = entity
                    && let Some(mobile_index) = fx.fx.mobile_index
                {
                    let stopped = self
                        .world
                        .mobile_elements
                        .get(usize::from(mobile_index))
                        .unwrap_or_else(|| {
                            panic!("mobile FX references missing master {mobile_index}")
                        })
                        .stopped;
                    if !stopped {
                        fx.element
                            .sprite
                            .increment_frame_modulated(fx.fx.animation_speed);
                    }
                    continue;
                }
                // Check if this is a patch FX entity.
                let patch_idx = match entity {
                    Entity::Fx(fx) => fx.fx.patch_index,
                    _ => None,
                };

                if let Some(pidx) = patch_idx {
                    // Patch FX progression depends on transition state.
                    let (applied, in_transition) = patch_states
                        .get(usize::from(pidx))
                        .copied()
                        .unwrap_or((false, false));

                    // If applied && in_transition → REVERSED, else DEFAULT.
                    let progression = if applied && in_transition {
                        FrameProgression::Reversed
                    } else {
                        FrameProgression::Default
                    };

                    let motion = {
                        let elem = entity.element_data_mut();
                        elem.sprite.perform_virgin_increment(progression)
                    };

                    // When transition animation finishes, queue the
                    // patch's final-apply effects.
                    if matches!(motion, MotionState::Terminated) && in_transition {
                        completed_patch_transitions.push(pidx);
                    }
                } else {
                    // Non-patch FX / Target: simple frame advance.
                    // Placeholder sprites no-op via the
                    // `increment_frame` empty-scripts guard.
                    //
                    // Target entities honour their stamped
                    // `TargetData.progression` so scripts that
                    // recorded `PlayAnimLoop` (→ Cyclically) or
                    // `PlayAnimFreeze` (→ FreezeWhenTerminated) keep
                    // cycling or hold the last frame instead of
                    // silently running with `Default`.
                    let progression = if let Entity::Target(t) = entity {
                        FrameProgression::from_ordinal(t.target.progression)
                    } else {
                        FrameProgression::Default
                    };
                    entity
                        .element_data_mut()
                        .sprite
                        .perform_virgin_increment(progression);
                }
                continue;
            }

            // Purses / coins use distinct progression flags depending
            // on flight phase:
            //   - flying  → `SkipShadow` (drops the trailing shadow
            //     frame so the animation stays tight).
            //   - landed  → switch to `OBJECT_BURSTING` and freeze on
            //     the last frame via `FreezeWhenTerminated`.
            // The default `Cyclically` flag would loop the bursting
            // animation forever instead of holding the empty-pouch
            // frame, so handle these projectiles explicitly here.
            if let Entity::Projectile(p) = entity
                && matches!(
                    p.object.object_type,
                    crate::element::ObjectType::Purse | crate::element::ObjectType::Coin
                )
            {
                let progression = if p.projectile.flying {
                    FrameProgression::SkipShadow
                } else {
                    FrameProgression::FreezeWhenTerminated
                };
                p.element.sprite.perform_virgin_increment(progression);
                continue;
            }

            // Other non-actor entities: Bonus (including Cape, which
            // advances with the default progression — frame state
            // matches `Cyclically` here), Scroll (advances the sprite
            // with the default progression in the same block that
            // fires the per-25-tick script Hourglass — the script
            // side is handled separately in
            // `dispatch_scroll_hourglasses`), Mobile, Net, and
            // projectiles without a custom progression branch above.
            // All of them want a plain cyclical frame advance;
            // `MotionState::Terminated` isn't consulted for any of
            // these element kinds, so calling the cheaper
            // `increment_frame` is equivalent to a default-progression
            // virgin increment for the sprite state.
            entity
                .element_data_mut()
                .sprite
                .increment_frame(FrameProgression::Cyclically);
        }

        // Process completed patch transitions (deferred to avoid borrow
        // conflicts during entity iteration).  Clear the in-transition
        // flag and apply the patch's final effects.
        for patch_idx in completed_patch_transitions {
            let effects = {
                if self.scripts.mission.is_none() {
                    continue;
                }
                let patch = match self
                    .script_domains
                    .interactables
                    .patches
                    .get_mut(usize::from(patch_idx))
                {
                    Some(p) => p,
                    None => continue,
                };
                patch.in_transition = false;
                patch.apply_final(false)
            };

            // Advance the gate-state machine of any door whose patch
            // just finished transitioning.  The drawbridge / portcullis
            // visual is driven by the patch FX entity; `gate_state`
            // is bookkeeping that mirrors that visual so downstream
            // code (pathfinding, AI planning) can query an accurate
            // passable/impassable answer instead of inspecting patch
            // internals.  "Applied" == "Open"; the state changes
            // atomically at apply-final time.
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

            self.process_patch_effects(assets, patch_idx, effects);
        }

        // Drain `non_interruptable_lifts` after the entity loop ends.
        // Lifts the sequence-element priority to NonInterruptable on
        // initialisation for the
        // FALLING_HIT_*/ROLLING/FALLING_LADDER_WALL/FALLING_PUSHED
        // anim families.
        for (seq_id, elem_idx) in non_interruptable_lifts {
            self.orders.sequence_manager.set_element_priority(
                seq_id,
                elem_idx,
                crate::sequence::SequencePriority::NonInterruptable,
            );
        }

        (ai_anim_done, combat_injury_terminated, completion_outcomes)
    }

    /// Dispatch per-frame animation sound triggers.
    ///
    /// Every animated element type runs a current-sound-id check
    /// during its tick: when the sprite's current frame has a
    /// non-zero sound ID, an FX sound is queued at the entity's
    /// position (with material for actors and projectiles, without
    /// for scenic FX/objects).
    ///
    /// Called once per tick, after [`EngineInner::tick_entity_movement`] and
    /// [`EngineInner::tick_entity_animations`] have advanced all sprite frames.
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
}
