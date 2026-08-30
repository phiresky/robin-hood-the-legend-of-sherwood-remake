#[cfg(test)]
mod suite {
    use super::super::*;
    use crate::entity_id::{PcId, SoldierId};

    fn test_pc() -> Entity {
        Entity::Pc(crate::element::ActorPc {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorPc,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            pc: Default::default(),
        })
    }

    #[test]
    fn line_goal_still_uses_pathfinder_when_thick_route_is_blocked() {
        use crate::sequence::MoveFlags;

        assert!(!movement_flags_force_direct_dispatch(MoveFlags::LINE));
        assert!(movement_flags_force_direct_dispatch(MoveFlags::MAP));
        assert!(movement_flags_force_direct_dispatch(MoveFlags::STRAIGHT));
    }

    #[test]
    fn postponed_move_resumes_directly_at_terminal_door_handoff() {
        use crate::element::{
            ActorData, ActorSoldier, Command, ElementData, ElementKind, HumanData, NpcData,
            Posture, SoldierData,
        };
        use crate::sequence::{Sequence, SequenceElement, SequenceElementData};

        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(64, 64);
        engine.world.fast_grid_mut().allocate_layers(3);
        let source = MapPoint::new(707.0, 1560.0);
        let destination = MapPoint::new(737.5406, 1709.7073);
        let mut element = ElementData {
            kind: ElementKind::ActorSoldier,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.set_layer(1);
        element.set_sector(crate::position_interface::SectorHandle::new(70));
        element
            .sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -6.0, -4.0, 6.0, 4.0,
            ));
        element.set_position_map(source);
        let owner = engine.add_entity(Entity::Soldier(ActorSoldier {
            element,
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            soldier: SoldierData::default(),
        }));

        let mut pass = SequenceElement::new_movement(
            1,
            Command::PassDoor,
            Some(owner),
            OrderType::WalkingUpright,
        );
        let SequenceElementData::Movement {
            destination: pass_destination,
            layer: pass_layer,
            gate_id,
            ..
        } = &mut pass.data
        else {
            unreachable!()
        };
        *pass_destination = source;
        *pass_layer = 1;
        *gate_id = Some(crate::gate::DoorIndex::new(114).expect("114 is a valid test door index"));

        let mut route_assert = SequenceElement::new_movement(
            2,
            Command::AssertPosition,
            Some(owner),
            OrderType::WalkingUpright,
        );
        let SequenceElementData::Movement {
            destination: assert_destination,
            ..
        } = &mut route_assert.data
        else {
            unreachable!()
        };
        *assert_destination = source;
        let route_move =
            SequenceElement::new_movement(3, Command::Move, Some(owner), OrderType::WalkingUpright);
        let mut route = Sequence::new();
        route.append_element(pass);
        route.append_element(route_assert);
        route.append_element(route_move);
        let route_id = engine.orders.sequence_manager.launch_sequence(route);

        assert!(
            engine
                .orders
                .sequence_manager
                .pop_next_hourglass_action()
                .is_some()
        );
        engine
            .orders
            .sequence_manager
            .element_in_progress(route_id, 0);
        engine
            .orders
            .sequence_manager
            .element_terminated(route_id, 0);
        engine.dispatch_condolations_for_owner_boundary(
            &crate::sim_rng::test_context(),
            owner,
            &LevelAssets::new(),
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .pop_next_hourglass_action()
                .is_some()
        );
        engine
            .orders
            .sequence_manager
            .element_in_progress(route_id, 1);
        engine
            .orders
            .sequence_manager
            .element_terminated(route_id, 1);
        engine.dispatch_condolations_for_owner_boundary(
            &crate::sim_rng::test_context(),
            owner,
            &LevelAssets::new(),
        );

        let mut postponed =
            SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
        let SequenceElementData::Movement {
            destination: stored_destination,
            layer,
            sector,
            tolerance,
            ..
        } = &mut postponed.data
        else {
            unreachable!()
        };
        *stored_destination = destination;
        *layer = 2;
        *sector = crate::position_interface::SectorHandle::new(88);
        *tolerance = 30.0;
        let postponed_id = engine.orders.sequence_manager.launch_element(postponed);

        assert!(has_deferred_post_door_route_continuation(
            &engine.orders.sequence_manager,
            owner,
            source,
            1,
        ));
        let outcome = engine.try_dispatch_move_path(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            owner,
            postponed_id,
            0,
            destination,
            OrderType::WalkingUpright,
        );
        assert!(matches!(outcome, MovePathOutcome::Success));
        let postponed = engine
            .orders
            .sequence_manager
            .get_element(postponed_id, 0)
            .unwrap();
        assert_eq!(postponed.command, Command::MoveOk);
        assert!(engine.orders.pending_path_requests.waiting.is_empty());
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
    fn entity_teardown_retains_in_flight_head_until_invalid_completion() {
        let removed = EntityId::Pc(PcId(3));
        let successor = EntityId::Soldier(SoldierId(4));
        let mut queue = PendingPathRequestQueue::default();
        queue.enqueue(request(removed, crate::pathfinder::PathFinderSpeed::Fast));
        queue.enqueue(request(
            successor,
            crate::pathfinder::PathFinderSpeed::Medium,
        ));
        let first = queue.pop_to_start().expect("removed owner starts");
        queue.set_in_flight(first, Some(vec![MapPoint::new(20.0, 20.0)]));

        queue.retain_not_owned_by(removed);

        assert!(queue.ignore_next_path);
        assert!(queue.in_flight.is_some(), "logical head remains in flight");
        assert_eq!(queue.waiting.len(), 1);
        assert_eq!(queue.waiting[0].owner, successor);
        let (completed, valid) = queue
            .take_completed()
            .expect("removed head consumes its completion slot");
        assert_eq!(completed.request.owner, removed);
        assert!(!valid);
    }

    #[test]
    fn entity_teardown_retains_waiting_logical_head() {
        let removed = EntityId::Pc(PcId(3));
        let successor = EntityId::Soldier(SoldierId(4));
        let mut queue = PendingPathRequestQueue::default();
        queue.enqueue(request(removed, crate::pathfinder::PathFinderSpeed::Fast));
        queue.enqueue(request(
            successor,
            crate::pathfinder::PathFinderSpeed::Medium,
        ));

        queue.retain_not_owned_by(removed);

        assert!(queue.ignore_next_path);
        assert_eq!(queue.waiting.len(), 2);
        assert_eq!(queue.waiting[0].owner, removed);
        assert_eq!(queue.waiting[1].owner, successor);
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
    fn path_schedule_context_expires_live_failure_without_mutating_sequence_state() {
        let owner = EntityId::Pc(PcId(0));
        let mut world = WorldState::new();
        world.entities.push(Some(test_pc()));
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

        let at_boundary = {
            let (entities, fast_grid, pathfinder) = world.path_schedule_parts();
            let (pending, failed, sequence_manager) = orders.path_schedule_parts();
            PathScheduleContext::new(
                110,
                entities,
                fast_grid,
                pathfinder,
                pending,
                failed,
                sequence_manager,
            )
            .take_next_expired_failure()
        };
        assert!(at_boundary.is_none());
        assert_eq!(orders.failed_path_requests.len(), 1);

        let after_boundary = {
            let (entities, fast_grid, pathfinder) = world.path_schedule_parts();
            let (pending, failed, sequence_manager) = orders.path_schedule_parts();
            PathScheduleContext::new(
                111,
                entities,
                fast_grid,
                pathfinder,
                pending,
                failed,
                sequence_manager,
            )
            .take_next_expired_failure()
        };
        let after_boundary = after_boundary.expect("failure expires at age 101");
        assert_eq!(after_boundary.request.owner, owner);
        assert_eq!(after_boundary.age, 101);
        assert!(after_boundary.owner_is_pc);
        assert!(orders.failed_path_requests.is_empty());
        assert_eq!(
            orders
                .sequence_manager
                .get_element(sequence_id, 0)
                .expect("path context retains the live movement element")
                .state,
            crate::sequence::SequenceState::InProgress,
            "sequence mutation remains a root-coordinator consequence"
        );
    }

    #[test]
    fn expired_failure_scan_rechecks_liveness_after_each_owner_boundary() {
        let first_owner = EntityId::Pc(PcId(0));
        let later_owner = EntityId::Pc(PcId(1));
        let mut world = WorldState::new();
        world.entities.push(Some(test_pc()));
        world.entities.push(Some(test_pc()));
        let mut orders = OrderRuntime::new();

        let mut launch_waiting = |owner| {
            let mut element = crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::MoveWaiting,
                Some(owner),
            );
            element.state = crate::sequence::SequenceState::InProgress;
            let sequence = orders.sequence_manager.launch_element(element);
            let element = orders
                .sequence_manager
                .get_element_mut(sequence, 0)
                .expect("launched movement element");
            element.state = crate::sequence::SequenceState::InProgress;
            element.command = crate::element::Command::MoveWaiting;
            sequence
        };
        let first_sequence = launch_waiting(first_owner);
        let later_sequence = launch_waiting(later_owner);
        orders.failed_path_requests.extend([
            FailedPathRequest::from_pending(
                PendingPathRequest::test_request(first_owner, first_sequence, 0),
                0,
            ),
            FailedPathRequest::from_pending(
                PendingPathRequest::test_request(later_owner, later_sequence, 0),
                0,
            ),
        ]);

        let first = {
            let (entities, fast_grid, pathfinder) = world.path_schedule_parts();
            let (pending, failed, sequence_manager) = orders.path_schedule_parts();
            PathScheduleContext::new(
                101,
                entities,
                fast_grid,
                pathfinder,
                pending,
                failed,
                sequence_manager,
            )
            .take_next_expired_failure()
        }
        .expect("first failure expires");
        assert_eq!(first.request.owner, first_owner);

        // Model a synchronous consequence of the first owner's condolation
        // invalidating the later request before the Original list walk reaches
        // it. A pre-batched scan would already have committed both outcomes.
        orders
            .sequence_manager
            .get_element_mut(later_sequence, 0)
            .expect("later movement remains registered")
            .state = crate::sequence::SequenceState::Interrupted;

        let later = {
            let (entities, fast_grid, pathfinder) = world.path_schedule_parts();
            let (pending, failed, sequence_manager) = orders.path_schedule_parts();
            PathScheduleContext::new(
                101,
                entities,
                fast_grid,
                pathfinder,
                pending,
                failed,
                sequence_manager,
            )
            .take_next_expired_failure()
        };
        assert!(later.is_none());
        assert!(orders.failed_path_requests.is_empty());
    }

    #[test]
    #[should_panic(expected = "retains a live sequence element but its owner entity is missing")]
    fn path_schedule_context_rejects_missing_owner_for_live_expired_failure() {
        let owner = EntityId::Pc(PcId(0));
        let mut world = WorldState::new();
        let mut orders = OrderRuntime::new();
        let mut element = crate::sequence::SequenceElement::new(
            1,
            crate::element::Command::MoveWaiting,
            Some(owner),
        );
        element.state = crate::sequence::SequenceState::InProgress;
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
                0,
            ));

        let (entities, fast_grid, pathfinder) = world.path_schedule_parts();
        let (pending, failed, sequence_manager) = orders.path_schedule_parts();
        let _ = PathScheduleContext::new(
            101,
            entities,
            fast_grid,
            pathfinder,
            pending,
            failed,
            sequence_manager,
        )
        .take_next_expired_failure();
    }
}
