//! `MissionLuaState` — the per-mission Lua interpreter wrapper.
//!
//! One instance is created when a mission with a `.lua` file is
//! loaded. It owns:
//!
//! - the `mlua::Lua` state itself (Luau dialect — see crate Cargo
//!   feature note),
//! - registered native bindings (callable from Lua as
//!   `GetActor("Robin")`, `StartSequence()`, etc.),
//! - a custom `require` function rooted at the mission directory
//!   so `require("lib.common")` resolves to the Spellforge `lib/`
//!   folder shipped with the mission.
//!
//! It does not own engine state — engine pointers are passed in per
//! call via [`MissionLuaState::with_host`]. The Lua state lives on
//! the host side (not in `Engine`) because `mlua::Lua` is not
//! serializable or rollback-friendly. See `docs/PARITY_AUDIT.md` for the
//! deterministic-mode restriction and required snapshot work.
//!
//! ## Why Luau, not Lua 5.4
//!
//! Luau's [`Lua::sandbox`] freezes the globals table and reroutes
//! script-local writes through a per-script environment, which is
//! the right safety primitive for running arbitrary downloaded
//! missions. Luau also ships without `io`, `os.execute`, or
//! `package.loadlib` — i.e. the destructive corners we'd otherwise
//! have to strip by hand. The trade-off is that Luau has no
//! built-in `require` (the `package` library isn't loaded); we
//! supply our own (see [`install_require`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mlua::Lua;
use robin_engine::natives::GameHost;

use crate::natives::HostPtr;

/// Registry key for the `SequenceCallbacks` table. Hidden from
/// the script's view of `_G` so the sandbox doesn't freeze it.
pub(crate) const SEQUENCE_CALLBACKS_KEY: &str = "robin_lua.sequence_callbacks";

/// Errors produced while loading or driving a mission `.lua`.
#[derive(Debug, thiserror::Error)]
pub enum MissionLuaError {
    #[error("reading {0}: {1}")]
    Io(PathBuf, #[source] std::io::Error),
    #[error("lua error in {0}: {1}")]
    Lua(PathBuf, #[source] mlua::Error),
    #[error("lua error: {0}")]
    Runtime(#[from] mlua::Error),
}

/// One mission's Lua state.
///
/// Created lazily when a mission's `.lua` companion file is present
/// next to its `.rhm`. Dropped when the mission unloads.
pub struct MissionLuaState {
    lua: Lua,
    /// Directory containing the mission's `.lua` file. Used as the
    /// root for the custom `require` resolver.
    mission_dir: PathBuf,
    /// Whether `register_natives` has been called. Guards against
    /// double-registration if a host accidentally rebinds twice.
    natives_registered: bool,
}

impl MissionLuaState {
    /// Create a fresh, empty Lua state for the mission at
    /// `mission_dir`. Natives are not registered yet — the caller
    /// must call [`crate::register_natives`] before loading scripts
    /// (so script top-level code can already reference natives).
    ///
    /// Standard libraries loaded: Luau's default safe set (`string`,
    /// `table`, `math`, `bit32`, `coroutine`, `utf8`, plus the
    /// `buffer` and `vector` Luau-specifics). `io` and `package`
    /// are absent by design — we supply our own scoped `require`.
    pub fn new(mission_dir: impl Into<PathBuf>) -> Result<Self, MissionLuaError> {
        let lua = Lua::new();
        let mission_dir = mission_dir.into();

        // Custom require rooted at the mission dir. Installed
        // before sandbox so it's part of the frozen baseline.
        install_require(&lua, &mission_dir)?;
        enforce_determinism(&lua)?;
        install_log(&lua)?;

        // `SequenceCallbacks` is the closure stash for
        // `SequenceCall(fn)` — see `SequenceCall` semantics in
        // the Spellforge sequence-callback contract. We keep it in the
        // registry rather than
        // `_G` so the sandbox's "globals are frozen" rule doesn't
        // block `SequenceCall` from inserting new ids. Scripts
        // never reach into the table directly (it's a private
        // implementation detail of `SequenceCall`), so hiding it
        // from globals is observation-preserving.
        let sequence_callbacks = lua.create_table()?;
        sequence_callbacks.set("__next_id", 10_000_i32)?;
        lua.set_named_registry_value(SEQUENCE_CALLBACKS_KEY, sequence_callbacks)?;

        // Freeze the global environment. After this call, scripts
        // still read globals normally; writes to `_G` go into a
        // per-script environment table that we can throw away
        // between missions without leaking state.
        //
        // Doing this *after* native registration would freeze them
        // unwritable — fine, since we never want scripts to
        // overwrite engine bindings. Native registration runs in
        // `register_natives` which is called after `new`, so we
        // sandbox there instead. See `register_natives`.

        Ok(Self {
            lua,
            mission_dir,
            natives_registered: false,
        })
    }

    /// Borrow the underlying `mlua::Lua`. Used by the natives module
    /// to register functions onto `globals()`.
    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    /// Whether [`crate::register_natives`] has run against this state.
    pub fn natives_registered(&self) -> bool {
        self.natives_registered
    }

    /// Mark natives as registered. Called by [`crate::register_natives`].
    pub(crate) fn mark_natives_registered(&mut self) {
        self.natives_registered = true;
    }

    /// Run `f` with the engine's [`GameHost`] attached as Lua app
    /// data. All registered natives can reach into the host while
    /// `f` is on the stack; once `f` returns, the pointer is
    /// removed so a stray Lua coroutine resumed later can't see
    /// stale state.
    ///
    /// **Safety**: the closure must not stash a reference to the
    /// host that outlives this call (no `lua.create_thread` that
    /// captures host state, no Rust upvalues holding `&mut
    /// GameHost`). All host access happens through registered
    /// native shims, which themselves only run synchronously
    /// inside this scope.
    pub fn with_host<R>(
        &self,
        host: &mut GameHost,
        f: impl FnOnce(&Lua) -> mlua::Result<R>,
    ) -> mlua::Result<R> {
        self.lua.set_app_data(HostPtr::new(host as *mut _));
        let result = f(&self.lua);
        // Always remove, even on Err, so the next call starts
        // clean. `remove_app_data` returns `Option<T>` — discard.
        let _ = self.lua.remove_app_data::<HostPtr>();
        result
    }

    /// Load and execute the mission's `.lua` file. The path is
    /// `<mission_dir>/<stem>.lua`; the leading directory matches
    /// what the custom `require` resolver expects, so `require()`
    /// calls inside the script find their helpers.
    pub fn load_script(&self, stem: &str) -> Result<(), MissionLuaError> {
        let path = self.mission_dir.join(format!("{stem}.lua"));
        let src = std::fs::read(&path).map_err(|e| MissionLuaError::Io(path.clone(), e))?;
        self.lua
            .load(&src)
            .set_name(stem)
            .exec()
            .map_err(|e| MissionLuaError::Lua(path, e))?;
        Ok(())
    }
}

/// Install a `require(path)` global resolving `"foo.bar"` to
/// `<mission_dir>/foo/bar.lua`. Caches each module's return value
/// in a closed-over `HashMap` so repeated requires return the same
/// table — matches Lua's standard semantics.
///
/// Luau doesn't ship `package` / `package.path` / `package.loaded`,
/// so we replicate just the pieces Spellforge mission scripts use.
/// In practice that's `require("lib.common")` and friends — a flat
/// dotted name resolving to a single `.lua` file.
fn install_require(lua: &Lua, mission_dir: &Path) -> Result<(), mlua::Error> {
    let cache: Arc<Mutex<HashMap<String, mlua::Value>>> = Arc::new(Mutex::new(HashMap::new()));
    let root = mission_dir.to_path_buf();

    let require = lua.create_function(move |lua: &Lua, name: String| {
        if let Some(v) = cache.lock().unwrap().get(&name) {
            return Ok(v.clone());
        }
        let rel = name.replace('.', "/") + ".lua";
        let path = root.join(&rel);
        let src = std::fs::read(&path).map_err(|e| {
            mlua::Error::RuntimeError(format!(
                "require('{name}'): cannot read {}: {e}",
                path.display()
            ))
        })?;
        let chunk = lua.load(&src).set_name(name.clone());
        let value: mlua::Value = chunk
            .eval()
            .map_err(|e| mlua::Error::RuntimeError(format!("require('{name}'): {e}")))?;
        cache.lock().unwrap().insert(name, value.clone());
        Ok(value)
    })?;
    lua.globals().set("require", require)?;
    Ok(())
}

/// Strip the non-deterministic corners of the standard library and
/// reroute `math.random` through the engine's seeded RNG.
///
/// This is the Factorio approach (see their [Lua libraries
/// docs][factorio]): block sources of wall-clock / platform /
/// scheduler variance entirely, and replace `math.random` with a
/// deterministic generator shared across all peers. Without this,
/// two clients running the same mission would diverge after the
/// first `math.random` call or the first `os.time()` read, and
/// rollback replay within a single peer would diverge after a
/// scripted coroutine yield.
///
/// What we cannot fix from Rust today (and accept the risk for):
///
/// - `math.sin` / `math.cos` / `math.exp` / `math.log` use the C
///   runtime's libm, which differs by platform. Within a single
///   Luau version compiled into the binary this is deterministic
///   per platform; across platforms (Linux vs. Mac vs. Wasm) it
///   may drift in the last bit. Factorio shims these with their
///   own platform-independent math; we don't, yet. Spellforge
///   missions in the audit don't use them, so it doesn't block
///   anything launched today.
/// - Luau's `pairs` / `next` iteration order over hash-mode
///   tables is "internal layout dependent". We ship a single
///   vendored Luau build to every peer so the order is the same
///   in a given binary; bumping the Luau version is a sync
///   point.
///
/// [factorio]: https://lua-api.factorio.com/2.0.76/auxiliary/libraries.html
fn enforce_determinism(lua: &Lua) -> mlua::Result<()> {
    let globals = lua.globals();

    // Coroutines can yield across the engine tick boundary, which
    // would leak script state between rollback re-simulations.
    // Just remove the library — Spellforge missions don't use it
    // (none of the 10 audited scripts touch `coroutine`).
    globals.set("coroutine", mlua::Value::Nil)?;

    // Luau's stripped-down `os` still exposes wall-clock readers.
    // Nil them so a script that calls `os.time()` produces a
    // clear error rather than a silently-divergent value.
    if let Ok(os) = globals.get::<mlua::Table>("os") {
        for key in ["time", "clock", "date", "difftime"] {
            os.set(key, mlua::Value::Nil)?;
        }
    }

    // Reroute `math.random` through the engine's `sim_rng`. Every
    // peer must use the same Engine-owned `fastrand::Rng`, so identical
    // script calls produce identical rolls.
    // PARITY TODO: Lua Initialize currently runs outside the tick's installed
    // RNG scope; thread an explicit Engine RNG context through every event.
    // Three calling conventions match stock Lua:
    //
    //   math.random()    -> float in [0, 1)
    //   math.random(n)   -> int in [1, n]
    //   math.random(a,b) -> int in [a, b]
    let math: mlua::Table = globals.get("math")?;
    let rng = lua.create_function(
        |_, args: mlua::Variadic<i32>| -> mlua::Result<mlua::Value> {
            match args.len() {
                0 => Ok(mlua::Value::Number(robin_engine::sim_rng::f32() as f64)),
                1 => {
                    let n = args[0];
                    if n < 1 {
                        return Err(mlua::Error::RuntimeError(format!(
                            "math.random: upper bound must be >= 1, got {n}"
                        )));
                    }
                    Ok(mlua::Value::Integer(robin_engine::sim_rng::i32(1..=n)))
                }
                2 => {
                    let (a, b) = (args[0], args[1]);
                    if a > b {
                        return Err(mlua::Error::RuntimeError(format!(
                            "math.random: empty interval [{a}, {b}]"
                        )));
                    }
                    Ok(mlua::Value::Integer(robin_engine::sim_rng::i32(a..=b)))
                }
                n => Err(mlua::Error::RuntimeError(format!(
                    "math.random: expected 0..=2 args, got {n}"
                ))),
            }
        },
    )?;
    math.set("random", rng)?;

    // `math.randomseed(x)` becomes a no-op. The engine seeds
    // `sim_rng` once at mission start via `EngineArgs::rng_seed`;
    // letting Lua reseed it would desync rollback (the replay
    // never sees the reseed call).
    let noop_seed = lua.create_function(|_, _: mlua::Variadic<mlua::Value>| Ok(()))?;
    math.set("randomseed", noop_seed)?;

    Ok(())
}

/// Install `log(msg)` — a Factorio-style logging helper that
/// routes script-side messages through `tracing` rather than
/// stdout. Useful for debugging mods without enabling `print`
/// (which would push lines straight at the terminal). The
/// `target` of `rh_lua_script` lets users filter via
/// `RUST_LOG=rh_lua_script=debug`.
fn install_log(lua: &Lua) -> mlua::Result<()> {
    let log = lua.create_function(|_, msg: mlua::Variadic<mlua::Value>| {
        let mut buf = String::new();
        for (i, v) in msg.iter().enumerate() {
            if i > 0 {
                buf.push('\t');
            }
            buf.push_str(&format_lua_value(v));
        }
        tracing::info!(target: "rh_lua_script", "{buf}");
        Ok(())
    })?;
    lua.globals().set("log", log)?;
    Ok(())
}

fn format_lua_value(v: &mlua::Value) -> String {
    match v {
        mlua::Value::Nil => "nil".to_owned(),
        mlua::Value::Boolean(b) => b.to_string(),
        mlua::Value::Integer(i) => i.to_string(),
        mlua::Value::Number(n) => n.to_string(),
        mlua::Value::String(s) => s.to_str().map(|s| s.to_string()).unwrap_or_default(),
        other => format!("<{}>", other.type_name()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_state() -> (MissionLuaState, TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = MissionLuaState::new(dir.path()).expect("new");
        (state, dir)
    }

    #[test]
    fn require_is_installed() {
        let (state, _dir) = make_state();
        let kind: String = state.lua().load("return type(require)").eval().unwrap();
        assert_eq!(kind, "function");
    }

    /// Confirms our `Lua::new()` baseline doesn't expose `os.execute`
    /// or `io` — Luau doesn't ship them, so the sandbox surface
    /// starts smaller than Lua 5.4. We don't need to nil them
    /// ourselves.
    #[test]
    fn dangerous_libs_absent_by_default() {
        let (state, _dir) = make_state();
        let io_kind: String = state.lua().load("return type(io)").eval().unwrap();
        assert_eq!(io_kind, "nil", "Luau must not load `io`");
        // `os` is present in Luau but only with a minimal set
        // (`os.time`, `os.clock`, `os.date`, `os.difftime`). The
        // dangerous entries (`execute`, `remove`, `getenv`, …)
        // aren't there. Spot-check `os.execute`.
        let exec: String = state
            .lua()
            .load("return type((os or {}).execute)")
            .eval()
            .unwrap();
        assert_eq!(exec, "nil");
    }

    /// `enforce_determinism` must strip every wall-clock reader on
    /// `os` and the entire `coroutine` library. Without these,
    /// rollback replay would diverge after the first `os.time()`
    /// call or coroutine yield.
    #[test]
    fn non_deterministic_libs_stripped() {
        let (state, _dir) = make_state();
        for snippet in &[
            "return type((os or {}).time)",
            "return type((os or {}).clock)",
            "return type((os or {}).date)",
            "return type((os or {}).difftime)",
            "return type(coroutine)",
        ] {
            let kind: String = state.lua().load(*snippet).eval().unwrap();
            assert_eq!(kind, "nil", "stripped by enforce_determinism: {snippet}");
        }
    }

    /// `math.random` must route through `sim_rng` so all peers
    /// produce identical rolls. The `with_seed` helper installs a
    /// fresh deterministic RNG; calling `math.random` twice with
    /// the same seed must yield the same sequence.
    #[test]
    fn math_random_uses_sim_rng() {
        let dir = tempfile::tempdir().unwrap();
        let take5 = |seed: u64| {
            robin_engine::sim_rng::with_seed(seed, || {
                let state = MissionLuaState::new(dir.path()).unwrap();
                (0..5)
                    .map(|_| {
                        state
                            .lua()
                            .load("return math.random(1, 1000000)")
                            .eval::<i64>()
                            .unwrap()
                    })
                    .collect::<Vec<_>>()
            })
        };
        // Same seed → same sequence.
        assert_eq!(take5(0xDEAD_BEEF), take5(0xDEAD_BEEF));
        // Different seed → different sequence (vanishingly small
        // chance of a false positive across 5 draws).
        assert_ne!(take5(0xDEAD_BEEF), take5(0xC0FFEE));
    }

    /// `math.randomseed` is a no-op — letting Lua scripts reseed
    /// the engine RNG would desync rollback (the replay can't see
    /// the reseed). Confirm the call doesn't panic and the
    /// following draw still comes from the engine seed.
    #[test]
    fn math_randomseed_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        let baseline = robin_engine::sim_rng::with_seed(7, || {
            let state = MissionLuaState::new(dir.path()).unwrap();
            state
                .lua()
                .load("math.randomseed(999); return math.random(1, 1000000)")
                .eval::<i64>()
                .unwrap()
        });
        let no_seed = robin_engine::sim_rng::with_seed(7, || {
            let state = MissionLuaState::new(dir.path()).unwrap();
            state
                .lua()
                .load("return math.random(1, 1000000)")
                .eval::<i64>()
                .unwrap()
        });
        assert_eq!(
            baseline, no_seed,
            "math.randomseed must not advance the engine RNG"
        );
    }

    #[test]
    fn load_script_runs_top_level() {
        let (state, dir) = make_state();
        fs::write(dir.path().join("mission.lua"), "_G.mission_loaded = 42\n").unwrap();
        state.load_script("mission").unwrap();
        let v: i64 = state.lua().globals().get("mission_loaded").unwrap();
        assert_eq!(v, 42);
    }

    #[test]
    fn require_resolves_lib_subdir() {
        let (state, dir) = make_state();
        fs::create_dir(dir.path().join("lib")).unwrap();
        fs::write(
            dir.path().join("lib/common.lua"),
            "return { hello = 'world' }\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("mission.lua"),
            "_G.greeting = require('lib.common').hello\n",
        )
        .unwrap();
        state.load_script("mission").unwrap();
        let g: String = state.lua().globals().get("greeting").unwrap();
        assert_eq!(g, "world");
    }

    /// Calling `require` twice on the same module returns the same
    /// table — matches stock Lua's `package.loaded` caching, which
    /// Spellforge's `lib/common.lua` relies on (it stashes
    /// mission-scoped state on the returned table).
    #[test]
    fn require_caches_modules() {
        let (state, dir) = make_state();
        fs::write(dir.path().join("m.lua"), "return {}\n").unwrap();
        let same: bool = state
            .lua()
            .load("return require('m') == require('m')")
            .eval()
            .unwrap();
        assert!(same, "require must cache");
    }
}
