
use super::*;
use crate::element::{
    ActorData, ActorPc, ActorSoldier, AiBrain, Command, ElementData, ElementKind, Entity,
    HumanData, PcData, Posture, SoldierData,
};
use crate::movement::ActiveMovement;
use crate::position_interface::SectorHandle;
use crate::sequence::{SequenceElementData, SequenceState};

#[test]
fn seek_refresh_expiry_treats_wrapped_high_bit_counter_as_expired() {
    assert!(!seek_refresh_wait_elapsed(25));
    assert!(!seek_refresh_wait_elapsed(1));
    assert!(seek_refresh_wait_elapsed(0));
    assert!(seek_refresh_wait_elapsed(u32::MAX));
    assert!(seek_refresh_wait_elapsed(i32::MIN as u32));
}

fn drop_ale_exit_doors() -> Vec<crate::gate::Door> {
    let mut doors = (0..8)
        .map(|_| crate::gate::Door {
            active: false,
            ..crate::gate::Door::default()
        })
        .collect::<Vec<_>>();
    doors[6] = crate::gate::Door {
        active: true,
        sector_in: crate::sector::SectorNumber::new(133),
        sector_out: crate::sector::SectorNumber::new(25),
        ..crate::gate::Door::default()
    };
    doors[7] = crate::gate::Door {
        active: true,
        sector_in: crate::sector::SectorNumber::new(133),
        sector_out: crate::sector::SectorNumber::new(22),
        ..crate::gate::Door::default()
    };
    doors
}

fn recorded_route(
    source_sector: SectorHandle,
    source_layer: u16,
    outcome: crate::gate::RecordedGateOutcome,
) -> crate::gate::RecordedGatePath {
    crate::gate::RecordedGatePath {
        source_sector: crate::sector::SectorNumber::new(u16::from(source_sector) as i16),
        source_sector_index: source_sector.arena_index(),
        source_layer,
        outcome,
    }
}

#[test]
fn recorded_drop_ale_gate_path_preserves_original_exit_over_alternate() {
    let expected = vec![crate::gate::GatePathStep {
        door_index: crate::gate::DoorIndex::new(7).expect("valid door index"),
        direct: false,
    }];
    assert_eq!(
        recorded_point_seek_gate_path(
            &drop_ale_exit_doors(),
            SectorHandle::new(133).unwrap(),
            11,
            SectorHandle::new(22).unwrap(),
            None,
        ),
        None,
        "live commands without recorded provenance must fall through to A*"
    );
    assert_eq!(
        recorded_point_seek_gate_path(
            &drop_ale_exit_doors(),
            SectorHandle::new(133).unwrap(),
            11,
            SectorHandle::new(22).unwrap(),
            Some(recorded_route(
                SectorHandle::new(133).unwrap(),
                11,
                crate::gate::RecordedGateOutcome::Success(expected.clone()),
            )),
        ),
        Some(Some(expected)),
        "Save034/r035 must retain Original gate 7 rather than recompute alternate gate 6"
    );
    assert_eq!(
        recorded_point_seek_gate_path(
            &drop_ale_exit_doors(),
            SectorHandle::new(133).unwrap(),
            11,
            SectorHandle::new(22).unwrap(),
            Some(recorded_route(
                SectorHandle::new(133).unwrap(),
                11,
                crate::gate::RecordedGateOutcome::Failure,
            )),
        ),
        Some(None),
        "an observed Original search failure must suppress live A*"
    );
}

#[test]
fn recorded_drop_ale_route_uses_dispatch_time_door_adapted_source() {
    let doors = drop_ale_exit_doors();
    let raw_owner_sector = SectorHandle::new(22).unwrap();
    let goal_sector = SectorHandle::new(22).unwrap();
    assert!(seek_sectors_match(raw_owner_sector, goal_sector));
    let (_, adapted_source, adapted_layer) =
        crate::engine::movement::adapt_source_to_current_door_with_identity(
            &doors,
            crate::position_interface::DoorHandle::new(7).expect("valid door index"),
            true,
        )
        .expect("a straddling actor adapts to the gate's in side at Seek dispatch");
    assert!(!seek_sectors_match(adapted_source, goal_sector));
    assert_eq!(
        recorded_point_seek_gate_path(
            &doors,
            adapted_source,
            adapted_layer,
            goal_sector,
            Some(recorded_route(
                adapted_source,
                adapted_layer,
                crate::gate::RecordedGateOutcome::Success(vec![crate::gate::GatePathStep {
                    door_index: crate::gate::DoorIndex::new(7).expect("valid door index"),
                    direct: false,
                }]),
            )),
        ),
        Some(Some(vec![crate::gate::GatePathStep {
            door_index: crate::gate::DoorIndex::new(7).expect("valid door index"),
            direct: false,
        }])),
        "raw owner==goal must not suppress a route whose dispatch-time door source differs"
    );
}

fn replay_owned_point_seek_fixture() -> (
    crate::engine::EngineInner,
    EntityId,
    SequenceId,
    crate::coordinates::MapPoint,
) {
    let mut engine = crate::engine::EngineInner::new();
    engine.scripts.mission = Some(minimal_mission());
    engine.script_domains.interactables.doors = drop_ale_exit_doors();
    let owner = engine.add_entity(test_pc_at(100.0, 100.0, 133));
    let destination = crate::coordinates::MapPoint::new(778.0, 1714.0);
    let sequence_id = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new_movement(
            1,
            Command::Seek,
            Some(owner),
            OrderType::WalkingUpright,
        ));
    (engine, owner, sequence_id, destination)
}

#[test]
#[should_panic(expected = "has no admitted RecordedDropAleRoute ExternalFact")]
fn original_replay_point_seek_rejects_missing_recorded_outcome() {
    let (mut engine, owner, sequence_id, destination) = replay_owned_point_seek_fixture();

    engine.try_dispatch_cross_sector_point_seek(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        owner,
        sequence_id,
        0,
        destination,
        SectorHandle::new(22),
        0,
        OrderType::WalkingUpright,
        MoveFlags::SEEK,
        0.0,
        None,
        crate::sequence::PointSeekRouteProvenance::OriginalReplay,
    );
}

#[test]
fn live_point_seek_without_recorded_outcome_uses_gate_graph() {
    let (mut engine, owner, sequence_id, destination) = replay_owned_point_seek_fixture();

    assert!(engine.try_dispatch_cross_sector_point_seek(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        owner,
        sequence_id,
        0,
        destination,
        SectorHandle::new(22),
        0,
        OrderType::WalkingUpright,
        MoveFlags::SEEK,
        0.0,
        None,
        crate::sequence::PointSeekRouteProvenance::Live,
    ));
    assert!(
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| &sequence.elements)
            .any(|element| matches!(
                element.data,
                SequenceElementData::Movement { gate_id: Some(door), .. }
                    if door.get() == 7
            )),
        "live point Seek must fall back to the reconstructed gate graph"
    );
}

#[test]
#[should_panic(expected = "is absent from the Rust mission")]
fn recorded_drop_ale_gate_path_rejects_missing_door() {
    recorded_point_seek_gate_path(
        &drop_ale_exit_doors(),
        SectorHandle::new(133).unwrap(),
        11,
        SectorHandle::new(22).unwrap(),
        Some(recorded_route(
            SectorHandle::new(133).unwrap(),
            11,
            crate::gate::RecordedGateOutcome::Success(vec![crate::gate::GatePathStep {
                door_index: crate::gate::DoorIndex::new(99).expect("valid door index"),
                direct: false,
            }]),
        )),
    );
}

#[test]
#[should_panic(expected = "does not continue from sector")]
fn recorded_drop_ale_gate_path_rejects_wrong_direction() {
    recorded_point_seek_gate_path(
        &drop_ale_exit_doors(),
        SectorHandle::new(133).unwrap(),
        11,
        SectorHandle::new(22).unwrap(),
        Some(recorded_route(
            SectorHandle::new(133).unwrap(),
            11,
            crate::gate::RecordedGateOutcome::Success(vec![crate::gate::GatePathStep {
                door_index: crate::gate::DoorIndex::new(7).expect("valid door index"),
                direct: true,
            }]),
        )),
    );
}

#[test]
#[should_panic(expected = "not goal")]
fn recorded_drop_ale_gate_path_rejects_wrong_terminal_sector() {
    recorded_point_seek_gate_path(
        &drop_ale_exit_doors(),
        SectorHandle::new(133).unwrap(),
        11,
        SectorHandle::new(22).unwrap(),
        Some(recorded_route(
            SectorHandle::new(133).unwrap(),
            11,
            crate::gate::RecordedGateOutcome::Success(vec![crate::gate::GatePathStep {
                door_index: crate::gate::DoorIndex::new(6).expect("valid door index"),
                direct: false,
            }]),
        )),
    );
}

#[test]
#[should_panic(expected = "public source sector differs at dispatch")]
fn recorded_drop_ale_failure_rejects_wrong_dispatch_source_sector() {
    recorded_point_seek_gate_path(
        &drop_ale_exit_doors(),
        SectorHandle::new(133).unwrap(),
        11,
        SectorHandle::new(22).unwrap(),
        Some(recorded_route(
            SectorHandle::new(25).unwrap(),
            11,
            crate::gate::RecordedGateOutcome::Failure,
        )),
    );
}

#[test]
#[should_panic(expected = "source layer differs at dispatch")]
fn recorded_drop_ale_failure_rejects_wrong_dispatch_source_layer() {
    recorded_point_seek_gate_path(
        &drop_ale_exit_doors(),
        SectorHandle::new(133).unwrap(),
        11,
        SectorHandle::new(22).unwrap(),
        Some(recorded_route(
            SectorHandle::new(133).unwrap(),
            2,
            crate::gate::RecordedGateOutcome::Failure,
        )),
    );
}

#[test]
#[should_panic(expected = "exact source sector differs at dispatch")]
fn recorded_drop_ale_failure_rejects_same_public_different_exact_source() {
    let live_source = SectorHandle::new(133)
        .unwrap()
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(57).unwrap());
    let recorded_source = SectorHandle::new(133)
        .unwrap()
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(58).unwrap());
    recorded_point_seek_gate_path(
        &drop_ale_exit_doors(),
        live_source,
        11,
        SectorHandle::new(22).unwrap(),
        Some(recorded_route(
            recorded_source,
            11,
            crate::gate::RecordedGateOutcome::Failure,
        )),
    );
}

#[test]
#[should_panic(expected = "exact source sector differs at dispatch")]
fn recorded_drop_ale_failure_rejects_missing_exact_source_identity() {
    let live_source = SectorHandle::new(133)
        .unwrap()
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(57).unwrap());
    recorded_point_seek_gate_path(
        &drop_ale_exit_doors(),
        live_source,
        11,
        SectorHandle::new(22).unwrap(),
        Some(recorded_route(
            SectorHandle::new(133).unwrap(),
            11,
            crate::gate::RecordedGateOutcome::Failure,
        )),
    );
}

#[test]
#[should_panic(expected = "exact source sector differs at dispatch")]
fn recorded_drop_ale_failure_rejects_spurious_exact_source_identity() {
    let recorded_source = SectorHandle::new(133)
        .unwrap()
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(57).unwrap());
    recorded_point_seek_gate_path(
        &drop_ale_exit_doors(),
        SectorHandle::new(133).unwrap(),
        11,
        SectorHandle::new(22).unwrap(),
        Some(recorded_route(
            recorded_source,
            11,
            crate::gate::RecordedGateOutcome::Failure,
        )),
    );
}

#[test]
fn lost_target_moveok_stop_transition_publishes_waiting_before_terminal_handoff() {
    let mut engine = crate::engine::EngineInner::new();
    let mut owner_entity = test_pc_at(100.0, 100.0, 1);
    {
        let actor = owner_entity.actor_data_mut().unwrap();
        actor.action_state = ActionState::MovingFast;
        actor.seek_target = None;
        actor.continuation.seek_to_point = false;
    }
    owner_entity.element_data_mut().sprite.last_action = OrderType::WalkingStairs;
    let owner = engine.add_entity(owner_entity);

    let mut movement =
        SequenceElement::new_movement(1, Command::MoveOk, Some(owner), OrderType::RunningUpright);
    let SequenceElementData::Movement { flags, .. } = &mut movement.data else {
        panic!("MoveOk fixture must be a movement element")
    };
    flags.insert(MoveFlags::SEEK);
    let order_id = engine.orders.allocate_order_id();
    movement.orders.push_back(crate::order::Order::new(
        OrderType::TransitionRunningUprightWaitingUpright,
        100.0,
        100.0,
        order_id,
    ));
    let sequence_id = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);
    {
        let actor = engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.active_movement = ActiveMovement::new(sequence_id, 0);
        actor.installed_order = Some(crate::element::InstalledActorOrder {
            order_id,
            order_type: OrderType::TransitionRunningUprightWaitingUpright,
        });
        actor.continuation.motion_state = MotionState::InProgress;
    }

    let mut tail_order = None;
    engine.tick_actor_animation_action_change_slots_with_hooks(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        |_, _| {},
        |_, _| {},
        |engine, execute_owner, movement, _, _, _, _| {
            assert_eq!(execute_owner, owner);
            let movement = movement.expect("live MoveOk must own the actor Execute slot");
            assert!(perform_seek_lost_actor_target(
                engine,
                execute_owner,
                movement,
            ));
            Some(MotionState::Terminated)
        },
        |_, tail_owner, order_type| {
            assert_eq!(tail_owner, owner);
            tail_order = Some(order_type);
        },
    );
    let entity = engine.get_entity(owner).unwrap();
    assert_eq!(entity.element_data().posture(), Posture::Upright);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::Waiting
    );
    assert_eq!(
        entity.element_data().sprite.last_action,
        OrderType::WalkingStairs,
        "seeking without a target never enters sprite processing; its surrounding execution arm owns only state updates"
    );
    let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
    assert_eq!(actor.installed_order, None);
    assert_eq!(actor.continuation.motion_state, MotionState::Terminated);
    assert_eq!(
        tail_order,
        Some(OrderType::NonanimationEnd),
        "the exhausted live MoveOk publishes the null mpOrder tail as NONANIMATION_END"
    );
}

fn test_pc_at(x: f32, y: f32, sector: u16) -> Entity {
    let mut pc = ActorPc {
        element: {
            let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
            initial_element.kind = ElementKind::ActorPc;
            initial_element.active = true;
            initial_element
        },
        actor: ActorData::default(),
        human: HumanData::default(),
        pc: PcData::default(),
    };
    pc.element
        .set_position_map(crate::coordinates::MapPoint { x, y });
    pc.element.set_sector(SectorHandle::new(sector));
    Entity::Pc(pc)
}

fn test_moving_soldier_at(position: crate::coordinates::WorldPoint3D) -> Entity {
    let mut soldier = ActorSoldier {
        element: {
            let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
            initial_element.kind = ElementKind::ActorSoldier;
            initial_element.active = true;
            initial_element
        },
        actor: ActorData {
            action_state: ActionState::Moving,
            ..Default::default()
        },
        human: HumanData::default(),
        npc: Default::default(),
        soldier: SoldierData::default(),
    };
    soldier.element.set_position(position);
    soldier.npc.ai_brain = AiBrain::Enemy(Box::default());
    Entity::Soldier(soldier)
}

fn minimal_mission() -> crate::engine::MissionScript {
    use crate::scb::{ClassEntry, Function, SCB_VERSION, ScbFile};
    use crate::vm::{Opcode, Quad};

    crate::engine::MissionScript::from_scb(ScbFile {
        version: SCB_VERSION,
        classes: vec![ClassEntry {
            source_file: "refresh_seek_test.scs".into(),
            class_name: "StartUp".into(),
            size_of_member_variables: 0,
            member_variables: Vec::new(),
            functions: vec![Function {
                name: "Initialize".into(),
                address: 0,
                num_parameters: 0,
                size_of_return_value: 0,
                size_of_parameters: 0,
                size_of_volatile: 0,
                size_of_temporary: 0,
            }],
            quads: vec![
                Quad {
                    operation: Opcode::BeginFunction as u8,
                    operands: [0; 8],
                },
                Quad {
                    operation: Opcode::Return as u8,
                    operands: [0; 8],
                },
            ],
        }],
    })
    .expect("minimal mission script")
}

#[test]
fn entity_seek_gate_search_keeps_pc126_exact_route_identity() {
    use crate::fast_find_grid::SectorIndex;
    use crate::gate::{Door, DoorIndex, build_gate_links};
    use crate::sector::SectorNumber;

    let arena = |index| SectorIndex::new(index).unwrap();
    let sector = |public, index| {
        SectorHandle::new(public)
            .unwrap()
            .with_arena_index(arena(index))
    };
    let mut doors = (0..74)
        .map(|_| Door {
            active: false,
            ..Door::default()
        })
        .collect::<Vec<_>>();

    // Cyrdach Pc126's Original route at frame 1427 starts by crossing
    // gate 73 in reverse (62 -> 24), then gate 18 directly (24 -> 27).
    doors[73] = Door {
        active: true,
        point_in: MapPoint::new(0.0, 0.0),
        point_out: MapPoint::new(20.0, 0.0),
        sector_in: SectorNumber::new(62),
        sector_out: SectorNumber::new(24),
        sector_in_index: Some(arena(162)),
        sector_out_index: Some(arena(124)),
        ..Door::default()
    };
    doors[18] = Door {
        active: true,
        point_out: MapPoint::new(30.0, 0.0),
        point_in: MapPoint::new(50.0, 0.0),
        sector_out: SectorNumber::new(24),
        sector_in: SectorNumber::new(27),
        sector_out_index: Some(arena(124)),
        sector_in_index: Some(arena(127)),
        ..Door::default()
    };
    build_gate_links(&mut doors);

    let path = find_seek_gate_path(
        &doors,
        MapPoint::new(0.0, 0.0),
        sector(62, 162),
        MapPoint::new(50.0, 0.0),
        sector(27, 127),
        None,
        false,
        &|_| true,
        &|_| None,
    )
    .expect("exact Pc126 Seek topology must find the two-gate route");
    assert_eq!(
        path,
        vec![
            crate::gate::GatePathStep {
                door_index: DoorIndex::new(73).expect("valid door index"),
                direct: false,
            },
            crate::gate::GatePathStep {
                door_index: DoorIndex::new(18).expect("valid door index"),
                direct: true,
            },
        ]
    );

    assert!(
        find_seek_gate_path(
            &doors,
            MapPoint::new(0.0, 0.0),
            // Same public sector as gate 73's in-side, but a distinct
            // Original arena object. Numeric routing would falsely use
            // gate 73 here; exact routing must reject it.
            sector(62, 262),
            MapPoint::new(50.0, 0.0),
            sector(27, 127),
            None,
            false,
            &|_| true,
            &|_| None,
        )
        .is_none()
    );
}

#[test]
fn refresh_seek_recovers_moved_owner_and_target_sectors_before_indexed_route() {
    use crate::coordinates::MapBBox;
    use crate::fast_find_grid::{GridSector, SectorIndex};
    use crate::gate::{Door, DoorIndex, build_gate_links};
    use crate::sector::{SectorNumber, SectorType};

    let arena = |index| SectorIndex::new(index).unwrap();
    let grid_sector = |number, layer, min_x, min_y, max_x, max_y| GridSector {
        points: vec![
            MapPoint::new(min_x, min_y),
            MapPoint::new(max_x, min_y),
            MapPoint::new(max_x, max_y),
            MapPoint::new(min_x, max_y),
        ],
        bounding_box: MapBBox::from_coords(min_x, min_y, max_x, max_y),
        sector_type: SectorType::MOTION | SectorType::AREA,
        layer,
        sector_number: SectorNumber::new(number),
        door_index: None,
        lift_type: None,
        lift_direction: 0,
        force_crouched: false,
        building_index: None,
        low_exit_point: None,
        high_exit_point: None,
        lowest_door_index: None,
        jump_line_indices: Vec::new(),
        gate_indices: Vec::new(),
        underlying_sector: None,
    };

    let sim = crate::sim_rng::test_context();
    let mut engine = crate::engine::EngineInner::new();
    engine.scripts.mission = Some(minimal_mission());
    engine.world.fast_grid_mut().size_map(10, 10);
    engine.world.fast_grid_mut().allocate_layers(3);
    let wrong_target = engine
        .world
        .fast_grid_mut()
        .add_sector(grid_sector(88, 2, 450.0, 450.0, 500.0, 500.0), 2);
    let wrong_source = engine
        .world
        .fast_grid_mut()
        .add_sector(grid_sector(0, 0, 450.0, 150.0, 500.0, 200.0), 0);
    let source = engine
        .world
        .fast_grid_mut()
        .add_sector(grid_sector(0, 0, 10.0, 10.0, 100.0, 100.0), 0);
    let middle = engine
        .world
        .fast_grid_mut()
        .add_sector(grid_sector(70, 1, 150.0, 10.0, 250.0, 100.0), 1);
    let exact_target = engine
        .world
        .fast_grid_mut()
        .add_sector(grid_sector(88, 2, 300.0, 10.0, 400.0, 100.0), 2);
    assert_ne!(wrong_target, exact_target);
    assert_ne!(wrong_source, source);

    let mut doors = (0..114)
        .map(|_| Door {
            active: false,
            ..Door::default()
        })
        .collect::<Vec<_>>();
    doors[111] = Door {
        active: true,
        point_out: MapPoint::new(90.0, 50.0),
        point_in: MapPoint::new(160.0, 50.0),
        layer_out: 0,
        layer_in: 1,
        sector_out: SectorNumber::new(0),
        sector_in: SectorNumber::new(70),
        sector_out_index: Some(arena(source)),
        sector_in_index: Some(arena(middle)),
        ..Door::default()
    };
    doors[113] = Door {
        active: true,
        point_in: MapPoint::new(240.0, 50.0),
        point_out: MapPoint::new(310.0, 50.0),
        layer_in: 1,
        layer_out: 2,
        sector_in: SectorNumber::new(70),
        sector_out: SectorNumber::new(88),
        sector_in_index: Some(arena(middle)),
        sector_out_index: Some(arena(exact_target)),
        ..Door::default()
    };
    build_gate_links(&mut doors);
    engine.script_domains.interactables.doors = doors;

    let owner = engine.add_entity(test_pc_at(50.0, 50.0, 0));
    {
        let owner_position = engine.get_entity_mut(owner).unwrap().position_iface_mut();
        // The adopted PC carries only the public sector. Original's
        // Sector lookup still supplies the exact sector reference to
        // movement-sequence construction, so recover the containing duplicate here.
        owner_position.set_sector(SectorHandle::new(0));
        owner_position.set_move_box(crate::coordinates::MoveBox::from_coords(
            -4.0, -4.0, 4.0, 4.0,
        ));
    }
    let target = engine.add_entity(test_pc_at(350.0, 50.0, 88));
    {
        let target_element = engine.get_entity_mut(target).unwrap().element_data_mut();
        target_element.set_layer(2);
        target_element.set_sector(SectorHandle::new(88));
    }

    let mut seek =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::RunningUpright);
    if let SequenceElementData::Movement {
        element,
        flags,
        tolerance,
        ..
    } = &mut seek.data
    {
        *element = Some(target);
        *flags = MoveFlags::SEEK;
        *tolerance = 0.0;
    }
    let seek_id = engine.orders.sequence_manager.launch_element(seek);
    engine
        .orders
        .sequence_manager
        .element_in_progress(seek_id, 0);

    assert!(engine.try_dispatch_cross_sector_entity_seek(
        &sim,
        &LevelAssets::new(),
        owner,
        seek_id,
        0,
        target,
        OrderType::RunningUpright,
        MoveFlags::SEEK,
        0.0,
    ));
    let gates = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .filter_map(|element| match element.data {
            SequenceElementData::Movement {
                gate_id: Some(gate),
                ..
            } if element.owner == Some(owner) => Some(gate),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        gates,
        vec![
            DoorIndex::new(111).expect("valid door index"),
            DoorIndex::new(113).expect("valid door index")
        ]
    );
}

#[test]
fn cross_sector_refresh_seek_does_not_append_pc_posture_recovery() {
    use crate::gate::Door;
    use crate::sector::SectorNumber;

    let sim = crate::sim_rng::test_context();
    let mut engine = crate::engine::EngineInner::new();
    engine.scripts.mission = Some(minimal_mission());
    engine.script_domains.interactables.doors = vec![Door {
        point_out: MapPoint::new(20.0, 0.0),
        point_in: MapPoint::new(30.0, 0.0),
        sector_out: SectorNumber::new(1),
        sector_in: SectorNumber::new(2),
        ..Door::default()
    }];

    let mut owner_entity = test_pc_at(0.0, 0.0, 1);
    owner_entity
        .element_data_mut()
        .publish_order_posture(Posture::HelpingToClimb);
    owner_entity
        .position_iface_mut()
        .set_move_box(crate::coordinates::MoveBox::from_coords(
            -4.0, -4.0, 4.0, 4.0,
        ));
    let owner = engine.add_entity(owner_entity);
    let mut target_entity = test_pc_at(50.0, 0.0, 2);
    target_entity
        .position_iface_mut()
        .set_move_box(crate::coordinates::MoveBox::from_coords(
            46.0, -4.0, 54.0, 4.0,
        ));
    let target = engine.add_entity(target_entity);

    let mut seek =
        SequenceElement::new_movement(1, Command::Seek, Some(owner), OrderType::WalkingUpright);
    if let SequenceElementData::Movement {
        element,
        flags,
        tolerance,
        ..
    } = &mut seek.data
    {
        *element = Some(target);
        *flags = MoveFlags::SEEK;
        *tolerance = 10.0;
    }
    let seek_id = engine.orders.sequence_manager.launch_element(seek);
    engine
        .orders
        .sequence_manager
        .element_in_progress(seek_id, 0);

    assert!(engine.try_dispatch_cross_sector_entity_seek(
        &sim,
        &LevelAssets::new(),
        owner,
        seek_id,
        0,
        target,
        OrderType::WalkingUpright,
        MoveFlags::SEEK,
        10.0,
    ));

    let replacement = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .find(|sequence| {
            sequence
                .elements
                .iter()
                .any(|element| element.owner == Some(owner) && element.command == Command::PassDoor)
        })
        .expect("cross-sector seek refresh must build a gate route");
    assert!(
        replacement
            .elements
            .iter()
            .all(|element| element.command != Command::EnterHelpingClimb),
        "Original seek refresh constructs the movement sequence directly and does not append PC posture recovery"
    );
}

#[test]
fn ordinary_cross_sector_pc_move_still_appends_posture_recovery() {
    use crate::gate::{Door, DoorIndex, GatePathStep};
    use crate::sector::SectorNumber;

    let sim = crate::sim_rng::test_context();
    let mut engine = crate::engine::EngineInner::new();
    engine.scripts.mission = Some(minimal_mission());
    engine.script_domains.interactables.doors = vec![Door {
        point_out: MapPoint::new(20.0, 0.0),
        point_in: MapPoint::new(30.0, 0.0),
        sector_out: SectorNumber::new(1),
        sector_in: SectorNumber::new(2),
        ..Door::default()
    }];
    let mut owner_entity = test_pc_at(0.0, 0.0, 1);
    owner_entity
        .element_data_mut()
        .publish_order_posture(Posture::HelpingToClimb);
    let owner = engine.add_entity(owner_entity);

    let sequence_id = engine
        .build_gate_movement_sequence(
            &sim,
            owner,
            crate::position_interface::SectorHandle::new(1),
            vec![GatePathStep {
                door_index: DoorIndex::new(0).expect("valid door index"),
                direct: true,
            }],
            GoalShape::Point {
                point: MapPoint::new(50.0, 0.0),
                tolerance: 0.0,
            },
            0,
            OrderType::WalkingUpright,
            true,
            1.0,
            MoveFlags::empty(),
            Vec::new(),
            Vec::new(),
            true,
            true,
        )
        .expect("ordinary cross-sector move route");
    let route = engine
        .orders
        .sequence_manager
        .get_sequence(sequence_id)
        .expect("ordinary route remains queued");
    assert!(
        route
            .elements
            .iter()
            .any(|element| element.command == Command::EnterHelpingClimb)
    );
}

fn resolve_stop_npc_seek_with_target_at(
    target_position: crate::coordinates::WorldPoint3D,
) -> (crate::ai::AiState, crate::ai::Substate) {
    let sim = crate::sim_rng::test_context();
    let mut engine = crate::engine::EngineInner::new();
    let mut assets = LevelAssets::new();
    let mut owner_entity = test_pc_at(0.0, 0.0, 1);
    owner_entity
        .element_data_mut()
        .set_position(crate::coordinates::WorldPoint3D::ZERO);
    owner_entity
        .position_iface_mut()
        .set_move_box(crate::coordinates::MoveBox::from_coords(
            -6.0, -4.0, 6.0, 4.0,
        ));
    owner_entity.actor_data_mut().unwrap().action_state = ActionState::Moving;
    let owner = engine.add_entity(owner_entity);
    let target = engine.add_entity(test_moving_soldier_at(target_position));
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);

    let _ = engine.resolve_entity_seek(
        &sim,
        &assets,
        owner,
        target,
        MoveFlags::SEEK | MoveFlags::SEEK_STOP_NPC,
        32.0,
    );

    let ai = engine
        .get_entity(target)
        .unwrap()
        .npc_data()
        .unwrap()
        .ai_brain
        .base()
        .unwrap();
    (ai.current_state, ai.current_substate)
}

#[test]
fn seek_stop_npc_uses_raw_3d_distance_across_elevations() {
    // Map projection is (20, 16), inside the 64-unit moving-target
    // stop radius, while the literal 3D distance is about 224.6.
    assert_eq!(
        resolve_stop_npc_seek_with_target_at(crate::coordinates::WorldPoint3D::new(
            20.0, 166.0, 150.0,
        )),
        (
            crate::ai::AiState::Default,
            crate::ai::Substate::DefaultOnPost,
        ),
        "a map-near actor on another elevation must not receive EventStop"
    );
}

#[test]
fn seek_stop_npc_still_stops_raw_3d_near_target() {
    assert_eq!(
        resolve_stop_npc_seek_with_target_at(crate::coordinates::WorldPoint3D::new(
            20.0, 16.0, 0.0,
        )),
        (
            crate::ai::AiState::Seeking,
            crate::ai::Substate::SeekingGotStopEvent,
        ),
        "a raw-3D-near moving NPC must receive EventStop"
    );
}

#[test]
fn refresh_seek_waits_when_same_sector_actor_target_is_passing_door() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = crate::engine::EngineInner::new();
    let assets = LevelAssets::new();
    let owner = engine.add_entity(test_pc_at(10.0, 10.0, 1));
    let target = engine.add_entity(test_pc_at(80.0, 10.0, 1));

    let mut seek =
        SequenceElement::new_movement(1, Command::Seek, Some(owner), OrderType::WalkingUpright);
    if let SequenceElementData::Movement {
        flags,
        element,
        tolerance,
        ..
    } = &mut seek.data
    {
        *flags = MoveFlags::SEEK;
        *element = Some(target);
        *tolerance = 10.0;
    }
    let seek_seq = engine.orders.sequence_manager.launch_element(seek);
    engine
        .orders
        .sequence_manager
        .element_in_progress(seek_seq, 0);
    {
        // Seek translation stamps the owner's base seek distance before
        // any refresh can run; seek refresh requires that live value.
        let actor = engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.active_movement = ActiveMovement::new(seek_seq, 0);
        actor.seek_target = Some(target);
        actor.seek_distance = 10.0;
    }

    let pass = SequenceElement::new_movement(
        1,
        Command::PassDoor,
        Some(target),
        OrderType::WalkingUpright,
    );
    let pass_seq = engine.orders.sequence_manager.launch_element(pass);
    engine
        .orders
        .sequence_manager
        .element_in_progress(pass_seq, 0);
    engine
        .get_entity_mut(target)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_movement = ActiveMovement::new(pass_seq, 0);

    engine.apply_seek_refresh(
        sim,
        &assets,
        owner,
        seek_seq,
        0,
        target,
        OrderType::WalkingUpright,
        MoveFlags::SEEK,
        crate::coordinates::MapPoint { x: 90.0, y: 10.0 },
    );

    assert_eq!(engine.orders.sequence_manager.sequence_count(), 2);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seek_seq, 0)
            .unwrap()
            .state,
        SequenceState::InProgress
    );
}

fn assert_moved_target_refresh_returns_explicit_in_progress(
    stale_sprite_motion: MotionState,
    target_sector: u16,
    expected_entry_state: SequenceState,
    element_tolerance: f32,
) {
    let sim = crate::sim_rng::test_context();
    let mut engine = crate::engine::EngineInner::new();
    crate::engine::test_support::ensure_ordinary_sector(&mut engine, 1, 0);
    crate::engine::test_support::ensure_ordinary_sector(&mut engine, target_sector, 0);
    let mut assets = LevelAssets::new();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .push(crate::profiles::CharacterProfile::default());
    let owner = engine.add_entity(test_pc_at(10.0, 10.0, 1));
    let target = engine.add_entity(test_pc_at(80.0, 10.0, target_sector));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .position_iface_mut()
        .set_move_box(crate::coordinates::MoveBox::from_coords(
            -6.0, -4.0, 6.0, 4.0,
        ));

    let mut seek =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
    seek.orders.push_back(crate::order::Order::test_new(
        OrderType::WalkingUpright,
        0.0,
        0.0,
    ));
    if let SequenceElementData::Movement {
        flags,
        element,
        tolerance,
        ..
    } = &mut seek.data
    {
        *flags = MoveFlags::SEEK;
        *element = Some(target);
        *tolerance = element_tolerance;
    }
    let seek_seq = engine.orders.sequence_manager.launch_element(seek);
    engine
        .orders
        .sequence_manager
        .element_in_progress(seek_seq, 0);
    {
        let owner_entity = engine.get_entity_mut(owner).unwrap();
        owner_entity.element_data_mut().sprite.last_motion_state = Some(stale_sprite_motion);
        let actor = owner_entity.actor_data_mut().unwrap();
        actor.active_movement = ActiveMovement::new(seek_seq, 0);
        actor.seek_target = Some(target);
        actor.seek_distance = 10.0;
        actor.seek_refresh_wait = 0;
        actor.last_seek_target_position = MapPoint::new(60.0, 10.0);
    }

    let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    engine.tick_actor_owner_envelopes(&sim, &assets, &positions);

    let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
    assert_eq!(actor.continuation.motion_state, MotionState::InProgress);
    if expected_entry_state == SequenceState::Impossible {
        assert_eq!(
            actor.installed_order, None,
            "failed seek refresh returns before installing replacement work"
        );
    }
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .element_data()
            .sprite
            .last_motion_state,
        Some(stale_sprite_motion),
        "seek refresh returns before sprite motion processing"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seek_seq, 0)
            .unwrap()
            .state,
        expected_entry_state
    );
    if expected_entry_state == SequenceState::Interrupted {
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .current_element_for_actor(owner),
            None,
            "seek refresh registers its replacement for the later manager phase"
        );
        let replacement = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .find(|element| {
                element.owner == Some(owner)
                    && element.command == Command::Move
                    && element.state == SequenceState::Todo
            })
            .expect("seek refresh must leave a fresh movement queued for the simulation tick");
        let SequenceElementData::Movement {
            flags,
            element,
            destination,
            tolerance,
            ..
        } = &replacement.data
        else {
            panic!("seek-refresh replacement changed element kind")
        };
        assert!(flags.contains(MoveFlags::SEEK));
        assert_eq!(*element, Some(target));
        assert_eq!(*destination, MapPoint::new(80.0, 10.0));
        assert_eq!(
            *tolerance, 10.0,
            "refresh uses the actor's live seek distance"
        );
    }
}

#[test]
fn moved_target_refresh_returns_in_progress_over_stale_terminated_sprite_motion() {
    assert_moved_target_refresh_returns_explicit_in_progress(
        MotionState::Terminated,
        1,
        SequenceState::Interrupted,
        10.0,
    );
}

#[test]
fn moved_target_refresh_returns_in_progress_over_stale_done_sprite_motion() {
    assert_moved_target_refresh_returns_explicit_in_progress(
        MotionState::Done,
        1,
        SequenceState::Interrupted,
        10.0,
    );
}

#[test]
fn failed_moved_target_refresh_keeps_explicit_in_progress_motion() {
    assert_moved_target_refresh_returns_explicit_in_progress(
        MotionState::Aborted,
        2,
        SequenceState::Impossible,
        10.0,
    );
}

#[test]
fn moved_target_refresh_ignores_stale_element_tolerance() {
    // 999 would admit the target as already in range if substituted for
    // the actor's live distance (10), and must not reach the replacement.
    assert_moved_target_refresh_returns_explicit_in_progress(
        MotionState::Done,
        1,
        crate::sequence::SequenceState::Interrupted,
        999.0,
    );
}

#[test]
fn climbing_seek_flag_does_not_run_perform_seek_refresh() {
    let sim = crate::sim_rng::test_context();
    let mut engine = crate::engine::EngineInner::new();
    let assets = LevelAssets::new();
    let owner = engine.add_entity(test_pc_at(10.0, 10.0, 1));
    let target = engine.add_entity(test_pc_at(80.0, 10.0, 2));

    let mut seek =
        SequenceElement::new_movement(1, Command::MoveOk, Some(owner), OrderType::ClimbingWallUp);
    if let SequenceElementData::Movement { flags, element, .. } = &mut seek.data {
        *flags = MoveFlags::SEEK;
        *element = Some(target);
    }
    seek.orders.push_back(crate::order::Order::test_new(
        OrderType::ClimbingWallUp,
        80.0,
        10.0,
    ));
    let seek_seq = engine.orders.sequence_manager.launch_element(seek);
    engine
        .orders
        .sequence_manager
        .element_in_progress(seek_seq, 0);
    {
        let actor = engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.active_movement = ActiveMovement::new(seek_seq, 0);
        actor.seek_refresh_wait = 0;
        actor.last_seek_target_position = MapPoint::ZERO;
    }

    assert!(!engine.tick_refresh_seek_for_owner(&sim, &assets, owner));
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seek_seq, 0)
            .unwrap()
            .state,
        SequenceState::InProgress
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .seek_refresh_wait,
        0
    );
}

#[test]
fn moved_target_refresh_uses_actor_owned_seek_target_over_element_target() {
    let mut engine = crate::engine::EngineInner::new();
    let owner = engine.add_entity(test_pc_at(10.0, 10.0, 1));
    let actor_target = engine.add_entity(test_pc_at(40.0, 10.0, 1));
    let competing_element_target = engine.add_entity(test_pc_at(100.0, 10.0, 1));

    let mut seek =
        SequenceElement::new_movement(1, Command::MoveOk, Some(owner), OrderType::WalkingUpright);
    if let SequenceElementData::Movement { flags, element, .. } = &mut seek.data {
        *flags = MoveFlags::SEEK;
        *element = Some(competing_element_target);
    }
    seek.orders.push_back(crate::order::Order::test_new(
        OrderType::WalkingUpright,
        80.0,
        10.0,
    ));
    let seek_seq = engine.orders.sequence_manager.launch_element(seek);
    engine
        .orders
        .sequence_manager
        .element_in_progress(seek_seq, 0);
    {
        let actor = engine
            .get_entity_mut(owner)
            .expect("owner")
            .actor_data_mut()
            .expect("owner is an actor");
        actor.active_movement = ActiveMovement::new(seek_seq, 0);
        actor.seek_target = Some(actor_target);
        actor.seek_distance = 10.0;
        actor.seek_refresh_wait = 0;
        actor.last_seek_target_position = MapPoint::new(40.0, 10.0);
    }

    assert!(
        engine.selected_seek_refresh_decision(owner).is_none(),
        "the element's competing target must not spuriously refresh seeking"
    );

    engine
        .get_entity_mut(actor_target)
        .expect("actor-owned seek target")
        .position_iface_mut()
        .set_map_position(MapPoint::new(60.0, 10.0));
    let decision = engine
        .selected_seek_refresh_decision(owner)
        .expect("the actor-owned target moved more than 10 units");
    assert_eq!(decision.2, actor_target);
    assert_eq!(decision.5, MapPoint::new(60.0, 10.0));
}

/// Human-actor execution faces the opponent
/// before it enters seeking, and the seek operation's
/// moved-target branch returns in-progress motion without reaching
/// motion processing. Opponent-facing's
/// Aspect-corrected direction selection followed by turning
/// therefore still run on the
/// seek-refresh frame.
#[test]
fn sword_walk_seek_refresh_still_faces_the_opponent() {
    let mut engine = crate::engine::EngineInner::new();

    let mut owner_entity = test_pc_at(0.0, 0.0, 1);
    owner_entity
        .actor_data_mut()
        .expect("test PC is an actor")
        .action_state = ActionState::MovingSword;
    owner_entity.element_data_mut().set_direction_instantly(0);
    let owner = engine.add_entity(owner_entity);
    let target = engine.add_entity(test_pc_at(100.0, 0.0, 1));
    engine
        .get_entity_mut(owner)
        .expect("owner")
        .human_data_mut()
        .expect("PC has human payload")
        .opponents
        .push(target);

    let mut seek =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingWithSword);
    if let SequenceElementData::Movement { flags, element, .. } = &mut seek.data {
        *flags = MoveFlags::SEEK;
        *element = Some(target);
    }
    seek.orders.push_back(crate::order::Order::test_new(
        OrderType::WalkingWithSword,
        90.0,
        0.0,
    ));
    let seek_seq = engine.orders.sequence_manager.launch_element(seek);
    engine
        .orders
        .sequence_manager
        .element_in_progress(seek_seq, 0);
    {
        let actor = engine
            .get_entity_mut(owner)
            .expect("owner")
            .actor_data_mut()
            .expect("owner is an actor");
        actor.active_movement = ActiveMovement::new(seek_seq, 0);
        actor.seek_target = Some(target);
        actor.seek_refresh_wait = 0;
        actor.seek_distance = 10.0;
        actor.last_seek_target_position = MapPoint::ZERO;
    }

    assert!(
        engine.selected_seek_refresh_decision(owner).is_some(),
        "the >10u moved target must select seeking's refresh branch"
    );

    engine.apply_pre_perform_seek_facing_prologue(owner);

    let owner_entity = engine.get_entity(owner).expect("owner");
    assert_eq!(
        owner_entity.position_iface().get_direction_goal().as_u8(),
        4,
        "opponent-facing aims the goal at the principal opponent"
    );
    assert_eq!(
        owner_entity.position_iface().get_direction().as_u8(),
        1,
        "opponent-facing advances one sector on the seek-refresh frame"
    );
}

#[test]
fn relaunch_seek_replacement_clears_selected_seek_goal_before_queuing_replacement() {
    let mut engine = crate::engine::EngineInner::new();
    let owner = engine.add_entity(test_pc_at(10.0, 10.0, 1));
    let stale_goal = MapPoint::new(70.0, 80.0);

    let seek =
        SequenceElement::new_movement(1, Command::Seek, Some(owner), OrderType::WalkingUpright);
    let seek_seq = engine.orders.sequence_manager.launch_element(seek);
    engine
        .orders
        .sequence_manager
        .element_in_progress(seek_seq, 0);
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        entity.actor_data_mut().unwrap().active_movement = ActiveMovement::new(seek_seq, 0);
        entity.position_iface_mut().set_map_goal(stale_goal);
    }

    let replacement =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
    engine.relaunch_seek_replacement(owner, seek_seq, 0, replacement);

    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal(),
        MapPoint::ZERO,
        "Original's synchronous selected-Seek condolence clears the old sprite goal"
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_movement,
        ActiveMovement::none()
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seek_seq, 0)
            .unwrap()
            .state,
        SequenceState::Interrupted
    );

    let replacement_seq = SequenceId(seek_seq.0 + 1);
    let replacement = engine
        .orders
        .sequence_manager
        .get_element(replacement_seq, 0)
        .expect("replacement Seek should remain queued for dispatch");
    assert_eq!(replacement.command, Command::Move);
    assert_eq!(replacement.state, SequenceState::Todo);
    assert!(
        engine
            .orders
            .sequence_manager
            .is_registered_to_go(replacement_seq, 0)
    );
}
