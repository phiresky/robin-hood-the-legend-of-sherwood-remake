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

    ai.begin_swordfight(&AiContext::test_fixture());

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

    ai.begin_swordfight(&AiContext::test_fixture());

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

    ai.attack_enemy(
        252,
        ThinkEnv::new(&crate::sim_rng::test_context(), &ctx, &tick, None),
    );

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

    ai.attack_enemy(
        137,
        ThinkEnv::new(&crate::sim_rng::test_context(), &ctx, &tick, None),
    );

    assert_eq!(ai.base.primary_target, Some(AiEntityHandle::new(137)));
    assert_eq!(ai.base.seek_position, live_target);
    assert_eq!(
        ai.base.seek_position.sector.unwrap().arena_index(),
        Some(crate::fast_find_grid::SectorIndex::new(137).unwrap())
    );
}

#[test]
fn failed_look_for_help_route_is_consumed_before_event_fallback() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(105);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingRunningToOfficer;
    // Officer alerting consumes the failed route before returning to this caller.
    ai.base.couldnt_reachpoint = false;
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
        ai.finish_battle_look_for_help(ThinkEnv::new(&sim, &ctx, &tick, None), false, &mut global);
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

    ai.begin_cassos_panic(252, &ctx);

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

    ai.begin_cassos_panic(0, &AiContext::test_fixture());

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

    ai.begin_cassos_panic(252, &AiContext::test_fixture());
}

#[test]
fn successful_look_for_help_continuation_draws_remark_and_logs_once() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(105);
    let mut global = AiGlobalState::default();

    let (_, draws) = crate::sim_rng::with_draw_trace(|| {
        ai.finish_battle_look_for_help(
            ThinkEnv::new(
                &sim,
                &AiContext::test_fixture(),
                &AiPerTickData::stub(),
                None,
            ),
            true,
            &mut global,
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

    assert!(
        !ai.execute_battle_decision(
            ThinkEnv::new(&sim, &ctx, &tick, None),
            Decision::Observe,
            Substate::AttackingReactiontimeRunning,
            0,
            &mut std::collections::BTreeMap::from([(198, 0)]),
            &mut AiGlobalState::default()
        )
        .unwrap()
        .is_some()
    );

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
        ThinkEnv::new(&sim, &ctx, &AiPerTickData::stub(), None),
        &mut AiGlobalState::default(),
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
}

fn proud_decision_speech(
    entry_substate: Substate,
    serialized_previous_substate: Substate,
) -> Vec<Remark> {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(91);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = entry_substate;
    ai.previous_substate = crate::ai::StoredEnumWord::new(serialized_previous_substate);
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
    tick.nearby_fighters = vec![FighterSnapshot {
        handle: 198,
        position: target_position,
        is_able_to_fight: true,
        is_pc: true,
        ..Default::default()
    }];

    ai.base.primary_target = Some(AiEntityHandle::new(198));
    ai.base.list_us = vec![91];
    ai.finish_battle_decisions(
        ThinkEnv::new(&sim, &ctx, &tick, None),
        &mut AiGlobalState::default(),
        entry_substate,
        BattleDecisionInputs {
            friends_lower_company: 0,
            soldiers_lower_pride: false,
            simple_soldiers_near: false,
            min_square_enemy_distance: 22500,
            num_enemies_i_can_see: 1,
            friends_nearer_to_enemy: 0,
        },
        std::collections::BTreeMap::from([(198, 0)]),
        Vec::new(),
    )
    .unwrap();
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

    assert!(
        ai.execute_battle_decision(
            ThinkEnv::new(
                &sim,
                &AiContext::test_fixture(),
                &AiPerTickData::stub(),
                None
            ),
            Decision::AlertSoldiers,
            Substate::AttackingReactiontime,
            0,
            &mut std::collections::BTreeMap::new(),
            &mut AiGlobalState::default()
        )
        .unwrap()
        .is_some()
    );

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

        assert!(
            ai.execute_battle_decision(
                ThinkEnv::new(
                    &sim,
                    &AiContext::test_fixture(),
                    &AiPerTickData::stub(),
                    None
                ),
                decision,
                Substate::AttackingReactiontime,
                0,
                &mut std::collections::BTreeMap::new(),
                &mut AiGlobalState::default()
            )
            .unwrap()
            .is_some()
        );

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
fn archer_step_back_without_a_live_target_transfers_to_shot_selection() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(91);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontime;
    let ctx = AiContext {
        remaining_arrows: 1,
        ..AiContext::test_fixture()
    };
    let call = ai
        .execute_battle_decision(
            ThinkEnv::new(&sim, &ctx, &AiPerTickData::stub(), None),
            Decision::ArcherStepBack,
            Substate::AttackingReactiontime,
            0,
            &mut std::collections::BTreeMap::new(),
            &mut AiGlobalState::default(),
        )
        .unwrap_err();
    assert!(matches!(
        call.tail,
        crate::ai::DutyTail::SelectShotTarget {
            old_substate: Substate::AttackingReactiontime,
            cover_shield_bearer: 0
        }
    ));
    assert_eq!(ai.base.primary_target, None);
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
    tick.nearby_fighters = vec![FighterSnapshot {
        handle: 198,
        position: stale_position,
        is_able_to_fight: true,
        is_pc: true,
        ..Default::default()
    }];

    ai.base.primary_target = Some(AiEntityHandle::new(198));
    ai.base.list_us = vec![91];
    ai.finish_battle_decisions(
        ThinkEnv::new(&sim, &ctx, &tick, None),
        &mut AiGlobalState::default(),
        Substate::AttackingReactiontime,
        BattleDecisionInputs {
            friends_lower_company: 0,
            soldiers_lower_pride: false,
            simple_soldiers_near: false,
            min_square_enemy_distance: 22500,
            num_enemies_i_can_see: 1,
            friends_nearer_to_enemy: 0,
        },
        std::collections::BTreeMap::from([(198, 0)]),
        Vec::new(),
    )
    .unwrap();

    assert_eq!(ai.base.primary_target, Some(AiEntityHandle::new(198)));
    assert_eq!(ai.base.seek_position, live_position);
    assert_eq!(ai.base.current_substate, Substate::AttackingTowerGuardAlert);
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

    assert!(
        ai.execute_battle_decision(
            ThinkEnv::new(&sim, &ctx, &tick, None),
            Decision::CoverBehindShieldBearer,
            Substate::AttackingReactiontimeRunning,
            73,
            &mut std::collections::BTreeMap::new(),
            &mut AiGlobalState::default()
        )
        .unwrap()
        .is_some()
    );

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
        (target_position.map_point() - expected_cover.map_point()).square_norm()
            >= ctx.sq_standard_view_radius,
        "fixture must reject the computed cover point at the subsequent view-radius gate"
    );

    assert!(
        ai.execute_battle_decision(
            ThinkEnv::new(&sim, &ctx, &tick, None),
            Decision::CoverBehindShieldBearer,
            Substate::AttackingReactiontime,
            73,
            &mut std::collections::BTreeMap::new(),
            &mut AiGlobalState::default()
        )
        .unwrap()
        .is_some()
    );

    assert_eq!(ai.base.seek_position, expected_cover);
    assert_eq!(ai.shield_bearer_before_me, None);
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingBowObservingLoading
    );
}
