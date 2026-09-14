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

use crate::coordinates::WorldPoint3D;
use crate::element::{Camp, Entity};
use crate::engine::EngineInner;

// The kernel adapter borrows actual actors; there is no copied combat roster.
#[derive(Clone, Copy)]
struct Fighters<'a>(&'a EngineInner, &'a crate::profiles::ProfileManager);
impl<'a> Fighters<'a> {
    fn actor(self, handle: u32) -> &'a Entity {
        self.0
            .get_entity(crate::entity_id::EntityId::Soldier(
                crate::entity_id::SoldierId(handle),
            ))
            .unwrap_or_else(|| panic!("missing kernel fighter {handle}"))
    }
}
impl CombatFighterAccess for Fighters<'_> {
    fn position(self, handle: u32) -> Position {
        let actor = self.actor(handle);
        let point = actor.element_data().position_map();
        Position {
            x: point.x,
            y: point.y,
            sector: actor.element_data().sector(),
            level: actor.element_data().layer(),
        }
    }
    fn elevation(self, handle: u32) -> f32 {
        self.actor(handle).element_data().position().z
    }
    fn direction(self, handle: u32) -> u16 {
        self.actor(handle).element_data().direction() as u16
    }
    fn hth_weapon_id(self, handle: u32) -> u32 {
        self.actor(handle).enemy_ai().unwrap().hth_weapon_id
    }
    fn sword_range_maximal(self, handle: u32) -> u16 {
        self.actor(handle).enemy_ai().unwrap().sword_range as u16
    }
    fn fighting_ability(self, handle: u32) -> u16 {
        self.actor(handle);
        0
    }
    fn rank(self, handle: u32) -> ProfileRank {
        self.actor(handle).enemy_ai().unwrap().get_rank(self.1)
    }
    fn is_pc(self, handle: u32) -> bool {
        matches!(self.actor(handle), Entity::Pc(_))
    }
    fn is_friendly(self, handle: u32) -> bool {
        self.actor(handle).friendly_ai().is_some()
    }
}
fn fighters(last: u32) -> EngineInner {
    let mut engine = EngineInner::new();
    for handle in 0..=last {
        let id = engine.add_test_entity(crate::engine::test_support::actors::make_test_ai_soldier(
            Camp::Lacklandists,
        ));
        assert_eq!(id.index(), handle);
        let ai = engine.get_entity_mut(id).unwrap().enemy_ai_mut().unwrap();
        ai.hth_weapon_id = 1;
        ai.sword_range = 100;
        ai.behavior_profile = crate::profiles::SoldierProfileIdx(0);
    }
    engine
}
fn place(engine: &mut EngineInner, handle: u32, x: f32, y: f32, z: f32, direction: i16) {
    let entity = engine
        .get_entity_mut(crate::entity_id::EntityId::Soldier(
            crate::entity_id::SoldierId(handle),
        ))
        .unwrap();
    entity
        .element_data_mut()
        .set_position(WorldPoint3D::new(x, y + z, z));
    entity.element_data_mut().set_direction_instantly(direction);
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
fn profiles() -> crate::profiles::ProfileManager {
    use crate::profiles::{
        HtHWeaponProfile, ThrustProfile, WeaponThrustDirection, WeaponThrustKind,
    };
    let mut weapon = HtHWeaponProfile {
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
    profiles.soldiers.push(crate::profiles::SoldierProfile {
        rank: ProfileRank::Knight,
        ..Default::default()
    });
    profiles
}
#[test]
#[should_panic(expected = "missing kernel fighter 2")]
fn damage_evaluation_rejects_a_missing_selected_target() {
    let profiles = profiles();
    let engine = fighters(1);
    estimate_damage(
        1,
        &mut combat_position(),
        Fighters(&engine, &profiles),
        &profiles,
        50,
    );
}
#[test]
#[should_panic(expected = "fighter 1 requires missing HtH weapon profile 1")]
fn damage_evaluation_rejects_a_missing_required_weapon() {
    let profiles = crate::profiles::ProfileManager::new();
    let engine = fighters(2);
    estimate_damage(
        1,
        &mut combat_position(),
        Fighters(&engine, &profiles),
        &profiles,
        50,
    );
}
#[test]
#[should_panic(expected = "fighter 1 requires missing HtH weapon profile 1")]
fn damage_evaluation_resolves_distant_live_combatants() {
    let profiles = crate::profiles::ProfileManager::new();
    let mut engine = fighters(2);
    place(&mut engine, 1, 10000.0, 10000.0, 0.0, 0);
    estimate_damage(
        1,
        &mut combat_position(),
        Fighters(&engine, &profiles),
        &profiles,
        50,
    );
}
#[test]
fn damage_evaluation_reuses_the_combat_position_cache() {
    let profiles = crate::profiles::ProfileManager::new();
    let engine = EngineInner::new();
    let mut position = combat_position();
    position.estimated_damage = 123;
    assert_eq!(
        estimate_damage(
            1,
            &mut position,
            Fighters(&engine, &profiles),
            &profiles,
            50
        ),
        123
    );
}
#[test]
fn damage_protection_uses_live_target_facing_not_proposed_facing() {
    let profiles = profiles();
    let mut engine = fighters(2);
    place(&mut engine, 1, 0.0, -10.0, 0.0, 0);
    place(&mut engine, 2, 0.0, 0.0, 0.0, 0);
    let mut position = combat_position();
    position.target_direction = 4;
    assert_eq!(
        estimate_damage(1, &mut position, Fighters(&engine, &profiles), &profiles, 0),
        10
    );
}
#[test]
fn damage_protection_sector_uses_live_ground_y() {
    let profiles = profiles();
    let mut engine = fighters(2);
    place(&mut engine, 1, 155.0, 104.0, 0.0, 0);
    place(&mut engine, 2, 0.0, 0.0, 150.0, 6);
    assert_eq!(
        estimate_damage(
            1,
            &mut combat_position(),
            Fighters(&engine, &profiles),
            &profiles,
            0
        ),
        1
    );
}
#[test]
fn combat_position_score_truncates_distance_before_fractional_penalty() {
    let profiles = crate::profiles::ProfileManager::new();
    let engine = EngineInner::new();
    let mut position = CombatPosition {
        attacker_position: Position {
            x: 52.9,
            ..Position::default()
        },
        change_position: true,
        ..CombatPosition::default()
    };
    assert_eq!(
        evaluate_combat_position_full(
            1,
            &Position::default(),
            &[],
            &mut position,
            &mut [],
            &mut [],
            Fighters(&engine, &profiles),
            &profiles,
            50
        ),
        -7
    );
}
