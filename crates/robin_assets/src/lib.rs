//! Asset-loading layer for Robin Hood.
//!
//! Defaults include `engine-adapters`: mission preparation, script tools,
//! resource managers, and the shipping-datadir container. Format-only users can
//! disable default features to decode sprite grids, pictures, RLE/JXL canvases,
//! and legacy frame banks without depending on the simulation engine.

#![feature(portable_simd)]

#[cfg(feature = "engine-adapters")]
pub mod actor_names;
pub mod adpcm_check;
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
pub mod serialize;
#[cfg(feature = "engine-adapters")]
pub mod shipping_datadir;
pub mod sprite_codec;
#[cfg(target_arch = "wasm32")]
mod wasm_alloc;
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
pub mod wasm_threads;
