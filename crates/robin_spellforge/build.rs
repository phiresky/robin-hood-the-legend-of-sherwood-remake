use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let rilua = manifest.join("../../vendor/rilua");
    let mut rilua_files = vec![rilua.join("Cargo.toml")];
    collect_files(&rilua.join("src"), &mut rilua_files);
    emit_source_digest("SPELLFORGE_RILUA_SOURCE_SHA256", &rilua, rilua_files);

    // Test-only dependency edits must not churn the executable ABI. Runtime
    // dependency pins/features are explicit ABI components in `src/runtime/abi.rs`.
    let mut runtime_files = vec![manifest.join("build.rs")];
    collect_files(&manifest.join("src"), &mut runtime_files);
    emit_source_digest("SPELLFORGE_RUNTIME_SOURCE_SHA256", &manifest, runtime_files);

    // Shared validators and typed ABI interpretation affect guest admission and
    // execution too. Keep them inside the source-fingerprint boundary after
    // moving them out of this crate.
    let engine = manifest.join("../robin_engine");
    let mut contract_files = vec![
        engine.join("src/spellforge.rs"),
        engine.join("src/natives/signatures.rs"),
    ];
    collect_files(&engine.join("src/spellforge"), &mut contract_files);
    emit_source_digest(
        "SPELLFORGE_ENGINE_CONTRACT_SOURCE_SHA256",
        &engine,
        contract_files,
    );
}

fn emit_source_digest(variable: &str, root: &Path, mut files: Vec<PathBuf>) {
    files.sort();
    let mut digest = Sha256::new();
    for path in files {
        println!("cargo::rerun-if-changed={}", path.display());
        let relative = path.strip_prefix(root).unwrap();
        let name = relative
            .to_str()
            .expect("vendored rilua paths must be UTF-8")
            .replace('\\', "/");
        let bytes = std::fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "cannot hash vendored rilua source {}: {error}",
                path.display()
            )
        });
        digest.update((name.len() as u64).to_le_bytes());
        digest.update(name.as_bytes());
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    let digest: [u8; 32] = digest.finalize().into();
    let mut hex = String::with_capacity(64);
    use std::fmt::Write as _;
    for byte in digest {
        write!(&mut hex, "{byte:02x}").unwrap();
    }
    println!("cargo::rustc-env={variable}={hex}");
}

fn collect_files(directory: &Path, files: &mut Vec<PathBuf>) {
    // Directory watches also cover added/removed modules and included Lua assets.
    println!("cargo::rerun-if-changed={}", directory.display());
    let mut entries = std::fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("cannot scan {}: {error}", directory.display()))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            collect_files(&path, files);
        } else {
            files.push(path);
        }
    }
}
