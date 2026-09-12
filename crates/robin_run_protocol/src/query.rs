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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
    /// Omit both rules digests for the combined board across all configurations.
    pub rules_config_sha256: Option<Digest32>,
    pub ruleset_manifest_sha256: Option<Digest32>,
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
        for digest in [self.rules_config_sha256, self.ruleset_manifest_sha256]
            .into_iter()
            .flatten()
        {
            if digest.is_zero() {
                return Err(ValidationError::Zero {
                    field: "run_filter.identity_digest",
                });
            }
        }
        if self.rules_config_sha256.is_some() != self.ruleset_manifest_sha256.is_some()
            || (self.competition_manifest_sha256.is_some()
                && self.ruleset_manifest_sha256.is_none())
        {
            return Err(ValidationError::ClaimMismatch {
                field: "run_filter.ruleset_selection",
            });
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
            || self.rules_config_sha256 != Some(manifest.rules_config_sha256)
            || self.ruleset_manifest_sha256 != Some(manifest.ruleset_manifest_sha256)
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
    /// Omit both rules digests for the combined board across all configurations.
    pub rules_config_sha256: Option<Digest32>,
    pub ruleset_manifest_sha256: Option<Digest32>,
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
        crate::validation::nonzero(
            "mission_facet.content_manifest_sha256",
            &self.content_manifest_sha256,
        )?;
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

impl RulesetFacetV1 {
    /// A ruleset can cover several missions and a separate full-campaign board.
    pub fn identity(&self) -> (Digest32, RunContentIdentityV1) {
        (self.ruleset_manifest_sha256, self.content)
    }
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
        crate::validation::nonzero(
            "competition.competition_run_grant_public_key",
            &self.competition_run_grant_public_key,
        )?;
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
                .all(|pair| pair[0].identity() < pair[1].identity())
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
        crate::validation::nonzero("public_participant.public_key", &self.public_key)?;
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
        crate::validation::nonzero("aggregate_public_participant.public_key", &self.public_key)?;
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
        crate::validation::nonzero(
            "player_run_history_filter.player_public_key",
            &self.player_public_key,
        )?;
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
        crate::validation::nonzero(
            "player_run_history_entry.player_public_key",
            &self.player_public_key,
        )?;
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
        if self.filter.ruleset_manifest_sha256 != Some(published.ruleset_manifest_sha256)
            || !self
                .filter
                .rules_config_sha256
                .is_some_and(|digest| manifest.admits_rules_config_digest(digest))
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
mod tests;
