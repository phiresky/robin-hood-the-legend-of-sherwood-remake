use super::{should_apply_drunken_turn, should_apply_plain_movement_turn, turn_drunken};
use crate::position_interface::{Direction, PositionInterface};

#[test]
fn fresh_order_turn_uses_retained_goal_before_motion_initialization() {
    let mut position = PositionInterface::new();
    position.set_direction_instantly(Direction::from_raw(8));

    // The next movement order points north (sector 0), but Original does
    // not install that direction goal until after this Execute prologue.
    turn_drunken(&mut position);
    assert_eq!(position.get_direction(), Direction::from_raw(8));
    assert_eq!(position.get_direction_goal(), Direction::from_raw(8));

    // Mirror the later motion-increment goal update: the
    // sprite keeps the old facing for this frame while future drunken
    // turns now see the new target.
    position.set_direction(Direction::from_raw(0));
    assert_eq!(position.get_direction(), Direction::from_raw(8));
    assert_eq!(position.get_direction_goal(), Direction::from_raw(0));
}

#[test]
fn seek_movement_does_not_add_a_drunken_turn_before_perform_seek() {
    assert!(!should_apply_drunken_turn(
        true,
        crate::order::OrderType::WalkingUpright
    ));
    assert!(should_apply_drunken_turn(
        false,
        crate::order::OrderType::WalkingUpright
    ));
}

#[test]
fn drunken_and_seek_turn_branches_match_original_execute_matrix() {
    for (name, drunk, seek, action, expect_drunken, expect_plain) in [
        (
            "drunk ordinary walk",
            true,
            false,
            crate::order::OrderType::WalkingUpright,
            true,
            false,
        ),
        (
            "drunk seek walk",
            true,
            true,
            crate::order::OrderType::WalkingUpright,
            false,
            true,
        ),
        (
            "drunk startup transition",
            true,
            false,
            crate::order::OrderType::TransitionWaitingUprightWalkingUpright,
            false,
            true,
        ),
        (
            "sober ordinary walk",
            false,
            false,
            crate::order::OrderType::WalkingUpright,
            false,
            true,
        ),
        (
            "sober seek walk",
            false,
            true,
            crate::order::OrderType::WalkingUpright,
            false,
            true,
        ),
    ] {
        assert_eq!(
            drunk && should_apply_drunken_turn(seek, action),
            expect_drunken,
            "{name}: drunken-turn branch"
        );
        let flags = if seek {
            crate::sequence::MoveFlags::SEEK
        } else {
            crate::sequence::MoveFlags::empty()
        };
        assert_eq!(
            should_apply_plain_movement_turn(drunk, flags, action),
            expect_plain,
            "{name}: plain Turn branch"
        );
    }
}
