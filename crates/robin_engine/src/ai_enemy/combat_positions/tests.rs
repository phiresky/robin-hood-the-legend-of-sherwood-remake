use super::*;
use crate::sim_rng::{RngSite, SimulationContext, with_draw_trace};

fn position(x: f32, y: f32) -> Position {
    Position {
        x,
        y,
        sector: None,
        level: 0,
    }
}

#[test]
fn combat_neighbour_distance_ulong_truncates_valid_geometry() {
    assert_eq!(combat_neighbour_distance_ulong(151.99), 151);
    let largest_below_two_to_32 = f32::from_bits(0x4f7f_ffff);
    assert_eq!(
        combat_neighbour_distance_ulong(largest_below_two_to_32),
        4_294_967_040
    );
}

#[test]
#[should_panic(expected = "outside the original-game 32-bit unsigned domain")]
fn combat_neighbour_distance_ulong_rejects_invalid_geometry() {
    let _ = combat_neighbour_distance_ulong(f32::NAN);
}

#[test]
#[should_panic(expected = "outside the original-game 32-bit unsigned domain")]
fn combat_neighbour_distance_ulong_rejects_negative_geometry() {
    let _ = combat_neighbour_distance_ulong(-1.0);
}

#[test]
#[should_panic(expected = "outside the original-game 32-bit unsigned domain")]
fn combat_neighbour_distance_ulong_rejects_infinite_geometry() {
    let _ = combat_neighbour_distance_ulong(f32::INFINITY);
}

#[test]
#[should_panic(expected = "outside the original-game 32-bit unsigned domain")]
fn combat_neighbour_distance_ulong_rejects_two_to_32() {
    let _ = combat_neighbour_distance_ulong(4_294_967_296.0_f32);
}

#[test]
fn already_in_cover_position_does_not_require_reachability() {
    // nicouzouf Savegame_010 replay-012 frame 515: the archer is already
    // behind Soldier 58. The direct cover corridor is obstructed, but
    // Original only compares this ideal offset with the archer's current
    // position and therefore keeps the relationship while shooting.
    let mut tick = AiPerTickData::stub();
    tick.fighter_registry.push(FighterSnapshot {
        handle: 58,
        position: position(1144.9557, 408.22668),
        direction: 7,
        current_substate: Substate::AttackingProtectingWithShield,
        ..FighterSnapshot::default()
    });
    let archer_position = position(1123.7424, 396.0593);
    let cover = EnemyAi::default()
        .shield_bearer_cover_position(58, &tick)
        .expect("linked shield bearer has an ideal cover position");

    assert!(
        (archer_position.map_point() - cover.map_point()).max_norm()
            < archer::COVER_POINT_TOLERANCE as f32
    );
}

#[test]
fn shield_bearer_cover_preserves_original_aspect_then_distance_rounding() {
    // Schema-16 seed 2,000,000, linux3/Profile_003/Savegame_029,
    // replay-017 frame 12826. The original game first multiplies
    // sector 10's Y component by ASPECT_RATIO, then operator*= applies
    // distance 30. Reassociating those products raises the destination Y
    // from bits 0x4268_b9b6 to 0x4268_b9b7 and eventually changes a
    // bit-exact visibility endpoint by two ULPs.
    let mut tick = AiPerTickData::stub();
    tick.fighter_registry.push(FighterSnapshot {
        handle: 82,
        position: position(1_072.624_8, 70.348_755),
        direction: 10,
        current_substate: Substate::AttackingProtectingWithShield,
        ..FighterSnapshot::default()
    });

    let cover = EnemyAi::default()
        .shield_bearer_cover_position(82, &tick)
        .expect("protecting shield bearer has a cover position");

    assert_eq!(cover.x.to_bits(), 0x4488_bad1);
    assert_eq!(cover.y.to_bits(), 0x4268_b9b6);
    assert_eq!(cover.x, 1_093.838);
    assert_eq!(cover.y, 58.181_36);
}

#[test]
fn nearest_shield_bearer_includes_inactive_running_to_phalanx_soldier() {
    // nicouzouf Profile_001 Savegame_045 replay-014 frame 1054. Soldier
    // 62 is inactive/script-locked while running to its phalanx slot, but
    // Original's global soldier scan still chooses it over Soldier 60.
    let ai = EnemyAi::new(66);
    let mut tick = AiPerTickData::stub();
    tick.fighter_registry.push(FighterSnapshot {
        handle: 66,
        raw_position: position(549.00867, 517.99506),
        is_friendly: true,
        is_archer_unit: true,
        ..FighterSnapshot::default()
    });
    let active_bearer = FighterSnapshot {
        handle: 60,
        position: position(669.8923, 746.43475),
        raw_position: position(669.8923, 746.43475),
        is_friendly: true,
        is_able_to_fight: true,
        is_shield_bearer: true,
        current_substate: Substate::AttackingPhalanx,
        ..FighterSnapshot::default()
    };
    let inactive_bearer = FighterSnapshot {
        handle: 62,
        position: position(484.0, 701.0),
        raw_position: position(484.0, 701.0),
        is_friendly: true,
        is_able_to_fight: false,
        is_shield_bearer: true,
        current_substate: Substate::AttackingRunningToPhalanx,
        ..FighterSnapshot::default()
    };
    tick.fighter_registry.push(active_bearer.clone());
    tick.fighter_registry.push(inactive_bearer);
    // The radius-limited/able-only list omits Soldier 62, which is why it
    // cannot be the backing collection for this Original global scan.
    tick.nearby_fighters.push(active_bearer);
    let ctx = AiContext {
        position: position(549.00867, 517.99506),
        ..AiContext::test_fixture()
    };

    assert_eq!(ai.get_nearest_free_shield_bearer(&ctx, &tick), Some(62));
}

#[test]
fn sober_drunk_combat_gate_preserves_original_draws_and_short_circuit() {
    let two_draw_seed = (0..10_000)
        .find(|seed| {
            let sim = SimulationContext::with_seed(*seed);
            crate::sim_rng::u16(&sim, RngSite::DrunkCombatFreeze, 0..100) != 0
                && crate::sim_rng::u16(&sim, RngSite::DrunkCombatFreeze, 0..100) != 0
        })
        .expect("find a seed whose first two drunk gates do not freeze a sober soldier");
    let sim = SimulationContext::with_seed(two_draw_seed);
    let (freezes, trace) = with_draw_trace(|| drunk_combat_freezes(&sim, 0));
    assert!(!freezes);
    assert_eq!(
        trace,
        vec![RngSite::DrunkCombatFreeze, RngSite::DrunkCombatFreeze],
        "a sober soldier must still consume both Original drunk gates"
    );

    let short_circuit_seed = (0..10_000)
        .find(|seed| {
            let sim = SimulationContext::with_seed(*seed);
            crate::sim_rng::u16(&sim, RngSite::DrunkCombatFreeze, 0..100) == 0
        })
        .expect("find a seed whose first drunk gate freezes a sober soldier");
    let sim = SimulationContext::with_seed(short_circuit_seed);
    let (freezes, trace) = with_draw_trace(|| drunk_combat_freezes(&sim, 0));
    assert!(freezes);
    assert_eq!(
        trace,
        vec![RngSite::DrunkCombatFreeze],
        "a successful first gate must preserve Original || short-circuiting"
    );
}

#[test]
fn swordfight_range_checks_use_original_uword_truncation() {
    assert_eq!(original_uword_norm(MapVec::new(90.7, 0.0)), 90);
    assert_eq!(original_uword_norm(MapVec::new(91.0, 0.0)), 91);
    assert!(original_uword_norm(MapVec::new(90.7, 0.0)) <= 90);
}

#[test]
fn swordfight_facing_guard_uses_ground_positions_before_rng() {
    // Schema-14 task 168 frame 2155: projected map positions misleadingly
    // put PC252 in Soldier137's facing sector because their elevations
    // differ. The original-game ground position puts the PC to the east, so
    // Swordfight reconsideration returns before its combat RNG gates.
    let soldier = position(1720.6782, 1984.8649);
    let pc = position(1749.6063, 1954.4141);
    let soldier_elevation = 17.4139;
    let pc_elevation = 45.0639;

    let projected_sector = vec_to_sector(pc.x - soldier.x, pc.y - soldier.y);
    assert_eq!(projected_sector, 1);
    assert_eq!((1_i32 + 16 - projected_sector as i32) % 16, 0);
    let ground_sector = vec_to_sector(
        pc.x - soldier.x,
        (pc.y + pc_elevation) - (soldier.y + soldier_elevation),
    );
    assert_eq!(ground_sector, 4);
    assert_eq!((1_i32 + 16 - ground_sector as i32) % 16, 13);
    assert!(!is_facing_swordfight_target(
        &soldier,
        soldier_elevation,
        1,
        &pc,
        pc_elevation,
    ));

    let sim = SimulationContext::with_seed(0);
    let (_, trace) = with_draw_trace(|| {
        if is_facing_swordfight_target(&soldier, soldier_elevation, 1, &pc, pc_elevation) {
            let _ = drunk_combat_freezes(&sim, 0);
        }
    });
    assert!(
        trace.is_empty(),
        "the facing return must precede combat RNG"
    );
}

#[test]
fn swordfight_facing_guard_uses_live_position_during_door_pass() {
    // Schema-14 Nescafe Profile_003/Savegame_001 replay-012 frame 1448:
    // The player-character position forecasts the far side of door 95, but the original game's
    // facing guard reads ground position and still sees the live PC.
    let soldier = position(1355.0133, 2248.718);
    let live_pc = position(1307.6046, 2_248.182);
    let forecast_pc = position(1304.0, 2276.0);
    let elevation = 45.0;
    let primary = FighterSnapshot {
        handle: 252,
        position: forecast_pc,
        elevation,
        ..FighterSnapshot::default()
    };
    let mut tick = AiPerTickData::stub();
    tick.primary_target_snapshot_handle = Some(AiEntityHandle::new(252));
    tick.primary_target_live_position = Some(live_pc);

    assert!(!is_facing_swordfight_target(
        &soldier,
        elevation,
        12,
        &primary.position,
        primary.elevation,
    ));
    assert!(is_facing_swordfight_target(
        &soldier,
        elevation,
        12,
        &swordfight_facing_target_position(&primary, &tick, |_| {
            panic!("stable principal must use the tick-captured literal position")
        }),
        primary.elevation,
    ));
}

#[test]
fn swordfight_facing_guard_uses_literal_position_after_principal_refresh() {
    // SuN1Sh1nE Savegame_024 replay-037 frame 922: Position(45)
    // forecast movement north far enough to fail this guard, while the
    // literal ground position of actor 45 remained due east and entered RNG.
    let soldier = position(972.988_8, 2075.3225);
    let forecast = position(1019.0, 2089.0);
    let live = position(1022.0, 2069.0);
    let target_elevation = 6.0522804;
    let refreshed_primary = FighterSnapshot {
        handle: 45,
        position: forecast,
        elevation: target_elevation,
        ..FighterSnapshot::default()
    };
    let mut tick = AiPerTickData::stub();
    tick.primary_target_snapshot_handle = Some(AiEntityHandle::new(152));
    tick.primary_target_live_position = Some(position(1031.0, 2089.0));
    let resolved = swordfight_facing_target_position(&refreshed_primary, &tick, |target| {
        assert_eq!(target, refreshed_primary.handle);
        live
    });

    assert!(!is_facing_swordfight_target(
        &soldier,
        0.0,
        4,
        &forecast,
        target_elevation,
    ));
    assert!(is_facing_swordfight_target(
        &soldier,
        0.0,
        4,
        &resolved,
        target_elevation,
    ));
}

#[test]
fn direct_fighter_lookup_reaches_beyond_nearby_radius_snapshot() {
    let ai = EnemyAi::default();
    let mut tick = AiPerTickData::stub();
    tick.nearby_fighters.push(FighterSnapshot {
        handle: 1,
        ..FighterSnapshot::default()
    });
    tick.fighter_registry.push(FighterSnapshot {
        handle: 2,
        ..FighterSnapshot::default()
    });

    assert_eq!(
        ai.find_fighter(1, &tick).map(|fighter| fighter.handle),
        Some(1)
    );
    assert_eq!(
        ai.find_fighter(2, &tick).map(|fighter| fighter.handle),
        Some(2)
    );
    assert!(ai.find_fighter(3, &tick).is_none());
}

#[test]
fn failed_observation_step_back_panics_without_speaking() {
    let mut ai = EnemyAi::new(91);
    let enemy_pos = position(663.922_5, 2096.012);

    ai.panic_from_position(enemy_pos, parameters_ai::AI_STANDARD_PANIC_RUNS as u8);
    let request = ai.base.outbox.actor.begin_panic.as_ref().unwrap();

    assert_eq!(ai.base.current_state, AiState::Fleeing);
    assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
    assert_eq!(request.center, Some(enemy_pos));
    assert_eq!(request.runs, parameters_ai::AI_STANDARD_PANIC_RUNS as u8);
    assert!(
        ai.base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .all(|work| !matches!(work, AiOwnerWork::Speech(_))),
        "Original's Panic fallback does not call Flee's Say(REMARK_PANIC)"
    );
}
