//! Browser Spellforge mission wiring.
//!
//! Gameplay uses the same pure-Rust Lua 5.1 runtime as native. Package
//! extraction is in-memory, so the interpreter never depends on a browser
//! filesystem; the launcher's mounted archive path is only the byte source.

use robin_engine::natives::{ScriptEffects, ScriptState};
use robin_engine::spellforge::{SpellforgePackage, SpellforgeRuntime};
use robin_spellforge::{ARCHIVE_BYTE_LIMIT, SpellforgeRuntime51, build_package_from_archives};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::main_entry::CliArgs;
use crate::main_menu::custom_missions::CustomMissionLaunch;

pub struct LuaSession {
    mission_basename: String,
    runtime: Arc<SpellforgeRuntime51>,
}

#[derive(Debug, thiserror::Error)]
pub enum SpellforgeSessionError {
    #[error(
        "Spellforge mission `{mission}` is disabled in Gameplay settings; enable `Enable Spellforge Missions` to launch it"
    )]
    Disabled { mission: String },
    #[error("cannot read the active profile while starting Spellforge: {0}")]
    Profile(String),
    #[error("vanilla custom mission `{mission}` unexpectedly carried a Spellforge package")]
    UnexpectedPackage { mission: String },
    #[error("required Spellforge startup failed for mission `{mission}`: {detail}")]
    Startup { mission: String, detail: String },
    #[error("required Spellforge session package is missing for mission `{mission}`")]
    RequiredSessionMissing { mission: String },
}

pub fn validate_launch_mode(
    _args: &CliArgs,
    _pending_replay: bool,
) -> Result<(), SpellforgeSessionError> {
    Ok(())
}

impl LuaSession {
    pub fn start_from_package(
        mission_basename: impl Into<String>,
        package: SpellforgePackage,
    ) -> Result<Self, SpellforgeSessionError> {
        let mission_basename = mission_basename.into();
        robin_spellforge::validate_package(&package).map_err(|error| {
            SpellforgeSessionError::Startup {
                mission: mission_basename.clone(),
                detail: error.to_string(),
            }
        })?;
        let runtime = Arc::new(SpellforgeRuntime51::new(package).map_err(|error| {
            SpellforgeSessionError::Startup {
                mission: mission_basename.clone(),
                detail: error.to_string(),
            }
        })?);
        Ok(Self {
            mission_basename,
            runtime,
        })
    }

    pub fn start_for_launch(
        launch: &CustomMissionLaunch,
        mods_root: &Path,
    ) -> Result<Option<Self>, SpellforgeSessionError> {
        if !launch.requires_spellforge {
            return Ok(None);
        }
        let package =
            build_package(launch, mods_root).map_err(|detail| SpellforgeSessionError::Startup {
                mission: launch.rhm_basename.clone(),
                detail,
            })?;
        let runtime = Arc::new(SpellforgeRuntime51::new(package).map_err(|detail| {
            SpellforgeSessionError::Startup {
                mission: launch.rhm_basename.clone(),
                detail: detail.to_string(),
            }
        })?);
        Ok(Some(Self {
            mission_basename: launch.rhm_basename.clone(),
            runtime,
        }))
    }

    pub fn mission_basename(&self) -> &str {
        &self.mission_basename
    }

    pub fn runtime(&self) -> Arc<dyn SpellforgeRuntime> {
        self.runtime.clone()
    }

    /// Engine construction dispatches the authoritative Initialize pair.
    pub fn run_required_startup_events(
        &self,
        _native_parts: Option<(
            &mut ScriptEffects,
            &mut ScriptState,
            &mut robin_engine::engine::ScriptDomains,
            &robin_engine::natives::AttachedScriptBindings,
            &robin_engine::natives::NativeSessionCapabilities<'_>,
        )>,
        _initialization_seed: i32,
    ) -> Result<(), SpellforgeSessionError> {
        Ok(())
    }
}

fn build_package(
    launch: &CustomMissionLaunch,
    mods_root: &Path,
) -> Result<SpellforgePackage, String> {
    let mission_bytes = match &launch.version_zip_bytes {
        Some(bytes) => bounded_archive_bytes(&launch.version_zip, bytes)?,
        None => read_archive_bytes(&launch.version_zip)?,
    };
    let shared_bytes = find_shared_lib_zip(mods_root)
        .map(|path| read_archive_bytes(&path))
        .transpose()?;
    build_package_from_archives(
        &mission_bytes,
        &launch.rhm_zip_entry,
        &launch.rhm_basename,
        shared_bytes.as_deref(),
    )
    .map_err(|error| error.to_string())
}

fn bounded_archive_bytes(path: &Path, bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() > ARCHIVE_BYTE_LIMIT {
        return Err(format!(
            "Spellforge archive {} is {} bytes; limit is {ARCHIVE_BYTE_LIMIT}",
            path.display(),
            bytes.len()
        ));
    }
    Ok(bytes.to_vec())
}

fn read_archive_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let declared = std::fs::metadata(path)
        .map_err(|error| format!("reading mounted archive {}: {error}", path.display()))?
        .len();
    if declared > ARCHIVE_BYTE_LIMIT as u64 {
        return Err(format!(
            "Spellforge archive {} is {declared} bytes; limit is {ARCHIVE_BYTE_LIMIT}",
            path.display()
        ));
    }
    let bytes = std::fs::read(path)
        .map_err(|error| format!("reading mounted archive {}: {error}", path.display()))?;
    bounded_archive_bytes(path, &bytes)
}

fn find_shared_lib_zip(mods_root: &Path) -> Option<PathBuf> {
    let mut entries = std::fs::read_dir(mods_root.join("lib"))
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries.pop()
}
