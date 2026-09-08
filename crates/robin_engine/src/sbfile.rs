//! Intentional migration bridge for the engine-neutral game-data reader.
//! New consumers should depend directly on `robin_data_io`.

pub use robin_data_io::sbfile::*;
