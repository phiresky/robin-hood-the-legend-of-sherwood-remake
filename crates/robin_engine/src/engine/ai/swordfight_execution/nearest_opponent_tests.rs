use super::*;
use crate::coordinates::WorldPoint3D;
use crate::engine::test_support::actors::make_test_ai_soldier;

fn fighter(engine: &mut EngineInner, x: f32) -> EntityId {
    let id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    engine
        .get_entity_mut(id)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D::new(x, 0.0, 0.0));
    id
}

#[test]
fn nearest_opponent_keeps_first_fractional_uword_tie_including_slot_zero() {
    let mut engine = EngineInner::new();
    let first = fighter(&mut engine, 10.9);
    assert_eq!(first.index(), 0);
    let maurice = fighter(&mut engine, 2000.0);
    let second = fighter(&mut engine, 10.1);
    let rene = fighter(&mut engine, 0.0);
    let opponents = &mut engine
        .get_entity_mut(maurice)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents;
    opponents.add_principal(second, None);
    opponents.add_principal(first, None);
    assert_eq!(engine.nearest_live_opponent(maurice, rene), Some(first));
}

#[test]
fn nearest_opponent_reads_all_live_opponents_outside_owner_neighbour_radius() {
    let mut engine = EngineInner::new();
    let maurice = fighter(&mut engine, 2000.0);
    let first = fighter(&mut engine, 40.0);
    let nearest = fighter(&mut engine, 5.0);
    let rene = fighter(&mut engine, 0.0);
    let opponents = &mut engine
        .get_entity_mut(maurice)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents;
    opponents.add_principal(first, None);
    opponents.add_principal(nearest, None);
    assert_eq!(engine.nearest_live_opponent(maurice, rene), Some(nearest));
    engine
        .get_entity_mut(nearest)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D::new(50.0, 0.0, 0.0));
    assert_eq!(engine.nearest_live_opponent(maurice, rene), Some(first));
}
