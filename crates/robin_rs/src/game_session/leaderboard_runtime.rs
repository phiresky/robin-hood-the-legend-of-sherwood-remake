//! Mission-lifetime inputs for the post-mission leaderboard surface.
//!
//! Ranked authority must exist before simulation begins. This module therefore
//! captures the exact pre-mission campaign and owns an explicit admission
//! state instead of trying to reconstruct a signed genesis at debrief time.
//! Native, browser, and multiplayer frontends all resolve the same prepared
//! authority before frame zero. Multiplayer transports retain the signed
//! lifecycle; this module never creates a second replay or identity lane.

use crate::leaderboard_browse::{LeaderboardBrowseEvent, LeaderboardBrowser};
use crate::leaderboard_mission_end::{
    ActiveMissionReplayExporter, HttpMissionEndLeaderboardBackend,
    LocalMissionEndSubmissionAuthorizer, MissionEndBoard, MissionEndLeaderboardAction,
    MissionEndLeaderboardController, MissionEndLeaderboardEvent, MissionEndOutcome,
    MissionEndPeerCoSigner, MissionEndReplayExporter, MissionEndRunBundle,
    MissionEndSubmissionAuthorizer, MissionEndSubmissionInput, MissionEndTask,
    ParticipantSigningProgress, PeerCoSignPoll, SubmissionAuthorizationRequest,
    SubmissionAuthorizationTask,
};
use crate::leaderboard_preferences::{LeaderboardPreferences, LeaderboardScope, LeaderboardTab};
use crate::leaderboard_service::{DEFAULT_BOARD_PAGE_LIMIT, LeaderboardApi};
use robin_engine::campaign::Campaign;
use robin_run_protocol::{
    BoardCategoryV1, BoardMetricV1, CampaignContentManifestV1, CampaignContinuationAuthorizationV1,
    CanonicalDocument as _, CompetitionStateV1, ContentManifestV1, Digest32, LeaderboardMetadataV1,
    LeaderboardQuerySubjectV1, LeaderboardQueryV1, LeaderboardSubjectV1, ParticipantSignatureV1,
    PublishedRulesetV1, RulesConfigIdentityV1, RulesetBoardScopeV1, RulesetOperationalStatusV1,
    RunContentIdentityV1, SCHEMA_VERSION_V1, ScopeRequestV1, Signature64, SignatureAlgorithmV1,
    SignedSubmissionV1, SimulationSpeechTimingSourceV1, SpeechTimingAuthorityV1,
    SubmissionOfferRequestV1, Validate as _, VersionedBuildManifest,
    official_content_manifest_name_v1, official_content_subjects_v1,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use crate::leaderboard_ranked_session::{
    CampaignContinuationReceiptSelectionRequestV1, CampaignContinuationReceiptSelectionResponseV1,
    CampaignContinuationReceiptSelectionV1, OfficialRankedSessionExpectationV1,
    OfficialRankedSessionSetupV1, OfficialRankedSessionWireSetupV1, RankedPreflightLobbyV1,
    RankedRunPreflightAdmissionV1,
};

use crate::ingame_menu::layout::{
    MenuTransform, dim_screen, draw_screen_background, enter_modal_gpu_phase, render_text_virt_font,
};
use crate::ingame_menu::widget_bridge::ModalCursor;
use crate::ingame_menu::{IngameMenuResources, MissionEndLeaderboardScreen};
use crate::renderer::Renderer;

mod admission;
pub(super) use admission::{
    PreparedRankedAdmission, RankedPreFramePlan, fetch_single_player_authority,
};

/// Authority fixed before the first simulation frame.
///
/// `BrowseOnly` is a deliberate state, not a failed attempt to invent the
/// missing signatures later. The authorized arm is the sole integration seam
/// for the ranked-session setup flow.
#[derive(Clone)]
pub(crate) enum RankedMissionAdmission {
    BrowseOnly { reason: String },
    Authorized(MissionEndSubmissionInput),
    Signed(SignedRankedMissionAdmission),
}

#[derive(Clone)]
pub(crate) struct SignedRankedMissionAdmission {
    lifecycle: crate::leaderboard_ranked_session::SharedRankedSessionLifecycle,
    scope_request: ScopeRequestV1,
    requested_metrics: Vec<BoardMetricV1>,
    campaign_controller_public_key: Option<robin_run_protocol::PublicKey32>,
}

impl RankedMissionAdmission {
    /// Public signed documents only; no private signer or live transport state.
    pub(crate) fn archive_input(
        &self,
        replay: &robin_engine::replay::ReplayData,
    ) -> Result<Option<MissionEndSubmissionInput>, String> {
        let mut copy = self.clone();
        copy.materialize_terminal_from_replay(
            &replay.header().mission_id,
            replay.header().campaign.clone().into(),
            replay,
        )?;
        match copy {
            Self::Authorized(input) => Ok(Some(input)),
            Self::BrowseOnly { .. } => Ok(None),
            Self::Signed(_) => Err("ranked evidence remained unresolved".into()),
        }
    }

    fn browse_only(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        assert!(!reason.is_empty(), "browse-only admission needs a reason");
        Self::BrowseOnly { reason }
    }

    fn materialize_terminal(
        &mut self,
        mission_id: &str,
        starting_campaign_bytes: Arc<[u8]>,
        replay_exports: &crate::replay_service::ReplayExports,
    ) -> Result<(), String> {
        if !matches!(self, Self::Signed(_)) {
            return Ok(());
        }
        let replay = replay_exports.snapshot()?.parse_sync()?;
        self.materialize_terminal_from_replay(mission_id, starting_campaign_bytes, &replay)
    }

    fn materialize_terminal_from_replay(
        &mut self,
        mission_id: &str,
        starting_campaign_bytes: Arc<[u8]>,
        replay: &robin_engine::replay::ReplayData,
    ) -> Result<(), String> {
        let Self::Signed(signed) = self else {
            return Ok(());
        };
        let evidence = signed
            .lifecycle
            .lock()
            .map_err(|_| "ranked session lifecycle lock was poisoned".to_owned())?
            .evidence_for_replay(replay)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "ranked session was downgraded before terminal evidence".to_owned())?;
        if evidence.session_genesis.claim.ranked_session.mission_id != mission_id {
            return Err("ranked session evidence names a different mission".to_owned());
        }
        let claim_count = u16::try_from(evidence.participant_claims.len())
            .map_err(|_| "ranked participant roster exceeds protocol bounds".to_owned())?;
        let (participant_instance_count, max_concurrent_players) = evidence
            .replay_session_transcript
            .validate_and_derive_counts()
            .map_err(|error| error.to_string())?;
        if claim_count != participant_instance_count {
            return Err(format!(
                "ranked participant claim count {claim_count} differs from authenticated transcript count {participant_instance_count}"
            ));
        }
        let ruleset_manifest_sha256 = evidence
            .session_genesis
            .claim
            .ranked_session
            .ruleset_manifest_sha256;
        let competition_manifest_sha256 = evidence
            .session_genesis
            .claim
            .ranked_session
            .competition_manifest_sha256;
        let offer_request = SubmissionOfferRequestV1 {
            schema_version: SCHEMA_VERSION_V1,
            max_concurrent_players,
            participant_instance_count,
            participant_claims: evidence.participant_claims,
            session_genesis: evidence.session_genesis,
            mission_id: mission_id.to_owned(),
            scope_request: signed.scope_request.clone(),
            ruleset_manifest_sha256,
            competition_manifest_sha256,
        };
        offer_request
            .validate()
            .map_err(|error| error.to_string())?;
        let input = MissionEndSubmissionInput {
            offer_request,
            replay_session_transcript: evidence.replay_session_transcript,
            requested_metrics: signed.requested_metrics.clone(),
            campaign_controller_public_key: signed.campaign_controller_public_key,
            starting_campaign_bytes,
        };
        // Use the public bundle validator as the final cross-document gate;
        // no partially constructed terminal evidence becomes submittable.
        let probe = MissionEndRunBundle {
            outcome: MissionEndOutcome::Won,
            multiplayer: false,
            boards: authorized_boards(&input, None)?,
            eligible_submission: Some(input.clone()),
            submission_unavailable_reason: None,
        };
        probe.validate().map_err(|error| error.to_string())?;
        *self = Self::Authorized(input);
        Ok(())
    }
}

/// Restored files carry signed evidence, not permission to invent a genesis.
/// Submission still passes the normal authorizer and server preflight checks.
fn validate_archived_ranked_input(
    input: &MissionEndSubmissionInput,
    replay: &robin_engine::replay::ReplayData,
) -> Result<(), String> {
    input.validate().map_err(|error| error.to_string())?;
    if input.starting_campaign_bytes.as_ref() != replay.header().campaign.as_slice()
        || input.offer_request.mission_id != replay.header().mission_id
    {
        return Err("archived ranked admission does not match the mission recording root".into());
    }
    let genesis = &input.offer_request.session_genesis;
    crate::leaderboard_ranked_session::validate_session_genesis(
        genesis,
        *genesis.claim.host_public_key.as_bytes(),
        &genesis.claim.ranked_session,
    )
    .map_err(|error| error.to_string())?;
    crate::leaderboard_ranked_session::validate_transcript_against_local_replay(
        replay,
        genesis,
        &input.offer_request.participant_claims,
        &input.replay_session_transcript,
    )
    .map_err(|error| error.to_string())?;
    replay
        .ranked_submission_verdict()
        .map_err(|error| format!("mission replay is ineligible: {error:?}"))
}

enum MetadataLoad {
    Loading(LeaderboardBrowser),
    Ready(LeaderboardMetadataV1),
    Failed(String),
}

/// State constructed at mission bootstrap and consumed exactly once when the
/// engine first reports a terminal result.
pub(super) struct MissionLeaderboardRuntime {
    preparation: Option<MissionEndPreparation>,
    mission_id: String,
    multiplayer: bool,
}

impl MissionLeaderboardRuntime {
    pub(super) fn new(
        starting_campaign: &Campaign,
        mission_id: String,
        multiplayer: bool,
        admission: RankedMissionAdmission,
        ranked_multiplayer_port: Option<crate::multiplayer::RankedMultiplayerPort>,
    ) -> Self {
        assert!(!mission_id.is_empty(), "leaderboard mission id is required");
        let starting_campaign_bytes: Arc<[u8]> = bitcode::encode(starting_campaign).into();
        assert!(
            !starting_campaign_bytes.is_empty(),
            "bitcode campaign encoding cannot be empty"
        );
        let (preferences, metadata) = match crate::leaderboard_preferences::load() {
            Ok(preferences) => match LeaderboardApi::from_preferences(&preferences) {
                Ok(api) => {
                    let mut browser = LeaderboardBrowser::new(api);
                    let metadata = match browser.begin_metadata() {
                        Ok(()) => MetadataLoad::Loading(browser),
                        Err(error) => MetadataLoad::Failed(error.to_string()),
                    };
                    (preferences, metadata)
                }
                Err(error) => (
                    preferences,
                    MetadataLoad::Failed(format!("leaderboard endpoint unavailable: {error}")),
                ),
            },
            Err(error) => (
                LeaderboardPreferences::default(),
                MetadataLoad::Failed(format!(
                    "leaderboard preferences could not be loaded: {error}"
                )),
            ),
        };
        Self {
            mission_id: mission_id.clone(),
            multiplayer,
            preparation: Some(MissionEndPreparation {
                mission_id,
                multiplayer,
                starting_campaign_bytes,
                admission,
                ranked_multiplayer_port,
                preferences,
                metadata,
                outcome: None,
            }),
        }
    }

    /// Restoring state after terminal presentation starts a new local attempt.
    /// Its frozen export belongs to the completed attempt. The next export
    /// adopts the original signed admission only after validating the resumed
    /// mission archive. Mid-mission loads keep the existing preparation.
    pub(super) fn after_state_restore(&mut self, campaign: &Campaign) {
        if self.preparation.is_some() {
            return;
        }
        *self = Self::new(
            campaign,
            self.mission_id.clone(),
            self.multiplayer,
            RankedMissionAdmission::browse_only(
                "restored mission requires its original archived ranked admission",
            ),
            None,
        );
    }

    /// Freeze the terminal outcome and transfer the prestarted metadata task
    /// to the cooperative UI owner. The caller invokes this before recorder
    /// finalization; the returned task must not be polled until the next outer
    /// frame, after the terminal replay record has been flushed.
    pub(super) fn capture_terminal(
        &mut self,
        outcome: MissionEndOutcome,
    ) -> Result<MissionEndPreparation, String> {
        let mut preparation = self
            .preparation
            .take()
            .ok_or_else(|| "mission-end leaderboard was captured more than once".to_owned())?;
        preparation.outcome = Some(outcome);
        Ok(preparation)
    }
}

/// Frame-polled work required before a validated [`MissionEndRunBundle`] can
/// be handed to the existing controller.
pub(super) struct MissionEndPreparation {
    mission_id: String,
    multiplayer: bool,
    starting_campaign_bytes: Arc<[u8]>,
    admission: RankedMissionAdmission,
    ranked_multiplayer_port: Option<crate::multiplayer::RankedMultiplayerPort>,
    preferences: LeaderboardPreferences,
    metadata: MetadataLoad,
    outcome: Option<MissionEndOutcome>,
}

impl MissionEndPreparation {
    pub(super) fn preferences(&self) -> &LeaderboardPreferences {
        &self.preferences
    }

    /// Poll once. `None` means the bounded HTTP task is still in flight.
    pub(super) fn poll_bundle(
        &mut self,
        replay_exports: &crate::replay_service::ReplayExports,
    ) -> Option<Result<MissionEndRunBundle, String>> {
        if let Some(restored) = replay_exports.restored_ranked_input() {
            match restored.and_then(|input| {
                let replay = replay_exports.snapshot()?.parse_sync()?;
                validate_archived_ranked_input(&input, &replay)?;
                Ok(input)
            }) {
                Ok(input) => {
                    self.starting_campaign_bytes = input.starting_campaign_bytes.clone();
                    self.admission = RankedMissionAdmission::Authorized(input);
                }
                Err(error) => {
                    self.admission = RankedMissionAdmission::browse_only(error);
                }
            }
        }
        let authors_submission = self
            .ranked_multiplayer_port
            .as_ref()
            .is_none_or(|port| port.role() == crate::multiplayer::RankedMultiplayerRole::Host);
        if authors_submission
            && matches!(self.admission, RankedMissionAdmission::Signed(_))
            && let Err(error) = self.admission.materialize_terminal(
                &self.mission_id,
                self.starting_campaign_bytes.clone(),
                replay_exports,
            )
        {
            self.admission = RankedMissionAdmission::browse_only(format!(
                "terminal ranked evidence could not be sealed: {error}"
            ));
        }
        // Signed admission already fixes all submission and query facets.
        // Metadata is only browse/display authority and must never block or
        // suppress an eligible always-submit flow.
        if matches!(self.admission, RankedMissionAdmission::Authorized(_)) {
            let metadata = poll_metadata_once(&mut self.metadata).ok().flatten();
            return Some(build_bundle(
                &self.mission_id,
                self.multiplayer,
                self.starting_campaign_bytes.clone(),
                self.outcome
                    .expect("terminal preparation must freeze an outcome before polling"),
                &self.admission,
                &self.preferences,
                metadata.as_ref(),
            ));
        }
        if matches!(self.admission, RankedMissionAdmission::Signed(_)) {
            return Some(build_bundle(
                &self.mission_id,
                self.multiplayer,
                self.starting_campaign_bytes.clone(),
                self.outcome
                    .expect("terminal preparation must freeze an outcome before polling"),
                &self.admission,
                &self.preferences,
                None,
            ));
        }
        let metadata = match poll_metadata_once(&mut self.metadata) {
            Ok(Some(metadata)) => metadata,
            Ok(None) => return None,
            Err(error) => return Some(Err(error)),
        };
        Some(build_bundle(
            &self.mission_id,
            self.multiplayer,
            self.starting_campaign_bytes.clone(),
            self.outcome
                .expect("terminal preparation must freeze an outcome before polling"),
            &self.admission,
            &self.preferences,
            Some(&metadata),
        ))
    }
}

fn poll_metadata_once(
    metadata: &mut MetadataLoad,
) -> Result<Option<LeaderboardMetadataV1>, String> {
    match metadata {
        MetadataLoad::Loading(browser) => match browser.poll() {
            None => Ok(None),
            Some(Ok(LeaderboardBrowseEvent::Metadata(loaded))) => {
                *metadata = MetadataLoad::Ready(loaded.clone());
                Ok(Some(loaded))
            }
            Some(Err(error)) => Err(error.to_string()),
        },
        MetadataLoad::Ready(metadata) => Ok(Some(metadata.clone())),
        MetadataLoad::Failed(error) => Err(error.clone()),
    }
}

mod signing;
use signing::{MultiplayerHostSubmissionAuthorizer, MultiplayerPeerCoSigner};

mod presentation;
pub(super) use presentation::{MissionEndLeaderboardTaskProgress, MissionEndLeaderboardTaskState};

fn build_bundle(
    mission_id: &str,
    multiplayer: bool,
    starting_campaign_bytes: Arc<[u8]>,
    outcome: MissionEndOutcome,
    admission: &RankedMissionAdmission,
    preferences: &LeaderboardPreferences,
    metadata: Option<&LeaderboardMetadataV1>,
) -> Result<MissionEndRunBundle, String> {
    if let Some(metadata) = metadata {
        metadata
            .validate()
            .map_err(|error| format!("leaderboard metadata is invalid: {error}"))?;
    }
    let (boards, eligible_submission, unavailable_reason) = match admission {
        RankedMissionAdmission::Authorized(input) => {
            if input.offer_request.mission_id != mission_id {
                return Err(format!(
                    "ranked admission mission `{}` does not match loaded mission `{mission_id}`",
                    input.offer_request.mission_id
                ));
            }
            if input.starting_campaign_bytes.as_ref() != starting_campaign_bytes.as_ref() {
                return Err(
                    "ranked admission starting campaign differs from the exact engine input"
                        .to_owned(),
                );
            }
            let boards = authorized_boards(input, metadata)?;
            if outcome.can_submit() {
                (boards, Some(input.clone()), None)
            } else {
                (
                    boards,
                    None,
                    Some("only won missions can be submitted".to_owned()),
                )
            }
        }
        RankedMissionAdmission::BrowseOnly { reason } => {
            let metadata = metadata.ok_or_else(|| {
                "leaderboard metadata is required to browse a run without ranked admission"
                    .to_owned()
            })?;
            (
                browse_boards(mission_id, multiplayer, preferences, metadata)?,
                None,
                Some(reason.clone()),
            )
        }
        RankedMissionAdmission::Signed(signed) => {
            (
                signed_admission_boards(mission_id, signed)?,
                None,
                Some(
                    "this peer retained ranked evidence; the host controls submission and requests each participant's co-signature"
                        .to_owned(),
                ),
            )
        }
    };
    let bundle = MissionEndRunBundle {
        outcome,
        multiplayer,
        boards,
        eligible_submission,
        submission_unavailable_reason: unavailable_reason,
    };
    bundle.validate().map_err(|error| error.to_string())?;
    // Cross-check the immutable campaign bytes even for browse-only sessions.
    // This catches accidental replacement of the bootstrap capture before a
    // future admission implementation can make the run eligible.
    if starting_campaign_bytes.is_empty() {
        return Err("captured starting campaign is empty".to_owned());
    }
    Ok(bundle)
}

fn signed_admission_boards(
    mission_id: &str,
    signed: &SignedRankedMissionAdmission,
) -> Result<Vec<MissionEndBoard>, String> {
    let lifecycle = signed
        .lifecycle
        .lock()
        .map_err(|_| "ranked multiplayer lifecycle lock is poisoned".to_owned())?;
    let config = if let Some(host) = lifecycle.ranked_session() {
        &host.genesis().claim.ranked_session
    } else if let Some(client) = lifecycle.ranked_client() {
        &client.session_genesis.claim.ranked_session
    } else if let Some(reason) = lifecycle.browse_only_reason() {
        return Err(format!("ranked multiplayer was downgraded: {reason}"));
    } else {
        return Err("ranked multiplayer admission is unresolved at mission end".to_owned());
    };
    if config.mission_id != mission_id {
        return Err("ranked multiplayer config names another mission".to_owned());
    }
    let category = match signed.scope_request {
        ScopeRequestV1::IndividualLevel => BoardCategoryV1::IndividualLevel,
        ScopeRequestV1::CampaignGenesis | ScopeRequestV1::CampaignContinuation { .. } => {
            BoardCategoryV1::Campaign
        }
    };
    let boards = metric_boards(
        LeaderboardQuerySubjectV1::Mission,
        Some(mission_id.to_owned()),
        Some(category),
        config.content_manifest_sha256,
        config.rules_config_sha256,
        config.ruleset_manifest_sha256,
        config.competition_manifest_sha256,
        None,
        &signed.requested_metrics,
    );
    if boards.is_empty() {
        return Err("ranked multiplayer admission exposes no supported board metrics".to_owned());
    }
    Ok(boards)
}

fn browse_boards(
    mission_id: &str,
    multiplayer: bool,
    preferences: &LeaderboardPreferences,
    metadata: &LeaderboardMetadataV1,
) -> Result<Vec<MissionEndBoard>, String> {
    let mission = metadata
        .missions
        .iter()
        .find(|mission| mission.mission_id == mission_id)
        .ok_or_else(|| format!("mission `{mission_id}` is not published for leaderboards"))?;
    let category = match preferences.preferred_scope {
        LeaderboardScope::IndividualLevel => BoardCategoryV1::IndividualLevel,
        LeaderboardScope::Campaign => BoardCategoryV1::Campaign,
        LeaderboardScope::FullCampaign => {
            return Err(
                "full-campaign boards require a verified campaign-chain context".to_owned(),
            );
        }
    };
    let ruleset = metadata
        .rulesets
        .iter()
        .filter(|ruleset| {
            ruleset.categories.contains(&category)
                && ruleset.content
                    == RunContentIdentityV1::Mission {
                        content_manifest_sha256: mission.content_manifest_sha256,
                    }
                && preferences
                    .preferred_preset_id
                    .as_deref()
                    .is_none_or(|id| ruleset.preset_id.as_str() == id)
                && preferences
                    .preferred_difficulty_id
                    .as_deref()
                    .is_none_or(|id| ruleset.difficulty_id.as_str() == id)
        })
        .min_by_key(|ruleset| ruleset.ruleset_manifest_sha256)
        .ok_or_else(|| {
            format!(
                "no published {:?} ruleset matches mission `{mission_id}` and the selected board facets",
                preferences.preferred_scope
            )
        })?;
    // The current transport does not expose an authenticated final roster to
    // this layer. A multiplayer browse query therefore leaves player count
    // unfiltered instead of falsely presenting the single-player default as
    // the just-played composition.
    let max_players = (!multiplayer)
        .then_some(preferences.preferred_max_concurrent_players)
        .flatten();
    let subject = LeaderboardQuerySubjectV1::Mission;
    let mut boards = metric_boards(
        subject,
        Some(mission_id.to_owned()),
        Some(category),
        mission.content_manifest_sha256,
        ruleset.rules_config_sha256,
        ruleset.ruleset_manifest_sha256,
        None,
        max_players,
        &ruleset.metrics,
    );
    if let Some(competition) = select_competition(
        metadata,
        mission_id,
        category,
        ruleset.content,
        ruleset.rules_config_sha256,
        ruleset.ruleset_manifest_sha256,
        max_players,
        preferences.preferred_competition_id.as_deref(),
    ) {
        boards.push(MissionEndBoard {
            tab: LeaderboardTab::Challenge,
            label: competition.manifest.display_name.clone(),
            query: query(
                subject,
                Some(mission_id.to_owned()),
                Some(category),
                mission.content_manifest_sha256,
                competition.manifest.rules_config_sha256,
                competition.manifest.ruleset_manifest_sha256,
                Some(competition.competition_manifest_sha256),
                max_players,
                competition.manifest.metric,
            ),
        });
    }
    if boards.is_empty() {
        return Err("selected ruleset publishes no score or time boards".to_owned());
    }
    Ok(boards)
}

fn authorized_boards(
    input: &MissionEndSubmissionInput,
    metadata: Option<&LeaderboardMetadataV1>,
) -> Result<Vec<MissionEndBoard>, String> {
    let request = &input.offer_request;
    let ranked = &request.session_genesis.claim.ranked_session;
    let category = match request.scope_request {
        ScopeRequestV1::IndividualLevel => BoardCategoryV1::IndividualLevel,
        ScopeRequestV1::CampaignGenesis | ScopeRequestV1::CampaignContinuation { .. } => {
            BoardCategoryV1::Campaign
        }
    };
    let mut boards = metric_boards(
        LeaderboardQuerySubjectV1::Mission,
        Some(request.mission_id.clone()),
        Some(category),
        ranked.content_manifest_sha256,
        ranked.rules_config_sha256,
        ranked.ruleset_manifest_sha256,
        None,
        Some(request.max_concurrent_players),
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
                    LeaderboardQuerySubjectV1::Mission,
                    Some(request.mission_id.clone()),
                    Some(category),
                    ranked.content_manifest_sha256,
                    ranked.rules_config_sha256,
                    ranked.ruleset_manifest_sha256,
                    Some(competition_sha256),
                    Some(request.max_concurrent_players),
                    competition.manifest.metric,
                ),
            });
        }
    }
    if boards.is_empty() {
        return Err("ranked admission requested no supported board metrics".to_owned());
    }
    Ok(boards)
}

fn metric_boards(
    subject_kind: LeaderboardQuerySubjectV1,
    mission_id: Option<String>,
    mission_scope: Option<BoardCategoryV1>,
    content_identity_sha256: robin_run_protocol::Digest32,
    rules_config_sha256: robin_run_protocol::Digest32,
    ruleset_manifest_sha256: robin_run_protocol::Digest32,
    competition_manifest_sha256: Option<robin_run_protocol::Digest32>,
    max_concurrent_players: Option<u16>,
    metrics: &[BoardMetricV1],
) -> Vec<MissionEndBoard> {
    [
        (BoardMetricV1::OriginalScore, LeaderboardTab::Score, "Score"),
        (BoardMetricV1::FastestSuccess, LeaderboardTab::Time, "Time"),
    ]
    .into_iter()
    .filter(|(metric, _, _)| metrics.contains(metric))
    .map(|(metric, tab, label)| MissionEndBoard {
        tab,
        label: label.to_owned(),
        query: query(
            subject_kind,
            mission_id.clone(),
            mission_scope,
            content_identity_sha256,
            rules_config_sha256,
            ruleset_manifest_sha256,
            competition_manifest_sha256,
            max_concurrent_players,
            metric,
        ),
    })
    .collect()
}

fn query(
    subject_kind: LeaderboardQuerySubjectV1,
    mission_id: Option<String>,
    mission_scope: Option<BoardCategoryV1>,
    content_identity_sha256: robin_run_protocol::Digest32,
    rules_config_sha256: robin_run_protocol::Digest32,
    ruleset_manifest_sha256: robin_run_protocol::Digest32,
    competition_manifest_sha256: Option<robin_run_protocol::Digest32>,
    max_concurrent_players: Option<u16>,
    metric: BoardMetricV1,
) -> LeaderboardQueryV1 {
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

fn select_competition<'a>(
    metadata: &'a LeaderboardMetadataV1,
    mission_id: &str,
    category: BoardCategoryV1,
    content: RunContentIdentityV1,
    rules_config_sha256: robin_run_protocol::Digest32,
    ruleset_manifest_sha256: robin_run_protocol::Digest32,
    max_players: Option<u16>,
    preferred_id: Option<&str>,
) -> Option<&'a robin_run_protocol::CompetitionSummaryV1> {
    metadata
        .competitions
        .iter()
        .filter(|competition| {
            competition.state == CompetitionStateV1::Active
                && competition.manifest.subject
                    == LeaderboardSubjectV1::Mission {
                        mission_id: mission_id.to_owned(),
                        category,
                    }
                && competition.manifest.content == content
                && competition.manifest.rules_config_sha256 == rules_config_sha256
                && competition.manifest.ruleset_manifest_sha256 == ruleset_manifest_sha256
                && max_players.is_none_or(|players| {
                    competition
                        .manifest
                        .participant_composition
                        .max_concurrent_players()
                        == players
                })
                && preferred_id.is_none_or(|id| competition.manifest.competition_id.as_str() == id)
        })
        .min_by_key(|competition| competition.competition_manifest_sha256)
}

#[cfg(test)]
mod tests {
    use super::admission::*;
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};
    use robin_engine::campaign::Campaign;
    use robin_engine::engine::{SimConfig, SimulationFrameInput};
    use robin_engine::replay::{ReplayFile, ReplayFrame, ReplayHeader};
    use robin_engine::replay_rankability::ReplayRankability;
    use robin_run_protocol::{
        ArtifactRefV1, CampaignChainReceiptV1, CampaignChainStateV1, CampaignRosterContinuityV1,
        ChallengeNonce32, Digest32, FreshRunPreflightGrantClaimV1, FreshRunPreflightGrantV1,
        FreshRunPreflightRequestClaimV1, FreshRunPreflightRequestV1, FreshRunScopeV1,
        MissionFacetV1, OfficialContentEditionV1, OfficialContentSubjectV1, OpaqueId, PublicKey32,
        RANKED_CAMPAIGN_MEDIA_TYPE_V1, RankedSessionConfigV1, ResourceLocaleRootV1, RulesetFacetV1,
        RunContentIdentityV1, Signature64, SimulationSeed64, SpeechTimingAuthorityV1,
    };
    use std::collections::BTreeMap;

    #[test]
    fn terminal_restore_terminal_owns_a_fresh_unranked_preparation() {
        let mut assets = robin_engine::engine::LevelAssets::new();
        let mut engine = robin_engine::engine::Engine::new_for_test(
            800.0,
            600.0,
            Campaign::default(),
            &mut assets,
        )
        .unwrap();
        let mut host = crate::host::Host::scratch(800.0, 600.0);
        let mut game = crate::game::Game::default();
        let checkpoint =
            crate::save_file::GameRuntimeSnapshot::capture(&engine, &host, &game).unwrap();
        let mut runtime = MissionLeaderboardRuntime::new(
            engine.campaign(),
            "RestartTest".into(),
            false,
            RankedMissionAdmission::browse_only("initial fixture admission"),
            None,
        );
        runtime.after_state_restore(engine.campaign());
        let first = runtime.capture_terminal(MissionEndOutcome::Lost).unwrap();
        assert!(
            matches!(&first.admission, RankedMissionAdmission::BrowseOnly { reason } if reason == "initial fixture admission")
        );
        assert!(runtime.capture_terminal(MissionEndOutcome::Lost).is_err());
        engine.test_set_frame_counter(123);
        checkpoint
            .apply_to_with_game(&mut engine, &mut host, &mut game, &assets)
            .unwrap();
        runtime.after_state_restore(engine.campaign());
        let second = runtime.capture_terminal(MissionEndOutcome::Lost).unwrap();
        assert_eq!(first.mission_id, second.mission_id);
        assert_eq!(
            second.starting_campaign_bytes.as_ref(),
            bitcode::encode(engine.campaign())
        );
        assert!(
            matches!(second.admission, RankedMissionAdmission::BrowseOnly { ref reason } if reason.contains("original archived ranked admission"))
        );
        assert!(second.ranked_multiplayer_port.is_none());
        assert!(runtime.capture_terminal(MissionEndOutcome::Lost).is_err());
    }

    fn digest(byte: u8) -> Digest32 {
        Digest32::from_bytes([byte; 32])
    }

    fn metadata() -> LeaderboardMetadataV1 {
        LeaderboardMetadataV1 {
            schema_version: SCHEMA_VERSION_V1,
            missions: vec![MissionFacetV1 {
                mission_id: "H01".to_owned(),
                display_name: "Huntingdon".to_owned(),
                content_manifest_sha256: digest(1),
            }],
            rulesets: vec![RulesetFacetV1 {
                ruleset_manifest_sha256: digest(3),
                rules_config_sha256: digest(2),
                display_name: "Original".to_owned(),
                preset_id: OpaqueId::new("original").unwrap(),
                preset_name: "Original".to_owned(),
                difficulty_id: OpaqueId::new("normal").unwrap(),
                difficulty_name: "Normal".to_owned(),
                content: RunContentIdentityV1::Mission {
                    content_manifest_sha256: digest(1),
                },
                categories: vec![BoardCategoryV1::IndividualLevel, BoardCategoryV1::Campaign],
                metrics: vec![BoardMetricV1::OriginalScore, BoardMetricV1::FastestSuccess],
                supports_full_campaign_boards: false,
            }],
            competitions: Vec::new(),
            full_campaign: None,
        }
    }

    fn official_fresh_setup(
        host_key: &SigningKey,
        ranked_session: RankedSessionConfigV1,
    ) -> OfficialRankedSessionSetupV1 {
        fn public_key(key: &SigningKey) -> PublicKey32 {
            PublicKey32::from_bytes(key.verifying_key().to_bytes())
        }
        fn signature(key: &SigningKey, bytes: &[u8]) -> Signature64 {
            Signature64::from_bytes(key.sign(bytes).to_bytes())
        }

        let authority_key = SigningKey::from_bytes(&[0x7a; 32]);
        let request_claim = FreshRunPreflightRequestClaimV1 {
            schema_version: SCHEMA_VERSION_V1,
            request_nonce: ChallengeNonce32::from_bytes([0x31; 32]),
            host_public_key: public_key(host_key),
            replay_session_id: digest(0x32),
            host_participant_instance_id: digest(0x33),
            host_nonce: ChallengeNonce32::from_bytes([0x34; 32]),
            scope: FreshRunScopeV1::IndividualLevel,
            starting_campaign: ArtifactRefV1 {
                sha256: ranked_session.starting_campaign_sha256,
                byte_length: ranked_session.starting_campaign_byte_length,
                media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
            },
            ranked_session: ranked_session.clone(),
        };
        let request = FreshRunPreflightRequestV1 {
            host_signature: signature(host_key, &request_claim.signing_bytes().unwrap()),
            claim: request_claim,
            algorithm: SignatureAlgorithmV1::Ed25519,
        };
        let grant_claim = FreshRunPreflightGrantClaimV1 {
            schema_version: SCHEMA_VERSION_V1,
            grant_id: OpaqueId::new("runtime-fresh-grant-test").unwrap(),
            grant_nonce: ChallengeNonce32::from_bytes([0x35; 32]),
            grant_authority_public_key: public_key(&authority_key),
            host_public_key: public_key(host_key),
            grant_request_sha256: request.canonical_digest().unwrap(),
            ranked_session_sha256: ranked_session.canonical_digest().unwrap(),
            replay_session_id: request.claim.replay_session_id,
            host_participant_instance_id: request.claim.host_participant_instance_id,
            host_nonce: request.claim.host_nonce,
            scope: request.claim.scope,
            starting_campaign: request.claim.starting_campaign.clone(),
            admitted_at_unix_ms: 1_000,
            expires_at_unix_ms: 2_000,
        };
        let grant = FreshRunPreflightGrantV1 {
            authority_signature: signature(&authority_key, &grant_claim.signing_bytes().unwrap()),
            claim: grant_claim,
            algorithm: SignatureAlgorithmV1::Ed25519,
        };
        OfficialRankedSessionSetupV1 {
            ranked_session,
            custom_package_present: false,
            run_preflight: RankedRunPreflightAdmissionV1::Fresh { request, grant },
            run_preflight_grant_public_key: public_key(&authority_key),
            trusted_now_unix_ms: 1_500,
        }
    }

    fn test_ranked_config(campaign_bytes: &[u8]) -> RankedSessionConfigV1 {
        let mission_id = "Dem_Lei_MP";
        RankedSessionConfigV1 {
            custom_rules_config: None,
            custom_canonical_campaign: None,
            schema_version: SCHEMA_VERSION_V1,
            mission_id: mission_id.to_owned(),
            content_edition: OfficialContentEditionV1::Demo,
            content_subject: OfficialContentSubjectV1::FieldMission {
                mission_id: mission_id.to_owned(),
            },
            simulation_seed: SimulationSeed64::new(7),
            starting_campaign_sha256: Digest32::digest_bytes(campaign_bytes),
            starting_campaign_byte_length: u64::try_from(campaign_bytes.len()).unwrap(),
            prepared_inputs_projection_sha256: digest(2),
            prepared_mission_inputs_seal_sha256: digest(3),
            build_manifest_sha256: digest(4),
            content_manifest_sha256: digest(5),
            campaign_content_manifest_sha256: None,
            rules_config_sha256: digest(6),
            ruleset_manifest_sha256: digest(7),
            competition_manifest_sha256: None,
            spellforge_content_sha256: None,
            resource_locale_root: ResourceLocaleRootV1::new("1033").unwrap(),
            speech_timing: SpeechTimingAuthorityV1::BaseInstallation,
        }
    }

    pub(super) fn signed_single_player_admission(
        campaign_bytes: &[u8],
    ) -> (RankedMissionAdmission, robin_engine::replay::ReplayData) {
        signed_single_player_admission_for_key(
            campaign_bytes,
            &ed25519_dalek::SigningKey::from_bytes(&[0x44; 32]),
        )
    }

    pub(super) fn signed_single_player_admission_for_key(
        campaign_bytes: &[u8],
        key: &ed25519_dalek::SigningKey,
    ) -> (RankedMissionAdmission, robin_engine::replay::ReplayData) {
        let mission_id = "Dem_Lei_MP";
        let config = test_ranked_config(campaign_bytes);
        let host = crate::leaderboard_ranked_session::RankedSessionHost::new_official(
            key,
            robin_engine::multiplayer::NET_PROTOCOL_VERSION,
            official_fresh_setup(key, config),
        )
        .unwrap();
        let replay = ReplayFile {
            header: ReplayHeader {
                mission_id: mission_id.to_owned(),
                mission_assets: robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                    mission_id, mission_id, mission_id,
                )
                .expect("valid built-in leaderboard-runtime test descriptor"),
                rng_seed: 7,
                sim_config: SimConfig::default(),
                spellforge_package: None,
                version: robin_engine::replay::REPLAY_SCHEMA_VERSION,
                total_frames: 1,
                rankability: ReplayRankability::rankable(),
                campaign: campaign_bytes.to_vec(),
            },
            frames: BTreeMap::from([(
                0,
                ReplayFrame {
                    timeline_before: 0,
                    timeline_after: 1,
                    input: SimulationFrameInput::default(),
                    host_controls: Vec::new(),
                },
            )]),
            hashes: BTreeMap::new(),
            save_markers: BTreeMap::new(),
            load_backs: BTreeMap::new(),
        }
        .try_into()
        .expect("valid replay fixture");
        (
            RankedMissionAdmission::Signed(SignedRankedMissionAdmission {
                lifecycle: Arc::new(Mutex::new(
                    crate::leaderboard_ranked_session::RankedSessionLifecycle::ranked(host),
                )),
                scope_request: ScopeRequestV1::IndividualLevel,
                requested_metrics: vec![BoardMetricV1::OriginalScore],
                campaign_controller_public_key: None,
            }),
            replay,
        )
    }

    #[test]
    fn multiplayer_host_proposal_may_only_change_published_board_documents() {
        let local = test_ranked_config(b"campaign");
        let mut board_only = local.clone();
        board_only.ruleset_manifest_sha256 = digest(0xa1);
        board_only.campaign_content_manifest_sha256 = Some(digest(0xa2));
        validate_host_proposal_against_local_prepared(&local, &board_only).unwrap();

        let mut changed_simulation = board_only;
        changed_simulation.simulation_seed = SimulationSeed64::new(8);
        assert!(
            validate_host_proposal_against_local_prepared(&local, &changed_simulation).is_err()
        );
    }

    #[test]
    fn early_client_progress_survives_a_late_peer_without_becoming_unbounded() {
        // An early client may spend almost one full phase waiting for the last
        // authenticated peer. Fresh setup therefore gets one composed initial
        // interval; a valid campaign selection then resets inactivity, while
        // the independent total bound continues to advance.
        assert_eq!(
            ranked_preflight_timeout(
                RANKED_PREFLIGHT_PHASE_TIMEOUT_MS + 1,
                RANKED_PREFLIGHT_PHASE_TIMEOUT_MS + 1,
                RANKED_PREFLIGHT_INITIAL_CLIENT_TIMEOUT_MS,
            ),
            None
        );
        assert_eq!(
            ranked_preflight_timeout(
                2 * RANKED_PREFLIGHT_PHASE_TIMEOUT_MS + 1,
                1,
                RANKED_PREFLIGHT_PHASE_TIMEOUT_MS,
            ),
            None
        );
        assert_eq!(
            ranked_preflight_timeout(
                RANKED_PREFLIGHT_TOTAL_TIMEOUT_MS,
                1,
                RANKED_PREFLIGHT_INITIAL_CLIENT_TIMEOUT_MS,
            ),
            Some(RankedPreflightTimeout::Total)
        );
    }

    #[test]
    fn ranked_preflight_no_progress_times_out_per_phase() {
        assert_eq!(
            ranked_preflight_timeout(
                RANKED_PREFLIGHT_PHASE_TIMEOUT_MS - 1,
                RANKED_PREFLIGHT_PHASE_TIMEOUT_MS,
                RANKED_PREFLIGHT_PHASE_TIMEOUT_MS,
            ),
            Some(RankedPreflightTimeout::PhaseInactivity)
        );
        assert_eq!(
            ranked_preflight_timeout(
                RANKED_PREFLIGHT_INITIAL_CLIENT_TIMEOUT_MS,
                RANKED_PREFLIGHT_INITIAL_CLIENT_TIMEOUT_MS,
                RANKED_PREFLIGHT_INITIAL_CLIENT_TIMEOUT_MS,
            ),
            Some(RankedPreflightTimeout::PhaseInactivity)
        );
    }

    fn continuation_selection_fixture() -> (
        CampaignContinuationReceiptSelectionRequestV1,
        CampaignChainReceiptV1,
        PublicKey32,
        PublicKey32,
    ) {
        let host = PublicKey32::from_bytes([1; 32]);
        let controller = PublicKey32::from_bytes([2; 32]);
        let mut config = test_ranked_config(b"campaign");
        config.content_edition = OfficialContentEditionV1::Full;
        config.campaign_content_manifest_sha256 = Some(digest(8));
        let request = CampaignContinuationReceiptSelectionRequestV1::from_lobby(
            RankedPreflightLobbyV1 {
                host_public_key: host,
                max_concurrent_players: 2,
                participant_public_keys: vec![host, controller],
            },
            config.clone(),
            CampaignRosterContinuityV1::ExactSameAuthenticatedKeysEverySession,
        )
        .unwrap();
        let receipt = CampaignChainReceiptV1 {
            schema_version: SCHEMA_VERSION_V1,
            chain_id: OpaqueId::new("chain-runtime-test").unwrap(),
            predecessor_run_id: OpaqueId::new("run-runtime-test").unwrap(),
            predecessor_verification_sha256: digest(9),
            expected_starting_campaign: request.starting_campaign.clone(),
            rules_config_sha256: config.rules_config_sha256,
            ruleset_manifest_sha256: config.ruleset_manifest_sha256,
            competition_manifest_sha256: config.competition_manifest_sha256,
            campaign_content_manifest_sha256: config.campaign_content_manifest_sha256.unwrap(),
            expected_max_concurrent_players: 2,
            participant_public_keys: vec![host, controller],
            campaign_controller_public_key: controller,
            state: CampaignChainStateV1::Active,
        };
        (request, receipt, host, controller)
    }

    #[test]
    fn transport_lobby_discovers_exact_multiplayer_receipt_without_pretransport_snapshot() {
        let (request, receipt, _, controller) = continuation_selection_fixture();
        let mut store = crate::leaderboard_chains::CampaignChainStore::empty();
        store.accepted(receipt.clone()).unwrap();
        assert!(
            select_campaign_receipt_from_store(&store, &request, controller)
                .unwrap()
                .is_some()
        );

        let mut different_cap = receipt;
        different_cap.expected_max_concurrent_players = 3;
        let mut different_store = crate::leaderboard_chains::CampaignChainStore::empty();
        different_store.accepted(different_cap).unwrap();
        assert!(
            select_campaign_receipt_from_store(&different_store, &request, controller)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn controller_signs_only_the_exact_selected_continuation_claim() {
        let (request, receipt, host, controller) = continuation_selection_fixture();
        let selection = CampaignContinuationReceiptSelectionV1 { request, receipt };
        let mut claim = crate::leaderboard_ranked_session::RankedSessionHost::prepare_campaign_continuation_preflight_claim(
            host,
            selection.request.ranked_session.clone(),
            selection.preflight_setup().unwrap(),
        )
        .unwrap();
        validate_controller_preflight_claim(&claim, &selection, host, controller).unwrap();

        claim.predecessor_verification_sha256 = digest(0xaa);
        assert!(validate_controller_preflight_claim(&claim, &selection, host, controller).is_err());
    }

    #[test]
    fn archived_admission_survives_process_restart_and_uses_the_normal_board() {
        use crate::replay_archive::MissionArchive;
        use crate::replay_recording::SharedReplayRecorder;
        use crate::save_file::{GameSaveFile, SaveProvenance};
        use robin_engine::replay::ReplayRecorder;

        let directory = tempfile::tempdir().unwrap();
        let mut assets = robin_engine::engine::LevelAssets::new();
        let mut engine = robin_engine::engine::Engine::new_for_test(
            1024.0,
            768.0,
            Campaign::default(),
            &mut assets,
        )
        .unwrap();
        let campaign_bytes = bitcode::encode(engine.campaign());
        let (admission, replay) = signed_single_player_admission(&campaign_bytes);
        let service = Arc::new(crate::replay_service::ReplayService::default());
        let archive = MissionArchive::create(&directory.path().join("original")).unwrap();
        let recorder = ReplayRecorder::with_writer(
            crate::game_session::replay_init::root_writer(
                archive.writer().unwrap(),
                service.recording().begin_recording(),
            ),
            replay.header().mission_id.clone(),
            replay.header().mission_assets.clone(),
            7,
            engine.sim_config(),
            engine.campaign(),
        )
        .unwrap();
        let recorder = SharedReplayRecorder::archived(recorder, archive);
        service
            .recording()
            .install_capture_recorder(Some(recorder.clone()));
        service.recording().set_ranked_source(admission);
        let mut host = crate::host::Host::scratch(1024.0, 768.0);
        let mut game = crate::game::Game::default();
        let mut save = GameSaveFile::capture_with_game(
            &engine,
            &host,
            &game,
            1,
            replay.header().mission_assets.clone(),
            "ranked checkpoint".into(),
            SaveProvenance::new("Mission".into(), 0, "Player".into()).unwrap(),
        )
        .unwrap();
        service.recording().attach_save_boundary(&mut save).unwrap();
        let original: MissionEndSubmissionInput = serde_json::from_slice(
            &std::fs::read(directory.path().join("original/ranked.json")).unwrap(),
        )
        .unwrap();
        drop(recorder);
        drop(service);

        let service = Arc::new(crate::replay_service::ReplayService::default());
        let archive = MissionArchive::create(&directory.path().join("new-process")).unwrap();
        let recorder = ReplayRecorder::with_writer(
            crate::game_session::replay_init::root_writer(
                archive.writer().unwrap(),
                service.recording().begin_recording(),
            ),
            replay.header().mission_id.clone(),
            replay.header().mission_assets.clone(),
            7,
            engine.sim_config(),
            engine.campaign(),
        )
        .unwrap();
        let recorder = SharedReplayRecorder::archived(recorder, archive);
        service
            .recording()
            .install_capture_recorder(Some(recorder.clone()));
        save.clone()
            .apply_to_with_game(&mut engine, &mut host, &mut game, &assets)
            .unwrap();
        let crate::replay_recording::ReplayRestoreBoundary {
            ordinal,
            timeline_frame: timeline,
            marker_ordinal: target,
        } = recorder.restore(&save, &service.recording()).unwrap();
        recorder.write_load_back(ordinal, target.unwrap(), false);
        recorder
            .commit_restore_boundary(
                timeline,
                robin_engine::replay::state_hash(&engine),
                &crate::mission_replays::RecordingIndex::disabled(),
            )
            .unwrap();
        let restored = service.exports().restored_ranked_input().unwrap().unwrap();
        assert_eq!(
            restored, original,
            "the original signatures and campaign must survive, without minting new admission"
        );
        let data = service.exports().snapshot().unwrap().parse_sync().unwrap();
        validate_archived_ranked_input(&restored, &data).unwrap();
        let mut preparation = MissionEndPreparation {
            mission_id: replay.header().mission_id.clone(),
            multiplayer: false,
            starting_campaign_bytes: Arc::from(b"unrelated fresh startup".as_slice()),
            admission: RankedMissionAdmission::browse_only("fresh process has no admission"),
            ranked_multiplayer_port: None,
            preferences: LeaderboardPreferences::default(),
            metadata: MetadataLoad::Failed("metadata must not block signed submission".into()),
            outcome: Some(MissionEndOutcome::Won),
        };
        let bundle = preparation
            .poll_bundle(&service.exports())
            .unwrap()
            .unwrap();
        assert_eq!(bundle.eligible_submission.unwrap(), original);
        assert!(
            bundle.boards.iter().all(|board| matches!(
                board.query.subject_kind,
                LeaderboardQuerySubjectV1::Mission
            ))
        );
        let mut forged = restored;
        forged.offer_request.session_genesis.host_signature = Signature64::from_bytes([0x55; 64]);
        assert!(validate_archived_ranked_input(&forged, &data).is_err());
    }

    #[test]
    fn signed_single_player_win_materializes_exact_terminal_submission() {
        let campaign_bytes = bitcode::encode(&Campaign::default());
        let (mut admission, replay) = signed_single_player_admission(&campaign_bytes);
        admission
            .materialize_terminal_from_replay(
                "Dem_Lei_MP",
                Arc::from(campaign_bytes.clone()),
                &replay,
            )
            .unwrap();

        let bundle = build_bundle(
            "Dem_Lei_MP",
            false,
            Arc::from(campaign_bytes),
            MissionEndOutcome::Won,
            &admission,
            &LeaderboardPreferences::default(),
            None,
        )
        .unwrap();
        let input = bundle.eligible_submission.unwrap();
        assert_eq!(input.offer_request.max_concurrent_players, 1);
        assert_eq!(input.offer_request.participant_instance_count, 1);
        assert_eq!(input.replay_session_transcript.max_concurrent_players, 1);
    }

    #[test]
    fn signed_lost_and_interrupted_runs_keep_boards_but_cannot_submit() {
        for outcome in [MissionEndOutcome::Lost, MissionEndOutcome::Interrupted] {
            let campaign_bytes = bitcode::encode(&Campaign::default());
            let (mut admission, replay) = signed_single_player_admission(&campaign_bytes);
            admission
                .materialize_terminal_from_replay(
                    "Dem_Lei_MP",
                    Arc::from(campaign_bytes.clone()),
                    &replay,
                )
                .unwrap();
            let bundle = build_bundle(
                "Dem_Lei_MP",
                false,
                Arc::from(campaign_bytes),
                outcome,
                &admission,
                &LeaderboardPreferences::default(),
                None,
            )
            .unwrap();
            assert_eq!(bundle.boards.len(), 1);
            assert!(bundle.eligible_submission.is_none());
            assert_eq!(
                bundle.submission_unavailable_reason.as_deref(),
                Some("only won missions can be submitted")
            );
        }
    }

    #[test]
    fn browse_only_builds_real_server_facets_but_never_submission_authority() {
        let bundle = build_bundle(
            "H01",
            false,
            Arc::from([1_u8, 2, 3]),
            MissionEndOutcome::Won,
            &RankedMissionAdmission::browse_only("missing signed genesis"),
            &LeaderboardPreferences::default(),
            Some(&metadata()),
        )
        .unwrap();

        assert_eq!(bundle.boards.len(), 2);
        assert!(bundle.eligible_submission.is_none());
        assert_eq!(
            bundle.submission_unavailable_reason.as_deref(),
            Some("missing signed genesis")
        );
        assert!(
            bundle
                .boards
                .iter()
                .all(|board| board.query.content_identity_sha256 == digest(1))
        );
    }

    #[test]
    fn every_terminal_outcome_gets_the_same_browse_boards() {
        for outcome in [
            MissionEndOutcome::Won,
            MissionEndOutcome::Lost,
            MissionEndOutcome::Interrupted,
        ] {
            let bundle = build_bundle(
                "H01",
                true,
                Arc::from([9_u8]),
                outcome,
                &RankedMissionAdmission::browse_only("no authority"),
                &LeaderboardPreferences::default(),
                Some(&metadata()),
            )
            .unwrap();
            assert_eq!(bundle.boards.len(), 2);
            assert!(bundle.multiplayer);
        }
    }

    #[test]
    fn missing_published_mission_fails_instead_of_fabricating_query_digests() {
        let error = build_bundle(
            "unknown",
            false,
            Arc::from([1_u8]),
            MissionEndOutcome::Won,
            &RankedMissionAdmission::browse_only("no authority"),
            &LeaderboardPreferences::default(),
            Some(&metadata()),
        )
        .unwrap_err();
        assert!(error.contains("not published"), "{error}");
    }
}
