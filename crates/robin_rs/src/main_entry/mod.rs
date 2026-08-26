//! Rust entry point for the game.
//!
//! Performs early initialization (data directory, logging, profiles, campaign),
//! then runs the game loop.
//!
//! Split by concern:
//! - [`cli`]       — command-line / URL-query argument parsing
//! - [`init`]      — data directory, locale, profile, and key-config setup
//! - [`callbacks`] — game-flow callbacks, save/load plumbing, launch helpers
//! - [`run`]       — the outer main-menu / mission run loops
//!
//! The actual gameplay code lives in dedicated modules:
//! - [`crate::main_menu`]    — graphical main menu screen
//! - [`crate::campaign_map`] — campaign map / mission selection screen
//! - [`crate::game_session`] — mission loop and per-mission setup
//! - [`crate::game_input`]   — left/right click handlers for the mission loop
//! - [`crate::game_render`]  — in-game rendering passes (entities, outlines, minimap, …)

mod callbacks;
mod cli;
mod init;
mod run;

pub use cli::{
    CliArgs, PendingLuaMission, RHREC_EXT, parse_cli, parse_cli_from, try_parse_cli_from,
};

pub use init::{
    FALLBACK_LOCALE_FOLDER, InitError, InitErrorCategory, LANGUAGE_FOLDERS, OVERLAY_DATA_DIRS_ENV,
    RustInit, overlay_mods_dir, register_language_data_paths_for_tool, rust_init,
    rust_init_with_data_dir, rust_init_with_shipping,
};

pub use callbacks::{PendingLevelLoad, PostLoadSync, SaveBannerKind, SaveLoadRequest};
pub(crate) use callbacks::{
    RustCallbacks, SaveLoadEvent, current_mission_id, detect_demo_mode_with_context,
    execute_app_effects, perform_pending_save_load, picture_to_surface,
    preflight_or_use_decoded_load, resolve_loading_pak, validate_save_mission,
    validated_save_reload_target,
};

pub use run::{run_rust_game, run_rust_game_headless};
