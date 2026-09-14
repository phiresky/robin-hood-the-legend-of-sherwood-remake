//! Enemy AI parity stderr payloads. Gates and evaluation remain at their callers.
//! These formatters preserve the byte-level diagnostic protocol.
//!
//! Events whose payload is strict JSON built only from integers, booleans and
//! fixed ASCII tags are typed serde structs serialized with serde_json (same
//! bytes as the former format strings, pinned by the golden tests). All other
//! events embed Rust `Debug` output (`Some(3)`, enum variant names, `[1, 2]`),
//! `Display` floats (`1` for `1.0f32`) or `key=value` text, which serde_json
//! cannot reproduce byte-for-byte; they are named-field `trace_event!` structs.

use serde::Serialize;

use crate::ai::HumanHandle;
use crate::ai::parity_trace::{trace_event, write_line};

#[cfg(test)]
mod tests;

fn json(event: &impl Serialize) -> String {
    serde_json::to_string(event)
        .expect("parity payload of integers, booleans and ASCII tags always serializes")
}

/// `SEEKAREA` phase-6 (personal seek point postprocessing) lines.
#[derive(Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub(super) enum SeekAreaPhase6 {
    Phase6Before {
        frame: u32,
        owner_handle: HumanHandle,
        owner_creation_order: u32,
        state: u32,
        substate: u32,
        flags: u16,
        seek_direction: u16,
        list_size: usize,
        list_empty: bool,
        location_first: bool,
        location_end: bool,
        personal1_constructor: &'static str,
    },
    Phase6Personal1 {
        frame: u32,
        owner_creation_order: u32,
        constructor: &'static str,
        inserted_id: u32,
        list_size: usize,
    },
    Phase6After {
        frame: u32,
        owner_creation_order: u32,
        personal2_inserted: bool,
        personal2_constructor: &'static str,
        list_size: usize,
    },
}

impl std::fmt::Display for SeekAreaPhase6 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "SEEKAREA {}", json(self))
    }
}

impl SeekAreaPhase6 {
    #[cold]
    #[inline(never)]
    pub(super) fn emit(&self) {
        write_line(self);
    }
}

trace_event! {
    SeekareaPointDump {
        frame: Display,
        index: Display,
        id: Display,
        x: Display,
        y: Display,
        level: Display,
        center_x: Display,
        center_y: Display,
        center_level: Display,
        norm: Display,
        norm_bits: Display,
        near: Display,
        frame_when_full_interest: Display;
    } => "SEEKAREA {{\"event\":\"point_dump\",\"frame\":{},\"index\":{},\"id\":{},\"x\":{},\"y\":{},\"level\":{},\"center\":[{},{},{}],\"norm\":{},\"norm_bits\":{},\"near\":{},\"frame_when_full_interest\":{}}}"
}

trace_event! {
    SeekareaPhase4Candidate {
        frame: Display,
        owner_handle: Display,
        owner_creation_order: Display,
        candidate_ordinal: Display,
        point_id: Display,
        point_index: Display,
        norm: Display,
        norm_bits: Display,
        frame_when_full_interest: Display,
        interest: Display,
        attempt_raw: Display,
        attempt_mod: Display,
        attempt_result: Display,
        insertion_raw: Display,
        insertion_index: Display,
        accumulator_before_bits: Display,
        accumulator_after_bits: Display;
    } => "SEEKAREA {{\"event\":\"phase4_candidate\",\"frame\":{},\"owner_handle\":{},\"owner_creation_order\":{},\"candidate_ordinal\":{},\"point_id\":{},\"point_index\":{},\"norm\":{},\"norm_bits\":{},\"frame_when_full_interest\":{},\"interest\":{},\"attempt_raw\":{},\"attempt_mod\":{},\"attempt_result\":{},\"insertion_raw\":{},\"insertion_index\":{},\"accumulator_before_bits\":{},\"accumulator_after_bits\":{}}}"
}

trace_event! {
    SeekareaSelectionSummary {
        frame: Display,
        owner_handle: Display,
        owner_creation_order: Debug,
        center_x: Display,
        center_y: Display,
        standard_radius: Display,
        near_points: Display,
        expected_for_one: Display,
        visible_friends: Display,
        clears_help: Display,
        expected_before_help_random: Display,
        expected_points: Display,
        phase4_attempts: Display,
        phase4_accepts: Display,
        preselection_rng_draws: Display,
        phase4_rng_draws: Display,
        selection_rng_draws: Display,
        accepted_interest_sum: Display;
    } => "SEEKAREA {{\"event\":\"selection_summary\",\"frame\":{},\"owner_handle\":{},\"owner_creation_order\":{:?},\"center\":[{},{}],\"standard_radius\":{},\"near_points\":{},\"expected_for_one\":{},\"visible_friends\":{},\"clears_help\":{},\"expected_before_help_random\":{},\"expected_points\":{},\"phase4_attempts\":{},\"phase4_accepts\":{},\"preselection_rng_draws\":{},\"phase4_rng_draws\":{},\"selection_rng_draws\":{},\"accepted_interest_sum\":{}}}"
}

trace_event! {
    SeekareaSelectionExtra {
        frame: Display,
        owner_creation_order: Debug,
        flags: Display,
        seek_direction: Display,
        center_level: Display,
        obligatory: Debug,
        obligatory2: Debug,
        selected_random: Debug;
    } => "SEEKAREA {{\"event\":\"selection_extra\",\"frame\":{},\"owner_creation_order\":{:?},\"flags\":{},\"seek_direction\":{},\"center_level\":{},\"obligatory\":{:?},\"obligatory2\":{:?},\"selected_random\":{:?}}}"
}

trace_event! {
    SeekareaPhase6Center {
        frame: Display,
        owner_creation_order: Debug,
        center_x: Display,
        center_y: Display,
        seek_position_x: Display,
        seek_position_y: Display;
    } => "SEEKAREA {{\"event\":\"phase6_center\",\"frame\":{},\"owner_creation_order\":{:?},\"center\":[{},{}],\"seek_position\":[{},{}]}}"
}
