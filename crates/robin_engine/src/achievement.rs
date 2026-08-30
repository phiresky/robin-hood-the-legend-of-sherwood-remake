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
//! Adding these types to deterministic engine and campaign state changes the
//! native save/replay state contract. Obsolete native Rust history layouts are
//! rejected by the mandatory typed-history schema; only Original C++ saves use
//! the explicit incomplete-evidence import path.

use std::{array, collections::BTreeSet, fmt};

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

    /// Canonical public identifier used by immutable ranked rulesets and
    /// verifier results. These strings are persistent protocol identities;
    /// never rename or reuse them.
    pub const fn protocol_id(self) -> &'static str {
        match self {
            Self::CleanHands => "clean-hands",
            Self::Ghost => "ghost",
            Self::PileOBones => "pile-o-bones",
            Self::AllEnemiesOneBuilding => "all-enemies-stashed",
        }
    }

    /// Campaign/lifetime aggregation semantics for this stable achievement.
    ///
    /// Keeping this policy beside the persistent identifier prevents campaign,
    /// profile, and UI code from growing separate name-specific conditionals.
    pub const fn aggregation_policy(self) -> AchievementAggregationPolicy {
        match self {
            Self::CleanHands | Self::Ghost => AchievementAggregationPolicy::AllRequiredMissions,
            Self::PileOBones | Self::AllEnemiesOneBuilding => {
                AchievementAggregationPolicy::AnyMissionOnce
            }
        }
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

/// Typed rule for lifting per-mission evidence into a campaign/lifetime badge.
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
pub enum AchievementAggregationPolicy {
    /// Earn only when a completed campaign envelope has the badge on every
    /// canonical mission required by that particular campaign path.
    AllRequiredMissions = 0,
    /// One eligible mission permanently satisfies the campaign/lifetime rule.
    AnyMissionOnce = 1,
}

impl From<AchievementAggregationPolicy> for u8 {
    fn from(value: AchievementAggregationPolicy) -> Self {
        value as u8
    }
}

impl TryFrom<u8> for AchievementAggregationPolicy {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::AllRequiredMissions),
            1 => Ok(Self::AnyMissionOnce),
            _ => Err(format!("unknown achievement aggregation policy {value}")),
        }
    }
}

/// Honest state of one campaign- or lifetime-level achievement envelope.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub enum AchievementAggregationStatus {
    /// The campaign/lifetime archive can still acquire the required evidence.
    InProgress = 0,
    /// Legacy or incomplete records prevent a truthful yes/no conclusion.
    Unverifiable = 1,
    /// A completed, fully evidenced envelope did not satisfy the rule.
    MissingRequirements = 2,
    /// The typed aggregation rule is satisfied.
    Earned = 3,
}

impl From<AchievementAggregationStatus> for u8 {
    fn from(value: AchievementAggregationStatus) -> Self {
        value as u8
    }
}

impl TryFrom<u8> for AchievementAggregationStatus {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::InProgress),
            1 => Ok(Self::Unverifiable),
            2 => Ok(Self::MissingRequirements),
            3 => Ok(Self::Earned),
            _ => Err(format!("unknown achievement aggregation status {value}")),
        }
    }
}

/// Derived progress for one stable achievement at campaign or lifetime scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AchievementAggregationProgress {
    pub id: AchievementId,
    pub policy: AchievementAggregationPolicy,
    pub status: AchievementAggregationStatus,
    /// Number of relevant missions which already carry eligible evidence.
    pub earned_missions: u32,
    /// Required mission count for `AllRequiredMissions`; one for
    /// `AnyMissionOnce` once any canonical mission evidence exists.
    pub required_missions: u32,
    /// Required/relevant missions whose historical evidence was lost.
    pub unverifiable_missions: u32,
}

impl AchievementAggregationProgress {
    pub const fn earned(self) -> bool {
        matches!(self.status, AchievementAggregationStatus::Earned)
    }
}

/// Fixed stable map of campaign/lifetime aggregation results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AchievementAggregationSummary([AchievementAggregationProgress; ACHIEVEMENT_COUNT]);

impl Default for AchievementAggregationSummary {
    fn default() -> Self {
        Self::from_inputs(|_| AchievementAggregationInput::default())
    }
}

impl AchievementAggregationSummary {
    pub fn from_inputs(
        mut input: impl FnMut(AchievementId) -> AchievementAggregationInput,
    ) -> Self {
        Self(array::from_fn(|index| {
            let id = AchievementId::ALL[index];
            aggregate_achievement(id, input(id))
        }))
    }

    pub const fn get(self, id: AchievementId) -> AchievementAggregationProgress {
        self.0[id.index()]
    }

    pub fn iter(self) -> impl Iterator<Item = AchievementAggregationProgress> {
        self.0.into_iter()
    }

    pub fn earned(self) -> AchievementSet {
        self.iter()
            .filter(|progress| progress.earned())
            .map(|progress| progress.id)
            .collect()
    }
}

/// Scope-neutral evidence counts consumed by the one aggregation evaluator.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AchievementAggregationInput {
    /// A canonical campaign envelope exists and can be judged conclusively.
    pub envelope_complete: bool,
    /// A legacy completion is known but its exact campaign path was lost.
    pub envelope_unverifiable: bool,
    pub earned_missions: u32,
    pub required_missions: u32,
    pub unverifiable_missions: u32,
}

/// Central typed evaluator shared by current-campaign and lifetime archives.
pub fn aggregate_achievement(
    id: AchievementId,
    mut input: AchievementAggregationInput,
) -> AchievementAggregationProgress {
    let policy = id.aggregation_policy();
    if policy == AchievementAggregationPolicy::AnyMissionOnce {
        input.earned_missions = u32::from(input.earned_missions != 0);
        input.required_missions = u32::from(input.required_missions != 0);
        input.unverifiable_missions =
            u32::from(input.earned_missions == 0 && input.unverifiable_missions != 0);
    }
    let status = match policy {
        AchievementAggregationPolicy::AllRequiredMissions => {
            if input.envelope_complete {
                assert!(
                    input.earned_missions <= input.required_missions,
                    "all-required achievement has more earned missions than required missions"
                );
                assert!(
                    input.unverifiable_missions <= input.required_missions,
                    "all-required achievement has more unverifiable missions than required missions"
                );
                assert!(
                    input
                        .earned_missions
                        .checked_add(input.unverifiable_missions)
                        .is_some_and(|known| known <= input.required_missions),
                    "all-required achievement mission evidence overlaps or overflows"
                );
            }
            if input.envelope_complete
                && input.required_missions != 0
                && input.earned_missions == input.required_missions
                && input.unverifiable_missions == 0
            {
                AchievementAggregationStatus::Earned
            } else if input.envelope_complete
                && (input.unverifiable_missions != 0 || input.envelope_unverifiable)
            {
                AchievementAggregationStatus::Unverifiable
            } else if input.envelope_complete {
                AchievementAggregationStatus::MissingRequirements
            } else if input.unverifiable_missions != 0 || input.envelope_unverifiable {
                AchievementAggregationStatus::Unverifiable
            } else {
                AchievementAggregationStatus::InProgress
            }
        }
        AchievementAggregationPolicy::AnyMissionOnce => {
            if input.earned_missions != 0 {
                AchievementAggregationStatus::Earned
            } else if input.unverifiable_missions != 0 {
                AchievementAggregationStatus::Unverifiable
            } else if input.envelope_complete {
                AchievementAggregationStatus::MissingRequirements
            } else {
                AchievementAggregationStatus::InProgress
            }
        }
    };
    AchievementAggregationProgress {
        id,
        policy,
        status,
        earned_missions: input.earned_missions,
        required_missions: input.required_missions,
        unverifiable_missions: input.unverifiable_missions,
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
    metrics: AchievementAttemptMetrics,
}

impl MissionAchievementResults {
    pub const fn provenance(self) -> AchievementTrackingProvenance {
        self.provenance
    }

    pub const fn evaluations(self) -> AchievementEvaluations {
        self.evaluations
    }

    /// Exact counters frozen with this attempt. Campaign history retains one
    /// of these records per successful replay rather than folding attempts
    /// into lossy mission totals.
    pub const fn metrics(self) -> AchievementAttemptMetrics {
        self.metrics
    }

    pub fn evaluation(self, id: AchievementId) -> AchievementEvaluation {
        self.evaluations.get(id)
    }

    pub fn earned(self) -> AchievementSet {
        self.evaluations.earned()
    }
}

/// Achievement-relevant facts retained for one completed attempt.
///
/// Counts are derived from exact entity/event sets while the mission is live.
/// The compact frozen form is intentionally sufficient to explain every
/// evaluation in mission history without persisting renderer-only state.
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
pub struct AchievementAttemptMetrics {
    pub duration_frames: u32,
    pub baseline_living_npcs: u32,
    pub baseline_dead_npcs: u32,
    pub encountered_hostiles: u32,
    pub player_caused_deaths: u32,
    pub npc_caused_deaths: u32,
    pub unique_hostile_observers: u32,
    pub unique_observed_player_characters: u32,
    pub max_bodies_in_one_building: u32,
    pub enemies_in_stash_building: u32,
    pub enemies_required_for_stash: u32,
}

/// Read-only live data used by optional HUD trackers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AchievementProgressSnapshot {
    pub evaluations: AchievementEvaluations,
    pub metrics: AchievementAttemptMetrics,
}

/// Exact responsibility carried by a fresh death event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AchievementDeathCause {
    PlayerControlled,
    Npc,
    EnvironmentOrScript,
}

/// Stable identity for an exact live building sector.
///
/// `SectorHandle` equality intentionally compares only its public number for
/// Original compatibility. Achievements must distinguish coincident arena
/// sectors, so this key retains both parts explicitly.
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
pub struct AchievementBuildingId {
    pub public_number: u16,
    pub arena_index: Option<crate::fast_find_grid::SectorIndex>,
}

/// One NPC human's current contribution to body/stash trackers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AchievementEntitySnapshot {
    pub entity: crate::element::EntityId,
    /// Whether this NPC contributes to the "all enemies" requirement.
    pub hostile: bool,
    pub out_of_order: bool,
    pub building: Option<AchievementBuildingId>,
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
    baseline_frame: u32,
    baseline_living_npcs: BTreeSet<crate::element::EntityId>,
    baseline_dead_npcs: BTreeSet<crate::element::EntityId>,
    encountered_hostiles: BTreeSet<crate::element::EntityId>,
    processed_deaths: BTreeSet<crate::element::EntityId>,
    player_caused_deaths: BTreeSet<crate::element::EntityId>,
    npc_caused_deaths: BTreeSet<crate::element::EntityId>,
    hostile_observers: BTreeSet<crate::element::EntityId>,
    observed_player_characters: BTreeSet<crate::element::EntityId>,
    observation_pairs: BTreeSet<(crate::element::EntityId, crate::element::EntityId)>,
    metrics: AchievementAttemptMetrics,
    pile_o_bones_earned: bool,
    history_promotion_attempted: bool,
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
            baseline_frame: 0,
            baseline_living_npcs: BTreeSet::new(),
            baseline_dead_npcs: BTreeSet::new(),
            encountered_hostiles: BTreeSet::new(),
            processed_deaths: BTreeSet::new(),
            player_caused_deaths: BTreeSet::new(),
            npc_caused_deaths: BTreeSet::new(),
            hostile_observers: BTreeSet::new(),
            observed_player_characters: BTreeSet::new(),
            observation_pairs: BTreeSet::new(),
            metrics: AchievementAttemptMetrics::default(),
            pile_o_bones_earned: false,
            history_promotion_attempted: false,
            finalized: None,
        }
    }

    pub fn from_incomplete_legacy_import() -> Self {
        Self {
            tracking_provenance: AchievementTrackingProvenance::LegacyImportIncomplete,
            verifiable: AchievementSet::empty(),
            live_evaluations: [None; ACHIEVEMENT_COUNT],
            baseline_frame: 0,
            baseline_living_npcs: BTreeSet::new(),
            baseline_dead_npcs: BTreeSet::new(),
            encountered_hostiles: BTreeSet::new(),
            processed_deaths: BTreeSet::new(),
            player_caused_deaths: BTreeSet::new(),
            npc_caused_deaths: BTreeSet::new(),
            hostile_observers: BTreeSet::new(),
            observed_player_characters: BTreeSet::new(),
            observation_pairs: BTreeSet::new(),
            metrics: AchievementAttemptMetrics::default(),
            pile_o_bones_earned: false,
            history_promotion_attempted: false,
            finalized: None,
        }
    }

    /// Install the authoritative post-startup baseline. Startup scripts may
    /// create or kill actors, so callers must invoke this only after mission
    /// initialization has completely settled.
    pub fn initialize_mission_baseline(
        &mut self,
        frame: u32,
        hostiles: impl IntoIterator<Item = (crate::element::EntityId, bool)>,
    ) {
        *self = Self::from_mission_start();
        self.baseline_frame = frame;
        for (entity, dead_at_start) in hostiles {
            if dead_at_start {
                self.baseline_dead_npcs.insert(entity);
                self.processed_deaths.insert(entity);
            } else {
                self.baseline_living_npcs.insert(entity);
            }
        }
        self.publish_basic_evaluations(false);
        self.refresh_metrics(frame);
    }

    /// Record one fresh hostile death using the damage element's exact origin.
    pub fn record_npc_death(
        &mut self,
        victim: crate::element::EntityId,
        cause: AchievementDeathCause,
        npc_deaths_invalidate_clean_hands: bool,
    ) -> Result<(), AchievementStateError> {
        self.ensure_not_finalized()?;
        if !self.processed_deaths.insert(victim) {
            return Ok(());
        }
        match cause {
            AchievementDeathCause::PlayerControlled => {
                self.player_caused_deaths.insert(victim);
            }
            AchievementDeathCause::Npc => {
                self.npc_caused_deaths.insert(victim);
            }
            AchievementDeathCause::EnvironmentOrScript => {}
        }
        self.publish_basic_evaluations(npc_deaths_invalidate_clean_hands);
        Ok(())
    }

    /// Apply a changed NPC-on-NPC Clean Hands rule to already recorded exact
    /// deaths as well as future ones.
    pub fn refresh_clean_hands_rule(
        &mut self,
        npc_deaths_invalidate_clean_hands: bool,
    ) -> Result<(), AchievementStateError> {
        self.ensure_not_finalized()?;
        self.publish_basic_evaluations(npc_deaths_invalidate_clean_hands);
        Ok(())
    }

    /// Latch an exact optical observation by a living hostile NPC.
    pub fn record_hostile_observation(
        &mut self,
        observer: crate::element::EntityId,
        pc: crate::element::EntityId,
    ) -> Result<(), AchievementStateError> {
        self.ensure_not_finalized()?;
        if self.observation_pairs.insert((observer, pc)) {
            self.hostile_observers.insert(observer);
            self.observed_player_characters.insert(pc);
            self.metrics.unique_hostile_observers = u32::try_from(self.hostile_observers.len())
                .expect("hostile observer count exceeds u32");
            self.metrics.unique_observed_player_characters =
                u32::try_from(self.observed_player_characters.len())
                    .expect("observed player character count exceeds u32");
        }
        self.live_evaluations[AchievementId::Ghost.index()] = Some(AchievementEvaluation::Failed);
        Ok(())
    }

    /// Recompute exact-building body and whole-enemy stash progress.
    pub fn refresh_hostile_arrangement(
        &mut self,
        frame: u32,
        npcs: impl IntoIterator<Item = AchievementEntitySnapshot>,
    ) -> Result<(), AchievementStateError> {
        self.ensure_not_finalized()?;
        let mut body_counts = std::collections::BTreeMap::new();
        let mut hostile_body_counts = std::collections::BTreeMap::new();
        for npc in npcs {
            if npc.hostile {
                self.encountered_hostiles.insert(npc.entity);
            }
            if npc.out_of_order
                && let Some(building) = npc.building
            {
                *body_counts.entry(building).or_insert(0_u32) += 1;
                if npc.hostile {
                    *hostile_body_counts.entry(building).or_insert(0_u32) += 1;
                }
            }
        }

        let max_bodies = body_counts.values().copied().max().unwrap_or(0);
        self.metrics.max_bodies_in_one_building =
            self.metrics.max_bodies_in_one_building.max(max_bodies);
        if max_bodies >= 10 {
            self.pile_o_bones_earned = true;
        }
        self.live_evaluations[AchievementId::PileOBones.index()] =
            Some(if self.pile_o_bones_earned {
                AchievementEvaluation::Earned
            } else {
                AchievementEvaluation::Failed
            });

        let required = u32::try_from(self.encountered_hostiles.len())
            .expect("hostile achievement entity count exceeds u32");
        let bundled = hostile_body_counts.values().copied().max().unwrap_or(0);
        self.metrics.enemies_in_stash_building = bundled;
        self.metrics.enemies_required_for_stash = required;
        self.live_evaluations[AchievementId::AllEnemiesOneBuilding.index()] =
            Some(if required > 0 && bundled == required {
                AchievementEvaluation::Earned
            } else {
                AchievementEvaluation::Failed
            });
        self.refresh_metrics(frame);
        Ok(())
    }

    pub fn progress(&self, frame: u32) -> AchievementProgressSnapshot {
        let mut metrics = self.metrics;
        metrics.duration_frames = frame.saturating_sub(self.baseline_frame);
        AchievementProgressSnapshot {
            evaluations: AchievementEvaluations(array::from_fn(|index| {
                let id = AchievementId::ALL[index];
                if self.verifiable.contains(id) {
                    self.live_evaluations[index].unwrap_or(AchievementEvaluation::Unverifiable)
                } else {
                    AchievementEvaluation::Unverifiable
                }
            })),
            metrics,
        }
    }

    fn publish_basic_evaluations(&mut self, npc_deaths_invalidate_clean_hands: bool) {
        let clean = self.player_caused_deaths.is_empty()
            && (!npc_deaths_invalidate_clean_hands || self.npc_caused_deaths.is_empty());
        self.live_evaluations[AchievementId::CleanHands.index()] = Some(if clean {
            AchievementEvaluation::Earned
        } else {
            AchievementEvaluation::Failed
        });
        self.live_evaluations[AchievementId::Ghost.index()] =
            Some(if self.observation_pairs.is_empty() {
                AchievementEvaluation::Earned
            } else {
                AchievementEvaluation::Failed
            });
    }

    fn refresh_metrics(&mut self, frame: u32) {
        self.metrics.duration_frames = frame.saturating_sub(self.baseline_frame);
        self.metrics.baseline_living_npcs = u32::try_from(self.baseline_living_npcs.len())
            .expect("living NPC baseline count exceeds u32");
        self.metrics.baseline_dead_npcs = u32::try_from(self.baseline_dead_npcs.len())
            .expect("dead NPC baseline count exceeds u32");
        self.metrics.encountered_hostiles = u32::try_from(self.encountered_hostiles.len())
            .expect("encountered hostile count exceeds u32");
        self.metrics.player_caused_deaths = u32::try_from(self.player_caused_deaths.len())
            .expect("player-caused death count exceeds u32");
        self.metrics.npc_caused_deaths = u32::try_from(self.npc_caused_deaths.len())
            .expect("NPC-caused death count exceeds u32");
        self.metrics.unique_hostile_observers = u32::try_from(self.hostile_observers.len())
            .expect("hostile observer count exceeds u32");
        self.metrics.unique_observed_player_characters =
            u32::try_from(self.observed_player_characters.len())
                .expect("observed player character count exceeds u32");
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

    pub const fn history_promotion_attempted(&self) -> bool {
        self.history_promotion_attempted
    }

    pub fn mark_history_promotion_attempted(&mut self) {
        self.history_promotion_attempted = true;
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
                metrics: self.metrics,
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

impl Default for AchievementRunKind {
    fn default() -> Self {
        Self::Campaign
    }
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
    fn aggregation_policy_is_typed_and_stable() {
        assert_eq!(
            AchievementId::CleanHands.aggregation_policy(),
            AchievementAggregationPolicy::AllRequiredMissions
        );
        assert_eq!(
            AchievementId::Ghost.aggregation_policy(),
            AchievementAggregationPolicy::AllRequiredMissions
        );
        assert_eq!(
            AchievementId::PileOBones.aggregation_policy(),
            AchievementAggregationPolicy::AnyMissionOnce
        );
        assert_eq!(
            AchievementId::AllEnemiesOneBuilding.aggregation_policy(),
            AchievementAggregationPolicy::AnyMissionOnce
        );
        assert_eq!(
            serde_json::to_string(&AchievementAggregationPolicy::AnyMissionOnce).unwrap(),
            "1"
        );
        assert!(serde_json::from_str::<AchievementAggregationPolicy>("2").is_err());
    }

    #[test]
    fn typed_aggregation_distinguishes_all_required_from_any_once() {
        let shared = AchievementAggregationInput {
            envelope_complete: false,
            envelope_unverifiable: false,
            earned_missions: 1,
            required_missions: 2,
            unverifiable_missions: 0,
        };
        assert_eq!(
            aggregate_achievement(AchievementId::CleanHands, shared).status,
            AchievementAggregationStatus::InProgress
        );
        assert_eq!(
            aggregate_achievement(AchievementId::PileOBones, shared).status,
            AchievementAggregationStatus::Earned
        );
        let any_progress = aggregate_achievement(
            AchievementId::PileOBones,
            AchievementAggregationInput {
                earned_missions: 7,
                required_missions: 12,
                unverifiable_missions: 4,
                ..Default::default()
            },
        );
        assert_eq!(
            (
                any_progress.earned_missions,
                any_progress.required_missions,
                any_progress.unverifiable_missions,
            ),
            (1, 1, 0),
            "any-once progress is a stable 0/1 or 1/1 envelope, not a mission total"
        );

        let completed = AchievementAggregationInput {
            envelope_complete: true,
            ..shared
        };
        assert_eq!(
            aggregate_achievement(AchievementId::CleanHands, completed).status,
            AchievementAggregationStatus::MissingRequirements
        );
        assert_eq!(
            aggregate_achievement(
                AchievementId::CleanHands,
                AchievementAggregationInput {
                    earned_missions: 2,
                    ..completed
                }
            )
            .status,
            AchievementAggregationStatus::Earned
        );
    }

    #[test]
    fn incomplete_envelope_cannot_be_promoted_to_all_required_success() {
        let progress = aggregate_achievement(
            AchievementId::Ghost,
            AchievementAggregationInput {
                envelope_complete: true,
                envelope_unverifiable: false,
                earned_missions: 1,
                required_missions: 2,
                unverifiable_missions: 1,
            },
        );
        assert_eq!(progress.status, AchievementAggregationStatus::Unverifiable);
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
    fn clean_hands_uses_fresh_exact_deaths_and_configurable_npc_rule() {
        use crate::entity_id::SoldierId;

        let baseline_dead = crate::element::EntityId::Soldier(SoldierId(1));
        let player_victim = crate::element::EntityId::Soldier(SoldierId(2));
        let npc_victim = crate::element::EntityId::Soldier(SoldierId(3));
        let mut state = MissionAchievementState::from_mission_start();
        state.initialize_mission_baseline(
            100,
            [
                (baseline_dead, true),
                (player_victim, false),
                (npc_victim, false),
            ],
        );
        assert_eq!(
            state.live_evaluation(AchievementId::CleanHands),
            Some(AchievementEvaluation::Earned)
        );

        state
            .record_npc_death(npc_victim, AchievementDeathCause::Npc, false)
            .unwrap();
        assert_eq!(
            state.live_evaluation(AchievementId::CleanHands),
            Some(AchievementEvaluation::Earned)
        );
        state.refresh_clean_hands_rule(true).unwrap();
        assert_eq!(
            state.live_evaluation(AchievementId::CleanHands),
            Some(AchievementEvaluation::Failed)
        );
        state.refresh_clean_hands_rule(false).unwrap();
        assert_eq!(
            state.live_evaluation(AchievementId::CleanHands),
            Some(AchievementEvaluation::Earned)
        );
        state
            .record_npc_death(
                player_victim,
                AchievementDeathCause::PlayerControlled,
                false,
            )
            .unwrap();
        assert_eq!(
            state.live_evaluation(AchievementId::CleanHands),
            Some(AchievementEvaluation::Failed)
        );
        assert_eq!(state.progress(125).metrics.duration_frames, 25);
        assert_eq!(state.progress(125).metrics.baseline_dead_npcs, 1);
    }

    #[test]
    fn ghost_latches_from_unique_hostile_optical_observations() {
        use crate::entity_id::{PcId, SoldierId};

        let mut state = MissionAchievementState::from_mission_start();
        state.initialize_mission_baseline(0, []);
        let observer = crate::element::EntityId::Soldier(SoldierId(4));
        let pc = crate::element::EntityId::Pc(PcId(5));
        state.record_hostile_observation(observer, pc).unwrap();
        state.record_hostile_observation(observer, pc).unwrap();
        let progress = state.progress(0);
        assert_eq!(
            progress.evaluations.get(AchievementId::Ghost),
            AchievementEvaluation::Failed
        );
        assert_eq!(progress.metrics.unique_hostile_observers, 1);
        assert_eq!(progress.metrics.unique_observed_player_characters, 1);
    }

    #[test]
    fn exact_building_trackers_latch_pile_but_recompute_whole_stash() {
        use crate::entity_id::SoldierId;

        let ids = (0..10)
            .map(|index| crate::element::EntityId::Soldier(SoldierId(index)))
            .collect::<Vec<_>>();
        let building = AchievementBuildingId {
            public_number: 7,
            arena_index: crate::fast_find_grid::SectorIndex::new(11),
        };
        let other_building = AchievementBuildingId {
            public_number: 7,
            arena_index: crate::fast_find_grid::SectorIndex::new(12),
        };
        let mut state = MissionAchievementState::from_mission_start();
        state.initialize_mission_baseline(0, ids.iter().copied().map(|id| (id, false)));
        state
            .refresh_hostile_arrangement(
                1,
                ids.iter().copied().map(|entity| AchievementEntitySnapshot {
                    entity,
                    hostile: true,
                    out_of_order: true,
                    building: Some(building),
                }),
            )
            .unwrap();
        assert_eq!(
            state.live_evaluation(AchievementId::PileOBones),
            Some(AchievementEvaluation::Earned)
        );
        assert_eq!(
            state.live_evaluation(AchievementId::AllEnemiesOneBuilding),
            Some(AchievementEvaluation::Earned)
        );

        state
            .refresh_hostile_arrangement(
                2,
                ids.iter()
                    .copied()
                    .enumerate()
                    .map(|(index, entity)| AchievementEntitySnapshot {
                        entity,
                        hostile: true,
                        out_of_order: true,
                        building: Some(if index == 0 { other_building } else { building }),
                    }),
            )
            .unwrap();
        assert_eq!(
            state.live_evaluation(AchievementId::PileOBones),
            Some(AchievementEvaluation::Earned),
            "Pile-o-Bones remains earned after its condition was met"
        );
        assert_eq!(
            state.live_evaluation(AchievementId::AllEnemiesOneBuilding),
            Some(AchievementEvaluation::Failed),
            "whole-enemy stash is evaluated at the terminal layout"
        );
    }

    #[test]
    fn pile_counts_non_hostile_npc_bodies_without_expanding_enemy_stash() {
        use crate::entity_id::{CivilianId, SoldierId};

        let building = AchievementBuildingId {
            public_number: 9,
            arena_index: crate::fast_find_grid::SectorIndex::new(3),
        };
        let hostile = crate::element::EntityId::Soldier(SoldierId(20));
        let civilians = (0..9)
            .map(|index| crate::element::EntityId::Civilian(CivilianId(index)))
            .collect::<Vec<_>>();
        let mut state = MissionAchievementState::from_mission_start();
        state.initialize_mission_baseline(0, []);
        state
            .refresh_hostile_arrangement(
                1,
                std::iter::once(AchievementEntitySnapshot {
                    entity: hostile,
                    hostile: true,
                    out_of_order: true,
                    building: Some(building),
                })
                .chain(civilians.iter().copied().map(|entity| {
                    AchievementEntitySnapshot {
                        entity,
                        hostile: false,
                        out_of_order: true,
                        building: Some(building),
                    }
                })),
            )
            .unwrap();

        let progress = state.progress(1);
        assert_eq!(
            progress.evaluations.get(AchievementId::PileOBones),
            AchievementEvaluation::Earned
        );
        assert_eq!(progress.metrics.max_bodies_in_one_building, 10);
        assert_eq!(progress.metrics.enemies_required_for_stash, 1);
        assert_eq!(progress.metrics.enemies_in_stash_building, 1);
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
