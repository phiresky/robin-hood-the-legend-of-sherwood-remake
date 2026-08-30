//! Lossless campaign mission-attempt history.
//!
//! The shipped game keeps only mutable campaign totals and the last status of
//! each mission.  That is sufficient for progression, but it cannot answer
//! questions such as "what happened on my second attempt?".  This module is
//! the append-only record used by the campaign tree and Sherwood museum.
//!
//! Unknown values reconstructed while adopting an Original C++ save are
//! represented as `None`; import never invents evidence that save did not
//! retain. Native Rust campaigns always record complete terminal evidence.

use serde::{Deserialize, Serialize};

use crate::achievement::{
    AchievementAggregationInput, AchievementAggregationPolicy, AchievementAggregationSummary,
    AchievementEvaluation, AchievementId, AchievementRunContext, AchievementSet,
    AchievementUnlockDecision, AchievementUnlockPolicy, MissionAchievementResults,
};
use crate::engine::SimConfig;
use crate::mission_stat::MissionStat;
use crate::player_profile::DifficultyLevel;
use crate::profiles::ProfileManager;

/// Schema of the per-mission history embedded in a native campaign.
pub const CAMPAIGN_HISTORY_SCHEMA_VERSION: u16 = 2;
pub const PROFILE_HISTORY_SCHEMA_VERSION: u16 = 3;

#[repr(u8)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum MissionAttemptOutcome {
    Won = 0,
    Lost = 1,
    Interrupted = 2,
    /// The Original save retained that a mission was recently launched but
    /// did not retain the terminal result of that particular launch.
    Unknown = 3,
}

#[repr(u8)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum MissionAttemptSource {
    /// Recorded at a native Rust engine terminal boundary.
    Native = 0,
    /// Reconstructed from fields serialized by the Original C++ game.
    OriginalSaveImport = 1,
}

#[repr(u8)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum MissionAttemptKind {
    Campaign = 0,
    /// A completed mission launched from campaign history.  Progression and
    /// inventory changes are rolled back, while this record is retained.
    HistoryReplay = 1,
}

/// Durable identity of one attempt across campaign and profile storage.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct MissionAttemptKey {
    pub campaign_run_id: u64,
    pub sequence: u64,
}

/// Host-policy decision attached exactly once to a frozen raw result.
///
/// The deterministic terminal command freezes calculation first. The host
/// then records the eligibility facts on that exact attempt key. Awarded
/// badge unions use this attestation, never the untrusted raw calculation.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct MissionAchievementAttestation {
    policy: AchievementUnlockPolicy,
    context: AchievementRunContext,
    decision: AchievementUnlockDecision,
}

impl MissionAchievementAttestation {
    pub const fn policy(self) -> AchievementUnlockPolicy {
        self.policy
    }

    pub const fn context(self) -> AchievementRunContext {
        self.context
    }

    pub const fn decision(self) -> AchievementUnlockDecision {
        self.decision
    }
}

/// Frozen simulation rules relevant when comparing attempts.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct MissionAttemptRules {
    pub difficulty: DifficultyLevel,
    pub scripts_enabled: bool,
    pub highlander: bool,
    pub highlander2: bool,
    pub golden_eye: bool,
    pub ignore_default_loss: bool,
}

impl From<SimConfig> for MissionAttemptRules {
    fn from(config: SimConfig) -> Self {
        Self {
            difficulty: config.difficulty,
            scripts_enabled: config.script_enabled,
            highlander: config.highlander,
            highlander2: config.highlander2,
            golden_eye: config.golden_eye,
            ignore_default_loss: config.ignore_default_loose,
        }
    }
}

/// Debriefing statistics frozen at the terminal boundary.
///
/// Fields are optional specifically so Original-save adoption can remain
/// honest about evidence that the C++ format discarded.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct MissionAttemptStats {
    pub collected_money: Option<u32>,
    pub bonus_money: Option<u32>,
    pub soldier_money: Option<u32>,
    pub living_soldiers: Option<u32>,
    pub total_soldiers: Option<u32>,
    pub recruited_peasants: Option<u32>,
    pub killed_peasants: Option<u32>,
    pub killed_allies: Option<u32>,
    pub added_score: Option<u32>,
    pub recruited_characters: Option<Vec<crate::mission_stat::PcStatName>>,
}

impl MissionAttemptStats {
    pub fn from_native(stat: &MissionStat) -> Self {
        Self {
            collected_money: Some(stat.collected_money),
            bonus_money: Some(stat.bonus_money),
            soldier_money: Some(stat.soldier_money),
            living_soldiers: Some(stat.living_soldier_count),
            total_soldiers: Some(stat.total_soldier_count),
            recruited_peasants: Some(stat.new_peasant_count),
            killed_peasants: Some(stat.killed_peasant_count),
            killed_allies: Some(stat.killed_allied_count),
            added_score: Some(stat.added_score),
            recruited_characters: Some(stat.pc_names.clone()),
        }
    }

    pub fn is_complete(&self) -> bool {
        self.collected_money.is_some()
            && self.bonus_money.is_some()
            && self.soldier_money.is_some()
            && self.living_soldiers.is_some()
            && self.total_soldiers.is_some()
            && self.recruited_peasants.is_some()
            && self.killed_peasants.is_some()
            && self.killed_allies.is_some()
            && self.added_score.is_some()
            && self.recruited_characters.is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.collected_money.is_none()
            && self.bonus_money.is_none()
            && self.soldier_money.is_none()
            && self.living_soldiers.is_none()
            && self.total_soldiers.is_none()
            && self.recruited_peasants.is_none()
            && self.killed_peasants.is_none()
            && self.killed_allies.is_none()
            && self.added_score.is_none()
            && self.recruited_characters.is_none()
    }
}

/// An immutable completed mission attempt.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct MissionAttempt {
    sequence: u64,
    outcome: MissionAttemptOutcome,
    source: MissionAttemptSource,
    kind: MissionAttemptKind,
    completed_at_unix_seconds: Option<i64>,
    duration_seconds: Option<u32>,
    rules: Option<MissionAttemptRules>,
    stats: MissionAttemptStats,
    achievements: Option<MissionAchievementResults>,
    achievement_attestation: Option<MissionAchievementAttestation>,
}

impl MissionAttempt {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn native(
        sequence: u64,
        outcome: MissionAttemptOutcome,
        kind: MissionAttemptKind,
        completed_at_unix_seconds: Option<i64>,
        duration_seconds: u32,
        config: SimConfig,
        stat: &MissionStat,
        achievements: Option<MissionAchievementResults>,
    ) -> Self {
        assert_ne!(
            outcome,
            MissionAttemptOutcome::Unknown,
            "native mission attempts require a terminal outcome"
        );
        Self {
            sequence,
            outcome,
            source: MissionAttemptSource::Native,
            kind,
            completed_at_unix_seconds,
            duration_seconds: Some(duration_seconds),
            rules: Some(config.into()),
            stats: MissionAttemptStats::from_native(stat),
            achievements,
            achievement_attestation: None,
        }
    }

    pub(crate) fn original_save_import(sequence: u64, outcome: MissionAttemptOutcome) -> Self {
        Self {
            sequence,
            outcome,
            source: MissionAttemptSource::OriginalSaveImport,
            kind: MissionAttemptKind::Campaign,
            completed_at_unix_seconds: None,
            duration_seconds: None,
            rules: None,
            stats: MissionAttemptStats::default(),
            achievements: None,
            achievement_attestation: None,
        }
    }

    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub const fn outcome(&self) -> MissionAttemptOutcome {
        self.outcome
    }

    pub const fn source(&self) -> MissionAttemptSource {
        self.source
    }

    pub const fn kind(&self) -> MissionAttemptKind {
        self.kind
    }

    pub const fn completed_at_unix_seconds(&self) -> Option<i64> {
        self.completed_at_unix_seconds
    }

    pub const fn duration_seconds(&self) -> Option<u32> {
        self.duration_seconds
    }

    pub const fn rules(&self) -> Option<MissionAttemptRules> {
        self.rules
    }

    pub const fn stats(&self) -> &MissionAttemptStats {
        &self.stats
    }

    pub const fn achievements(&self) -> Option<MissionAchievementResults> {
        self.achievements
    }

    pub const fn achievement_attestation(&self) -> Option<MissionAchievementAttestation> {
        self.achievement_attestation
    }

    pub const fn key(&self, campaign_run_id: u64) -> MissionAttemptKey {
        MissionAttemptKey {
            campaign_run_id,
            sequence: self.sequence,
        }
    }

    fn attest_achievements(
        &mut self,
        policy: AchievementUnlockPolicy,
        context: AchievementRunContext,
    ) -> Result<MissionAchievementAttestation, MissionAchievementAttestationError> {
        let results = self.achievements.ok_or(
            MissionAchievementAttestationError::AttemptHasNoCalculatedResults {
                sequence: self.sequence,
            },
        )?;
        let attestation = MissionAchievementAttestation {
            policy,
            context,
            decision: policy.evaluate(context, results),
        };
        match self.achievement_attestation {
            None => self.achievement_attestation = Some(attestation),
            Some(existing) if existing == attestation => {}
            Some(_) => {
                return Err(MissionAchievementAttestationError::ConflictingAttestation {
                    sequence: self.sequence,
                });
            }
        }
        Ok(attestation)
    }

    fn merge_attestation_from(&mut self, source: &Self) -> Result<bool, ()> {
        let mut existing_raw = self.clone();
        existing_raw.achievement_attestation = None;
        let mut source_raw = source.clone();
        source_raw.achievement_attestation = None;
        if existing_raw != source_raw {
            return Err(());
        }
        match (self.achievement_attestation, source.achievement_attestation) {
            (None, Some(attestation)) => {
                self.achievement_attestation = Some(attestation);
                Ok(true)
            }
            (Some(existing), Some(source)) if existing != source => Err(()),
            _ => Ok(false),
        }
    }

    pub fn validate_evidence(&self) -> Result<(), &'static str> {
        match self.source {
            MissionAttemptSource::Native => {
                if self.outcome == MissionAttemptOutcome::Unknown {
                    return Err("native mission attempt has an unknown outcome");
                }
                if self.duration_seconds.is_none()
                    || self.rules.is_none()
                    || !self.stats.is_complete()
                {
                    return Err("native mission attempt is missing replay-grade evidence");
                }
                if self.achievement_attestation.is_some() && self.achievements.is_none() {
                    return Err("mission attempt attestation has no calculated results");
                }
            }
            MissionAttemptSource::OriginalSaveImport => {
                if self.completed_at_unix_seconds.is_some()
                    || self.duration_seconds.is_some()
                    || self.rules.is_some()
                    || !self.stats.is_empty()
                    || self.achievements.is_some()
                    || self.achievement_attestation.is_some()
                {
                    return Err("Original save import fabricated unavailable attempt evidence");
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissionAchievementAttestationError {
    CampaignHasNoRunId,
    CampaignRunMismatch { expected: u64, actual: u64 },
    AttemptNotFound(MissionAttemptKey),
    AttemptHasNoCalculatedResults { sequence: u64 },
    ConflictingAttestation { sequence: u64 },
}

impl std::fmt::Display for MissionAchievementAttestationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CampaignHasNoRunId => {
                write!(
                    formatter,
                    "achievement attestation campaign has no run identity"
                )
            }
            Self::CampaignRunMismatch { expected, actual } => write!(
                formatter,
                "achievement attestation campaign run mismatch: expected {expected}, got {actual}"
            ),
            Self::AttemptNotFound(key) => write!(
                formatter,
                "achievement attestation attempt {} was not found in campaign run {}",
                key.sequence, key.campaign_run_id
            ),
            Self::AttemptHasNoCalculatedResults { sequence } => write!(
                formatter,
                "mission attempt {sequence} has no calculated achievement results"
            ),
            Self::ConflictingAttestation { sequence } => write!(
                formatter,
                "mission attempt {sequence} already has a different eligibility attestation"
            ),
        }
    }
}

impl std::error::Error for MissionAchievementAttestationError {}

/// Append-only history for one mission.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct MissionAttemptHistory {
    schema_version: u16,
    attempts: Vec<MissionAttempt>,
}

impl Default for MissionAttemptHistory {
    fn default() -> Self {
        Self {
            schema_version: CAMPAIGN_HISTORY_SCHEMA_VERSION,
            attempts: Vec::new(),
        }
    }
}

impl MissionAttemptHistory {
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    pub fn attempts(&self) -> &[MissionAttempt] {
        &self.attempts
    }

    pub fn latest(&self) -> Option<&MissionAttempt> {
        self.attempts.last()
    }

    pub fn has_success(&self) -> bool {
        self.attempts
            .iter()
            .any(|attempt| attempt.outcome == MissionAttemptOutcome::Won)
    }

    pub fn eligible_badges(&self) -> AchievementSet {
        self.attempts
            .iter()
            .fold(AchievementSet::empty(), |mut badges, attempt| {
                if let Some(attestation) = attempt.achievement_attestation {
                    badges.union_with(attestation.decision.eligible_earned);
                }
                badges
            })
    }

    pub fn best_eligible_achievement(
        &self,
        id: crate::achievement::AchievementId,
    ) -> Option<crate::achievement::AchievementEvaluation> {
        self.attempts
            .iter()
            .filter(|attempt| {
                attempt
                    .achievement_attestation
                    .is_some_and(|attestation| attestation.decision.may_persist())
            })
            .filter_map(|attempt| attempt.achievements)
            .map(|results| results.evaluation(id))
            .max_by_key(|evaluation| evaluation.history_rank())
    }

    pub(crate) fn attest_achievements(
        &mut self,
        sequence: u64,
        policy: AchievementUnlockPolicy,
        context: AchievementRunContext,
    ) -> Result<Option<MissionAchievementAttestation>, MissionAchievementAttestationError> {
        let Some(attempt) = self
            .attempts
            .iter_mut()
            .find(|attempt| attempt.sequence == sequence)
        else {
            return Ok(None);
        };
        attempt.attest_achievements(policy, context).map(Some)
    }

    pub(crate) fn append(&mut self, attempt: MissionAttempt) {
        assert_eq!(
            self.schema_version, CAMPAIGN_HISTORY_SCHEMA_VERSION,
            "cannot append to unsupported campaign history schema"
        );
        if let Some(previous) = self.attempts.last() {
            assert!(
                attempt.sequence() > previous.sequence(),
                "mission attempt sequence must be strictly increasing"
            );
        }
        self.attempts.push(attempt);
    }

    pub fn validate_schema(&self) -> Result<(), u16> {
        if self.schema_version != CAMPAIGN_HISTORY_SCHEMA_VERSION {
            return Err(self.schema_version);
        }
        let mut previous = None;
        for attempt in &self.attempts {
            if previous.is_some_and(|sequence| attempt.sequence() <= sequence) {
                return Err(self.schema_version);
            }
            if attempt.validate_evidence().is_err() {
                return Err(self.schema_version);
            }
            previous = Some(attempt.sequence());
        }
        Ok(())
    }
}

/// Totals derived from immutable attempt records.  No redundant aggregate is
/// serialized, so edited/corrupt data cannot leave two sources of truth.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignHistoryTotals {
    pub attempts: u64,
    pub wins: u64,
    pub losses: u64,
    pub interrupted: u64,
    pub unknown_outcomes: u64,
    pub known_duration_seconds: u64,
    pub known_score: u64,
    pub known_money: u64,
    pub incomplete_attempts: u64,
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct LifetimeMissionAttempt {
    campaign_run_id: u64,
    mission_id: u32,
    mission_name: String,
    attempt: MissionAttempt,
}

/// Frozen identity of one completed canonical campaign path.
///
/// Attempts remain the only achievement-evidence source. This envelope stores
/// only the fact that the Original completion boundary was crossed and the
/// exact successful path which AllRequiredMissions must evaluate. A later
/// practice replay may add evidence for those IDs but cannot rewrite the set.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct LifetimeCampaignAchievementEnvelope {
    campaign_run_id: u64,
    completion_sequence: u64,
    required_mission_ids: Vec<u32>,
}

impl LifetimeCampaignAchievementEnvelope {
    pub const fn campaign_run_id(&self) -> u64 {
        self.campaign_run_id
    }

    pub const fn completion_sequence(&self) -> u64 {
        self.completion_sequence
    }

    pub fn required_mission_ids(&self) -> &[u32] {
        &self.required_mission_ids
    }

    fn validate(&self) -> Result<(), String> {
        if self.campaign_run_id == 0 {
            return Err("completed campaign achievement envelope has run ID zero".to_owned());
        }
        if self.completion_sequence == 0 {
            return Err(format!(
                "completed campaign achievement envelope {} has sequence zero",
                self.campaign_run_id
            ));
        }
        if self.required_mission_ids.is_empty() {
            return Err(format!(
                "completed campaign achievement envelope {} has no required missions",
                self.campaign_run_id
            ));
        }
        if self.required_mission_ids.iter().any(|&id| id == 0) {
            return Err(format!(
                "completed campaign achievement envelope {} contains mission ID zero",
                self.campaign_run_id
            ));
        }
        if self
            .required_mission_ids
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            return Err(format!(
                "completed campaign achievement envelope {} mission IDs are not strictly sorted and unique",
                self.campaign_run_id
            ));
        }
        Ok(())
    }
}

impl LifetimeMissionAttempt {
    pub const fn campaign_run_id(&self) -> u64 {
        self.campaign_run_id
    }

    pub const fn mission_id(&self) -> u32 {
        self.mission_id
    }

    pub fn mission_name(&self) -> &str {
        &self.mission_name
    }

    pub const fn attempt(&self) -> &MissionAttempt {
        &self.attempt
    }
}

/// Versioned lossless attempt archive owned by the player profile rather than
/// a replaceable campaign/save slot.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ProfileCampaignHistory {
    schema_version: u16,
    attempts: Vec<LifetimeMissionAttempt>,
    /// Frozen completed paths used for lifetime AllRequiredMissions badges.
    /// This field is mandatory: obsolete Rust profile schemas fail closed.
    completed_campaigns: Vec<LifetimeCampaignAchievementEnvelope>,
}

impl Default for ProfileCampaignHistory {
    fn default() -> Self {
        Self {
            schema_version: PROFILE_HISTORY_SCHEMA_VERSION,
            attempts: Vec::new(),
            completed_campaigns: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileHistoryPromotionError {
    UnsupportedSchema(u16),
    MissingCampaignRunId { stored_attempts: usize },
    MissingMissionProfile { mission_index: usize },
    ConflictingAttempt(MissionAttemptKey),
    ConflictingCampaignEnvelope { campaign_run_id: u64 },
}

impl std::fmt::Display for ProfileHistoryPromotionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedSchema(version) => {
                write!(
                    formatter,
                    "unsupported profile campaign-history schema {version}"
                )
            }
            Self::MissingCampaignRunId { stored_attempts } => write!(
                formatter,
                "campaign has {stored_attempts} stored attempt(s) but no run identity"
            ),
            Self::MissingMissionProfile { mission_index } => {
                write!(formatter, "campaign mission {mission_index} has no profile")
            }
            Self::ConflictingAttempt(key) => write!(
                formatter,
                "lifetime campaign attempt {} in run {} conflicts with its canonical campaign record",
                key.sequence, key.campaign_run_id
            ),
            Self::ConflictingCampaignEnvelope { campaign_run_id } => write!(
                formatter,
                "completed campaign envelope for run {campaign_run_id} conflicts with its canonical path"
            ),
        }
    }
}

impl std::error::Error for ProfileHistoryPromotionError {}

impl ProfileCampaignHistory {
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    pub fn attempts(&self) -> &[LifetimeMissionAttempt] {
        &self.attempts
    }

    pub fn completed_campaigns(&self) -> &[LifetimeCampaignAchievementEnvelope] {
        &self.completed_campaigns
    }
    pub fn totals(&self) -> CampaignHistoryTotals {
        let mut totals = CampaignHistoryTotals::default();
        for entry in &self.attempts {
            totals.include(entry.attempt());
        }
        totals
    }

    pub fn achievement_aggregation(&self) -> AchievementAggregationSummary {
        AchievementAggregationSummary::from_inputs(|id| {
            let mut any_earned_missions = std::collections::BTreeSet::new();
            let mut any_unverifiable_missions = std::collections::BTreeSet::new();
            for entry in &self.attempts {
                if entry.attempt.outcome != MissionAttemptOutcome::Won {
                    continue;
                }
                if entry
                    .attempt
                    .achievement_attestation
                    .is_some_and(|attestation| attestation.decision.eligible_earned.contains(id))
                {
                    any_earned_missions.insert(entry.mission_id);
                } else if lifetime_attempt_evidence_incomplete(&entry.attempt, id) {
                    any_unverifiable_missions.insert(entry.mission_id);
                }
            }
            for mission_id in &any_earned_missions {
                any_unverifiable_missions.remove(mission_id);
            }

            let mut best_required = 0_u32;
            let mut best_earned = 0_u32;
            let mut best_unverifiable = 0_u32;
            let mut best_category = 0_u8;
            let mut have_envelope = false;
            for envelope in &self.completed_campaigns {
                let required = u32::try_from(envelope.required_mission_ids.len())
                    .expect("lifetime campaign required mission count exceeds u32");
                let mut earned = 0_u32;
                let mut unverifiable = 0_u32;
                for &mission_id in &envelope.required_mission_ids {
                    let matching = self.attempts.iter().filter(|entry| {
                        entry.campaign_run_id == envelope.campaign_run_id
                            && entry.mission_id == mission_id
                            && entry.attempt.outcome == MissionAttemptOutcome::Won
                    });
                    let matching = matching.collect::<Vec<_>>();
                    if matching.iter().any(|entry| {
                        entry
                            .attempt
                            .achievement_attestation
                            .is_some_and(|attestation| {
                                attestation.decision.eligible_earned.contains(id)
                            })
                    }) {
                        earned = earned
                            .checked_add(1)
                            .expect("lifetime earned mission count exceeds u32");
                    } else if matching.is_empty()
                        || matching
                            .iter()
                            .any(|entry| lifetime_attempt_evidence_incomplete(&entry.attempt, id))
                    {
                        unverifiable = unverifiable
                            .checked_add(1)
                            .expect("lifetime unverifiable mission count exceeds u32");
                    }
                }
                let category = if required != 0 && earned == required && unverifiable == 0 {
                    3
                } else if unverifiable != 0 {
                    2
                } else {
                    1
                };
                let better_fraction = u64::from(earned) * u64::from(best_required)
                    > u64::from(best_earned) * u64::from(required);
                let better = !have_envelope
                    || category > best_category
                    || (category == best_category && better_fraction)
                    || (category == best_category
                        && !better_fraction
                        && earned == best_earned
                        && unverifiable < best_unverifiable);
                if better {
                    have_envelope = true;
                    best_category = category;
                    best_required = required;
                    best_earned = earned;
                    best_unverifiable = unverifiable;
                }
            }

            let (earned_missions, required_missions, unverifiable_missions) =
                match id.aggregation_policy() {
                    AchievementAggregationPolicy::AllRequiredMissions => {
                        (best_earned, best_required, best_unverifiable)
                    }
                    AchievementAggregationPolicy::AnyMissionOnce => (
                        u32::from(!any_earned_missions.is_empty()),
                        u32::from(!self.attempts.is_empty()),
                        u32::from(
                            any_earned_missions.is_empty() && !any_unverifiable_missions.is_empty(),
                        ),
                    ),
                };
            AchievementAggregationInput {
                envelope_complete: have_envelope,
                envelope_unverifiable: false,
                earned_missions,
                required_missions,
                unverifiable_missions,
            }
        })
    }

    pub fn eligible_badges(&self) -> AchievementSet {
        self.achievement_aggregation().earned()
    }

    pub fn eligible_badges_for_mission(&self, mission_id: u32) -> AchievementSet {
        self.attempts
            .iter()
            .filter(|entry| entry.mission_id == mission_id)
            .fold(AchievementSet::empty(), |mut badges, entry| {
                if let Some(attestation) = entry.attempt.achievement_attestation {
                    badges.union_with(attestation.decision.eligible_earned);
                }
                badges
            })
    }

    /// Idempotently promote every campaign attempt into lifetime
    /// storage. `(campaign_run_id, attempt.sequence)` is the durable key.
    /// The returned count includes an existing raw entry refreshed with its
    /// later exactly-once eligibility attestation.
    pub fn promote_campaign(
        &mut self,
        campaign: &crate::campaign::Campaign,
        profiles: &ProfileManager,
    ) -> Result<usize, ProfileHistoryPromotionError> {
        if self.schema_version != PROFILE_HISTORY_SCHEMA_VERSION {
            return Err(ProfileHistoryPromotionError::UnsupportedSchema(
                self.schema_version,
            ));
        }
        let stored_attempts = campaign
            .missions
            .iter()
            .flat_map(|mission| mission.attempt_history().attempts())
            .count();
        if stored_attempts == 0 {
            return Ok(0);
        }
        let Some(campaign_run_id) = campaign.history_run_id() else {
            let native_attempts = campaign
                .missions
                .iter()
                .flat_map(|mission| mission.attempt_history().attempts())
                .filter(|attempt| attempt.source() == MissionAttemptSource::Native)
                .count();
            if native_attempts != 0 {
                return Err(ProfileHistoryPromotionError::MissingCampaignRunId { stored_attempts });
            }
            // An Original save has no durable Rust run identity. Keep its
            // incomplete records in the live campaign until the first native
            // terminal command assigns one; never invent an identity merely
            // to satisfy profile storage.
            return Ok(0);
        };
        let mut added = 0;
        for (mission_index, mission) in campaign.missions.iter().enumerate() {
            let attempts = mission.attempt_history().attempts();
            if attempts.is_empty() {
                continue;
            }
            let profile_index = mission
                .profile_idx
                .ok_or(ProfileHistoryPromotionError::MissingMissionProfile { mission_index })?
                as usize;
            let profile = profiles
                .missions
                .get(profile_index)
                .ok_or(ProfileHistoryPromotionError::MissingMissionProfile { mission_index })?;
            for attempt in attempts {
                let existing = self.attempts.iter_mut().find(|entry| {
                    entry.campaign_run_id == campaign_run_id
                        && entry.attempt.sequence() == attempt.sequence()
                });
                if let Some(existing) = existing {
                    if existing.mission_id != profile.id
                        || existing.mission_name != profile.mission_name
                    {
                        return Err(ProfileHistoryPromotionError::ConflictingAttempt(
                            attempt.key(campaign_run_id),
                        ));
                    }
                    let refreshed =
                        existing
                            .attempt
                            .merge_attestation_from(attempt)
                            .map_err(|()| {
                                ProfileHistoryPromotionError::ConflictingAttempt(
                                    attempt.key(campaign_run_id),
                                )
                            })?;
                    added += usize::from(refreshed);
                } else {
                    self.attempts.push(LifetimeMissionAttempt {
                        campaign_run_id,
                        mission_id: profile.id,
                        mission_name: profile.mission_name.clone(),
                        attempt: attempt.clone(),
                    });
                    added += 1;
                }
            }
        }
        self.attempts
            .sort_by_key(|entry| (entry.campaign_run_id, entry.attempt.sequence()));

        if campaign.achievement_envelope_complete(profiles) {
            match self
                .completed_campaigns
                .iter()
                .find(|envelope| envelope.campaign_run_id == campaign_run_id)
            {
                Some(existing) => {
                    let required_mission_ids = campaign
                        .required_achievement_mission_ids_through(
                            profiles,
                            existing.completion_sequence,
                        )
                        .into_iter()
                        .collect::<Vec<_>>();
                    if existing.required_mission_ids != required_mission_ids {
                        return Err(ProfileHistoryPromotionError::ConflictingCampaignEnvelope {
                            campaign_run_id,
                        });
                    }
                }
                None => {
                    let completion_sequence = campaign
                        .missions
                        .iter()
                        .flat_map(|mission| mission.attempt_history().attempts())
                        .filter(|attempt| attempt.kind() == MissionAttemptKind::Campaign)
                        .map(MissionAttempt::sequence)
                        .max()
                        .expect(
                            "completed native campaign has no ordinary mission attempt sequence",
                        );
                    let required_mission_ids = campaign
                        .required_achievement_mission_ids_through(profiles, completion_sequence)
                        .into_iter()
                        .collect::<Vec<_>>();
                    assert!(
                        !required_mission_ids.is_empty(),
                        "completed campaign run {campaign_run_id} has no required canonical missions"
                    );
                    self.completed_campaigns
                        .push(LifetimeCampaignAchievementEnvelope {
                            campaign_run_id,
                            completion_sequence,
                            required_mission_ids,
                        });
                    self.completed_campaigns
                        .sort_by_key(|envelope| envelope.campaign_run_id);
                    added += 1;
                }
            }
        }
        Ok(added)
    }

    pub fn validate_schema(&self) -> Result<(), u16> {
        if self.schema_version != PROFILE_HISTORY_SCHEMA_VERSION {
            return Err(self.schema_version);
        }
        let mut keys = std::collections::BTreeSet::new();
        let mut previous_key = None;
        for entry in &self.attempts {
            let key = (entry.campaign_run_id, entry.attempt.sequence());
            if previous_key.is_some_and(|previous| key <= previous)
                || !keys.insert(key)
                || entry.attempt.validate_evidence().is_err()
            {
                return Err(self.schema_version);
            }
            previous_key = Some(key);
        }
        if self
            .completed_campaigns
            .iter()
            .any(|envelope| envelope.validate().is_err())
            || self
                .completed_campaigns
                .windows(2)
                .any(|pair| pair[0].campaign_run_id >= pair[1].campaign_run_id)
        {
            return Err(self.schema_version);
        }
        for envelope in &self.completed_campaigns {
            let completion_exists = self.attempts.iter().any(|entry| {
                entry.campaign_run_id == envelope.campaign_run_id
                    && entry.attempt.sequence() == envelope.completion_sequence
                    && entry.attempt.kind() == MissionAttemptKind::Campaign
            });
            let every_requirement_has_path_win =
                envelope.required_mission_ids.iter().all(|&mission_id| {
                    self.attempts.iter().any(|entry| {
                        entry.campaign_run_id == envelope.campaign_run_id
                            && entry.mission_id == mission_id
                            && entry.attempt.sequence() <= envelope.completion_sequence
                            && entry.attempt.kind() == MissionAttemptKind::Campaign
                            && entry.attempt.outcome() == MissionAttemptOutcome::Won
                    })
                });
            if !completion_exists || !every_requirement_has_path_win {
                return Err(self.schema_version);
            }
        }
        Ok(())
    }
}

fn lifetime_attempt_evidence_incomplete(attempt: &MissionAttempt, id: AchievementId) -> bool {
    if attempt.outcome != MissionAttemptOutcome::Won {
        return false;
    }
    if attempt.source == MissionAttemptSource::OriginalSaveImport {
        return true;
    }
    let Some(results) = attempt.achievements else {
        return true;
    };
    match results.evaluation(id) {
        AchievementEvaluation::Unverifiable => true,
        AchievementEvaluation::Failed => false,
        AchievementEvaluation::Earned => attempt.achievement_attestation.is_none(),
    }
}

impl CampaignHistoryTotals {
    pub fn include(&mut self, attempt: &MissionAttempt) {
        self.attempts += 1;
        match attempt.outcome() {
            MissionAttemptOutcome::Won => self.wins += 1,
            MissionAttemptOutcome::Lost => self.losses += 1,
            MissionAttemptOutcome::Interrupted => self.interrupted += 1,
            MissionAttemptOutcome::Unknown => self.unknown_outcomes += 1,
        }
        if let Some(duration) = attempt.duration_seconds() {
            self.known_duration_seconds += u64::from(duration);
        }
        if let Some(score) = attempt.stats().added_score {
            self.known_score += u64::from(score);
        }
        if let Some(money) = attempt.stats().collected_money {
            self.known_money += u64::from(money);
        }
        if attempt.duration_seconds().is_none()
            || attempt.rules().is_none()
            || !attempt.stats().is_complete()
        {
            self.incomplete_attempts += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aggregation_profiles() -> ProfileManager {
        let mut profiles = ProfileManager::new();
        profiles.missions.push(crate::profiles::MissionProfile {
            id: 1,
            mission_name: "Sherwood".into(),
            mission_type: crate::profiles::MissionType::Hq,
            ..Default::default()
        });
        profiles.missions.push(crate::profiles::MissionProfile {
            id: 0x3141,
            mission_name: "Field One".into(),
            mission_type: crate::profiles::MissionType::Historical,
            ..Default::default()
        });
        profiles.missions.push(crate::profiles::MissionProfile {
            id: 0x4948,
            mission_name: "H12".into(),
            mission_type: crate::profiles::MissionType::Historical,
            ..Default::default()
        });
        profiles
    }

    fn aggregation_campaign(profiles: &ProfileManager) -> crate::campaign::Campaign {
        let mut campaign = crate::campaign::Campaign::default();
        campaign.missions = profiles
            .missions
            .iter()
            .enumerate()
            .map(|(index, _)| crate::mission::Mission {
                profile_idx: Some(index as u32),
                ..crate::mission::Mission::new()
            })
            .collect();
        campaign
    }

    fn achievement_result(
        id: AchievementId,
        evaluation: AchievementEvaluation,
    ) -> MissionAchievementResults {
        let mut tracker = crate::achievement::MissionAchievementState::from_mission_start();
        tracker.record_evaluation(id, evaluation).unwrap();
        *tracker.finalize_success()
    }

    fn record_attested_win(
        campaign: &mut crate::campaign::Campaign,
        profiles: &ProfileManager,
        mission_index: usize,
        id: AchievementId,
        evaluation: AchievementEvaluation,
        run_id: u64,
    ) {
        campaign.missions[mission_index].status = crate::mission::MissionStatus::Won;
        campaign.current_mission_idx = Some(mission_index);
        campaign.record_mission_attempt(
            mission_index,
            MissionAttemptOutcome::Won,
            Some(100),
            Some(run_id),
            60,
            SimConfig::default(),
            &MissionStat::default(),
            Some(achievement_result(id, evaluation)),
        );
        campaign
            .attest_mission_achievement_attempt(
                campaign.latest_mission_attempt_key().unwrap(),
                AchievementUnlockPolicy::default(),
                AchievementRunContext::default(),
                profiles,
            )
            .unwrap();
    }

    #[test]
    fn original_save_attempts_preserve_unknowns_instead_of_faking_zeroes() {
        let attempt = MissionAttempt::original_save_import(1, MissionAttemptOutcome::Unknown);
        assert_eq!(attempt.duration_seconds(), None);
        assert_eq!(attempt.rules(), None);
        assert_eq!(attempt.stats().added_score, None);
        assert!(!attempt.stats().is_complete());
    }

    #[test]
    fn derived_totals_report_incomplete_original_imports() {
        let mut totals = CampaignHistoryTotals::default();
        totals.include(&MissionAttempt::original_save_import(
            1,
            MissionAttemptOutcome::Unknown,
        ));
        assert_eq!(totals.attempts, 1);
        assert_eq!(totals.unknown_outcomes, 1);
        assert_eq!(totals.incomplete_attempts, 1);
    }

    #[test]
    fn original_import_waits_for_a_real_run_identity_before_profile_promotion() {
        let mut campaign = crate::campaign::Campaign::default();
        campaign.missions.push(crate::mission::Mission {
            profile_idx: Some(0),
            ..crate::mission::Mission::new()
        });
        campaign.reconstruct_original_save_history(&[0]);
        let mut profiles = ProfileManager::new();
        profiles.missions.push(crate::profiles::MissionProfile {
            id: 42,
            mission_name: "Imported mission".into(),
            ..Default::default()
        });
        let mut lifetime = ProfileCampaignHistory::default();

        assert_eq!(lifetime.promote_campaign(&campaign, &profiles).unwrap(), 0);
        assert!(lifetime.attempts().is_empty());
        assert!(lifetime.completed_campaigns().is_empty());
    }

    #[test]
    fn first_native_terminal_archives_original_attempts_without_synthesizing_import_identity() {
        let profiles = aggregation_profiles();
        let mut campaign = aggregation_campaign(&profiles);
        campaign.missions[2].status = crate::mission::MissionStatus::Won;
        campaign.reconstruct_original_save_history(&[]);

        let mut lifetime = ProfileCampaignHistory::default();
        assert_eq!(lifetime.promote_campaign(&campaign, &profiles).unwrap(), 0);
        assert!(lifetime.attempts().is_empty());

        record_attested_win(
            &mut campaign,
            &profiles,
            1,
            AchievementId::Ghost,
            AchievementEvaluation::Earned,
            0xace,
        );
        assert_eq!(lifetime.promote_campaign(&campaign, &profiles).unwrap(), 3);
        assert_eq!(lifetime.attempts().len(), 2);
        assert_eq!(lifetime.completed_campaigns().len(), 1);
        assert_eq!(lifetime.completed_campaigns()[0].campaign_run_id(), 0xace);
        assert_eq!(
            lifetime
                .achievement_aggregation()
                .get(AchievementId::Ghost)
                .status,
            crate::achievement::AchievementAggregationStatus::Unverifiable,
            "the imported won mission remains an honest evidence gap"
        );
    }

    #[test]
    fn obsolete_rust_campaign_and_profile_history_schemas_fail_closed() {
        let mut campaign_json = serde_json::to_value(MissionAttemptHistory::default()).unwrap();
        campaign_json["schema_version"] = serde_json::json!(1);
        let campaign_history: MissionAttemptHistory =
            serde_json::from_value(campaign_json).unwrap();
        assert_eq!(campaign_history.validate_schema(), Err(1));

        let mut profile_json = serde_json::to_value(ProfileCampaignHistory::default()).unwrap();
        profile_json["schema_version"] = serde_json::json!(1);
        let profile_history: ProfileCampaignHistory = serde_json::from_value(profile_json).unwrap();
        assert_eq!(profile_history.validate_schema(), Err(1));
    }

    #[test]
    fn original_import_with_fabricated_evidence_is_rejected() {
        let mut json = serde_json::to_value(MissionAttempt::original_save_import(
            1,
            MissionAttemptOutcome::Unknown,
        ))
        .unwrap();
        json["duration_seconds"] = serde_json::json!(0);
        let attempt: MissionAttempt = serde_json::from_value(json).unwrap();
        assert_eq!(
            attempt.validate_evidence(),
            Err("Original save import fabricated unavailable attempt evidence")
        );
    }

    #[test]
    fn native_stats_preserve_every_mission_stat_field() {
        let original = MissionStat {
            collected_money: 101,
            bonus_money: 202,
            soldier_money: 303,
            living_soldier_count: 4,
            total_soldier_count: 5,
            new_peasant_count: 6,
            killed_peasant_count: 7,
            killed_allied_count: 8,
            added_score: 909,
            pc_names: vec![crate::mission_stat::PcStatName::new(
                "Rescued outlaw".into(),
                Some(crate::pc_status::SpecialPeasantName::B),
            )],
        };
        let frozen = MissionAttemptStats::from_native(&original);

        assert_eq!(frozen.collected_money, Some(original.collected_money));
        assert_eq!(frozen.bonus_money, Some(original.bonus_money));
        assert_eq!(frozen.soldier_money, Some(original.soldier_money));
        assert_eq!(frozen.living_soldiers, Some(original.living_soldier_count));
        assert_eq!(frozen.total_soldiers, Some(original.total_soldier_count));
        assert_eq!(frozen.recruited_peasants, Some(original.new_peasant_count));
        assert_eq!(frozen.killed_peasants, Some(original.killed_peasant_count));
        assert_eq!(frozen.killed_allies, Some(original.killed_allied_count));
        assert_eq!(frozen.added_score, Some(original.added_score));
        assert_eq!(frozen.recruited_characters, Some(original.pc_names));
        assert!(frozen.is_complete());
    }

    #[test]
    fn versioned_attempt_history_round_trips_losslessly() {
        let mut history = MissionAttemptHistory::default();
        history.append(MissionAttempt::native(
            17,
            MissionAttemptOutcome::Won,
            MissionAttemptKind::HistoryReplay,
            Some(1_700_000_000),
            321,
            SimConfig::default(),
            &MissionStat {
                collected_money: 111,
                bonus_money: 222,
                soldier_money: 333,
                added_score: 444,
                ..MissionStat::default()
            },
            None,
        ));

        let json = serde_json::to_string(&history).expect("serialize mission attempt history");
        let restored: MissionAttemptHistory =
            serde_json::from_str(&json).expect("deserialize mission attempt history");
        assert_eq!(restored, history);
        assert_eq!(restored.schema_version(), CAMPAIGN_HISTORY_SCHEMA_VERSION);
        assert!(restored.attempts()[0].stats().is_complete());
    }

    #[test]
    fn practice_replay_fills_frozen_lifetime_envelope_and_survives_campaign_replacement() {
        let profiles = aggregation_profiles();
        let mut campaign = aggregation_campaign(&profiles);
        let run_id = 0x55aa;
        record_attested_win(
            &mut campaign,
            &profiles,
            1,
            AchievementId::Ghost,
            AchievementEvaluation::Earned,
            run_id,
        );
        record_attested_win(
            &mut campaign,
            &profiles,
            2,
            AchievementId::Ghost,
            AchievementEvaluation::Failed,
            run_id,
        );

        let mut lifetime = ProfileCampaignHistory::default();
        assert_eq!(lifetime.promote_campaign(&campaign, &profiles).unwrap(), 3);
        assert_eq!(lifetime.completed_campaigns().len(), 1);
        let completion_sequence = lifetime.completed_campaigns()[0].completion_sequence();
        assert_eq!(
            lifetime
                .achievement_aggregation()
                .get(AchievementId::Ghost)
                .status,
            crate::achievement::AchievementAggregationStatus::MissingRequirements
        );

        campaign.select_next_mission(Some(2), &profiles);
        campaign.snapshot_with_simulation(9, SimConfig::default());
        campaign.current_mission_idx = Some(2);
        campaign.record_mission_attempt(
            2,
            MissionAttemptOutcome::Won,
            Some(200),
            Some(run_id),
            40,
            SimConfig::default(),
            &MissionStat::default(),
            Some(achievement_result(
                AchievementId::Ghost,
                AchievementEvaluation::Earned,
            )),
        );
        campaign
            .attest_mission_achievement_attempt(
                campaign.latest_mission_attempt_key().unwrap(),
                AchievementUnlockPolicy::default(),
                AchievementRunContext::default(),
                &profiles,
            )
            .unwrap();
        assert_eq!(lifetime.promote_campaign(&campaign, &profiles).unwrap(), 1);
        assert_eq!(lifetime.completed_campaigns().len(), 1);
        assert_eq!(
            lifetime.completed_campaigns()[0].completion_sequence(),
            completion_sequence
        );
        assert!(
            lifetime
                .achievement_aggregation()
                .earned()
                .contains(AchievementId::Ghost)
        );

        let replacement_campaign = aggregation_campaign(&profiles);
        assert!(
            replacement_campaign
                .achievement_aggregation(&profiles)
                .earned()
                .is_empty()
        );
        assert!(
            lifetime
                .achievement_aggregation()
                .earned()
                .contains(AchievementId::Ghost),
            "replacing/resetting the campaign must not erase the profile archive"
        );

        let json = serde_json::to_string(&lifetime).unwrap();
        let from_json: ProfileCampaignHistory = serde_json::from_str(&json).unwrap();
        assert_eq!(from_json, lifetime);
        let bytes = bitcode::encode(&lifetime);
        let from_bitcode: ProfileCampaignHistory = bitcode::decode(&bytes).unwrap();
        assert_eq!(from_bitcode, lifetime);
    }

    #[test]
    fn ordinary_wins_after_completion_do_not_rewrite_the_frozen_path() {
        let profiles = aggregation_profiles();
        let run_id = 0x44;
        let mut campaign = aggregation_campaign(&profiles);
        record_attested_win(
            &mut campaign,
            &profiles,
            2,
            AchievementId::CleanHands,
            AchievementEvaluation::Earned,
            run_id,
        );

        let mut lifetime = ProfileCampaignHistory::default();
        assert_eq!(lifetime.promote_campaign(&campaign, &profiles).unwrap(), 2);
        let frozen = lifetime.completed_campaigns()[0].clone();
        assert_eq!(frozen.required_mission_ids(), &[0x4948]);

        record_attested_win(
            &mut campaign,
            &profiles,
            1,
            AchievementId::CleanHands,
            AchievementEvaluation::Failed,
            run_id,
        );
        assert_eq!(lifetime.promote_campaign(&campaign, &profiles).unwrap(), 1);
        assert_eq!(
            lifetime.completed_campaigns(),
            std::slice::from_ref(&frozen)
        );
        assert!(
            lifetime
                .achievement_aggregation()
                .earned()
                .contains(AchievementId::CleanHands),
            "ordinary attempts after the first completed-envelope promotion do not expand it"
        );
    }

    #[test]
    fn invalid_completed_envelope_is_rejected_during_profile_schema_validation() {
        let mut history = ProfileCampaignHistory::default();
        history
            .completed_campaigns
            .push(LifetimeCampaignAchievementEnvelope {
                campaign_run_id: 7,
                completion_sequence: 2,
                required_mission_ids: vec![42, 42],
            });
        assert_eq!(
            history.validate_schema(),
            Err(PROFILE_HISTORY_SCHEMA_VERSION)
        );
    }

    #[test]
    fn duplicate_campaign_run_envelopes_are_rejected_during_schema_validation() {
        let mut history = ProfileCampaignHistory::default();
        for completion_sequence in [2, 3] {
            history
                .completed_campaigns
                .push(LifetimeCampaignAchievementEnvelope {
                    campaign_run_id: 7,
                    completion_sequence,
                    required_mission_ids: vec![42],
                });
        }
        assert_eq!(
            history.validate_schema(),
            Err(PROFILE_HISTORY_SCHEMA_VERSION)
        );
    }

    #[test]
    fn lifetime_all_required_uses_one_satisfied_run_and_never_cross_run_union() {
        let profiles = aggregation_profiles();
        let mut lifetime = ProfileCampaignHistory::default();

        let mut first = aggregation_campaign(&profiles);
        record_attested_win(
            &mut first,
            &profiles,
            1,
            AchievementId::CleanHands,
            AchievementEvaluation::Earned,
            11,
        );
        record_attested_win(
            &mut first,
            &profiles,
            2,
            AchievementId::CleanHands,
            AchievementEvaluation::Failed,
            11,
        );
        lifetime.promote_campaign(&first, &profiles).unwrap();

        let mut second = aggregation_campaign(&profiles);
        record_attested_win(
            &mut second,
            &profiles,
            1,
            AchievementId::CleanHands,
            AchievementEvaluation::Failed,
            22,
        );
        record_attested_win(
            &mut second,
            &profiles,
            2,
            AchievementId::CleanHands,
            AchievementEvaluation::Earned,
            22,
        );
        lifetime.promote_campaign(&second, &profiles).unwrap();
        assert_eq!(
            lifetime
                .achievement_aggregation()
                .get(AchievementId::CleanHands)
                .status,
            crate::achievement::AchievementAggregationStatus::MissingRequirements,
            "per-mission lifetime union must not mix evidence across campaign run IDs"
        );

        let mut third = aggregation_campaign(&profiles);
        record_attested_win(
            &mut third,
            &profiles,
            2,
            AchievementId::CleanHands,
            AchievementEvaluation::Earned,
            33,
        );
        lifetime.promote_campaign(&third, &profiles).unwrap();
        let progress = lifetime
            .achievement_aggregation()
            .get(AchievementId::CleanHands);
        assert_eq!(
            progress.status,
            crate::achievement::AchievementAggregationStatus::Earned
        );
        assert_eq!(
            (progress.earned_missions, progress.required_missions),
            (1, 1)
        );
    }

    #[test]
    fn native_campaign_promotion_is_idempotent_and_lifetime_scoped() {
        let mut profiles = ProfileManager::new();
        profiles.missions.push(crate::profiles::MissionProfile {
            id: 42,
            mission_name: "The Rescue".into(),
            ..Default::default()
        });
        let mut campaign = crate::campaign::Campaign::default();
        campaign.missions.push(crate::mission::Mission {
            profile_idx: Some(0),
            ..crate::mission::Mission::new()
        });
        campaign.current_mission_idx = Some(0);
        campaign.record_mission_attempt(
            0,
            MissionAttemptOutcome::Won,
            Some(100),
            Some(0xfeed),
            60,
            SimConfig::default(),
            &MissionStat::default(),
            None,
        );

        let mut lifetime = ProfileCampaignHistory::default();
        assert_eq!(lifetime.promote_campaign(&campaign, &profiles).unwrap(), 1);
        assert_eq!(lifetime.promote_campaign(&campaign, &profiles).unwrap(), 0);
        assert_eq!(lifetime.attempts().len(), 1);
        assert_eq!(lifetime.attempts()[0].mission_id(), 42);
        assert_eq!(lifetime.totals().wins, 1);
    }

    #[test]
    fn lifetime_promotion_refreshes_a_later_attestation_by_attempt_key() {
        let mut profiles = ProfileManager::new();
        profiles.missions.push(crate::profiles::MissionProfile {
            id: 42,
            mission_name: "The Rescue".into(),
            ..Default::default()
        });
        let mut tracker = crate::achievement::MissionAchievementState::from_mission_start();
        tracker
            .record_evaluation(
                crate::achievement::AchievementId::PileOBones,
                crate::achievement::AchievementEvaluation::Earned,
            )
            .unwrap();
        let results = *tracker.finalize_success();
        let mut campaign = crate::campaign::Campaign::default();
        campaign.missions.push(crate::mission::Mission {
            profile_idx: Some(0),
            ..crate::mission::Mission::new()
        });
        campaign.current_mission_idx = Some(0);
        campaign.record_mission_attempt(
            0,
            MissionAttemptOutcome::Won,
            Some(100),
            Some(0xfeed),
            60,
            SimConfig::default(),
            &MissionStat::default(),
            Some(results),
        );

        let mut lifetime = ProfileCampaignHistory::default();
        assert_eq!(lifetime.promote_campaign(&campaign, &profiles).unwrap(), 1);
        assert!(lifetime.eligible_badges().is_empty());

        campaign
            .attest_mission_achievement_attempt(
                campaign.latest_mission_attempt_key().unwrap(),
                AchievementUnlockPolicy::default(),
                AchievementRunContext::default(),
                &profiles,
            )
            .unwrap();
        assert_eq!(lifetime.promote_campaign(&campaign, &profiles).unwrap(), 1);
        assert!(
            lifetime
                .eligible_badges()
                .contains(crate::achievement::AchievementId::PileOBones)
        );
        assert_eq!(lifetime.promote_campaign(&campaign, &profiles).unwrap(), 0);
    }
}
