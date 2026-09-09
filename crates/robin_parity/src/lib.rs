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
pub const LANGUAGE_FOLDERS: &[&str] = &[
    "1031", "2047", "1036", "1040", "2070", "3082", "1049", "1041", "1029", "1045", "1046", "1028",
    "1042", "2052", "1054",
];

/// Register localized data paths after a replay tool has entered its datadir.
pub fn register_language_data_paths() {
    let _ = SbFile::add_alternate_path("1033");
    for &folder in LANGUAGE_FOLDERS {
        if SbFile::exists(folder) {
            tracing::info!(folder, "detected parity replay language folder");
            let _ = SbFile::add_alternate_path(folder);
            return;
        }
    }
    tracing::info!("no locale folder found; using the 1033 fallback path");
}

const MENU_TEXT_TABLE_ID: i32 = 1_000_507;
const MENU_TEXT_TABLE_ID_DEMO: i32 = 1_000_040;
const MENU_TEXT_TABLE_ID_DEMO2: i32 = 1_000_034;

fn menu_text_string(resources: &mut ResourceManager, sub_id: usize) -> Option<String> {
    // The first demo's full table is missing entry 53, shifting the following
    // strings by one. This is the same probe used by the interactive client.
    let old_demo = resources
        .get_string(MENU_TEXT_TABLE_ID, 53)
        .map(|value| !value.contains("3D"))
        .unwrap_or(false);
    for table_id in [
        MENU_TEXT_TABLE_ID,
        MENU_TEXT_TABLE_ID_DEMO,
        MENU_TEXT_TABLE_ID_DEMO2,
    ] {
        let effective_sub_id =
            if table_id == MENU_TEXT_TABLE_ID && (54..=166).contains(&sub_id) && old_demo {
                sub_id - 1
            } else {
                sub_id
            };
        if let Ok(value) = resources.get_string(table_id, effective_sub_id) {
            return Some(value.to_owned());
        }
    }
    None
}

/// Populate localized names consumed by deterministic civilian construction.
pub fn populate_localized_names(assets: &mut LevelAssets) -> Result<(), String> {
    let mut resources = ResourceManager::legacy_tool();
    resources
        .attach_resource_file("Data/Text/Level.res")
        .map_err(|error| format!("load Data/Text/Level.res: {error}"))?;

    assets.peasant_firstnames = (100..122)
        .filter_map(|id| menu_text_string(&mut resources, id))
        .collect();
    assets.peasant_surnames = (122..144)
        .filter_map(|id| menu_text_string(&mut resources, id))
        .collect();
    assets.fixed_vip_names = [
        "Robin des bois",
        "Robin des villes",
        "Will Ecarlate",
        "Petit Jean",
        "Frere Tuck",
        "Lady Marianne",
        "Stutely",
    ]
    .into_iter()
    .enumerate()
    .filter_map(|(offset, profile)| {
        menu_text_string(&mut resources, 144 + offset)
            .map(|localized| (profile.to_owned(), localized))
    })
    .collect();
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
    let candidates = [
        format!("{level_directory}/{ambiance_dir}/{map_name}.map"),
        format!("{level_directory}/Day/{map_name}.map"),
        format!("{level_directory}/{map_name}.map"),
    ];
    for path in &candidates {
        let png_path = format!("{path}.png");
        if SbFile::exists(&png_path) {
            let bytes = SbFile::read_all(&png_path)
                .map_err(|status| format!("read terrain PNG {png_path}: status {status}"))?;
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
        let Ok(bytes) = SbFile::read_all(path) else {
            continue;
        };
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
pub fn populate_sound_duration_tables(
    assets: &mut LevelAssets,
    profiles: &ProfileManager,
    _sound_directory: &str,
) -> Result<(), String> {
    let files = SbFile::snapshot_legacy_file_system();
    // TODO: accept an explicit core-datadir path for separately installed parity tools.
    let core = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/core-datadir");
    let status = files.add_overlay_path(core.to_str().ok_or("non-UTF8 core datadir")?);
    if status != robin_engine::sbfile::SBFILE_NO_ERROR
        && status != robin_engine::sbfile::SBFILE_ERROR_PATH_ALREADY_PRESENT
    {
        return Err(format!("cannot mount parity core audio timing: {status}"));
    }
    assets.audio.required_exclamation_ids.extend(
        profiles
            .characters
            .iter()
            .map(|p| p.exclamation_id)
            .chain(profiles.soldiers.iter().map(|p| p.exclamation_id))
            .chain(profiles.civilians.iter().map(|p| p.exclamation_id))
            .filter(|id| *id != 0),
    );
    robin_engine::audio_durations::AudioDurations::load(&files)?
        .populate(&mut assets.audio, profiles)
}
