//! Desktop datadir discovery: explicit override, environment, or the locator.

use std::path::{Path, PathBuf};

use robin_engine::sbfile::SbFileSystem;

use super::{configured_data_dir, install_primary_data_dir};
use crate::main_entry::init::{InitError, add_language_folder_with_files, add_overlay_data_dirs};

/// Set up the working directory so that `Data/` is accessible.
///
/// `data_dir_override` (e.g. a tool's `--data-dir` flag) takes priority
/// over the `ROBINHOOD_DATA_DIR` environment variable.
pub fn setup_data_dir(
    data_dir_override: Option<&Path>,
    files: &SbFileSystem,
) -> Result<(), InitError> {
    let data_dir = configured_data_dir(data_dir_override, std::env::var("ROBINHOOD_DATA_DIR").ok());
    if let Some(data_dir) = data_dir {
        tracing::info!("using primary datadir {}", data_dir);
        install_primary_data_dir(files, data_dir)?;
    } else {
        // No override and no env var: reuse the remembered datadir, or
        // auto-detect (working directory, executable directory, well-known
        // CD/GOG/Steam install locations — validated via Data/robinhood.bks)
        // and confirm with the player through the native dialog / folder
        // picker. See `datadir_locator::resolve_datadir`.
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        // Only non-interactive discovery may fall back to loose, unmarked Data/.
        // Cancelling the picker must stop startup before installing any data.
        let chosen = startup_data_dir(crate::datadir_locator::resolve_datadir(exe_dir.as_deref()))?;
        tracing::info!("using primary datadir {}", chosen.display());
        install_primary_data_dir(files, chosen.to_string_lossy().into_owned())?;
    }

    // Find the Data directory case-insensitively (some installs use "data", "DATA", etc.)
    if !files
        .try_exists("Data")
        .map_err(|status| InitError::DataDirectoryInstall {
            path: "Data".into(),
            status,
        })?
    {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "?".into());
        return Err(InitError::DataDirectoryMissing {
            cwd,
            gog_store_url: crate::datadir_locator::GOG_STORE_URL,
        });
    }

    add_overlay_data_dirs(files)?;
    add_language_folder_with_files(files)?;
    Ok(())
}

fn startup_data_dir(
    resolution: crate::datadir_locator::DataDirResolution,
) -> Result<PathBuf, InitError> {
    use crate::datadir_locator::DataDirResolution;
    match resolution {
        DataDirResolution::Selected(path) => Ok(path),
        DataDirResolution::Unavailable => Ok(PathBuf::from(".")),
        DataDirResolution::Cancelled => Err(InitError::DataDirectoryCancelled),
    }
}

#[cfg(test)]
#[test]
fn datadir_cancellation_does_not_fall_back_to_working_directory() {
    use crate::datadir_locator::DataDirResolution;
    assert!(matches!(
        startup_data_dir(DataDirResolution::Cancelled),
        Err(InitError::DataDirectoryCancelled)
    ));
    assert_eq!(
        startup_data_dir(DataDirResolution::Unavailable).unwrap(),
        PathBuf::from(".")
    );
    let selected = PathBuf::from("/chosen/game");
    assert_eq!(
        startup_data_dir(DataDirResolution::Selected(selected.clone())).unwrap(),
        selected
    );
}
