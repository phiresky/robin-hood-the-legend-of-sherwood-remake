//! Locale-aware sample identity resolution, independent of the mixer/cache.
use std::path::{Path, PathBuf};

pub(super) fn resolve_sample(
    sound_dir: &Path,
    file_name: &str,
    files: &robin_engine::sbfile::SbFileSystem,
) -> Result<PathBuf, String> {
    let normalised = file_name.replace('\\', "/");
    let absolute = Path::new(&normalised).is_absolute();
    let path = if absolute {
        PathBuf::from(&normalised)
    } else {
        sound_dir.join(&normalised)
    };
    let candidates = if absolute {
        vec![path.clone()]
    } else {
        // actors.res stores speech paths relative to its Exclamations
        // directory (for example `Expressions/X_SD_...wav`), whereas
        // ordinary FX/source paths are relative to Data/Sounds.
        vec![
            path.clone(),
            sound_dir.join("Exclamations").join(&normalised),
        ]
    };
    for candidate in candidates.into_iter().flat_map(|path| {
        let opus = path.with_extension("opus");
        [path, opus]
    }) {
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
