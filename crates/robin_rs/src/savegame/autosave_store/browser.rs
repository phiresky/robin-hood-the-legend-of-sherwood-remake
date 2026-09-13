use super::*;
use crate::blob_store::{BlobStore as _, BrowserLocalStorage};

pub(crate) fn browser_namespace(save_directory: &str) -> String {
    use base64::Engine as _;
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(save_directory.as_bytes());
    format!(
        "{BROWSER_STORAGE_PREFIX}.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
    )
}

pub(crate) fn browser_key(save_directory: &str, suffix: &str) -> String {
    format!("{}.{}", browser_namespace(save_directory), suffix)
}

pub(crate) fn load_manifest(save_directory: &str) -> Result<Option<AutosaveManifest>> {
    let storage = BrowserLocalStorage::open()?;
    let key = browser_key(save_directory, "manifest");
    let Some(encoded) = storage
        .read_text(&key)
        .map_err(|error| anyhow::anyhow!("reading browser autosave manifest failed: {error}"))?
    else {
        return Ok(None);
    };
    let manifest: AutosaveManifest = decode_browser_blob(&encoded)?;
    manifest.validate()?;
    Ok(Some(manifest))
}

pub(crate) fn persist_manifest(save_directory: &str, manifest: &AutosaveManifest) -> Result<()> {
    manifest.validate()?;
    let storage = BrowserLocalStorage::open()?;
    let key = browser_key(save_directory, "manifest");
    let value = encode_browser_blob(manifest)?;
    storage
        .write_text(&key, &value)
        .map_err(|error| anyhow::anyhow!("publishing browser autosave manifest failed: {error}"))
}

pub(crate) fn persist_payload(
    save_directory: &str,
    filename: &str,
    payload: &GameSaveFile,
    thumbnail: Option<&Thumbnail>,
) -> Result<()> {
    validate_generated_filename(filename)?;
    let storage = BrowserLocalStorage::open()?;
    let payload_key = browser_key(save_directory, &format!("payload.{filename}"));
    let payload_value = encode_browser_blob(payload)?;
    storage
        .write_text(&payload_key, &payload_value)
        .map_err(|error| anyhow::anyhow!("writing browser autosave payload failed: {error}"))?;
    if let Some(thumbnail) = thumbnail {
        let thumbnail_key = browser_key(save_directory, &format!("thumbnail.{filename}"));
        match encode_browser_blob(thumbnail) {
            Ok(thumbnail_value) => {
                if let Err(error) = storage.write_text(&thumbnail_key, &thumbnail_value) {
                    // Keep the payload loadable when browser quota permits
                    // the game state but not its optional preview image.
                    tracing::warn!(
                        filename,
                        "browser autosave thumbnail could not be written: {error}"
                    );
                }
            }
            Err(error) => {
                tracing::warn!(
                    filename,
                    "browser autosave thumbnail could not be encoded: {error:#}"
                );
            }
        }
    }
    Ok(())
}

pub(crate) fn read_payload(save_directory: &str, filename: &str) -> Result<GameSaveFile> {
    validate_generated_filename(filename)?;
    let storage = BrowserLocalStorage::open()?;
    let key = browser_key(save_directory, &format!("payload.{filename}"));
    let encoded = storage
        .read_text(&key)
        .map_err(|error| anyhow::anyhow!("reading browser autosave payload failed: {error}"))?
        .with_context(|| format!("browser autosave payload {filename:?} is missing"))?;
    decode_browser_blob(&encoded)
}

pub(crate) fn payload_exists(save_directory: &str, filename: &str) -> Result<bool> {
    validate_generated_filename(filename)?;
    let storage = BrowserLocalStorage::open()?;
    storage
        .read_text(&browser_key(save_directory, &format!("payload.{filename}")))
        .map(|value| value.is_some())
        .map_err(|error| anyhow::anyhow!("checking browser autosave payload failed: {error}"))
}

pub(crate) fn read_thumbnail(save_directory: &str, filename: &str) -> Result<Option<Thumbnail>> {
    validate_generated_filename(filename)?;
    let storage = BrowserLocalStorage::open()?;
    let key = browser_key(save_directory, &format!("thumbnail.{filename}"));
    let Some(encoded) = storage
        .read_text(&key)
        .map_err(|error| anyhow::anyhow!("reading browser autosave thumbnail failed: {error}"))?
    else {
        return Ok(None);
    };
    decode_browser_blob(&encoded).map(Some)
}

pub(crate) fn remove_payload(save_directory: &str, filename: &str) -> Result<()> {
    validate_generated_filename(filename)?;
    let storage = BrowserLocalStorage::open()?;
    for suffix in [
        format!("payload.{filename}"),
        format!("thumbnail.{filename}"),
    ] {
        storage
            .remove(&browser_key(save_directory, &suffix))
            .map_err(|error| anyhow::anyhow!("removing browser autosave failed: {error}"))?;
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
    let storage = BrowserLocalStorage::open()?;
    let namespace = browser_namespace(save_directory);
    let mut remove = Vec::new();
    for key in storage
        .keys()
        .map_err(|error| anyhow::anyhow!("enumerating browser autosave storage failed: {error}"))?
    {
        if let Some(filename) = browser_autosave_filename_from_key(&namespace, &key)
            && !referenced.contains(filename)
        {
            remove.push(key);
        }
    }
    for key in remove {
        storage.remove(&key).map_err(|error| {
            anyhow::anyhow!("removing orphan browser autosave {key:?} failed: {error}")
        })?;
    }
    Ok(())
}
