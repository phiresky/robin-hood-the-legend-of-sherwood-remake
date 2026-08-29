//! Deterministic achievement state and persistence policy.
//!
//! The original game has no achievement subsystem.  This module therefore
//! keeps two concerns deliberately separate:
//!
//! - [`MissionAchievementState`] is simulation-owned, serialized and hashed.
//!   Feature systems record their live evaluation here and the mission freezes
//!   a result at the successful terminal boundary.
//! - [`AchievementUnlockPolicy`] is host policy.  Replay, headless, custom and
//!   cheated runs may calculate a result without mutating campaign/profile
//!   unlock history.
//!
//! Compatibility note: adding these types to engine/campaign state changes the
//! native-save bitcode layout and replay state-hash contract. The integration
//! which lands the feature set must bump the then-current native-save and
//! replay schema versions together; this foundation intentionally does not
//! churn those shared versions in isolation.

use std::{array, fmt};

use serde::{Deserialize, Serialize};

/// Number of stable achievement identifiers understood by this build.
pub const ACHIEVEMENT_COUNT: usize = 4;

/// Stable achievement identifiers used by simulation, campaign and profile
/// persistence.
///
/// Discriminants are persistent data.  Never reorder or reuse them.
#[repr(u8)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(try_from = "u8", into = "u8")]
pub enum AchievementId {
    CleanHands = 0,
    Ghost = 1,
    PileOBones = 2,
    AllEnemiesOneBuilding = 3,
}

impl AchievementId {
    pub const ALL: [Self; ACHIEVEMENT_COUNT] = [
        Self::CleanHands,
        Self::Ghost,
        Self::PileOBones,
        Self::AllEnemiesOneBuilding,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    /// Numeric identifier persisted by compact sets and available to external
    /// metadata/UI code. Values are append-only and never reused.
    pub const fn stable_id(self) -> u8 {
        self as u8
    }

    /// Resolve a persisted numeric identifier without inventing a fallback for
    /// corrupt or newer data.
    pub const fn from_stable_id(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::CleanHands),
            1 => Some(Self::Ghost),
            2 => Some(Self::PileOBones),
            3 => Some(Self::AllEnemiesOneBuilding),
            _ => None,
        }
    }

    const fn bit(self) -> u64 {
        1_u64 << self as u8
    }
}

impl From<AchievementId> for u8 {
    fn from(value: AchievementId) -> Self {
        value.stable_id()
    }
}

impl TryFrom<u8> for AchievementId {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::from_stable_id(value).ok_or_else(|| format!("unknown achievement identifier {value}"))
    }
}

/// Compact deterministic set of [`AchievementId`] values.
#[derive(
    Debug,
    Default,
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
#[serde(transparent)]
pub struct AchievementSet(u64);

impl AchievementSet {
    const KNOWN_BITS: u64 = (1_u64 << ACHIEVEMENT_COUNT) - 1;

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn all() -> Self {
        Self(Self::KNOWN_BITS)
    }

    pub fn from_ids(ids: impl IntoIterator<Item = AchievementId>) -> Self {
        let mut result = Self::empty();
        for id in ids {
            result.insert(id);
        }
        result
    }

    pub const fn contains(self, id: AchievementId) -> bool {
        self.0 & id.bit() != 0
    }

    /// Returns true when this call added a previously absent identifier.
    pub fn insert(&mut self, id: AchievementId) -> bool {
        let before = self.0;
        self.0 |= id.bit();
        self.0 != before
    }

    /// Returns true when this call removed a previously present identifier.
    pub fn remove(&mut self, id: AchievementId) -> bool {
        let before = self.0;
        self.0 &= !id.bit();
        self.0 != before
    }

    pub fn union_with(&mut self, other: Self) {
        self.0 |= other.0;
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0 & Self::KNOWN_BITS)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn len(self) -> usize {
        (self.0 & Self::KNOWN_BITS).count_ones() as usize
    }

    pub fn iter(self) -> impl Iterator<Item = AchievementId> {
        AchievementId::ALL
            .into_iter()
            .filter(move |&id| self.contains(id))
    }
}

impl FromIterator<AchievementId> for AchievementSet {
    fn from_iter<T: IntoIterator<Item = AchievementId>>(iter: T) -> Self {
        Self::from_ids(iter)
    }
}

/// Terminal evaluation for one achievement in one successful mission run.
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
#[serde(try_from = "u8", into = "u8")]
pub enum AchievementEvaluation {
    /// The required historical evidence is unavailable.  This is never
    /// treated as a failure or silently promoted to success.
    Unverifiable = 0,
    Failed = 1,
    Earned = 2,
}

impl AchievementEvaluation {
    pub(crate) const fn history_rank(self) -> u8 {
        match self {
            Self::Unverifiable => 0,
            Self::Failed => 1,
            Self::Earned => 2,
        }
    }
}

impl From<AchievementEvaluation> for u8 {
    fn from(value: AchievementEvaluation) -> Self {
        value as u8
    }
}

impl TryFrom<u8> for AchievementEvaluation {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Unverifiable),
            1 => Ok(Self::Failed),
            2 => Ok(Self::Earned),
            _ => Err(format!("unknown achievement evaluation {value}")),
        }
    }
}

/// Fixed, stable map from achievement identifier to terminal evaluation.
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
pub struct AchievementEvaluations([AchievementEvaluation; ACHIEVEMENT_COUNT]);

impl AchievementEvaluations {
    pub const fn all_unverifiable() -> Self {
        Self([AchievementEvaluation::Unverifiable; ACHIEVEMENT_COUNT])
    }

    pub fn get(self, id: AchievementId) -> AchievementEvaluation {
        self.0[id.index()]
    }

    pub fn iter(self) -> impl Iterator<Item = (AchievementId, AchievementEvaluation)> {
        AchievementId::ALL
            .into_iter()
            .map(move |id| (id, self.get(id)))
    }

    pub fn earned(self) -> AchievementSet {
        self.iter()
            .filter_map(|(id, result)| (result == AchievementEvaluation::Earned).then_some(id))
            .collect()
    }
}

impl Default for AchievementEvaluations {
    fn default() -> Self {
        Self::all_unverifiable()
    }
}

/// Whether tracking started with complete mission-start history.
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
#[serde(try_from = "u8", into = "u8")]
pub enum AchievementTrackingProvenance {
    MissionStart = 0,
    /// An Original-format mid-mission import cannot reconstruct prior kills,
    /// sightings or body arrangements.
    LegacyImportIncomplete = 1,
}

impl From<AchievementTrackingProvenance> for u8 {
    fn from(value: AchievementTrackingProvenance) -> Self {
        value as u8
    }
}

impl TryFrom<u8> for AchievementTrackingProvenance {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::MissionStart),
            1 => Ok(Self::LegacyImportIncomplete),
            _ => Err(format!("unknown achievement tracking provenance {value}")),
        }
    }
}

/// Frozen evaluations for one successfully completed mission attempt.
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
pub struct MissionAchievementResults {
    provenance: AchievementTrackingProvenance,
    evaluations: AchievementEvaluations,
}

impl MissionAchievementResults {
    pub const fn provenance(self) -> AchievementTrackingProvenance {
        self.provenance
    }

    pub const fn evaluations(self) -> AchievementEvaluations {
        self.evaluations
    }

    pub fn evaluation(self, id: AchievementId) -> AchievementEvaluation {
        self.evaluations.get(id)
    }

    pub fn earned(self) -> AchievementSet {
        self.evaluations.earned()
    }
}

/// Error returned when a feature hook violates mission tracker invariants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AchievementStateError {
    ResultsAlreadyFinalized,
    IncompleteEvidence(AchievementId),
}

impl fmt::Display for AchievementStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ResultsAlreadyFinalized => {
                formatter.write_str("mission achievement results are already finalized")
            }
            Self::IncompleteEvidence(id) => write!(
                formatter,
                "achievement {id:?} cannot be earned or failed without complete evidence"
            ),
        }
    }
}

impl std::error::Error for AchievementStateError {}

/// Simulation-owned achievement state for the active mission.
///
/// Feature-specific trackers may add deterministic fields to this aggregate;
/// they should publish only their terminal evaluation through
/// [`Self::record_evaluation`].
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
pub struct MissionAchievementState {
    tracking_provenance: AchievementTrackingProvenance,
    verifiable: AchievementSet,
    live_evaluations: [Option<AchievementEvaluation>; ACHIEVEMENT_COUNT],
    finalized: Option<MissionAchievementResults>,
}

impl Default for MissionAchievementState {
    fn default() -> Self {
        Self::from_mission_start()
    }
}

impl MissionAchievementState {
    pub fn from_mission_start() -> Self {
        Self {
            tracking_provenance: AchievementTrackingProvenance::MissionStart,
            verifiable: AchievementSet::all(),
            live_evaluations: [None; ACHIEVEMENT_COUNT],
            finalized: None,
        }
    }

    pub fn from_incomplete_legacy_import() -> Self {
        Self {
            tracking_provenance: AchievementTrackingProvenance::LegacyImportIncomplete,
            verifiable: AchievementSet::empty(),
            live_evaluations: [None; ACHIEVEMENT_COUNT],
            finalized: None,
        }
    }

    pub const fn tracking_provenance(&self) -> AchievementTrackingProvenance {
        self.tracking_provenance
    }

    pub const fn verifiable_achievements(&self) -> AchievementSet {
        self.verifiable
    }

    pub const fn finalized_results(&self) -> Option<&MissionAchievementResults> {
        self.finalized.as_ref()
    }

    pub fn live_evaluation(&self, id: AchievementId) -> Option<AchievementEvaluation> {
        self.live_evaluations[id.index()]
    }

    /// Mark one tracker as reconstructible after an incomplete import.
    pub fn mark_verifiable(&mut self, id: AchievementId) -> Result<(), AchievementStateError> {
        self.ensure_not_finalized()?;
        self.verifiable.insert(id);
        if self.live_evaluations[id.index()] == Some(AchievementEvaluation::Unverifiable) {
            self.live_evaluations[id.index()] = None;
        }
        Ok(())
    }

    /// Explicitly invalidate historical evidence for one tracker.
    pub fn mark_unverifiable(&mut self, id: AchievementId) -> Result<(), AchievementStateError> {
        self.ensure_not_finalized()?;
        self.verifiable.remove(id);
        self.live_evaluations[id.index()] = Some(AchievementEvaluation::Unverifiable);
        Ok(())
    }

    /// Publish the current evaluation produced by a feature tracker.
    pub fn record_evaluation(
        &mut self,
        id: AchievementId,
        evaluation: AchievementEvaluation,
    ) -> Result<(), AchievementStateError> {
        self.ensure_not_finalized()?;
        if evaluation != AchievementEvaluation::Unverifiable && !self.verifiable.contains(id) {
            return Err(AchievementStateError::IncompleteEvidence(id));
        }
        if evaluation == AchievementEvaluation::Unverifiable {
            self.verifiable.remove(id);
        }
        self.live_evaluations[id.index()] = Some(evaluation);
        Ok(())
    }

    /// Freeze a terminal result after a successful mission.
    ///
    /// Calling this again is idempotent. A verifiable tracker which did not
    /// publish an evaluation is conservatively `Unverifiable`, never a fake
    /// failure or success.
    pub fn finalize_success(&mut self) -> &MissionAchievementResults {
        if self.finalized.is_none() {
            let evaluations = AchievementEvaluations(array::from_fn(|index| {
                let id = AchievementId::ALL[index];
                if self.verifiable.contains(id) {
                    self.live_evaluations[index].unwrap_or(AchievementEvaluation::Unverifiable)
                } else {
                    AchievementEvaluation::Unverifiable
                }
            }));
            self.finalized = Some(MissionAchievementResults {
                provenance: self.tracking_provenance,
                evaluations,
            });
        }
        self.finalized
            .as_ref()
            .expect("achievement result was assigned immediately above")
    }

    fn ensure_not_finalized(&self) -> Result<(), AchievementStateError> {
        if self.finalized.is_some() {
            Err(AchievementStateError::ResultsAlreadyFinalized)
        } else {
            Ok(())
        }
    }
}

/// Broad mission source used by host-side unlock policy.
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
#[serde(try_from = "u8", into = "u8")]
pub enum AchievementRunKind {
    Campaign = 0,
    CustomMission = 1,
}

impl From<AchievementRunKind> for u8 {
    fn from(value: AchievementRunKind) -> Self {
        value as u8
    }
}

impl TryFrom<u8> for AchievementRunKind {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Campaign),
            1 => Ok(Self::CustomMission),
            _ => Err(format!("unknown achievement run kind {value}")),
        }
    }
}

/// Host facts which may suppress persistence without changing calculated
/// simulation results.
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
pub struct AchievementRunContext {
    pub kind: AchievementRunKind,
    pub multiplayer: bool,
    pub replay_playback: bool,
    pub headless: bool,
    pub cheat_used: bool,
}

impl Default for AchievementRunContext {
    fn default() -> Self {
        Self {
            kind: AchievementRunKind::Campaign,
            multiplayer: false,
            replay_playback: false,
            headless: false,
            cheat_used: false,
        }
    }
}

/// Configurable host policy.
///
/// The switches can disable persistence globally or for multiplayer campaign
/// sessions. They deliberately cannot opt custom missions, replay playback,
/// headless tools, or cheated runs into persistence: those run kinds may show
/// calculated progress, but are never achievement-authoritative.
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
pub struct AchievementUnlockPolicy {
    pub enabled: bool,
    pub allow_multiplayer: bool,
}

impl Default for AchievementUnlockPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_multiplayer: true,
        }
    }
}

/// Reasons why a calculated result cannot mutate unlock history.
#[derive(
    Debug,
    Default,
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
#[serde(transparent)]
pub struct AchievementUnlockBlockers(u16);

impl AchievementUnlockBlockers {
    pub const CAMPAIGN_DISABLED: u16 = 1 << 0;
    pub const MULTIPLAYER_DISABLED: u16 = 1 << 1;
    pub const CUSTOM_MISSION: u16 = 1 << 2;
    pub const REPLAY_PLAYBACK: u16 = 1 << 3;
    pub const HEADLESS: u16 = 1 << 4;
    pub const CHEAT_USED: u16 = 1 << 5;

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, blocker: u16) -> bool {
        self.0 & blocker != 0
    }

    fn insert(&mut self, blocker: u16) {
        self.0 |= blocker;
    }
}

/// Pure result of applying host unlock policy to a calculated mission result.
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
pub struct AchievementUnlockDecision {
    pub blockers: AchievementUnlockBlockers,
    pub eligible_earned: AchievementSet,
}

impl AchievementUnlockDecision {
    pub const fn may_persist(self) -> bool {
        self.blockers.is_empty()
    }
}

impl AchievementUnlockPolicy {
    pub fn evaluate(
        self,
        context: AchievementRunContext,
        results: MissionAchievementResults,
    ) -> AchievementUnlockDecision {
        let mut blockers = AchievementUnlockBlockers::default();
        if !self.enabled {
            blockers.insert(AchievementUnlockBlockers::CAMPAIGN_DISABLED);
        }
        match context.kind {
            AchievementRunKind::Campaign => {}
            AchievementRunKind::CustomMission => {
                blockers.insert(AchievementUnlockBlockers::CUSTOM_MISSION);
            }
        }
        if context.multiplayer && !self.allow_multiplayer {
            blockers.insert(AchievementUnlockBlockers::MULTIPLAYER_DISABLED);
        }
        if context.replay_playback {
            blockers.insert(AchievementUnlockBlockers::REPLAY_PLAYBACK);
        }
        if context.headless {
            blockers.insert(AchievementUnlockBlockers::HEADLESS);
        }
        if context.cheat_used {
            blockers.insert(AchievementUnlockBlockers::CHEAT_USED);
        }

        AchievementUnlockDecision {
            blockers,
            eligible_earned: if blockers.is_empty() {
                results.earned()
            } else {
                AchievementSet::empty()
            },
        }
    }
}

/// Error from recording a successful result into campaign mission history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AchievementHistoryError {
    pub mission_index: usize,
    pub mission_count: usize,
}

impl fmt::Display for AchievementHistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "achievement history mission index {} is out of bounds for {} missions",
            self.mission_index, self.mission_count
        )
    }
}

impl std::error::Error for AchievementHistoryError {}

/// Outcome of attempting to persist one successful calculated result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AchievementHistoryUpdate {
    /// Non-empty means the run was computed but intentionally not persisted.
    pub blockers: AchievementUnlockBlockers,
    /// Profile/campaign-global identifiers first earned by this update.
    pub newly_earned: AchievementSet,
    /// Full badge set now displayed for this mission.
    pub mission_badges: AchievementSet,
}

impl AchievementHistoryUpdate {
    pub const fn persisted(self) -> bool {
        self.blockers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn earned_clean_hands() -> MissionAchievementResults {
        let mut state = MissionAchievementState::from_mission_start();
        state
            .record_evaluation(AchievementId::CleanHands, AchievementEvaluation::Earned)
            .unwrap();
        *state.finalize_success()
    }

    #[test]
    fn stable_ids_and_set_iteration_are_canonical() {
        assert_eq!(AchievementId::CleanHands as u8, 0);
        assert_eq!(AchievementId::Ghost as u8, 1);
        assert_eq!(AchievementId::PileOBones as u8, 2);
        assert_eq!(AchievementId::AllEnemiesOneBuilding as u8, 3);
        for id in AchievementId::ALL {
            assert_eq!(AchievementId::from_stable_id(id.stable_id()), Some(id));
        }
        assert_eq!(AchievementId::from_stable_id(4), None);
        assert_eq!(serde_json::to_string(&AchievementId::Ghost).unwrap(), "1");
        assert_eq!(
            serde_json::from_str::<AchievementId>("3").unwrap(),
            AchievementId::AllEnemiesOneBuilding
        );
        assert!(serde_json::from_str::<AchievementId>("4").is_err());

        let set = AchievementSet::from_ids([
            AchievementId::AllEnemiesOneBuilding,
            AchievementId::CleanHands,
        ]);
        assert_eq!(
            set.iter().collect::<Vec<_>>(),
            vec![
                AchievementId::CleanHands,
                AchievementId::AllEnemiesOneBuilding
            ]
        );
    }

    #[test]
    fn deterministic_state_roundtrips_through_supported_codecs() {
        let mut state = MissionAchievementState::from_mission_start();
        state
            .record_evaluation(AchievementId::CleanHands, AchievementEvaluation::Earned)
            .unwrap();
        state
            .record_evaluation(AchievementId::Ghost, AchievementEvaluation::Failed)
            .unwrap();
        state.mark_unverifiable(AchievementId::PileOBones).unwrap();

        let json = serde_json::to_string(&state).unwrap();
        let from_json: MissionAchievementState = serde_json::from_str(&json).unwrap();
        assert_eq!(from_json, state);

        let native = bitcode::encode(&state);
        let from_native: MissionAchievementState = bitcode::decode(&native).unwrap();
        assert_eq!(from_native, state);
    }

    #[test]
    fn provenance_and_evaluations_participate_in_state_hash() {
        let mission_start = MissionAchievementState::from_mission_start();
        let legacy_import = MissionAchievementState::from_incomplete_legacy_import();
        assert_ne!(
            robin_util::state_hash::compute(&mission_start),
            robin_util::state_hash::compute(&legacy_import)
        );

        let mut earned = MissionAchievementState::from_mission_start();
        earned
            .record_evaluation(AchievementId::Ghost, AchievementEvaluation::Earned)
            .unwrap();
        assert_ne!(
            robin_util::state_hash::compute(&mission_start),
            robin_util::state_hash::compute(&earned)
        );
    }

    #[test]
    fn incomplete_import_never_fabricates_a_result() {
        let mut state = MissionAchievementState::from_incomplete_legacy_import();
        assert_eq!(
            state.record_evaluation(AchievementId::CleanHands, AchievementEvaluation::Earned),
            Err(AchievementStateError::IncompleteEvidence(
                AchievementId::CleanHands
            ))
        );
        let results = *state.finalize_success();
        for id in AchievementId::ALL {
            assert_eq!(
                results.evaluation(id),
                AchievementEvaluation::Unverifiable,
                "{id:?} must not infer pre-import mission history"
            );
        }
        assert!(results.earned().is_empty());
    }

    #[test]
    fn finalized_results_are_frozen() {
        let mut state = MissionAchievementState::from_mission_start();
        state
            .record_evaluation(AchievementId::Ghost, AchievementEvaluation::Failed)
            .unwrap();
        let first = *state.finalize_success();
        let second = *state.finalize_success();
        assert_eq!(first, second);
        assert_eq!(
            state.record_evaluation(AchievementId::Ghost, AchievementEvaluation::Earned),
            Err(AchievementStateError::ResultsAlreadyFinalized)
        );
    }

    #[test]
    fn accepted_policy_computes_but_blocks_non_gameplay_unlocks() {
        let policy = AchievementUnlockPolicy::default();
        let results = earned_clean_hands();
        let normal = policy.evaluate(AchievementRunContext::default(), results);
        assert!(normal.may_persist());
        assert!(normal.eligible_earned.contains(AchievementId::CleanHands));

        for context in [
            AchievementRunContext {
                kind: AchievementRunKind::CustomMission,
                ..Default::default()
            },
            AchievementRunContext {
                replay_playback: true,
                ..Default::default()
            },
            AchievementRunContext {
                headless: true,
                ..Default::default()
            },
            AchievementRunContext {
                cheat_used: true,
                ..Default::default()
            },
        ] {
            let decision = policy.evaluate(context, results);
            assert!(!decision.may_persist());
            assert!(decision.eligible_earned.is_empty());
        }

        let multiplayer = policy.evaluate(
            AchievementRunContext {
                multiplayer: true,
                ..Default::default()
            },
            results,
        );
        assert!(multiplayer.may_persist());
    }

    #[test]
    fn policy_reports_every_blocker_and_cannot_authorize_tool_runs() {
        let decision = AchievementUnlockPolicy {
            enabled: false,
            allow_multiplayer: false,
        }
        .evaluate(
            AchievementRunContext {
                kind: AchievementRunKind::CustomMission,
                multiplayer: true,
                replay_playback: true,
                headless: true,
                cheat_used: true,
            },
            earned_clean_hands(),
        );

        for blocker in [
            AchievementUnlockBlockers::CAMPAIGN_DISABLED,
            AchievementUnlockBlockers::MULTIPLAYER_DISABLED,
            AchievementUnlockBlockers::CUSTOM_MISSION,
            AchievementUnlockBlockers::REPLAY_PLAYBACK,
            AchievementUnlockBlockers::HEADLESS,
            AchievementUnlockBlockers::CHEAT_USED,
        ] {
            assert!(decision.blockers.contains(blocker));
        }
        assert!(decision.eligible_earned.is_empty());
    }
}
