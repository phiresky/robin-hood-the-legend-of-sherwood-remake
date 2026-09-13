//! Android datadir setup: an optional loose override over the APK asset bundle.

use std::path::Path;

use robin_engine::sbfile::SbFileSystem;

use super::{configured_data_dir, install_primary_data_dir};
use crate::main_entry::init::InitError;

/// Android uses a pre-converted shipping datadir bundled as an APK
/// asset. If loose files are present (developer override), set the cwd
/// up the same way as desktop; otherwise rely on the installed
/// `ShippingDatadir` / `asset_fs` bundle.
pub fn setup_data_dir(
    data_dir_override: Option<&Path>,
    files: &SbFileSystem,
) -> Result<(), InitError> {
    let data_dir = configured_data_dir(data_dir_override, std::env::var("ROBINHOOD_DATA_DIR").ok());
    if let Some(data_dir) = data_dir {
        install_primary_data_dir(files, data_dir)?;
    }

    if robin_engine::sbfile::resolve_case_insensitive(Path::new("Data")).is_none()
        && files.mount_snapshot().asset_vfs.is_empty()
    {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "?".into());
        return Err(InitError::DataDirectoryAndroidAssetsMissing { cwd });
    }

    Ok(())
}
