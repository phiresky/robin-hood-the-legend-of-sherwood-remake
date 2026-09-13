use super::*;

#[test]
fn only_live_locomotion_transitions_retain_the_outgoing_goal() {
    assert!(movement_transition_retains_goal(
        OrderType::TransitionWaitingUprightRunningUpright
    ));
    assert!(movement_transition_retains_goal(
        OrderType::TransitionRunningUprightWaitingUpright
    ));
    assert!(!movement_transition_retains_goal(OrderType::RunningUpright));
    assert!(!movement_transition_retains_goal(OrderType::WalkingUpright));
}
