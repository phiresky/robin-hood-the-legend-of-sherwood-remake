//! Asset-loading layer for Robin Hood.
//!
//! Defaults include `engine-adapters`: mission preparation, script tools,
//! resource managers, and the shipping-datadir container. Format-only users can
//! disable default features to decode sprite grids, pictures, RLE/JXL canvases,
//! and legacy frame banks without depending on the simulation engine.

#![feature(portable_simd)]

#[cfg(test)]
mod original_data {
    pub use robin_test_support::original_data::*;

    #[cfg(feature = "engine-adapters")]
    pub fn demo_scb_path() -> std::path::PathBuf {
        data_file("Data/Levels/Dem_Lei_MP.scb")
    }
}

pub mod binary_reader;
#[cfg(feature = "engine-adapters")]
pub mod custom_sprites;
pub mod frame_holder;
#[cfg(feature = "engine-adapters")]
pub mod interface_metadata;
pub mod late_sprites;
#[cfg(feature = "engine-adapters")]
pub mod original_text;
pub mod packed_sprite;
pub mod picture;
#[cfg(feature = "engine-adapters")]
pub mod res_descr;
#[cfg(feature = "engine-adapters")]
pub mod resource_manager;
pub mod rle_jxl;
#[cfg(feature = "engine-adapters")]
pub mod scb;
#[cfg(feature = "engine-adapters")]
pub mod shipping_boot_trim;
#[cfg(feature = "engine-adapters")]
pub mod shipping_datadir;
pub mod sprite_codec;
#[cfg(feature = "engine-adapters")]
pub mod sprite_groups;
pub mod sprite_pixels;
pub mod terrain_source;
#[cfg(target_arch = "wasm32")]
mod wasm_alloc;
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
pub mod wasm_threads;
