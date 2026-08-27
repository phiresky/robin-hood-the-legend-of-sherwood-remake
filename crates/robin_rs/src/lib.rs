//! robin_rs — the Rust portion of the Robin Hood: The Legend of Sherwood
//! Rust port.
//!
//! Each subsystem lives in its own module.

#![deny(unsafe_op_in_unsafe_fn)]
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]
// Rollback multiplayer requires every gameplay RNG pull to use the explicit
// context derived from `EngineInner::control.rng` via `robin_engine::sim_rng`.
// See `clippy.toml` for the banned function list. Individual escape hatches
// (UI, audio jitter, tests) must carry an
// `#[allow(clippy::disallowed_methods)]` with a comment.
#![warn(clippy::disallowed_methods)]

use std::sync::{Mutex, Once, OnceLock};

#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
pub mod auto_update;
pub mod version;

static TRACING_INIT: Once = Once::new();
#[cfg(not(target_arch = "wasm32"))]
static REPLAY_LOG_FILE: OnceLock<Mutex<Option<std::fs::File>>> = OnceLock::new();

/// Initialize the tracing subscriber for library use.
/// Safe to call multiple times — only the first call takes effect.
/// On wasm the bin entry installs `tracing-wasm` *before* the bundle
/// fetch + `wasm_boot`, so this becomes a no-op (any `init` here would
/// panic with `SetGlobalDefaultError`).
pub fn init_tracing() {
    TRACING_INIT.call_once(|| {
        #[cfg(not(target_arch = "wasm32"))]
        {
            use std::io::IsTerminal;
            use tracing_subscriber::{filter::LevelFilter, prelude::*};

            let ansi = std::io::stderr().is_terminal();
            let stderr_layer = tracing_subscriber::fmt::layer()
                .with_ansi(ansi)
                .with_writer(std::io::stderr)
                .with_filter(build_env_filter());
            let replay_file_layer = tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(ReplayLogMakeWriter)
                .with_filter(LevelFilter::DEBUG);
            tracing_subscriber::registry()
                .with(stderr_layer)
                .with(replay_file_layer)
                .init();
        }
    });
}

/// Start writing DEBUG-and-higher tracing events to `path`.
///
/// The tracing subscriber is installed during early process startup,
/// before the replay filename is known, so this swaps the destination
/// used by the always-installed replay log layer.
#[cfg(not(target_arch = "wasm32"))]
pub fn set_replay_log_file(path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path.as_ref())?;
    *REPLAY_LOG_FILE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("replay log file mutex poisoned") = Some(file);
    tracing::info!("Replay debug log → {}", path.as_ref().display());
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
struct ReplayLogMakeWriter;

#[cfg(not(target_arch = "wasm32"))]
struct ReplayLogWriter;

#[cfg(not(target_arch = "wasm32"))]
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for ReplayLogMakeWriter {
    type Writer = ReplayLogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        ReplayLogWriter
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl std::io::Write for ReplayLogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Some(file) = REPLAY_LOG_FILE
            .get_or_init(|| Mutex::new(None))
            .lock()
            .expect("replay log file mutex poisoned")
            .as_mut()
        {
            file.write_all(buf)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(file) = REPLAY_LOG_FILE
            .get_or_init(|| Mutex::new(None))
            .lock()
            .expect("replay log file mutex poisoned")
            .as_mut()
        {
            file.flush()?;
        }
        Ok(())
    }
}

/// Build the `EnvFilter` honoring `RUST_LOG`, but ensure WARN-level
/// events still surface when the user scoped `RUST_LOG` to specific
/// targets (e.g. `RUST_LOG=robin_engine=info,robin_rs=info`).  Without
/// this floor, the `robin` binary's own `tracing::error!` calls — and
/// anything else outside the listed targets — would be silenced.
#[cfg(not(target_arch = "wasm32"))]
fn build_env_filter() -> tracing_subscriber::EnvFilter {
    use tracing_subscriber::EnvFilter;
    match std::env::var("RUST_LOG") {
        Ok(s) if !s.is_empty() => {
            let composed = compose_env_filter(&s);
            EnvFilter::try_new(&composed).unwrap_or_else(|_| EnvFilter::new("info"))
        }
        _ => EnvFilter::new("info"),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn compose_env_filter(s: &str) -> String {
    // A bare global directive such as `RUST_LOG=debug` should mean
    // "debug the game" during normal development, not "enable
    // chatty debug tracing from wgpu/naga/symphonia/etc.".  Keep
    // dependency targets at WARN unless the user explicitly names
    // them in RUST_LOG.
    let trimmed = s.trim();
    if matches!(trimmed, "trace" | "debug") {
        return format!(
            "warn,robin_rs={trimmed},robin_engine={trimmed},robin_assets={trimmed},robin_util={trimmed}"
        );
    }

    // A "global" directive is one without a `target=` prefix; it
    // sets the default level for unlisted targets.  If the user
    // already provided one (e.g. `RUST_LOG=info,robin_engine=trace`),
    // respect it.  Otherwise prepend `warn,` so warnings and errors
    // from every target still show.
    let has_global = s
        .split(',')
        .any(|d| !d.trim().is_empty() && !d.contains('='));
    if has_global {
        s.to_owned()
    } else {
        format!("warn,{s}")
    }
}

// ──────────────────────────────────────────────────────────────────
// Host-local modules (files in robin_rs/src/)
// ──────────────────────────────────────────────────────────────────
#[cfg(target_os = "android")]
pub mod android;
pub mod app_effect;
pub mod bg_cache;
pub mod blit_to_map;
pub mod campaign_map;
pub mod console_overlay;
pub mod corner_hud;
pub mod cursor;
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
pub mod datadir_locator;
pub mod debug_stub;
pub mod draw_manager;
pub mod focus_manager;
pub mod font;
pub mod game;
pub mod game_input;
pub mod game_render;
pub mod game_session;
pub mod gamepad;
pub mod gpu_upscale;
pub mod hardware;
pub mod host;
pub mod host_mouse;
pub mod http_server;
pub mod hud_text;
pub mod level_loading_host;
pub mod shader_preset;
/// Host-side runtime state used by the game loop and developer tooling.
///
/// Engine and asset types are intentionally not re-exported from this crate;
/// consumers should import them from `robin_engine` and `robin_assets`.
pub use host::Host;
pub mod audio_backend;
pub mod audio_duration_cache;
pub mod gfx_types;
pub mod ingame_menu;
pub mod input;
pub mod input_translator;
pub(crate) mod json_value;
pub mod key_config;
pub mod key_config_store;
pub mod loading_dissolve_gpu;
pub mod loading_screen;
#[cfg(not(target_arch = "wasm32"))]
pub mod lua_session;
#[cfg(target_arch = "wasm32")]
#[path = "lua_session_wasm.rs"]
pub mod lua_session;
pub mod main_entry;
pub mod main_menu;
pub mod markers;
pub mod menu;
pub mod mod_pack;
pub mod mouse_trail;
pub mod mouse_way;
pub mod multiplayer;
pub mod native_font;
pub mod pc_info_overlay;
pub mod portrait_bar;
pub mod presentation;
pub mod process_asset_cache;
pub mod profiler;
pub mod recon_report;
pub mod renderer;
pub mod replay_format;
pub mod rewind;
pub mod rollback_checker;
pub mod save_file;
pub mod savegame;
pub mod settings;
pub mod shadow_polygon;
pub mod sherwood_hud;
pub mod sim_timeline;
pub mod sound;
pub mod stature_hud;
pub mod titbit_renderer;
pub mod toolbox;
pub mod ui;
pub mod ui_panel;
pub mod ui_screens;
pub mod video_player;
pub mod widget;
pub mod window;
pub mod zoom_hud;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::compose_env_filter;

    #[test]
    fn bare_debug_filters_to_game_crates() {
        assert_eq!(
            compose_env_filter("debug"),
            "warn,robin_rs=debug,robin_engine=debug,robin_assets=debug,robin_util=debug"
        );
    }

    #[test]
    fn scoped_filter_keeps_warning_floor() {
        assert_eq!(
            compose_env_filter("engine_api::movement=debug"),
            "warn,engine_api::movement=debug"
        );
    }

    #[test]
    fn explicit_global_filter_is_respected() {
        assert_eq!(
            compose_env_filter("info,robin_engine=trace"),
            "info,robin_engine=trace"
        );
    }
}
