//! Stable simulation values and configuration shared without compiling the engine.
//!
//! Keep this crate independent of simulation state and host integrations so
//! edits to engine behavior can reuse its derives and checked implementations.
#![deny(unused_must_use)]

#[doc(hidden)]
pub mod bitcode_adapters;
pub mod character_kind;
pub mod coordinates;
pub mod entity_id;
pub mod gameplay_config;
pub mod geo2d;
pub mod graphic_config;
pub mod multiplayer_config;
pub mod parameters_ai;
pub mod resource_ids;
mod serde_defaults;
pub mod sound_config;
pub mod static_arc;
pub use robin_data_io::legacy_io;
