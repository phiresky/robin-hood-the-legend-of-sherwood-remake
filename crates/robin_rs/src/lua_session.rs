//! Shared native/browser host wiring for the deterministic Spellforge runtime.
//!
//! When a Spellforge custom mission is launched, the picker hands us a
//! [`crate::main_menu::custom_missions::CustomMissionLaunch`] with the
//! version zip and the basename of the chosen `.rhm`. We:
//!
//! 1. Read the matching `.lua` companion file and the shared
//!    `lib/*.lua` helpers from the mounted overlay zips.
//! 2. Build the exact versioned package from those bytes.
//! 3. Construct the shared safe-Rust Lua 5.1 runtime used by native and wasm.
//!
//! Live event dispatch is owned by the engine's sole synchronous callback
//! driver. The host session installs a versioned package/runtime attachment;
//! timer, victory, finalize, and every per-entity callback then pass through
//! the same yield/resume path as SCB.
//! The optional native Lua VM exists only in the comparison/unwind test
//! harness (`tests/legacy_vm.rs`); nothing in this file depends on it.

use robin_engine::spellforge::SpellforgePackage;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use robin_spellforge::{
    ARCHIVE_BYTE_LIMIT, SpellforgePackageError, SpellforgeRuntime51, build_package_from_archives,
};

use crate::main_entry::CliArgs;
use crate::main_menu::custom_missions::CustomMissionLaunch;

/// One mission's worth of Lua state, attached to a launched custom
/// Spellforge mission for as long as the session runs.
pub struct LuaSession {
    /// Bare basename of the mission script — the `.lua` filename without the
    /// extension. Used in diagnostics around the engine-owned runtime.
    mission_basename: String,
    /// Engine-facing deterministic runtime. Live mission events use this one
    /// through `LevelAssets` so saves, rollback and networking share a tape.
    runtime: Arc<SpellforgeRuntime51>,
}

#[derive(Debug, thiserror::Error)]
pub enum LuaSessionError {
    #[error("opening mission zip {0}: {1}")]
    OpenZip(PathBuf, #[source] std::io::Error),
    #[error("reading mission zip {0}: {1}")]
    ZipReader(PathBuf, #[source] zip::result::ZipError),
    #[error("no `.lua` found alongside {rhm_entry} in {zip}")]
    NoLuaCompanion { zip: PathBuf, rhm_entry: String },
    #[error("writing {0}: {1}")]
    WriteFile(PathBuf, #[source] std::io::Error),
    #[error("invalid Spellforge contract: {0}")]
    Contract(String),
    #[error("invalid Spellforge package: {0}")]
    Package(#[from] SpellforgePackageError),
    #[error("Spellforge archive {path} is {bytes} bytes; limit is {limit}")]
    ArchiveLimit {
        path: PathBuf,
        bytes: u64,
        limit: usize,
    },
}

/// Contextual failures at the boundary between mission setup and Spellforge.
#[derive(Debug, thiserror::Error)]
pub enum SpellforgeSessionError {
    #[error(
        "Spellforge mission `{mission}` is disabled in Gameplay settings; enable `Enable Spellforge Missions` to launch it"
    )]
    Disabled { mission: String },
    #[error("cannot read the active profile while starting Spellforge: {0}")]
    Profile(String),
    #[error("required Spellforge startup failed for mission `{mission}`: {source}")]
    Startup {
        mission: String,
        #[source]
        source: LuaSessionError,
    },
    #[error("required Spellforge session was not created for mission `{mission}`")]
    RequiredSessionMissing { mission: String },
    #[error("vanilla mission `{mission}` unexpectedly carried a Spellforge package")]
    UnexpectedPackage { mission: String },
}

/// Spellforge's package/tape contract supports every native launch mode. Keep
/// this boundary so new modes cannot quietly add a host-only restriction.
pub fn validate_launch_mode(
    _args: &CliArgs,
    _pending_replay: bool,
) -> Result<(), SpellforgeSessionError> {
    Ok(())
}

impl LuaSession {
    /// Construct the live runtime from exact replay/network package bytes.
    /// This path deliberately performs no archive or local-filesystem lookup.
    pub fn start_from_package(
        mission_basename: impl Into<String>,
        package: SpellforgePackage,
    ) -> Result<Self, SpellforgeSessionError> {
        let mission_basename = mission_basename.into();
        let startup = |source| SpellforgeSessionError::Startup {
            mission: mission_basename.clone(),
            source,
        };
        robin_spellforge::validate_package(&package)
            .map_err(|error| startup(LuaSessionError::Contract(error.to_string())))?;
        let runtime = Arc::new(
            SpellforgeRuntime51::new(package)
                .map_err(|error| startup(LuaSessionError::Contract(error.to_string())))?,
        );
        Ok(Self {
            mission_basename,
            runtime,
        })
    }

    /// Build a Lua session for the chosen mission, or return `None` only when
    /// the launch is Vanilla. A Spellforge launch with no companion or any
    /// extraction/loading failure returns a typed error; callers must not
    /// continue with only the engine's `.scb` path.
    pub fn start(
        launch: &CustomMissionLaunch,
        mods_root: &Path,
    ) -> Result<Option<Self>, LuaSessionError> {
        if !launch.requires_spellforge {
            tracing::info!(
                "LuaSession: mission '{}' is Vanilla — no Lua state",
                launch.rhm_basename
            );
            return Ok(None);
        }
        let mission_basename = launch.rhm_basename.clone();
        let mission_archive = match &launch.version_zip_bytes {
            Some(bytes) => bounded_archive_bytes(&launch.version_zip, bytes)?,
            None => read_archive_bytes(&launch.version_zip)?,
        };
        let shared_archive = find_shared_lib_zip(mods_root)
            .map(|path| read_archive_bytes(&path))
            .transpose()?;
        let package = build_package_from_archives(
            &mission_archive,
            &launch.rhm_zip_entry,
            &mission_basename,
            shared_archive.as_deref(),
        )?;

        // Do not load mission source into the legacy direct-call VM here.
        // Doing so would execute top-level Lua twice and make native launch
        // acceptance depend on a second dialect that browser builds do not
        // use. `robin_lua` remains available to its developer tools/tests;
        // live gameplay exclusively uses the runtime below.
        let runtime = Arc::new(
            SpellforgeRuntime51::new(package)
                .map_err(|error| LuaSessionError::Contract(error.to_string()))?,
        );

        Ok(Some(Self {
            mission_basename,
            runtime,
        }))
    }

    /// Build the session required by a launch and retain mission context on
    /// every failure. A Spellforge-tagged launch returning `None` is an
    /// invariant violation rather than permission to continue without Lua.
    pub fn start_for_launch(
        launch: &CustomMissionLaunch,
        mods_root: &Path,
    ) -> Result<Option<Self>, SpellforgeSessionError> {
        let session =
            Self::start(launch, mods_root).map_err(|source| SpellforgeSessionError::Startup {
                mission: launch.rhm_basename.clone(),
                source,
            })?;
        if launch.requires_spellforge && session.is_none() {
            return Err(SpellforgeSessionError::RequiredSessionMissing {
                mission: launch.rhm_basename.clone(),
            });
        }
        Ok(session)
    }

    /// Mission basename (e.g. `"H06_Lin_VL"`) — used in log lines.
    pub fn mission_basename(&self) -> &str {
        &self.mission_basename
    }

    pub fn runtime(&self) -> Arc<dyn robin_engine::spellforge::SpellforgeRuntime> {
        self.runtime.clone()
    }
}

fn bounded_archive_bytes(path: &Path, bytes: &[u8]) -> Result<Vec<u8>, LuaSessionError> {
    if bytes.len() > ARCHIVE_BYTE_LIMIT {
        return Err(LuaSessionError::ArchiveLimit {
            path: path.to_path_buf(),
            bytes: bytes.len() as u64,
            limit: ARCHIVE_BYTE_LIMIT,
        });
    }
    Ok(bytes.to_vec())
}

fn read_archive_bytes(path: &Path) -> Result<Vec<u8>, LuaSessionError> {
    let declared = fs::metadata(path)
        .map_err(|error| LuaSessionError::OpenZip(path.to_path_buf(), error))?
        .len();
    if declared > ARCHIVE_BYTE_LIMIT as u64 {
        return Err(LuaSessionError::ArchiveLimit {
            path: path.to_path_buf(),
            bytes: declared,
            limit: ARCHIVE_BYTE_LIMIT,
        });
    }
    let bytes =
        fs::read(path).map_err(|error| LuaSessionError::OpenZip(path.to_path_buf(), error))?;
    bounded_archive_bytes(path, &bytes)
}

/// Find the newest `lib_*.zip` under `<mods_root>/lib/` — matches
/// what [`crate::mod_pack::mount_for_launch`] uses, so the Lua
/// session and the SbFile overlay see the same shared library.
fn find_shared_lib_zip(mods_root: &Path) -> Option<PathBuf> {
    let lib_dir = mods_root.join("lib");
    let mut entries: Vec<PathBuf> = fs::read_dir(&lib_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|f| f.to_str())
                    .is_some_and(|f| f.to_ascii_lowercase().ends_with(".zip"))
        })
        .collect();
    entries.sort();
    entries.pop()
}

/// All tests, including the legacy `robin_lua` direct-call VM harness
/// (`tests/legacy_vm.rs`), live behind this single gate.
#[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
mod tests;
