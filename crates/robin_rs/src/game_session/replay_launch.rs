//! Replay-specific launch reconstruction and immutable asset admission.
use super::{ApplicationContext, Campaign, MissionLocation, engine_api, engine_profiles};

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct PreparedReplayLaunch {
    pub(crate) campaign: Campaign,
    pub(crate) mission_idx: usize,
    pub(crate) location: MissionLocation,
    pub(crate) launch: crate::main_entry::MissionLaunch,
    pub(crate) rng_seed: u64,
    pub(crate) sim_config: engine_api::SimConfig,
}

pub(super) fn prepare_replay_mission(
    profiles: &mut engine_profiles::ProfileManager,
    args: &crate::main_entry::MissionLaunch,
    data: robin_engine::replay::ReplayData,
    paused: bool,
) -> Result<PreparedReplayLaunch, String> {
    crate::replay_format::validate_replay_data(&data)
        .map_err(|error| format!("invalid replay: {error}"))?;
    let campaign: Campaign = bitcode::decode(&data.header().campaign)
        .map_err(|error| format!("failed to restore replay campaign: {error}"))?;
    campaign
        .validate_history_schema()
        .map_err(|error| format!("invalid replay campaign history: {error}"))?;
    let mission_id = data.header().mission_id.clone();
    let mission_assets = &data.header().mission_assets;
    let mission_idx = campaign.current_mission_idx.ok_or_else(|| {
        format!("replay campaign has no current mission for header mission `{mission_id}`")
    })?;
    let mission = campaign
        .missions
        .get(mission_idx)
        .ok_or_else(|| format!("replay current mission index {mission_idx} is out of range"))?;
    let profile_idx = mission.profile_idx.ok_or_else(|| {
        format!("replay mission `{mission_id}` at index {mission_idx} has no profile")
    })? as usize;
    if profile_idx == profiles.missions.len() {
        // Forced/custom missions append one synthetic profile immediately
        // before recording starts. That profile is intentionally absent from
        // the freshly loaded base ProfileManager during replay bootstrap, but
        // its index remains in the serialized campaign.
        let restored_idx = profiles.add_forced_mission(
            mission_assets.proto_level_filename.clone(),
            mission_assets.mission_basename.clone(),
            mission_assets.mission_basename.clone(),
        ) as usize;
        assert_eq!(
            restored_idx, profile_idx,
            "forced replay profile must restore its serialized allocation"
        );
    } else if profile_idx > profiles.missions.len() {
        return Err(format!(
            "replay mission `{mission_id}` references missing profile {profile_idx}, but only {} profiles are loaded",
            profiles.missions.len()
        ));
    }
    let profile = campaign.missions[mission_idx].profile(profiles);
    if profile.mission_filename != mission_id {
        return Err(format!(
            "replay campaign mission at index {mission_idx} resolves to `{}`, not header mission `{mission_id}`",
            profile.mission_filename
        ));
    }
    if !profile
        .proto_level_filename
        .eq_ignore_ascii_case(&mission_assets.proto_level_filename)
    {
        return Err(format!(
            "replay campaign mission `{mission_id}` resolves to proto `{}`, not descriptor proto `{}`",
            profile.proto_level_filename, mission_assets.proto_level_filename
        ));
    }
    let location = profile.location;
    let rng_seed = data.header().rng_seed;
    let sim_config = data.header().sim_config;
    // A queued replay can supersede a live custom/multiplayer mission. Its
    // persisted descriptor/package are the sole authority; never let ambient
    // launch metadata trigger a second archive or Lua lookup.
    let replay_args = args.for_replay(data, paused);
    Ok(PreparedReplayLaunch {
        campaign,
        mission_idx,
        location,
        launch: replay_args,
        rng_seed,
        sim_config,
    })
}

/// Resolve the exact immutable mission bytes before consulting the profile
/// manager, then reconstruct the campaign/profile selection from the admitted
/// replay header. Every production replay entry point uses this boundary.
pub(crate) async fn prepare_replay_launch(
    application_context: &ApplicationContext,
    profiles: &mut engine_profiles::ProfileManager,
    args: &crate::main_entry::MissionLaunch,
    data: robin_engine::replay::ReplayData,
    paused: bool,
) -> Result<PreparedReplayLaunch, String> {
    crate::replay_format::validate_replay_data(&data)
        .map_err(|error| format!("invalid replay: {error}"))?;
    if args.mission_start_legacy_save.is_some() {
        return Err(
            "custom/current replay playback cannot be combined with Original parity save capture"
                .to_owned(),
        );
    }
    if data.header().spellforge_package.is_some()
        && !application_context
            .with_active_profile(|profile| profile.gameplay_config.enable_spellforge_missions)?
    {
        return Err(format!(
            "Spellforge mission `{}` is disabled in Gameplay settings; playback did not change the saved preference",
            data.header().mission_assets.mission_basename
        ));
    }

    let resolved = resolve_replay_mission_assets(application_context, &data).await?;
    let mut prepared = prepare_replay_mission(profiles, args, data, paused)?;
    prepared.launch.resolved_mission_assets = Some(std::sync::Arc::new(resolved));
    Ok(prepared)
}

async fn resolve_replay_mission_assets(
    application_context: &ApplicationContext,
    data: &robin_engine::replay::ReplayData,
) -> Result<crate::mission_asset_restore::ResolvedMissionAssets, String> {
    let descriptor = &data.header().mission_assets;
    let package = data.header().spellforge_package.as_ref();
    // Built-in descriptors are validated values, not mounted archives. Resolve
    // them without granting or demanding filesystem authority; actual mission
    // preparation still requires the application's explicit reader.
    if matches!(
        descriptor.source,
        robin_engine::mission_assets::MissionAssetSource::BuiltIn
    ) {
        return crate::mission_asset_restore::resolve_built_in_mission_assets(descriptor, package)
            .map_err(|error| error.to_string());
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let roots = crate::mission_asset_restore::MissionAssetRoots::discover();
        let with_cache = application_context.with_distributed_mod_cache_mut(|cache| {
            crate::mission_asset_restore::resolve_native_mission_assets(
                descriptor,
                package,
                &roots,
                Some(cache),
                application_context.preparation_files()?.clone(),
            )
            .map_err(|error| error.to_string())
        });
        match with_cache {
            Ok(resolved) => Ok(resolved),
            Err(cache_error) => crate::mission_asset_restore::resolve_native_mission_assets(
                descriptor,
                package,
                &roots,
                None,
                application_context.preparation_files()?.clone(),
            )
            .map_err(|without_cache| {
                format!(
                    "restore replay mission assets without cache: {without_cache}; cache attempt: {cache_error}"
                )
            }),
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        use robin_engine::mission_assets::MissionAssetSource;
        match &descriptor.source {
            MissionAssetSource::BuiltIn => {
                crate::mission_asset_restore::resolve_built_in_mission_assets(descriptor, package)
                    .map_err(|error| error.to_string())
            }
            MissionAssetSource::Archive(archive) => {
                let cache_identity = archive.distributed_cache.as_ref().ok_or_else(|| {
                    format!(
                        "browser cold replay `{}` has no exact distributed-cache identity",
                        descriptor.mission_basename
                    )
                })?;
                let lease = crate::distributed_mod_cache::acquire(cache_identity.full_mod_sha256)
                    .await
                    .map_err(|error| format!("acquire browser replay mission cache: {error}"))?
                    .ok_or_else(|| {
                        format!(
                            "browser replay mission cache has no exact object {}",
                            robin_engine::spellforge::hex_hash(&cache_identity.full_mod_sha256)
                        )
                    })?;
                crate::mission_asset_restore::resolve_cached_mission_assets(
                    descriptor,
                    package,
                    lease,
                    application_context.preparation_files()?.clone(),
                )
                .map_err(|error| error.to_string())
            }
        }
    }
}

pub(super) fn choose_pending_replay(
    newly_queued: Option<crate::replay_service::PendingReplay>,
    restart_fallback: &mut Option<crate::replay_service::PendingReplay>,
) -> Option<crate::replay_service::PendingReplay> {
    if newly_queued.is_some() {
        // A newly queued replay supersedes the whole prior replay lifecycle,
        // including its restart copy. Do not leave the old recording armed
        // for a later loop iteration after the new replay exits.
        *restart_fallback = None;
        newly_queued
    } else {
        restart_fallback.take()
    }
}
