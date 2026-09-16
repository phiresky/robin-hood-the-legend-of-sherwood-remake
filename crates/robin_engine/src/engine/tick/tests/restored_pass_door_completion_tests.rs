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
            ..ActorData::default()
        },
        human: HumanData::default(),
        pc: PcData::default(),
    })
}

#[test]
fn crenel_exit_completes_without_live_door() {
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
    assert!(pc.position_iface().get_door().is_none());
}

#[test]
fn ladder_down_exits_complete_without_live_door() {
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
        assert!(actor.position_iface().get_door().is_none());
    }
}

#[test]
fn wall_down_exit_completes_without_live_door() {
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(airborne_pc());
    engine
        .get_entity_mut(owner)
        .unwrap()
        .set_posture(Posture::OnWall);
    engine.apply_door_pass_transition_completion_side_effects(
        &LevelAssets::new(),
        owner,
        OrderType::TransitionClimbingWallDownWaitingUpright,
    );

    let pc = engine.get_entity(owner).unwrap();
    assert_eq!(pc.element_data().posture(), Posture::Upright);
    assert_eq!(pc.actor_data().unwrap().action_state, ActionState::Waiting);
    assert!(pc.position_iface().get_door().is_none());
}
