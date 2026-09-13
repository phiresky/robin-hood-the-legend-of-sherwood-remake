//! Shared source inventory for the source-structural guardrail tests.
//!
//! Every guardrail binary walks and reads Rust sources through
//! [`rust_sources`], so a root is walked and its files are read once per test
//! binary no matter how many checks inspect it. Parsing lives in the sibling
//! `syntax.rs`, which only the binaries that inspect syntax trees include.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// One Rust source file read from disk.
pub struct SourceFile {
    pub path: PathBuf,
    pub text: String,
}

/// Every `*.rs` file under `root` (or `root` itself when it names a file),
/// sorted by path. `target/` and `.git/` directories are skipped. Unreadable
/// directories or files fail the calling test instead of shrinking the
/// inventory.
///
/// The inventory is cached per root for the lifetime of the test binary.
pub fn rust_sources(root: &Path) -> Arc<[SourceFile]> {
    static TREES: OnceLock<Mutex<HashMap<PathBuf, Arc<[SourceFile]>>>> = OnceLock::new();
    let trees = TREES.get_or_init(Default::default);
    if let Some(files) = trees.lock().expect("source inventory cache lock").get(root) {
        return Arc::clone(files);
    }
    // Walk without holding the lock so concurrent tests reading other roots
    // are not serialized behind a large walk; a racing duplicate walk of the
    // same root produces identical contents.
    let mut paths = Vec::new();
    if root.is_file() {
        paths.push(root.to_path_buf());
    } else {
        collect_rust_paths(root, &mut paths);
    }
    paths.sort();
    let files: Arc<[SourceFile]> = paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            SourceFile { path, text }
        })
        .collect();
    Arc::clone(
        trees
            .lock()
            .expect("source inventory cache lock")
            .entry(root.to_path_buf())
            .or_insert(files),
    )
}

fn collect_rust_paths(directory: &Path, paths: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!("failed to read entry in {}: {error}", directory.display())
        });
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            if name != "target" && name != ".git" {
                collect_rust_paths(&path, paths);
            }
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            paths.push(path);
        }
    }
}
