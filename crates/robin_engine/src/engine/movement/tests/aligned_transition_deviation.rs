#[cfg(test)]
mod suite {
    use super::super::should_clear_deviated_for_aligned_transition_start;
    use crate::coordinates::MapPoint;

    #[test]
    fn pc_preserves_deviation_across_aligned_movement_exit_transition() {
        let aligned = MapPoint::new(407.967_35, 786.937_9);

        assert!(!should_clear_deviated_for_aligned_transition_start(
            true,
            true,
            true,
            crate::order::OrderType::TransitionWalkingUprightWaitingUpright,
            true,
            aligned,
            aligned,
        ));
    }

    #[test]
    fn pc_retires_stale_deviation_at_aligned_movement_start_transition() {
        let aligned = MapPoint::new(1_297.909_3, 720.281_25);

        assert!(should_clear_deviated_for_aligned_transition_start(
            true,
            true,
            true,
            crate::order::OrderType::TransitionWaitingUprightWalkingUpright,
            true,
            aligned,
            aligned,
        ));
    }

    #[test]
    fn non_pc_clears_deviation_only_on_aligned_movement_start_transition() {
        let aligned = MapPoint::new(100.0, 200.0);

        assert!(!should_clear_deviated_for_aligned_transition_start(
            false,
            true,
            true,
            crate::order::OrderType::TransitionWalkingUprightWaitingUpright,
            true,
            aligned,
            aligned,
        ));
        assert!(should_clear_deviated_for_aligned_transition_start(
            false,
            true,
            true,
            crate::order::OrderType::TransitionWaitingUprightWalkingUpright,
            true,
            aligned,
            aligned,
        ));
        // Savegame_032 replay-010: Soldier 52 reaches an aligned generated
        // running startup with the observable anti-vibration history
        // `deviated=true, direction_count=-2`. The next clockwise shield
        // turn rotates on its first call in the Original, which is possible
        // only when this running startup drops the deviation latch.
        assert!(should_clear_deviated_for_aligned_transition_start(
            false,
            true,
            true,
            crate::order::OrderType::TransitionWaitingUprightRunningUpright,
            true,
            aligned,
            aligned,
        ));
        assert!(!should_clear_deviated_for_aligned_transition_start(
            false,
            false,
            true,
            crate::order::OrderType::TransitionWalkingUprightWaitingUpright,
            true,
            aligned,
            aligned,
        ));
        assert!(!should_clear_deviated_for_aligned_transition_start(
            false,
            true,
            false,
            crate::order::OrderType::TransitionWalkingUprightWaitingUpright,
            true,
            aligned,
            aligned,
        ));
    }
}
