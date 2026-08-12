//! Data-directory, locale, profile, and key-config initialization.

#[cfg(any(not(target_arch = "wasm32"), target_os = "android"))]
use std::path::Path;

use crate::host::ApplicationContext;
use crate::key_config_store::KeyConfigStore;
use robin_assets::shipping_datadir as assets_shipping_datadir;
use robin_engine::campaign::Campaign;
use robin_engine::engine as engine_api;
use robin_engine::player_profile::{DifficultyLevel, PlayerProfileManager};
use robin_engine::profiles as engine_profiles;
use robin_engine::profiles::ProfileManager;
use robin_engine::sbfile as engine_sbfile;
use robin_engine::sbfile::{SBFILE_ERROR_PATH_ALREADY_PRESENT, SBFILE_NO_ERROR, SbFile};

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

/// Register the shipped language-data directory for developer tools that
/// already established their own data-directory working directory.
///
/// Normal entry points do this as part of `setup_data_dir`. Direct engine
/// tools must opt in before loading localized text, voices, or movies.
pub fn register_language_data_paths_for_tool() {
    add_language_folder();
}

/// Set up the working directory so that `Data/` is accessible.
///
/// `data_dir_override` (e.g. a tool's `--data-dir` flag) takes priority
/// over the `ROBINHOOD_DATA_DIR` environment variable.
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn setup_data_dir(data_dir_override: Option<&Path>) -> Result<(), String> {
    let data_dir = data_dir_override
        .map(|dir| dir.to_string_lossy().into_owned())
        .or_else(|| std::env::var("ROBINHOOD_DATA_DIR").ok());
    if let Some(data_dir) = data_dir {
        tracing::info!("using primary datadir {}", data_dir);
        let status = SbFile::set_primary_path(&data_dir);
        if status != SBFILE_NO_ERROR {
            return Err(format!(
                "Unable to install datadir {data_dir}: SBFile error {status}"
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
fn setup_data_dir(data_dir_override: Option<&Path>) -> Result<(), String> {
    let data_dir = data_dir_override
        .map(|dir| dir.to_string_lossy().into_owned())
        .or_else(|| std::env::var("ROBINHOOD_DATA_DIR").ok());
    if let Some(data_dir) = data_dir {
        tracing::info!("changing working directory to datadir {}", data_dir);
        std::env::set_current_dir(&data_dir)
            .map_err(|e| format!("Unable to chdir to {}: {}", data_dir, e))?;
    }

    if robin_engine::sbfile::resolve_case_insensitive(Path::new("Data")).is_none()
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
/// `robin_util::asset_fs` consults for every read.  All we do here is
/// bootstrap language-folder detection.
#[cfg(target_arch = "wasm32")]
fn setup_data_dir(_data_dir_override: Option<&Path>) -> Result<(), String> {
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
    rust_init_with_data_dir(None)
}

/// [`rust_init`] with an explicit primary datadir (e.g. from a tool's
/// `--data-dir` flag), taking priority over `ROBINHOOD_DATA_DIR`.
pub fn rust_init_with_data_dir(data_dir: Option<&Path>) -> Result<RustInit, String> {
    crate::init_tracing();
    setup_data_dir(data_dir)?;
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

/// Initialize from a shipping datadir decoded and installed by the platform
/// bootstrap (the wasm host or Android NativeActivity entry point), skipping
/// the filesystem-backed [`assets_shipping_datadir::try_load`] path.
pub fn rust_init_with_shipping(
    shipping: Option<std::sync::Arc<assets_shipping_datadir::ShippingDatadir>>,
) -> Result<RustInit, String> {
    crate::init_tracing();
    setup_data_dir(None)?;
    tracing::info!("Robin Hood — Rust entry point (preinstalled shipping data)");
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

    let application_context =
        ApplicationContext::complete(options, player_profiles, key_configs, shipping)?;

    let campaign = Campaign::create(&profiles, application_context.sim_config().difficulty);

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
