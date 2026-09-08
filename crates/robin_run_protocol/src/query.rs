use std::cmp::Ordering;
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::{
    CampaignAggregationConsentV1, CampaignChainReceiptV1, CampaignSessionKindV1, ChallengeNonce32,
    InputProvenanceStatusV1, OfficialContentEditionV1, OfficialContentSubjectV1, ReplayArtifactV1,
    RunScopeKindV1, Signature64, SignatureAlgorithmV1, VerificationLimitsV1, VerificationRequestV1,
    VerificationResultV1, VerificationStatusV1, VerifiedAchievementV1, VerifiedCampaignAggregateV1,
};
use crate::{
    CanonicalDocument as _, Digest32, OpaqueId, PublicKey32, PublishedRulesetV1,
    RankedSessionConfigV1, RulesetBoardScopeV1, SimulationSeed64, SubmissionOfferV1,
    TerminalOutcomeV1, TickDurationV1, Validate, ValidationError,
};

pub const SUBMISSION_OWNER_STATUS_SIGNATURE_DOMAIN_V1: &[u8] =
    b"robinhood/leaderboards/1/submission-owner-status\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardCategoryV1 {
    IndividualLevel,
    Campaign,
}

/// A leaderboard ranks either one mission attempt (from a canonical
/// Individual Level start or a verified Campaign chain) or a complete
/// verified campaign. Full-campaign boards never use a magic mission ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LeaderboardSubjectV1 {
    Mission {
        mission_id: String,
        category: BoardCategoryV1,
    },
    FullCampaign,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunContentIdentityV1 {
    Mission {
        content_manifest_sha256: Digest32,
    },
    FullCampaign {
        campaign_content_manifest_sha256: Digest32,
    },
}

impl RunContentIdentityV1 {
    pub const fn digest(self) -> Digest32 {
        match self {
            Self::Mission {
                content_manifest_sha256,
            } => content_manifest_sha256,
            Self::FullCampaign {
                campaign_content_manifest_sha256,
            } => campaign_content_manifest_sha256,
        }
    }

    fn validate_for_subject(&self, subject: &LeaderboardSubjectV1) -> Result<(), ValidationError> {
        let shape_matches = matches!(
            (self, subject),
            (Self::Mission { .. }, LeaderboardSubjectV1::Mission { .. })
                | (
                    Self::FullCampaign { .. },
                    LeaderboardSubjectV1::FullCampaign
                )
        );
        if self.digest().is_zero() {
            return Err(ValidationError::Zero {
                field: "run_content_identity.digest",
            });
        }
        if !shape_matches {
            return Err(ValidationError::ClaimMismatch {
                field: "run_content_identity.subject",
            });
        }
        Ok(())
    }
}

impl LeaderboardSubjectV1 {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if let Self::Mission { mission_id, .. } = self {
            crate::validation::text("leaderboard_subject.mission_id", mission_id, 256)?;
        }
        Ok(())
    }

    pub const fn is_full_campaign(&self) -> bool {
        matches!(self, Self::FullCampaign)
    }
}

/// Artifact composition behind a public ranked result. A mission result owns
/// one replay. A full-campaign aggregate owns no synthetic replay and instead
/// links every ordered, independently verified field/HQ session. The private
/// campaign-chain locator never enters public composition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VerifiedRunCompositionV1 {
    Mission {
        replay_sha256: Digest32,
    },
    FullCampaign {
        ordered_session_run_ids: Vec<OpaqueId>,
    },
}

impl VerifiedRunCompositionV1 {
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Mission { replay_sha256 } if replay_sha256.is_zero() => {
                return Err(ValidationError::Zero {
                    field: "run_composition.replay_sha256",
                });
            }
            Self::FullCampaign {
                ordered_session_run_ids,
                ..
            } => {
                if ordered_session_run_ids.is_empty() || ordered_session_run_ids.len() > 4_096 {
                    return Err(ValidationError::CountOutOfRange {
                        field: "run_composition.ordered_session_run_ids",
                    });
                }
                if ordered_session_run_ids
                    .iter()
                    .collect::<BTreeSet<_>>()
                    .len()
                    != ordered_session_run_ids.len()
                {
                    return Err(ValidationError::Duplicate {
                        field: "run_composition.ordered_session_run_ids",
                        value: "run_id".into(),
                    });
                }
            }
            Self::Mission { .. } => {}
        }
        Ok(())
    }

    pub const fn is_full_campaign(&self) -> bool {
        matches!(self, Self::FullCampaign { .. })
    }
}

fn validate_subject_composition(
    subject: &LeaderboardSubjectV1,
    composition: &VerifiedRunCompositionV1,
) -> Result<(), ValidationError> {
    subject.validate()?;
    composition.validate()?;
    if subject.is_full_campaign() != composition.is_full_campaign() {
        return Err(ValidationError::ClaimMismatch {
            field: "subject/composition",
        });
    }
    Ok(())
}

/// The two normal ranked dimensions. Ransom remains a verifier-derived stat
/// and filter/facet; it is not a default leaderboard metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardMetricV1 {
    OriginalScore,
    FastestSuccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunMetricsV1 {
    pub original_score_delta: i64,
    pub active_simulation_ticks: u64,
    pub ransom_collected: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunFilterV1 {
    pub schema_version: u32,
    pub subject: LeaderboardSubjectV1,
    pub metric: BoardMetricV1,
    pub content: RunContentIdentityV1,
    pub rules_config_sha256: Digest32,
    pub ruleset_manifest_sha256: Digest32,
    pub competition_manifest_sha256: Option<Digest32>,
    pub max_concurrent_players: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_public_key: Option<PublicKey32>,
}

impl Validate for RunFilterV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("RunFilterV1", self.schema_version)?;
        self.subject.validate()?;
        self.content.validate_for_subject(&self.subject)?;
        for digest in [self.rules_config_sha256, self.ruleset_manifest_sha256] {
            if digest.is_zero() {
                return Err(ValidationError::Zero {
                    field: "run_filter.identity_digest",
                });
            }
        }
        if self.max_concurrent_players == Some(0) {
            return Err(ValidationError::EmptyPlayerCount);
        }
        if self.player_public_key.is_some_and(|key| key.is_zero()) {
            return Err(ValidationError::Zero {
                field: "run_filter.player_public_key",
            });
        }
        if self
            .competition_manifest_sha256
            .is_some_and(|digest| digest.is_zero())
        {
            return Err(ValidationError::Zero {
                field: "run_filter.competition_manifest_sha256",
            });
        }
        Ok(())
    }
}

impl RunFilterV1 {
    pub fn validate_against_competition(
        &self,
        competition: &CompetitionSummaryV1,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        competition.validate()?;
        let manifest = &competition.manifest;
        if self.competition_manifest_sha256 != Some(competition.competition_manifest_sha256)
            || self.subject != manifest.subject
            || self.metric != manifest.metric
            || self.content != manifest.content
            || self.rules_config_sha256 != manifest.rules_config_sha256
            || self.ruleset_manifest_sha256 != manifest.ruleset_manifest_sha256
            || self.max_concurrent_players
                != Some(manifest.participant_composition.max_concurrent_players())
        {
            return Err(ValidationError::ClaimMismatch {
                field: "run_filter.competition_tuple",
            });
        }
        Ok(())
    }
}

/// Flat subject discriminator accepted by `GET /api/v1/leaderboards`.
///
/// Keeping this scalar avoids relying on nested tagged-enum decoding in
/// `application/x-www-form-urlencoded` query extractors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaderboardQuerySubjectV1 {
    Mission,
    FullCampaign,
}

/// Exact flat query document accepted by `GET /api/v1/leaderboards`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaderboardQueryV1 {
    pub schema_version: u32,
    pub subject_kind: LeaderboardQuerySubjectV1,
    pub mission_id: Option<String>,
    pub mission_scope: Option<BoardCategoryV1>,
    pub metric: BoardMetricV1,
    /// Exact mission-manifest or campaign-catalog digest, selected by
    /// `subject_kind`. This query remains flat for URL form decoding.
    pub content_identity_sha256: Digest32,
    pub rules_config_sha256: Digest32,
    pub ruleset_manifest_sha256: Digest32,
    pub competition_manifest_sha256: Option<Digest32>,
    pub max_concurrent_players: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_public_key: Option<PublicKey32>,
    pub limit: u16,
    pub cursor: Option<String>,
}

impl LeaderboardQueryV1 {
    pub fn leaderboard_subject(&self) -> Result<LeaderboardSubjectV1, ValidationError> {
        match (self.subject_kind, &self.mission_id, self.mission_scope) {
            (LeaderboardQuerySubjectV1::Mission, Some(mission_id), Some(category)) => {
                crate::validation::text("leaderboard_query.mission_id", mission_id, 256)?;
                Ok(LeaderboardSubjectV1::Mission {
                    mission_id: mission_id.clone(),
                    category,
                })
            }
            (LeaderboardQuerySubjectV1::FullCampaign, None, None) => {
                Ok(LeaderboardSubjectV1::FullCampaign)
            }
            _ => Err(ValidationError::ClaimMismatch {
                field: "leaderboard_query.subject_kind/mission_id/mission_scope",
            }),
        }
    }

    pub fn filter(&self) -> Result<RunFilterV1, ValidationError> {
        let subject = self.leaderboard_subject()?;
        let content = match subject {
            LeaderboardSubjectV1::Mission { .. } => RunContentIdentityV1::Mission {
                content_manifest_sha256: self.content_identity_sha256,
            },
            LeaderboardSubjectV1::FullCampaign => RunContentIdentityV1::FullCampaign {
                campaign_content_manifest_sha256: self.content_identity_sha256,
            },
        };
        Ok(RunFilterV1 {
            schema_version: self.schema_version,
            subject,
            metric: self.metric,
            content,
            rules_config_sha256: self.rules_config_sha256,
            ruleset_manifest_sha256: self.ruleset_manifest_sha256,
            competition_manifest_sha256: self.competition_manifest_sha256,
            max_concurrent_players: self.max_concurrent_players,
            player_public_key: self.player_public_key,
        })
    }
}

impl TryFrom<LeaderboardQueryV1> for RunFilterV1 {
    type Error = ValidationError;

    fn try_from(query: LeaderboardQueryV1) -> Result<Self, Self::Error> {
        query.filter()
    }
}

impl Validate for LeaderboardQueryV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.filter()?.validate()?;
        if !(1..=100).contains(&self.limit) {
            return Err(ValidationError::CountOutOfRange {
                field: "leaderboard_query.limit",
            });
        }
        if let Some(cursor) = &self.cursor {
            crate::validation::text("leaderboard_query.cursor", cursor, 4096)?;
        }
        Ok(())
    }
}

/// Verification-derived fields needed to render a board row or run card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSummaryV1 {
    pub schema_version: u32,
    pub run_id: OpaqueId,
    pub subject: LeaderboardSubjectV1,
    pub composition: VerifiedRunCompositionV1,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u32,
    pub outcome: TerminalOutcomeV1,
    pub metrics: RunMetricsV1,
    pub content: RunContentIdentityV1,
    pub rules_config_sha256: Digest32,
    pub ruleset_manifest_sha256: Digest32,
    pub competition_manifest_sha256: Option<Digest32>,
}

impl Validate for RunSummaryV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("RunSummaryV1", self.schema_version)?;
        validate_subject_composition(&self.subject, &self.composition)?;
        self.content.validate_for_subject(&self.subject)?;
        for digest in [self.rules_config_sha256, self.ruleset_manifest_sha256] {
            if digest.is_zero() {
                return Err(ValidationError::Zero {
                    field: "run_summary.identity_digest",
                });
            }
        }
        if self.max_concurrent_players == 0
            || self.participant_instance_count < u32::from(self.max_concurrent_players)
        {
            return Err(ValidationError::EmptyPlayerCount);
        }
        Ok(())
    }
}

/// One mission available to the board browser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionFacetV1 {
    pub mission_id: String,
    pub display_name: String,
    pub content_manifest_sha256: Digest32,
}

impl Validate for MissionFacetV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::text("mission_facet.mission_id", &self.mission_id, 256)?;
        crate::validation::text("mission_facet.display_name", &self.display_name, 100)?;
        if self.content_manifest_sha256.is_zero() {
            return Err(ValidationError::Zero {
                field: "mission_facet.content_manifest_sha256",
            });
        }
        Ok(())
    }
}

/// One allowlisted ruleset, including its user-facing preset and difficulty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RulesetFacetV1 {
    pub ruleset_manifest_sha256: Digest32,
    pub rules_config_sha256: Digest32,
    pub display_name: String,
    pub preset_id: OpaqueId,
    pub preset_name: String,
    pub difficulty_id: OpaqueId,
    pub difficulty_name: String,
    pub content: RunContentIdentityV1,
    pub categories: Vec<BoardCategoryV1>,
    pub metrics: Vec<BoardMetricV1>,
    pub supports_full_campaign_boards: bool,
}

impl Validate for RulesetFacetV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::text("ruleset_facet.display_name", &self.display_name, 100)?;
        crate::validation::text("ruleset_facet.preset_name", &self.preset_name, 100)?;
        crate::validation::text("ruleset_facet.difficulty_name", &self.difficulty_name, 100)?;
        for digest in [self.ruleset_manifest_sha256, self.rules_config_sha256] {
            if digest.is_zero() {
                return Err(ValidationError::Zero {
                    field: "ruleset_facet.identity_digest",
                });
            }
        }
        if self.content.digest().is_zero() {
            return Err(ValidationError::Zero {
                field: "ruleset_facet.content",
            });
        }
        if self.categories.is_empty() || !crate::validation::strictly_sorted(&self.categories) {
            return Err(ValidationError::NotCanonicalOrder {
                field: "ruleset_facet.categories",
            });
        }
        if self.metrics.is_empty() || !crate::validation::strictly_sorted(&self.metrics) {
            return Err(ValidationError::InvalidMetrics {
                field: "ruleset_facet.metrics",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompetitionStateV1 {
    Upcoming,
    Active,
    Ended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CompetitionSeedPolicyV1 {
    Open,
    Pinned { simulation_seed: SimulationSeed64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CompetitionParticipantCompositionV1 {
    SinglePlayer,
    Multiplayer { max_concurrent_players: u16 },
}

impl CompetitionParticipantCompositionV1 {
    pub const fn max_concurrent_players(self) -> u16 {
        match self {
            Self::SinglePlayer => 1,
            Self::Multiplayer {
                max_concurrent_players,
            } => max_concurrent_players,
        }
    }
}

/// Complete immutable Challenge board tuple. The digest, not the editable
/// display ID, is carried through genesis, offer, verifier result and queries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompetitionManifestV1 {
    pub schema_version: u32,
    pub competition_id: OpaqueId,
    pub competition_version: u32,
    pub display_name: String,
    pub description: String,
    pub subject: LeaderboardSubjectV1,
    pub metric: BoardMetricV1,
    pub rules_config_sha256: Digest32,
    pub ruleset_manifest_sha256: Digest32,
    /// Public logical requirement; the operator-private artifact identity is
    /// intentionally absent from competition documents and Pages.
    pub canonical_campaign_state: crate::CanonicalCampaignStateRequirementV1,
    pub content: RunContentIdentityV1,
    pub seed_policy: CompetitionSeedPolicyV1,
    pub participant_composition: CompetitionParticipantCompositionV1,
    /// Dedicated service key which signs unpredictable pre-run grants. This is
    /// immutable competition policy, not the player's identity key.
    pub competition_run_grant_public_key: PublicKey32,
    pub starts_at_unix_ms: u64,
    pub ends_at_unix_ms: u64,
}

impl Validate for CompetitionManifestV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("CompetitionManifestV1", self.schema_version)?;
        if self.competition_version == 0 {
            return Err(ValidationError::Zero {
                field: "competition.competition_version",
            });
        }
        crate::validation::text("competition.display_name", &self.display_name, 100)?;
        crate::validation::text("competition.description", &self.description, 500)?;
        self.subject.validate()?;
        self.content.validate_for_subject(&self.subject)?;
        for digest in [self.rules_config_sha256, self.ruleset_manifest_sha256] {
            if digest.is_zero() {
                return Err(ValidationError::Zero {
                    field: "competition.identity_digest",
                });
            }
        }
        self.canonical_campaign_state.validate()?;
        if self.canonical_campaign_state.rules_config_sha256 != self.rules_config_sha256 {
            return Err(ValidationError::ClaimMismatch {
                field: "competition.canonical_campaign_state.rules_config_sha256",
            });
        }
        let player_count = self.participant_composition.max_concurrent_players();
        if player_count == 0 || player_count > crate::MAX_REPLAY_SEATS_V1 {
            return Err(ValidationError::CountOutOfRange {
                field: "competition.max_concurrent_players",
            });
        }
        if matches!(
            self.participant_composition,
            CompetitionParticipantCompositionV1::Multiplayer {
                max_concurrent_players: 1
            }
        ) {
            return Err(ValidationError::CountOutOfRange {
                field: "competition.multiplayer.max_concurrent_players",
            });
        }
        if self.competition_run_grant_public_key.is_zero() {
            return Err(ValidationError::Zero {
                field: "competition.competition_run_grant_public_key",
            });
        }
        if self.starts_at_unix_ms == 0 || self.ends_at_unix_ms <= self.starts_at_unix_ms {
            return Err(ValidationError::CountOutOfRange {
                field: "competition.starts_at_unix_ms/ends_at_unix_ms",
            });
        }
        Ok(())
    }
}

impl CompetitionManifestV1 {
    pub fn validate_ranked_session(
        &self,
        ranked: &RankedSessionConfigV1,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        ranked.validate()?;
        let digest = self
            .canonical_digest()
            .map_err(|_| ValidationError::ClaimMismatch {
                field: "competition.manifest_digest",
            })?;
        let subject_matches = match (&self.subject, &ranked.content_subject) {
            (
                LeaderboardSubjectV1::Mission { mission_id, .. },
                crate::OfficialContentSubjectV1::FieldMission {
                    mission_id: ranked_mission,
                },
            ) => mission_id == ranked_mission && mission_id == &ranked.mission_id,
            (LeaderboardSubjectV1::FullCampaign, _) => true,
            _ => false,
        };
        let content_matches = match self.content {
            RunContentIdentityV1::Mission {
                content_manifest_sha256,
            } => ranked.content_manifest_sha256 == content_manifest_sha256,
            RunContentIdentityV1::FullCampaign {
                campaign_content_manifest_sha256,
            } => ranked.campaign_content_manifest_sha256 == Some(campaign_content_manifest_sha256),
        };
        let seed_matches = match self.seed_policy {
            CompetitionSeedPolicyV1::Open => true,
            CompetitionSeedPolicyV1::Pinned { simulation_seed } => {
                ranked.simulation_seed == simulation_seed
            }
        };
        if ranked.competition_manifest_sha256 != Some(digest)
            || ranked.rules_config_sha256 != self.rules_config_sha256
            || ranked.ruleset_manifest_sha256 != self.ruleset_manifest_sha256
            || !subject_matches
            || !content_matches
            || !seed_matches
        {
            return Err(ValidationError::ClaimMismatch {
                field: "competition.ranked_session_tuple",
            });
        }
        Ok(())
    }

    pub fn validate_run_grant(
        &self,
        grant: &crate::CompetitionRunGrantV1,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        grant.validate()?;
        let digest = self
            .canonical_digest()
            .map_err(|_| ValidationError::ClaimMismatch {
                field: "competition.manifest_digest",
            })?;
        if grant.claim.competition_manifest_sha256 != digest
            || grant.claim.grant_authority_public_key != self.competition_run_grant_public_key
            || grant.claim.admitted_at_unix_ms < self.starts_at_unix_ms
            || grant.claim.expires_at_unix_ms >= self.ends_at_unix_ms
        {
            return Err(ValidationError::ClaimMismatch {
                field: "competition.run_grant",
            });
        }
        Ok(())
    }

    pub fn validate_submission_offer(
        &self,
        offer: &SubmissionOfferV1,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        offer.validate()?;
        let digest = self
            .canonical_digest()
            .map_err(|_| ValidationError::ClaimMismatch {
                field: "competition.manifest_digest",
            })?;
        let ranked: &RankedSessionConfigV1 = &offer.session_genesis.claim.ranked_session;
        let subject_matches = match (&self.subject, &offer.starting_state) {
            (
                LeaderboardSubjectV1::Mission {
                    mission_id,
                    category: BoardCategoryV1::IndividualLevel,
                },
                crate::InitialStateExpectationV1::IndividualLevel { .. },
            ) => mission_id == &offer.mission_id,
            (
                LeaderboardSubjectV1::Mission {
                    mission_id,
                    category: BoardCategoryV1::Campaign,
                },
                crate::InitialStateExpectationV1::CampaignGenesis { .. }
                | crate::InitialStateExpectationV1::CampaignContinuation { .. },
            ) => mission_id == &offer.mission_id,
            (
                LeaderboardSubjectV1::FullCampaign,
                crate::InitialStateExpectationV1::CampaignGenesis { .. }
                | crate::InitialStateExpectationV1::CampaignContinuation { .. },
            ) => true,
            _ => false,
        };
        let seed_matches = match self.seed_policy {
            CompetitionSeedPolicyV1::Open => true,
            CompetitionSeedPolicyV1::Pinned { simulation_seed } => {
                ranked.simulation_seed == simulation_seed
            }
        };
        if offer.competition_manifest_sha256 != Some(digest)
            || ranked.competition_manifest_sha256 != Some(digest)
            || match self.content {
                RunContentIdentityV1::Mission {
                    content_manifest_sha256,
                } => offer.content_manifest_sha256 != content_manifest_sha256,
                RunContentIdentityV1::FullCampaign {
                    campaign_content_manifest_sha256,
                } => {
                    ranked.campaign_content_manifest_sha256
                        != Some(campaign_content_manifest_sha256)
                }
            }
            || offer.rules_config_sha256 != self.rules_config_sha256
            || offer.ruleset_manifest_sha256 != self.ruleset_manifest_sha256
            || offer.starting_state.campaign_state_requirement() != self.canonical_campaign_state
            || offer.max_concurrent_players != self.participant_composition.max_concurrent_players()
            || !offer.allowed_metrics.contains(&self.metric)
            || !subject_matches
            || !seed_matches
        {
            return Err(ValidationError::ClaimMismatch {
                field: "competition.submission_offer_tuple",
            });
        }
        Ok(())
    }
}

/// A scheduled named competition which still ranks by a normal score/time
/// metric. A competition ID is a board dimension, never an upload challenge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompetitionSummaryV1 {
    pub competition_manifest_sha256: Digest32,
    pub manifest: CompetitionManifestV1,
    pub state: CompetitionStateV1,
}

impl Validate for CompetitionSummaryV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.manifest.validate()?;
        if self
            .manifest
            .canonical_digest()
            .map_err(|_| ValidationError::ClaimMismatch {
                field: "competition.manifest",
            })?
            != self.competition_manifest_sha256
        {
            return Err(ValidationError::ClaimMismatch {
                field: "competition.competition_manifest_sha256",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FullCampaignFacetV1 {
    pub display_name: String,
    pub description: String,
}

impl Validate for FullCampaignFacetV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::text("full_campaign_facet.display_name", &self.display_name, 100)?;
        crate::validation::text("full_campaign_facet.description", &self.description, 500)
    }
}

/// Complete, versioned facet document used to build board selectors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaderboardMetadataV1 {
    pub schema_version: u32,
    pub missions: Vec<MissionFacetV1>,
    pub rulesets: Vec<RulesetFacetV1>,
    pub competitions: Vec<CompetitionSummaryV1>,
    pub full_campaign: Option<FullCampaignFacetV1>,
}

impl Validate for LeaderboardMetadataV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("LeaderboardMetadataV1", self.schema_version)?;
        if self.missions.len() > 4_096
            || self.rulesets.len() > 1_024
            || self.competitions.len() > 1_024
        {
            return Err(ValidationError::CountOutOfRange {
                field: "leaderboard_metadata.collections",
            });
        }
        for mission in &self.missions {
            mission.validate()?;
        }
        for ruleset in &self.rulesets {
            ruleset.validate()?;
        }
        for competition in &self.competitions {
            competition.validate()?;
        }
        if !self
            .missions
            .windows(2)
            .all(|pair| pair[0].mission_id < pair[1].mission_id)
            || !self
                .rulesets
                .windows(2)
                .all(|pair| pair[0].ruleset_manifest_sha256 < pair[1].ruleset_manifest_sha256)
            || !self.competitions.windows(2).all(|pair| {
                pair[0].competition_manifest_sha256 < pair[1].competition_manifest_sha256
            })
        {
            return Err(ValidationError::NotCanonicalOrder {
                field: "leaderboard_metadata.collections",
            });
        }
        if let Some(facet) = &self.full_campaign {
            facet.validate()?;
        }
        Ok(())
    }
}

/// Public identity for one authenticated seat. The durable public key and
/// session seat are the complete published identity; the private
/// replay-scoped participant instance never enters a public document. The
/// short fingerprint is display-only and may collide.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicParticipantV1 {
    pub seat: u16,
    pub username: String,
    pub public_key: PublicKey32,
    pub public_key_fingerprint: String,
}

impl Validate for PublicParticipantV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.seat >= crate::MAX_REPLAY_SEATS_V1 {
            return Err(ValidationError::CountOutOfRange {
                field: "public_participant.seat",
            });
        }
        if self.public_key.is_zero() {
            return Err(ValidationError::Zero {
                field: "public_participant.public_key",
            });
        }
        crate::validation::text("public_participant.username", &self.username, 48)?;
        crate::validation::text(
            "public_participant.public_key_fingerprint",
            &self.public_key_fingerprint,
            80,
        )?;
        if self.public_key_fingerprint != self.public_key.short_fingerprint() {
            return Err(ValidationError::ClaimMismatch {
                field: "public_participant.public_key_fingerprint",
            });
        }
        Ok(())
    }
}

/// Durable named identity shown for a full-campaign aggregate. Seats belong to
/// individual sessions and are intentionally absent: different keys may
/// truthfully occupy the same seat over time. Replay-scoped participant
/// instances remain private at every public scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AggregatePublicParticipantV1 {
    pub current_display_name: String,
    pub public_key: PublicKey32,
    pub public_key_fingerprint: String,
}

impl Validate for AggregatePublicParticipantV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.public_key.is_zero() {
            return Err(ValidationError::Zero {
                field: "aggregate_public_participant.public_key",
            });
        }
        crate::validation::text(
            "aggregate_public_participant.current_display_name",
            &self.current_display_name,
            48,
        )?;
        if self.public_key_fingerprint != self.public_key.short_fingerprint() {
            return Err(ValidationError::ClaimMismatch {
                field: "aggregate_public_participant.public_key_fingerprint",
            });
        }
        Ok(())
    }
}

/// Public mutable profile for one durable key identity. Usernames are not
/// unique; UI must display the fingerprint alongside them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerProfileV1 {
    pub schema_version: u32,
    pub username: String,
    pub public_key: PublicKey32,
    pub public_key_fingerprint: String,
}

impl Validate for PlayerProfileV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("PlayerProfileV1", self.schema_version)?;
        PublicParticipantV1 {
            seat: 0,
            username: self.username.clone(),
            public_key: self.public_key,
            public_key_fingerprint: self.public_key_fingerprint.clone(),
        }
        .validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerRunHistoryQueryV1 {
    pub schema_version: u32,
    pub limit: u16,
    pub cursor: Option<String>,
}

/// Canonical identity of one player's bounded public-history query.
///
/// Pagination cursors bind this document rather than the transport query so
/// that the opaque cursor itself is excluded while the path-owned player key
/// remains part of the authenticated query identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerRunHistoryFilterV1 {
    pub schema_version: u32,
    pub player_public_key: PublicKey32,
    pub limit: u16,
}

impl Validate for PlayerRunHistoryFilterV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("PlayerRunHistoryFilterV1", self.schema_version)?;
        if self.player_public_key.is_zero() {
            return Err(ValidationError::Zero {
                field: "player_run_history_filter.player_public_key",
            });
        }
        if !(1..=100).contains(&self.limit) {
            return Err(ValidationError::CountOutOfRange {
                field: "player_run_history_filter.limit",
            });
        }
        Ok(())
    }
}

impl Validate for PlayerRunHistoryQueryV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("PlayerRunHistoryQueryV1", self.schema_version)?;
        if !(1..=100).contains(&self.limit) {
            return Err(ValidationError::CountOutOfRange {
                field: "player_run_history_query.limit",
            });
        }
        if let Some(cursor) = &self.cursor {
            crate::validation::text("player_run_history_query.cursor", cursor, 4_096)?;
        }
        Ok(())
    }
}

impl PlayerRunHistoryQueryV1 {
    pub const fn filter_for_player(
        &self,
        player_public_key: PublicKey32,
    ) -> PlayerRunHistoryFilterV1 {
        PlayerRunHistoryFilterV1 {
            schema_version: self.schema_version,
            player_public_key,
            limit: self.limit,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerRunHistoryEntryV1 {
    pub player_public_key: PublicKey32,
    pub run: RunSummaryV1,
    pub verified_at_unix_ms: u64,
}

impl Validate for PlayerRunHistoryEntryV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.player_public_key.is_zero() {
            return Err(ValidationError::Zero {
                field: "player_run_history_entry.player_public_key",
            });
        }
        self.run.validate()?;
        if self.verified_at_unix_ms == 0 {
            return Err(ValidationError::Zero {
                field: "player_run_history_entry.verified_at_unix_ms",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerPersonalBestV1 {
    pub filter: RunFilterV1,
    pub run_id: OpaqueId,
    pub metric_value: BoardMetricValueV1,
}

impl Validate for PlayerPersonalBestV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.filter.validate()?;
        if self.filter.player_public_key.is_none() {
            return Err(ValidationError::ClaimMismatch {
                field: "player_personal_best.filter.player_public_key",
            });
        }
        if !matches!(
            (self.filter.metric, &self.metric_value),
            (
                BoardMetricV1::OriginalScore,
                BoardMetricValueV1::OriginalScore { points: 0.. }
            ) | (
                BoardMetricV1::FastestSuccess,
                BoardMetricValueV1::FastestSuccess { .. }
            )
        ) {
            return Err(ValidationError::ClaimMismatch {
                field: "player_personal_best.metric_value",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerRunHistoryPageV1 {
    pub schema_version: u32,
    pub player: PlayerProfileV1,
    pub accepted_sequence_watermark: u64,
    pub runs: Vec<PlayerRunHistoryEntryV1>,
    pub personal_bests: Vec<PlayerPersonalBestV1>,
    pub next_cursor: Option<String>,
}

impl Validate for PlayerRunHistoryPageV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("PlayerRunHistoryPageV1", self.schema_version)?;
        self.player.validate()?;
        if self.runs.len() > 100 || self.personal_bests.len() > 512 {
            return Err(ValidationError::CountOutOfRange {
                field: "player_run_history_page.entries",
            });
        }
        let mut run_ids = BTreeSet::new();
        for run in &self.runs {
            run.validate()?;
            if run.player_public_key != self.player.public_key {
                return Err(ValidationError::ClaimMismatch {
                    field: "player_run_history_page.runs.player_public_key",
                });
            }
            if !run_ids.insert(run.run.run_id.clone()) {
                return Err(ValidationError::Duplicate {
                    field: "player_run_history_page.runs.run_id",
                    value: run.run.run_id.as_str().to_owned(),
                });
            }
        }
        let mut best_filters = BTreeSet::new();
        for best in &self.personal_bests {
            best.validate()?;
            if best.filter.player_public_key != Some(self.player.public_key) {
                return Err(ValidationError::ClaimMismatch {
                    field: "player_run_history_page.personal_bests.player_public_key",
                });
            }
            let digest =
                best.filter
                    .canonical_digest()
                    .map_err(|_| ValidationError::ClaimMismatch {
                        field: "player_run_history_page.personal_bests.filter",
                    })?;
            if !best_filters.insert(digest) {
                return Err(ValidationError::Duplicate {
                    field: "player_run_history_page.personal_bests.filter",
                    value: digest.to_string(),
                });
            }
        }
        if let Some(cursor) = &self.next_cursor {
            crate::validation::text("player_run_history_page.next_cursor", cursor, 4_096)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "metric", rename_all = "snake_case", deny_unknown_fields)]
pub enum BoardMetricValueV1 {
    OriginalScore {
        points: i64,
    },
    FastestSuccess {
        active_simulation_ticks: u64,
        tick_duration: TickDurationV1,
    },
}

impl BoardMetricValueV1 {
    pub const fn metric(&self) -> BoardMetricV1 {
        match self {
            Self::OriginalScore { .. } => BoardMetricV1::OriginalScore,
            Self::FastestSuccess { .. } => BoardMetricV1::FastestSuccess,
        }
    }

    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::OriginalScore { points } if !(0..=i64::from(u32::MAX)).contains(points) => {
                Err(ValidationError::InvalidOriginalScore)
            }
            Self::FastestSuccess { tick_duration, .. } => tick_duration.validate(),
            Self::OriginalScore { .. } => Ok(()),
        }
    }
}

fn validate_public_roster(
    max_concurrent_players: u16,
    participant_instance_count: u32,
    named_participant_instance_count: u32,
    named_participants: &[PublicParticipantV1],
    aggregate_named_participants: &[AggregatePublicParticipantV1],
    anonymous_participant_instance_count: u32,
    mission_instances_required: bool,
) -> Result<(), ValidationError> {
    if max_concurrent_players == 0
        || participant_instance_count < u32::from(max_concurrent_players)
        || named_participant_instance_count.checked_add(anonymous_participant_instance_count)
            != Some(participant_instance_count)
    {
        return Err(ValidationError::EmptyPlayerCount);
    }
    if mission_instances_required {
        if !aggregate_named_participants.is_empty()
            || usize::try_from(named_participant_instance_count).ok()
                != Some(named_participants.len())
        {
            return Err(ValidationError::InvalidParticipantClaims);
        }
        for participant in named_participants {
            participant.validate()?;
        }
        if !named_participants
            .windows(2)
            .all(|pair| (pair[0].seat, pair[0].public_key) < (pair[1].seat, pair[1].public_key))
            || named_participants
                .iter()
                .map(|participant| participant.public_key)
                .collect::<BTreeSet<_>>()
                .len()
                != named_participants.len()
        {
            return Err(ValidationError::InvalidParticipantClaims);
        }
    } else {
        let identity_count = u32::try_from(aggregate_named_participants.len()).map_err(|_| {
            ValidationError::CountOutOfRange {
                field: "aggregate_named_participants",
            }
        })?;
        if !named_participants.is_empty()
            || (named_participant_instance_count > 0 && aggregate_named_participants.is_empty())
            || identity_count > named_participant_instance_count
        {
            return Err(ValidationError::InvalidParticipantClaims);
        }
        for participant in aggregate_named_participants {
            participant.validate()?;
        }
        if !aggregate_named_participants
            .windows(2)
            .all(|pair| pair[0].public_key < pair[1].public_key)
        {
            return Err(ValidationError::InvalidParticipantClaims);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaderboardEntryV1 {
    /// Stable one-based SQL/window position in this immutable snapshot.
    pub position: u64,
    pub rank: u64,
    pub run_id: OpaqueId,
    pub composition: VerifiedRunCompositionV1,
    pub metric_value: BoardMetricValueV1,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u32,
    pub named_participant_instance_count: u32,
    pub named_participants: Vec<PublicParticipantV1>,
    pub aggregate_named_participants: Vec<AggregatePublicParticipantV1>,
    pub anonymous_participant_instance_count: u32,
    pub accepted_sequence: u64,
    pub verified_at_unix_ms: u64,
}

impl Validate for LeaderboardEntryV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.position == 0 || self.rank == 0 || self.rank > self.position {
            return Err(ValidationError::Zero {
                field: "leaderboard_entry.position/rank",
            });
        }
        if self.accepted_sequence == 0 || self.verified_at_unix_ms == 0 {
            return Err(ValidationError::Zero {
                field: "leaderboard_entry.verified_at_unix_ms",
            });
        }
        self.metric_value.validate()?;
        self.composition.validate()?;
        validate_public_roster(
            self.max_concurrent_players,
            self.participant_instance_count,
            self.named_participant_instance_count,
            &self.named_participants,
            &self.aggregate_named_participants,
            self.anonymous_participant_instance_count,
            matches!(self.composition, VerifiedRunCompositionV1::Mission { .. }),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaderboardOrderAnchorV1 {
    pub position: u64,
    pub rank: u64,
    pub metric_value: BoardMetricValueV1,
    pub accepted_sequence: u64,
    pub verified_at_unix_ms: u64,
    pub run_id: OpaqueId,
}

impl LeaderboardOrderAnchorV1 {
    fn from_entry(entry: &LeaderboardEntryV1) -> Self {
        Self {
            position: entry.position,
            rank: entry.rank,
            metric_value: entry.metric_value.clone(),
            accepted_sequence: entry.accepted_sequence,
            verified_at_unix_ms: entry.verified_at_unix_ms,
            run_id: entry.run_id.clone(),
        }
    }

    fn validate(&self) -> Result<(), ValidationError> {
        if self.position == 0
            || self.rank == 0
            || self.rank > self.position
            || self.accepted_sequence == 0
            || self.verified_at_unix_ms == 0
        {
            return Err(ValidationError::CountOutOfRange {
                field: "leaderboard_order_anchor",
            });
        }
        self.metric_value.validate()
    }
}

/// Decoded, authenticated cursor contract. `opaque_token` is issued and
/// authenticated by the service; the public fields let clients and protocol
/// validation fail closed on query/snapshot substitution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaderboardCursorV1 {
    pub schema_version: u32,
    pub query_sha256: Digest32,
    pub accepted_sequence_watermark: u64,
    pub last: LeaderboardOrderAnchorV1,
    pub opaque_token: String,
}

impl Validate for LeaderboardCursorV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("LeaderboardCursorV1", self.schema_version)?;
        if self.query_sha256.is_zero() || self.accepted_sequence_watermark == 0 {
            return Err(ValidationError::Zero {
                field: "leaderboard_cursor.snapshot",
            });
        }
        self.last.validate()?;
        if self.last.accepted_sequence > self.accepted_sequence_watermark {
            return Err(ValidationError::ClaimMismatch {
                field: "leaderboard_cursor.watermark",
            });
        }
        crate::validation::text("leaderboard_cursor.opaque_token", &self.opaque_token, 4096)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaderboardPageV1 {
    pub schema_version: u32,
    pub filter: RunFilterV1,
    pub accepted_sequence_watermark: u64,
    /// Decoded cursor supplied for this page; absent only on the first page.
    pub previous_cursor: Option<LeaderboardCursorV1>,
    pub entries: Vec<LeaderboardEntryV1>,
    pub next_cursor: Option<LeaderboardCursorV1>,
}

impl Validate for LeaderboardPageV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("LeaderboardPageV1", self.schema_version)?;
        self.filter.validate()?;
        if self.entries.len() > 100 {
            return Err(ValidationError::CountOutOfRange {
                field: "leaderboard.entries/watermark",
            });
        }
        if self.entries.is_empty() {
            if self.previous_cursor.is_some() || self.next_cursor.is_some() {
                return Err(ValidationError::ClaimMismatch {
                    field: "leaderboard.empty_page_cursor",
                });
            }
            return Ok(());
        }
        if self.accepted_sequence_watermark == 0 {
            return Err(ValidationError::CountOutOfRange {
                field: "leaderboard.entries/watermark",
            });
        }
        let query_sha256 =
            self.filter
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "leaderboard.filter_digest",
                })?;
        if let Some(cursor) = &self.previous_cursor {
            cursor.validate()?;
            if cursor.query_sha256 != query_sha256
                || cursor.accepted_sequence_watermark != self.accepted_sequence_watermark
            {
                return Err(ValidationError::ClaimMismatch {
                    field: "leaderboard.previous_cursor",
                });
            }
        }
        if self
            .entries
            .iter()
            .map(|entry| &entry.run_id)
            .collect::<BTreeSet<_>>()
            .len()
            != self.entries.len()
            || self
                .entries
                .iter()
                .map(|entry| entry.accepted_sequence)
                .collect::<BTreeSet<_>>()
                .len()
                != self.entries.len()
        {
            return Err(ValidationError::Duplicate {
                field: "leaderboard.entries",
                value: "run_id/accepted_sequence".into(),
            });
        }
        for entry in &self.entries {
            entry.validate()?;
            validate_subject_composition(&self.filter.subject, &entry.composition)?;
            if entry.metric_value.metric() != self.filter.metric {
                return Err(ValidationError::MetricValueMismatch {
                    field: "leaderboard.entries.metric_value",
                });
            }
            if matches!(self.filter.subject, LeaderboardSubjectV1::Mission { .. })
                && matches!(
                    entry.metric_value,
                    BoardMetricValueV1::OriginalScore { points } if points < 0
                )
            {
                return Err(ValidationError::InvalidOriginalScore);
            }
            if self
                .filter
                .max_concurrent_players
                .is_some_and(|count| count != entry.max_concurrent_players)
            {
                return Err(ValidationError::CountOutOfRange {
                    field: "leaderboard.entries.max_concurrent_players",
                });
            }
            if entry.accepted_sequence > self.accepted_sequence_watermark {
                return Err(ValidationError::ClaimMismatch {
                    field: "leaderboard.entries.accepted_sequence_watermark",
                });
            }
        }
        let mut previous = self
            .previous_cursor
            .as_ref()
            .map(|cursor| cursor.last.clone());
        for entry in &self.entries {
            if let Some(prior) = &previous {
                validate_leaderboard_order(self.filter.metric, prior, entry)?;
            } else if entry.position != 1 || entry.rank != 1 {
                return Err(ValidationError::ClaimMismatch {
                    field: "leaderboard.first_entry",
                });
            }
            previous = Some(LeaderboardOrderAnchorV1::from_entry(entry));
        }
        if let Some(cursor) = &self.next_cursor {
            cursor.validate()?;
            if cursor.query_sha256 != query_sha256
                || cursor.accepted_sequence_watermark != self.accepted_sequence_watermark
                || self
                    .entries
                    .last()
                    .map(LeaderboardOrderAnchorV1::from_entry)
                    != Some(cursor.last.clone())
            {
                return Err(ValidationError::ClaimMismatch {
                    field: "leaderboard.next_cursor",
                });
            }
        }
        Ok(())
    }
}

impl LeaderboardPageV1 {
    pub fn validate_against_ruleset(
        &self,
        published: &PublishedRulesetV1,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        published.validate()?;
        let manifest = &published.manifest;
        let scope = match self.filter.subject {
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
        if self.filter.ruleset_manifest_sha256 != published.ruleset_manifest_sha256
            || self.filter.rules_config_sha256 != manifest.rules_config_sha256
            || match self.filter.content {
                RunContentIdentityV1::Mission {
                    content_manifest_sha256,
                } => manifest
                    .allowed_content_manifest_sha256
                    .binary_search(&content_manifest_sha256)
                    .is_err(),
                RunContentIdentityV1::FullCampaign {
                    campaign_content_manifest_sha256,
                } => manifest
                    .allowed_campaign_content_manifest_sha256
                    .binary_search(&campaign_content_manifest_sha256)
                    .is_err(),
            }
            || manifest.board_scopes.binary_search(&scope).is_err()
            || manifest.metrics.binary_search(&self.filter.metric).is_err()
        {
            return Err(ValidationError::ClaimMismatch {
                field: "leaderboard.ruleset_tuple",
            });
        }
        for entry in &self.entries {
            if entry.max_concurrent_players
                < manifest
                    .participant_eligibility
                    .minimum_max_concurrent_players
                || entry.max_concurrent_players
                    > manifest
                        .participant_eligibility
                        .maximum_max_concurrent_players
            {
                return Err(ValidationError::CountOutOfRange {
                    field: "leaderboard.ruleset.player_count",
                });
            }
            if let BoardMetricValueV1::FastestSuccess { tick_duration, .. } = &entry.metric_value
                && tick_duration != &manifest.tick_duration
            {
                return Err(ValidationError::ClaimMismatch {
                    field: "leaderboard.ruleset.tick_duration",
                });
            }
        }
        Ok(())
    }
}

fn primary_metric_order(
    metric: BoardMetricV1,
    left: &BoardMetricValueV1,
    right: &BoardMetricValueV1,
) -> Result<Ordering, ValidationError> {
    match (metric, left, right) {
        (
            BoardMetricV1::OriginalScore,
            BoardMetricValueV1::OriginalScore { points: left },
            BoardMetricValueV1::OriginalScore { points: right },
        ) => Ok(right.cmp(left)),
        (
            BoardMetricV1::FastestSuccess,
            BoardMetricValueV1::FastestSuccess {
                active_simulation_ticks: left,
                ..
            },
            BoardMetricValueV1::FastestSuccess {
                active_simulation_ticks: right,
                ..
            },
        ) => Ok(left.cmp(right)),
        _ => Err(ValidationError::MetricValueMismatch {
            field: "leaderboard.order.metric",
        }),
    }
}

fn validate_leaderboard_order(
    metric: BoardMetricV1,
    prior: &LeaderboardOrderAnchorV1,
    current: &LeaderboardEntryV1,
) -> Result<(), ValidationError> {
    if current.position != prior.position.saturating_add(1) {
        return Err(ValidationError::ClaimMismatch {
            field: "leaderboard.order.position",
        });
    }
    match primary_metric_order(metric, &prior.metric_value, &current.metric_value)? {
        Ordering::Greater => {
            return Err(ValidationError::ClaimMismatch {
                field: "leaderboard.order.primary_metric",
            });
        }
        Ordering::Equal => {
            if current.rank != prior.rank
                || (
                    current.accepted_sequence,
                    current.verified_at_unix_ms,
                    &current.run_id,
                ) <= (
                    prior.accepted_sequence,
                    prior.verified_at_unix_ms,
                    &prior.run_id,
                )
            {
                return Err(ValidationError::ClaimMismatch {
                    field: "leaderboard.order.tie",
                });
            }
        }
        Ordering::Less => {
            if current.rank != current.position {
                return Err(ValidationError::ClaimMismatch {
                    field: "leaderboard.order.rank_skip",
                });
            }
        }
    }
    Ok(())
}

mod submission_status;
pub use submission_status::{
    SubmissionAcceptedV1, SubmissionFailureCodeV1, SubmissionLifecycleV1,
    SubmissionOwnerStatusChallengeRequestV1, SubmissionOwnerStatusChallengeV1,
    SubmissionOwnerStatusEnvelopeV1, SubmissionOwnerStatusResponseV1,
};
mod public_proof;
pub use public_proof::{
    AchievementSummaryV1, CampaignSessionDetailV1, FullCampaignSessionKindV1,
    FullCampaignSessionV1, PublicAchievementDecisionV1, PublicBuildV1,
    PublicCampaignAggregateProofV1, PublicCampaignAggregateRequestV1,
    PublicCampaignCompleteEvidenceV1, PublicCampaignSessionBindingV1,
    PublicNamedParticipantClaimV1, PublicVerificationProofV1, PublicVerificationRequestV1,
    RunDetailV1, ViewerAvailabilityV1, ViewerContentRequirementV1, ViewerLaunchV1,
};
#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        ParticipantClaimV1, ParticipantPublicDisclosureV1, SCHEMA_VERSION_V1,
        VerifiedAchievementEvaluationV1, VerifiedRunV1,
    };

    fn id(value: &str) -> OpaqueId {
        OpaqueId::new(value).unwrap()
    }

    fn campaign_artifact(byte: u8) -> crate::ArtifactRefV1 {
        crate::ArtifactRefV1 {
            sha256: Digest32::from_bytes([byte; 32]),
            byte_length: 100 + u64::from(byte),
            media_type: crate::RANKED_CAMPAIGN_MEDIA_TYPE_V1.into(),
        }
    }

    fn replay_artifact(byte: u8) -> ReplayArtifactV1 {
        ReplayArtifactV1 {
            artifact: crate::ArtifactRefV1 {
                sha256: Digest32::from_bytes([byte; 32]),
                byte_length: 1,
                media_type: crate::RANKED_REPLAY_MEDIA_TYPE_V1.into(),
            },
            replay_schema_version: crate::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
        }
    }

    fn submission_artifacts() -> crate::SubmissionArtifactsV1 {
        crate::SubmissionArtifactsV1 {
            replay: replay_artifact(6),
            starting_campaign: campaign_artifact(8),
        }
    }

    fn fresh_preflight_grant(
        ranked: &crate::RankedSessionConfigV1,
        campaign: bool,
    ) -> crate::FreshRunPreflightGrantV1 {
        crate::FreshRunPreflightGrantV1 {
            claim: crate::FreshRunPreflightGrantClaimV1 {
                schema_version: SCHEMA_VERSION_V1,
                grant_id: id("fresh-grant-1"),
                grant_nonce: ChallengeNonce32::from_bytes([21; 32]),
                grant_authority_public_key: PublicKey32::from_bytes([22; 32]),
                host_public_key: PublicKey32::from_bytes([4; 32]),
                grant_request_sha256: Digest32::from_bytes([23; 32]),
                ranked_session_sha256: ranked.canonical_digest().unwrap(),
                replay_session_id: Digest32::from_bytes([14; 32]),
                host_participant_instance_id: Digest32::from_bytes([12; 32]),
                host_nonce: ChallengeNonce32::from_bytes([15; 32]),
                scope: if campaign {
                    crate::FreshRunScopeV1::CampaignGenesis
                } else {
                    crate::FreshRunScopeV1::IndividualLevel
                },
                starting_campaign: campaign_artifact(8),
                admitted_at_unix_ms: 1,
                expires_at_unix_ms: 2,
            },
            algorithm: SignatureAlgorithmV1::Ed25519,
            authority_signature: Signature64::from_bytes([24; 64]),
        }
    }

    fn receipt() -> CampaignChainReceiptV1 {
        CampaignChainReceiptV1 {
            schema_version: SCHEMA_VERSION_V1,
            chain_id: id("campaign-1"),
            predecessor_run_id: id("run-1"),
            predecessor_verification_sha256: Digest32::from_bytes([30; 32]),
            campaign_controller_public_key: PublicKey32::from_bytes([4; 32]),
            expected_starting_campaign: campaign_artifact(1),
            rules_config_sha256: Digest32::from_bytes([7; 32]),
            ruleset_manifest_sha256: Digest32::from_bytes([2; 32]),
            competition_manifest_sha256: None,
            campaign_content_manifest_sha256: Digest32::from_bytes([29; 32]),
            expected_max_concurrent_players: 1,
            participant_public_keys: vec![PublicKey32::from_bytes([4; 32])],
            state: crate::CampaignChainStateV1::Active,
        }
    }

    fn host_claim() -> ParticipantClaimV1 {
        ParticipantClaimV1 {
            seat: 0,
            participant_instance_id: Digest32::from_bytes([12; 32]),
            public_key: PublicKey32::from_bytes([4; 32]),
            public_disclosure: ParticipantPublicDisclosureV1::NamedProfile,
            join_attestation: None,
        }
    }

    fn transcript() -> crate::ReplaySessionTranscriptV1 {
        crate::ReplaySessionTranscriptV1 {
            schema_version: SCHEMA_VERSION_V1,
            session_genesis_sha256: Digest32::from_bytes([13; 32]),
            replay_session_id: Digest32::from_bytes([14; 32]),
            host_participant_instance_id: Digest32::from_bytes([12; 32]),
            participant_instance_count: 1,
            max_concurrent_players: 1,
            events: vec![crate::ReplaySeatLifecycleEventV1 {
                event_ordinal: 0,
                replay_ordinal: 0,
                seat: 0,
                participant_instance_id: Digest32::from_bytes([12; 32]),
                lifecycle: crate::ReplaySeatLifecycleKindV1::Connected {
                    connection_epoch: 0,
                },
            }],
        }
    }

    fn verification_pair(
        category: BoardCategoryV1,
        guest_disclosure: Option<ParticipantPublicDisclosureV1>,
    ) -> (VerificationRequestV1, VerificationResultV1) {
        let campaign = category == BoardCategoryV1::Campaign;
        let (mission_id, content_edition) = if campaign {
            (
                crate::OFFICIAL_FULL_CAMPAIGN_GENESIS_MISSION_ID_V1,
                OfficialContentEditionV1::Full,
            )
        } else {
            (
                crate::OFFICIAL_DEMO_FIELD_MISSION_IDS_V1[0],
                OfficialContentEditionV1::Demo,
            )
        };
        let ranked = RankedSessionConfigV1 {
            schema_version: SCHEMA_VERSION_V1,
            mission_id: mission_id.into(),
            content_edition,
            content_subject: OfficialContentSubjectV1::FieldMission {
                mission_id: mission_id.into(),
            },
            simulation_seed: SimulationSeed64::new(42),
            starting_campaign_sha256: Digest32::from_bytes([8; 32]),
            starting_campaign_byte_length: 108,
            prepared_inputs_projection_sha256: Digest32::from_bytes([30; 32]),
            prepared_mission_inputs_seal_sha256: Digest32::from_bytes([31; 32]),
            build_manifest_sha256: Digest32::from_bytes([9; 32]),
            content_manifest_sha256: Digest32::from_bytes([3; 32]),
            campaign_content_manifest_sha256: campaign.then_some(Digest32::from_bytes([29; 32])),
            rules_config_sha256: Digest32::from_bytes([7; 32]),
            ruleset_manifest_sha256: Digest32::from_bytes([2; 32]),
            competition_manifest_sha256: None,
            spellforge_content_sha256: None,
            resource_locale_root: crate::ResourceLocaleRootV1::new("1033").unwrap(),
            speech_timing: crate::SpeechTimingAuthorityV1::BaseInstallation,
        };
        let session_genesis = crate::ReplaySessionGenesisV1 {
            claim: crate::ReplaySessionGenesisClaimV1 {
                schema_version: SCHEMA_VERSION_V1,
                network_protocol_version: crate::CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1,
                host_public_key: PublicKey32::from_bytes([4; 32]),
                replay_session_id: Digest32::from_bytes([14; 32]),
                host_participant_instance_id: Digest32::from_bytes([12; 32]),
                host_nonce: ChallengeNonce32::from_bytes([15; 32]),
                fresh_run_preflight_grant: Some(fresh_preflight_grant(&ranked, campaign)),
                campaign_continuation_preflight_grant: None,
                ranked_session: ranked,
                competition_run_grant: None,
            },
            algorithm: SignatureAlgorithmV1::Ed25519,
            host_signature: Signature64::from_bytes([16; 64]),
        };
        let session_genesis_sha256 = session_genesis.canonical_digest().unwrap();
        let mut claims = vec![host_claim()];
        if let Some(public_disclosure) = guest_disclosure {
            let anonymous_key = PublicKey32::from_bytes([0xa1; 32]);
            claims.push(ParticipantClaimV1 {
                seat: 1,
                participant_instance_id: Digest32::from_bytes([0xa4; 32]),
                public_key: anonymous_key,
                public_disclosure,
                join_attestation: Some(crate::NamedSeatJoinAttestationV1 {
                    claim: crate::NamedSeatJoinClaimV1 {
                        schema_version: SCHEMA_VERSION_V1,
                        session_genesis_sha256,
                        public_key: anonymous_key,
                        transport_endpoint_id: PublicKey32::from_bytes([0xa2; 32]),
                        host_endpoint_id: PublicKey32::from_bytes([4; 32]),
                        replay_session_id: Digest32::from_bytes([14; 32]),
                        participant_instance_id: Digest32::from_bytes([0xa4; 32]),
                        seat: 1,
                        connection_epoch: 0,
                        join_event_ordinal: 1,
                        mission_id: mission_id.into(),
                        content_manifest_sha256: Digest32::from_bytes([3; 32]),
                        rules_config_sha256: Digest32::from_bytes([7; 32]),
                        ruleset_manifest_sha256: Digest32::from_bytes([2; 32]),
                        competition_manifest_sha256: None,
                        host_nonce: ChallengeNonce32::from_bytes([15; 32]),
                    },
                    algorithm: SignatureAlgorithmV1::Ed25519,
                    signature: Signature64::from_bytes([0xa3; 64]),
                }),
            });
        }
        let participant_count = if guest_disclosure.is_some() { 2 } else { 1 };
        let events = if guest_disclosure.is_some() {
            vec![
                crate::ReplaySeatLifecycleEventV1 {
                    event_ordinal: 0,
                    replay_ordinal: 0,
                    seat: 0,
                    participant_instance_id: Digest32::from_bytes([12; 32]),
                    lifecycle: crate::ReplaySeatLifecycleKindV1::Connected {
                        connection_epoch: 0,
                    },
                },
                crate::ReplaySeatLifecycleEventV1 {
                    event_ordinal: 1,
                    replay_ordinal: 1,
                    seat: 1,
                    participant_instance_id: Digest32::from_bytes([0xa4; 32]),
                    lifecycle: crate::ReplaySeatLifecycleKindV1::Connected {
                        connection_epoch: 0,
                    },
                },
            ]
        } else {
            transcript().events
        };
        let offer = SubmissionOfferV1 {
            schema_version: SCHEMA_VERSION_V1,
            upload_challenge_id: id("upload-1"),
            upload_challenge_nonce: ChallengeNonce32::from_bytes([17; 32]),
            expires_at_unix_ms: 1,
            max_concurrent_players: participant_count,
            participant_instance_count: participant_count,
            participant_claims: claims.clone(),
            session_genesis,
            mission_id: mission_id.into(),
            competition_manifest_sha256: None,
            build_manifest_sha256: Digest32::from_bytes([9; 32]),
            content_manifest_sha256: Digest32::from_bytes([3; 32]),
            rules_config_sha256: Digest32::from_bytes([7; 32]),
            ruleset_manifest_sha256: Digest32::from_bytes([2; 32]),
            starting_state: if campaign {
                crate::InitialStateExpectationV1::CampaignGenesis {
                    template_id: id("campaign-template"),
                    campaign_state_requirement: crate::CanonicalCampaignStateRequirementV1 {
                        edition: crate::OfficialContentEditionV1::Full,
                        kind: crate::CanonicalCampaignStateKindV1::FullCampaignGenesis,
                        rules_config_sha256: Digest32::from_bytes([7; 32]),
                    },
                    campaign_sha256: Digest32::from_bytes([8; 32]),
                    starting_campaign_byte_length: 108,
                }
            } else {
                crate::InitialStateExpectationV1::IndividualLevel {
                    template_id: id("individual-template"),
                    campaign_state_requirement: crate::CanonicalCampaignStateRequirementV1 {
                        edition: crate::OfficialContentEditionV1::Demo,
                        kind: crate::CanonicalCampaignStateKindV1::IndividualTemplate,
                        rules_config_sha256: Digest32::from_bytes([7; 32]),
                    },
                    campaign_sha256: Digest32::from_bytes([8; 32]),
                    starting_campaign_byte_length: 108,
                }
            },
            allowed_metrics: vec![BoardMetricV1::OriginalScore],
        };
        let submission = crate::SubmissionEnvelopeV1 {
            schema_version: SCHEMA_VERSION_V1,
            offer,
            replay_session_transcript: crate::ReplaySessionTranscriptV1 {
                schema_version: SCHEMA_VERSION_V1,
                session_genesis_sha256,
                replay_session_id: Digest32::from_bytes([14; 32]),
                host_participant_instance_id: Digest32::from_bytes([12; 32]),
                participant_instance_count: participant_count,
                max_concurrent_players: participant_count,
                events: events.clone(),
            },
            artifacts: submission_artifacts(),
            campaign_aggregation_consent: if campaign {
                CampaignAggregationConsentV1::AuthorizeSignedSessionInServerRecognizedChainV1
            } else {
                CampaignAggregationConsentV1::NotAuthorized
            },
            campaign_continuation_authorization: None,
            requested_metrics: vec![BoardMetricV1::OriginalScore],
        };
        let request = VerificationRequestV1 {
            schema_version: SCHEMA_VERSION_V1,
            request_id: id(
                if guest_disclosure == Some(ParticipantPublicDisclosureV1::Anonymous) {
                    "verification-anonymous"
                } else {
                    "verification-1"
                },
            ),
            submission: crate::SignedSubmissionV1 {
                schema_version: SCHEMA_VERSION_V1,
                submission,
                algorithm: SignatureAlgorithmV1::Ed25519,
                participant_signatures: claims
                    .iter()
                    .map(|claim| crate::ParticipantSignatureV1 {
                        public_key: claim.public_key,
                        signature: if claim.public_disclosure
                            == ParticipantPublicDisclosureV1::Anonymous
                        {
                            Signature64::from_bytes([0xa3; 64])
                        } else {
                            Signature64::from_bytes([18; 64])
                        },
                    })
                    .collect(),
            },
            limits: VerificationLimitsV1 {
                max_input_bytes: 1024,
                max_compressed_bytes: 1024,
                max_decompressed_bytes: 4096,
                max_base64_payload_bytes: 2048,
                max_campaign_bytes: 4096,
                max_frames: 1000,
                max_version_bytes: 128,
                max_mission_id_bytes: 256,
                max_metadata_records: 64,
                max_entries_per_frame: 64,
            },
        };
        let request_sha256 = request.canonical_digest().unwrap();
        let achievement = VerifiedAchievementV1 {
            achievement_id: id("clean-hands"),
            evaluation: VerifiedAchievementEvaluationV1::Earned,
            evidence: BTreeMap::from([(
                "killed_enemies".into(),
                crate::CanonicalValue::Unsigned(0),
            )]),
        };
        let result = VerificationResultV1 {
            schema_version: SCHEMA_VERSION_V1,
            request_id: request.request_id.clone(),
            verification_request_sha256: request_sha256,
            artifacts: submission_artifacts(),
            session_genesis_sha256,
            build_manifest_sha256: Digest32::from_bytes([9; 32]),
            content_manifest_sha256: Digest32::from_bytes([3; 32]),
            rules_config_sha256: Digest32::from_bytes([7; 32]),
            ruleset_manifest_sha256: Digest32::from_bytes([2; 32]),
            competition_manifest_sha256: None,
            input_provenance: Some(InputProvenanceStatusV1::Rankable),
            status: VerificationStatusV1::Verified(VerifiedRunV1 {
                scope_kind: if campaign {
                    RunScopeKindV1::Campaign
                } else {
                    RunScopeKindV1::IndividualLevel
                },
                campaign_aggregation_consent: if campaign {
                    CampaignAggregationConsentV1::AuthorizeSignedSessionInServerRecognizedChainV1
                } else {
                    CampaignAggregationConsentV1::NotAuthorized
                },
                campaign_session_kind: campaign.then(|| CampaignSessionKindV1::FieldMission {
                    mission_id: mission_id.into(),
                }),
                campaign_session_ordinal: campaign.then_some(0),
                max_concurrent_players: participant_count,
                participant_instance_count: participant_count,
                named_participant_instance_count: 1 + u16::from(
                    guest_disclosure == Some(ParticipantPublicDisclosureV1::NamedProfile),
                ),
                anonymous_participant_instance_count: u16::from(
                    guest_disclosure == Some(ParticipantPublicDisclosureV1::Anonymous),
                ),
                authenticated_participant_claims: claims,
                replay_session_transcript: crate::ReplaySessionTranscriptV1 {
                    schema_version: SCHEMA_VERSION_V1,
                    session_genesis_sha256,
                    replay_session_id: Digest32::from_bytes([14; 32]),
                    host_participant_instance_id: Digest32::from_bytes([12; 32]),
                    participant_instance_count: participant_count,
                    max_concurrent_players: participant_count,
                    events,
                },
                outcome: TerminalOutcomeV1::Won,
                starting_campaign: campaign_artifact(8),
                final_campaign: campaign_artifact(1),
                starting_campaign_score: 0,
                final_campaign_score: 1_000,
                final_state_sha256: Digest32::from_bytes([11; 32]),
                replay_frames: 100,
                original_score_delta: 1_000,
                active_simulation_ticks: 90,
                ransom_collected: 250,
                campaign_complete_evidence: campaign.then(|| crate::CampaignCompleteEvidenceV1 {
                    schema_version: SCHEMA_VERSION_V1,
                    campaign_content_manifest_sha256: Digest32::from_bytes([29; 32]),
                    content_manifest_sha256: Digest32::from_bytes([3; 32]),
                    rules_config_sha256: Digest32::from_bytes([7; 32]),
                    ruleset_manifest_sha256: Digest32::from_bytes([2; 32]),
                    verification_request_sha256: request_sha256,
                    replay_sha256: Digest32::from_bytes([6; 32]),
                    terminal_subject: OfficialContentSubjectV1::FieldMission {
                        mission_id: mission_id.into(),
                    },
                    final_campaign_sha256: Digest32::from_bytes([1; 32]),
                    final_state_sha256: Digest32::from_bytes([11; 32]),
                    observed_progression_percent: 100,
                }),
                achievements: vec![achievement],
                diagnostics: if guest_disclosure == Some(ParticipantPublicDisclosureV1::Anonymous) {
                    BTreeMap::from([(
                        "private.anonymous_name".into(),
                        crate::CanonicalValue::String("AnonymousSentinelUsername".into()),
                    )])
                } else {
                    BTreeMap::new()
                },
            }),
        };
        request.validate().unwrap();
        result.validate().unwrap();
        (request, result)
    }

    fn sequential_named_participants_with_reconnect()
    -> (VerificationRequestV1, VerificationResultV1) {
        let (mut request, mut result) = verification_pair(
            BoardCategoryV1::IndividualLevel,
            Some(ParticipantPublicDisclosureV1::NamedProfile),
        );
        let submission = &mut request.submission.submission;
        let offer = &mut submission.offer;
        let session_genesis_sha256 = offer.session_genesis.canonical_digest().unwrap();
        let second_guest_key = PublicKey32::from_bytes([0xb1; 32]);
        let second_guest_instance = Digest32::from_bytes([0xb4; 32]);
        offer.participant_claims.push(ParticipantClaimV1 {
            seat: 1,
            participant_instance_id: second_guest_instance,
            public_key: second_guest_key,
            public_disclosure: ParticipantPublicDisclosureV1::NamedProfile,
            join_attestation: Some(crate::NamedSeatJoinAttestationV1 {
                claim: crate::NamedSeatJoinClaimV1 {
                    schema_version: SCHEMA_VERSION_V1,
                    session_genesis_sha256,
                    public_key: second_guest_key,
                    transport_endpoint_id: PublicKey32::from_bytes([0xb2; 32]),
                    host_endpoint_id: PublicKey32::from_bytes([4; 32]),
                    replay_session_id: Digest32::from_bytes([14; 32]),
                    participant_instance_id: second_guest_instance,
                    seat: 1,
                    connection_epoch: 0,
                    join_event_ordinal: 3,
                    mission_id: crate::OFFICIAL_DEMO_FIELD_MISSION_IDS_V1[0].into(),
                    content_manifest_sha256: Digest32::from_bytes([3; 32]),
                    rules_config_sha256: Digest32::from_bytes([7; 32]),
                    ruleset_manifest_sha256: Digest32::from_bytes([2; 32]),
                    competition_manifest_sha256: None,
                    host_nonce: ChallengeNonce32::from_bytes([15; 32]),
                },
                algorithm: SignatureAlgorithmV1::Ed25519,
                signature: Signature64::from_bytes([0xb3; 64]),
            }),
        });
        offer.max_concurrent_players = 2;
        offer.participant_instance_count = 3;
        request
            .submission
            .participant_signatures
            .push(crate::ParticipantSignatureV1 {
                public_key: second_guest_key,
                signature: Signature64::from_bytes([0xb3; 64]),
            });

        let VerificationStatusV1::Verified(verified) = &mut result.status else {
            unreachable!()
        };
        verified.max_concurrent_players = 2;
        verified.participant_instance_count = 3;
        verified.named_participant_instance_count = 3;
        verified.anonymous_participant_instance_count = 0;
        verified.authenticated_participant_claims = offer.participant_claims.clone();
        let first_guest_instance = offer.participant_claims[1].participant_instance_id;
        verified
            .replay_session_transcript
            .participant_instance_count = 3;
        verified.replay_session_transcript.max_concurrent_players = 2;
        verified.replay_session_transcript.events = vec![
            crate::ReplaySeatLifecycleEventV1 {
                event_ordinal: 0,
                replay_ordinal: 0,
                seat: 0,
                participant_instance_id: offer.participant_claims[0].participant_instance_id,
                lifecycle: crate::ReplaySeatLifecycleKindV1::Connected {
                    connection_epoch: 0,
                },
            },
            crate::ReplaySeatLifecycleEventV1 {
                event_ordinal: 1,
                replay_ordinal: 1,
                seat: 1,
                participant_instance_id: first_guest_instance,
                lifecycle: crate::ReplaySeatLifecycleKindV1::Connected {
                    connection_epoch: 0,
                },
            },
            crate::ReplaySeatLifecycleEventV1 {
                event_ordinal: 2,
                replay_ordinal: 2,
                seat: 1,
                participant_instance_id: first_guest_instance,
                lifecycle: crate::ReplaySeatLifecycleKindV1::Disconnected,
            },
            crate::ReplaySeatLifecycleEventV1 {
                event_ordinal: 3,
                replay_ordinal: 3,
                seat: 1,
                participant_instance_id: second_guest_instance,
                lifecycle: crate::ReplaySeatLifecycleKindV1::Connected {
                    connection_epoch: 0,
                },
            },
            crate::ReplaySeatLifecycleEventV1 {
                event_ordinal: 4,
                replay_ordinal: 4,
                seat: 1,
                participant_instance_id: second_guest_instance,
                lifecycle: crate::ReplaySeatLifecycleKindV1::Disconnected,
            },
            crate::ReplaySeatLifecycleEventV1 {
                event_ordinal: 5,
                replay_ordinal: 5,
                seat: 1,
                participant_instance_id: second_guest_instance,
                lifecycle: crate::ReplaySeatLifecycleKindV1::Connected {
                    connection_epoch: 1,
                },
            },
        ];
        request.submission.submission.replay_session_transcript =
            verified.replay_session_transcript.clone();
        result.verification_request_sha256 = request.canonical_digest().unwrap();
        request.validate().unwrap();
        result.validate().unwrap();
        (request, result)
    }

    fn rebind_private_participant_instances(
        request: &mut VerificationRequestV1,
        result: &mut VerificationResultV1,
        replacements: &[(PublicKey32, Digest32)],
    ) {
        let replacement = |key: PublicKey32| {
            replacements
                .iter()
                .find_map(|(candidate, instance)| (*candidate == key).then_some(*instance))
                .unwrap_or_else(|| panic!("missing private instance replacement for {key:?}"))
        };
        let offer = &mut request.submission.submission.offer;
        offer.session_genesis.claim.host_participant_instance_id =
            replacement(offer.session_genesis.claim.host_public_key);
        offer
            .session_genesis
            .claim
            .fresh_run_preflight_grant
            .as_mut()
            .expect("fresh fixture carries a preflight grant")
            .claim
            .host_participant_instance_id =
            offer.session_genesis.claim.host_participant_instance_id;
        let session_genesis_sha256 = offer.session_genesis.canonical_digest().unwrap();
        let old_to_new = offer
            .participant_claims
            .iter()
            .map(|claim| (claim.participant_instance_id, replacement(claim.public_key)))
            .collect::<BTreeMap<_, _>>();
        for claim in &mut offer.participant_claims {
            claim.participant_instance_id = replacement(claim.public_key);
            if let Some(attestation) = &mut claim.join_attestation {
                attestation.claim.session_genesis_sha256 = session_genesis_sha256;
                attestation.claim.participant_instance_id = claim.participant_instance_id;
            }
        }
        offer
            .participant_claims
            .sort_by_key(|claim| (claim.seat, claim.participant_instance_id));

        result.session_genesis_sha256 = session_genesis_sha256;
        let VerificationStatusV1::Verified(verified) = &mut result.status else {
            unreachable!()
        };
        for claim in &mut verified.authenticated_participant_claims {
            claim.participant_instance_id = replacement(claim.public_key);
            if let Some(attestation) = &mut claim.join_attestation {
                attestation.claim.session_genesis_sha256 = session_genesis_sha256;
                attestation.claim.participant_instance_id = claim.participant_instance_id;
            }
        }
        verified
            .authenticated_participant_claims
            .sort_by_key(|claim| (claim.seat, claim.participant_instance_id));
        verified.replay_session_transcript.session_genesis_sha256 = session_genesis_sha256;
        verified
            .replay_session_transcript
            .host_participant_instance_id =
            replacement(offer.session_genesis.claim.host_public_key);
        for event in &mut verified.replay_session_transcript.events {
            event.participant_instance_id = *old_to_new
                .get(&event.participant_instance_id)
                .expect("private transcript instance must be authenticated");
        }
        request.submission.submission.replay_session_transcript =
            verified.replay_session_transcript.clone();
        let request_sha256 = request.canonical_digest().unwrap();
        result.verification_request_sha256 = request_sha256;
        if let Some(evidence) = &mut verified.campaign_complete_evidence {
            evidence.verification_request_sha256 = request_sha256;
        }
        request.validate().unwrap();
        result.validate().unwrap();
    }

    fn run_detail(category: BoardCategoryV1) -> RunDetailV1 {
        let mission_id = if category == BoardCategoryV1::Campaign {
            crate::OFFICIAL_FULL_CAMPAIGN_GENESIS_MISSION_ID_V1
        } else {
            crate::OFFICIAL_DEMO_FIELD_MISSION_IDS_V1[0]
        };
        let achievement = VerifiedAchievementV1 {
            achievement_id: id("clean-hands"),
            evaluation: VerifiedAchievementEvaluationV1::Earned,
            evidence: BTreeMap::from([(
                "killed_enemies".into(),
                crate::CanonicalValue::Unsigned(0),
            )]),
        };
        let (verification_request, verification_result) = verification_pair(category, None);
        let verification_proof =
            PublicVerificationProofV1::from_private(&verification_request, &verification_result)
                .unwrap();
        let public_request_sha256 = verification_proof.public_request_sha256;
        let public_result_sha256 = verification_proof.canonical_digest().unwrap();
        RunDetailV1 {
            schema_version: SCHEMA_VERSION_V1,
            run_id: id("run-1"),
            rank: Some(1),
            subject: LeaderboardSubjectV1::Mission {
                mission_id: mission_id.into(),
                category,
            },
            mission: Some(MissionFacetV1 {
                mission_id: mission_id.into(),
                display_name: "Mission 1".into(),
                content_manifest_sha256: Digest32::from_bytes([3; 32]),
            }),
            composition: VerifiedRunCompositionV1::Mission {
                replay_sha256: Digest32::from_bytes([6; 32]),
            },
            outcome: TerminalOutcomeV1::Won,
            metrics: RunMetricsV1 {
                original_score_delta: 1_000,
                active_simulation_ticks: 90,
                ransom_collected: 250,
            },
            metric_value: BoardMetricValueV1::OriginalScore { points: 1_000 },
            max_concurrent_players: 1,
            participant_instance_count: 1,
            named_participant_instance_count: 1,
            named_participants: vec![PublicParticipantV1 {
                seat: 0,
                username: "Robin".into(),
                public_key: PublicKey32::from_bytes([4; 32]),
                public_key_fingerprint: PublicKey32::from_bytes([4; 32]).short_fingerprint(),
            }],
            aggregate_named_participants: Vec::new(),
            anonymous_participant_instance_count: 0,
            verified_at_unix_ms: 1,
            public_request_sha256,
            public_result_sha256,
            verification_proof: Some(verification_proof),
            campaign_aggregate: None,
            replay: Some(replay_artifact(6)),
            content: RunContentIdentityV1::Mission {
                content_manifest_sha256: Digest32::from_bytes([3; 32]),
            },
            campaign_content_manifest_sha256: (category == BoardCategoryV1::Campaign)
                .then_some(Digest32::from_bytes([29; 32])),
            rules_config_sha256: Digest32::from_bytes([7; 32]),
            ruleset_manifest_sha256: Digest32::from_bytes([2; 32]),
            competition_manifest_sha256: None,
            starting_campaign: campaign_artifact(8),
            final_campaign: campaign_artifact(1),
            starting_campaign_score: 0,
            final_campaign_score: 1_000,
            input_provenance: InputProvenanceStatusV1::Rankable,
            build: Some(PublicBuildV1 {
                manifest_sha256: Digest32::from_bytes([9; 32]),
                source_commit: "a".repeat(40),
                display_name: "Official verifier".into(),
            }),
            achievements: vec![AchievementSummaryV1 {
                display_name: "Clean Hands".into(),
                verified: PublicAchievementDecisionV1::from_private(&achievement),
            }],
            viewer: Some(ViewerLaunchV1 {
                build_manifest_sha256: Digest32::from_bytes([9; 32]),
                availability: ViewerAvailabilityV1::Available {
                    content_requirement: if category == BoardCategoryV1::Campaign {
                        ViewerContentRequirementV1::UserLocalRetail {
                            content_manifest_sha256: Digest32::from_bytes([3; 32]),
                        }
                    } else {
                        ViewerContentRequirementV1::BundledDemo {
                            content_manifest_sha256: Digest32::from_bytes([3; 32]),
                        }
                    },
                },
            }),
            full_campaign_sessions: Vec::new(),
            trust_statement: "Server-resimulated deterministic replay.".into(),
        }
    }

    fn private_result_with_anonymous_guest(
        category: BoardCategoryV1,
    ) -> (VerificationRequestV1, VerificationResultV1) {
        let (request, mut result) =
            verification_pair(category, Some(ParticipantPublicDisclosureV1::Anonymous));
        let VerificationStatusV1::Verified(verified) = &mut result.status else {
            unreachable!()
        };
        verified.achievements[0].evidence.insert(
            "private_verifier_evidence".into(),
            crate::CanonicalValue::String("AchievementEvidenceSentinel".into()),
        );
        result.validate().unwrap();
        (request, result)
    }

    fn full_campaign_detail() -> RunDetailV1 {
        let mut detail = run_detail(BoardCategoryV1::Campaign);
        let mission = detail.mission.take().unwrap();
        let completion_evidence_sha256 = detail
            .verification_proof
            .as_ref()
            .unwrap()
            .campaign_complete_evidence
            .as_ref()
            .map(|evidence| evidence.canonical_digest().unwrap());
        let session = FullCampaignSessionV1 {
            ordinal: 0,
            run_id: detail.run_id.clone(),
            session: FullCampaignSessionKindV1::FieldMission {
                mission_id: mission.mission_id.clone(),
            },
            content_subject: OfficialContentSubjectV1::FieldMission {
                mission_id: mission.mission_id.clone(),
            },
            display_name: mission.display_name.clone(),
            mission: Some(mission),
            replay: detail.replay.as_ref().unwrap().clone(),
            public_verification_request_sha256: detail.public_request_sha256,
            public_verification_result_sha256: detail.public_result_sha256,
            verification_proof: detail.verification_proof.take().unwrap(),
            starting_campaign: detail.starting_campaign.clone(),
            final_campaign: detail.final_campaign.clone(),
            starting_campaign_score: detail.starting_campaign_score,
            final_campaign_score: detail.final_campaign_score,
            public_campaign_complete_evidence_sha256: completion_evidence_sha256,
            content_manifest_sha256: detail.content.digest(),
            rules_config_sha256: detail.rules_config_sha256,
            ruleset_manifest_sha256: detail.ruleset_manifest_sha256,
            competition_manifest_sha256: detail.competition_manifest_sha256,
            max_concurrent_players: 1,
            participant_instance_count: 1,
            named_participant_instance_count: 1,
            anonymous_participant_instance_count: 0,
            named_participants: detail.named_participants.clone(),
            input_provenance: InputProvenanceStatusV1::Rankable,
            metrics: detail.metrics.clone(),
            achievements: detail.achievements.clone(),
            build: detail.build.take().unwrap(),
            viewer: detail.viewer.take().unwrap(),
        };
        detail.run_id = id("full-campaign-1");
        detail.subject = LeaderboardSubjectV1::FullCampaign;
        detail.achievements.clear();
        let mission_identity = detail.named_participants.pop().unwrap();
        detail.aggregate_named_participants = vec![AggregatePublicParticipantV1 {
            current_display_name: mission_identity.username,
            public_key: mission_identity.public_key,
            public_key_fingerprint: mission_identity.public_key_fingerprint,
        }];
        detail.composition = VerifiedRunCompositionV1::FullCampaign {
            ordered_session_run_ids: vec![session.run_id.clone()],
        };
        detail.replay = None;
        detail.content = RunContentIdentityV1::FullCampaign {
            campaign_content_manifest_sha256: Digest32::from_bytes([29; 32]),
        };
        detail.campaign_content_manifest_sha256 = None;
        let private_completion_evidence_sha256 = Digest32::from_bytes([44; 32]);
        let aggregate = VerifiedCampaignAggregateV1 {
            schema_version: SCHEMA_VERSION_V1,
            aggregate_request_sha256: Digest32::from_bytes([45; 32]),
            chain_id: id("campaign-1"),
            full_campaign_run_id: detail.run_id.clone(),
            campaign_complete_terminal_run_id: session.run_id.clone(),
            campaign_complete_evidence_sha256: private_completion_evidence_sha256,
            sessions: vec![crate::VerifiedCampaignSessionV1 {
                ordinal: session.ordinal,
                run_id: session.run_id.clone(),
                kind: CampaignSessionKindV1::FieldMission {
                    mission_id: session.content_subject.mission_id().into(),
                },
                content_subject: session.content_subject.clone(),
                campaign_aggregation_consent:
                    CampaignAggregationConsentV1::AuthorizeSignedSessionInServerRecognizedChainV1,
                replay: session.replay.clone(),
                build_manifest_sha256: session.build.manifest_sha256,
                content_manifest_sha256: session.content_manifest_sha256,
                rules_config_sha256: session.rules_config_sha256,
                ruleset_manifest_sha256: session.ruleset_manifest_sha256,
                competition_manifest_sha256: session.competition_manifest_sha256,
                verification_request_sha256: Digest32::from_bytes([46; 32]),
                verification_result_sha256: Digest32::from_bytes([47; 32]),
                starting_campaign: session.starting_campaign.clone(),
                final_campaign: session.final_campaign.clone(),
                starting_campaign_score: session.starting_campaign_score,
                final_campaign_score: session.final_campaign_score,
                max_concurrent_players: session.max_concurrent_players,
                participant_instance_count: session.participant_instance_count,
                named_participant_instance_count: session.named_participant_instance_count,
                anonymous_participant_instance_count: session.anonymous_participant_instance_count,
                authenticated_participant_keys: session
                    .named_participants
                    .iter()
                    .map(|participant| participant.public_key)
                    .collect(),
                active_simulation_ticks: session.metrics.active_simulation_ticks,
                ransom_collected: session.metrics.ransom_collected,
                campaign_complete_evidence_sha256: Some(private_completion_evidence_sha256),
            }],
            max_concurrent_players: detail.max_concurrent_players,
            participant_instance_count: detail.participant_instance_count,
            named_participant_instance_count: detail.named_participant_instance_count,
            anonymous_participant_instance_count: detail.anonymous_participant_instance_count,
            authenticated_participant_keys: session
                .named_participants
                .iter()
                .map(|participant| participant.public_key)
                .collect(),
            campaign_controller_public_key: PublicKey32::from_bytes([4; 32]),
            campaign_content_manifest_sha256: detail.content.digest(),
            rules_config_sha256: detail.rules_config_sha256,
            ruleset_manifest_sha256: detail.ruleset_manifest_sha256,
            competition_manifest_sha256: detail.competition_manifest_sha256,
            canonical_genesis_campaign: detail.starting_campaign.clone(),
            final_campaign: detail.final_campaign.clone(),
            starting_campaign_score: detail.starting_campaign_score,
            final_campaign_score: detail.final_campaign_score,
            active_simulation_ticks: detail.metrics.active_simulation_ticks,
            ransom_collected: detail.metrics.ransom_collected,
        };
        let aggregate_proof = PublicCampaignAggregateProofV1::from_private(
            &aggregate,
            std::slice::from_ref(&session.verification_proof),
        )
        .unwrap();
        detail.public_request_sha256 = aggregate_proof.public_request_sha256;
        detail.public_result_sha256 = aggregate_proof.canonical_digest().unwrap();
        detail.campaign_aggregate = Some(aggregate_proof);
        detail.full_campaign_sessions = vec![session];
        detail
    }

    fn leaderboard_filter(metric: BoardMetricV1) -> RunFilterV1 {
        RunFilterV1 {
            schema_version: SCHEMA_VERSION_V1,
            subject: LeaderboardSubjectV1::Mission {
                mission_id: "mission_1".into(),
                category: BoardCategoryV1::IndividualLevel,
            },
            metric,
            content: RunContentIdentityV1::Mission {
                content_manifest_sha256: Digest32::from_bytes([1; 32]),
            },
            rules_config_sha256: Digest32::from_bytes([2; 32]),
            ruleset_manifest_sha256: Digest32::from_bytes([3; 32]),
            competition_manifest_sha256: None,
            max_concurrent_players: Some(1),
            player_public_key: None,
        }
    }

    fn leaderboard_entry(
        position: u64,
        rank: u64,
        points: i64,
        accepted_sequence: u64,
    ) -> LeaderboardEntryV1 {
        let key = PublicKey32::from_bytes([4; 32]);
        LeaderboardEntryV1 {
            position,
            rank,
            run_id: id(&format!("run-{position}")),
            composition: VerifiedRunCompositionV1::Mission {
                replay_sha256: Digest32::from_bytes([6; 32]),
            },
            metric_value: BoardMetricValueV1::OriginalScore { points },
            max_concurrent_players: 1,
            participant_instance_count: 1,
            named_participant_instance_count: 1,
            named_participants: vec![PublicParticipantV1 {
                seat: 0,
                username: "Robin".into(),
                public_key: key,
                public_key_fingerprint: key.short_fingerprint(),
            }],
            aggregate_named_participants: Vec::new(),
            anonymous_participant_instance_count: 0,
            accepted_sequence,
            verified_at_unix_ms: 1_800_000_000_000 + accepted_sequence,
        }
    }

    fn competition_manifest() -> CompetitionManifestV1 {
        CompetitionManifestV1 {
            schema_version: SCHEMA_VERSION_V1,
            competition_id: id("daily-mission-1"),
            competition_version: 1,
            display_name: "Daily Mission 1".into(),
            description: "A pinned-seed daily board.".into(),
            subject: LeaderboardSubjectV1::Mission {
                mission_id: "mission_1".into(),
                category: BoardCategoryV1::IndividualLevel,
            },
            metric: BoardMetricV1::OriginalScore,
            rules_config_sha256: Digest32::from_bytes([2; 32]),
            ruleset_manifest_sha256: Digest32::from_bytes([3; 32]),
            canonical_campaign_state: crate::CanonicalCampaignStateRequirementV1 {
                edition: crate::OfficialContentEditionV1::Demo,
                kind: crate::CanonicalCampaignStateKindV1::IndividualTemplate,
                rules_config_sha256: Digest32::from_bytes([2; 32]),
            },
            content: RunContentIdentityV1::Mission {
                content_manifest_sha256: Digest32::from_bytes([1; 32]),
            },
            seed_policy: CompetitionSeedPolicyV1::Pinned {
                simulation_seed: SimulationSeed64::new(42),
            },
            participant_composition: CompetitionParticipantCompositionV1::SinglePlayer,
            competition_run_grant_public_key: PublicKey32::from_bytes([7; 32]),
            starts_at_unix_ms: 1_800_000_000_000,
            ends_at_unix_ms: 1_800_086_400_000,
        }
    }

    #[test]
    fn public_run_detail_cannot_serialize_private_campaign_receipt() {
        let detail = run_detail(BoardCategoryV1::Campaign);
        assert!(detail.validate().is_ok());
        let json = serde_json::to_string(&detail).unwrap();
        assert!(!json.contains("campaign_chain_receipt"));
        for legacy_field in [
            "private_replay",
            "public_replay",
            "private_initial_campaign",
            "public_initial_campaign",
            "private_final_campaign",
            "public_final_campaign",
            "private_final_state_sha256",
            "public_final_state_sha256",
        ] {
            assert!(!json.contains(legacy_field));
        }
        assert!(json.contains(&"06".repeat(32)));
    }

    #[test]
    fn full_campaign_has_ordered_session_replays_but_no_synthetic_replay() {
        let mut detail = full_campaign_detail();
        assert!(detail.validate().is_ok());
        assert!(detail.replay.is_none());
        assert!(detail.build.is_none());
        assert!(detail.viewer.is_none());

        detail.full_campaign_sessions[0].run_id = id("substituted-session");
        assert!(matches!(
            detail.validate(),
            Err(ValidationError::ClaimMismatch {
                field: "run.subject_artifacts"
            })
        ));
    }

    #[test]
    fn aggregate_session_detail_is_ordinal_and_exact_artifact_bound() {
        let aggregate = full_campaign_detail();
        let detail = CampaignSessionDetailV1 {
            schema_version: SCHEMA_VERSION_V1,
            aggregate_run_id: aggregate.run_id.clone(),
            public_aggregate_result_sha256: aggregate.public_result_sha256,
            ordinal: 0,
            session: aggregate.full_campaign_sessions[0].clone(),
        };
        assert!(detail.validate_against_aggregate(&aggregate).is_ok());

        let mut wrong_ordinal = detail.clone();
        wrong_ordinal.ordinal = 1;
        assert!(
            wrong_ordinal
                .validate_against_aggregate(&aggregate)
                .is_err()
        );

        let mut wrong_length = detail;
        wrong_length.session.replay.artifact.byte_length += 1;
        assert!(wrong_length.validate_against_aggregate(&aggregate).is_err());
    }

    #[test]
    fn campaign_public_fields_cannot_diverge_from_typed_verifier_result() {
        let mut wrong_start_score = full_campaign_detail();
        wrong_start_score.full_campaign_sessions[0].starting_campaign_score -= 1;
        assert!(wrong_start_score.validate().is_err());

        let mut wrong_kind = full_campaign_detail();
        wrong_kind.full_campaign_sessions[0].session =
            FullCampaignSessionKindV1::Headquarters { hq_sequence: 1 };
        wrong_kind.full_campaign_sessions[0].mission = None;
        assert!(wrong_kind.validate().is_err());

        let mut wrong_completion = full_campaign_detail();
        wrong_completion.full_campaign_sessions[0].public_campaign_complete_evidence_sha256 =
            Some(Digest32::from_bytes([99; 32]));
        assert!(wrong_completion.validate().is_err());

        let mut wrong_achievement = run_detail(BoardCategoryV1::Campaign);
        wrong_achievement.achievements[0].verified.evaluation =
            VerifiedAchievementEvaluationV1::NotEarned;
        assert!(wrong_achievement.validate().is_err());
    }

    #[test]
    fn public_achievement_summary_preserves_unverifiable_without_awarding_it() {
        let mut detail = run_detail(BoardCategoryV1::IndividualLevel);
        detail.achievements[0].verified.evaluation = VerifiedAchievementEvaluationV1::Unverifiable;
        detail
            .verification_proof
            .as_mut()
            .expect("fixture must carry its verifier proof")
            .achievements[0]
            .evaluation = VerifiedAchievementEvaluationV1::Unverifiable;
        detail.public_result_sha256 = detail
            .verification_proof
            .as_ref()
            .unwrap()
            .canonical_digest()
            .unwrap();

        let validation = detail.validate();
        assert!(validation.is_ok(), "{validation:?}");
        assert!(!detail.achievements[0].verified.is_awarded());
        let canonical = detail.canonical_bytes().unwrap();
        assert!(
            canonical
                .windows(b"unverifiable".len())
                .any(|window| window == b"unverifiable")
        );
    }

    #[test]
    fn public_proof_rejects_noncanonical_achievements_and_campaign_bindings() {
        let detail = run_detail(BoardCategoryV1::IndividualLevel);
        let proof = detail.verification_proof.unwrap();

        let mut duplicate = proof.clone();
        duplicate
            .achievements
            .push(duplicate.achievements[0].clone());
        assert!(matches!(
            duplicate.validate(),
            Err(ValidationError::NotCanonicalOrder {
                field: "public_verification_proof.achievements"
            })
        ));

        let mut reordered = proof;
        reordered.achievements.push(PublicAchievementDecisionV1 {
            achievement_id: id("aaa-before-clean-hands"),
            evaluation: VerifiedAchievementEvaluationV1::NotEarned,
        });
        assert!(matches!(
            reordered.validate(),
            Err(ValidationError::NotCanonicalOrder {
                field: "public_verification_proof.achievements"
            })
        ));

        let detail = full_campaign_detail();
        let mut duplicate_binding = detail.campaign_aggregate.unwrap();
        let mut second = duplicate_binding.public_request.sessions[0].clone();
        second.ordinal = 1;
        duplicate_binding.public_request.sessions.push(second);
        assert!(matches!(
            duplicate_binding.validate(),
            Err(ValidationError::ClaimMismatch {
                field: "public_campaign_aggregate_request.structure"
            })
        ));
    }

    #[test]
    fn public_projection_digests_are_canonical_and_private_fields_are_unknown() {
        let detail = run_detail(BoardCategoryV1::Campaign);
        let proof = detail.verification_proof.unwrap();
        assert_eq!(
            proof.public_request.canonical_digest().unwrap(),
            proof.public_request_sha256
        );
        assert_eq!(
            proof.canonical_digest().unwrap(),
            detail.public_result_sha256
        );

        let mut unknown_request = serde_json::to_value(&proof.public_request).unwrap();
        unknown_request
            .as_object_mut()
            .unwrap()
            .insert("request_id".into(), serde_json::json!("private-request"));
        assert!(serde_json::from_value::<PublicVerificationRequestV1>(unknown_request).is_err());

        let mut unknown_proof = serde_json::to_value(&proof).unwrap();
        unknown_proof.as_object_mut().unwrap().insert(
            "verification_result_sha256".into(),
            serde_json::json!("aa".repeat(32)),
        );
        assert!(serde_json::from_value::<PublicVerificationProofV1>(unknown_proof).is_err());

        let mut substituted = proof.clone();
        substituted.public_request.simulation_seed = SimulationSeed64::new(43);
        assert!(matches!(
            substituted.validate(),
            Err(ValidationError::ClaimMismatch {
                field: "public_verification_proof.public_request_sha256"
            })
        ));

        let mut duplicate_named = proof;
        duplicate_named
            .public_request
            .named_participants
            .push(duplicate_named.public_request.named_participants[0].clone());
        duplicate_named
            .public_request
            .named_participant_instance_count = 2;
        duplicate_named.public_request.participant_instance_count = 2;
        assert!(matches!(
            duplicate_named.public_request.validate(),
            Err(ValidationError::InvalidParticipantClaims)
        ));

        let aggregate = full_campaign_detail();
        let aggregate_proof = aggregate.campaign_aggregate.unwrap();
        assert_eq!(
            aggregate_proof.public_request.canonical_digest().unwrap(),
            aggregate_proof.public_request_sha256
        );
        assert_eq!(
            aggregate_proof.canonical_digest().unwrap(),
            aggregate.public_result_sha256
        );
        let mut unknown_aggregate = serde_json::to_value(&aggregate_proof).unwrap();
        unknown_aggregate.as_object_mut().unwrap().insert(
            "aggregate_result_sha256".into(),
            serde_json::json!("bb".repeat(32)),
        );
        assert!(
            serde_json::from_value::<PublicCampaignAggregateProofV1>(unknown_aggregate).is_err()
        );
    }

    #[test]
    fn signed_genesis_binds_single_named_and_anonymous_public_projection_chains() {
        for disclosure in [
            None,
            Some(ParticipantPublicDisclosureV1::NamedProfile),
            Some(ParticipantPublicDisclosureV1::Anonymous),
        ] {
            let (request, result) = verification_pair(BoardCategoryV1::IndividualLevel, disclosure);
            let signed_genesis_sha256 = request
                .submission
                .submission
                .offer
                .session_genesis
                .canonical_digest()
                .unwrap();
            let VerificationStatusV1::Verified(verified) = &result.status else {
                unreachable!()
            };
            assert_eq!(result.session_genesis_sha256, signed_genesis_sha256);
            assert_eq!(
                verified.replay_session_transcript.session_genesis_sha256,
                signed_genesis_sha256
            );
            let proof = PublicVerificationProofV1::from_private(&request, &result).unwrap();
            assert_eq!(
                proof.public_request.named_participant_instance_count,
                if disclosure == Some(ParticipantPublicDisclosureV1::NamedProfile) {
                    2
                } else {
                    1
                }
            );
            assert_eq!(
                proof.public_request.anonymous_participant_instance_count,
                u16::from(disclosure == Some(ParticipantPublicDisclosureV1::Anonymous))
            );
        }

        let (request, mut claim_only) = verification_pair(BoardCategoryV1::IndividualLevel, None);
        let claim_sha256 = request
            .submission
            .submission
            .offer
            .session_genesis
            .claim
            .canonical_digest()
            .unwrap();
        let VerificationStatusV1::Verified(verified) = &mut claim_only.status else {
            unreachable!()
        };
        claim_only.session_genesis_sha256 = claim_sha256;
        verified.replay_session_transcript.session_genesis_sha256 = claim_sha256;
        assert!(claim_only.validate().is_ok());
        assert!(PublicVerificationProofV1::from_private(&request, &claim_only).is_err());

        let (mut substituted_request, result) =
            verification_pair(BoardCategoryV1::IndividualLevel, None);
        substituted_request
            .submission
            .submission
            .offer
            .session_genesis
            .host_signature = Signature64::from_bytes([0xfe; 64]);
        substituted_request
            .submission
            .submission
            .replay_session_transcript
            .session_genesis_sha256 = substituted_request
            .submission
            .submission
            .offer
            .session_genesis
            .canonical_digest()
            .unwrap();
        assert!(substituted_request.validate().is_ok());
        assert!(PublicVerificationProofV1::from_private(&substituted_request, &result).is_err());
    }

    #[test]
    fn public_proof_is_invariant_to_private_instance_ids_across_sequential_keys_and_reconnect() {
        let (request, result) = sequential_named_participants_with_reconnect();
        let baseline_private_request_sha256 = request.canonical_digest().unwrap();
        let baseline_private_result_sha256 = result.canonical_digest().unwrap();
        let baseline = PublicVerificationProofV1::from_private(&request, &result).unwrap();
        assert_eq!(
            baseline
                .public_request
                .named_participants
                .iter()
                .map(|participant| (participant.seat, participant.public_key))
                .collect::<Vec<_>>(),
            vec![
                (0, PublicKey32::from_bytes([4; 32])),
                (1, PublicKey32::from_bytes([0xa1; 32])),
                (1, PublicKey32::from_bytes([0xb1; 32])),
            ],
            "sequential named keys on one seat must remain publicly distinguishable"
        );

        let mut mutated_request = request;
        let mut mutated_result = result;
        let replacements = [
            (
                PublicKey32::from_bytes([4; 32]),
                Digest32::from_bytes([0xd1; 32]),
            ),
            (
                PublicKey32::from_bytes([0xa1; 32]),
                Digest32::from_bytes([0xd3; 32]),
            ),
            (
                PublicKey32::from_bytes([0xb1; 32]),
                Digest32::from_bytes([0xd2; 32]),
            ),
        ];
        rebind_private_participant_instances(
            &mut mutated_request,
            &mut mutated_result,
            &replacements,
        );
        assert_ne!(
            mutated_request.canonical_digest().unwrap(),
            baseline_private_request_sha256
        );
        assert_ne!(
            mutated_result.canonical_digest().unwrap(),
            baseline_private_result_sha256
        );

        let mutated =
            PublicVerificationProofV1::from_private(&mutated_request, &mutated_result).unwrap();
        assert_eq!(
            baseline.public_request.canonical_bytes().unwrap(),
            mutated.public_request.canonical_bytes().unwrap(),
        );
        assert_eq!(
            baseline.public_request.canonical_digest().unwrap(),
            mutated.public_request.canonical_digest().unwrap(),
        );
        assert_eq!(
            baseline.canonical_bytes().unwrap(),
            mutated.canonical_bytes().unwrap(),
        );
        assert_eq!(
            baseline.canonical_digest().unwrap(),
            mutated.canonical_digest().unwrap(),
        );

        let public_value = serde_json::to_value(&mutated).unwrap();
        fn assert_instance_ids_absent(value: &serde_json::Value) {
            match value {
                serde_json::Value::Array(values) => {
                    values.iter().for_each(assert_instance_ids_absent)
                }
                serde_json::Value::Object(values) => {
                    assert!(!values.contains_key("participant_instance_id"));
                    values.values().for_each(assert_instance_ids_absent);
                }
                _ => {}
            }
        }
        assert_instance_ids_absent(&public_value);
        let public_bytes = serde_json::to_vec(&public_value).unwrap();
        for (_, private_instance) in replacements {
            assert!(
                !String::from_utf8_lossy(&public_bytes).contains(&private_instance.to_string())
            );
        }
    }

    #[test]
    fn public_run_and_campaign_detail_recursively_redact_anonymous_authentication() {
        let (mut individual_request, mut raw_individual) =
            private_result_with_anonymous_guest(BoardCategoryV1::IndividualLevel);
        let individual_private_instances = [
            (
                PublicKey32::from_bytes([4; 32]),
                Digest32::from_bytes([0xd4; 32]),
            ),
            (
                PublicKey32::from_bytes([0xa1; 32]),
                Digest32::from_bytes([0xd5; 32]),
            ),
        ];
        rebind_private_participant_instances(
            &mut individual_request,
            &mut raw_individual,
            &individual_private_instances,
        );
        raw_individual.validate().unwrap();
        let private_json = serde_json::to_string(&raw_individual).unwrap();
        let round_tripped: VerificationResultV1 = serde_json::from_str(&private_json).unwrap();
        round_tripped.validate().unwrap();
        assert_eq!(
            serde_json::to_string(&round_tripped).unwrap(),
            private_json,
            "the private authoritative artifact JSON must round-trip exactly"
        );

        let anonymous_key = "a1".repeat(32);
        let transport_key = "a2".repeat(32);
        let anonymous_signature = "a3".repeat(64);
        for sentinel in [
            anonymous_key.as_str(),
            transport_key.as_str(),
            anonymous_signature.as_str(),
            "AnonymousSentinelUsername",
            "AchievementEvidenceSentinel",
        ] {
            assert!(private_json.contains(sentinel));
        }
        for (_, private_instance) in &individual_private_instances {
            assert!(private_json.contains(&private_instance.to_string()));
        }

        let mut public_run = run_detail(BoardCategoryV1::IndividualLevel);
        let proof =
            PublicVerificationProofV1::from_private(&individual_request, &raw_individual).unwrap();
        public_run.public_request_sha256 = proof.public_request_sha256;
        public_run.public_result_sha256 = proof.canonical_digest().unwrap();
        public_run.verification_proof = Some(proof);
        public_run.max_concurrent_players = 2;
        public_run.participant_instance_count = 2;
        public_run.named_participant_instance_count = 1;
        public_run.anonymous_participant_instance_count = 1;
        public_run.validate().unwrap();

        let (mut campaign_request, mut raw_campaign) =
            private_result_with_anonymous_guest(BoardCategoryV1::Campaign);
        let campaign_private_instances = [
            (
                PublicKey32::from_bytes([4; 32]),
                Digest32::from_bytes([0xe1; 32]),
            ),
            (
                PublicKey32::from_bytes([0xa1; 32]),
                Digest32::from_bytes([0xe2; 32]),
            ),
        ];
        rebind_private_participant_instances(
            &mut campaign_request,
            &mut raw_campaign,
            &campaign_private_instances,
        );
        raw_campaign.validate().unwrap();
        let campaign_proof =
            PublicVerificationProofV1::from_private(&campaign_request, &raw_campaign).unwrap();
        let mut aggregate = full_campaign_detail();
        let mut session = aggregate.full_campaign_sessions.remove(0);
        session.public_verification_request_sha256 = campaign_proof.public_request_sha256;
        session.public_verification_result_sha256 = campaign_proof.canonical_digest().unwrap();
        session.public_campaign_complete_evidence_sha256 = campaign_proof
            .campaign_complete_evidence
            .as_ref()
            .map(|evidence| evidence.canonical_digest().unwrap());
        session.verification_proof = campaign_proof;
        session.max_concurrent_players = 2;
        session.participant_instance_count = 2;
        session.named_participant_instance_count = 1;
        session.anonymous_participant_instance_count = 1;
        let campaign_detail = CampaignSessionDetailV1 {
            schema_version: SCHEMA_VERSION_V1,
            aggregate_run_id: aggregate.run_id,
            public_aggregate_result_sha256: aggregate.public_result_sha256,
            ordinal: 0,
            session,
        };
        campaign_detail.validate().unwrap();

        let mut full_campaign = full_campaign_detail();
        full_campaign.full_campaign_sessions[0] = campaign_detail.session.clone();
        full_campaign.max_concurrent_players = 2;
        full_campaign.participant_instance_count = 2;
        full_campaign.named_participant_instance_count = 1;
        full_campaign.anonymous_participant_instance_count = 1;
        let private_aggregate = VerifiedCampaignAggregateV1 {
            schema_version: SCHEMA_VERSION_V1,
            aggregate_request_sha256: Digest32::from_bytes([45; 32]),
            chain_id: id("private-chain-sentinel-do-not-publish"),
            full_campaign_run_id: full_campaign.run_id.clone(),
            campaign_complete_terminal_run_id: campaign_detail.session.run_id.clone(),
            campaign_complete_evidence_sha256: Digest32::from_bytes([44; 32]),
            sessions: vec![crate::VerifiedCampaignSessionV1 {
                ordinal: 0,
                run_id: campaign_detail.session.run_id.clone(),
                kind: CampaignSessionKindV1::FieldMission {
                    mission_id: campaign_detail.session.content_subject.mission_id().into(),
                },
                content_subject: campaign_detail.session.content_subject.clone(),
                campaign_aggregation_consent:
                    CampaignAggregationConsentV1::AuthorizeSignedSessionInServerRecognizedChainV1,
                replay: campaign_detail.session.replay.clone(),
                build_manifest_sha256: campaign_detail.session.build.manifest_sha256,
                content_manifest_sha256: campaign_detail.session.content_manifest_sha256,
                rules_config_sha256: campaign_detail.session.rules_config_sha256,
                ruleset_manifest_sha256: campaign_detail.session.ruleset_manifest_sha256,
                competition_manifest_sha256: campaign_detail.session.competition_manifest_sha256,
                verification_request_sha256: Digest32::from_bytes([46; 32]),
                verification_result_sha256: Digest32::from_bytes([47; 32]),
                starting_campaign: campaign_detail.session.starting_campaign.clone(),
                final_campaign: campaign_detail.session.final_campaign.clone(),
                starting_campaign_score: campaign_detail.session.starting_campaign_score,
                final_campaign_score: campaign_detail.session.final_campaign_score,
                max_concurrent_players: 2,
                participant_instance_count: 2,
                named_participant_instance_count: 1,
                anonymous_participant_instance_count: 1,
                authenticated_participant_keys: vec![
                    PublicKey32::from_bytes([4; 32]),
                    PublicKey32::from_bytes([0xa1; 32]),
                ],
                active_simulation_ticks: campaign_detail.session.metrics.active_simulation_ticks,
                ransom_collected: campaign_detail.session.metrics.ransom_collected,
                campaign_complete_evidence_sha256: Some(Digest32::from_bytes([44; 32])),
            }],
            max_concurrent_players: 2,
            participant_instance_count: 2,
            named_participant_instance_count: 1,
            anonymous_participant_instance_count: 1,
            authenticated_participant_keys: vec![
                PublicKey32::from_bytes([4; 32]),
                PublicKey32::from_bytes([0xa1; 32]),
            ],
            campaign_controller_public_key: PublicKey32::from_bytes([4; 32]),
            campaign_content_manifest_sha256: full_campaign.content.digest(),
            rules_config_sha256: full_campaign.rules_config_sha256,
            ruleset_manifest_sha256: full_campaign.ruleset_manifest_sha256,
            competition_manifest_sha256: full_campaign.competition_manifest_sha256,
            canonical_genesis_campaign: full_campaign.starting_campaign.clone(),
            final_campaign: full_campaign.final_campaign.clone(),
            starting_campaign_score: full_campaign.starting_campaign_score,
            final_campaign_score: full_campaign.final_campaign_score,
            active_simulation_ticks: full_campaign.metrics.active_simulation_ticks,
            ransom_collected: full_campaign.metrics.ransom_collected,
        };
        let public_aggregate = PublicCampaignAggregateProofV1::from_private(
            &private_aggregate,
            std::slice::from_ref(&campaign_detail.session.verification_proof),
        )
        .unwrap();
        let mut private_only_mutation = private_aggregate.clone();
        private_only_mutation.chain_id = id("second-private-chain-sentinel");
        private_only_mutation.authenticated_participant_keys[1] =
            PublicKey32::from_bytes([0xa2; 32]);
        private_only_mutation.sessions[0].authenticated_participant_keys[1] =
            PublicKey32::from_bytes([0xa2; 32]);
        private_only_mutation.validate().unwrap();
        let mutation_projection = PublicCampaignAggregateProofV1::from_private(
            &private_only_mutation,
            std::slice::from_ref(&campaign_detail.session.verification_proof),
        )
        .unwrap();
        assert_eq!(
            crate::canonical_json_bytes(&mutation_projection).unwrap(),
            crate::canonical_json_bytes(&public_aggregate).unwrap(),
            "private anonymous identities must not affect public bytes"
        );
        let mut campaign_mutation = private_aggregate.clone();
        campaign_mutation.canonical_genesis_campaign = campaign_artifact(9);
        campaign_mutation.sessions[0].starting_campaign = campaign_artifact(9);
        campaign_mutation.final_campaign = campaign_artifact(10);
        campaign_mutation.sessions[0].final_campaign = campaign_artifact(10);
        campaign_mutation.validate().unwrap();
        assert!(
            PublicCampaignAggregateProofV1::from_private(
                &campaign_mutation,
                std::slice::from_ref(&campaign_detail.session.verification_proof),
            )
            .is_err(),
            "canonical campaign artifacts must remain bound to the session proof"
        );
        full_campaign.public_request_sha256 = public_aggregate.public_request_sha256;
        full_campaign.public_result_sha256 = public_aggregate.canonical_digest().unwrap();
        full_campaign.campaign_aggregate = Some(public_aggregate);
        full_campaign.validate().unwrap();

        let private_digest_sentinels = [
            individual_request.canonical_digest().unwrap().to_string(),
            raw_individual.canonical_digest().unwrap().to_string(),
            campaign_request.canonical_digest().unwrap().to_string(),
            raw_campaign.canonical_digest().unwrap().to_string(),
            private_aggregate.canonical_digest().unwrap().to_string(),
            private_aggregate.aggregate_request_sha256.to_string(),
            private_aggregate
                .campaign_complete_evidence_sha256
                .to_string(),
            private_aggregate.chain_id.as_str().to_owned(),
            private_only_mutation.chain_id.as_str().to_owned(),
        ];
        let forbidden_exact_keys = BTreeSet::from([
            "request_id",
            "verification_request_sha256",
            "verification_result_sha256",
            "aggregate_request_sha256",
            "aggregate_result_sha256",
            "chain_id",
            "participant_claims",
            "participant_instance_id",
            "authenticated_participant_claims",
            "authenticated_participant_keys",
            "campaign_controller_public_key",
            "private_replay",
            "public_replay",
            "private_starting_campaign",
            "public_starting_campaign",
            "private_final_campaign",
            "public_final_campaign",
            "private_final_state_sha256",
            "public_final_state_sha256",
            "replay_session_transcript",
            "replay_session_id",
            "canonical_campaign_state",
            "campaign_state_requirement",
            "canonical_campaign_state_json",
            "transport_endpoint_id",
            "join_attestation",
            "signature",
        ]);
        fn assert_no_forbidden_keys(value: &serde_json::Value, forbidden: &BTreeSet<&str>) {
            match value {
                serde_json::Value::Array(values) => {
                    for value in values {
                        assert_no_forbidden_keys(value, forbidden);
                    }
                }
                serde_json::Value::Object(values) => {
                    for (key, value) in values {
                        assert!(
                            !forbidden.contains(key.as_str()),
                            "private key leaked: {key}"
                        );
                        assert_no_forbidden_keys(value, forbidden);
                    }
                }
                _ => {}
            }
        }

        for public_json in [
            serde_json::to_string(&public_run).unwrap(),
            serde_json::to_string(&campaign_detail).unwrap(),
            serde_json::to_string(&full_campaign).unwrap(),
        ] {
            assert_no_forbidden_keys(
                &serde_json::from_str(&public_json).unwrap(),
                &forbidden_exact_keys,
            );
            for sentinel in &private_digest_sentinels {
                assert!(
                    !public_json.contains(sentinel),
                    "public proof committed a private digest sentinel `{sentinel}`"
                );
            }
            for sentinel in [
                anonymous_key.as_str(),
                transport_key.as_str(),
                anonymous_signature.as_str(),
                "AnonymousSentinelUsername",
                "AchievementEvidenceSentinel",
                "participant_claims",
                "authenticated_participant_keys",
                "campaign_controller_public_key",
                "campaign_chain_receipt",
                "expected_starting_campaign",
                "private_replay",
                "public_replay",
                "private_starting_campaign",
                "public_starting_campaign",
                "private_final_campaign",
                "public_final_campaign",
                "private_final_state_sha256",
                "public_final_state_sha256",
                "replay_session_transcript",
                "replay_session_id",
                "canonical_campaign_state",
                "campaign_state_requirement",
                "canonical_campaign_state_json",
                "transport_endpoint_id",
                "join_attestation",
                "signature",
                "verification-anonymous",
                "private-chain-sentinel-do-not-publish",
                "second-private-chain-sentinel",
            ] {
                assert!(
                    !public_json.contains(sentinel),
                    "public proof leaked private sentinel `{sentinel}`: {public_json}"
                );
            }
            for (_, private_instance) in individual_private_instances
                .iter()
                .chain(&campaign_private_instances)
            {
                assert!(
                    !public_json.contains(&private_instance.to_string()),
                    "public API body leaked private participant instance {private_instance}"
                );
            }
        }
    }

    #[test]
    fn aggregate_roster_represents_sequential_seat_owners_without_fake_seats() {
        let first_key = PublicKey32::from_bytes([4; 32]);
        let second_key = PublicKey32::from_bytes([5; 32]);
        let identities = vec![
            AggregatePublicParticipantV1 {
                current_display_name: "Robin".into(),
                public_key: first_key,
                public_key_fingerprint: first_key.short_fingerprint(),
            },
            AggregatePublicParticipantV1 {
                current_display_name: "Marian".into(),
                public_key: second_key,
                public_key_fingerprint: second_key.short_fingerprint(),
            },
        ];
        assert!(validate_public_roster(2, 3, 2, &[], &identities, 1, false).is_ok());

        // A durable key may truthfully own multiple replay-scoped instances;
        // the aggregate identity remains one key while the instance count is 2.
        assert!(validate_public_roster(1, 2, 2, &[], &identities[..1], 0, false).is_ok());

        let mut noncanonical = identities;
        noncanonical.swap(0, 1);
        assert!(validate_public_roster(2, 3, 2, &[], &noncanonical, 1, false).is_err());
    }

    #[test]
    fn published_run_rejects_proof_substitution_and_tainted_provenance() {
        let mut detail = run_detail(BoardCategoryV1::Campaign);
        detail.mission.as_mut().unwrap().content_manifest_sha256 = Digest32::from_bytes([99; 32]);
        assert!(detail.validate().is_err());

        let mut detail = run_detail(BoardCategoryV1::Campaign);
        detail.viewer.as_mut().unwrap().build_manifest_sha256 = Digest32::from_bytes([99; 32]);
        assert!(detail.validate().is_err());

        let mut detail = run_detail(BoardCategoryV1::Campaign);
        let tainted = InputProvenanceStatusV1::Tainted {
            taints: vec![crate::InputTaintV1 {
                kind: crate::InputTaintKindV1::DebugInputInjection,
                first_frame: 0,
            }],
        };
        detail.input_provenance = tainted.clone();
        detail.verification_proof.as_mut().unwrap().input_provenance = tainted;
        assert_eq!(
            detail.validate(),
            Err(ValidationError::VerifiedRunNotRankable)
        );
    }

    #[test]
    fn submission_accepted_is_queue_acknowledgement_not_rank_acceptance() {
        let mut response = SubmissionAcceptedV1 {
            schema_version: SCHEMA_VERSION_V1,
            submission_id: id("submission-1"),
            state: SubmissionLifecycleV1::Queued,
            retry_after_ms: 250,
        };
        assert!(response.validate().is_ok());
        response.state = SubmissionLifecycleV1::Accepted {
            run_id: id("run-1"),
            campaign_chain_receipt: Some(receipt()),
        };
        assert!(response.validate().is_err());

        response.state = SubmissionLifecycleV1::Failed {
            code: SubmissionFailureCodeV1::VerificationInfrastructure,
            safe_message: "Verification infrastructure failed after bounded retries.".into(),
        };
        assert!(response.validate().is_err());
        let envelope = SubmissionOwnerStatusEnvelopeV1 {
            schema_version: SCHEMA_VERSION_V1,
            challenge: SubmissionOwnerStatusChallengeV1 {
                schema_version: SCHEMA_VERSION_V1,
                owner_status_challenge_id: id("owner-status-1"),
                owner_status_challenge_nonce: ChallengeNonce32::from_bytes([55; 32]),
                expires_at_unix_ms: 1_800_000_000_000,
                controller_public_key: PublicKey32::from_bytes([4; 32]),
                submission_id: id("submission-1"),
            },
            algorithm: SignatureAlgorithmV1::Ed25519,
            signature: Signature64::from_bytes([56; 64]),
        };
        let status = SubmissionOwnerStatusResponseV1 {
            schema_version: SCHEMA_VERSION_V1,
            submission_id: id("submission-1"),
            controller_public_key: PublicKey32::from_bytes([4; 32]),
            owner_status_envelope_sha256: envelope.canonical_digest().unwrap(),
            state: SubmissionLifecycleV1::Accepted {
                run_id: id("run-1"),
                campaign_chain_receipt: Some(receipt()),
            },
        };
        assert!(status.validate().is_ok());
        assert!(status.validate_against_envelope(&envelope).is_ok());
        let mut substituted_owner = status;
        substituted_owner.controller_public_key = PublicKey32::from_bytes([99; 32]);
        assert!(substituted_owner.validate().is_err());
    }

    #[test]
    fn owner_status_signature_is_one_use_domain_separated_and_exactly_bound() {
        let request = SubmissionOwnerStatusChallengeRequestV1 {
            schema_version: SCHEMA_VERSION_V1,
            controller_public_key: PublicKey32::from_bytes([4; 32]),
            submission_id: id("opaque-maybe-existing-submission"),
        };
        assert!(request.validate().is_ok());
        let mut envelope = SubmissionOwnerStatusEnvelopeV1 {
            schema_version: SCHEMA_VERSION_V1,
            challenge: SubmissionOwnerStatusChallengeV1 {
                schema_version: SCHEMA_VERSION_V1,
                owner_status_challenge_id: id("owner-status-1"),
                owner_status_challenge_nonce: ChallengeNonce32::from_bytes([55; 32]),
                expires_at_unix_ms: 1_800_000_000_000,
                controller_public_key: request.controller_public_key,
                submission_id: request.submission_id,
            },
            algorithm: SignatureAlgorithmV1::Ed25519,
            signature: Signature64::from_bytes([56; 64]),
        };
        let baseline = envelope.signing_bytes().unwrap();
        assert!(baseline.starts_with(SUBMISSION_OWNER_STATUS_SIGNATURE_DOMAIN_V1));
        assert!(envelope.validate().is_ok());

        envelope.challenge.submission_id = id("substituted-submission");
        assert_ne!(baseline, envelope.signing_bytes().unwrap());
        envelope.challenge.owner_status_challenge_nonce = ChallengeNonce32::from_bytes([57; 32]);
        assert_ne!(baseline, envelope.signing_bytes().unwrap());
        envelope.signature = Signature64::default();
        assert!(envelope.validate_signing_claim().is_ok());
        assert!(envelope.validate().is_err());
    }

    #[test]
    fn leaderboard_query_has_bounded_pagination() {
        let mut query = LeaderboardQueryV1 {
            schema_version: SCHEMA_VERSION_V1,
            subject_kind: LeaderboardQuerySubjectV1::Mission,
            mission_id: Some("mission_1".into()),
            mission_scope: Some(BoardCategoryV1::Campaign),
            metric: BoardMetricV1::OriginalScore,
            content_identity_sha256: Digest32::from_bytes([1; 32]),
            rules_config_sha256: Digest32::from_bytes([2; 32]),
            ruleset_manifest_sha256: Digest32::from_bytes([3; 32]),
            competition_manifest_sha256: None,
            max_concurrent_players: Some(1),
            player_public_key: None,
            limit: 50,
            cursor: None,
        };
        assert!(query.validate().is_ok());
        assert_eq!(
            query.filter().unwrap().subject,
            LeaderboardSubjectV1::Mission {
                mission_id: "mission_1".into(),
                category: BoardCategoryV1::Campaign,
            }
        );
        query.limit = 101;
        assert!(query.validate().is_err());
        query.limit = 50;
        query.subject_kind = LeaderboardQuerySubjectV1::FullCampaign;
        assert!(query.validate().is_err());
        query.mission_id = None;
        query.mission_scope = None;
        assert_eq!(
            query.filter().unwrap().subject,
            LeaderboardSubjectV1::FullCampaign
        );
    }

    #[test]
    fn leaderboard_query_round_trips_as_flat_url_fields() {
        let query = LeaderboardQueryV1 {
            schema_version: SCHEMA_VERSION_V1,
            subject_kind: LeaderboardQuerySubjectV1::Mission,
            mission_id: Some("mission_1".into()),
            mission_scope: Some(BoardCategoryV1::IndividualLevel),
            metric: BoardMetricV1::FastestSuccess,
            content_identity_sha256: Digest32::from_bytes([1; 32]),
            rules_config_sha256: Digest32::from_bytes([2; 32]),
            ruleset_manifest_sha256: Digest32::from_bytes([3; 32]),
            competition_manifest_sha256: Some(Digest32::from_bytes([4; 32])),
            max_concurrent_players: Some(1),
            player_public_key: Some(PublicKey32::from_bytes([5; 32])),
            limit: 50,
            cursor: Some("next-page".into()),
        };
        let encoded = serde_urlencoded::to_string(&query).unwrap();
        assert!(encoded.contains("subject_kind=mission"));
        assert!(encoded.contains("mission_id=mission_1"));
        assert!(encoded.contains("mission_scope=individual_level"));
        assert!(encoded.contains("player_public_key="));
        assert!(!encoded.contains("%7B"));
        let decoded: LeaderboardQueryV1 = serde_urlencoded::from_str(&encoded).unwrap();
        assert_eq!(decoded, query);
        let filter = RunFilterV1::try_from(decoded).unwrap();
        let digest = filter.canonical_digest().unwrap();
        let mut other_player = filter;
        other_player.player_public_key = Some(PublicKey32::from_bytes([6; 32]));
        assert_ne!(digest, other_player.canonical_digest().unwrap());
    }

    #[test]
    fn player_history_cursor_filter_is_canonical_and_path_identity_bound() {
        let query = PlayerRunHistoryQueryV1 {
            schema_version: SCHEMA_VERSION_V1,
            limit: 50,
            cursor: Some("opaque-first-page-cursor".to_owned()),
        };
        let player = PublicKey32::from_bytes([5; 32]);
        let filter = query.filter_for_player(player);
        filter.validate().unwrap();
        let digest = filter.canonical_digest().unwrap();

        let mut next_page = query.clone();
        next_page.cursor = Some("opaque-next-page-cursor".to_owned());
        assert_eq!(
            digest,
            next_page
                .filter_for_player(player)
                .canonical_digest()
                .unwrap(),
            "the transport cursor must not recursively change its own filter digest"
        );
        assert_ne!(
            digest,
            query
                .filter_for_player(PublicKey32::from_bytes([6; 32]))
                .canonical_digest()
                .unwrap()
        );
        let mut other_limit = query;
        other_limit.limit = 25;
        assert_ne!(
            digest,
            other_limit
                .filter_for_player(player)
                .canonical_digest()
                .unwrap()
        );
    }

    #[test]
    fn leaderboard_page_rejects_reordering_duplicates_and_forged_ties() {
        let valid = LeaderboardPageV1 {
            schema_version: SCHEMA_VERSION_V1,
            filter: leaderboard_filter(BoardMetricV1::OriginalScore),
            accepted_sequence_watermark: 10,
            previous_cursor: None,
            entries: vec![
                leaderboard_entry(1, 1, 1_000, 1),
                leaderboard_entry(2, 2, 900, 2),
            ],
            next_cursor: None,
        };
        assert!(valid.validate().is_ok());

        let mut reordered = valid.clone();
        reordered.entries[0].metric_value = BoardMetricValueV1::OriginalScore { points: 900 };
        reordered.entries[1].metric_value = BoardMetricValueV1::OriginalScore { points: 1_000 };
        assert!(reordered.validate().is_err());

        let mut duplicate = valid.clone();
        duplicate.entries[1].run_id = duplicate.entries[0].run_id.clone();
        assert!(duplicate.validate().is_err());

        let mut tied = valid.clone();
        tied.entries[1].metric_value = BoardMetricValueV1::OriginalScore { points: 1_000 };
        tied.entries[1].rank = 1;
        assert!(tied.validate().is_ok());
        tied.entries[1].rank = 2;
        assert!(tied.validate().is_err());

        let mut reversed_tie = valid.clone();
        reversed_tie.entries[0].metric_value = BoardMetricValueV1::OriginalScore { points: 1_000 };
        reversed_tie.entries[1].metric_value = BoardMetricValueV1::OriginalScore { points: 1_000 };
        reversed_tie.entries[1].rank = 1;
        reversed_tie.entries[0].accepted_sequence = 3;
        assert!(reversed_tie.validate().is_err());

        let mut oversized = valid;
        oversized.entries = vec![leaderboard_entry(1, 1, 1_000, 1); 101];
        assert!(oversized.validate().is_err());

        oversized.entries.clear();
        oversized.accepted_sequence_watermark = 0;
        assert!(oversized.validate().is_ok());

        oversized.next_cursor = Some(LeaderboardCursorV1 {
            schema_version: SCHEMA_VERSION_V1,
            query_sha256: oversized.filter.canonical_digest().unwrap(),
            accepted_sequence_watermark: 1,
            last: LeaderboardOrderAnchorV1::from_entry(&leaderboard_entry(1, 1, 1_000, 1)),
            opaque_token: "not-valid-on-empty".into(),
        });
        assert!(oversized.validate().is_err());
    }

    #[test]
    fn competition_digest_binds_content_seed_and_board_tuple() {
        let manifest = competition_manifest();
        let digest = manifest.canonical_digest().unwrap();
        let summary = CompetitionSummaryV1 {
            competition_manifest_sha256: digest,
            manifest: manifest.clone(),
            state: CompetitionStateV1::Active,
        };
        assert!(summary.validate().is_ok());

        let mut filter = leaderboard_filter(BoardMetricV1::OriginalScore);
        filter.competition_manifest_sha256 = Some(digest);
        assert!(filter.validate_against_competition(&summary).is_ok());

        let mut substituted_content = summary.clone();
        substituted_content.manifest.content = RunContentIdentityV1::Mission {
            content_manifest_sha256: Digest32::from_bytes([9; 32]),
        };
        assert!(substituted_content.validate().is_err());

        let mut substituted_seed = summary;
        substituted_seed.manifest.seed_policy = CompetitionSeedPolicyV1::Pinned {
            simulation_seed: SimulationSeed64::new(43),
        };
        substituted_seed.competition_manifest_sha256 =
            substituted_seed.manifest.canonical_digest().unwrap();
        assert!(
            filter
                .validate_against_competition(&substituted_seed)
                .is_err()
        );
    }

    #[test]
    fn player_profile_uses_full_key_and_bounded_username() {
        let mut profile = PlayerProfileV1 {
            schema_version: SCHEMA_VERSION_V1,
            username: "Robin".into(),
            public_key: PublicKey32::from_bytes([4; 32]),
            public_key_fingerprint: PublicKey32::from_bytes([4; 32]).short_fingerprint(),
        };
        assert!(profile.validate().is_ok());
        profile.public_key_fingerprint = "deadbeef".into();
        assert!(matches!(
            profile.validate(),
            Err(ValidationError::ClaimMismatch {
                field: "public_participant.public_key_fingerprint"
            })
        ));
        profile.public_key_fingerprint = profile.public_key.short_fingerprint();
        profile.username = "x".repeat(49);
        assert!(profile.validate().is_err());
    }
}
