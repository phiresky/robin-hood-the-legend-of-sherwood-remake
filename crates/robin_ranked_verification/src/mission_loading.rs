//! CPU-only official mission assets used by ranked resimulation.
//!
//! The interactive client has additional presentation caches, but the sealed
//! simulation needs only the data assembled here. Every read goes through the
//! mission-owned confined resolver; no renderer, audio device, window, user
//! profile, or network service participates.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use robin_assets::frame_holder::{FrameHolder, PublishedFrameHolder};
use robin_assets::picture::Picture;
use robin_assets::resource_manager::ResourceManager;
use robin_engine::campaign::Campaign;
use robin_engine::engine::{Ambiance, GroundMarkSpriteData, LevelAssets, SimConfig};
use robin_engine::profiles::{CivilianType, ProfileManager};
use robin_engine::resource_ids::*;
use robin_engine::sbfile::{SB_FILE_READ, SbFileSystem};
use robin_engine::sound::ExclamationGroup;
use robin_engine::sound_cache::{IndexedCache, SampleLoader, SoundCache};
use robin_engine::sprite_variant::SpriteVariant;
use robin_engine::titbit::SpriteRow;
use robin_run_protocol::SpeechTimingAuthorityV1;

use crate::ranked_verifier::RankedVerifierLoadError;

#[cfg(test)]
mod resource_tests {
    use super::*;

    #[test]
    fn concurrent_ranked_readers_and_sample_loaders_keep_independent_roots() {
        use robin_engine::sbfile::SBFILE_NO_ERROR;
        use robin_util::asset_fs::AssetVfs;

        let before = std::env::current_dir().unwrap();
        let roots = [
            Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf(),
            Path::new(env!("CARGO_MANIFEST_DIR")).with_file_name("robin_engine"),
        ];
        let expected = [
            include_bytes!("lib.rs").as_slice(),
            include_bytes!("../../robin_engine/src/lib.rs").as_slice(),
        ];
        std::thread::scope(|scope| {
            for (root, expected) in roots.iter().zip(expected) {
                scope.spawn(move || {
                    let files = SbFileSystem::new(Arc::new(AssetVfs::new()));
                    assert_eq!(
                        files.lock_ranked_verifier_primary_path(root),
                        SBFILE_NO_ERROR
                    );
                    for _ in 0..20 {
                        let (bytes, _, _) = read_sample(&files, "src", "lib.rs").unwrap();
                        assert_eq!(bytes, expected);
                        assert!(read_sample(&files, "src", "../../Cargo.toml").is_none());
                    }
                });
            }
        });
        assert_eq!(std::env::current_dir().unwrap(), before);
    }

    #[test]
    fn captured_ranked_resources_preserve_confined_reads_and_reject_late_ambient_assets() {
        use robin_engine::sbfile::{SBFILE_NO_ERROR, SbFileSystem};
        use robin_engine::sprite_script::{FrameKind, MissionResourceEnvironment, SpriteScriptor};
        use robin_util::asset_fs::{AssetVfs, Bundle};

        let vfs = Arc::new(AssetVfs::new());
        let files = SbFileSystem::new(vfs.clone());
        assert_eq!(
            files.lock_ranked_verifier_primary_path(Path::new(env!("CARGO_MANIFEST_DIR"))),
            SBFILE_NO_ERROR,
        );
        let prepared = files.snapshot();
        let resources = Arc::new(MissionResourceEnvironment::from_files(&prepared));
        vfs.mount_bundle_first(Arc::new(Bundle::from([(
            "cargo.toml".into(),
            b"unapproved ambient replacement".as_slice().into(),
        )])))
        .unwrap();
        assert_eq!(
            resources.read_required_asset("Cargo.toml").unwrap(),
            include_bytes!("../Cargo.toml").as_slice(),
        );
        assert!(
            resources
                .read_required_asset("../robin_engine/Cargo.toml")
                .is_err()
        );
        let scriptor = SpriteScriptor::with_resources(resources);
        let error = scriptor
            .resolve_rhs_path(
                FrameKind::Animation,
                "Data/Animations",
                "deliberately-absent",
                None,
            )
            .unwrap_err();
        assert!(
            !error.contains("unbound"),
            "bound lookup should report absent RHS: {error}"
        );
    }
}

pub(crate) struct RawMissionInputs {
    pub(crate) assets: LevelAssets,
    pub(crate) loaded: robin_engine::level_data::LoadedLevel,
    pub(crate) ground_mark_sprite: Option<GroundMarkSpriteData>,
    pub(crate) titbit_row_frame_counts: Vec<u16>,
    pub(crate) bg_pixel_dims: (f32, f32),
    pub(crate) level_directory: String,
}

pub(crate) fn load_raw_mission_inputs(
    campaign: &Campaign,
    profiles: &ProfileManager,
    options: &robin_engine::engine::GlobalOptions,
    speech_timing: &SpeechTimingAuthorityV1,
    sim_config: SimConfig,
    files: Arc<SbFileSystem>,
) -> Result<RawMissionInputs, RankedVerifierLoadError> {
    let mut text = ResourceManager::with_files(files.clone());
    text.attach_resource_file("Data/Text/Level.res")
        .map_err(|error| RankedVerifierLoadError::ResourceArchive {
            path: "Data/Text/Level.res",
            message: error.to_string(),
        })?;
    let mut interface = ResourceManager::with_files(files.clone());
    interface
        .attach_resource_file("Data/Interface/DEFAULT.RES")
        .map_err(|error| RankedVerifierLoadError::ResourceArchive {
            path: "Data/Interface/DEFAULT.RES",
            message: error.to_string(),
        })?;
    let ground_mark_sprite = extract_ground_mark_sprite_data(&mut interface);
    let titbit_row_frame_counts = extract_titbit_row_frame_counts(&mut interface);

    let mut assets = LevelAssets::new();
    // Admission owns this confined resolver. Preparation and execution share
    // its authority without capturing or changing process-global state.
    let resources =
        Arc::new(robin_engine::sprite_script::MissionResourceEnvironment::from_files(&files));
    assets.sprite_scriptor = Arc::new(robin_engine::sprite_script::SpriteScriptor::with_resources(
        resources,
    ));
    assets.profile_manager = Arc::new(profiles.clone());
    let mut frame_holder = FrameHolder::new();
    frame_holder
        .initialize_sprite_bank_with_progress_and_files(".", &mut |_| {}, None, &files)
        .map_err(|error| RankedVerifierLoadError::SpriteBank(error.to_string()))?;

    let mission_name = campaign
        .current_mission_idx
        .and_then(|index| campaign.missions.get(index))
        .map(|mission| {
            mission
                .profile(&assets.profile_manager)
                .mission_filename
                .clone()
        })
        .ok_or_else(|| {
            RankedVerifierLoadError::Mission("approved campaign has no current mission".into())
        })?;

    // `lock_ranked_verifier_primary_path_with_locale` removes every overlay,
    // so the interactive hackable-RHS loader is deliberately inapplicable.
    // Retail RHS files remain part of the authenticated raw datadir and are
    // loaded below through the engine's canonical Sprite loader.
    assets.bank_signature = frame_holder.signature();
    let script_path = format!("Data/Levels/{mission_name}.scb");
    let script_bytes = files.read_all(&script_path).map_err(|status| {
        RankedVerifierLoadError::MissionScriptRead {
            path: script_path.clone(),
            status,
        }
    })?;
    let script = robin_assets::scb::parse_bytes(&script_bytes).map_err(|error| {
        RankedVerifierLoadError::MissionScriptParse {
            path: script_path.clone(),
            message: error.to_string(),
        }
    })?;
    assets.scripts.mission_programs = Arc::new(BTreeMap::from([(
        mission_name,
        Arc::new(
            robin_engine::script_manager::ScriptProgram::from_scb(script).map_err(|error| {
                RankedVerifierLoadError::MissionScriptParse {
                    path: script_path,
                    message: error.to_string(),
                }
            })?,
        ),
    )]));

    (assets.peasant_firstnames, assets.peasant_surnames) = load_peasant_name_pool(&mut text);
    assets.fixed_vip_names = load_fixed_vip_name_map(&mut text);

    let level_directory = options.level_directory.clone();
    let loaded = robin_engine::engine::level_loading::load_mission_for_campaign_with_files(
        campaign,
        profiles,
        &level_directory,
        &mut |_| {},
        &files,
    )
    .map_err(|error| RankedVerifierLoadError::Mission(error.to_string()))?;
    let authored_ambiance = Ambiance::from_raw(loaded.mission.header.ambiance);
    let background = decode_background_map(
        &loaded.mission.header.map_filename,
        authored_ambiance.directory(),
        &level_directory,
        &files,
    )
    .map_err(RankedVerifierLoadError::Background)?
    .ok_or(RankedVerifierLoadError::MissingBackground)?;
    let bg_pixel_dims = (background.width as f32, background.height as f32);

    assets.required_exclamation_ids = required_mission_exclamation_ids(&loaded, campaign, profiles)
        .map_err(RankedVerifierLoadError::SpeechClosure)?;
    let ambiance_mask = authored_ambiance.to_bitmask();
    assets.sound_source_required_ids = loaded
        .proto
        .sound_sources
        .iter()
        .filter(|source| source.ambience_filter & ambiance_mask != 0)
        .map(|source| source.id as u32)
        .collect();
    populate_ranked_sound_duration_tables(&mut assets, profiles, speech_timing, files)
        .map_err(RankedVerifierLoadError::SoundTiming)?;

    initialize_sprite_variants_for_ambiance(
        &mut frame_holder,
        authored_ambiance,
        sim_config.bypass_fog_sprites_crash,
    );
    let (night_r, night_g, night_b) = authored_ambiance.night_color_rgb();
    frame_holder.apply_arno_law(robin_util::color::rgb565(night_r, night_g, night_b));
    assets.pixel_opacity = Some(Arc::new(PublishedFrameHolder::new(Arc::new(frame_holder))));

    Ok(RawMissionInputs {
        assets,
        loaded,
        ground_mark_sprite,
        titbit_row_frame_counts,
        bg_pixel_dims,
        level_directory,
    })
}

fn extract_ground_mark_sprite_data(
    resources: &mut ResourceManager,
) -> Option<GroundMarkSpriteData> {
    let pictures = resources.get_pictures(RHID_GROUND_FOCUS).ok()?;
    let first = pictures.iter().find_map(Option::as_ref)?;
    let frame_sizes = pictures
        .iter()
        .filter_map(|picture| {
            picture
                .as_ref()
                .map(|picture| (picture.width, picture.height))
        })
        .collect::<Vec<_>>();
    if frame_sizes.is_empty() {
        return None;
    }
    let (width, height) = first
        .opaque_bounds_16()
        .map(|(_, _, width, height)| (width, height))
        .unwrap_or(frame_sizes[0]);
    let per_frame_offsets = pictures
        .iter()
        .map(|picture| {
            picture
                .as_ref()
                .and_then(Picture::opaque_bounds_16)
                .map(|(x, y, _, _)| (x as i16, y as i16))
                .unwrap_or((0, 0))
        })
        .collect();
    Some(GroundMarkSpriteData {
        half_w: width as f32 * 0.5,
        half_h: height as f32 * 0.5,
        frame_sizes,
        per_frame_offsets,
    })
}

fn titbit_sprite_row_resources() -> &'static [(SpriteRow, i32)] {
    &[
        (SpriteRow::Impact, RHID_ONE_STAR),
        (SpriteRow::OneStar, RHID_ONE_STAR),
        (SpriteRow::TwoStars, RHID_TWO_STARS),
        (SpriteRow::ThreeStars, RHID_THREE_STARS),
        (SpriteRow::FourStars, RHID_FOUR_STARS),
        (SpriteRow::FiveStars, RHID_FIVE_STARS),
        (SpriteRow::QuickActionTitbits, RHID_QUICKACTION_TITBITS),
        (SpriteRow::Smoke, RHID_ONE_STAR),
        (SpriteRow::Water, RHID_TITBIT_WATER),
        (SpriteRow::Lock, RHID_TITBIT_WATER),
        (SpriteRow::EmoticonGrowingQMark, RHID_EMOTICONS_WHAT1),
        (SpriteRow::EmoticonQMark, RHID_EMOTICONS_WHAT2),
        (SpriteRow::EmoticonXMark, RHID_EMOTICONS_ACH),
        (SpriteRow::EmoticonZzz, RHIDEMOTICONS_ZZZ),
        (SpriteRow::EmoticonThunderstorm, RHID_EMOTICONS_ANGRY),
        (SpriteRow::EmoticonCloud, RHID_EMOTICONS_DISAPPOINTED),
        (SpriteRow::EmoticonDrunken, RHID_EMOTICONS_DRUNKEN),
        (SpriteRow::EmoticonSun, RHID_EMOTICONS_HAPPY),
        (SpriteRow::EmoticonKo, RHID_EMOTICONS_KO),
        (SpriteRow::Plouf, RHID_TITBIT_PLOUF),
        (SpriteRow::Ghost, RHID_GHOST_LITTLE_JOHN_SHORT_LEGS),
        (SpriteRow::AppleSmell, RHID_TITBIT_APPLE_SMELL),
        (SpriteRow::Speak, RHID_TITBIT_SPEAK),
        (SpriteRow::DangerPoint, RHID_TITBIT_DANGER_POINT),
        (SpriteRow::Hidden, RHID_TITBIT_HIDDEN),
        (SpriteRow::WorkIconArrows, RHWORKICON_ARROWS),
        (SpriteRow::WorkIconPurses, RHWORKICON_PURSES),
        (SpriteRow::WorkIconStones, RHWORKICON_STONES),
        (SpriteRow::WorkIconApples, RHWORKICON_APPLES),
        (SpriteRow::WorkIconBeer, RHWORKICON_BEER),
        (SpriteRow::WorkIconLegs, RHWORKICON_LEGS),
        (SpriteRow::WorkIconPlants, RHWORKICON_PLANTS),
        (SpriteRow::WorkIconNets, RHWORKICON_NETS),
        (SpriteRow::WorkIconWasps, RHWORKICON_WASPS),
        (SpriteRow::WorkIconBowTraining, RHWORKICON_BOW_TRAINING),
        (SpriteRow::WorkIconSwordTraining, RHWORKICON_SWORD_TRAINING),
        (SpriteRow::WorkIconRegeneration, RHWORKICON_REGENERATE),
    ]
}

fn extract_titbit_row_frame_counts(resources: &mut ResourceManager) -> Vec<u16> {
    let mut counts = vec![0; SpriteRow::NumberOfRows as usize];
    for &(row, resource_id) in titbit_sprite_row_resources() {
        let count = resources
            .get_pictures(resource_id)
            .map(|pictures| {
                pictures
                    .iter()
                    .filter(|picture| {
                        picture
                            .as_ref()
                            .is_some_and(|picture| picture.width > 0 && picture.height > 0)
                    })
                    .count() as u16
            })
            .unwrap_or(0);
        counts[row as usize] = count;
    }
    counts
}

const MENU_TEXT_TABLE_ID: i32 = 1_000_507;
const MENU_TEXT_TABLE_ID_DEMO: i32 = 1_000_040;
const MENU_TEXT_TABLE_ID_DEMO2: i32 = 1_000_034;

fn menu_text_string(resources: &mut ResourceManager, sub_id: usize) -> Option<String> {
    let old_demo = resources
        .get_string(MENU_TEXT_TABLE_ID, 53)
        .map(|value| !value.contains("3D"))
        .unwrap_or(false);
    for table_id in [
        MENU_TEXT_TABLE_ID,
        MENU_TEXT_TABLE_ID_DEMO,
        MENU_TEXT_TABLE_ID_DEMO2,
    ] {
        let effective =
            if table_id == MENU_TEXT_TABLE_ID && (54..=166).contains(&sub_id) && old_demo {
                sub_id - 1
            } else {
                sub_id
            };
        if let Ok(value) = resources.get_string(table_id, effective) {
            return Some(value.to_owned());
        }
    }
    None
}

fn load_peasant_name_pool(resources: &mut ResourceManager) -> (Vec<String>, Vec<String>) {
    let firstnames = (100..122)
        .filter_map(|id| menu_text_string(resources, id))
        .collect();
    let surnames = (122..144)
        .filter_map(|id| menu_text_string(resources, id))
        .collect();
    (firstnames, surnames)
}

fn load_fixed_vip_name_map(resources: &mut ResourceManager) -> BTreeMap<String, String> {
    [
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
        menu_text_string(resources, 144 + offset).map(|localized| (profile.to_owned(), localized))
    })
    .collect()
}

fn decode_background_map(
    map_name: &str,
    ambiance_dir: &str,
    level_directory: &str,
    files: &SbFileSystem,
) -> Result<Option<robin_engine::engine::level_loading::PreDecodedBackground>, String> {
    if map_name.is_empty() {
        return Ok(None);
    }
    let candidates = [
        format!("{level_directory}/{ambiance_dir}/{map_name}.map"),
        format!("{level_directory}/Day/{map_name}.map"),
        format!("{level_directory}/{map_name}.map"),
    ];
    let mut picture = None;
    for path in &candidates {
        let Ok(mut file) = files.open(path, SB_FILE_READ) else {
            continue;
        };
        picture = Some(
            Picture::load_terrain_from_stream(&mut file)
                .map_err(|error| format!("failed to decode map {path}: {error}"))?,
        );
        break;
    }
    let Some(picture) = picture else {
        return Err(format!(
            "unable to find map {map_name}; tried {candidates:?}"
        ));
    };
    Ok(Some(
        robin_engine::engine::level_loading::PreDecodedBackground {
            width: picture.width,
            height: picture.height,
            pixels: bytemuck::cast_slice::<u8, u16>(&picture.data).to_vec(),
            // Official manifests currently contain the Original's binary map
            // family. Continuous PNG depth overlays are post-port mod input
            // and cannot be mounted after verifier confinement.
            occlusion_depth: None,
        },
    ))
}

fn required_mission_exclamation_ids(
    loaded: &robin_engine::level_data::LoadedLevel,
    campaign: &Campaign,
    profiles: &ProfileManager,
) -> Result<BTreeSet<u32>, String> {
    let forest_level = loaded
        .proto
        .misc
        .as_ref()
        .is_some_and(|misc| misc.forest_level);
    let normalize = |index: u32| -> Result<usize, String> {
        let profile = profiles
            .characters
            .get(index as usize)
            .ok_or_else(|| format!("required speech character {index} does not exist"))?;
        if !matches!(profile.filename.as_str(), "RobinHood" | "RobinTown") {
            return Ok(index as usize);
        }
        let wanted = if forest_level {
            "RobinHood"
        } else {
            "RobinTown"
        };
        profiles
            .characters
            .iter()
            .position(|profile| profile.filename == wanted)
            .ok_or_else(|| format!("required normalized speech profile {wanted} is absent"))
    };
    let mut ids = BTreeSet::new();
    let add_character = |ids: &mut BTreeSet<u32>, index: u32| -> Result<(), String> {
        let id = profiles.characters[normalize(index)?].exclamation_id;
        if id != 0 {
            ids.insert(id);
        }
        Ok(())
    };
    let mission_index = campaign
        .current_mission_idx
        .ok_or_else(|| "speech closure requires a current campaign mission".to_owned())?;
    let mission = campaign
        .missions
        .get(mission_index)
        .ok_or_else(|| format!("current campaign mission {mission_index} does not exist"))?;
    for &index in &mission.profile(profiles).required_character_indices {
        add_character(&mut ids, index)?;
    }
    for soldier in &loaded.mission.soldiers {
        let profile = profiles
            .soldiers
            .get(soldier.profile_number as usize)
            .ok_or_else(|| {
                format!(
                    "mission soldier references missing speech profile {}",
                    soldier.profile_number
                )
            })?;
        if profile.exclamation_id != 0 {
            ids.insert(profile.exclamation_id);
        }
    }
    for civilian in &loaded.mission.civilians {
        let profile = profiles
            .civilians
            .get(civilian.profile_number as usize)
            .ok_or_else(|| {
                format!(
                    "mission civilian references missing speech profile {}",
                    civilian.profile_number
                )
            })?;
        if profile.exclamation_id != 0 {
            ids.insert(profile.exclamation_id);
        }
    }
    for rescued in &loaded.mission.pcs_to_rescue {
        add_character(&mut ids, rescued.profile_index)?;
    }
    for &character in &campaign.mission_team_indices {
        let description = campaign
            .characters
            .get(character)
            .ok_or_else(|| format!("mission team references missing character {character}"))?;
        let profile = description
            .character_profile_idx
            .ok_or_else(|| format!("mission-team character {character} has no profile"))?;
        add_character(&mut ids, profile.0)?;
    }
    for &character in &campaign.gang_indices {
        let description = campaign
            .characters
            .get(character)
            .ok_or_else(|| format!("gang references missing character {character}"))?;
        if description.instanced {
            continue;
        }
        let profile = description
            .character_profile_idx
            .ok_or_else(|| format!("gang character {character} has no profile"))?;
        if !profiles
            .get_character(profile)
            .ok_or_else(|| format!("gang references missing profile {}", profile.0))?
            .vip
        {
            add_character(&mut ids, profile.0)?;
        }
    }
    Ok(ids)
}

fn build_exclamation_cache(
    profiles: &ProfileManager,
    reader: Arc<SbFileSystem>,
) -> Result<IndexedCache, String> {
    let mut resources = ResourceManager::with_files(reader.clone());
    resources
        .attach_resource_file("Data/Sounds/Exclamations/actors.res")
        .map_err(|error| format!("load authoritative actors.res: {error:#}"))?;
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
        let bytes = match reader.read_all(&path) {
            Ok(bytes) => bytes,
            Err(status) => {
                tracing::warn!(
                    path,
                    status,
                    "authoritative exclamation definition is absent"
                );
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

fn read_sample(files: &SbFileSystem, base: &str, file_name: &str) -> Option<(Vec<u8>, u32, u32)> {
    let normalized = file_name.replace('\\', "/");
    let candidates = [
        format!("{base}/{normalized}"),
        format!("{base}/Exclamations/{normalized}"),
    ];
    let bytes = candidates.iter().find_map(|path| {
        files.read_all(path).ok().or_else(|| {
            let opus = Path::new(path).with_extension("opus");
            files.read_all(&opus.to_string_lossy()).ok()
        })
    })?;
    let size = u32::try_from(bytes.len()).ok()?;
    // Match the interactive sample loader: a readable but unrecognized file
    // remains an admitted sample with zero milliseconds. The simulation then
    // rounds that to its one-frame minimum instead of silently dropping the
    // authored sound identity.
    let duration = wav_or_ogg_duration_ms(&bytes).unwrap_or(0);
    Some((bytes, size, duration))
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

fn populate_ranked_sound_duration_tables(
    assets: &mut LevelAssets,
    profiles: &ProfileManager,
    speech_timing: &SpeechTimingAuthorityV1,
    files: Arc<SbFileSystem>,
) -> Result<(), String> {
    // The resolver has already locked the exact signed locale root. Both
    // authority variants therefore resolve through a single immutable search
    // graph; the variant remains in the prepared-input seal and is checked by
    // ranked content admission.
    match speech_timing {
        SpeechTimingAuthorityV1::BaseInstallation => {}
        SpeechTimingAuthorityV1::LanguagePack { canonical_locale } => {
            if canonical_locale.is_empty() {
                return Err("canonical speech locale is empty".into());
            }
        }
    }
    let speech_cache = build_exclamation_cache(profiles, files.clone())?;
    let loader: Box<SampleLoader> = Box::new(move |name| read_sample(&files, "Data/Sounds", name));
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
    let mut catalog = robin_engine::engine::SpeechTimingCatalog::default();
    for (&group_id, group) in &speech_cache.groups {
        let prefix = group_id & 0xffff_0000;
        if !assets
            .required_exclamation_ids
            .iter()
            .any(|profile| profile & 0xffff_0000 == prefix)
        {
            continue;
        }
        let variants = group
            .entry_indices
            .iter()
            .map(|&index| {
                let entry = speech_cache.entries.get(index).ok_or_else(|| {
                    format!("speech group {group_id:#010x} references missing entry {index}")
                })?;
                Ok(robin_engine::engine::SpeechTimingVariant {
                    sample_identity: entry.file_name.clone(),
                    duration_frames: loader(&entry.file_name)
                        .map(|(_, _, milliseconds)| frames_from_ms(milliseconds)),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let duration = variants
            .iter()
            .filter_map(|variant| variant.duration_frames)
            .max();
        catalog.groups.insert(
            group_id,
            robin_engine::engine::SpeechTimingGroup {
                gaps: group.gaps,
                variants,
            },
        );
        if let Some(duration) = duration {
            for (&profile, groups) in &groups_by_profile {
                if profile & 0xffff_0000 == prefix {
                    for &kind in groups {
                        exclamation_durations.insert((kind, profile, group_id as u16), duration);
                    }
                }
            }
        }
    }

    let mut source_cache = SoundCache::new();
    source_cache.initialize_sound_source_cache(&assets.sound_source_required_ids);
    let source_durations = source_cache
        .source_cache
        .entries
        .iter()
        .filter_map(|(&id, entry)| {
            loader(&entry.file_name).map(|(_, _, milliseconds)| (id, frames_from_ms(milliseconds)))
        })
        .collect();
    assets.exclamation_durations = Arc::new(exclamation_durations);
    assets.speech_timing_catalog = Arc::new(catalog);
    assets.source_durations = Arc::new(source_durations);
    Ok(())
}

fn initialize_sprite_variants_for_ambiance(
    frame_holder: &mut FrameHolder,
    ambiance: Ambiance,
    bypass_fog_sprites_crash: bool,
) {
    if bypass_fog_sprites_crash {
        frame_holder.drop_variant_dictionaries(SpriteVariant::Night);
        frame_holder.drop_variant_dictionaries(SpriteVariant::Fog);
        return;
    }
    match ambiance {
        Ambiance::Fog => {
            frame_holder.drop_variant_dictionaries(SpriteVariant::Night);
            frame_holder.generate_fog_dictionaries();
            frame_holder.set_global_shadow(10);
            frame_holder.set_global_blip_shadow(40);
        }
        Ambiance::Night => {
            frame_holder.drop_variant_dictionaries(SpriteVariant::Fog);
            frame_holder.generate_night_dictionaries();
            frame_holder.set_global_shadow(40);
            frame_holder.set_global_blip_shadow(60);
        }
        _ => {
            frame_holder.drop_variant_dictionaries(SpriteVariant::Night);
            frame_holder.drop_variant_dictionaries(SpriteVariant::Fog);
            frame_holder.set_global_shadow(40);
            frame_holder.set_global_blip_shadow(60);
        }
    }
}
