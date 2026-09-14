//! Prepare mission-end leaderboard uploads from the recording itself.
use crate::ingame_menu::layout::{
    MenuTransform, dim_screen, draw_screen_background, enter_modal_gpu_phase, render_text_virt_font,
};
use crate::ingame_menu::widget_bridge::ModalCursor;
use crate::ingame_menu::{IngameMenuResources, MissionEndLeaderboardScreen};
use crate::leaderboard_mission_end::{
    HttpMissionEndLeaderboardBackend, LocalMissionEndSubmissionAuthorizer, MissionEndBoard,
    MissionEndLeaderboardAction, MissionEndLeaderboardController, MissionEndLeaderboardEvent,
    MissionEndOutcome, MissionEndRunBundle, MissionEndSubmissionInput, RecordedReplayExporter,
};
use crate::leaderboard_preferences::{LeaderboardPreferences, LeaderboardTab};
use crate::leaderboard_service::{DEFAULT_BOARD_PAGE_LIMIT, LeaderboardApi};
use crate::renderer::Renderer;
use robin_engine::campaign::Campaign;
use robin_run_protocol::{
    BoardCategoryV1, BoardMetricV1, CanonicalDocument as _, ContentManifestV1, Digest32,
    LeaderboardMetadataV1, LeaderboardQuerySubjectV1, LeaderboardQueryV1, PublishedRulesetV1,
    RulesConfigIdentityV1, RulesetBoardScopeV1, RulesetOperationalStatusV1, RunContentIdentityV1,
    SCHEMA_VERSION_V1, ScopeRequestV1, SignatureAlgorithmV1, SpeechTimingAuthorityV1,
    SubmissionOfferRequestV1, VersionedBuildManifest,
};
use std::sync::Arc;
mod error;
use error::RankedError;
mod admission;
mod presentation;
pub(super) use presentation::{MissionEndLeaderboardTaskProgress, MissionEndLeaderboardTaskState};
/// Prepare an upload from the selected recording, without a saved admission file.
pub(crate) async fn prepare_recorded_submission(
    replay: &robin_engine::replay::ReplayData,
    preferences: &LeaderboardPreferences,
) -> Result<(MissionEndSubmissionInput, Arc<[u8]>), String> {
    use crate::leaderboard_signing::{GameIdentitySigner as _, PlatformSigner};
    use robin_run_protocol::{
        ArtifactRefV1, ReplayArtifactV1, ReplaySessionGenesisClaimV1, ReplaySessionGenesisV1,
    };
    let header = replay.header();
    let bytes: Arc<[u8]> =
        crate::replay_format::encode_compact(replay, robin_replay_format::ENGINE_VERSION_HASH)
            .map_err(|error| error.to_string())?
            .into_bytes()
            .into();
    let artifact = ReplayArtifactV1 {
        artifact: ArtifactRefV1 {
            sha256: Digest32::digest_bytes(&bytes),
            byte_length: bytes.len() as u64,
            media_type: robin_run_protocol::RANKED_REPLAY_MEDIA_TYPE_V1.to_owned(),
        },
        replay_schema_version: header.version,
    };
    let authority =
        admission::fetch_replay_authority(&header.mission_id, header.sim_config, preferences)
            .await
            .map_err(|error| error.to_string())?;
    let uploader = PlatformSigner::public_key()
        .await
        .map_err(|error| error.to_string())?;
    let replay_id = replay.submission_id();
    // Session identifiers describe this upload's anonymous seat events. They
    // confer no authority over the replay or over another player's account.
    let mut transcript = replay.submission_transcript(replay_id, replay_id)?;
    let starting_campaign = ArtifactRefV1 {
        sha256: Digest32::digest_bytes(&header.campaign),
        byte_length: header.campaign.len() as u64,
        media_type: robin_run_protocol::RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
    };
    let custom = authority.published_ruleset.manifest.rules_config_constraint
        == robin_run_protocol::RulesConfigConstraintV1::AnyCanonicalSimConfig;
    let content = &authority.content_manifest;
    let genesis = ReplaySessionGenesisV1 {
        claim: ReplaySessionGenesisClaimV1 {
            schema_version: 1,
            network_protocol_version: robin_engine::multiplayer::NET_PROTOCOL_VERSION,
            host_public_key: uploader,
            replay_session_id: replay_id,
            host_participant_instance_id: transcript.host_participant_instance_id,
            host_nonce: robin_run_protocol::ChallengeNonce32::from_bytes(replay_id.into_bytes()),
            ranked_session: robin_run_protocol::RankedSessionConfigV1 {
                recorded_replay: Some(artifact),
                schema_version: 1,
                mission_id: header.mission_id.clone(),
                content_edition: content.edition,
                content_subject: content.subject.clone(),
                simulation_seed: robin_run_protocol::SimulationSeed64::new(header.rng_seed),
                starting_campaign_sha256: starting_campaign.sha256,
                starting_campaign_byte_length: starting_campaign.byte_length,
                prepared_inputs_projection_sha256: None,
                prepared_mission_inputs_seal_sha256: None,
                build_manifest_sha256: authority.build_manifest_sha256,
                content_manifest_sha256: content
                    .canonical_digest()
                    .map_err(|error| error.to_string())?,
                campaign_content_manifest_sha256: None,
                rules_config_sha256: authority
                    .rules_config
                    .canonical_digest()
                    .map_err(|error| error.to_string())?,
                custom_rules_config: custom.then_some(authority.rules_config),
                custom_canonical_campaign: custom.then_some(starting_campaign),
                ruleset_manifest_sha256: authority.published_ruleset.ruleset_manifest_sha256,
                competition_manifest_sha256: None,
                spellforge_content_sha256: None,
                resource_locale_root: content.resource_locale_root.clone(),
                speech_timing: SpeechTimingAuthorityV1::CoreAudioDurationsV1,
            },
            fresh_run_preflight_grant: None,
            campaign_continuation_preflight_grant: None,
            competition_run_grant: None,
        },
        algorithm: SignatureAlgorithmV1::Ed25519,
        host_signature: None,
    };
    transcript.session_genesis_sha256 = genesis
        .canonical_digest()
        .map_err(|error| error.to_string())?;
    let input = MissionEndSubmissionInput {
        offer_request: SubmissionOfferRequestV1 {
            schema_version: 1,
            max_concurrent_players: transcript.max_concurrent_players,
            participant_instance_count: transcript.participant_instance_count,
            participant_claims: vec![robin_run_protocol::ParticipantClaimV1 {
                seat: 0,
                participant_instance_id: transcript.host_participant_instance_id,
                public_key: uploader,
                public_disclosure: robin_run_protocol::ParticipantPublicDisclosureV1::NamedProfile,
                join_attestation: None,
            }],
            session_genesis: genesis,
            mission_id: header.mission_id.clone(),
            scope_request: ScopeRequestV1::IndividualLevel,
            ruleset_manifest_sha256: authority.published_ruleset.ruleset_manifest_sha256,
            competition_manifest_sha256: None,
        },
        replay_session_transcript: transcript,
        requested_metrics: authority.requested_metrics,
        campaign_controller_public_key: None,
        starting_campaign_bytes: header.campaign.clone().into(),
    };
    input.validate().map_err(|error| error.to_string())?;
    Ok((input, bytes))
}

/// One presentation per completed attempt; no network work runs at mission launch.
pub(super) struct MissionLeaderboardRuntime {
    preparation: Option<MissionEndPreparation>,
}
impl MissionLeaderboardRuntime {
    pub(super) fn new() -> Self {
        let (preferences, error) = match crate::leaderboard_preferences::load() {
            Ok(preferences) => (preferences, None),
            Err(error) => (LeaderboardPreferences::default(), Some(error.to_string())),
        };
        Self {
            preparation: Some(MissionEndPreparation {
                preferences,
                upload: error.map(|error| {
                    crate::leaderboard::task::PollTask::start(async move { Err(error) })
                }),
            }),
        }
    }
    pub(super) fn after_state_restore(&mut self, _campaign: &Campaign) {
        if self.preparation.is_none() {
            *self = Self::new();
        }
    }
    pub(super) fn capture_terminal(
        &mut self,
        _outcome: MissionEndOutcome,
    ) -> Result<MissionEndPreparation, RankedError> {
        self.preparation.take().ok_or_else(|| {
            RankedError::lifecycle("mission-end leaderboard was captured more than once")
        })
    }
}
pub(super) struct MissionEndPreparation {
    preferences: LeaderboardPreferences,
    upload: Option<
        crate::leaderboard::task::PollTask<Result<(MissionEndRunBundle, Arc<[u8]>), String>>,
    >,
}
impl MissionEndPreparation {
    pub(super) fn preferences(&self) -> &LeaderboardPreferences {
        &self.preferences
    }

    /// Poll once. `None` means the bounded HTTP task is still in flight.
    pub(super) fn poll_bundle(
        &mut self,
        replay_exports: &crate::replay_service::ReplayExports,
    ) -> Option<Result<(MissionEndRunBundle, Arc<[u8]>), RankedError>> {
        if self.upload.is_none() {
            let snapshot = match replay_exports.snapshot() {
                Ok(snapshot) => snapshot,
                Err(error) => return Some(Err(RankedError::evidence(error))),
            };
            let preferences = self.preferences.clone();
            let task = crate::leaderboard::task::PollTask::spawn_background(
                "prepare-replay-upload",
                move || async move {
                    let replay = snapshot.parse_sync()?;
                    let (input, bytes) = prepare_recorded_submission(&replay, &preferences).await?;
                    let boards =
                        authorized_boards(&input, None).map_err(|error| error.to_string())?;
                    let bundle = MissionEndRunBundle {
                        outcome: MissionEndOutcome::from_replay(&replay),
                        multiplayer: input.offer_request.max_concurrent_players > 1,
                        boards,
                        eligible_submission: Some(input),
                        submission_unavailable_reason: None,
                    };
                    bundle.validate().map_err(|error| error.to_string())?;
                    Ok((bundle, bytes))
                },
            );
            self.upload = match task {
                Ok(task) => Some(task),
                Err(error) => return Some(Err(RankedError::unavailable(error.to_string()))),
            };
        }
        self.upload
            .as_ref()
            .expect("upload task initialized")
            .poll(|| "Replay upload preparation stopped unexpectedly".to_owned())
            .map(|result| result.map_err(RankedError::unavailable))
    }
}
pub(crate) fn authorized_boards(
    input: &MissionEndSubmissionInput,
    metadata: Option<&LeaderboardMetadataV1>,
) -> Result<Vec<MissionEndBoard>, RankedError> {
    let request = &input.offer_request;
    let ranked = &request.session_genesis.claim.ranked_session;
    let category = match request.scope_request {
        ScopeRequestV1::IndividualLevel => BoardCategoryV1::IndividualLevel,
        ScopeRequestV1::CampaignGenesis | ScopeRequestV1::CampaignContinuation { .. } => {
            BoardCategoryV1::Campaign
        }
    };
    let mut boards = metric_boards(
        BoardQueryIdentity {
            subject_kind: LeaderboardQuerySubjectV1::Mission,
            mission_id: Some(request.mission_id.clone()),
            mission_scope: Some(category),
            content_identity_sha256: ranked.content_manifest_sha256,
            rules_config_sha256: ranked.rules_config_sha256,
            ruleset_manifest_sha256: ranked.ruleset_manifest_sha256,
            competition_manifest_sha256: None,
            max_concurrent_players: Some(request.max_concurrent_players),
        },
        &input.requested_metrics,
    );
    if let Some(competition_sha256) = request.competition_manifest_sha256 {
        let competition = metadata.and_then(|metadata| {
            metadata
                .competitions
                .iter()
                .find(|competition| competition.competition_manifest_sha256 == competition_sha256)
        });
        // The competition digest does not itself identify its scoring metric.
        // Omit the optional Challenge tab until authenticated metadata names
        // that facet; submission authority remains wholly unaffected.
        if let Some(competition) = competition {
            boards.push(MissionEndBoard {
                tab: LeaderboardTab::Challenge,
                label: competition.manifest.display_name.clone(),
                query: query(
                    BoardQueryIdentity {
                        subject_kind: LeaderboardQuerySubjectV1::Mission,
                        mission_id: Some(request.mission_id.clone()),
                        mission_scope: Some(category),
                        content_identity_sha256: ranked.content_manifest_sha256,
                        rules_config_sha256: ranked.rules_config_sha256,
                        ruleset_manifest_sha256: ranked.ruleset_manifest_sha256,
                        competition_manifest_sha256: Some(competition_sha256),
                        max_concurrent_players: Some(request.max_concurrent_players),
                    },
                    competition.manifest.metric,
                ),
            });
        }
    }
    if boards.is_empty() {
        return Err(RankedError::unavailable(
            "ranked admission requested no supported board metrics",
        ));
    }
    Ok(boards)
}

/// Every facet of a leaderboard query except its metric.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct BoardQueryIdentity {
    subject_kind: LeaderboardQuerySubjectV1,
    mission_id: Option<String>,
    mission_scope: Option<BoardCategoryV1>,
    content_identity_sha256: robin_run_protocol::Digest32,
    rules_config_sha256: robin_run_protocol::Digest32,
    ruleset_manifest_sha256: robin_run_protocol::Digest32,
    competition_manifest_sha256: Option<robin_run_protocol::Digest32>,
    max_concurrent_players: Option<u16>,
}

fn metric_boards(identity: BoardQueryIdentity, metrics: &[BoardMetricV1]) -> Vec<MissionEndBoard> {
    [
        (BoardMetricV1::OriginalScore, LeaderboardTab::Score, "Score"),
        (BoardMetricV1::FastestSuccess, LeaderboardTab::Time, "Time"),
    ]
    .into_iter()
    .filter(|(metric, _, _)| metrics.contains(metric))
    .map(|(metric, tab, label)| MissionEndBoard {
        tab,
        label: label.to_owned(),
        query: query(identity.clone(), metric),
    })
    .collect()
}

fn query(identity: BoardQueryIdentity, metric: BoardMetricV1) -> LeaderboardQueryV1 {
    let BoardQueryIdentity {
        subject_kind,
        mission_id,
        mission_scope,
        content_identity_sha256,
        rules_config_sha256,
        ruleset_manifest_sha256,
        competition_manifest_sha256,
        max_concurrent_players,
    } = identity;
    LeaderboardQueryV1 {
        schema_version: SCHEMA_VERSION_V1,
        subject_kind,
        mission_id,
        mission_scope,
        metric,
        content_identity_sha256,
        rules_config_sha256: Some(rules_config_sha256),
        ruleset_manifest_sha256: Some(ruleset_manifest_sha256),
        competition_manifest_sha256,
        max_concurrent_players,
        player_public_key: None,
        limit: DEFAULT_BOARD_PAGE_LIMIT,
        cursor: None,
    }
}
