//! Shared admission workflow with compile-time storage adapters.
#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
pub use browser::*;
#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
pub mod workflow;
