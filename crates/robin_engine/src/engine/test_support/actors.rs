//! Shared synthetic actors. These constructors supply real allegiance and
//! pathfinder identities; callers still register actors through `add_test_entity`
//! and call `complete_test_runtime_fixture` when a scenario needs live assets.
//! Invalid-data tests should construct their invalid inputs explicitly.

use crate::element::Entity;

/// Low-level actor fixture without a pathfinder, profile, or allegiance.
/// Use `make_test_soldier` for scenarios that need a loaded enemy instead.
pub(crate) fn unbound_soldier(posture: crate::element::Posture) -> crate::element::ActorSoldier {
    let mut element = crate::element::ElementData::from_initial_posture(posture);
    element.kind = crate::element::ElementKind::ActorSoldier;
    crate::element::ActorSoldier {
        element,
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }
}

/// Low-level PC fixture without a pathfinder or campaign profile binding.
pub(crate) fn unbound_pc(posture: crate::element::Posture) -> crate::element::ActorPc {
    let mut element = crate::element::ElementData::from_initial_posture(posture);
    element.kind = crate::element::ElementKind::ActorPc;
    crate::element::ActorPc {
        element,
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    }
}

/// Return stable semantic roles while varying their publication order.
pub(crate) fn add_pair_in_creation_order(
    engine: &mut crate::engine::EngineInner,
    first: Entity,
    second: Entity,
    first_is_earlier: bool,
) -> (crate::element::EntityId, crate::element::EntityId) {
    if first_is_earlier {
        let first_id = engine.add_test_entity(first);
        let second_id = engine.add_test_entity(second);
        (first_id, second_id)
    } else {
        let second_id = engine.add_test_entity(second);
        let first_id = engine.add_test_entity(first);
        (first_id, second_id)
    }
}

/// Build a minimal soldier entity for posture / command tests.
pub(crate) fn make_test_soldier(posture: crate::element::Posture) -> Entity {
    // A directly constructed test soldier stands in for a loaded enemy
    // soldier. Production loading always supplies an allegiance; leaving the
    // sentinel `Camp::Error` here makes unrelated full-engine fixtures
    // invalid as soon as they exercise diplomacy-aware combat scans.
    let soldier_data = crate::element::SoldierData {
        cached_camp: crate::element::Camp::Lacklandists,
        ..Default::default()
    };
    let mut entity = Entity::Soldier(crate::element::ActorSoldier {
        soldier: soldier_data,
        ..unbound_soldier(posture)
    });
    entity
        .position_iface_mut()
        .set_pathfinder_index(crate::position_interface::PathfinderIndex::new(0).unwrap());
    entity
}

/// Build a minimal civilian entity for NPC-translate tests.
pub(crate) fn make_test_civilian(posture: crate::element::Posture) -> Entity {
    // Loaded civilian profiles likewise always provide a real allegiance.
    let civilian_data = crate::element::CivilianData {
        cached_camp: crate::element::Camp::Royalists,
        ..Default::default()
    };
    let mut entity = Entity::Civilian(crate::element::ActorCivilian {
        element: {
            let mut initial_element = crate::element::ElementData::from_initial_posture(posture);
            initial_element.kind = crate::element::ElementKind::ActorCivilian;
            initial_element
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

pub(crate) fn make_test_pc(posture: crate::element::Posture) -> Entity {
    let mut entity = Entity::Pc(unbound_pc(posture));
    entity
        .position_iface_mut()
        .set_pathfinder_index(crate::position_interface::PathfinderIndex::new(0).unwrap());
    entity
}

pub(crate) fn make_test_ai_soldier(camp: crate::element::Camp) -> Entity {
    let mut entity = make_test_soldier(crate::element::Posture::Upright);
    let Entity::Soldier(soldier) = &mut entity else {
        unreachable!("make_test_soldier returned non-soldier");
    };
    soldier.soldier.cached_camp = camp;
    soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    entity
}

#[test]
fn unbound_fixtures_do_not_supply_runtime_bindings() {
    use crate::element::{Camp, ElementKind, Posture};

    let soldier = unbound_soldier(Posture::Flying);
    assert_eq!(soldier.element.posture(), Posture::Flying);
    assert_eq!(soldier.element.kind, ElementKind::ActorSoldier);
    assert!(soldier.element.active);
    assert_eq!(soldier.soldier.cached_camp, Camp::Error);
    assert_eq!(
        soldier.element.sprite.position_iface.get_pathfinder_index(),
        None
    );

    let pc = unbound_pc(Posture::Undefined);
    assert_eq!(pc.element.posture(), Posture::Undefined);
    assert_eq!(pc.element.kind, ElementKind::ActorPc);
    assert!(pc.element.active);
    assert_eq!(
        pc.element.sprite.position_iface.get_pathfinder_index(),
        None
    );

    let loaded = make_test_soldier(Posture::Flying);
    assert!(loaded.position_iface().get_pathfinder_index().is_some());
    let Entity::Soldier(loaded) = loaded else {
        unreachable!()
    };
    assert_eq!(loaded.soldier.cached_camp, Camp::Lacklandists);
}
