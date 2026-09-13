//! Native (desktop and Android) startup hooks.

use std::path::Path;

use robin_engine::profiles as engine_profiles;
use robin_engine::sbfile::SbFileSystem;

use crate::host::ApplicationContext;
use crate::main_entry::init::{InitError, MODS_DIR, resolve_install_resource_dir};
use crate::main_entry::launch::{MissionContent, MissionRequest};

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

/// Admit a direct `--custom-mission` archive before profile selection. Returns
/// the launch unchanged when it names no custom mission.
pub fn prepare_direct_custom_mission_args(
    request: MissionRequest,
    profiles: &engine_profiles::ProfileManager,
    application_context: &ApplicationContext,
) -> Result<MissionRequest, crate::main_entry::LaunchError> {
    use crate::main_entry::LaunchError;
    let Some(archive) = request.content.custom_mission.as_deref() else {
        return Ok(request);
    };
    let cli = &request.config.cli;
    let mission = cli.mission.as_deref().ok_or_else(|| {
        LaunchError::arguments(
            "--custom-mission requires --mission even when arguments bypass clap",
        )
    })?;
    let map = cli
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
        cli.custom_mission_entry.as_deref(),
    )
    // TODO(10/F11): leaf returns String (custom-mission archive admission).
    .map_err(|error| LaunchError::content(format!("--custom-mission: {error}")))?;

    if prepared.spellforge_package.is_some() {
        return Err(LaunchError::arguments(
            "--custom-mission accepts vanilla archives only; launch Spellforge content from the Custom Missions menu",
        ));
    }
    // The archive path stays part of the launch; the admitted lease joins it.
    let content = MissionContent {
        resolved_mission_assets: Some(prepared.resolved),
        ..request.content
    };
    Ok(MissionRequest { content, ..request })
}
