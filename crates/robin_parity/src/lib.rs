//! CPU-only support for Original trace replay.
//!
//! Keep this crate independent of `robin_rs`: corpus replay needs the
//! simulation and legacy assets, but it does not need a renderer, window,
//! audio device, updater, gamepad, or networking stack.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use robin_assets::picture::Picture;
use robin_assets::resource_manager::ResourceManager;
use robin_engine::engine::LevelAssets;
use robin_engine::profiles::{CivilianType, ProfileManager};
use robin_engine::sbfile::SbFile;
use robin_engine::sound::ExclamationGroup;
use robin_engine::sound_cache::{IndexedCache, SoundCache};

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
    let mut resources = ResourceManager::new();
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

fn exclamation_cache(profiles: &ProfileManager) -> Result<IndexedCache, String> {
    let mut resources = ResourceManager::new();
    resources
        .attach_resource_file("Data/Sounds/Exclamations/actors.res")
        .map_err(|error| format!("load Data/Sounds/Exclamations/actors.res: {error}"))?;
    let mut files = BTreeMap::<u32, String>::new();
    for id in profiles
        .characters
        .iter()
        .map(|profile| profile.exclamation_id)
        .chain(
            profiles
                .soldiers
                .iter()
                .map(|profile| profile.exclamation_id),
        )
        .chain(
            profiles
                .civilians
                .iter()
                .map(|profile| profile.exclamation_id),
        )
        .filter(|id| *id != 0)
    {
        let suffix = id
            .to_le_bytes()
            .into_iter()
            .filter(|byte| *byte != 0)
            .map(char::from)
            .collect::<String>();
        files.insert(id, format!("actor{suffix}.dat"));
    }

    let mut cache = SoundCache::new();
    for (id, filename) in files {
        let path = format!("Data/Sounds/Exclamations/{filename}");
        let bytes = match SbFile::read_all(&path) {
            Ok(bytes) => bytes,
            Err(status) => {
                tracing::warn!(path, status, "exclamation definition is absent");
                continue;
            }
        };
        let (table, exclamations) =
            robin_engine::sound_cache::parse_exclamation_file(&bytes, id & 0xffff_0000)
                .map_err(|error| format!("parse {path}: {error}"))?;
        let mut resolved = Vec::with_capacity(exclamations.len());
        for (action, variants) in exclamations {
            let paths = variants
                .into_iter()
                .filter_map(
                    |variant| match resources.get_sample(table as i32, variant as usize) {
                        Ok(path) => Some(path.to_owned()),
                        Err(error) => {
                            tracing::warn!(table, variant, %error, "actors.res variant is absent");
                            None
                        }
                    },
                )
                .collect();
            resolved.push((action, paths));
        }
        cache.initialize_exclamations_for_profile(&resolved);
    }
    Ok(cache.speech_cache)
}

fn sample_duration_ms(base: &str, file_name: &str) -> Option<u32> {
    let normalized = file_name.replace('\\', "/");
    let candidates = [
        format!("{base}/{normalized}"),
        format!("{base}/Exclamations/{normalized}"),
    ];
    let bytes = candidates.iter().find_map(|path| {
        SbFile::read_all(path).ok().or_else(|| {
            let opus = Path::new(path).with_extension("opus");
            SbFile::read_all(&opus.to_string_lossy()).ok()
        })
    })?;
    // Match the game client's admission rule: readable unknown encodings
    // remain one-frame sounds instead of disappearing from simulation state.
    Some(wav_or_ogg_duration_ms(&bytes).unwrap_or(0))
}

fn wav_or_ogg_duration_ms(bytes: &[u8]) -> Option<u32> {
    if bytes.get(..4)? == b"OggS" {
        let segments = *bytes.get(26)? as usize;
        let body = bytes.get(27 + segments..)?;
        if body.len() < 16 || body[0] != 1 || body.get(1..7)? != b"vorbis" {
            return None;
        }
        let sample_rate = u32::from_le_bytes(body[12..16].try_into().ok()?);
        if sample_rate == 0 {
            return None;
        }
        let mut granule = 0;
        let mut offset = 0;
        while offset + 27 <= bytes.len() {
            if bytes.get(offset..offset + 4) == Some(b"OggS") {
                let value = u64::from_le_bytes(bytes[offset + 6..offset + 14].try_into().ok()?);
                if value != u64::MAX {
                    granule = value;
                }
                let count = bytes[offset + 26] as usize;
                let lacing = bytes.get(offset + 27..offset + 27 + count)?;
                offset += 27 + count + lacing.iter().map(|value| *value as usize).sum::<usize>();
            } else {
                offset += 1;
            }
        }
        return u32::try_from(granule.checked_mul(1000)?.checked_div(sample_rate.into())?).ok();
    }
    if bytes.len() < 44 || bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut offset = 12;
    let mut byte_rate = 0;
    let mut data_size = 0;
    while offset + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().ok()?);
        if bytes.get(offset..offset + 4)? == b"fmt " && offset + 20 <= bytes.len() {
            byte_rate = u32::from_le_bytes(bytes[offset + 16..offset + 20].try_into().ok()?);
        } else if bytes.get(offset..offset + 4)? == b"data" {
            data_size = size;
        }
        offset = offset.checked_add(8 + size as usize)?;
        if !offset.is_multiple_of(2) {
            offset += 1;
        }
    }
    u32::try_from(
        u64::from(data_size)
            .checked_mul(1000)?
            .checked_div(u64::from(byte_rate))?,
    )
    .ok()
}

/// Populate the sound durations that affect simulation without creating an
/// audio device or compiling the interactive audio backend.
pub fn populate_sound_duration_tables(
    assets: &mut LevelAssets,
    profiles: &ProfileManager,
    sound_directory: &str,
) -> Result<(), String> {
    let speech_cache = exclamation_cache(profiles)?;
    let frames_from_ms = |milliseconds: u32| ((milliseconds.saturating_add(39)) / 40).max(1);

    let mut groups_by_profile: BTreeMap<u32, BTreeSet<ExclamationGroup>> = BTreeMap::new();
    for profile in &profiles.characters {
        if profile.exclamation_id != 0 {
            groups_by_profile
                .entry(profile.exclamation_id)
                .or_default()
                .insert(ExclamationGroup::Pc);
        }
    }
    for profile in &profiles.soldiers {
        if profile.exclamation_id != 0 {
            let groups = groups_by_profile.entry(profile.exclamation_id).or_default();
            groups.insert(ExclamationGroup::Civilian);
            groups.insert(ExclamationGroup::Soldier);
            if profile.vip {
                groups.insert(ExclamationGroup::Vip);
            }
        }
    }
    for profile in &profiles.civilians {
        if profile.exclamation_id != 0 {
            let groups = groups_by_profile.entry(profile.exclamation_id).or_default();
            groups.insert(ExclamationGroup::Civilian);
            if profile.civilian_type == CivilianType::Vip {
                groups.insert(ExclamationGroup::Vip);
            }
        }
    }

    let mut exclamation_durations = BTreeMap::new();
    for (&group_id, group) in &speech_cache.groups {
        let prefix = group_id & 0xffff_0000;
        let duration = group
            .entry_indices
            .iter()
            .map(|&index| {
                let entry = speech_cache.entries.get(index).ok_or_else(|| {
                    format!("speech group {group_id:#010x} references missing entry {index}")
                })?;
                let milliseconds = sample_duration_ms(sound_directory, &entry.file_name)
                    .ok_or_else(|| format!("speech sample `{}` is unavailable", entry.file_name))?;
                Ok(frames_from_ms(milliseconds))
            })
            .collect::<Result<Vec<_>, String>>()?
            .into_iter()
            .max()
            .ok_or_else(|| format!("speech group {group_id:#010x} has no samples"))?;
        for (&profile, groups) in &groups_by_profile {
            if profile & 0xffff_0000 == prefix {
                for &kind in groups {
                    exclamation_durations.insert((kind, profile, group_id as u16), duration);
                }
            }
        }
    }

    let mut source_cache = SoundCache::new();
    source_cache.initialize_sound_source_cache(&assets.sound_source_required_ids);
    let mut source_durations = BTreeMap::new();
    for (&id, entry) in &source_cache.source_cache.entries {
        if let Some(milliseconds) = sample_duration_ms(sound_directory, &entry.file_name) {
            source_durations.insert(id, frames_from_ms(milliseconds));
        }
    }
    tracing::info!(
        exclamations = exclamation_durations.len(),
        sources = source_durations.len(),
        "populated headless parity sound durations"
    );
    assets.exclamation_durations = Arc::new(exclamation_durations);
    assets.source_durations = Arc::new(source_durations);
    Ok(())
}
