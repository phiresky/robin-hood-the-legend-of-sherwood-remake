use super::*;

fn alert_candidate(handle: u32, position: Position) -> CampSoldierInfo {
    CampSoldierInfo {
        handle,
        active: true,
        position,
        position_world: WorldPoint3D::new(position.x, position.y, 0.0),
        direction: 0,
        rank: ProfileRank::Soldier,
        ai_state: AiState::Default,
        ai_substate: Substate::DefaultOnPost,
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
        forecast_destination: None,
        detectable_bodies: Vec::new(),
        seek_position: Position::default(),
        current_task_priority: 0,
        minimal_task_priority: 0,
        view_direction: [1.0, 0.0],
        view_radius: 300,
        real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
        eye_blind: false,
    }
}

fn soldier_entity_view(position: Position) -> crate::ai_entity_view::AiEntityView {
    let entity = crate::element::Entity::Soldier(crate::element::ActorSoldier {
        element: crate::element::ElementData::default(),
        actor: crate::element::ActorData::default(),
        human: crate::element::HumanData::default(),
        npc: crate::element::NpcData::default(),
        soldier: crate::element::SoldierData::default(),
    });
    let mut view = crate::ai_entity_view::entity_view_from_entity(
        &entity,
        127,
        false,
        None,
        None,
        crate::order::OrderType::WaitingUpright,
    );
    view.position = position;
    view
}

#[test]
fn alert_soldier_radius_keeps_positive_strict_float_gates() {
    let officer = Position::default();
    let at = |x, y| Position {
        x,
        y,
        ..Position::default()
    };

    assert!(alert_soldier_is_inside_radius(at(3.0, 4.0), officer, 6.0));
    assert!(
        !alert_soldier_is_inside_radius(at(5.0, 0.0), officer, 5.0),
        "the strict maximum-norm comparison rejects the boundary"
    );
    assert!(
        !alert_soldier_is_inside_radius(at(3.0, 4.0), officer, 5.0),
        "the strict squared-norm comparison rejects its boundary"
    );
    assert!(
        !alert_soldier_is_inside_radius(at(4.0, 4.0), officer, 5.0),
        "the squared-radius gate still rejects points inside the maximum-norm box"
    );
    assert!(
        !alert_soldier_is_inside_radius(at(f32::NAN, 0.0), officer, 5.0),
        "an unordered X coordinate must fail Original's positive comparisons"
    );
    assert!(
        !alert_soldier_is_inside_radius(at(0.0, f32::NAN), officer, 5.0),
        "an unordered Y coordinate must fail Original's positive comparisons"
    );
}

#[test]
fn alert_soldiers_radius_uses_door_resolved_ai_position() {
    // Save018/r040: Soldier96's interpolated body lies inside the
    // officer's circle, while Position(Soldier96) resolves its active
    // door pass to gate 27's point-out and lies outside it. Original
    // therefore rejects the soldier before delivering CALL_ALERT.
    let officer = Position {
        x: 1723.7462,
        y: 747.5348,
        ..Position::default()
    };
    let raw_body = Position {
        x: 1458.0,
        y: 331.0,
        ..Position::default()
    };
    let gate_point_out = Position {
        x: 1416.0,
        y: 344.0,
        ..Position::default()
    };
    let radius = combat::ALERT_RADIUS as f32;
    assert!(alert_soldier_is_inside_radius(raw_body, officer, radius));
    assert!(!alert_soldier_is_inside_radius(
        gate_point_out,
        officer,
        radius
    ));

    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(96, soldier_entity_view(gate_point_out));
    let ctx = AiContext {
        position: officer,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.camp_soldiers.push(alert_candidate(96, raw_body));

    let mut ai = EnemyAi::new(99);
    ai.soldier_profile_rank = ProfileRank::Officer;
    assert!(!ai.alert_soldiers(
        Position::default(),
        0,
        &AiGlobalState::default(),
        None,
        &ctx,
        &tick,
        AlertSoldiersFailureContinuation::None,
    ));
    assert!(ai.base.outbox.reentrant.cross_npc_actions.is_empty());
}

#[test]
fn formation_direction_uses_original_aspect_ratio_classifier() {
    assert_eq!(formation_direction(1.0, 1.0), 7);
    assert_eq!(formation_direction(-1.0, 1.0), 9);
    assert_ne!(
        formation_direction(1.0, 1.0),
        crate::position_interface::vector_to_sector_0_to_15_with_aspect(1.0, 1.0, 1.0) as u16,
        "the aspect-1 classifier would start the formation sweep one sector early"
    );
}

#[test]
fn formation_sweep_preserves_raw_cursor_after_sector_wrap() {
    // nicouzouf Save014/r013: the sweep starts at 8 and accepts its
    // fourteenth attempt, projected sector 5. Original retains raw cursor
    // 21 and instructs the soldiers with 21 ^ 8 = 29, not normalized 13.
    let (raw, projected) = formation_sweep_cursor(8, 13);
    assert_eq!(raw, 21);
    assert_eq!(projected, 5);
    assert_eq!(raw ^ 8, 29);
    assert_eq!((raw ^ 8) & 15, 13);
}

#[test]
fn coincident_alerted_soldier_poisons_attack_formation_direction() {
    let officer = Position {
        x: 744.0,
        y: 675.0,
        ..Position::default()
    };
    let alerted = [
        Position {
            x: 683.0,
            y: 713.0,
            ..Position::default()
        },
        officer,
        Position {
            x: 486.0,
            y: 880.0,
            ..Position::default()
        },
    ];

    let average = average_alerted_direction_vector(officer, &alerted);
    assert!(average.0.is_nan() && average.1.is_nan());
    assert_eq!(formation_direction(average.0, average.1), 0);
}

#[test]
fn inactive_duty_soldier_uses_indoor_stay_on_post_answer() {
    assert!(!alert_soldier_stays_on_post(
        false, false, false, true, 0, 0
    ));
    assert!(alert_soldier_stays_on_post(true, false, false, true, 0, 0));
    assert!(!alert_soldier_stays_on_post(true, true, false, true, 0, 0));
}

#[test]
fn drunkenness_precedes_active_outdoor_stay_on_post_branch() {
    let limit = crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT as u8;
    assert!(!alert_soldier_stays_on_post(
        false, false, false, false, 0, limit
    ));
    assert!(alert_soldier_stays_on_post(
        false,
        false,
        false,
        false,
        0,
        limit + 1
    ));
    assert!(alert_soldier_stays_on_post(
        true,
        true,
        false,
        false,
        0,
        limit + 1
    ));
}

#[test]
fn patrol_chief_can_alert_drunk_soldier_despite_stay_on_post_answer() {
    let candidate_position = Position {
        x: 10.0,
        ..Position::default()
    };
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(96, soldier_entity_view(candidate_position));
    let ctx = AiContext {
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    let mut candidate = alert_candidate(96, candidate_position);
    candidate.blood_alcohol = (crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT + 1) as u8;
    let mut tick = AiPerTickData::stub();
    tick.camp_soldiers.push(candidate.clone());

    let mut unrelated_officer = EnemyAi::new(99);
    unrelated_officer.soldier_profile_rank = ProfileRank::Officer;
    assert!(!unrelated_officer.alert_soldiers(
        Position::default(),
        0,
        &AiGlobalState::default(),
        None,
        &ctx,
        &tick,
        AlertSoldiersFailureContinuation::None,
    ));

    tick.camp_soldiers[0].patrol_chief = Some(crate::element::EntityId::Soldier(
        crate::entity_id::SoldierId(99),
    ));
    let mut patrol_chief = EnemyAi::new(99);
    patrol_chief.soldier_profile_rank = ProfileRank::Officer;
    assert!(patrol_chief.alert_soldiers(
        Position::default(),
        0,
        &AiGlobalState::default(),
        None,
        &ctx,
        &tick,
        AlertSoldiersFailureContinuation::None,
    ));
    assert_eq!(
        patrol_chief.base.outbox.reentrant.cross_npc_actions.len(),
        1
    );
}

#[test]
fn attack_point_direction_uses_world_y() {
    let ctx = AiContext {
        position: Position {
            x: 0.0,
            y: 0.0,
            sector: None,
            level: 0,
        },
        elevation: 100.0,
        ..AiContext::test_fixture()
    };
    let target = Position {
        x: 100.0,
        y: 100.0,
        sector: None,
        level: 0,
    };

    // Both ground points have world Y = 100, so the target is due east.
    // Classifying their projected-map delta would incorrectly include a
    // +100 Y component.
    assert_eq!(
        attack_point_direction(&ctx, target),
        vec_to_sector(100.0, 0.0)
    );
    assert_ne!(
        attack_point_direction(&ctx, target),
        vec_to_sector(100.0, 100.0)
    );
}

#[test]
fn alert_officer_distance_uses_world_y_before_isometric_stretch() {
    let owner = Position {
        x: 1173.4828,
        y: 1187.3944,
        sector: None,
        level: 2,
    };
    let officer_47 = Position {
        x: 802.99005,
        y: 1669.0012,
        sector: None,
        level: 0,
    };
    let officer_66 = Position {
        x: 363.0,
        y: 1118.0,
        sector: None,
        level: 0,
    };

    let distance_47 = alert_officer_distance(&officer_47, 0.0, 0, false, &owner, 110.001, 2);
    let distance_66 = alert_officer_distance(&officer_66, 0.0, 0, false, &owner, 110.001, 2);

    assert!(distance_47 < distance_66);
    // Raw map-Y incorrectly reverses this ordering for the Derby control.
    let raw_map_distance_47 = (officer_47.x - owner.x)
        .abs()
        .max((officer_47.y - owner.y).abs() * INVERSE_ASPECT_RATIO);
    let raw_map_distance_66 = (officer_66.x - owner.x)
        .abs()
        .max((officer_66.y - owner.y).abs() * INVERSE_ASPECT_RATIO);
    assert!(raw_map_distance_47 > raw_map_distance_66);
}

#[test]
fn alert_officer_rejects_inactive_default_officer_able_to_help() {
    // nicouzouf Savegame_067 replay-008, frame 1509: Officer55 is
    // inactive inside a building. The original game rejects it as unable to fight,
    // while ability-to-help checking would accept its Default state.
    let is_able_to_help =
        crate::ai_enemy::soldier_is_able_to_help_state(true, AiState::Default, Substate::None);
    assert!(is_able_to_help, "the mismatched legacy predicate admits it");
    assert!(!can_alert_officer(
        ProfileRank::Officer,
        false,
        AiState::Default,
        false,
    ));
    assert!(can_alert_officer(
        ProfileRank::Officer,
        true,
        AiState::Default,
        false,
    ));
}

#[test]
fn alert_officer_scan_includes_reporting_owner_substates() {
    // Nescafe Savegame_001 replay-007, frame 1539: Soldier139 reaches a
    // now-busy officer while already in RUNNING_TO_OFFICER. Original's
    // global camp scan encounters Soldier139 itself and refuses to pick
    // another officer; the owner-omitting Rust snapshot used to select
    // Officer124 and launch an extra movement path instead.
    for substate in [
        Substate::SeekingSoldierCalledByOfficer,
        Substate::SeekingSoldierGoToOfficer,
        Substate::SeekingSoldierGetInstructedByOfficer,
        Substate::SeekingSoldierReturnToOfficer,
        Substate::SeekingSoldierGiveReportToOfficer,
        Substate::SeekingSoldierGiveAlertingReportToOfficerStart,
        Substate::SeekingSoldierGiveAlertingReportToOfficerPoint,
        Substate::SeekingSoldierGiveAlertingReportToOfficerEnd,
        Substate::SeekingGroupCalledByOfficer,
        Substate::SeekingGroupGoToOfficer,
        Substate::SeekingGroupGetInstructedByOfficer,
        Substate::SeekingRunningToOfficer,
        Substate::SeekingRunningToOfficerSeen,
    ] {
        assert!(is_alerting_an_officer(substate), "{substate:?}");
    }
    assert!(!is_alerting_an_officer(Substate::DefaultOnPost));
    assert!(!is_alerting_an_officer(
        Substate::SeekingOfficerWaitForInstructedSoldier
    ));
}

#[test]
fn alert_officer_splices_owner_into_registry_order() {
    // Linux3/Profile003/Save019/r003 frame 18995: reporting Soldier100
    // precedes the evaluating Soldier101. Original observes Soldier100's
    // visibility query and aborts before it ever reaches the owner.
    let without_owner = [100_u32, 102];
    assert_eq!(
        sorted_owner_insertion_index(&without_owner, 101, |handle| *handle),
        1
    );
    assert_eq!(
        sorted_owner_insertion_index(&without_owner, 99, |handle| *handle),
        0
    );
    assert_eq!(
        sorted_owner_insertion_index(&without_owner, 103, |handle| *handle),
        2
    );
}

#[test]
fn alert_soldier_sort_distance_uses_literal_3d_position() {
    let officer = WorldPoint3D::new(884.283, 565.7891, 0.0);
    let soldier_53 = WorldPoint3D::new(1056.0076, 220.9945, 36.001);
    let soldier_54 = WorldPoint3D::new(1191.9963, 250.0275, 0.0);

    let distance_53 = alert_soldier_sort_distance(soldier_53, officer);
    let distance_54 = alert_soldier_sort_distance(soldier_54, officer);
    assert!(
        distance_54 > distance_53,
        "Original's farthest-first list puts ground-level Soldier54 before elevated Soldier53"
    );

    let officer_map = officer.to_map();
    let soldier_53_map = soldier_53.to_map();
    let soldier_54_map = soldier_54.to_map();
    let raw_map_distance_53 = (soldier_53_map.x - officer_map.x).powi(2)
        + ((soldier_53_map.y - officer_map.y) * INVERSE_ASPECT_RATIO).powi(2);
    let raw_map_distance_54 = (soldier_54_map.x - officer_map.x).powi(2)
        + ((soldier_54_map.y - officer_map.y) * INVERSE_ASPECT_RATIO).powi(2);
    assert!(
        raw_map_distance_53 > raw_map_distance_54,
        "the old projected-2D shortcut must demonstrate the representative reversal"
    );
}

#[test]
fn alert_soldier_sort_is_farthest_first_and_later_first_on_ties() {
    let mut alerted = [(51, 25.0, 0), (52, 100.0, 1), (53, 25.0, 2)];

    sort_alerted_soldiers(&mut alerted);

    assert_eq!(
        alerted.map(|(handle, _, _)| handle),
        [52, 53, 51],
        "Original inserts farther soldiers first and equal-distance newcomers before older entries"
    );
}

#[test]
fn tower_guard_runner_uses_registry_prefix_and_last_qualifier() {
    let position = |x| WorldPoint3D { x, y: 0.0, z: 0.0 };
    let registry = [
        (10, position(50.0)),
        (11, position(5.0)),
        (12, position(9.0)),
        (13, position(1.0)),
    ];

    assert_eq!(
        tower_guard_runner_from_registry_prefix(registry, 3, position(0.0), 100),
        Some(12),
        "Original ignores the closest actor outside the first N registry entries and keeps the last qualifying prefix actor"
    );
}

#[test]
fn tower_guard_runner_prefix_restores_owner_at_registry_position() {
    let position = |x| WorldPoint3D { x, y: 0.0, z: 0.0 };
    let registry_without_owner = [
        (105, position(20.0)),
        (108, position(5.0)),
        (109, position(4.0)),
    ];
    let complete = tower_guard_complete_registry(registry_without_owner, (106, position(10.0)));

    assert_eq!(
        tower_guard_runner_from_registry_prefix(complete, 2, position(0.0), 100),
        None,
        "the tower guard occupies its Original registry slot, so the later Rust-only runner must remain outside the first-N prefix"
    );
}

#[test]
fn tower_guard_runner_truncates_squared_distances_before_comparing() {
    let position = |x| WorldPoint3D { x, y: 0.0, z: 0.0 };
    let officer_distance = tower_guard_square_distance(position(0.0), position(10.04));

    assert_eq!(officer_distance, 100);
    assert_eq!(
        tower_guard_runner_from_registry_prefix(
            [(7, position(10.02))],
            1,
            position(0.0),
            officer_distance,
        ),
        None,
        "100.4004 and 100.8016 both truncate to unsigned integer 100, so strict less-than must reject the candidate"
    );
}
