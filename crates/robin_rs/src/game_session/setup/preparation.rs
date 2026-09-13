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
    MissionError,
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
        .map_err(|error| {
            MissionError::asset(format!("{ambiance:?} background map load failed: {error}"))
        })?;
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

/// Mission resource environment: the mounted shipping archive's, else one over
/// the preparation file system.
pub(super) fn mission_resource_environment(
    host: &Host,
    mission_name: Option<&str>,
    files: &std::sync::Arc<engine_sbfile::SbFileSystem>,
) -> Result<std::sync::Arc<engine_sprite_script::MissionResourceEnvironment>, MissionError> {
    Ok(match host.frontend.resources.shipping.as_ref() {
        Some(shipping) => match mission_name
            .ok_or_else(|| anyhow::anyhow!("shipping launch has no current mission"))
            .and_then(|name| shipping.mission_resource_environment(name))
        {
            Ok(resources) => resources,
            Err(error) => {
                return Err(MissionError::asset(format!(
                    "prepare shipping mission resources: {error:#}"
                )));
            }
        },
        None => {
            std::sync::Arc::new(engine_sprite_script::MissionResourceEnvironment::from_files(files))
        }
    })
}

/// Install the sprite bank from the application asset cache and the
/// hackable character sprites, then publish the bank signature.
pub(super) fn install_mission_sprites(
    host: &mut Host,
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
    assets: &mut LevelAssets,
    files: &std::sync::Arc<engine_sbfile::SbFileSystem>,
    feedback: &mut MissionLoadFeedback<'_>,
    timer: &mut PhaseTimer,
) -> Result<(), MissionError> {
    let (event_pump, loading_screen) = feedback;
    if let Some(ls) = loading_screen.as_mut() {
        ls.set_status("Loading sprite bank...", 0.56);
    }
    {
        let cache_owner = match host.application_context().asset_cache() {
            Ok(cache) => cache,
            Err(message) => {
                return Err(MissionError::application(message));
            }
        };
        let asset_cache = cache_owner.get_or_build(
            host.frontend.resources.shipping.as_deref(),
            profiles,
            files.clone(),
        );
        match asset_cache.sprite_bank.as_ref() {
            Some(bank) => host
                .frontend
                .resources
                .install_frame_holder_before_publication(bank.clone()),
            None => tracing::warn!("Sprite bank unavailable in application asset cache"),
        }
        tick_progress(loading_screen, event_pump.as_deref_mut(), 1.0);
    }
    timer.step("sprite bank from application asset cache");
    let custom_sprites = prepare_custom_character_dirs(campaign, &assets.profile_manager, files)?;
    custom_sprites.install(
        host.frontend
            .resources
            .frame_holder_before_publication_mut(),
        assets.sprite_scriptor_mut(),
    )?;
    timer.step("hackable character preload");
    // Publish the sprite-bank signature into LevelAssets so engine-side
    // sprite-script loaders can detect bank changes.
    assets.bank_signature = host.frontend.resources.frame_holder().signature();
    tick_progress(loading_screen, event_pump.as_deref_mut(), 1.0);
    Ok(())
}

/// Finish the wasm fallback terrain decode inline, then resolve the
/// background pixel dimensions `Engine::new` needs: `(dims, background,
/// minimap, still-pending decode)`.
pub(super) fn resolve_background_dims(
    host: &Host,
    pending_terrain: crate::level_loading_host::PendingTerrainDecode,
    map_name: &str,
    ambiance_dir: &str,
    level_directory: &str,
    files: &std::sync::Arc<engine_sbfile::SbFileSystem>,
    feedback: &mut MissionLoadFeedback<'_>,
) -> Result<
    (
        (f32, f32),
        Option<engine_api::level_loading::PreDecodedBackground>,
        Option<engine_api::level_loading::PreDecodedMinimap>,
        Option<crate::level_loading_host::PendingTerrainDecode>,
    ),
    MissionError,
> {
    let (event_pump, loading_screen) = feedback;
    // Run the single-threaded wasm fallback decode here — the exact point
    // the old synchronous branch used — so the loading bar behaves the same
    // when no worker pool exists. Threaded decodes pass through untouched.
    let pending_terrain = {
        let mut sync_progress = |u: assets_frame_holder::ProgressUpdate| match u {
            assets_frame_holder::ProgressUpdate::Tick(d) => {
                tick_progress(loading_screen, event_pump.as_deref_mut(), d);
            }
            assets_frame_holder::ProgressUpdate::Phase(text, _local) => {
                if let Some(ls) = loading_screen.as_mut() {
                    ls.set_status(text, LOADING_MAP_DECODE_PROGRESS);
                }
            }
        };
        pending_terrain.decode_inline_if_pending(&mut sync_progress)
    };

    // `Engine::new` needs the background bitmap's pixel dimensions to size
    // the fast-find grid. They are probed cheaply from the map header while
    // the decode keeps running; when the probe cannot say (missing/corrupt
    // map, or no map at all) the decode outcome is resolved right here so
    // the existing pre-engine error path reports it.
    let mut pre_decoded_bg: Option<engine_api::level_loading::PreDecodedBackground> = None;
    let mut pre_decoded_mm: Option<engine_api::level_loading::PreDecodedMinimap> = None;
    let install_decoded_terrain =
        |decoded: crate::level_loading_host::DecodedTerrainBitmaps,
         bg: &mut Option<engine_api::level_loading::PreDecodedBackground>,
         mm: &mut Option<engine_api::level_loading::PreDecodedMinimap>|
         -> Result<(f32, f32), MissionError> {
            // TODO(10/F11): leaf returns String (terrain decode worker).
            let background = decoded.background.map_err(MissionError::asset)?;
            let dims = background
                .as_ref()
                .map(|b| (b.width as f32, b.height as f32))
                .unwrap_or((0.0, 0.0));
            *bg = background;
            *mm = decoded.minimap;
            Ok(dims)
        };
    let (bg_pixel_dims, bg_pending) = match pending_terrain.try_take_ready() {
        Ok(decoded) => {
            match install_decoded_terrain(decoded, &mut pre_decoded_bg, &mut pre_decoded_mm) {
                Ok(dims) => (dims, None),
                Err(message) => return Err(message),
            }
        }
        Err(pending) => match pending.known_dimensions().or_else(|| {
            crate::level_loading_host::probe_background_map_dims_with_files(
                map_name,
                ambiance_dir,
                level_directory,
                host.frontend.resources.shipping.as_deref(),
                files,
            )
        }) {
            Some((w, h)) => ((w as f32, h as f32), Some(pending)),
            None => {
                let decoded = pending.join_now_or_redecode(&mut |_| {});
                match install_decoded_terrain(decoded, &mut pre_decoded_bg, &mut pre_decoded_mm) {
                    Ok(dims) => (dims, None),
                    Err(message) => return Err(message),
                }
            }
        },
    };
    Ok((bg_pixel_dims, pre_decoded_bg, pre_decoded_mm, bg_pending))
}

/// Ambiances whose deterministic audio the mission needs: the initial one,
/// plus every scheduled cue when dynamic ambience is enabled (a multiplayer
/// session decides that from its Welcome SimConfig).
pub(super) fn mission_ambiance_mask(
    host: &Host,
    loaded: &robin_engine::level_data::LoadedLevel,
    effective_initial_ambiance: engine_api::Ambiance,
    authoritative_sim_config: engine_api::SimConfig,
) -> u32 {
    let dynamic_ambience_enabled = if host.transport.net().is_some() {
        host.transport
            .mission_sim_config()
            .unwrap_or_else(|| {
                panic!("active multiplayer transport is missing its Welcome SimConfig")
            })
            .enable_dynamic_ambience
    } else {
        authoritative_sim_config.enable_dynamic_ambience
    };
    let mut ambiance_mask = effective_initial_ambiance.to_bitmask();
    if dynamic_ambience_enabled {
        for cue in &loaded.mission.ambience_schedule {
            ambiance_mask |= cue.ambiance.to_bitmask();
        }
    }
    ambiance_mask
}

/// Place the minimap widget for this screen size when the interface has one.
pub(super) fn setup_minimap_widget(
    host: &mut Host,
    minimap_widget: Option<engine_api::MinimapWidgetSetup>,
    screen_width: f32,
    screen_height: f32,
) {
    if let Some(mm) = minimap_widget {
        host.frontend
            .presentation
            .engine_display
            .setup_minimap_widget(
                engine_coordinates::ScreenPoint::new(screen_width - 83.0, 38.0),
                mm.corner_size,
                mm.button_hit_mask,
                screen_width,
                screen_height,
            );
    }
}

/// The engine's starting RNG seed and SimConfig: a multiplayer session's
/// Welcome values, else the launch's authoritative ones.
pub(super) fn session_simulation_start(
    host: &Host,
    authoritative_rng_seed: u64,
    authoritative_sim_config: engine_api::SimConfig,
) -> (u64, engine_api::SimConfig) {
    if host.transport.net().is_some() {
        let rng_seed = host.transport.mission_seed().unwrap_or_else(|| {
            panic!("active multiplayer transport is missing its Welcome mission seed")
        });
        let sim_config = host.transport.mission_sim_config().unwrap_or_else(|| {
            panic!("active multiplayer transport is missing its Welcome SimConfig")
        });
        (rng_seed, sim_config)
    } else {
        (authoritative_rng_seed, authoritative_sim_config)
    }
}

/// Generate the initial sprite variants and shadow key for the presented
/// ambiance and publish pixel opacity; returns `(dynamic_visuals,
/// initial_shadow_key)`.
pub(super) fn publish_initial_sprite_variants(
    host: &mut Host,
    assets: &mut LevelAssets,
    effective_initial_ambiance: engine_api::Ambiance,
    authored_initial_ambiance: engine_api::Ambiance,
    bypass_fog_sprites_crash: bool,
    timer: &mut PhaseTimer,
) -> (bool, u16) {
    let dynamic_visuals = host
        .application_context()
        .with_active_profile(|profile| profile.graphic_config.dynamic_ambience_visuals)
        .unwrap_or_else(|error| {
            panic!("mission presentation preparation requires an active profile: {error}")
        });
    let presentation_initial_ambiance = if dynamic_visuals {
        effective_initial_ambiance
    } else {
        authored_initial_ambiance
    };
    crate::level_loading_host::initialize_sprite_variants_for_ambiance(
        host,
        presentation_initial_ambiance,
        bypass_fog_sprites_crash,
    );
    timer.step("initial sprite variants");
    let (night_r, night_g, night_b) = presentation_initial_ambiance.night_color_rgb();
    let initial_shadow_key = robin_util::color::rgb565(night_r, night_g, night_b);
    host.frontend
        .resources
        .frame_holder_before_publication_mut()
        .apply_arno_law(initial_shadow_key);
    assets.attachments.pixel_opacity = Some(host.frontend.resources.publish_frame_holder_opacity());
    timer.step("initial sprite shadow and opacity publication");
    (dynamic_visuals, initial_shadow_key)
}

pub(super) fn load_mission_binaries(
    host: &Host,
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
    mission_name: Option<&str>,
    level_directory: &str,
    files: &engine_sbfile::SbFileSystem,
    feedback: &mut MissionLoadFeedback<'_>,
) -> Result<robin_engine::level_data::LoadedLevel, MissionError> {
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
            .map_err(|e| MissionError::asset(format!("Level patch failed: {e}")))
    } else {
        engine_api::level_loading::load_mission_for_campaign_with_files(
            campaign,
            profiles,
            level_directory,
            &mut progress,
            &files,
        )
        .map_err(|e| MissionError::asset(format!("Level load failed: {e}")))
    }
}

pub(super) fn prepare_mission_programs(
    resources: &engine_sprite_script::MissionResourceEnvironment,
    assets: &mut LevelAssets,
    mission_name: Option<&str>,
    script_enabled: bool,
    capture_original_save: bool,
) -> Result<Option<assets_scb::ScbFile>, MissionError> {
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
                let program =
                    engine_script_manager::ScriptProgram::from_scb(scb).map_err(|error| {
                        MissionError::script(format!("prepare mission script {name}: {error}"))
                    })?;
                script_programs.insert(name.to_owned(), std::sync::Arc::new(program));
            }
            Err(e)
                if engine_sprite_script::original_mission_program_required(
                    assets,
                    script_enabled,
                ) =>
            {
                return Err(MissionError::script(format!("Mission script {name}: {e}")));
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
) -> Result<crate::level_loading_host::PendingTerrainDecode, MissionError> {
    let early_terrain = match (mission_name, host.frontend.resources.shipping.as_ref()) {
        (Some(mission), Some(shipping)) => {
            let cache = host
                .application_context()
                .asset_cache()
                .map_err(MissionError::application)?;
            match cache.take_early_terrain(shipping, mission, map_name, ambiance_dir) {
                // TODO(10/F11): leaf returns String (early terrain job).
                Some(job) => match job
                    .matches_source(shipping, files, level_directory)
                    .map_err(MissionError::asset)?
                {
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
) -> Result<(), MissionError> {
    assets.audio.required_exclamation_ids =
        required_mission_exclamation_ids(loaded, campaign, profiles)
            .map_err(|error| error.context("Deterministic speech dependency load failed"))?;
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
        .map_err(|error| {
            MissionError::audio(format!("Deterministic audio metadata load failed: {error}"))
        })
}

pub(super) fn prepare_localized_names(
    assets: &mut LevelAssets,
    text_res: &mut ResourceManager,
) -> Result<(), MissionError> {
    (assets.peasant_firstnames, assets.peasant_surnames) = load_peasant_name_pool(text_res)
        .map_err(|error| MissionError::asset(format!("Localized names: {error:#}")))?;
    assets.fixed_vip_names = load_fixed_vip_name_map(text_res)
        .map_err(|error| MissionError::asset(format!("Localized VIP names: {error:#}")))?;
    Ok(())
}
