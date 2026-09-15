//! Build script: emit exact and short source identities.
//!
//! Shaders are now consumed directly as WGSL by wgpu at runtime — no
//! offline compilation step needed.

#[path = "../../build-support/robin_build.rs"]
mod shared;

fn main() {
    shared::main();
}
