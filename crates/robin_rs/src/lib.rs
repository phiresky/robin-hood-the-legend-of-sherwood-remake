//! robin_rs — the Rust portion of the Robin Hood: The Legend of Sherwood
//! Rust port.
//!
//! Each subsystem lives in its own module.

#![deny(unsafe_op_in_unsafe_fn)]
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]
// Rollback multiplayer requires every gameplay RNG pull to come from
// `Engine::rng` via `crate::sim_rng::*`. See `clippy.toml` for the banned
// function list. Individual escape hatches (UI, audio jitter, tests) must
// carry an `#[allow(clippy::disallowed_methods)]` with a comment.
#![warn(clippy::disallowed_methods)]

use robin_engine::engine as engine_api;
use std::sync::{Mutex, Once, OnceLock};

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
pub use robin_engine::alert_colors;
pub use robin_util::asset_fs;
#[cfg(target_os = "android")]
pub mod android;
pub mod bg_cache;
pub mod blit_to_map;
pub mod campaign_map;
pub mod console_overlay;
pub mod corner_hud;
pub mod cursor;
pub mod debug_stub;
pub mod draw_manager;
pub mod focus_manager;
pub mod font;
pub mod font_manager;
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
pub use bg_cache::BackgroundDecal;
pub use host::Host;
// Convenience re-exports so host-side submodules can use
// `crate::Engine` / `crate::LevelAssets` etc. without fully qualifying
// every call. These are stable entry points, not migration shims.
pub use engine_api::level_loading;
pub use robin_engine::engine::{Engine, LevelAssets, PendingBgBlit};
pub mod ingame_menu;
pub mod input;
pub mod input_translator;
pub(crate) mod json_value;
pub mod key_config_store;
pub mod loading_dissolve_gpu;
pub mod loading_screen;
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
pub mod profiler;
pub mod recon_report;
pub mod renderer;
pub use robin_assets::resource_manager;
pub mod gfx_types;
pub mod rewind;
pub mod rollback_checker;
pub mod save_file;
pub mod savegame;
pub mod sdl_audio;
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

// ──────────────────────────────────────────────────────────────────
// Convenience re-exports — host-side submodules address engine/asset
// modules via `crate::<name>` without having to fully qualify every
// call.
// ──────────────────────────────────────────────────────────────────
pub use robin_engine::abilities;
pub use robin_engine::ai;
pub use robin_engine::ai_enemy;
pub use robin_engine::ai_entity_view;
pub use robin_engine::ai_friendly;
pub use robin_engine::ai_vision;
pub use robin_engine::bow_shot;
pub use robin_engine::campaign;
pub use robin_engine::change;
pub use robin_engine::combat;
pub use robin_engine::console;
pub use robin_engine::element;
pub use robin_engine::engine;
pub use robin_engine::entity_id;
pub use robin_engine::event;
pub use robin_engine::fast_find_grid;
pub use robin_engine::game_operation;
pub use robin_engine::gate;
pub use robin_engine::graphic_config;
pub use robin_engine::interp;
pub use robin_engine::inventory;
pub use robin_engine::jump_line;
pub use robin_engine::level_data as level_loader;
pub use robin_engine::macro_store;
pub use robin_engine::mask;
pub use robin_engine::md5;
pub use robin_engine::messenger;
pub use robin_engine::minimap;
pub use robin_engine::mission;
pub use robin_engine::mission_stat;
pub use robin_engine::movement;
pub use robin_engine::natives;
pub use robin_engine::order;
pub use robin_engine::parameters_ai;
pub use robin_engine::patch;
pub use robin_engine::path;
pub use robin_engine::pathfinder;
pub use robin_engine::pc_status;
pub use robin_engine::player_command;
pub use robin_engine::player_profile;
pub use robin_engine::position_interface;
pub use robin_engine::profiles;
pub use robin_engine::replay;
pub mod replay_format;
pub use robin_engine::resource_ids;
pub use robin_engine::rhline;
pub use robin_engine::sbfile;
pub use robin_engine::script_manager;
pub use robin_engine::sector;
pub use robin_engine::sector_production;
pub use robin_engine::sequence;
pub use robin_engine::sherwood_stat;
pub use robin_engine::short_briefings;
pub use robin_engine::sight_obstacle;
pub use robin_engine::sim_rng;
pub use robin_engine::sound_cache;
pub use robin_engine::sound_config;
pub use robin_engine::sound_geometry;
pub use robin_engine::sound_source;
pub use robin_engine::sprite;
pub use robin_engine::sprite_script as sprite_scriptor;
pub use robin_engine::stealth;
pub use robin_engine::titbit;
pub use robin_engine::water_zones;
pub use robin_engine::weapons;

pub use robin_assets::actor_names;
pub use robin_assets::adpcm_check;
pub use robin_assets::decompile;
pub use robin_assets::disasm;
pub use robin_assets::frame_holder;
pub use robin_assets::keyconfig;
pub use robin_assets::picture;
pub use robin_assets::res_descr;
pub use robin_assets::sb3d;
pub use robin_assets::scb;
pub use robin_assets::serialize;
pub use robin_assets::shipping_datadir;
pub use robin_engine::vm;

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
