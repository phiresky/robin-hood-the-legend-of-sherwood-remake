#[cfg(test)]
mod suite {
    use super::super::*;
    use crate::element::{
        ActionState, ActorData, ActorPc, ActorSoldier, Command, ElementData, ElementKind, Entity,
        HumanData, NpcData, PcData, Posture, SoldierData,
    };
    use crate::sequence::{
        Field, FieldValue, MoveFlags, Sequence, SequenceElement, SequenceElementData,
        SequencePriority, SequenceState,
    };

    fn extraction_test_pc(posture: Posture) -> Entity {
        Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture,
                ..Default::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        })
    }

    fn extraction_test_assets() -> LevelAssets {
        let mut profiles = crate::profiles::ProfileManager::new();
        profiles
            .characters
            .push(crate::profiles::CharacterProfile::default());
        LevelAssets {
            profile_manager: std::sync::Arc::new(profiles),
            ..LevelAssets::new()
        }
    }

    #[test]
    fn line_jump_click_tail_jumps_then_moves_to_click_without_route_flags() {
        let owner = EntityId::Pc(crate::entity_id::PcId(7));
        let source_idx = crate::jump_line::JumpLineIndex::new(2).unwrap();
        let dest_idx = crate::jump_line::JumpLineIndex::new(3).unwrap();

        let tail = build_line_jump_click_tail(
            owner,
            OrderType::RunningUpright,
            source_idx,
            dest_idx,
            crate::coordinates::map_pt(90.0, 120.0),
            5,
            1.0,
        );

        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].command, Command::JumpCmd);
        assert!(matches!(
            tail[0].get_property(Field::JumplineSource),
            Some(FieldValue::LineId(idx)) if *idx == source_idx
        ));
        assert!(matches!(
            tail[0].get_property(Field::JumplineDestination),
            Some(FieldValue::LineId(idx)) if *idx == dest_idx
        ));

        assert_eq!(tail[1].command, Command::Move);
        match &tail[1].data {
            SequenceElementData::Movement {
                destination,
                layer,
                flags,
                line_id,
                ..
            } => {
                assert_eq!((destination.x, destination.y), (90.0, 120.0));
                assert_eq!(*layer, 5);
                assert!(flags.is_empty());
                assert_eq!(*line_id, None);
            }
            other => panic!("expected final movement element, got {other:?}"),
        }
    }

    #[test]
    fn shoulder_line_jump_routes_only_the_approach_on_the_carrier() {
        let mut engine = EngineInner::new();
        let carrier = engine.add_entity(extraction_test_pc(Posture::CarryingOnShoulders));
        let mut rider = extraction_test_pc(Posture::OnShoulders);
        rider.human_data_mut().expect("test rider is human").carrier = Some(carrier);
        let rider = engine.add_entity(rider);

        assert_eq!(line_jump_approach_owner(&engine, rider), carrier);
        assert_eq!(
            line_jump_approach_owner(&engine, carrier),
            carrier,
            "a selected carrier remains the owner of its own line approach"
        );
        let tail = build_line_jump_click_tail(
            rider,
            OrderType::WalkingUpright,
            crate::jump_line::JumpLineIndex::new(1).unwrap(),
            crate::jump_line::JumpLineIndex::new(2).unwrap(),
            crate::coordinates::MapPoint::ZERO,
            0,
            1.0,
        );
        assert!(tail.iter().all(|element| element.owner == Some(rider)));
    }

    #[test]
    fn running_with_sword_uses_distance_motion() {
        assert_eq!(
            sword_movement_dispatch_action(OrderType::WalkingUpright),
            OrderType::WalkingWithSword
        );
        assert_eq!(
            sword_movement_dispatch_action(OrderType::WalkingWithCorpse),
            OrderType::WalkingWithSword
        );
        assert_eq!(
            sword_movement_dispatch_action(OrderType::RunningUpright),
            OrderType::RunningWithSword
        );
        assert!(order_uses_distance_motion(OrderType::RunningWithSword));
        assert!(order_uses_distance_motion(OrderType::WalkingWithSword));
        assert!(order_uses_distance_motion(OrderType::WalkingSword));
        assert!(!order_uses_distance_motion(
            OrderType::TransitionRunningUprightWaitingUpright
        ));
        assert!(!order_uses_distance_motion(
            OrderType::TransitionSpecialWaitingUpright
        ));
    }

    #[test]
    fn turn_minimum_is_applied_after_movement_speed_factor() {
        let distance = scaled_motion_distance(2.0, 0.582_163_33, true, true);
        assert_eq!(
            distance, 0.7,
            "Original scales 2.0 by the patrol factor, then applies the 0.6 turn slowdown and 0.7 minimum"
        );
        assert_eq!(
            scaled_motion_distance(2.0, 0.582_163_33, true, false),
            1.164_326_7
        );
        assert_eq!(
            scaled_motion_distance(2.0, 0.25, false, false),
            2.0,
            "transition motion does not use the movement element's speed factor"
        );
    }

    #[test]
    fn movement_execute_state_effects_match_transition_execute() {
        use crate::element::{ActionState, Posture};
        use crate::sprite::MotionState;

        assert_eq!(
            movement_execute_state_effect(
                OrderType::TransitionSpecialWaitingUpright,
                MotionState::Done
            ),
            Some((Posture::Upright, ActionState::Waiting))
        );
        assert_eq!(
            movement_execute_state_effect(
                OrderType::TransitionWaitingUprightWalkingUpright,
                MotionState::Terminated
            ),
            Some((Posture::Upright, ActionState::Waiting))
        );
        assert_eq!(
            movement_execute_state_effect(OrderType::WalkingUpright, MotionState::Start),
            Some((Posture::Upright, ActionState::Moving))
        );
        assert_eq!(
            movement_execute_state_effect(
                OrderType::TransitionWaitingUprightBoredWaitingUpright,
                MotionState::Done
            ),
            Some((Posture::Upright, ActionState::Waiting))
        );
        assert_eq!(
            movement_execute_state_effect(
                OrderType::TransitionWaitingUprightWaitingUprightBored,
                MotionState::Done
            ),
            Some((Posture::Upright, ActionState::Bored))
        );
        assert_eq!(
            movement_execute_state_effect(
                OrderType::TransitionWalkingCrouchedWaitingCrouched,
                MotionState::Terminated
            ),
            Some((Posture::Crouched, ActionState::Waiting)),
            "a seek arrival in the crouched stop transition must publish Waiting before its post-seek interaction is instructed"
        );
        assert_eq!(
            movement_execute_state_effect(OrderType::TransitionCrouchingDown, MotionState::Done),
            Some((Posture::Crouched, ActionState::Waiting))
        );
        assert_eq!(
            movement_execute_state_effect(
                OrderType::TransitionLeaningOutWaitingAlerted,
                MotionState::Done
            ),
            Some((Posture::Upright, ActionState::Waiting))
        );
    }

    #[test]
    fn terminal_first_running_upright_execute_still_stamps_moving_fast() {
        use crate::element::{ActionState, Posture};
        use crate::sprite::MotionState;

        assert_eq!(
            movement_execute_state_effect(OrderType::RunningUpright, MotionState::Terminated),
            Some((Posture::Upright, ActionState::MovingFast))
        );
    }

    #[test]
    fn in_progress_walking_with_shield_stamps_moving_shield() {
        use crate::element::{ActionState, Posture};
        use crate::sprite::MotionState;

        assert_eq!(
            movement_execute_state_effect(OrderType::WalkingWithShield, MotionState::InProgress),
            Some((Posture::Upright, ActionState::MovingShield))
        );
    }

    #[test]
    fn transition_distance_first_execute_is_consumed_once() {
        let mut continuation = true;

        assert!(take_transition_distance_first_execute(&mut continuation));
        assert!(!continuation);
        assert!(!take_transition_distance_first_execute(&mut continuation));

        let mut continuation = true;
        assert!(take_transition_distance_first_execute(&mut continuation));
        assert!(
            !continuation,
            "the marker belongs to the first Execute slot, even when that slot hides START"
        );
        assert!(!take_transition_distance_first_execute(&mut continuation));
    }

    #[test]
    fn deferred_movement_state_start_promotes_only_the_successor_handoff() {
        let mut deferred = true;

        assert!(take_deferred_movement_state_start(&mut deferred));
        assert!(!deferred);
        assert!(
            !take_deferred_movement_state_start(&mut deferred),
            "the synthetic START is a one-shot order handoff"
        );
    }

    #[test]
    fn entity_target_seek_does_not_synthesize_deferred_pc_movement_start() {
        assert!(should_defer_pc_movement_state_start(true, false));
        assert!(
            !should_defer_pc_movement_state_start(true, true),
            "entity-target PerformSeek hides START from the Original Execute arm"
        );
        assert!(!should_defer_pc_movement_state_start(false, false));
    }

    fn run_fast_wall_anti_collision_fixture(
        first_distance: u16,
    ) -> (MapPoint, crate::movement_diagnostics::ParityMovementStep) {
        use crate::element::{
            ActorData, ActorPc, ElementData, ElementKind, HumanData, PcData, Posture,
        };
        use crate::fast_find_grid::GridSector;
        use crate::order::Order;
        use crate::sector::{LiftType, SectorNumber, SectorType};
        use crate::sequence::SequenceElement;
        use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

        let mut engine = EngineInner::new();
        let start = MapPoint::new(1_760.418_7, 1_011.022);
        let goal = MapPoint::new(1762.0, 996.0);
        let physical = OrderType::ClimbingWallDown;
        let script = SpriteScript {
            action_id: physical as u16,
            action_done: 7,
            average_speed: 4.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 32,
            frame_ids: vec![1; 8],
            delays: vec![0; 8],
            distances: std::iter::once(first_distance)
                .chain(std::iter::repeat_n(4, 7))
                .collect(),
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 8],
            sound_ids: vec![0; 8],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[physical as usize] = 0;
        let mut pc = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture: Posture::OnWall,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        });
        pc.element_data_mut().active = true;
        pc.element_data_mut().sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );
        pc.element_data_mut().set_position_map(start);
        pc.element_data_mut().set_direction_instantly(0);
        pc.element_data_mut().set_sector(Some(
            crate::position_interface::SectorHandle::new(1).unwrap(),
        ));
        pc.position_iface_mut()
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -4.0, -4.0, 4.0, 4.0,
            ));
        pc.position_iface_mut().set_anti_collision_on(true);
        let owner = engine.add_entity(pc);
        {
            let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
            level.sector_number_map.insert(SectorNumber::new(1), 0);
            level.sectors.push(GridSector {
                points: Vec::new(),
                bounding_box: crate::coordinates::MapBBox::new(),
                sector_type: SectorType::LIFT,
                layer: 0,
                sector_number: SectorNumber::new(1),
                door_index: None,
                lift_type: Some(LiftType::Wall),
                lift_direction: 0,
                force_crouched: false,
                building_index: None,
                low_exit_point: Some(goal),
                high_exit_point: Some(start),
                lowest_door_index: None,
                jump_line_indices: Vec::new(),
                gate_indices: Vec::new(),
                underlying_sector: None,
            });
        }
        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::ClimbingWallDownFast,
        );
        movement.orders.push_back(Order::test_new(
            OrderType::ClimbingWallDownFast,
            goal.x,
            goal.y,
        ));
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .active_movement = ActiveMovement::new(sequence, 0);

        crate::movement_diagnostics::begin_parity_movement_capture();
        engine.tick_entity_movement(&crate::sim_rng::test_context(), &LevelAssets::new());
        let captures = crate::movement_diagnostics::take_parity_movement_capture();

        let position = engine
            .get_entity(owner)
            .unwrap()
            .element_data()
            .position_map();
        let capture = captures
            .iter()
            .find(|capture| capture.entity == owner)
            .expect("fast wall owner must emit a production movement capture")
            .clone();
        (position, capture)
    }

    #[test]
    fn fast_wall_anti_collision_commits_each_perform_motion_before_the_next() {
        let (position, capture) = run_fast_wall_anti_collision_fixture(4);
        assert_eq!(capture.split_calls.len(), 2);
        assert_eq!(capture.split_calls[0].pre_position.x.bits, 1155272038);
        assert_eq!(capture.split_calls[0].post_position.x.bits, 1155275468);
        assert_eq!(capture.split_calls[1].pre_position.x.bits, 1155275468);
        assert_eq!(capture.split_calls[1].post_position.x.bits, 1155278898);
        assert_eq!(position.x.to_bits(), 1155278898);
        assert_eq!(position.y.to_bits(), 1148896312);
        assert_ne!(
            position.x.to_bits(),
            1155278899,
            "one aggregated eight-unit commit reproduces the captured +1 ULP defect"
        );
    }

    #[test]
    fn zero_distance_first_fast_wall_call_still_executes_the_second() {
        let (position, capture) = run_fast_wall_anti_collision_fixture(0);
        assert_ne!(
            position.x.to_bits(),
            1155272038,
            "the nonzero second PerformMotion must still commit"
        );
        assert_eq!(
            capture.split_calls.len(),
            1,
            "Original emits no movement-step record for the zero-distance first call"
        );
        assert_eq!(capture.split_calls[0].frame_distance_raw.value, 4.0);
        assert_eq!(capture.split_calls[0].pre_position.x.bits, 1155272038);
    }

    #[test]
    fn running_stairs_second_call_observes_first_call_arrival_snap() {
        use crate::element::{
            ActorData, ActorPc, ElementData, ElementKind, HumanData, PcData, Posture,
        };
        use crate::order::Order;
        use crate::sequence::SequenceElement;
        use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

        let mut engine = EngineInner::new();
        let start = MapPoint::new(697.114_87, 1_420.993_2);
        let goal = MapPoint::new(696.0, 1423.0);
        let physical = OrderType::WalkingStairs;
        let script = SpriteScript {
            action_id: physical as u16,
            action_done: 7,
            average_speed: 5.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 40,
            frame_ids: vec![1; 8],
            delays: vec![0; 8],
            distances: vec![5; 8],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 8],
            sound_ids: vec![0; 8],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[physical as usize] = 0;
        let mut pc = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        });
        pc.element_data_mut().active = true;
        pc.element_data_mut().sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );
        pc.element_data_mut().set_position_map(start);
        pc.element_data_mut().set_direction_instantly(11);
        pc.position_iface_mut().set_anti_collision_on(false);
        let owner = engine.add_entity(pc);

        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::RunningStairs,
        );
        movement
            .orders
            .push_back(Order::test_new(OrderType::RunningStairs, goal.x, goal.y));
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .active_movement = ActiveMovement::new(sequence, 0);

        crate::movement_diagnostics::begin_parity_movement_capture();
        engine.tick_entity_movement(&crate::sim_rng::test_context(), &LevelAssets::new());
        let capture = crate::movement_diagnostics::take_parity_movement_capture()
            .into_iter()
            .find(|capture| capture.entity == owner)
            .expect("running-stairs owner must emit a production movement capture");

        assert_eq!(capture.split_calls.len(), 2);
        assert_eq!(
            capture.split_calls[0].pre_position.x.bits,
            start.x.to_bits()
        );
        assert_eq!(
            capture.split_calls[0].post_position.x.bits,
            goal.x.to_bits()
        );
        assert_eq!(
            capture.split_calls[0].post_position.y.bits,
            goal.y.to_bits()
        );
        assert_eq!(capture.split_calls[1].pre_position.x.bits, goal.x.to_bits());
        assert_eq!(capture.split_calls[1].pre_position.y.bits, goal.y.to_bits());
        assert_ne!(
            capture.split_calls[0].requested_delta.x.bits, 0,
            "the first call must genuinely overshoot before its arrival snap"
        );
        assert_ne!(
            capture.split_calls[1].requested_delta.x.bits, 0,
            "RunningStairs must still execute its second PerformMotion after termination"
        );

        let offset_point = start + crate::coordinates::MapVec::new(7.0, -3.0);
        let line_a = start + crate::coordinates::MapVec::new(-2.0, 4.0);
        let line_b = start + crate::coordinates::MapVec::new(6.0, 4.0);
        let mut snapshot = crate::engine::anti_collision::ActorSnapshot {
            id: owner,
            active: true,
            is_actor: true,
            is_human: true,
            is_ignored_for_anti_collision: false,
            position_map: start,
            layer: 0,
            sector: None,
            posture: Posture::Upright,
            element_kind: ElementKind::ActorPc,
            target_element: None,
            is_swordfighting: false,
            repulsive_point: None,
            extra_repulsive_points: vec![crate::repulsive::RepulsivePoint::new(
                offset_point,
                4.0,
                12.0,
            )],
            repulsive_lines: vec![crate::repulsive::RepulsiveLine::new(
                line_a, line_b, 0.0, 5.0,
            )],
        };
        sync_snapshot_after_committed_step(&mut snapshot, start, goal);
        let committed = goal - start;
        assert_eq!(
            snapshot.extra_repulsive_points[0].position,
            offset_point + committed,
            "offset repulsive geometry must follow the snapped commit, not the raw overshoot"
        );
        assert_eq!(snapshot.repulsive_lines[0].a, line_a + committed);
        assert_eq!(snapshot.repulsive_lines[0].b, line_b + committed);
    }

    fn run_running_stairs_outer_crossing_fixture(
        crosses_first_substep: bool,
    ) -> (
        EngineInner,
        EntityId,
        crate::movement_diagnostics::ParityMovementStep,
        crate::fast_find_grid::LineIndex,
    ) {
        use crate::element::{
            ActorData, ActorPc, ElementData, ElementKind, HumanData, PcData, Posture,
        };
        use crate::fast_find_grid::GridLine;
        use crate::order::Order;
        use crate::sequence::SequenceElement;
        use crate::sight_obstacle::SightObstacle;
        use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(30, 30);
        engine.world.fast_grid_mut().allocate_layers(1);

        let start = MapPoint::new(697.114_87, 1_420.993_2);
        let goal = MapPoint::new(696.0, 1423.0);
        let physical = OrderType::WalkingStairs;
        let script = SpriteScript {
            action_id: physical as u16,
            action_done: 7,
            average_speed: 5.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 40,
            frame_ids: vec![1; 8],
            delays: vec![0; 8],
            distances: vec![5; 8],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 8],
            sound_ids: vec![0; 8],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[physical as usize] = 0;
        let mut pc = Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        });
        pc.element_data_mut().sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );
        pc.element_data_mut().set_position_map(start);
        pc.element_data_mut().set_direction_instantly(11);
        pc.position_iface_mut().set_anti_collision_on(false);
        let owner = engine.add_entity(pc);

        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::RunningStairs,
        );
        movement
            .orders
            .push_back(Order::test_new(OrderType::RunningStairs, goal.x, goal.y));
        movement.orders.push_back(Order::test_new(
            OrderType::RunningStairs,
            goal.x - 20.0,
            goal.y + 20.0,
        ));
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .active_movement = ActiveMovement::new(sequence, 0);

        // A short perpendicular bond through the midpoint is crossed only
        // by the first start->goal substep. The control translates that
        // bond away from the complete outer movement segment.
        let midpoint = MapPoint::new((start.x + goal.x) * 0.5, (start.y + goal.y) * 0.5);
        let travel = goal - start;
        let offset_x = if crosses_first_substep { 0.0 } else { 30.0 };
        let line_center = MapPoint::new(midpoint.x + offset_x, midpoint.y);
        let line_a = MapPoint::new(
            line_center.x - travel.y * 2.0,
            line_center.y + travel.x * 2.0,
        );
        let line_b = MapPoint::new(
            line_center.x + travel.y * 2.0,
            line_center.y - travel.x * 2.0,
        );
        let line_index = engine
            .world
            .fast_grid_mut()
            .add_line(GridLine::new_elevation(line_a, line_b, None, Some(0)), 0);

        let mut ramp = SightObstacle::new_default(1);
        // z = 0.5*x - 340: a real sloped plane whose 3D movement vector
        // changes the facing selected by ComputeIncrementAll.
        ramp.top_plane_points = [
            [690.0, 1400.0, 5.0],
            [710.0, 1400.0, 15.0],
            [690.0, 1450.0, 5.0],
        ];
        let mut assets = LevelAssets::new();
        assets.static_sight_obstacles = std::sync::Arc::new(vec![ramp]);
        engine.world.static_sight_obstacle_active = vec![true];

        crate::movement_diagnostics::begin_parity_movement_capture();
        engine.tick_entity_movement(&crate::sim_rng::test_context(), &assets);
        let capture = crate::movement_diagnostics::take_parity_movement_capture()
            .into_iter()
            .find(|capture| capture.entity == owner)
            .expect("running-stairs owner must emit a production movement capture");
        (engine, owner, capture, line_index)
    }

    #[test]
    fn running_stairs_crossing_uses_outer_pre_position() {
        let (engine, owner, capture, line_index) = run_running_stairs_outer_crossing_fixture(true);
        assert_eq!(capture.split_calls.len(), 2);
        let outer_pre = MapPoint::new(
            capture.split_calls[0].pre_position.x.value,
            capture.split_calls[0].pre_position.y.value,
        );
        let first_post = MapPoint::new(
            capture.split_calls[0].post_position.x.value,
            capture.split_calls[0].post_position.y.value,
        );
        let second_post = MapPoint::new(
            capture.split_calls[1].post_position.x.value,
            capture.split_calls[1].post_position.y.value,
        );
        assert_eq!(
            engine
                .world
                .fast_grid
                .get_crossing_elevation_line_indices(0, outer_pre, first_post),
            vec![line_index],
            "the bond must be crossed by the first literal PerformMotion commit"
        );
        assert!(
            engine
                .world
                .fast_grid
                .get_crossing_elevation_line_indices(0, first_post, second_post)
                .is_empty(),
            "the post-first position used by the old Rust code must miss the bond"
        );

        let entity = engine.get_entity(owner).unwrap();
        assert_eq!(
            entity.element_data().obstacle_index().map(u16::from),
            Some(0)
        );
        let pi = entity.position_iface();
        assert!(
            pi.get_position().z > 0.0,
            "crossing must project onto the ramp"
        );
        assert_ne!(
            pi.get_increment().z,
            0.0,
            "the ramp must rebuild the 3D increment"
        );
        assert_eq!(
            pi.get_direction_goal(),
            crate::position_interface::vector_to_direction(
                pi.get_increment().x,
                pi.get_increment().y,
            ),
            "the post-crossing ComputeIncrementAll must refresh direction_goal"
        );
    }

    #[test]
    fn running_stairs_without_outer_crossing_keeps_ground_plane() {
        let (engine, owner, capture, line_index) = run_running_stairs_outer_crossing_fixture(false);
        let outer_pre = MapPoint::new(
            capture.split_calls[0].pre_position.x.value,
            capture.split_calls[0].pre_position.y.value,
        );
        let second_post = MapPoint::new(
            capture.split_calls[1].post_position.x.value,
            capture.split_calls[1].post_position.y.value,
        );
        assert!(
            engine
                .world
                .fast_grid
                .get_crossing_elevation_line_indices(0, outer_pre, second_post)
                .is_empty()
        );
        assert!(engine.world.fast_grid.level.lines[usize::from(line_index)].is_elevation);
        let entity = engine.get_entity(owner).unwrap();
        assert_eq!(entity.element_data().obstacle_index(), None);
        assert_eq!(entity.position_iface().get_position().z, 0.0);
    }

    #[test]
    fn entity_seek_hides_initial_sword_motion_start_from_execute() {
        use crate::sprite::MotionState;

        assert_eq!(
            movement_execute_visible_motion(
                OrderType::RunningWithSword,
                MotionState::Start,
                false,
                true,
            ),
            MotionState::InProgress,
            "entity-target PerformSeek returns IN_PROGRESS around the sprite's START"
        );
        assert_eq!(
            movement_execute_state_effect(
                OrderType::RunningWithSword,
                movement_execute_visible_motion(
                    OrderType::RunningWithSword,
                    MotionState::Start,
                    false,
                    true,
                ),
            ),
            None,
            "the Human Execute switch must retain WaitingSword"
        );
        assert_eq!(
            movement_execute_visible_motion(
                OrderType::RunningWithSword,
                MotionState::Start,
                false,
                false,
            ),
            MotionState::Start,
            "point and ordinary movement expose the sprite's START"
        );
        assert_eq!(
            movement_execute_visible_motion(
                OrderType::RunningUpright,
                MotionState::Start,
                false,
                true,
            ),
            MotionState::Start,
            "running upright sets MovingFast after PerformSeek unconditionally"
        );
    }

    #[test]
    fn exact_goal_motion_does_not_wait_for_nonzero_animation_speed() {
        assert!(
            !stationary_motion_waits(0.0, false, 0.0),
            "an exact-position walk must reach the shared arrival tail on its first Execute"
        );
        assert!(
            stationary_motion_waits(0.0, false, 1.0),
            "a stationary motion away from its goal must remain current"
        );
        assert!(
            !stationary_motion_waits(0.0, true, 1.0),
            "a pre-motion seek-tolerance arrival must complete without displacement"
        );
        assert!(
            stationary_motion_waits(0.0, false, f32::NAN),
            "a zero-distance animation frame must not multiply an invalid movement increment by zero"
        );
    }

    #[test]
    fn first_fast_climb_commit_keeps_lift_forecast_when_second_call_is_stationary() {
        use crate::coordinates::WorldVec3D;

        let script = crate::sprite_script::SpriteScript {
            action_id: OrderType::ClimbingWallUp as u16,
            action_done: 0,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1],
            delays: vec![1],
            distances: vec![0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        let mut sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script]),
            std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]),
        );

        let set_increment_z = |sprite: &mut crate::sprite::Sprite, z| {
            let mut state = sprite.position_iface.v48_serialized_state();
            state.increment = WorldVec3D::new(0.25, -0.5, z);
            sprite.position_iface.restore_v48_serialized_state(state);
        };

        set_increment_z(&mut sprite, 0.75);
        refresh_motion_forecast(&mut sprite, 4.0, None);
        assert_eq!(
            sprite.position_iface.get_forecasted_movement(),
            WorldVec3D::new(0.5, -1.0, 1.5),
            "an upward first PerformMotion commit must publish positive lift movement"
        );

        // A zero-distance second PerformMotion never reaches Original's
        // forecast write. The positive first-call value therefore remains.
        refresh_motion_forecast(&mut sprite, 0.0, Some((0.0, 0.0)));
        assert_eq!(
            sprite.position_iface.get_forecasted_movement(),
            WorldVec3D::new(0.5, -1.0, 1.5),
            "a stationary second call must not clear the first call's forecast"
        );

        set_increment_z(&mut sprite, -0.75);
        refresh_motion_forecast(&mut sprite, 4.0, None);
        assert_eq!(
            sprite.position_iface.get_forecasted_movement(),
            WorldVec3D::new(0.5, -1.0, -1.5),
            "a downward first PerformMotion commit must publish negative lift movement"
        );
    }

    #[test]
    fn only_nonzero_distance_transition_frames_recompute_an_exact_position() {
        assert!(motion_recomputes_exact_position(true, true, 2.0, 0.0));
        assert!(
            !motion_recomputes_exact_position(false, true, 2.0, 0.0),
            "an ordinary exact-goal move takes the arrival path and must preserve its coordinates"
        );
        assert!(
            !motion_recomputes_exact_position(true, true, 0.0, 0.0),
            "a zero-distance transition frame never enters Original's position-update block"
        );
    }

    #[test]
    fn exact_goal_transition_clears_stale_running_forecast() {
        use crate::element::{
            ActionState, ActorData, ActorPc, ElementData, ElementKind, HumanData, PcData, Posture,
        };
        use crate::order::Order;
        use crate::sequence::{SequenceElement, SequencePriority};
        use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

        let mut engine = EngineInner::new();
        let position = MapPoint::new(100.0, 100.0);
        let transition = OrderType::TransitionRunningUprightWaitingUpright;
        let script = SpriteScript {
            action_id: transition as u16,
            action_done: 2,
            average_speed: 2.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 6,
            frame_ids: vec![1, 2, 3],
            delays: vec![0; 3],
            distances: vec![2; 3],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
            sound_ids: vec![0; 3],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[transition as usize] = 0;

        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );
        element.sprite.last_action = transition;
        element
            .sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -4.0, -4.0, 4.0, 4.0,
            ));
        element.sprite.position_iface.set_anti_collision_on(true);
        element.sprite.position_iface.deviated = true;
        element.set_position_map(position);
        element
            .sprite
            .position_iface
            .set_map_goal(MapPoint::new(110.0, 100.0));
        element.sprite.position_iface.compute_increment_all(true);
        element
            .sprite
            .position_iface
            .update_forecasted_movement(5.0, 1);
        assert_ne!(
            element.sprite.position_iface.get_forecasted_movement(),
            crate::coordinates::WorldVec3D::ZERO
        );
        element.sprite.position_iface.zero_all_increments();

        let owner = engine.add_entity(Entity::Pc(ActorPc {
            element,
            actor: ActorData {
                action_state: ActionState::MovingFast,
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData::default(),
        }));
        let order_id = engine.orders.allocate_order_id();
        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::RunningUpright,
        );
        movement.priority = SequencePriority::Normal;
        movement
            .orders
            .push_back(Order::new(transition, position.x, position.y, order_id));
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .active_movement = ActiveMovement::new(sequence, 0);

        engine.tick_entity_movement(&crate::sim_rng::test_context(), &LevelAssets::new());

        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .position_iface()
                .get_forecasted_movement(),
            crate::coordinates::WorldVec3D::ZERO,
            "Original's nonzero animation-distance block refreshes the forecast with the zero goal increment"
        );
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .element_data()
                .position_map(),
            position,
            "the forecast refresh must not move an actor already at the transition goal"
        );
        assert!(
            !engine
                .get_entity(owner)
                .unwrap()
                .position_iface()
                .is_deviated(),
            "Original still runs zero-increment anti-collision recovery for a nonzero transition animation distance"
        );
    }

    #[test]
    fn entity_seek_hides_transition_done_until_wrapper_termination() {
        use crate::element::ActionState;
        use crate::sprite::MotionState;

        let visible = movement_execute_visible_motion(
            OrderType::TransitionRunningUprightWalkingUpright,
            MotionState::Done,
            false,
            true,
        );
        assert_eq!(visible, MotionState::InProgress);
        assert_eq!(
            movement_execute_state_effect(
                OrderType::TransitionRunningUprightWalkingUpright,
                visible,
            ),
            None,
            "raw sprite DONE must not change a live entity seek from MovingFast to Moving"
        );
        assert_eq!(
            movement_execute_visible_motion(
                OrderType::TransitionRunningUprightWalkingUpright,
                MotionState::Terminated,
                false,
                true,
            ),
            MotionState::Terminated,
            "the Execute switch must observe the seek wrapper's terminal result"
        );
        assert_eq!(
            movement_execute_visible_motion(
                OrderType::TransitionRunningUprightWalkingUpright,
                MotionState::Done,
                false,
                false,
            ),
            MotionState::Done,
            "ordinary and point movement expose the sprite's DONE result"
        );
        assert_eq!(
            movement_execute_state_effect(
                OrderType::TransitionRunningUprightWalkingUpright,
                MotionState::Terminated,
            ),
            Some((crate::element::Posture::Upright, ActionState::Moving))
        );
    }

    #[test]
    fn committed_step_arrival_outranks_every_sprite_result() {
        use crate::element::{ActionState, Posture};
        use crate::sprite::MotionState;

        for motion in [
            MotionState::Start,
            MotionState::InProgress,
            MotionState::Done,
        ] {
            let visible =
                movement_execute_visible_motion(OrderType::WalkingWithCorpse, motion, true, false);
            assert_eq!(
                visible,
                MotionState::Terminated,
                "a step that satisfies the goal predicate reaches Execute as a termination"
            );
            assert_eq!(
                committed_arrival_post_completion_override(motion, visible, true),
                Some(MotionState::Terminated),
                "staged arrival must preserve the Execute result for the post-completion latch"
            );
        }
        assert_eq!(
            movement_execute_state_effect(
                OrderType::WalkingWithCorpse,
                movement_execute_visible_motion(
                    OrderType::WalkingWithCorpse,
                    MotionState::InProgress,
                    true,
                    false,
                ),
            ),
            Some((Posture::CarryingCorpse, ActionState::Waiting)),
            "the corpse carrier settles back to Waiting on the waypoint it reaches"
        );
        assert_eq!(
            movement_execute_state_effect(
                OrderType::WalkingWithCorpse,
                movement_execute_visible_motion(
                    OrderType::WalkingWithCorpse,
                    MotionState::InProgress,
                    false,
                    false,
                ),
            ),
            None,
            "a walk that has not reached its waypoint owns no state effect"
        );
    }

    #[test]
    fn entity_seek_refresh_countdown_preserves_original_unsigned_wrap() {
        assert_eq!(age_seek_refresh_wait(25), 24);
        assert_eq!(age_seek_refresh_wait(0), u32::MAX);
        assert_eq!(age_seek_refresh_wait(u32::MAX), u32::MAX - 1);
    }

    #[test]
    fn seek_refresh_dispatch_follows_selected_execute_arm() {
        assert_eq!(perform_seek_calls_per_execute(OrderType::WalkingUpright), 1);
        assert_eq!(perform_seek_calls_per_execute(OrderType::RunningStairs), 2);
        assert_eq!(perform_seek_calls_per_execute(OrderType::ClimbingWallUp), 0);
        assert_eq!(
            perform_seek_calls_per_execute(OrderType::ClimbingWallDown),
            0
        );
    }

    #[test]
    fn final_path_metadata_depends_on_raw_waypoint_count() {
        let antagonist = Some(EntityId::Pc(crate::entity_id::PcId(9)));

        assert_eq!(
            original_final_path_metadata(1, 28.9, antagonist),
            (0.0, None)
        );
        assert_eq!(
            original_final_path_metadata(2, 28.9, antagonist),
            (28.9, antagonist)
        );
    }

    #[test]
    fn path_source_is_skipped_before_postprocess_even_when_nan_breaks_source_equality() {
        let mut waypoints = vec![
            MapPoint::new(f32::from_bits(0xffc0_0000), f32::from_bits(0xffc0_0000)),
            MapPoint::new(342.5066, 1546.6641),
        ];

        let raw_waypoint_count = prepare_path_waypoints_for_postprocess(&mut waypoints, false);

        assert_eq!(raw_waypoint_count, 2);
        assert_eq!(waypoints, vec![MapPoint::new(342.5066, 1546.6641)]);
    }

    #[test]
    fn only_explicit_in_place_movement_transitions_accept_zero_target() {
        assert!(is_in_place_movement_transition(
            OrderType::TransitionSpecialWaitingUpright
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionWaitingUprightSpecial
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionWaitingUprightBoredWaitingUpright
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionWaitingUprightWaitingUprightBored
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionCrouchingUp
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionCrouchingDown
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionSittingWaitingUpright
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionLeaningOutWaitingAlerted
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionClimbingWallDownWaitingUpright
        ));
        assert!(is_in_place_movement_transition(OrderType::StandingUp));
        assert!(is_in_place_movement_transition(OrderType::StandingUpSword));
        assert!(is_in_place_movement_transition(OrderType::StandingUpBow));
        assert!(is_in_place_movement_transition(OrderType::LoweringShield));
        assert!(!is_in_place_movement_transition(
            OrderType::TransitionWaitingUprightWalkingUpright
        ));
        assert!(!is_in_place_movement_transition(
            OrderType::TransitionWalkingUprightWaitingUpright
        ));
    }

    #[test]
    fn elevation_crossing_matches_null_obstacle_side() {
        assert_eq!(
            EngineInner::crossed_elevation_obstacle(None, None, Some(50)),
            Some(Some(50))
        );
        assert_eq!(
            EngineInner::crossed_elevation_obstacle(Some(50), None, Some(50)),
            Some(None)
        );
        assert_eq!(
            EngineInner::crossed_elevation_obstacle(Some(49), Some(49), Some(50)),
            Some(Some(50))
        );
        assert_eq!(
            EngineInner::crossed_elevation_obstacle(Some(99), Some(49), Some(50)),
            None
        );
    }

    #[test]
    fn command_extraction_expands_move_box_like_original() {
        let bbox = MapBBox::from_coords(10.0, 20.0, 30.0, 40.0);
        let expanded = EngineInner::expand_move_box_for_command_extraction(bbox);
        assert_eq!(expanded.x_min(), 9.5);
        assert_eq!(expanded.y_min(), 19.5);
        assert_eq!(expanded.x_max(), 30.5);
        assert_eq!(expanded.y_max(), 40.5);
    }

    fn dispatch_instruction_extraction_fixture(
        command: Command,
        obstruct_owner: bool,
        self_seek: bool,
    ) -> (
        MapPoint,
        MapPoint,
        SequenceState,
        bool,
        bool,
        Option<crate::actor_state::ActorSeekSector>,
        u16,
    ) {
        use crate::fast_find_grid::GridLine;
        use crate::position_interface::SectorHandle;

        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(4, 4);
        engine.world.fast_grid_mut().allocate_layers(1);
        if obstruct_owner {
            engine.world.fast_grid_mut().add_line(
                GridLine::new(MapPoint::new(0.0, 128.0), MapPoint::new(256.0, 128.0), true),
                0,
            );
        }

        let start = MapPoint::new(130.0, 130.0);
        let mut owner_entity = extraction_test_pc(Posture::Upright);
        owner_entity.element_data_mut().set_position_map(start);
        owner_entity.element_data_mut().set_layer(0);
        owner_entity
            .element_data_mut()
            .set_sector(SectorHandle::new(1));
        owner_entity
            .position_iface_mut()
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -10.0, -5.0, 10.0, 5.0,
            ));
        owner_entity.position_iface_mut().set_map_position(start);
        let owner = engine.add_entity(owner_entity);
        {
            let actor = engine
                .get_entity_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.continuation.seek_to_point = true;
            actor.continuation.seek_layer = 7;
            actor.continuation.seek_sector = Some(crate::actor_state::ActorSeekSector::Position(
                SectorHandle::new(1).unwrap(),
            ));
        }

        let mut target_entity = Entity::Soldier(ActorSoldier {
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                posture: Posture::Upright,
                ..Default::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            soldier: SoldierData::default(),
        });
        let target_position = MapPoint::new(200.0, 200.0);
        target_entity
            .element_data_mut()
            .set_position_map(target_position);
        target_entity.element_data_mut().set_layer(0);
        target_entity
            .element_data_mut()
            .set_sector(SectorHandle::new(2));
        target_entity
            .position_iface_mut()
            .set_map_position(target_position);
        let target = engine.add_entity(target_entity);

        let mut movement =
            SequenceElement::new_movement(1, command, Some(owner), OrderType::WalkingUpright);
        movement.priority = SequencePriority::Normal;
        movement.posture_after_transition = Posture::Upright;
        movement.action_state_after_transition = ActionState::Waiting;
        let SequenceElementData::Movement {
            destination,
            element,
            flags,
            tolerance,
            post_seek_sequence,
            ..
        } = &mut movement.data
        else {
            unreachable!("new_movement must create movement data")
        };
        *destination = target_position;
        *tolerance = 20.0;
        if command == Command::Seek {
            *element = Some(if self_seek { owner } else { target });
            *flags |= MoveFlags::SEEK;
            if self_seek {
                let mut post_seek = Sequence::new();
                post_seek.append_element(SequenceElement::new(1, Command::Wait, Some(owner)));
                *post_seek_sequence = Some(post_seek.into_post_seek());
            }
        } else {
            *flags |= MoveFlags::MAP;
        }
        let sequence = engine.orders.sequence_manager.launch_element(movement);

        let before = engine
            .get_entity(owner)
            .expect("fixture owner exists before dispatch")
            .element_data()
            .position_map();
        let _ = engine.dispatch_ordered_move_seek_instruct(
            &crate::sim_rng::test_context(),
            &extraction_test_assets(),
            owner,
            sequence,
            0,
        );
        let after = engine
            .get_entity(owner)
            .expect("fixture owner exists after dispatch")
            .element_data()
            .position_map();
        let state = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("fixture movement remains inspectable")
            .state;
        let post_seek_launched = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .any(|candidate| {
                candidate.id != sequence
                    && candidate.elements.iter().any(|element| {
                        element.owner == Some(owner) && element.command == Command::Wait
                    })
            });
        let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
        (
            before,
            after,
            state,
            post_seek_launched,
            actor.continuation.seek_to_point,
            actor.continuation.seek_sector,
            actor.continuation.seek_layer,
        )
    }

    #[test]
    fn cross_sector_seek_extracts_owner_before_refresh_route_failure() {
        let (before, after, state, _, seek_to_point, seek_sector, seek_layer) =
            dispatch_instruction_extraction_fixture(Command::Seek, true, false);
        assert_ne!(
            after, before,
            "the old late-only extraction leaves a cross-sector Seek owner embedded"
        );
        assert_eq!(state, SequenceState::Impossible);
        assert!(!seek_to_point, "entity Seek must replace stale point mode");
        assert_eq!(
            seek_sector,
            Some(crate::actor_state::ActorSeekSector::Position(
                crate::position_interface::SectorHandle::new(1).unwrap()
            )),
            "entity Seek preserves dormant point-sector metadata"
        );
        assert_eq!(seek_layer, 7, "entity Seek preserves dormant point layer");
    }

    #[test]
    fn authorized_cross_sector_seek_does_not_move_owner() {
        let (before, after, state, _, seek_to_point, seek_sector, seek_layer) =
            dispatch_instruction_extraction_fixture(Command::Seek, false, false);
        assert_eq!(after, before);
        assert_eq!(state, SequenceState::Impossible);
        assert!(!seek_to_point);
        assert_eq!(
            seek_sector,
            Some(crate::actor_state::ActorSeekSector::Position(
                crate::position_interface::SectorHandle::new(1).unwrap()
            ))
        );
        assert_eq!(seek_layer, 7);
    }

    #[test]
    fn direct_move_keeps_instruction_boundary_extraction() {
        let (before, after, state, _, _, _, _) =
            dispatch_instruction_extraction_fixture(Command::Move, true, false);
        assert_ne!(after, before);
        assert_eq!(state, SequenceState::InProgress);
    }

    #[test]
    fn unauthorized_self_seek_skips_extraction_and_launches_post_seek() {
        let (before, after, state, post_seek_launched, _, _, _) =
            dispatch_instruction_extraction_fixture(Command::Seek, true, true);
        assert_eq!(after, before, "self-Seek returns before MOVE extraction");
        assert_eq!(state, SequenceState::Terminated);
        assert!(post_seek_launched, "self-Seek still launches its successor");
    }

    #[test]
    fn face_opponent_uses_original_displacement_to_facing_angle_sign() {
        use crate::element::ActionState;

        let right = combat_movement_angle((1.0, 0.0), (0.0, 1.0));
        assert_eq!(
            combat_directional_animation(ActionState::MovingSword, right),
            OrderType::StrafingRightSword,
            "Angle(eastward displacement, northward facing) is +90 degrees"
        );

        let left = combat_movement_angle((1.0, 0.0), (0.0, -1.0));
        assert_eq!(
            combat_directional_animation(ActionState::MovingSword, left),
            OrderType::StrafingLeftSword,
            "reversing the facing vector selects the opposite strafe"
        );

        // A destination the actor already stands on leaves both the dot and
        // the determinant at zero, which the Original resolves as a half turn
        // regardless of where the opponent is.
        for facing in [(0.0, 1.0), (1.0, 0.0), (-3.0, 7.5)] {
            assert_eq!(
                combat_directional_animation(
                    ActionState::MovingSword,
                    combat_movement_angle((0.0, 0.0), facing)
                ),
                OrderType::WalkingBackwardsSword,
                "a zero-length displacement walks backwards, not toward the facing sector"
            );
        }

        // Collinear vectors still resolve through the determinant test.
        assert_eq!(
            combat_directional_animation(
                ActionState::MovingSword,
                combat_movement_angle((2.0, 0.0), (5.0, 0.0))
            ),
            OrderType::WalkingSword,
            "moving straight at the opponent walks forward"
        );
        assert_eq!(
            combat_directional_animation(
                ActionState::MovingSword,
                combat_movement_angle((2.0, 0.0), (-5.0, 0.0))
            ),
            OrderType::WalkingBackwardsSword,
            "moving directly away from the opponent walks backwards"
        );
    }

    #[test]
    fn combat_nan_angle_uses_original_release_fallback_animation() {
        use crate::element::ActionState;

        let angle = combat_movement_angle((f32::NAN, f32::NAN), (1.0, 0.0));
        assert!(angle.is_nan());
        assert_eq!(
            combat_directional_animation(ActionState::MovingSword, angle),
            OrderType::WalkingSword
        );
        assert_eq!(
            combat_directional_animation(ActionState::MovingShield, angle),
            OrderType::WalkingShield
        );
    }

    #[test]
    fn face_opponent_direction_uses_isometric_map_aspect() {
        use crate::position_interface::{
            ASPECT_RATIO, vector_to_sector_0_to_15, vector_to_sector_0_to_15_iso,
        };

        let bare = vector_to_sector_0_to_15(1.0, ASPECT_RATIO);
        let isometric = vector_to_sector_0_to_15_iso(1.0, ASPECT_RATIO);
        assert_eq!(
            isometric, 6,
            "isometric stretch restores a 45-degree vector"
        );
        assert_ne!(
            bare, isometric,
            "raw map-space binning must not replace GetSector0to15(ASPECT_RATIO)"
        );
    }
}
