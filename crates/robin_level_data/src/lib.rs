//! Static profiles and level content decoded before simulation construction.

pub mod content_patch;
pub mod level_data;
pub mod profiles;

pub(crate) use robin_data_io::{legacy_io, sbfile};
pub(crate) use robin_engine_types::{coordinates, diplomacy, geo2d, human_control};
