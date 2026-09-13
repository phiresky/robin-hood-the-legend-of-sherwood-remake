//! Bit-identity of [`AiMapVec`] / `Position::map_point` against the tuple
//! helpers they replaced in `ai_enemy/util.rs`. The `old` module keeps those
//! helpers verbatim (including their `geo2d::dot` / `geo2d::cross` calls);
//! every comparison is on `f32::to_bits`, so NaN payloads, signed zeros and
//! infinities must match too.

use super::AiMapVec;
use crate::ai::Position;
use crate::coordinates::MapVec;
use crate::position_interface::{ASPECT_RATIO, INVERSE_ASPECT_RATIO, SWORDFIGHT_ASPECT_RATIO};

/// The removed tuple helpers, copied unchanged.
mod old {
    use crate::ai::Position;

    pub(super) fn sector_to_vector(sector: u16) -> (f32, f32) {
        let [x, y] = crate::shadow_polygon::sector_to_direction(sector as i16);
        (x, y)
    }

    pub(super) fn dot2(a: (f32, f32), b: (f32, f32)) -> f32 {
        crate::geo2d::dot(crate::geo2d::pt(a.0, a.1), crate::geo2d::pt(b.0, b.1))
    }

    pub(super) fn det2(a: (f32, f32), b: (f32, f32)) -> f32 {
        crate::geo2d::cross(crate::geo2d::pt(a.0, a.1), crate::geo2d::pt(b.0, b.1))
    }

    pub(super) fn max_norm(v: (f32, f32)) -> f32 {
        v.0.abs().max(v.1.abs())
    }

    pub(super) fn square_norm(v: (f32, f32)) -> f32 {
        v.0 * v.0 + v.1 * v.1
    }

    pub(super) fn get_normal(v: (f32, f32)) -> (f32, f32) {
        (-v.1, v.0)
    }

    pub(super) fn get_normal_right(v: (f32, f32)) -> (f32, f32) {
        (v.1, -v.0)
    }

    pub(super) fn pos_diff(a: &Position, b: &Position) -> (f32, f32) {
        (a.x - b.x, a.y - b.y)
    }

    pub(super) fn iso_norm(v: (f32, f32), aspect_ratio: f32) -> f32 {
        let yi = v.1 / aspect_ratio;
        (v.0 * v.0 + yi * yi).sqrt()
    }

    pub(super) fn iso_normalize(v: (f32, f32), aspect_ratio: f32) -> (f32, f32) {
        let norm = iso_norm(v, aspect_ratio);
        (v.0 / norm, v.1 / norm)
    }

    pub(super) fn sector_to_vector_iso(sector: u16, aspect_ratio: f32) -> (f32, f32) {
        let [x, y] = crate::shadow_polygon::sector_to_direction(sector as i16);
        (x, y * aspect_ratio)
    }

    pub(super) fn vec_to_sector_ar(dx: f32, dy: f32, aspect_ratio: f32) -> u16 {
        crate::position_interface::vector_to_sector_0_to_15_with_aspect(dx, dy, aspect_ratio) as u16
    }

    pub(super) fn get_normal_iso(v: (f32, f32), direct: bool, _aspect_ratio: f32) -> (f32, f32) {
        let [x, y] = crate::position_interface::vector_normal_iso(v.0, v.1, direct);
        (x, y)
    }
}

/// Component values spanning signs, zeros, fractions, map-scale and
/// overflow-scale magnitudes, subnormals and non-finite values (NaN is
/// reachable: zero-vector normalization produces it).
const SAMPLES: [f32; 24] = [
    0.0,
    -0.0,
    1.0,
    -1.0,
    0.5,
    -0.5,
    3.25,
    -7.125,
    40.0,
    -40.0,
    123.456,
    -987.654,
    1.0e-7,
    -1.0e-7,
    65535.5,
    -65535.5,
    3.0e30,
    -3.0e30,
    f32::MIN_POSITIVE,
    1.0e-40,
    f32::MAX,
    f32::INFINITY,
    f32::NEG_INFINITY,
    f32::NAN,
];

/// Aspect ratios the call sites pass (standard, sword-fight / default 1.0)
/// plus two more to exercise the division.
const ASPECTS: [f32; 4] = [
    ASPECT_RATIO,
    SWORDFIGHT_ASPECT_RATIO,
    INVERSE_ASPECT_RATIO,
    0.5,
];

fn position(x: f32, y: f32) -> Position {
    Position {
        x,
        y,
        sector: None,
        level: 0,
    }
}

#[track_caller]
fn assert_f32(label: &str, old: f32, new: f32) {
    assert_eq!(
        old.to_bits(),
        new.to_bits(),
        "{label}: old {old:?} ({:#010x}) vs new {new:?} ({:#010x})",
        old.to_bits(),
        new.to_bits()
    );
}

#[track_caller]
fn assert_vec(label: &str, old: (f32, f32), new: MapVec) {
    assert_f32(&format!("{label}.x"), old.0, new.x);
    assert_f32(&format!("{label}.y"), old.1, new.y);
}

#[test]
fn sector_vectors_are_bit_identical() {
    for sector in 0..16u16 {
        assert_vec(
            &format!("from_sector({sector})"),
            old::sector_to_vector(sector),
            MapVec::from_sector(sector),
        );
        for aspect in ASPECTS {
            assert_vec(
                &format!("from_sector_with_aspect({sector}, {aspect})"),
                old::sector_to_vector_iso(sector, aspect),
                MapVec::from_sector_with_aspect(sector, aspect),
            );
        }
        // Standard-aspect call sites use the position_interface table.
        assert_vec(
            &format!("from_sector_iso({sector})"),
            old::sector_to_vector_iso(sector, ASPECT_RATIO),
            MapVec::from_sector_iso(sector),
        );
    }
}

#[test]
fn binary_products_are_bit_identical() {
    for ax in SAMPLES {
        for ay in SAMPLES {
            for bx in SAMPLES {
                for by in SAMPLES {
                    let (a, b) = ((ax, ay), (bx, by));
                    let label = format!("{a:?}, {b:?}");
                    assert_f32(
                        &format!("dot({label})"),
                        old::dot2(a, b),
                        MapVec::new(ax, ay).dot(MapVec::new(bx, by)),
                    );
                    assert_f32(
                        &format!("det({label})"),
                        old::det2(a, b),
                        MapVec::new(ax, ay).det(MapVec::new(bx, by)),
                    );
                }
            }
        }
    }
}

#[test]
fn unary_helpers_are_bit_identical() {
    for x in SAMPLES {
        for y in SAMPLES {
            let v = (x, y);
            let new = MapVec::new(x, y);
            assert_f32(&format!("max_norm{v:?}"), old::max_norm(v), new.max_norm());
            assert_f32(
                &format!("square_norm{v:?}"),
                old::square_norm(v),
                new.square_norm(),
            );
            assert_vec(
                &format!("normal_left{v:?}"),
                old::get_normal(v),
                new.normal_left(),
            );
            assert_vec(
                &format!("normal_right{v:?}"),
                old::get_normal_right(v),
                new.normal_right(),
            );
            for direct in [true, false] {
                for aspect in ASPECTS {
                    assert_vec(
                        &format!("normal_iso{v:?} direct={direct}"),
                        old::get_normal_iso(v, direct, aspect),
                        new.normal_iso(direct),
                    );
                }
            }
            for aspect in ASPECTS {
                assert_f32(
                    &format!("iso_norm{v:?} ar={aspect}"),
                    old::iso_norm(v, aspect),
                    new.iso_norm(aspect),
                );
                assert_vec(
                    &format!("iso_normalize{v:?} ar={aspect}"),
                    old::iso_normalize(v, aspect),
                    new.iso_normalize(aspect),
                );
                assert_eq!(
                    old::vec_to_sector_ar(x, y, aspect),
                    new.sector_with_aspect(aspect),
                    "sector_with_aspect{v:?} ar={aspect}"
                );
            }
        }
    }
}

#[test]
fn position_difference_is_bit_identical() {
    for ax in SAMPLES {
        for ay in SAMPLES {
            for bx in SAMPLES {
                for by in [ay, -ay, 0.0, f32::NAN] {
                    let a = position(ax, ay);
                    let b = position(bx, by);
                    assert_vec(
                        &format!("pos_diff({ax}, {ay}; {bx}, {by})"),
                        old::pos_diff(&a, &b),
                        a.map_point() - b.map_point(),
                    );
                }
            }
        }
    }
}
