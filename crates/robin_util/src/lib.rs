//! General-purpose helpers shared by both sides of the codebase
//! (sim engine and host renderer). Add things here only if they have no
//! business belonging to either side specifically.

pub mod asset_fs;
pub mod color;
mod diagnostic_only;
pub mod display_text;
pub mod json_value;
pub mod persistence_validation;
pub mod state_hash;
pub mod static_arc;
