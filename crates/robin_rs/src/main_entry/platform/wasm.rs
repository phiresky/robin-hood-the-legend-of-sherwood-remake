//! Browser (wasm32) startup hooks.

use std::path::Path;

use robin_engine::profiles as engine_profiles;
use robin_engine::sbfile::SbFileSystem;

use crate::host::ApplicationContext;
use crate::main_entry::cli::MissionLaunch;
use crate::main_entry::init::InitError;

/// Wasm version: there is no cwd or directory enumeration.  The Data/
/// prefix is anchored at `ROBINHOOD_DATA_URL` (default `./data`), which
/// `robin_util::asset_fs` consults for every read.  All we do here is
/// bootstrap language-folder detection.
pub fn setup_data_dir(
    _data_dir_override: Option<&Path>,
    _files: &SbFileSystem,
) -> Result<(), InitError> {
    Ok(())
}

/// Browser builds ship no installation `mods/` directory.
pub fn overlay_mods_dir() -> Option<std::path::PathBuf> {
    None
}

pub fn prepare_direct_custom_mission_args(
    args: &MissionLaunch,
    _profiles: &engine_profiles::ProfileManager,
    _application_context: &ApplicationContext,
) -> Result<Option<MissionLaunch>, crate::main_entry::LaunchError> {
    if args.custom_mission.is_some() {
        return Err(crate::main_entry::LaunchError::arguments(
            "--custom-mission filesystem paths are unavailable in browser builds; use canonical host-distributed content",
        ));
    }
    Ok(None)
}
