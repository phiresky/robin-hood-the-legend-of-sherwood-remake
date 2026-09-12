//! Browser VFS path resolution; no host filesystem directory walk.

use super::SbFileError;
use std::path::{Path, PathBuf};

pub fn resolve_case_insensitive(path: &Path) -> Option<PathBuf> {
    // The fallible helper logs before this compatibility facade discards status.
    try_resolve_case_insensitive(path).ok().flatten()
}

pub(super) fn try_resolve_case_insensitive(path: &Path) -> Result<Option<PathBuf>, SbFileError> {
    let Some(path_str) = path.to_str() else {
        tracing::warn!("asset path is not UTF-8: {}", path.display());
        return Err(SbFileError::Read);
    };
    let normalized = path_str.replace('\\', "/");
    // Browser-authored datadirs use exact-cased paths; there is no read_dir.
    robin_util::asset_fs::try_exists(&normalized)
        .map(|exists| exists.then(|| PathBuf::from(normalized)))
        .map_err(|error| {
            tracing::warn!(
                "asset existence check failed for {}: {error}",
                path.display()
            );
            SbFileError::Read
        })
}

pub(super) fn resolve_contained_file(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let resolved = resolve_case_insensitive(candidate)?;
    (resolved.starts_with(root)).then_some(resolved)
}

pub(super) fn resolve_contained_directory(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let resolved = resolve_case_insensitive(candidate)?;
    (resolved.starts_with(root) && resolved.is_dir()).then_some(resolved)
}

pub(super) fn try_resolve_contained_file(
    root: &Path,
    candidate: &Path,
) -> Result<Option<PathBuf>, SbFileError> {
    Ok(resolve_contained_file(root, candidate))
}

pub(super) fn path_exists_contained(root: &Path, candidate: &Path) -> Result<bool, SbFileError> {
    Ok(resolve_case_insensitive(candidate).is_some_and(|resolved| resolved.starts_with(root)))
}
