#[cfg(test)]
mod suite {
    use super::super::*;
    use crate::entity_id::{PcId, SoldierId};

    #[test]
    fn line_goal_still_uses_pathfinder_when_thick_route_is_blocked() {
        use crate::sequence::MoveFlags;

        assert!(!movement_flags_force_direct_dispatch(MoveFlags::LINE));
        assert!(movement_flags_force_direct_dispatch(MoveFlags::MAP));
        assert!(movement_flags_force_direct_dispatch(MoveFlags::STRAIGHT));
    }

    #[test]
    fn every_non_direct_request_uses_the_source_extraction_gate() {
        assert!(path_request_needs_source_extraction(false, false));
        assert!(!path_request_needs_source_extraction(false, true));
        assert!(!path_request_needs_source_extraction(true, false));
    }

    fn request(owner: EntityId, speed: crate::pathfinder::PathFinderSpeed) -> PendingPathRequest {
        PendingPathRequest {
            restored_from_v48: false,
            owner,
            seq_id: crate::sequence::SequenceId(1),
            elem_idx: 0,
            source: MapPoint::new(10.0, 10.0),
            dest: MapPoint::new(20.0, 20.0),
            layer: 0,
            sector: 0,
            legacy_sector: 0,
            half_diagonal_idx: 0,
            use_first_point: false,
            move_action: OrderType::WalkingUpright,
            speed,
            reverse: false,
            tolerance: 0.0,
            antagonist: None,
            is_pass_door: false,
            elem_flags: crate::sequence::MoveFlags::empty(),
            sword_movement_context: false,
            is_fast: false,
        }
    }

    fn advance_fake_frame(queue: &mut PendingPathRequestQueue) -> Option<EntityId> {
        let completed = queue.take_completed().map(|(processed, valid)| {
            assert!(valid, "ordinary fake-frame request became ignored");
            processed.request.owner
        });
        if let Some(request) = queue.pop_to_start() {
            queue.set_in_flight(request, Some(Vec::new()));
        }
        completed
    }

    #[test]
    fn original_two_request_special_case_keeps_first_request_first() {
        let npc = EntityId::Soldier(SoldierId(1));
        let pc = EntityId::Pc(PcId(2));
        let mut queue = PendingPathRequestQueue::default();

        // AddPathRequest appends unconditionally while fewer than two entries
        // exist, so this later FAST PC does not overtake the first MEDIUM NPC.
        queue.enqueue(request(npc, crate::pathfinder::PathFinderSpeed::Medium));
        queue.enqueue(request(pc, crate::pathfinder::PathFinderSpeed::Fast));

        assert_eq!(
            advance_fake_frame(&mut queue),
            None,
            "frame 1 only starts NPC"
        );
        assert_eq!(advance_fake_frame(&mut queue), Some(npc), "frame 2");
        assert_eq!(advance_fake_frame(&mut queue), Some(pc), "frame 3");
        assert_eq!(advance_fake_frame(&mut queue), None, "frame 4 is empty");
    }

    #[test]
    fn resolves_one_per_frame_in_original_priority_and_in_flight_order() {
        let npc_1 = EntityId::Soldier(SoldierId(1));
        let npc_2 = EntityId::Soldier(SoldierId(2));
        let pc_1 = EntityId::Pc(PcId(3));
        let pc_2 = EntityId::Pc(PcId(4));
        let mut queue = PendingPathRequestQueue::default();

        queue.enqueue(request(npc_1, crate::pathfinder::PathFinderSpeed::Medium));
        queue.enqueue(request(npc_2, crate::pathfinder::PathFinderSpeed::Medium));
        queue.enqueue(request(pc_1, crate::pathfinder::PathFinderSpeed::Fast));

        let mut completed_by_frame = Vec::new();
        assert_eq!(advance_fake_frame(&mut queue), None, "frame 1 starts pc_1");
        completed_by_frame.push((2, advance_fake_frame(&mut queue).unwrap()));

        // Frame 2 returns pc_1 and starts npc_1. A new FAST request can
        // overtake queued npc_2, but cannot displace in-flight npc_1.
        queue.enqueue(request(pc_2, crate::pathfinder::PathFinderSpeed::Fast));
        completed_by_frame.push((3, advance_fake_frame(&mut queue).unwrap()));
        completed_by_frame.push((4, advance_fake_frame(&mut queue).unwrap()));
        completed_by_frame.push((5, advance_fake_frame(&mut queue).unwrap()));

        assert_eq!(
            completed_by_frame,
            vec![(2, pc_1), (3, npc_1), (4, pc_2), (5, npc_2)]
        );
        assert_eq!(advance_fake_frame(&mut queue), None, "frame 6 is empty");
    }

    #[test]
    fn cancelled_head_is_delivered_invalid_and_occupies_its_result_slot() {
        let cancelled = EntityId::Pc(PcId(3));
        let successor = EntityId::Soldier(SoldierId(4));
        let mut queue = PendingPathRequestQueue::default();
        queue.enqueue(request(cancelled, crate::pathfinder::PathFinderSpeed::Fast));
        queue.enqueue(request(
            successor,
            crate::pathfinder::PathFinderSpeed::Medium,
        ));
        let first = queue.pop_to_start().expect("cancelled head starts");
        queue.set_in_flight(first, Some(vec![MapPoint::new(20.0, 20.0)]));

        queue.cancel_for_owner(cancelled);
        let (completed, valid) = queue
            .take_completed()
            .expect("cancelled head still completes");
        assert_eq!(completed.request.owner, cancelled);
        assert!(!valid);

        let next = queue.pop_to_start().expect("successor remains queued");
        assert_eq!(next.owner, successor);
    }

    #[test]
    fn cancelled_waiting_head_starts_even_after_its_element_dies() {
        let cancelled = EntityId::Soldier(SoldierId(3));
        let mut queue = PendingPathRequestQueue::default();
        queue.enqueue(request(cancelled, crate::pathfinder::PathFinderSpeed::Fast));

        queue.cancel_for_owner(cancelled);
        assert!(queue.ignore_next_path);
        let retained = queue.pop_to_start().expect("cancelled head remains queued");
        let cancelled_result = retained_cancelled_path_result(queue.ignore_next_path)
            .expect("cancelled path search exits with an empty raw path");
        assert!(cancelled_result.is_empty());
        queue.set_in_flight(retained, Some(cancelled_result));

        let (completed, valid) = queue
            .take_completed()
            .expect("cancelled head still consumes a completion slot");
        assert_eq!(completed.request.owner, cancelled);
        assert!(!valid);
    }

    #[test]
    fn parity_snapshot_keeps_ready_head_before_waiting_fifo() {
        let ready_owner = EntityId::Pc(PcId(3));
        let waiting_owner = EntityId::Soldier(SoldierId(4));
        let mut queue = PendingPathRequestQueue::default();
        queue.enqueue(request(
            ready_owner,
            crate::pathfinder::PathFinderSpeed::Fast,
        ));
        queue.enqueue(request(
            waiting_owner,
            crate::pathfinder::PathFinderSpeed::Medium,
        ));
        let ready = queue.pop_to_start().expect("head starts synchronously");
        queue.set_in_flight(ready, None);
        queue.cancel_for_owner(ready_owner);

        let mut grid = crate::fast_find_grid::FastFindGrid::new();
        grid.add_move_box_half_diagonal(crate::coordinates::MoveBoxHalfDiagonal::new(1.0, 1.0));
        let (ignored, snapshot) = queue.parity_state(&grid);

        assert!(ignored);
        assert_eq!(snapshot.len(), 2);
        assert!(snapshot[0].in_flight);
        assert_eq!(snapshot[0].request.actor, ready_owner);
        assert_eq!(snapshot[0].waypoints, Some(Vec::new()));
        assert!(!snapshot[1].in_flight);
        assert_eq!(snapshot[1].request.actor, waiting_owner);
        assert_eq!(snapshot[1].waypoints, None);
    }

    #[test]
    fn pending_make_rewrites_first_request_like_original_pathfinder() {
        let owner = EntityId::Pc(PcId(3));
        let mut queue = PendingPathRequestQueue::default();
        queue.enqueue(request(owner, crate::pathfinder::PathFinderSpeed::Fast));

        queue.make_fast(owner, 7);
        assert_eq!(queue.waiting[0].move_action, OrderType::RunningUpright);
        assert_eq!(queue.waiting[0].half_diagonal_idx, 7);

        queue.make_slow(owner, 7);
        assert_eq!(queue.waiting[0].move_action, OrderType::WalkingUpright);

        queue.make_crouched(owner, 9);
        assert_eq!(queue.waiting[0].move_action, OrderType::WalkingCrouched);
        assert_eq!(queue.waiting[0].half_diagonal_idx, 9);

        queue.make_upright(owner, 7);
        assert_eq!(queue.waiting[0].move_action, OrderType::WalkingUpright);
        assert_eq!(queue.waiting[0].half_diagonal_idx, 7);
    }

    #[test]
    fn movement_context_expires_live_failure_only_after_100_frames() {
        let owner = EntityId::Pc(PcId(7));
        let mut world = WorldState::new();
        let mut orders = OrderRuntime::new();
        let mut element = crate::sequence::SequenceElement::new(
            1,
            crate::element::Command::MoveWaiting,
            Some(owner),
        );
        element.priority = crate::sequence::SequencePriority::Normal;
        let sequence_id = orders.sequence_manager.launch_element(element);
        let element = orders
            .sequence_manager
            .get_element_mut(sequence_id, 0)
            .expect("launched movement element");
        element.state = crate::sequence::SequenceState::InProgress;
        element.command = crate::element::Command::MoveWaiting;
        orders
            .failed_path_requests
            .push(FailedPathRequest::from_pending(
                PendingPathRequest::test_request(owner, sequence_id, 0),
                10,
            ));

        let at_boundary =
            MovementContext::new(110, &mut world, &mut orders).take_expired_failures();
        assert!(at_boundary.is_empty());
        assert_eq!(orders.failed_path_requests.len(), 1);

        let after_boundary =
            MovementContext::new(111, &mut world, &mut orders).take_expired_failures();
        assert_eq!(after_boundary.len(), 1);
        assert_eq!(after_boundary[0].request.owner, owner);
        assert_eq!(after_boundary[0].age, 101);
        assert!(
            !after_boundary[0].owner_is_pc,
            "missing entity is not fabricated"
        );
        assert!(orders.failed_path_requests.is_empty());
    }
}
