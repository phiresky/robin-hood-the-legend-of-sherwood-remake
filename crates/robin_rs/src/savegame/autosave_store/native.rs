use super::*;

pub(crate) fn manifest_path(save_directory: &str) -> PathBuf {
    Path::new(save_directory).join(AUTOSAVE_MANIFEST_FILE)
}

pub(crate) fn load_manifest(save_directory: &str) -> Result<Option<AutosaveManifest>> {
    let path = manifest_path(save_directory);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let manifest: AutosaveManifest =
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))?;
    manifest.validate()?;
    Ok(Some(manifest))
}

pub(crate) fn persist_manifest(save_directory: &str, manifest: &AutosaveManifest) -> Result<()> {
    manifest.validate()?;
    let bytes = serde_json::to_vec_pretty(manifest).context("serializing autosave manifest")?;
    crate::save_file::atomic_write(&manifest_path(save_directory), &bytes)
}

pub(crate) fn persist_payload(
    save_directory: &str,
    filename: &str,
    payload: &GameSaveFile,
    thumbnail: Option<&Thumbnail>,
) -> Result<()> {
    validate_generated_filename(filename)?;
    let path = Path::new(save_directory)
        .join(filename)
        .with_extension("json");
    payload.write_to(&path)?;
    if let Some(thumbnail) = thumbnail {
        let path = Path::new(save_directory).join(format!("{filename}_thumb.png"));
        if let Err(error) = thumbnail.write_to(&path) {
            // A thumbnail is auxiliary: never discard a valid recovery point
            // because its preview could not be written, but do report it.
            tracing::warn!(
                filename,
                "autosave thumbnail could not be written: {error:#}"
            );
        }
    }
    Ok(())
}

pub(crate) fn read_payload(save_directory: &str, filename: &str) -> Result<GameSaveFile> {
    validate_generated_filename(filename)?;
    GameSaveFile::read_from(
        &Path::new(save_directory)
            .join(filename)
            .with_extension("json"),
    )
}

pub(crate) fn payload_exists(save_directory: &str, filename: &str) -> Result<bool> {
    validate_generated_filename(filename)?;
    let path = Path::new(save_directory)
        .join(filename)
        .with_extension("json");
    path.try_exists()
        .with_context(|| format!("checking autosave payload {}", path.display()))
}

pub(crate) fn read_thumbnail(save_directory: &str, filename: &str) -> Result<Option<Thumbnail>> {
    validate_generated_filename(filename)?;
    let path = Path::new(save_directory).join(format!("{filename}_thumb.png"));
    Thumbnail::read_optional_from(&path)
}

pub(crate) fn remove_payload(save_directory: &str, filename: &str) -> Result<()> {
    validate_generated_filename(filename)?;
    for path in [
        Path::new(save_directory)
            .join(filename)
            .with_extension("json"),
        Path::new(save_directory).join(format!("{filename}_thumb.png")),
    ] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("removing {}", path.display()));
            }
        }
    }
    Ok(())
}

pub(crate) fn garbage_collect_orphans(
    save_directory: &str,
    manifest: &AutosaveManifest,
) -> Result<()> {
    let referenced: BTreeSet<_> = manifest
        .saves
        .iter()
        .map(|save| save.filename.as_str())
        .collect();
    let directory = Path::new(save_directory);
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("enumerating autosave directory {}", directory.display())
            });
        }
    };
    for entry in entries {
        let entry = entry.with_context(|| {
            format!(
                "reading an entry from autosave directory {}",
                directory.display()
            )
        })?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("reading file type for {}", entry.path().display()))?;
        if !file_type.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let orphan = generated_filename_from_storage_name(name)
            .is_some_and(|filename| !referenced.contains(filename));
        let interrupted_stage = name.starts_with(".robin-autosave-staging-");
        if orphan || interrupted_stage {
            std::fs::remove_file(entry.path()).with_context(|| {
                format!(
                    "removing orphan autosave artifact {}",
                    entry.path().display()
                )
            })?;
        }
    }
    Ok(())
}

pub(crate) fn generated_filename_from_storage_name(name: &str) -> Option<&str> {
    let filename = name
        .strip_suffix("_thumb.png")
        .or_else(|| name.strip_suffix(".json"))?;
    crate::savegame::is_generated_autosave_filename(filename).then_some(filename)
}
