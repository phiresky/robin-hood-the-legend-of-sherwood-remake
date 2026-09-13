use super::{ai_square_distance_world, legacy_nearest_door_distance};
use crate::coordinates::WorldPoint3D;

#[test]
fn nearest_door_distance_uses_original_uword_maluses() {
    assert_eq!(legacy_nearest_door_distance(100.9, 90.0, false, false), 100);
    assert_eq!(legacy_nearest_door_distance(65_000.0, 0.0, true, true), 264);
    assert_eq!(
        legacy_nearest_door_distance(65_535.0, 0.0, false, false),
        u16::MAX,
    );
}

#[test]
fn door_endpoint_is_not_substituted_for_literal_body_point() {
    // Arrow protection compares PHALANX_ATTACK_DISTANCE (100) with
    // squared distance to the nearest enemy. A PC can be physically outside that
    // radius while AI Position() has already snapped it to the near door
    // endpoint; Original uses the physical body point.
    let owner = WorldPoint3D::new(0.0, 0.0, 0.0);
    let literal_body = WorldPoint3D::new(101.0, 0.0, 0.0);
    let ai_door_endpoint = WorldPoint3D::new(99.0, 0.0, 0.0);

    assert!(ai_square_distance_world(&literal_body, &owner) >= 100.0 * 100.0);
    assert!(ai_square_distance_world(&ai_door_endpoint, &owner) < 100.0 * 100.0);
}
