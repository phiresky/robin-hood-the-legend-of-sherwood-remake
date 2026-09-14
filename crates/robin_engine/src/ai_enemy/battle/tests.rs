use super::*;

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
