//! Host-side wiring for `robin_lua` mission scripts.
//!
//! When a Spellforge custom mission is launched, the picker hands us a
//! [`crate::main_menu::custom_missions::CustomMissionLaunch`] with the
//! version zip and the basename of the chosen `.rhm`. We:
//!
//! 1. Extract the matching `.lua` companion file and the shared
//!    `lib/*.lua` helpers from the mounted overlay zips into a
//!    per-launch tempdir.
//! 2. Build a [`MissionLuaState`] anchored at that tempdir so
//!    `package.path` resolves `require("lib.common")` correctly.
//! 3. Register every native binding so the script's top-level can
//!    already call into the engine.
//! 4. Execute the script body (top-level statements run once).
//!
//! Engine event dispatch is then driven through [`LuaSession::run_event`].
//! At this revision only the two startup events are connected; the remaining
//! Spellforge event surface still has no host call path.
//!
//! ## What is and is not wired up
//!
//! Wired for partial compatibility with Spellforge missions on rhmods.com:
//! - `Initialize(seed)` — fired once after the engine has finished
//!   level load, before the first frame ticks.
//! - `PostInitialize()` — fired immediately after `Initialize`.
//!
//! `Timer`, victory checks, finalization, and per-actor / per-target /
//! per-scroll / per-zone / per-waypoint routing are *not* yet wired through
//! this session. Mission scripts whose flow depends on those events will run
//! their global startup path but miss the later dispatch.

use robin_engine::natives::{GameHost, ScriptState};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use robin_lua::{MissionLuaError, MissionLuaState, register_natives};
use tempfile::TempDir;

use crate::main_entry::CliArgs;
use crate::main_menu::custom_missions::CustomMissionLaunch;

/// One mission's worth of Lua state, attached to a launched custom
/// Spellforge mission for as long as the session runs.
pub struct LuaSession {
    /// Tempdir holding the extracted `.lua` files. Lives at least as
    /// long as `state` so `require()` lookups stay valid; dropped on
    /// session teardown.
    _tempdir: TempDir,
    /// The Lua interpreter + registered natives.
    state: MissionLuaState,
    /// Bare basename of the mission script — the `.lua` filename
    /// without the extension. Used as the `chunkname` in stack
    /// traces and as the lookup key for the top-level event
    /// functions (which the script registered as globals when its
    /// body ran).
    mission_basename: String,
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
    #[error("lua: {0}")]
    Lua(#[from] MissionLuaError),
    #[error("mlua: {0}")]
    Mlua(#[from] mlua::Error),
    #[error("Lua event `{event}` failed for mission `{mission}`: {source}")]
    Event {
        mission: String,
        event: String,
        #[source]
        source: mlua::Error,
    },
    #[error(
        "Lua event `{event}` for mission `{mission}` returned Lua {actual}; expected an integer, integral number, boolean, or nil"
    )]
    UnexpectedEventReturn {
        mission: String,
        event: String,
        actual: String,
    },
    #[error(
        "Lua event `{event}` for mission `{mission}` returned integer {value}, which is outside the signed 32-bit game ABI range"
    )]
    EventIntegerOutOfRange {
        mission: String,
        event: String,
        value: i64,
    },
}

/// Authoritative modes whose snapshots omit the host-owned Lua interpreter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedSpellforgeMode {
    ReplayPlayback,
    RollbackVerification,
    MultiplayerHost,
    MultiplayerClient,
}

impl std::fmt::Display for UnsupportedSpellforgeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ReplayPlayback => "replay playback",
            Self::RollbackVerification => "rollback/determinism verification",
            Self::MultiplayerHost => "multiplayer host networking",
            Self::MultiplayerClient => "multiplayer client networking",
        })
    }
}

/// Contextual failures at the boundary between mission setup and Spellforge.
#[derive(Debug, thiserror::Error)]
pub enum SpellforgeSessionError {
    #[error(
        "Spellforge mission `{mission}` cannot start in {mode}: Lua state is host-owned and absent from authoritative snapshots"
    )]
    UnsupportedMode {
        mission: String,
        mode: UnsupportedSpellforgeMode,
    },
    #[error("required Spellforge startup failed for mission `{mission}`: {source}")]
    Startup {
        mission: String,
        #[source]
        source: LuaSessionError,
    },
    #[error("required Spellforge session was not created for mission `{mission}`")]
    RequiredSessionMissing { mission: String },
    #[error(
        "required Spellforge event `{event}` for mission `{mission}` has no mission-script GameHost"
    )]
    MissingGameHost {
        mission: String,
        event: &'static str,
    },
    #[error("required Spellforge event `{event}` failed for mission `{mission}`: {source}")]
    RequiredEvent {
        mission: String,
        event: &'static str,
        #[source]
        source: LuaSessionError,
    },
}

/// Reject a pending Spellforge launch before any deterministic or networked
/// authoritative simulation is constructed.
///
/// `pending_replay` covers the script-RPC slot consumed later by replay setup;
/// normal CLI and wasm replay launches are represented directly on `args`.
pub fn validate_launch_mode(
    args: &CliArgs,
    pending_replay: bool,
) -> Result<(), SpellforgeSessionError> {
    let Some(pending) = args
        .pending_lua_mission
        .as_ref()
        .filter(|pending| pending.requires_spellforge)
    else {
        return Ok(());
    };

    let mode = if pending_replay || args.replay.is_some() || args.replay_data.is_some() {
        Some(UnsupportedSpellforgeMode::ReplayPlayback)
    } else if args.server.is_some() {
        Some(UnsupportedSpellforgeMode::MultiplayerHost)
    } else if args.connect.is_some() {
        Some(UnsupportedSpellforgeMode::MultiplayerClient)
    } else if args.rollback_check {
        Some(UnsupportedSpellforgeMode::RollbackVerification)
    } else {
        None
    };

    match mode {
        Some(mode) => Err(SpellforgeSessionError::UnsupportedMode {
            mission: pending.rhm_basename.clone(),
            mode,
        }),
        None => Ok(()),
    }
}

impl LuaSession {
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
        let tempdir = TempDir::with_prefix("robin-lua-mission-")
            .map_err(|e| LuaSessionError::WriteFile(PathBuf::from("<tempdir>"), e))?;
        let mission_basename = launch.rhm_basename.clone();

        // The `.lua` companion sits at the same zip path as the
        // `.rhm` but with the `.lua` extension. Some mod zips also
        // bundle a local `lib/` next to the script; others rely on
        // the shared `lib_*.zip` mounted underneath the version
        // zip. Try the mission zip first, then fall back to the
        // shared one.
        let script_extracted =
            extract_companion_script(&launch.version_zip, &launch.rhm_basename, tempdir.path())?;
        let lib_extracted_from_mission =
            extract_lib_dir_if_present(&launch.version_zip, tempdir.path())?;
        // If the mod didn't bundle its own lib, pull it from the
        // shared `mods_root/lib/lib_*.zip` that `mount_for_launch`
        // also mounted as an SbFile overlay.
        if !lib_extracted_from_mission && let Some(shared_lib_zip) = find_shared_lib_zip(mods_root)
        {
            extract_lib_dir_if_present(&shared_lib_zip, tempdir.path())?;
        }
        tracing::info!(
            "LuaSession: extracted {} (script: {}) to {}",
            launch.slug,
            script_extracted.display(),
            tempdir.path().display()
        );

        let mut state = MissionLuaState::new(tempdir.path())?;
        register_natives(&mut state)?;
        // Loading the script runs its top-level statements, which
        // define `Initialize`, `Timer`, `Actor = {...}`, etc. on
        // globals. No host is attached here — Spellforge scripts
        // don't call natives from their module-level body (they
        // only *define* event functions there), so app-data access
        // isn't required. If a script ever does, registration
        // surfaces a clear "no GameHost attached" runtime error.
        state.load_script(&mission_basename)?;

        Ok(Some(Self {
            _tempdir: tempdir,
            state,
            mission_basename,
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

    /// Look up a top-level event function on the Lua globals and
    /// call it with the engine's [`GameHost`] attached. No-op (with
    /// a `debug!`) if the script didn't define it — Spellforge
    /// missions cherry-pick which events they override, and missing
    /// ones are perfectly valid.
    ///
    /// Returns the integer-compatible result of the Lua call. A missing
    /// function or no explicit return is a successful no-op; a Lua failure
    /// or incompatible return is preserved as a typed [`LuaSessionError`].
    ///
    /// TODO(parity): The Spellforge DLL's `luaRun` implementation is not in
    /// `original-code`; verify its accepted event return conversions if that
    /// source becomes available. Runtime errors must remain errors regardless.
    pub fn run_event(
        &self,
        host: &mut GameHost,
        script_state: &mut ScriptState,
        event_name: &str,
        args: &[i32],
    ) -> Result<i32, LuaSessionError> {
        self.run_event_with_bindings(
            host,
            script_state,
            robin_engine::natives::AttachedScriptBindings::empty_ref(),
            robin_engine::natives::NativeQueryViews::default(),
            event_name,
            args,
        )
    }

    fn run_event_with_bindings(
        &self,
        host: &mut GameHost,
        script_state: &mut ScriptState,
        bindings: &robin_engine::natives::AttachedScriptBindings,
        queries: robin_engine::natives::NativeQueryViews<'_>,
        event_name: &str,
        args: &[i32],
    ) -> Result<i32, LuaSessionError> {
        let result =
            self.state
                .with_host_state_and_bindings(host, script_state, bindings, queries, |lua| {
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
                });

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
                    value: value.into(),
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
    /// supplies the engine's live script host while its authoritative RNG
    /// scope is installed. Failure stops startup immediately and is returned
    /// with both mission and event context.
    pub fn run_required_startup_events(
        &self,
        native_parts: Option<(
            &mut GameHost,
            &mut ScriptState,
            &robin_engine::natives::AttachedScriptBindings,
            robin_engine::natives::NativeQueryViews<'_>,
        )>,
        initialization_seed: i32,
    ) -> Result<(), SpellforgeSessionError> {
        let Some((host, script_state, bindings, queries)) = native_parts else {
            return Err(SpellforgeSessionError::MissingGameHost {
                mission: self.mission_basename.clone(),
                event: "Initialize",
            });
        };
        for (event, args) in [
            ("Initialize", std::slice::from_ref(&initialization_seed)),
            ("PostInitialize", &[][..]),
        ] {
            self.run_event_with_bindings(host, script_state, bindings, queries, event, args)
                .map_err(|source| SpellforgeSessionError::RequiredEvent {
                    mission: self.mission_basename.clone(),
                    event,
                    source,
                })?;
        }
        Ok(())
    }
}

/// Extract `<basename>.lua` from `zip_path` into `out_dir`,
/// regardless of how deeply it's nested inside the zip. Mod zips
/// put the script next to the `.rhm` at varying depths
/// (`H01_Lin_VL.lua`, `English/DATA/Levels/H06_Lin_VL.lua`, …); we
/// just walk every entry and grab the one matching the basename.
fn extract_companion_script(
    zip_path: &Path,
    basename: &str,
    out_dir: &Path,
) -> Result<PathBuf, LuaSessionError> {
    let file = fs::File::open(zip_path)
        .map_err(|e| LuaSessionError::OpenZip(zip_path.to_path_buf(), e))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| LuaSessionError::ZipReader(zip_path.to_path_buf(), e))?;
    let expected = format!("{basename}.lua").to_ascii_lowercase();
    let mut found_entry: Option<String> = None;
    for i in 0..archive.len() {
        let entry = archive
            .by_index_raw(i)
            .map_err(|e| LuaSessionError::ZipReader(zip_path.to_path_buf(), e))?;
        let name = entry.name().replace('\\', "/");
        let leaf = name.rsplit_once('/').map(|(_, l)| l).unwrap_or(&name);
        if leaf.to_ascii_lowercase() == expected {
            found_entry = Some(name);
            break;
        }
    }
    let Some(entry_name) = found_entry else {
        return Err(LuaSessionError::NoLuaCompanion {
            zip: zip_path.to_path_buf(),
            rhm_entry: format!("{basename}.lua"),
        });
    };
    let mut entry = archive
        .by_name(&entry_name)
        .map_err(|e| LuaSessionError::ZipReader(zip_path.to_path_buf(), e))?;
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut bytes)
        .map_err(|e| LuaSessionError::WriteFile(out_dir.join(&entry_name), e))?;
    drop(entry);
    let out_path = out_dir.join(format!("{basename}.lua"));
    fs::write(&out_path, &bytes).map_err(|e| LuaSessionError::WriteFile(out_path.clone(), e))?;
    Ok(out_path)
}

/// Walk `zip_path` for any entry whose path contains `/lib/` and
/// ends in `.lua`, copying it into `out_dir/lib/<leaf>.lua`. Returns
/// `true` if at least one lib file was extracted (so the shared lib
/// fallback knows whether to also extract).
fn extract_lib_dir_if_present(zip_path: &Path, out_dir: &Path) -> Result<bool, LuaSessionError> {
    let file = fs::File::open(zip_path)
        .map_err(|e| LuaSessionError::OpenZip(zip_path.to_path_buf(), e))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| LuaSessionError::ZipReader(zip_path.to_path_buf(), e))?;
    let lib_dir = out_dir.join("lib");
    // Don't create the lib dir until we know something goes in it
    // — that way an "empty lib" return value is unambiguous.
    let mut any = false;
    let mut to_extract: Vec<(String, String)> = Vec::new();
    for i in 0..archive.len() {
        let entry = archive
            .by_index_raw(i)
            .map_err(|e| LuaSessionError::ZipReader(zip_path.to_path_buf(), e))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().replace('\\', "/");
        let lower = name.to_ascii_lowercase();
        // Match `<...>/lib/<leaf>.lua` (any leading path), but
        // skip the api-only file — it's all stubs and the engine
        // doesn't need it at runtime.
        if !lower.ends_with(".lua") {
            continue;
        }
        let Some(rest) = lower.rsplit_once("/lib/").map(|(_, r)| r) else {
            // Some zips put lib at the root: `lib/api.lua` with no
            // leading slash split. Match that too.
            if let Some(leaf) = lower.strip_prefix("lib/")
                && !leaf.contains('/')
            {
                to_extract.push((name.clone(), leaf.to_string()));
            }
            continue;
        };
        if rest.contains('/') {
            // Nested under lib/ — Spellforge's lib has no
            // subdirs, so skip anything else.
            continue;
        }
        to_extract.push((name, rest.to_string()));
    }
    if !to_extract.is_empty() {
        fs::create_dir_all(&lib_dir).map_err(|e| LuaSessionError::WriteFile(lib_dir.clone(), e))?;
        for (entry_name, leaf) in to_extract {
            let mut entry = archive
                .by_name(&entry_name)
                .map_err(|e| LuaSessionError::ZipReader(zip_path.to_path_buf(), e))?;
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            entry
                .read_to_end(&mut bytes)
                .map_err(|e| LuaSessionError::WriteFile(lib_dir.join(&leaf), e))?;
            let out_path = lib_dir.join(&leaf);
            fs::write(&out_path, &bytes).map_err(|e| LuaSessionError::WriteFile(out_path, e))?;
            any = true;
        }
    }
    Ok(any)
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

    fn session_with_script(source: &str) -> LuaSession {
        let tempdir = tempfile::tempdir().expect("tempdir");
        fs::write(tempdir.path().join("test_mission.lua"), source).expect("write script");
        let mut state = MissionLuaState::new(tempdir.path()).expect("new Lua state");
        register_natives(&mut state).expect("register natives");
        state.load_script("test_mission").expect("load script");
        LuaSession {
            _tempdir: tempdir,
            state,
            mission_basename: "test_mission".to_owned(),
        }
    }

    fn spellforge_args() -> CliArgs {
        let mut args = CliArgs {
            rollback_check: false,
            ..CliArgs::default()
        };
        args.pending_lua_mission = Some(PendingLuaMission {
            slug: "test-mod".to_owned(),
            rhm_basename: "test_mission".to_owned(),
            version_zip: PathBuf::from("unused.zip"),
            mods_root: PathBuf::from("unused-mods"),
            requires_spellforge: true,
        });
        args
    }

    fn assert_rejected_mode(args: &CliArgs, expected: UnsupportedSpellforgeMode) {
        assert!(matches!(
            validate_launch_mode(args, false),
            Err(SpellforgeSessionError::UnsupportedMode { mission, mode })
                if mission == "test_mission" && mode == expected
        ));
    }

    #[test]
    fn deterministic_modes_reject_spellforge_before_startup() {
        let mut replay = spellforge_args();
        replay.replay = Some("unused.rhrec.jsonl".to_owned());
        assert_rejected_mode(&replay, UnsupportedSpellforgeMode::ReplayPlayback);

        let mut rollback = spellforge_args();
        rollback.rollback_check = true;
        assert_rejected_mode(&rollback, UnsupportedSpellforgeMode::RollbackVerification);

        let mut host = spellforge_args();
        host.server = Some(":7878".to_owned());
        assert_rejected_mode(&host, UnsupportedSpellforgeMode::MultiplayerHost);

        let mut client = spellforge_args();
        client.connect = Some("localhost:7878".to_owned());
        assert_rejected_mode(&client, UnsupportedSpellforgeMode::MultiplayerClient);

        assert!(matches!(
            validate_launch_mode(&spellforge_args(), true),
            Err(SpellforgeSessionError::UnsupportedMode {
                mode: UnsupportedSpellforgeMode::ReplayPlayback,
                ..
            })
        ));
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
            version_zip: PathBuf::from("definitely-missing-spellforge.zip"),
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
        let mut host = GameHost::new();
        let mut script_state = ScriptState::default();
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
                    .run_event(&mut host, &mut script_state, event, &[])
                    .unwrap(),
                expected
            );
        }

        assert!(matches!(
            session.run_event(&mut host, &mut script_state, "BadReturn", &[]),
            Err(LuaSessionError::UnexpectedEventReturn { actual, .. }) if actual == "table"
        ));
        #[cfg(target_pointer_width = "64")]
        assert!(matches!(
            session.run_event(&mut host, &mut script_state, "WideIntegerReturn", &[]),
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
        let mut host = GameHost::new();
        let mut script_state = ScriptState::default();

        let err = session
            .run_event(&mut host, &mut script_state, "Fails", &[])
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
        let mut host = GameHost::new();
        let mut script_state = ScriptState::default();
        let bindings = robin_engine::natives::AttachedScriptBindings::default();

        let err = robin_engine::sim_rng::with_seed(7, || {
            session
                .run_required_startup_events(
                    Some((
                        &mut host,
                        &mut script_state,
                        &bindings,
                        robin_engine::natives::NativeQueryViews::default(),
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
    fn required_startup_rejects_a_missing_game_host() {
        let session = session_with_script("function Initialize() end");
        assert!(matches!(
            session.run_required_startup_events(None, 0),
            Err(SpellforgeSessionError::MissingGameHost {
                event: "Initialize",
                ..
            })
        ));
    }

    #[test]
    fn startup_random_draw_uses_the_installed_authoritative_scope() {
        let session = session_with_script(
            r#"
            function Initialize()
                startup_roll = math.random(1, 1000000)
            end
            "#,
        );
        let mut host = GameHost::new();
        let mut script_state = ScriptState::default();
        let bindings = robin_engine::natives::AttachedScriptBindings::default();
        robin_engine::sim_rng::with_seed(0x5eed, || {
            session
                .run_required_startup_events(
                    Some((
                        &mut host,
                        &mut script_state,
                        &bindings,
                        robin_engine::natives::NativeQueryViews::default(),
                    )),
                    0,
                )
                .unwrap();
        });
        let startup_roll: i64 = session.state.lua().globals().get("startup_roll").unwrap();
        assert!((1..=1_000_000).contains(&startup_roll));
    }
}
