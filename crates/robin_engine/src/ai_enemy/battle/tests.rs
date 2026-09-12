use super::*;

#[test]
fn lost_pc_overview_uses_handle_keyed_forecast_instead_of_stale_seek_position() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(66);
    ai.missed_pc = Some(AiEntityHandle::new(126));
    ai.pc_gone_away_in_this_direction = 7;
    ai.base.seek_position = Position {
        x: 1810.0,
        y: 1155.0,
        sector: crate::position_interface::SectorHandle::new(78),
        level: 2,
    };
    let forecast = Position {
        x: 1759.0,
        y: 1033.0,
        sector: crate::position_interface::SectorHandle::new(75),
        level: 4,
    };
    let mut tick = AiPerTickData::stub();
    tick.enemy_detectable_forecasts.push((
        126,
        crate::ai::PreparedForecastDestination::fixed(forecast, 8),
    ));

    ai.refresh_missed_pc_forecast(&sim, &tick);

    assert_eq!(ai.base.seek_position, forecast);
    assert_eq!(ai.pc_gone_away_in_this_direction, 8);
}

#[test]
#[should_panic(expected = "lost-PC overview target 126 has no prepared destination forecast")]
fn lost_pc_overview_never_falls_back_to_stale_seek_position() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(66);
    ai.missed_pc = Some(AiEntityHandle::new(126));
    ai.base.seek_position = Position {
        x: 1810.0,
        y: 1155.0,
        sector: crate::position_interface::SectorHandle::new(78),
        level: 2,
    };

    ai.refresh_missed_pc_forecast(&sim, &AiPerTickData::stub());
}

#[test]
fn begin_swordfight_publishes_reciprocal_combat_neighbour_clears() {
    // Beginning an enemy swordfight clears the formation with
    // clearing both combat neighbours. Dropping only the local
    // pointers leaves stale back-pointers that a later phalanx insertion
    // can follow and use to detach a live chain.
    let mut ai = EnemyAi::new(132);
    ai.base.primary_target = Some(AiEntityHandle::new(343));
    ai.left_combat_neighbour = Some(AiEntityHandle::new(131));
    ai.right_combat_neighbour = Some(AiEntityHandle::new(133));

    ai.begin_swordfight(&AiContext::test_fixture(), &AiPerTickData::stub());

    assert_eq!(ai.left_combat_neighbour, None);
    assert_eq!(ai.right_combat_neighbour, None);
    assert!(matches!(
        ai.base.outbox.reentrant.cross_npc_actions.as_slice(),
        [
            CrossNpcAction::SetRightCombatNeighbour {
                target: 131,
                neighbour: None,
            },
            CrossNpcAction::SetLeftCombatNeighbour {
                target: 133,
                neighbour: None,
            },
        ]
    ));
    assert!(
        ai.base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .any(|work| matches!(work, AiOwnerWork::NearbyCiviliansPanic))
    );
    assert!(
        !ai.base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .any(|work| matches!(work, AiOwnerWork::NearbyCiviliansPanic180))
    );
}

#[test]
fn begin_swordfight_retains_approach_jump_line_when_live_probe_is_none() {
    // Enemy-approach reconsideration stores the chosen
    // line in the jump-line reference. Swordfight entry later copies that state to
    // jump-line destination even if the victim has moved far enough
    // that a fresh table-swordfight eligibility probe would now return null.
    let mut ai = EnemyAi::new(222);
    ai.base.primary_target = Some(AiEntityHandle::new(317));
    ai.my_line_jump = Some(19);
    let mut tick = AiPerTickData::stub();
    tick.primary_target_jump_line = None;

    ai.begin_swordfight(&AiContext::test_fixture(), &tick);

    assert_eq!(ai.my_line_jump, Some(19));
    let staged = ai
        .base
        .outbox
        .reentrant
        .owner_work
        .iter()
        .find_map(|work| match work {
            AiOwnerWork::StateChange(notification) => {
                notification.actor_effects_before_callback.as_ref()
            }
            _ => None,
        })
        .expect("swordfight-entry actor effects must precede its state-change callback");
    assert_eq!(staged.enter_swordfight_jump_line, Some(19));
}

/// nicouzouf Savegame_047 replay-004, frame 563: Soldier51 (a rider in
/// AttackingReactiontimeRunning) plans a charge approach against Pc76.
/// Inputs captured bit-exact from the parity replay. The Original's
/// `operator*=` sites round `RIDER_CHARGE_LATERAL_DISTANCE / fCosAlpha`
/// and `fCosAlpha * fMeToEnemyNorm` once before the component
/// multiplies; the per-component `n * 40.0 / cos` order previously
/// produced goal.y = 0x4425e9c8 (one ULP low), which propagated
/// through the stop-transition splice into the running order's goal,
/// its normalized increment, and the frame-564 movement_map drift.
#[test]
fn rider_charge_goal_matches_original_scalar_rounding() {
    let me = (f32::from_bits(0x448f_3c66), f32::from_bits(0x43dc_a7ea));
    let enemy = (f32::from_bits(0x443a_7ea7), f32::from_bits(0x4418_a6d2));

    let geometry = match rider_charge_goal_geometry(me, 11, enemy) {
        Ok(geometry) => geometry,
        Err(_) => panic!("frame-563 fixture must produce a charge goal"),
    };

    assert_eq!(geometry.goal.0.to_bits(), 0x442f_2b23);
    assert_eq!(geometry.goal.1.to_bits(), 0x4425_e9c9);
    // The strike-zone / begin-charge inputs the caller consumes.
    assert_eq!(geometry.me_to_hit.0.to_bits(), 0xc3ba_d233);
    assert_eq!(geometry.hit_norm_len.to_bits(), 0x43d0_d28f);
}

#[test]
fn rider_charge_friend_corridor_uses_full_fighter_registry() {
    // nicouzouf Profile_001 Savegame_047 replay-005, frame 1433:
    // Soldier62 is outside Soldier51's nearby-fighter window but stands
    // inside the strike corridor toward PC76. Original's global
    // Fighter enumeration rejects this charge.
    let mut ai = EnemyAi::new(51);
    ai.base.primary_target = Some(AiEntityHandle::new(76));
    ai.list_them = vec![76];
    let rider = Position {
        x: f32::from_bits(0x44c7_1d8e),
        y: f32::from_bits(0x4421_e39e),
        ..Position::default()
    };
    let target = FighterSnapshot {
        handle: 76,
        position: Position {
            x: f32::from_bits(0x4474_03e3),
            y: f32::from_bits(0x43bb_a89f),
            ..Position::default()
        },
        raw_position: Position {
            x: f32::from_bits(0x4474_03e3),
            y: f32::from_bits(0x43bb_a89f),
            ..Position::default()
        },
        is_able_to_fight: true,
        is_pc: true,
        ..FighterSnapshot::default()
    };
    let blocking_friend = FighterSnapshot {
        handle: 62,
        position: Position {
            x: f32::from_bits(0x447b_182c),
            y: f32::from_bits(0x43b8_eb5e),
            ..Position::default()
        },
        raw_position: Position {
            x: f32::from_bits(0x447b_182c),
            y: f32::from_bits(0x43b8_eb5e),
            ..Position::default()
        },
        is_friendly: true,
        is_soldier: true,
        ..FighterSnapshot::default()
    };
    let mut tick = AiPerTickData::stub();
    tick.nearby_fighters = vec![target.clone()];
    tick.fighter_registry = vec![target, blocking_friend];
    let ctx = AiContext {
        self_is_rider: true,
        position: rider,
        direction: 11,
        ..AiContext::test_fixture()
    };

    assert!(!ai.maybe_make_rider_attack(&ctx, &tick, None));
    assert_eq!(ai.base.primary_target, Some(AiEntityHandle::new(76)));
    assert!(ai.base.outbox.actor.orders.is_empty());
}

#[test]
fn rider_charge_retains_out_of_range_target_position_for_return_face() {
    // Original-game rider-attack setup stores the primary target's position
    // after selecting the charge.  A raw target pointer remains valid
    // outside Rust's radius-limited nearby-fighter snapshot; the later
    // GettingDistance reach-point handler faces this stored position.
    let mut ai = EnemyAi::new(51);
    ai.base.primary_target = Some(AiEntityHandle::new(76));
    ai.base.seek_position = Position {
        x: 900.0,
        y: 700.0,
        ..Position::default()
    };
    let target_position = Position {
        x: 0.0,
        y: -200.0,
        ..Position::default()
    };
    let target = FighterSnapshot {
        handle: 76,
        position: target_position,
        raw_position: target_position,
        is_able_to_fight: true,
        is_pc: true,
        ..FighterSnapshot::default()
    };
    let mut tick = AiPerTickData::stub();
    tick.nearby_fighters.clear();
    tick.fighter_registry = vec![target];
    let ctx = AiContext {
        self_is_rider: true,
        position: Position::default(),
        direction: 0,
        ..AiContext::test_fixture()
    };

    assert!(ai.maybe_make_rider_attack(&ctx, &tick, None));
    assert_eq!(ai.base.primary_target, Some(AiEntityHandle::new(76)));
    assert_eq!(ai.base.seek_position, target_position);
}

#[test]
fn rider_charge_geometry_uses_raw_target_position_during_door_transit() {
    // `Position(target)` substitutes the active door endpoint, while
    // Rider-attack destination selection reads the target's map position. Put
    // those points on opposite sides of the rider so the accessor choice
    // is observable without reproducing the shipped door grid.
    let mut ai = EnemyAi::new(51);
    ai.base.primary_target = Some(AiEntityHandle::new(76));
    ai.list_them = vec![76];
    let target = FighterSnapshot {
        handle: 76,
        position: Position {
            y: -50.0,
            level: 1,
            ..Position::default()
        },
        raw_position: Position {
            y: 50.0,
            ..Position::default()
        },
        is_able_to_fight: true,
        is_pc: true,
        ..FighterSnapshot::default()
    };
    let mut tick = AiPerTickData::stub();
    tick.nearby_fighters = vec![target.clone()];
    tick.fighter_registry = vec![target];
    let ctx = AiContext {
        self_is_rider: true,
        direction: 0,
        ..AiContext::test_fixture()
    };
    let initial_state = (ai.base.current_state, ai.base.current_substate);

    assert!(!ai.maybe_make_rider_attack(&ctx, &tick, None));
    assert_eq!(
        (ai.base.current_state, ai.base.current_substate),
        initial_state
    );
    assert!(ai.base.outbox.actor.orders.is_empty());
}

fn pc_view() -> crate::ai_entity_view::AiEntityView {
    let entity = crate::element::Entity::Pc(crate::element::ActorPc {
        element: {
            let mut initial_element =
                crate::element::ElementData::from_initial_posture(crate::element::Posture::Upright);
            initial_element.kind = crate::element::ElementKind::ActorPc;
            initial_element
        },
        actor: Default::default(),
        human: Default::default(),
        pc: crate::element::PcData {
            life_points: 100,
            ..Default::default()
        },
    });
    crate::ai_entity_view::entity_view_from_entity(
        &entity,
        41,
        false,
        None,
        None,
        crate::order::OrderType::NonanimationEnd,
    )
}

fn pc_view_at(position: Position) -> crate::ai_entity_view::AiEntityView {
    let mut view = pc_view();
    view.position = position;
    view.detection_position = crate::coordinates::MapPoint::new(position.x, position.y);
    view.detection_position_world =
        crate::coordinates::WorldPoint3D::new(position.x, position.y, 0.0);
    view
}

fn add_owner_sword_range(tick: &mut AiPerTickData, owner: u32, range: u16) {
    tick.fighter_registry.push(FighterSnapshot {
        handle: owner,
        sword_range_default: range,
        ..FighterSnapshot::default()
    });
}

#[test]
fn attack_enemy_prefers_matching_position_snapshot_over_fighter_geometry() {
    // Enemy attack handling writes
    // seek target becomes the enemy planning position. The target-specific tick
    // field is that source read; nearby-fighter geometry can represent an
    // older owner boundary and must not replace it for the same handle.
    let mut ai = EnemyAi::new(104);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontime;

    let authoritative = Position {
        x: 1578.1302,
        y: 1894.2336,
        sector: crate::position_interface::SectorHandle::new(119),
        level: 8,
    };
    let stale_fighter_position = Position {
        x: 1524.8396,
        y: 1704.1648,
        sector: crate::position_interface::SectorHandle::new(0),
        level: 0,
    };
    let mut tick = AiPerTickData::stub();
    tick.primary_target_snapshot_handle = Some(AiEntityHandle::new(252));
    tick.primary_target_position = Some(authoritative);
    tick.nearby_fighters.push(FighterSnapshot {
        handle: 252,
        position: stale_fighter_position,
        ..FighterSnapshot::default()
    });
    // Force enemy approach reconsideration's already-fighting early return so
    // the assertion observes the enemy-attack assignment directly.
    let ctx = AiContext {
        is_swordfighting: true,
        ..AiContext::test_fixture()
    };

    ai.attack_enemy(252, &ctx, &tick, None);

    assert_eq!(ai.base.primary_target, Some(AiEntityHandle::new(252)));
    assert_eq!(ai.base.seek_position, authoritative);
}

#[test]
fn attack_enemy_retarget_uses_live_exact_sector_over_number_only_fighter_snapshot() {
    // Continue/replay-026, frame 7896: battle planning initially
    // snapshots PC282, then an attacking friend contributes PC137. Both
    // target positions use public sector 88, but only the live Position()
    // carries the arena object needed to find gates 111 and 114.
    let mut ai = EnemyAi::new(153);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontimeRunning;

    let public = crate::position_interface::SectorHandle::new(88).unwrap();
    let exact = public.with_arena_index(crate::fast_find_grid::SectorIndex::new(137).unwrap());
    let live_target = Position {
        x: 684.1841,
        y: 1545.0576,
        sector: Some(exact),
        level: 2,
    };
    let number_only = Position {
        sector: Some(public),
        ..live_target
    };

    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(137, pc_view_at(live_target));
    let ctx = AiContext {
        is_swordfighting: true,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.primary_target_snapshot_handle = Some(AiEntityHandle::new(282));
    tick.primary_target_position = Some(Position {
        x: 900.0,
        y: 1800.0,
        sector: crate::position_interface::SectorHandle::new(0),
        level: 0,
    });
    tick.nearby_fighters.push(FighterSnapshot {
        handle: 137,
        position: number_only,
        ..FighterSnapshot::default()
    });

    ai.attack_enemy(137, &ctx, &tick, None);

    assert_eq!(ai.base.primary_target, Some(AiEntityHandle::new(137)));
    assert_eq!(ai.base.seek_position, live_target);
    assert_eq!(
        ai.base.seek_position.sector.unwrap().arena_index(),
        Some(crate::fast_find_grid::SectorIndex::new(137).unwrap())
    );
}

fn reconsider_approach_lift_grid() -> crate::fast_find_grid::FastFindGrid {
    let mut grid = crate::fast_find_grid::FastFindGrid::new();
    let lift_number = crate::sector::SectorNumber::new(42);
    let ordinary_number = crate::sector::SectorNumber::new(5);
    let level = std::sync::Arc::make_mut(&mut grid.level);
    level.sector_number_map.insert(lift_number, 0);
    level.sector_number_map.insert(ordinary_number, 1);
    level.door_projection_infos = vec![
        crate::fast_find_grid::DoorProjectionInfo {
            point_out: crate::coordinates::MapPoint::new(410.0, 120.0),
            sector_out: crate::sector::SectorNumber::new(7),
            layer_out: 3,
            ..Default::default()
        },
        crate::fast_find_grid::DoorProjectionInfo {
            point_out: crate::coordinates::MapPoint::new(430.0, 300.0),
            sector_out: ordinary_number,
            layer_out: 0,
            ..Default::default()
        },
    ];
    let sector =
        |sector_number, sector_type, lift_type, gate_indices| crate::fast_find_grid::GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type,
            layer: 0,
            sector_number,
            door_index: None,
            lift_type,
            lift_direction: 0,
            force_crouched: false,
            building_index: None,
            low_exit_point: None,
            high_exit_point: None,
            lowest_door_index: None,
            jump_line_indices: Vec::new(),
            gate_indices,
            underlying_sector: None,
        };
    level.sectors.push(sector(
        lift_number,
        crate::sector::SectorType::LIFT,
        Some(crate::sector::LiftType::Ladder),
        vec![
            crate::gate::DoorIndex::new(0).expect("valid door index"),
            crate::gate::DoorIndex::new(1).expect("valid door index"),
        ],
    ));
    level.sectors.push(sector(
        ordinary_number,
        crate::sector::SectorType::AREA | crate::sector::SectorType::MOTION,
        None,
        Vec::new(),
    ));
    grid
}

#[test]
fn failed_look_for_help_route_is_consumed_before_event_fallback() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(105);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingRunningToOfficer;
    ai.base.couldnt_reachpoint = true;
    ai.base.primary_target = Some(AiEntityHandle::new(252));
    ai.list_them = vec![252];

    let threat = Position {
        x: 1050.0,
        y: 1780.0,
        ..Position::default()
    };
    let stale_threat = Position {
        x: 2145.0,
        y: 1976.0,
        ..Position::default()
    };
    let mut tick = AiPerTickData::stub();
    tick.fighter_registry = vec![FighterSnapshot {
        handle: 252,
        position: stale_threat,
        is_pc: true,
        is_able_to_fight: true,
        ..FighterSnapshot::default()
    }];
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    let target_view = pc_view_at(threat);
    views.insert(252, target_view);
    views.insert(105, pc_view_at(Position::default()));
    let ctx = AiContext {
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut global = AiGlobalState::default();

    let (_, draws) = crate::sim_rng::with_draw_trace(|| {
        ai.resume_battle_look_for_help_after_alert_officer(&sim, &mut global, &ctx, &tick);
    });

    assert_eq!(draws, vec![crate::sim_rng::RngSite::BattlePanicRemark]);
    assert!(!ai.base.couldnt_reachpoint);
    assert_eq!(ai.base.current_state, AiState::Fleeing);
    assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
    assert!(ai.my_seek_points.is_empty());
    let panic = ai
        .base
        .outbox
        .actor
        .begin_panic
        .expect("failed LookForHelp must continue through Cassos Panic");
    assert_eq!(panic.center, Some(threat));
    let log = ai.base.ai_log.last().expect("Cassos decision log");
    assert_eq!(log.line_type, LogLineType::BattleDecision);
    assert_eq!(log.info, Decision::Cassos as u16);
}

#[test]
fn cassos_uses_live_target_position_instead_of_stale_fighter_or_seek_position() {
    let mut ai = EnemyAi::new(117);
    ai.base.seek_position = Position {
        x: 2145.0,
        y: 1976.0,
        ..Position::default()
    };
    let live = Position {
        x: 873.0,
        y: 1717.0,
        sector: crate::position_interface::SectorHandle::new(309),
        level: 8,
    };
    let stale = Position {
        x: 900.0,
        y: 1800.0,
        ..Position::default()
    };
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(252, pc_view_at(live));
    let ctx = AiContext {
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.nearby_fighters.push(FighterSnapshot {
        handle: 252,
        position: stale,
        ..FighterSnapshot::default()
    });

    ai.begin_cassos_panic(252, &ctx, &tick);

    let panic = ai
        .base
        .outbox
        .actor
        .begin_panic
        .expect("directed Cassos must stage Panic");
    assert_eq!(panic.center, Some(live));
    assert!(ai.base.directed_panic);
}

#[test]
fn cassos_without_a_selected_target_uses_undirected_panic() {
    let mut ai = EnemyAi::new(117);
    ai.base.current_state = AiState::Fleeing;
    ai.base.current_substate = Substate::FleeingPanic;
    ai.base.lasting_panic_runs = 11;
    ai.base.seek_position = Position {
        x: 2145.0,
        y: 1976.0,
        ..Position::default()
    };

    ai.begin_cassos_panic(0, &AiContext::test_fixture(), &AiPerTickData::stub());

    let panic = ai
        .base
        .outbox
        .actor
        .begin_panic
        .expect("undirected Cassos must stage Panic");
    assert_eq!(panic.center, None);
    assert!(!ai.base.directed_panic);
    assert_eq!(panic.runs, parameters_ai::AI_STANDARD_PANIC_RUNS as u8);
    assert_eq!(
        ai.base.lasting_panic_runs, 11,
        "the engine drain owns repeated-panic upgrade semantics"
    );
}

#[test]
#[should_panic(expected = "required entity view for handle 252 missing")]
fn cassos_does_not_replace_a_missing_live_target_with_fighter_or_seek_position() {
    let mut ai = EnemyAi::new(117);
    ai.base.seek_position = Position {
        x: 2145.0,
        y: 1976.0,
        ..Position::default()
    };
    let mut tick = AiPerTickData::stub();
    tick.nearby_fighters.push(FighterSnapshot {
        handle: 252,
        position: Position {
            x: 900.0,
            y: 1800.0,
            ..Position::default()
        },
        ..FighterSnapshot::default()
    });

    ai.begin_cassos_panic(252, &AiContext::test_fixture(), &tick);
}

#[test]
fn successful_look_for_help_continuation_draws_remark_and_logs_once() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(105);
    let mut global = AiGlobalState::default();

    let (_, draws) = crate::sim_rng::with_draw_trace(|| {
        ai.resume_battle_look_for_help_after_alert_officer(
            &sim,
            &mut global,
            &AiContext::test_fixture(),
            &AiPerTickData::stub(),
        );
    });

    assert_eq!(draws, vec![crate::sim_rng::RngSite::BattlePanicRemark]);
    assert_eq!(ai.base.ai_log.len(), 1);
    let log = ai.base.ai_log.last().expect("LookForHelp decision log");
    assert_eq!(log.line_type, LogLineType::BattleDecision);
    assert_eq!(log.info, Decision::LookForHelp as u16);
}

#[test]
fn reconsider_approach_uses_raw_truncated_map_distance() {
    let soldier = Position {
        x: 655.007_8,
        y: 1744.445,
        ..Position::default()
    };
    let target = Position {
        x: 585.0,
        y: 1726.0,
        ..Position::default()
    };

    assert_eq!(reconsider_approach_distance(soldier, target), 72.0);
    let dx = soldier.x - target.x;
    let dy = (soldier.y - target.y) * INVERSE_ASPECT_RATIO;
    assert!(
        (dx * dx + dy * dy).sqrt() > 75.0,
        "the general aspect-corrected distance would miss this swordfight boundary"
    );
}

#[test]
fn battle_friend_visibility_is_evaluated_at_the_decision_call_site() {
    let mut target = pc_view();
    target.position.x = 100.0;
    target.detection_position_world.x = 100.0;
    let ctx = AiContext {
        self_view_radius: 500,
        self_is_active: true,
        ..AiContext::test_fixture()
    };

    assert!(battle_friend_detected_360(
        &ctx,
        1,
        2,
        target.detection_position_world,
        target.direction,
        &target,
    ));

    target.in_building = true;
    assert!(!battle_friend_detected_360(
        &ctx,
        1,
        2,
        target.detection_position_world,
        target.direction,
        &target,
    ));

    target.in_building = false;
    target.active = false;
    assert!(!battle_friend_detected_360(
        &ctx,
        1,
        2,
        target.detection_position_world,
        target.direction,
        &target,
    ));

    target.active = true;
    let inactive_owner = AiContext {
        self_is_active: false,
        ..ctx
    };
    assert!(!battle_friend_detected_360(
        &inactive_owner,
        1,
        2,
        target.detection_position_world,
        target.direction,
        &target,
    ));
}

#[test]
fn battle_fighter_scan_preserves_interleaved_registry_order() {
    let fighter = |handle, is_pc, is_friendly, is_able_to_fight| FighterSnapshot {
        handle,
        is_pc,
        is_soldier: !is_pc,
        is_friendly,
        is_able_to_fight,
        ..FighterSnapshot::default()
    };
    let registry = vec![
        fighter(54, false, true, true),
        fighter(47, false, true, true),
        fighter(167, true, true, true),
        fighter(48, false, true, true),
        fighter(36, true, false, true),
        fighter(49, false, true, false),
    ];

    assert_eq!(
        battle_fighter_candidates(&registry, 54)
            .map(|candidate| candidate.handle)
            .collect::<Vec<_>>(),
        vec![47, 167, 48],
    );
}

#[test]
fn out_of_view_alerting_soldier_does_not_suppress_officer_alert() {
    assert!(!has_nearby_alerting_soldier(
        65,
        &[65],
        [(64, Substate::SeekingRunningToOfficer)],
    ));
}

#[test]
fn admitted_alerting_soldier_suppresses_duplicate_officer_alert() {
    assert!(has_nearby_alerting_soldier(
        65,
        &[65, 64],
        [(64, Substate::SeekingRunningToOfficer)],
    ));
    assert!(!has_nearby_alerting_soldier(
        65,
        &[65, 64],
        [(64, Substate::DefaultOnPost)],
    ));
}

#[test]
fn rider_charge_approach_focuses_target_for_immediate_visibility_refresh() {
    let mut ai = EnemyAi::new(51);
    ai.base.primary_target = Some(AiEntityHandle::new(76));

    let ctx = AiContext {
        self_is_rider: true,
        position: Position::default(),
        direction: 0,
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.nearby_fighters = vec![FighterSnapshot {
        handle: 76,
        position: Position {
            y: -200.0,
            ..Position::default()
        },
        raw_position: Position {
            y: -200.0,
            ..Position::default()
        },
        is_able_to_fight: true,
        is_pc: true,
        ..FighterSnapshot::default()
    }];

    assert!(ai.maybe_make_rider_attack(&ctx, &tick, None));

    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingRiderChargingApproaching
    );
    let focus = ai.base.outbox.actor.focus.or_else(|| {
        ai.base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .find_map(|work| match work {
                crate::ai::AiOwnerWork::StateChange(notification) => notification
                    .actor_effects_before_callback
                    .as_ref()
                    .and_then(|effects| effects.focus),
                _ => None,
            })
    });
    assert_eq!(focus, Some(AiEntityHandle::new(76)));
}

#[test]
fn immediate_rider_charge_replaces_target_focus_with_unfocus() {
    let mut ai = EnemyAi::new(51);
    ai.base.primary_target = Some(AiEntityHandle::new(76));

    let ctx = AiContext {
        self_is_rider: true,
        position: Position::default(),
        direction: 0,
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.nearby_fighters = vec![FighterSnapshot {
        handle: 76,
        position: Position {
            y: -50.0,
            ..Position::default()
        },
        raw_position: Position {
            y: -50.0,
            ..Position::default()
        },
        is_able_to_fight: true,
        is_pc: true,
        ..FighterSnapshot::default()
    }];

    assert!(ai.maybe_make_rider_attack(&ctx, &tick, None));

    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingRiderChargingPassing
    );
    assert_eq!(ai.base.outbox.actor.focus, None);
    let unfocus = ai.base.outbox.actor.unfocus
        || ai
            .base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .any(|work| match work {
                crate::ai::AiOwnerWork::StateChange(notification) => notification
                    .actor_effects_before_callback
                    .as_ref()
                    .is_some_and(|effects| effects.unfocus),
                _ => false,
            });
    assert!(unfocus);
}

#[test]
fn rider_charge_trusts_persistent_primary_over_transient_camp_classification() {
    let mut ai = EnemyAi::new(51);
    ai.base.primary_target = Some(AiEntityHandle::new(76));

    let ctx = AiContext {
        self_is_rider: true,
        position: Position::default(),
        direction: 0,
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.nearby_fighters = vec![FighterSnapshot {
        handle: 76,
        position: Position {
            y: -50.0,
            ..Position::default()
        },
        raw_position: Position {
            y: -50.0,
            ..Position::default()
        },
        is_able_to_fight: false,
        is_friendly: true,
        ..FighterSnapshot::default()
    }];

    assert!(ai.maybe_make_rider_attack(&ctx, &tick, None));
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingRiderChargingPassing
    );
}

#[test]
fn sleeping_enemy_visibility_is_evaluated_only_by_the_fallback() {
    let mut target = pc_view();
    target.position.x = 100.0;
    target.is_unconscious = true;
    let candidate = SleepingEnemyInfo {
        handle: 198,
        position: target.position,
        is_pc: true,
        is_robin: false,
        is_vip: false,
    };
    let ctx = AiContext {
        self_view_radius: 500,
        ..AiContext::test_fixture()
    };

    assert!(sleeping_enemy_detected_360(&ctx, &candidate, &target));
    target.in_building = true;
    assert!(!sleeping_enemy_detected_360(&ctx, &candidate, &target));
}

#[test]
fn trainer_sleeping_enemy_scan_waits_for_return_to_duty_continuation() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(91);
    ai.combat_trainer = true;
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingBowObserving;

    let mut target = pc_view_at(Position {
        x: 100.0,
        ..Position::default()
    });
    target.is_unconscious = true;
    let mut owner = pc_view();
    owner.is_pc = false;
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(91, owner);
    views.insert(198, target.clone());
    let ctx = AiContext {
        self_view_radius: 500,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.nearby_sleeping_enemies = vec![SleepingEnemyInfo {
        handle: 198,
        position: target.position,
        is_pc: true,
        is_robin: false,
        is_vip: false,
    }];

    ai.kill_nearby_sleeping_enemies(&sim, &ctx, &tick);

    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingBowObserving,
        "the sleeping scan must not run before ReturnToDuty completes"
    );
    assert!(matches!(
        ai.base.outbox.reentrant.owner_work.as_slice(),
        [
            crate::ai::AiOwnerWork::ResumeReturnToDutyAfterPatrolInit { .. },
            crate::ai::AiOwnerWork::ResumeKillNearbySleepingEnemiesAfterReturnToDuty,
        ]
    ));

    ai.resume_kill_nearby_sleeping_enemies_after_return_to_duty(&sim, &ctx, &tick);
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingApproachingSleepingEnemy
    );
    assert_eq!(ai.base.primary_target, Some(AiEntityHandle::new(198)));
}

fn sleeping_target_case(
    first: Position,
    second: Position,
    expected: HumanHandle,
) -> (EnemyAi, AiContext) {
    let mut first_view = pc_view_at(first);
    first_view.elevation = 0.0;
    first_view.is_unconscious = true;
    let mut second_view = pc_view_at(second);
    second_view.elevation = 0.0;
    second_view.is_unconscious = true;
    let owner_position = Position {
        x: 1377.2015,
        y: 252.88869,
        sector: crate::position_interface::SectorHandle::new(14),
        ..Position::default()
    };
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(346, first_view);
    views.insert(345, second_view);
    views.insert(139, pc_view_at(owner_position));
    let ctx = AiContext {
        position: owner_position,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let targets = [
        SleepingEnemyInfo {
            handle: 346,
            position: first,
            is_pc: true,
            is_robin: false,
            is_vip: false,
        },
        SleepingEnemyInfo {
            handle: 345,
            position: second,
            is_pc: true,
            is_robin: false,
            is_vip: false,
        },
    ];
    let mut ai = EnemyAi::new(139);
    ai.approach_sleeping_enemies(
        &crate::sim_rng::test_context(),
        &targets,
        &ctx,
        &AiPerTickData::stub(),
    );

    assert_eq!(ai.base.primary_target, Some(AiEntityHandle::new(expected)));
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingApproachingSleepingEnemy
    );
    let order = ai
        .base
        .outbox
        .actor
        .orders
        .last()
        .expect("sleeping target selection must queue approach movement");
    let target = ctx.entity_view(expected).unwrap().position;
    assert_eq!((order.target_x, order.target_y), (target.x, target.y));
    (ai, ctx)
}

#[test]
fn sleeping_enemy_selection_uses_isometric_get_position_distance() {
    let raw_nearer = Position {
        x: 1394.2125,
        y: 328.31696,
        sector: crate::position_interface::SectorHandle::new(14),
        ..Position::default()
    };
    let isometric_nearer = Position {
        x: 1417.7587,
        y: 185.4791,
        sector: crate::position_interface::SectorHandle::new(14),
        ..Position::default()
    };

    let owner = Position {
        x: 1377.2015,
        y: 252.88869,
        ..Position::default()
    };
    let raw_sq = |target: Position| {
        let dx = target.x - owner.x;
        let dy = target.y - owner.y;
        dx * dx + dy * dy
    };
    let isometric_sq = |target: Position| {
        let dx = target.x - owner.x;
        let dy = (target.y - owner.y) * INVERSE_ASPECT_RATIO;
        dx * dx + dy * dy
    };
    assert!(raw_sq(raw_nearer) < raw_sq(isometric_nearer));
    assert!(isometric_sq(isometric_nearer) < isometric_sq(raw_nearer));

    let _ = sleeping_target_case(raw_nearer, isometric_nearer, 345);
}

#[test]
fn sleeping_enemy_selection_keeps_ordinary_nearest_target() {
    let nearest = Position {
        x: 1417.0,
        y: 250.0,
        sector: crate::position_interface::SectorHandle::new(14),
        ..Position::default()
    };
    let farther = Position {
        x: 1500.0,
        y: 400.0,
        sector: crate::position_interface::SectorHandle::new(14),
        ..Position::default()
    };

    let _ = sleeping_target_case(nearest, farther, 346);
}

#[test]
fn reconsider_approach_resolves_position_after_synchronous_retarget() {
    let mut ai = EnemyAi::new(110);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontime;
    ai.base.primary_target = Some(AiEntityHandle::new(91));
    ai.sword_range = 50;

    let target_position = Position {
        x: 695.0,
        y: 2073.0,
        ..Position::default()
    };
    let mut target_view = pc_view();
    target_view.position = target_position;
    target_view.forecasted_destination =
        crate::ai::PreparedForecastDestination::fixed(target_position, 0);
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(91, target_view);
    let ctx = AiContext {
        position: Position {
            x: 698.0,
            y: 2119.0,
            ..Position::default()
        },
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    let mut tick = AiPerTickData::stub();
    add_owner_sword_range(&mut tick, 110, 50);
    tick.primary_target_snapshot_handle = Some(AiEntityHandle::new(58));
    tick.primary_target_position = Some(Position {
        x: 712.0,
        y: 2053.0,
        ..Position::default()
    });

    ai.reconsider_enemy_approach(false, &ctx, &tick, None);

    // begin_swordfight raises Engage before its state change suspends the
    // actor-outbox prefix into the queued state-change owner work; the
    // engine reapplies that prefix when it drains the callback. Read the
    // request from either place.
    let engage = ai.base.outbox.actor.enter_swordfight.or_else(|| {
        ai.base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .find_map(|work| match work {
                crate::ai::AiOwnerWork::StateChange(notification) => notification
                    .actor_effects_before_callback
                    .as_ref()
                    .and_then(|effects| effects.enter_swordfight),
                _ => None,
            })
    });
    assert_eq!(
        engage,
        Some(EnterSwordfightRequest::Engage(AiEntityHandle::new(91)))
    );
    assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
}

#[test]
fn reconsider_approach_uses_selected_door_lift_after_synchronous_retarget() {
    let mut ai = EnemyAi::new(84);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingQuittingSwordfight;
    ai.base.primary_target = Some(AiEntityHandle::new(173));
    ai.sword_range = 50;

    let committed_lift_position = Position {
        x: 500.0,
        y: 500.0,
        sector: crate::position_interface::SectorHandle::new(42),
        level: 0,
    };
    let mut replacement_view = pc_view();
    // A selected PassDoor makes AI Position(target) report the committed
    // endpoint while the world position remains at the interpolated body.
    // Poison the latter so this test cannot pass through raw geometry.
    replacement_view.position = committed_lift_position;
    replacement_view.detection_position = crate::coordinates::MapPoint::new(70.0, 80.0);
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(173, replacement_view);
    let ctx = AiContext {
        position: Position {
            x: 0.0,
            y: 0.0,
            level: 3,
            ..Position::default()
        },
        frame: 1058,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        fast_grid: std::sync::Arc::new(reconsider_approach_lift_grid()),
        ..AiContext::test_fixture()
    };
    assert_eq!(
        ctx.entity_view(173).unwrap().detection_position,
        crate::coordinates::MapPoint::new(70.0, 80.0)
    );
    assert_eq!(
        ctx.entity_view(173).unwrap().position,
        committed_lift_position
    );

    let mut tick = AiPerTickData::stub();
    add_owner_sword_range(&mut tick, 84, 50);
    tick.owner_live_position = Some(ctx.position);
    tick.primary_target_snapshot_handle = Some(AiEntityHandle::new(47));
    tick.primary_target_position = Some(Position {
        x: 900.0,
        y: 900.0,
        sector: crate::position_interface::SectorHandle::new(5),
        level: 0,
    });

    ai.reconsider_enemy_approach(false, &ctx, &tick, None);

    let expected_entry = Position {
        x: 410.0,
        y: 120.0,
        sector: crate::position_interface::SectorHandle::new(7),
        level: 3,
    };
    assert_eq!(ai.base.primary_target, Some(AiEntityHandle::new(173)));
    let focused = ai.base.outbox.actor.focus.or_else(|| {
        ai.base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .find_map(|work| match work {
                crate::ai::AiOwnerWork::StateChange(notification) => notification
                    .actor_effects_before_callback
                    .as_ref()
                    .and_then(|effects| effects.focus),
                _ => None,
            })
    });
    assert_eq!(focused, Some(AiEntityHandle::new(173)));
    assert_eq!(ai.base.current_substate, Substate::AttackingRunningToLadder);
    assert_eq!(ai.base.seek_position, expected_entry);
    assert_eq!(ai.base.outbox.actor.orders.len(), 1);
    let order = &ai.base.outbox.actor.orders[0];
    assert_eq!(order.order_type, crate::order::OrderType::RunningUpright);
    assert_eq!((order.target_x, order.target_y), (410.0, 120.0));
    assert_eq!(order.target_sector, expected_entry.sector);
    assert_eq!(order.target_layer, Some(3));
    assert_eq!(order.tolerance, 30.0);
    assert!(ai.base.timer_is_running);
    assert_eq!(ai.base.when_does_timer_ring, 1088);
}

#[test]
fn reconsider_approach_does_not_reuse_old_lift_after_synchronous_retarget() {
    let mut ai = EnemyAi::new(84);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingQuittingSwordfight;
    ai.base.primary_target = Some(AiEntityHandle::new(173));
    ai.sword_range = 50;

    let ordinary_replacement = Position {
        x: 700.0,
        y: 0.0,
        sector: crate::position_interface::SectorHandle::new(5),
        level: 0,
    };
    let mut replacement_view = pc_view();
    replacement_view.position = ordinary_replacement;
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(173, replacement_view);
    let ctx = AiContext {
        position: Position::default(),
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        fast_grid: std::sync::Arc::new(reconsider_approach_lift_grid()),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    // The serialized EnemyAi cache deliberately disagrees with the live
    // actor weapon. The original game asks the sword for the latter on every
    // reconsidered enemy approach movement.
    add_owner_sword_range(&mut tick, 84, 65);
    tick.primary_target_snapshot_handle = Some(AiEntityHandle::new(47));
    tick.primary_target_position = Some(Position {
        x: 500.0,
        y: 500.0,
        sector: crate::position_interface::SectorHandle::new(42),
        level: 0,
    });

    ai.reconsider_enemy_approach(false, &ctx, &tick, None);

    assert_eq!(ai.base.primary_target, Some(AiEntityHandle::new(173)));
    assert_eq!(ai.base.current_substate, Substate::AttackingRunningToEnemy);
    assert_ne!(ai.base.current_substate, Substate::AttackingRunningToLadder);
    assert_eq!(ai.base.seek_position, ordinary_replacement);
    let order = ai.base.outbox.actor.orders.first().or_else(|| {
        ai.base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .find_map(|work| match work {
                crate::ai::AiOwnerWork::StateChange(notification) => notification
                    .actor_effects_before_callback
                    .as_ref()
                    .and_then(|effects| effects.orders.first()),
                crate::ai::AiOwnerWork::ActorEffects(effects) => effects.orders.first(),
                _ => None,
            })
    });
    let order = order.expect("ordinary replacement must queue its running approach");
    assert_eq!(order.order_type, crate::order::OrderType::RunningUpright);
    assert_eq!((order.target_x, order.target_y), (700.0, 0.0));
    assert_eq!(order.target_sector, ordinary_replacement.sector);
    assert_eq!(order.tolerance, 65.0);
}

#[test]
fn reconsider_approach_move_precedes_state_change_callback() {
    let mut ai = EnemyAi::new(180);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingTooProudToAttackApproach;
    ai.base.primary_target = Some(AiEntityHandle::new(198));
    ai.base.think_recursion_depth = 1;
    ai.sword_range = 50;

    let target_position = Position {
        x: 1731.4956,
        y: 2379.8796,
        ..Position::default()
    };
    let ctx = AiContext {
        position: Position {
            x: 1773.7925,
            y: 2523.631,
            ..Position::default()
        },
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    add_owner_sword_range(&mut tick, 180, 50);
    tick.primary_target_snapshot_handle = Some(AiEntityHandle::new(198));
    tick.primary_target_position = Some(target_position);

    ai.reconsider_enemy_approach(true, &ctx, &tick, None);

    assert_eq!(ai.base.current_substate, Substate::AttackingRunningToEnemy);
    let transition = ai
        .base
        .outbox
        .reentrant
        .owner_work
        .iter()
        .find_map(|work| match work {
            crate::ai::AiOwnerWork::StateChange(notification)
                if notification.incoming_substate == Substate::AttackingRunningToEnemy =>
            {
                Some(notification)
            }
            _ => None,
        })
        .expect("running approach must queue its state-change callback");
    assert!(transition.actor_effects_before_callback.is_none());
    let work = &ai.base.outbox.reentrant.owner_work;
    let actor_effects_index = work
        .iter()
        .position(|work| matches!(work, crate::ai::AiOwnerWork::ActorEffects(_)))
        .expect("approach movement must be sealed as an actor-effects owner boundary");
    let resume_index = work
        .iter()
        .position(|work| {
            matches!(
                work,
                crate::ai::AiOwnerWork::ResumeReconsiderEnemyApproachAfterGoNear { .. }
            )
        })
        .expect("failed-route continuation must remain queued");
    assert!(actor_effects_index < resume_index);
    let state_change_index = work
        .iter()
        .position(|work| matches!(work, crate::ai::AiOwnerWork::StateChange(_)))
        .expect("approach state change remains queued after its route prefix");
    assert!(actor_effects_index < state_change_index);
    let crate::ai::AiOwnerWork::ActorEffects(prefix) = &work[actor_effects_index] else {
        unreachable!()
    };
    assert_eq!(prefix.orders.len(), 1);
    assert_eq!(
        prefix.orders[0].order_type,
        crate::order::OrderType::RunningUpright
    );
    assert_eq!(prefix.orders[0].tolerance, 50.0);
    assert!(ai.base.outbox.actor.orders.is_empty());
    assert!(
        work[state_change_index + 1..resume_index]
            .iter()
            .any(|work| matches!(
                work,
                crate::ai::AiOwnerWork::ActorEffects(effects)
                    if effects.set_attentive_mode.map(|effect| effect.target) == Some(true)
            ))
    );
    assert!(
        ai.base
            .outbox
            .reentrant
            .reconsider_approach_completion_pending
    );
    assert!(ai.base.outbox.reentrant.owner_work.iter().any(|work| {
        matches!(
            work,
            crate::ai::AiOwnerWork::ResumeReconsiderEnemyApproachAfterGoNear {
                target: 198,
                target_position: queued_target,
            } if *queued_target == target_position
        )
    }));
}

#[test]
fn reconsider_approach_same_substate_seals_live_move_without_stealing_old_callback() {
    let mut ai = EnemyAi::new(180);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingRunningToEnemy;
    ai.base.primary_target = Some(AiEntityHandle::new(198));
    ai.base.think_recursion_depth = 1;
    ai.sword_range = 50;
    ai.base
        .outbox
        .reentrant
        .owner_work
        .push(crate::ai::AiOwnerWork::StateChange(
            crate::ai::AiStateChangeNotification {
                outgoing_state: AiState::Attacking,
                outgoing_substate: Substate::AttackingTooProudToAttackApproach,
                incoming_state: AiState::Attacking,
                incoming_substate: Substate::AttackingRunningToEnemy,
                source: crate::ai::AiStateChangeSource::from_optional_human(198),
                actor_effects_before_callback: None,
            },
        ));

    let target_position = Position {
        x: 1731.4956,
        y: 2379.8796,
        ..Position::default()
    };
    let ctx = AiContext {
        position: Position {
            x: 1773.7925,
            y: 2523.631,
            ..Position::default()
        },
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    add_owner_sword_range(&mut tick, 180, 50);
    tick.primary_target_snapshot_handle = Some(AiEntityHandle::new(198));
    tick.primary_target_position = Some(target_position);

    ai.reconsider_enemy_approach(true, &ctx, &tick, None);

    let work = &ai.base.outbox.reentrant.owner_work;
    let crate::ai::AiOwnerWork::StateChange(old_notification) = &work[0] else {
        panic!("older matching callback must retain its owner slot")
    };
    assert!(old_notification.actor_effects_before_callback.is_none());
    assert_eq!(
        work.iter()
            .filter(|work| matches!(work, crate::ai::AiOwnerWork::StateChange(_)))
            .count(),
        1,
        "same-substate updates must not manufacture a callback"
    );
    let actor_effects_index = work
        .iter()
        .enumerate()
        .skip(1)
        .find_map(|(index, work)| match work {
            crate::ai::AiOwnerWork::ActorEffects(effects)
                if effects.orders.iter().any(|order| {
                    order.order_type == crate::order::OrderType::RunningUpright
                        && order.tolerance == 50.0
                }) =>
            {
                Some(index)
            }
            _ => None,
        })
        .expect("same-substate approach movement must become real actor owner work");
    let resume_index = work
        .iter()
        .position(|work| {
            matches!(
                work,
                crate::ai::AiOwnerWork::ResumeReconsiderEnemyApproachAfterGoNear { .. }
            )
        })
        .expect("route completion must resume the source statement");
    assert!(actor_effects_index < resume_index);
    let attentive_effects_index = work
        .iter()
        .enumerate()
        .skip(actor_effects_index + 1)
        .find_map(|(index, work)| match work {
            crate::ai::AiOwnerWork::ActorEffects(effects)
                if effects.set_attentive_mode.map(|effect| effect.target) == Some(true) =>
            {
                Some(index)
            }
            _ => None,
        })
        .expect("same-substate update tail must remain after the approach boundary");
    assert!(attentive_effects_index < resume_index);
    let crate::ai::AiOwnerWork::ActorEffects(route_effects) = &work[actor_effects_index] else {
        unreachable!()
    };
    assert!(route_effects.set_attentive_mode.is_none());
    assert!(ai.base.outbox.actor.orders.is_empty());
    assert!(
        ai.base
            .outbox
            .reentrant
            .reconsider_approach_completion_pending
    );
}

#[test]
fn failed_reconsider_approach_resumes_with_avenger_roof_wait() {
    let mut ai = EnemyAi::new(205);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingRunningToEnemy;
    ai.base.primary_target = Some(AiEntityHandle::new(298));
    ai.base.couldnt_reachpoint = true;
    let target_position = Position {
        x: 264.0,
        y: 1358.0,
        ..Position::default()
    };
    let wait_position = Position {
        x: 250.0,
        y: 1200.0,
        sector: crate::position_interface::SectorHandle::new(64),
        level: 1,
    };

    ai.resume_reconsider_enemy_approach_after_go_near(
        target_position,
        Some(wait_position),
        &AiContext::test_fixture(),
    );

    assert!(!ai.base.couldnt_reachpoint);
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingRunToAvengerOnRoof
    );
    assert_eq!(ai.base.seek_position, target_position);
    assert_eq!(ai.base.outbox.actor.orders.len(), 1);
    let order = &ai.base.outbox.actor.orders[0];
    assert_eq!(order.target_x, wait_position.x);
    assert_eq!(order.target_y, wait_position.y);
    assert_eq!(order.target_sector, wait_position.sector);
    assert_eq!(order.target_layer, Some(wait_position.level));
    assert_eq!(order.tolerance, 50.0);
    assert!(
        !order.defer_instruction,
        "an ordinary synchronous route failure instructs its roof fallback this frame"
    );
}

#[test]
fn close_avenger_roof_wait_position_completes_without_an_order() {
    let mut ai = EnemyAi::new(205);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingRunningToEnemy;
    ai.base.primary_target = Some(AiEntityHandle::new(298));
    ai.base.couldnt_reachpoint = true;
    ai.base.think_recursion_depth = 1;
    ai.base
        .outbox
        .reentrant
        .reconsider_approach_replaced_path_waiter = true;
    let target_position = Position {
        x: 264.0,
        y: 1358.0,
        ..Position::default()
    };
    let wait_position = Position {
        x: 250.0,
        y: 1200.0,
        sector: crate::position_interface::SectorHandle::new(64),
        level: 1,
    };
    let ctx = AiContext {
        position: wait_position,
        self_layer: wait_position.level,
        ..AiContext::test_fixture()
    };

    ai.resume_reconsider_enemy_approach_after_go_near(target_position, Some(wait_position), &ctx);

    assert!(!ai.base.couldnt_reachpoint);
    assert!(ai.base.already_on_point);
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingRunToAvengerOnRoof
    );
    assert_eq!(ai.base.seek_position, target_position);
    assert!(ai.base.outbox.actor.orders.is_empty());
    assert!(
        !ai.base
            .outbox
            .reentrant
            .reconsider_approach_replaced_path_waiter
    );
}

#[test]
fn failed_reconsider_approach_replacing_path_waiter_halts_roof_after_launch() {
    let mut ai = EnemyAi::new(205);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingRunningToEnemy;
    ai.base.primary_target = Some(AiEntityHandle::new(298));
    ai.base.couldnt_reachpoint = true;
    ai.base
        .outbox
        .reentrant
        .reconsider_approach_replaced_path_waiter = true;

    ai.resume_reconsider_enemy_approach_after_go_near(
        Position {
            x: 264.0,
            y: 1358.0,
            ..Position::default()
        },
        Some(Position {
            x: 250.0,
            y: 1200.0,
            sector: crate::position_interface::SectorHandle::new(64),
            level: 1,
        }),
        &AiContext::test_fixture(),
    );

    assert!(ai.base.outbox.actor.orders[0].halt_after_launch_for_path_waiter);
    assert!(!ai.base.outbox.actor.orders[0].defer_instruction);
    assert!(
        !ai.base
            .outbox
            .reentrant
            .reconsider_approach_replaced_path_waiter,
        "path-waiter provenance is one-shot"
    );
}

#[test]
fn reconsider_approach_already_near_engages_before_approach_state_change() {
    let mut ai = EnemyAi::new(180);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontime;
    ai.base.primary_target = Some(AiEntityHandle::new(198));
    ai.base.think_recursion_depth = 1;
    ai.sword_range = 150;
    ai.sword_is_charge_weapon = true;

    let target_position = Position {
        x: 100.0,
        ..Position::default()
    };
    let ctx = AiContext::test_fixture();
    let mut tick = AiPerTickData::stub();
    add_owner_sword_range(&mut tick, 180, 150);
    tick.primary_target_snapshot_handle = Some(AiEntityHandle::new(198));
    tick.primary_target_position = Some(target_position);
    // Preserve the charge branch past Original's intentionally broad
    // "walking circus pyramid" command comparison.
    tick.primary_target_animation = Some(crate::order::OrderType::WalkingCarryingOnShoulders);

    ai.reconsider_enemy_approach(false, &ctx, &tick, None);

    assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
    assert!(!ai.base.already_on_point);
    assert!(ai.base.outbox.actor.orders.is_empty());
    assert!(
        !ai.base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .any(|work| matches!(
                work,
                crate::ai::AiOwnerWork::StateChange(notification)
                    if notification.incoming_substate
                        == Substate::AttackingChargingEnemy
            )),
        "the already-near branch must engage before entering the charging-enemy state"
    );
}

#[test]
fn observe_threshold_keeps_fractional_courage_bonus() {
    // One visible enemy and courage 45 yields 3.025 in Original. Three
    // nearer friends are therefore insufficient; four are sufficient.
    assert!(!enough_nearer_friends_to_observe(3, 1, 45));
    assert!(enough_nearer_friends_to_observe(4, 1, 45));
}

#[test]
fn friend_distance_gate_uses_selected_target_with_source_units() {
    // Exercise the live battle helpers, not the removed detection-time
    // aggregate whose value was discarded before every decision.
    let owner_world = crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0);
    let target_world = crate::coordinates::WorldPoint3D::new(100.0, 0.0, 0.0);
    let target = Position {
        x: 100.0,
        ..Position::default()
    };
    let friend = Position {
        x: 50.0,
        ..Position::default()
    };
    let distance = battle_owner_target_square_distance(owner_world, target_world);
    assert!(battle_friend_is_nearer(friend, target, distance));
    let other_target = Position {
        x: -1000.0,
        ..Position::default()
    };
    assert!(!battle_friend_is_nearer(friend, other_target, distance));
}

#[test]
fn elevated_owner_distance_does_not_count_two_door_friends_as_nearer() {
    // linux2/Profile_002/Savegame_001/replay-001, immediately before
    // frame 2147. Soldier 137 and PC 252 are separated vertically, so
    // Literal 3D squared distance is much smaller than the
    // distance obtained after projecting both actors to map space.
    let owner_world = crate::coordinates::WorldPoint3D::new(1720.6782, 2002.2788, 17.413866);
    let target_world = crate::coordinates::WorldPoint3D::new(1741.3412, 2000.2783, 37.16403);
    let owner_target_sq = battle_owner_target_square_distance(owner_world, target_world);
    assert_eq!(owner_target_sq, 829);

    let target = Position {
        x: 1741.3412,
        y: 1963.1143,
        ..Position::default()
    };
    // Soldiers 104 and 142 are both passing door 100 directly. AI
    // Position() commits both to point_in=(1712, 1994), whose raw map
    // distance is outside the correct 829 threshold but inside the old
    // projected-map threshold (~1865).
    let door_point_in = Position {
        x: 1712.0,
        y: 1994.0,
        ..Position::default()
    };
    assert!(!battle_friend_is_nearer(
        door_point_in,
        target,
        owner_target_sq
    ));

    let ordinary_nearer_friend = Position {
        x: 1722.0557,
        y: 1983.415,
        ..Position::default()
    };
    let mut nearer_friends = 1_u16; // Soldier 139 is already swordfighting.
    for friend in [ordinary_nearer_friend, door_point_in, door_point_in] {
        if battle_friend_is_nearer(friend, target, owner_target_sq) {
            nearer_friends += 1;
        }
    }
    assert_eq!(nearer_friends, 2);

    let decision = if enough_nearer_friends_to_observe(nearer_friends, 1, 40) {
        Decision::Observe
    } else {
        Decision::Fight
    };
    assert_eq!(decision, Decision::Fight);
}

#[test]
fn reserve_cutoff_uses_literal_3d_square_distance() {
    // linux3/Profile_003/Savegame_010/replay-003, frame 19649.
    // Projecting the actors to map space puts Soldier 144 just outside
    // the 150-unit reserve radius, while Original's literal 3D
    // World-position distance puts it inside and falls through to Observe.
    let owner_world = crate::coordinates::WorldPoint3D::new(669.78064, 1511.1458, 35.611286);
    let target_world = crate::coordinates::WorldPoint3D::new(745.8495, 1441.5433, 50.001003);
    let literal_3d = battle_owner_target_square_distance(owner_world, target_world);
    assert!(literal_3d < combat::MIN_SQUARE_RESERVE_DISTANCE as u32);

    let dx = 745.8495_f32 - 669.78064_f32;
    let dy = (1391.5424_f32 - 1475.5344_f32) * INVERSE_ASPECT_RATIO;
    let projected_map = (dx * dx + dy * dy) as u32;
    assert!(projected_map > combat::MIN_SQUARE_RESERVE_DISTANCE as u32);
}

#[test]
fn observe_move_precedes_state_change_callback() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(91);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontimeRunning;
    ai.list_them = vec![198];
    ai.base.stop_all();

    let target_position = Position {
        x: 500.0,
        ..Position::default()
    };
    let target_view = pc_view_at(target_position);
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(198, target_view);
    views.insert(91, pc_view_at(Position::default()));
    let ctx = AiContext {
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    // Deliberately leave nearby_fighters empty: the persistent Them-list
    // target can lie just outside that 500-unit decision snapshot.
    let tick = AiPerTickData::stub();

    assert!(!ai.execute_battle_decision(
        &sim,
        Decision::Observe,
        Substate::AttackingReactiontimeRunning,
        0,
        &mut std::collections::BTreeMap::from([(198, 0)]),
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    ));

    let [
        crate::ai::AiOwnerWork::ActorEffects(route),
        crate::ai::AiOwnerWork::ResumeBattleObserveAfterGoNear {
            target,
            target_position: queued_target_position,
        },
    ] = ai.base.outbox.reentrant.owner_work.as_slice()
    else {
        panic!(
            "routed Observe must settle movement before resuming its source-ordered tail: {:?}",
            ai.base.outbox.reentrant.owner_work
        );
    };
    assert!(route.halt);
    assert_eq!(route.orders.len(), 1);
    assert_eq!(
        route.orders[0].order_type,
        crate::order::OrderType::WalkingUpright
    );
    assert_eq!(*target, 198);
    assert_eq!(*queued_target_position, target_position);
    assert!(ai.base.outbox.reentrant.battle_observe_completion_pending);
    assert!(ai.base.outbox.actor.orders.is_empty());
    // `battle_observe_route_settles_before_source_ordered_tail` exercises
    // the engine drain that consumes this continuation and performs the
    // following state-change callback at Original's synchronous boundary.
}

#[test]
fn failed_fight_approach_resumes_inline_observe_decision() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(91);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingRunningToEnemy;
    ai.base.primary_target = Some(AiEntityHandle::new(198));
    ai.base.couldnt_reachpoint = true;
    // Rust has temporarily unwound the recursion depth while the first
    // engine-owned route is settled, but the continuation still belongs
    // to the enclosing original-game decision tick.
    ai.base.completion_latch_inside_think = true;
    ai.list_them = vec![198];

    let target_position = Position {
        x: 500.0,
        ..Position::default()
    };
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(198, pc_view_at(target_position));
    views.insert(91, pc_view_at(Position::default()));
    let ctx = AiContext {
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    ai.resume_battle_fight_after_reconsider(
        &sim,
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
    );

    assert!(!ai.base.couldnt_reachpoint);
    assert_eq!(ai.base.primary_target, Some(AiEntityHandle::new(198)));
    let [
        crate::ai::AiOwnerWork::ActorEffects(route),
        crate::ai::AiOwnerWork::ResumeBattleObserveAfterGoNear {
            target,
            target_position: queued_target_position,
        },
    ] = ai.base.outbox.reentrant.owner_work.as_slice()
    else {
        panic!(
            "failed Fight must continue through Observe on the same owner boundary: {:?}",
            ai.base.outbox.reentrant.owner_work
        );
    };
    assert_eq!(*target, 198);
    assert_eq!(*queued_target_position, target_position);
    assert_eq!(route.orders.len(), 1);
    assert_eq!(
        route.orders[0].order_type,
        crate::order::OrderType::WalkingUpright
    );
    assert!(ai.base.outbox.reentrant.battle_observe_completion_pending);
    assert!(
        ai.base.completion_latch_inside_think,
        "the nested Observe route must retain the enclosing Think's completion ownership"
    );
}

fn proud_decision_speech(
    entry_substate: Substate,
    serialized_previous_substate: Substate,
) -> Vec<Remark> {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(91);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = entry_substate;
    ai.previous_substate = serialized_previous_substate as i32;
    ai.forced_next_battle_decision = Decision::TooProudToAttack;
    ai.list_them = vec![198];

    let target_position = Position {
        x: 150.0,
        ..Position::default()
    };
    let mut target_view = pc_view();
    target_view.position = target_position;
    target_view.forecasted_destination =
        crate::ai::PreparedForecastDestination::fixed(target_position, 0);
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    let mut owner_view = pc_view();
    owner_view.is_pc = false;
    owner_view.kind = crate::ai_entity_view::EntityKind::Soldier;
    owner_view.camp = crate::element::Camp::Lacklandists;
    views.insert(91, owner_view);
    views.insert(198, target_view);
    let ctx = AiContext {
        camp: crate::element::Camp::Lacklandists,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.enemy_sq_distances = vec![(198, 150 * 150)];
    tick.nearby_fighters = vec![FighterSnapshot {
        handle: 198,
        position: target_position,
        is_able_to_fight: true,
        is_pc: true,
        ..Default::default()
    }];

    ai.battle_decisions(&sim, &mut AiGlobalState::default(), &ctx, &tick, None);
    ai.base
        .outbox
        .reentrant
        .owner_work
        .iter()
        .filter_map(|work| match work {
            crate::ai::AiOwnerWork::Speech(attempt) => Some(attempt.remark),
            _ => None,
        })
        .collect()
}

#[test]
fn proud_first_decision_uses_entry_substate_not_serialized_previous_substate() {
    assert_eq!(
        proud_decision_speech(Substate::AttackingReactiontime, Substate::DefaultOnPost,),
        vec![Remark::ProudDontFight]
    );
}

#[test]
fn proud_later_decision_ignores_stale_reactiontime_previous_substate() {
    assert!(
        proud_decision_speech(
            Substate::AttackingTooProudToAttack,
            Substate::AttackingReactiontime,
        )
        .is_empty()
    );
}

#[test]
fn alert_soldiers_without_a_live_target_falls_back_to_reserve() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(91);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontime;

    assert!(ai.execute_battle_decision(
        &sim,
        Decision::AlertSoldiers,
        Substate::AttackingReactiontime,
        0,
        &mut std::collections::BTreeMap::new(),
        &mut AiGlobalState::default(),
        &AiContext::test_fixture(),
        &AiPerTickData::stub(),
        None,
    ));

    assert_eq!(ai.base.primary_target, None);
    assert!(!ai.base.friends_are_alerted);
    assert_eq!(ai.base.current_substate, Substate::AttackingReserve);
    assert!(ai.base.timer_is_running);
    assert!(ai.base.outbox.reentrant.cross_npc_actions.is_empty());
}

#[test]
fn tower_guard_decisions_without_a_live_target_fall_back_to_reserve() {
    let sim = crate::sim_rng::test_context();

    for decision in [Decision::TowerGuardAlert, Decision::TowerGuardObserve] {
        let mut ai = EnemyAi::new(91);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingReactiontime;

        assert!(ai.execute_battle_decision(
            &sim,
            decision,
            Substate::AttackingReactiontime,
            0,
            &mut std::collections::BTreeMap::new(),
            &mut AiGlobalState::default(),
            &AiContext::test_fixture(),
            &AiPerTickData::stub(),
            None,
        ));

        assert_eq!(ai.base.primary_target, None, "{decision:?}");
        assert!(!ai.base.friends_are_alerted, "{decision:?}");
        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingReserve,
            "{decision:?}"
        );
        assert!(ai.base.timer_is_running, "{decision:?}");
        assert!(ai.base.outbox.actor.orders.is_empty(), "{decision:?}");
    }
}

#[test]
fn archer_step_back_without_a_live_target_falls_back_through_shoot() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(91);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontime;
    let ctx = AiContext {
        remaining_arrows: 1,
        ..AiContext::test_fixture()
    };

    assert!(ai.execute_battle_decision(
        &sim,
        Decision::ArcherStepBack,
        Substate::AttackingReactiontime,
        0,
        &mut std::collections::BTreeMap::new(),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    ));

    assert_eq!(ai.base.primary_target, None);
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingBowObservingLoading
    );
}

#[test]
fn tower_guard_uses_live_ai_position_instead_of_stale_nearby_snapshot() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(91);
    ai.base.owner_entity_id = Some(crate::element::EntityId::Soldier(
        crate::element::SoldierId(91),
    ));
    ai.tower_guard = true;
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontime;
    ai.list_them = vec![198];
    ai.forced_next_battle_decision = Decision::TowerGuardAlert;

    let live_position = Position {
        x: 1386.0,
        y: 1356.0,
        sector: crate::position_interface::SectorHandle::new(19).map(|sector| {
            sector.with_arena_index(crate::fast_find_grid::SectorIndex::new(0).unwrap())
        }),
        level: 1,
    };
    let stale_position = Position {
        x: 1389.106,
        y: 1361.8235,
        ..live_position
    };
    let mut owner_view = pc_view();
    owner_view.is_pc = false;
    owner_view.kind = crate::ai_entity_view::EntityKind::Soldier;
    owner_view.camp = crate::element::Camp::Lacklandists;
    let mut target_view = pc_view();
    target_view.position = live_position;
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(91, owner_view);
    views.insert(198, target_view);
    let ctx = AiContext {
        camp: crate::element::Camp::Lacklandists,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture_with_motion_sector(19, 1)
    };
    let mut tick = AiPerTickData::stub();
    tick.enemy_sq_distances = vec![(198, 100)];
    tick.nearby_fighters = vec![FighterSnapshot {
        handle: 198,
        position: stale_position,
        is_able_to_fight: true,
        is_pc: true,
        ..Default::default()
    }];

    ai.battle_decisions(&sim, &mut AiGlobalState::default(), &ctx, &tick, None);

    assert_eq!(ai.base.primary_target, Some(AiEntityHandle::new(198)));
    assert_eq!(ai.base.seek_position, live_position);
    assert_eq!(ai.base.current_substate, Substate::AttackingTowerGuardAlert);
}

fn battle_cleanup_context(
    target: crate::ai_entity_view::AiEntityView,
) -> (AiContext, AiPerTickData) {
    let mut owner = pc_view();
    owner.is_pc = false;
    owner.kind = crate::ai_entity_view::EntityKind::Soldier;
    owner.camp = crate::element::Camp::Lacklandists;
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(91, owner);
    views.insert(198, target);
    (
        AiContext {
            camp: crate::element::Camp::Lacklandists,
            frame: 700,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::test_fixture()
        },
        AiPerTickData::stub(),
    )
}

fn battle_with_unavailable_initial_target(
    reason: Option<crate::ai_entity_view::AiObservationUnavailable>,
) {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(91);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingOverviewLookRight;
    ai.list_them = vec![198];
    let (mut ctx, tick) = battle_cleanup_context(pc_view());
    let views = std::sync::Arc::get_mut(&mut ctx.entity_views).unwrap();
    views.entities.remove(&198);
    if let Some(reason) = reason {
        views.unavailable_entities.insert(198, reason);
    }
    // Initial target selection requires spatial state, before the later
    // cleanup of friend-contributed entries. Do not broaden that admission
    // policy merely because unavailable observations now have typed reasons.
    ai.battle_decisions(&sim, &mut AiGlobalState::default(), &ctx, &tick, None);
}

#[test]
#[should_panic(expected = "required enemy-list entry 198 missing")]
fn absent_initial_battle_target_remains_an_invariant_failure() {
    battle_with_unavailable_initial_target(None);
}

#[test]
#[should_panic(expected = "required enemy-list entry 198 missing")]
fn missing_layer_initial_battle_target_remains_an_invariant_failure() {
    battle_with_unavailable_initial_target(Some(
        crate::ai_entity_view::AiObservationUnavailable::MissingLayer,
    ));
}

#[test]
#[should_panic(expected = "required enemy-list entry 198 missing")]
fn excluded_initial_battle_target_remains_an_invariant_failure() {
    battle_with_unavailable_initial_target(Some(
        crate::ai_entity_view::AiObservationUnavailable::ExcludedEntity,
    ));
}

#[test]
fn stale_same_camp_them_entry_preserves_visible_count_for_reserve() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(91);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingOverviewLookRight;
    ai.list_them = vec![198];
    ai.forced_next_battle_decision = Decision::Reserve;

    let mut stale_friend = pc_view();
    stale_friend.camp = crate::element::Camp::Lacklandists;
    let (ctx, tick) = battle_cleanup_context(stale_friend);

    ai.battle_decisions(&sim, &mut AiGlobalState::default(), &ctx, &tick, None);

    assert!(ai.list_them.is_empty(), "the stale friend must be removed");
    assert_eq!(ai.base.current_state, AiState::Attacking);
    assert_eq!(ai.base.current_substate, Substate::AttackingReserve);
    assert!(ai.base.timer_is_running);
    assert_eq!(ai.base.when_does_timer_ring, 750);
}

#[test]
fn unable_nonfriend_them_entry_consumes_visible_count_and_returns_to_duty() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(91);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingOverviewLookRight;
    ai.list_them = vec![198];
    ai.forced_next_battle_decision = Decision::Reserve;

    let mut unable_enemy = pc_view();
    unable_enemy.camp = crate::element::Camp::Royalists;
    unable_enemy.is_able_to_fight = false;
    let (ctx, tick) = battle_cleanup_context(unable_enemy);

    ai.battle_decisions(&sim, &mut AiGlobalState::default(), &ctx, &tick, None);

    assert!(ai.list_them.is_empty());
    assert_ne!(ai.base.current_substate, Substate::AttackingReserve);
    assert!(!ai.base.timer_is_running);
    assert!(ai.base.outbox.reentrant.owner_work.iter().any(|work| {
        matches!(
            work,
            crate::ai::AiOwnerWork::ResumeReturnToDutyAfterPatrolInit { .. }
        )
    }));
}

#[test]
fn battle_decisions_preserves_enemy_list_from_last_explicit_rebuild() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(91);
    ai.base.current_state = AiState::Attacking;
    ai.base.primary_target = Some(AiEntityHandle::new(198));
    ai.list_them = vec![198, 199];

    // The predecision pass walks the persistent us-list through the
    // shared entity-view table, and that list always includes the
    // evaluating soldier itself.
    let me_entity = crate::element::Entity::Soldier(crate::element::ActorSoldier {
        element: {
            let mut initial_element =
                crate::element::ElementData::from_initial_posture(crate::element::Posture::Upright);
            initial_element.kind = crate::element::ElementKind::ActorSoldier;
            initial_element.active = true;
            initial_element
        },
        actor: Default::default(),
        human: Default::default(),
        npc: crate::element::NpcData {
            life_points: 50,
            ai: crate::element::AiActorData {
                ai_brain: crate::element::AiBrain::Enemy(Box::default()),
                ..Default::default()
            },
        },
        soldier: Default::default(),
    });
    let me_view = crate::ai_entity_view::entity_view_from_entity(
        &me_entity,
        40,
        false,
        None,
        None,
        crate::order::OrderType::NonanimationEnd,
    );
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(91, me_view);
    views.insert(198, pc_view());
    views.insert(199, pc_view());
    assert!(views[&198].is_able_to_fight);
    assert!(views[&199].is_able_to_fight);
    let ctx = AiContext {
        camp: crate::element::Camp::Lacklandists,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.enemy_sq_distances = vec![(198, 100)];
    tick.nearby_fighters = vec![
        FighterSnapshot {
            handle: 198,
            position: Position::default(),
            is_able_to_fight: true,
            is_pc: true,
            ..Default::default()
        },
        FighterSnapshot {
            handle: 199,
            position: Position {
                x: 20.0,
                ..Position::default()
            },
            is_able_to_fight: true,
            is_pc: true,
            ..Default::default()
        },
    ];

    ai.battle_decisions(&sim, &mut AiGlobalState::default(), &ctx, &tick, None);

    assert!(
        ai.list_them.contains(&199),
        "battle planning must not replace the persistent list with its tick snapshot"
    );
}

#[test]
fn cover_behind_untargeted_shield_bearer_falls_back_to_archer_observe() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(69);
    ai.is_archer_unit = true;
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontimeRunning;
    ai.base.primary_target = Some(AiEntityHandle::new(126));

    let ctx = AiContext {
        remaining_arrows: 10,
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.nearby_fighters = vec![FighterSnapshot {
        handle: 73,
        is_friendly: true,
        is_soldier: true,
        is_shield_bearer: true,
        primary_target: None,
        ..FighterSnapshot::default()
    }];

    assert!(ai.execute_battle_decision(
        &sim,
        Decision::CoverBehindShieldBearer,
        Substate::AttackingReactiontimeRunning,
        73,
        &mut std::collections::BTreeMap::new(),
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    ));

    assert_eq!(ai.shield_bearer_before_me, None);
    assert_eq!(ai.base.primary_target, None);
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingBowObservingLoading
    );
    let mut launch_commands = Vec::new();
    for work in &ai.base.outbox.reentrant.owner_work {
        if let crate::ai::AiOwnerWork::StateChange(notification) = work
            && let Some(effects) = &notification.actor_effects_before_callback
        {
            launch_commands.extend(effects.launch_commands.iter().copied());
        }
    }
    launch_commands.extend(ai.base.outbox.actor.launch_commands.iter().copied());
    assert_eq!(launch_commands, vec![crate::element::Command::EquipBow]);
}

#[test]
fn rejected_shield_cover_keeps_computed_seek_position_before_shoot_fallback() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(69);
    ai.is_archer_unit = true;
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontime;
    ai.base.primary_target = Some(AiEntityHandle::new(126));
    ai.base.seek_position = Position {
        x: 10.0,
        y: 20.0,
        ..Position::default()
    };

    let bearer_position = Position {
        x: 200.0,
        y: 300.0,
        ..Position::default()
    };
    let target_position = Position {
        x: 1000.0,
        y: 1000.0,
        ..Position::default()
    };
    let ctx = AiContext {
        remaining_arrows: 10,
        sq_standard_view_radius: 100.0,
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.nearby_fighters = vec![
        FighterSnapshot {
            handle: 73,
            position: bearer_position,
            direction: 0,
            is_friendly: true,
            is_soldier: true,
            is_shield_bearer: true,
            primary_target: Some(AiEntityHandle::new(126)),
            ..FighterSnapshot::default()
        },
        FighterSnapshot {
            handle: 126,
            position: target_position,
            is_able_to_fight: true,
            is_pc: true,
            ..FighterSnapshot::default()
        },
    ];
    let expected_cover = ai
        .shield_bearer_cover_position(73, &tick)
        .expect("fixture shield bearer must produce a cover position");
    assert!(
        square_norm(pos_diff(&target_position, &expected_cover)) >= ctx.sq_standard_view_radius,
        "fixture must reject the computed cover point at the subsequent view-radius gate"
    );

    assert!(ai.execute_battle_decision(
        &sim,
        Decision::CoverBehindShieldBearer,
        Substate::AttackingReactiontime,
        73,
        &mut std::collections::BTreeMap::new(),
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    ));

    assert_eq!(ai.base.seek_position, expected_cover);
    assert_eq!(ai.shield_bearer_before_me, None);
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingBowObservingLoading
    );
}
