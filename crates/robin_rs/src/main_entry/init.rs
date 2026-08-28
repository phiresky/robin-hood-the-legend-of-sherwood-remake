//! Data-directory, locale, profile, and key-config initialization.

use std::path::Path;
#[cfg(any(not(target_arch = "wasm32"), target_os = "android"))]
use std::path::PathBuf;

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
use thiserror::Error;

/// Coarse startup stage used by launchers to classify initialization failures
/// without parsing their user-facing messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitErrorCategory {
    DataDirectory,
    Content,
    PlayerProfile,
    Platform,
}

/// Failure while preparing the deterministic game data and host services.
///
/// The variants deliberately retain the startup stage. Launchers still show
/// the same messages as before, while diagnostics and tests can distinguish a
/// bad installation from corrupt content, player-profile state, or host
/// integration.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum InitError {
    #[error("Unable to install datadir {path}: SBFile error {status}")]
    DataDirectoryInstall { path: String, status: i32 },

    #[error(
        "ERROR: 'Data' directory not found in {cwd}\nSet ROBINHOOD_DATA_DIR=/path/to/game to the directory that\ncontains the game's Data/ folder (with Data/robinhood.bks).\nIf you do not own the game, I recommend buying it on GOG:\n{gog_store_url}"
    )]
    DataDirectoryMissing {
        cwd: String,
        gog_store_url: &'static str,
    },

    #[cfg(target_os = "android")]
    #[error("Unable to chdir to {path}: {source}")]
    DataDirectoryChange {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[cfg(target_os = "android")]
    #[error(
        "ERROR: neither APK asset Data/datadir.bin nor a loose Data directory was found in {cwd}"
    )]
    DataDirectoryAndroidAssetsMissing { cwd: String },

    #[error("shipping datadir: {source:#}")]
    ContentShippingDatadir {
        #[source]
        source: anyhow::Error,
    },

    #[error("{message}")]
    ContentProfilesJson { path: &'static str, message: String },

    #[error("Failed to open {path}: error {status}")]
    ContentProfilesOpen { path: &'static str, status: i32 },

    #[error("Failed to read profiles from {path}: error {source}")]
    ContentProfilesRead {
        path: &'static str,
        #[source]
        source: robin_engine::legacy_io::LegacyIoError,
    },

    #[error("Failed to apply soldier profile patch {path}: {message}")]
    ContentSoldierProfilePatch { path: String, message: String },

    #[error("localization: {source}")]
    ContentLocalization {
        #[source]
        source: crate::localization::LocalizationError,
    },

    #[error("{message}")]
    PlayerProfileState {
        save_directory: std::path::PathBuf,
        message: String,
    },

    #[error("install shipping datadir: {source:#}")]
    PlatformShippingDatadirInstall {
        #[source]
        source: anyhow::Error,
    },
}

impl InitError {
    pub const fn category(&self) -> InitErrorCategory {
        match self {
            Self::DataDirectoryInstall { .. } | Self::DataDirectoryMissing { .. } => {
                InitErrorCategory::DataDirectory
            }
            #[cfg(target_os = "android")]
            Self::DataDirectoryChange { .. } | Self::DataDirectoryAndroidAssetsMissing { .. } => {
                InitErrorCategory::DataDirectory
            }
            Self::ContentShippingDatadir { .. }
            | Self::ContentProfilesJson { .. }
            | Self::ContentProfilesOpen { .. }
            | Self::ContentProfilesRead { .. }
            | Self::ContentSoldierProfilePatch { .. }
            | Self::ContentLocalization { .. } => InitErrorCategory::Content,
            Self::PlayerProfileState { .. } => InitErrorCategory::PlayerProfile,
            Self::PlatformShippingDatadirInstall { .. } => InitErrorCategory::Platform,
        }
    }
}

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

/// Directory whose immediate subdirectories are registered as overlay
/// datadirs at startup.  Repository-shipped mods (e.g. hackable JSON
/// levels) live here.
pub const MODS_DIR: &str = "mods";

/// Engine-shipped overlay datadir: assets every installation needs on
/// top of its game data (e.g. the native bitmap fonts the Steam release
/// is missing). Registered before the `mods/` overlays.
pub const CORE_OVERLAY_DIR: &str = "assets/core-datadir";

/// Resolve an engine-shipped resource directory that lives next to the
/// installation: try the working directory first, then next to the
/// executable, then the dev layout (executable in `target/<profile>/`,
/// resources at the workspace root). The game may be launched from any
/// working directory since the datadir is resolved independently.
#[cfg(not(target_arch = "wasm32"))]
fn resolve_install_resource_dir(name: &str) -> Option<std::path::PathBuf> {
    let mut candidates = vec![PathBuf::from(name)];
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        candidates.push(exe_dir.join(name));
        candidates.push(exe_dir.join("..").join("..").join(name));
    }
    candidates.into_iter().find(|path| path.is_dir())
}

/// Resolve the repository/install `mods/` directory whose subdirectories
/// are auto-mounted as overlay datadirs.  `None` when the installation
/// ships no such directory.  Also scanned by the Custom Missions picker
/// so overlay-shipped mods (hackable levels) can carry a `details.json`.
#[cfg(not(target_arch = "wasm32"))]
pub fn overlay_mods_dir() -> Option<std::path::PathBuf> {
    resolve_install_resource_dir(MODS_DIR)
}

#[cfg(target_arch = "wasm32")]
pub fn overlay_mods_dir() -> Option<std::path::PathBuf> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn add_overlay_data_dirs() {
    match resolve_install_resource_dir(CORE_OVERLAY_DIR) {
        Some(dir) => {
            let dir = dir.to_string_lossy().into_owned();
            match SbFile::add_overlay_path(&dir) {
                SBFILE_NO_ERROR => tracing::info!("Registered core overlay datadir: {dir}"),
                SBFILE_ERROR_PATH_ALREADY_PRESENT => {}
                err => tracing::warn!("Core overlay datadir {dir} unavailable: {err}"),
            }
        }
        None => tracing::warn!(
            "Core overlay datadir {CORE_OVERLAY_DIR} was not found next to the game; \
             engine-shipped assets (fonts, UI icons) will be missing"
        ),
    }

    if let Some(mods_dir) = resolve_install_resource_dir(MODS_DIR)
        && let Ok(entries) = std::fs::read_dir(mods_dir)
    {
        // Sort for a deterministic overlay lookup order.
        let mut mod_dirs: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
            .map(|entry| entry.path().to_string_lossy().into_owned())
            .collect();
        mod_dirs.sort();
        for dir in mod_dirs {
            match SbFile::add_overlay_path(&dir) {
                SBFILE_NO_ERROR => tracing::info!("Registered mod overlay datadir: {dir}"),
                SBFILE_ERROR_PATH_ALREADY_PRESENT => {}
                err => tracing::warn!("Failed to register mod overlay datadir {dir}: {err}"),
            }
        }
    }

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
fn setup_data_dir(data_dir_override: Option<&Path>) -> Result<(), InitError> {
    let data_dir = data_dir_override
        .map(|dir| dir.to_string_lossy().into_owned())
        .or_else(|| {
            std::env::var("ROBINHOOD_DATA_DIR")
                .ok()
                .filter(|dir| !dir.is_empty())
        });
    if let Some(data_dir) = data_dir {
        tracing::info!("using primary datadir {}", data_dir);
        let status = SbFile::set_primary_path(&data_dir);
        if status != SBFILE_NO_ERROR {
            return Err(InitError::DataDirectoryInstall {
                path: data_dir,
                status,
            });
        }
    } else {
        // No override and no env var: reuse the remembered datadir, or
        // auto-detect (working directory, executable directory, well-known
        // CD/GOG/Steam install locations — validated via Data/robinhood.bks)
        // and confirm with the player through the native dialog / folder
        // picker. See `datadir_locator::resolve_datadir`.
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        // Fall back to the working directory when nothing was found or the
        // player cancelled the picker; a loose unmarked `Data/` there keeps
        // working, anything else hits the descriptive error below.
        let chosen = crate::datadir_locator::resolve_datadir(exe_dir.as_deref())
            .unwrap_or_else(|| PathBuf::from("."));
        tracing::info!("using primary datadir {}", chosen.display());
        let status = SbFile::set_primary_path(&chosen.to_string_lossy());
        if status != SBFILE_NO_ERROR {
            return Err(InitError::DataDirectoryInstall {
                path: chosen.display().to_string(),
                status,
            });
        }
    }

    // Find the Data directory case-insensitively (some installs use "data", "DATA", etc.)
    if !SbFile::exists("Data") {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "?".into());
        return Err(InitError::DataDirectoryMissing {
            cwd,
            gog_store_url: crate::datadir_locator::GOG_STORE_URL,
        });
    }

    add_overlay_data_dirs();
    Ok(())
}

/// Android uses a pre-converted shipping datadir bundled as an APK
/// asset. If loose files are present (developer override), set the cwd
/// up the same way as desktop; otherwise rely on the installed
/// `ShippingDatadir` / `asset_fs` bundle.
#[cfg(target_os = "android")]
fn setup_data_dir(data_dir_override: Option<&Path>) -> Result<(), InitError> {
    let data_dir = data_dir_override
        .map(|dir| dir.to_string_lossy().into_owned())
        .or_else(|| {
            std::env::var("ROBINHOOD_DATA_DIR")
                .ok()
                .filter(|dir| !dir.is_empty())
        });
    if let Some(data_dir) = data_dir {
        tracing::info!("changing working directory to datadir {}", data_dir);
        std::env::set_current_dir(&data_dir).map_err(|source| InitError::DataDirectoryChange {
            path: data_dir,
            source,
        })?;
    }

    if robin_engine::sbfile::resolve_case_insensitive(Path::new("Data")).is_none()
        && assets_shipping_datadir::global().is_none()
    {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "?".into());
        return Err(InitError::DataDirectoryAndroidAssetsMissing { cwd });
    }

    Ok(())
}

/// Wasm version: there is no cwd or directory enumeration.  The Data/
/// prefix is anchored at `ROBINHOOD_DATA_URL` (default `./data`), which
/// `robin_util::asset_fs` consults for every read.  All we do here is
/// bootstrap language-folder detection.
#[cfg(target_arch = "wasm32")]
fn setup_data_dir(_data_dir_override: Option<&Path>) -> Result<(), InitError> {
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
pub fn rust_init() -> Result<RustInit, InitError> {
    rust_init_with_data_dir(None)
}

/// [`rust_init`] with an explicit primary datadir (e.g. from a tool's
/// `--data-dir` flag), taking priority over `ROBINHOOD_DATA_DIR`.
pub fn rust_init_with_data_dir(data_dir: Option<&Path>) -> Result<RustInit, InitError> {
    crate::init_tracing();
    setup_data_dir(data_dir)?;
    tracing::info!("Robin Hood — Rust entry point");

    // Load the shipping datadir if one exists. When present, subsystem
    // loaders prefer it over legacy disk I/O.
    let shipping = if let Some(path) = robin_engine::sbfile::resolve_data_path("Data/datadir.bin") {
        let datadir = assets_shipping_datadir::ShippingDatadir::load_from_file(&path)
            .map_err(|source| InitError::ContentShippingDatadir { source })?;
        Some(
            assets_shipping_datadir::install_global(std::sync::Arc::new(datadir))
                .map_err(|source| InitError::PlatformShippingDatadirInstall { source })?,
        )
    } else {
        None
    };

    rust_init_finish(shipping)
}

/// Initialize from a shipping datadir decoded and installed by the platform
/// bootstrap (the wasm host or Android NativeActivity entry point), skipping
/// the filesystem-backed [`assets_shipping_datadir::try_load`] path.
pub fn rust_init_with_shipping(
    shipping: Option<std::sync::Arc<assets_shipping_datadir::ShippingDatadir>>,
) -> Result<RustInit, InitError> {
    crate::init_tracing();
    setup_data_dir(None)?;
    tracing::info!("Robin Hood — Rust entry point (preinstalled shipping data)");
    rust_init_finish(shipping)
}

fn rust_init_finish(
    shipping: Option<std::sync::Arc<assets_shipping_datadir::ShippingDatadir>>,
) -> Result<RustInit, InitError> {
    let localization = crate::localization::LocalizationService::initialize(shipping.as_deref())
        .map_err(|source| InitError::ContentLocalization { source })?;
    let options = engine_api::GlobalOptions::default();
    let profiles = std::sync::Arc::new(load_profiles(shipping.as_deref(), &options)?);
    tracing::info!(
        "Rust profiles: {} chars, {} soldiers, {} missions, {} weapons",
        profiles.characters.len(),
        profiles.soldiers.len(),
        profiles.missions.len(),
        profiles.hth_weapons.len()
    );
    let player_profile_directory = crate::save_file::default_save_directory();
    let (player_profiles, player_profiles_regenerated) =
        load_player_profile_manager(&player_profile_directory);
    let key_configs = load_key_config_store(&player_profile_directory, player_profiles_regenerated);

    let application_context = ApplicationContext::complete_with_localization(
        options,
        player_profiles,
        key_configs,
        shipping,
        localization,
    )
    .map_err(|message| InitError::PlayerProfileState {
        save_directory: player_profile_directory,
        message,
    })?;

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
///
/// TODO(content-loading): `RHProfileManager::RHProfileManager(char*)` in the
/// Original imports the authored CSV directory and writes `profile.cpf` when
/// the compiled file is absent. The Rust runtime does not yet implement that
/// development fallback, so absence of all three supported representations
/// remains a fatal required-content error.
fn load_profiles(
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
    options: &engine_api::GlobalOptions,
) -> Result<ProfileManager, InitError> {
    if let Some(dd) = shipping
        && let Some(p) = dd.profiles.as_ref()
    {
        tracing::info!("Profiles: loaded from language-independent shipping datadir index");
        // Shipping profiles are baked at `convert_datadir` time
        // (`convert_shipping` calls `import_beam_mes` before storing
        // `dd.profiles`), so the per-mission `number_of_beam_mes` /
        // `required_actions` fields are already populated — no
        // post-processing needed here.
        return apply_soldier_profile_patches(p.clone());
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
        // TODO(typed-errors): make `ProfileManager::load_json` return a typed
        // error. Its current String boundary has already discarded the
        // underlying UTF-8 / serde_json source before startup sees it.
        let mut mgr = ProfileManager::load_json(json_path).map_err(|message| {
            InitError::ContentProfilesJson {
                path: json_path,
                message,
            }
        })?;
        mgr.import_beam_mes(level_dir);
        return apply_soldier_profile_patches(mgr);
    }
    let cpf_path = "Data/Configuration/profile.cpf";
    tracing::info!("Profiles: loading legacy CPF {cpf_path}");
    let mut file =
        engine_sbfile::SbFile::open(cpf_path, engine_sbfile::SB_FILE_READ).map_err(|status| {
            InitError::ContentProfilesOpen {
                path: cpf_path,
                status,
            }
        })?;
    let mut mgr = ProfileManager::new();
    mgr.load_all_legacy_cpf(&mut file)
        .map_err(|source| InitError::ContentProfilesRead {
            path: cpf_path,
            source,
        })?;
    mgr.import_beam_mes(level_dir);
    apply_soldier_profile_patches(mgr)
}

const SOLDIER_PROFILE_PATCH_PATH: &str = "Data/Configuration/soldier-profiles.patch.json";

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SoldierProfilePatch {
    #[serde(default)]
    soldiers: Vec<SoldierProfileAddition>,
    #[serde(default)]
    characters: Vec<CharacterProfileAddition>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CharacterProfileAddition {
    template: String,
    /// Retail soldier profile supplying the promoted NPC's combat statistics.
    /// The character template still supplies player actions and ammunition.
    #[serde(default)]
    combat_profile: Option<String>,
    filename: String,
    profile_name: String,
    display_name: String,
    exclamation_profile: String,
    /// Contextual actions inherited from the playable template that the NPC's
    /// sprite set cannot actually perform.
    #[serde(default)]
    remove_contextual_actions: Vec<engine_profiles::Action>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SoldierProfileAddition {
    template: String,
    /// Optional preceding colour tier. When present, the new profile starts
    /// from `template` and extrapolates one more step of the original combat
    /// stat progression (`template + (template - progression_from)`).
    #[serde(default)]
    progression_from: Option<String>,
    filename: String,
    #[serde(default)]
    profile_name: Option<String>,
    display_name: String,
    #[serde(default)]
    hostile: Option<bool>,
}

fn resolve_soldier_profile_template(
    profiles: &ProfileManager,
    reference: &str,
) -> Result<engine_profiles::SoldierProfile, String> {
    let mut exact = profiles
        .soldiers
        .iter()
        .filter(|profile| profile.filename == reference);
    if let Some(profile) = exact.next()
        && exact.next().is_none()
    {
        return Ok(profile.clone());
    }

    profiles.soldier_idx_by_identifier(reference).map(|index| {
        profiles
            .get_soldier(index)
            .expect("resolved soldier profile index disappeared")
            .clone()
    })
}

fn extrapolate_progressive_stat(previous: u16, current: u16) -> u16 {
    let next = i32::from(current) * 2 - i32::from(previous);
    next.clamp(0, i32::from(u16::MAX)) as u16
}

fn extrapolate_capacity(previous: u16, current: u16) -> u16 {
    extrapolate_progressive_stat(previous, current).min(100)
}

fn extrapolate_soldier_progression(
    profile: &mut engine_profiles::SoldierProfile,
    previous: &engine_profiles::SoldierProfile,
) -> Result<(), String> {
    if profile.rank != previous.rank
        || profile.rider != previous.rider
        || profile.heavy != previous.heavy
        || profile.pathfinder_index != previous.pathfinder_index
        || profile.hth_weapon_id != previous.hth_weapon_id
        || profile.shooting_weapon_id != previous.shooting_weapon_id
    {
        return Err(format!(
            "progression profiles {:?} and {:?} are different soldier archetypes",
            previous.filename, profile.filename
        ));
    }

    profile.life_point = extrapolate_progressive_stat(previous.life_point, profile.life_point);
    // Original treats these as 0..=100 capacities. In particular,
    // `RHartificialmalignity.cpp` computes `100 - courage` and
    // `100 - intelligence`; exceeding 100 would underflow its UWORD math.
    profile.intelligence = extrapolate_capacity(previous.intelligence, profile.intelligence);
    profile.courage = extrapolate_capacity(previous.courage, profile.courage);
    profile.initiative = extrapolate_capacity(previous.initiative, profile.initiative);
    profile.pride = extrapolate_capacity(previous.pride, profile.pride);
    profile.shooting = extrapolate_capacity(previous.shooting, profile.shooting);
    profile.fighting = extrapolate_capacity(previous.fighting, profile.fighting);
    profile.endurance = extrapolate_capacity(previous.endurance, profile.endurance);
    Ok(())
}

fn apply_soldier_profile_patch(
    profiles: &mut ProfileManager,
    patch: SoldierProfilePatch,
) -> Result<(), String> {
    for addition in patch.characters {
        if profiles
            .characters
            .iter()
            .any(|profile| profile.filename == addition.filename)
        {
            return Err(format!(
                "new character filename {:?} already exists",
                addition.filename
            ));
        }
        let mut exclamation_matches = profiles
            .soldiers
            .iter()
            .map(|profile| (&profile.filename, profile.exclamation_id))
            .chain(
                profiles
                    .civilians
                    .iter()
                    .map(|profile| (&profile.filename, profile.exclamation_id)),
            )
            .filter(|(filename, _)| filename.as_str() == addition.exclamation_profile);
        let exclamation_id = exclamation_matches
            .next()
            .map(|(_, id)| id)
            .ok_or_else(|| {
                format!(
                    "character exclamation profile {:?} does not exist",
                    addition.exclamation_profile
                )
            })?;
        if let Some((_, conflicting_id)) = exclamation_matches.find(|(_, id)| *id != exclamation_id)
        {
            return Err(format!(
                "character exclamation profile {:?} has conflicting voice banks {exclamation_id} and {conflicting_id}",
                addition.exclamation_profile,
            ));
        }
        if exclamation_id == 0 {
            return Err(format!(
                "character exclamation profile {:?} has no voice bank",
                addition.exclamation_profile
            ));
        }

        let mut templates = profiles
            .characters
            .iter()
            .filter(|profile| profile.filename == addition.template);
        let mut profile = templates
            .next()
            .cloned()
            .ok_or_else(|| format!("character template {:?} does not exist", addition.template))?;
        if templates.next().is_some() {
            return Err(format!(
                "character template {:?} is ambiguous",
                addition.template
            ));
        }
        profile.index = profiles.characters.len() as u32;
        profile.filename = addition.filename;
        profile.profile_name = addition.profile_name;
        profile.display_name = addition.display_name;
        profile.exclamation_id = exclamation_id;
        if let Some(combat_reference) = addition.combat_profile.as_deref() {
            let combat = resolve_soldier_profile_template(profiles, combat_reference)
                .map_err(|message| format!("combat_profile {combat_reference:?}: {message}"))?;
            profile.shooting = combat.shooting;
            profile.fighting = combat.fighting;
            profile.endurance = combat.endurance;
            profile.hth_weapon_id = combat.hth_weapon_id;
            profile.shooting_weapon_id = combat.shooting_weapon_id;
            profile.wake_up = combat.wake_up;
            profile.weapon_material = combat.weapon_material;
            profile.armor_material = combat.armor_material;
        }
        for action in addition.remove_contextual_actions {
            let slot = profile
                .contextual_actions
                .iter_mut()
                .find(|inherited| **inherited == action)
                .ok_or_else(|| {
                    format!(
                        "character template {:?} does not have contextual action {action:?}",
                        addition.template
                    )
                })?;
            *slot = engine_profiles::Action::NoAction;
        }
        profile.alternative_profile_name.clear();
        profile.valid_alternative_profile = false;
        profiles.characters.push(profile);
    }

    for addition in patch.soldiers {
        if profiles
            .soldiers
            .iter()
            .any(|profile| profile.filename == addition.filename)
        {
            return Err(format!(
                "new soldier filename {:?} already exists",
                addition.filename
            ));
        }
        let mut profile = resolve_soldier_profile_template(profiles, &addition.template)
            .map_err(|message| format!("template {:?}: {message}", addition.template))?;
        if let Some(previous_reference) = addition.progression_from.as_deref() {
            let previous = resolve_soldier_profile_template(profiles, previous_reference)
                .map_err(|message| format!("progression_from {previous_reference:?}: {message}"))?;
            extrapolate_soldier_progression(&mut profile, &previous)?;
        }
        profile.filename = addition.filename;
        if let Some(profile_name) = addition.profile_name {
            profile.profile_name = profile_name;
        }
        profile.display_name = addition.display_name;
        if let Some(hostile) = addition.hostile {
            profile.hostile = hostile;
        }
        profiles.soldiers.push(profile);
    }
    Ok(())
}

fn apply_soldier_profile_patches(
    mut profiles: ProfileManager,
) -> Result<ProfileManager, InitError> {
    for root in engine_sbfile::SbFile::overlay_paths() {
        let path = Path::new(&root).join(SOLDIER_PROFILE_PATCH_PATH);
        if !path.is_file() {
            continue;
        }
        let result = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|e| e.to_string()))
            .and_then(|patch| apply_soldier_profile_patch(&mut profiles, patch));
        if let Err(message) = result {
            return Err(InitError::ContentSoldierProfilePatch {
                path: path.display().to_string(),
                message,
            });
        }
        tracing::info!("Applied soldier profile patch {}", path.display());
    }
    Ok(profiles)
}

/// Load the player-profile service owned by [`ApplicationContext`].
///
/// The boolean reports regeneration so the parallel Rust key-config store can
/// be reset with the profile-owned key bindings that the Original recreated.
fn load_player_profile_manager(save_dir: &Path) -> (PlayerProfileManager, bool) {
    let save_dir_str = save_dir.to_string_lossy().into_owned();

    // Original behavior: `RHPlayerProfileManager::Load` in
    // `original-code/RHplayerprofilemanager.cpp` recreates the default Robin
    // profile when the player archive is absent or invalid. This recovery is
    // player-state compatibility, not a fallback for required game content.
    match PlayerProfileManager::load(&save_dir_str) {
        Ok(mgr)
            if mgr
                .active_index
                .and_then(|index| mgr.profiles.get(index))
                .is_some() =>
        {
            (mgr, false)
        }
        Ok(mgr) => (
            regenerate_default_player_profiles(
                save_dir_str,
                format!(
                    "archive has {} profiles and active index {:?}",
                    mgr.profiles.len(),
                    mgr.active_index
                ),
            ),
            true,
        ),
        Err(error) => (
            regenerate_default_player_profiles(save_dir_str, error.to_string()),
            true,
        ),
    }
}

/// Recreate the Original's first-launch profile after an absent or invalid
/// archive. `RHPlayerProfileManager::CreateDefaultProfiles` marks the manager
/// as default-backed and immediately saves it; keeping both details here
/// prevents a corrupt archive from failing on every launch or skipping the
/// new-player prompt.
fn regenerate_default_player_profiles(
    save_directory: String,
    reason: String,
) -> PlayerProfileManager {
    tracing::warn!(
        "Failed to load player profiles from {save_directory} ({reason}); creating defaults"
    );
    let mut manager = PlayerProfileManager::new(save_directory);
    let index = manager.create_profile("Robin".to_owned(), DifficultyLevel::Medium);
    manager.set_active(index);
    manager.default_profiles = true;
    if let Err(error) = manager.save() {
        // The Original launcher also keeps running after CreateDefaultProfiles
        // reports a save failure. Retain the usable in-memory profile, but do
        // not hide that persistence is unavailable.
        tracing::warn!(
            "Failed to persist regenerated player profiles to {}: {error}",
            manager.save_directory
        );
    }
    manager
}

/// Load the key-config service owned by [`ApplicationContext`]. First-run
/// stores are intentionally empty; `ApplicationContext::complete` creates
/// the active profile's original-compatible default entry.
fn load_key_config_store(save_dir: &Path, player_profiles_regenerated: bool) -> KeyConfigStore {
    let save_dir_str = save_dir.to_string_lossy().into_owned();

    if player_profiles_regenerated {
        tracing::warn!(
            "Ignoring key configs in {save_dir_str} because player profiles were regenerated"
        );
        let store = KeyConfigStore::new(save_dir_str);
        if let Err(error) = store.save() {
            tracing::warn!(
                "Failed to persist reset key configs to {}: {error}",
                store.save_directory
            );
        }
        return store;
    }

    KeyConfigStore::load(&save_dir_str).unwrap_or_else(|err| {
        tracing::warn!(
            "Failed to load key configs from {save_dir_str} ({err}); starting with empty store"
        );
        KeyConfigStore::new(save_dir_str)
    })
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::*;

    #[test]
    fn initialization_errors_retain_their_startup_category() {
        let cases = [
            (
                InitError::DataDirectoryInstall {
                    path: "/game".to_owned(),
                    status: -1,
                },
                InitErrorCategory::DataDirectory,
            ),
            (
                InitError::ContentShippingDatadir {
                    source: anyhow::anyhow!("decode failed"),
                },
                InitErrorCategory::Content,
            ),
            (
                InitError::ContentProfilesOpen {
                    path: "Data/Configuration/profile.cpf",
                    status: -2,
                },
                InitErrorCategory::Content,
            ),
            (
                InitError::PlayerProfileState {
                    save_directory: "/saves".into(),
                    message: "no active player profile".to_owned(),
                },
                InitErrorCategory::PlayerProfile,
            ),
            (
                InitError::PlatformShippingDatadirInstall {
                    source: anyhow::anyhow!("mount failed"),
                },
                InitErrorCategory::Platform,
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.category(), expected);
        }
    }

    #[test]
    fn initialization_error_display_preserves_launcher_messages() {
        let missing = InitError::DataDirectoryMissing {
            cwd: "/missing".to_owned(),
            gog_store_url: "https://example.invalid/game",
        };
        assert_eq!(
            missing.to_string(),
            "ERROR: 'Data' directory not found in /missing\n\
             Set ROBINHOOD_DATA_DIR=/path/to/game to the directory that\n\
             contains the game's Data/ folder (with Data/robinhood.bks).\n\
             If you do not own the game, I recommend buying it on GOG:\n\
             https://example.invalid/game"
        );

        let profile = InitError::ContentProfilesOpen {
            path: "Data/Configuration/profile.cpf",
            status: -7,
        };
        assert_eq!(
            profile.to_string(),
            "Failed to open Data/Configuration/profile.cpf: error -7"
        );
    }

    #[test]
    fn soldier_profile_patch_appends_without_mutating_the_template() {
        let mut profiles = ProfileManager::new();
        profiles.soldiers.push(engine_profiles::SoldierProfile {
            filename: "Knight03".to_owned(),
            display_name: "Red Cavalier".to_owned(),
            hostile: true,
            ..Default::default()
        });
        let patch = SoldierProfilePatch {
            characters: Vec::new(),
            soldiers: vec![SoldierProfileAddition {
                template: "Knight03".to_owned(),
                progression_from: None,
                filename: "Knight00".to_owned(),
                profile_name: Some("Blue Cavalier".to_owned()),
                display_name: "Blue Cavalier".to_owned(),
                hostile: Some(false),
            }],
        };

        apply_soldier_profile_patch(&mut profiles, patch).unwrap();

        assert_eq!(profiles.soldiers.len(), 2);
        assert_eq!(profiles.soldiers[0].filename, "Knight03");
        assert_eq!(profiles.soldiers[0].display_name, "Red Cavalier");
        assert!(profiles.soldiers[0].hostile);
        assert_eq!(profiles.soldiers[1].filename, "Knight00");
        assert_eq!(profiles.soldiers[1].profile_name, "Blue Cavalier");
        assert_eq!(profiles.soldiers[1].display_name, "Blue Cavalier");
        assert!(!profiles.soldiers[1].hostile);
    }

    #[test]
    fn soldier_profile_patch_keeps_character_rhs_key_separate_from_display_name() {
        let mut profiles = ProfileManager::new();
        profiles.characters.push(engine_profiles::CharacterProfile {
            filename: "RobinHood".to_owned(),
            profile_name: "Robin des Bois".to_owned(),
            contextual_actions: [
                engine_profiles::Action::Search,
                engine_profiles::Action::Climb,
                engine_profiles::Action::Jump,
                engine_profiles::Action::NoAction,
            ],
            ..Default::default()
        });
        profiles.soldiers.push(engine_profiles::SoldierProfile {
            filename: "Guisbourne".to_owned(),
            exclamation_id: 0x4747_0016,
            fighting: 100,
            endurance: 80,
            hth_weapon_id: 19,
            weapon_material: engine_profiles::WeaponMaterial::Steel,
            armor_material: engine_profiles::ArmorMaterial::Plate,
            ..Default::default()
        });
        profiles.civilians.push(engine_profiles::CivilianProfile {
            filename: "Guisbourne".to_owned(),
            exclamation_id: 0x4747_0016,
            ..Default::default()
        });
        let patch = SoldierProfilePatch {
            characters: vec![CharacterProfileAddition {
                template: "RobinHood".to_owned(),
                combat_profile: Some("Guisbourne".to_owned()),
                filename: "Guisbourne".to_owned(),
                profile_name: "Guisbourne".to_owned(),
                display_name: "Guy of Guisbourne".to_owned(),
                exclamation_profile: "Guisbourne".to_owned(),
                remove_contextual_actions: vec![engine_profiles::Action::Jump],
            }],
            soldiers: Vec::new(),
        };

        apply_soldier_profile_patch(&mut profiles, patch).unwrap();

        assert_eq!(profiles.characters.len(), 2);
        assert_eq!(profiles.characters[0].profile_name, "Robin des Bois");
        assert!(profiles.characters[0].display_name.is_empty());
        assert_eq!(profiles.characters[1].filename, "Guisbourne");
        assert_eq!(profiles.characters[1].profile_name, "Guisbourne");
        assert_eq!(profiles.characters[1].display_name, "Guy of Guisbourne");
        assert_eq!(profiles.characters[1].exclamation_id, 0x4747_0016);
        assert_eq!(profiles.characters[1].fighting, 100);
        assert_eq!(profiles.characters[1].endurance, 80);
        assert_eq!(profiles.characters[1].hth_weapon_id, 19);
        assert_eq!(
            profiles.characters[1].weapon_material,
            engine_profiles::WeaponMaterial::Steel
        );
        assert_eq!(
            profiles.characters[1].armor_material,
            engine_profiles::ArmorMaterial::Plate
        );
        assert!(
            !profiles.characters[1]
                .contextual_actions
                .contains(&engine_profiles::Action::Jump)
        );
    }

    #[test]
    fn soldier_profile_patch_extrapolates_an_elite_tier_from_original_progression() {
        let mut profiles = ProfileManager::new();
        profiles.soldiers.push(engine_profiles::SoldierProfile {
            filename: "Soldier B03".to_owned(),
            life_point: 135,
            intelligence: 95,
            courage: 95,
            pride: 80,
            fighting: 90,
            endurance: 95,
            rank: engine_profiles::ProfileRank::Knight,
            hth_weapon_id: 17,
            ..Default::default()
        });
        profiles.soldiers.push(engine_profiles::SoldierProfile {
            filename: "Soldier B04".to_owned(),
            life_point: 145,
            intelligence: 100,
            courage: 100,
            pride: 90,
            fighting: 100,
            endurance: 100,
            rank: engine_profiles::ProfileRank::Knight,
            hth_weapon_id: 17,
            ..Default::default()
        });
        let patch = SoldierProfilePatch {
            characters: Vec::new(),
            soldiers: vec![SoldierProfileAddition {
                template: "soldier_b04".to_owned(),
                progression_from: Some("soldier_b03".to_owned()),
                filename: "Fabri18 RoyalPurple Knight".to_owned(),
                profile_name: None,
                display_name: "Fabri18 Royal Purple Knight".to_owned(),
                hostile: Some(false),
            }],
        };

        apply_soldier_profile_patch(&mut profiles, patch).unwrap();

        let elite = &profiles.soldiers[2];
        assert_eq!(elite.life_point, 155);
        assert_eq!(elite.intelligence, 100);
        assert_eq!(elite.courage, 100);
        assert_eq!(elite.pride, 100);
        assert_eq!(elite.fighting, 100);
        assert_eq!(elite.endurance, 100);
        assert!(!elite.hostile);
    }

    #[test]
    fn soldier_profile_patch_accepts_an_explicit_duplicate_identifier() {
        let mut profiles = ProfileManager::new();
        profiles.soldiers.push(engine_profiles::SoldierProfile {
            filename: "Knight02".to_owned(),
            life_point: 105,
            ..Default::default()
        });
        profiles.soldiers.push(engine_profiles::SoldierProfile {
            filename: "Knight02".to_owned(),
            life_point: 145,
            ..Default::default()
        });
        let patch = SoldierProfilePatch {
            characters: Vec::new(),
            soldiers: vec![SoldierProfileAddition {
                template: "knight02__1".to_owned(),
                progression_from: None,
                filename: "Fabri18 CavalryBlack Cavalryman".to_owned(),
                profile_name: None,
                display_name: "Fabri18 Cavalry Black Cavalryman".to_owned(),
                hostile: Some(false),
            }],
        };

        apply_soldier_profile_patch(&mut profiles, patch).unwrap();

        assert_eq!(profiles.soldiers[2].life_point, 145);
    }

    #[test]
    fn initialization_error_exposes_underlying_source() {
        let error = InitError::ContentShippingDatadir {
            source: anyhow::anyhow!("invalid shipping payload"),
        };

        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("invalid shipping payload")
        );
        assert_eq!(
            error.to_string(),
            "shipping datadir: invalid shipping payload"
        );
    }

    #[test]
    fn semantically_invalid_player_archive_is_regenerated_and_persisted() {
        let directory = tempfile::tempdir().expect("temporary player-profile directory");
        let directory_string = directory.path().to_string_lossy().into_owned();
        let mut invalid = PlayerProfileManager::new(directory_string.clone());
        invalid.create_profile("orphan".to_owned(), DifficultyLevel::Hard);
        invalid.active_index = Some(99);
        invalid
            .save()
            .expect("write invalid player-profile fixture");
        let mut stale_key_configs = KeyConfigStore::new(directory_string.clone());
        stale_key_configs.entry_or_default(0);
        stale_key_configs
            .save()
            .expect("write stale key-config fixture");

        let (recovered, regenerated) = load_player_profile_manager(directory.path());
        assert!(regenerated);
        assert_eq!(recovered.profiles.len(), 1);
        assert_eq!(
            recovered.get_active().map(|profile| profile.name.as_str()),
            Some("Robin")
        );
        assert!(recovered.default_profiles);

        let persisted = PlayerProfileManager::load(&directory_string)
            .expect("reload regenerated player-profile archive");
        assert_eq!(
            persisted.get_active().map(|profile| profile.name.as_str()),
            Some("Robin")
        );
        assert!(persisted.default_profiles);

        let key_configs = load_key_config_store(directory.path(), regenerated);
        assert!(key_configs.configs.is_empty());
        assert!(
            KeyConfigStore::load(&directory_string)
                .expect("reload reset key configs")
                .configs
                .is_empty()
        );
    }
}
