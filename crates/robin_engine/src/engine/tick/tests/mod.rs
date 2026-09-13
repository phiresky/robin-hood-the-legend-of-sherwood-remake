//! Tick test suites split out of `tick.rs`.
//!
//! The submodules were written as direct children of `tick` and import its
//! items through `use super::*`; the glob below re-exposes the parent's
//! items at this level so those imports keep resolving unchanged.
use super::*;

mod active_ability_owner_selection_tests;
mod bow_command_body_parity_tests;
mod drop_ammo_merge_tests;
mod drunken_path_deviation_tests;
mod frozen_actor_entry_condolation_tests;
mod generic_actor_line_crossing_tests;
mod mobile_owner_boundary_tests;
mod restored_pass_door_completion_tests;
mod soldier_take_drink_parity_tests;
mod specialized_execute_motion_tests;
