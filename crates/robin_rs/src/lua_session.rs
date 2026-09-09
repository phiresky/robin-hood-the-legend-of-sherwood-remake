//! Host-side wiring for `robin_lua` mission scripts.
//!
//! When a Spellforge custom mission is launched, the picker hands us a
//! [`crate::main_menu::custom_missions::CustomMissionLaunch`] with the
//! version zip and the basename of the chosen `.rhm`. We:
//!
//! 1. Extract the matching `.lua` companion file and the shared
//!    `lib/*.lua` helpers from the mounted overlay zips into a
//!    per-launch tempdir.
//! 2. Build the exact versioned package from those bytes.
//! 3. Construct the shared safe-Rust Lua 5.1 runtime used by native and wasm.
//!
//! Live event dispatch is owned by the engine's sole synchronous callback
//! driver. The host session installs a versioned package/runtime attachment;
//! timer, victory, finalize, and every per-entity callback then pass through
//! the same yield/resume path as SCB.

#[cfg(test)]
use robin_engine::natives::{ScriptEffects, ScriptState};
use robin_engine::spellforge::SpellforgePackage;
#[cfg(test)]
use robin_engine::spellforge::{SPELLFORGE_CONTRACT_VERSION, SpellforgeScriptMode};
#[cfg(test)]
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(test)]
use robin_lua::{MissionLuaError, MissionLuaState, register_natives};
use robin_spellforge::{
    ARCHIVE_BYTE_LIMIT, SpellforgePackageError, SpellforgeRuntime51, build_package_from_archives,
};
#[cfg(test)]
use robin_spellforge::{compute_package_sha256, spellforge_vm_abi};
#[cfg(test)]
use tempfile::TempDir;

use crate::main_entry::CliArgs;
use crate::main_menu::custom_missions::CustomMissionLaunch;

/// One mission's worth of Lua state, attached to a launched custom
/// Spellforge mission for as long as the session runs.
pub struct LuaSession {
    /// Tempdir holding the extracted `.lua` files. Lives at least as
    /// long as `state` so `require()` lookups stay valid; dropped on
    /// session teardown.
    #[cfg(test)]
    _tempdir: TempDir,
    /// Legacy direct-call interpreter retained only for focused host/native
    /// adapter tests. Live missions never load their package into this second
    /// VM; all bootstrap and event code runs exactly once in `runtime`.
    #[cfg(test)]
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
    #[cfg(test)]
    #[error("lua: {0}")]
    Lua(#[from] MissionLuaError),
    #[cfg(test)]
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
    #[cfg(test)]
    #[error("Lua event `{event}` failed for mission `{mission}`: {source}")]
    Event {
        mission: String,
        event: String,
        #[source]
        source: mlua::Error,
    },
    #[cfg(test)]
    #[error(
        "Lua event `{event}` for mission `{mission}` returned Lua {actual}; expected an integer, integral number, boolean, or nil"
    )]
    UnexpectedEventReturn {
        mission: String,
        event: String,
        actual: String,
    },
    #[cfg(test)]
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
    #[cfg(test)]
    #[error(
        "required Spellforge event `{event}` for mission `{mission}` has no mission-script ScriptEffects"
    )]
    MissingScriptEffects {
        mission: String,
        event: &'static str,
    },
    #[cfg(test)]
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
        #[cfg(test)]
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
            #[cfg(test)]
            _tempdir: tempdir,
            #[cfg(test)]
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

        #[cfg(test)]
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
            #[cfg(test)]
            _tempdir: tempdir,
            #[cfg(test)]
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
    #[cfg(test)]
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

    #[cfg(test)]
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
    #[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::main_entry::PendingLuaMission;

    fn write_test_zip(path: &Path, entries: &[(&str, &[u8])]) {
        use std::io::Write as _;
        let file = fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }

    fn session_with_script(source: &str) -> LuaSession {
        let tempdir = tempfile::tempdir().expect("tempdir");
        fs::write(tempdir.path().join("test_mission.lua"), source).expect("write script");
        let mut state = MissionLuaState::new(tempdir.path()).expect("new Lua state");
        register_natives(&mut state).expect("register natives");
        state.load_script("test_mission").expect("load script");
        let mut package = SpellforgePackage {
            contract_version: SPELLFORGE_CONTRACT_VERSION,
            vm_abi: spellforge_vm_abi().to_owned(),
            script_mode: SpellforgeScriptMode::Replace,
            entrypoint: "test_mission.lua".to_owned(),
            files: BTreeMap::from([("test_mission.lua".to_owned(), source.as_bytes().to_vec())]),
            sha256: [0; 32],
        };
        package.sha256 = compute_package_sha256(&package);
        LuaSession {
            _tempdir: tempdir,
            state,
            mission_basename: "test_mission".to_owned(),
            runtime: Arc::new(SpellforgeRuntime51::new(package).expect("runtime")),
        }
    }

    fn spellforge_args() -> crate::main_entry::MissionLaunch {
        let mut args = crate::main_entry::MissionLaunch::from(CliArgs {
            rollback_check: false,
            ..CliArgs::default()
        });
        args.pending_lua_mission = Some(PendingLuaMission {
            rhm_basename: "test_mission".to_owned(),
            requires_spellforge: true,
            spellforge_package: None,
        });
        args
    }

    #[test]
    fn deterministic_and_network_modes_allow_spellforge() {
        let mut replay = spellforge_args();
        replay.replay = Some("unused.rhrec.jsonl".to_owned());
        validate_launch_mode(&replay, false).unwrap();

        let mut rollback = spellforge_args();
        rollback.rollback_check = true;
        validate_launch_mode(&rollback, false).unwrap();

        let mut host = spellforge_args();
        host.server = true;
        validate_launch_mode(&host, false).unwrap();

        let mut client = spellforge_args();
        client.connect = Some("an-endpoint-id".to_owned());
        validate_launch_mode(&client, false).unwrap();

        validate_launch_mode(&spellforge_args(), true).unwrap();
    }

    #[test]
    fn nested_library_paths_have_one_native_browser_package_identity() {
        let temporary = tempfile::tempdir().unwrap();
        let archive = temporary.path().join("mission.zip");
        write_test_zip(
            &archive,
            &[
                ("English/Data/Levels/test_mission.rhm", b"rhm"),
                (
                    "English/Data/Levels/test_mission.lua",
                    b"require('lib.helpers.values')",
                ),
                (
                    "English/Data/Levels/lib/helpers/values.lua",
                    b"nested_value=17",
                ),
            ],
        );
        let bytes = fs::read(&archive).unwrap();
        let package = build_package_from_archives(
            &bytes,
            "English/Data/Levels/test_mission.rhm",
            "test_mission",
            None,
        )
        .unwrap();
        assert_eq!(
            package.files.get("lib/helpers/values.lua").unwrap(),
            b"nested_value=17"
        );
        assert_eq!(
            package.sha256,
            robin_spellforge::compute_package_sha256(&package)
        );
    }

    #[test]
    fn unsafe_nested_library_archive_path_is_rejected() {
        let temporary = tempfile::tempdir().unwrap();
        let archive = temporary.path().join("mission.zip");
        write_test_zip(
            &archive,
            &[
                ("test_mission.rhm", b"rhm"),
                ("test_mission.lua", b"return 1"),
                ("lib/../escape.lua", b"return 1"),
            ],
        );
        let bytes = fs::read(&archive).unwrap();
        let error = build_package_from_archives(&bytes, "test_mission.rhm", "test_mission", None)
            .expect_err("parent traversal must be rejected");
        assert_eq!(
            error.kind,
            robin_spellforge::SpellforgePackageErrorKind::UnsafePath
        );
    }

    #[test]
    fn normal_single_player_and_vanilla_launches_remain_allowed() {
        let spellforge = spellforge_args();
        validate_launch_mode(&spellforge, false).unwrap();

        let mut vanilla = spellforge;
        vanilla
            .pending_lua_mission
            .as_mut()
            .unwrap()
            .requires_spellforge = false;
        vanilla.rollback_check = true;
        vanilla.replay = Some("unused.rhrec.jsonl".to_owned());
        validate_launch_mode(&vanilla, false).unwrap();
    }

    #[test]
    fn required_spellforge_construction_error_keeps_mission_context() {
        let launch = CustomMissionLaunch {
            slug: "test-mod".to_owned(),
            mod_title: "Test Mod".to_owned(),
            claimed_author: "Test Author".to_owned(),
            version: "1".to_owned(),
            source_url: "https://example.invalid".to_owned(),
            license: "CC0-1.0".to_owned(),
            version_zip: PathBuf::from("definitely-missing-spellforge.zip"),
            installed_source: None,
            version_zip_bytes: None,
            rhm_zip_entry: "test_mission.rhm".to_owned(),
            rhm_basename: "test_mission".to_owned(),
            map_filename: String::new(),
            requires_spellforge: true,
        };
        assert!(matches!(
            LuaSession::start_for_launch(&launch, Path::new("unused-mods")),
            Err(SpellforgeSessionError::Startup { mission, source: LuaSessionError::OpenZip(_, _) })
                if mission == "test_mission"
        ));
    }

    #[test]
    fn event_returns_are_checked_table_driven() {
        let session = session_with_script(
            r#"
            function NoReturn() end
            function IntegerReturn() return 17 end
            function IntegralNumberReturn() return 18 / 1 end
            function BooleanReturn() return true end
            function WideIntegerReturn() return 2147483648 end
            function BadReturn() return {} end
            "#,
        );
        let mut host = ScriptEffects::new();
        let mut entities = robin_engine::entities::Entities::new();
        let mut ai_global = robin_engine::ai::AiGlobalState::default();
        let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
        let simulation = robin_engine::sim_rng::SimulationContext::with_seed(1);
        let capabilities = robin_engine::natives::NativeSessionCapabilities::new(
            &simulation,
            &mut entities,
            &mut ai_global,
            &mut fast_grid,
        );
        let mut script_state = ScriptState::default();
        let mut script_domains = robin_engine::engine::ScriptDomains::default();
        let valid_cases = [
            ("Missing", 0),
            ("NoReturn", 0),
            ("IntegerReturn", 17),
            ("IntegralNumberReturn", 18),
            ("BooleanReturn", 1),
        ];
        for (event, expected) in valid_cases {
            assert_eq!(
                session
                    .run_event(
                        &mut host,
                        &mut script_state,
                        &mut script_domains,
                        &capabilities,
                        event,
                        &[],
                    )
                    .unwrap(),
                expected
            );
        }

        assert!(matches!(
            session.run_event(
                &mut host,
                        &mut script_state,
                        &mut script_domains,
                        &capabilities,
                "BadReturn",
                &[],
            ),
            Err(LuaSessionError::UnexpectedEventReturn { actual, .. }) if actual == "table"
        ));
        #[cfg(target_pointer_width = "64")]
        assert!(matches!(
            session.run_event(
                &mut host,
                &mut script_state,
                &mut script_domains,
                &capabilities,
                "WideIntegerReturn",
                &[],
            ),
            Err(LuaSessionError::EventIntegerOutOfRange {
                value: 2_147_483_648,
                ..
            })
        ));
    }

    #[test]
    fn event_lua_errors_are_not_replaced_with_zero() {
        let session = session_with_script(
            r#"
            function Fails()
                error("deliberate failure")
            end
            "#,
        );
        let mut host = ScriptEffects::new();
        let mut entities = robin_engine::entities::Entities::new();
        let mut ai_global = robin_engine::ai::AiGlobalState::default();
        let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
        let simulation = robin_engine::sim_rng::SimulationContext::with_seed(1);
        let capabilities = robin_engine::natives::NativeSessionCapabilities::new(
            &simulation,
            &mut entities,
            &mut ai_global,
            &mut fast_grid,
        );
        let mut script_state = ScriptState::default();
        let mut script_domains = robin_engine::engine::ScriptDomains::default();

        let err = session
            .run_event(
                &mut host,
                &mut script_state,
                &mut script_domains,
                &capabilities,
                "Fails",
                &[],
            )
            .unwrap_err();
        assert!(matches!(err, LuaSessionError::Event { .. }));
        assert!(err.to_string().contains("deliberate failure"));
    }

    #[test]
    fn required_startup_event_error_aborts_the_startup_pair() {
        let session = session_with_script(
            r#"
            post_initialized = false
            function Initialize()
                error("deliberate startup failure")
            end
            function PostInitialize()
                post_initialized = true
            end
            "#,
        );
        let mut host = ScriptEffects::new();
        let mut entities = robin_engine::entities::Entities::new();
        let mut ai_global = robin_engine::ai::AiGlobalState::default();
        let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
        let mut script_state = ScriptState::default();
        let mut script_domains = robin_engine::engine::ScriptDomains::default();
        let bindings = robin_engine::natives::AttachedScriptBindings::default();

        let err = robin_engine::sim_rng::with_seed(7, |sim| {
            let capabilities = robin_engine::natives::NativeSessionCapabilities::new(
                sim,
                &mut entities,
                &mut ai_global,
                &mut fast_grid,
            );
            session
                .run_required_startup_events(
                    Some((
                        &mut host,
                        &mut script_state,
                        &mut script_domains,
                        &bindings,
                        &capabilities,
                    )),
                    123,
                )
                .unwrap_err()
        });
        assert!(matches!(
            err,
            SpellforgeSessionError::RequiredEvent {
                event: "Initialize",
                source: LuaSessionError::Event { .. },
                ..
            }
        ));
        let post_initialized: bool = session
            .state
            .lua()
            .globals()
            .get("post_initialized")
            .unwrap();
        assert!(
            !post_initialized,
            "PostInitialize must not run after Initialize fails"
        );
    }

    #[test]
    fn required_startup_rejects_a_missing_script_effects() {
        let session = session_with_script("function Initialize() end");
        assert!(matches!(
            session.run_required_startup_events(None, 0),
            Err(SpellforgeSessionError::MissingScriptEffects {
                event: "Initialize",
                ..
            })
        ));
    }

    #[test]
    fn engine_lua_startup_mutates_the_canonical_campaign_owner() {
        use robin_engine::campaign::{Campaign, CampaignValue};
        use robin_engine::engine::{Engine, LevelAssets};
        use robin_engine::profiles::MissionProfile;
        use robin_engine::scb::{ClassEntry, SCB_VERSION, ScbFile};
        use robin_engine::script_manager::ScriptProgram;

        let session = session_with_script(
            r#"
            function Initialize()
                SetCustomCampaignValue(7, 4242)
            end
            "#,
        );

        let startup = ClassEntry {
            source_file: "lua_campaign_owner_test.scs".into(),
            class_name: "StartUp".into(),
            size_of_member_variables: 0,
            member_variables: Vec::new(),
            functions: Vec::new(),
            quads: Vec::new(),
        };
        let program = ScriptProgram::from_scb(ScbFile {
            version: SCB_VERSION,
            classes: vec![startup],
        })
        .expect("prepare empty test bytecode");

        let mut assets = LevelAssets::new();
        std::sync::Arc::make_mut(&mut assets.profile_manager)
            .missions
            .push(MissionProfile {
                mission_filename: "lua_campaign_owner_test".into(),
                ..MissionProfile::default()
            });
        assets.scripts.mission_programs =
            std::sync::Arc::new(std::collections::BTreeMap::from([(
                "lua_campaign_owner_test".to_owned(),
                std::sync::Arc::new(program),
            )]));

        let mut engine = Engine::new_for_test_with_simulation(
            800.0,
            600.0,
            Campaign::default(),
            &mut assets,
            0,
            robin_engine::engine::SimConfig::default(),
        )
        .expect("construct engine with the minimal mission script");
        engine.test_with_mission_script_effects_and_rng(&assets, |_simulation, native_parts| {
            session
                .run_required_startup_events(native_parts, 0)
                .expect("Lua startup campaign native succeeds")
        });

        let slot = CampaignValue::custom(7).expect("custom campaign slot 7");
        assert_eq!(
            engine.campaign().values[slot],
            4242,
            "Lua must mutate Engine's canonical campaign through the opaque query capability",
        );
    }

    #[test]
    fn engine_lua_startup_borrows_the_scoped_canonical_ai_global() {
        use robin_engine::campaign::Campaign;
        use robin_engine::engine::{Engine, LevelAssets};
        use robin_engine::profiles::MissionProfile;
        use robin_engine::scb::{ClassEntry, SCB_VERSION, ScbFile};
        use robin_engine::script_manager::ScriptProgram;

        let session = session_with_script(
            r#"
            function Initialize()
                local id = AddRepulsivePoint(GetLocationScript(0), 10.0, 20.0, 0)
                DeleteRepulsivePoint(id)
            end
            "#,
        );

        let startup = ClassEntry {
            source_file: "lua_ai_owner_test.scs".into(),
            class_name: "StartUp".into(),
            size_of_member_variables: 0,
            member_variables: Vec::new(),
            functions: Vec::new(),
            quads: Vec::new(),
        };
        let program = ScriptProgram::from_scb(ScbFile {
            version: SCB_VERSION,
            classes: vec![startup],
        })
        .expect("prepare empty test bytecode");

        let mut assets = LevelAssets::new();
        std::sync::Arc::make_mut(&mut assets.profile_manager)
            .missions
            .push(MissionProfile {
                mission_filename: "lua_ai_owner_test".into(),
                ..MissionProfile::default()
            });
        assets.scripts.mission_programs =
            std::sync::Arc::new(std::collections::BTreeMap::from([(
                "lua_ai_owner_test".to_owned(),
                std::sync::Arc::new(program),
            )]));
        assets.scripts.location_count = 1;
        assets.scripts.point_count = 1;
        assets.scripts.location_positions = std::sync::Arc::new(vec![(12.0, 34.0)]);
        assets.scripts.location_layers = std::sync::Arc::new(vec![2]);
        assets.scripts.location_sectors = std::sync::Arc::new(vec![44]);

        let mut engine = Engine::new_for_test_with_simulation(
            800.0,
            600.0,
            Campaign::default(),
            &mut assets,
            0,
            robin_engine::engine::SimConfig::default(),
        )
        .expect("construct engine with the minimal mission script");
        engine.test_with_mission_script_effects_and_rng(&assets, |_simulation, native_parts| {
            session
                .run_required_startup_events(native_parts, 0)
                .expect("Lua startup AI natives succeed")
        });

        assert_eq!(engine.ai_global().next_repulsive_point_id, 2);
        assert!(engine.ai_global().repulsive_points.is_empty());
    }

    #[test]
    fn startup_random_draw_uses_the_attached_authoritative_context() {
        let session = session_with_script(
            r#"
            function Initialize()
                startup_roll = math.random(1, 1000000)
            end
            "#,
        );
        let mut host = ScriptEffects::new();
        let mut entities = robin_engine::entities::Entities::new();
        let mut ai_global = robin_engine::ai::AiGlobalState::default();
        let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
        let mut script_state = ScriptState::default();
        let mut script_domains = robin_engine::engine::ScriptDomains::default();
        let bindings = robin_engine::natives::AttachedScriptBindings::default();
        robin_engine::sim_rng::with_seed(0x5eed, |sim| {
            let capabilities = robin_engine::natives::NativeSessionCapabilities::new(
                sim,
                &mut entities,
                &mut ai_global,
                &mut fast_grid,
            );
            session
                .run_required_startup_events(
                    Some((
                        &mut host,
                        &mut script_state,
                        &mut script_domains,
                        &bindings,
                        &capabilities,
                    )),
                    0,
                )
                .unwrap();
        });
        let startup_roll: i64 = session.state.lua().globals().get("startup_roll").unwrap();
        assert!((1..=1_000_000).contains(&startup_roll));
    }
}
