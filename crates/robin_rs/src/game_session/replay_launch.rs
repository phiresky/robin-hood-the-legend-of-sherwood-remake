//! Replay-specific launch reconstruction and immutable asset admission.
use super::{
    ApplicationContext, Campaign, MissionError, MissionLocation, engine_api, engine_profiles,
};

// Per-platform replay asset restore; both modules export the same signature.
#[cfg(not(target_arch = "wasm32"))]
#[path = "replay_launch/native.rs"]
mod platform;
#[cfg(target_arch = "wasm32")]
#[path = "replay_launch/wasm.rs"]
mod platform;

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
) -> Result<PreparedReplayLaunch, MissionError> {
    crate::replay_format::validate_replay_data(&data)
        .map_err(|error| MissionError::replay(format!("invalid replay: {error}")))?;
    let campaign: Campaign = bitcode::decode(&data.header().campaign).map_err(|error| {
        MissionError::replay(format!("failed to restore replay campaign: {error}"))
    })?;
    campaign.validate_history_schema().map_err(|error| {
        MissionError::replay(format!("invalid replay campaign history: {error}"))
    })?;
    let mission_id = data.header().mission_id.clone();
    let mission_assets = &data.header().mission_assets;
    let mission_idx = campaign.current_mission_idx.ok_or_else(|| {
        MissionError::replay(format!(
            "replay campaign has no current mission for header mission `{mission_id}`"
        ))
    })?;
    let mission = campaign.missions.get(mission_idx).ok_or_else(|| {
        MissionError::replay(format!(
            "replay current mission index {mission_idx} is out of range"
        ))
    })?;
    let profile_idx = mission.profile_idx.ok_or_else(|| {
        MissionError::replay(format!(
            "replay mission `{mission_id}` at index {mission_idx} has no profile"
        ))
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
        return Err(MissionError::replay(format!(
            "replay mission `{mission_id}` references missing profile {profile_idx}, but only {} profiles are loaded",
            profiles.missions.len()
        )));
    }
    let profile = campaign.missions[mission_idx].profile(profiles);
    if profile.mission_filename != mission_id {
        return Err(MissionError::replay(format!(
            "replay campaign mission at index {mission_idx} resolves to `{}`, not header mission `{mission_id}`",
            profile.mission_filename
        )));
    }
    if !profile
        .proto_level_filename
        .eq_ignore_ascii_case(&mission_assets.proto_level_filename)
    {
        return Err(MissionError::replay(format!(
            "replay campaign mission `{mission_id}` resolves to proto `{}`, not descriptor proto `{}`",
            profile.proto_level_filename, mission_assets.proto_level_filename
        )));
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
) -> Result<PreparedReplayLaunch, MissionError> {
    crate::replay_format::validate_replay_data(&data)
        .map_err(|error| MissionError::replay(format!("invalid replay: {error}")))?;
    if args.mission_start_legacy_save.is_some() {
        return Err(MissionError::launch(
            "custom/current replay playback cannot be combined with Original parity save capture",
        ));
    }
    if data.header().spellforge_package.is_some()
        && !application_context
            .with_active_profile(|profile| profile.gameplay_config.enable_spellforge_missions)
            .map_err(MissionError::application)?
    {
        return Err(MissionError::launch(format!(
            "Spellforge mission `{}` is disabled in Gameplay settings; playback did not change the saved preference",
            data.header().mission_assets.mission_basename
        )));
    }

    let resolved = resolve_replay_mission_assets(application_context, &data).await?;
    let mut prepared = prepare_replay_mission(profiles, args, data, paused)?;
    prepared.launch.resolved_mission_assets = Some(std::sync::Arc::new(resolved));
    Ok(prepared)
}

async fn resolve_replay_mission_assets(
    application_context: &ApplicationContext,
    data: &robin_engine::replay::ReplayData,
) -> Result<crate::mission_asset_restore::ResolvedMissionAssets, MissionError> {
    let descriptor = &data.header().mission_assets;
    let package = data.header().spellforge_package.as_ref();
    // Built-in descriptors are validated values, not mounted archives. Resolve
    // them without granting or demanding filesystem authority; actual mission
    // preparation still requires the application's explicit reader.
    if matches!(
        descriptor.source,
        robin_engine::mission_assets::MissionAssetSource::BuiltIn
    ) {
        return Ok(
            crate::mission_asset_restore::resolve_built_in_mission_assets(descriptor, package)?,
        );
    }

    platform::resolve_non_built_in_mission_assets(application_context, descriptor, package).await
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
