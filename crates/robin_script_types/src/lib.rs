//! Script contracts shared by the simulation and guest runtimes.
//!
//! Keeping native metadata and snapshot types independent of the simulation
//! lets runtime implementations reuse their compiled contracts after engine edits.

pub use robin_engine_types::bitcode_adapters;
pub mod natives;
pub mod spellforge;
