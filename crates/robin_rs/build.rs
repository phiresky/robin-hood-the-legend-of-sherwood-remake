//! Build script: emit exact and short source identities.
//!
//! Shaders are now consumed directly as WGSL by wgpu at runtime — no
//! offline compilation step needed.

#[path = "../../build-support/robin_build.rs"]
mod shared;

fn main() {
    shared::main();
    println!("cargo:rerun-if-changed=windows/robin.rc");
    println!("cargo:rerun-if-changed=windows/robin.manifest");
    // A missing privilege/compatibility manifest must fail Windows builds.
    embed_resource::compile_for("windows/robin.rc", ["robin"], embed_resource::NONE)
        .manifest_required()
        .expect("cannot embed the Windows application manifest");
}
