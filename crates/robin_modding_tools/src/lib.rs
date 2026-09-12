//! Script inspection tools shared by offline utilities and live RPC diagnostics.
//! Parsing and runtime asset admission remain in `robin_assets`.

pub mod actor_names;
pub mod decompile;
pub mod disasm;

#[cfg(test)]
mod original_data {
    use robin_test_support::original_data::data_directory;

    pub fn demo_scb_path() -> std::path::PathBuf {
        data_directory(".").join("Data/Levels/Dem_Lei_MP.scb")
    }
}
