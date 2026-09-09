//! Non-blocking mission-end leaderboard and ranked-submission coordination.
//!
//! The game-loop owner calls [`MissionEndLeaderboardController::poll`] once
//! per rendered frame and feeds it explicit [`MissionEndLeaderboardAction`]s.
//! No method in this module awaits a network request or performs a synchronous
//! HTTP operation. Boards are available after wins, losses, and interrupted
//! attempts; upload consent is accepted only for eligible won missions.
//!
//! Multiplayer final co-signing is deliberately an injected, typed task. The
//! transport implementation must authenticate every response to its occupied
//! seat. This module independently checks that the returned envelope is exact,
//! that the signer set equals the offer's participant set, and that every
//! Ed25519 signature covers the canonical final submission bytes.

use crate::leaderboard_http::HttpTask;
use crate::leaderboard_preferences::{LeaderboardPreferences, LeaderboardTab};
use crate::leaderboard_service::{
    LeaderboardApi, decode_board, decode_offer, decode_submission_accepted,
};
use robin_run_protocol::{
    ArtifactRefV1, BoardMetricV1, CampaignAggregationConsentV1,
    CampaignContinuationAuthorizationClaimV1, CampaignContinuationAuthorizationV1,
    CanonicalDocument as _, Digest32, InitialStateExpectationV1, LeaderboardPageV1,
    LeaderboardQueryV1, PublicKey32, RANKED_CAMPAIGN_MEDIA_TYPE_V1, RANKED_REPLAY_MEDIA_TYPE_V1,
    ReplayArtifactV1, ReplaySessionTranscriptV1, SCHEMA_VERSION_V1, SignatureAlgorithmV1,
    SignedSubmissionV1, SubmissionAcceptedV1, SubmissionArtifactsV1, SubmissionEnvelopeV1,
    SubmissionOfferRequestV1, SubmissionOfferV1, Validate as _,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionEndOutcome {
    Won,
    Lost,
    Interrupted,
}

impl MissionEndOutcome {
    pub const fn can_submit(self) -> bool {
        matches!(self, Self::Won)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionEndBoard {
    pub tab: LeaderboardTab,
    pub label: String,
    pub query: LeaderboardQueryV1,
}

impl MissionEndBoard {
    fn validate(&self) -> Result<(), MissionEndLeaderboardError> {
        if self.label.is_empty()
            || self.label.len() > 64
            || self.label.chars().any(char::is_control)
        {
            return Err(MissionEndLeaderboardError::InvalidRunBundle(
                "board labels must contain 1..=64 safe bytes".to_owned(),
            ));
        }
        self.query
            .validate()
            .map_err(|error| MissionEndLeaderboardError::InvalidRunBundle(error.to_string()))
    }
}

/// Lossless runtime evidence required after the server authors an offer.
///
/// `starting_campaign_bytes` is the exact bitcode campaign captured before
/// mission construction. `replay_session_transcript` is identity-bearing
/// co-sign evidence and remains separate from the canonical gameplay replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionEndSubmissionInput {
    pub offer_request: SubmissionOfferRequestV1,
    pub replay_session_transcript: ReplaySessionTranscriptV1,
    pub requested_metrics: Vec<BoardMetricV1>,
    /// Immutable controller established by campaign genesis. Required only
    /// for a server-recognized continuation and not assumed to be this
    /// session's host.
    pub campaign_controller_public_key: Option<PublicKey32>,
    #[serde(with = "arc_bytes")]
    pub starting_campaign_bytes: Arc<[u8]>,
}

impl MissionEndSubmissionInput {
    pub(crate) fn validate(&self) -> Result<(), MissionEndLeaderboardError> {
        self.offer_request
            .validate()
            .map_err(invalid_bundle_protocol)?;
        self.replay_session_transcript
            .validate()
            .map_err(invalid_bundle_protocol)?;
        if self.starting_campaign_bytes.is_empty() {
            return Err(MissionEndLeaderboardError::InvalidRunBundle(
                "ranked starting campaign is empty".to_owned(),
            ));
        }
        if self.requested_metrics.is_empty()
            || !self
                .requested_metrics
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        {
            return Err(MissionEndLeaderboardError::InvalidRunBundle(
                "requested metrics must be non-empty and strictly sorted".to_owned(),
            ));
        }
        let ranked = &self.offer_request.session_genesis.claim.ranked_session;
        let is_continuation = matches!(
            &self.offer_request.scope_request,
            robin_run_protocol::ScopeRequestV1::CampaignContinuation { .. }
        );
        if self.campaign_controller_public_key.is_some() != is_continuation
            || self
                .campaign_controller_public_key
                .is_some_and(|controller| {
                    !self
                        .offer_request
                        .participant_claims
                        .iter()
                        .any(|participant| participant.public_key == controller)
                })
        {
            return Err(MissionEndLeaderboardError::InvalidRunBundle(
                "campaign continuation controller is missing or is not an authenticated participant"
                    .to_owned(),
            ));
        }
        let campaign_length = u64::try_from(self.starting_campaign_bytes.len()).map_err(|_| {
            MissionEndLeaderboardError::InvalidRunBundle(
                "starting campaign length does not fit the protocol".to_owned(),
            )
        })?;
        if Digest32::digest_bytes(&self.starting_campaign_bytes) != ranked.starting_campaign_sha256
            || campaign_length != ranked.starting_campaign_byte_length
            || self.replay_session_transcript.replay_session_id
                != self.offer_request.session_genesis.claim.replay_session_id
        {
            return Err(MissionEndLeaderboardError::InvalidRunBundle(
                "submission evidence does not match the signed ranked genesis".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionEndRunBundle {
    pub outcome: MissionEndOutcome,
    pub multiplayer: bool,
    pub boards: Vec<MissionEndBoard>,
    /// Present only when the recorder and ranked genesis passed their local
    /// eligibility checks. A tainted/missing capture remains browse-only.
    /// The compact replay is checked again immediately before authorization;
    /// this early flag is presentation state, not the trust boundary.
    pub eligible_submission: Option<MissionEndSubmissionInput>,
    pub submission_unavailable_reason: Option<String>,
}

impl MissionEndRunBundle {
    pub fn validate(&self) -> Result<(), MissionEndLeaderboardError> {
        if self.boards.is_empty() {
            return Err(MissionEndLeaderboardError::InvalidRunBundle(
                "mission-end view requires at least one board".to_owned(),
            ));
        }
        let mut tabs = Vec::new();
        for board in &self.boards {
            board.validate()?;
            if tabs.contains(&board.tab) {
                return Err(MissionEndLeaderboardError::InvalidRunBundle(
                    "mission-end board tabs must be unique".to_owned(),
                ));
            }
            tabs.push(board.tab);
        }
        if let Some(input) = &self.eligible_submission {
            if !self.outcome.can_submit() {
                return Err(MissionEndLeaderboardError::InvalidRunBundle(
                    "only won missions may expose a submission".to_owned(),
                ));
            }
            input.validate()?;
        }
        if self.eligible_submission.is_some() && self.submission_unavailable_reason.is_some() {
            return Err(MissionEndLeaderboardError::InvalidRunBundle(
                "submission cannot be both eligible and unavailable".to_owned(),
            ));
        }
        if self
            .submission_unavailable_reason
            .as_ref()
            .is_some_and(|reason| {
                reason.is_empty() || reason.len() > 500 || reason.chars().any(char::is_control)
            })
        {
            return Err(MissionEndLeaderboardError::InvalidRunBundle(
                "submission-unavailable reason must be a safe bounded message".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticipantSigningProgress {
    pub expected: Vec<PublicKey32>,
    pub signed: Vec<PublicKey32>,
}

impl ParticipantSigningProgress {
    pub fn validate(&self) -> Result<(), MissionEndLeaderboardError> {
        if self.expected.is_empty()
            || !strictly_sorted(&self.expected)
            || !strictly_sorted(&self.signed)
            || self
                .signed
                .iter()
                .any(|key| self.expected.binary_search(key).is_err())
        {
            return Err(MissionEndLeaderboardError::Authorization(
                "co-sign progress is not a canonical subset of expected participants".to_owned(),
            ));
        }
        Ok(())
    }
}

fn strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

pub trait MissionEndTask<T> {
    /// Return `None` while work remains pending. Implementations must never
    /// block the calling render frame.
    fn try_take(&mut self) -> Option<Result<T, String>>;
}

pub trait SubmissionAuthorizationTask: MissionEndTask<SignedSubmissionV1> {
    fn progress(&self) -> ParticipantSigningProgress;
}

pub trait MissionEndSubmissionAuthorizer {
    /// Start exact participant/controller authorization. Multiplayer
    /// implementations must keep every message typed and session-bound and
    /// must not expose generic signing. A transport signature over a run
    /// digest is correlation/consent evidence only: it cannot be inserted as
    /// a [`ParticipantSignatureV1`](robin_run_protocol::ParticipantSignatureV1).
    /// The returned protocol signatures must cover the exact finalized
    /// [`SubmissionEnvelopeV1::signing_bytes`] contract.
    fn begin(
        &mut self,
        request: SubmissionAuthorizationRequest,
    ) -> Result<Box<dyn SubmissionAuthorizationTask>, String>;

    /// Notify an authenticated remote campaign controller after the server has
    /// durably queued the exact signed submission. Local/single-player
    /// authorizers need no transport acknowledgement.
    fn submission_accepted(&mut self, _accepted: &SubmissionAcceptedV1) -> Result<(), String> {
        Ok(())
    }

    /// Whether this process owns durable verification polling for an accepted
    /// upload. A multiplayer host can upload for a remote campaign controller;
    /// in that case only the authenticated controller persists the watcher.
    fn owns_receipt_watch(&self) -> Result<bool, String> {
        Ok(true)
    }
}

/// Non-host participant state for a host-authored ranked submission. Consent
/// is local and explicit; the responder validates the typed host context
/// against this process's one canonical replay before exposing the fixed
/// co-sign operation to the durable identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerCoSignPoll {
    AwaitingConsent,
    AwaitingHost,
    Signing(ParticipantSigningProgress),
    ResponseSent(ParticipantSigningProgress),
    Accepted(SubmissionAcceptedV1),
    Failed(String),
}

pub trait MissionEndPeerCoSigner {
    fn consent(&mut self) -> Result<(), String>;
    fn poll(&mut self) -> PeerCoSignPoll;
    fn has_pending_work(&self) -> bool;
}

pub trait MissionEndReplayExporter {
    fn begin(&mut self) -> Result<Box<dyn MissionEndTask<Arc<[u8]>>>, String>;
}

pub trait MissionEndLeaderboardBackend {
    fn board(
        &mut self,
        query: LeaderboardQueryV1,
    ) -> Result<Box<dyn MissionEndTask<LeaderboardPageV1>>, String>;

    fn offer(
        &mut self,
        request: SubmissionOfferRequestV1,
    ) -> Result<Box<dyn MissionEndTask<SubmissionOfferV1>>, String>;

    fn submit(
        &mut self,
        submission: SignedSubmissionV1,
        replay_bytes: Arc<[u8]>,
        starting_campaign_bytes: Arc<[u8]>,
    ) -> Result<Box<dyn MissionEndTask<SubmissionAcceptedV1>>, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmissionAuthorizationRequest {
    /// Exact locally-authored request that preceded the server offer. Remote
    /// participants retain and compare this independently before arming any
    /// co-sign request; the server-authored offer is not authority to replace
    /// the requested mission, roster, rules, or campaign predecessor.
    pub offer_request: SubmissionOfferRequestV1,
    pub offer: SubmissionOfferV1,
    pub replay_session_transcript: ReplaySessionTranscriptV1,
    pub artifacts: SubmissionArtifactsV1,
    pub requested_metrics: Vec<BoardMetricV1>,
    pub campaign_controller_public_key: Option<PublicKey32>,
}

impl SubmissionAuthorizationRequest {
    pub fn validate_exact_context(&self) -> Result<(), MissionEndLeaderboardError> {
        self.offer_request
            .validate()
            .map_err(|error| MissionEndLeaderboardError::Authorization(error.to_string()))?;
        validate_offer_matches_request(&self.offer, &self.offer_request).map_err(|error| {
            MissionEndLeaderboardError::Authorization(format!(
                "submission offer/request context mismatch: {error}"
            ))
        })?;
        self.replay_session_transcript
            .validate()
            .map_err(|error| MissionEndLeaderboardError::Authorization(error.to_string()))?;
        self.artifacts
            .validate()
            .map_err(|error| MissionEndLeaderboardError::Authorization(error.to_string()))?;
        if self.replay_session_transcript.replay_session_id
            != self.offer_request.session_genesis.claim.replay_session_id
            || self.replay_session_transcript.replay_session_id
                != self.offer.session_genesis.claim.replay_session_id
        {
            return Err(MissionEndLeaderboardError::Authorization(
                "submission transcript belongs to a different ranked session".to_owned(),
            ));
        }
        if self.requested_metrics.is_empty()
            || !self
                .requested_metrics
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        {
            return Err(MissionEndLeaderboardError::Authorization(
                "submission metrics must be non-empty and strictly sorted".to_owned(),
            ));
        }
        let is_continuation = matches!(
            &self.offer_request.scope_request,
            robin_run_protocol::ScopeRequestV1::CampaignContinuation { .. }
        );
        if self.campaign_controller_public_key.is_some() != is_continuation
            || self
                .campaign_controller_public_key
                .is_some_and(|controller| {
                    !self
                        .offer_request
                        .participant_claims
                        .iter()
                        .any(|participant| participant.public_key == controller)
                })
        {
            return Err(MissionEndLeaderboardError::Authorization(
                "campaign continuation controller is missing or outside the authenticated roster"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    pub fn expected_participants(&self) -> Vec<PublicKey32> {
        self.offer
            .participant_claims
            .iter()
            .map(|claim| claim.public_key)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn continuation_claim(
        &self,
    ) -> Result<Option<CampaignContinuationAuthorizationClaimV1>, MissionEndLeaderboardError> {
        let InitialStateExpectationV1::CampaignContinuation {
            chain_id,
            predecessor_run_id,
            predecessor_verification_sha256,
            ..
        } = &self.offer.starting_state
        else {
            return Ok(None);
        };
        let controller = self.campaign_controller_public_key.ok_or_else(|| {
            MissionEndLeaderboardError::Authorization(
                "campaign continuation controller is unavailable".to_owned(),
            )
        })?;
        Ok(Some(CampaignContinuationAuthorizationClaimV1 {
            schema_version: SCHEMA_VERSION_V1,
            campaign_controller_public_key: controller,
            chain_id: chain_id.clone(),
            predecessor_run_id: predecessor_run_id.clone(),
            predecessor_verification_sha256: *predecessor_verification_sha256,
            next_session_genesis_sha256: self
                .offer
                .session_genesis
                .canonical_digest()
                .map_err(|error| MissionEndLeaderboardError::Authorization(error.to_string()))?,
            next_artifacts: self.artifacts.clone(),
        }))
    }

    pub fn envelope(
        &self,
        continuation: Option<CampaignContinuationAuthorizationV1>,
    ) -> SubmissionEnvelopeV1 {
        SubmissionEnvelopeV1 {
            schema_version: SCHEMA_VERSION_V1,
            offer: self.offer.clone(),
            replay_session_transcript: self.replay_session_transcript.clone(),
            artifacts: self.artifacts.clone(),
            campaign_aggregation_consent: match &self.offer.starting_state {
                InitialStateExpectationV1::IndividualLevel { .. } => {
                    CampaignAggregationConsentV1::NotAuthorized
                }
                InitialStateExpectationV1::CampaignGenesis { .. }
                | InitialStateExpectationV1::CampaignContinuation { .. } => {
                    CampaignAggregationConsentV1::AuthorizeSignedSessionInServerRecognizedChainV1
                }
            },
            campaign_continuation_authorization: continuation,
            requested_metrics: self.requested_metrics.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardLoadState {
    Loading,
    Ready(LeaderboardPageV1),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissionSubmissionState {
    WonMissionRequired,
    Unavailable(String),
    AwaitingConsent,
    PreparingArtifacts,
    AwaitingParticipantSignatures(ParticipantSigningProgress),
    Uploading,
    Queued(SubmissionAcceptedV1),
    Failed(String),
}

impl MissionSubmissionState {
    pub const fn is_busy(&self) -> bool {
        matches!(
            self,
            Self::PreparingArtifacts | Self::AwaitingParticipantSignatures(_) | Self::Uploading
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionEndLeaderboardAction {
    SelectTab(LeaderboardTab),
    RetryBoard,
    SubmitThisRun,
    RetrySubmission,
    SetAlwaysSubmitRuns(bool),
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissionEndLeaderboardEvent {
    None,
    PreferencesChanged,
    Closed,
}

#[derive(Debug, thiserror::Error)]
pub enum MissionEndLeaderboardError {
    #[error("invalid mission-end leaderboard bundle: {0}")]
    InvalidRunBundle(String),
    #[error("leaderboard backend rejected the operation: {0}")]
    Backend(String),
    #[error("ranked submission authorization failed: {0}")]
    Authorization(String),
    #[error("ranked replay export failed: {0}")]
    ReplayExport(String),
    #[error("leaderboard preference update failed: {0}")]
    Preferences(String),
}

struct PreparingSubmission {
    offer_task: Box<dyn MissionEndTask<SubmissionOfferV1>>,
    replay_task: Box<dyn MissionEndTask<Arc<[u8]>>>,
    offer: Option<SubmissionOfferV1>,
    replay: Option<Arc<[u8]>>,
}

struct AuthorizingSubmission {
    task: Box<dyn SubmissionAuthorizationTask>,
    request: SubmissionAuthorizationRequest,
    replay: Arc<[u8]>,
    presentation: MissionSubmissionState,
}

enum SubmissionTask {
    /// No host task; peer work remains owned by the peer co-signer.
    Dormant(MissionSubmissionState),
    /// Upload succeeded; this owner survives until durable receipt handoff.
    Queued {
        presentation: MissionSubmissionState,
        persisted: bool,
    },
    Preparing(PreparingSubmission),
    Authorizing(AuthorizingSubmission),
    Uploading(Box<dyn MissionEndTask<SubmissionAcceptedV1>>),
}

impl SubmissionTask {
    fn presentation(&self) -> &MissionSubmissionState {
        match self {
            Self::Dormant(state) => state,
            Self::Queued { presentation, .. } => presentation,
            Self::Preparing(_) => &MissionSubmissionState::PreparingArtifacts,
            Self::Authorizing(task) => &task.presentation,
            Self::Uploading(_) => &MissionSubmissionState::Uploading,
        }
    }

    fn is_pending(&self) -> bool {
        matches!(
            self,
            Self::Preparing(_) | Self::Authorizing(_) | Self::Uploading(_)
        )
    }

    fn set_idle(&mut self, state: MissionSubmissionState) {
        // A peer may report the same acceptance on successive polls.
        let persisted = matches!(self, Self::Queued { presentation, persisted: true } if *presentation == state);
        *self = if matches!(state, MissionSubmissionState::Queued(_)) {
            Self::Queued {
                presentation: state,
                persisted,
            }
        } else {
            Self::Dormant(state)
        };
    }

    fn mark_receipt_persisted(&mut self) {
        let Self::Queued { persisted, .. } = self else {
            panic!("receipt handoff requires a queued submission");
        };
        *persisted = true;
    }
}

enum BoardTask {
    Unrequested,
    Loading(Box<dyn MissionEndTask<LeaderboardPageV1>>),
    Complete(BoardLoadState),
}

impl BoardTask {
    fn presentation(&self) -> &BoardLoadState {
        match self {
            Self::Unrequested | Self::Loading(_) => &BoardLoadState::Loading,
            Self::Complete(state) => state,
        }
    }
}

pub struct MissionEndLeaderboardController {
    run: MissionEndRunBundle,
    preferences: LeaderboardPreferences,
    selected_tab: LeaderboardTab,
    board_task: BoardTask,
    submission_task: SubmissionTask,
    backend: Box<dyn MissionEndLeaderboardBackend>,
    authorizer: Box<dyn MissionEndSubmissionAuthorizer>,
    replay_exporter: Box<dyn MissionEndReplayExporter>,
    peer_co_signer: Option<Box<dyn MissionEndPeerCoSigner>>,
    peer_receipt_controller_public_key: Option<PublicKey32>,
    closed: bool,
}

/// Session-lifetime owner for consented mission-end work after its local
/// presentation has closed. Keeping this outside the mission UI means a level
/// transition never waits for HTTP while the exact authorization/upload task
/// continues to be polled cooperatively.
#[derive(Default)]
pub struct MissionEndLeaderboardBackground {
    active: Vec<MissionEndLeaderboardController>,
}

impl MissionEndLeaderboardBackground {
    pub fn adopt(&mut self, controller: MissionEndLeaderboardController) {
        assert!(
            controller.is_closed(),
            "only a closed mission-end controller can leave its presentation owner"
        );
        assert!(
            controller.requires_background_work(),
            "detached mission-end controller must own in-flight or unpersisted verification work"
        );
        self.active.push(controller);
    }

    /// Poll all detached tasks once without blocking the current game frame.
    pub fn poll(&mut self, application_context: &crate::host::ApplicationContext) {
        self.poll_with_receipt_sink(|handoff| {
            application_context.enqueue_leaderboard_receipt_watch(handoff)
        });
    }

    fn poll_with_receipt_sink(
        &mut self,
        mut enqueue: impl FnMut(
            crate::leaderboard_receipt_watcher::QueuedSubmissionReceiptWatch,
        ) -> Result<bool, String>,
    ) {
        let mut index = 0;
        while index < self.active.len() {
            self.active[index].poll();
            if let Err(error) = self.active[index].persist_queued_receipt_watch_with(&mut enqueue) {
                tracing::error!(
                    "queued leaderboard verification could not be handed to durable tracking: {error}"
                );
                index += 1;
                continue;
            }
            if self.active[index].can_retire_after_close() {
                let controller = self.active.swap_remove(index);
                match controller.submission_state() {
                    MissionSubmissionState::Queued(accepted) => tracing::info!(
                        submission_id = %accepted.submission_id,
                        "background leaderboard submission was accepted"
                    ),
                    MissionSubmissionState::Failed(error) => {
                        tracing::warn!("background leaderboard submission failed: {error}")
                    }
                    other => tracing::warn!(
                        ?other,
                        "background leaderboard controller retired in an unexpected state"
                    ),
                }
            } else {
                index += 1;
            }
        }
    }

    pub fn active_count(&self) -> usize {
        self.active.len()
    }
}

impl MissionEndLeaderboardController {
    pub fn new(
        run: MissionEndRunBundle,
        preferences: LeaderboardPreferences,
        backend: Box<dyn MissionEndLeaderboardBackend>,
        authorizer: Box<dyn MissionEndSubmissionAuthorizer>,
        replay_exporter: Box<dyn MissionEndReplayExporter>,
    ) -> Result<Self, MissionEndLeaderboardError> {
        run.validate()?;
        let preferences = preferences
            .validate()
            .map_err(|error| MissionEndLeaderboardError::Preferences(error.to_string()))?;
        let selected_tab = run
            .boards
            .iter()
            .find(|board| board.tab == preferences.preferred_tab)
            .or_else(|| run.boards.first())
            .expect("run validation requires one board")
            .tab;
        let submission_state = if !run.outcome.can_submit() {
            MissionSubmissionState::WonMissionRequired
        } else if run.eligible_submission.is_some() {
            MissionSubmissionState::AwaitingConsent
        } else {
            MissionSubmissionState::Unavailable(
                run.submission_unavailable_reason
                    .clone()
                    .unwrap_or_else(|| "this run was not recorded as rank-eligible".to_owned()),
            )
        };
        let mut controller = Self {
            run,
            preferences,
            selected_tab,
            board_task: BoardTask::Unrequested,
            submission_task: SubmissionTask::Dormant(submission_state),
            backend,
            authorizer,
            replay_exporter,
            peer_co_signer: None,
            peer_receipt_controller_public_key: None,
            closed: false,
        };
        if controller.preferences.show_mission_end_boards {
            controller.start_selected_board();
        }
        if controller
            .preferences
            .automatically_submit(controller.run.outcome.can_submit())
            && controller.run.eligible_submission.is_some()
        {
            controller.start_submission();
        }
        Ok(controller)
    }

    /// Construct a non-host multiplayer panel. It browses the same boards but
    /// its submit control grants only this participant's co-sign consent; it
    /// never creates a competing offer or uploads a second replay.
    pub fn new_peer(
        run: MissionEndRunBundle,
        preferences: LeaderboardPreferences,
        backend: Box<dyn MissionEndLeaderboardBackend>,
        peer_co_signer: Box<dyn MissionEndPeerCoSigner>,
        receipt_controller_public_key: Option<PublicKey32>,
        replay_exports: crate::replay_service::ReplayExports,
    ) -> Result<Self, MissionEndLeaderboardError> {
        let mut controller = Self::new(
            run,
            preferences,
            backend,
            Box::new(LocalMissionEndSubmissionAuthorizer),
            Box::new(ActiveMissionReplayExporter::new(replay_exports)),
        )?;
        controller.peer_co_signer = Some(peer_co_signer);
        controller.peer_receipt_controller_public_key = receipt_controller_public_key;
        controller.submission_task =
            SubmissionTask::Dormant(if controller.run.outcome.can_submit() {
                MissionSubmissionState::AwaitingConsent
            } else {
                MissionSubmissionState::WonMissionRequired
            });
        if controller
            .preferences
            .automatically_submit(controller.run.outcome.can_submit())
        {
            controller.start_submission();
        }
        Ok(controller)
    }

    pub fn is_visible(&self) -> bool {
        self.preferences.show_mission_end_boards && !self.closed
    }

    pub fn is_multiplayer(&self) -> bool {
        self.run.multiplayer
    }

    pub fn outcome(&self) -> MissionEndOutcome {
        self.run.outcome
    }

    pub fn boards(&self) -> &[MissionEndBoard] {
        &self.run.boards
    }

    pub fn selected_tab(&self) -> LeaderboardTab {
        self.selected_tab
    }

    pub fn board_state(&self) -> &BoardLoadState {
        self.board_task.presentation()
    }

    pub fn submission_state(&self) -> &MissionSubmissionState {
        self.submission_task.presentation()
    }

    pub fn preferences(&self) -> &LeaderboardPreferences {
        &self.preferences
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Whether a consented run still has preparation, co-signing, or upload
    /// work in flight. A presentation owner may dismiss the overlay and keep
    /// polling this controller in a background UI-task variant until false.
    pub fn has_pending_submission(&self) -> bool {
        self.submission_task.is_pending()
            || self
                .peer_co_signer
                .as_ref()
                .is_some_and(|peer| peer.has_pending_work())
    }

    pub fn can_retire_after_close(&self) -> bool {
        self.closed && !self.has_pending_submission() && !self.has_unpersisted_receipt_watch()
    }

    pub fn requires_background_work(&self) -> bool {
        self.has_pending_submission() || self.has_unpersisted_receipt_watch()
    }

    /// Build and durably enqueue the exact authenticated owner-status key once
    /// `POST /submissions` has accepted the canonical replay into verification.
    /// This method is idempotent and is called both while the panel remains
    /// open and by the detached background owner.
    pub fn persist_queued_receipt_watch(
        &mut self,
        application_context: &crate::host::ApplicationContext,
    ) -> Result<bool, String> {
        self.persist_queued_receipt_watch_with(|handoff| {
            application_context.enqueue_leaderboard_receipt_watch(handoff)
        })
    }

    fn persist_queued_receipt_watch_with(
        &mut self,
        enqueue: impl FnOnce(
            crate::leaderboard_receipt_watcher::QueuedSubmissionReceiptWatch,
        ) -> Result<bool, String>,
    ) -> Result<bool, String> {
        if matches!(
            self.submission_task,
            SubmissionTask::Queued {
                persisted: true,
                ..
            }
        ) {
            return Ok(false);
        }
        let MissionSubmissionState::Queued(accepted) = self.submission_state() else {
            return Ok(false);
        };
        let controller_public_key = if self.peer_co_signer.is_some() {
            let Some(controller) = self.peer_receipt_controller_public_key else {
                self.submission_task.mark_receipt_persisted();
                return Ok(false);
            };
            controller
        } else {
            if !self.authorizer.owns_receipt_watch()? {
                self.submission_task.mark_receipt_persisted();
                return Ok(false);
            }
            let input = self.run.eligible_submission.as_ref().ok_or_else(|| {
                "queued leaderboard submission lost its exact ranked input".to_owned()
            })?;
            match &input.offer_request.scope_request {
                robin_run_protocol::ScopeRequestV1::CampaignContinuation { .. } => {
                    input.campaign_controller_public_key.ok_or_else(|| {
                        "queued campaign continuation lost its authenticated controller".to_owned()
                    })?
                }
                robin_run_protocol::ScopeRequestV1::IndividualLevel
                | robin_run_protocol::ScopeRequestV1::CampaignGenesis => {
                    input.offer_request.session_genesis.claim.host_public_key
                }
            }
        };
        let handoff =
            crate::leaderboard_receipt_watcher::QueuedSubmissionReceiptWatch::from_accepted(
                accepted,
                controller_public_key,
            )
            .map_err(|error| error.to_string())?;
        enqueue(handoff)?;
        self.submission_task.mark_receipt_persisted();
        Ok(true)
    }

    fn has_unpersisted_receipt_watch(&self) -> bool {
        matches!(
            self.submission_task,
            SubmissionTask::Queued {
                persisted: false,
                ..
            }
        )
    }

    /// Advance each active task by one non-blocking poll.
    pub fn poll(&mut self) {
        self.poll_board();
        self.poll_submission();
    }

    pub fn apply_action(
        &mut self,
        action: MissionEndLeaderboardAction,
    ) -> Result<MissionEndLeaderboardEvent, MissionEndLeaderboardError> {
        match action {
            MissionEndLeaderboardAction::SelectTab(tab) => {
                if !self.run.boards.iter().any(|board| board.tab == tab) {
                    return Err(MissionEndLeaderboardError::InvalidRunBundle(
                        "selected board tab is unavailable for this run".to_owned(),
                    ));
                }
                let previous_tab = self.selected_tab;
                let previous_preferences = self.preferences.clone();
                self.selected_tab = tab;
                self.preferences.preferred_tab = tab;
                if let Err(error) = self.persist_preferences() {
                    self.selected_tab = previous_tab;
                    self.preferences = previous_preferences;
                    return Err(error);
                }
                self.start_selected_board();
                Ok(MissionEndLeaderboardEvent::PreferencesChanged)
            }
            MissionEndLeaderboardAction::RetryBoard => {
                self.start_selected_board();
                Ok(MissionEndLeaderboardEvent::None)
            }
            MissionEndLeaderboardAction::SubmitThisRun => {
                if !matches!(
                    self.submission_state(),
                    MissionSubmissionState::AwaitingConsent
                ) {
                    return Err(MissionEndLeaderboardError::Authorization(
                        "this run is not awaiting upload consent".to_owned(),
                    ));
                }
                self.start_submission();
                Ok(MissionEndLeaderboardEvent::None)
            }
            MissionEndLeaderboardAction::RetrySubmission => {
                if !matches!(self.submission_state(), MissionSubmissionState::Failed(_)) {
                    return Err(MissionEndLeaderboardError::Authorization(
                        "only a failed eligible submission can be retried".to_owned(),
                    ));
                }
                self.start_submission();
                Ok(MissionEndLeaderboardEvent::None)
            }
            MissionEndLeaderboardAction::SetAlwaysSubmitRuns(enabled) => {
                let previous_preferences = self.preferences.clone();
                self.preferences.always_submit_eligible_runs = enabled;
                if let Err(error) = self.persist_preferences() {
                    self.preferences = previous_preferences;
                    return Err(error);
                }
                if enabled
                    && matches!(
                        self.submission_state(),
                        MissionSubmissionState::AwaitingConsent
                    )
                {
                    self.start_submission();
                }
                Ok(MissionEndLeaderboardEvent::PreferencesChanged)
            }
            MissionEndLeaderboardAction::Close => {
                self.closed = true;
                Ok(MissionEndLeaderboardEvent::Closed)
            }
        }
    }

    fn persist_preferences(&self) -> Result<(), MissionEndLeaderboardError> {
        crate::leaderboard_preferences::persist(&self.preferences)
            .map_err(|error| MissionEndLeaderboardError::Preferences(error.to_string()))
    }

    fn start_selected_board(&mut self) {
        let query = self
            .run
            .boards
            .iter()
            .find(|board| board.tab == self.selected_tab)
            .expect("selected tab is kept inside the validated board set")
            .query
            .clone();
        match self.backend.board(query) {
            Ok(task) => {
                self.board_task = BoardTask::Loading(task);
            }
            Err(error) => {
                self.board_task = BoardTask::Complete(BoardLoadState::Failed(error));
            }
        }
    }

    fn poll_board(&mut self) {
        let BoardTask::Loading(task) = &mut self.board_task else {
            return;
        };
        let Some(result) = task.try_take() else {
            return;
        };
        self.board_task = BoardTask::Complete(match result {
            Ok(page) => BoardLoadState::Ready(page),
            Err(error) => BoardLoadState::Failed(error),
        });
    }

    fn start_submission(&mut self) {
        if let Some(peer) = self.peer_co_signer.as_mut() {
            if !self.run.outcome.can_submit() {
                self.submission_task =
                    SubmissionTask::Dormant(MissionSubmissionState::WonMissionRequired);
                return;
            }
            match peer.consent() {
                Ok(()) => {
                    self.submission_task =
                        SubmissionTask::Dormant(MissionSubmissionState::PreparingArtifacts);
                }
                Err(error) => self.fail_submission(error),
            }
            return;
        }
        let Some(input) = self.run.eligible_submission.as_ref() else {
            self.submission_task = SubmissionTask::Dormant(MissionSubmissionState::Unavailable(
                self.run
                    .submission_unavailable_reason
                    .clone()
                    .unwrap_or_else(|| "this run is not rank-eligible".to_owned()),
            ));
            return;
        };
        if !self.run.outcome.can_submit() {
            self.submission_task =
                SubmissionTask::Dormant(MissionSubmissionState::WonMissionRequired);
            return;
        }
        let offer_task = match self.backend.offer(input.offer_request.clone()) {
            Ok(task) => task,
            Err(error) => {
                self.fail_submission(error);
                return;
            }
        };
        let replay_task = match self.replay_exporter.begin() {
            Ok(task) => task,
            Err(error) => {
                self.fail_submission(error);
                return;
            }
        };
        self.submission_task = SubmissionTask::Preparing(PreparingSubmission {
            offer_task,
            replay_task,
            offer: None,
            replay: None,
        });
    }

    fn poll_submission(&mut self) {
        if let Some(peer) = self.peer_co_signer.as_mut() {
            self.submission_task.set_idle(match peer.poll() {
                PeerCoSignPoll::AwaitingConsent => MissionSubmissionState::AwaitingConsent,
                PeerCoSignPoll::AwaitingHost => MissionSubmissionState::PreparingArtifacts,
                PeerCoSignPoll::Signing(progress) => {
                    MissionSubmissionState::AwaitingParticipantSignatures(progress)
                }
                PeerCoSignPoll::ResponseSent(progress) => {
                    let _ = progress;
                    MissionSubmissionState::Uploading
                }
                PeerCoSignPoll::Accepted(accepted) => MissionSubmissionState::Queued(accepted),
                PeerCoSignPoll::Failed(error) => MissionSubmissionState::Failed(error),
            });
            return;
        }
        if !self.submission_task.is_pending() {
            return;
        }
        let mut task = std::mem::replace(
            &mut self.submission_task,
            SubmissionTask::Dormant(MissionSubmissionState::AwaitingConsent),
        );
        match &mut task {
            SubmissionTask::Dormant(_) | SubmissionTask::Queued { .. } => {
                unreachable!("pending task checked before polling")
            }
            SubmissionTask::Preparing(preparing) => {
                if preparing.offer.is_none()
                    && let Some(result) = preparing.offer_task.try_take()
                {
                    match result {
                        Ok(offer) => preparing.offer = Some(offer),
                        Err(error) => {
                            self.fail_submission(error);
                            return;
                        }
                    }
                }
                if preparing.replay.is_none()
                    && let Some(result) = preparing.replay_task.try_take()
                {
                    match result {
                        Ok(replay) => preparing.replay = Some(replay),
                        Err(error) => {
                            self.fail_submission(error);
                            return;
                        }
                    }
                }
                if preparing.offer.is_some() && preparing.replay.is_some() {
                    let offer = preparing.offer.take().expect("offer checked above");
                    let replay = preparing.replay.take().expect("replay checked above");
                    if let Err(error) = self.begin_authorization(offer, replay) {
                        self.fail_submission(error.to_string());
                    }
                } else {
                    self.submission_task = task;
                }
            }
            SubmissionTask::Authorizing(authorizing) => {
                let progress = authorizing.task.progress();
                if let Err(error) = progress.validate() {
                    self.fail_submission(error.to_string());
                    return;
                }
                authorizing.presentation =
                    MissionSubmissionState::AwaitingParticipantSignatures(progress);
                let Some(result) = authorizing.task.try_take() else {
                    self.submission_task = task;
                    return;
                };
                let signed = match result {
                    Ok(signed) => signed,
                    Err(error) => {
                        self.fail_submission(error);
                        return;
                    }
                };
                if let Err(error) = validate_authorized_submission(&authorizing.request, &signed) {
                    self.fail_submission(error.to_string());
                    return;
                }
                let input = self
                    .run
                    .eligible_submission
                    .as_ref()
                    .expect("authorization exists only for eligible input");
                match self.backend.submit(
                    signed,
                    Arc::clone(&authorizing.replay),
                    Arc::clone(&input.starting_campaign_bytes),
                ) {
                    Ok(upload) => {
                        self.submission_task = SubmissionTask::Uploading(upload);
                    }
                    Err(error) => self.fail_submission(error),
                }
            }
            SubmissionTask::Uploading(upload) => match upload.try_take() {
                None => self.submission_task = task,
                Some(Ok(accepted)) => {
                    if let Err(error) = self.authorizer.submission_accepted(&accepted) {
                        tracing::warn!(
                            "ranked submission was queued but its peer acknowledgement failed: {error}"
                        );
                    }
                    self.submission_task
                        .set_idle(MissionSubmissionState::Queued(accepted));
                }
                Some(Err(error)) => self.fail_submission(error),
            },
        }
    }

    fn begin_authorization(
        &mut self,
        offer: SubmissionOfferV1,
        replay: Arc<[u8]>,
    ) -> Result<(), MissionEndLeaderboardError> {
        let input = self
            .run
            .eligible_submission
            .as_ref()
            .expect("submission preparation exists only for eligible input");
        validate_offer_matches_request(&offer, &input.offer_request)?;
        let replay_artifact = canonical_replay_artifact(
            &replay,
            &input.starting_campaign_bytes,
            &offer.mission_id,
            &input.replay_session_transcript,
        )?;
        let campaign_artifact = ArtifactRefV1 {
            sha256: Digest32::digest_bytes(&input.starting_campaign_bytes),
            byte_length: u64::try_from(input.starting_campaign_bytes.len()).map_err(|_| {
                MissionEndLeaderboardError::ReplayExport(
                    "starting campaign length does not fit the protocol".to_owned(),
                )
            })?,
            media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
        };
        let request = SubmissionAuthorizationRequest {
            offer_request: input.offer_request.clone(),
            offer,
            replay_session_transcript: input.replay_session_transcript.clone(),
            artifacts: SubmissionArtifactsV1 {
                replay: replay_artifact,
                starting_campaign: campaign_artifact,
            },
            requested_metrics: input.requested_metrics.clone(),
            campaign_controller_public_key: input.campaign_controller_public_key,
        };
        let task = self
            .authorizer
            .begin(request.clone())
            .map_err(MissionEndLeaderboardError::Authorization)?;
        let progress = task.progress();
        progress.validate()?;
        if progress.expected != request.expected_participants() {
            return Err(MissionEndLeaderboardError::Authorization(
                "authorizer expected a different participant set".to_owned(),
            ));
        }
        self.submission_task = SubmissionTask::Authorizing(AuthorizingSubmission {
            task,
            request,
            replay,
            presentation: MissionSubmissionState::AwaitingParticipantSignatures(progress),
        });
        Ok(())
    }

    fn fail_submission(&mut self, error: String) {
        self.submission_task = SubmissionTask::Dormant(MissionSubmissionState::Failed(error));
    }
}

fn invalid_bundle_protocol(error: impl std::fmt::Display) -> MissionEndLeaderboardError {
    MissionEndLeaderboardError::InvalidRunBundle(error.to_string())
}

fn validate_offer_matches_request(
    offer: &SubmissionOfferV1,
    request: &SubmissionOfferRequestV1,
) -> Result<(), MissionEndLeaderboardError> {
    offer
        .validate()
        .map_err(|error| MissionEndLeaderboardError::Backend(error.to_string()))?;
    robin_run_protocol::validate_offer_binding(request, offer)
        .map_err(|error| MissionEndLeaderboardError::Backend(error.to_string()))
}

pub(crate) fn canonical_replay_artifact(
    bytes: &[u8],
    expected_starting_campaign: &[u8],
    expected_mission_id: &str,
    transcript: &ReplaySessionTranscriptV1,
) -> Result<ReplayArtifactV1, MissionEndLeaderboardError> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        MissionEndLeaderboardError::ReplayExport("compact replay is not UTF-8".to_owned())
    })?;
    let limits = robin_replay_format::ReplayAdmissionLimits {
        max_input_bytes: bytes.len(),
        ..Default::default()
    };
    let (engine_hash, replay) = robin_replay_format::decode_compact_bounded(text, &limits)
        .map_err(|error| MissionEndLeaderboardError::ReplayExport(error.to_string()))?;
    robin_replay_format::validate_engine_hash(&engine_hash)
        .map_err(|error| MissionEndLeaderboardError::ReplayExport(error.to_string()))?;
    let canonical = robin_replay_format::encode_compact(&replay, &engine_hash)
        .map_err(|error| MissionEndLeaderboardError::ReplayExport(error.to_string()))?;
    if canonical.as_bytes() != bytes {
        return Err(MissionEndLeaderboardError::ReplayExport(
            "replay bytes are not their canonical compact re-encoding".to_owned(),
        ));
    }
    if replay.header().campaign.as_slice() != expected_starting_campaign
        || replay.header().mission_id != expected_mission_id
    {
        return Err(MissionEndLeaderboardError::ReplayExport(
            "compact replay does not contain the exact mission start selected for upload"
                .to_owned(),
        ));
    }
    replay.ranked_submission_verdict().map_err(|reason| {
        MissionEndLeaderboardError::ReplayExport(format!(
            "compact replay is not eligible for ranked submission: {}",
            reason.stable_code()
        ))
    })?;
    replay
        .validate_ranked_command_admission(transcript)
        .map_err(MissionEndLeaderboardError::ReplayExport)?;
    Ok(ReplayArtifactV1 {
        artifact: ArtifactRefV1 {
            sha256: Digest32::digest_bytes(bytes),
            byte_length: u64::try_from(bytes.len()).map_err(|_| {
                MissionEndLeaderboardError::ReplayExport(
                    "compact replay length does not fit the protocol".to_owned(),
                )
            })?,
            media_type: RANKED_REPLAY_MEDIA_TYPE_V1.to_owned(),
        },
        replay_schema_version: robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
    })
}

pub fn validate_authorized_submission(
    request: &SubmissionAuthorizationRequest,
    signed: &SignedSubmissionV1,
) -> Result<(), MissionEndLeaderboardError> {
    request.validate_exact_context()?;
    signed
        .validate()
        .map_err(|error| MissionEndLeaderboardError::Authorization(error.to_string()))?;
    if signed.algorithm != SignatureAlgorithmV1::Ed25519
        || signed.submission.offer != request.offer
        || signed.submission.replay_session_transcript != request.replay_session_transcript
        || signed.submission.artifacts != request.artifacts
        || signed.submission.requested_metrics != request.requested_metrics
    {
        return Err(MissionEndLeaderboardError::Authorization(
            "authorizer changed the exact submission claim".to_owned(),
        ));
    }
    let expected_claim = request.continuation_claim()?;
    match (
        expected_claim,
        &signed.submission.campaign_continuation_authorization,
    ) {
        (None, None) => {}
        (Some(expected), Some(actual)) if actual.claim == expected => {
            verify_ed25519(
                actual.claim.campaign_controller_public_key,
                actual.signature.as_bytes(),
                &actual
                    .signing_bytes(&signed.submission.offer)
                    .map_err(|error| {
                        MissionEndLeaderboardError::Authorization(error.to_string())
                    })?,
            )?;
        }
        _ => {
            return Err(MissionEndLeaderboardError::Authorization(
                "campaign continuation authorization was omitted or substituted".to_owned(),
            ));
        }
    }
    let signing_bytes = signed
        .signing_bytes()
        .map_err(|error| MissionEndLeaderboardError::Authorization(error.to_string()))?;
    for signature in &signed.participant_signatures {
        verify_ed25519(
            signature.public_key,
            signature.signature.as_bytes(),
            &signing_bytes,
        )?;
    }
    Ok(())
}

fn verify_ed25519(
    public_key: PublicKey32,
    signature: &[u8; 64],
    message: &[u8],
) -> Result<(), MissionEndLeaderboardError> {
    robin_run_protocol::verify_ed25519_strict(public_key.as_bytes(), signature, message)
        .map_err(|error| MissionEndLeaderboardError::Authorization(error.to_string()))
}

/// Production backend adapter over the bounded native/browser HTTP client.
pub struct HttpMissionEndLeaderboardBackend {
    api: LeaderboardApi,
}

impl HttpMissionEndLeaderboardBackend {
    pub fn new(api: LeaderboardApi) -> Self {
        Self { api }
    }
}

struct BoardHttpTask {
    task: HttpTask,
    query: LeaderboardQueryV1,
}

impl MissionEndTask<LeaderboardPageV1> for BoardHttpTask {
    fn try_take(&mut self) -> Option<Result<LeaderboardPageV1, String>> {
        self.task
            .try_take()
            .map(|result| decode_board(result, &self.query).map_err(|error| error.to_string()))
    }
}

struct OfferHttpTask(HttpTask);

impl MissionEndTask<SubmissionOfferV1> for OfferHttpTask {
    fn try_take(&mut self) -> Option<Result<SubmissionOfferV1, String>> {
        self.0
            .try_take()
            .map(|result| decode_offer(result).map_err(|error| error.to_string()))
    }
}

struct UploadHttpTask(HttpTask);

impl MissionEndTask<SubmissionAcceptedV1> for UploadHttpTask {
    fn try_take(&mut self) -> Option<Result<SubmissionAcceptedV1, String>> {
        self.0
            .try_take()
            .map(|result| decode_submission_accepted(result).map_err(|error| error.to_string()))
    }
}

impl MissionEndLeaderboardBackend for HttpMissionEndLeaderboardBackend {
    fn board(
        &mut self,
        query: LeaderboardQueryV1,
    ) -> Result<Box<dyn MissionEndTask<LeaderboardPageV1>>, String> {
        let task = self.api.board(&query).map_err(|error| error.to_string())?;
        Ok(Box::new(BoardHttpTask { task, query }))
    }

    fn offer(
        &mut self,
        request: SubmissionOfferRequestV1,
    ) -> Result<Box<dyn MissionEndTask<SubmissionOfferV1>>, String> {
        Ok(Box::new(OfferHttpTask(
            self.api
                .submission_offer(&request)
                .map_err(|error| error.to_string())?,
        )))
    }

    fn submit(
        &mut self,
        submission: SignedSubmissionV1,
        replay_bytes: Arc<[u8]>,
        starting_campaign_bytes: Arc<[u8]>,
    ) -> Result<Box<dyn MissionEndTask<SubmissionAcceptedV1>>, String> {
        Ok(Box::new(UploadHttpTask(
            self.api
                .submit(&submission, replay_bytes, starting_campaign_bytes)
                .map_err(|error| error.to_string())?,
        )))
    }
}

/// Active bounded recorder export. Snapshotting shares complete spool chunks;
/// parsing and compact-bitcode encoding run off the graphical call stack.
#[derive(Serialize, Deserialize)]
pub struct ActiveMissionReplayExporter {
    exports: crate::replay_service::ReplayExports,
}

impl ActiveMissionReplayExporter {
    pub fn new(exports: crate::replay_service::ReplayExports) -> Self {
        Self { exports }
    }
}

struct ReplayExportTask(crate::replay_service::ExportResult);

impl Serialize for ReplayExportTask {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str("live replay export task")
    }
}

impl<'de> Deserialize<'de> for ReplayExportTask {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "replay export tasks must be constructed by their consumer",
        ))
    }
}

impl MissionEndTask<Arc<[u8]>> for ReplayExportTask {
    fn try_take(&mut self) -> Option<Result<Arc<[u8]>, String>> {
        match self.0.try_recv() {
            Ok(result) => Some(
                result
                    .map(|compact| Arc::<[u8]>::from(compact.into_bytes()))
                    .map_err(|error| error.to_string()),
            ),
            Err(async_channel::TryRecvError::Empty) => None,
            Err(async_channel::TryRecvError::Closed) => Some(Err(
                "ranked replay export task stopped unexpectedly".to_owned(),
            )),
        }
    }
}

impl MissionEndReplayExporter for ActiveMissionReplayExporter {
    fn begin(&mut self) -> Result<Box<dyn MissionEndTask<Arc<[u8]>>>, String> {
        let snapshot = self.exports.snapshot()?;
        Ok(Box::new(ReplayExportTask(
            self.exports.export_snapshot(snapshot),
        )))
    }
}

/// Single-player production authorizer. Multiplayer uses the same exact
/// request type through an authenticated co-sign transport adapter.
pub struct LocalMissionEndSubmissionAuthorizer;

#[cfg(any(test, not(target_arch = "wasm32")))]
struct ImmediateAuthorizationTask {
    result: Option<Result<SignedSubmissionV1, String>>,
    progress: ParticipantSigningProgress,
}

#[cfg(any(test, not(target_arch = "wasm32")))]
impl MissionEndTask<SignedSubmissionV1> for ImmediateAuthorizationTask {
    fn try_take(&mut self) -> Option<Result<SignedSubmissionV1, String>> {
        self.result.take()
    }
}

#[cfg(any(test, not(target_arch = "wasm32")))]
impl SubmissionAuthorizationTask for ImmediateAuthorizationTask {
    fn progress(&self) -> ParticipantSigningProgress {
        self.progress.clone()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl MissionEndSubmissionAuthorizer for LocalMissionEndSubmissionAuthorizer {
    fn begin(
        &mut self,
        request: SubmissionAuthorizationRequest,
    ) -> Result<Box<dyn SubmissionAuthorizationTask>, String> {
        let expected = request.expected_participants();
        if expected.len() != 1 {
            return Err(
                "multiplayer submission requires the authenticated co-sign transport".to_owned(),
            );
        }
        let result = authorize_local_native(&request).map_err(|error| error.to_string());
        let signed = result
            .as_ref()
            .ok()
            .map(|signed| {
                signed
                    .participant_signatures
                    .iter()
                    .map(|signature| signature.public_key)
                    .collect()
            })
            .unwrap_or_default();
        Ok(Box::new(ImmediateAuthorizationTask {
            result: Some(result),
            progress: ParticipantSigningProgress { expected, signed },
        }))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn authorize_local_native(
    request: &SubmissionAuthorizationRequest,
) -> Result<SignedSubmissionV1, MissionEndLeaderboardError> {
    let continuation = request
        .continuation_claim()?
        .map(|claim| crate::leaderboard_signing::sign_campaign_continuation(&request.offer, claim))
        .transpose()
        .map_err(|error| MissionEndLeaderboardError::Authorization(error.to_string()))?;
    let envelope = request.envelope(continuation);
    envelope
        .validate()
        .map_err(|error| MissionEndLeaderboardError::Authorization(error.to_string()))?;
    let signature = crate::leaderboard_signing::sign_submission_claim(&envelope)
        .map_err(|error| MissionEndLeaderboardError::Authorization(error.to_string()))?;
    let signed = SignedSubmissionV1 {
        schema_version: SCHEMA_VERSION_V1,
        submission: envelope,
        algorithm: SignatureAlgorithmV1::Ed25519,
        participant_signatures: vec![signature],
    };
    validate_authorized_submission(request, &signed)?;
    Ok(signed)
}

#[cfg(target_arch = "wasm32")]
struct BrowserAuthorizationTask {
    receiver: async_channel::Receiver<Result<SignedSubmissionV1, String>>,
    progress: ParticipantSigningProgress,
}

#[cfg(target_arch = "wasm32")]
impl MissionEndTask<SignedSubmissionV1> for BrowserAuthorizationTask {
    fn try_take(&mut self) -> Option<Result<SignedSubmissionV1, String>> {
        match self.receiver.try_recv() {
            Ok(result) => {
                if let Ok(signed) = &result {
                    self.progress.signed = signed
                        .participant_signatures
                        .iter()
                        .map(|signature| signature.public_key)
                        .collect();
                }
                Some(result)
            }
            Err(async_channel::TryRecvError::Empty) => None,
            Err(async_channel::TryRecvError::Closed) => Some(Err(
                "browser submission signer stopped unexpectedly".to_owned(),
            )),
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl SubmissionAuthorizationTask for BrowserAuthorizationTask {
    fn progress(&self) -> ParticipantSigningProgress {
        self.progress.clone()
    }
}

#[cfg(target_arch = "wasm32")]
impl MissionEndSubmissionAuthorizer for LocalMissionEndSubmissionAuthorizer {
    fn begin(
        &mut self,
        request: SubmissionAuthorizationRequest,
    ) -> Result<Box<dyn SubmissionAuthorizationTask>, String> {
        let expected = request.expected_participants();
        if expected.len() != 1 {
            return Err(
                "multiplayer submission requires the authenticated co-sign transport".to_owned(),
            );
        }
        let (sender, receiver) = async_channel::bounded(1);
        wasm_bindgen_futures::spawn_local(async move {
            let result = authorize_local_browser(&request)
                .await
                .map_err(|error| error.to_string());
            let _ = sender.send(result).await;
        });
        Ok(Box::new(BrowserAuthorizationTask {
            receiver,
            progress: ParticipantSigningProgress {
                expected,
                signed: Vec::new(),
            },
        }))
    }
}

#[cfg(target_arch = "wasm32")]
async fn authorize_local_browser(
    request: &SubmissionAuthorizationRequest,
) -> Result<SignedSubmissionV1, MissionEndLeaderboardError> {
    let continuation = match request.continuation_claim()? {
        Some(claim) => Some(
            crate::leaderboard_signing::browser_game_sign_campaign_continuation(
                &request.offer,
                &claim,
            )
            .await
            .map_err(|error| MissionEndLeaderboardError::Authorization(error.to_string()))?,
        ),
        None => None,
    };
    let envelope = request.envelope(continuation);
    envelope
        .validate()
        .map_err(|error| MissionEndLeaderboardError::Authorization(error.to_string()))?;
    let signature = crate::leaderboard_signing::browser_game_sign_submission_claim(&envelope)
        .await
        .map_err(|error| MissionEndLeaderboardError::Authorization(error.to_string()))?;
    let signed = SignedSubmissionV1 {
        schema_version: SCHEMA_VERSION_V1,
        submission: envelope,
        algorithm: SignatureAlgorithmV1::Ed25519,
        participant_signatures: vec![signature],
    };
    validate_authorized_submission(request, &signed)?;
    Ok(signed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Signer as _;
    use robin_run_protocol::{
        BoardCategoryV1, CanonicalCampaignStateKindV1, CanonicalCampaignStateRequirementV1,
        ChallengeNonce32, LeaderboardQuerySubjectV1, OfficialContentEditionV1,
        OfficialContentSubjectV1, OpaqueId, ParticipantClaimV1, ParticipantPublicDisclosureV1,
        ParticipantSignatureV1, RankedSessionConfigV1, ReplaySeatLifecycleEventV1,
        ReplaySeatLifecycleKindV1, ReplaySessionGenesisClaimV1, ReplaySessionGenesisV1,
        ResourceLocaleRootV1, ScopeRequestV1, Signature64, SimulationSeed64,
        SpeechTimingAuthorityV1, SubmissionLifecycleV1,
    };
    use std::collections::BTreeMap;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    #[test]
    fn replay_exporter_diagnostics_cannot_restore_export_authority() {
        let service = Arc::new(crate::replay_service::ReplayService::default());
        let exporter = ActiveMissionReplayExporter::new(service.exports());
        let diagnostic = serde_json::to_string(&exporter).unwrap();
        assert!(serde_json::from_str::<ActiveMissionReplayExporter>(&diagnostic).is_err());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn replay_exporter_uses_injected_service_and_freezes_its_generation() {
        let service = Arc::new(crate::replay_service::ReplayService::default());
        let other = Arc::new(crate::replay_service::ReplayService::default());
        let mut recorder = robin_engine::replay::ReplayRecorder::with_writer(
            Box::new(service.recording().begin_recording()),
            "injected-export".to_owned(),
            robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                "injected-export",
                "export-map",
                "export-map",
            )
            .unwrap(),
            17,
            robin_engine::engine::SimConfig::default(),
            &robin_engine::campaign::Campaign::default(),
        )
        .unwrap();
        assert!(recorder.write_frame(
            0,
            0,
            1,
            robin_engine::engine::SimulationFrameInput::default(),
            Vec::new(),
            None,
        ));
        let mut exporter = ActiveMissionReplayExporter::new(service.exports());
        let task = exporter.begin().unwrap();
        let empty_task = ActiveMissionReplayExporter::new(other.exports())
            .begin()
            .unwrap();
        // Replacing the active generation must not replace the admitted export.
        let _replacement = service.recording().begin_recording();
        let finish = |mut task: Box<dyn MissionEndTask<Arc<[u8]>>>| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                if let Some(result) = task.try_take() {
                    break result;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "export worker timed out"
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        };
        // Snapshot admission is synchronous; an empty service's missing replay
        // header is rejected by the asynchronous parsing/encoding worker.
        assert!(finish(empty_task).is_err());
        let bytes = finish(task).unwrap();
        let (_, replay) =
            robin_replay_format::decode_compact(std::str::from_utf8(&bytes).unwrap()).unwrap();
        assert_eq!(replay.header().mission_id, "injected-export");
        drop(recorder);
    }

    struct ImmediateTask<T>(Option<Result<T, String>>);

    impl<T> MissionEndTask<T> for ImmediateTask<T> {
        fn try_take(&mut self) -> Option<Result<T, String>> {
            self.0.take()
        }
    }

    struct DelayedTask<T> {
        remaining_polls: usize,
        result: Option<Result<T, String>>,
    }

    impl<T> MissionEndTask<T> for DelayedTask<T> {
        fn try_take(&mut self) -> Option<Result<T, String>> {
            if self.remaining_polls > 0 {
                self.remaining_polls -= 1;
                None
            } else {
                self.result.take()
            }
        }
    }

    #[derive(Default)]
    struct BackendCalls {
        boards: usize,
        offers: usize,
        uploads: usize,
    }

    struct TestBackend {
        page: LeaderboardPageV1,
        offer: SubmissionOfferV1,
        calls: Arc<Mutex<BackendCalls>>,
        upload_delay_polls: usize,
    }

    impl MissionEndLeaderboardBackend for TestBackend {
        fn board(
            &mut self,
            _query: LeaderboardQueryV1,
        ) -> Result<Box<dyn MissionEndTask<LeaderboardPageV1>>, String> {
            self.calls.lock().unwrap().boards += 1;
            Ok(Box::new(ImmediateTask(Some(Ok(self.page.clone())))))
        }

        fn offer(
            &mut self,
            _request: SubmissionOfferRequestV1,
        ) -> Result<Box<dyn MissionEndTask<SubmissionOfferV1>>, String> {
            self.calls.lock().unwrap().offers += 1;
            Ok(Box::new(ImmediateTask(Some(Ok(self.offer.clone())))))
        }

        fn submit(
            &mut self,
            submission: SignedSubmissionV1,
            replay_bytes: Arc<[u8]>,
            starting_campaign_bytes: Arc<[u8]>,
        ) -> Result<Box<dyn MissionEndTask<SubmissionAcceptedV1>>, String> {
            submission.validate().map_err(|error| error.to_string())?;
            assert!(!replay_bytes.is_empty());
            assert!(!starting_campaign_bytes.is_empty());
            self.calls.lock().unwrap().uploads += 1;
            Ok(Box::new(DelayedTask {
                remaining_polls: self.upload_delay_polls,
                result: Some(Ok(SubmissionAcceptedV1 {
                    schema_version: SCHEMA_VERSION_V1,
                    submission_id: id("submission-1"),
                    state: SubmissionLifecycleV1::Queued,
                    retry_after_ms: 250,
                })),
            }))
        }
    }

    struct TestExporter(Arc<[u8]>);

    impl MissionEndReplayExporter for TestExporter {
        fn begin(&mut self) -> Result<Box<dyn MissionEndTask<Arc<[u8]>>>, String> {
            Ok(Box::new(ImmediateTask(Some(Ok(Arc::clone(&self.0))))))
        }
    }

    struct TestAuthorizer(ed25519_dalek::SigningKey);

    impl MissionEndSubmissionAuthorizer for TestAuthorizer {
        fn begin(
            &mut self,
            request: SubmissionAuthorizationRequest,
        ) -> Result<Box<dyn SubmissionAuthorizationTask>, String> {
            assert!(request.continuation_claim().unwrap().is_none());
            let envelope = request.envelope(None);
            envelope.validate().map_err(|error| error.to_string())?;
            let signature = self.0.sign(
                &envelope
                    .signing_bytes()
                    .map_err(|error| error.to_string())?,
            );
            let public_key = PublicKey32::from_bytes(self.0.verifying_key().to_bytes());
            let signed = SignedSubmissionV1 {
                schema_version: SCHEMA_VERSION_V1,
                submission: envelope,
                algorithm: SignatureAlgorithmV1::Ed25519,
                participant_signatures: vec![ParticipantSignatureV1 {
                    public_key,
                    signature: Signature64::from_bytes(signature.to_bytes()),
                }],
            };
            Ok(Box::new(ImmediateAuthorizationTask {
                result: Some(Ok(signed)),
                progress: ParticipantSigningProgress {
                    expected: vec![public_key],
                    signed: vec![public_key],
                },
            }))
        }
    }

    struct TestPeerCoSigner {
        consented: bool,
        polls: VecDeque<PeerCoSignPoll>,
    }

    impl MissionEndPeerCoSigner for TestPeerCoSigner {
        fn consent(&mut self) -> Result<(), String> {
            if self.consented {
                return Err("duplicate peer consent".to_owned());
            }
            self.consented = true;
            Ok(())
        }

        fn poll(&mut self) -> PeerCoSignPoll {
            if !self.consented {
                return PeerCoSignPoll::AwaitingConsent;
            }
            self.polls.pop_front().unwrap_or(PeerCoSignPoll::Failed(
                "test peer exhausted its scripted lifecycle".to_owned(),
            ))
        }

        fn has_pending_work(&self) -> bool {
            self.consented && !self.polls.is_empty()
        }
    }

    fn id(value: &str) -> OpaqueId {
        OpaqueId::new(value).unwrap()
    }

    fn query(metric: BoardMetricV1) -> LeaderboardQueryV1 {
        LeaderboardQueryV1 {
            schema_version: SCHEMA_VERSION_V1,
            subject_kind: LeaderboardQuerySubjectV1::Mission,
            mission_id: Some("Dem_Lei_MP".to_owned()),
            mission_scope: Some(BoardCategoryV1::IndividualLevel),
            metric,
            content_identity_sha256: Digest32::from_bytes([5; 32]),
            rules_config_sha256: Some(Digest32::from_bytes([6; 32])),
            ruleset_manifest_sha256: Some(Digest32::from_bytes([7; 32])),
            competition_manifest_sha256: None,
            max_concurrent_players: Some(1),
            player_public_key: None,
            limit: 50,
            cursor: None,
        }
    }

    fn board_page(query: &LeaderboardQueryV1) -> LeaderboardPageV1 {
        LeaderboardPageV1 {
            schema_version: SCHEMA_VERSION_V1,
            filter: query.filter().unwrap(),
            accepted_sequence_watermark: 0,
            previous_cursor: None,
            entries: Vec::new(),
            next_cursor: None,
        }
    }

    struct Fixture {
        bundle: MissionEndRunBundle,
        offer: SubmissionOfferV1,
        compact: Arc<[u8]>,
        key: ed25519_dalek::SigningKey,
    }

    fn fixture(outcome: MissionEndOutcome) -> Fixture {
        let key = ed25519_dalek::SigningKey::from_bytes(&[31; 32]);
        let public_key = PublicKey32::from_bytes(key.verifying_key().to_bytes());
        let campaign = robin_engine::campaign::Campaign::default();
        let campaign_bytes = bitcode::encode(&campaign);
        let mission_id = "Dem_Lei_MP".to_owned();
        let ranked = RankedSessionConfigV1 {
            custom_rules_config: None,
            custom_canonical_campaign: None,
            schema_version: SCHEMA_VERSION_V1,
            mission_id: mission_id.clone(),
            content_edition: OfficialContentEditionV1::Demo,
            content_subject: OfficialContentSubjectV1::FieldMission {
                mission_id: mission_id.clone(),
            },
            simulation_seed: SimulationSeed64::new(42),
            starting_campaign_sha256: Digest32::digest_bytes(&campaign_bytes),
            starting_campaign_byte_length: campaign_bytes.len() as u64,
            prepared_inputs_projection_sha256: Digest32::from_bytes([18; 32]),
            prepared_mission_inputs_seal_sha256: Digest32::from_bytes([19; 32]),
            build_manifest_sha256: Digest32::from_bytes([4; 32]),
            content_manifest_sha256: Digest32::from_bytes([5; 32]),
            campaign_content_manifest_sha256: None,
            rules_config_sha256: Digest32::from_bytes([6; 32]),
            ruleset_manifest_sha256: Digest32::from_bytes([7; 32]),
            competition_manifest_sha256: None,
            spellforge_content_sha256: None,
            resource_locale_root: ResourceLocaleRootV1::new("1033").unwrap(),
            speech_timing: SpeechTimingAuthorityV1::LanguagePack {
                canonical_locale: "en-US".to_owned(),
            },
        };
        // An individual-level offer request is only valid when the genesis
        // carries the fresh-run preflight grant that admitted it, bound to
        // the same host identity, ranked session, and starting campaign.
        let fresh_run_preflight_grant = robin_run_protocol::FreshRunPreflightGrantV1 {
            claim: robin_run_protocol::FreshRunPreflightGrantClaimV1 {
                schema_version: SCHEMA_VERSION_V1,
                grant_id: id("fresh-grant-1"),
                grant_nonce: ChallengeNonce32::from_bytes([21; 32]),
                grant_authority_public_key: PublicKey32::from_bytes([22; 32]),
                host_public_key: public_key,
                grant_request_sha256: Digest32::from_bytes([23; 32]),
                ranked_session_sha256: ranked.canonical_digest().unwrap(),
                replay_session_id: Digest32::from_bytes([11; 32]),
                host_participant_instance_id: Digest32::from_bytes([12; 32]),
                host_nonce: ChallengeNonce32::from_bytes([13; 32]),
                scope: robin_run_protocol::FreshRunScopeV1::IndividualLevel,
                starting_campaign: robin_run_protocol::ArtifactRefV1 {
                    sha256: Digest32::digest_bytes(&campaign_bytes),
                    byte_length: campaign_bytes.len() as u64,
                    media_type: robin_run_protocol::RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
                },
                admitted_at_unix_ms: 1,
                expires_at_unix_ms: 1_800_000_000_000,
            },
            algorithm: SignatureAlgorithmV1::Ed25519,
            authority_signature: Signature64::from_bytes([24; 64]),
        };
        let genesis = ReplaySessionGenesisV1 {
            claim: ReplaySessionGenesisClaimV1 {
                schema_version: SCHEMA_VERSION_V1,
                network_protocol_version: robin_engine::multiplayer::NET_PROTOCOL_VERSION,
                host_public_key: public_key,
                replay_session_id: Digest32::from_bytes([11; 32]),
                host_participant_instance_id: Digest32::from_bytes([12; 32]),
                host_nonce: ChallengeNonce32::from_bytes([13; 32]),
                ranked_session: ranked,
                fresh_run_preflight_grant: Some(fresh_run_preflight_grant),
                campaign_continuation_preflight_grant: None,
                competition_run_grant: None,
            },
            algorithm: SignatureAlgorithmV1::Ed25519,
            host_signature: Signature64::from_bytes([14; 64]),
        };
        let host = ParticipantClaimV1 {
            seat: 0,
            participant_instance_id: genesis.claim.host_participant_instance_id,
            public_key,
            public_disclosure: ParticipantPublicDisclosureV1::NamedProfile,
            join_attestation: None,
        };
        let offer_request = SubmissionOfferRequestV1 {
            schema_version: SCHEMA_VERSION_V1,
            max_concurrent_players: 1,
            participant_instance_count: 1,
            participant_claims: vec![host.clone()],
            session_genesis: genesis.clone(),
            mission_id: mission_id.clone(),
            scope_request: ScopeRequestV1::IndividualLevel,
            ruleset_manifest_sha256: Digest32::from_bytes([7; 32]),
            competition_manifest_sha256: None,
        };
        let transcript = ReplaySessionTranscriptV1 {
            schema_version: SCHEMA_VERSION_V1,
            session_genesis_sha256: genesis.canonical_digest().unwrap(),
            replay_session_id: genesis.claim.replay_session_id,
            host_participant_instance_id: genesis.claim.host_participant_instance_id,
            participant_instance_count: 1,
            max_concurrent_players: 1,
            events: vec![ReplaySeatLifecycleEventV1 {
                event_ordinal: 0,
                replay_ordinal: 0,
                seat: 0,
                participant_instance_id: genesis.claim.host_participant_instance_id,
                lifecycle: ReplaySeatLifecycleKindV1::Connected {
                    connection_epoch: 0,
                },
            }],
        };
        let offer = SubmissionOfferV1 {
            schema_version: SCHEMA_VERSION_V1,
            upload_challenge_id: id("upload-1"),
            upload_challenge_nonce: ChallengeNonce32::from_bytes([2; 32]),
            expires_at_unix_ms: 1_800_000_000_000,
            max_concurrent_players: 1,
            participant_instance_count: 1,
            participant_claims: vec![host],
            session_genesis: genesis,
            mission_id: mission_id.clone(),
            competition_manifest_sha256: None,
            build_manifest_sha256: Digest32::from_bytes([4; 32]),
            content_manifest_sha256: Digest32::from_bytes([5; 32]),
            rules_config_sha256: Digest32::from_bytes([6; 32]),
            ruleset_manifest_sha256: Digest32::from_bytes([7; 32]),
            starting_state: InitialStateExpectationV1::IndividualLevel {
                template_id: id("demo-template"),
                campaign_state_requirement: CanonicalCampaignStateRequirementV1 {
                    edition: OfficialContentEditionV1::Demo,
                    kind: CanonicalCampaignStateKindV1::IndividualTemplate,
                    rules_config_sha256: Digest32::from_bytes([6; 32]),
                },
                campaign_sha256: Digest32::digest_bytes(&campaign_bytes),
                starting_campaign_byte_length: campaign_bytes.len() as u64,
            },
            allowed_metrics: vec![BoardMetricV1::OriginalScore, BoardMetricV1::FastestSuccess],
        };
        let replay = robin_engine::replay::ReplayData::try_from(robin_engine::replay::ReplayFile {
            header: robin_engine::replay::ReplayHeader {
                mission_id: mission_id.clone(),
                mission_assets: robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                    &mission_id,
                    &mission_id,
                    &mission_id,
                )
                .expect("valid built-in mission-end test descriptor"),
                rng_seed: 42,
                sim_config: robin_engine::engine::SimConfig::default(),
                spellforge_package: None,
                version: robin_engine::replay::REPLAY_SCHEMA_VERSION,
                total_frames: 1,
                rankability: robin_engine::replay_rankability::ReplayRankability::rankable(),
                campaign: campaign_bytes.clone(),
            },
            frames: BTreeMap::from([(
                0,
                robin_engine::replay::ReplayFrame {
                    timeline_before: 0,
                    timeline_after: 0,
                    input: robin_engine::engine::SimulationFrameInput::default(),
                    host_controls: Vec::new(),
                },
            )]),
            hashes: BTreeMap::new(),
            save_markers: BTreeMap::new(),
            load_backs: BTreeMap::new(),
        });
        let replay = replay.expect("valid mission-end replay fixture");
        let compact: Arc<[u8]> =
            robin_replay_format::encode_compact(&replay, robin_replay_format::ENGINE_VERSION_HASH)
                .unwrap()
                .into_bytes()
                .into();
        let score = query(BoardMetricV1::OriginalScore);
        let eligible_submission = outcome.can_submit().then_some(MissionEndSubmissionInput {
            offer_request,
            replay_session_transcript: transcript,
            requested_metrics: vec![BoardMetricV1::OriginalScore],
            campaign_controller_public_key: None,
            starting_campaign_bytes: campaign_bytes.into(),
        });
        Fixture {
            bundle: MissionEndRunBundle {
                outcome,
                multiplayer: false,
                boards: vec![MissionEndBoard {
                    tab: LeaderboardTab::Score,
                    label: "Score".to_owned(),
                    query: score,
                }],
                eligible_submission,
                submission_unavailable_reason: None,
            },
            offer,
            compact,
            key,
        }
    }

    fn controller(
        fixture: Fixture,
        preferences: LeaderboardPreferences,
        calls: Arc<Mutex<BackendCalls>>,
    ) -> MissionEndLeaderboardController {
        let page = board_page(&fixture.bundle.boards[0].query);
        MissionEndLeaderboardController::new(
            fixture.bundle,
            preferences,
            Box::new(TestBackend {
                page,
                offer: fixture.offer,
                calls,
                upload_delay_polls: 0,
            }),
            Box::new(TestAuthorizer(fixture.key)),
            Box::new(TestExporter(fixture.compact)),
        )
        .unwrap()
    }

    #[test]
    fn preparing_retains_either_early_result_after_presentation_closes() {
        for (offer_delay, replay_delay) in [(0, 3), (3, 0)] {
            let fixture = fixture(MissionEndOutcome::Won);
            let offer = fixture.offer.clone();
            let replay = Arc::clone(&fixture.compact);
            let calls = Arc::new(Mutex::new(BackendCalls::default()));
            let mut controller = controller(
                fixture,
                LeaderboardPreferences::default(),
                Arc::clone(&calls),
            );
            controller.submission_task = SubmissionTask::Preparing(PreparingSubmission {
                offer_task: Box::new(DelayedTask {
                    remaining_polls: offer_delay,
                    result: Some(Ok(offer)),
                }),
                replay_task: Box::new(DelayedTask {
                    remaining_polls: replay_delay,
                    result: Some(Ok(replay)),
                }),
                offer: None,
                replay: None,
            });
            controller
                .apply_action(MissionEndLeaderboardAction::Close)
                .unwrap();
            for _ in 0..8 {
                controller.poll();
            }
            assert!(matches!(
                controller.submission_state(),
                MissionSubmissionState::Queued(_)
            ));
            assert_eq!(calls.lock().unwrap().uploads, 1);
            assert!(!controller.has_pending_submission());
            assert!(controller.has_unpersisted_receipt_watch());
        }
    }

    #[test]
    fn preparing_failure_retires_both_tasks_while_counterpart_is_pending() {
        for offer_fails in [true, false] {
            let fixture = fixture(MissionEndOutcome::Won);
            let offer = fixture.offer.clone();
            let replay = Arc::clone(&fixture.compact);
            let calls = Arc::new(Mutex::new(BackendCalls::default()));
            let mut controller = controller(
                fixture,
                LeaderboardPreferences::default(),
                Arc::clone(&calls),
            );
            controller.submission_task = SubmissionTask::Preparing(PreparingSubmission {
                offer_task: Box::new(DelayedTask {
                    remaining_polls: if offer_fails { 0 } else { 10 },
                    result: Some(if offer_fails {
                        Err("offer failed".into())
                    } else {
                        Ok(offer)
                    }),
                }),
                replay_task: Box::new(DelayedTask {
                    remaining_polls: if offer_fails { 10 } else { 0 },
                    result: Some(if offer_fails {
                        Ok(replay)
                    } else {
                        Err("replay failed".into())
                    }),
                }),
                offer: None,
                replay: None,
            });
            controller
                .apply_action(MissionEndLeaderboardAction::Close)
                .unwrap();
            for _ in 0..12 {
                controller.poll();
            }
            assert!(matches!(
                controller.submission_state(),
                MissionSubmissionState::Failed(_)
            ));
            assert!(controller.can_retire_after_close());
            assert_eq!(calls.lock().unwrap().uploads, 0);
        }
    }

    #[test]
    fn boards_open_for_failed_missions_but_submission_never_does() {
        let calls = Arc::new(Mutex::new(BackendCalls::default()));
        let mut controller = controller(
            fixture(MissionEndOutcome::Lost),
            LeaderboardPreferences::default(),
            Arc::clone(&calls),
        );
        assert!(controller.is_visible());
        assert_eq!(
            controller.submission_state(),
            &MissionSubmissionState::WonMissionRequired
        );
        assert!(matches!(controller.board_state(), BoardLoadState::Loading));
        controller.poll();
        assert!(matches!(controller.board_state(), BoardLoadState::Ready(_)));
        let calls = calls.lock().unwrap();
        assert_eq!(calls.boards, 1);
        assert_eq!(calls.offers, 0);
        assert_eq!(calls.uploads, 0);
    }

    #[test]
    fn presentation_toggle_hides_only_the_board_and_does_not_invent_work() {
        let calls = Arc::new(Mutex::new(BackendCalls::default()));
        let preferences = LeaderboardPreferences {
            show_mission_end_boards: false,
            ..LeaderboardPreferences::default()
        };
        let mut controller = controller(
            fixture(MissionEndOutcome::Lost),
            preferences,
            Arc::clone(&calls),
        );
        assert!(!controller.is_visible());
        controller.poll();
        let calls = calls.lock().unwrap();
        assert_eq!(calls.boards, 0);
        assert_eq!(calls.offers, 0);
        assert_eq!(calls.uploads, 0);
    }

    #[test]
    fn default_consent_is_off_and_explicit_consent_advances_one_stage_per_poll() {
        let calls = Arc::new(Mutex::new(BackendCalls::default()));
        let mut controller = controller(
            fixture(MissionEndOutcome::Won),
            LeaderboardPreferences::default(),
            Arc::clone(&calls),
        );
        assert_eq!(
            controller.submission_state(),
            &MissionSubmissionState::AwaitingConsent
        );
        assert_eq!(calls.lock().unwrap().offers, 0);
        controller
            .apply_action(MissionEndLeaderboardAction::SubmitThisRun)
            .unwrap();
        assert_eq!(
            controller.submission_state(),
            &MissionSubmissionState::PreparingArtifacts
        );
        controller.poll();
        assert!(matches!(
            controller.submission_state(),
            MissionSubmissionState::AwaitingParticipantSignatures(_)
        ));
        controller.poll();
        assert_eq!(
            controller.submission_state(),
            &MissionSubmissionState::Uploading
        );
        controller.poll();
        assert!(matches!(
            controller.submission_state(),
            MissionSubmissionState::Queued(_)
        ));
        let calls = calls.lock().unwrap();
        assert_eq!(calls.offers, 1);
        assert_eq!(calls.uploads, 1);
    }

    #[test]
    fn peer_consent_never_uploads_and_controller_persists_the_accepted_receipt() {
        let calls = Arc::new(Mutex::new(BackendCalls::default()));
        let mut fixture = fixture(MissionEndOutcome::Won);
        let controller_key = PublicKey32::from_bytes(fixture.key.verifying_key().to_bytes());
        fixture.bundle.eligible_submission = None;
        fixture.bundle.submission_unavailable_reason = Some(
            "the host owns the one ranked upload and requests this peer's signature".to_owned(),
        );
        let page = board_page(&fixture.bundle.boards[0].query);
        let accepted = SubmissionAcceptedV1 {
            schema_version: SCHEMA_VERSION_V1,
            submission_id: id("peer-submission"),
            state: SubmissionLifecycleV1::Queued,
            retry_after_ms: 250,
        };
        let progress = ParticipantSigningProgress {
            expected: vec![controller_key],
            signed: vec![controller_key],
        };
        let mut controller = MissionEndLeaderboardController::new_peer(
            fixture.bundle,
            LeaderboardPreferences::default(),
            Box::new(TestBackend {
                page,
                offer: fixture.offer,
                calls: Arc::clone(&calls),
                upload_delay_polls: 0,
            }),
            Box::new(TestPeerCoSigner {
                consented: false,
                polls: VecDeque::from([
                    PeerCoSignPoll::ResponseSent(progress),
                    PeerCoSignPoll::Accepted(accepted),
                ]),
            }),
            Some(controller_key),
            Arc::new(crate::replay_service::ReplayService::default()).exports(),
        )
        .unwrap();

        assert_eq!(
            controller.submission_state(),
            &MissionSubmissionState::AwaitingConsent
        );
        controller
            .apply_action(MissionEndLeaderboardAction::SubmitThisRun)
            .unwrap();
        controller.poll();
        assert_eq!(
            controller.submission_state(),
            &MissionSubmissionState::Uploading
        );
        assert!(controller.has_pending_submission());
        controller.poll();
        assert!(matches!(
            controller.submission_state(),
            MissionSubmissionState::Queued(_)
        ));
        assert_eq!(calls.lock().unwrap().offers, 0);
        assert_eq!(calls.lock().unwrap().uploads, 0);

        let mut handed_off = None;
        assert!(
            controller
                .persist_queued_receipt_watch_with(|handoff| {
                    handed_off = Some(handoff);
                    Ok(true)
                })
                .unwrap()
        );
        assert_eq!(
            handed_off.unwrap().key.controller_public_key,
            controller_key
        );
    }

    #[test]
    fn dismissing_does_not_cancel_a_consented_submission() {
        let calls = Arc::new(Mutex::new(BackendCalls::default()));
        let fixture = fixture(MissionEndOutcome::Won);
        let expected_controller = PublicKey32::from_bytes(fixture.key.verifying_key().to_bytes());
        let mut controller = controller(fixture, LeaderboardPreferences::default(), calls);
        controller
            .apply_action(MissionEndLeaderboardAction::SubmitThisRun)
            .unwrap();
        assert!(controller.has_pending_submission());
        assert_eq!(
            controller
                .apply_action(MissionEndLeaderboardAction::Close)
                .unwrap(),
            MissionEndLeaderboardEvent::Closed
        );
        assert!(controller.is_closed());
        assert!(controller.has_pending_submission());
        assert!(!controller.can_retire_after_close());

        controller.poll();
        controller.poll();
        controller.poll();
        assert!(!controller.has_pending_submission());
        assert!(!controller.can_retire_after_close());
        let mut handed_off = None;
        assert!(
            controller
                .persist_queued_receipt_watch_with(|handoff| {
                    handed_off = Some(handoff);
                    Ok(true)
                })
                .unwrap()
        );
        assert_eq!(
            handed_off.unwrap().key.controller_public_key,
            expected_controller
        );
        assert!(controller.can_retire_after_close());
    }

    #[test]
    fn closed_delayed_upload_moves_to_session_owner_without_blocking_transition() {
        let calls = Arc::new(Mutex::new(BackendCalls::default()));
        let fixture = fixture(MissionEndOutcome::Won);
        let page = board_page(&fixture.bundle.boards[0].query);
        let mut controller = MissionEndLeaderboardController::new(
            fixture.bundle,
            LeaderboardPreferences {
                show_mission_end_boards: false,
                always_submit_eligible_runs: true,
                ..LeaderboardPreferences::default()
            },
            Box::new(TestBackend {
                page,
                offer: fixture.offer,
                calls: Arc::clone(&calls),
                upload_delay_polls: 2,
            }),
            Box::new(TestAuthorizer(fixture.key)),
            Box::new(TestExporter(fixture.compact)),
        )
        .unwrap();
        controller
            .apply_action(MissionEndLeaderboardAction::Close)
            .unwrap();

        let mut background = MissionEndLeaderboardBackground::default();
        background.adopt(controller);
        assert_eq!(background.active_count(), 1);
        let mut handed_off = 0;

        // Foreground presentation/mission state has already relinquished the
        // controller. Several later frames may pass before HTTP completes.
        for _ in 0..4 {
            background.poll_with_receipt_sink(|_| {
                handed_off += 1;
                Ok(true)
            });
            assert_eq!(background.active_count(), 1);
        }
        background.poll_with_receipt_sink(|_| {
            handed_off += 1;
            Ok(true)
        });
        assert_eq!(background.active_count(), 0);
        assert_eq!(handed_off, 1);
        assert_eq!(calls.lock().unwrap().uploads, 1);
    }

    #[test]
    fn always_submit_is_explicit_and_starts_only_eligible_wins() {
        let calls = Arc::new(Mutex::new(BackendCalls::default()));
        let preferences = LeaderboardPreferences {
            always_submit_eligible_runs: true,
            ..LeaderboardPreferences::default()
        };
        let won = controller(
            fixture(MissionEndOutcome::Won),
            preferences.clone(),
            Arc::clone(&calls),
        );
        assert_eq!(
            won.submission_state(),
            &MissionSubmissionState::PreparingArtifacts
        );
        let lost = controller(
            fixture(MissionEndOutcome::Interrupted),
            preferences,
            Arc::clone(&calls),
        );
        assert_eq!(
            lost.submission_state(),
            &MissionSubmissionState::WonMissionRequired
        );
        assert_eq!(calls.lock().unwrap().offers, 1);
    }

    #[test]
    fn exact_authorization_rejects_substituted_offer_transcript_and_artifacts() {
        let fixture = fixture(MissionEndOutcome::Won);
        let input = fixture.bundle.eligible_submission.as_ref().unwrap();
        let artifacts = SubmissionArtifactsV1 {
            replay: canonical_replay_artifact(
                &fixture.compact,
                &input.starting_campaign_bytes,
                "Dem_Lei_MP",
                &input.replay_session_transcript,
            )
            .unwrap(),
            starting_campaign: ArtifactRefV1 {
                sha256: Digest32::digest_bytes(&input.starting_campaign_bytes),
                byte_length: input.starting_campaign_bytes.len() as u64,
                media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
            },
        };
        let request = SubmissionAuthorizationRequest {
            offer_request: input.offer_request.clone(),
            offer: fixture.offer,
            replay_session_transcript: input.replay_session_transcript.clone(),
            artifacts,
            requested_metrics: input.requested_metrics.clone(),
            campaign_controller_public_key: None,
        };
        let envelope = request.envelope(None);
        let signature = fixture.key.sign(&envelope.signing_bytes().unwrap());
        let public_key = PublicKey32::from_bytes(fixture.key.verifying_key().to_bytes());
        let mut signed = SignedSubmissionV1 {
            schema_version: SCHEMA_VERSION_V1,
            submission: envelope,
            algorithm: SignatureAlgorithmV1::Ed25519,
            participant_signatures: vec![ParticipantSignatureV1 {
                public_key,
                signature: Signature64::from_bytes(signature.to_bytes()),
            }],
        };
        validate_authorized_submission(&request, &signed).unwrap();

        let mut substituted_offer_request = request.clone();
        substituted_offer_request.offer_request.mission_id = "foreign-mission".to_owned();
        assert!(substituted_offer_request.validate_exact_context().is_err());

        let mut substituted_transcript = request.clone();
        substituted_transcript
            .replay_session_transcript
            .replay_session_id = Digest32::from_bytes([98; 32]);
        assert!(substituted_transcript.validate_exact_context().is_err());

        signed.submission.artifacts.replay.artifact.sha256 = Digest32::from_bytes([99; 32]);
        assert!(validate_authorized_submission(&request, &signed).is_err());
    }

    #[test]
    fn canonical_artifact_rejects_tainted_rankability_evidence() {
        let fixture = fixture(MissionEndOutcome::Won);
        let input = fixture.bundle.eligible_submission.as_ref().unwrap();
        let text = std::str::from_utf8(&fixture.compact).unwrap();
        let (engine_hash, mut replay) = robin_replay_format::decode_compact(text).unwrap();
        replay
            .try_edit_header(|header| {
                header.rankability.taint(
                    robin_engine::replay_rankability::InputTaintKind::HttpPlayerCommand,
                    0,
                )
            })
            .unwrap();
        let tainted = robin_replay_format::encode_compact(&replay, &engine_hash).unwrap();

        let error = canonical_replay_artifact(
            tainted.as_bytes(),
            &input.starting_campaign_bytes,
            "Dem_Lei_MP",
            &input.replay_session_transcript,
        )
        .unwrap_err();
        assert!(error.to_string().contains("http_player_command"));
    }

    #[test]
    fn cosign_progress_must_be_an_exact_canonical_subset() {
        let first = PublicKey32::from_bytes([1; 32]);
        let second = PublicKey32::from_bytes([2; 32]);
        assert!(
            ParticipantSigningProgress {
                expected: vec![first, second],
                signed: vec![second],
            }
            .validate()
            .is_ok()
        );
        assert!(
            ParticipantSigningProgress {
                expected: vec![first, second],
                signed: vec![PublicKey32::from_bytes([3; 32])],
            }
            .validate()
            .is_err()
        );
    }
}

mod arc_bytes {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::sync::Arc;

    pub fn serialize<S>(bytes: &Arc<[u8]>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        bytes.as_ref().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Arc<[u8]>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Vec::<u8>::deserialize(deserializer).map(Arc::from)
    }
}
