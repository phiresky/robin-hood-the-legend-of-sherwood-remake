//! Script inspection tools shared by offline utilities and live RPC diagnostics.
//! Parsing and runtime asset admission remain in `robin_assets`.

pub mod actor_names;
pub mod decompile;
pub mod disasm;

#[cfg(test)]
#[allow(dead_code)]
mod original_data {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../test-support/original_data.rs"
    ));

    pub fn demo_scb_path() -> std::path::PathBuf {
        data_directory(".").join("Data/Levels/Dem_Lei_MP.scb")
    }
}
