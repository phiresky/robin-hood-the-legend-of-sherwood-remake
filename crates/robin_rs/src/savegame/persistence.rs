//! Durable index publication boundary. Accepts only a data snapshot and the
//! caller-bound root; it cannot change runtime slot identity or lifecycle.
use super::{SaveIndex, SpecialSaveRecovery};
use crate::save_file;
use std::path::Path;

pub(super) fn publish_payload(
    receipt_path: &Path,
    payload_path: &Path,
    receipt: &SpecialSaveRecovery,
    bytes: &[u8],
    overwrite: bool,
) -> anyhow::Result<()> {
    receipt.slot.validate_published_metadata()?;
    anyhow::ensure!(
        !receipt.slot.is_autosave(),
        "owned publication cannot target an autosave"
    );
    #[cfg(test)]
    fail_at(FailurePoint::BeforeReceipt)?;
    save_file::atomic_write(receipt_path, &serde_json::to_vec(receipt)?)?;
    #[cfg(test)]
    fail_at(FailurePoint::BeforePayload)?;
    if overwrite {
        save_file::atomic_write(payload_path, bytes)?;
    } else {
        save_file::atomic_write_new(payload_path, bytes)?;
    }
    #[cfg(test)]
    fail_at(FailurePoint::AfterPayload)?;
    Ok(())
}

pub(super) fn publish_index(
    root: &str,
    index: &SaveIndex,
    quick_receipt: &Path,
) -> Result<(), String> {
    #[cfg(test)]
    fail_at(FailurePoint::BeforeIndex).map_err(|error| error.to_string())?;
    let path = Path::new(root).join("saves.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let json = serde_json::to_string_pretty(index).map_err(|e| format!("serialize: {e}"))?;
    save_file::atomic_write(&path, json.as_bytes()).map_err(|e| format!("write: {e:#}"))?;
    #[cfg(test)]
    fail_at(FailurePoint::AfterIndex).map_err(|error| error.to_string())?;
    // Retire recovery only after the authoritative index is durable. This
    // also protects subsequent index-only edits from stale receipt replay.
    match std::fs::remove_file(quick_receipt) {
        Ok(()) => {
            #[cfg(all(unix, not(target_arch = "wasm32")))]
            std::fs::File::open(root)
                .and_then(|directory| directory.sync_all())
                .map_err(|error| format!("sync receipt retirement: {error}"))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("retire quick-save recovery receipt: {error}")),
    }
    Ok(())
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum FailurePoint {
    BeforeReceipt,
    BeforePayload,
    AfterPayload,
    BeforeIndex,
    AfterIndex,
}

#[cfg(test)]
thread_local! {
    static FAILURE: std::cell::Cell<Option<FailurePoint>> = const { std::cell::Cell::new(None) };
}

#[cfg(all(test, not(target_arch = "wasm32")))]
pub(super) fn inject_failure(point: FailurePoint) {
    FAILURE.set(Some(point));
}

#[cfg(test)]
fn fail_at(point: FailurePoint) -> anyhow::Result<()> {
    if FAILURE.get() == Some(point) {
        FAILURE.set(None);
        anyhow::bail!("injected publication failure at {point:?}");
    }
    Ok(())
}
