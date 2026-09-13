//! Browser trust storage: one page-global localStorage entry.

use super::SpellforgeTrustStore;
use crate::blob_store::BlobStoreError;

const BROWSER_STORE_KEY: &str = "robin-hood-spellforge-trust-v1";

impl SpellforgeTrustStore {
    /// Browser trust grants are global to the page's localStorage.
    pub(super) fn store_path(_directory: &str) -> &'static str {
        BROWSER_STORE_KEY
    }
}

/// localStorage has no directories; the single key needs no preparation.
pub(super) fn create_store_directory(_directory: &str) -> Result<(), String> {
    Ok(())
}

pub(super) fn describe_store_error(error: BlobStoreError, operation: &str, _key: &str) -> String {
    error
        .into_io(&format!("{operation} browser Spellforge trust store"))
        .to_string()
}
