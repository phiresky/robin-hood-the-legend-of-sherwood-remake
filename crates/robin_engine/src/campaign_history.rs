//! Lossless campaign mission-attempt history.
//!
//! The shipped game keeps only mutable campaign totals and the last status of
//! each mission.  That is sufficient for progression, but it cannot answer
//! questions such as "what happened on my second attempt?".  This module is
//! the append-only record used by the campaign tree and Sherwood museum.
//!
//! Unknown legacy values are represented as `None`; migration never invents
//! zeroes for information an old save did not retain.

use serde::{Deserialize, Serialize};

use crate::achievement::{
    AchievementRunContext, AchievementSet, AchievementUnlockDecision, AchievementUnlockPolicy,
    MissionAchievementResults,
};
use crate::engine::SimConfig;
use crate::mission_stat::MissionStat;
use crate::player_profile::DifficultyLevel;
use crate::profiles::ProfileManager;

/// Schema of the per-mission history embedded in a native campaign.
pub const CAMPAIGN_HISTORY_SCHEMA_VERSION: u16 = 1;
pub const PROFILE_HISTORY_SCHEMA_VERSION: u16 = 1;

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
    /// Synthesized from the status/totals in a save which predates history.
    LegacyAggregateMigration = 1,
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
/// Fields are optional specifically so legacy migration can remain honest
/// about evidence that was discarded by the old aggregate-only format.
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
    #[serde(default)]
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

    pub(crate) fn legacy_aggregate(sequence: u64, outcome: MissionAttemptOutcome) -> Self {
        Self {
            sequence,
            outcome,
            source: MissionAttemptSource::LegacyAggregateMigration,
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
}

/// Totals derived from immutable attempt records.  No redundant aggregate is
/// serialized, so edited/corrupt data cannot leave two sources of truth.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignHistoryTotals {
    pub attempts: u64,
    pub wins: u64,
    pub losses: u64,
    pub interrupted: u64,
    pub known_duration_seconds: u64,
    pub known_score: u64,
    pub known_money: u64,
    pub incomplete_attempts: u64,
}

/// Honest snapshot of the profile totals that existed before lifetime
/// per-attempt storage. It is not assigned to any mission or counted as an
/// attempt because that information was never retained.
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
pub struct LegacyProfileAggregate {
    pub score: u32,
    pub ransom: u32,
    pub preserved_lives_percent: u32,
    pub play_time_seconds: u32,
    pub progression_percent: u32,
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
    legacy_profile_aggregate: Option<LegacyProfileAggregate>,
}

impl Default for ProfileCampaignHistory {
    fn default() -> Self {
        Self {
            schema_version: PROFILE_HISTORY_SCHEMA_VERSION,
            attempts: Vec::new(),
            legacy_profile_aggregate: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileHistoryPromotionError {
    UnsupportedSchema(u16),
    MissingCampaignRunId { native_attempts: usize },
    MissingMissionProfile { mission_index: usize },
    ConflictingAttempt(MissionAttemptKey),
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
            Self::MissingCampaignRunId { native_attempts } => write!(
                formatter,
                "campaign has {native_attempts} native attempt(s) but no run identity"
            ),
            Self::MissingMissionProfile { mission_index } => {
                write!(formatter, "campaign mission {mission_index} has no profile")
            }
            Self::ConflictingAttempt(key) => write!(
                formatter,
                "lifetime campaign attempt {} in run {} conflicts with its canonical campaign record",
                key.sequence, key.campaign_run_id
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

    pub const fn legacy_profile_aggregate(&self) -> Option<LegacyProfileAggregate> {
        self.legacy_profile_aggregate
    }

    pub fn totals(&self) -> CampaignHistoryTotals {
        let mut totals = CampaignHistoryTotals::default();
        for entry in &self.attempts {
            totals.include(entry.attempt());
        }
        totals
    }

    pub fn eligible_badges(&self) -> AchievementSet {
        self.attempts
            .iter()
            .fold(AchievementSet::empty(), |mut badges, entry| {
                if let Some(attestation) = entry.attempt.achievement_attestation {
                    badges.union_with(attestation.decision.eligible_earned);
                }
                badges
            })
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

    pub fn migrate_legacy_profile_aggregate(&mut self, aggregate: LegacyProfileAggregate) -> bool {
        if self.legacy_profile_aggregate.is_some() || !self.attempts.is_empty() {
            return false;
        }
        self.legacy_profile_aggregate = Some(aggregate);
        true
    }

    /// Idempotently promote every native campaign attempt into lifetime
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
        let native_attempts = campaign
            .missions
            .iter()
            .flat_map(|mission| mission.attempt_history().attempts())
            .filter(|attempt| attempt.source() == MissionAttemptSource::Native)
            .count();
        if native_attempts == 0 {
            return Ok(0);
        }
        let campaign_run_id = campaign
            .history_run_id()
            .ok_or(ProfileHistoryPromotionError::MissingCampaignRunId { native_attempts })?;
        let mut added = 0;
        for (mission_index, mission) in campaign.missions.iter().enumerate() {
            let native_attempts: Vec<&MissionAttempt> = mission
                .attempt_history()
                .attempts()
                .iter()
                .filter(|attempt| attempt.source() == MissionAttemptSource::Native)
                .collect();
            if native_attempts.is_empty() {
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
            for attempt in native_attempts {
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
        Ok(added)
    }
}

impl CampaignHistoryTotals {
    pub fn include(&mut self, attempt: &MissionAttempt) {
        self.attempts += 1;
        match attempt.outcome() {
            MissionAttemptOutcome::Won => self.wins += 1,
            MissionAttemptOutcome::Lost => self.losses += 1,
            MissionAttemptOutcome::Interrupted => self.interrupted += 1,
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

    #[test]
    fn legacy_attempts_preserve_unknowns_instead_of_faking_zeroes() {
        let attempt = MissionAttempt::legacy_aggregate(1, MissionAttemptOutcome::Won);
        assert_eq!(attempt.duration_seconds(), None);
        assert_eq!(attempt.rules(), None);
        assert_eq!(attempt.stats().added_score, None);
        assert!(!attempt.stats().is_complete());
    }

    #[test]
    fn derived_totals_report_incomplete_migrations() {
        let mut totals = CampaignHistoryTotals::default();
        totals.include(&MissionAttempt::legacy_aggregate(
            1,
            MissionAttemptOutcome::Lost,
        ));
        assert_eq!(totals.attempts, 1);
        assert_eq!(totals.losses, 1);
        assert_eq!(totals.incomplete_attempts, 1);
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
    fn legacy_profile_aggregate_is_not_misreported_as_an_attempt() {
        let mut history = ProfileCampaignHistory::default();
        assert!(
            history.migrate_legacy_profile_aggregate(LegacyProfileAggregate {
                score: 50,
                ransom: 100,
                preserved_lives_percent: 90,
                play_time_seconds: 300,
                progression_percent: 10,
            })
        );
        assert!(
            !history.migrate_legacy_profile_aggregate(LegacyProfileAggregate {
                score: 0,
                ransom: 0,
                preserved_lives_percent: 0,
                play_time_seconds: 0,
                progression_percent: 0,
            })
        );
        assert_eq!(history.totals().attempts, 0);
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
                crate::achievement::AchievementId::Ghost,
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
            )
            .unwrap();
        assert_eq!(lifetime.promote_campaign(&campaign, &profiles).unwrap(), 1);
        assert!(
            lifetime
                .eligible_badges()
                .contains(crate::achievement::AchievementId::Ghost)
        );
        assert_eq!(lifetime.promote_campaign(&campaign, &profiles).unwrap(), 0);
    }
}
