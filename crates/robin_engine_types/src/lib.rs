//! Stable simulation values and configuration shared without compiling the engine.
//!
//! Keep this crate independent of simulation state and host integrations so
//! edits to engine behavior can reuse its derives and checked implementations.
#![deny(unused_must_use)]

#[doc(hidden)]
pub mod bitcode_adapters;
pub mod camp;
pub mod character_kind;
pub mod coordinates;
pub mod diplomacy;
pub mod entity_id;
pub mod gameplay_config;
pub mod geo2d;
pub mod graphic_config;
pub mod human_control;
pub mod mission_environment;
pub mod multiplayer_config;
pub mod parameters_ai;
pub mod resource_ids;
mod serde_defaults;
pub mod sound_config;
pub mod sprite_ambiance;
pub mod sprite_content;
pub mod static_arc;
pub use robin_data_io::legacy_io;
pub use sprite_content::{PixelOpacityLookup, SpriteVariant};
