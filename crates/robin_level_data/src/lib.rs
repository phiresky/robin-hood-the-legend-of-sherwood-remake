//! Static profiles and level content decoded before simulation construction.

mod compiled_elevation;
mod compiled_masks;
pub mod content_patch;
pub mod level_data;
pub mod physical_stair;
pub mod profiles;
pub mod stair_navigation;
pub mod stair_navigation_floor;

pub(crate) use robin_data_io::{legacy_io, sbfile};
pub(crate) use robin_engine_types::{coordinates, diplomacy, geo2d, human_control};
