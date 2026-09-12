//! Mission launch, cold-save admission, and restart boundary policy.

use super::*;

/// Construct the optional custom-mission Lua state before level loading.
/// A Spellforge-tagged launch treats construction as required; only Vanilla
/// custom missions may legitimately produce no session.
pub(crate) fn install_pending_lua_session(
    host: &mut Host,
    args: &crate::main_entry::MissionLaunch,
) -> Result<(), crate::lua_session::SpellforgeSessionError> {
    if let Some(package) = args
        .replay_data
        .as_ref()
        .and_then(|data| data.header().spellforge_package.as_ref())
    {
        let mission = args
            .replay_data
            .as_ref()
            .expect("package came from replay data")
            .header()
            .mission_id
            .clone();
        if !host
            .application_context()
            .with_active_profile(|profile| profile.gameplay_config.enable_spellforge_missions)
            .map_err(crate::lua_session::SpellforgeSessionError::Profile)?
        {
            return Err(crate::lua_session::SpellforgeSessionError::Disabled { mission });
        }
        let session = LuaSession::start_from_package(mission, package.clone())?;
        host.scripting.lua_session = Some(session);
        return Ok(());
    }
    let Some(pending) = args.pending_lua_mission.as_ref() else {
        return Ok(());
    };
    if pending.requires_spellforge
        && !host
            .application_context()
            .with_active_profile(|profile| profile.gameplay_config.enable_spellforge_missions)
            .map_err(crate::lua_session::SpellforgeSessionError::Profile)?
    {
        return Err(crate::lua_session::SpellforgeSessionError::Disabled {
            mission: pending.rhm_basename.clone(),
        });
    }
    if let Some(package) = pending.spellforge_package.as_ref() {
        if !pending.requires_spellforge {
            return Err(
                crate::lua_session::SpellforgeSessionError::UnexpectedPackage {
                    mission: pending.rhm_basename.clone(),
                },
            );
        }
        let session =
            LuaSession::start_from_package(pending.rhm_basename.clone(), package.clone())?;
        host.scripting.lua_session = Some(session);
        return Ok(());
    }
    if pending.requires_spellforge {
        return Err(
            crate::lua_session::SpellforgeSessionError::RequiredSessionMissing {
                mission: pending.rhm_basename.clone(),
            },
        );
    }
    // Vanilla archive missions intentionally have no Lua session. Live and
    // cold Spellforge launchers must supply the already-admitted embedded
    // package above; this boundary never rereads archive paths.
    Ok(())
}

/// Return the exact executable package owned by a preflighted save awaiting
/// application to the freshly constructed mission. No archive-derived or
/// ambient library package is accepted as a substitute.
pub(crate) fn pending_cold_save_lua_launch(
    callbacks: &RustCallbacks,
    args: &crate::main_entry::MissionLaunch,
) -> Result<Option<(String, robin_engine::spellforge::SpellforgePackage)>, String> {
    let Some(SaveLoadRequest::ApplyLoad(load)) = callbacks.pending_request() else {
        return Ok(None);
    };
    let save = load.save();
    let resolved = args.resolved_mission_assets.as_ref().ok_or_else(|| {
        "preflighted save reached engine construction without a resolved mission asset lifetime"
            .to_owned()
    })?;
    if resolved.descriptor() != &save.header.mission_assets {
        return Err(
            "preflighted save descriptor differs from its resolved mission asset lifetime"
                .to_owned(),
        );
    }
    Ok(save.engine.spellforge_package().map(|package| {
        (
            save.header.mission_assets.mission_basename.clone(),
            package.as_ref().clone(),
        )
    }))
}

pub(crate) fn install_cold_save_lua_session(
    host: &mut Host,
    args: &crate::main_entry::MissionLaunch,
    launch: Option<(String, robin_engine::spellforge::SpellforgePackage)>,
) -> Result<(), crate::lua_session::SpellforgeSessionError> {
    let Some((mission, package)) = launch else {
        return Ok(());
    };
    assert!(
        args.replay_data.is_none()
            && args.replay.is_none()
            && args.pending_lua_mission.is_none()
            && args.custom_mission.is_none(),
        "cold save package must be the sole Lua startup authority"
    );
    let session = LuaSession::start_from_package(mission, package)?;
    host.scripting.lua_session = Some(session);
    Ok(())
}

/// Borrow menu resources required by a confirmation or pause-menu action.
///
/// The original game constructs the Really
/// Quit Yes/No menu and changes the game operation only for `YES`; resource
/// absence cannot be interpreted as confirmation.
pub(crate) fn required_menu_resources<'a>(
    resources: &'a Option<IngameMenuResources>,
    context: &str,
) -> &'a IngameMenuResources {
    resources
        .as_ref()
        .unwrap_or_else(|| panic!("{context}: in-game menu resources are missing"))
}

pub(crate) fn selected_pc_profile_indices(
    engine: &engine_api::PresentationView<'_>,
    seat: engine_player_command::PlayerId,
) -> Vec<engine_profiles::CharacterProfileIdx> {
    engine
        .hero_selection(seat)
        .iter()
        .filter_map(|&id| match engine.get_entity(id)? {
            engine_element::Entity::Pc(pc) => Some(pc.pc.profile_index),
            _ => None,
        })
        .collect()
}

/// Ensure a mission that bypassed campaign selection still has an exact
/// save/restart boundary before its Engine exists. Existing session/replay
/// checkpoints are authoritative and are never overwritten.
pub(crate) fn establish_mission_restart_boundary(
    mut campaign: Campaign,
    rng_seed: u64,
    sim_config: engine_api::SimConfig,
) -> Campaign {
    if !campaign.has_restart_simulation_checkpoint() {
        campaign.snapshot_preselected_with_simulation(rng_seed, sim_config);
    }
    campaign
}

/// Restore construction-time simulation controls for a mission restart while
/// retaining profile settings the player deterministically changed during the
/// just-finished attempt. Replay restarts must instead return to their exact
/// header config and let the recorded commands reapply the edits.
pub(super) fn simulation_config_for_level_restart(
    mut checkpoint: engine_api::SimConfig,
    outcome: engine_api::SimConfig,
    replay_restart: bool,
) -> engine_api::SimConfig {
    if !replay_restart {
        checkpoint.amount_of_speaking = outcome.amount_of_speaking;
        checkpoint.enable_unbinding = outcome.enable_unbinding;
        checkpoint.reusable_cloaks = outcome.reusable_cloaks;
        checkpoint.item_gameplay = outcome.item_gameplay;
        checkpoint.noise_distraction_feedback = outcome.noise_distraction_feedback;
        checkpoint.sherwood_trading = outcome.sherwood_trading;
        checkpoint.enable_timed_missions = outcome.enable_timed_missions;
        checkpoint.enable_dynamic_ambience = outcome.enable_dynamic_ambience;
    }
    checkpoint
}

pub(super) fn clear_ambient_custom_launch(args: &mut crate::main_entry::MissionLaunch) {
    args.custom_mission = None;
    args.pending_lua_mission = None;
    args.pending_distributed_mod = None;
}

/// Resolve and mount a save's exact content before consulting the static
/// profile graph. This ordering is the cold-load authority boundary shared by
/// initial menu loads, in-game cross-mission loads, quick-loads, and committed
/// multiplayer snapshot transitions.
pub(super) async fn prepare_cold_save_mission(
    application_context: &ApplicationContext,
    profiles: &mut engine_profiles::ProfileManager,
    save: &crate::save_file::GameSaveFile,
) -> Result<
    (
        usize,
        MissionLocation,
        Arc<crate::mission_asset_restore::ResolvedMissionAssets>,
    ),
    String,
> {
    save.validate_current_schema()
        .map_err(|error| format!("invalid current save schema: {error:#}"))?;
    #[cfg(target_arch = "wasm32")]
    if let Some(link) = &save.header.replay
        && let Err(error) = crate::replay_archive::prepare_browser_directory(std::path::Path::new(
            &link.mission_directory,
        ))
        .await
    {
        // Preserve the existing self-contained save load. The recording owner
        // will explicitly invalidate continuation if its history is missing.
        tracing::warn!("Saved replay history could not be prepared: {error:#}");
    }
    validate_cold_save_spellforge_enabled(application_context, save)?;
    let resolved = resolve_cold_save_mission_assets(application_context, save).await?;
    assert_eq!(
        resolved.descriptor(),
        &save.header.mission_assets,
        "cold save resolver returned a different descriptor"
    );
    // Rebuild custom static state transactionally. A malformed save must not
    // leave a synthetic profile appended after the resolved mount is rejected.
    let mut prepared_profiles = profiles.clone();
    let mission_idx = install_and_validate_saved_profile(&mut prepared_profiles, save)?;
    let location = save.engine.campaign().missions[mission_idx]
        .profile(&prepared_profiles)
        .location;
    *profiles = prepared_profiles;
    Ok((mission_idx, location, Arc::new(resolved)))
}

pub(super) fn validate_cold_save_spellforge_enabled(
    application_context: &ApplicationContext,
    save: &crate::save_file::GameSaveFile,
) -> Result<(), String> {
    validate_cold_save_spellforge_preference(
        application_context,
        &save.header.mission_assets.mission_basename,
        save.engine.spellforge_package().is_some(),
    )
}

pub(super) fn validate_cold_save_spellforge_preference(
    application_context: &ApplicationContext,
    mission_basename: &str,
    requires_spellforge: bool,
) -> Result<(), String> {
    if !requires_spellforge {
        return Ok(());
    }
    let enabled = application_context
        .with_active_profile(|profile| profile.gameplay_config.enable_spellforge_missions)
        .map_err(|error| format!("read Spellforge gameplay preference: {error}"))?;
    if !enabled {
        return Err(format!(
            "Spellforge mission `{mission_basename}` is disabled in Gameplay settings"
        ));
    }
    Ok(())
}

pub(super) async fn resolve_cold_save_mission_assets(
    application_context: &ApplicationContext,
    save: &crate::save_file::GameSaveFile,
) -> Result<crate::mission_asset_restore::ResolvedMissionAssets, String> {
    let descriptor = &save.header.mission_assets;
    let package = save.engine.spellforge_package();
    if matches!(
        descriptor.source,
        robin_engine::mission_assets::MissionAssetSource::BuiltIn
    ) {
        return crate::mission_asset_restore::resolve_built_in_mission_assets(
            descriptor,
            package.as_deref(),
        )
        .map_err(|error| format!("restore built-in save mission assets: {error}"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let roots = crate::mission_asset_restore::MissionAssetRoots::discover();
        let with_cache = application_context.with_distributed_mod_cache_mut(|cache| {
            crate::mission_asset_restore::resolve_native_mission_assets(
                descriptor,
                package.as_deref(),
                &roots,
                Some(cache),
                application_context.preparation_files()?.clone(),
            )
            .map_err(|error| format!("restore exact save mission assets: {error}"))
        });
        match with_cache {
            Ok(resolved) => Ok(resolved),
            Err(cache_error) => crate::mission_asset_restore::resolve_native_mission_assets(
                descriptor,
                package.as_deref(),
                &roots,
                None,
                application_context.preparation_files()?.clone(),
            )
            .map_err(|without_cache| {
                format!(
                    "restore save mission assets without cache: {without_cache}; cache attempt: {cache_error}"
                )
            }),
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        let cache_identity = descriptor
            .archive_assets()
            .and_then(|archive| archive.distributed_cache.as_ref())
            .ok_or_else(|| {
                "browser cold save has no exact durable-cache identity; installed filesystem locators are unavailable"
                    .to_owned()
            })?;
        let lease = crate::distributed_mod_cache::acquire(cache_identity.full_mod_sha256)
            .await?
            .ok_or_else(|| {
                format!(
                    "browser durable cache has no exact save mission object {}",
                    robin_engine::spellforge::hex_hash(&cache_identity.full_mod_sha256)
                )
            })?;
        crate::mission_asset_restore::resolve_cached_mission_assets(
            descriptor,
            package.as_deref(),
            lease,
            application_context.preparation_files()?.clone(),
        )
        .map_err(|error| format!("restore exact browser save mission assets: {error}"))
    }
}

pub(crate) fn install_and_validate_saved_profile(
    profiles: &mut engine_profiles::ProfileManager,
    save: &crate::save_file::GameSaveFile,
) -> Result<usize, String> {
    let descriptor = &save.header.mission_assets;
    let campaign = save.engine.campaign();
    let mission_idx = campaign.current_mission_idx.ok_or_else(|| {
        format!(
            "save campaign has no current mission for descriptor `{}`",
            descriptor.mission_basename
        )
    })?;
    let mission = campaign.missions.get(mission_idx).ok_or_else(|| {
        format!("save current mission index {mission_idx} is outside its campaign")
    })?;
    let profile_idx = mission
        .profile_idx
        .ok_or_else(|| format!("save campaign mission at index {mission_idx} has no profile_idx"))?
        as usize;
    if profile_idx == profiles.missions.len() {
        if descriptor.archive_assets().is_none() {
            return Err(format!(
                "built-in save mission `{}` references missing static profile {profile_idx}",
                descriptor.mission_basename
            ));
        }
        let restored = profiles.add_forced_mission(
            descriptor.proto_level_filename.clone(),
            descriptor.mission_basename.clone(),
            descriptor.mission_basename.clone(),
        ) as usize;
        if restored != profile_idx {
            return Err(format!(
                "forced save profile restored at {restored}, expected serialized index {profile_idx}"
            ));
        }
    } else if profile_idx > profiles.missions.len() {
        return Err(format!(
            "save mission `{}` references missing profile {profile_idx}, but only {} static profiles are installed",
            descriptor.mission_basename,
            profiles.missions.len()
        ));
    }
    let profile = profiles.missions.get(profile_idx).ok_or_else(|| {
        format!("save mission profile {profile_idx} disappeared during reconstruction")
    })?;
    if profile.id != save.header.mission_id {
        return Err(format!(
            "save header mission id {} does not match exact profile id {}",
            save.header.mission_id, profile.id
        ));
    }
    if profile.mission_filename != descriptor.mission_basename
        || profile.proto_level_filename != descriptor.proto_level_filename
    {
        return Err(format!(
            "save mission descriptor {}/{} does not match exact profile {}/{}",
            descriptor.mission_basename,
            descriptor.proto_level_filename,
            profile.mission_filename,
            profile.proto_level_filename
        ));
    }
    crate::main_entry::validate_save_mission(save, profiles)?;
    Ok(mission_idx)
}

/// Run a single mission game loop.
///
/// Creates a Game + Engine, runs frames until the mission ends.
/// Returns the exit GameCode.
/// Prepare the cooperative cross-mission quick-load confirmation.
///
/// Decode and strictly validate the exact queued QuickLoad payload before
/// deciding whether a cross-mission confirmation is required. The decoded
/// bytes are carried into `Load`, so neither a stale `saves.json` entry nor a
/// file replacement after the modal can change what is eventually applied.
pub(super) fn prepare_quickload_cross_mission(
    callbacks: &mut RustCallbacks,
    engine: &Engine,
    game: &crate::game::Game,
    profiles: &engine_profiles::ProfileManager,
    event_pump: &mut GameWindow,
    renderer: &mut Renderer,
    menu_resources: &Option<IngameMenuResources>,
) -> Option<ui_task_state::ActiveUiTask> {
    let use_backup = match callbacks.pending_request() {
        Some(SaveLoadRequest::QuickLoad { use_backup }) => *use_backup,
        _ => return None,
    };
    let slot_name = if use_backup {
        special_slots::EX_QUICK
    } else {
        special_slots::QUICK
    };
    let idx = callbacks.save_manager.find_by_filename(slot_name)?;
    if !callbacks.save_manager.slot_file_exists(idx) {
        return None;
    }
    let load = match callbacks
        .save_manager
        .slot_handle(idx)
        .and_then(|slot| {
            crate::main_entry::PreparedLoad::preflight(&callbacks.save_manager, Some(slot))
        })
        .and_then(|load| load.ok_or_else(|| anyhow::anyhow!("quick-load slot is unavailable")))
    {
        Ok(load) => load,
        Err(error) => {
            tracing::error!("QuickLoad confirmation preflight failed for {slot_name}: {error:#}");
            callbacks.clear_operation();
            return None;
        }
    };
    let save = load.save();
    if let Err(error) = callbacks.save_manager.validate_slot_identity(idx, save) {
        tracing::error!("QuickLoad confirmation rejected stale {slot_name} slot: {error:#}");
        callbacks.clear_operation();
        return None;
    }
    let current = current_mission_id(engine.campaign(), profiles);
    let active_mission_assets = match game.mission_assets() {
        Ok(descriptor) => descriptor,
        Err(error) => {
            tracing::error!("QuickLoad confirmation rejected {slot_name}: {error}");
            callbacks.clear_operation();
            return None;
        }
    };
    let target_mission_id = match validated_save_reload_target(
        save,
        profiles,
        current,
        active_mission_assets,
        engine.spellforge_package().as_deref(),
    ) {
        Ok(target) => target,
        Err(error) => {
            tracing::error!("QuickLoad confirmation rejected {slot_name}: {error}");
            callbacks.clear_operation();
            return None;
        }
    };
    if target_mission_id.is_none() {
        callbacks.queue_operation(SaveLoadRequest::ApplyLoad(load));
        return None;
    }
    let resources = required_menu_resources(menu_resources, "cross-mission QuickLoad confirmation");
    let msg = resources.menu_text.get(MT_MSG_REALLY_LOAD_QUICKSAVE);
    // Remove the intent while the question is open. Accepting restores an
    // exact, already-decoded `Load`; cancelling leaves the queue empty.
    callbacks.clear_operation();
    Some(ui_task_state::ActiveUiTask::QuickLoad(
        ui_task_state::QuickLoadTaskState::new(event_pump, renderer, resources, msg, load),
    ))
}

/// Shared cold-restart policy for graphical and headless direct missions.
/// Replay restarts retain the admitted initial world and configuration; live
/// restarts restore the campaign checkpoint and carry current gameplay options.
pub(super) fn prepare_direct_restart(
    campaign: &mut Campaign,
    args: &mut crate::main_entry::MissionLaunch,
    replay_restart: Option<&(Campaign, u64, engine_api::SimConfig)>,
    outcome_sim_config: engine_api::SimConfig,
) -> Result<(u64, engine_api::SimConfig), String> {
    if let Some((replay_campaign, seed, config)) = replay_restart {
        *campaign = replay_campaign.clone();
        return Ok((
            *seed,
            simulation_config_for_level_restart(*config, outcome_sim_config, true),
        ));
    }
    if !restore_direct_restart_boundary(campaign, args) {
        return Err("direct LevelRestart is missing its preselected mission checkpoint".to_owned());
    }
    let (seed, config) = campaign.restart_simulation_checkpoint();
    Ok((
        seed,
        simulation_config_for_level_restart(config, outcome_sim_config, false),
    ))
}

/// Match campaign-loop handoff policy only after a direct, non-replay host
/// restart has restored its checkpoint. Failed admission never changes policy.
pub(super) fn restore_direct_restart_boundary(
    campaign: &mut Campaign,
    args: &mut crate::main_entry::MissionLaunch,
) -> bool {
    let restored = campaign.restore_snapshot() && campaign.pre_mission_was_preselected;
    carry_direct_restart_multiplayer_continuation(args, restored);
    restored
}

pub(super) fn carry_direct_restart_multiplayer_continuation(
    args: &mut crate::main_entry::MissionLaunch,
    restored_checkpoint: bool,
) {
    if restored_checkpoint && args.server && args.replay.is_none() && args.replay_data.is_none() {
        args.mp_continue_session = true;
    }
}

/// Consume an admitted RPC replay once at a completed mission boundary. The
/// caller owns its launch args so releasing the old asset lease really unmounts
/// that overlay before canonical replay resolution installs a replacement.
pub(super) async fn prepare_pending_direct_replay(
    pending: &mut Option<crate::replay_service::PendingReplay>,
    application_context: &ApplicationContext,
    profiles: &mut engine_profiles::ProfileManager,
    args: &mut crate::main_entry::MissionLaunch,
) -> Result<Option<(Campaign, usize, MissionLocation, u64, engine_api::SimConfig)>, String> {
    let Some(pending) = pending.take() else {
        return Ok(None);
    };
    args.resolved_mission_assets = None;
    clear_ambient_custom_launch(args);
    // This RPC has an explicit pause policy; do not inherit the previous
    // replay's pause request when replacing its owned launch arguments.
    args.start_paused = pending.paused;
    let prepared = prepare_replay_launch(
        application_context,
        profiles,
        args,
        pending.data,
        pending.paused,
    )
    .await
    .map_err(|error| format!("pending direct-mission replay launch failed: {error}"))?;
    *args = prepared.launch;
    Ok(Some((
        prepared.campaign,
        prepared.mission_idx,
        prepared.location,
        prepared.rng_seed,
        prepared.sim_config,
    )))
}

pub(super) fn unprepared_replay_launch_error(
    args: &crate::main_entry::MissionLaunch,
) -> Option<String> {
    if args.replay.is_some() {
        return Some(
            "replay path/compact input reached mission construction before canonical decode and cold asset resolution"
                .to_owned(),
        );
    }
    if args.replay_data.is_some() && args.resolved_mission_assets.is_none() {
        return Some(
            "decoded replay reached mission construction before exact cold asset resolution"
                .to_owned(),
        );
    }
    None
}

#[allow(clippy::too_many_arguments)]

pub(super) async fn ensure_shipping_mission<F>(
    args: &crate::main_entry::MissionLaunch,
    mission: &str,
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
    has_decoded_saved_world: bool,
    progress: F,
) -> Result<(), String>
where
    F: FnMut(crate::shipping_mission::MissionLoadProgress<'_>),
{
    let shipping = args.global_options.shipping_arc()?;
    let Some(datadir) = shipping.as_ref() else {
        return Ok(());
    };
    if !datadir.missions.is_empty()
        && !datadir.has_mission(mission)
        && !robin_engine::level_data::hackable_level_exists(mission)
    {
        return Err(format!(
            "shipping manifest has no payload for required mission {mission}"
        ));
    }
    if !datadir.has_mission(mission) {
        return Ok(());
    }
    crate::shipping_mission::ensure_loaded(
        &args.global_options,
        shipping.as_ref(),
        mission,
        campaign,
        profiles,
        has_decoded_saved_world,
        args.global_options.sound_enabled,
        progress,
    )
    .await
    .map_err(|error| format!("load mission assets for {mission}: {error:#}"))
}

pub(super) fn pending_decoded_saved_world(callbacks: &RustCallbacks) -> bool {
    matches!(
        callbacks.pending_request(),
        Some(SaveLoadRequest::ApplyLoad(_))
    )
}
