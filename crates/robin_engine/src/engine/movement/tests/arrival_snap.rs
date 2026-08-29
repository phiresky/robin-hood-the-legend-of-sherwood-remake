#[cfg(test)]
mod suite {
    use super::super::{
        MovementDeferred, both_sword_ranges_contain_distance,
        perform_seek_exposes_motion_termination, queue_committed_arrival_crossing,
        should_snap_arrival,
    };
    use crate::coordinates::MapPoint;
    use crate::element::{EntityId, PcId};

    #[test]
    fn committed_seek_arrival_retains_both_actor_crossing_passes() {
        let mut deferred = MovementDeferred::default();
        let owner = EntityId::Pc(PcId(252));
        let old_pos = MapPoint::new(1_284.826_3, 2_277.009_3);

        assert!(queue_committed_arrival_crossing(
            &mut deferred,
            owner,
            old_pos,
            0,
            true,
            true,
        ));
        assert_eq!(deferred.line_cross_checks, vec![(owner, old_pos, 0)]);
        assert_eq!(
            deferred.non_elevation_cross_checks,
            vec![(owner, old_pos, 0)]
        );
    }

    #[test]
    fn stationary_or_ineligible_seek_arrival_does_not_queue_crossing() {
        let mut deferred = MovementDeferred::default();
        let owner = EntityId::Pc(PcId(252));
        let old_pos = MapPoint::new(1_284.826_3, 2_277.009_3);

        assert!(!queue_committed_arrival_crossing(
            &mut deferred,
            owner,
            old_pos,
            0,
            false,
            true,
        ));
        assert!(!queue_committed_arrival_crossing(
            &mut deferred,
            owner,
            old_pos,
            0,
            true,
            false,
        ));
        assert!(deferred.line_cross_checks.is_empty());
        assert!(deferred.non_elevation_cross_checks.is_empty());
    }

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
        // terminal gate inside Human::Execute at 89.7441025. Actor::Hourglass
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
