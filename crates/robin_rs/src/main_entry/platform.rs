//! Startup hooks whose bodies differ per platform.
//!
//! Every platform module exports the same signatures, so `init` and `run`
//! call `platform::<hook>` without `#[cfg]` at the call site:
//! - `setup_data_dir` — install the primary datadir and its overlay/locale roots
//! - `overlay_mods_dir` — the installation's auto-mounted `mods/` directory
//! - `prepare_direct_custom_mission_args` — admit a `--custom-mission` archive

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(not(target_arch = "wasm32"))]
pub use native::{overlay_mods_dir, prepare_direct_custom_mission_args, setup_data_dir};
#[cfg(target_arch = "wasm32")]
pub use wasm::{overlay_mods_dir, prepare_direct_custom_mission_args, setup_data_dir};
