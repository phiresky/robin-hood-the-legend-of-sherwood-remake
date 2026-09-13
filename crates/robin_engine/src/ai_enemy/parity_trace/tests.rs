//! Golden byte-level captures of enemy-AI parity payload lines. Replay tooling
//! parses these; any change to the expected strings is a protocol change.

use crate::ai::parity_trace::tests::capture;

#[test]
fn seekarea_next_point_roll_line_is_byte_stable() {
    let lines = capture(|| {
        super::seekarea_next_point_roll(
            &7u32,
            &42u32,
            &Some(3u32),
            &1111u32,
            &90u8,
            &12u8,
            &true,
            &vec![1111u32, 2222],
        )
    });
    assert_eq!(
        lines,
        [
            r#"SEEKAREA {"event":"next_point_roll","frame":7,"owner_handle":42,"owner_creation_order":Some(3),"point_id":1111,"interest":90,"roll":12,"accepted":true,"remaining":[1111, 2222]}"#
        ]
    );
}

#[test]
fn seekarea_next_point_locked_line_is_byte_stable() {
    let lines =
        capture(|| super::seekarea_next_point_locked(&7u32, &42u32, &None::<u32>, &2222u32));
    assert_eq!(
        lines,
        [
            r#"SEEKAREA {"event":"next_point_locked","frame":7,"owner_handle":42,"owner_creation_order":None,"point_id":2222}"#
        ]
    );
}

#[test]
fn seekarea_selection_summary_line_is_byte_stable() {
    let lines = capture(|| {
        super::seekarea_selection_summary(
            &8u32,
            &42u32,
            &None::<u32>,
            &1.0f32,
            &(-2.5f32),
            &300u16,
            &4usize,
            &1e-7f32,
            &2u32,
            &false,
            &3.25f32,
            &5u32,
            &6u32,
            &7u32,
            &1u32,
            &13u32,
            &14u32,
            &0.1f32,
        )
    });
    assert_eq!(
        lines,
        [
            r#"SEEKAREA {"event":"selection_summary","frame":8,"owner_handle":42,"owner_creation_order":None,"center":[1,-2.5],"standard_radius":300,"near_points":4,"expected_for_one":0.0000001,"visible_friends":2,"clears_help":false,"expected_before_help_random":3.25,"expected_points":5,"phase4_attempts":6,"phase4_accepts":7,"preselection_rng_draws":1,"phase4_rng_draws":13,"selection_rng_draws":14,"accepted_interest_sum":0.1}"#
        ]
    );
}

#[test]
fn reconsider_them_candidate_line_is_byte_stable() {
    let lines = capture(|| {
        super::reconsider_them_candidate(
            &3u32,
            &42u32,
            &17u32,
            &true,
            &false,
            &(-0.0f32),
            &"not\"detected\n",
        )
    });
    assert_eq!(
        lines,
        [
            r#"RECONSIDER {"event":"them_candidate","frame":3,"owner":42,"fighter":17,"friendly":true,"able":false,"distance":-0,"result":"not\"detected\n"}"#
        ]
    );
}

#[test]
fn seekarea_caller_line_is_byte_stable() {
    let lines = capture(|| super::seekarea_caller_couldnt_reach_emergency(&11u32, &42u32, &5u32));
    assert_eq!(
        lines,
        [
            r#"SEEKAREA_CALLER {"frame":11,"owner_handle":42,"owner_creation_order":5,"caller":"couldnt_reach_emergency","stimulus":"event_couldnt_reach_point"}"#
        ]
    );
}

#[test]
fn seekarea_phase6_lines_are_byte_stable() {
    let lines = capture(|| {
        super::seekarea_phase6_before(
            &11u32,
            &42u32,
            &5u32,
            &4u32,
            &37u32,
            &0x8001u16,
            &65535u16,
            &0usize,
            &true,
            &true,
            &false,
            &"position",
        );
        super::seekarea_phase6_personal1(&11u32, &5u32, &"direction", &1usize);
        super::seekarea_phase6_after(&11u32, &5u32, &false, &"none", &3usize);
    });
    assert_eq!(
        lines,
        [
            r#"SEEKAREA {"event":"phase6_before","frame":11,"owner_handle":42,"owner_creation_order":5,"state":4,"substate":37,"flags":32769,"seek_direction":65535,"list_size":0,"list_empty":true,"location_first":true,"location_end":false,"personal1_constructor":"position"}"#,
            r#"SEEKAREA {"event":"phase6_personal1","frame":11,"owner_creation_order":5,"constructor":"direction","inserted_id":1111,"list_size":1}"#,
            r#"SEEKAREA {"event":"phase6_after","frame":11,"owner_creation_order":5,"personal2_inserted":false,"personal2_constructor":"none","list_size":3}"#,
        ]
    );
}

#[test]
fn reconsider_us_candidate_line_is_byte_stable() {
    let lines = capture(|| {
        super::reconsider_us_candidate(&3u32, &42u32, &17u32, &true, &true, &65535u16, &"self")
    });
    assert_eq!(
        lines,
        [
            r#"RECONSIDER {"event":"us_candidate","frame":3,"owner":42,"fighter":17,"friendly":true,"able":true,"distance_uword":65535,"result":"self"}"#
        ]
    );
}

#[test]
fn archerstep_decision_line_with_named_captures_is_byte_stable() {
    let lines = capture(|| {
        super::archerstep_decision(
            &1u32,
            &Some(2u32),
            &3u32,
            &(1.5f32, -2.0f32),
            &"Walk",
            &4i32,
            &true,
            &false,
            &0u32,
            &true,
            &"Idle",
            &9u32,
            &None::<(f32, f32)>,
            &Some((0.25f32, 1.0f32)),
        )
    });
    assert_eq!(
        lines,
        [
            r#"[ARCHERSTEP frame=1 co=Some(2) me=3 phase=decision old_substate="Idle" target=9 owner_pos=(1.5, -2.0) enemy_pos=None goal=Some((0.25, 1.0)) animation="Walk" action_state=4 reached_done=true timer_running=false timer_ring=0 already_on_point=true]"#
        ]
    );
}

#[test]
fn aidecision_rider_reject_straight_hex_line_is_byte_stable() {
    let lines = capture(|| {
        super::aidecision_rider_candidate_result_reject_straight(
            &1u32,
            &2u32,
            &3u32,
            &(-2i32),
            &0x10u32,
            &1.0f32.to_bits(),
            &0u32,
            &u32::MAX,
        )
    });
    assert_eq!(
        lines,
        [
            r#"AIDECISION frame=1 owner=2 stage=rider_candidate_result candidate=3 result=reject_straight goal=(fffffffe,00000010) forward_dot_bits=3f800000 sq_norm_bits=00000000 cos_bits=ffffffff"#
        ]
    );
}
