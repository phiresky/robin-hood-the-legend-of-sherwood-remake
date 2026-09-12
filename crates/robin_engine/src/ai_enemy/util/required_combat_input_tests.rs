use super::*;

#[test]
fn formation_proposal_round_trip_preserves_live_slot_zero() {
    let position = CombatPosition {
        attacker: Some(AiEntityHandle::new(0)),
        target: Some(AiEntityHandle::new(2)),
        left_neighbour: Some(AiEntityHandle::new(0)),
        right_neighbour: None,
        ..CombatPosition::default()
    };
    let json = serde_json::to_string(&position).unwrap();
    assert!(json.contains(r#""attacker":{"entity":0}"#));
    assert!(json.contains(r#""left_neighbour":{"entity":0}"#));
    let restored: CombatPosition = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.attacker, Some(AiEntityHandle::new(0)));
    assert_eq!(restored.left_neighbour, Some(AiEntityHandle::new(0)));
    assert_eq!(restored.right_neighbour, None);
}

fn combat_position() -> CombatPosition {
    CombatPosition {
        attacker: Some(AiEntityHandle::new(1)),
        target: Some(AiEntityHandle::new(2)),
        attacker_position: Position::default(),
        target_position: Position {
            x: 10.0,
            ..Position::default()
        },
        ..CombatPosition::default()
    }
}

fn fighter(handle: HumanHandle) -> FighterSnapshot {
    FighterSnapshot {
        handle,
        sword_range_maximal: 100,
        hth_weapon_id: 1,
        ..FighterSnapshot::default()
    }
}

fn view(fighters: &[FighterSnapshot]) -> FighterView<'_> {
    FighterView {
        near: fighters,
        registry: &[],
    }
}

#[test]
#[should_panic(expected = "combat position target 2 is absent")]
fn damage_evaluation_rejects_a_missing_selected_target() {
    let fighters = [fighter(1)];
    let mut position = combat_position();
    estimate_damage(
        1,
        &mut position,
        view(&fighters),
        &crate::profiles::ProfileManager::new(),
        50,
    );
}

#[test]
#[should_panic(expected = "fighter 1 requires missing HtH weapon profile 1")]
fn damage_evaluation_rejects_a_missing_required_weapon() {
    let fighters = [fighter(1), fighter(2)];
    let mut position = combat_position();
    estimate_damage(
        1,
        &mut position,
        view(&fighters),
        &crate::profiles::ProfileManager::new(),
        50,
    );
}

#[test]
#[should_panic(expected = "fighter 1 requires missing HtH weapon profile 1")]
fn damage_evaluation_resolves_combatants_through_the_full_registry() {
    // Attacker 1 stands outside the neighbour radius but is still named by
    // the us/them lists, so evaluation must reach it through the registry
    // and get as far as the weapon lookup.
    let near = [fighter(2)];
    let registry = [fighter(1), fighter(2)];
    let mut position = combat_position();
    estimate_damage(
        1,
        &mut position,
        FighterView {
            near: &near,
            registry: &registry,
        },
        &crate::profiles::ProfileManager::new(),
        50,
    );
}

#[test]
fn damage_evaluation_reuses_the_combat_position_cache() {
    let mut position = combat_position();
    position.estimated_damage = 123;
    assert_eq!(
        estimate_damage(
            1,
            &mut position,
            view(&[]),
            &crate::profiles::ProfileManager::new(),
            50,
        ),
        123
    );
}

#[test]
fn damage_protection_uses_live_target_facing_not_proposed_facing() {
    use crate::profiles::{
        HtHWeaponProfile, ThrustProfile, WeaponThrustDirection, WeaponThrustKind,
    };

    let mut weapon = HtHWeaponProfile {
        // Front is unprotected while the left side absorbs 90%. If the
        // proposed facing leaks into protection calculation, this test returns 1
        // damage instead of the Original's 10.
        protection_by_localization: [0, 0, 90, 0, 0],
        ..HtHWeaponProfile::default()
    };
    weapon.thrusts[0] = ThrustProfile {
        kind: WeaponThrustKind::Straight,
        direction: WeaponThrustDirection::NonApplicable,
        cutting: 90,
        maximal_distance: 100,
        ..ThrustProfile::default()
    };
    let mut profiles = crate::profiles::ProfileManager::new();
    profiles.hth_weapons.push(weapon);

    let attacker = FighterSnapshot {
        position: Position {
            y: -10.0,
            ..Position::default()
        },
        ..fighter(1)
    };
    let target = FighterSnapshot {
        // Live facing is sector 0. The hypothetical combat position says
        // sector 4, as combat-position evaluation may do when the defender is
        // expected to turn toward a proposed attacker position.
        direction: 0,
        position: Position::default(),
        ..fighter(2)
    };
    let fighters = [attacker, target];
    let mut position = CombatPosition {
        attacker: Some(AiEntityHandle::new(1)),
        attacker_position: Position::default(),
        target: Some(AiEntityHandle::new(2)),
        target_position: Position {
            x: 10.0,
            ..Position::default()
        },
        target_direction: 4,
        ..CombatPosition::default()
    };

    assert_eq!(
        estimate_damage(1, &mut position, view(&fighters), &profiles, 0),
        10
    );
}

#[test]
fn damage_protection_sector_uses_live_ground_y() {
    use crate::profiles::{
        HtHWeaponProfile, ThrustProfile, WeaponThrustDirection, WeaponThrustKind,
    };

    let mut weapon = HtHWeaponProfile {
        // With Original ground coordinates the attacker is on the
        // defender's protected left. Ignoring elevation instead puts the
        // same two projected map positions directly in front.
        protection_by_localization: [0, 0, 90, 0, 0],
        ..HtHWeaponProfile::default()
    };
    weapon.thrusts[0] = ThrustProfile {
        kind: WeaponThrustKind::Straight,
        direction: WeaponThrustDirection::NonApplicable,
        cutting: 90,
        maximal_distance: 100,
        ..ThrustProfile::default()
    };
    let mut profiles = crate::profiles::ProfileManager::new();
    profiles.hth_weapons.push(weapon);

    let attacker = FighterSnapshot {
        position: Position {
            x: 155.0,
            y: 104.0,
            ..Position::default()
        },
        elevation: 0.0,
        ..fighter(1)
    };
    let target = FighterSnapshot {
        position: Position::default(),
        elevation: 150.0,
        direction: 6,
        ..fighter(2)
    };
    let fighters = [attacker, target];
    let mut position = CombatPosition {
        attacker: Some(AiEntityHandle::new(1)),
        attacker_position: Position::default(),
        target: Some(AiEntityHandle::new(2)),
        target_position: Position {
            x: 10.0,
            ..Position::default()
        },
        ..CombatPosition::default()
    };

    assert_eq!(
        estimate_damage(1, &mut position, view(&fighters), &profiles, 0),
        1
    );
}

#[test]
fn combat_position_score_truncates_distance_before_fractional_penalty() {
    let mut position = CombatPosition {
        attacker_position: Position {
            x: 52.9,
            ..Position::default()
        },
        change_position: true,
        ..CombatPosition::default()
    };
    let mut friends = [];
    let mut enemies = [];
    assert_eq!(
        evaluate_combat_position_full(
            1,
            &Position::default(),
            &[],
            &mut position,
            &mut friends,
            &mut enemies,
            view(&[]),
            &crate::profiles::ProfileManager::new(),
            50,
        ),
        -7
    );
}

#[test]
fn nearest_opponent_keeps_first_fractional_uword_tie() {
    let maurice = FighterSnapshot {
        handle: 10,
        opponent_handles: vec![0, 30],
        ..FighterSnapshot::default()
    };
    let first = FighterSnapshot {
        handle: 0,
        position: Position {
            x: 10.9,
            ..Position::default()
        },
        ..FighterSnapshot::default()
    };
    let fractionally_nearer = FighterSnapshot {
        handle: 30,
        position: Position {
            x: 10.1,
            ..Position::default()
        },
        ..FighterSnapshot::default()
    };
    let fighters = [maurice, first, fractionally_nearer];
    assert_eq!(
        calculate_opponent_nearest_to_rene(
            |handle| fighters.iter().find(|f| f.handle == handle),
            10,
            &Position::default(),
        ),
        Some(AiEntityHandle::new(0)),
    );
}

/// `CalculateOpponentOfMauriceWhoIsNearestToRene` dereferences Maurice's
/// live opponent pointers, so an opponent outside the caller's
/// proximity-limited `nearby_fighters` snapshot still participates. The
/// lookup closure resolves through the complete registry.
#[test]
fn nearest_opponent_resolves_opponents_outside_the_nearby_snapshot() {
    let maurice = FighterSnapshot {
        handle: 10,
        opponent_handles: vec![20, 30],
        ..FighterSnapshot::default()
    };
    let far_but_nearest = FighterSnapshot {
        handle: 30,
        position: Position {
            x: 5.0,
            ..Position::default()
        },
        ..FighterSnapshot::default()
    };
    let near_snapshot_entry = FighterSnapshot {
        handle: 20,
        position: Position {
            x: 40.0,
            ..Position::default()
        },
        ..FighterSnapshot::default()
    };
    // `nearby` deliberately omits handle 30 — only the wider registry
    // knows about it, exactly like a fighter beyond the 500-unit radius.
    let nearby = [maurice.clone(), near_snapshot_entry.clone()];
    let registry = [maurice, near_snapshot_entry, far_but_nearest];
    assert_eq!(
        calculate_opponent_nearest_to_rene(
            |handle| nearby
                .iter()
                .find(|f| f.handle == handle)
                .or_else(|| registry.iter().find(|f| f.handle == handle)),
            10,
            &Position::default(),
        ),
        Some(AiEntityHandle::new(30)),
        "an opponent known only to the complete registry must still win the maximum-norm scan"
    );
}
