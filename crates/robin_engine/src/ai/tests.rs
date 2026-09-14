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
    assert!(!h.arrow_reserve);
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
