//! Read-only receipt interpretation. No catalog mutation or receipt retirement:
//! the owner applies candidates before publishing an index and retiring evidence.
use super::{QuickSaveRecovery, SaveGame, SpecialSaveRecovery};
use crate::save_file;
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::Path;

pub(super) fn owned_candidate(root: &str, receipt_path: &Path) -> Result<Option<SaveGame>> {
    let bytes = match std::fs::read(receipt_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("read owned save recovery receipt"),
    };
    let receipt: SpecialSaveRecovery =
        serde_json::from_slice(&bytes).context("decode owned save recovery receipt")?;
    receipt.slot.validate_published_metadata()?;
    // Manual saves share this receipt. Basename and exact special-kind
    // agreement are validated above; autosaves retain manifest authority.
    anyhow::ensure!(
        !receipt.slot.is_autosave(),
        "owned recovery receipt cannot target an autosave"
    );
    let path = Path::new(root).join(format!("{}.json", receipt.slot.filename));
    let payload = match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("read owned recovery payload"),
    };
    if payload
        .as_ref()
        .is_some_and(|bytes| <[u8; 32]>::from(Sha256::digest(bytes)) == receipt.digest)
    {
        return Ok(Some(receipt.slot));
    }
    Ok(None)
}

pub(super) fn quick_candidates(root: &str, receipt_path: &Path) -> Result<Option<Vec<SaveGame>>> {
    let bytes = match std::fs::read(receipt_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("reading quick-save recovery receipt"),
    };
    let recovery: QuickSaveRecovery =
        serde_json::from_slice(&bytes).context("decoding quick-save recovery receipt")?;
    let mut candidates = Vec::new();
    // Validate the whole receipt before any referenced payload is read.
    for (slot, _) in &recovery.slots {
        slot.validate_published_metadata()?;
        anyhow::ensure!(
            matches!(
                slot.filename.as_str(),
                save_file::special_slots::QUICK | save_file::special_slots::EX_QUICK
            ),
            "recovery receipt names a non-quick-save slot"
        );
    }
    for (slot, digest) in recovery.slots {
        let path = Path::new(root).join(format!("{}.json", slot.filename));
        let payload = match std::fs::read(&path) {
            Ok(payload) => payload,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", path.display()));
            }
        };
        let actual: [u8; 32] = Sha256::digest(&payload).into();
        if actual != digest {
            // This prospective payload was not published, or a later
            // operation replaced it. Never install stale receipt metadata.
            continue;
        }
        candidates.push(slot);
    }
    Ok(Some(candidates))
}
