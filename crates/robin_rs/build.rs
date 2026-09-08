//! Build script: emit exact and short source identities.
//!
//! Shaders are now consumed directly as WGSL by wgpu at runtime — no
//! offline compilation step needed.

use std::path::PathBuf;

#[path = "../../build-support/robin_build.rs"]
mod shared;

#[path = "build_support/projection_static.rs"]
mod projection_static_build_policy;

fn main() {
    shared::main();
    build_projection_musl_libdl_compatibility_archive();
}

/// Rust's self-contained MUSL sysroot provides the `dlopen` family in libc,
/// but intentionally has no separate compatibility `libdl.a`. Some native
/// dependencies still request `-ldl`; provide that empty compatibility
/// archive only for the closed static projection-authoring target.
fn build_projection_musl_libdl_compatibility_archive() {
    let projection_export_enabled = std::env::var_os("CARGO_FEATURE_PROJECTION_EXPORT").is_some();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if !projection_static_build_policy::needs_musl_libdl_compatibility_archive(
        &target_env,
        projection_export_enabled,
    ) {
        return;
    }

    println!("cargo:rerun-if-changed=build_support/musl_libdl_compat.c");
    let output_dir = std::env::var_os("OUT_DIR").expect("Cargo must provide OUT_DIR");
    cc::Build::new()
        .file("build_support/musl_libdl_compat.c")
        .warnings(false)
        .cargo_metadata(false)
        .compile("dl");
    println!(
        "cargo:rustc-link-search=native={}",
        PathBuf::from(output_dir).display()
    );
}
