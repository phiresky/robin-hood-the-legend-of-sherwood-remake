//! Official-datadir loader for isolated ranked replay verification.
//!
//! This is deliberately separate from interactive/headless session startup:
//! it accepts no user profile, overlay, persistence, network, renderer, or
//! audio-device state. Each preparation is irreversibly confined to one validated
//! raw datadir before any legacy loader runs, then the resulting semantic
//! inputs are admitted against the independently mounted eight-document
//! catalog before an engine can be obtained.

use std::path::Path;

use robin_engine::engine::{LevelAssets, SimConfig};
use robin_engine::sbfile::{SBFILE_NO_ERROR, SbFileSystem};
use robin_engine::simulation_inputs::RankedContentAdmissionV1;
use robin_run_protocol::{
    ContentManifestV1, Digest32, RulesConfigIdentityV1, SimulationContentComponentDocumentV1,
    SpeechTimingAuthorityV1,
};

use super::replay_campaign_validation::{
    ApprovedRankedReplayPreparation, ApprovedReplayEngine, ReplayCampaignApprovedContentIdentity,
    ReplayCampaignApprovedContentMetadata, ReplayCampaignApprovedContentResolver,
    decode_and_validate_replay_campaign, derive_replay_campaign_approved_content_metadata,
    validate_replay_campaign_approved_content,
};
use crate::mission_loading::load_raw_mission_inputs;

/// Engine resources retained for the complete deterministic resimulation.
pub struct PreparedRankedReplayMission {
    preparation: ApprovedRankedReplayPreparation,
    assets: LevelAssets,
    /// Exact shipping mission route selected from the confined, approved
    /// profile catalog and the RHM bytes which produced `preparation`.
    approved_mission_assets: robin_engine::mission_assets::MissionAssetDescriptor,
}

impl PreparedRankedReplayMission {
    pub fn seal(&self) -> &robin_run_protocol::PreparedMissionInputsSealV1 {
        self.preparation.seal()
    }

    pub fn run_projection_sha256(
        &self,
    ) -> Result<Digest32, robin_engine::simulation_inputs::ProjectionError> {
        self.preparation.run_projection_sha256()
    }

    pub const fn starting_campaign_score(&self) -> i32 {
        self.preparation.starting_campaign_score()
    }

    /// Return the exact BuiltIn descriptor derived from approved content.
    ///
    /// This value is deliberately captured before the loaded RHM is consumed
    /// into the sealed engine. It lets the isolated worker bind the replay's
    /// durable asset identity to the content route it actually loaded rather
    /// than trusting uploader-provided filenames.
    pub const fn approved_mission_assets(
        &self,
    ) -> &robin_engine::mission_assets::MissionAssetDescriptor {
        &self.approved_mission_assets
    }

    pub fn into_engine_and_assets(self) -> (ApprovedReplayEngine, LevelAssets) {
        (self.preparation.into_engine(), self.assets)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RankedVerifierLoadError {
    #[error("cannot confine the mission-owned asset resolver (status {0})")]
    AssetResolverConfinement(i32),
    #[error("load official profiles: {0}")]
    Profiles(String),
    #[error("validate submitted campaign: {0}")]
    Campaign(#[from] super::replay_campaign_validation::ReplayCampaignValidationError),
    #[error("load deterministic resource archive {path}: {message}")]
    ResourceArchive { path: &'static str, message: String },
    #[error("load official sprite bank: {0}")]
    SpriteBank(String),
    #[error("load mission script {path}: status {status}")]
    MissionScriptRead { path: String, status: i32 },
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
    #[error("initialize clean approved Sherwood reference engine: {0}")]
    SherwoodReferenceEngine(robin_engine::engine::EngineError),
    #[error("derive approved Sherwood campaign metadata: {0}")]
    SherwoodMetadata(
        #[from] super::replay_campaign_validation::ReplayCampaignMetadataDerivationError,
    ),
    #[error("validate submitted campaign against approved content: {0}")]
    CampaignContent(
        #[from] super::replay_campaign_validation::ReplayCampaignContentValidationError,
    ),
    #[error("prepare ranked engine: {0}")]
    Engine(#[from] robin_engine::engine::EngineError),
}

struct ApprovedResolver {
    identity: ReplayCampaignApprovedContentIdentity,
    sherwood: Option<ReplayCampaignApprovedContentMetadata>,
}

impl ReplayCampaignApprovedContentResolver for ApprovedResolver {
    fn approved_content_identity(&self) -> ReplayCampaignApprovedContentIdentity {
        self.identity
    }

    fn sherwood_campaign_metadata(&self) -> Option<&ReplayCampaignApprovedContentMetadata> {
        self.sherwood.as_ref()
    }
}

/// Load and seal the exact engine which may execute a ranked replay.
#[allow(clippy::too_many_arguments)]
pub fn prepare_ranked_replay_mission(
    raw_content_root: &Path,
    starting_campaign_bytes: &[u8],
    mission_id: &str,
    replay_limits: &robin_replay_format::ReplayAdmissionLimits,
    build_manifest_sha256: Digest32,
    content_manifest_sha256: Digest32,
    content_manifest: &ContentManifestV1,
    mounted_documents: &[SimulationContentComponentDocumentV1],
    rules_config: &RulesConfigIdentityV1,
    speech_timing: &SpeechTimingAuthorityV1,
    rng_seed: u64,
    sim_config: SimConfig,
) -> Result<PreparedRankedReplayMission, RankedVerifierLoadError> {
    let files = std::sync::Arc::new(SbFileSystem::new(std::sync::Arc::new(
        robin_util::asset_fs::AssetVfs::new(),
    )));
    let status = files.lock_ranked_verifier_primary_path_with_locale(
        raw_content_root,
        content_manifest.resource_locale_root.as_str(),
    );
    if status != SBFILE_NO_ERROR {
        return Err(RankedVerifierLoadError::AssetResolverConfinement(status));
    }
    let options = robin_engine::engine::GlobalOptions::default();
    let profiles = crate::profile_loading::load_profiles(&options, &files)
        .map_err(|error| RankedVerifierLoadError::Profiles(error.to_string()))?;
    let validated_campaign = decode_and_validate_replay_campaign(
        starting_campaign_bytes,
        mission_id,
        &profiles,
        replay_limits,
    )?;
    validated_campaign.validate_checkpoint_simulation_authority(sim_config)?;
    let campaign = validated_campaign.campaign_for_approved_loading();
    let mut submitted = load_raw_mission_inputs(
        campaign,
        &profiles,
        &options,
        speech_timing,
        sim_config,
        files.clone(),
    )?;
    let mission_index = campaign.current_mission_idx.ok_or_else(|| {
        RankedVerifierLoadError::Mission(
            "approved campaign has no current mission for asset binding".into(),
        )
    })?;
    let profile = campaign
        .missions
        .get(mission_index)
        .map(|mission| mission.profile(&profiles))
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
        Some(derive_approved_sherwood_metadata(
            &profiles,
            &options,
            speech_timing,
            sim_config,
            files,
        )?)
    };
    let resolver = ApprovedResolver {
        identity: ReplayCampaignApprovedContentIdentity {
            build_manifest_sha256: build_manifest_sha256.into_bytes(),
            content_manifest_sha256: content_manifest_sha256.into_bytes(),
        },
        sherwood,
    };
    let approved = validate_replay_campaign_approved_content(validated_campaign, &resolver)?;
    let admission = RankedContentAdmissionV1 {
        manifest: content_manifest,
        mounted_documents,
        rules_config,
        speech_timing: speech_timing.clone(),
    };
    let preparation = approved.prepare_ranked_engine(
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
        admission,
    )?;
    Ok(PreparedRankedReplayMission {
        preparation,
        assets: submitted.assets,
        approved_mission_assets,
    })
}

fn derive_approved_sherwood_metadata(
    profiles: &robin_engine::profiles::ProfileManager,
    options: &robin_engine::engine::GlobalOptions,
    speech_timing: &SpeechTimingAuthorityV1,
    sim_config: SimConfig,
    files: std::sync::Arc<SbFileSystem>,
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

    // This campaign is built only from the approved profile catalog. No byte
    // or value from the submitted campaign can influence the phase-two
    // topology authority.
    let mut clean_campaign = robin_engine::campaign::Campaign::from_profiles(
        profiles,
        robin_engine::player_profile::DifficultyLevel::Medium,
    );
    clean_campaign.current_mission_idx = Some(sherwood_index);
    clean_campaign.add_all_to_mission_team();

    let mut reference = load_raw_mission_inputs(
        &clean_campaign,
        profiles,
        options,
        speech_timing,
        sim_config,
        files,
    )?;
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
    use robin_engine::engine::{Engine, EngineArgs, LevelLoadArgs};
    use robin_engine::player_profile::DifficultyLevel;
    use robin_engine::sector_production::Point;
    use robin_engine::simulation_inputs::PREPARED_MISSION_RUN_PROJECTION_SCHEMA_V1;
    use robin_run_protocol::{
        ArtifactRefV1, CanonicalDocument as _, CanonicalValue, ContentClosureKindV1,
        OfficialContentEditionV1, OfficialContentSubjectV1, RankedSimulationPolicyV1,
        RulesConfigIdentityV1, SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1,
        SimulationContentComponentV1, SimulationSpeechTimingSourceV1,
    };

    const TEST_SEED: u64 = 0x7268_2d76_6572_7631;

    fn fixture_sim_config() -> SimConfig {
        robin_engine::engine::RankedSimulationPolicy::standard_medium().expected_config()
    }

    #[test]
    fn fixture_simulation_config_matches_declared_ranked_policy() {
        let config = fixture_sim_config();
        let (validated, _) =
            robin_engine::simulation_inputs::validate_ranked_simulation_policy_rules_config_v1(
                &rules_config(config),
            )
            .expect("fixture configuration must match its declared Standard/Medium policy");
        assert_eq!(validated, config);
    }

    fn rules_config(sim_config: SimConfig) -> RulesConfigIdentityV1 {
        let CanonicalValue::Object(sim_config) =
            CanonicalValue::from_serializable(&sim_config).expect("canonical fixture SimConfig")
        else {
            panic!("SimConfig must serialize as a canonical object")
        };
        RulesConfigIdentityV1 {
            schema_version: 1,
            replay_schema_version: robin_engine::replay::REPLAY_SCHEMA_VERSION,
            ranked_simulation_policy: RankedSimulationPolicyV1::standard(
                robin_run_protocol::RankedSimulationDifficultyV1::Medium,
            ),
            sim_config,
            rules: std::collections::BTreeMap::from([(
                "policy".into(),
                CanonicalValue::String("ranked".into()),
            )]),
        }
    }

    fn operator_datadir(name: &str) -> std::path::PathBuf {
        if let Some(root) = std::env::var_os("ROBINHOOD_DATA_DIR") {
            return root.into();
        }
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../datadirs")
            .join(name)
    }

    fn enter_operator_datadir(
        root: &Path,
        resource_locale_root: &str,
    ) -> (
        robin_engine::engine::GlobalOptions,
        robin_engine::profiles::ProfileManager,
        std::sync::Arc<SbFileSystem>,
    ) {
        let files = std::sync::Arc::new(SbFileSystem::new(std::sync::Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        )));
        assert_eq!(
            files.lock_ranked_verifier_primary_path_with_locale(root, resource_locale_root),
            SBFILE_NO_ERROR,
            "confine real-data adapter test"
        );
        let options = robin_engine::engine::GlobalOptions::default();
        let profiles = crate::profile_loading::load_profiles(&options, &files)
            .expect("load real-data profile catalog");
        (options, profiles, files)
    }

    fn campaign_for_mission(
        profiles: &robin_engine::profiles::ProfileManager,
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

    fn content_manifest(
        name: &str,
        edition: OfficialContentEditionV1,
        subject: OfficialContentSubjectV1,
        resource_locale_root: &str,
        documents: &[SimulationContentComponentDocumentV1],
    ) -> ContentManifestV1 {
        ContentManifestV1 {
            schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
            name: name.into(),
            edition,
            subject,
            closure: ContentClosureKindV1::StaticPreparedMissionContentProjection,
            projection_schema_version: PREPARED_MISSION_RUN_PROJECTION_SCHEMA_V1,
            resource_locale_root: robin_run_protocol::ResourceLocaleRootV1::new(
                resource_locale_root,
            )
            .expect("fixture resource locale root"),
            speech_timing: SimulationSpeechTimingSourceV1::BaseInstallation,
            components: documents
                .iter()
                .map(|document| {
                    let bytes = document
                        .canonical_bytes()
                        .expect("canonical projected component");
                    SimulationContentComponentV1 {
                        kind: document.kind,
                        component_schema_version: document.component_schema_version,
                        artifact: ArtifactRefV1 {
                            sha256: Digest32::digest_bytes(&bytes),
                            byte_length: u64::try_from(bytes.len())
                                .expect("component length fits u64"),
                            media_type: SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1.into(),
                        },
                    }
                })
                .collect(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_fixture_projection(
        campaign: &Campaign,
        profiles: &robin_engine::profiles::ProfileManager,
        options: &robin_engine::engine::GlobalOptions,
        speech_timing: &SpeechTimingAuthorityV1,
        sim_config: SimConfig,
        edition: OfficialContentEditionV1,
        subject: OfficialContentSubjectV1,
        resource_locale_root: &str,
        files: std::sync::Arc<SbFileSystem>,
    ) -> (
        ContentManifestV1,
        Vec<SimulationContentComponentDocumentV1>,
        robin_run_protocol::PreparedMissionInputsSealV1,
    ) {
        let mut raw = load_raw_mission_inputs(
            campaign,
            profiles,
            options,
            speech_timing,
            sim_config,
            files,
        )
        .expect("load exact raw fixture inputs");
        let prepared = Engine::prepare_preserving_campaign(EngineArgs {
            campaign: campaign.clone(),
            level: LevelLoadArgs {
                assets: &mut raw.assets,
                level_directory: &raw.level_directory,
                progress: &mut |_| {},
                loaded: raw.loaded,
                bg_pixel_dims: raw.bg_pixel_dims,
            },
            ground_mark_sprite: raw.ground_mark_sprite,
            titbit_row_frame_counts: raw.titbit_row_frame_counts,
            rng_seed: TEST_SEED,
            original_rng_replay: None,
            sim_config,
        })
        .unwrap_or_else(|(error, _)| panic!("prepare exact raw fixture engine: {error}"));
        let documents = prepared
            .static_projection()
            .components()
            .iter()
            .map(|component| component.document.clone())
            .collect::<Vec<_>>();
        let manifest = content_manifest(
            "real ranked adapter fixture",
            edition,
            subject,
            resource_locale_root,
            &documents,
        );
        let rules_config = rules_config(sim_config);
        let seal = prepared
            .admit_ranked_content(
                &manifest,
                &documents,
                rules_config
                    .canonical_digest()
                    .expect("canonical fixture rules config"),
                speech_timing.clone(),
            )
            .expect("seal exact fixture projection");
        (manifest, documents, seal)
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_through_adapter(
        root: &Path,
        campaign: &Campaign,
        mission_id: &str,
        manifest: &ContentManifestV1,
        documents: &[SimulationContentComponentDocumentV1],
        speech_timing: &SpeechTimingAuthorityV1,
        sim_config: SimConfig,
    ) -> Result<PreparedRankedReplayMission, RankedVerifierLoadError> {
        let rules_config = rules_config(sim_config);
        prepare_ranked_replay_mission(
            root,
            &bitcode::encode(campaign),
            mission_id,
            &robin_replay_format::ReplayAdmissionLimits::default(),
            Digest32::from_bytes([1; 32]),
            manifest
                .canonical_digest()
                .expect("content manifest digest"),
            manifest,
            documents,
            &rules_config,
            speech_timing,
            TEST_SEED,
            sim_config,
        )
    }

    // Each fixture owns its confined resolver; different datadirs may coexist
    // without changing process CWD or global mounts.
    #[test]
    #[ignore = "requires operator-mounted demo Leicester data"]
    fn real_demo_field_mints_only_the_exact_sealed_engine_capability() {
        let root = operator_datadir("demo_leicester_ecoste");
        let resource_locale_root = "1033";
        let (options, profiles, files) = enter_operator_datadir(&root, resource_locale_root);
        let sim_config = fixture_sim_config();
        let speech_timing = SpeechTimingAuthorityV1::BaseInstallation;
        let mission_id = "Dem_Lei_MP";
        let campaign = campaign_for_mission(&profiles, mission_id, sim_config);
        let (manifest, documents, expected_seal) = prepare_fixture_projection(
            &campaign,
            &profiles,
            &options,
            &speech_timing,
            sim_config,
            OfficialContentEditionV1::Demo,
            OfficialContentSubjectV1::FieldMission {
                mission_id: mission_id.into(),
            },
            resource_locale_root,
            files,
        );

        let preparation = prepare_through_adapter(
            &root,
            &campaign,
            mission_id,
            &manifest,
            &documents,
            &speech_timing,
            sim_config,
        )
        .expect("real demo adapter preparation");
        assert_eq!(preparation.seal(), &expected_seal);
        assert_eq!(
            preparation.run_projection_sha256().unwrap(),
            expected_seal.prepared_inputs_projection_sha256
        );

        let mut substituted = documents.clone();
        substituted[0].payload = robin_run_protocol::CanonicalValue::String("forged".into());
        assert!(matches!(
            prepare_through_adapter(
                &root,
                &campaign,
                mission_id,
                &manifest,
                &substituted,
                &speech_timing,
                sim_config,
            ),
            Err(RankedVerifierLoadError::Engine(_))
        ));
    }

    #[test]
    #[ignore = "requires operator-mounted full retail data"]
    fn real_full_hq_and_later_field_validate_clean_sherwood_authority() {
        let root = operator_datadir("fullgame_linux");
        let resource_locale_root = "2047";
        let (options, profiles, files) = enter_operator_datadir(&root, resource_locale_root);
        let sim_config = fixture_sim_config();
        let speech_timing = SpeechTimingAuthorityV1::BaseInstallation;

        let metadata = derive_approved_sherwood_metadata(
            &profiles,
            &options,
            &speech_timing,
            sim_config,
            files.clone(),
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
        let (hq_manifest, hq_documents, hq_expected_seal) = prepare_fixture_projection(
            &hq_campaign,
            &profiles,
            &options,
            &speech_timing,
            sim_config,
            OfficialContentEditionV1::Full,
            OfficialContentSubjectV1::Headquarters {
                mission_id: "Sherwood".into(),
            },
            resource_locale_root,
            files.clone(),
        );
        let hq = prepare_through_adapter(
            &root,
            &hq_campaign,
            "Sherwood",
            &hq_manifest,
            &hq_documents,
            &speech_timing,
            sim_config,
        )
        .expect("real full HQ adapter preparation");
        assert_eq!(hq.seal(), &hq_expected_seal);

        let field_id = "Emb01_FoA_EC";
        let field_index = profiles
            .missions
            .iter()
            .position(|profile| profile.mission_filename.eq_ignore_ascii_case(field_id))
            .expect("full fixture field mission");
        let mut field_campaign = hq_campaign.clone();
        field_campaign.current_mission_idx = Some(field_index);
        field_campaign.add_all_to_mission_team();
        field_campaign.snapshot_with_simulation(TEST_SEED, sim_config);
        let (field_manifest, field_documents, field_expected_seal) = prepare_fixture_projection(
            &field_campaign,
            &profiles,
            &options,
            &speech_timing,
            sim_config,
            OfficialContentEditionV1::Full,
            OfficialContentSubjectV1::FieldMission {
                mission_id: field_id.into(),
            },
            resource_locale_root,
            files,
        );
        let field = prepare_through_adapter(
            &root,
            &field_campaign,
            field_id,
            &field_manifest,
            &field_documents,
            &speech_timing,
            sim_config,
        )
        .expect("real later-field adapter preparation");
        assert_eq!(field.seal(), &field_expected_seal);

        let mut forged = field_campaign;
        forged.production_sectors[0].production_points[0].sector = u16::MAX;
        forged.snapshot_with_simulation(TEST_SEED, sim_config);
        assert!(matches!(
            prepare_through_adapter(
                &root,
                &forged,
                field_id,
                &field_manifest,
                &field_documents,
                &speech_timing,
                sim_config,
            ),
            Err(RankedVerifierLoadError::CampaignContent(
                crate::replay_campaign_validation::ReplayCampaignContentValidationError::ProductionPointTopologyMissing { .. }
            ))
        ));
    }
}
