use super::*;
use crate::element::{
    ActionState, ActorData, ActorPc, ElementData, ElementKind, Entity, HumanData, PcData, Posture,
};
use crate::order::OrderType;

fn airborne_pc() -> Entity {
    Entity::Pc(ActorPc {
        element: {
            let mut initial_element = ElementData::from_initial_posture(Posture::Flying);
            initial_element.kind = ElementKind::ActorPc;
            initial_element.active = true;
            initial_element
        },
        actor: ActorData {
            action_state: ActionState::Moving,
            active_door_pass: None,
            ..ActorData::default()
        },
        human: HumanData::default(),
        pc: PcData::default(),
    })
}

#[test]
fn restored_crenel_exit_completes_without_active_door_pass() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(airborne_pc());
    engine.apply_door_pass_transition_completion_side_effects(
        &LevelAssets::new(),
        owner,
        OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel,
    );

    let pc = engine.get_entity(owner).unwrap();
    assert_eq!(pc.element_data().posture(), Posture::Crouched);
    assert_eq!(pc.actor_data().unwrap().action_state, ActionState::Waiting);
    assert!(pc.actor_data().unwrap().active_door_pass.is_none());
}

#[test]
fn restored_ladder_down_exits_complete_without_active_door_pass() {
    for action in [
        OrderType::TransitionClimbingLadderDownWaitingUpright,
        OrderType::TransitionClimbingLadderDownWaitingUprightAlerted,
    ] {
        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(airborne_pc());
        engine
            .get_entity_mut(owner)
            .unwrap()
            .set_posture(Posture::OnLadder);

        engine.apply_door_pass_transition_completion_side_effects(
            &LevelAssets::new(),
            owner,
            action,
        );

        let actor = engine.get_entity(owner).unwrap();
        assert_eq!(
            actor.element_data().posture(),
            Posture::Upright,
            "{action:?}"
        );
        assert_eq!(
            actor.actor_data().unwrap().action_state,
            ActionState::Waiting,
            "{action:?}"
        );
        assert!(actor.actor_data().unwrap().active_door_pass.is_none());
    }
}

#[test]
fn unrelated_transition_still_requires_active_door_pass() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(airborne_pc());
    engine.apply_door_pass_transition_completion_side_effects(
        &LevelAssets::new(),
        owner,
        OrderType::TransitionClimbingWallDownWaitingUpright,
    );

    let pc = engine.get_entity(owner).unwrap();
    assert_eq!(pc.element_data().posture(), Posture::Flying);
    assert_eq!(pc.actor_data().unwrap().action_state, ActionState::Moving);
}
