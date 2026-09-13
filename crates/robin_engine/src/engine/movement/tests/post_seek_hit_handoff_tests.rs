use super::*;

#[test]
fn hit_init_range_uses_raw_map_square_norm_and_includes_exact_boundary() {
    let owner = MapPoint::new(1_099.375_2, 1_823.835_4);
    let nescafe_target = MapPoint::new(1055.0, 1790.0);
    assert!(interaction_exceeds_init_range(owner, nescafe_target));

    let owner = MapPoint::new(1_483.855_8, 2720.03);
    let cyrdach_target = MapPoint::new(1470.0, 2759.0);
    assert!(interaction_exceeds_init_range(owner, cyrdach_target));

    assert!(!interaction_exceeds_init_range(
        MapPoint::ZERO,
        MapPoint::new(40.0, 0.0)
    ));
    assert!(interaction_exceeds_init_range(
        MapPoint::ZERO,
        MapPoint::new(f32::from_bits(40.0f32.to_bits() + 1), 0.0)
    ));
}
