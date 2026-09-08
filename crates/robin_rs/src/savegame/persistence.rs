//! Durable index publication boundary. Accepts only a data snapshot and the
//! caller-bound root; it cannot change runtime slot identity or lifecycle.
use super::SaveIndex;
use crate::save_file;
use std::path::Path;

pub(super) fn publish_index(
    root: &str,
    index: &SaveIndex,
    quick_receipt: &Path,
) -> Result<(), String> {
    let path = Path::new(root).join("saves.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let json = serde_json::to_string_pretty(index).map_err(|e| format!("serialize: {e}"))?;
    save_file::atomic_write(&path, json.as_bytes()).map_err(|e| format!("write: {e:#}"))?;
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
