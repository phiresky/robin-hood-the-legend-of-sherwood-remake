use super::*;

#[test]
fn seek_point_interest_accumulator_narrows_once_after_double_arithmetic() {
    let accumulated = accumulate_seek_point_interest(0.0, 10);
    let all_f32 = 10.0_f32 * 0.01_f32;

    assert_eq!(accumulated.to_bits(), 0x3dcc_cccd);
    assert_eq!(all_f32.to_bits(), 0x3dcc_cccc);
}

#[test]
fn seek_direction_delta_preserves_original_uword_wrap() {
    assert_eq!(legacy_seek_direction_delta(0, 17), u16::MAX);
    assert_eq!(legacy_seek_direction_delta(14, 0), 30);
}

#[test]
fn seek_point_interest_accumulator_preserves_original_threshold_crossing() {
    let interests = [55, 19, 32, 44, 1, 44, 27, 97, 56, 15, 83, 26, 14, 0, 56, 31];
    let accumulated = interests
        .into_iter()
        .fold(0.0, accumulate_seek_point_interest);
    let all_f32 = interests.into_iter().fold(0.0_f32, |current, interest| {
        current + f32::from(interest) * 0.01_f32
    });

    assert_eq!(accumulated.to_bits(), 0x40c0_0001);
    assert_eq!(all_f32.to_bits(), 0x40bf_ffff);
    assert!(accumulated >= 6.0);
    assert!(all_f32 < 6.0);
}

#[test]
fn seek_point_interest_accumulator_keeps_exact_threshold_control() {
    let accumulated = [50, 50]
        .into_iter()
        .fold(0.0, accumulate_seek_point_interest);

    assert_eq!(accumulated, 1.0);
}

#[test]
fn area_candidates_preserve_distance_ties_and_strict_radius_boundaries() {
    let global = AiGlobalState {
        seek_points: [(10.0, 0), (-10.0, 0), (1_000.0, 0), (0.0, 1)]
            .into_iter()
            .enumerate()
            .map(|(index, (x, level))| SeekPoint {
                position: Position {
                    x,
                    level,
                    ..Position::default()
                },
                frame_when_full_interest: 0,
                directions: vec![],
                last_calculated_interest: 100,
                locked: false,
                id: index as u16,
            })
            .collect(),
        ..Default::default()
    };
    let candidates = SeekAreaCandidates::new(
        SeekAreaSpec {
            center: Position::default(),
            standard_radius: 10,
            flags: SeekFlags::empty(),
            seek_direction: vec_to_sector(10.0, 0.0),
        },
        &global,
    );

    // Equal distances keep global-array order, including the layer penalty.
    assert_eq!(candidates.square_norms, [100.0, 100.0, 1_000_000.0, 100.0]);
    assert_eq!(candidates.near_sorted, [0, 1, 3]);
    // Exactly on the standard radius does not increase the initial count.
    assert_eq!(candidates.expected_points_for_one, 1);
    assert_eq!(candidates.obligatory_idx, Some(0));
}

#[test]
fn area_global_selection_keeps_first_insertion_draw_and_obligatory_duplicate() {
    use crate::sim_rng::{RngSite, with_draw_trace};

    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(118);
    let mut global = AiGlobalState {
        seek_points: vec![SeekPoint {
            position: Position {
                x: 10.0,
                ..Position::default()
            },
            frame_when_full_interest: 0,
            directions: vec![],
            last_calculated_interest: 7,
            locked: false,
            id: 0,
        }],
        ..Default::default()
    };
    let spec = SeekAreaSpec {
        center: Position::default(),
        standard_radius: 100,
        flags: SeekFlags::empty(),
        seek_direction: vec_to_sector(10.0, 0.0),
    };
    let (_, draws) = with_draw_trace(|| {
        ai.append_global_area_seek_points(&sim, 500, None, 0, false, spec, &mut global);
    });

    assert_eq!(
        draws,
        [RngSite::SeekPointSelection, RngSite::SeekPointSelection]
    );
    assert_eq!(ai.my_seek_points, [0, 0]);
    assert_eq!(global.seek_points[0].last_calculated_interest, 100);
}
