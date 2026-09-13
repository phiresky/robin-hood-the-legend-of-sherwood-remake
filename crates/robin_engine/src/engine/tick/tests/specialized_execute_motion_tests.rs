use super::{
    apply_post_completion_execute_override, is_start_stop_movement_rewrite,
    project_post_completion_motion, specialized_execute_motion,
    specialized_order_advanced_after_execute,
};
use crate::order::OrderType;
use crate::sprite::MotionState;

#[test]
fn beggar_idle_returns_in_progress_while_retaining_the_sprite_start() {
    assert_eq!(
        specialized_execute_motion(Some(MotionState::Start), true, false),
        Some(MotionState::InProgress)
    );
    assert_eq!(
        specialized_execute_motion(Some(MotionState::Done), false, false),
        Some(MotionState::Done)
    );
    assert_eq!(
        specialized_execute_motion(Some(MotionState::Start), false, true),
        Some(MotionState::InProgress)
    );
    assert_eq!(
        specialized_execute_motion(Some(MotionState::Terminated), false, true),
        Some(MotionState::Terminated)
    );
}

#[test]
fn impossible_entry_element_preserves_aborted_across_condolence_sprite_edges() {
    assert_eq!(
        project_post_completion_motion(MotionState::Terminated, true, false, true),
        MotionState::Aborted
    );
    assert_eq!(
        project_post_completion_motion(MotionState::Done, true, true, true),
        MotionState::Aborted
    );
}

#[test]
fn manager_resident_wait_without_an_installed_order_does_not_mask_completion() {
    assert_eq!(
        project_post_completion_motion(MotionState::Done, false, false, true),
        MotionState::Terminated
    );
}

#[test]
fn exhausted_jump_landing_retains_terminated_without_a_successor() {
    assert_eq!(
        project_post_completion_motion(MotionState::Terminated, false, false, true),
        MotionState::Terminated
    );
}

#[test]
fn jump_landing_with_an_installed_successor_resumes_in_progress() {
    assert_eq!(
        project_post_completion_motion(MotionState::Terminated, false, true, true),
        MotionState::InProgress
    );
}

#[test]
fn synchronous_line_crossing_interruption_preserves_nonterminal_execute_result() {
    assert!(!specialized_order_advanced_after_execute(
        Some(MotionState::InProgress),
        false,
        true,
        true,
        false,
    ));
    assert_eq!(
        project_post_completion_motion(MotionState::InProgress, false, false, false),
        MotionState::InProgress,
        "the line callback cannot turn the preceding Execute result into Terminated"
    );

    assert!(specialized_order_advanced_after_execute(
        Some(MotionState::Terminated),
        false,
        true,
        false,
        false,
    ));
    assert!(specialized_order_advanced_after_execute(
        Some(MotionState::Done),
        false,
        false,
        false,
        false,
    ));
}

#[test]
fn committed_arrival_termination_survives_line_callback_interruption() {
    assert_eq!(
        apply_post_completion_execute_override(
            MotionState::InProgress,
            Some(MotionState::Terminated),
            true,
            false,
        ),
        MotionState::Terminated,
        "the original game latches terminal arrival before the line callback interrupts the movement"
    );
    assert_eq!(
        apply_post_completion_execute_override(
            MotionState::InProgress,
            Some(MotionState::Terminated),
            true,
            true,
        ),
        MotionState::InProgress,
        "order advancement's installed successor must still project the actor motion back to InProgress"
    );
    assert_eq!(
        apply_post_completion_execute_override(
            MotionState::InProgress,
            Some(MotionState::Terminated),
            false,
            false,
        ),
        MotionState::InProgress,
        "an ordinary committed arrival must retain the normal post-completion projection"
    );
}

#[test]
fn stop_movement_new_id_is_not_a_successor_order_advance() {
    assert!(is_start_stop_movement_rewrite(
        std::num::NonZeroU32::new(10).unwrap(),
        OrderType::WalkingUpright,
        std::num::NonZeroU32::new(11).unwrap(),
        OrderType::TransitionWalkingUprightWaitingUpright,
        MotionState::Start,
    ));
    assert!(is_start_stop_movement_rewrite(
        std::num::NonZeroU32::new(10).unwrap(),
        OrderType::RunningUpright,
        std::num::NonZeroU32::new(11).unwrap(),
        OrderType::TransitionRunningUprightWaitingUpright,
        MotionState::Done,
    ));
    assert!(is_start_stop_movement_rewrite(
        std::num::NonZeroU32::new(10).unwrap(),
        OrderType::WalkingCrouched,
        std::num::NonZeroU32::new(11).unwrap(),
        OrderType::TransitionWalkingCrouchedWaitingCrouched,
        MotionState::InProgress,
    ));
    assert!(!is_start_stop_movement_rewrite(
        std::num::NonZeroU32::new(10).unwrap(),
        OrderType::WalkingUpright,
        std::num::NonZeroU32::new(11).unwrap(),
        OrderType::TransitionWalkingUprightWaitingUpright,
        MotionState::Terminated,
    ));
    assert!(!is_start_stop_movement_rewrite(
        std::num::NonZeroU32::new(10).unwrap(),
        OrderType::WalkingUpright,
        std::num::NonZeroU32::new(11).unwrap(),
        OrderType::WalkingUpright,
        MotionState::Start,
    ));
    assert!(!is_start_stop_movement_rewrite(
        std::num::NonZeroU32::new(11).unwrap(),
        OrderType::RunningUpright,
        std::num::NonZeroU32::new(10).unwrap(),
        OrderType::TransitionRunningUprightWaitingUpright,
        MotionState::Start,
    ));
}

#[test]
fn stop_movement_reseed_preserves_outgoing_done_latch() {
    let rewritten_by_stop = is_start_stop_movement_rewrite(
        std::num::NonZeroU32::new(10).unwrap(),
        OrderType::RunningUpright,
        std::num::NonZeroU32::new(11).unwrap(),
        OrderType::TransitionRunningUprightWaitingUpright,
        MotionState::Done,
    );
    assert!(rewritten_by_stop);

    let advanced = specialized_order_advanced_after_execute(
        Some(MotionState::Done),
        rewritten_by_stop,
        false,
        false,
        false,
    );
    assert!(!advanced);
    assert_eq!(
        project_post_completion_motion(MotionState::Done, false, true, advanced),
        MotionState::Done
    );
}
