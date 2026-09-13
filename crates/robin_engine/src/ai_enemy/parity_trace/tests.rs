//! Golden byte-level captures of enemy-AI parity payload lines. Replay tooling
//! parses these; any change to the expected strings is a protocol change.

#[test]
fn seekarea_next_point_roll_line_is_byte_stable() {
    let line = super::SeekareaNextPointRoll {
        frame: &7u32,
        owner_handle: &42u32,
        owner_creation_order: &Some(3u32),
        point_id: &1111u32,
        interest: &90u8,
        roll: &12u8,
        accepted: &true,
        remaining: &vec![1111u32, 2222],
    }
    .to_string();
    assert_eq!(
        line,
        r#"SEEKAREA {"event":"next_point_roll","frame":7,"owner_handle":42,"owner_creation_order":Some(3),"point_id":1111,"interest":90,"roll":12,"accepted":true,"remaining":[1111, 2222]}"#
    );
}

#[test]
fn seekarea_next_point_locked_line_is_byte_stable() {
    let line = super::SeekareaNextPointLocked {
        frame: &7u32,
        owner_handle: &42u32,
        owner_creation_order: &None::<u32>,
        point_id: &2222u32,
    }
    .to_string();
    assert_eq!(
        line,
        r#"SEEKAREA {"event":"next_point_locked","frame":7,"owner_handle":42,"owner_creation_order":None,"point_id":2222}"#
    );
}

#[test]
fn seekarea_selection_summary_line_is_byte_stable() {
    let line = super::SeekareaSelectionSummary {
        frame: &8u32,
        owner_handle: &42u32,
        owner_creation_order: &None::<u32>,
        center_x: &1.0f32,
        center_y: &(-2.5f32),
        standard_radius: &300u16,
        near_points: &4usize,
        expected_for_one: &1e-7f32,
        visible_friends: &2u32,
        clears_help: &false,
        expected_before_help_random: &3.25f32,
        expected_points: &5u32,
        phase4_attempts: &6u32,
        phase4_accepts: &7u32,
        preselection_rng_draws: &1u32,
        phase4_rng_draws: &13u32,
        selection_rng_draws: &14u32,
        accepted_interest_sum: &0.1f32,
    }
    .to_string();
    assert_eq!(
        line,
        r#"SEEKAREA {"event":"selection_summary","frame":8,"owner_handle":42,"owner_creation_order":None,"center":[1,-2.5],"standard_radius":300,"near_points":4,"expected_for_one":0.0000001,"visible_friends":2,"clears_help":false,"expected_before_help_random":3.25,"expected_points":5,"phase4_attempts":6,"phase4_accepts":7,"preselection_rng_draws":1,"phase4_rng_draws":13,"selection_rng_draws":14,"accepted_interest_sum":0.1}"#
    );
}

#[test]
fn reconsider_them_candidate_line_is_byte_stable() {
    let line = super::ReconsiderThemCandidate {
        frame: &3u32,
        owner: &42u32,
        fighter: &17u32,
        friendly: &true,
        able: &false,
        distance: &(-0.0f32),
        result: &"not\"detected\n",
    }
    .to_string();
    assert_eq!(
        line,
        r#"RECONSIDER {"event":"them_candidate","frame":3,"owner":42,"fighter":17,"friendly":true,"able":false,"distance":-0,"result":"not\"detected\n"}"#
    );
}

#[test]
fn seekarea_caller_line_is_byte_stable() {
    let line = super::SeekAreaCaller {
        frame: 11,
        owner_handle: 42,
        owner_creation_order: 5,
        caller: "couldnt_reach_emergency",
        stimulus: "event_couldnt_reach_point",
    }
    .to_string();
    assert_eq!(
        line,
        r#"SEEKAREA_CALLER {"frame":11,"owner_handle":42,"owner_creation_order":5,"caller":"couldnt_reach_emergency","stimulus":"event_couldnt_reach_point"}"#
    );
}

#[test]
fn seekarea_phase6_lines_are_byte_stable() {
    let lines = [
        super::SeekAreaPhase6::Phase6Before {
            frame: 11,
            owner_handle: 42,
            owner_creation_order: 5,
            state: 4,
            substate: 37,
            flags: 0x8001,
            seek_direction: 65535,
            list_size: 0,
            list_empty: true,
            location_first: true,
            location_end: false,
            personal1_constructor: "position",
        }
        .to_string(),
        super::SeekAreaPhase6::Phase6Personal1 {
            frame: 11,
            owner_creation_order: 5,
            constructor: "direction",
            inserted_id: 1111,
            list_size: 1,
        }
        .to_string(),
        super::SeekAreaPhase6::Phase6After {
            frame: 11,
            owner_creation_order: 5,
            personal2_inserted: false,
            personal2_constructor: "none",
            list_size: 3,
        }
        .to_string(),
    ];
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
    let line = super::ReconsiderJson::UsCandidate {
        frame: 3,
        owner: 42,
        fighter: 17,
        friendly: true,
        able: true,
        distance_uword: 65535,
        result: "self",
    }
    .to_string();
    assert_eq!(
        line,
        r#"RECONSIDER {"event":"us_candidate","frame":3,"owner":42,"fighter":17,"friendly":true,"able":true,"distance_uword":65535,"result":"self"}"#
    );
}

#[test]
fn archerstep_decision_line_with_named_captures_is_byte_stable() {
    let line = super::ArcherstepDecision {
        frame: &1u32,
        co: &Some(2u32),
        me: &3u32,
        owner_pos: &(1.5f32, -2.0f32),
        animation: &"Walk",
        action_state: &4i32,
        reached_done: &true,
        timer_running: &false,
        timer_ring: &0u32,
        already_on_point: &true,
        old_substate: &"Idle",
        target: &9u32,
        enemy_pos: &None::<(f32, f32)>,
        goal: &Some((0.25f32, 1.0f32)),
    }
    .to_string();
    assert_eq!(
        line,
        r#"[ARCHERSTEP frame=1 co=Some(2) me=3 phase=decision old_substate="Idle" target=9 owner_pos=(1.5, -2.0) enemy_pos=None goal=Some((0.25, 1.0)) animation="Walk" action_state=4 reached_done=true timer_running=false timer_ring=0 already_on_point=true]"#
    );
}

#[test]
fn aidecision_rider_reject_straight_hex_line_is_byte_stable() {
    let line = super::AidecisionRiderCandidateResultRejectStraight {
        frame: &1u32,
        owner: &2u32,
        candidate: &3u32,
        goal_x_bits: &(-2i32),
        goal_y_bits: &0x10u32,
        forward_dot_bits: &1.0f32.to_bits(),
        sq_norm_bits: &0u32,
        cos_bits: &u32::MAX,
    }
    .to_string();
    assert_eq!(
        line,
        r#"AIDECISION frame=1 owner=2 stage=rider_candidate_result candidate=3 result=reject_straight goal=(fffffffe,00000010) forward_dot_bits=3f800000 sq_norm_bits=00000000 cos_bits=ffffffff"#
    );
}
