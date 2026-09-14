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
        &live_pc,
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
        &live,
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
