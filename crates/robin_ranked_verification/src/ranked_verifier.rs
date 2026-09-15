//! Official-datadir loader for isolated ranked replay verification.
//!
//! This is deliberately separate from interactive/headless session startup:
//! it accepts no user profile, overlay, persistence, network, renderer, or
//! audio-device state. Each preparation is irreversibly confined to one raw
//! official datadir before any legacy loader runs.

use std::path::Path;
use std::sync::Arc;

use robin_engine::engine::{LevelAssets, RankedSimulationPolicy, SimConfig};
use robin_engine::profiles::ProfileManager;
use robin_engine::sbfile::{SbFileError, SbFileSystem};

use super::replay_campaign_validation::{
    ApprovedReplayEngine, ReplayCampaignApprovedContentMetadata,
    decode_and_validate_replay_campaign, derive_replay_campaign_approved_content_metadata,
    validate_replay_campaign_approved_content,
};
use crate::mission_loading::load_raw_mission_inputs;

/// Engine resources retained for the complete deterministic resimulation.
pub struct PreparedRankedReplayMission {
    engine: ApprovedReplayEngine,
    assets: LevelAssets,
    /// Exact shipping mission route selected from the confined official
    /// profile catalog and the RHM bytes which produced `engine`.
    approved_mission_assets: robin_engine::mission_assets::MissionAssetDescriptor,
}

impl PreparedRankedReplayMission {
    pub const fn starting_campaign_score(&self) -> i32 {
        self.engine.starting_campaign_score()
    }

    /// Return the exact BuiltIn descriptor derived from official content, so
    /// the worker binds the replay's asset identity to the content route it
    /// actually loaded rather than trusting uploader-provided filenames.
    pub const fn approved_mission_assets(
        &self,
    ) -> &robin_engine::mission_assets::MissionAssetDescriptor {
        &self.approved_mission_assets
    }

    pub fn into_engine_and_assets(self) -> (ApprovedReplayEngine, LevelAssets) {
        (self.engine, self.assets)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RankedVerifierLoadError {
    #[error("cannot confine the mission-owned asset resolver (status {0})")]
    AssetResolverConfinement(SbFileError),
    #[error("load official profiles: {0}")]
    Profiles(#[source] crate::profile_loading::ProfileLoadError),
    #[error("validate submitted campaign: {0}")]
    Campaign(#[from] super::replay_campaign_validation::ReplayCampaignValidationError),
    #[error("load deterministic resource archive {path}: {message}")]
    ResourceArchive { path: &'static str, message: String },
    #[error("load official sprite bank: {0}")]
    SpriteBank(String),
    #[error("load mission script {path}: status {status}")]
    MissionScriptRead { path: String, status: SbFileError },
    #[error("parse mission script {path}: {message}")]
    MissionScriptParse { path: String, message: String },
    #[error("load official mission: {0}")]
    Mission(String),
    #[error("decode official background map: {0}")]
    Background(String),
    #[error("mission has no decodable background map")]
    MissingBackground,
    #[error("resolve deterministic speech closure: {0}")]
    SpeechClosure(String),
    #[error("resolve deterministic sound timing: {0}")]
    SoundTiming(String),
    #[error("official profiles have no unique `Sherwood` headquarters mission")]
    MissingSherwoodProfile,
    #[error("initialize clean Sherwood reference engine: {0}")]
    SherwoodReferenceEngine(robin_engine::engine::EngineError),
    #[error("derive Sherwood campaign metadata: {0}")]
    SherwoodMetadata(
        #[from] super::replay_campaign_validation::ReplayCampaignMetadataDerivationError,
    ),
    #[error("validate submitted campaign against official content: {0}")]
    CampaignContent(
        #[from] super::replay_campaign_validation::ReplayCampaignContentValidationError,
    ),
    #[error("prepare ranked engine: {0}")]
    Engine(#[from] robin_engine::engine::EngineError),
}

/// Confine a fresh resolver to one raw official datadir and exactly one
/// resource locale directory (for example `1033`).
pub fn confined_official_files(
    raw_content_root: &Path,
    resource_locale_root: &str,
) -> Result<Arc<SbFileSystem>, RankedVerifierLoadError> {
    let files = Arc::new(SbFileSystem::new(Arc::new(
        robin_util::asset_fs::AssetVfs::new(),
    )));
    files
        .lock_ranked_verifier_primary_path_with_locale(raw_content_root, resource_locale_root)
        .map_err(RankedVerifierLoadError::AssetResolverConfinement)?;
    Ok(files)
}

/// Load the official profile catalog from a confined resolver.
pub fn load_official_profiles(
    files: &SbFileSystem,
) -> Result<ProfileManager, RankedVerifierLoadError> {
    crate::profile_loading::load_profiles(&robin_engine::engine::GlobalOptions::default(), files)
        .map_err(RankedVerifierLoadError::Profiles)
}

/// Load and construct the exact engine which may execute a ranked replay.
pub fn prepare_ranked_replay_mission(
    files: Arc<SbFileSystem>,
    profiles: &ProfileManager,
    starting_campaign_bytes: &[u8],
    mission_id: &str,
    replay_limits: &robin_replay_format::ReplayAdmissionLimits,
    simulation_policy: RankedSimulationPolicy,
    rng_seed: u64,
    sim_config: SimConfig,
) -> Result<PreparedRankedReplayMission, RankedVerifierLoadError> {
    let options = robin_engine::engine::GlobalOptions::default();
    let validated_campaign = decode_and_validate_replay_campaign(
        starting_campaign_bytes,
        mission_id,
        profiles,
        replay_limits,
    )?;
    validated_campaign.validate_checkpoint_simulation_authority(sim_config)?;
    let campaign = validated_campaign.campaign_for_approved_loading();
    let mut submitted =
        load_raw_mission_inputs(campaign, profiles, &options, sim_config, files.clone())?;
    let mission_index = campaign.current_mission_idx.ok_or_else(|| {
        RankedVerifierLoadError::Mission(
            "approved campaign has no current mission for asset binding".into(),
        )
    })?;
    let profile = campaign
        .missions
        .get(mission_index)
        .map(|mission| mission.profile(profiles))
        .ok_or_else(|| {
            RankedVerifierLoadError::Mission(
                "approved campaign current mission is absent for asset binding".into(),
            )
        })?;
    let approved_mission_assets = robin_engine::mission_assets::MissionAssetDescriptor::built_in(
        profile.mission_filename.clone(),
        profile.proto_level_filename.clone(),
        submitted.loaded.mission.header.map_filename.clone(),
    )
    .map_err(|error| {
        RankedVerifierLoadError::Mission(format!(
            "approved mission asset descriptor is invalid: {error}"
        ))
    })?;

    let sherwood = if validated_campaign.deferred_content_checks().is_empty() {
        None
    } else {
        Some(derive_sherwood_metadata(
            profiles, &options, sim_config, files,
        )?)
    };
    let approved =
        validate_replay_campaign_approved_content(validated_campaign, sherwood.as_ref())?;
    let engine = approved.construct_ranked_engine(
        robin_engine::engine::LevelLoadArgs {
            assets: &mut submitted.assets,
            level_directory: &submitted.level_directory,
            progress: &mut |_| {},
            loaded: submitted.loaded,
            bg_pixel_dims: submitted.bg_pixel_dims,
        },
        submitted.ground_mark_sprite,
        submitted.titbit_row_frame_counts,
        rng_seed,
        sim_config,
        simulation_policy,
    )?;
    Ok(PreparedRankedReplayMission {
        engine,
        assets: submitted.assets,
        approved_mission_assets,
    })
}

fn derive_sherwood_metadata(
    profiles: &ProfileManager,
    options: &robin_engine::engine::GlobalOptions,
    sim_config: SimConfig,
    files: Arc<SbFileSystem>,
) -> Result<ReplayCampaignApprovedContentMetadata, RankedVerifierLoadError> {
    let mut sherwood_indices =
        profiles
            .missions
            .iter()
            .enumerate()
            .filter_map(|(index, profile)| {
                (profile.location == robin_engine::profiles::MissionLocation::Sherwood
                    && profile.mission_filename == "Sherwood")
                    .then_some(index)
            });
    let sherwood_index = sherwood_indices
        .next()
        .ok_or(RankedVerifierLoadError::MissingSherwoodProfile)?;
    if sherwood_indices.next().is_some() {
        return Err(RankedVerifierLoadError::MissingSherwoodProfile);
    }

    // This campaign is built only from the official profile catalog. No byte
    // or value from the submitted campaign can influence the topology
    // authority.
    let mut clean_campaign = robin_engine::campaign::Campaign::from_profiles(
        profiles,
        robin_engine::player_profile::DifficultyLevel::Medium,
    );
    clean_campaign.current_mission_idx = Some(sherwood_index);
    clean_campaign.add_all_to_mission_team();

    let mut reference =
        load_raw_mission_inputs(&clean_campaign, profiles, options, sim_config, files)?;
    let derivation_level = reference.loaded.clone();
    // A fixed verifier-owned seed avoids turning uploader-selected randomness
    // into content authority. Sherwood production-zone registration is
    // authored startup behavior and must be stable under this clean bootstrap.
    const SHERWOOD_METADATA_SEED: u64 = 0x7268_2d68_712d_7631;
    let engine = robin_engine::engine::Engine::new(robin_engine::engine::EngineArgs {
        campaign: clean_campaign,
        level: robin_engine::engine::LevelLoadArgs {
            assets: &mut reference.assets,
            level_directory: &reference.level_directory,
            progress: &mut |_| {},
            loaded: reference.loaded,
            bg_pixel_dims: reference.bg_pixel_dims,
        },
        ground_mark_sprite: reference.ground_mark_sprite,
        titbit_row_frame_counts: reference.titbit_row_frame_counts,
        rng_seed: SHERWOOD_METADATA_SEED,
        original_rng_replay: None,
        sim_config,
    })
    .map_err(RankedVerifierLoadError::SherwoodReferenceEngine)?;
    derive_replay_campaign_approved_content_metadata(&derivation_level, engine.campaign())
        .map_err(RankedVerifierLoadError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::campaign::Campaign;
    use robin_engine::player_profile::DifficultyLevel;
    use robin_engine::sector_production::Point;

    const TEST_SEED: u64 = 0x7268_2d76_6572_7631;

    fn fixture_policy() -> RankedSimulationPolicy {
        RankedSimulationPolicy::standard_medium()
    }

    fn operator_datadir(name: &str) -> std::path::PathBuf {
        if let Some(root) = std::env::var_os("ROBINHOOD_DATA_DIR") {
            return root.into();
        }
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../datadirs")
            .join(name)
    }

    fn campaign_for_mission(
        profiles: &ProfileManager,
        mission_id: &str,
        sim_config: SimConfig,
    ) -> Campaign {
        let mission_index = profiles
            .missions
            .iter()
            .position(|profile| profile.mission_filename.eq_ignore_ascii_case(mission_id))
            .unwrap_or_else(|| panic!("real profile catalog has no mission `{mission_id}`"));
        let mut campaign = Campaign::from_profiles(profiles, DifficultyLevel::Medium);
        campaign.current_mission_idx = Some(mission_index);
        campaign.add_all_to_mission_team();
        campaign.snapshot_with_simulation(TEST_SEED, sim_config);
        campaign
    }

    fn prepare(
        root: &Path,
        locale: &str,
        campaign: &Campaign,
        mission_id: &str,
    ) -> Result<PreparedRankedReplayMission, RankedVerifierLoadError> {
        let files = confined_official_files(root, locale).expect("confine real-data test");
        let profiles = load_official_profiles(&files).expect("load real-data profiles");
        prepare_ranked_replay_mission(
            files,
            &profiles,
            &bitcode::encode(campaign),
            mission_id,
            &robin_replay_format::ReplayAdmissionLimits::default(),
            fixture_policy(),
            TEST_SEED,
            fixture_policy().expected_config(),
        )
    }

    #[test]
    #[ignore = "requires operator-mounted demo Leicester data"]
    fn real_demo_field_constructs_a_ranked_engine_from_raw_content() {
        let root = operator_datadir("demo_leicester_ecoste");
        let files = confined_official_files(&root, "1033").unwrap();
        let profiles = load_official_profiles(&files).unwrap();
        let campaign =
            campaign_for_mission(&profiles, "Dem_Lei_MP", fixture_policy().expected_config());
        let preparation =
            prepare(&root, "1033", &campaign, "Dem_Lei_MP").expect("real demo preparation");
        assert_eq!(
            preparation.starting_campaign_score(),
            campaign.get_value(robin_engine::campaign::CampaignValue::Score)
        );
    }

    #[test]
    #[ignore = "requires operator-mounted full retail data"]
    fn real_full_ranked_startup_matches_native_frame_zero() {
        let root = operator_datadir("fullgame_gog");
        let files = confined_official_files(&root, "2047").unwrap();
        let profiles = load_official_profiles(&files).unwrap();
        let config = fixture_policy().expected_config();
        let campaign = campaign_for_mission(&profiles, "H01_Lin_VL", config);
        let mut reference = load_raw_mission_inputs(
            &campaign,
            &profiles,
            &robin_engine::engine::GlobalOptions::default(),
            config,
            files,
        )
        .unwrap();
        let mut native = robin_engine::engine::Engine::new(robin_engine::engine::EngineArgs {
            campaign: campaign.clone(),
            level: robin_engine::engine::LevelLoadArgs {
                assets: &mut reference.assets,
                level_directory: &reference.level_directory,
                progress: &mut |_| {},
                loaded: reference.loaded,
                bg_pixel_dims: reference.bg_pixel_dims,
            },
            ground_mark_sprite: reference.ground_mark_sprite,
            titbit_row_frame_counts: reference.titbit_row_frame_counts,
            rng_seed: TEST_SEED,
            original_rng_replay: None,
            sim_config: config,
        })
        .unwrap();
        let unprepared_hash = robin_engine::replay::state_hash(&native);
        native
            .register_mission_peasant_names(&reference.assets, &mut std::array::from_fn(|_| None))
            .unwrap();
        native
            .connect_initial_seat(
                &reference.assets,
                robin_engine::player_command::PlayerId::HOST,
                String::new(),
            )
            .unwrap();
        let expected = robin_engine::replay::state_hash(&native);
        assert_ne!(expected, unprepared_hash);

        let (approved, _) = prepare(&root, "2047", &campaign, "H01_Lin_VL")
            .unwrap()
            .into_engine_and_assets();
        let (ranked, _, _, _) = approved.into_parts();
        assert_eq!(robin_engine::replay::state_hash(&ranked), expected);
        assert_eq!(ranked.frame_counter(), 0);
        assert_eq!(ranked.rng_seed(), native.rng_seed());
    }

    #[test]
    #[ignore = "requires operator-mounted full retail data"]
    fn real_full_hq_campaign_validates_against_clean_sherwood_metadata() {
        let root = operator_datadir("fullgame_linux");
        let locale = "2047";
        let files = confined_official_files(&root, locale).unwrap();
        let profiles = load_official_profiles(&files).unwrap();
        let sim_config = fixture_policy().expected_config();
        let metadata = derive_sherwood_metadata(
            &profiles,
            &robin_engine::engine::GlobalOptions::default(),
            sim_config,
            files,
        )
        .expect("derive clean real Sherwood metadata");
        let sector = metadata
            .production_sectors
            .first()
            .expect("real Sherwood has production topology");
        let authored_point = sector.polygon[0];

        let mut hq_campaign = campaign_for_mission(&profiles, "Sherwood", sim_config);
        hq_campaign.production_sectors[0]
            .production_points
            .push(Point {
                x: authored_point.x,
                y: authored_point.y,
                layer: sector.topology.map_layer,
                sector: sector.topology.sector,
                obstacle: None,
            });
        hq_campaign.snapshot_with_simulation(TEST_SEED, sim_config);
        prepare(&root, locale, &hq_campaign, "Sherwood").expect("real full HQ preparation");

        let mut forged = hq_campaign;
        forged.production_sectors[0].production_points[0].sector = u16::MAX;
        forged.snapshot_with_simulation(TEST_SEED, sim_config);
        assert!(matches!(
            prepare(&root, locale, &forged, "Sherwood"),
            Err(RankedVerifierLoadError::CampaignContent(
                crate::replay_campaign_validation::ReplayCampaignContentValidationError::ProductionPointTopologyMissing { .. }
            ))
        ));
    }
}
