//! Asset-loading layer for Robin Hood.
//!
//! Defaults include `engine-adapters`: mission preparation, script tools,
//! resource managers, and the shipping-datadir container. Format-only users can
//! disable default features to decode sprite grids, pictures, RLE/JXL canvases,
//! and legacy frame banks without depending on the simulation engine.

#![feature(portable_simd)]

#[cfg(test)]
#[allow(dead_code)] // Consumers use different subsets under different feature sets.
mod original_data {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../test-support/original_data.rs"
    ));

    pub fn demo_scb_path() -> std::path::PathBuf {
        // scb::parse_file resolves case-insensitive original asset paths;
        // extracted demo data can use DATA rather than Data.
        data_directory(".").join("Data/Levels/Dem_Lei_MP.scb")
    }
}

#[cfg(feature = "engine-adapters")]
pub mod actor_names;
pub mod binary_reader;
#[cfg(feature = "engine-adapters")]
pub mod decompile;
#[cfg(feature = "engine-adapters")]
pub mod disasm;
pub mod frame_holder;
pub mod late_sprites;
pub mod packed_sprite;
pub mod picture;
#[cfg(feature = "engine-adapters")]
pub mod res_descr;
#[cfg(feature = "engine-adapters")]
pub mod resource_manager;
pub mod rle_jxl;
pub mod sb3d;
#[cfg(feature = "engine-adapters")]
pub mod scb;
#[cfg(feature = "engine-adapters")]
pub mod shipping_boot_trim;
#[cfg(feature = "engine-adapters")]
pub mod shipping_datadir;
pub mod sprite_codec;
#[cfg(feature = "engine-adapters")]
pub mod sprite_groups;
#[cfg(target_arch = "wasm32")]
mod wasm_alloc;
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
pub mod wasm_threads;
