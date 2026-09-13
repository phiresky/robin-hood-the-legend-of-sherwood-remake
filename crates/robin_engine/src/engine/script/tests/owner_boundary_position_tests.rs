use super::*;
use crate::element::{ElementData, ElementProjectile, ObjectData, ProjectileData};

fn projectile_with_layer(layer: Option<u16>) -> Entity {
    let mut element = {
        let mut initial_element = ElementData::default();
        initial_element.active = true;
        initial_element
    };
    element.set_position_map(crate::coordinates::MapPoint::new(12.0, 34.0));
    match layer {
        Some(layer) => element.set_layer(layer),
        None => element.clear_layer(),
    }
    Entity::Projectile(ElementProjectile {
        element,
        object: ObjectData::default(),
        projectile: ProjectileData::default(),
    })
}

#[test]
fn raw_owner_boundary_snapshot_preserves_no_layer_entities_without_ai_projection() {
    let mut engine = EngineInner::new();
    let detached = engine.add_test_entity(projectile_with_layer(None));
    let placed = engine.add_test_entity(projectile_with_layer(Some(3)));

    let positions =
        collect_raw_owner_boundary_positions(&engine, [detached.index(), placed.index()]);
    let detached_raw = positions
        .get(&detached.index())
        .copied()
        .expect("detached projectile remains visible to raw mutation tracking");
    assert_eq!(detached_raw.2, None);
    assert_eq!(raw_owner_boundary_position_to_ai(detached_raw), None);

    let placed_position = positions
        .get(&placed.index())
        .copied()
        .and_then(raw_owner_boundary_position_to_ai)
        .expect("placed projectile has an AI-compatible boundary position");
    assert_eq!(placed_position.level, 3);
    assert_eq!((placed_position.x, placed_position.y), (12.0, 34.0));

    let ai_boundary_positions = collect_raw_owner_boundary_positions(&engine, [placed.index()]);
    assert!(!ai_boundary_positions.contains_key(&detached.index()));
}
