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
