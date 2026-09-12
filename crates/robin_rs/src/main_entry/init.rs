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
#[cfg(any(test, not(target_arch = "wasm32")))]
use robin_engine::sbfile::{SBFILE_ERROR_PATH_ALREADY_PRESENT, SBFILE_NO_ERROR};
use robin_engine::sbfile::{SbFile, SbFileSystem};
#[cfg(all(feature = "projection-export", not(target_arch = "wasm32")))]
use robin_run_protocol::{
    OfficialBuiltInOverlaySourceManifestV2, OfficialProjectionSourceFormatV1, ResourceLocaleRootV1,
    RulesConfigIdentityV1,
};
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
    #[error("Game data selection cancelled")]
    DataDirectoryCancelled,

    #[error("Unable to install datadir {path}: file error {status}")]
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

    #[error("{source}")]
    ContentProfilesJson {
        path: &'static str,
        #[source]
        source: robin_engine::profiles::ProfileJsonLoadError,
    },

    #[error("Failed to open {path}: error {status}")]
    ContentProfilesOpen { path: &'static str, status: i32 },

    #[error("Failed to read profiles from {path}: error {source}")]
    ContentProfilesRead {
        path: &'static str,
        #[source]
        source: robin_engine::legacy_io::LegacyIoError,
    },

    #[error("Failed to apply profile patch {path}: {message}")]
    ContentProfilePatch { path: String, message: String },

    #[error("core audio timing: {message}")]
    ContentAudioDurations { message: String },

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

    #[error("core overlay startup validation failed for {path}: {source:#}")]
    PlatformCoreOverlay {
        path: std::path::PathBuf,
        #[source]
        source: anyhow::Error,
    },

    #[error("official projection initialization rejected: {message}")]
    OfficialProjectionAuthority { message: String },
}

impl InitError {
    pub const fn category(&self) -> InitErrorCategory {
        match self {
            Self::DataDirectoryInstall { .. }
            | Self::DataDirectoryMissing { .. }
            | Self::DataDirectoryCancelled => InitErrorCategory::DataDirectory,
            #[cfg(target_os = "android")]
            Self::DataDirectoryChange { .. } | Self::DataDirectoryAndroidAssetsMissing { .. } => {
                InitErrorCategory::DataDirectory
            }
            Self::ContentShippingDatadir { .. }
            | Self::ContentProfilesJson { .. }
            | Self::ContentProfilesOpen { .. }
            | Self::ContentProfilesRead { .. }
            | Self::ContentProfilePatch { .. }
            | Self::ContentAudioDurations { .. }
            | Self::ContentLocalization { .. } => InitErrorCategory::Content,
            Self::PlayerProfileState { .. } => InitErrorCategory::PlayerProfile,
            Self::PlatformShippingDatadirInstall { .. }
            | Self::PlatformCoreOverlay { .. }
            | Self::OfficialProjectionAuthority { .. } => InitErrorCategory::Platform,
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
pub use robin_assets::original_text::LANGUAGE_FOLDERS;

/// English fallback locale folder, always added first in the international build.
pub use robin_assets::original_text::FALLBACK_LOCALE_FOLDER;

/// Environment variable containing additional datadir roots to overlay on top
/// of the primary `ROBINHOOD_DATA_DIR`.  Native builds use the platform path
/// separator (`:` on Unix, `;` on Windows).
pub const OVERLAY_DATA_DIRS_ENV: &str = "ROBINHOOD_OVERLAY_DATA_DIRS";

/// Directory whose immediate subdirectories are registered as overlay
/// datadirs at startup.  Repository-shipped mods (e.g. hackable JSON
/// levels) live here.
#[cfg(not(target_arch = "wasm32"))]
pub const MODS_DIR: &str = "mods";

/// Engine-shipped overlay datadir: assets every installation needs on
/// top of its game data (e.g. the native bitmap fonts the Steam release
/// is missing). Registered before the `mods/` overlays.
#[cfg(not(target_arch = "wasm32"))]
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
fn add_overlay_data_dirs(files: &SbFileSystem) -> Result<(), InitError> {
    let core_dir = resolve_install_resource_dir(CORE_OVERLAY_DIR).ok_or_else(|| {
        InitError::PlatformCoreOverlay {
            path: PathBuf::from(CORE_OVERLAY_DIR),
            source: anyhow::anyhow!(
                "required core overlay directory was not found next to the game"
            ),
        }
    })?;
    let manifest = crate::core_overlay::mount_validated_native_directory(
        &core_dir,
        |path| files.add_overlay_path(path),
        |path| {
            files
                .read_shared(path)
                .map_err(|status| anyhow::anyhow!("file read error {status}"))
        },
    )
    .map_err(|source| InitError::PlatformCoreOverlay {
        path: core_dir.clone(),
        source,
    })?;
    tracing::info!(
        path = %core_dir.display(),
        files = manifest.files.len(),
        shipping_schema = manifest.shipping_datadir_schema,
        "Registered validated native core overlay datadir"
    );

    let mut mod_roots = Vec::new();
    if let Some(root) = resolve_install_resource_dir(MODS_DIR) {
        mod_roots.push(root);
    }
    let configured_root = crate::mod_pack::default_mods_root();
    if !mod_roots.contains(&configured_root) {
        mod_roots.push(configured_root);
    }
    for mods_dir in mod_roots {
        let entries = match std::fs::read_dir(&mods_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                tracing::warn!("Cannot scan mod directory {}: {error}", mods_dir.display());
                continue;
            }
        };
        let mut roots = Vec::new();
        for entry in entries {
            match entry {
                Ok(entry) => {
                    let path = entry.path();
                    if path.is_dir()
                        || path
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
                    {
                        roots.push(path);
                    }
                }
                Err(error) => tracing::warn!("Cannot read mod directory entry: {error}"),
            }
        }
        roots.sort();
        for path in roots {
            match crate::mod_pack::mount_mod_overlay(files, &path) {
                SBFILE_NO_ERROR => tracing::info!("Registered mod overlay: {}", path.display()),
                SBFILE_ERROR_PATH_ALREADY_PRESENT => {}
                error => {
                    tracing::warn!("Failed to register mod overlay {}: {error}", path.display())
                }
            }
        }
    }

    let Ok(value) = std::env::var(OVERLAY_DATA_DIRS_ENV) else {
        return Ok(());
    };
    for path in std::env::split_paths(&value) {
        if path.as_os_str().is_empty() {
            continue;
        }
        let path = path.to_string_lossy().into_owned();
        match crate::mod_pack::mount_mod_overlay(files, Path::new(&path)) {
            SBFILE_NO_ERROR => tracing::info!("Registered overlay datadir: {path}"),
            SBFILE_ERROR_PATH_ALREADY_PRESENT => {
                tracing::debug!("Overlay datadir already registered: {path}")
            }
            err => tracing::warn!("Failed to register overlay datadir {path}: {err}"),
        }
    }
    Ok(())
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

/// Legacy installations still require their bootstrap alternate roots when
/// runtime pack validation cannot recognize a demo's raw Start.sxt format.
/// Keep the original English-first order, scoped to this application reader.
#[cfg(any(test, not(target_arch = "wasm32")))]
fn add_language_folder_with_files(files: &SbFileSystem) -> Result<(), InitError> {
    let add = |path: &str| match files.add_alternate_path(path) {
        SBFILE_NO_ERROR | SBFILE_ERROR_PATH_ALREADY_PRESENT => Ok(()),
        status => Err(InitError::DataDirectoryInstall {
            path: path.into(),
            status,
        }),
    };
    add(FALLBACK_LOCALE_FOLDER)?;
    for &folder in LANGUAGE_FOLDERS {
        if files
            .try_exists(folder)
            .map_err(|status| InitError::DataDirectoryInstall {
                path: folder.into(),
                status,
            })?
        {
            tracing::info!("Detected language folder: {folder}");
            add(folder)?;
            return Ok(());
        }
    }
    tracing::info!(
        "No locale-specific language folder found; relying on '1033' fallback for localized resources"
    );
    Ok(())
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
fn setup_data_dir(data_dir_override: Option<&Path>, files: &SbFileSystem) -> Result<(), InitError> {
    let data_dir = data_dir_override
        .map(|dir| dir.to_string_lossy().into_owned())
        .or_else(|| {
            std::env::var("ROBINHOOD_DATA_DIR")
                .ok()
                .filter(|dir| !dir.is_empty())
        });
    if let Some(data_dir) = data_dir {
        tracing::info!("using primary datadir {}", data_dir);
        let status = files.set_primary_path(&data_dir);
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
        // Only non-interactive discovery may fall back to loose, unmarked Data/.
        // Cancelling the picker must stop startup before installing any data.
        let chosen = startup_data_dir(crate::datadir_locator::resolve_datadir(exe_dir.as_deref()))?;
        tracing::info!("using primary datadir {}", chosen.display());
        let status = files.set_primary_path(&chosen.to_string_lossy());
        if status != SBFILE_NO_ERROR {
            return Err(InitError::DataDirectoryInstall {
                path: chosen.display().to_string(),
                status,
            });
        }
    }

    // Find the Data directory case-insensitively (some installs use "data", "DATA", etc.)
    if !files
        .try_exists("Data")
        .map_err(|status| InitError::DataDirectoryInstall {
            path: "Data".into(),
            status,
        })?
    {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "?".into());
        return Err(InitError::DataDirectoryMissing {
            cwd,
            gog_store_url: crate::datadir_locator::GOG_STORE_URL,
        });
    }

    add_overlay_data_dirs(files)?;
    add_language_folder_with_files(files)?;
    Ok(())
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn startup_data_dir(
    resolution: crate::datadir_locator::DataDirResolution,
) -> Result<PathBuf, InitError> {
    use crate::datadir_locator::DataDirResolution;
    match resolution {
        DataDirResolution::Selected(path) => Ok(path),
        DataDirResolution::Unavailable => Ok(PathBuf::from(".")),
        DataDirResolution::Cancelled => Err(InitError::DataDirectoryCancelled),
    }
}

#[cfg(all(test, not(target_arch = "wasm32"), not(target_os = "android")))]
#[test]
fn datadir_cancellation_does_not_fall_back_to_working_directory() {
    use crate::datadir_locator::DataDirResolution;
    assert!(matches!(
        startup_data_dir(DataDirResolution::Cancelled),
        Err(InitError::DataDirectoryCancelled)
    ));
    assert_eq!(
        startup_data_dir(DataDirResolution::Unavailable).unwrap(),
        PathBuf::from(".")
    );
    let selected = PathBuf::from("/chosen/game");
    assert_eq!(
        startup_data_dir(DataDirResolution::Selected(selected.clone())).unwrap(),
        selected
    );
}

/// Android uses a pre-converted shipping datadir bundled as an APK
/// asset. If loose files are present (developer override), set the cwd
/// up the same way as desktop; otherwise rely on the installed
/// `ShippingDatadir` / `asset_fs` bundle.
#[cfg(target_os = "android")]
fn setup_data_dir(data_dir_override: Option<&Path>, files: &SbFileSystem) -> Result<(), InitError> {
    let data_dir = data_dir_override
        .map(|dir| dir.to_string_lossy().into_owned())
        .or_else(|| {
            std::env::var("ROBINHOOD_DATA_DIR")
                .ok()
                .filter(|dir| !dir.is_empty())
        });
    if let Some(data_dir) = data_dir {
        let status = files.set_primary_path(&data_dir);
        if status != SBFILE_NO_ERROR {
            return Err(InitError::DataDirectoryInstall {
                path: data_dir,
                status,
            });
        }
    }

    if robin_engine::sbfile::resolve_case_insensitive(Path::new("Data")).is_none()
        && files.mount_snapshot().asset_vfs.is_empty()
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
fn setup_data_dir(
    _data_dir_override: Option<&Path>,
    _files: &SbFileSystem,
) -> Result<(), InitError> {
    Ok(())
}

/// Result tuple for [`rust_init`] / [`rust_init_with_shipping`] /
/// [`rust_init_finish`]: the loaded campaign, mission profile manager, and
/// explicit application context (player profiles, key bindings, options,
/// and optional shipping data).
pub type RustInit = (
    Campaign,
    std::sync::Arc<engine_profiles::ProfileManager>,
    crate::host::ReadyApplicationContext,
);

/// Pure-Rust initialization: logging, data dir, profiles, campaign.
pub fn rust_init() -> Result<RustInit, InitError> {
    rust_init_with_data_dir(None)
}

/// [`rust_init`] with an explicit primary datadir (e.g. from a tool's
/// `--data-dir` flag), taking priority over `ROBINHOOD_DATA_DIR`.
pub fn rust_init_with_data_dir(data_dir: Option<&Path>) -> Result<RustInit, InitError> {
    crate::init_tracing();
    let files = std::sync::Arc::new(SbFileSystem::new(std::sync::Arc::new(
        robin_util::asset_fs::AssetVfs::new(),
    )));
    setup_data_dir(data_dir, &files)?;
    tracing::info!("Robin Hood — Rust entry point");

    // Load the shipping datadir if one exists. When present, subsystem
    // loaders prefer it over legacy disk I/O.
    let shipping = if let Some(path) = files.resolve_data_path("Data/datadir.bin") {
        let datadir =
            assets_shipping_datadir::ShippingDatadir::load_from_vfs(files.asset_vfs(), &path)
                .map_err(|source| InitError::ContentShippingDatadir { source })?;
        Some(
            assets_shipping_datadir::ShippingAssets::install(
                std::sync::Arc::new(datadir),
                files.asset_vfs().clone(),
            )
            .map_err(|source| InitError::PlatformShippingDatadirInstall { source })?
            .datadir()
            .clone(),
        )
    } else {
        None
    };

    rust_init_finish(shipping, files)
}

#[cfg(all(feature = "projection-export", not(target_arch = "wasm32")))]
fn forbidden_official_projection_environment() -> Vec<String> {
    let mut names = std::env::vars_os()
        .filter_map(|(name, _)| name.into_string().ok())
        .filter(|name| name.starts_with("ROBIN") || name.starts_with("PARITY_DEBUG_"))
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// Initialize a fresh exporter process from an already authenticated source
/// closure. This path deliberately bypasses all persisted profile, identity,
/// localization-preference, save, mod, and duration-cache discovery.
#[cfg(all(feature = "projection-export", not(target_arch = "wasm32")))]
pub fn rust_init_official_projection(
    data_dir: &Path,
    core_overlay_root: &Path,
    core_overlay_manifest: &OfficialBuiltInOverlaySourceManifestV2,
    resource_locale_root: &ResourceLocaleRootV1,
    source_format: OfficialProjectionSourceFormatV1,
    rules_config: &RulesConfigIdentityV1,
) -> Result<RustInit, InitError> {
    crate::init_tracing();
    fn authority_error(message: impl Into<String>) -> InitError {
        InitError::OfficialProjectionAuthority {
            message: message.into(),
        }
    }

    let forbidden_environment = forbidden_official_projection_environment();
    if !forbidden_environment.is_empty() {
        return Err(authority_error(format!(
            "environment overrides are forbidden: {}",
            forbidden_environment.join(", ")
        )));
    }
    if engine_api::GlobalOptions::global().is_some() {
        return Err(authority_error(
            "process-global launcher options were installed before official initialization",
        ));
    }
    if assets_shipping_datadir::global().is_some() {
        return Err(authority_error(
            "a shipping datadir was installed before official initialization",
        ));
    }
    let mounts = SbFile::mount_snapshot();
    if !mounts.alternate_paths.is_empty()
        || mounts.selected_locale.is_some()
        || mounts.fallback_locale.is_some()
        || !mounts.overlay_paths.is_empty()
        || mounts.primary_path.is_some()
        || !mounts.asset_vfs.is_empty()
        || mounts.official_projection_strict
    {
        return Err(authority_error(format!(
            "filesystem/VFS authorities were installed before official initialization: {mounts:?}"
        )));
    }

    crate::core_overlay::validate_official_projection_source(
        core_overlay_root,
        core_overlay_manifest,
    )
    .map_err(|error| authority_error(format!("invalid built-in overlay: {error:#}")))?;
    let sim_config =
        robin_engine::simulation_inputs::validate_official_projection_rules_config_v1(rules_config)
            .map_err(|error| authority_error(format!("invalid rules config: {error}")))?;

    SbFile::configure_official_projection_mounts(
        data_dir,
        resource_locale_root.as_str(),
        core_overlay_root,
    )
    .map_err(authority_error)?;

    // Seal both filesystem views while the shared VFS is still empty.
    // Installing shipping data below mounts its authenticated raw/locale
    // bundles; doing that first makes the fresh-authority check reject them.
    let files = std::sync::Arc::new(SbFileSystem::new(robin_util::asset_fs::global().clone()));
    files
        .configure_official_projection_mounts(
            data_dir,
            resource_locale_root.as_str(),
            core_overlay_root,
        )
        .map_err(authority_error)?;

    let shipping_path = robin_engine::sbfile::resolve_data_path("Data/datadir.bin");
    let shipping = match source_format {
        OfficialProjectionSourceFormatV1::LooseNativeV1 => {
            if shipping_path.is_some() {
                return Err(authority_error(
                    "loose official source unexpectedly contains Data/datadir.bin",
                ));
            }
            None
        }
        OfficialProjectionSourceFormatV1::ShippingDatadirV10 => {
            let path = shipping_path.ok_or_else(|| {
                authority_error("shipping official source is missing Data/datadir.bin")
            })?;
            let datadir = assets_shipping_datadir::ShippingDatadir::load_from_file(&path)
                .map_err(|error| authority_error(format!("decode shipping datadir: {error:#}")))?;
            let locale = datadir
                .locale(resource_locale_root.as_str())
                .map_err(|error| authority_error(format!("inspect shipping locale: {error:#}")))?
                .ok_or_else(|| {
                    authority_error(format!(
                        "shipping datadir has no exact LCID {}",
                        resource_locale_root.as_str()
                    ))
                })?;
            if locale.source_lcid.as_deref() != Some(resource_locale_root.as_str()) {
                return Err(authority_error(format!(
                    "shipping locale source identity differs: expected {}, found {:?}",
                    resource_locale_root.as_str(),
                    locale.source_lcid
                )));
            }
            let datadir = assets_shipping_datadir::install_global(std::sync::Arc::new(datadir))
                .map_err(|error| authority_error(format!("install shipping datadir: {error:#}")))?;
            datadir
                .set_active_locale(Some(resource_locale_root.as_str()))
                .map_err(|error| authority_error(format!("select shipping LCID: {error:#}")))?;
            Some(datadir)
        }
    };

    let options = engine_api::GlobalOptions {
        script_enabled: sim_config.script_enabled,
        highlander: sim_config.highlander,
        highlander2: sim_config.highlander2,
        golden_eye: sim_config.golden_eye,
        ignore_default_loose: sim_config.ignore_default_loose,
        bypass_fog_sprites_crash: sim_config.bypass_fog_sprites_crash,
        ..Default::default()
    };
    let profiles = std::sync::Arc::new(load_profiles_with_files(
        shipping.as_deref(),
        &options,
        &files,
    )?);
    let application_context = ApplicationContext::complete_official_projection_with_files(
        options,
        sim_config,
        shipping,
        Some(files),
    )
    .and_then(crate::host::ReadyApplicationContext::try_from)
    .map_err(authority_error)?;
    let campaign = Campaign::create(&profiles, application_context.sim_config().difficulty);
    Ok((campaign, profiles, application_context))
}

/// Initialize from a shipping datadir decoded and installed by the platform
/// bootstrap (the wasm host or Android NativeActivity entry point), skipping
/// the filesystem-backed [`assets_shipping_datadir::try_load`] path.
pub fn rust_init_with_shipping(
    shipping: Option<std::sync::Arc<assets_shipping_datadir::ShippingDatadir>>,
) -> Result<RustInit, InitError> {
    crate::init_tracing();
    let files = std::sync::Arc::new(SbFileSystem::new(
        shipping
            .as_ref()
            .map(|shipping| shipping.asset_vfs().clone())
            .unwrap_or_else(|| robin_util::asset_fs::global().clone()),
    ));
    setup_data_dir(None, &files)?;
    tracing::info!("Robin Hood — Rust entry point (preinstalled shipping data)");
    rust_init_finish(shipping, files)
}

fn rust_init_finish(
    shipping: Option<std::sync::Arc<assets_shipping_datadir::ShippingDatadir>>,
    files: std::sync::Arc<SbFileSystem>,
) -> Result<RustInit, InitError> {
    // The shipping installation owns its VFS. Keep startup mounts, but do
    // not retain the temporary loose-file VFS after discovering shipping data.
    let files = std::sync::Arc::new(match shipping.as_ref() {
        Some(shipping) => files.with_asset_vfs(shipping.asset_vfs().clone()),
        None => files.snapshot(),
    });
    robin_engine::audio_durations::AudioDurations::load(&files)
        .map_err(|message| InitError::ContentAudioDurations { message })?;
    let localization = crate::localization::LocalizationService::initialize_with_files(
        shipping.as_deref(),
        files.clone(),
    )
    .map_err(|source| InitError::ContentLocalization { source })?;
    let options = engine_api::GlobalOptions::default();
    let profiles = std::sync::Arc::new(load_profiles_with_files(
        shipping.as_deref(),
        &options,
        &files,
    )?);
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

    let application_context = ApplicationContext::complete_with_localization_and_files(
        crate::player_profile_store::PlayerProfileStore::for_directory(
            &player_profile_directory.to_string_lossy(),
        ),
        options,
        player_profiles,
        key_configs,
        shipping,
        localization,
        Some(files),
    )
    .and_then(crate::host::ReadyApplicationContext::try_from)
    .map_err(|message| InitError::PlayerProfileState {
        save_directory: player_profile_directory.clone(),
        message,
    })?;
    reconcile_spellforge_trust_after_profile_load(
        &application_context,
        &player_profile_directory,
        player_profiles_regenerated,
    );

    let campaign = Campaign::create(&profiles, application_context.sim_config().difficulty);

    Ok((campaign, profiles, application_context))
}

/// Numeric profile ids scope Spellforge approvals. If the complete profile
/// archive was replaced during recovery, no old id retains its identity, so
/// conservatively revoke every prior grant before any menu or multiplayer
/// admission can observe the regenerated profile.
fn reconcile_spellforge_trust_after_profile_load(
    application_context: &ApplicationContext,
    save_dir: &Path,
    player_profiles_regenerated: bool,
) {
    if !player_profiles_regenerated {
        return;
    }
    if let Err(error) = application_context.reset_spellforge_trust_after_profile_recovery() {
        tracing::error!(
            "Failed to durably reset Spellforge trust after player-profile recovery in {}: {error}. Remote content trust remains unavailable until explicitly reset.",
            save_dir.display()
        );
    }
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
/// TODO(content-loading): The original game profile setup in the
/// Original imports the authored CSV directory and writes `profile.cpf` when
/// the compiled file is absent. The Rust runtime does not yet implement that
/// development fallback, so absence of all three supported representations
/// remains a fatal required-content error.
fn load_profiles_with_files(
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
    options: &engine_api::GlobalOptions,
    files: &SbFileSystem,
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
        return apply_profile_patches_with_files(p.clone(), files);
    }
    // Both the JSON and legacy-CPF paths skip the beam-me post-processing
    // step, so without this call every mission profile ends up with
    // `number_of_beam_mes = 0` / `required_actions` empty — silently
    // hiding required-action glyphs in the briefing UI and breaking
    // auto-gang-selection.  Walk every mission `.rhm` file and fold
    // beam-me action flags into the profile.
    let level_dir = &options.level_directory;

    let json_path = "Data/Configuration/profile.cpf.json";
    if files
        .try_exists(json_path)
        .map_err(|status| InitError::ContentProfilesOpen {
            path: json_path,
            status,
        })?
    {
        tracing::info!("Profiles: loading JSON dump {json_path}");
        let document =
            ProfileManager::load_json_document_with_files(json_path, files).map_err(|source| {
                InitError::ContentProfilesJson {
                    path: json_path,
                    source,
                }
            })?;
        let mut mgr = apply_profile_document_with_files(document, files)?;
        mgr.import_beam_mes_with_files(level_dir, files);
        return Ok(mgr);
    }
    let cpf_path = "Data/Configuration/profile.cpf";
    tracing::info!("Profiles: loading legacy CPF {cpf_path}");
    let mut file = files
        .open(cpf_path)
        .map_err(|status| InitError::ContentProfilesOpen {
            path: cpf_path,
            status,
        })?;
    let mut mgr = ProfileManager::new();
    mgr.load_all_legacy_cpf(&mut file)
        .map_err(|source| InitError::ContentProfilesRead {
            path: cpf_path,
            source,
        })?;
    let mut mgr = apply_profile_patches_with_files(mgr, files)?;
    mgr.import_beam_mes_with_files(level_dir, files);
    Ok(mgr)
}

fn apply_profile_patches_with_files(
    profiles: ProfileManager,
    files: &SbFileSystem,
) -> Result<ProfileManager, InitError> {
    let document = robin_engine::content_patch::profile_document(&profiles).map_err(|message| {
        InitError::ContentProfilePatch {
            path: robin_engine::content_patch::PROFILE_PATCH_PATH.into(),
            message,
        }
    })?;
    apply_profile_document_with_files(document, files)
}

fn apply_profile_document_with_files(
    mut document: serde_json::Value,
    files: &SbFileSystem,
) -> Result<ProfileManager, InitError> {
    robin_engine::content_patch::reject_legacy(
        files,
        "Data/Configuration/soldier-profiles.patch.json",
        robin_engine::content_patch::PROFILE_PATCH_PATH,
    )
    .map_err(|message| InitError::ContentProfilePatch {
        path: "Data/Configuration/soldier-profiles.patch.json".into(),
        message,
    })?;
    let path = robin_engine::content_patch::PROFILE_PATCH_PATH;
    let layers = robin_engine::content_patch::read_layers(files, path).map_err(|message| {
        InitError::ContentProfilePatch {
            path: path.into(),
            message,
        }
    })?;
    for (index, bytes) in layers.iter().enumerate() {
        document = robin_engine::content_patch::apply_profile_document(&document, bytes).map_err(
            |message| InitError::ContentProfilePatch {
                path: path.into(),
                message: format!("layer {index}: {message}"),
            },
        )?;
        tracing::info!("Applied JSON profile patch {path}, layer {index}");
    }
    robin_engine::content_patch::profiles_from_document(document).map_err(|message| {
        InitError::ContentProfilePatch {
            path: path.into(),
            message,
        }
    })
}

/// Load the player-profile service owned by [`ApplicationContext`].
///
/// The boolean reports regeneration so parallel key-config and Spellforge
/// trust stores cannot attach old numeric ids to replacement identities.
fn load_player_profile_manager(save_dir: &Path) -> (PlayerProfileManager, bool) {
    let save_dir_str = save_dir.to_string_lossy().into_owned();

    // Original game behavior: profile loading in
    // The original game recreates the default Robin
    // profile when the player archive is absent or invalid. This recovery is
    // player-state compatibility, not a fallback for required game content.
    match crate::player_profile_store::PlayerProfileStore::for_directory(&save_dir_str).load() {
        Ok(mgr)
            if mgr
                .active_index
                .and_then(|index| mgr.profiles.get(index))
                .is_some() =>
        {
            let regenerated = mgr.default_profiles;
            (mgr, regenerated)
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
/// archive. Creating default profiles marks the manager
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
    if let Err(error) =
        crate::player_profile_store::PlayerProfileStore::for_directory(&manager.save_directory)
            .save(&manager)
    {
        // The original-game launcher also keeps running after creating default profiles
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
    fn invalid_demo_start_text_keeps_reader_local_bootstrap_language_fallback() {
        let root = tempfile::tempdir().unwrap();
        let text = root.path().join("1033/Data/Text");
        let interface = root.path().join("1033/Data/Interface");
        std::fs::create_dir_all(&text).unwrap();
        std::fs::create_dir_all(&interface).unwrap();
        let level_res = b"SRES\0\x01\0\0\0\0\0\0";
        std::fs::write(text.join("Level.res"), level_res).unwrap();
        std::fs::write(interface.join("Start.sxt"), [0, 4, 0, 3]).unwrap();
        let files = std::sync::Arc::new(SbFileSystem::new(std::sync::Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        )));
        assert_eq!(
            files.set_primary_path(root.path().to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        add_language_folder_with_files(&files).unwrap();
        let service =
            crate::localization::LocalizationService::initialize_in_memory_for_test(files.clone())
                .unwrap();
        assert!(service.installed().is_empty());
        assert_eq!(files.locale_paths(), (None, None));
        assert_eq!(files.read_all("Data/Text/Level.res").unwrap(), level_res);
        let other = SbFileSystem::new(std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert!(other.mount_snapshot().alternate_paths.is_empty());
        assert_eq!(
            files
                .mount_snapshot()
                .alternate_paths
                .first()
                .map(String::as_str),
            Some("1033")
        );
    }

    #[test]
    fn profile_preparation_uses_only_the_supplied_reader() {
        let valid_vfs = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
        let invalid_vfs = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
        let path = "Data/Configuration/profile.cpf.json";
        valid_vfs
            .install_preloaded_asset(
                path,
                serde_json::to_vec(
                    &robin_engine::content_patch::profile_document(&ProfileManager::new()).unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
        invalid_vfs
            .install_preloaded_asset(path, b"not profile JSON".to_vec())
            .unwrap();
        let valid = SbFileSystem::new(valid_vfs);
        let invalid = SbFileSystem::new(invalid_vfs);
        let options = engine_api::GlobalOptions::default();
        assert!(load_profiles_with_files(None, &options, &valid).is_ok());
        let error = load_profiles_with_files(None, &options, &invalid).unwrap_err();
        assert!(matches!(error, InitError::ContentProfilesJson { .. }));
        let source = std::error::Error::source(&error).unwrap();
        assert!(source.is::<robin_engine::profiles::ProfileJsonLoadError>());
        assert!(source.source().unwrap().is::<serde_json::Error>());
        assert!(load_profiles_with_files(None, &options, &valid).is_ok());
    }

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
    fn canonical_profile_loader_keeps_authored_keys_and_appends_unlisted_entries() {
        let base = ProfileManager {
            soldiers: vec![engine_profiles::SoldierProfile {
                filename: "Guard".into(),
                life_point: 40,
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut document = robin_engine::content_patch::profile_document(&base).unwrap();
        let guard = document["soldiers"]
            .as_object_mut()
            .unwrap()
            .remove("Guard")
            .unwrap();
        document["soldiers"]["template"] = guard;
        document["soldier_order"][0] = serde_json::json!("template");
        let vfs = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
        vfs.install_preloaded_asset(
            "Data/Configuration/profile.cpf.json",
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
        vfs.install_preloaded_asset(
            robin_engine::content_patch::PROFILE_PATCH_PATH,
            br#"[
                {"op":"copy","from":"/soldiers/template","path":"/soldiers/Zulu"},
                {"op":"replace","path":"/soldiers/Zulu/life_point","value":70},
                {"op":"copy","from":"/soldiers/template","path":"/soldiers/Alpha"},
                {"op":"replace","path":"/soldiers/Alpha/life_point","value":60}
            ]"#
            .to_vec(),
        )
        .unwrap();
        let files = SbFileSystem::new(vfs);
        let profiles =
            load_profiles_with_files(None, &engine_api::GlobalOptions::default(), &files).unwrap();
        assert_eq!(
            profiles
                .soldiers
                .iter()
                .map(|p| p.life_point)
                .collect::<Vec<_>>(),
            vec![40, 60, 70]
        );
    }

    #[test]
    fn profile_patch_loader_applies_actual_data_and_rejects_legacy_files() {
        let profiles = ProfileManager {
            soldiers: vec![engine_profiles::SoldierProfile {
                filename: "Knight03".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let vfs = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
        vfs.install_preloaded_asset(
            robin_engine::content_patch::PROFILE_PATCH_PATH,
            br#"[{"op":"replace","path":"/soldiers/Knight03/life_point","value":150}]"#.to_vec(),
        )
        .unwrap();
        let files = SbFileSystem::new(vfs.clone());
        assert_eq!(
            apply_profile_patches_with_files(profiles.clone(), &files)
                .unwrap()
                .soldiers[0]
                .life_point,
            150
        );
        vfs.install_preloaded_asset(
            "Data/Configuration/soldier-profiles.patch.json",
            b"{}".to_vec(),
        )
        .unwrap();
        let error = apply_profile_patches_with_files(profiles, &files)
            .unwrap_err()
            .to_string();
        assert!(error.contains("no longer supported"), "{error}");
    }

    #[test]
    fn json_profile_patches_compose_across_directory_and_zip_layers() {
        use std::io::Write;
        let patch_path = robin_engine::content_patch::PROFILE_PATCH_PATH;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(patch_path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let patch = |template: &str, filename: &str| {
            serde_json::to_vec(&serde_json::json!([
                {"op":"copy", "from":format!("/soldiers/{template}"), "path":format!("/soldiers/{filename}")},
                {"op":"replace", "path":format!("/soldiers/{filename}/filename"), "value":filename},
                {"op":"replace", "path":format!("/soldiers/{filename}/display_name"), "value":filename}
            ]))
            .unwrap()
        };
        std::fs::write(path, patch("Knight03", "Knight00")).unwrap();
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer
            .start_file(
                format!("Wrapped/{patch_path}"),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(&patch("Knight00", "Knight01")).unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        let files = SbFileSystem::new(std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new()));
        assert_eq!(
            files.add_overlay_path(directory.path().to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        assert_eq!(
            files.add_overlay_zip_bytes_for_mission("patch", bytes.into(), None),
            SBFILE_NO_ERROR
        );
        let mut profiles = ProfileManager::new();
        profiles.soldiers.push(engine_profiles::SoldierProfile {
            filename: "Knight03".into(),
            ..Default::default()
        });
        let profiles = apply_profile_patches_with_files(profiles, &files).unwrap();
        assert_eq!(
            profiles
                .soldiers
                .iter()
                .map(|soldier| soldier.filename.as_str())
                .collect::<Vec<_>>(),
            ["Knight03", "Knight00", "Knight01"]
        );
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
        // Seed corruption directly; the persistence API rejects invalid snapshots.
        std::fs::write(
            directory.path().join("profiles.json"),
            serde_json::to_vec(&invalid).expect("serialize invalid player-profile fixture"),
        )
        .expect("write invalid player-profile fixture");
        let mut stale_key_configs = KeyConfigStore::new(directory_string.clone());
        stale_key_configs.entry_or_default(0);
        stale_key_configs
            .save()
            .expect("write stale key-config fixture");
        let mut stale_trust =
            crate::spellforge_trust::SpellforgeTrustStore::new(directory_string.clone());
        let stale_key = crate::spellforge_trust::SpellforgeTrustKey {
            full_mod_sha256: [7; 32],
            package_sha256: Some([8; 32]),
        };
        stale_trust
            .grant(
                0,
                stale_key,
                crate::spellforge_trust::SpellforgeTrustMetadata {
                    mission: "OldMission".into(),
                    title: "Old mission".into(),
                    claimed_author: "Author".into(),
                    version: "1".into(),
                    source_url: "https://example.invalid/old".into(),
                    license: "CC0".into(),
                    host_endpoint_id: "old-endpoint".into(),
                    package_vm_abi: None,
                    compressed_bytes: 1,
                },
                10,
            )
            .expect("write stale Spellforge trust fixture");

        let (recovered, regenerated) = load_player_profile_manager(directory.path());
        assert!(regenerated);
        assert_eq!(recovered.profiles.len(), 1);
        assert_eq!(
            recovered.get_active().map(|profile| profile.name.as_str()),
            Some("Robin")
        );
        assert!(recovered.default_profiles);

        let persisted =
            crate::player_profile_store::PlayerProfileStore::for_directory(&directory_string)
                .load()
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

        let context = ApplicationContext::complete(
            crate::player_profile_store::PlayerProfileStore::for_directory(&directory_string),
            engine_api::GlobalOptions::default(),
            recovered,
            key_configs,
            None,
        )
        .unwrap();
        reconcile_spellforge_trust_after_profile_load(&context, directory.path(), regenerated);
        let trust = crate::spellforge_trust::SpellforgeTrustStore::load(&directory_string)
            .expect("reload reset trust store");
        assert!(trust.grants_for_profile(0).is_empty());
        assert!(!trust.is_trusted(0, stale_key).unwrap());
    }

    #[test]
    fn absent_player_archive_reports_identity_regeneration() {
        let directory = tempfile::tempdir().expect("temporary player-profile directory");
        let (profiles, regenerated) = load_player_profile_manager(directory.path());
        assert!(regenerated);
        assert!(profiles.default_profiles);
        assert_eq!(profiles.get_active().unwrap().id, 0);
    }
}
