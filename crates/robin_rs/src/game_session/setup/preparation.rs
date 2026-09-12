//! Independent preparation stages. The caller retains campaign ownership and stage ordering.
use super::*;

pub(super) fn prepare_scheduled_ambiances(
    host: &Host,
    loaded: &robin_engine::level_data::LoadedLevel,
    effective_initial_ambiance: engine_api::Ambiance,
    bg_pixel_dims: (f32, f32),
    level_directory: &str,
    files: &engine_sbfile::SbFileSystem,
    feedback: &mut MissionLoadFeedback<'_>,
) -> Result<
    (
        Vec<(
            engine_api::Ambiance,
            engine_api::level_loading::PreDecodedBackground,
        )>,
        Vec<(
            engine_api::Ambiance,
            engine_api::level_loading::PreDecodedMinimap,
        )>,
    ),
    String,
> {
    let (event_pump, loading_screen) = feedback;
    let map_name = &loaded.mission.header.map_filename;
    let authored_initial_ambiance = engine_api::Ambiance::from_raw(loaded.mission.header.ambiance);
    let mut scheduled_ambiances = Vec::new();
    if authored_initial_ambiance != effective_initial_ambiance {
        scheduled_ambiances.push(authored_initial_ambiance);
    }
    for cue in &loaded.mission.ambience_schedule {
        if cue.ambiance != effective_initial_ambiance
            && !scheduled_ambiances.contains(&cue.ambiance)
        {
            scheduled_ambiances.push(cue.ambiance);
        }
    }
    let mut pre_decoded_ambience_backgrounds = Vec::new();
    let mut pre_decoded_ambience_minimaps = Vec::new();
    for ambiance in scheduled_ambiances {
        let dir = ambiance.directory();
        let mut update = |u: assets_frame_holder::ProgressUpdate| match u {
            assets_frame_holder::ProgressUpdate::Tick(delta) => {
                tick_progress(loading_screen, event_pump.as_deref_mut(), delta);
            }
            assets_frame_holder::ProgressUpdate::Phase(text, _local) => {
                if let Some(screen) = loading_screen.as_mut() {
                    screen.set_status(text, LOADING_MAP_DECODE_PROGRESS);
                }
            }
        };
        let decoded = crate::level_loading_host::pre_decode_background_map_with_files(
            map_name,
            dir,
            level_directory,
            host.frontend.resources.shipping.as_deref(),
            &mut update,
            files,
        )
        .map_err(|error| format!("{ambiance:?} background map load failed: {error}"))?;
        drop(update);
        if let Some(decoded) = decoded {
            let decoded_dims = (decoded.width as f32, decoded.height as f32);
            if bg_pixel_dims == (0.0, 0.0) || decoded_dims == bg_pixel_dims {
                pre_decoded_ambience_backgrounds.push((ambiance, decoded));
            } else {
                tracing::warn!(
                    ?ambiance,
                    ?decoded_dims,
                    ?bg_pixel_dims,
                    "ignoring runtime ambience background with mismatched dimensions"
                );
            }
        }
        let mut progress = |delta: f32| {
            tick_progress(loading_screen, event_pump.as_deref_mut(), delta);
        };
        if let Some(decoded) = crate::level_loading_host::pre_decode_minimap_with_files(
            map_name,
            dir,
            level_directory,
            host.frontend.resources.shipping.as_deref(),
            &mut progress,
            files,
        ) {
            pre_decoded_ambience_minimaps.push((ambiance, decoded));
        }
    }

    Ok((
        pre_decoded_ambience_backgrounds,
        pre_decoded_ambience_minimaps,
    ))
}

pub(super) fn load_mission_binaries(
    host: &Host,
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
    mission_name: Option<&str>,
    level_directory: &str,
    files: &engine_sbfile::SbFileSystem,
    feedback: &mut MissionLoadFeedback<'_>,
) -> Result<robin_engine::level_data::LoadedLevel, String> {
    let (event_pump, loading_screen) = feedback;
    let mut progress = |delta: f32| {
        tick_progress(loading_screen, event_pump.as_deref_mut(), delta);
    };
    if let Some(name) = mission_name
        && let Some(level) = host
            .frontend
            .resources
            .shipping
            .as_ref()
            .and_then(|datadir| datadir.loaded_level(name))
    {
        tracing::info!(mission = name, "level loaded from shipping mission payload");
        progress(1.0);
        progress(1.0);
        engine_api::level_loading::apply_loaded_level_patch(level, name, files)
            .map_err(|e| format!("Level patch failed: {e}"))
    } else {
        engine_api::level_loading::load_mission_for_campaign_with_files(
            campaign,
            profiles,
            level_directory,
            &mut progress,
            &files,
        )
        .map_err(|e| format!("Level load failed: {e}"))
    }
}

pub(super) fn prepare_mission_programs(
    resources: &engine_sprite_script::MissionResourceEnvironment,
    assets: &mut LevelAssets,
    mission_name: Option<&str>,
    script_enabled: bool,
    capture_original_save: bool,
) -> Result<Option<assets_scb::ScbFile>, String> {
    // Shipping programs have already crossed bytecode validation. Retain their
    // Arcs rather than cloning SCBs and decoding a second runtime copy.
    let mut script_programs = resources.programs().clone();
    if let Some(name) = mission_name
        && !script_programs.contains_key(name)
    {
        let path = format!("Data/Levels/{name}.scb");
        match resources
            .read_required_asset(&path)
            .and_then(|b| assets_scb::parse_bytes(&b).map_err(|e| format!("parse {path}: {e}")))
        {
            Ok(scb) => {
                let program = engine_script_manager::ScriptProgram::from_scb(scb)
                    .map_err(|error| format!("prepare mission script {name}: {error}"))?;
                script_programs.insert(name.to_owned(), std::sync::Arc::new(program));
            }
            Err(e)
                if engine_sprite_script::original_mission_program_required(
                    assets,
                    script_enabled,
                ) =>
            {
                return Err(format!("Mission script {name}: {e}"));
            }
            Err(e) => tracing::warn!(
                "Original script intentionally optional for disabled scripting or a prepared replacement: {name}: {e}"
            ),
        }
    }
    let capture_scb = capture_original_save.then(|| {
        let name = mission_name.expect("legacy frame-zero capture requires a current mission");
        script_programs
            .get(name)
            .unwrap_or_else(|| panic!("legacy frame-zero capture has no mission script {name}"))
            .scb()
            .clone()
    });
    assets.scripts.mission_programs = std::sync::Arc::new(script_programs);
    Ok(capture_scb)
}

pub(super) fn start_mission_terrain(
    host: &Host,
    mission_name: Option<&str>,
    map_name: &str,
    ambiance_dir: &str,
    level_directory: &str,
    files: &std::sync::Arc<engine_sbfile::SbFileSystem>,
) -> Result<crate::level_loading_host::PendingTerrainDecode, String> {
    let early_terrain = match (mission_name, host.frontend.resources.shipping.as_ref()) {
        (Some(mission), Some(shipping)) => {
            let cache = host.application_context().asset_cache()?;
            match cache.take_early_terrain(shipping, mission, map_name, ambiance_dir) {
                Some(job) => match job.matches_source(shipping, files, level_directory)? {
                    true => Some(job),
                    false => {
                        tracing::debug!(
                            "discarding early terrain overridden by preparation reader"
                        );
                        None
                    }
                },
                None => None,
            }
        }
        _ => None,
    };
    Ok(if let Some(job) = early_terrain {
        tracing::info!("early terrain decode handed to mission setup");
        crate::level_loading_host::PendingTerrainDecode::Early {
            job,
            level_directory: level_directory.to_owned(),
            shipping: host
                .frontend
                .resources
                .shipping
                .clone()
                .expect("early terrain requires shipping"),
            files: files.clone(),
        }
    } else {
        crate::level_loading_host::PendingTerrainDecode::start_with_files(
            map_name,
            ambiance_dir,
            level_directory,
            host.frontend.resources.shipping.clone(),
            files.clone(),
        )
    })
}

pub(super) fn prepare_deterministic_audio(
    host: &mut Host,
    assets: &mut LevelAssets,
    loaded: &robin_engine::level_data::LoadedLevel,
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
    files: &std::sync::Arc<engine_sbfile::SbFileSystem>,
    ambiance_mask: u32,
) -> Result<(), String> {
    assets.audio.required_exclamation_ids =
        required_mission_exclamation_ids(loaded, campaign, profiles)
            .map_err(|error| format!("Deterministic speech dependency load failed: {error}"))?;
    assets.audio.sound_source_required_ids = loaded
        .proto
        .sound_sources
        .iter()
        .filter(|source| source.ambience_filter & ambiance_mask != 0)
        .map(|source| source.id as u32)
        .collect();
    initialize_mission_sound_caches(
        host,
        profiles,
        &assets.audio.sound_source_required_ids,
        files.clone(),
    )?;
    robin_engine::audio_durations::AudioDurations::load(files)
        .and_then(|timing| timing.populate(&mut assets.audio, profiles))
        .map_err(|error| format!("Deterministic audio metadata load failed: {error}"))
}

pub(super) fn prepare_localized_names(
    assets: &mut LevelAssets,
    text_res: &mut ResourceManager,
) -> Result<(), String> {
    (assets.peasant_firstnames, assets.peasant_surnames) =
        load_peasant_name_pool(text_res).map_err(|error| format!("Localized names: {error:#}"))?;
    assets.fixed_vip_names = load_fixed_vip_name_map(text_res)
        .map_err(|error| format!("Localized VIP names: {error:#}"))?;
    Ok(())
}
