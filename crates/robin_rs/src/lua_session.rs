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
//! Engine event dispatch is then driven through [`LuaSession::run_event`]:
//! the session's frame loop calls it at the same points Robin's
//! `.scb` VM gets its own `Initialize` / `Timer` / `CheckVictoryCondition`
//! / `Finalize` invocations.
//!
//! ## What is and is not wired up
//!
//! Wired for parity with Spellforge missions on rhmods.com:
//! - `Initialize(seed)` — fired once after the engine has finished
//!   level load, before the first frame ticks.
//! - `PostInitialize()` — fired immediately after `Initialize`.
//! - `Timer(seconds)` — fired once per game-second.
//! - `CheckVictoryCondition(seconds)` — fired every three game-seconds.
//! - `Finalize(unk)` — fired on mission end.
//!
//! Per-actor / per-target / per-scroll / per-zone / per-waypoint event
//! routing (`ActionChange`, `FilterAiEvent`, `ProcessMessage`, the
//! `Target.ActivatedBy*` family, etc.) is *not* yet wired through this
//! session. Mission scripts whose flow depends on those events will run
//! their global `Initialize` path but miss the per-entity dispatch —
//! follow-up commits add the engine hooks.

use robin_engine::natives::GameHost;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use robin_lua::{MissionLuaError, MissionLuaState, register_natives};
use tempfile::TempDir;

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
}

impl LuaSession {
    /// Build a Lua session for the chosen mission, or return `None`
    /// (with a log line) if the mission is Vanilla / has no `.lua`
    /// companion / extraction fails. Vanilla missions get no Lua —
    /// the engine's `.scb` path handles them as before.
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
        event_name: &str,
        args: &[i32],
    ) -> Result<i32, LuaSessionError> {
        let result = self.state.with_host(host, |lua| {
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
                variadic.push(mlua::Value::Integer(*a));
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
            Some(mlua::Value::Integer(value)) => Ok(value),
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

    #[test]
    fn event_returns_are_checked_table_driven() {
        let session = session_with_script(
            r#"
            function NoReturn() end
            function IntegerReturn() return 17 end
            function IntegralNumberReturn() return 18 / 1 end
            function BooleanReturn() return true end
            function BadReturn() return {} end
            "#,
        );
        let mut host = GameHost::new();
        let valid_cases = [
            ("Missing", 0),
            ("NoReturn", 0),
            ("IntegerReturn", 17),
            ("IntegralNumberReturn", 18),
            ("BooleanReturn", 1),
        ];
        for (event, expected) in valid_cases {
            assert_eq!(session.run_event(&mut host, event, &[]).unwrap(), expected);
        }

        assert!(matches!(
            session.run_event(&mut host, "BadReturn", &[]),
            Err(LuaSessionError::UnexpectedEventReturn { actual, .. }) if actual == "table"
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

        let err = session.run_event(&mut host, "Fails", &[]).unwrap_err();
        assert!(matches!(err, LuaSessionError::Event { .. }));
        assert!(err.to_string().contains("deliberate failure"));
    }
}
