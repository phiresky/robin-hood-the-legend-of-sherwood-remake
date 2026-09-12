use super::SaveGame;
use crate::save_file::{GameSaveFile, Thumbnail};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

/// Number of independently loadable autosave generations retained per profile.
pub const AUTOSAVE_SLOT_COUNT: usize = 3;
pub(crate) const AUTOSAVE_MANIFEST_VERSION: u32 = 1;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const AUTOSAVE_MANIFEST_FILE: &str = "autosaves.json";

/// The authoritative list of autosaves published independently of manual saves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutosaveManifest {
    pub version: u32,
    pub saves: Vec<SaveGame>,
}

impl Default for AutosaveManifest {
    fn default() -> Self {
        Self {
            version: AUTOSAVE_MANIFEST_VERSION,
            saves: Vec::new(),
        }
    }
}

impl AutosaveManifest {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.version != AUTOSAVE_MANIFEST_VERSION {
            bail!(
                "unsupported autosave manifest version: expected {}, got {}",
                AUTOSAVE_MANIFEST_VERSION,
                self.version
            );
        }
        if self.saves.len() > AUTOSAVE_SLOT_COUNT {
            bail!(
                "autosave manifest contains {} slots; policy permits {AUTOSAVE_SLOT_COUNT}",
                self.saves.len()
            );
        }
        let mut filenames = std::collections::BTreeSet::new();
        for save in &self.saves {
            save.validate_published_metadata()?;
            if !crate::savegame::is_generated_autosave_filename(&save.filename) {
                bail!(
                    "autosave manifest contains non-autosave filename {:?}",
                    save.filename
                );
            }
            if !filenames.insert(&save.filename) {
                bail!(
                    "autosave manifest contains duplicate filename {:?}",
                    save.filename
                );
            }
        }
        Ok(())
    }
}

pub(crate) fn staged_manifest(
    existing: AutosaveManifest,
    metadata: SaveGame,
) -> (AutosaveManifest, Vec<String>) {
    let mut saves = existing.saves;
    saves.retain(|save| save.filename != metadata.filename);
    saves.push(metadata);
    let remove_count = saves.len().saturating_sub(AUTOSAVE_SLOT_COUNT);
    let evicted_filenames = saves
        .drain(..remove_count)
        .map(|save| save.filename)
        .collect();
    (
        AutosaveManifest {
            version: AUTOSAVE_MANIFEST_VERSION,
            saves,
        },
        evicted_filenames,
    )
}

/// Commit one immutable generation. The closure ordering is the crash-safety
/// contract: payload first, manifest publication second, obsolete generation
/// cleanup only after the new manifest is durable. Cleanup failures cannot
/// roll back an already-published recovery point and are repaired by orphan
/// collection on the next open/write.
pub(crate) fn commit_generation(
    existing: AutosaveManifest,
    metadata: SaveGame,
    mut write_payload: impl FnMut() -> Result<()>,
    mut publish_manifest: impl FnMut(&AutosaveManifest) -> Result<()>,
    mut cleanup_generation: impl FnMut(&str) -> Result<()>,
) -> Result<AutosaveManifest> {
    let (manifest, evicted_filenames) = staged_manifest(existing, metadata);
    manifest.validate()?;
    write_payload().context("committing autosave payload before manifest publication")?;
    publish_manifest(&manifest).context("publishing autosave manifest after payload commit")?;
    for filename in evicted_filenames {
        if let Err(error) = cleanup_generation(&filename) {
            tracing::warn!(
                filename,
                "published autosave but could not remove rotated generation: {error:#}"
            );
        }
    }
    Ok(manifest)
}

pub(crate) fn validate_metadata_payload_binding(
    metadata: &SaveGame,
    payload: &GameSaveFile,
) -> Result<()> {
    if !crate::savegame::is_generated_autosave_filename(&metadata.filename) {
        bail!("published autosave metadata has an invalid filename");
    }
    if metadata.mission_id != payload.header.mission_id {
        bail!(
            "autosave {:?} mission mismatch: manifest {}, payload {}",
            metadata.filename,
            metadata.mission_id,
            payload.header.mission_id
        );
    }
    if metadata.version != payload.header.version {
        bail!(
            "autosave {:?} version mismatch: manifest {}, payload {}",
            metadata.filename,
            metadata.version,
            payload.header.version
        );
    }
    let timestamp = metadata.timestamp.parse::<u64>().with_context(|| {
        format!(
            "autosave {:?} manifest timestamp is not an unsigned integer",
            metadata.filename
        )
    })?;
    if timestamp != payload.header.timestamp_unix {
        bail!(
            "autosave {:?} timestamp mismatch: manifest {}, payload {}",
            metadata.filename,
            timestamp,
            payload.header.timestamp_unix
        );
    }
    Ok(())
}

pub(crate) fn validate_generated_filename(filename: &str) -> Result<()> {
    if !crate::savegame::is_generated_autosave_filename(filename) {
        bail!("invalid autosave storage filename {filename:?}");
    }
    Ok(())
}

#[cfg(any(test, target_arch = "wasm32"))]
pub(crate) fn browser_autosave_filename_from_key<'a>(
    namespace: &str,
    key: &'a str,
) -> Option<&'a str> {
    let payload_prefix = format!("{namespace}.payload.");
    let thumbnail_prefix = format!("{namespace}.thumbnail.");
    let filename = key
        .strip_prefix(&payload_prefix)
        .or_else(|| key.strip_prefix(&thumbnail_prefix))?;
    crate::savegame::is_generated_autosave_filename(filename).then_some(filename)
}

// Browser autosaves use compressed, checksummed localStorage records. The
// browser's storage API commits each key atomically, and the separate manifest
// is published only after the immutable payload key succeeds.
#[cfg(target_arch = "wasm32")]
pub(crate) const BROWSER_STORAGE_PREFIX: &str = "robinhood.autosave.v1";

#[cfg(any(test, target_arch = "wasm32"))]
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BrowserBlob {
    pub(crate) version: u32,
    pub(crate) sha256: String,
    pub(crate) compressed_base64: String,
}

#[cfg(any(test, target_arch = "wasm32"))]
pub(crate) fn encode_browser_blob<T: Serialize>(value: &T) -> Result<String> {
    use sha2::{Digest, Sha256};
    let json = serde_json::to_vec(value).context("serializing browser autosave value")?;
    let mut encoded =
        base64::write::EncoderStringWriter::new(&base64::engine::general_purpose::STANDARD);
    zstd::stream::copy_encode(std::io::Cursor::new(&json), &mut encoded, 3)
        .context("compressing browser autosave value")?;
    let blob = BrowserBlob {
        version: 1,
        sha256: hex::encode(Sha256::digest(&json)),
        compressed_base64: encoded.into_inner(),
    };
    serde_json::to_string(&blob).context("serializing browser autosave envelope")
}

#[cfg(any(test, target_arch = "wasm32"))]
pub(crate) fn decode_browser_blob<T: for<'de> Deserialize<'de>>(encoded: &str) -> Result<T> {
    use sha2::{Digest, Sha256};
    let blob: BrowserBlob =
        serde_json::from_str(encoded).context("parsing browser autosave envelope")?;
    if blob.version != 1 {
        bail!(
            "unsupported browser autosave envelope version {}",
            blob.version
        );
    }
    let compressed = base64::read::DecoderReader::new(
        blob.compressed_base64.as_bytes(),
        &base64::engine::general_purpose::STANDARD,
    );
    let json = zstd::stream::decode_all(compressed)
        .context("decoding and decompressing browser autosave value")?;
    let actual = hex::encode(Sha256::digest(&json));
    if actual != blob.sha256 {
        bail!(
            "browser autosave checksum mismatch: expected {}, got {actual}",
            blob.sha256
        );
    }
    serde_json::from_slice(&json).context("parsing browser autosave value")
}

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::*;
#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(target_arch = "wasm32")]
pub(crate) use browser::*;
