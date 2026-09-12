//! Small JSON text stores with the same platform and private-file policy.
//!
//! This deliberately excludes autosave workers and disposable mod caches:
//! their scheduling, transaction and eviction contracts are different.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("failed to read leaderboard state {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to persist leaderboard state {path}: {source}")]
    Persist {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[cfg(target_arch = "wasm32")]
    #[error("browser leaderboard storage unavailable: {0}")]
    Browser(String),
}

pub(super) fn display_path(file: &str, browser_key: &str) -> PathBuf {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = browser_key;
        crate::save_file::default_save_directory().join(file)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = file;
        PathBuf::from(browser_key)
    }
}

pub(super) fn read(file: &str, browser_key: &str) -> Result<Option<String>, StoreError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = display_path(file, browser_key);
        super::storage::read_private_utf8(&path).map_err(|source| StoreError::Read { path, source })
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = file;
        crate::browser_storage::local_storage()
            .map_err(StoreError::Browser)?
            .get_item(browser_key)
            .map_err(|error| StoreError::Browser(format!("{error:?}")))
    }
}

pub(super) fn write(
    file: &str,
    browser_key: &str,
    prefix: &str,
    encoded: &[u8],
) -> Result<(), StoreError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = display_path(file, browser_key);
        super::storage::replace_private(&path, prefix, encoded)
            .map_err(|source| StoreError::Persist { path, source })
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (file, prefix);
        let encoded = std::str::from_utf8(encoded)
            .map_err(|error| StoreError::Browser(format!("JSON text is not UTF-8: {error}")))?;
        crate::browser_storage::local_storage()
            .map_err(StoreError::Browser)?
            .set_item(browser_key, encoded)
            .map_err(|error| StoreError::Browser(format!("{error:?}")))
    }
}
