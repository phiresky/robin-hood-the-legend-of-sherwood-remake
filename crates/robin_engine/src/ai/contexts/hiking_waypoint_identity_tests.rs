use super::*;

#[test]
fn waypoint_route_position_carries_exact_arena_identity() {
    let exact = crate::position_interface::SectorHandle::new(82)
        .unwrap()
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(17).unwrap());
    let ctx = AiContext {
        hiking_waypoint_sectors: Some(Arc::new(vec![vec![exact]])),
        ..AiContext::test_fixture()
    };

    let route_position = Position {
        x: 1432.0,
        y: 930.0,
        sector: ctx.hiking_waypoint_sector(0, 0, 82),
        level: 6,
    };

    assert_eq!(route_position.sector.unwrap().get(), 82);
    assert_eq!(
        route_position.sector.unwrap().arena_index(),
        crate::fast_find_grid::SectorIndex::new(17)
    );
}
