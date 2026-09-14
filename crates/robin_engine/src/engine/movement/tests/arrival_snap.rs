#[cfg(test)]
mod suite {
    use super::super::{
        both_sword_ranges_contain_distance, perform_seek_exposes_motion_termination,
        should_snap_arrival,
    };
    use crate::coordinates::MapPoint;
    use crate::element::{EntityId, PcId};

    #[test]
    fn exact_goal_without_a_committed_step_does_not_snap() {
        assert!(!should_snap_arrival(false, false, 0.0, false));
        assert!(should_snap_arrival(true, false, 0.0, false));
        assert!(!should_snap_arrival(true, true, 0.0, false));
        assert!(!should_snap_arrival(true, false, 1.0, false));
        assert!(!should_snap_arrival(true, false, 0.0, true));
    }

    #[test]
    fn entity_seek_wait_hides_wrapped_motion_termination() {
        assert!(!perform_seek_exposes_motion_termination(false, Some(true)));
        assert!(perform_seek_exposes_motion_termination(true, Some(true)));
        assert!(perform_seek_exposes_motion_termination(false, None));
    }

    #[test]
    fn sword_provoke_range_is_snapshotted_before_line_crossing_projection() {
        // Linux2/Profile002/Savegame_015/replay-016: Original evaluates the
        // terminal gate inside human action execution at 89.7441025. The actor update
        // then projects the owner onto a crossed elevation line, where the
        // same live-position calculation becomes 90.76145. The owner's
        // MAXIMAL boundary is 90, so re-evaluating after crossing invents a
        // Provoke that Original never registered.
        let execute_distance = f32::from_bits(0x42b3_7cfb);
        let after_crossing_distance = 90.76145_f32;
        assert!(!both_sword_ranges_contain_distance(
            execute_distance,
            90,
            150,
            70,
            150
        ));
        assert!(both_sword_ranges_contain_distance(
            after_crossing_distance,
            90,
            150,
            70,
            150
        ));
    }
}
