//! Shared synthetic actors. These constructors supply real allegiance and
//! pathfinder identities; callers still register actors through `add_entity`
//! and call `complete_test_runtime_fixture` when a scenario needs live assets.
//! Invalid-data tests should construct their invalid inputs explicitly.

use super::*;

/// Build a minimal soldier entity for posture / command tests.
pub(super) fn make_test_soldier(posture: crate::element::Posture) -> Entity {
    let mut soldier_data = crate::element::SoldierData::default();
    // A directly constructed test soldier stands in for a loaded enemy
    // soldier. Production loading always supplies an allegiance; leaving the
    // sentinel `Camp::Error` here makes unrelated full-engine fixtures
    // invalid as soon as they exercise diplomacy-aware combat scans.
    soldier_data.cached_camp = crate::element::Camp::Lacklandists;
    let mut entity = Entity::Soldier(crate::element::ActorSoldier {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorSoldier,
            posture,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: soldier_data,
    });
    entity
        .position_iface_mut()
        .set_pathfinder_index(crate::position_interface::PathfinderIndex::new(0).unwrap());
    entity
}

/// Build a minimal civilian entity for NPC-translate tests.
pub(super) fn make_test_civilian(posture: crate::element::Posture) -> Entity {
    let mut civilian_data = crate::element::CivilianData::default();
    // Loaded civilian profiles likewise always provide a real allegiance.
    civilian_data.cached_camp = crate::element::Camp::Royalists;
    let mut entity = Entity::Civilian(crate::element::ActorCivilian {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorCivilian,
            posture,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        civilian: civilian_data,
    });
    entity
        .position_iface_mut()
        .set_pathfinder_index(crate::position_interface::PathfinderIndex::new(0).unwrap());
    entity
}

pub(super) fn make_test_pc(posture: crate::element::Posture) -> Entity {
    let mut entity = Entity::Pc(crate::element::ActorPc {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorPc,
            posture,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });
    entity
        .position_iface_mut()
        .set_pathfinder_index(crate::position_interface::PathfinderIndex::new(0).unwrap());
    entity
}

pub(in crate::engine) fn make_test_ai_soldier(camp: crate::element::Camp) -> Entity {
    let mut entity = make_test_soldier(crate::element::Posture::Upright);
    let Entity::Soldier(soldier) = &mut entity else {
        unreachable!("make_test_soldier returned non-soldier");
    };
    soldier.soldier.cached_camp = camp;
    soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    entity
}
