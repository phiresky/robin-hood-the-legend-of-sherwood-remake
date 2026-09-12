//! Redacted public verification proofs, campaign proof binding and run-detail validation.

use super::{
    AggregatePublicParticipantV1, BoardCategoryV1, BoardMetricV1, BoardMetricValueV1,
    LeaderboardSubjectV1, MissionFacetV1, PublicParticipantV1, RunContentIdentityV1, RunMetricsV1,
    VerifiedRunCompositionV1, validate_public_roster, validate_subject_composition,
};
use crate::CanonicalDocument as _;
use crate::{
    CampaignAggregationConsentV1, CampaignContentManifestV1, CampaignSessionKindV1, Digest32,
    InputProvenanceStatusV1, OfficialContentEditionV1, OfficialContentSubjectV1, OpaqueId,
    ParticipantPublicDisclosureV1, PublicKey32, PublishedRulesetV1, ReplayArtifactV1,
    RulesetBoardScopeV1, RunScopeKindV1, SimulationSeed64, TerminalOutcomeV1, Validate,
    ValidationError, VerificationLimitsV1, VerificationRequestV1, VerificationResultV1,
    VerificationStatusV1, VerifiedAchievementEvaluationV1, VerifiedAchievementV1,
    VerifiedCampaignAggregateV1, VerifiedCampaignSessionV1,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicBuildV1 {
    pub manifest_sha256: Digest32,
    pub source_commit: String,
    pub display_name: String,
}

impl Validate for PublicBuildV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::nonzero("run.build.manifest_sha256", &self.manifest_sha256)?;
        if !matches!(self.source_commit.len(), 40 | 64)
            || !self
                .source_commit
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ValidationError::InvalidRelativePath {
                field: "run.build.source_commit",
                reason: "expected a full lowercase 40- or 64-character Git object id",
            });
        }
        crate::validation::text("run.build.display_name", &self.display_name, 100)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AchievementSummaryV1 {
    pub display_name: String,
    pub verified: PublicAchievementDecisionV1,
}

impl Validate for AchievementSummaryV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::text("run.achievement.display_name", &self.display_name, 100)?;
        self.verified.validate()
    }
}

/// Public achievement state deliberately excludes free-form verifier evidence,
/// which remains private because it may contain implementation diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicAchievementDecisionV1 {
    pub achievement_id: OpaqueId,
    pub evaluation: VerifiedAchievementEvaluationV1,
}

impl PublicAchievementDecisionV1 {
    pub fn from_private(value: &VerifiedAchievementV1) -> Self {
        Self {
            achievement_id: value.achievement_id.clone(),
            evaluation: value.evaluation,
        }
    }

    pub const fn is_awarded(&self) -> bool {
        matches!(self.evaluation, VerifiedAchievementEvaluationV1::Earned)
    }
}

impl Validate for PublicAchievementDecisionV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ViewerContentRequirementV1 {
    BundledDemo { content_manifest_sha256: Digest32 },
    UserLocalRetail { content_manifest_sha256: Digest32 },
}

impl ViewerContentRequirementV1 {
    pub const fn content_manifest_sha256(&self) -> Digest32 {
        match self {
            Self::BundledDemo {
                content_manifest_sha256,
            }
            | Self::UserLocalRetail {
                content_manifest_sha256,
            } => *content_manifest_sha256,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ViewerAvailabilityV1 {
    Available {
        content_requirement: ViewerContentRequirementV1,
    },
    Unavailable {
        safe_reason: String,
    },
}

/// Authenticated viewer launch facts. Artifact locations come only from the
/// canonical build manifest and a compile-time/deployment allowlisted origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewerLaunchV1 {
    pub build_manifest_sha256: Digest32,
    pub availability: ViewerAvailabilityV1,
}

impl Validate for ViewerLaunchV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::nonzero(
            "run.viewer.build_manifest_sha256",
            &self.build_manifest_sha256,
        )?;
        match &self.availability {
            ViewerAvailabilityV1::Available {
                content_requirement,
            } if content_requirement.content_manifest_sha256().is_zero() => {
                return Err(ValidationError::Zero {
                    field: "run.viewer.content_manifest_sha256",
                });
            }
            ViewerAvailabilityV1::Unavailable { safe_reason } => {
                crate::validation::text("run.viewer.safe_reason", safe_reason, 500)?;
            }
            ViewerAvailabilityV1::Available { .. } => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FullCampaignSessionKindV1 {
    FieldMission { mission_id: String },
    Headquarters { hq_sequence: u32 },
}

impl FullCampaignSessionKindV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::FieldMission { mission_id } => {
                crate::validation::text("full_campaign_session.mission_id", mission_id, 256)
            }
            Self::Headquarters { hq_sequence } if *hq_sequence == 0 => Err(ValidationError::Zero {
                field: "full_campaign_session.hq_sequence",
            }),
            Self::Headquarters { .. } => Ok(()),
        }
    }
}

/// A named participant identity which is safe to bind into a public request.
/// Public keys are privately verified as unique within the session, so
/// `(seat, public_key)` distinguishes sequential occupants without publishing
/// their private transcript-scoped instance IDs. Anonymous seats are represented
/// only by the aggregate anonymous count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicNamedParticipantClaimV1 {
    pub seat: u16,
    pub public_key: PublicKey32,
}

impl Validate for PublicNamedParticipantClaimV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::nonzero("public_named_participant.identity", &self.public_key)?;
        if self.seat >= crate::MAX_REPLAY_SEATS_V1 {
            return Err(ValidationError::CountOutOfRange {
                field: "public_named_participant.seat",
            });
        }
        Ok(())
    }
}

/// Canonical public projection of a private `VerificationRequestV1`.
///
/// It binds only facts intentionally published with a run. In particular it
/// excludes the request ID, upload challenge, replay-session transcript,
/// starting-campaign bytes, prepared-input closure, chain/predecessor locators,
/// anonymous keys, join attestations, signatures, transport identities,
/// nonces, and host identity. Its replay reference is the exact submitted and
/// verified artifact, not a projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicVerificationRequestV1 {
    pub schema_version: u32,
    pub replay: ReplayArtifactV1,
    pub content_edition: OfficialContentEditionV1,
    pub content_subject: OfficialContentSubjectV1,
    pub simulation_seed: SimulationSeed64,
    pub scope_kind: RunScopeKindV1,
    pub campaign_aggregation_consent: CampaignAggregationConsentV1,
    pub build_manifest_sha256: Digest32,
    pub content_manifest_sha256: Digest32,
    pub campaign_content_manifest_sha256: Option<Digest32>,
    pub rules_config_sha256: Digest32,
    pub ruleset_manifest_sha256: Digest32,
    pub competition_manifest_sha256: Option<Digest32>,
    pub requested_metrics: Vec<BoardMetricV1>,
    pub limits: VerificationLimitsV1,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u16,
    pub named_participant_instance_count: u16,
    pub anonymous_participant_instance_count: u16,
    pub named_participants: Vec<PublicNamedParticipantClaimV1>,
}

impl PublicVerificationRequestV1 {
    pub fn from_private(
        request: &VerificationRequestV1,
        result: &VerificationResultV1,
    ) -> Result<Self, ValidationError> {
        request.validate()?;
        result.validate()?;
        let request_sha256 =
            request
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "public_verification_request.private_request",
                })?;
        let submission = &request.submission.submission;
        let offer = &submission.offer;
        let ranked = &offer.session_genesis.claim.ranked_session;
        let session_genesis_sha256 = offer.session_genesis.canonical_digest().map_err(|_| {
            ValidationError::ClaimMismatch {
                field: "public_verification_request.session_genesis",
            }
        })?;
        let VerificationStatusV1::Verified(verified) = &result.status else {
            return Err(ValidationError::ClaimMismatch {
                field: "public_verification_request.private_result.status",
            });
        };
        if result.request_id != request.request_id
            || result.verification_request_sha256 != request_sha256
            || result.artifacts != submission.artifacts
            || result.session_genesis_sha256 != session_genesis_sha256
            || result.build_manifest_sha256 != offer.build_manifest_sha256
            || result.content_manifest_sha256 != offer.content_manifest_sha256
            || result.rules_config_sha256 != offer.rules_config_sha256
            || result.ruleset_manifest_sha256 != offer.ruleset_manifest_sha256
            || result.competition_manifest_sha256 != offer.competition_manifest_sha256
            || verified.scope_kind != offer.starting_state.scope_kind()
            || verified.campaign_aggregation_consent != submission.campaign_aggregation_consent
            || verified.max_concurrent_players != offer.max_concurrent_players
            || verified.participant_instance_count != offer.participant_instance_count
            || verified.authenticated_participant_claims != offer.participant_claims
            || verified.replay_session_transcript != submission.replay_session_transcript
        {
            return Err(ValidationError::ClaimMismatch {
                field: "public_verification_request.private_binding",
            });
        }
        let mut named_participants = offer
            .participant_claims
            .iter()
            .filter(|claim| claim.public_disclosure == ParticipantPublicDisclosureV1::NamedProfile)
            .map(|claim| PublicNamedParticipantClaimV1 {
                seat: claim.seat,
                public_key: claim.public_key,
            })
            .collect::<Vec<_>>();
        named_participants.sort_by_key(|participant| (participant.seat, participant.public_key));
        let public = Self {
            schema_version: crate::SCHEMA_VERSION_V1,
            replay: submission.artifacts.replay.clone(),
            content_edition: ranked.content_edition,
            content_subject: ranked.content_subject.clone(),
            simulation_seed: ranked.simulation_seed,
            scope_kind: verified.scope_kind,
            campaign_aggregation_consent: submission.campaign_aggregation_consent,
            build_manifest_sha256: offer.build_manifest_sha256,
            content_manifest_sha256: offer.content_manifest_sha256,
            campaign_content_manifest_sha256: ranked.campaign_content_manifest_sha256,
            rules_config_sha256: offer.rules_config_sha256,
            ruleset_manifest_sha256: offer.ruleset_manifest_sha256,
            competition_manifest_sha256: offer.competition_manifest_sha256,
            requested_metrics: submission.requested_metrics.clone(),
            limits: request.limits.clone(),
            max_concurrent_players: verified.max_concurrent_players,
            participant_instance_count: verified.participant_instance_count,
            named_participant_instance_count: verified.named_participant_instance_count,
            anonymous_participant_instance_count: verified.anonymous_participant_instance_count,
            named_participants,
        };
        public.validate()?;
        Ok(public)
    }
}

impl Validate for PublicVerificationRequestV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("PublicVerificationRequestV1", self.schema_version)?;
        self.replay.validate()?;
        self.content_subject.validate()?;
        self.limits.validate()?;
        for digest in [
            self.build_manifest_sha256,
            self.content_manifest_sha256,
            self.rules_config_sha256,
            self.ruleset_manifest_sha256,
        ] {
            if digest.is_zero() {
                return Err(ValidationError::Zero {
                    field: "public_verification_request.identity_digest",
                });
            }
        }
        if self
            .competition_manifest_sha256
            .is_some_and(|digest| digest.is_zero())
            || self
                .campaign_content_manifest_sha256
                .is_some_and(|digest| digest.is_zero())
        {
            return Err(ValidationError::Zero {
                field: "public_verification_request.optional_digest",
            });
        }
        match (
            self.scope_kind,
            self.campaign_aggregation_consent,
            self.campaign_content_manifest_sha256,
        ) {
            (
                RunScopeKindV1::IndividualLevel,
                CampaignAggregationConsentV1::NotAuthorized,
                None,
            )
            | (
                RunScopeKindV1::Campaign,
                CampaignAggregationConsentV1::AuthorizeSignedSessionInServerRecognizedChainV1,
                Some(_),
            ) => {}
            _ => {
                return Err(ValidationError::ClaimMismatch {
                    field: "public_verification_request.scope",
                });
            }
        }
        if self.max_concurrent_players == 0
            || self.participant_instance_count < self.max_concurrent_players
            || self
                .named_participant_instance_count
                .checked_add(self.anonymous_participant_instance_count)
                != Some(self.participant_instance_count)
            || self.named_participants.len() != usize::from(self.named_participant_instance_count)
            || !self
                .named_participants
                .windows(2)
                .all(|pair| (pair[0].seat, pair[0].public_key) < (pair[1].seat, pair[1].public_key))
        {
            return Err(ValidationError::InvalidParticipantClaims);
        }
        for participant in &self.named_participants {
            participant.validate()?;
        }
        if self
            .named_participants
            .iter()
            .map(|participant| participant.public_key)
            .collect::<BTreeSet<_>>()
            .len()
            != self.named_participants.len()
        {
            return Err(ValidationError::InvalidParticipantClaims);
        }
        if self.requested_metrics.is_empty()
            || !crate::validation::strictly_sorted(&self.requested_metrics)
        {
            return Err(ValidationError::InvalidMetrics {
                field: "public_verification_request.requested_metrics",
            });
        }
        Ok(())
    }
}

/// Public completion evidence containing only independently published facts.
/// Its canonical digest replaces every public reference to the private
/// `CampaignCompleteEvidenceV1` digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicCampaignCompleteEvidenceV1 {
    pub schema_version: u32,
    pub campaign_content_manifest_sha256: Digest32,
    pub content_manifest_sha256: Digest32,
    pub rules_config_sha256: Digest32,
    pub ruleset_manifest_sha256: Digest32,
    pub public_verification_request_sha256: Digest32,
    pub terminal_subject: OfficialContentSubjectV1,
    pub final_campaign: crate::ArtifactRefV1,
    pub final_state_sha256: Digest32,
    pub observed_progression_percent: u8,
}

impl Validate for PublicCampaignCompleteEvidenceV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("PublicCampaignCompleteEvidenceV1", self.schema_version)?;
        for digest in [
            self.campaign_content_manifest_sha256,
            self.content_manifest_sha256,
            self.rules_config_sha256,
            self.ruleset_manifest_sha256,
            self.public_verification_request_sha256,
            self.final_state_sha256,
        ] {
            if digest.is_zero() {
                return Err(ValidationError::Zero {
                    field: "public_campaign_complete_evidence.digest",
                });
            }
        }
        self.terminal_subject.validate()?;
        self.final_campaign.validate()?;
        if self.final_campaign.media_type != crate::RANKED_CAMPAIGN_MEDIA_TYPE_V1 {
            return Err(ValidationError::ClaimMismatch {
                field: "public_campaign_complete_evidence.campaign.media_type",
            });
        }
        if self.observed_progression_percent == 0 || self.observed_progression_percent > 100 {
            return Err(ValidationError::CountOutOfRange {
                field: "public_campaign_complete_evidence.observed_progression_percent",
            });
        }
        Ok(())
    }
}

/// Public, redacted projection of one private authoritative verifier result.
/// Its own canonical digest is the public result identity; no private request,
/// result, or evidence digest is included or indirectly committed here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicVerificationProofV1 {
    pub schema_version: u32,
    pub public_request: PublicVerificationRequestV1,
    pub public_request_sha256: Digest32,
    pub input_provenance: InputProvenanceStatusV1,
    pub campaign_session_kind: Option<CampaignSessionKindV1>,
    pub campaign_session_ordinal: Option<u32>,
    pub starting_campaign: crate::ArtifactRefV1,
    pub final_campaign: crate::ArtifactRefV1,
    /// Terminal snapshot digest from the sole canonical resimulation.
    pub final_state_sha256: Digest32,
    pub replay_frame_count: u32,
    pub outcome: TerminalOutcomeV1,
    pub metrics: RunMetricsV1,
    pub starting_campaign_score: i32,
    pub final_campaign_score: i32,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u16,
    pub named_participant_instance_count: u16,
    pub anonymous_participant_instance_count: u16,
    pub campaign_complete_evidence: Option<PublicCampaignCompleteEvidenceV1>,
    pub achievements: Vec<PublicAchievementDecisionV1>,
}

impl PublicVerificationProofV1 {
    pub fn from_private(
        request: &VerificationRequestV1,
        result: &VerificationResultV1,
    ) -> Result<Self, ValidationError> {
        let public_request = PublicVerificationRequestV1::from_private(request, result)?;
        let public_request_sha256 =
            public_request
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "public_verification_proof.public_request",
                })?;
        let VerificationStatusV1::Verified(verified) = &result.status else {
            return Err(ValidationError::ClaimMismatch {
                field: "public_verification_proof.private_result.status",
            });
        };
        let campaign_complete_evidence = verified
            .campaign_complete_evidence
            .as_ref()
            .map(|private| {
                if private.verification_request_sha256 != result.verification_request_sha256
                    || private.replay_sha256 != result.artifacts.replay.artifact.sha256
                    || private.final_campaign_sha256 != verified.final_campaign.sha256
                    || private.final_state_sha256 != verified.final_state_sha256
                    || private.campaign_content_manifest_sha256
                        != public_request.campaign_content_manifest_sha256.ok_or(
                            ValidationError::ClaimMismatch {
                                field: "public_verification_proof.completion.scope",
                            },
                        )?
                    || private.content_manifest_sha256 != public_request.content_manifest_sha256
                    || private.rules_config_sha256 != public_request.rules_config_sha256
                    || private.ruleset_manifest_sha256 != public_request.ruleset_manifest_sha256
                    || private.terminal_subject != public_request.content_subject
                {
                    return Err(ValidationError::ClaimMismatch {
                        field: "public_verification_proof.completion.private_binding",
                    });
                }
                Ok(PublicCampaignCompleteEvidenceV1 {
                    schema_version: crate::SCHEMA_VERSION_V1,
                    campaign_content_manifest_sha256: private.campaign_content_manifest_sha256,
                    content_manifest_sha256: private.content_manifest_sha256,
                    rules_config_sha256: private.rules_config_sha256,
                    ruleset_manifest_sha256: private.ruleset_manifest_sha256,
                    public_verification_request_sha256: public_request_sha256,
                    terminal_subject: private.terminal_subject.clone(),
                    final_campaign: verified.final_campaign.clone(),
                    final_state_sha256: verified.final_state_sha256,
                    observed_progression_percent: private.observed_progression_percent,
                })
            })
            .transpose()?;
        let proof = Self {
            schema_version: crate::SCHEMA_VERSION_V1,
            public_request,
            public_request_sha256,
            input_provenance: result.input_provenance.clone().ok_or(
                ValidationError::ClaimMismatch {
                    field: "public_verification_proof.private_result.input_provenance",
                },
            )?,
            campaign_session_kind: verified.campaign_session_kind.clone(),
            campaign_session_ordinal: verified.campaign_session_ordinal,
            starting_campaign: verified.starting_campaign.clone(),
            final_campaign: verified.final_campaign.clone(),
            final_state_sha256: verified.final_state_sha256,
            replay_frame_count: verified.replay_frames,
            outcome: verified.outcome,
            metrics: verified.metrics(),
            starting_campaign_score: verified.starting_campaign_score,
            final_campaign_score: verified.final_campaign_score,
            max_concurrent_players: verified.max_concurrent_players,
            participant_instance_count: verified.participant_instance_count,
            named_participant_instance_count: verified.named_participant_instance_count,
            anonymous_participant_instance_count: verified.anonymous_participant_instance_count,
            campaign_complete_evidence,
            achievements: verified
                .achievements
                .iter()
                .map(PublicAchievementDecisionV1::from_private)
                .collect(),
        };
        proof.validate()?;
        Ok(proof)
    }
}

impl Validate for PublicVerificationProofV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("PublicVerificationProofV1", self.schema_version)?;
        for digest in [self.public_request_sha256, self.final_state_sha256] {
            if digest.is_zero() {
                return Err(ValidationError::Zero {
                    field: "public_verification_proof.digest",
                });
            }
        }
        self.public_request.validate()?;
        if self
            .public_request
            .canonical_digest()
            .map_err(|_| ValidationError::ClaimMismatch {
                field: "public_verification_proof.public_request",
            })?
            != self.public_request_sha256
        {
            return Err(ValidationError::ClaimMismatch {
                field: "public_verification_proof.public_request_sha256",
            });
        }
        for campaign in [&self.starting_campaign, &self.final_campaign] {
            campaign.validate()?;
            if campaign.media_type != crate::RANKED_CAMPAIGN_MEDIA_TYPE_V1 {
                return Err(ValidationError::ClaimMismatch {
                    field: "public_verification_proof.campaign.media_type",
                });
            }
        }
        if self.replay_frame_count == 0
            || self.max_concurrent_players == 0
            || self.participant_instance_count < self.max_concurrent_players
            || self
                .named_participant_instance_count
                .checked_add(self.anonymous_participant_instance_count)
                != Some(self.participant_instance_count)
        {
            return Err(ValidationError::ClaimMismatch {
                field: "public_verification_proof.count_or_optional_digest",
            });
        }
        if self.max_concurrent_players != self.public_request.max_concurrent_players
            || self.participant_instance_count != self.public_request.participant_instance_count
            || self.named_participant_instance_count
                != self.public_request.named_participant_instance_count
            || self.anonymous_participant_instance_count
                != self.public_request.anonymous_participant_instance_count
        {
            return Err(ValidationError::ClaimMismatch {
                field: "public_verification_proof.public_request_counts",
            });
        }
        match (
            self.public_request.scope_kind,
            self.public_request.campaign_aggregation_consent,
            &self.campaign_session_kind,
            self.campaign_session_ordinal,
        ) {
            (
                RunScopeKindV1::IndividualLevel,
                CampaignAggregationConsentV1::NotAuthorized,
                None,
                None,
            ) => {}
            (
                RunScopeKindV1::Campaign,
                CampaignAggregationConsentV1::AuthorizeSignedSessionInServerRecognizedChainV1,
                Some(kind),
                Some(ordinal),
            ) if ordinal < crate::MAX_CAMPAIGN_SESSIONS_V1 => kind.validate()?,
            _ => {
                return Err(ValidationError::ClaimMismatch {
                    field: "public_verification_proof.campaign_session",
                });
            }
        }
        if let Some(evidence) = &self.campaign_complete_evidence {
            evidence.validate()?;
            if self.public_request.scope_kind != RunScopeKindV1::Campaign
                || evidence.public_verification_request_sha256 != self.public_request_sha256
                || evidence.campaign_content_manifest_sha256
                    != self.public_request.campaign_content_manifest_sha256.ok_or(
                        ValidationError::ClaimMismatch {
                            field: "public_verification_proof.completion.scope",
                        },
                    )?
                || evidence.content_manifest_sha256 != self.public_request.content_manifest_sha256
                || evidence.rules_config_sha256 != self.public_request.rules_config_sha256
                || evidence.ruleset_manifest_sha256 != self.public_request.ruleset_manifest_sha256
                || evidence.terminal_subject != self.public_request.content_subject
                || evidence.final_campaign != self.final_campaign
                || evidence.final_state_sha256 != self.final_state_sha256
            {
                return Err(ValidationError::ClaimMismatch {
                    field: "public_verification_proof.campaign_complete_evidence",
                });
            }
        }
        self.input_provenance.validate()?;
        if !self.input_provenance.is_rankable() || self.outcome != TerminalOutcomeV1::Won {
            return Err(ValidationError::VerifiedRunNotRankable);
        }
        for achievement in &self.achievements {
            achievement.validate()?;
        }
        if self.achievements.len() > 256
            || !self
                .achievements
                .windows(2)
                .all(|pair| pair[0].achievement_id < pair[1].achievement_id)
        {
            return Err(ValidationError::NotCanonicalOrder {
                field: "public_verification_proof.achievements",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicCampaignSessionBindingV1 {
    pub ordinal: u32,
    pub run_id: OpaqueId,
    pub public_verification_request_sha256: Digest32,
    pub public_verification_result_sha256: Digest32,
}

/// Canonical public request for a full-campaign reduction. Private aggregate
/// request/result digests, chain locators, controller keys, all-seat
/// identities, and verifier-only campaign authority never enter this document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicCampaignAggregateRequestV1 {
    pub schema_version: u32,
    pub full_campaign_run_id: OpaqueId,
    pub campaign_complete_terminal_run_id: OpaqueId,
    pub public_campaign_complete_evidence_sha256: Digest32,
    pub sessions: Vec<PublicCampaignSessionBindingV1>,
    pub campaign_content_manifest_sha256: Digest32,
    pub rules_config_sha256: Digest32,
    pub ruleset_manifest_sha256: Digest32,
    pub competition_manifest_sha256: Option<Digest32>,
}

impl Validate for PublicCampaignAggregateRequestV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("PublicCampaignAggregateRequestV1", self.schema_version)?;
        for digest in [
            self.public_campaign_complete_evidence_sha256,
            self.campaign_content_manifest_sha256,
            self.rules_config_sha256,
            self.ruleset_manifest_sha256,
        ] {
            if digest.is_zero() {
                return Err(ValidationError::Zero {
                    field: "public_campaign_aggregate_request.digest",
                });
            }
        }
        if self.sessions.is_empty()
            || self.sessions.len() > 4_096
            || self
                .competition_manifest_sha256
                .is_some_and(|digest| digest.is_zero())
            || self.sessions.iter().enumerate().any(|(ordinal, session)| {
                session.ordinal != ordinal as u32
                    || session.public_verification_request_sha256.is_zero()
                    || session.public_verification_result_sha256.is_zero()
            })
            || self
                .sessions
                .iter()
                .map(|session| &session.run_id)
                .collect::<BTreeSet<_>>()
                .len()
                != self.sessions.len()
            || self
                .sessions
                .last()
                .is_none_or(|session| session.run_id != self.campaign_complete_terminal_run_id)
        {
            return Err(ValidationError::ClaimMismatch {
                field: "public_campaign_aggregate_request.structure",
            });
        }
        Ok(())
    }
}

/// Redacted public proof of a private full-campaign reduction. Its canonical
/// digest is the public aggregate result identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicCampaignAggregateProofV1 {
    pub schema_version: u32,
    pub public_request: PublicCampaignAggregateRequestV1,
    pub public_request_sha256: Digest32,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u32,
    pub named_participant_instance_count: u32,
    pub anonymous_participant_instance_count: u32,
    pub canonical_genesis_campaign: crate::ArtifactRefV1,
    pub final_campaign: crate::ArtifactRefV1,
    pub starting_campaign_score: i32,
    pub final_campaign_score: i32,
    pub metrics: RunMetricsV1,
}

impl PublicCampaignAggregateProofV1 {
    pub fn from_private(
        aggregate: &VerifiedCampaignAggregateV1,
        session_proofs: &[PublicVerificationProofV1],
    ) -> Result<Self, ValidationError> {
        aggregate.validate()?;
        if aggregate.sessions.len() != session_proofs.len() {
            return Err(ValidationError::ClaimMismatch {
                field: "public_campaign_aggregate.session_proofs",
            });
        }
        let mut sessions = Vec::with_capacity(aggregate.sessions.len());
        for (session, proof) in aggregate.sessions.iter().zip(session_proofs) {
            validate_public_proof_against_campaign_session(proof, session)?;
            sessions.push(PublicCampaignSessionBindingV1 {
                ordinal: session.ordinal,
                run_id: session.run_id.clone(),
                public_verification_request_sha256: proof.public_request_sha256,
                public_verification_result_sha256: proof.canonical_digest().map_err(|_| {
                    ValidationError::ClaimMismatch {
                        field: "public_campaign_aggregate.session_proof",
                    }
                })?,
            });
        }
        let terminal_evidence = session_proofs
            .last()
            .and_then(|proof| proof.campaign_complete_evidence.as_ref())
            .ok_or(ValidationError::ClaimMismatch {
                field: "public_campaign_aggregate.terminal_completion",
            })?;
        let public_request = PublicCampaignAggregateRequestV1 {
            schema_version: crate::SCHEMA_VERSION_V1,
            full_campaign_run_id: aggregate.full_campaign_run_id.clone(),
            campaign_complete_terminal_run_id: aggregate.campaign_complete_terminal_run_id.clone(),
            public_campaign_complete_evidence_sha256: terminal_evidence
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "public_campaign_aggregate.terminal_completion",
                })?,
            sessions,
            campaign_content_manifest_sha256: aggregate.campaign_content_manifest_sha256,
            rules_config_sha256: aggregate.rules_config_sha256,
            ruleset_manifest_sha256: aggregate.ruleset_manifest_sha256,
            competition_manifest_sha256: aggregate.competition_manifest_sha256,
        };
        public_request.validate()?;
        let proof = Self {
            schema_version: crate::SCHEMA_VERSION_V1,
            public_request_sha256: public_request.canonical_digest().map_err(|_| {
                ValidationError::ClaimMismatch {
                    field: "public_campaign_aggregate.public_request",
                }
            })?,
            public_request,
            max_concurrent_players: aggregate.max_concurrent_players,
            participant_instance_count: aggregate.participant_instance_count,
            named_participant_instance_count: aggregate.named_participant_instance_count,
            anonymous_participant_instance_count: aggregate.anonymous_participant_instance_count,
            canonical_genesis_campaign: aggregate.canonical_genesis_campaign.clone(),
            final_campaign: aggregate.final_campaign.clone(),
            starting_campaign_score: aggregate.starting_campaign_score,
            final_campaign_score: aggregate.final_campaign_score,
            metrics: aggregate.metrics(),
        };
        proof.validate()?;
        Ok(proof)
    }

    fn validate_against_ruleset(
        &self,
        published: &PublishedRulesetV1,
        campaign_content: &CampaignContentManifestV1,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        published.validate()?;
        campaign_content.validate()?;
        if self.public_request.ruleset_manifest_sha256 != published.ruleset_manifest_sha256
            || !published
                .manifest
                .admits_rules_config_digest(self.public_request.rules_config_sha256)
            || published
                .manifest
                .allowed_campaign_content_manifest_sha256
                .binary_search(&self.public_request.campaign_content_manifest_sha256)
                .is_err()
            || campaign_content
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "public_campaign_aggregate.campaign_content_manifest",
                })?
                != self.public_request.campaign_content_manifest_sha256
            || published
                .manifest
                .board_scopes
                .binary_search(&RulesetBoardScopeV1::FullCampaign)
                .is_err()
        {
            return Err(ValidationError::ClaimMismatch {
                field: "public_campaign_aggregate.ruleset_tuple",
            });
        }
        Ok(())
    }
}

impl Validate for PublicCampaignAggregateProofV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("PublicCampaignAggregateProofV1", self.schema_version)?;
        self.public_request.validate()?;
        if self.public_request_sha256.is_zero()
            || self.public_request.canonical_digest().map_err(|_| {
                ValidationError::ClaimMismatch {
                    field: "public_campaign_aggregate.public_request",
                }
            })? != self.public_request_sha256
        {
            return Err(ValidationError::ClaimMismatch {
                field: "public_campaign_aggregate.public_request_sha256",
            });
        }
        for campaign in [&self.canonical_genesis_campaign, &self.final_campaign] {
            campaign.validate()?;
            if campaign.media_type != crate::RANKED_CAMPAIGN_MEDIA_TYPE_V1 {
                return Err(ValidationError::ClaimMismatch {
                    field: "public_campaign_aggregate.campaign.media_type",
                });
            }
        }
        if self.max_concurrent_players == 0
            || self.participant_instance_count < u32::from(self.max_concurrent_players)
            || self
                .named_participant_instance_count
                .checked_add(self.anonymous_participant_instance_count)
                != Some(self.participant_instance_count)
        {
            return Err(ValidationError::ClaimMismatch {
                field: "public_campaign_aggregate.structure",
            });
        }
        if i64::from(self.final_campaign_score) - i64::from(self.starting_campaign_score)
            != self.metrics.original_score_delta
        {
            return Err(ValidationError::InvalidOriginalScore);
        }
        Ok(())
    }
}

fn validate_public_proof_against_campaign_session(
    proof: &PublicVerificationProofV1,
    session: &VerifiedCampaignSessionV1,
) -> Result<(), ValidationError> {
    proof.validate()?;
    session.validate()?;
    let completion_present = proof.campaign_complete_evidence.is_some();
    if proof.public_request.scope_kind != RunScopeKindV1::Campaign
        || proof.public_request.campaign_aggregation_consent
            != CampaignAggregationConsentV1::AuthorizeSignedSessionInServerRecognizedChainV1
        || proof.campaign_session_kind.as_ref() != Some(&session.kind)
        || proof.campaign_session_ordinal != Some(session.ordinal)
        || proof.public_request.replay != session.replay
        || proof.public_request.build_manifest_sha256 != session.build_manifest_sha256
        || proof.public_request.content_manifest_sha256 != session.content_manifest_sha256
        || proof.public_request.rules_config_sha256 != session.rules_config_sha256
        || proof.public_request.ruleset_manifest_sha256 != session.ruleset_manifest_sha256
        || proof.public_request.competition_manifest_sha256 != session.competition_manifest_sha256
        || proof.starting_campaign != session.starting_campaign
        || proof.final_campaign != session.final_campaign
        || proof.starting_campaign_score != session.starting_campaign_score
        || proof.final_campaign_score != session.final_campaign_score
        || proof.max_concurrent_players != session.max_concurrent_players
        || proof.participant_instance_count != session.participant_instance_count
        || proof.named_participant_instance_count != session.named_participant_instance_count
        || proof.anonymous_participant_instance_count
            != session.anonymous_participant_instance_count
        || proof.metrics.original_score_delta != session.score_delta()
        || proof.metrics.active_simulation_ticks != session.active_simulation_ticks
        || proof.metrics.ransom_collected != session.ransom_collected
        || completion_present != session.campaign_complete_evidence_sha256.is_some()
    {
        return Err(ValidationError::ClaimMismatch {
            field: "public_campaign_aggregate.session_proof",
        });
    }
    Ok(())
}

/// One independently verified replay contributing to a full-campaign result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FullCampaignSessionV1 {
    /// Zero-based position in the aggregate's gap-free ordered session list.
    pub ordinal: u32,
    pub run_id: OpaqueId,
    pub session: FullCampaignSessionKindV1,
    /// Stable simulation-content subject, including the exact engine mission
    /// identity used for HQ sessions.
    pub content_subject: OfficialContentSubjectV1,
    pub display_name: String,
    /// Present exactly for field missions. HQ sessions remain first-class
    /// verified sessions without pretending to be mission leaderboard runs.
    pub mission: Option<MissionFacetV1>,
    pub replay: ReplayArtifactV1,
    pub public_verification_request_sha256: Digest32,
    pub public_verification_result_sha256: Digest32,
    pub verification_proof: PublicVerificationProofV1,
    pub starting_campaign: crate::ArtifactRefV1,
    pub final_campaign: crate::ArtifactRefV1,
    pub starting_campaign_score: i32,
    pub final_campaign_score: i32,
    pub public_campaign_complete_evidence_sha256: Option<Digest32>,
    pub content_manifest_sha256: Digest32,
    pub rules_config_sha256: Digest32,
    pub ruleset_manifest_sha256: Digest32,
    pub competition_manifest_sha256: Option<Digest32>,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u16,
    pub named_participant_instance_count: u16,
    pub anonymous_participant_instance_count: u16,
    /// Only participants who explicitly selected NamedProfile disclosure.
    pub named_participants: Vec<PublicParticipantV1>,
    pub input_provenance: InputProvenanceStatusV1,
    pub metrics: RunMetricsV1,
    pub achievements: Vec<AchievementSummaryV1>,
    pub build: PublicBuildV1,
    pub viewer: ViewerLaunchV1,
}

impl FullCampaignSessionV1 {
    fn validate(
        &self,
        rules_config_sha256: Digest32,
        ruleset_manifest_sha256: Digest32,
    ) -> Result<(), ValidationError> {
        self.session.validate()?;
        self.content_subject.validate()?;
        crate::validation::text(
            "full_campaign_session.display_name",
            &self.display_name,
            100,
        )?;
        match (&self.session, &self.content_subject, &self.mission) {
            (
                FullCampaignSessionKindV1::FieldMission { mission_id },
                OfficialContentSubjectV1::FieldMission {
                    mission_id: content_mission_id,
                },
                Some(mission),
            ) => {
                mission.validate()?;
                if mission.mission_id != *mission_id
                    || content_mission_id != mission_id
                    || mission.content_manifest_sha256 != self.content_manifest_sha256
                {
                    return Err(ValidationError::ClaimMismatch {
                        field: "full_campaign_session.mission",
                    });
                }
            }
            (
                FullCampaignSessionKindV1::Headquarters { .. },
                OfficialContentSubjectV1::Headquarters { .. },
                None,
            ) => {}
            _ => {
                return Err(ValidationError::ClaimMismatch {
                    field: "full_campaign_session.kind/mission",
                });
            }
        }
        self.replay.validate()?;
        for campaign in [&self.starting_campaign, &self.final_campaign] {
            campaign.validate()?;
            if campaign.media_type != crate::RANKED_CAMPAIGN_MEDIA_TYPE_V1 {
                return Err(ValidationError::ClaimMismatch {
                    field: "full_campaign_session.campaign.media_type",
                });
            }
        }
        for digest in [
            self.public_verification_request_sha256,
            self.public_verification_result_sha256,
            self.content_manifest_sha256,
            self.rules_config_sha256,
            self.ruleset_manifest_sha256,
        ] {
            if digest.is_zero() {
                return Err(ValidationError::Zero {
                    field: "full_campaign_session.proof_digest",
                });
            }
        }
        if self
            .competition_manifest_sha256
            .is_some_and(|digest| digest.is_zero())
        {
            return Err(ValidationError::Zero {
                field: "full_campaign_session.competition_manifest_sha256",
            });
        }
        if self.rules_config_sha256 != rules_config_sha256
            || self.ruleset_manifest_sha256 != ruleset_manifest_sha256
        {
            return Err(ValidationError::ClaimMismatch {
                field: "full_campaign_session.chain_identity",
            });
        }
        let wrapped_mission_score = i64::from(
            self.final_campaign_score
                .wrapping_sub(self.starting_campaign_score) as u32,
        );
        if self.metrics.original_score_delta != wrapped_mission_score {
            return Err(ValidationError::InvalidOriginalScore);
        }
        self.input_provenance.validate()?;
        if !self.input_provenance.is_rankable() {
            return Err(ValidationError::VerifiedRunNotRankable);
        }
        for achievement in &self.achievements {
            achievement.validate()?;
        }
        self.verification_proof.validate()?;
        validate_public_roster(
            self.max_concurrent_players,
            u32::from(self.participant_instance_count),
            u32::from(self.named_participant_instance_count),
            &self.named_participants,
            &[],
            u32::from(self.anonymous_participant_instance_count),
            true,
        )?;
        let kind_matches = match (
            &self.verification_proof.campaign_session_kind,
            &self.session,
        ) {
            (
                Some(CampaignSessionKindV1::FieldMission { mission_id }),
                FullCampaignSessionKindV1::FieldMission {
                    mission_id: public_mission_id,
                },
            ) => mission_id == public_mission_id,
            (
                Some(CampaignSessionKindV1::Headquarters { hq_sequence }),
                FullCampaignSessionKindV1::Headquarters {
                    hq_sequence: public_hq_sequence,
                },
            ) => hq_sequence == public_hq_sequence,
            _ => false,
        };
        let proof_result_sha256 = self.verification_proof.canonical_digest().map_err(|_| {
            ValidationError::ClaimMismatch {
                field: "full_campaign_session.verification_proof_digest",
            }
        })?;
        let proof_evidence_sha256 = self
            .verification_proof
            .campaign_complete_evidence
            .as_ref()
            .map(|evidence| evidence.canonical_digest())
            .transpose()
            .map_err(|_| ValidationError::ClaimMismatch {
                field: "full_campaign_session.completion_evidence_digest",
            })?;
        if proof_result_sha256 != self.public_verification_result_sha256
            || self.verification_proof.public_request_sha256
                != self.public_verification_request_sha256
            || self.verification_proof.public_request.replay != self.replay
            || self.verification_proof.public_request.build_manifest_sha256
                != self.build.manifest_sha256
            || self
                .verification_proof
                .public_request
                .content_manifest_sha256
                != self.content_manifest_sha256
            || self.verification_proof.public_request.rules_config_sha256
                != self.rules_config_sha256
            || self
                .verification_proof
                .public_request
                .ruleset_manifest_sha256
                != self.ruleset_manifest_sha256
            || self
                .verification_proof
                .public_request
                .competition_manifest_sha256
                != self.competition_manifest_sha256
            || self.verification_proof.input_provenance != self.input_provenance
            || self.verification_proof.public_request.scope_kind != RunScopeKindV1::Campaign
            || self
                .verification_proof
                .public_request
                .campaign_aggregation_consent
                != CampaignAggregationConsentV1::AuthorizeSignedSessionInServerRecognizedChainV1
            || !kind_matches
            || self.verification_proof.campaign_session_ordinal != Some(self.ordinal)
            || self.verification_proof.max_concurrent_players != self.max_concurrent_players
            || self.verification_proof.participant_instance_count != self.participant_instance_count
            || self.verification_proof.named_participant_instance_count
                != self.named_participant_instance_count
            || self.verification_proof.anonymous_participant_instance_count
                != self.anonymous_participant_instance_count
            || self.verification_proof.outcome != TerminalOutcomeV1::Won
            || self.verification_proof.starting_campaign != self.starting_campaign
            || self.verification_proof.final_campaign != self.final_campaign
            || self.verification_proof.starting_campaign_score != self.starting_campaign_score
            || self.verification_proof.final_campaign_score != self.final_campaign_score
            || proof_evidence_sha256 != self.public_campaign_complete_evidence_sha256
            || self.verification_proof.metrics != self.metrics
            || self
                .achievements
                .iter()
                .map(|achievement| &achievement.verified)
                .ne(self.verification_proof.achievements.iter())
        {
            return Err(ValidationError::ClaimMismatch {
                field: "full_campaign_session.verification_proof",
            });
        }
        if self
            .named_participants
            .iter()
            .map(|participant| (participant.seat, participant.public_key))
            .ne(self
                .verification_proof
                .public_request
                .named_participants
                .iter()
                .map(|participant| (participant.seat, participant.public_key)))
        {
            return Err(ValidationError::ClaimMismatch {
                field: "full_campaign_session.named_participants",
            });
        }
        self.build.validate()?;
        self.viewer.validate()?;
        if self.build.manifest_sha256 != self.viewer.build_manifest_sha256 {
            return Err(ValidationError::ClaimMismatch {
                field: "full_campaign_session.viewer.build_manifest_sha256",
            });
        }
        if let ViewerAvailabilityV1::Available {
            content_requirement,
        } = &self.viewer.availability
            && content_requirement.content_manifest_sha256() != self.content_manifest_sha256
        {
            return Err(ValidationError::ClaimMismatch {
                field: "full_campaign_session.viewer.content_manifest_sha256",
            });
        }
        Ok(())
    }
}

/// Aggregate-authorized detail returned by
/// `GET /api/v1/runs/{aggregate_run_id}/sessions/{ordinal}`. The associated
/// `/replay` endpoint returns exactly `session.replay` under the aggregate's
/// authorization and tombstone state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignSessionDetailV1 {
    pub schema_version: u32,
    pub aggregate_run_id: OpaqueId,
    pub public_aggregate_result_sha256: Digest32,
    pub ordinal: u32,
    pub session: FullCampaignSessionV1,
}

impl CampaignSessionDetailV1 {
    pub fn validate_against_aggregate(
        &self,
        aggregate: &RunDetailV1,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        aggregate.validate()?;
        if self.aggregate_run_id != aggregate.run_id
            || self.public_aggregate_result_sha256 != aggregate.public_result_sha256
            || aggregate.subject != LeaderboardSubjectV1::FullCampaign
            || aggregate.full_campaign_sessions.get(self.ordinal as usize) != Some(&self.session)
        {
            return Err(ValidationError::ClaimMismatch {
                field: "campaign_session_detail.aggregate",
            });
        }
        Ok(())
    }

    /// Validates both the ordinal aggregate binding and the immutable
    /// ruleset semantics used to classify the campaign.
    pub fn validate_against_ruleset(
        &self,
        aggregate: &RunDetailV1,
        published: &PublishedRulesetV1,
        campaign_content: &CampaignContentManifestV1,
    ) -> Result<(), ValidationError> {
        self.validate_against_aggregate(aggregate)?;
        aggregate.validate_against_ruleset(published, Some(campaign_content))
    }
}

impl Validate for CampaignSessionDetailV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("CampaignSessionDetailV1", self.schema_version)?;
        if self.public_aggregate_result_sha256.is_zero() || self.ordinal != self.session.ordinal {
            return Err(ValidationError::ClaimMismatch {
                field: "campaign_session_detail.proof",
            });
        }
        self.session.validate(
            self.session.rules_config_sha256,
            self.session.ruleset_manifest_sha256,
        )
    }
}

/// Public, verifier-derived run view. The viewer availability and trust text
/// are presentation facts, not claims that an official binary produced input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunDetailV1 {
    pub schema_version: u32,
    pub run_id: OpaqueId,
    pub rank: Option<u64>,
    pub subject: LeaderboardSubjectV1,
    pub mission: Option<MissionFacetV1>,
    pub composition: VerifiedRunCompositionV1,
    pub outcome: TerminalOutcomeV1,
    pub metrics: RunMetricsV1,
    pub metric_value: BoardMetricValueV1,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u32,
    pub named_participant_instance_count: u32,
    pub named_participants: Vec<PublicParticipantV1>,
    pub aggregate_named_participants: Vec<AggregatePublicParticipantV1>,
    pub anonymous_participant_instance_count: u32,
    pub verified_at_unix_ms: u64,
    /// Canonical digest of the embedded public request projection.
    pub public_request_sha256: Digest32,
    /// Canonical digest of the embedded public proof projection.
    pub public_result_sha256: Digest32,
    pub verification_proof: Option<PublicVerificationProofV1>,
    pub campaign_aggregate: Option<PublicCampaignAggregateProofV1>,
    /// Exact verified artifact for standalone mission runs. Full-campaign
    /// aggregates have no synthetic replay; their session entries each carry
    /// their own artifact.
    pub replay: Option<ReplayArtifactV1>,
    pub content: RunContentIdentityV1,
    /// Campaign catalog identity for a campaign-mission chain link. Individual
    /// levels carry `None`; full-campaign details bind it through `content`.
    pub campaign_content_manifest_sha256: Option<Digest32>,
    pub rules_config_sha256: Digest32,
    pub ruleset_manifest_sha256: Digest32,
    pub competition_manifest_sha256: Option<Digest32>,
    pub starting_campaign: crate::ArtifactRefV1,
    pub final_campaign: crate::ArtifactRefV1,
    pub starting_campaign_score: i32,
    pub final_campaign_score: i32,
    pub input_provenance: InputProvenanceStatusV1,
    pub build: Option<PublicBuildV1>,
    pub achievements: Vec<AchievementSummaryV1>,
    pub viewer: Option<ViewerLaunchV1>,
    pub full_campaign_sessions: Vec<FullCampaignSessionV1>,
    pub trust_statement: String,
}

impl Validate for RunDetailV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("RunDetailV1", self.schema_version)?;
        if self.rank == Some(0) {
            return Err(ValidationError::Zero { field: "run.rank" });
        }
        if self.verified_at_unix_ms == 0 {
            return Err(ValidationError::Zero {
                field: "run.verified_at_unix_ms",
            });
        }
        validate_subject_composition(&self.subject, &self.composition)?;
        self.content.validate_for_subject(&self.subject)?;
        let expects_campaign_catalog = matches!(
            self.subject,
            LeaderboardSubjectV1::Mission {
                category: BoardCategoryV1::Campaign,
                ..
            }
        );
        if expects_campaign_catalog != self.campaign_content_manifest_sha256.is_some()
            || self
                .campaign_content_manifest_sha256
                .is_some_and(|digest| digest.is_zero())
        {
            return Err(ValidationError::ClaimMismatch {
                field: "run.campaign_content_manifest_sha256",
            });
        }
        self.metric_value.validate()?;
        if self.outcome != TerminalOutcomeV1::Won {
            return Err(ValidationError::ClaimMismatch {
                field: "run.outcome",
            });
        }
        match &self.metric_value {
            BoardMetricValueV1::OriginalScore { points }
                if *points != self.metrics.original_score_delta =>
            {
                return Err(ValidationError::MetricValueMismatch {
                    field: "run.metric_value.original_score",
                });
            }
            BoardMetricValueV1::FastestSuccess {
                active_simulation_ticks,
                ..
            } if *active_simulation_ticks != self.metrics.active_simulation_ticks => {
                return Err(ValidationError::MetricValueMismatch {
                    field: "run.metric_value.active_simulation_ticks",
                });
            }
            BoardMetricValueV1::OriginalScore { .. }
            | BoardMetricValueV1::FastestSuccess { .. } => {}
        }
        match (
            &self.subject,
            &self.mission,
            &self.composition,
            &self.replay,
            &self.build,
            &self.viewer,
            self.full_campaign_sessions.as_slice(),
            &self.verification_proof,
            &self.campaign_aggregate,
        ) {
            (
                LeaderboardSubjectV1::Mission { mission_id, .. },
                Some(mission),
                VerifiedRunCompositionV1::Mission { replay_sha256 },
                Some(replay),
                Some(build),
                Some(viewer),
                [],
                Some(verification_proof),
                None,
            ) => {
                mission.validate()?;
                replay.validate()?;
                build.validate()?;
                viewer.validate()?;
                if mission.mission_id != *mission_id
                    || mission.content_manifest_sha256 != self.content.digest()
                    || *replay_sha256 != replay.artifact.sha256
                    || build.manifest_sha256 != viewer.build_manifest_sha256
                    || matches!(
                        &viewer.availability,
                        ViewerAvailabilityV1::Available { content_requirement }
                            if content_requirement.content_manifest_sha256()
                                != self.content.digest()
                    )
                {
                    return Err(ValidationError::ClaimMismatch {
                        field: "run.mission_artifact_proof",
                    });
                }
                self.validate_mission_verification_proof(
                    verification_proof,
                    replay,
                    build.manifest_sha256,
                )?;
            }
            (
                LeaderboardSubjectV1::FullCampaign,
                None,
                VerifiedRunCompositionV1::FullCampaign {
                    ordered_session_run_ids,
                    ..
                },
                None,
                None,
                None,
                sessions,
                None,
                Some(aggregate),
            ) if sessions
                .iter()
                .map(|session| &session.run_id)
                .eq(ordered_session_run_ids.iter()) =>
            {
                self.validate_campaign_aggregate(aggregate, ordered_session_run_ids)?;
                if !self.achievements.is_empty() {
                    return Err(ValidationError::ClaimMismatch {
                        field: "run.full_campaign_achievements",
                    });
                }
                let mut next_hq_sequence = 1_u32;
                for (ordinal, session) in sessions.iter().enumerate() {
                    if session.ordinal != ordinal as u32 {
                        return Err(ValidationError::ClaimMismatch {
                            field: "run.full_campaign_session.ordinal",
                        });
                    }
                    if let FullCampaignSessionKindV1::Headquarters { hq_sequence } = session.session
                    {
                        if hq_sequence != next_hq_sequence {
                            return Err(ValidationError::ClaimMismatch {
                                field: "run.full_campaign_session.hq_sequence",
                            });
                        }
                        next_hq_sequence = next_hq_sequence.checked_add(1).ok_or(
                            ValidationError::CountOutOfRange {
                                field: "run.full_campaign_session.hq_sequence",
                            },
                        )?;
                    }
                    session.validate(self.rules_config_sha256, self.ruleset_manifest_sha256)?;
                }
                if sessions.first().is_none_or(|session| {
                    session.starting_campaign != self.starting_campaign
                        || session.starting_campaign_score != self.starting_campaign_score
                }) || sessions.last().is_none_or(|session| {
                    session.final_campaign != self.final_campaign
                        || session.final_campaign_score != self.final_campaign_score
                }) || sessions
                    .windows(2)
                    .any(|pair| pair[0].final_campaign_score != pair[1].starting_campaign_score)
                {
                    return Err(ValidationError::ClaimMismatch {
                        field: "run.full_campaign_chain_continuity",
                    });
                }
                let score_delta =
                    i64::from(self.final_campaign_score) - i64::from(self.starting_campaign_score);
                let summed_score = sessions.iter().try_fold(0_i64, |sum, session| {
                    sum.checked_add(
                        i64::from(session.final_campaign_score)
                            - i64::from(session.starting_campaign_score),
                    )
                });
                let summed_ticks = sessions.iter().try_fold(0_u64, |sum, session| {
                    sum.checked_add(session.metrics.active_simulation_ticks)
                });
                let summed_ransom = sessions.iter().try_fold(0_u64, |sum, session| {
                    sum.checked_add(session.metrics.ransom_collected)
                });
                if self.metrics.original_score_delta != score_delta
                    || score_delta < 0
                    || summed_score != Some(score_delta)
                    || summed_ticks != Some(self.metrics.active_simulation_ticks)
                    || summed_ransom != Some(self.metrics.ransom_collected)
                {
                    return Err(ValidationError::ClaimMismatch {
                        field: "run.full_campaign_aggregate_metrics",
                    });
                }
            }
            _ => {
                return Err(ValidationError::ClaimMismatch {
                    field: "run.subject_artifacts",
                });
            }
        }
        validate_public_roster(
            self.max_concurrent_players,
            self.participant_instance_count,
            self.named_participant_instance_count,
            &self.named_participants,
            &self.aggregate_named_participants,
            self.anonymous_participant_instance_count,
            matches!(self.subject, LeaderboardSubjectV1::Mission { .. }),
        )?;
        self.input_provenance.validate()?;
        if !self.input_provenance.is_rankable() {
            return Err(ValidationError::VerifiedRunNotRankable);
        }
        for achievement in &self.achievements {
            achievement.validate()?;
        }
        for digest in [
            self.public_request_sha256,
            self.public_result_sha256,
            self.rules_config_sha256,
            self.ruleset_manifest_sha256,
        ] {
            if digest.is_zero() {
                return Err(ValidationError::Zero {
                    field: "run.proof_digest",
                });
            }
        }
        for campaign in [&self.starting_campaign, &self.final_campaign] {
            campaign.validate()?;
            if campaign.media_type != crate::RANKED_CAMPAIGN_MEDIA_TYPE_V1 {
                return Err(ValidationError::ClaimMismatch {
                    field: "run.public_campaign.media_type",
                });
            }
        }
        if self
            .competition_manifest_sha256
            .is_some_and(|digest| digest.is_zero())
        {
            return Err(ValidationError::Zero {
                field: "run.competition_manifest_sha256",
            });
        }
        match self.subject {
            LeaderboardSubjectV1::Mission { .. } => {
                let wrapped_mission_score = i64::from(
                    self.final_campaign_score
                        .wrapping_sub(self.starting_campaign_score) as u32,
                );
                if self.metrics.original_score_delta != wrapped_mission_score {
                    return Err(ValidationError::InvalidOriginalScore);
                }
            }
            LeaderboardSubjectV1::FullCampaign => {
                let signed_campaign_delta =
                    i64::from(self.final_campaign_score) - i64::from(self.starting_campaign_score);
                if signed_campaign_delta < 0
                    || self.metrics.original_score_delta != signed_campaign_delta
                {
                    return Err(ValidationError::InvalidOriginalScore);
                }
            }
        }
        crate::validation::text("run.trust_statement", &self.trust_statement, 1000)
    }
}

impl RunDetailV1 {
    /// Applies immutable rankability semantics after the self-contained proof
    /// checks. Services must call this before publishing a run under a board.
    pub fn validate_against_ruleset(
        &self,
        published: &PublishedRulesetV1,
        campaign_content: Option<&CampaignContentManifestV1>,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        published.validate()?;
        let manifest = &published.manifest;
        let scope = match self.subject {
            LeaderboardSubjectV1::Mission {
                category: BoardCategoryV1::IndividualLevel,
                ..
            } => RulesetBoardScopeV1::IndividualLevel,
            LeaderboardSubjectV1::Mission {
                category: BoardCategoryV1::Campaign,
                ..
            } => RulesetBoardScopeV1::CampaignMission,
            LeaderboardSubjectV1::FullCampaign => RulesetBoardScopeV1::FullCampaign,
        };
        if self.ruleset_manifest_sha256 != published.ruleset_manifest_sha256
            || !manifest.admits_rules_config_digest(self.rules_config_sha256)
            || match self.subject {
                LeaderboardSubjectV1::Mission { .. } => manifest
                    .allowed_content_manifest_sha256
                    .binary_search(&self.content.digest())
                    .is_err(),
                LeaderboardSubjectV1::FullCampaign => manifest
                    .allowed_campaign_content_manifest_sha256
                    .binary_search(&self.content.digest())
                    .is_err(),
            }
            || manifest.board_scopes.binary_search(&scope).is_err()
            || manifest
                .metrics
                .binary_search(&self.metric_value.metric())
                .is_err()
            || self.max_concurrent_players
                < manifest
                    .participant_eligibility
                    .minimum_max_concurrent_players
            || self.max_concurrent_players
                > manifest
                    .participant_eligibility
                    .maximum_max_concurrent_players
            || self.participant_instance_count
                > u32::from(
                    manifest
                        .participant_eligibility
                        .maximum_participant_instances,
                )
            || (!manifest.participant_eligibility.allow_single_player
                && self.max_concurrent_players == 1)
            || (!manifest.participant_eligibility.allow_multiplayer
                && self.max_concurrent_players > 1)
            || (manifest.participant_eligibility.anonymous_policy
                == crate::AnonymousParticipantPolicyV1::Forbidden
                && self.anonymous_participant_instance_count != 0)
        {
            return Err(ValidationError::ClaimMismatch {
                field: "run.ruleset_tuple",
            });
        }
        if let BoardMetricValueV1::FastestSuccess { tick_duration, .. } = &self.metric_value
            && tick_duration != &manifest.tick_duration
        {
            return Err(ValidationError::ClaimMismatch {
                field: "run.ruleset.tick_duration",
            });
        }
        if let Some(aggregate) = &self.campaign_aggregate {
            let campaign_content = campaign_content.ok_or(ValidationError::ClaimMismatch {
                field: "run.campaign_content_manifest",
            })?;
            aggregate.validate_against_ruleset(published, campaign_content)?;
            if self.full_campaign_sessions.iter().any(|session| {
                manifest
                    .allowed_build_manifest_sha256
                    .binary_search(&session.build.manifest_sha256)
                    .is_err()
                    || manifest
                        .replay_schema_versions
                        .binary_search(&session.replay.replay_schema_version)
                        .is_err()
                    || manifest
                        .allowed_content_manifest_sha256
                        .binary_search(&session.content_manifest_sha256)
                        .is_err()
                    || campaign_content.content_for(&session.content_subject)
                        != Some(session.content_manifest_sha256)
                    || session.max_concurrent_players
                        < manifest
                            .participant_eligibility
                            .minimum_max_concurrent_players
                    || session.max_concurrent_players
                        > manifest
                            .participant_eligibility
                            .maximum_max_concurrent_players
                    || session.participant_instance_count
                        > manifest
                            .participant_eligibility
                            .maximum_participant_instances
                    || (!manifest.participant_eligibility.allow_single_player
                        && session.max_concurrent_players == 1)
                    || (!manifest.participant_eligibility.allow_multiplayer
                        && session.max_concurrent_players > 1)
                    || (manifest.participant_eligibility.anonymous_policy
                        == crate::AnonymousParticipantPolicyV1::Forbidden
                        && session.anonymous_participant_instance_count != 0)
            }) {
                return Err(ValidationError::ClaimMismatch {
                    field: "run.full_campaign_session.ruleset_tuple",
                });
            }
            for session in &self.full_campaign_sessions {
                validate_public_achievement_summaries(manifest, &session.achievements)?;
            }
        } else if self.build.as_ref().is_none_or(|build| {
            manifest
                .allowed_build_manifest_sha256
                .binary_search(&build.manifest_sha256)
                .is_err()
        }) {
            return Err(ValidationError::ClaimMismatch {
                field: "run.ruleset.build_manifest_sha256",
            });
        } else {
            validate_public_achievement_summaries(manifest, &self.achievements)?;
        }
        Ok(())
    }

    fn public_participant_keys(&self) -> Vec<PublicKey32> {
        let mut keys = match self.subject {
            LeaderboardSubjectV1::Mission { .. } => self
                .named_participants
                .iter()
                .map(|participant| participant.public_key)
                .collect::<Vec<_>>(),
            LeaderboardSubjectV1::FullCampaign => self
                .aggregate_named_participants
                .iter()
                .map(|participant| participant.public_key)
                .collect::<Vec<_>>(),
        };
        keys.sort_unstable();
        keys
    }

    fn validate_mission_verification_proof(
        &self,
        proof: &PublicVerificationProofV1,
        replay: &ReplayArtifactV1,
        build_manifest_sha256: Digest32,
    ) -> Result<(), ValidationError> {
        proof.validate()?;
        let expected_scope = match self.subject {
            LeaderboardSubjectV1::Mission {
                category: BoardCategoryV1::IndividualLevel,
                ..
            } => RunScopeKindV1::IndividualLevel,
            LeaderboardSubjectV1::Mission {
                category: BoardCategoryV1::Campaign,
                ..
            } => RunScopeKindV1::Campaign,
            LeaderboardSubjectV1::FullCampaign => {
                return Err(ValidationError::ClaimMismatch {
                    field: "run.verification_result.subject",
                });
            }
        };
        let campaign_kind_matches = match (&self.subject, &proof.campaign_session_kind) {
            (
                LeaderboardSubjectV1::Mission {
                    mission_id,
                    category: BoardCategoryV1::Campaign,
                },
                Some(CampaignSessionKindV1::FieldMission {
                    mission_id: result_mission_id,
                }),
            ) => mission_id == result_mission_id,
            (
                LeaderboardSubjectV1::Mission {
                    category: BoardCategoryV1::IndividualLevel,
                    ..
                },
                None,
            ) => true,
            _ => false,
        };
        let public_result_sha256 =
            proof
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "run.public_verification_result",
                })?;
        if public_result_sha256 != self.public_result_sha256
            || proof.public_request_sha256 != self.public_request_sha256
            || &proof.public_request.replay != replay
            || proof.public_request.build_manifest_sha256 != build_manifest_sha256
            || proof.public_request.content_manifest_sha256 != self.content.digest()
            || proof.public_request.campaign_content_manifest_sha256
                != self.campaign_content_manifest_sha256
            || proof.public_request.rules_config_sha256 != self.rules_config_sha256
            || proof.public_request.ruleset_manifest_sha256 != self.ruleset_manifest_sha256
            || proof.public_request.competition_manifest_sha256 != self.competition_manifest_sha256
            || proof.input_provenance != self.input_provenance
            || proof.public_request.scope_kind != expected_scope
            || !campaign_kind_matches
            || proof.max_concurrent_players != self.max_concurrent_players
            || u32::from(proof.participant_instance_count) != self.participant_instance_count
            || u32::from(proof.named_participant_instance_count)
                != self.named_participant_instance_count
            || u32::from(proof.anonymous_participant_instance_count)
                != self.anonymous_participant_instance_count
            || proof.outcome != self.outcome
            || proof.starting_campaign != self.starting_campaign
            || proof.final_campaign != self.final_campaign
            || proof.starting_campaign_score != self.starting_campaign_score
            || proof.final_campaign_score != self.final_campaign_score
            || proof.metrics != self.metrics
            || self
                .achievements
                .iter()
                .map(|achievement| &achievement.verified)
                .ne(proof.achievements.iter())
        {
            return Err(ValidationError::ClaimMismatch {
                field: "run.verification_proof",
            });
        }
        if self
            .named_participants
            .iter()
            .map(|participant| (participant.seat, participant.public_key))
            .ne(proof
                .public_request
                .named_participants
                .iter()
                .map(|participant| (participant.seat, participant.public_key)))
        {
            return Err(ValidationError::ClaimMismatch {
                field: "run.verification_proof.named_participants",
            });
        }
        Ok(())
    }

    fn validate_campaign_aggregate(
        &self,
        aggregate: &PublicCampaignAggregateProofV1,
        ordered_session_run_ids: &[OpaqueId],
    ) -> Result<(), ValidationError> {
        aggregate.validate()?;
        let disclosed_session_key_union = self
            .full_campaign_sessions
            .iter()
            .flat_map(|session| {
                session
                    .named_participants
                    .iter()
                    .map(|participant| participant.public_key)
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let public_result_sha256 =
            aggregate
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "run.public_campaign_aggregate_result",
                })?;
        if public_result_sha256 != self.public_result_sha256
            || aggregate.public_request_sha256 != self.public_request_sha256
            || aggregate.public_request.full_campaign_run_id != self.run_id
            || aggregate
                .public_request
                .sessions
                .iter()
                .map(|session| &session.run_id)
                .ne(ordered_session_run_ids.iter())
            || aggregate.max_concurrent_players != self.max_concurrent_players
            || aggregate.participant_instance_count != self.participant_instance_count
            || aggregate.named_participant_instance_count != self.named_participant_instance_count
            || aggregate.anonymous_participant_instance_count
                != self.anonymous_participant_instance_count
            || self.public_participant_keys() != disclosed_session_key_union
            || aggregate.public_request.campaign_content_manifest_sha256 != self.content.digest()
            || aggregate.public_request.rules_config_sha256 != self.rules_config_sha256
            || aggregate.public_request.ruleset_manifest_sha256 != self.ruleset_manifest_sha256
            || aggregate.public_request.competition_manifest_sha256
                != self.competition_manifest_sha256
            || aggregate.canonical_genesis_campaign != self.starting_campaign
            || aggregate.final_campaign != self.final_campaign
            || aggregate.starting_campaign_score != self.starting_campaign_score
            || aggregate.final_campaign_score != self.final_campaign_score
            || aggregate.metrics != self.metrics
        {
            return Err(ValidationError::ClaimMismatch {
                field: "run.campaign_aggregate.proof",
            });
        }
        if aggregate.public_request.sessions.len() != self.full_campaign_sessions.len() {
            return Err(ValidationError::ClaimMismatch {
                field: "run.campaign_aggregate.sessions",
            });
        }
        let terminal_evidence_sha256 = self
            .full_campaign_sessions
            .last()
            .and_then(|session| {
                session
                    .verification_proof
                    .campaign_complete_evidence
                    .as_ref()
            })
            .map(|evidence| evidence.canonical_digest())
            .transpose()
            .map_err(|_| ValidationError::ClaimMismatch {
                field: "run.campaign_aggregate.terminal_completion",
            })?;
        if terminal_evidence_sha256
            != Some(
                aggregate
                    .public_request
                    .public_campaign_complete_evidence_sha256,
            )
        {
            return Err(ValidationError::ClaimMismatch {
                field: "run.campaign_aggregate.terminal_completion",
            });
        }
        for (proof, public) in aggregate
            .public_request
            .sessions
            .iter()
            .zip(&self.full_campaign_sessions)
        {
            if proof.run_id != public.run_id
                || proof.ordinal != public.ordinal
                || proof.public_verification_request_sha256
                    != public.public_verification_request_sha256
                || proof.public_verification_result_sha256
                    != public.public_verification_result_sha256
            {
                return Err(ValidationError::ClaimMismatch {
                    field: "run.campaign_aggregate.session_proof",
                });
            }
        }
        Ok(())
    }
}

fn validate_public_achievement_summaries(
    manifest: &crate::RulesetManifestV1,
    achievements: &[AchievementSummaryV1],
) -> Result<(), ValidationError> {
    if achievements.len() != manifest.achievement_policies.len()
        || achievements
            .iter()
            .zip(&manifest.achievement_policies)
            .any(|(result, policy)| result.verified.achievement_id != policy.achievement_id)
    {
        return Err(ValidationError::ClaimMismatch {
            field: "run.achievements.catalog",
        });
    }
    if achievements
        .iter()
        .zip(&manifest.achievement_policies)
        .any(|(result, policy)| {
            policy.mode == crate::AchievementPolicyModeV1::Required
                && result.verified.evaluation == VerifiedAchievementEvaluationV1::Unverifiable
        })
    {
        return Err(ValidationError::ClaimMismatch {
            field: "run.achievements.required_unverifiable",
        });
    }
    Ok(())
}
