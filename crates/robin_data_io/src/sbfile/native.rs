//! Native case-folded path resolution and physical mount confinement.

use super::SbFileError;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn resolve_case_insensitive(path: &Path) -> Option<PathBuf> {
    // The fallible helper logs before this compatibility facade discards status.
    try_resolve_case_insensitive(path).ok().flatten()
}

fn path_resolution_error(operation: &str, path: &Path, error: std::io::Error) -> SbFileError {
    tracing::warn!("asset {operation} {} failed: {error}", path.display());
    SbFileError::Read
}

fn is_missing_component(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
    )
}

fn candidate_exists(path: &Path) -> Result<bool, SbFileError> {
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if is_missing_component(&error) => Ok(false),
        Err(error) => Err(path_resolution_error("metadata", path, error)),
    }
}

pub(super) fn first_case_folded_entry(
    directory: &Path,
    target_lower: &str,
    entries: impl IntoIterator<Item = std::io::Result<PathBuf>>,
) -> Result<Option<PathBuf>, SbFileError> {
    for entry in entries {
        let entry =
            entry.map_err(|error| path_resolution_error("directory entry", directory, error))?;
        if let Some(name) = entry.file_name().and_then(|name| name.to_str())
            && !name.starts_with('.')
            && name.to_ascii_lowercase() == target_lower
        {
            return Ok(Some(entry));
        }
    }
    Ok(None)
}

pub(super) fn try_resolve_case_insensitive(path: &Path) -> Result<Option<PathBuf>, SbFileError> {
    let Some(path_str) = path.to_str() else {
        tracing::warn!("asset path is not UTF-8: {}", path.display());
        return Err(SbFileError::Read);
    };
    if cfg!(windows) {
        // The case-fold walk below cannot rebuild drive/verbatim prefixes
        // (`C:\`, canonicalize's `\\?\C:\`), and Windows filesystems are
        // case-insensitive already, so a direct probe is both sufficient
        // and the only thing that works. Verbatim paths forbid forward
        // slashes, so fold separators to backslashes first.
        let backslashed = PathBuf::from(path_str.replace('/', "\\"));
        return Ok(candidate_exists(&backslashed)?.then_some(backslashed));
    }
    let normalised = path_str.replace('\\', "/");
    let path = Path::new(&normalised);
    let mut components = path.components().peekable();
    let mut resolved = match components.peek() {
        Some(std::path::Component::RootDir) => {
            components.next();
            PathBuf::from("/")
        }
        _ => PathBuf::from("."),
    };
    for component in components {
        let target = component
            .as_os_str()
            .to_str()
            .expect("components of a UTF-8 path");
        let candidate = resolved.join(target);
        if candidate_exists(&candidate)? {
            resolved = candidate;
            continue;
        }
        let target_lower = target.to_ascii_lowercase();
        let entries = match fs::read_dir(&resolved) {
            Ok(entries) => entries,
            Err(error) if is_missing_component(&error) => return Ok(None),
            Err(error) => return Err(path_resolution_error("read directory", &resolved, error)),
        };
        let Some(found) = first_case_folded_entry(
            &resolved,
            &target_lower,
            entries.map(|entry| entry.map(|entry| entry.path())),
        )?
        else {
            return Ok(None);
        };
        // Once an entry matches, even disappearance or a dangling symlink is
        // a failed selected asset, not absence permitting a lower-priority one.
        fs::metadata(&found)
            .map_err(|error| path_resolution_error("selected entry metadata", &found, error))?;
        resolved = found;
    }
    Ok(Some(resolved))
}

pub(super) fn resolve_contained_file(root: &Path, candidate: &Path) -> Option<PathBuf> {
    try_resolve_contained_file(root, candidate).ok().flatten()
}

pub(super) fn resolve_contained_directory(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let resolved = try_resolve_contained(root, candidate).ok().flatten()?;
    let metadata = fs::metadata(&resolved)
        .map_err(|error| path_resolution_error("metadata", &resolved, error))
        .ok()?;
    metadata.is_dir().then_some(resolved)
}

fn try_resolve_contained(root: &Path, candidate: &Path) -> Result<Option<PathBuf>, SbFileError> {
    let Some(resolved) = try_resolve_case_insensitive(candidate)? else {
        return Ok(None);
    };
    let resolved = fs::canonicalize(&resolved)
        .map_err(|error| path_resolution_error("canonicalize", &resolved, error))?;
    if !resolved.starts_with(root) {
        tracing::warn!(
            "asset {} escapes mount {}",
            resolved.display(),
            root.display()
        );
        return Err(SbFileError::Read);
    }
    Ok(Some(resolved))
}

pub(super) fn try_resolve_contained_file(
    root: &Path,
    candidate: &Path,
) -> Result<Option<PathBuf>, SbFileError> {
    let Some(resolved) = try_resolve_contained(root, candidate)? else {
        return Ok(None);
    };
    let metadata = fs::metadata(&resolved)
        .map_err(|error| path_resolution_error("metadata", &resolved, error))?;
    Ok(metadata.is_file().then_some(resolved))
}

pub(super) fn path_exists_contained(root: &Path, candidate: &Path) -> Result<bool, SbFileError> {
    try_resolve_contained(root, candidate).map(|path| path.is_some())
}
