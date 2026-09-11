//! CPU-only support for Original trace replay.
//!
//! Keep this crate independent of `robin_rs`: corpus replay needs the
//! simulation and legacy assets, but it does not need a renderer, window,
//! audio device, updater, gamepad, or networking stack.

#![recursion_limit = "256"]

pub mod original_parity_replay;
pub mod result;

use robin_assets::picture::Picture;
use robin_assets::resource_manager::ResourceManager;
use robin_engine::engine::LevelAssets;
use robin_engine::profiles::ProfileManager;
use robin_engine::sbfile::SbFile;

/// Locale-specific directories searched by the Original international build.
pub use robin_assets::original_text::LANGUAGE_FOLDERS;

/// Register localized data paths after a replay tool has entered its datadir.
pub fn register_language_data_paths() {
    let _ = SbFile::add_alternate_path(robin_assets::original_text::FALLBACK_LOCALE_FOLDER);
    for &folder in LANGUAGE_FOLDERS {
        if SbFile::exists(folder) {
            tracing::info!(folder, "detected parity replay language folder");
            let _ = SbFile::add_alternate_path(folder);
            return;
        }
    }
    tracing::info!("no locale folder found; using the 1033 fallback path");
}

/// Populate localized names consumed by deterministic civilian construction.
pub fn populate_localized_names(assets: &mut LevelAssets) -> Result<(), String> {
    let mut resources = ResourceManager::legacy_tool();
    resources
        .attach_resource_file("Data/Text/Level.res")
        .map_err(|error| format!("load Data/Text/Level.res: {error}"))?;

    (assets.peasant_firstnames, assets.peasant_surnames) =
        robin_assets::original_text::load_peasant_name_pool(&mut resources)
            .map_err(|error| format!("{error:#}"))?;
    assets.fixed_vip_names = robin_assets::original_text::load_fixed_vip_name_map(&mut resources)
        .map_err(|error| format!("{error:#}"))?;
    Ok(())
}

/// Read just enough of the terrain to size the simulation grid.
pub fn background_dimensions(
    map_name: &str,
    ambiance_dir: &str,
    level_directory: &str,
) -> Result<(f32, f32), String> {
    if map_name.is_empty() {
        return Err("mission has no background map".to_owned());
    }
    let candidates = robin_assets::terrain_source::candidate_paths(
        level_directory,
        ambiance_dir,
        map_name,
        "map",
    );
    let files = SbFile::snapshot_legacy_file_system();
    for path in &candidates {
        let png_path = format!("{path}.png");
        if let Some(file) = robin_assets::terrain_source::open_candidate(&png_path, &files)? {
            let bytes = file.into_shared_bytes();
            let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
            let reader = decoder
                .read_info()
                .map_err(|error| format!("read terrain PNG header {png_path}: {error}"))?;
            let info = reader.info();
            let width = u16::try_from(info.width)
                .map_err(|_| format!("terrain PNG {png_path} width exceeds u16"))?;
            let height = u16::try_from(info.height)
                .map_err(|_| format!("terrain PNG {png_path} height exceeds u16"))?;
            return Ok((f32::from(width), f32::from(height)));
        }
        let Some(file) = robin_assets::terrain_source::open_candidate(path, &files)? else {
            continue;
        };
        let bytes = file.into_shared_bytes();
        let (width, height) = Picture::terrain_dimensions(&bytes)
            .map_err(|error| format!("read terrain dimensions from {path}: {error}"))?;
        return Ok((f32::from(width), f32::from(height)));
    }
    Err(format!(
        "unable to find map {map_name}; tried {candidates:?}"
    ))
}

/// Use engine-owned timing even for CPU-only trace replay. Recorded Original
/// boundary facts remain explicit replay inputs, not inferred audio durations.
pub fn prepare_core_audio_timing(
    core: &std::path::Path,
) -> Result<robin_engine::audio_durations::AudioDurations, String> {
    // A fresh reader admits only this root: neither legacy mounts nor CWD can
    // supply a missing explicit timing file. Decode once before the runner chdir.
    let files = robin_util::asset_fs::AssetVfs::new();
    files
        .mount_directory(core)
        .map_err(|error| format!("core datadir {}: {error}", core.display()))?;
    let bytes = files
        .read(robin_engine::audio_durations::AUDIO_DURATIONS_PATH)
        .map_err(|error| format!("core datadir {}: {error}", core.display()))?;
    let timing = robin_engine::audio_durations::AudioDurations::from_json(&bytes)
        .map_err(|error| format!("core datadir {}: {error}", core.display()))?;
    use sha2::Digest as _;
    let hash: String = sha2::Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    tracing::info!(core_datadir = %core.display(), audio_durations_sha256 = %hash, "prepared parity core timing");
    Ok(timing)
}

/// Publish the already-admitted timing input without reopening source files.
pub fn populate_sound_duration_tables(
    assets: &mut LevelAssets,
    profiles: &ProfileManager,
    timing: &robin_engine::audio_durations::AudioDurations,
) -> Result<(), String> {
    assets.audio.required_exclamation_ids.extend(
        profiles
            .characters
            .iter()
            .map(|p| p.exclamation_id)
            .chain(profiles.soldiers.iter().map(|p| p.exclamation_id))
            .chain(profiles.civilians.iter().map(|p| p.exclamation_id))
            .filter(|id| *id != 0),
    );
    timing.populate(&mut assets.audio, profiles)
}
