//! Locale-aware sample identity resolution, independent of the mixer/cache.
use std::path::{Path, PathBuf};

pub(super) fn resolve_sample(
    sound_dir: &Path,
    file_name: &str,
    files: &robin_engine::sbfile::SbFileSystem,
) -> Result<PathBuf, String> {
    let candidates = super::sample_base_paths(sound_dir, file_name);
    let path = candidates
        .first()
        .expect("sample lookup always includes a primary path")
        .clone();
    for candidate in super::with_opus_fallback(candidates) {
        if files
            .try_exists(&candidate.to_string_lossy())
            .map_err(|status| {
                format!("audio lookup failed for {}: {status}", candidate.display())
            })?
        {
            return Ok(candidate);
        }
    }
    Err(format!("audio sample not found: {}", path.display()))
}
