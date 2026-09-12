//! CPU-only official mission assets used by ranked resimulation.
//!
//! The interactive client has additional presentation caches, but the sealed
//! simulation needs only the data assembled here. Every read goes through the
//! mission-owned confined resolver; no renderer, audio device, window, user
//! profile, or network service participates.

use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::path::Path;
use std::sync::Arc;

use robin_assets::frame_holder::{FrameHolder, PublishedFrameHolder};
use robin_assets::picture::Picture;
use robin_assets::resource_manager::ResourceManager;
use robin_engine::campaign::Campaign;
use robin_engine::engine::{Ambiance, GroundMarkSpriteData, LevelAssets, SimConfig};
use robin_engine::profiles::ProfileManager;
use robin_engine::sbfile::SbFileSystem;
use robin_engine::sprite_variant::SpriteVariant;
use robin_run_protocol::SpeechTimingAuthorityV1;

use crate::ranked_verifier::RankedVerifierLoadError;

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
    let ground_mark_sprite = robin_assets::interface_metadata::ground_mark_sprite_data(
        &mut interface,
    )
    .map_err(|error| RankedVerifierLoadError::ResourceArchive {
        path: "Data/Interface/DEFAULT.RES",
        message: format!("{error:#}"),
    })?;
    let titbit_row_frame_counts = robin_assets::interface_metadata::titbit_row_frame_counts(
        &mut interface,
    )
    .map_err(|error| RankedVerifierLoadError::ResourceArchive {
        path: "Data/Interface/DEFAULT.RES",
        message: format!("{error:#}"),
    })?;

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

    (assets.peasant_firstnames, assets.peasant_surnames) =
        robin_assets::original_text::load_peasant_name_pool(&mut text).map_err(|error| {
            RankedVerifierLoadError::ResourceArchive {
                path: "Data/Text/Level.res",
                message: format!("{error:#}"),
            }
        })?;
    assets.fixed_vip_names = robin_assets::original_text::load_fixed_vip_name_map(&mut text)
        .map_err(|error| RankedVerifierLoadError::ResourceArchive {
            path: "Data/Text/Level.res",
            message: format!("{error:#}"),
        })?;

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

    assets.audio.required_exclamation_ids =
        required_mission_exclamation_ids(&loaded, campaign, profiles)
            .map_err(RankedVerifierLoadError::SpeechClosure)?;
    let ambiance_mask = authored_ambiance.to_bitmask();
    assets.audio.sound_source_required_ids = loaded
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
    assets.attachments.pixel_opacity =
        Some(Arc::new(PublishedFrameHolder::new(Arc::new(frame_holder))));

    Ok(RawMissionInputs {
        assets,
        loaded,
        ground_mark_sprite,
        titbit_row_frame_counts,
        bg_pixel_dims,
        level_directory,
    })
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
    let candidates = robin_assets::terrain_source::candidate_paths(
        level_directory,
        ambiance_dir,
        map_name,
        "map",
    );
    let mut picture = None;
    for path in &candidates {
        let Some(mut file) = robin_assets::terrain_source::open_candidate(path, files)? else {
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
        let index = soldier.profile_index(profiles)?;
        let profile = profiles
            .get_soldier(index)
            .expect("resolved soldier profile");
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

fn populate_ranked_sound_duration_tables(
    assets: &mut LevelAssets,
    profiles: &ProfileManager,
    speech_timing: &SpeechTimingAuthorityV1,
    _files: Arc<SbFileSystem>,
) -> Result<(), String> {
    if *speech_timing != SpeechTimingAuthorityV1::CoreAudioDurationsV1 {
        return Err(
            "this engine requires core audio durations; regenerate the ranked content projection"
                .into(),
        );
    }
    // The confined verifier mount contains retail data only. Bind the exact
    // same core timing file to the verifier build; never reopen a user cache,
    // derive local voice lengths, or add an ambient filesystem search root.
    // A missing core file is a build error here and a startup error in the game.
    robin_engine::audio_durations::AudioDurations::from_json(include_bytes!(
        "../../../assets/core-datadir/Data/AudioDurations.json"
    ))?
    .populate(&mut assets.audio, profiles)
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

#[cfg(test)]
mod resource_tests {
    use super::*;

    #[test]
    fn ranked_terrain_rejects_bad_first_candidate_and_does_not_accept_png_overlays() {
        let vfs = Arc::new(robin_util::asset_fs::AssetVfs::new());
        let picture = Picture {
            width: 1,
            height: 1,
            pitch: 2,
            pixel_format: robin_assets::picture::PixelFormat::Rgb16,
            data: vec![0, 0],
            palette: None,
        };
        let bytes = picture
            .write_sixteen_to_bytes(robin_assets::picture::SixteenPacking::None)
            .unwrap();
        vfs.install_preloaded_asset("Levels/Day/Test.map", bytes.clone())
            .unwrap();
        vfs.install_preloaded_asset("Levels/Night/Test.map.png", vec![1])
            .unwrap();
        let files = SbFileSystem::new(vfs.clone());
        assert!(
            decode_background_map("Test", "Night", "Levels", &files)
                .unwrap()
                .is_some()
        );
        vfs.install_preloaded_asset("Levels/Night/Test.map", vec![1])
            .unwrap();
        assert!(decode_background_map("Test", "Night", "Levels", &files).is_err());
        assert!(
            decode_background_map("Test", "Night", "../Levels", &files)
                .err()
                .unwrap()
                .contains("failed to probe")
        );
    }

    #[test]
    fn concurrent_ranked_readers_and_sample_loaders_keep_independent_roots() {
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
                    files.lock_ranked_verifier_primary_path(root).unwrap();
                    for _ in 0..20 {
                        let bytes = files.read_all("src/lib.rs").unwrap();
                        assert_eq!(bytes, expected);
                        assert!(files.read_all("src/../../Cargo.toml").is_err());
                    }
                });
            }
        });
        assert_eq!(std::env::current_dir().unwrap(), before);
    }

    #[test]
    fn captured_ranked_resources_preserve_confined_reads_and_reject_late_ambient_assets() {
        use robin_engine::sbfile::SbFileSystem;
        use robin_engine::sprite_script::{FrameKind, MissionResourceEnvironment, SpriteScriptor};
        use robin_util::asset_fs::{AssetVfs, Bundle};

        let vfs = Arc::new(AssetVfs::new());
        let files = SbFileSystem::new(vfs.clone());
        files
            .lock_ranked_verifier_primary_path(Path::new(env!("CARGO_MANIFEST_DIR")))
            .unwrap();
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
