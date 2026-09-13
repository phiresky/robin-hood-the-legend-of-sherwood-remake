use super::step_back_direction_sector;
use crate::ai_enemy::map_vec_ext::AiMapVec;
use crate::coordinates::MapVec;

#[test]
fn zero_vector_normalization_preserves_original_nan_result() {
    // The shipped game produces NaN when normalizing a zero vector. Schema-14 seed
    // 1000000, linux2 P002 Savegame_029 replay-010 frame 4851 exercises
    // this through swordfight observation reconsideration.
    let zero = std::hint::black_box(0.0_f32);
    let normalized = MapVec::new(zero, zero).iso_normalize(std::hint::black_box(
        crate::position_interface::ASPECT_RATIO,
    ));
    assert_eq!(normalized.x.to_bits(), 0xffc0_0000);
    assert_eq!(normalized.y.to_bits(), 0xffc0_0000);
}

#[test]
fn negative_step_back_offsets_keep_signed_remainder() {
    assert_eq!(step_back_direction_sector(1, -2), 15);
    assert_eq!(step_back_direction_sector(1, -1), 0);
}

#[test]
fn positive_step_back_offsets_keep_source_modulo_fifteen() {
    assert_eq!(step_back_direction_sector(15, 1), 1);
}

#[test]
fn step_back_geometry_honours_swordfight_aspect_ratio() {
    let vertical = MapVec::new(0.0, 40.0);
    assert_eq!(vertical.iso_norm(1.0), 40.0);
    assert!(vertical.iso_norm(crate::position_interface::ASPECT_RATIO) > 69.0);

    let sector = vertical.sector_with_aspect(1.0);
    let direction = MapVec::from_sector_with_aspect(sector, 1.0);
    assert_eq!(sector, 8);
    assert_eq!(direction, MapVec::new(0.0, 1.0));
}
