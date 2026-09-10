//! Read-only receipt interpretation. No catalog mutation or receipt retirement:
//! the owner applies candidates before publishing an index and retiring evidence.
use super::{QuickSaveRecovery, SaveGame, SpecialSaveRecovery};
use crate::save_file;
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Missing payloads are expected after an interrupted publication; other read
/// failures must remain visible. Hash in bounded storage, not a save-sized Vec.
fn payload_digest(path: &Path) -> std::io::Result<Option<[u8; 32]>> {
    use std::io::Read;
    let mut input = match std::fs::File::open(path) {
        Ok(input) => input,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut digest = Sha256::new();
    let mut buffer = [0; 8192];
    loop {
        match input.read(&mut buffer) {
            Ok(0) => return Ok(Some(digest.finalize().into())),
            Ok(read) => digest.update(&buffer[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

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
    if payload_digest(&path).context("read owned recovery payload")? == Some(receipt.digest) {
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
        let actual =
            payload_digest(&path).with_context(|| format!("reading {}", path.display()))?;
        if actual != Some(digest) {
            // This prospective payload was not published, or a later
            // operation replaced it. Never install stale receipt metadata.
            continue;
        }
        candidates.push(slot);
    }
    Ok(Some(candidates))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamed_payload_digest_matches_whole_file_and_distinguishes_absence() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("save.json");
        assert_eq!(payload_digest(&path).unwrap(), None);
        for size in [0, 1, 8191, 8192, 8193, 131_072] {
            let bytes: Vec<_> = (0..size).map(|index| (index % 251) as u8).collect();
            std::fs::write(&path, &bytes).unwrap();
            assert_eq!(
                payload_digest(&path).unwrap(),
                Some(Sha256::digest(&bytes).into())
            );
        }
        assert!(payload_digest(&path.join("not-a-directory")).is_err());
        assert!(payload_digest(directory.path()).is_err());
    }
}
