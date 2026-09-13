//! Script test suites split out of `script.rs`.
//!
//! The submodules were written as direct children of `script` and import
//! its items through `use super::*`; the glob below re-exposes the parent's
//! items at this level so those imports keep resolving unchanged.
use super::*;

mod owner_boundary_position_tests;
mod script_context_tests;
mod sound_completion_tests;
