//! Movement test suites split out of `movement.rs`.
//!
//! The submodules were written as direct children of `movement` and import
//! its items through `use super::super::*` from their inner `suite`
//! modules; the glob below re-exposes the parent's items at this level so
//! those imports keep resolving unchanged.
use super::*;

mod aligned_transition_deviation;
mod arrival_snap;
mod line_jump;
mod movement_transition_state;
mod orphaned_sword_movement;
mod owner_phases;
mod path_request_timing;

// Former inline `#[cfg(test)] mod …_tests` blocks of `movement.rs`.
mod arrival_speech_topology_tests;
mod door_pass_posture_tests;
mod drunken_turn_timing_tests;
mod exact_lift_sector_tests;
mod group_move_authorization_tests;
mod line_crossing_eligibility_tests;
mod post_seek_hit_handoff_tests;
mod route_source_tests;
mod selected_movement_preparation_tests;
