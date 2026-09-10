//! Locale-aware sample identity resolution, independent of the mixer/cache.
use std::path::{Path, PathBuf};

pub(super) fn resolve_sample(
    sound_dir: &Path,
    file_name: &str,
    files: &robin_engine::sbfile::SbFileSystem,
) -> Result<PathBuf, String> {
    let candidates = super::sample_base_paths(sound_dir, file_name);
    let path = candidates.0.clone();
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

pub(super) fn resolve_music(
    files: &robin_engine::sbfile::SbFileSystem,
    path: &str,
) -> Result<PathBuf, String> {
    let resolve_one = |path: &str| {
        files
            .try_exists(path)
            .map(|exists| exists.then(|| PathBuf::from(path)))
            .map_err(|status| format!("music lookup failed for {path}: {status}"))
    };

    if let Some(resolved) = resolve_one(path)? {
        return Ok(resolved);
    }

    let raw = PathBuf::from(path);
    let alternate_extension = raw.extension().and_then(|extension| {
        if extension.eq_ignore_ascii_case("wav") {
            Some("ogg")
        } else if extension.eq_ignore_ascii_case("ogg") {
            Some("wav")
        } else {
            None
        }
    });
    if let Some(extension) = alternate_extension {
        let alternate = raw.with_extension(extension);
        if let Some(path) = alternate.to_str()
            && let Some(resolved) = resolve_one(path)?
        {
            return Ok(resolved);
        }
    }

    let opus = raw.with_extension("opus");
    if let Some(resolved) = resolve_one(&opus.to_string_lossy())? {
        return Ok(resolved);
    }
    Err(format!("music asset not found: {}", raw.display()))
}

#[test]
fn music_candidates_keep_requested_format_then_alternate_then_opus() {
    for (requested, alternate) in [
        ("wav", "ogg"),
        ("ogg", "wav"),
        ("WAV", "ogg"),
        ("OGG", "wav"),
    ] {
        let paths = [
            format!("Data/Music/resolver-test.{requested}"),
            format!("Data/Music/resolver-test.{alternate}"),
            "Data/Music/resolver-test.opus".to_owned(),
        ];
        for first in 0..paths.len() {
            let assets = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
            for path in &paths[first..] {
                assets.install_preloaded_asset(path, vec![1]).unwrap();
            }
            let files = robin_engine::sbfile::SbFileSystem::new(assets);
            assert_eq!(
                resolve_music(&files, &paths[0]).unwrap(),
                PathBuf::from(&paths[first])
            );
        }
    }
}
