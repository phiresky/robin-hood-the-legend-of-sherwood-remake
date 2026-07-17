//! Rust entry point for the game.
//!
//! Performs early initialization (data directory, logging, profiles, campaign),
//! then runs the game loop.
//!
//! The actual gameplay code lives in dedicated modules:
//! - [`crate::main_menu`]    — graphical main menu screen
//! - [`crate::campaign_map`] — campaign map / mission selection screen
//! - [`crate::game_session`] — mission loop and per-mission setup
//! - [`crate::game_input`]   — left/right click handlers for the mission loop
//! - [`crate::game_render`]  — in-game rendering passes (entities, outlines, minimap, …)

use robin_assets::picture::Picture;
use robin_assets::shipping_datadir as assets_shipping_datadir;
use robin_engine::campaign as engine_campaign;
use robin_engine::engine as engine_api;
use robin_engine::profiles as engine_profiles;
use robin_engine::profiles::ProfileManager;
use robin_engine::replay as engine_replay;
use robin_engine::sbfile as engine_sbfile;
use std::ffi::OsString;
#[cfg(any(not(target_arch = "wasm32"), target_os = "android"))]
use std::path::Path;

use clap::Parser;
use serde::Deserialize;

use crate::app_effect::{AppEffect, AppEffectExecutionError, AppEffectQueue};
use crate::campaign::Campaign;
use crate::host::ApplicationContext;
use crate::key_config_store::KeyConfigStore;
use crate::player_profile::{DifficultyLevel, PlayerProfileManager};
use crate::replay_format::COMPACT_PREFIX;
use crate::save_file::special_slots;
use crate::savegame::{SaveGameManager, SpecialSlot};
use crate::sbfile::{SBFILE_ERROR_PATH_ALREADY_PRESENT, SBFILE_NO_ERROR, SbFile};
use crate::sound::{Jingle as SoundJingle, SoundMode as AudioSoundMode};

/// Extension required for replay files — keeps the format searchable
/// and future-proofs us if we ever want to associate `.rhrec.jsonl` with
/// a handler at the OS level.
pub const RHREC_EXT: &str = ".rhrec.jsonl";

/// clap `value_parser` for `--record`: rejects anything that doesn't
/// end in `.rhrec.jsonl`. Recording always goes to the legacy JSONL
/// streaming format for crash-safety; the compact sharing format is
/// produced on demand from a finished recording.
fn parse_record_path(s: &str) -> Result<String, String> {
    if s.ends_with(RHREC_EXT) {
        Ok(s.to_string())
    } else {
        Err(format!("record path must end in `{RHREC_EXT}` (got `{s}`)"))
    }
}

/// clap `value_parser` for `--replay`: accepts either an inline
/// `rhrec-…` compact string (shared replay pasted on the command line)
/// or a filesystem path. Path validation is lenient because the loader
/// (`replay_format::load_replay_spec`) auto-detects JSONL vs. a file
/// holding a `rhrec-…` string, regardless of extension.
fn parse_replay_spec(s: &str) -> Result<String, String> {
    if s.trim_start().starts_with(COMPACT_PREFIX) {
        return Ok(s.to_string());
    }
    // A path — we don't require any particular extension, but a
    // friendlier error is easy to give when the user clearly fat-
    // fingered a `rhrec` variant.
    Ok(s.to_string())
}

/// Robin Hood — The Legend of Sherwood (Rust port)
#[derive(Parser, Debug, Clone, Deserialize)]
#[command(version, about)]
#[serde(default, rename_all = "kebab-case")]
pub struct CliArgs {
    /// Disable audio playback.
    #[arg(long)]
    pub no_sound: bool,

    /// Disable mission script execution.
    #[arg(long)]
    pub no_script: bool,

    /// GoldenEye mode: NPCs cannot see player characters
    #[arg(long)]
    pub goldeneye: bool,

    /// Spawn enemy NPCs as invulnerable.
    #[arg(long)]
    pub highlander2: bool,

    /// Bypass fog sprite loading that can crash on some converted data.
    #[arg(long)]
    pub no_fog: bool,

    /// Show the AI "whatsup" debug overlay.
    #[arg(long)]
    pub whatsup: bool,

    /// Ignore the default mission-lost condition.
    #[arg(long)]
    pub no_default_loose: bool,

    /// Validate cached sound data during startup.
    #[arg(long)]
    pub check_sound_data: bool,

    /// Show view cones for all NPCs at all times
    #[arg(long)]
    pub view_cones: bool,

    /// Show the debug-surfaces overlay (walkable motion areas + selected
    /// character's surface and committed path).  Toggle at runtime with
    /// the `SURFACE` console command.
    #[arg(long)]
    pub debug_surfaces: bool,

    /// Record a replay to the given file path (must end in `.rhrec.jsonl`)
    #[arg(long, value_parser = parse_record_path)]
    pub record: Option<String>,

    /// Play back a replay. Accepts any of:
    ///   - an inline `rhrec-…` compact string (the sharing format),
    ///   - a file containing a `rhrec-…` string,
    ///   - a legacy `*.rhrec.jsonl` recording.
    ///
    /// The replay's header picks the mission to load.
    #[arg(long, value_parser = parse_replay_spec)]
    pub replay: Option<String>,

    /// Decoded replay payload supplied by the wasm shell over script RPC.
    ///
    /// Kept separate from `replay` so the engine can be seeded before
    /// construction without serializing an already-decoded replay back
    /// into a command-line string.
    #[arg(skip)]
    #[serde(skip)]
    pub replay_data: Option<engine_replay::ReplayData>,

    /// Runtime rollback consistency checker: rewind a short window of
    /// engine state and re-simulate it to detect desyncs.
    /// On by default — pass `--rollback-check=false` to disable.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub rollback_check: bool,

    /// Skip the main menu and drop directly into the Sherwood (HQ)
    /// mission — useful for iterating on the Sherwood HUD without
    /// clicking through the menu + campaign-map flow.
    #[arg(long)]
    pub sherwood: bool,

    /// Force the graphical main menu even when a demo data directory is
    /// detected.  Demo datadirs normally auto-start their bundled mission.
    #[arg(long)]
    pub force_main_menu: bool,

    /// Skip the menus and launch this mission filename directly, like
    /// the original launcher's `-MISSION`. Pass the base name without
    /// `.rhm`; when `--proto` is omitted, the mission name is also used
    /// as the proto-level name.
    #[arg(
        long,
        value_name = "MISSION",
        conflicts_with_all = ["sherwood", "replay", "wait_for_command"]
    )]
    pub mission: Option<String>,

    /// Proto-level filename to use with `--mission`, like the original
    /// launcher's `-PROTO`. Pass the base name without `.rhp`.
    #[arg(long, value_name = "PROTO", requires = "mission")]
    pub proto: Option<String>,

    /// Run only the headless multiplayer lobby broker on this WebSocket
    /// bind address.  With no value, binds `0.0.0.0:7879`.
    #[arg(
        long,
        value_name = "HOST:PORT",
        num_args = 0..=1,
        default_missing_value = crate::multiplayer::lobby::DEFAULT_LOBBY_BIND
    )]
    pub lobby_server: Option<String>,

    /// TCP port for the local script-RPC HTTP server.
    /// Default 17640 (loopback only). Set to 0 to disable.
    /// See `crate::http_server` for the wire format.
    #[arg(long, default_value_t = crate::http_server::DEFAULT_PORT)]
    pub http_server: u16,

    /// Run the frame loop with no 25 fps pacing sleep — ticks and
    /// renders happen back-to-back at full CPU/GPU speed.  Useful for
    /// automated tests, replay scrubbing, and profiling.  Independent
    /// of the in-game fast-forward toggle (which also skips rendering);
    /// with this flag rendering still runs every frame.
    #[arg(long)]
    pub fast_forward: bool,

    /// Skip the per-frame render pass entirely: no `pre_render` GPU
    /// drains, no scene draw, no cursor update, no `present()`.  The
    /// SDL window is still created (set `SDL_VIDEODRIVER=dummy` for a
    /// truly displayless host) so input / events still flow, but no
    /// pixels are produced.  Implies no pacing sleep — the loop runs
    /// at full CPU speed, just like `--fast-forward`.  Useful for
    /// replay scrubbing, automated tests, and CI runs without a GPU.
    #[arg(long)]
    pub headless: bool,

    /// Open the mission with the simulation paused — the engine tick is
    /// suspended until a `/step-forward` HTTP request (or any other
    /// path that flips `pause` off) drives it forward.  Rendering, HUD,
    /// and input still run normally; the pause menu is not shown.
    /// Useful for scripted test drivers that want full control over
    /// when frames advance.
    #[arg(long)]
    pub start_paused: bool,

    /// Finish data load, then idle on a "waiting for command" loading
    /// screen until the script-RPC `load-replay` endpoint queues a
    /// replay.  The replay's header picks the mission; no auto-start
    /// (demo detection, `--sherwood`, main menu) fires.  Used by the
    /// wasm host so URL-driven replay load isn't racing the
    /// auto-start — JS needs a window after Rust init to send
    /// `load-replay` before a mission gets to consume the pending
    /// slot.
    #[arg(long)]
    pub wait_for_command: bool,

    /// Run as a multiplayer server, listening for peer connections on
    /// the given `host:port` (e.g. `0.0.0.0:7878`).  Bare-string `:7878`
    /// also works (binds all interfaces).  This process drives seat 0
    /// (`PlayerId::HOST`); peers receive `PlayerId(1+)` in join order.
    ///
    /// Mutually exclusive with `--connect`.
    #[arg(long, value_name = "HOST:PORT")]
    pub server: Option<String>,

    /// Run as a multiplayer client, connecting to `host:port`.  The
    /// server assigns a join-order seat which the client then drives
    /// for the rest of the session.
    ///
    /// Mutually exclusive with `--server`.
    #[arg(long, value_name = "HOST:PORT")]
    pub connect: Option<String>,

    /// Internal lobby handoff: keep the simulation paused until this
    /// wall-clock timestamp so host and joiners begin together.
    #[arg(long, hide = true)]
    pub mp_start_at_epoch_ms: Option<u64>,

    /// Internal lobby handoff: total player count the host should wait
    /// for at the multiplayer ready barrier.
    #[arg(long, hide = true)]
    pub mp_expected_players: Option<u32>,

    /// Nickname shown in the portrait "controlled by" overlay on
    /// peers.  Defaults to a host-name-derived fallback when omitted.
    #[arg(long, value_name = "NICKNAME", default_value = "")]
    pub mp_nickname: String,

    /// Runtime startup options consumed by engine/UI layers that have
    /// not been threaded through `CliArgs` directly.
    #[clap(skip)]
    #[serde(skip)]
    pub global_options: ApplicationContext,

    /// Internal handoff from the custom-mission picker. Spellforge-tagged
    /// launches carry the bits needed to construct a required `LuaSession`;
    /// Vanilla-tagged custom missions carry the same launch metadata but
    /// intentionally produce no Lua state. `None` for every non-mod launch.
    /// Not a real CLI flag; not serialised.
    #[clap(skip)]
    #[serde(skip)]
    pub pending_lua_mission: Option<PendingLuaMission>,

    /// Internal one-shot render request used by the `render_mission_map`
    /// example.  The mission session captures the complete level at its
    /// initial post-`Initialize` state, writes it here, and exits before the
    /// first simulation tick.  This is deliberately not a launcher flag:
    /// the Cargo example is the supported CLI for this specialized tool.
    #[clap(skip)]
    #[serde(skip)]
    pub mission_start_map_output: Option<std::path::PathBuf>,

    /// Apply the original `UBIQUITY` / `UNBLIP` reveal-all-NPCs cheat to
    /// the one-shot mission-start map before it is rendered.
    #[clap(skip)]
    #[serde(skip)]
    pub mission_start_reveal_all: bool,
}

/// Subset of [`crate::main_menu::custom_missions::CustomMissionLaunch`]
/// needed to decide containment and, for Spellforge, construct a
/// [`crate::lua_session::LuaSession`] inside `run_mission`. Kept as a flat
/// clonable struct so it can ride along on `CliArgs` (which is `Clone`).
#[derive(Debug, Clone)]
pub struct PendingLuaMission {
    pub slug: String,
    pub rhm_basename: String,
    pub version_zip: std::path::PathBuf,
    pub mods_root: std::path::PathBuf,
    pub requires_spellforge: bool,
}

impl Default for CliArgs {
    fn default() -> Self {
        let mut args = Self {
            no_sound: false,
            no_script: false,
            goldeneye: false,
            highlander2: false,
            no_fog: false,
            whatsup: false,
            no_default_loose: false,
            check_sound_data: false,
            view_cones: false,
            debug_surfaces: false,
            record: None,
            replay: None,
            replay_data: None,
            rollback_check: true,
            sherwood: false,
            force_main_menu: false,
            mission: None,
            proto: None,
            lobby_server: None,
            http_server: crate::http_server::DEFAULT_PORT,
            fast_forward: false,
            headless: false,
            start_paused: false,
            wait_for_command: false,
            server: None,
            connect: None,
            mp_start_at_epoch_ms: None,
            mp_expected_players: None,
            mp_nickname: String::new(),
            global_options: ApplicationContext::default(),
            pending_lua_mission: None,
            mission_start_map_output: None,
            mission_start_reveal_all: false,
        };
        install_global_options(&mut args);
        args
    }
}

fn install_global_options(args: &mut CliArgs) {
    let opts = engine_api::GlobalOptions {
        sound_enabled: !args.no_sound,
        script_enabled: !args.no_script,
        highlander2: args.highlander2,
        bypass_fog_sprites_crash: args.no_fog,
        whatsup: args.whatsup,
        debug_surfaces: args.debug_surfaces,
        golden_eye: args.goldeneye,
        ignore_default_loose: args.no_default_loose,
        check_sound_data: args.check_sound_data,
        ..Default::default()
    };

    args.global_options = ApplicationContext::bootstrap(opts.clone());
    // Install the process-wide `GlobalOptions` so UI layers that don't
    // have a `Game` or `CliArgs` in scope can still read startup flags.
    engine_api::GlobalOptions::set_global(opts);
}

pub fn try_parse_cli_from<I, T>(itr: I) -> Result<CliArgs, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let mut args = CliArgs::try_parse_from(itr)?;
    install_global_options(&mut args);
    Ok(args)
}

pub fn parse_cli_from<I, T>(itr: I) -> CliArgs
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    try_parse_cli_from(itr).unwrap_or_else(|e| e.exit())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn parse_cli() -> CliArgs {
    parse_cli_from(std::env::args_os())
}

#[cfg(target_arch = "wasm32")]
pub fn parse_cli() -> CliArgs {
    wasm_cli_args_from_location()
}

#[cfg(target_arch = "wasm32")]
fn wasm_cli_args_from_location() -> CliArgs {
    let query = web_sys::window()
        .and_then(|window| window.location().search().ok())
        .unwrap_or_default();
    let query = query.strip_prefix('?').unwrap_or(&query);
    let query = normalize_wasm_query(query);
    let mut args = match serde_urlencoded::from_str::<CliArgs>(&query) {
        Ok(args) => args,
        Err(e) => {
            tracing::warn!("invalid wasm URL options: {e}; using defaults");
            CliArgs::default()
        }
    };
    if args.replay.is_some() {
        // URL replays are loaded by the shell over RPC after Rust has
        // finished initialization, so the mission header can choose the
        // correct mission without racing demo auto-start.
        args.wait_for_command = true;
        args.replay = None;
    }
    install_global_options(&mut args);
    args
}

#[cfg(target_arch = "wasm32")]
fn normalize_wasm_query(query: &str) -> String {
    query
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let Some((key, value)) = part.split_once('=') else {
                return format!("{}=true", part.replace('_', "-"));
            };
            let key = key.replace('_', "-");
            let value = match value {
                "" | "1" | "yes" | "on" => "true",
                "0" | "no" | "off" => "false",
                _ => value,
            };
            format!("{key}={value}")
        })
        .collect::<Vec<_>>()
        .join("&")
}

#[cfg(test)]
mod tests {
    use clap::error::ErrorKind;
    use robin_engine::profiles::ProfileManager;

    use super::{current_mission_id, required_mission_id, try_parse_cli_from};
    use crate::campaign::Campaign;

    #[test]
    fn clap_launcher_flags_populate_global_options() {
        let args = try_parse_cli_from([
            "robin",
            "--no-sound",
            "--no-script",
            "--highlander2",
            "--no-fog",
            "--whatsup",
            "--goldeneye",
            "--no-default-loose",
            "--check-sound-data",
        ])
        .unwrap();

        assert!(!args.global_options.sound_enabled);
        assert!(!args.global_options.script_enabled);
        assert!(args.global_options.highlander2);
        assert!(args.global_options.bypass_fog_sprites_crash);
        assert!(args.global_options.whatsup);
        assert!(args.goldeneye);
        assert!(args.global_options.golden_eye);
        assert!(args.global_options.ignore_default_loose);
        assert!(args.global_options.check_sound_data);
    }

    #[test]
    fn legacy_launcher_flags_are_rejected_by_clap() {
        let err = try_parse_cli_from(["robin", "-NOSOUND"]).unwrap_err();

        assert_eq!(err.kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn mission_flag_defaults_proto_to_mission_name() {
        let args = try_parse_cli_from(["robin", "--mission", "Dem_Lei_MP"]).unwrap();

        assert_eq!(args.mission.as_deref(), Some("Dem_Lei_MP"));
        assert_eq!(args.proto.as_deref().unwrap_or("Dem_Lei_MP"), "Dem_Lei_MP");
    }

    #[test]
    fn proto_requires_mission() {
        let err = try_parse_cli_from(["robin", "--proto", "Leicester"]).unwrap_err();

        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
    }

    #[test]
    #[should_panic(expected = "required test mission: mission ID zero is invalid")]
    fn required_mission_id_rejects_zero() {
        required_mission_id(Some(0), "required test mission");
    }

    #[test]
    #[should_panic(expected = "current_mission_id: campaign must have a valid current mission")]
    fn current_mission_id_rejects_missing_current_mission() {
        current_mission_id(&Campaign::default(), &ProfileManager::new());
    }
}
use crate::game_operation::GameCode;
use crate::game_session::{SessionResult, run_mission, run_mission_headless, run_session};
use crate::main_menu::multiplayer_lobby::MultiplayerRole;
use crate::main_menu::{MainMenuChoice, show_main_menu};
use crate::profiles::MissionLocation;
use crate::renderer::Renderer;
use crate::window::GameWindow;

// ─── Data directory setup ───────────────────────────────────────────

/// Locale-specific subfolders the game data may ship with.
///
/// Each entry is a Windows LCID string. The game's localized resources
/// (`<lcid>/Data/Text/Level.res`, `<lcid>/Data/Interface/Start.sxt`, etc.)
/// override the unlocalized files under `Data/`.
///
/// Order is the international-build order:
/// German, "neutral" (2047 — used by some French builds), French, Italian,
/// Brazilian Portuguese, Mexican Spanish, Russian, Japanese, Czech, Polish,
/// Portuguese, Traditional Chinese, Korean, Simplified Chinese, Thai.
pub const LANGUAGE_FOLDERS: &[&str] = &[
    "1031", "2047", "1036", "1040", "2070", "3082", "1049", "1041", "1029", "1045", "1046", "1028",
    "1042", "2052", "1054",
];

/// English fallback locale folder, always added first in the international build.
pub const FALLBACK_LOCALE_FOLDER: &str = "1033";

/// Environment variable containing additional datadir roots to overlay on top
/// of the primary `ROBINHOOD_DATA_DIR`.  Native builds use the platform path
/// separator (`:` on Unix, `;` on Windows).
pub const OVERLAY_DATA_DIRS_ENV: &str = "ROBINHOOD_OVERLAY_DATA_DIRS";

#[cfg(not(target_arch = "wasm32"))]
fn add_overlay_data_dirs() {
    let Ok(value) = std::env::var(OVERLAY_DATA_DIRS_ENV) else {
        return;
    };
    for path in std::env::split_paths(&value) {
        if path.as_os_str().is_empty() {
            continue;
        }
        let path = path.to_string_lossy().into_owned();
        match SbFile::add_overlay_path(&path) {
            SBFILE_NO_ERROR => tracing::info!("Registered overlay datadir: {path}"),
            SBFILE_ERROR_PATH_ALREADY_PRESENT => {
                tracing::debug!("Overlay datadir already registered: {path}")
            }
            err => tracing::warn!("Failed to register overlay datadir {path}: {err}"),
        }
    }
}

/// Detect which locale subfolder is shipped with the data and register it
/// as an alternate path so localized resources resolve correctly.
///
/// The international build always adds `1033` first (English fallback) and
/// then the first existing locale folder from [`LANGUAGE_FOLDERS`].
///
/// Must be called after `chdir`-ing into the data directory but before any
/// resource files are loaded — `SbFile::open` consults the alternate paths
/// when the requested file is not at the primary location, so localized
/// `Data/...` files are picked up transparently.
fn add_language_folder() {
    // English fallback — always added in the international build, even if
    // the folder doesn't exist (the alt-path lookup is harmless when there's
    // no `1033/`).
    let _ = SbFile::add_alternate_path(FALLBACK_LOCALE_FOLDER);

    // Probe each candidate with `SbFile::exists` (which also walks already-
    // registered alternate paths) and stop at the first hit.
    for &folder in LANGUAGE_FOLDERS {
        if SbFile::exists(folder) {
            tracing::info!("Detected language folder: {folder}");
            let _ = SbFile::add_alternate_path(folder);
            return;
        }
    }
    tracing::info!(
        "No locale-specific language folder found; relying on '1033' fallback for localized resources"
    );
}

/// Set up the working directory so that `Data/` is accessible.
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn setup_data_dir() -> Result<(), String> {
    if let Ok(data_dir) = std::env::var("ROBINHOOD_DATA_DIR") {
        tracing::info!("ROBINHOOD_DATA_DIR set, using primary datadir {}", data_dir);
        let status = SbFile::set_primary_path(&data_dir);
        if status != SBFILE_NO_ERROR {
            return Err(format!(
                "Unable to install ROBINHOOD_DATA_DIR {data_dir}: SBFile error {status}"
            ));
        }
    } else if !Path::new("Data").is_dir()
        && let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        tracing::info!(
            "Using executable directory as primary datadir: {}",
            parent.display()
        );
        let status = SbFile::set_primary_path(&parent.to_string_lossy());
        if status != SBFILE_NO_ERROR {
            return Err(format!(
                "Unable to install executable directory {}: SBFile error {status}",
                parent.display()
            ));
        }
    } else {
        let status = SbFile::set_primary_path(".");
        if status != SBFILE_NO_ERROR {
            return Err(format!(
                "Unable to install current directory as datadir: SBFile error {status}"
            ));
        }
    }

    // Find the Data directory case-insensitively (some installs use "data", "DATA", etc.)
    if !SbFile::exists("Data") {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "?".into());
        return Err(format!(
            "ERROR: 'Data' directory not found in {}\n\
             Set ROBINHOOD_DATA_DIR=/path/to/game to the directory that\n\
             contains the game's Data/ folder.",
            cwd
        ));
    }

    add_overlay_data_dirs();
    add_language_folder();

    Ok(())
}

/// Android uses a pre-converted shipping datadir bundled as an APK
/// asset. If loose files are present (developer override), set the cwd
/// up the same way as desktop; otherwise rely on the installed
/// `ShippingDatadir` / `asset_fs` bundle.
#[cfg(target_os = "android")]
fn setup_data_dir() -> Result<(), String> {
    if let Ok(data_dir) = std::env::var("ROBINHOOD_DATA_DIR") {
        tracing::info!(
            "ROBINHOOD_DATA_DIR set, changing working directory to {}",
            data_dir
        );
        std::env::set_current_dir(&data_dir)
            .map_err(|e| format!("Unable to chdir to {}: {}", data_dir, e))?;
    }

    if crate::sbfile::resolve_case_insensitive(Path::new("Data")).is_none()
        && assets_shipping_datadir::global().is_none()
    {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "?".into());
        return Err(format!(
            "ERROR: neither APK asset Data/datadir.bin nor a loose Data directory was found in {cwd}"
        ));
    }

    add_language_folder();
    Ok(())
}

/// Wasm version: there is no cwd or directory enumeration.  The Data/
/// prefix is anchored at `ROBINHOOD_DATA_URL` (default `./data`), which
/// `crate::asset_fs` consults for every read.  All we do here is
/// bootstrap language-folder detection.
#[cfg(target_arch = "wasm32")]
fn setup_data_dir() -> Result<(), String> {
    add_language_folder();
    Ok(())
}

/// Result tuple for [`rust_init`] / [`rust_init_with_shipping`] /
/// [`rust_init_finish`]: the loaded campaign, mission profile manager, and
/// explicit application context (player profiles, key bindings, options,
/// and optional shipping data).
pub type RustInit = (
    Campaign,
    std::sync::Arc<engine_profiles::ProfileManager>,
    ApplicationContext,
);

/// Pure-Rust initialization: logging, data dir, profiles, campaign.
pub fn rust_init() -> Result<RustInit, String> {
    crate::init_tracing();
    setup_data_dir()?;
    tracing::info!("Robin Hood — Rust entry point");

    // Load the shipping datadir if one exists. When present, subsystem
    // loaders prefer it over legacy disk I/O.
    let shipping = assets_shipping_datadir::try_load(std::path::Path::new("Data"))
        .map_err(|e| format!("shipping datadir: {e:#}"))?
        .map(std::sync::Arc::new);
    if let Some(ref dd) = shipping {
        assets_shipping_datadir::install_global(dd.clone())
            .map_err(|error| format!("install shipping datadir: {error:#}"))?;
    }

    rust_init_finish(shipping)
}

/// Wasm variant of [`rust_init`] — the JS host has already decoded the
/// shipping datadir from the fetched `datadir.bin` bytes and installed
/// it via `install_global`, so we skip the
/// `try_load` step and reuse the supplied handle.
pub fn rust_init_with_shipping(
    shipping: Option<std::sync::Arc<assets_shipping_datadir::ShippingDatadir>>,
) -> Result<RustInit, String> {
    crate::init_tracing();
    setup_data_dir()?;
    tracing::info!("Robin Hood — Rust entry point (wasm boot)");
    rust_init_finish(shipping)
}

fn rust_init_finish(
    shipping: Option<std::sync::Arc<assets_shipping_datadir::ShippingDatadir>>,
) -> Result<RustInit, String> {
    let options = engine_api::GlobalOptions::default();
    let profiles = std::sync::Arc::new(load_profiles(shipping.as_deref(), &options)?);
    tracing::info!(
        "Rust profiles: {} chars, {} soldiers, {} missions, {} weapons",
        profiles.characters.len(),
        profiles.soldiers.len(),
        profiles.missions.len(),
        profiles.hth_weapons.len()
    );
    let player_profiles = load_player_profile_manager();
    let key_configs = load_key_config_store();

    let application_context = ApplicationContext::complete(
        options,
        player_profiles.clone(),
        key_configs.clone(),
        shipping,
    )?;

    // PARITY TODO(app-context): menu/options and level-load helpers outside
    // this slice still use their historical singleton APIs. Mirror the same
    // loaded values until those call sites can accept `ApplicationContext`;
    // gameplay difficulty and the host side-effects changed in this slice do
    // not read these mirrors.
    *PlayerProfileManager::global() = Some(player_profiles);
    *KeyConfigStore::global() = Some(key_configs);

    let campaign = Campaign::create(&profiles);

    Ok((campaign, profiles, application_context))
}

/// Load the character / soldier / mission profile pool.
///
/// Priority:
///   1. Pre-built `ProfileManager` carried by a shipping datadir.
///   2. JSON dump at `Data/Configuration/profile.cpf.json` (produced by
///      the `cpf_to_json` example).
///   3. Binary `.cpf` at `Data/Configuration/profile.cpf` parsed via the
///      legacy CPF reader.
fn load_profiles(
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
    options: &engine_api::GlobalOptions,
) -> Result<ProfileManager, String> {
    if let Some(dd) = shipping
        && let Some(p) = &dd.profiles
    {
        tracing::info!("Profiles: loaded from shipping datadir");
        // Shipping profiles are baked at `convert_datadir` time
        // (`convert_shipping` calls `import_beam_mes` before storing
        // `dd.profiles`), so the per-mission `number_of_beam_mes` /
        // `required_actions` fields are already populated — no
        // post-processing needed here.
        return Ok(p.clone());
    }
    // Both the JSON and legacy-CPF paths skip the beam-me post-processing
    // step, so without this call every mission profile ends up with
    // `number_of_beam_mes = 0` / `required_actions` empty — silently
    // hiding required-action glyphs in the briefing UI and breaking
    // auto-gang-selection.  Walk every mission `.rhm` file and fold
    // beam-me action flags into the profile.
    let level_dir = &options.level_directory;

    let json_path = "Data/Configuration/profile.cpf.json";
    if engine_sbfile::SbFile::exists(json_path) {
        tracing::info!("Profiles: loading JSON dump {json_path}");
        let mut mgr = ProfileManager::load_json(json_path)?;
        mgr.import_beam_mes(level_dir);
        return Ok(mgr);
    }
    let cpf_path = "Data/Configuration/profile.cpf";
    tracing::info!("Profiles: loading legacy CPF {cpf_path}");
    let mut file = engine_sbfile::SbFile::open(cpf_path, engine_sbfile::SB_FILE_READ)
        .map_err(|e| format!("Failed to open {cpf_path}: error {e}"))?;
    let mut mgr = ProfileManager::new();
    mgr.load_all_legacy_cpf(&mut file)
        .map_err(|e| format!("Failed to read profiles from {cpf_path}: error {e}"))?;
    mgr.import_beam_mes(level_dir);
    Ok(mgr)
}

/// Load the player-profile service owned by [`ApplicationContext`].
fn load_player_profile_manager() -> PlayerProfileManager {
    let save_dir = crate::save_file::default_save_directory();
    let save_dir_str = save_dir.to_string_lossy().into_owned();

    match PlayerProfileManager::load(&save_dir_str) {
        Ok(mgr) => mgr,
        Err(err) => {
            tracing::warn!(
                "Failed to load player profiles from {save_dir_str} ({err}); creating defaults"
            );
            let mut mgr = PlayerProfileManager::new(save_dir_str);
            let idx = mgr.create_profile("Robin".to_owned(), DifficultyLevel::Medium);
            mgr.set_active(idx);
            mgr
        }
    }
}

/// Load the key-config service owned by [`ApplicationContext`]. First-run
/// stores are intentionally empty; `ApplicationContext::complete` creates
/// the active profile's original-compatible default entry.
fn load_key_config_store() -> KeyConfigStore {
    let save_dir = crate::save_file::default_save_directory();
    let save_dir_str = save_dir.to_string_lossy().into_owned();

    KeyConfigStore::load(&save_dir_str).unwrap_or_else(|err| {
        tracing::warn!(
            "Failed to load key configs from {save_dir_str} ({err}); starting with empty store"
        );
        KeyConfigStore::new(save_dir_str)
    })
}

// ─── Game callbacks (pure-Rust path) ────────────────────────────────

/// Real implementation of [`GameCallbacks`](crate::game::GameCallbacks)
/// for the pure-Rust path.  Owns the [`SaveGameManager`] and serves as
/// the integration point between the Game state machine and persistent
/// storage.
///
/// Non-save callbacks are still stubs — they will be filled in as the
/// corresponding subsystems (menus, sound, debriefing) are ported.
///
/// ### Save/load semantics
///
/// The callback trait passes the `Campaign` but not the `Engine`, so
/// `serialize_save` / `serialize_load` only queue an intent here.
/// The actual file I/O — which needs live engine access — is performed
/// by [`crate::game_session::perform_pending_save_load`] before the next
/// engine tick, using [`crate::save_file::GameSaveFile`].
pub(crate) struct RustCallbacks {
    /// Explicit persistence context for entry-point-owned mission paths.
    /// `None` remains only for the untouched `game_session::run_session`
    /// compatibility constructor.
    application_context: Option<ApplicationContext>,
    /// Save-slot metadata manager, persists slot list as `saves.json`.
    pub save_manager: SaveGameManager,
    /// Pending save/load request queued by the state machine, handled
    /// before the next engine tick in `game_session`.
    pub pending: Option<SaveLoadRequest>,
    /// Cached "is loading requested" flag, queried by the debriefing UI.
    pub loading_requested: bool,
    /// Result returned by the debriefing UI; determines whether the
    /// post-mission flow transitions to LevelLoad.
    pub debriefing_code: GameCode,
    /// Ordered game-flow effects awaiting the host executor. A FIFO is
    /// required because one transition can request Menu and then Mission
    /// sound in the same frame; neither request may overwrite the other.
    pub app_effects: AppEffectQueue,
    /// Set by `perform_pending_save_load` after a successful Load so the
    /// frame loop can call `Game::apply_post_load_sync`.
    pub post_load_sync: Option<PostLoadSync>,
    /// Pending in-game banner queued by `perform_pending_save_load`
    /// after a successful save or load. Consumed by the frame loop,
    /// which copies the text onto the live `Game::message_text` /
    /// `message_delay` fields.
    pub pending_save_banner: Option<SaveBannerKind>,
    /// Pending request to re-forward an input-reset after a load.
    /// Consumed by the frame loop, which clears the input translator's
    /// key-edge state so half-pressed keys at save time do not stick
    /// across the load.
    pub pending_reset_input: bool,
    /// Cross-mission load request stashed by `perform_pending_save_load`
    /// when the selected save's header mission differs from the mission
    /// currently running. The frame loop forces `GameCode::LevelLoad` on
    /// the active `Game` so `run_mission` exits; the outer session loop
    /// then switches the campaign's current mission to `target_mission_id`
    /// and re-queues a `SaveLoadRequest::Load` on the fresh engine so the
    /// first frame of the new mission applies the save.
    pub pending_level_load: Option<PendingLevelLoad>,
}

/// Save-slot bookkeeping passed from an in-mission "load" click through
/// the `GameCode::LevelLoad` exit back into the outer session loop.
#[derive(Debug, Clone, Copy)]
pub struct PendingLevelLoad {
    /// Save-slot index in [`crate::savegame::SaveGameManager`].
    pub slot: usize,
    /// Mission profile ID the save's header reports.
    pub target_mission_id: u32,
}

/// Slot-type-dependent post-load state the frame loop must apply once
/// the engine has been loaded. Thread the slot type through so we can
/// replay the continue / campaign-map fix-ups without the save-I/O layer
/// needing a `&mut Game` handle.
#[derive(Debug, Clone, Copy)]
pub struct PostLoadSync {
    /// True when the loaded slot is the Continue auto-save.
    pub is_continue: bool,
}

/// Which banner to show after a save/load succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveBannerKind {
    Saved,
    Loaded,
}

/// Pending save/load intent set by the state machine and consumed
/// outside the callback boundary by `game_session`.
#[derive(Debug, Clone)]
pub enum SaveLoadRequest {
    /// Persist the current engine state to the caller-provided slot.
    /// `None` slot = write the Continue auto-save.
    Save {
        slot: Option<usize>,
        mission_id: u32,
    },
    /// Load a save and apply it to the engine.
    /// `None` slot = load the Continue auto-save.
    ///
    /// `mission_id` is the mission the caller expected the save to match.
    /// The load path reads the on-disk header and compares it against
    /// this field; a mismatch is logged at warn level (the save/load
    /// menu can cross missions at the cost of a campaign-map round-trip,
    /// rather than refusing the load outright).
    Load {
        slot: Option<usize>,
        mission_id: u32,
    },
    /// Write the Restart auto-save (pre-restart snapshot), called once
    /// per mission right after level init finishes. The live campaign
    /// supplies the required mission ID when the request is processed.
    Restart,
    /// Apply the Restart auto-save to the engine, restoring the
    /// pristine post-level-init state without reloading the level.
    /// Called when the op code transitions to a level-restart
    /// (typically from a script command).
    LoadRestart,
    /// Write the Continue auto-save (quit / end-of-mission flow).
    Continue { mission_id: u32 },
    /// Write the QuickSave auto-save (F5 hotkey).
    /// Rotates the previous QuickSave to ExQuickSave and then writes
    /// the fresh snapshot.
    QuickSave { mission_id: u32 },
    /// Load the QuickSave auto-save (F12 hotkey).
    ///
    /// `use_backup` selects the previous (ExQuickSave) slot when `Shift`
    /// is held at keypress time. Without Shift, loads the newest
    /// QuickSave.
    QuickLoad { use_backup: bool },
    /// Write the Sherwood checkpoint save (transition out of Sherwood map).
    Sherwood { mission_id: u32 },
}

impl RustCallbacks {
    pub fn new() -> Self {
        Self {
            application_context: None,
            save_manager: SaveGameManager::open_default(),
            pending: None,
            loading_requested: false,
            debriefing_code: GameCode::LevelInProgress,
            app_effects: AppEffectQueue::default(),
            post_load_sync: None,
            pending_level_load: None,
            pending_save_banner: None,
            pending_reset_input: false,
        }
    }

    pub fn with_application_context(application_context: ApplicationContext) -> Self {
        Self {
            application_context: Some(application_context),
            ..Self::new()
        }
    }
}

impl SaveLoadRequest {
    /// Whether this request writes a save payload and should get a fresh
    /// thumbnail from a rendered frame.
    pub(crate) fn writes_save_payload(&self) -> bool {
        matches!(
            self,
            SaveLoadRequest::Save { .. }
                | SaveLoadRequest::Restart
                | SaveLoadRequest::Continue { .. }
                | SaveLoadRequest::QuickSave { .. }
                | SaveLoadRequest::Sherwood { .. }
        )
    }
}

impl crate::game::GameCallbacks for RustCallbacks {
    fn serialize_save(&mut self, campaign: &Campaign, profiles: &engine_profiles::ProfileManager) {
        let mission_id = current_mission_id(campaign, profiles);
        self.pending = Some(SaveLoadRequest::Save {
            slot: None,
            mission_id,
        });
    }
    fn serialize_load(&mut self, mission_id: u32) {
        self.pending = Some(SaveLoadRequest::Load {
            slot: None,
            mission_id,
        });
    }
    fn serialize_for_restart(&mut self, write: bool) {
        self.pending = Some(if write {
            SaveLoadRequest::Restart
        } else {
            SaveLoadRequest::LoadRestart
        });
    }
    fn serialize_continue_save(&mut self, mission_id: u32) {
        self.pending = Some(SaveLoadRequest::Continue { mission_id });
    }
    fn save_profiles(&mut self) {
        if let Some(context) = self.application_context.as_ref() {
            match context.with_player_profiles_mut(|mgr| mgr.save()) {
                Ok(Ok(())) => {}
                Ok(Err(err)) => tracing::error!("save_profiles failed: {err}"),
                Err(error) => panic!("save_profiles lost its ApplicationContext: {error}"),
            }
            return;
        }

        // PARITY TODO(app-context): `game_session::run_session` still creates
        // callbacks without accepting the explicit context. Remove this
        // compatibility branch when that excluded module is migrated.
        // Persist the currently loaded profile manager to
        // `<save_dir>/profiles.json` on quit.
        let guard = PlayerProfileManager::global();
        if let Some(ref mgr) = *guard {
            if let Err(err) = mgr.save() {
                tracing::error!("save_profiles failed: {err}");
            }
        } else {
            tracing::warn!("save_profiles: no global profile manager to save");
        }
    }
    fn synchronize_profile_with_campaign(
        &mut self,
        campaign: &Campaign,
        profiles: &engine_profiles::ProfileManager,
    ) {
        if let Some(context) = self.application_context.as_ref() {
            let mission_secs = self.get_current_playing_time(campaign);
            context
                .with_player_profiles_mut(|manager| {
                    let profile = manager
                        .get_active_mut()
                        .expect("ApplicationContext lost its required active player profile");
                    profile.score =
                        campaign.get_value(engine_campaign::CampaignValue::Score) as u32;
                    profile.ransom =
                        campaign.get_value(engine_campaign::CampaignValue::Ransom) as u32;
                    profile.progression = campaign.get_progression(profiles);
                    profile.play_time += mission_secs;

                    let dead =
                        campaign.get_value(engine_campaign::CampaignValue::DeadSoldiers) as u32;
                    let alive =
                        campaign.get_value(engine_campaign::CampaignValue::LivingSoldiers) as u32;
                    profile.preserved_lives = if dead != 0 || alive != 0 {
                        (100.0 * alive as f32 / (dead + alive) as f32) as u32
                    } else {
                        0
                    };
                })
                .unwrap_or_else(|error| {
                    panic!("profile synchronization lost its ApplicationContext: {error}")
                });
            return;
        }

        // PARITY TODO(app-context): compatibility for the still-global
        // callbacks constructed inside excluded `game_session::run_session`.
        // Copy end-of-mission campaign values (score, ransom, play time,
        // dead/alive soldiers → preserved_lives ratio) into the active
        // profile.
        //
        // We lock the global manager twice — once to snapshot the active
        // index, then drop the lock before `synchronize_with_campaign`
        // re-locks internally.
        let active_idx = {
            let guard = PlayerProfileManager::global();
            guard.as_ref().and_then(|m| m.active_index)
        };
        if let Some(idx) = active_idx {
            // Route through `get_current_playing_time` so profile sync
            // stays coupled to the mission clock abstraction. The clock
            // is deterministic now: `MissionLength` advances from sim
            // frames while the mission is actually ticking.
            let mission_secs = self.get_current_playing_time(campaign);
            crate::player_profile::synchronize_with_campaign(idx, campaign, profiles, mission_secs);
        }
    }
    fn save_game_file_exists(&self) -> bool {
        self.save_manager
            .find_by_filename(special_slots::CONTINUE)
            .map(|idx| self.save_manager.slot_file_exists(idx))
            .unwrap_or(false)
    }
    fn save_game_mission_id(&self) -> u32 {
        let idx = self
            .save_manager
            .find_by_filename(special_slots::CONTINUE)
            .expect("save_game_mission_id: Continue slot must exist after file-exists check");
        required_mission_id(
            self.save_manager.slot_mission_id(idx),
            "save_game_mission_id: Continue slot must have a cached mission ID",
        )
    }
    fn emit_app_effect(&mut self, effect: AppEffect) {
        self.app_effects.push(effect);
    }
    fn send_script_message(&mut self, target: u32, message: u32) {
        tracing::debug!("Script message: target={} msg={}", target, message);
    }
    fn display_ingame_menu(&mut self) {
        tracing::warn!("display_ingame_menu: stub");
    }
    fn display_debriefing(&mut self, won: bool) {
        tracing::info!("Debriefing (won={}): stub", won);
    }
    fn is_loading_requested(&self) -> bool {
        self.loading_requested
    }
    fn get_debriefing_game_code(&self) -> GameCode {
        self.debriefing_code
    }
    fn start_play_time(&mut self) {
        // Original C++ used GetTickCount() here. In the Rust port the
        // campaign is rollback-hashed, so mission length advances from
        // deterministic 25 Hz engine frames instead.
    }
    fn suspend_play_time(&mut self) {
        // See `start_play_time`: pausing or ending a mission stops
        // engine ticks, which already stops mission-length advancement.
    }
    fn get_current_playing_time(&self, campaign: &Campaign) -> u32 {
        // Returns deterministic simulation seconds. Downstream consumers
        // (debriefing mission-length, profile `play_time` sync) all see
        // the campaign-owned counter.
        campaign
            .get_value(engine_campaign::CampaignValue::MissionLength)
            .max(0) as u32
    }
}

/// Require a real mission ID from state that has already been established.
///
/// Original: `original-code/RHgame.cpp:1644-1646` and `:3215` directly
/// dereference the campaign's current mission/profile when comparing or
/// writing saves; zero is not substituted for missing required state.
pub(crate) fn required_mission_id(mission_id: Option<u32>, context: &str) -> u32 {
    let mission_id = mission_id.unwrap_or_else(|| panic!("{context}"));
    assert_ne!(mission_id, 0, "{context}: mission ID zero is invalid");
    mission_id
}

/// Resolve the required mission profile ID of the campaign's current mission.
pub(crate) fn current_mission_id(
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
) -> u32 {
    required_mission_id(
        campaign
            .current_mission_idx
            .and_then(|idx| campaign.missions.get(idx))
            .map(|m| m.profile(profiles).id),
        "current_mission_id: campaign must have a valid current mission",
    )
}

/// Execute queued game-flow effects against the narrow host facilities they
/// are allowed to mutate: sound playback and mouse-event acceptance.
///
/// Call after `game.process_operation` and before any early return from code
/// that emitted an effect. The callback boundary cannot touch these host
/// resources directly because the frame loop owns them.
pub(crate) fn execute_app_effects(
    effects: &mut AppEffectQueue,
    sound: &mut crate::sound::SoundManager,
    threaded_input: &mut crate::input::ThreadedInput,
    mut audio_backend: Option<&mut dyn crate::sound::AudioBackend>,
) {
    let result: Result<(), AppEffectExecutionError<std::convert::Infallible>> = effects
        .try_execute(|effect| {
            match effect {
                AppEffect::SetSoundMode(mode) => {
                    // `None` is an explicit audio-disabled state established
                    // by setup (`-NOSOUND`, headless, or a logged init
                    // failure), so acknowledging sound-only effects here is
                    // intentional rather than a fabricated fallback value.
                    if let Some(backend) = audio_backend.as_deref_mut() {
                        let sound_mode = match mode {
                            crate::app_effect::SoundMode::Menu => AudioSoundMode::Menu,
                            crate::app_effect::SoundMode::Mission => AudioSoundMode::Mission,
                        };
                        sound.set_mode(sound_mode, backend);
                    }
                }
                AppEffect::PlayJingle(jingle) => {
                    if let Some(backend) = audio_backend.as_deref_mut() {
                        let sound_jingle = match jingle {
                            crate::app_effect::Jingle::MissionWon => SoundJingle::MissionWon,
                            crate::app_effect::Jingle::MissionLost => SoundJingle::MissionLost,
                        };
                        sound.play_jingle(sound_jingle, backend);
                    }
                }
                AppEffect::SetMouseEnabled(enabled) => {
                    // Disable the input pump's mouse-event branch during
                    // cinematics / mission briefings / movie playback so
                    // motion and clicks do not leak to the game.
                    threaded_input.set_enabled(enabled);
                }
            }
            Ok(())
        });

    if let Err(AppEffectExecutionError::Effect { source, .. }) = result {
        match source {}
    }
}

/// Flush any pending save/load request queued by the Game state machine.
///
/// Called from `game_session` between `game.process_operation` and the
/// next engine tick.  This is where the actual disk I/O happens, with
/// live access to both engine and campaign.
///
/// `thumbnail` is the captured screen preview written alongside save
/// slots, right after the main save payload.  Callers should render a
/// dedicated throwaway frame and read it back, mirroring the HTTP
/// screenshot path, so thumbnail contents never depend on stale render
/// target state.  If the capture failed or the caller has no renderer
/// handy, pass `None` and the save is written without a thumbnail.
pub(crate) fn perform_pending_save_load(
    host: &mut crate::Host,
    game: &mut crate::game::Game,
    callbacks: &mut RustCallbacks,
    engine: &mut engine_api::Engine,
    assets: &engine_api::LevelAssets,
    profiles: &engine_profiles::ProfileManager,
    thumbnail: Option<crate::save_file::Thumbnail>,
) -> bool {
    let Some(request) = callbacks.pending.take() else {
        return false;
    };
    let thumb_ref = thumbnail.as_ref();
    match request {
        SaveLoadRequest::Save { slot, mission_id } => {
            // `slot = None` ⇒ auto Continue-save.
            // `slot = Some(idx)` ⇒ player-chosen slot.
            let (result, explicit_slot) = match slot {
                Some(idx) => (
                    callbacks
                        .save_manager
                        .write_save_from_engine(
                            host,
                            game,
                            idx,
                            engine,
                            mission_id,
                            Some(profiles),
                            thumb_ref,
                        )
                        .and_then(|()| {
                            callbacks
                                .save_manager
                                .save_index()
                                .map_err(|e| anyhow::anyhow!(e))
                        }),
                    true,
                ),
                None => (
                    callbacks.save_manager.write_continue_save(
                        host,
                        game,
                        engine,
                        mission_id,
                        Some(profiles),
                        thumb_ref,
                    ),
                    false,
                ),
            };
            if let Err(err) = result {
                tracing::error!("Save failed: {err:#}");
            } else {
                tracing::info!("Save completed (mission={mission_id})");
                // Mirror the manual save into the Continue slot. The
                // guard keeps Continue→Continue copies from clobbering
                // themselves; Restart / Sherwood slots also skip the
                // mirror and the banner branch.
                if explicit_slot {
                    let is_special = slot
                        .and_then(|idx| callbacks.save_manager.get(idx))
                        .and_then(|s| s.special);
                    let is_continue_or_restart = matches!(
                        is_special,
                        Some(SpecialSlot::Continue) | Some(SpecialSlot::Restart)
                    );
                    if !is_continue_or_restart
                        && let Err(err) = callbacks.save_manager.write_continue_save(
                            host,
                            game,
                            engine,
                            mission_id,
                            Some(profiles),
                            thumb_ref,
                        )
                    {
                        tracing::warn!("Continue-mirror after save failed: {err:#}");
                    }
                    // Show "Game saved." banner unless the slot is one
                    // of the filtered types (Restart / Sherwood).
                    let is_sherwood = matches!(is_special, Some(SpecialSlot::Sherwood));
                    if !is_continue_or_restart && !is_sherwood {
                        callbacks.pending_save_banner = Some(SaveBannerKind::Saved);
                    }
                }
            }
        }
        SaveLoadRequest::Load { slot, mission_id } => {
            // If the save targets a different mission than the one currently
            // running, stash a `PendingLevelLoad` and let the session loop
            // switch missions before re-applying. This replaces the previous
            // warn-and-apply behaviour, which corrupted engine state when
            // the payload's mission didn't match the active level.
            let target = callbacks.save_manager.find_load_target(slot);
            match target {
                Some(idx) => {
                    if mission_id != 0 {
                        let target_mission_id = callbacks.save_manager.slot_mission_id(idx);
                        if let Some(target_mission_id) = target_mission_id
                            && target_mission_id != mission_id
                        {
                            tracing::info!(
                                "Load slot {idx}: cross-mission load (header={}, current={}) — \
                                 routing through session LevelLoad",
                                target_mission_id,
                                mission_id,
                            );
                            callbacks.pending_level_load = Some(PendingLevelLoad {
                                slot: idx,
                                target_mission_id,
                            });
                            return true;
                        }
                    }
                    match callbacks
                        .save_manager
                        .load_save_into_engine(idx, engine, host, game, assets)
                    {
                        Err(err) => {
                            tracing::error!("Load failed: {err:#}");
                        }
                        _ => {
                            // Thread the slot type through so the frame loop
                            // can replay the continue / campaign-map fix-ups.
                            let is_continue = callbacks
                                .save_manager
                                .get(idx)
                                .map(|s| s.is_continue())
                                .unwrap_or(false);
                            let is_restart = callbacks
                                .save_manager
                                .get(idx)
                                .map(|s| s.is_restart())
                                .unwrap_or(false);
                            let is_sherwood = callbacks
                                .save_manager
                                .get(idx)
                                .map(|s| s.is_sherwood())
                                .unwrap_or(false);
                            callbacks.post_load_sync = Some(PostLoadSync { is_continue });
                            // The frame loop clears the translator's
                            // key-edge state so half-pressed keys at save
                            // time don't stick across the load.
                            callbacks.pending_reset_input = true;
                            // Mirror the load into the Continue slot,
                            // guarded by IsContinue/IsRestart so we
                            // don't clobber the slot we just loaded.
                            let mid = required_mission_id(
                                callbacks.save_manager.slot_mission_id(idx),
                                "loaded save slot must retain its cached mission ID",
                            );
                            if !is_continue && !is_restart {
                                callbacks.save_manager.write_continue_save_background(
                                    host,
                                    game,
                                    engine,
                                    mid,
                                    Some(profiles),
                                    thumb_ref,
                                );
                            }
                            // Show "Game loaded." banner unless the slot
                            // is Restart / Sherwood.
                            if !is_restart && !is_sherwood {
                                callbacks.pending_save_banner = Some(SaveBannerKind::Loaded);
                            }
                            tracing::info!("Load completed from slot {idx}");
                        }
                    }
                }
                None => {
                    tracing::warn!("Load requested but no matching save slot found");
                }
            }
        }
        SaveLoadRequest::Restart => {
            let campaign = engine
                .campaign()
                .expect("Restart save requires the engine campaign");
            let mid = current_mission_id(campaign, profiles);
            if let Err(err) = callbacks.save_manager.write_restart_save(
                host,
                game,
                engine,
                mid,
                Some(profiles),
                thumb_ref,
            ) {
                tracing::error!("Restart save failed: {err:#}");
            }
        }
        SaveLoadRequest::LoadRestart => {
            match callbacks
                .save_manager
                .load_restart_save(host, game, engine, assets)
            {
                Ok(true) => {
                    // Restart = never Continue slot; still sync campaign-map state.
                    callbacks.post_load_sync = Some(PostLoadSync { is_continue: false });
                    tracing::info!("Restart snapshot restored");
                }
                Ok(false) => {
                    // No restart save on disk — the caller should fall
                    // back to reinitializing the mission from scratch.
                    // Silent no-op when the restart snapshot is missing.
                    tracing::warn!("LoadRestart requested but no restart snapshot exists");
                }
                Err(err) => tracing::error!("Restart load failed: {err:#}"),
            }
        }
        SaveLoadRequest::Continue { mission_id } => {
            if let Err(err) = callbacks.save_manager.write_continue_save(
                host,
                game,
                engine,
                mission_id,
                Some(profiles),
                thumb_ref,
            ) {
                tracing::error!("Continue save failed: {err:#}");
            }
        }
        SaveLoadRequest::QuickSave { mission_id } => {
            match callbacks.save_manager.write_quick_save(
                host,
                game,
                engine,
                mission_id,
                Some(profiles),
                thumb_ref,
            ) {
                Err(err) => {
                    tracing::error!("Quick save failed: {err:#}");
                }
                _ => {
                    tracing::info!("Quick save written (mission={mission_id})");
                    // QuickSave is neither Continue nor Restart, so the
                    // Continue-slot mirror runs.
                    if let Err(err) = callbacks.save_manager.write_continue_save(
                        host,
                        game,
                        engine,
                        mission_id,
                        Some(profiles),
                        thumb_ref,
                    ) {
                        tracing::warn!("Continue-mirror after quick-save failed: {err:#}");
                    }
                    callbacks.pending_save_banner = Some(SaveBannerKind::Saved);
                }
            }
        }
        SaveLoadRequest::QuickLoad { use_backup } => {
            // Shift+F12 loads `ExQuickSave` (the backup).
            // Plain F12 loads `QuickSave`.
            let slot_name = if use_backup {
                special_slots::EX_QUICK
            } else {
                special_slots::QUICK
            };
            let idx = callbacks.save_manager.find_by_filename(slot_name);
            match idx {
                Some(i) if callbacks.save_manager.slot_file_exists(i) => {
                    match callbacks
                        .save_manager
                        .load_save_into_engine(i, engine, host, game, assets)
                    {
                        Err(err) => {
                            tracing::error!("Quick load ({slot_name}) failed: {err:#}");
                        }
                        _ => {
                            // QuickSave is not the Continue slot; just re-sync
                            // campaign-map state.
                            callbacks.post_load_sync = Some(PostLoadSync { is_continue: false });
                            callbacks.pending_reset_input = true;
                            // Mirror into the Continue slot — QuickSave is
                            // neither Continue nor Restart so it always
                            // mirrors.
                            let mid = required_mission_id(
                                callbacks.save_manager.slot_mission_id(i),
                                "loaded QuickSave slot must retain its cached mission ID",
                            );
                            callbacks.save_manager.write_continue_save_background(
                                host,
                                game,
                                engine,
                                mid,
                                Some(profiles),
                                thumb_ref,
                            );
                            callbacks.pending_save_banner = Some(SaveBannerKind::Loaded);
                            tracing::info!("Quick save loaded from {slot_name}");
                        }
                    }
                }
                _ => tracing::warn!("Quick load requested but no {slot_name} save on disk"),
            }
        }
        SaveLoadRequest::Sherwood { mission_id } => {
            match callbacks.save_manager.write_sherwood_save(
                host,
                game,
                engine,
                mission_id,
                Some(profiles),
                thumb_ref,
            ) {
                Err(err) => {
                    tracing::error!("Sherwood checkpoint save failed: {err:#}");
                }
                _ => {
                    tracing::info!("Sherwood checkpoint saved (mission={mission_id})");
                }
            }
        }
    }
    true
}

// ─── Resource helpers ───────────────────────────────────────────────

/// Upload a 16-bit RGB565 Picture into a new renderer surface.
pub(crate) fn picture_to_surface(renderer: &mut Renderer, pic: &Picture) -> u32 {
    let pixels: Vec<u16> = pic
        .data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    renderer
        .create_surface_from_rgb565(pic.width, pic.height, &pixels)
        .expect("picture_to_surface: decoded picture dimensions must match RGB565 payload")
}

// ─── Top-level entry ────────────────────────────────────────────────

/// Detect demo mode at runtime by checking for demo mission files.
/// Returns `(mission_name, proto_name, pc_string, location)` if a demo is detected.
pub(crate) fn detect_demo_mode_with_context(
    application_context: &ApplicationContext,
) -> Option<(&'static str, &'static str, &'static str, MissionLocation)> {
    let resolve = SbFile::exists;
    let shipping_has_level = |mission: &str| {
        application_context
            .shipping()
            .expect("demo detection requires an initialized ApplicationContext")
            .is_some_and(|dd| dd.levels.contains_key(mission))
    };
    if resolve("Data/Levels/Dem_Lei_MP.rhm") || shipping_has_level("Dem_Lei_MP") {
        // Leicester demo — R=Robin, J=Jean, M=Marianne, T=Tuck, F=Ferris.
        Some((
            "Dem_Lei_MP",
            "Leicester",
            "RJMTF",
            MissionLocation::Leicester,
        ))
    } else if resolve("Data/Levels/Demo_Lin.rhm") || shipping_has_level("Demo_Lin") {
        // Lincoln demo — R=Robin, S=Stutely, A/B/C=Peasants
        Some(("Demo_Lin", "Lincoln", "RSABC", MissionLocation::Lincoln))
    } else {
        None
    }
}

/// Compatibility shim for the excluded `game_session` setup path.
///
/// PARITY TODO(app-context): pass `ApplicationContext` into `run_mission`
/// setup and delete this remaining shipping-global read.
pub(crate) fn detect_demo_mode()
-> Option<(&'static str, &'static str, &'static str, MissionLocation)> {
    let resolve = SbFile::exists;
    let shipping_has_level = |mission: &str| {
        assets_shipping_datadir::global().is_some_and(|dd| dd.levels.contains_key(mission))
    };
    if resolve("Data/Levels/Dem_Lei_MP.rhm") || shipping_has_level("Dem_Lei_MP") {
        Some((
            "Dem_Lei_MP",
            "Leicester",
            "RJMTF",
            MissionLocation::Leicester,
        ))
    } else if resolve("Data/Levels/Demo_Lin.rhm") || shipping_has_level("Demo_Lin") {
        Some(("Demo_Lin", "Lincoln", "RSABC", MissionLocation::Lincoln))
    } else {
        None
    }
}

/// Resolve the loading screen `.pak` file path.
///
/// First probes `Data/Levels/<ambience:%02u>/<proto_level_filename>.pak`,
/// falling back to `Data/Interface/Loading.pak` when the per-ambience file
/// is missing. Returns `None` when neither exists.
///
/// `proto_level_filename` comes from the mission's profile. The caller
/// threads it from `campaign_ref.missions[mission_idx].profile(..)`.
///
/// `ambience` is the raw ambience bitmask (1=Day, 2=Fog, 4=Night) read
/// from the `.rhm` header. The loading screen is shown *before* opening
/// the mission file, so the precise ambience isn't known yet — pass
/// `None` and we probe each candidate (`01`, `02`, `04`) in turn. Only
/// one ambience pak ever ships per mission, so the probe degenerates to
/// the same answer as an exact lookup.
pub(crate) fn resolve_loading_pak(
    proto_level_filename: Option<&str>,
    ambience: Option<u32>,
) -> Option<String> {
    fn data_asset_exists(path: &str) -> bool {
        crate::sbfile::resolve_data_path(path).is_some() || robin_util::asset_fs::exists(path)
    }

    if let Some(proto) = proto_level_filename {
        // Day=1, Fog=2, Night=4. Probe all three when the caller doesn't
        // have the exact ambience yet; only one mission-specific pak
        // exists per mission, so the result matches an exact lookup
        // either way.
        let single = ambience.map(|a| [a]);
        let candidates: &[u32] = match single.as_ref() {
            Some(arr) => arr,
            None => &[1, 2, 4],
        };
        for &amb in candidates {
            let candidate = format!("Data/Levels/{:02}/{}.pak", amb, proto);
            if data_asset_exists(&candidate) {
                tracing::info!("Loading screen .pak: using mission-specific {candidate}");
                return Some(candidate);
            }
        }
    }
    let default_path = "Data/Interface/Loading.pak";
    if data_asset_exists(default_path)
        || assets_shipping_datadir::global()
            .is_some_and(|dd| dd.pak_files.contains_key("interface/loading.pak"))
    {
        Some(default_path.to_string())
    } else {
        tracing::info!("Loading screen .pak not found at {}", default_path);
        None
    }
}

fn force_mission_launch(
    campaign: &mut Campaign,
    profiles: &mut std::sync::Arc<ProfileManager>,
    args: &CliArgs,
) -> Result<Option<(usize, MissionLocation)>, String> {
    let Some(mission_name) = args.mission.as_deref() else {
        return Ok(None);
    };
    let proto_name = args.proto.as_deref().unwrap_or(mission_name);

    tracing::info!("--mission: launching `{mission_name}` with proto-level `{proto_name}`");

    let profiles_mut = std::sync::Arc::make_mut(profiles);
    campaign.reset(profiles_mut);
    let idx = campaign
        .force_next_mission_by_name(profiles_mut, mission_name, proto_name, true)
        .ok_or_else(|| {
            format!("--mission: failed to force mission `{mission_name}` with proto `{proto_name}`")
        })?;
    campaign.current_mission_idx = Some(idx);
    let location = campaign.missions[idx].profile(profiles_mut).location;

    Ok(Some((idx, location)))
}

/// Synchronous compatibility boundary for main-menu modules outside this
/// slice. All guards are dropped before the caller enters an async menu.
fn mirror_context_to_legacy_stores(context: &ApplicationContext) -> Result<(), String> {
    let (profiles, keys) = context.legacy_service_snapshots()?;
    *PlayerProfileManager::global() = Some(profiles);
    *KeyConfigStore::global() = Some(keys);
    Ok(())
}

/// Adopt profile/key edits made by the legacy main menu after its future has
/// completed. Snapshot each singleton separately; no guard crosses an await.
fn adopt_legacy_stores_into_context(context: &ApplicationContext) -> Result<(), String> {
    let profiles = PlayerProfileManager::global()
        .as_ref()
        .cloned()
        .ok_or_else(|| "legacy main menu removed the player-profile store".to_string())?;
    let keys = KeyConfigStore::global()
        .as_ref()
        .cloned()
        .ok_or_else(|| "legacy main menu removed the key-config store".to_string())?;
    context.replace_legacy_service_snapshots(profiles, keys)
}

/// Run the game loop: main menu -> mission selection -> game -> repeat.
///
/// Outer loop: main menu (Start/Exit) -> campaign map -> game loop ->
/// back to menu.
pub async fn run_rust_game(
    window: &mut GameWindow,
    mut campaign: Campaign,
    mut profiles: std::sync::Arc<engine_profiles::ProfileManager>,
    application_context: ApplicationContext,
    args: &CliArgs,
) -> Result<i32, String> {
    // Combine parsed launcher options with the services loaded by `rust_init`.
    // Every lock-backed value used below is copied into an owned snapshot
    // before the first `.await`; futures never retain a profile/key guard.
    let application_context =
        application_context.with_options(args.global_options.options().clone());
    let shipping = application_context.shipping()?;
    let mut run_args = args.clone();
    run_args.global_options = application_context.clone();
    let args = &run_args;

    // Bring up the script-RPC transport. Native binds a loopback HTTP
    // listener; wasm installs the in-process JS bridge queue. The
    // handle lives in a process-global so the per-tick drain in
    // `game_session` can reach it without threading the queue through
    // every signature.
    crate::http_server::start_global(args.http_server)?;

    // `--headless` previously routed SDL through its dummy video/audio
    // driver via env vars set before `sdl3::init`.  With the wgpu/winit
    // port the SDL boot path is gone; the headless code in
    // `game_session` already short-circuits the per-frame render block,
    // so this is now a no-op flag.
    if args.headless {
        tracing::info!("--headless: rendering disabled in game_session");
    }

    // The window/GPU was constructed by `crate::window::run_with_game`
    // and handed in as `window: &mut GameWindow`.  Just stamp the
    // logical render size so cursor/mouse events get back-transformed
    // through the present-time letterbox into logical coords.
    window.set_logical_size(window.width, window.height);

    // Env-var equivalent of `--wait-for-command`, used by the wasm
    // host — `Module.arguments` doesn't reach `std::env::args()` on
    // the `-sPROXY_TO_PTHREAD` worker, but `preRun`-set env vars do.
    let wait_for_command =
        args.wait_for_command || std::env::var_os("ROBIN_WAIT_FOR_COMMAND").is_some();

    // ── `--wait-for-command`: idle until a replay arrives via RPC ──
    // Data is fully loaded at this point (`rust_init` ran before
    // `run_rust_game`), so we just spin on the pending-replay slot
    // while pumping SDL events.  When a replay lands, its header
    // picks the mission, then we move the decoded replay into
    // `CliArgs::replay_data` before `run_mission` so engine
    // construction can use the recording's RNG seed. Skips every
    // auto-start branch below (demo / sherwood / --replay / menu) by
    // design — the whole point is to let the JS side drive mission
    // selection without racing a hard-coded default.
    if wait_for_command {
        tracing::info!("--wait-for-command: data loaded, idling until load-replay RPC arrives");
        let mission_id = wait_for_replay_command(window).await;
        // Lookup order:
        //   1. real `.rhm` filename match (new recordings)
        //   2. legacy `mission_{N}` placeholder — use N as the index
        //      directly.  Older replays (pre-rust-merge) stamped the
        //      raw index into the header, and we'd rather load them
        //      than demand users re-record.
        let idx = campaign
            .missions
            .iter()
            .position(|m| m.profile(&profiles).mission_filename == mission_id)
            .or_else(|| {
                mission_id
                    .strip_prefix("mission_")
                    .and_then(|n| n.parse::<usize>().ok())
                    .filter(|&n| n < campaign.missions.len())
            });
        let Some(idx) = idx else {
            // Drop the queued replay so a subsequent `--wait-for-command`
            // loop (if any caller reuses the process) doesn't pick
            // up a stale mission reference.
            let _ = crate::http_server::take_pending_replay();
            return Err(format!(
                "--wait-for-command: replay references mission '{mission_id}' \
                 which is not in the campaign"
            ));
        };
        let location = campaign.missions[idx].profile(&profiles).location;
        tracing::info!(
            "--wait-for-command: launching mission `{mission_id}` (idx={idx}) from queued replay"
        );
        // Align exactly with the demo-mode auto-start: reset, build
        // the gang from the demo's PC roster, add-all-to-team, stamp
        // `current_mission_idx`.  Deviating from this (eg. calling
        // `force_next_mission` which also sets `next_mission_idx`)
        // leaves the campaign in a different state than a fresh
        // record session produces, causing every tick to `state_hash`
        // desync at frame 0 vs the recording.
        campaign.reset(&profiles);
        if let Some((_, _, pcs, _)) = detect_demo_mode_with_context(&application_context) {
            campaign.create_gang_from_pcs(pcs, &profiles);
        }
        campaign.add_all_to_mission_team();
        campaign.current_mission_idx = Some(idx);
        let Some(pending) = crate::http_server::take_pending_replay() else {
            return Err("--wait-for-command: replay disappeared before mission start".into());
        };
        let mut replay_args = args.clone();
        replay_args.replay_data = Some(pending.data);
        replay_args.start_paused = replay_args.start_paused || pending.paused;
        let mut callbacks = RustCallbacks::with_application_context(application_context.clone());
        run_mission(
            window,
            &mut callbacks,
            &mut campaign,
            &profiles,
            idx,
            location,
            &replay_args,
        )
        .await?;
        return Ok(0);
    }

    // ── `--mission`: original-launcher style direct mission forcing. ──
    // Mirrors `-MISSION foo [-PROTO bar]`: select an existing profile
    // when present, otherwise append a synthetic profile and launch it.
    if let Some((idx, location)) = force_mission_launch(&mut campaign, &mut profiles, args)? {
        let mut callbacks = RustCallbacks::with_application_context(application_context.clone());
        run_mission(
            window,
            &mut callbacks,
            &mut campaign,
            &profiles,
            idx,
            location,
            args,
        )
        .await?;
        return Ok(0);
    }

    // Demo detection: check which demo data files exist.
    let demo_config = if args.force_main_menu {
        tracing::info!("--force-main-menu: skipping demo auto-start detection");
        None
    } else {
        detect_demo_mode_with_context(&application_context)
    };
    if let Some((mission_name, proto_name, pcs, location)) = demo_config {
        tracing::info!(
            "Demo mode detected — mission={mission_name}, proto={proto_name}, PCs={pcs}"
        );
        campaign.reset(&profiles);
        // Parse the PC string to build the gang from specific characters.
        campaign.create_gang_from_pcs(pcs, &profiles);
        campaign.add_all_to_mission_team();
        // Demo mission is index 1 (index 0 = Sherwood)
        campaign.current_mission_idx = Some(1);
        let mut callbacks = RustCallbacks::with_application_context(application_context.clone());
        run_mission(
            window,
            &mut callbacks,
            &mut campaign,
            &profiles,
            1,
            location,
            args,
        )
        .await?;
        return Ok(0);
    }

    // ── `--sherwood`: skip the main menu, drop into Sherwood HQ. ──
    // Resets the campaign (same as clicking "Start"), forces the next
    // mission slot to Sherwood (idx 0), and runs the mission directly
    // — bypassing the campaign-map overlay that normally sits between
    // menu and Sherwood.
    if args.sherwood {
        tracing::info!("--sherwood: launching directly into the Sherwood HQ mission");
        campaign.reset(&profiles);
        campaign.force_next_mission(0);
        campaign.current_mission_idx = Some(0);
        let mut callbacks = RustCallbacks::with_application_context(application_context.clone());
        run_mission(
            window,
            &mut callbacks,
            &mut campaign,
            &profiles,
            0,
            MissionLocation::Sherwood,
            args,
        )
        .await?;
        return Ok(0);
    }

    // ── `--replay`: select the mission from the replay header ──
    // The replay's `mission_id` is the mission's base `.rhm` filename
    // (e.g. `"Dem_Lei_MP"`). Look it up in the campaign mission table,
    // force `current_mission_idx` to match, and jump straight into
    // `run_mission` so the replay drives a fresh engine configured for
    // the right level. Bypasses the main-menu / campaign-map flow.
    if let Some(spec) = args.replay.as_ref() {
        match crate::replay_format::load_replay_spec(spec) {
            Ok(data) => {
                let mission_name = data.header.mission_id.clone();
                let idx = campaign
                    .missions
                    .iter()
                    .position(|m| m.profile(&profiles).mission_filename == mission_name);
                if let Some(idx) = idx {
                    let location = campaign.missions[idx].profile(&profiles).location;
                    tracing::info!(
                        "--replay: launching mission `{mission_name}` (idx={idx}) from replay header"
                    );
                    if let Some(bytes) = data.header.campaign.as_deref() {
                        campaign = bitcode::deserialize(bytes)
                            .map_err(|e| format!("failed to restore replay campaign: {e}"))?;
                        tracing::info!(
                            bytes = bytes.len(),
                            "--replay: restored campaign snapshot from replay header"
                        );
                    } else {
                        tracing::warn!(
                            "--replay: replay header has no campaign snapshot; using reset campaign"
                        );
                        campaign.reset(&profiles);
                    }
                    campaign.force_next_mission(idx);
                    campaign.current_mission_idx = Some(idx);
                    let mut callbacks =
                        RustCallbacks::with_application_context(application_context.clone());
                    run_mission(
                        window,
                        &mut callbacks,
                        &mut campaign,
                        &profiles,
                        idx,
                        location,
                        args,
                    )
                    .await?;
                    return Ok(0);
                } else {
                    tracing::warn!(
                        "--replay: mission `{mission_name}` not found in campaign; falling through to default startup"
                    );
                }
            }
            Err(e) => {
                tracing::error!("--replay: failed to load replay spec: {e}");
                return Err(format!("failed to load replay: {e}"));
            }
        }
    }

    // ── `--headless` requires a non-menu entry path ──
    // The main menu is fully rendered: with no display there's no way
    // to navigate it.  The demo and `--sherwood` branches above
    // already cover the headless use cases (replay scrubbing,
    // automated tests, CI).
    if args.headless {
        return Err(
            "--headless requires --sherwood or a demo data dir; the main \
             menu cannot be navigated without a display."
                .into(),
        );
    }

    // ── Full game: outer main menu loop ──
    loop {
        // PARITY TODO(app-context): `main_menu` is outside this slice and
        // still edits singleton profile/key stores. Mirror owned snapshots on
        // either side of its await; the guards themselves never survive the
        // synchronous helper calls.
        mirror_context_to_legacy_stores(&application_context)?;
        let menu_choice = show_main_menu(window, &campaign, &profiles, shipping.as_deref()).await?;
        adopt_legacy_stores_into_context(&application_context)?;

        match menu_choice {
            MainMenuChoice::Start => {
                // Reset campaign for a new game
                campaign.reset(&profiles);
                tracing::info!("Campaign reset for new game");

                if let Some((mission_name, _proto_name, pcs, location)) =
                    detect_demo_mode_with_context(&application_context)
                {
                    tracing::info!(
                        "Main menu Start: demo datadir detected, launching `{mission_name}`"
                    );
                    campaign.create_gang_from_pcs(pcs, &profiles);
                    campaign.add_all_to_mission_team();
                    let idx = campaign
                        .missions
                        .iter()
                        .position(|m| m.profile(&profiles).mission_filename == mission_name)
                        .ok_or_else(|| {
                            format!("demo mission `{mission_name}` is present in data but missing from campaign")
                        })?;
                    campaign.current_mission_idx = Some(idx);
                    let mut callbacks =
                        RustCallbacks::with_application_context(application_context.clone());
                    run_mission(
                        window,
                        &mut callbacks,
                        &mut campaign,
                        &profiles,
                        idx,
                        location,
                        args,
                    )
                    .await?;
                    tracing::info!("Returned to main menu");
                    continue;
                }

                // Session always returns to menu (window close causes Quit → QuitToMenu)
                let SessionResult::QuitToMenu =
                    run_session(window, &mut campaign, &profiles, args, None).await?;
                adopt_legacy_stores_into_context(&application_context)?;
                tracing::info!("Returned to main menu");
            }
            MainMenuChoice::Load { slot, mission_id } => {
                // Route the save into the session's `perform_pending_save_load`
                // path.  Point `next_mission_idx` at the save's mission so
                // the first `determine_next_mission` call enters the right
                // level; the session's cross-mission logic
                // (`game_session::run_session:LevelLoad`) handles any
                // mismatch if needed.
                if let Some(idx) = campaign
                    .missions
                    .iter()
                    .position(|m| m.profile(&profiles).id == mission_id)
                {
                    campaign.force_next_mission(idx);
                } else {
                    tracing::warn!(
                        "Main menu Load: no mission matching save header id {mission_id} — \
                         session will start at the default mission and apply the save in place"
                    );
                }
                tracing::info!("Main menu Load: slot={slot}, mission_id={mission_id}");
                let SessionResult::QuitToMenu = run_session(
                    window,
                    &mut campaign,
                    &profiles,
                    args,
                    Some(SaveLoadRequest::Load {
                        slot: Some(slot),
                        mission_id,
                    }),
                )
                .await?;
                adopt_legacy_stores_into_context(&application_context)?;
                tracing::info!("Returned to main menu from Load");
            }
            MainMenuChoice::Multiplayer(launch) => {
                let Some(idx) = campaign
                    .missions
                    .iter()
                    .position(|m| m.profile(&profiles).id == launch.mission_id)
                else {
                    return Err(format!(
                        "Multiplayer lobby selected unknown mission id {} ({})",
                        launch.mission_id, launch.mission_name
                    ));
                };
                campaign.reset(&profiles);
                if let Some((_, _, pcs, _)) = detect_demo_mode_with_context(&application_context) {
                    campaign.create_gang_from_pcs(pcs, &profiles);
                }
                campaign.force_next_mission(idx);
                let mut mp_args = args.clone();
                match launch.role {
                    MultiplayerRole::Host { bind_addr } => {
                        tracing::info!(
                            mission = %launch.mission_name,
                            bind = %bind_addr,
                            "Main menu Multiplayer: hosting selected mission"
                        );
                        mp_args.server = Some(bind_addr);
                        mp_args.connect = None;
                    }
                    MultiplayerRole::Client { connect_addr } => {
                        tracing::info!(
                            mission = %launch.mission_name,
                            connect = %connect_addr,
                            "Main menu Multiplayer: joining selected mission"
                        );
                        mp_args.server = None;
                        mp_args.connect = Some(connect_addr);
                    }
                }
                mp_args.mp_start_at_epoch_ms = launch.start_at_epoch_ms;
                mp_args.mp_expected_players = Some(launch.expected_players);
                let SessionResult::QuitToMenu =
                    run_session(window, &mut campaign, &profiles, &mp_args, None).await?;
                adopt_legacy_stores_into_context(&application_context)?;
                tracing::info!("Returned to main menu from Multiplayer");
            }
            MainMenuChoice::CustomMission(launch) => {
                let mods_root = crate::mod_pack::default_mods_root();
                tracing::info!(
                    "Main menu CustomMission: slug={} rhm={} map={} spellforge={}",
                    launch.slug,
                    launch.rhm_basename,
                    launch.map_filename,
                    launch.requires_spellforge
                );
                let mount_guard = match crate::mod_pack::mount_for_launch(
                    &launch.version_zip,
                    launch.requires_spellforge,
                    &mods_root,
                ) {
                    Ok(g) => g,
                    Err(e) => {
                        tracing::error!("CustomMission: mount failed: {e}");
                        continue;
                    }
                };
                let profiles_mut = std::sync::Arc::make_mut(&mut profiles);
                campaign.reset(profiles_mut);
                let idx = match campaign.force_next_mission_by_name(
                    profiles_mut,
                    &launch.rhm_basename,
                    &launch.map_filename,
                    true,
                ) {
                    Some(i) => i,
                    None => {
                        tracing::error!(
                            "CustomMission: force_next_mission_by_name returned None for rhm={} proto={}",
                            launch.rhm_basename,
                            launch.map_filename
                        );
                        drop(mount_guard);
                        continue;
                    }
                };
                campaign.current_mission_idx = Some(idx);
                // Demo-mode init: if the active datadir is a demo, the
                // gang has to be created from the PCs declared in the
                // demo manifest, same as MainMenuChoice::Start. Custom
                // missions don't dictate roster, they piggyback on
                // whatever the datadir's campaign would have used.
                if let Some((_, _, pcs, _)) = detect_demo_mode_with_context(&application_context) {
                    campaign.create_gang_from_pcs(pcs, &profiles);
                    campaign.add_all_to_mission_team();
                }
                // If the mission ships a Lua companion, hand it off
                // to `run_mission` via the CLI args so `game_session`
                // can build a `LuaSession` against the just-loaded
                // engine. `LuaSession::start` is the one that decides
                // there's nothing to do for vanilla launches; passing
                // the pending struct unconditionally keeps the
                // decision in one place.
                let mut session_args = args.clone();
                session_args.pending_lua_mission = Some(crate::main_entry::PendingLuaMission {
                    slug: launch.slug.clone(),
                    rhm_basename: launch.rhm_basename.clone(),
                    version_zip: launch.version_zip.clone(),
                    mods_root: mods_root.clone(),
                    requires_spellforge: launch.requires_spellforge,
                });
                if launch.requires_spellforge && session_args.rollback_check {
                    // The checker is a default diagnostic, not a mode selected
                    // by the custom-mission picker. Spellforge cannot
                    // participate because its Lua state is not snapshotted;
                    // make that opt-out explicit and visible while preserving
                    // ordinary single-player custom-mission launch.
                    tracing::warn!(
                        mission = %launch.rhm_basename,
                        "Spellforge: disabling the default rollback checker for this normal single-player launch; explicit deterministic modes reject Spellforge"
                    );
                    session_args.rollback_check = false;
                }
                let SessionResult::QuitToMenu =
                    run_session(window, &mut campaign, &profiles, &session_args, None).await?;
                adopt_legacy_stores_into_context(&application_context)?;
                drop(mount_guard);
                tracing::info!("Returned to main menu from CustomMission");
            }
            MainMenuChoice::Exit => {
                tracing::info!("Player exited from main menu");
                return Ok(0);
            }
        }
    }
}

pub async fn run_rust_game_headless(
    mut campaign: Campaign,
    mut profiles: std::sync::Arc<engine_profiles::ProfileManager>,
    application_context: ApplicationContext,
    args: &CliArgs,
) -> Result<i32, String> {
    let application_context =
        application_context.with_options(args.global_options.options().clone());
    let mut run_args = args.clone();
    run_args.global_options = application_context.clone();
    let args = &run_args;

    #[cfg(not(target_arch = "wasm32"))]
    crate::http_server::start_global(args.http_server)?;

    tracing::info!("--headless: running without winit, wgpu, renderer, or audio backend");

    let launch = if let Some(launch) = force_mission_launch(&mut campaign, &mut profiles, args)? {
        Some(launch)
    } else if let Some((mission_name, _proto_name, pcs, location)) =
        detect_demo_mode_with_context(&application_context)
    {
        campaign.reset(&profiles);
        campaign.create_gang_from_pcs(pcs, &profiles);
        campaign.add_all_to_mission_team();
        let idx = campaign
            .missions
            .iter()
            .position(|m| m.profile(&profiles).mission_filename == mission_name)
            .ok_or_else(|| {
                format!(
                    "demo mission `{mission_name}` is present in data but missing from campaign"
                )
            })?;
        campaign.current_mission_idx = Some(idx);
        Some((idx, location))
    } else if args.sherwood {
        campaign.reset(&profiles);
        campaign.force_next_mission(0);
        campaign.current_mission_idx = Some(0);
        Some((0, MissionLocation::Sherwood))
    } else if let Some(spec) = args.replay.as_ref() {
        let data = crate::replay_format::load_replay_spec(spec)
            .map_err(|e| format!("failed to load replay: {e}"))?;
        let mission_name = data.header.mission_id.clone();
        let idx = campaign
            .missions
            .iter()
            .position(|m| m.profile(&profiles).mission_filename == mission_name)
            .ok_or_else(|| format!("--replay: mission `{mission_name}` not found in campaign"))?;
        if let Some(bytes) = data.header.campaign.as_deref() {
            campaign = bitcode::deserialize(bytes)
                .map_err(|e| format!("failed to restore replay campaign: {e}"))?;
        } else {
            tracing::warn!(
                "--replay: replay header has no campaign snapshot; using reset campaign"
            );
            campaign.reset(&profiles);
        }
        campaign.force_next_mission(idx);
        campaign.current_mission_idx = Some(idx);
        let location = campaign.missions[idx].profile(&profiles).location;
        Some((idx, location))
    } else {
        None
    };

    let Some((idx, location)) = launch else {
        return Err(
            "--headless requires --sherwood, --replay, or a demo data dir; the main menu cannot be navigated without a display."
                .into(),
        );
    };

    let mut callbacks = RustCallbacks::with_application_context(application_context);
    run_mission_headless(
        &mut callbacks,
        &mut campaign,
        &profiles,
        idx,
        location,
        args,
    )
    .await?;
    Ok(0)
}

/// Block until a `load-replay` RPC call queues a pending replay,
/// returning the mission-id stamped in that replay's header.
///
/// Paints a dark blue canvas (so the user sees *something* other
/// than the browser's default white) and pumps SDL events on a 20 Hz
/// poll.  The pending-replay slot is peeked, not consumed — the
/// caller hands control to `run_mission`, which drains the slot
/// inside `init_replay_and_rollback`.
async fn wait_for_replay_command(window: &mut GameWindow) -> String {
    loop {
        // Pump events — winit needs the app to drain its queue every
        // frame to stay responsive (mirrors the SDL3 emscripten note).
        let _ = window.poll_events();

        // Drain RPCs — the `load-replay` endpoint is how this loop
        // exits, and the normal `drain_global` path needs an engine.
        // `drain_pre_engine` handles `load-replay` / `info` and
        // rejects everything else with an "engine not ready" reply.
        crate::http_server::drain_pre_engine();

        window.clear_to_color(wgpu::Color {
            r: 0.01,
            g: 0.02,
            b: 0.08,
            a: 1.0,
        });

        if let Some(mission_id) = crate::http_server::peek_pending_replay_mission_id() {
            return mission_id;
        }

        crate::window::sleep_ms(50).await;
    }
}
