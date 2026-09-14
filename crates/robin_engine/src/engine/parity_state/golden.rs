//! Byte-exact golden snapshots of the public parity projection JSON.
//!
//! Guards the typed-projection refactor (review 01/F3): every projection a
//! fixture test builds is compared byte-for-byte against the committed
//! `serde_json::to_string` output under `parity_state/golden/`. Regenerate
//! intentionally changed snapshots with `ROBIN_BLESS_PARITY_GOLDEN=1`.

use std::path::PathBuf;

fn golden_path(label: &str) -> PathBuf {
    assert!(
        label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
        "golden label {label:?} must be a plain file stem"
    );
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/engine/parity_state/golden")
        .join(format!("{label}.json"))
}

pub(super) fn assert_golden(label: &str, value: &serde_json::Value) {
    let actual = serde_json::to_string(value).expect("parity projection must serialize");
    let path = golden_path(label);
    if std::env::var_os("ROBIN_BLESS_PARITY_GOLDEN").is_some() {
        std::fs::write(&path, format!("{actual}\n"))
            .unwrap_or_else(|error| panic!("write golden {}: {error}", path.display()));
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "missing golden {} ({error}); rerun with ROBIN_BLESS_PARITY_GOLDEN=1",
            path.display()
        )
    });
    assert!(
        expected.strip_suffix('\n') == Some(actual.as_str()),
        "parity projection {label} differs from golden {}\nexpected: {}\nactual:   {actual}",
        path.display(),
        expected.trim_end()
    );
}

/// Covers the resolved-geometry branches (sector, door, sight obstacle,
/// target, jump line, AI door/handles, building sector) that the per-schema
/// oracle fixtures leave empty.
#[test]
fn soldier_geometry_references_match_golden() {
    use super::*;
    use crate::element::{ActorSoldier, AiBrain, ElementData, ElementKind, Entity, NpcData};
    use crate::sight_obstacle::SightObstacle;

    let mut inner = EngineInner::new();
    let mut assets = LevelAssets::new();
    assets.environment.static_sight_obstacles = std::sync::Arc::new(vec![
        SightObstacle::new_default(10),
        SightObstacle::new(42, crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA),
    ]);
    let sector = crate::engine::test_support::ensure_ordinary_sector(&mut inner, 31, 2);
    let mut door = crate::gate::Door::default();
    door.gate_type = crate::gate::GateType::Jump;
    door.point_out = crate::coordinates::MapPoint::new(-0.0, 2.5);
    door.point_in = crate::coordinates::MapPoint::new(13.0, -9.0);
    inner.script_domains.interactables.doors.push(door);
    std::sync::Arc::make_mut(&mut inner.world.fast_grid)
        .level_mut()
        .jump_lines
        .push(crate::jump_line::JumpLine::new(
            crate::coordinates::MapPoint::new(1.0, 2.0),
            crate::coordinates::MapPoint::new(3.0, 4.0),
            0.0,
            0.0,
        ));
    let mut target_element = ElementData::default();
    target_element.kind = ElementKind::Fx;
    let target = inner.add_test_entity(Entity::Fx(crate::element::ElementFx {
        element: target_element,
        fx: Default::default(),
    }));

    let mut enemy = Box::<crate::ai_enemy::EnemyAi>::default();
    enemy.my_line_jump = Some(0);
    enemy.base.my_door_index = crate::gate::DoorIndex::new(0);
    enemy.base.primary_target = Some(crate::ai::AiEntityHandle::new(target.index()));
    enemy.base.patrol_chief = Some(target);
    let mut element = ElementData::default();
    element.kind = ElementKind::ActorSoldier;
    element.set_position_map(crate::coordinates::MapPoint::new(12.5, -3.25));
    element.set_sector(Some(sector));
    element.set_obstacle_index(
        crate::position_interface::ObstacleHandle::new(1),
        Some(crate::position_interface::PlaneZCoeffs {
            az: 0.0,
            bz: 0.0,
            dz: 0.0,
        }),
    );
    element
        .sprite
        .position_iface
        .set_door_for_test(crate::gate::DoorIndex::new(0).expect("door zero"));
    element
        .sprite
        .position_iface
        .set_target_element(Some(target));
    let mut npc = NpcData::default();
    npc.ai.ai_brain = AiBrain::Enemy(enemy);
    let mut human = crate::element::HumanData::default();
    human.building_sector = Some(sector);
    let id = inner.add_test_entity(Entity::Soldier(ActorSoldier {
        element,
        actor: Default::default(),
        human,
        npc,
        soldier: crate::element::SoldierData {
            cached_camp: crate::element::Camp::Lacklandists,
            ..Default::default()
        },
    }));
    let engine = Engine {
        inner,
        bootstrap_open: false,
    };
    assert_golden(
        "entity_soldier_geometry",
        &engine.parity_entity_runtime_state(id, &assets),
    );
}
