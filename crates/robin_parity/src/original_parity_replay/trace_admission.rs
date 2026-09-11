//! Schema admission and JSON decoding, independent of comparison policy.
use super::{
    BTreeMap, OLDEST_SUPPORTED_TRACE_SCHEMA, TRACE_SCHEMA_VERSION, TraceEntityId, TraceFrame,
    TraceHeader, TraceHuman, TraceRecordMarker, TraceStartState,
};

pub(super) fn validate_trace_schema(schema: u32) {
    assert!(
        trace_schema_is_supported(schema),
        "unsupported parity trace schema {schema}; schemas {OLDEST_SUPPORTED_TRACE_SCHEMA} through {TRACE_SCHEMA_VERSION} are supported"
    );
}

pub(super) fn trace_schema_is_supported(schema: u32) -> bool {
    (OLDEST_SUPPORTED_TRACE_SCHEMA..=TRACE_SCHEMA_VERSION).contains(&schema)
}

pub(super) fn validate_trace_header(header: &TraceHeader) {
    validate_trace_schema(header.schema);
    assert_eq!(
        header.record_type, "header",
        "invalid parity header record type"
    );
    assert_eq!(
        header.simulation_hz, 25,
        "parity replay requires the Original's 25 Hz simulation"
    );
    assert_eq!(
        header.rng_stream, "libc_rand_raw_global_draw_order",
        "unsupported parity RNG stream"
    );
    assert_eq!(
        header.visibility_queries, "opaque_is_reachable",
        "unsupported parity visibility-query contract"
    );
}

pub(super) fn decode_and_validate_initial_save(header: &TraceHeader) -> Option<Vec<u8>> {
    match (header.start_state, header.initial_save.as_ref()) {
        (TraceStartState::MissionStart, None) => None,
        (TraceStartState::MissionStart, Some(_)) => {
            panic!("mission_start traces must not contain initial_save")
        }
        (TraceStartState::LoadedSave, None) => {
            panic!("loaded_save traces require initial_save")
        }
        (TraceStartState::LoadedSave, Some(initial_save)) => {
            let mission_index = header
                .campaign
                .current_mission_index
                .expect("loaded_save campaign has no current mission");
            let mission = header
                .campaign
                .missions
                .get(mission_index)
                .unwrap_or_else(|| {
                    panic!("loaded_save current mission index {mission_index} is out of range")
                });
            Some(
                initial_save
                    .decode_and_validate(mission.profile_id)
                    .unwrap_or_else(|error| panic!("invalid initial_save: {error}")),
            )
        }
    }
}

pub(super) fn validate_trace_start(
    start_state: TraceStartState,
    session_index: u32,
    initial_frame: u64,
) {
    match start_state {
        TraceStartState::MissionStart => assert_eq!(
            initial_frame, 0,
            "parity session {session_index} is marked mission_start but begins at frame {initial_frame}"
        ),
        // A loaded automatic mission-start save is reconstructible from the
        // recorded campaign/config/RNG prefix and the ordinary mission
        // loader. Do not reject loaded sessions solely because of their
        // provenance: the normal setup-draw and first-frame isomorphic state
        // comparisons below remain authoritative and fail loudly for a
        // genuinely mid-mission save whose live state is not represented by
        // the header.
        TraceStartState::LoadedSave => {}
    }
}

pub(super) fn validate_jump_line_shapes(frame: &TraceFrame, legacy_additive_omissions: bool) {
    for element in &frame.elements {
        let Some(human) = element.human.as_ref() else {
            continue;
        };
        validate_human_jump_line_shape(&element.entity_id, human, legacy_additive_omissions);
    }
}

pub(super) fn validate_human_jump_line_shape(
    entity: &TraceEntityId,
    human: &TraceHuman,
    legacy_additive_omissions: bool,
) {
    // The opponent jump-line diagnostic was added after the first schema-16
    // recordings. Its v66 compatibility representation preserves absence as
    // `None`, but the original v66 -> current migration flattened that to an
    // empty vector before archival reblocking. Such a vector means "not
    // recorded" only when the header also lacks `initial_npc_transients`,
    // which was introduced later than the jump-line diagnostic. Do not invent
    // null jump lines: comparison already skips all unavailable additive
    // human fields for that legacy header generation. Modern recordings, and
    // legacy recordings carrying any jump-line entry, remain strict.
    //
    // Original emits exactly one JumpLineState slot per opponent
    // including JSON null for no line.
    if legacy_additive_omissions && human.opponent_jump_lines.is_empty() {
        return;
    }
    assert_eq!(
        human.opponents.len(),
        human.opponent_jump_lines.len(),
        "schema-{TRACE_SCHEMA_VERSION} {entity:?} opponent and jump-line arrays differ in length"
    );
}

pub(super) fn validate_sequence_diagnostic_order(frame: &TraceFrame) {
    let proposals = &frame.strike_proposal_events;
    let lifecycle = &frame.sequence_lifecycle_events;
    for (expected, event) in proposals.iter().enumerate() {
        assert_eq!(
            event.ordinal, expected as u64,
            "schema-{TRACE_SCHEMA_VERSION} strike-proposal event ordinals are not contiguous"
        );
    }
    for (expected, event) in lifecycle.iter().enumerate() {
        assert_eq!(
            event.ordinal, expected as u64,
            "schema-{TRACE_SCHEMA_VERSION} sequence-lifecycle event ordinals are not contiguous"
        );
    }
    for (expected, event) in frame.target_lifecycle_events.iter().enumerate() {
        assert_eq!(
            event.ordinal, expected as u64,
            "schema-{TRACE_SCHEMA_VERSION} target-lifecycle event ordinals are not contiguous"
        );
    }

    let mut frame_ordinals = proposals
        .iter()
        .map(|event| event.frame_ordinal)
        .chain(lifecycle.iter().map(|event| event.frame_ordinal))
        .chain(
            frame
                .target_lifecycle_events
                .iter()
                .filter_map(|event| event.frame_ordinal),
        )
        .collect::<Vec<_>>();
    frame_ordinals.sort_unstable();
    assert_eq!(
        frame_ordinals,
        (0..frame_ordinals.len() as u64).collect::<Vec<_>>(),
        "schema-{TRACE_SCHEMA_VERSION} strike/sequence/target frame ordinals are not a single contiguous timeline"
    );

    let mut invocation_state = BTreeMap::<u32, (bool, bool)>::new();
    for event in proposals {
        let state = invocation_state.entry(event.invocation).or_default();
        assert!(!state.1, "strike-proposal event follows its result");
        if event.phase == "entry" {
            assert!(!state.0, "strike-proposal invocation has duplicate entry");
            state.0 = true;
        } else {
            assert!(state.0, "strike-proposal event precedes its entry");
        }
        if event.phase == "result" {
            state.1 = true;
        }
    }
    for (invocation, (started, finished)) in invocation_state {
        assert!(
            started,
            "strike-proposal invocation {invocation} has no entry"
        );
        assert!(
            finished,
            "strike-proposal invocation {invocation} has no result"
        );
    }
}

#[cfg(test)]
pub(super) fn validate_trace_frame(schema: u32, frame: &TraceFrame) {
    validate_trace_frame_with_legacy_additive_omissions(schema, frame, false);
}

pub(super) fn validate_trace_frame_with_legacy_additive_omissions(
    schema: u32,
    frame: &TraceFrame,
    legacy_additive_omissions: bool,
) {
    // Opponent jump-line slots were added with schema 16. Schemas 12-15
    // legitimately decode the absent additive array as empty.
    if schema >= 16 {
        validate_jump_line_shapes(frame, legacy_additive_omissions);
    }
    validate_sequence_diagnostic_order(frame);
}

pub(super) fn parse_trace_frame(line: &str, line_number: usize) -> Option<TraceFrame> {
    match serde_json::from_str(line) {
        Ok(frame) => {
            let frame: TraceFrame = frame;
            assert_eq!(
                frame.record_type, "frame",
                "invalid parity frame record type on line {line_number}"
            );
            Some(frame)
        }
        Err(frame_error) => {
            let marker: TraceRecordMarker = serde_json::from_str(line).unwrap_or_else(|_| {
                panic!("parse trace frame on line {line_number}: {frame_error}")
            });
            if marker.record_type.as_deref() == Some("rng_suffix") {
                None
            } else {
                panic!("parse trace frame on line {line_number}: {frame_error}");
            }
        }
    }
}
