use super::legacy_nearest_door_distance;

#[test]
fn nearest_door_distance_uses_original_uword_maluses() {
    assert_eq!(legacy_nearest_door_distance(100.9, 90.0, false, false), 100);
    assert_eq!(legacy_nearest_door_distance(65_000.0, 0.0, true, true), 264);
    assert_eq!(
        legacy_nearest_door_distance(65_535.0, 0.0, false, false),
        u16::MAX,
    );
}
