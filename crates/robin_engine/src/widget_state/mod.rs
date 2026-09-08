//! Sim-side widget-state structs.
//!
//! These compute derived values (blazon bar numbers, mission requirement
//! slots) that the sim/script engine needs to pass between tick systems
//! and eventually hand off to the host renderer. Rendering itself lives
//! in `robin_rs::widget`.

pub mod blazon_bar;
pub mod blazon_set;
pub mod requirements;
