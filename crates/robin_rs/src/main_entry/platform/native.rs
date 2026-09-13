//! Native (desktop and Android) startup hooks.

use std::path::Path;

use robin_engine::profiles as engine_profiles;
use robin_engine::sbfile::SbFileSystem;

use crate::host::ApplicationContext;
use crate::main_entry::cli::MissionLaunch;
use crate::main_entry::init::{InitError, MODS_DIR, resolve_install_resource_dir};

#[cfg(target_os = "android")]
mod android;
#[cfg(not(target_os = "android"))]
mod desktop;

#[cfg(target_os = "android")]
pub use android::setup_data_dir;
#[cfg(not(target_os = "android"))]
pub use desktop::setup_data_dir;

/// Resolve the repository/install `mods/` directory whose subdirectories
/// are auto-mounted as overlay datadirs.  `None` when the installation
/// ships no such directory.  Also scanned by the Custom Missions picker
/// so overlay-shipped mods (hackable levels) can carry a `details.json`.
pub fn overlay_mods_dir() -> Option<std::path::PathBuf> {
    resolve_install_resource_dir(MODS_DIR)
}

fn configured_data_dir(explicit: Option<&Path>, environment: Option<String>) -> Option<String> {
    explicit
        .map(|dir| dir.to_string_lossy().into_owned())
        .or_else(|| environment.filter(|dir| !dir.is_empty()))
}

fn install_primary_data_dir(files: &SbFileSystem, path: String) -> Result<(), InitError> {
    files
        .set_primary_path(&path)
        .map_err(|status| InitError::DataDirectoryInstall { path, status })
}

#[cfg(test)]
#[test]
fn explicit_datadir_precedes_environment_without_hiding_an_empty_override() {
    assert_eq!(
        configured_data_dir(Some(Path::new("explicit")), Some("environment".into())),
        Some("explicit".into())
    );
    assert_eq!(
        configured_data_dir(Some(Path::new("")), Some("environment".into())),
        Some(String::new())
    );
    assert_eq!(configured_data_dir(None, Some(String::new())), None);
    assert_eq!(configured_data_dir(None, None), None);
    assert_eq!(
        configured_data_dir(None, Some("environment".into())),
        Some("environment".into())
    );
}

pub fn prepare_direct_custom_mission_args(
    args: &MissionLaunch,
    profiles: &engine_profiles::ProfileManager,
    application_context: &ApplicationContext,
) -> Result<Option<MissionLaunch>, String> {
    let Some(archive) = args.custom_mission.as_deref() else {
        return Ok(None);
    };
    let mission = args.mission.as_deref().ok_or_else(|| {
        "--custom-mission requires --mission even when arguments bypass clap".to_owned()
    })?;
    let map = args
        .proto
        .clone()
        .or_else(|| {
            profiles
                .missions
                .iter()
                .find(|profile| profile.mission_filename.eq_ignore_ascii_case(mission))
                .map(|profile| profile.proto_level_filename.clone())
        })
        .unwrap_or_else(|| mission.to_owned());

    let prepared = crate::mission_asset_launch::prepare_direct_custom_mission(
        application_context,
        archive,
        mission,
        &map,
        args.custom_mission_entry.as_deref(),
    )
    .map_err(|error| format!("--custom-mission: {error}"))?;

    if prepared.spellforge_package.is_some() {
        return Err(
            "--custom-mission accepts vanilla archives only; launch Spellforge content from the Custom Missions menu"
                .to_owned(),
        );
    }
    let mut prepared_args = args.clone();
    prepared_args.resolved_mission_assets = Some(prepared.resolved);
    Ok(Some(prepared_args))
}
