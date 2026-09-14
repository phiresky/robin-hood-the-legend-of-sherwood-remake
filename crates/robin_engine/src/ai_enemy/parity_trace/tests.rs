//! Golden byte-level captures of enemy-AI parity payload lines. Replay tooling
//! parses these; any change to the expected strings is a protocol change.

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
