//! Native trust storage: one JSON file inside the selected save directory.

use super::SpellforgeTrustStore;
use crate::blob_store::BlobStoreError;
use std::path::{Path, PathBuf};

pub(super) const NATIVE_STORE_FILE: &str = "spellforge-trust.json";

impl SpellforgeTrustStore {
    pub(super) fn store_path(directory: &str) -> PathBuf {
        Path::new(directory).join(NATIVE_STORE_FILE)
    }
}

/// Create the directory up front so its failure keeps the user-facing
/// "create Spellforge trust directory" message profile recovery reports.
pub(super) fn create_store_directory(directory: &str) -> Result<(), String> {
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("create Spellforge trust directory {directory}: {error}"))
}

pub(super) fn describe_store_error(error: BlobStoreError, operation: &str, path: &Path) -> String {
    format!("{operation} {}: {error}", path.display())
}
