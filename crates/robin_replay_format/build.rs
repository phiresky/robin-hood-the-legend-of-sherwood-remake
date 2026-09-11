mod shared {
    include!("../../build-support/robin_build.rs");
}
mod admission_identity;

fn main() {
    shared::main();
    let root =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let mut configuration: Vec<_> = std::env::vars()
        .filter(|(key, _)| {
            key.starts_with("CARGO_CFG_")
                || key.starts_with("CARGO_FEATURE_")
                || matches!(key.as_str(), "TARGET" | "CARGO_ENCODED_RUSTFLAGS")
        })
        .collect();
    configuration.sort();
    let fingerprint = admission_identity::fingerprint(&root, &configuration, |path| {
        println!("cargo:rerun-if-changed={}", path.display());
    })
    .expect("cannot fingerprint native replay admission dependencies");
    println!("cargo:rustc-env=ROBIN_ADMISSION_SOURCE_SHA256={fingerprint}");
}
