//! Immutable ranked rules, eligibility policies and published ruleset validation.

use super::ArtifactRefV1;
use crate::CanonicalDocument as _;
use crate::{
    BoardMetricV1, CampaignContentManifestV1, CanonicalValue, Digest32, OfficialContentEditionV1,
    OfficialContentSubjectV1, OpaqueId, PublicKey32, Validate, ValidationError,
    VerifiedAchievementEvaluationV1, VerifiedAchievementV1,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Exact engine policy implemented by a current ranked rules configuration.
///
/// This is deliberately typed rather than encoded in
/// [`RulesConfigIdentityV1::rules`]. A service may still publish additional
/// ranking predicates in that map, but neither a client nor a verifier is
/// allowed to interpret an arbitrary string as the policy which seals the
/// deterministic engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RankedSimulationPresetV1 {
    Standard,
    OriginalParity,
    Custom,
}

impl RankedSimulationPresetV1 {
    /// Stable ruleset facet ID corresponding to this policy.
    pub const fn preset_id(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::OriginalParity => "original",
            Self::Custom => "custom",
        }
    }

    pub const fn preset_name(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::OriginalParity => "Original",
            Self::Custom => "Custom",
        }
    }
}

/// Retail difficulty selected by an immutable ranked simulation policy.
/// Custom and Legendary use the explicit custom policy; they cannot silently
/// enter the Standard/Original board families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RankedSimulationDifficultyV1 {
    Easy,
    Medium,
    Hard,
    Legendary,
    Custom,
}

impl RankedSimulationDifficultyV1 {
    /// Existing public board IDs call the shipped Medium preset `normal`.
    pub const fn difficulty_id(self) -> &'static str {
        match self {
            Self::Easy => "easy",
            Self::Medium => "normal",
            Self::Hard => "hard",
            Self::Legendary => "legendary",
            Self::Custom => "custom",
        }
    }

    pub const fn difficulty_name(self) -> &'static str {
        match self {
            Self::Easy => "Easy",
            Self::Medium => "Normal",
            Self::Hard => "Hard",
            Self::Legendary => "Legendary",
            Self::Custom => "Custom",
        }
    }

    /// Exact serde spelling used by the engine's `DifficultyLevel` wire type.
    pub const fn sim_config_wire_name(self) -> &'static str {
        match self {
            Self::Easy => "Easy",
            Self::Medium => "Medium",
            Self::Hard => "Hard",
            Self::Legendary => "Legendary",
            Self::Custom => "Custom",
        }
    }
}

pub const RANKED_SIMULATION_POLICY_VERSION_V1: u32 = 1;

/// Typed, versioned engine policy carried inside the content-addressed rules
/// configuration. The complete canonical `SimConfig` remains adjacent to it;
/// the verifier must prove that map is the one fixed configuration generated
/// by this preset/difficulty pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RankedSimulationPolicyV1 {
    pub version: u32,
    pub preset: RankedSimulationPresetV1,
    pub difficulty: RankedSimulationDifficultyV1,
}

impl RankedSimulationPolicyV1 {
    pub const fn standard(difficulty: RankedSimulationDifficultyV1) -> Self {
        Self {
            version: RANKED_SIMULATION_POLICY_VERSION_V1,
            preset: RankedSimulationPresetV1::Standard,
            difficulty,
        }
    }

    pub const fn original_parity(difficulty: RankedSimulationDifficultyV1) -> Self {
        Self {
            version: RANKED_SIMULATION_POLICY_VERSION_V1,
            preset: RankedSimulationPresetV1::OriginalParity,
            difficulty,
        }
    }

    pub fn matches_ruleset_labels(
        self,
        preset_id: &str,
        preset_name: &str,
        difficulty_id: &str,
        difficulty_name: &str,
    ) -> bool {
        preset_id == self.preset.preset_id()
            && preset_name == self.preset.preset_name()
            && difficulty_id == self.difficulty.difficulty_id()
            && difficulty_name == self.difficulty.difficulty_name()
    }
}

impl Validate for RankedSimulationPolicyV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.version != RANKED_SIMULATION_POLICY_VERSION_V1 {
            return Err(ValidationError::SchemaVersion {
                document: "RankedSimulationPolicyV1",
                expected: RANKED_SIMULATION_POLICY_VERSION_V1,
                actual: self.version,
            });
        }
        if self.preset != RankedSimulationPresetV1::Custom
            && matches!(
                self.difficulty,
                RankedSimulationDifficultyV1::Legendary | RankedSimulationDifficultyV1::Custom
            )
        {
            return Err(ValidationError::ClaimMismatch {
                field: "ranked_simulation_policy.difficulty",
            });
        }
        Ok(())
    }
}

/// Identity of the full deterministic game configuration and board rules.
///
/// The engine serializes `SimConfig` into `sim_config`; the service publishes
/// eligibility/admission facts in `rules`.  Keeping both exact documents in
/// one content-addressed identity prevents current application defaults from
/// silently redefining an existing board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RulesConfigIdentityV1 {
    pub schema_version: u32,
    pub replay_schema_version: u32,
    pub ranked_simulation_policy: RankedSimulationPolicyV1,
    pub sim_config: BTreeMap<String, CanonicalValue>,
    pub rules: BTreeMap<String, CanonicalValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RulesetBoardScopeV1 {
    IndividualLevel,
    CampaignMission,
    FullCampaign,
}

/// Immutable predicate which turns one independently verified campaign
/// session into the terminal proof for a full-campaign chain.
///
/// The content manifest is intentionally not embedded here: a ruleset admits
/// content-addressed campaign catalogs, and validation against the selected
/// catalog proves that this exact subject exists in that catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignCompletionPolicyV1 {
    pub terminal_subject: OfficialContentSubjectV1,
    pub required_progression_percent: u8,
}

/// Explicit presence marker. This is deliberately not an `Option`: serde
/// otherwise accepts an omitted field as `None`, allowing an older producer
/// to silently erase a full-campaign terminal policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "policy", rename_all = "snake_case")]
pub enum CampaignCompletionPolicyRequirementV1 {
    NotOffered,
    Required(CampaignCompletionPolicyV1),
}

impl CampaignCompletionPolicyRequirementV1 {
    pub const fn required(&self) -> Option<&CampaignCompletionPolicyV1> {
        match self {
            Self::NotOffered => None,
            Self::Required(policy) => Some(policy),
        }
    }
}

impl Validate for CampaignCompletionPolicyV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.terminal_subject.validate()?;
        if self.required_progression_percent == 0 || self.required_progression_percent > 100 {
            return Err(ValidationError::CountOutOfRange {
                field: "campaign_completion_policy.required_progression_percent",
            });
        }
        Ok(())
    }
}

/// Completion predicate for the official full retail campaign. Original
/// The original game makes a won H12 mission the terminal 100% progression
/// state; spelling the engine mission id here keeps that rule immutable.
pub fn official_full_campaign_completion_policy_v1() -> CampaignCompletionPolicyV1 {
    CampaignCompletionPolicyV1 {
        terminal_subject: OfficialContentSubjectV1::FieldMission {
            mission_id: "H12_Not_MP".into(),
        },
        required_progression_percent: 100,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RulesetSeedPolicyV1 {
    Open,
    ServerPinned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActiveTimeDefinitionV1 {
    /// Simulation ticks from the canonical start through independently reached
    /// terminal success, excluding paused/loading/debrief wall time.
    SuccessfulSimulationTicks,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImmutablePolicyIdentityV1 {
    pub kind: ImmutablePolicyKindV1,
    pub version: u32,
    pub manifest_sha256: Digest32,
}

impl Validate for ImmutablePolicyIdentityV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.version == 0 || self.manifest_sha256.is_zero() {
            return Err(ValidationError::Zero {
                field: "ruleset.policy_identity",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImmutablePolicyKindV1 {
    InputProvenance,
    CommandAdmission,
    SubmissionAdmission,
    Verification,
}

/// An immutable, digest-addressed policy document used when the typed
/// `RulesetManifestV1` needs to pin detailed admission behavior. The typed
/// ruleset fields remain the classification contract; this document makes the
/// exact implementation-policy tables independently inspectable rather than a
/// mutable server-side convention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImmutablePolicyManifestV1 {
    pub schema_version: u32,
    pub kind: ImmutablePolicyKindV1,
    pub version: u32,
    pub rules: BTreeMap<String, CanonicalValue>,
}

impl Validate for ImmutablePolicyManifestV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("ImmutablePolicyManifestV1", self.schema_version)?;
        if self.version == 0 {
            return Err(ValidationError::Zero {
                field: "immutable_policy.version",
            });
        }
        if self.rules.is_empty() {
            return Err(ValidationError::Empty {
                field: "immutable_policy.rules",
            });
        }
        for (key, value) in &self.rules {
            crate::validation::text("immutable_policy.rules.key", key, 256)?;
            value.validate_depth(64)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalStartPolicyV1 {
    RulesConfigBoundOperatorStateAndVerifiedPredecessor,
    /// Reconstruct the official mission/team selection and restart checkpoint
    /// from fresh state before comparing the replay's starting bytes.
    RulesConfigBoundMissionSetupAndVerifiedPredecessor,
}

impl CanonicalStartPolicyV1 {
    pub const fn requires_exact_operator_artifact(self) -> bool {
        matches!(
            self,
            Self::RulesConfigBoundOperatorStateAndVerifiedPredecessor
        )
    }
}

/// Logical class of the operator-private campaign state from which an
/// official ranked lineage may start. The bytes and their digest deliberately
/// stay outside public ruleset/competition documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalCampaignStateKindV1 {
    IndividualTemplate,
    FullCampaignGenesis,
}

/// Public, non-secret binding between a ranked rules configuration and the
/// class of canonical campaign state required for that edition. This is safe
/// to publish: unlike `CanonicalCampaignStatePinV1`, it contains no campaign
/// digest, length, path, or other private object identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalCampaignStateRequirementV1 {
    pub edition: OfficialContentEditionV1,
    pub kind: CanonicalCampaignStateKindV1,
    pub rules_config_sha256: Digest32,
}

impl Validate for CanonicalCampaignStateRequirementV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.rules_config_sha256.is_zero() {
            return Err(ValidationError::Zero {
                field: "canonical_campaign_state_requirement.rules_config_sha256",
            });
        }
        let expected = match self.edition {
            OfficialContentEditionV1::Demo => CanonicalCampaignStateKindV1::IndividualTemplate,
            OfficialContentEditionV1::Full => CanonicalCampaignStateKindV1::FullCampaignGenesis,
        };
        if self.kind != expected {
            return Err(ValidationError::ClaimMismatch {
                field: "canonical_campaign_state_requirement.edition_kind",
            });
        }
        Ok(())
    }
}

/// Deployment-private exact canonical campaign object. Multiple logical pins
/// may reference the same physical bytes, but the requirement (including its
/// exact rules-config digest) remains part of every identity comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalCampaignStatePinV1 {
    pub requirement: CanonicalCampaignStateRequirementV1,
    pub artifact: ArtifactRefV1,
}

impl Validate for CanonicalCampaignStatePinV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.requirement.validate()?;
        self.artifact.validate()?;
        if self.artifact.media_type != crate::RANKED_CAMPAIGN_MEDIA_TYPE_V1 {
            return Err(ValidationError::ClaimMismatch {
                field: "canonical_campaign_state_pin.artifact.media_type",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FullCampaignChainPolicyV1 {
    CanonicalGenesisEveryFieldAndHeadquartersSessionIndependentCompletion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignRosterContinuityV1 {
    UnionOfVerifiedSessionSubsets,
    ExactSameAuthenticatedKeysEverySession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignAggregationConsentPolicyV1 {
    EveryAuthenticatedKeyFinalCosignsEachSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamedParticipantPolicyV1 {
    HostGenesisGuestTransportJoinAttestationAndFinalCosign,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnonymousParticipantPolicyV1 {
    AllowedAuthenticatedButPubliclyRedacted,
    Forbidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticipantEligibilityV1 {
    pub allow_single_player: bool,
    pub allow_multiplayer: bool,
    pub named_policy: NamedParticipantPolicyV1,
    pub anonymous_policy: AnonymousParticipantPolicyV1,
    pub minimum_max_concurrent_players: u16,
    pub maximum_max_concurrent_players: u16,
    pub maximum_participant_instances: u16,
}

impl Validate for ParticipantEligibilityV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if !self.allow_single_player && !self.allow_multiplayer {
            return Err(ValidationError::Empty {
                field: "ruleset.participants.allowed_modes",
            });
        }
        if self.minimum_max_concurrent_players == 0
            || self.minimum_max_concurrent_players > self.maximum_max_concurrent_players
            || self.maximum_max_concurrent_players > crate::MAX_REPLAY_SEATS_V1
            || self.maximum_participant_instances < self.maximum_max_concurrent_players
            || self.maximum_participant_instances > crate::MAX_PARTICIPANT_INSTANCES_V1
            || (!self.allow_multiplayer && self.maximum_max_concurrent_players != 1)
        {
            return Err(ValidationError::CountOutOfRange {
                field: "ruleset.participants.count_bounds",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputProvenanceEligibilityV1 {
    /// Only the current runtime schema and its single identity-free canonical
    /// compact bitcode replay are eligible. Older Rust replay schemas are not
    /// compatibility lanes for the service.
    CurrentSchemaCanonicalReplayOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalResultPolicyV1 {
    IndependentlyReachedWonOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoreAlgorithmV1 {
    OriginalMissionAttemptWrappingSubtotalCampaignDeltaV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoreOverflowPolicyV1 {
    RejectCampaignOrAggregateOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricRankingPolicyV1 {
    OriginalScoreDescending,
    FastestSuccessAscending,
}

impl MetricRankingPolicyV1 {
    const fn metric(self) -> BoardMetricV1 {
        match self {
            Self::OriginalScoreDescending => BoardMetricV1::OriginalScore,
            Self::FastestSuccessAscending => BoardMetricV1::FastestSuccess,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibleTiePolicyV1 {
    EqualPrimaryMetricSharesRank,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaginationTieBreakV1 {
    AcceptedSequenceThenVerificationTimeThenRunIdOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameCountingPolicyV1 {
    ZeroBasedEventsBeforeExclusiveReplayFrameCount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FullCampaignTimeAggregationV1 {
    CheckedSumEveryVerifiedFieldAndHeadquartersSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunCompositionPolicyV1 {
    MissionSingleReplayFullCampaignOrderedSessionsNoSyntheticReplay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RulesConfigConstraintV1 {
    ExactCanonicalDigestOnly,
    /// Accept a complete run-specific configuration whose digest is signed
    /// before simulation. Gameplay settings remain fixed throughout the run.
    AnyCanonicalSimConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AchievementPolicyModeV1 {
    /// Failure to produce a definitive decision rejects official verification.
    Required,
    /// A typed `unverifiable` result may be reported but is never an award.
    Reported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AchievementPolicyV1 {
    pub achievement_id: OpaqueId,
    pub mode: AchievementPolicyModeV1,
}

/// The achievement catalog for the official ranked ruleset. The returned
/// vector is already in canonical stable-ID order.
pub fn official_achievement_policies_v1() -> Vec<AchievementPolicyV1> {
    [
        "a-legend-is-born",
        "all-banners-purchased",
        "all-beggar-info",
        "charity",
        "clean-hands",
        "different-kind-of-scarlet",
        "for-king-richard",
        "ghost",
        "im-off-home",
        "kill-a-civilian",
        "leave-everyone-standing",
        "many-hands",
        "no-banners-purchased",
        "no-empty-places",
        "not-a-scratch",
        "on-my-mark",
        "people-behind-the-legend",
        "pile-o-bones",
        "round-on-the-friar",
        "ruthless",
        "something-in-the-air",
        "string-theory",
        "whole-merry-company",
        "you-never-saw-us-leave",
    ]
    .into_iter()
    .map(|achievement_id| AchievementPolicyV1 {
        achievement_id: OpaqueId::new(achievement_id)
            .expect("official achievement IDs are protocol constants"),
        mode: AchievementPolicyModeV1::Required,
    })
    .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TickDurationV1 {
    pub numerator_micros: u64,
    pub denominator: u64,
}

impl Validate for TickDurationV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.numerator_micros == 0 || self.denominator == 0 {
            return Err(ValidationError::Zero {
                field: "ruleset.tick_duration",
            });
        }
        if gcd(self.numerator_micros, self.denominator) != 1 {
            return Err(ValidationError::ClaimMismatch {
                field: "ruleset.tick_duration.canonical_fraction",
            });
        }
        Ok(())
    }
}

const fn gcd(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

/// Immutable board/ranking policy. Its canonical digest is
/// `ruleset_manifest_sha256`; the separately addressed
/// `RulesConfigIdentityV1` canonical digest is `rules_config_sha256`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RulesetManifestV1 {
    pub schema_version: u32,
    pub display_name: String,
    pub preset_id: OpaqueId,
    pub preset_name: String,
    pub difficulty_id: OpaqueId,
    pub difficulty_name: String,
    pub rules_config_sha256: Digest32,
    pub rules_config_constraint: RulesConfigConstraintV1,
    pub allowed_build_manifest_sha256: Vec<Digest32>,
    pub allowed_content_manifest_sha256: Vec<Digest32>,
    pub allowed_campaign_content_manifest_sha256: Vec<Digest32>,
    pub board_scopes: Vec<RulesetBoardScopeV1>,
    /// Present exactly when this ruleset offers a full-campaign board.
    pub campaign_completion_policy: CampaignCompletionPolicyRequirementV1,
    pub metrics: Vec<BoardMetricV1>,
    pub metric_ranking: Vec<MetricRankingPolicyV1>,
    /// Exact authoritative achievement catalog, sorted uniquely by stable ID.
    /// IDs absent here must not appear in `VerifiedRunV1::achievements`.
    pub achievement_policies: Vec<AchievementPolicyV1>,
    pub canonical_start_policy: CanonicalStartPolicyV1,
    /// Public requirement only. The matching private artifact pin is selected
    /// by the operator and must never be copied into Pages manifests.
    pub canonical_campaign_state: CanonicalCampaignStateRequirementV1,
    /// Dedicated authority for pre-frame admission of exact fresh campaign
    /// artifacts and verified continuations. The signing seed is private.
    pub run_preflight_grant_public_key: PublicKey32,
    pub full_campaign_chain_policy: FullCampaignChainPolicyV1,
    pub campaign_roster_continuity: CampaignRosterContinuityV1,
    pub campaign_aggregation_consent_policy: CampaignAggregationConsentPolicyV1,
    pub participant_eligibility: ParticipantEligibilityV1,
    pub replay_schema_versions: Vec<u32>,
    pub network_protocol_versions: Vec<u32>,
    pub input_provenance_policy: ImmutablePolicyIdentityV1,
    pub command_admission_policy: ImmutablePolicyIdentityV1,
    pub submission_admission_policy: ImmutablePolicyIdentityV1,
    pub verifier_policy: ImmutablePolicyIdentityV1,
    pub input_provenance_eligibility: InputProvenanceEligibilityV1,
    pub terminal_result_policy: TerminalResultPolicyV1,
    pub score_algorithm: ScoreAlgorithmV1,
    pub score_overflow_policy: ScoreOverflowPolicyV1,
    pub visible_tie_policy: VisibleTiePolicyV1,
    pub pagination_tie_break: PaginationTieBreakV1,
    pub tick_duration: TickDurationV1,
    pub active_time_definition: ActiveTimeDefinitionV1,
    pub frame_counting_policy: FrameCountingPolicyV1,
    pub full_campaign_time_aggregation: FullCampaignTimeAggregationV1,
    pub run_composition_policy: RunCompositionPolicyV1,
    pub main_board_seed_policy: RulesetSeedPolicyV1,
    pub competition_seed_policy: RulesetSeedPolicyV1,
    pub allow_save_creation: bool,
    pub allow_autosave: bool,
    pub allow_state_load: bool,
    pub allow_mission_restart: bool,
}

impl RulesetManifestV1 {
    pub fn admits_campaign_state_requirement(
        &self,
        requirement: crate::CanonicalCampaignStateRequirementV1,
    ) -> bool {
        let mut expected = self.canonical_campaign_state;
        if self.rules_config_constraint == RulesConfigConstraintV1::AnyCanonicalSimConfig {
            expected.rules_config_sha256 = requirement.rules_config_sha256;
        }
        expected == requirement
    }

    pub fn admits_rules_config_digest(&self, digest: Digest32) -> bool {
        !digest.is_zero()
            && (self.rules_config_constraint == RulesConfigConstraintV1::AnyCanonicalSimConfig
                || self.rules_config_sha256 == digest)
    }
    /// Validate the cross-document engine-policy identity. Neither document
    /// may supply free-form preset/difficulty labels which disagree with the
    /// typed, content-addressed policy or point at different config bytes.
    pub fn validate_ranked_simulation_policy(
        &self,
        rules_config: &RulesConfigIdentityV1,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        rules_config.validate()?;
        let rules_config_sha256 =
            rules_config
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "ruleset.rules_config_canonicalization",
                })?;
        if self.rules_config_constraint == RulesConfigConstraintV1::AnyCanonicalSimConfig {
            if self.preset_id.as_str() != "any" || self.difficulty_id.as_str() != "any" {
                return Err(ValidationError::ClaimMismatch {
                    field: "ruleset.any_config_labels",
                });
            }
            return Ok(());
        }
        if self.rules_config_sha256 != rules_config_sha256 {
            return Err(ValidationError::ClaimMismatch {
                field: "ruleset.rules_config_sha256",
            });
        }
        if !rules_config
            .ranked_simulation_policy
            .matches_ruleset_labels(
                self.preset_id.as_str(),
                &self.preset_name,
                self.difficulty_id.as_str(),
                &self.difficulty_name,
            )
        {
            return Err(ValidationError::ClaimMismatch {
                field: "ruleset.ranked_simulation_policy_labels",
            });
        }
        Ok(())
    }
}

impl Validate for RulesetManifestV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("RulesetManifestV1", self.schema_version)?;
        crate::validation::text("ruleset.display_name", &self.display_name, 100)?;
        crate::validation::text("ruleset.preset_name", &self.preset_name, 100)?;
        crate::validation::text("ruleset.difficulty_name", &self.difficulty_name, 100)?;
        if self.rules_config_sha256.is_zero() || self.run_preflight_grant_public_key.is_zero() {
            return Err(ValidationError::Zero {
                field: "ruleset.identity_digest",
            });
        }
        self.canonical_campaign_state.validate()?;
        if self.canonical_campaign_state.rules_config_sha256 != self.rules_config_sha256 {
            return Err(ValidationError::ClaimMismatch {
                field: "ruleset.canonical_campaign_state.rules_config_sha256",
            });
        }
        for (field, values) in [
            (
                "ruleset.allowed_build_manifest_sha256",
                &self.allowed_build_manifest_sha256,
            ),
            (
                "ruleset.allowed_content_manifest_sha256",
                &self.allowed_content_manifest_sha256,
            ),
        ] {
            if values.is_empty()
                || values.iter().any(Digest32::is_zero)
                || !crate::validation::strictly_sorted(values)
            {
                return Err(ValidationError::NotCanonicalOrder { field });
            }
        }
        let allows_full_campaign = self
            .board_scopes
            .binary_search(&RulesetBoardScopeV1::FullCampaign)
            .is_ok();
        if allows_full_campaign != !self.allowed_campaign_content_manifest_sha256.is_empty()
            || allows_full_campaign != self.campaign_completion_policy.required().is_some()
            || self
                .allowed_campaign_content_manifest_sha256
                .iter()
                .any(Digest32::is_zero)
            || !crate::validation::strictly_sorted(&self.allowed_campaign_content_manifest_sha256)
        {
            return Err(ValidationError::NotCanonicalOrder {
                field: "ruleset.allowed_campaign_content_manifest_sha256",
            });
        }
        if let Some(policy) = self.campaign_completion_policy.required() {
            policy.validate()?;
        }
        if self.board_scopes.is_empty() || !crate::validation::strictly_sorted(&self.board_scopes) {
            return Err(ValidationError::NotCanonicalOrder {
                field: "ruleset.board_scopes",
            });
        }
        if self.metrics.is_empty() || !crate::validation::strictly_sorted(&self.metrics) {
            return Err(ValidationError::InvalidMetrics {
                field: "ruleset.metrics",
            });
        }
        if self.metric_ranking.is_empty()
            || !crate::validation::strictly_sorted(&self.metric_ranking)
            || self
                .metric_ranking
                .iter()
                .map(|policy| policy.metric())
                .ne(self.metrics.iter().copied())
        {
            return Err(ValidationError::ClaimMismatch {
                field: "ruleset.metric_ranking",
            });
        }
        if self.achievement_policies.is_empty()
            || !self
                .achievement_policies
                .windows(2)
                .all(|pair| pair[0].achievement_id < pair[1].achievement_id)
        {
            return Err(ValidationError::NotCanonicalOrder {
                field: "ruleset.achievement_policies",
            });
        }
        for (field, versions) in [
            (
                "ruleset.replay_schema_versions",
                &self.replay_schema_versions,
            ),
            (
                "ruleset.network_protocol_versions",
                &self.network_protocol_versions,
            ),
        ] {
            if versions.is_empty()
                || versions.contains(&0)
                || !crate::validation::strictly_sorted(versions)
            {
                return Err(ValidationError::NotCanonicalOrder { field });
            }
        }
        let expected_replay_schema = crate::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1;
        if self.replay_schema_versions.as_slice() != [expected_replay_schema] {
            return Err(ValidationError::ClaimMismatch {
                field: "ruleset.input_provenance_eligibility.replay_schema_versions",
            });
        }
        for (expected_kind, policy) in [
            (
                ImmutablePolicyKindV1::InputProvenance,
                &self.input_provenance_policy,
            ),
            (
                ImmutablePolicyKindV1::CommandAdmission,
                &self.command_admission_policy,
            ),
            (
                ImmutablePolicyKindV1::SubmissionAdmission,
                &self.submission_admission_policy,
            ),
            (ImmutablePolicyKindV1::Verification, &self.verifier_policy),
        ] {
            policy.validate()?;
            if policy.kind != expected_kind {
                return Err(ValidationError::ClaimMismatch {
                    field: "ruleset.policy_identity.kind",
                });
            }
        }
        self.participant_eligibility.validate()?;
        self.tick_duration.validate()?;
        Ok(())
    }
}

impl RulesetManifestV1 {
    /// Prove that this ruleset's terminal subject is part of the exact
    /// content-addressed campaign catalog selected for a run. A digest
    /// allowlist alone cannot prove subject membership.
    pub fn validate_campaign_completion_catalog(
        &self,
        campaign_content: &CampaignContentManifestV1,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        campaign_content.validate()?;
        let Some(policy) = self.campaign_completion_policy.required() else {
            return Err(ValidationError::ClaimMismatch {
                field: "ruleset.campaign_completion_policy",
            });
        };
        if campaign_content
            .content_for(&policy.terminal_subject)
            .is_none()
        {
            return Err(ValidationError::ClaimMismatch {
                field: "ruleset.campaign_completion_policy.terminal_subject",
            });
        }
        Ok(())
    }

    /// Validates the complete authoritative achievement result against this
    /// ruleset's catalog. This deliberately requires an exact ID match: an
    /// implementation cannot inject an unconfigured achievement or omit a
    /// configured decision without failing closed.
    pub fn validate_authoritative_achievements(
        &self,
        achievements: &[VerifiedAchievementV1],
    ) -> Result<(), ValidationError> {
        if achievements.len() != self.achievement_policies.len()
            || achievements
                .iter()
                .zip(&self.achievement_policies)
                .any(|(result, policy)| result.achievement_id != policy.achievement_id)
        {
            return Err(ValidationError::ClaimMismatch {
                field: "verified_run.achievements.catalog",
            });
        }
        for (result, policy) in achievements.iter().zip(&self.achievement_policies) {
            result.validate()?;
            if policy.mode == AchievementPolicyModeV1::Required
                && result.evaluation == VerifiedAchievementEvaluationV1::Unverifiable
            {
                return Err(ValidationError::ClaimMismatch {
                    field: "verified_run.achievements.required_unverifiable",
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum RulesetOperationalStatusV1 {
    Active,
    Quarantined {
        audit_id: OpaqueId,
        reason_code: String,
        since_unix_ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedRulesetV1 {
    pub schema_version: u32,
    pub ruleset_manifest_sha256: Digest32,
    pub manifest: RulesetManifestV1,
    /// Mutable emergency status may deny use but never changes or reinterprets
    /// the immutable manifest's ranking semantics.
    pub operational_status: RulesetOperationalStatusV1,
}

impl Validate for PublishedRulesetV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("PublishedRulesetV1", self.schema_version)?;
        self.manifest.validate()?;
        if self
            .manifest
            .canonical_digest()
            .map_err(|_| ValidationError::ClaimMismatch {
                field: "published_ruleset.manifest",
            })?
            != self.ruleset_manifest_sha256
        {
            return Err(ValidationError::ClaimMismatch {
                field: "published_ruleset.ruleset_manifest_sha256",
            });
        }
        if let RulesetOperationalStatusV1::Quarantined {
            reason_code,
            since_unix_ms,
            ..
        } = &self.operational_status
        {
            crate::validation::text("published_ruleset.reason_code", reason_code, 128)?;
            if *since_unix_ms == 0 {
                return Err(ValidationError::Zero {
                    field: "published_ruleset.since_unix_ms",
                });
            }
        }
        Ok(())
    }
}

impl Validate for RulesConfigIdentityV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("RulesConfigIdentityV1", self.schema_version)?;
        if self.replay_schema_version == 0 {
            return Err(ValidationError::Zero {
                field: "rules_config.replay_schema_version",
            });
        }
        if self.sim_config.is_empty() {
            return Err(ValidationError::Empty {
                field: "rules_config.sim_config",
            });
        }
        if self.rules.is_empty() {
            return Err(ValidationError::Empty {
                field: "rules_config.rules",
            });
        }
        self.ranked_simulation_policy.validate()?;
        let difficulty_matches = match self.ranked_simulation_policy.difficulty {
            RankedSimulationDifficultyV1::Custom => matches!(self.sim_config.get("difficulty"),
                Some(CanonicalValue::Object(value)) if value.len() == 1 && value.contains_key("Custom")),
            difficulty => {
                self.sim_config.get("difficulty")
                    == Some(&CanonicalValue::String(
                        difficulty.sim_config_wire_name().into(),
                    ))
            }
        };
        if !difficulty_matches {
            return Err(ValidationError::ClaimMismatch {
                field: "rules_config.ranked_simulation_policy.difficulty",
            });
        }
        for (key, value) in self.sim_config.iter().chain(&self.rules) {
            crate::validation::text("rules_config.key", key, 256)?;
            value.validate_depth(64)?;
        }
        Ok(())
    }
}
