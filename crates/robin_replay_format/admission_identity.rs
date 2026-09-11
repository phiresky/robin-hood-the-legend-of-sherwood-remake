//! Conservative local source closure of the native decoder and package validator.
//! Keep this list in sync when their local dependencies change. Registry sources
//! are pinned by Cargo.lock; local source contents must also cover dirty builds.
use sha2::{Digest, Sha256};
use std::path::Path;

pub const SOURCES: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "build-support",
    "crates/robin_replay_format",
    "crates/robin_engine",
    "crates/robin_content",
    "crates/robin_data_io",
    "crates/robin_util",
    "crates/robin_state_hash_derive",
    "crates/robin_run_protocol",
    "crates/robin_spellforge",
    "vendor/rilua",
];

pub fn fingerprint(
    root: &Path,
    configuration: &[(String, String)],
    mut watch: impl FnMut(&Path),
) -> std::io::Result<String> {
    fn field(hash: &mut Sha256, bytes: &[u8]) {
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    fn visit(
        root: &Path,
        path: &Path,
        hash: &mut Sha256,
        watch: &mut impl FnMut(&Path),
    ) -> std::io::Result<()> {
        watch(path);
        if path.is_dir() {
            let mut entries = std::fs::read_dir(path)?
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<std::io::Result<Vec<_>>>()?;
            entries.sort();
            for entry in entries {
                visit(root, &entry, hash, watch)?;
            }
        } else {
            // Relative, slash-normalized names keep installed/source checkout
            // locations out of identity while binding file boundaries and names.
            field(
                hash,
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
                    .as_bytes(),
            );
            field(hash, &std::fs::read(path)?);
        }
        Ok(())
    }
    let mut hash = Sha256::new();
    field(&mut hash, b"robin-native-admission-source-v1");
    for source in SOURCES {
        visit(root, &root.join(source), &mut hash, &mut watch)?;
    }
    for (key, value) in configuration {
        field(&mut hash, key.as_bytes());
        field(&mut hash, value.as_bytes());
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
