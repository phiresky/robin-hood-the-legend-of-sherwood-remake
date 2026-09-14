use super::*;
use crate::coordinates::MapPoint;

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
    }
}

#[test]
fn officer_search_charly_preserves_macro_until_live_alert_execution() {
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

    let call = ai
        .search_charly(
            ThinkEnv::new(&sim, &ctx, &AiPerTickData::stub(), None),
            &mut global,
        )
        .expect_err("officer alert executes in the engine");

    assert!(ai.base.macro_in_progress);
    assert_eq!(ai.base.macro_command_offset, 23);
    assert_eq!(ai.base.number_of_remaining_macro_bytes, 0);
    assert_eq!(ai.base.current_state, AiState::Default);
    assert!(matches!(call.tail, crate::ai::DutyTail::AlertSoldiers {
        flags, failure: crate::ai::AlertSoldiersFailureContinuation::SeekMissedCharly { .. }, ..
    } if flags == SeekFlags::CHARLY_SEEK.bits()));
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
        ThinkEnv::new(&sim, &ctx, &AiPerTickData::stub(), None),
        &mut AiGlobalState::default(),
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
        ai.append_global_area_seek_points(&sim, ctx.frame, None, 0, false, spec, &mut global);
    });

    assert_eq!(
        draws,
        [RngSite::SeekPointSelection, RngSite::SeekPointSelection]
    );
    assert_eq!(ai.my_seek_points, [0, 0]);
    assert_eq!(global.seek_points[0].last_calculated_interest, 100);
}
