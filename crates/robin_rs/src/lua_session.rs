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
//! The optional native Lua VM exists only in the comparison/unwind test harness.

#[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
use robin_engine::natives::{ScriptEffects, ScriptState};
use robin_engine::spellforge::SpellforgePackage;
#[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
use robin_engine::spellforge::{SPELLFORGE_CONTRACT_VERSION, SpellforgeScriptMode};
#[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
use robin_lua::{MissionLuaError, MissionLuaState, register_natives};
use robin_spellforge::{
    ARCHIVE_BYTE_LIMIT, SpellforgePackageError, SpellforgeRuntime51, build_package_from_archives,
};
#[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
use robin_spellforge::{compute_package_sha256, spellforge_vm_abi};
#[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
use tempfile::TempDir;

use crate::main_entry::CliArgs;
use crate::main_menu::custom_missions::CustomMissionLaunch;

/// One mission's worth of Lua state, attached to a launched custom
/// Spellforge mission for as long as the session runs.
pub struct LuaSession {
    /// Tempdir holding the extracted `.lua` files. Lives at least as
    /// long as `state` so `require()` lookups stay valid; dropped on
    /// session teardown.
    #[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
    _tempdir: TempDir,
    /// Legacy direct-call interpreter retained only for focused host/native
    /// adapter tests. Live missions never load their package into this second
    /// VM; all bootstrap and event code runs exactly once in `runtime`.
    #[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
    state: MissionLuaState,
    /// Bare basename of the mission script — the `.lua` filename without the
    /// extension. Used in diagnostics around the engine-owned runtime.
    mission_basename: String,
    /// Engine-facing deterministic runtime. The legacy direct state above is
    /// retained for developer tools/tests; live mission events use this one
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
    #[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
    #[error("lua: {0}")]
    Lua(#[from] MissionLuaError),
    #[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
    #[error("mlua: {0}")]
    Mlua(#[from] mlua::Error),
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
    #[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
    #[error("Lua event `{event}` failed for mission `{mission}`: {source}")]
    Event {
        mission: String,
        event: String,
        #[source]
        source: mlua::Error,
    },
    #[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
    #[error(
        "Lua event `{event}` for mission `{mission}` returned Lua {actual}; expected an integer, integral number, boolean, or nil"
    )]
    UnexpectedEventReturn {
        mission: String,
        event: String,
        actual: String,
    },
    #[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
    #[error(
        "Lua event `{event}` for mission `{mission}` returned integer {value}, which is outside the signed 32-bit game ABI range"
    )]
    EventIntegerOutOfRange {
        mission: String,
        event: String,
        value: i64,
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
    #[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
    #[error(
        "required Spellforge event `{event}` for mission `{mission}` has no mission-script ScriptEffects"
    )]
    MissingScriptEffects {
        mission: String,
        event: &'static str,
    },
    #[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
    #[error("required Spellforge event `{event}` failed for mission `{mission}`: {source}")]
    RequiredEvent {
        mission: String,
        event: &'static str,
        #[source]
        source: LuaSessionError,
    },
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
        #[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
        let (tempdir, state) = {
            let tempdir = TempDir::with_prefix("robin-lua-replay-").map_err(|error| {
                startup(LuaSessionError::WriteFile(
                    PathBuf::from("<tempdir>"),
                    error,
                ))
            })?;
            write_test_package(tempdir.path(), &package).map_err(&startup)?;
            let mut state = MissionLuaState::new(tempdir.path())
                .map_err(|source| startup(LuaSessionError::Lua(source)))?;
            register_natives(&mut state).map_err(|source| {
                startup(LuaSessionError::Lua(MissionLuaError::Runtime(source)))
            })?;
            (tempdir, state)
        };
        let runtime = Arc::new(
            SpellforgeRuntime51::new(package)
                .map_err(|error| startup(LuaSessionError::Contract(error.to_string())))?,
        );
        Ok(Self {
            #[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
            _tempdir: tempdir,
            #[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
            state,
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

        #[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
        let (tempdir, state) = {
            let tempdir = TempDir::with_prefix("robin-lua-mission-")
                .map_err(|e| LuaSessionError::WriteFile(PathBuf::from("<tempdir>"), e))?;
            write_test_package(tempdir.path(), &package)?;
            let mut state = MissionLuaState::new(tempdir.path())?;
            register_natives(&mut state)?;
            (tempdir, state)
        };
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
            #[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
            _tempdir: tempdir,
            #[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
            state,
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

    /// Look up a top-level event function on the Lua globals and
    /// call it with the engine's [`ScriptEffects`] attached. No-op (with
    /// a `debug!`) if the script didn't define it — Spellforge
    /// missions cherry-pick which events they override, and missing
    /// ones are perfectly valid.
    ///
    /// Returns the integer-compatible result of the Lua call. A missing
    /// function or no explicit return is a successful no-op; a Lua failure
    /// or incompatible return is preserved as a typed [`LuaSessionError`].
    ///
    /// TODO(parity): The Spellforge DLL's `luaRun` implementation is not in
    /// the available material; verify its accepted event return conversions if that
    /// source becomes available. Runtime errors must remain errors regardless.
    #[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
    pub fn run_event(
        &self,
        host: &mut ScriptEffects,
        script_state: &mut ScriptState,
        script_domains: &mut robin_engine::engine::ScriptDomains,
        capabilities: &robin_engine::natives::NativeSessionCapabilities<'_>,
        event_name: &str,
        args: &[i32],
    ) -> Result<i32, LuaSessionError> {
        self.run_event_with_bindings(
            host,
            script_state,
            script_domains,
            robin_engine::natives::AttachedScriptBindings::empty_ref(),
            capabilities,
            event_name,
            args,
        )
    }

    #[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
    fn run_event_with_bindings(
        &self,
        host: &mut ScriptEffects,
        script_state: &mut ScriptState,
        script_domains: &mut robin_engine::engine::ScriptDomains,
        bindings: &robin_engine::natives::AttachedScriptBindings,
        capabilities: &robin_engine::natives::NativeSessionCapabilities<'_>,
        event_name: &str,
        args: &[i32],
    ) -> Result<i32, LuaSessionError> {
        let result = self.state.with_host_state_and_bindings(
            host,
            script_state,
            script_domains,
            bindings,
            capabilities,
            |lua| {
                let globals = lua.globals();
                let v: mlua::Value = globals.get(event_name)?;
                let Some(func) = (match &v {
                    mlua::Value::Function(f) => Some(f.clone()),
                    _ => None,
                }) else {
                    tracing::debug!(
                        "LuaSession[{}]: no global function `{event_name}`",
                        self.mission_basename
                    );
                    return Ok(None);
                };
                // Variadic call — `mlua::Variadic` lets us pass a
                // slice without knowing arity statically. Convert i32
                // args once.
                let mut variadic: mlua::Variadic<mlua::Value> = mlua::Variadic::new();
                for a in args {
                    variadic.push(mlua::Value::Integer((*a).into()));
                }
                let ret: mlua::MultiValue = func.call(variadic)?;
                Ok(ret.into_iter().next())
            },
        );

        let returned = result.map_err(|source| LuaSessionError::Event {
            mission: self.mission_basename.clone(),
            event: event_name.to_owned(),
            source,
        })?;
        match returned {
            None | Some(mlua::Value::Nil) => Ok(0),
            Some(mlua::Value::Integer(value)) => {
                i32::try_from(value).map_err(|_| LuaSessionError::EventIntegerOutOfRange {
                    mission: self.mission_basename.clone(),
                    event: event_name.to_owned(),
                    value,
                })
            }
            Some(mlua::Value::Number(value))
                if value.is_finite()
                    && value.fract() == 0.0
                    && value >= i32::MIN as f64
                    && value <= i32::MAX as f64 =>
            {
                Ok(value as i32)
            }
            Some(mlua::Value::Boolean(value)) => Ok(i32::from(value)),
            Some(value) => Err(LuaSessionError::UnexpectedEventReturn {
                mission: self.mission_basename.clone(),
                event: event_name.to_owned(),
                actual: value.type_name().to_owned(),
            }),
        }
    }

    /// Dispatch the required Spellforge startup pair in order. The caller
    /// supplies the engine's live script host with its authoritative
    /// simulation context attached. Failure stops startup immediately and is returned
    /// with both mission and event context.
    #[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
    pub fn run_required_startup_events(
        &self,
        native_parts: Option<(
            &mut ScriptEffects,
            &mut ScriptState,
            &mut robin_engine::engine::ScriptDomains,
            &robin_engine::natives::AttachedScriptBindings,
            &robin_engine::natives::NativeSessionCapabilities<'_>,
        )>,
        initialization_seed: i32,
    ) -> Result<(), SpellforgeSessionError> {
        let Some((host, script_state, script_domains, bindings, capabilities)) = native_parts
        else {
            return Err(SpellforgeSessionError::MissingScriptEffects {
                mission: self.mission_basename.clone(),
                event: "Initialize",
            });
        };
        for (event, args) in [
            ("Initialize", std::slice::from_ref(&initialization_seed)),
            ("PostInitialize", &[][..]),
        ] {
            self.run_event_with_bindings(
                host,
                script_state,
                script_domains,
                bindings,
                capabilities,
                event,
                args,
            )
            .map_err(|source| SpellforgeSessionError::RequiredEvent {
                mission: self.mission_basename.clone(),
                event,
                source,
            })?;
        }
        Ok(())
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

#[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
fn write_test_package(
    directory: &Path,
    package: &SpellforgePackage,
) -> Result<(), LuaSessionError> {
    for (relative, bytes) in &package.files {
        let path = directory.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| LuaSessionError::WriteFile(parent.to_path_buf(), error))?;
        }
        fs::write(&path, bytes).map_err(|error| LuaSessionError::WriteFile(path, error))?;
    }
    Ok(())
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

#[cfg(all(test, feature = "lua", not(target_arch = "wasm32")))]
mod tests;
