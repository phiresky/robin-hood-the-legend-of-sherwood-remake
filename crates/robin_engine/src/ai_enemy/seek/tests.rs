use super::*;
use crate::ai_enemy::CampSoldierInfo;
use crate::coordinates::MapPoint;

fn seek_test_building_door(
    door_index: u32,
    point_out: MapPoint,
    position_in: Position,
    sector_out_index: Option<crate::fast_find_grid::SectorIndex>,
) -> DoorSeekInfo {
    DoorSeekInfo {
        door_index: crate::gate::DoorIndex::new(door_index).expect("valid door index"),
        door_type: crate::gate::DoorType::Building,
        point_out,
        position_in,
        sector_out: 88,
        sector_out_index,
        sector_in: position_in
            .sector
            .expect("test door has an inside sector")
            .get(),
        layer_out: 2,
        npc_villain_authorized_direct: true,
    }
}

#[test]
fn enemy_behind_door_uses_exact_outside_sector_identity() {
    let exact_outside = crate::fast_find_grid::SectorIndex::new(10).unwrap();
    let duplicate_outside = crate::fast_find_grid::SectorIndex::new(11).unwrap();
    let center = Position {
        x: 100.0,
        y: 100.0,
        sector: SectorHandle::new(88).map(|sector| sector.with_arena_index(exact_outside)),
        level: 2,
    };
    let wrong_inside = Position {
        x: 900.0,
        y: 900.0,
        sector: SectorHandle::new(18),
        level: 0,
    };
    let correct_inside = Position {
        x: 200.0,
        y: 200.0,
        sector: SectorHandle::new(19),
        level: 0,
    };
    let global = AiGlobalState {
        // The wrong duplicate is closer and would win Rust's old public-only
        // comparison. The original game's sector-reference comparison must skip it.
        door_seek_infos: vec![
            seek_test_building_door(
                0,
                MapPoint::new(101.0, 100.0),
                wrong_inside,
                Some(duplicate_outside),
            ),
            seek_test_building_door(
                1,
                MapPoint::new(110.0, 100.0),
                correct_inside,
                Some(exact_outside),
            ),
        ],
        houses: vec![
            House {
                sector_index: 18,
                ..House::default()
            },
            House {
                sector_index: 19,
                ..House::default()
            },
        ],
        ..Default::default()
    };
    let ai = EnemyAi::new(150);
    let ctx = AiContext::test_fixture();
    let direction = vec_to_sector(10.0, 0.0);

    let mut exact_center = center;
    ai.find_door_enemy_could_be_behind(
        &mut exact_center,
        direction,
        &global,
        &ctx,
        &AiPerTickData::stub(),
    );
    assert_eq!(exact_center, correct_inside);

    // Explicitly number-only compatibility data cannot distinguish the
    // duplicate arena objects and retains the legacy public comparison.
    let mut numeric_center = Position {
        sector: SectorHandle::new(88),
        ..center
    };
    ai.find_door_enemy_could_be_behind(
        &mut numeric_center,
        direction,
        &global,
        &ctx,
        &AiPerTickData::stub(),
    );
    assert_eq!(numeric_center, wrong_inside);
}

#[test]
#[should_panic(expected = "lacks exact arena identity required by seek center")]
fn enemy_behind_door_rejects_missing_exact_outside_identity() {
    let mut center = Position {
        x: 100.0,
        y: 100.0,
        sector: SectorHandle::new(88).map(|sector| {
            sector.with_arena_index(crate::fast_find_grid::SectorIndex::new(10).unwrap())
        }),
        level: 2,
    };
    let inside = Position {
        sector: SectorHandle::new(18),
        ..Position::default()
    };
    let mut global = AiGlobalState::default();
    global.door_seek_infos.push(seek_test_building_door(
        0,
        MapPoint::new(101.0, 100.0),
        inside,
        None,
    ));
    global.houses.push(House {
        sector_index: 18,
        ..House::default()
    });

    EnemyAi::new(150).find_door_enemy_could_be_behind(
        &mut center,
        vec_to_sector(1.0, 0.0),
        &global,
        &AiContext::test_fixture(),
        &AiPerTickData::stub(),
    );
}

#[test]
fn sectorless_group_seek_center_recovers_original_position_sector() {
    use crate::coordinates::{MapBBox, MapPoint};
    use crate::fast_find_grid::{FastFindGrid, GridSector};
    use crate::sector::{SectorNumber, SectorType};

    let points = vec![
        MapPoint::new(0.0, 0.0),
        MapPoint::new(128.0, 0.0),
        MapPoint::new(128.0, 128.0),
        MapPoint::new(0.0, 128.0),
    ];
    let mut bounding_box = MapBBox::new();
    for &point in &points {
        bounding_box.expand_point(point);
    }
    let mut grid = FastFindGrid::new();
    grid.size_map(4, 4);
    grid.allocate_layers(1);
    grid.add_sector(
        GridSector {
            points,
            bounding_box,
            sector_type: SectorType::MOUSE | SectorType::MOTION | SectorType::AREA,
            layer: 0,
            sector_number: SectorNumber::new(42),
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
        },
        0,
    );
    let ctx = AiContext {
        position: Position {
            x: 32.0,
            y: 32.0,
            sector: crate::position_interface::SectorHandle::new(42),
            level: 0,
        },
        fast_grid: std::sync::Arc::new(grid),
        ..AiContext::test_fixture()
    };

    let sectorless = Position {
        x: 64.0,
        y: 64.0,
        sector: None,
        level: 0,
    };
    assert_eq!(
        resolve_seek_area_center_sector(sectorless, &ctx).sector,
        crate::position_interface::SectorHandle::new(42)
    );
    assert_eq!(
        resolve_seek_area_center_sector(sectorless, &ctx)
            .sector
            .and_then(|sector| sector.arena_index()),
        crate::fast_find_grid::SectorIndex::new(0),
        "a reconstructed position must retain the exact sector object"
    );

    let matching_number_only = Position {
        sector: crate::position_interface::SectorHandle::new(42),
        ..sectorless
    };
    assert_eq!(
        resolve_seek_area_center_sector(matching_number_only, &ctx)
            .sector
            .and_then(|sector| sector.arena_index()),
        crate::fast_find_grid::SectorIndex::new(0),
        "a static seek point's authored number must be enriched with its original-game identity"
    );

    let authoritative = Position {
        sector: crate::position_interface::SectorHandle::new(7),
        ..sectorless
    };
    assert_eq!(
        resolve_seek_area_center_sector(authoritative, &ctx).sector,
        crate::position_interface::SectorHandle::new(7),
        "an already-authored position sector must not be spatially replaced"
    );
    assert!(
        resolve_seek_area_center_sector(authoritative, &ctx)
            .sector
            .is_some_and(|sector| sector.arena_index().is_none()),
        "a conflicting authored sector must not inherit an unrelated arena identity"
    );
}

fn charly_view() -> crate::ai_entity_view::AiEntityView {
    use crate::ai_entity_view::EntityKind;
    use crate::element::{Camp, Posture};

    crate::ai_entity_view::AiEntityView {
        original_creation_order: 96,
        position: Position::default(),
        detection_position: crate::coordinates::MapPoint::new(0.0, 0.0),
        detection_position_world: crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0),
        direction: 0,
        posture: Posture::Upright,
        camp: Camp::Lacklandists,
        is_pc: false,
        is_robin: false,
        is_vip: false,
        is_beggar: false,
        is_child: false,
        kind: EntityKind::Soldier,
        is_tower_guard: false,
        is_swordfighting: false,
        is_able_to_fight: true,
        active: true,
        is_unconscious: false,
        action_state: crate::element::ActionState::Waiting,
        is_moving_map: false,
        passing_door: false,
        obstacle_idx: None,
        in_building: false,
        building_sector: None,
        script_locked: false,
        forecasted_destination: crate::ai::PreparedForecastDestination::fixed(
            Position::default(),
            0,
        ),
        ai_state: AiState::Default,
        ai_substate: Substate::DefaultEnroute,
        current_animation: crate::order::OrderType::WaitingUpright,
        elevation: 0.0,
        object_type: crate::element_kinds::ObjectType::None,
        is_dead: false,
        is_carried: false,
        is_archer: false,
        is_rider: false,
        stuck_under_net: false,
        covering_nets: Vec::new(),
        in_coma: false,
        guard: None,
        has_patrol_path: false,
        initial_position: Position::default(),
        number_of_arrows: 0,
        rank: ProfileRank::Soldier,
        reported_to_officer: false,
        looted_after_money_fight: false,
        current_money: 0,
        macro_in_progress: false,
        path_current_waypoint_index: 0,
        path_last_waypoint_index: 0,
        path_forward_movement: true,
        patrol_hiking_path_index: None,
        interesting_object: None,
        report_type: ReportType::Nothing,
        report_seek_position: Position::default(),
        report_seen_bodies: Vec::new(),
        report_charly: None,
    }
}

fn alert_test_officer(handle: u32, substate: Substate) -> CampSoldierInfo {
    let position = Position {
        x: 80.0,
        y: 40.0,
        ..Position::default()
    };
    CampSoldierInfo {
        handle,
        active: true,
        position,
        position_world: crate::coordinates::WorldPoint3D::new(80.0, 40.0, 0.0),
        direction: 0,
        rank: ProfileRank::Officer,
        ai_state: AiState::Default,
        ai_substate: substate,
        is_able_to_fight: true,
        is_dead: false,
        knocked_out_in_money_fight: false,
        primary_target: None,
        pride: 0,
        is_able_to_help: true,
        script_locked: false,
        ai_lock_frozen: false,
        layer: 0,
        report_type: ReportType::Nothing,
        report_seek_position: Position::default(),
        report_seen_bodies: Vec::new(),
        report_charly: None,
        alert_soldiers_point: Position::default(),
        patrol_chief: None,
        antagonist: None,
        detected_body: None,
        blood_alcohol: 0,
        duty_flag: false,
        is_tower_guard: false,
        company_number: 0,
        in_building: false,
        forecast_destination: Some(crate::ai::PreparedForecastDestination::fixed(position, 0)),
        detectable_bodies: Vec::new(),
        seek_position: Position::default(),
        current_task_priority: 0,
        minimal_task_priority: 0,
        view_direction: [1.0, 0.0],
        view_radius: 500,
        real_half_aperture: 1.0,
        eye_blind: false,
    }
}

#[test]
fn officer_search_charly_preserves_check_for_macro_into_inline_seek_area() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(202);
    ai.soldier_profile_rank = ProfileRank::Officer;
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultLookingForCharly;
    ai.base.macro_in_progress = true;
    ai.base.macro_command_offset = 23;
    ai.base.number_of_remaining_macro_bytes = 0;
    ai.base.checkpoint_charly = Some(AiEntityHandle::new(96));

    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(96, charly_view());
    let ctx = AiContext {
        frame: 34_426,
        camp: crate::element::Camp::Lacklandists,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut global = AiGlobalState::default();
    global.seek_points.push(SeekPoint {
        position: Position {
            x: 100.0,
            ..Position::default()
        },
        frame_when_full_interest: 0,
        directions: vec![4],
        last_calculated_interest: 100,
        locked: false,
        id: 0,
    });

    ai.search_charly(&sim, &mut global, &ctx, &AiPerTickData::stub(), None);

    assert!(ai.base.macro_in_progress);
    assert_eq!(ai.base.macro_command_offset, 23);
    assert_eq!(ai.base.number_of_remaining_macro_bytes, 0);
    assert_eq!(ai.base.current_state, AiState::Seeking);
}

#[test]
fn soldier_search_charly_preserves_authored_waypoint_sector() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(202);
    ai.soldier_profile_rank = ProfileRank::Soldier;
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultLookingForCharly;
    ai.base.checkpoint_charly = Some(AiEntityHandle::new(96));

    let mut charly = charly_view();
    charly.has_patrol_path = true;
    charly.patrol_hiking_path_index = PathId::new(0);
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(96, charly);
    let ctx = AiContext {
        position: Position {
            x: 10.0,
            y: 20.0,
            sector: crate::position_interface::SectorHandle::new(3),
            level: 0,
        },
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        hiking_paths: std::sync::Arc::new(vec![crate::level_data::RawHikingPath {
            waypoints: vec![crate::level_data::RawWaypoint {
                x: 30,
                y: 40,
                sector: 77,
                level: 2,
                command: crate::level_data::WaypointCommand::None,
            }],
        }]),
        ..AiContext::test_fixture()
    };

    ai.search_charly(
        &sim,
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(
        ai.search_charly_way[0].sector.map(|sector| sector.get()),
        Some(77)
    );
    assert_eq!(
        ai.base
            .outbox
            .actor
            .orders
            .last()
            .expect("missing-PC search queues its first waypoint")
            .target_sector
            .map(|sector| sector.get()),
        Some(77)
    );
}

#[test]
fn dead_body_alert_with_officer_queues_actor_prefix_then_typed_resume() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(40);
    ai.company_number = 0;
    ai.base.antagonist = Some(AiEntityHandle::new(99));
    let center = Position {
        x: 10.0,
        y: 20.0,
        ..Position::default()
    };
    let ctx = AiContext {
        camp: crate::element::Camp::Lacklandists,
        in_building: true,
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.camp_soldiers
        .push(alert_test_officer(7, Substate::DefaultGotoPost));

    ai.dead_body_alert(
        &sim,
        center,
        SeekFlags::empty(),
        &mut AiGlobalState::default(),
        None,
        &ctx,
        &tick,
    );

    assert!(ai.base.outbox.reentrant.dead_body_alert_completion_pending);
    assert_eq!(ai.base.outbox.reentrant.owner_work.len(), 3);
    assert!(matches!(
        ai.base.outbox.reentrant.owner_work[0],
        crate::ai::AiOwnerWork::StateChange(_)
    ));
    assert!(matches!(
        ai.base.outbox.reentrant.owner_work[1],
        crate::ai::AiOwnerWork::ActorEffects(_)
    ));
    assert!(matches!(
        ai.base.outbox.reentrant.owner_work[2],
        crate::ai::AiOwnerWork::ResumeDeadBodyAlertAfterAlertOfficer {
            center: queued_center,
            radius: 300
        } if queued_center == center
    ));
    assert!(ai.base.outbox.actor.orders.is_empty());

    // Settle the queued approach as a route failure, then run the exact
    // typed tail that owns that result.
    ai.base.couldnt_reachpoint = true;
    ai.base.outbox.reentrant.dead_body_alert_completion_pending = false;
    ai.resume_dead_body_alert_after_alert_officer(
        &sim,
        center,
        300,
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
    );
    assert_eq!(
        ai.seek_flags,
        SeekFlags::LOCATION_END | SeekFlags::BODY_SEEK
    );
    assert!(ai.personal_seek_point_2.is_some());
    assert!(ai.base.outbox.reentrant.self_stimuli.is_empty());
}

#[test]
fn dead_body_alert_instructed_group_route_never_queues_fallback() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(44);
    ai.company_number = 0;
    ai.base.antagonist = Some(AiEntityHandle::new(7));
    ai.seek_flags = SeekFlags::REPORT_OFFICER_AFTER;
    let ctx = AiContext {
        camp: crate::element::Camp::Lacklandists,
        in_building: true,
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.camp_soldiers.push(alert_test_officer(
        7,
        Substate::SeekingOfficerWaitForInstructedGroup,
    ));

    ai.dead_body_alert(
        &sim,
        Position::default(),
        SeekFlags::empty(),
        &mut AiGlobalState::default(),
        None,
        &ctx,
        &tick,
    );

    // Route construction reports failure only after the AI borrow is
    // released. With no typed tail queued, that later result cannot run
    // the ordinary corpse-alert fallback.
    ai.base.couldnt_reachpoint = true;
    assert!(!ai.base.outbox.reentrant.dead_body_alert_completion_pending);
    assert!(
        !ai.base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .any(|work| matches!(
                work,
                crate::ai::AiOwnerWork::ResumeDeadBodyAlertAfterAlertOfficer { .. }
            ))
    );
    assert!(ai.personal_seek_point_2.is_none());
    assert!(!ai.seek_flags.contains(SeekFlags::BODY_SEEK));
    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingSoldierReturnToOfficer
    );
}

#[test]
fn failed_dead_body_alert_officer_route_falls_back_to_body_seek() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(41);
    ai.company_number = 0;
    ai.base.couldnt_reachpoint = true;
    let center = Position {
        x: 120.0,
        y: 240.0,
        ..Position::default()
    };
    let ctx = AiContext {
        camp: crate::element::Camp::Lacklandists,
        in_building: true,
        ..AiContext::test_fixture()
    };

    ai.resume_dead_body_alert_after_alert_officer(
        &sim,
        center,
        300,
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
    );

    assert!(!ai.base.couldnt_reachpoint);
    assert_eq!(
        ai.seek_flags,
        SeekFlags::LOCATION_END | SeekFlags::BODY_SEEK
    );
    assert_eq!(ai.my_seek_points, vec![2222]);
    assert_eq!(
        ai.personal_seek_point_2
            .as_ref()
            .expect("body search owns its personal endpoint")
            .position,
        center
    );
    assert!(ai.base.outbox.reentrant.self_stimuli.is_empty());
}

#[test]
fn successful_dead_body_alert_officer_route_has_no_fallback() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(42);
    ai.company_number = 0;
    let ctx = AiContext {
        camp: crate::element::Camp::Lacklandists,
        in_building: true,
        ..AiContext::test_fixture()
    };

    ai.resume_dead_body_alert_after_alert_officer(
        &sim,
        Position::default(),
        300,
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
    );

    assert!(ai.seek_flags.is_empty());
    assert!(ai.my_seek_points.is_empty());
    assert!(ai.personal_seek_point_2.is_none());
    assert!(ai.base.outbox.actor.orders.is_empty());
}

#[test]
fn dead_body_alert_without_officer_falls_back_immediately() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(43);
    ai.company_number = 0;
    // Force the officer-alert path independently of the random answer.
    ai.base.antagonist = Some(AiEntityHandle::new(99));
    let center = Position {
        x: 60.0,
        y: 90.0,
        ..Position::default()
    };
    let ctx = AiContext {
        camp: crate::element::Camp::Lacklandists,
        in_building: true,
        ..AiContext::test_fixture()
    };

    ai.dead_body_alert(
        &sim,
        center,
        SeekFlags::empty(),
        &mut AiGlobalState::default(),
        None,
        &ctx,
        &AiPerTickData::stub(),
    );

    assert_eq!(
        ai.seek_flags,
        SeekFlags::LOCATION_END | SeekFlags::BODY_SEEK
    );
    assert_eq!(ai.my_seek_points, vec![2222]);
    assert!(ai.personal_seek_point_2.is_some());
    assert!(!ai.base.outbox.reentrant.dead_body_alert_completion_pending);
    assert!(
        !ai.base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .any(|work| matches!(
                work,
                crate::ai::AiOwnerWork::ResumeDeadBodyAlertAfterAlertOfficer { .. }
            ))
    );
}

#[test]
fn seek_point_interest_accumulator_narrows_once_after_double_arithmetic() {
    let accumulated = accumulate_seek_point_interest(0.0, 10);
    let all_f32 = 10.0_f32 * 0.01_f32;

    assert_eq!(accumulated.to_bits(), 0x3dcc_cccd);
    assert_eq!(all_f32.to_bits(), 0x3dcc_cccc);
}

#[test]
fn seek_direction_delta_preserves_original_uword_wrap() {
    assert_eq!(legacy_seek_direction_delta(0, 17), u16::MAX);
    assert_eq!(legacy_seek_direction_delta(14, 0), 30);
}

#[test]
fn seek_point_interest_accumulator_preserves_original_threshold_crossing() {
    let interests = [55, 19, 32, 44, 1, 44, 27, 97, 56, 15, 83, 26, 14, 0, 56, 31];
    let accumulated = interests
        .into_iter()
        .fold(0.0, accumulate_seek_point_interest);
    let all_f32 = interests.into_iter().fold(0.0_f32, |current, interest| {
        current + f32::from(interest) * 0.01_f32
    });

    assert_eq!(accumulated.to_bits(), 0x40c0_0001);
    assert_eq!(all_f32.to_bits(), 0x40bf_ffff);
    assert!(accumulated >= 6.0);
    assert!(all_f32 < 6.0);
}

#[test]
fn seek_point_interest_accumulator_keeps_exact_threshold_control() {
    let accumulated = [50, 50]
        .into_iter()
        .fold(0.0, accumulate_seek_point_interest);

    assert_eq!(accumulated, 1.0);
}

#[test]
fn seek_area_obligatory_selection_respects_original_finite_sentinel() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(131);
    let center = Position {
        x: 1_585.620_1,
        y: 2_454.293_2,
        sector: None,
        level: 0,
    };
    let mut global = AiGlobalState {
        seek_points: vec![
            SeekPoint {
                position: Position {
                    x: 1547.0,
                    y: 2488.0,
                    sector: None,
                    level: 0,
                },
                frame_when_full_interest: 0,
                directions: vec![],
                last_calculated_interest: 100,
                locked: false,
                id: 212,
            },
            SeekPoint {
                position: Position {
                    x: 1753.0,
                    y: 2670.0,
                    sector: None,
                    level: 0,
                },
                frame_when_full_interest: 0,
                directions: vec![],
                last_calculated_interest: 100,
                locked: false,
                id: 218,
            },
        ],
        ..Default::default()
    };
    let ctx = AiContext {
        camp: crate::element::Camp::Lacklandists,
        in_building: true,
        ..AiContext::test_fixture()
    };

    ai.seek_area(
        &sim,
        center,
        300,
        SeekFlags::empty(),
        8,
        &mut global,
        &ctx,
        &AiPerTickData::stub(),
    );

    assert_eq!(ai.my_seek_points.first(), Some(&212));
}

#[test]
fn seek_next_point_preserves_the_search_center() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(118);
    let search_center = Position {
        x: 1_397.773,
        y: 1_864.478_5,
        sector: None,
        level: 0,
    };
    let route_point = Position {
        x: 1236.0,
        y: 1589.0,
        sector: None,
        level: 8,
    };
    ai.base.seek_position = search_center;
    ai.my_seek_points.push(1111);
    ai.personal_seek_point_1 = Some(SeekPoint {
        position: route_point,
        frame_when_full_interest: 0,
        directions: vec![4, 10, 15],
        last_calculated_interest: 100,
        locked: false,
        id: 1111,
    });

    ai.seek_next_point(
        &sim,
        &mut AiGlobalState::default(),
        &AiContext::test_fixture(),
        &AiPerTickData::stub(),
    );

    assert_eq!(ai.base.last_goto_destination, route_point);
    assert_eq!(ai.base.seek_position, search_center);
}

#[test]
fn locked_seek_point_skips_interest_recalculation_and_acceptance_draw() {
    use crate::sim_rng::{RngSite, with_draw_trace};

    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(118);
    ai.my_seek_points = vec![0, 1];
    let locked_position = Position {
        x: 100.0,
        ..Position::default()
    };
    let accepted_position = Position {
        x: 200.0,
        ..Position::default()
    };
    let mut global = AiGlobalState {
        seek_points: vec![
            SeekPoint {
                position: locked_position,
                frame_when_full_interest: 1_000,
                directions: vec![2],
                last_calculated_interest: 7,
                locked: true,
                id: 0,
            },
            SeekPoint {
                position: accepted_position,
                frame_when_full_interest: 0,
                directions: vec![4],
                last_calculated_interest: 3,
                locked: false,
                id: 1,
            },
        ],
        ..Default::default()
    };
    let ctx = AiContext {
        frame: 500,
        ..AiContext::test_fixture()
    };

    let (_, draws) = with_draw_trace(|| {
        ai.seek_next_point(&sim, &mut global, &ctx, &AiPerTickData::stub());
    });

    assert_eq!(draws, [RngSite::SeekPointAcceptance]);
    assert_eq!(global.seek_points[0].last_calculated_interest, 7);
    assert!(!global.seek_points[0].locked);
    assert_eq!(global.seek_points[1].last_calculated_interest, 100);
    assert!(global.seek_points[1].locked);
    assert_eq!(ai.actual_seek_point, Some(1));
    assert_eq!(ai.base.last_goto_destination, accepted_position);
}

#[test]
fn unlocked_seek_point_recalculates_draws_subtracts_and_locks() {
    use crate::sim_rng::{RngSite, with_draw_trace};

    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(118);
    ai.my_seek_points.push(0);
    let destination = Position {
        x: 300.0,
        ..Position::default()
    };
    let mut global = AiGlobalState::default();
    global.seek_points.push(SeekPoint {
        position: destination,
        frame_when_full_interest: 501,
        directions: vec![6],
        last_calculated_interest: 7,
        locked: false,
        id: 0,
    });
    let ctx = AiContext {
        frame: 500,
        ..AiContext::test_fixture()
    };

    let (_, draws) = with_draw_trace(|| {
        ai.seek_next_point(&sim, &mut global, &ctx, &AiPerTickData::stub());
    });

    assert_eq!(draws, [RngSite::SeekPointAcceptance]);
    assert_eq!(global.seek_points[0].last_calculated_interest, 100);
    assert_eq!(global.seek_points[0].frame_when_full_interest, 5_501);
    assert!(global.seek_points[0].locked);
    assert_eq!(ai.actual_seek_point, Some(0));
    assert_eq!(ai.base.last_goto_destination, destination);
}

#[test]
fn beggar_detour_retains_old_seek_point_for_second_unlock() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(223);
    ai.actual_seek_point = Some(0);
    ai.beggars_to_control.push(999);
    ai.positions_of_beggars_to_control.push(Position {
        x: 50.0,
        y: 60.0,
        ..Position::default()
    });

    let next_position = Position {
        x: 300.0,
        y: 400.0,
        ..Position::default()
    };
    let mut global = AiGlobalState {
        seek_points: vec![
            SeekPoint {
                position: Position {
                    x: 1176.0,
                    y: 1958.0,
                    ..Position::default()
                },
                frame_when_full_interest: 0,
                directions: vec![2],
                last_calculated_interest: 55,
                locked: true,
                id: 0,
            },
            SeekPoint {
                position: next_position,
                frame_when_full_interest: 0,
                directions: vec![4],
                last_calculated_interest: 100,
                locked: false,
                id: 1,
            },
        ],
        ..Default::default()
    };
    let ctx = AiContext::test_fixture();

    ai.seek_next_point(&sim, &mut global, &ctx, &AiPerTickData::stub());

    assert_eq!(ai.actual_seek_point, Some(0));
    assert!(!global.seek_points[0].locked);
    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingSeekpointApproachingBeggar
    );

    // A different investigator selects the shared point while this AI is
    // away identifying the beggar. The original game's retained identity makes the
    // resumed next-point selection clear that intervening lock again.
    global.seek_points[0].locked = true;
    ai.beggar_to_examine = None;
    ai.my_seek_points.push(1);

    ai.seek_next_point(&sim, &mut global, &ctx, &AiPerTickData::stub());

    assert!(!global.seek_points[0].locked);
    assert_eq!(ai.actual_seek_point, Some(1));
    assert!(global.seek_points[1].locked);
    assert_eq!(ai.base.last_goto_destination, next_position);
}

#[test]
fn area_candidates_preserve_distance_ties_and_strict_radius_boundaries() {
    let global = AiGlobalState {
        seek_points: [(10.0, 0), (-10.0, 0), (1_000.0, 0), (0.0, 1)]
            .into_iter()
            .enumerate()
            .map(|(index, (x, level))| SeekPoint {
                position: Position {
                    x,
                    level,
                    ..Position::default()
                },
                frame_when_full_interest: 0,
                directions: vec![],
                last_calculated_interest: 100,
                locked: false,
                id: index as u16,
            })
            .collect(),
        ..Default::default()
    };
    let candidates = SeekAreaCandidates::new(
        SeekAreaSpec {
            center: Position::default(),
            standard_radius: 10,
            flags: SeekFlags::empty(),
            seek_direction: vec_to_sector(10.0, 0.0),
        },
        &global,
    );

    // Equal distances keep global-array order, including the layer penalty.
    assert_eq!(candidates.square_norms, [100.0, 100.0, 1_000_000.0, 100.0]);
    assert_eq!(candidates.near_sorted, [0, 1, 3]);
    // Exactly on the standard radius does not increase the initial count.
    assert_eq!(candidates.expected_points_for_one, 1);
    assert_eq!(candidates.obligatory_idx, Some(0));
}

#[test]
fn area_global_selection_keeps_first_insertion_draw_and_obligatory_duplicate() {
    use crate::sim_rng::{RngSite, with_draw_trace};

    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(118);
    let mut global = AiGlobalState {
        seek_points: vec![SeekPoint {
            position: Position {
                x: 10.0,
                ..Position::default()
            },
            frame_when_full_interest: 0,
            directions: vec![],
            last_calculated_interest: 7,
            locked: false,
            id: 0,
        }],
        ..Default::default()
    };
    let spec = SeekAreaSpec {
        center: Position::default(),
        standard_radius: 100,
        flags: SeekFlags::empty(),
        seek_direction: vec_to_sector(10.0, 0.0),
    };
    let ctx = AiContext {
        frame: 500,
        ..AiContext::test_fixture()
    };
    let (_, draws) = with_draw_trace(|| {
        ai.append_global_area_seek_points(&sim, spec, &mut global, &ctx, &AiPerTickData::stub());
    });

    assert_eq!(
        draws,
        [RngSite::SeekPointSelection, RngSite::SeekPointSelection]
    );
    assert_eq!(ai.my_seek_points, [0, 0]);
    assert_eq!(global.seek_points[0].last_calculated_interest, 100);
}
