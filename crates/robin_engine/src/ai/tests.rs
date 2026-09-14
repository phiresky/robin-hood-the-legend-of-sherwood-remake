use super::*;

#[test]
fn substate_groups() {
    assert!(Substate::SeekingSeekpoint.is_seek_area());
    assert!(!Substate::DefaultOnPost.is_seek_area());
    assert!(Substate::AttackingSwordfight.is_any_swordfight());
    assert!(Substate::AttackingSwordfight.is_real_swordfight());
    assert!(!Substate::AttackingBowShooting.is_any_swordfight());
}

#[test]
fn ai_timer_clamps_zero_while_macro_timer_preserves_raw_ulong_deadlines() {
    let mut ai = AiController::new(17);
    ai.current_substate = Substate::DefaultInMacro;

    ai.launch_timer(0, 123);
    assert_eq!(ai.when_does_timer_ring, 124);
    assert_eq!(ai.substate_at_last_timer_launch, Substate::DefaultInMacro);

    ai.launch_timer(1, 456);
    assert_eq!(ai.when_does_timer_ring, 457);

    ai.launch_timer(20, 456);
    assert_eq!(ai.when_does_timer_ring, 476);

    ai.launch_macro_timer(0, 456);
    assert_eq!(ai.when_does_macro_timer_ring, 456);

    ai.launch_timer(5, u32::MAX - 2);
    ai.launch_macro_timer(7, u32::MAX - 3);
    assert_eq!(ai.when_does_timer_ring, 2);
    assert_eq!(ai.when_does_macro_timer_ring, 3);
}

#[test]
fn break_macro_preserves_serialized_cursor_and_remaining_bytes() {
    let mut ai = AiController::new(17);
    ai.macro_command = vec![10, 20, 30, 40, 50];
    ai.macro_command_offset = 3;
    ai.number_of_remaining_macro_bytes = 2;
    ai.macro_in_progress = true;
    ai.macro_timer_is_running = true;

    ai.break_macro();

    assert!(!ai.macro_in_progress);
    assert!(!ai.macro_timer_is_running);
    assert_eq!(ai.macro_command, [10, 20, 30, 40, 50]);
    assert_eq!(ai.macro_command_offset, 3);
    assert_eq!(ai.number_of_remaining_macro_bytes, 2);
}

#[test]
fn invalid_patrol_assignment_preserves_original_partial_mutation() {
    use crate::ai::{PathId, PatrolAssignment};

    let mut ai = AiController::new(17);
    ai.has_patrol_path = false;
    ai.macro_in_progress = true;
    ai.macro_timer_is_running = true;

    let assigned = ai.assign_new_patrol_path(
        PatrolAssignment::Index(PathId::new(3).unwrap()),
        Position::default(),
        0,
        &[crate::level_data::RawHikingPath {
            waypoints: Vec::new(),
        }],
    );

    assert!(!assigned);
    assert!(
        ai.has_patrol_path,
        "Original sets mbHasPatrolPath before rejecting the index"
    );
    assert!(ai.path_id.is_none());
    assert!(!ai.macro_in_progress);
    assert!(!ai.macro_timer_is_running);
}

#[test]
fn detached_patrol_status_preserves_original_cursor_history_across_reassignment() {
    use crate::ai::PatrolAssignment;
    use crate::ai::macro_patrol::{PathHistoryEntry, PathId, PatrolPath};
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    // The original game writes current, last, direction, path
    // and history in the original game, while initialization resets only
    // current/direction. Patrol-path assignment uses that initialization path in
    // the original-game AI state.
    let paths = vec![RawHikingPath {
        waypoints: vec![
            RawWaypoint {
                x: 10,
                y: 20,
                sector: 1,
                level: 0,
                command: WaypointCommand::None,
            },
            RawWaypoint {
                x: 30,
                y: 40,
                sector: 2,
                level: 0,
                command: WaypointCommand::None,
            },
        ],
    }];
    let path_id = PathId::new(0).unwrap();
    let history = vec![PathHistoryEntry {
        position: Position {
            x: 7.0,
            y: 9.0,
            sector: SectorHandle::new(1),
            level: 0,
        },
        direction: 3,
        distance: 11,
    }];
    let mut path = PatrolPath::new(path_id, &paths).unwrap();
    path.current_waypoint_index = 1;
    path.last_waypoint_index = 9;
    path.forward = false;
    path.history = history.clone();

    let mut ai = AiController::new(17);
    ai.has_patrol_path = true;
    ai.patrol_path = Some(path);
    ai.detach_patrol_path(None, false);
    assert!(ai.patrol_path.is_none());
    assert_eq!(ai.detached_patrol_path_status.current_waypoint_index, 1);
    assert_eq!(ai.detached_patrol_path_status.last_waypoint_index, 9);
    assert!(!ai.detached_patrol_path_status.forward);
    assert_eq!(ai.detached_patrol_path_status.history.len(), 1);
    assert_eq!(ai.detached_patrol_path_status.history[0].direction, 3);
    assert_eq!(ai.detached_patrol_path_status.history[0].distance, 11);

    assert!(ai.assign_new_patrol_path(
        PatrolAssignment::Index(path_id),
        Position::default(),
        0,
        &paths,
    ));
    let restored = ai.patrol_path.as_ref().unwrap();
    assert_eq!(restored.current_waypoint_index, 0);
    assert_eq!(restored.last_waypoint_index, 9);
    assert!(restored.forward);
    assert_eq!(restored.history.len(), 1);
    assert_eq!(restored.history[0].direction, 3);
    assert_eq!(restored.history[0].distance, 11);
}

#[test]
fn script_way_assignment_keeps_special_action() {
    // Script-level path assignment routes through the original game's
    // hiking patrol-path assignment overload. Its valid-path branch
    // clears only the likes-to-sit flag; special-action state survives, so a
    // leisure-authored NPC on a scripted route later fails movement's
    // already-on-point shortcut and runs a real zero-length move when
    // returning to its route (contrasting with the
    // index-based path-assignment variant).
    use crate::ai::{PathId, PatrolAssignment};

    let paths = vec![crate::level_data::RawHikingPath {
        waypoints: vec![crate::level_data::RawWaypoint {
            x: 10,
            y: 20,
            sector: 1,
            level: 0,
            command: crate::level_data::WaypointCommand::None,
        }],
    }];

    let mut ai = AiController::new(17);
    ai.special_action = true;
    ai.likes_to_sit_around = true;
    let assigned = ai.assign_new_patrol_path(
        PatrolAssignment::ScriptWay(PathId::new(0).unwrap()),
        Position::default(),
        0,
        &paths,
    );
    assert!(assigned);
    assert!(ai.has_patrol_path);
    assert!(
        ai.special_action,
        "the hiking-path variant must not clear the special-action flag"
    );
    assert!(!ai.likes_to_sit_around);

    // The waypoint-macro index overload clears both flags.
    let mut ai = AiController::new(17);
    ai.special_action = true;
    ai.likes_to_sit_around = true;
    let assigned = ai.assign_new_patrol_path(
        PatrolAssignment::Index(PathId::new(0).unwrap()),
        Position::default(),
        0,
        &paths,
    );
    assert!(assigned);
    assert!(
        !ai.special_action,
        "the 16-bit index variant clears the special-action flag"
    );
    assert!(!ai.likes_to_sit_around);
}

#[test]
fn ai_log_stimulus_strings_match_original_names_and_fallback() {
    assert_eq!(
        StimulusType::log_string_from_u16(StimulusType::EventView as u16),
        "EVENT-VIEW"
    );
    assert_eq!(
        StimulusType::log_string_from_u16(StimulusType::EventSeesFriendInTrouble as u16),
        "EVENT-SEESFRIENDINTROUBLE"
    );
    assert_eq!(
        StimulusType::log_string_from_u16(StimulusType::NoEvent as u16),
        "EVENT-???"
    );
    assert_eq!(StimulusType::log_string_from_u16(u16::MAX), "EVENT-???");
}

#[test]
fn ai_log_substate_strings_match_original_names_and_fallback() {
    assert_eq!(
        Substate::log_string_from_u16(Substate::DefaultGotoPost as u16),
        "SUBSTATE-DEFAULT-GOTOPOST"
    );
    assert_eq!(
        Substate::log_string_from_u16(Substate::AttackingSwordfight as u16),
        "SUBSTATE-ATTACKING-SWORDFIGHT"
    );
    assert_eq!(
        Substate::log_string_from_u16(Substate::AttackingArcherWaitOnArcheryPath as u16),
        "SUBSTATE-ATTACKING-ARCHER-WAIT-ON-ACHERY-PATH"
    );
    assert_eq!(
        Substate::log_string_from_u16(Substate::DefaultGotoChief as u16),
        "SUBSTATE-DEFAULT-GOTOCHIEF"
    );
    assert_eq!(
        Substate::log_string_from_u16(Substate::AttackingRunToAvengerOnRoof as u16),
        "SUBSTATE-???"
    );
    assert_eq!(Substate::log_string_from_u16(u16::MAX), "SUBSTATE-???");
}

#[test]
fn ai_log_decision_strings_match_original_names_and_fallback() {
    assert_eq!(
        Decision::log_string_from_u16(Decision::Fight as u16),
        "DECISION-FIGHT"
    );
    assert_eq!(
        Decision::log_string_from_u16(Decision::LookForHelp as u16),
        "DECISION-LOOK-4-HELP"
    );
    assert_eq!(
        Decision::log_string_from_u16(Decision::PredecisionOffensive as u16),
        "DECISION-???"
    );
    assert_eq!(Decision::log_string_from_u16(u16::MAX), "DECISION-???");
}

#[test]
fn ai_log_remark_strings_match_original_speech_and_fallback() {
    assert_eq!(
        Remark::log_string_from_u16(Remark::SeesBody as u16),
        "Ca va?"
    );
    assert_eq!(
        Remark::log_string_from_u16(Remark::TheSoundOfSilence as u16),
        " ........... "
    );
    assert_eq!(Remark::log_string_from_u16(u16::MAX), " ........... ");
}

#[test]
fn stimulus_similarity() {
    let a = Stimulus::new(StimulusType::EventTimer);
    let b = Stimulus::new(StimulusType::EventTimer);
    assert!(a.is_similar(&b));

    let c = Stimulus::new(StimulusType::EventDone);
    assert!(!a.is_similar(&c));

    let d = Stimulus::with_human(StimulusType::EventView, 42);
    let e = Stimulus::with_human(StimulusType::EventView, 42);
    assert!(d.is_similar(&e));

    let f = Stimulus::with_human(StimulusType::EventView, 99);
    assert!(!d.is_similar(&f));
}

#[test]
fn consideration_accumulator() {
    let mut acc = ConsiderationAccumulator::default();
    acc.consider_value(true, 80, 1, 0);
    acc.consider_value(true, 60, 1, 0);
    let result = acc.evaluate();
    assert_eq!(result, 70);
}

#[test]
fn value_between() {
    // The Windows x87 path retains the slightly-low binary32 value of
    // 0.01 through the complete expression before truncating.
    assert_eq!(AiController::value_between(0, 100, 50), 49);
    assert_eq!(AiController::value_between(0, 100, 0), 0);
    assert_eq!(AiController::value_between(0, 100, 99), 98);
    assert_eq!(AiController::value_between(0, 100, 100), 99);
    assert_eq!(AiController::value_between(10, 90, 50), 49);
    assert_eq!(AiController::value_between(10, 90, 100), 89);
    assert_eq!(AiController::value_between(90, 10, 50), 50);
}

#[test]
fn ai_controller_defaults() {
    let ai = AiController::new(1);
    assert_eq!(ai.me, 1);
    assert_eq!(ai.current_state, AiState::Default);
    assert_eq!(ai.current_substate, Substate::DefaultOnPost);
    assert_eq!(ai.attitude, Attitude::Suspicious);
    assert!(!ai.ai_is_locked());
}

#[test]
fn goto_route_arrival_launches_turn_even_when_already_facing_route() {
    use crate::ai::macro_patrol::{PathId, PatrolPath};
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    let paths = vec![RawHikingPath {
        waypoints: vec![
            RawWaypoint {
                x: 0,
                y: 0,
                sector: 1,
                level: 0,
                command: WaypointCommand::None,
            },
            RawWaypoint {
                x: 10,
                y: 0,
                sector: 1,
                level: 0,
                command: WaypointCommand::Macro(vec![1]),
            },
        ],
    }];
    let mut path = PatrolPath::new(PathId::new(0).unwrap(), &paths).unwrap();
    path.advance();

    let mut ai = AiController::new(1);
    ai.current_state = AiState::Default;
    ai.current_substate = Substate::DefaultGotoRoute;
    ai.patrol_path = Some(path);

    let route_direction = crate::position_interface::vector_to_sector_0_to_15(10.0, 0.0) as u16;
    let position = Position {
        x: 10.0,
        y: 0.0,
        sector: SectorHandle::new(1),
        level: 0,
    };

    let direction = ai.route_arrival_turn_direction(position, &paths);
    assert_eq!(
        direction,
        Some(route_direction),
        "arrival retains an explicit turn even when already facing its direction"
    );
}

#[test]
fn goto_route_turn_lookup_preserves_original_endpoint_direction_flip() {
    use crate::ai::macro_patrol::{PathId, PatrolPath};
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    let paths = vec![RawHikingPath {
        waypoints: vec![
            RawWaypoint {
                x: 0,
                y: 0,
                sector: 1,
                level: 0,
                command: WaypointCommand::Macro(vec![1]),
            },
            RawWaypoint {
                x: 10,
                y: 0,
                sector: 1,
                level: 0,
                command: WaypointCommand::None,
            },
        ],
    }];

    let mut ai = AiController::new(1);
    ai.current_state = AiState::Default;
    ai.current_substate = Substate::DefaultGotoRoute;
    ai.patrol_path = PatrolPath::new(PathId::new(0).unwrap(), &paths);
    let position = Position {
        x: 0.0,
        y: 0.0,
        sector: SectorHandle::new(1),
        level: 0,
    };

    assert!(ai.route_arrival_turn_direction(position, &paths).is_some());

    let path = ai.patrol_path.as_ref().expect("patrol path");
    assert_eq!(path.current_waypoint_index, 0);
    assert!(
        !path.forward,
        "Original's live --path/++path lookup reverses traversal at waypoint zero"
    );
}

#[test]
fn emoticon_transient() {
    let mut ai = AiController::new(1);
    ai.set_transient_emoticon(EmoticonType::QuestionMark, 100, 500);
    assert_eq!(ai.current_emoticon_type, EmoticonType::QuestionMark);
    assert!(ai.emoticon_has_expiration_date);
    assert_eq!(ai.emoticon_expiration_date, 600);
}

#[test]
fn recon_report() {
    let mut report = ReconnaissanceReport::default();
    assert_eq!(report.report_type, ReportType::Nothing);

    report.update(
        ReportType::Body,
        Position {
            x: 10.0,
            y: 20.0,
            sector: None,
            level: 0,
        },
    );
    assert_eq!(report.report_type, ReportType::Body);

    // Lower priority update should be ignored
    report.update(
        ReportType::Noise,
        Position {
            x: 30.0,
            y: 40.0,
            sector: None,
            level: 0,
        },
    );
    assert_eq!(report.report_type, ReportType::Body);
    assert_eq!(report.seek_position.x, 10.0);

    // Higher priority update should apply
    report.update(
        ReportType::Enemy,
        Position {
            x: 50.0,
            y: 60.0,
            sector: None,
            level: 0,
        },
    );
    assert_eq!(report.report_type, ReportType::Enemy);
    assert_eq!(report.seek_position.x, 50.0);
}

#[test]
fn position_to_point_3d_recovers_number_only_sector_by_position() {
    let mut bbox = crate::coordinates::MapBBox::new();
    let points = vec![
        MapPoint::new(0.0, 0.0),
        MapPoint::new(100.0, 0.0),
        MapPoint::new(100.0, 100.0),
        MapPoint::new(0.0, 100.0),
    ];
    for &point in &points {
        bbox.expand_point(point);
    }

    let sector_number = crate::sector::SectorNumber::new(7);
    let mut level = crate::fast_find_grid::LevelGrid::default();
    level.sectors.push(crate::fast_find_grid::GridSector {
        points,
        bounding_box: bbox,
        sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
        layer: 2,
        sector_number,
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
    });
    // A public number is not an exact identity. Put a second authored sector
    // behind the lossy map entry so the number-only compatibility position
    // must be resolved from its point and layer.
    let duplicate_points = vec![
        MapPoint::new(200.0, 200.0),
        MapPoint::new(300.0, 200.0),
        MapPoint::new(300.0, 300.0),
        MapPoint::new(200.0, 300.0),
    ];
    let mut duplicate_bbox = crate::coordinates::MapBBox::new();
    for &point in &duplicate_points {
        duplicate_bbox.expand_point(point);
    }
    level.sectors.push(crate::fast_find_grid::GridSector {
        points: duplicate_points,
        bounding_box: duplicate_bbox,
        sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
        layer: 2,
        sector_number,
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
    });
    level.sector_number_map.insert(sector_number, 1);

    let mut obstacle = crate::sight_obstacle::SightObstacle::new(
        0,
        crate::sight_obstacle::SIGHTOBSTACLE_SOLID
            | crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA,
    );
    obstacle.obstacle_points = vec![
        crate::sight_obstacle::ObstaclePoint {
            x: 0.0,
            y: 0.0,
            z_bottom: 0.0,
            z_top: 20.0,
        },
        crate::sight_obstacle::ObstaclePoint {
            x: 100.0,
            y: 0.0,
            z_bottom: 0.0,
            z_top: 20.0,
        },
        crate::sight_obstacle::ObstaclePoint {
            x: 100.0,
            y: 100.0,
            z_bottom: 0.0,
            z_top: 20.0,
        },
        crate::sight_obstacle::ObstaclePoint {
            x: 0.0,
            y: 100.0,
            z_bottom: 0.0,
            z_top: 20.0,
        },
    ];
    obstacle.set_projection_area_ref(
        crate::position_interface::Layer::new(2).unwrap(),
        crate::fast_find_grid::SectorIndex::new(0).unwrap(),
    );
    obstacle.top_plane_points = [[0.0, 0.0, 20.0], [100.0, 0.0, 20.0], [0.0, 100.0, 20.0]];
    obstacle.bottom_plane_points = [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [0.0, 100.0, 0.0]];
    obstacle.rebuild_geometry();

    let fast_grid = std::sync::Arc::new(crate::fast_find_grid::FastFindGrid {
        level: std::sync::Arc::new(level),
        line_active: Vec::new(),
        sector_active: vec![true, true],
        mask_active: Vec::new(),
        lift_state: std::collections::BTreeMap::new(),
        sector_type_overlay: std::collections::BTreeMap::new(),
    });
    let sight_obstacles = [obstacle];

    let point = ai_position_to_point_3d(
        &fast_grid,
        crate::sight_obstacle::ObstacleList::from_slice_all_active(&sight_obstacles),
        Position {
            x: 50.0,
            y: 50.0,
            sector: SectorHandle::new(7),
            level: 2,
        },
    );

    assert_eq!(point.x, 50.0);
    assert_eq!(point.y, 70.0);
    assert_eq!(point.z, 20.0);
}

#[test]
fn position_to_point_3d_uses_building_door_outside_projection() {
    use crate::fast_find_grid::{DoorProjectionInfo, GridSector};
    use crate::sector::{SectorNumber, SectorType};

    let mut level = crate::fast_find_grid::LevelGrid::default();
    let building_number = SectorNumber::new(7);
    level.sector_number_map.insert(building_number, 0);
    level.sectors.push(GridSector {
        sector_type: SectorType::AREA | SectorType::MOTION | SectorType::BUILDING,
        sector_number: building_number,
        gate_indices: vec![crate::gate::DoorIndex::new(0).expect("valid door index")],
        points: Vec::new(),
        bounding_box: crate::coordinates::MapBBox::new(),
        layer: 0,
        door_index: None,
        lift_type: None,
        lift_direction: 0,
        force_crouched: false,
        building_index: None,
        low_exit_point: None,
        high_exit_point: None,
        lowest_door_index: None,
        jump_line_indices: Vec::new(),
        underlying_sector: None,
    });
    level.door_projection_infos.push(DoorProjectionInfo {
        point_in: MapPoint::new(50.0, 50.0),
        point_out: MapPoint::new(45.0, 55.0),
        sector_out: SectorNumber::new(8),
        sector_out_index: crate::fast_find_grid::SectorIndex::new(8),
        layer_out: 2,
    });

    let mut obstacle = crate::sight_obstacle::SightObstacle::new(
        0,
        crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA,
    );
    obstacle.box_projection = crate::coordinates::MapBBox::from_geo(
        crate::geo2d::BBox2D::from_coords(0.0, 0.0, 100.0, 100.0),
    );
    obstacle.obstacle_points = vec![
        crate::sight_obstacle::ObstaclePoint {
            x: 0.0,
            y: 0.0,
            z_bottom: 0.0,
            z_top: 20.0,
        },
        crate::sight_obstacle::ObstaclePoint {
            x: 100.0,
            y: 0.0,
            z_bottom: 0.0,
            z_top: 20.0,
        },
        crate::sight_obstacle::ObstaclePoint {
            x: 100.0,
            y: 100.0,
            z_bottom: 0.0,
            z_top: 20.0,
        },
        crate::sight_obstacle::ObstaclePoint {
            x: 0.0,
            y: 100.0,
            z_bottom: 0.0,
            z_top: 20.0,
        },
    ];
    obstacle.set_projection_area_ref(
        crate::position_interface::Layer::new(2).unwrap(),
        crate::fast_find_grid::SectorIndex::new(8).unwrap(),
    );
    obstacle.top_plane_points = [[0.0, 0.0, 20.0], [100.0, 0.0, 20.0], [0.0, 100.0, 20.0]];
    obstacle.bottom_plane_points = [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [0.0, 100.0, 0.0]];
    obstacle.rebuild_geometry();

    let fast_grid = std::sync::Arc::new(crate::fast_find_grid::FastFindGrid {
        level: std::sync::Arc::new(level),
        line_active: Vec::new(),
        sector_active: vec![true],
        mask_active: Vec::new(),
        lift_state: std::collections::BTreeMap::new(),
        sector_type_overlay: std::collections::BTreeMap::new(),
    });
    let sight_obstacles = [obstacle];

    let point = ai_position_to_point_3d(
        &fast_grid,
        crate::sight_obstacle::ObstacleList::from_slice_all_active(&sight_obstacles),
        Position {
            x: 50.0,
            y: 50.0,
            sector: SectorHandle::new(7),
            level: 9,
        },
    );

    assert_eq!(point.x, 50.0);
    assert_eq!(point.y, 70.0);
    assert_eq!(point.z, 20.0);
}

// ── House / building-AI tests ─────────────────────────────────

#[test]
fn house_default_values() {
    let h = House::default();
    assert_eq!(h.sector_index, 0);
    assert_eq!(h.building_index, None);
    assert!(h.door_indices.is_empty());
    assert!(h.occupant_ids.is_empty());
    assert!(!h.arrow_reserve);
}

/// Mirrors the enter / leave sequence that `execute_pass_door`
/// runs when an actor walks through a building door — a direct
/// unit-level exercise of the same Vec `push` / `retain` logic
/// used by the runtime hooks, so regressions in
/// `House::occupant_ids` semantics are caught without needing a
/// full engine fixture.
#[test]
fn house_occupant_enter_leave_cycle() {
    use crate::element::EntityId;

    let mut h = House {
        sector_index: 42,
        ..House::default()
    };
    let a = EntityId::Pc(crate::entity_id::PcId(1));
    let b = EntityId::Pc(crate::entity_id::PcId(2));

    // Enter A, then B
    if !h.occupant_ids.contains(&a) {
        h.occupant_ids.push(a);
    }
    if !h.occupant_ids.contains(&b) {
        h.occupant_ids.push(b);
    }
    assert_eq!(h.occupant_ids, vec![a, b]);

    // Dedup: re-entering A while already inside is a no-op.
    if !h.occupant_ids.contains(&a) {
        h.occupant_ids.push(a);
    }
    assert_eq!(h.occupant_ids, vec![a, b]);

    // Leave A — B stays.
    h.occupant_ids.retain(|&e| e != a);
    assert_eq!(h.occupant_ids, vec![b]);

    // Leave B — empty list, house entry still alive.
    h.occupant_ids.retain(|&e| e != b);
    assert!(h.occupant_ids.is_empty());
    assert_eq!(h.sector_index, 42);
}

#[test]
fn house_occupancy_helpers() {
    use crate::element::EntityId;
    let mut h = House::default();
    assert_eq!(h.occupant_count(), 0);
    h.occupant_ids.push(EntityId::Pc(crate::entity_id::PcId(1)));
    h.occupant_ids.push(EntityId::Pc(crate::entity_id::PcId(2)));
    assert_eq!(h.occupant_count(), 2);
    assert!(h.contains_occupant(EntityId::Pc(crate::entity_id::PcId(1))));
    assert!(!h.contains_occupant(EntityId::Pc(crate::entity_id::PcId(99))));
}

#[test]
fn ambush_point_init_lift_defaults() {
    // New AmbushPoints default to z=0 and id=0 before init runs.
    let ap = AmbushPoint {
        position: Position {
            x: 100.0,
            y: 200.0,
            sector: None,
            level: 0,
        },
        direction: 0,
        position_3d: crate::coordinates::WorldPoint3D::default(),
        id: 0,
    };
    assert_eq!(ap.position_3d.z, 0.0);
    assert_eq!(ap.id, 0);
}

#[test]
fn ai_outbox_drain_barriers_are_independent_and_serializable() {
    let mut outbox = AiOutbox::default();
    outbox.actor.halt = true;
    outbox.actor.stop_menace = true;
    outbox.actor.quit_swordfight = true;
    outbox
        .reentrant
        .self_stimuli
        .push(StimulusType::EventDone.into());
    outbox.music.instant_change = true;

    let encoded = serde_json::to_string(&outbox).expect("serialize AI outbox");
    let mut decoded: AiOutbox = serde_json::from_str(&encoded).expect("deserialize AI outbox");

    assert!(decoded.actor.take_halt());
    let preemption = decoded.actor.take_movement_prefixes();
    assert!(preemption.stop_menace);
    assert!(!preemption.lower_shield);
    assert!(!decoded.actor.halt);
    assert!(decoded.actor.quit_swordfight);
    assert_eq!(
        decoded.reentrant.self_stimuli,
        vec![StimulusType::EventDone]
    );
    assert!(decoded.music.instant_change);

    let core = decoded.actor.take_core();
    assert!(core.quit_swordfight);
    assert!(!decoded.actor.quit_swordfight);
    assert_eq!(
        decoded.reentrant.self_stimuli,
        vec![StimulusType::EventDone]
    );
    assert!(decoded.music.instant_change);

    assert_eq!(
        decoded.reentrant.self_stimuli,
        vec![StimulusType::EventDone]
    );
    assert!(decoded.music.instant_change);
}

#[test]
fn self_stimulus_provenance_is_live_only_and_survives_queue_conversion() {
    let queued = QueuedSelfStimulus::new(
        StimulusType::EventCouldntReachPoint,
        SelfStimulusOrigin::EngineCompletion,
    );
    let stimulus = Stimulus::from_queued_self(queued);
    assert_eq!(stimulus.stimulus_type, StimulusType::EventCouldntReachPoint);
    assert_eq!(stimulus.self_origin, SelfStimulusOrigin::EngineCompletion);

    let encoded = serde_json::to_string(&queued).expect("serialize queued self-stimulus");
    assert_eq!(
        encoded,
        serde_json::to_string(&StimulusType::EventCouldntReachPoint)
            .expect("serialize legacy self-stimulus")
    );
    let decoded: QueuedSelfStimulus =
        serde_json::from_str(&encoded).expect("deserialize queued self-stimulus");
    assert_eq!(decoded.stimulus_type, StimulusType::EventCouldntReachPoint);
    assert_eq!(decoded.origin, SelfStimulusOrigin::Ordinary);

    assert_eq!(
        robin_util::state_hash::compute(&vec![queued]),
        robin_util::state_hash::compute(&vec![StimulusType::EventCouldntReachPoint]),
        "runtime provenance must not perturb the prior replay-state hash"
    );
}

#[test]
fn clear_all_pending_clears_every_outbox_barrier() {
    let mut ai = AiController::default();
    ai.outbox.patrol.direction_broadcast = Some(7);
    ai.outbox
        .detection
        .stimuli
        .push(Stimulus::new(StimulusType::EventView));
    ai.outbox.detection.mark_alerted = true;
    ai.outbox
        .reentrant
        .self_stimuli
        .push(StimulusType::EventDone.into());

    ai.outbox.actor.blink_all_enemies = true;
    ai.outbox.actor.enemy_in_house_alert = true;
    ai.outbox.actor.set_attentive_mode = Some(AttentiveModeEffect::new(true, false));
    ai.outbox.actor.set_guarded_pc = Some(GuardedPcEffect {
        old: Some(crate::entity_id::PcId(4)),
        new: Some(crate::entity_id::PcId(5)),
    });
    ai.outbox.actor.begin_panic = Some(PanicRequest {
        center: None,
        runs: 2,
        alert: AlertLevel::Yellow,
        is_new_panic: true,
    });

    ai.outbox.recovery.inform_resurrection = true;
    ai.outbox.recovery.set_eye_status = Some(crate::element::EyeStatus::Closed);
    ai.outbox.music.instant_change = true;

    ai.clear_all_pending();

    assert_eq!(
        serde_json::to_value(&ai.outbox).expect("serialize cleared outbox"),
        serde_json::to_value(AiOutbox::default()).expect("serialize default outbox")
    );
}

#[test]
fn every_real_substate_has_exactly_one_numeric_family() {
    use Substate::*;

    for raw in 0..NumberOfSubstates as u32 {
        let substate = Substate::try_from(raw).expect("contiguous substate discriminant");
        let is_marker = matches!(
            substate,
            StartSleepingSubstates
                | EndSleepingSubstates
                | StartDefaultSubstates
                | EndDefaultSubstates
                | StartWonderingSubstates
                | EndWonderingSubstates
                | StartSeekingSubstates
                | EndSeekingSubstates
                | StartAttackingSubstates
                | EndAttackingSubstates
                | StartMenacingSubstates
                | EndMenacingSubstates
                | StartFleeingSubstates
                | EndFleeingSubstates
                | BeginAdditionalSubstates
        );
        assert_eq!(
            substate.ai_state_family().is_none(),
            is_marker,
            "unexpected numeric family mapping for {substate:?}"
        );
    }

    assert_eq!(Substate::None.ai_state_family(), std::option::Option::None);
    assert_eq!(
        Substate::NumberOfSubstates.ai_state_family(),
        std::option::Option::None
    );
}
