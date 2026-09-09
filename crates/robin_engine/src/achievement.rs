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
//! rejected by the mandatory typed-history schema; only original-game saves use
//! the explicit incomplete-evidence import path.

use std::{array, collections::BTreeSet, fmt};

use serde::{Deserialize, Serialize};

/// Number of stable achievement identifiers understood by this build.
pub const ACHIEVEMENT_COUNT: usize = 24;

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
    // Stable ID 3 is retired; never reuse it.
    Ruthless = 4,
    ImOffHome = 5,
    Charity = 6,
    AllBeggarInfo = 7,
    NoBannersPurchased = 8,
    AllBannersPurchased = 9,
    ALegendIsBorn = 10,
    ForKingRichard = 11,
    WholeMerryCompany = 12,
    NoEmptyPlaces = 13,
    ManyHands = 14,
    KillCivilian = 15,
    LeaveEveryoneStanding = 16,
    NotAScratch = 17,
    OnMyMark = 18,
    YouNeverSawUsLeave = 19,
    StringTheory = 20,
    RoundOnTheFriar = 21,
    SomethingInTheAir = 22,
    DifferentKindOfScarlet = 23,
    PeopleBehindTheLegend = 24,
}

impl AchievementId {
    pub const fn campaign_only(self) -> bool {
        matches!(
            self,
            Self::ALegendIsBorn
                | Self::ForKingRichard
                | Self::WholeMerryCompany
                | Self::NoEmptyPlaces
                | Self::ManyHands
                | Self::KillCivilian
                | Self::PileOBones
                | Self::Charity
                | Self::StringTheory
                | Self::DifferentKindOfScarlet
                | Self::OnMyMark
                | Self::YouNeverSawUsLeave
                | Self::RoundOnTheFriar
                | Self::SomethingInTheAir
        )
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::CleanHands => "Clean Hands",
            Self::Ghost => "Ghost",
            Self::PileOBones => "Pile-o-Bones",
            Self::Ruthless => "Ruthless",
            Self::ImOffHome => "I'm off home",
            Self::Charity => "Nothing in Return",
            Self::AllBeggarInfo => "Word on the Street",
            Self::NoBannersPurchased => "Earned, Not Bought",
            Self::AllBannersPurchased => "Spare No Expense",
            Self::ALegendIsBorn => "A Legend Is Born",
            Self::ForKingRichard => "For King Richard",
            Self::WholeMerryCompany => "The Whole Merry Company",
            Self::NoEmptyPlaces => "No Empty Places at the Table",
            Self::ManyHands => "Many Hands Make Sherwood",
            Self::KillCivilian => "Kill a Civilian",
            Self::LeaveEveryoneStanding => "Leave Everyone Standing",
            Self::NotAScratch => "Not a Scratch",
            Self::OnMyMark => "On My Mark",
            Self::YouNeverSawUsLeave => "You Never Saw Us Leave",
            Self::StringTheory => "String Theory",
            Self::RoundOnTheFriar => "A Round on the Friar",
            Self::SomethingInTheAir => "Something in the Air",
            Self::DifferentKindOfScarlet => "A Different Kind of Scarlet",
            Self::PeopleBehindTheLegend => "The People Behind the Legend",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::CleanHands => {
                "Complete the mission without player-caused deaths (NPC deaths also count when enabled)."
            }
            Self::Ghost => {
                "Complete the mission without a living hostile observing a player character."
            }
            Self::PileOBones => {
                "Place ten unconscious or dead NPCs in one building, then complete the mission."
            }
            Self::Ruthless => "Complete the mission with every enemy dead.",
            Self::ImOffHome => {
                "Knock every rich civilian unconscious at least once, then complete the mission. They may wake up."
            }
            Self::Charity => {
                "Give a beggar money after all their information is exhausted, receiving no information in return."
            }
            Self::AllBeggarInfo => {
                "Get all information from every beggar and complete the mission. Mission badge only."
            }
            Self::NoBannersPurchased => {
                "Complete a banner mission without purchasing any of its preparation banners."
            }
            Self::AllBannersPurchased => {
                "Complete a banner mission having purchased every preparation banner, rather than earning any."
            }
            Self::ALegendIsBorn => "Complete the full campaign.",
            Self::ForKingRichard => {
                "Complete Lackland's Plan and send the ransom with Allan-a-Dale."
            }
            Self::WholeMerryCompany => {
                "Complete the campaign after winning a mission with each of Robin's five named companions."
            }
            Self::NoEmptyPlaces => {
                "Complete the campaign without permanently losing a recruited hero or Merry Man, including strategic assignments."
            }
            Self::LeaveEveryoneStanding => {
                "Complete the mission unseen without harming or incapacitating any NPC. Distractions are allowed."
            }
            Self::NotAScratch => "Complete the mission without any party member losing health.",
            Self::OnMyMark => {
                "Have three characters successfully act on three distinct enemies in one quick-action execution, then win."
            }
            Self::YouNeverSawUsLeave => {
                "Escape three simultaneous pursuers without killing them or leaving the map, then win."
            }
            Self::StringTheory => {
                "Complete a mission with three player-knocked-out enemies alive, bound, and inside a building."
            }
            Self::RoundOnTheFriar => {
                "Have three different enemies drink beer placed by Tuck in one successful mission."
            }
            Self::SomethingInTheAir => {
                "A single player-thrown wasp nest must sting three different enemies in a successful mission."
            }
            Self::DifferentKindOfScarlet => {
                "Have Will Scarlet knock out six different enemies with his sling and finish with Clean Hands."
            }
            Self::PeopleBehindTheLegend => {
                "Win an optional ambush or tactical mission with only generic Merry Men."
            }
            Self::KillCivilian => {
                "Kill a civilian during a successful mission, including indirect deaths. Civilians already dead at mission start do not count."
            }
            Self::ManyHands => {
                "Complete the campaign after three distinct generic Merry Men each contribute to a mission victory and complete production or training in Sherwood."
            }
        }
    }
    pub const ALL: [Self; ACHIEVEMENT_COUNT] = [
        Self::CleanHands,
        Self::Ghost,
        Self::PileOBones,
        Self::Ruthless,
        Self::ImOffHome,
        Self::Charity,
        Self::AllBeggarInfo,
        Self::NoBannersPurchased,
        Self::AllBannersPurchased,
        Self::ALegendIsBorn,
        Self::ForKingRichard,
        Self::WholeMerryCompany,
        Self::NoEmptyPlaces,
        Self::ManyHands,
        Self::KillCivilian,
        Self::LeaveEveryoneStanding,
        Self::NotAScratch,
        Self::OnMyMark,
        Self::YouNeverSawUsLeave,
        Self::StringTheory,
        Self::RoundOnTheFriar,
        Self::SomethingInTheAir,
        Self::DifferentKindOfScarlet,
        Self::PeopleBehindTheLegend,
    ];

    pub const fn index(self) -> usize {
        let value = self as usize;
        if value > 3 { value - 1 } else { value }
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
            Self::Ruthless => "ruthless",
            Self::ImOffHome => "im-off-home",
            Self::Charity => "charity",
            Self::AllBeggarInfo => "all-beggar-info",
            Self::NoBannersPurchased => "no-banners-purchased",
            Self::AllBannersPurchased => "all-banners-purchased",
            Self::ALegendIsBorn => "a-legend-is-born",
            Self::ForKingRichard => "for-king-richard",
            Self::WholeMerryCompany => "whole-merry-company",
            Self::NoEmptyPlaces => "no-empty-places",
            Self::ManyHands => "many-hands",
            Self::KillCivilian => "kill-a-civilian",
            Self::LeaveEveryoneStanding => "leave-everyone-standing",
            Self::NotAScratch => "not-a-scratch",
            Self::OnMyMark => "on-my-mark",
            Self::YouNeverSawUsLeave => "you-never-saw-us-leave",
            Self::StringTheory => "string-theory",
            Self::RoundOnTheFriar => "round-on-the-friar",
            Self::SomethingInTheAir => "something-in-the-air",
            Self::DifferentKindOfScarlet => "different-kind-of-scarlet",
            Self::PeopleBehindTheLegend => "people-behind-the-legend",
        }
    }

    /// Campaign/lifetime aggregation semantics for this stable achievement.
    ///
    /// Keeping this policy beside the persistent identifier prevents campaign,
    /// profile, and UI code from growing separate name-specific conditionals.
    pub const fn aggregation_policy(self) -> AchievementAggregationPolicy {
        if matches!(self, Self::CleanHands | Self::Ghost) {
            AchievementAggregationPolicy::AllRequiredMissions
        } else if self.campaign_only() {
            AchievementAggregationPolicy::AnyMissionOnce
        } else {
            AchievementAggregationPolicy::MissionOnly
        }
    }

    /// Resolve a persisted numeric identifier without inventing a fallback for
    /// corrupt or newer data.
    pub const fn from_stable_id(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::CleanHands),
            1 => Some(Self::Ghost),
            2 => Some(Self::PileOBones),
            4 => Some(Self::Ruthless),
            5 => Some(Self::ImOffHome),
            6 => Some(Self::Charity),
            7 => Some(Self::AllBeggarInfo),
            8 => Some(Self::NoBannersPurchased),
            9 => Some(Self::AllBannersPurchased),
            10 => Some(Self::ALegendIsBorn),
            11 => Some(Self::ForKingRichard),
            12 => Some(Self::WholeMerryCompany),
            13 => Some(Self::NoEmptyPlaces),
            14 => Some(Self::ManyHands),
            15 => Some(Self::KillCivilian),
            16 => Some(Self::LeaveEveryoneStanding),
            17 => Some(Self::NotAScratch),
            18 => Some(Self::OnMyMark),
            19 => Some(Self::YouNeverSawUsLeave),
            20 => Some(Self::StringTheory),
            21 => Some(Self::RoundOnTheFriar),
            22 => Some(Self::SomethingInTheAir),
            23 => Some(Self::DifferentKindOfScarlet),
            24 => Some(Self::PeopleBehindTheLegend),
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
    /// Deliberately excluded from campaign and profile awards.
    MissionOnly = 2,
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
            2 => Ok(Self::MissionOnly),
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
        AchievementAggregationPolicy::MissionOnly => {
            AchievementAggregationStatus::MissingRequirements
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
    const KNOWN_BITS: u64 = ((1_u64 << (ACHIEVEMENT_COUNT + 1)) - 1) & !(1_u64 << 3);

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
    NotApplicable = 3,
}

impl AchievementEvaluation {
    pub(crate) const fn history_rank(self) -> u8 {
        match self {
            Self::Unverifiable | Self::NotApplicable => 0,
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
            3 => Ok(Self::NotApplicable),
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
    pub dead_enemies: u32,
    pub rich_civilians: u32,
    pub rich_civilians_knocked_out: u32,
    pub beggars: u32,
    pub beggars_exhausted: u32,
    pub charitable_payments: u32,
    pub banners_purchased: u32,
    pub purchasable_banners: u32,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AchievementEntitySnapshot {
    pub entity: crate::element::EntityId,
    /// Whether this NPC contributes to the "all enemies" requirement.
    pub hostile: bool,
    pub dead: bool,
    pub health: i32,
    pub rich_civilian: bool,
    pub unconscious: bool,
    pub bound: bool,
    pub beggar: bool,
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

/// Campaign-owned evidence, restored by restart/practice snapshots along with
/// the economy. Character indices identify individuals, not peasant templates.
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
pub struct CampaignDeeds {
    pub complete_evidence: bool,
    pub lost_members: BTreeSet<usize>,
    pub companion_victories: BTreeSet<u8>,
    pub contributing_veterans: BTreeSet<usize>,
    pub workers: BTreeSet<usize>,
    pub purchased_banners: std::collections::BTreeMap<u32, u32>,
}
impl Default for CampaignDeeds {
    fn default() -> Self {
        Self {
            complete_evidence: true,
            lost_members: BTreeSet::new(),
            companion_victories: BTreeSet::new(),
            contributing_veterans: BTreeSet::new(),
            workers: BTreeSet::new(),
            purchased_banners: std::collections::BTreeMap::new(),
        }
    }
}

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
    rich_civilians: BTreeSet<crate::element::EntityId>,
    rich_knockouts: BTreeSet<crate::element::EntityId>,
    beggars: BTreeSet<crate::element::EntityId>,
    exhausted_beggars: BTreeSet<crate::element::EntityId>,
    paid_beggars: BTreeSet<crate::element::EntityId>,
    pending_charity: BTreeSet<crate::element::EntityId>,
    dead_hostiles: BTreeSet<crate::element::EntityId>,
    processed_deaths: BTreeSet<crate::element::EntityId>,
    player_caused_deaths: BTreeSet<crate::element::EntityId>,
    npc_caused_deaths: BTreeSet<crate::element::EntityId>,
    hostile_observers: BTreeSet<crate::element::EntityId>,
    observed_player_characters: BTreeSet<crate::element::EntityId>,
    observation_pairs: BTreeSet<(crate::element::EntityId, crate::element::EntityId)>,
    #[serde(with = "serde_json_any_key::any_key_map_sized")]
    npc_baselines: std::collections::BTreeMap<crate::element::EntityId, (i32, bool)>,
    harmed_npc: bool,
    pub contributors: BTreeSet<usize>,
    #[serde(with = "serde_json_any_key::any_key_map_sized")]
    pub party_health: std::collections::BTreeMap<crate::element::EntityId, i32>,
    pub party_hurt: bool,
    pub knockouts: BTreeSet<crate::element::EntityId>,
    pub scarlet_knockouts: BTreeSet<crate::element::EntityId>,
    pub beer_by_tuck: BTreeSet<crate::element::EntityId>,
    pub beer_drinkers: BTreeSet<crate::element::EntityId>,
    #[serde(with = "serde_json_any_key::any_key_map_sized")]
    pub wasp_targets:
        std::collections::BTreeMap<crate::element::EntityId, BTreeSet<crate::element::EntityId>>,
    pub escape_pursuers: BTreeSet<crate::element::EntityId>,
    pub escape_earned: bool,
    #[serde(with = "serde_json_any_key::any_key_map_sized")]
    pub pending_stings:
        std::collections::BTreeMap<crate::element::EntityId, crate::element::EntityId>,
    pub replaying_qa: bool,
    pub named_party_participated: bool,
    pub qa_execution: u32,
    #[serde(with = "serde_json_any_key::any_key_map_sized")]
    pub qa_actors:
        std::collections::BTreeMap<crate::element::EntityId, (u32, crate::element::EntityId)>,
    #[serde(with = "qa_success_map_serde")]
    pub qa_successes: std::collections::BTreeMap<
        u32,
        std::collections::BTreeMap<crate::element::EntityId, crate::element::EntityId>,
    >,
    metrics: AchievementAttemptMetrics,
    pile_o_bones_earned: bool,
    history_promotion_attempted: bool,
    finalized: Option<MissionAchievementResults>,
}

/// Adapt the inner entity-keyed maps, not just the already JSON-compatible
/// execution IDs. Keep the runtime map types unchanged: native bitcode and
/// StateHash must continue to encode/hash the original typed maps directly.
mod qa_success_map_serde {
    use crate::element::EntityId;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;

    type Successes = BTreeMap<u32, BTreeMap<EntityId, EntityId>>;

    #[derive(Serialize, Deserialize)]
    #[serde(transparent)]
    struct Targets(
        #[serde(with = "serde_json_any_key::any_key_map_sized")] BTreeMap<EntityId, EntityId>,
    );

    pub(super) fn serialize<S: serde::Serializer>(
        successes: &Successes,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(successes.len()))?;
        for (execution, targets) in successes {
            // The wrapper exists only at the serde boundary, never in live
            // state. Copy one small completed-QA target map at a time.
            map.serialize_entry(execution, &Targets(targets.clone()))?;
        }
        map.end()
    }

    pub(super) fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Successes, D::Error> {
        BTreeMap::<u32, Targets>::deserialize(deserializer).map(|executions| {
            executions
                .into_iter()
                .map(|(execution, targets)| (execution, targets.0))
                .collect()
        })
    }
}

impl Default for MissionAchievementState {
    fn default() -> Self {
        Self::from_mission_start()
    }
}

impl MissionAchievementState {
    pub fn latch(&mut self, id: AchievementId) {
        self.ensure_not_finalized()
            .expect("achievement effect after finalization");
        self.live_evaluations[id.index()] = Some(AchievementEvaluation::Earned);
    }

    pub fn record_qa_success(
        &mut self,
        actor: crate::element::EntityId,
        target: crate::element::EntityId,
    ) {
        let Some(&(group, expected_target)) = self.qa_actors.get(&actor) else {
            return;
        };
        if target != expected_target {
            return;
        }
        self.qa_actors.remove(&actor);
        let successes = self.qa_successes.entry(group).or_default();
        successes.insert(actor, target);
        if successes.values().copied().collect::<BTreeSet<_>>().len() >= 3 {
            self.latch(AchievementId::OnMyMark);
        }
    }

    pub fn record_beer_drunk(
        &mut self,
        soldier: crate::element::EntityId,
        bottle: crate::element::EntityId,
    ) {
        if self.beer_by_tuck.contains(&bottle) {
            self.beer_drinkers.insert(soldier);
            if self.beer_drinkers.len() >= 3 {
                self.latch(AchievementId::RoundOnTheFriar);
            }
        }
    }

    pub fn record_wasp_sting(
        &mut self,
        nest: crate::element::EntityId,
        victim: crate::element::EntityId,
    ) {
        // Only nests registered on a successful player throw participate.
        if let Some(targets) = self.wasp_targets.get_mut(&nest) {
            targets.insert(victim);
            if targets.len() >= 3 {
                self.latch(AchievementId::SomethingInTheAir);
            }
        }
    }

    pub fn record_npc_harm(&mut self) {
        self.harmed_npc = true;
    }

    pub fn refresh_pursuit(
        &mut self,
        pursuing: BTreeSet<crate::element::EntityId>,
        present_alive: BTreeSet<crate::element::EntityId>,
    ) {
        if self
            .escape_pursuers
            .iter()
            .any(|id| !present_alive.contains(id))
        {
            self.escape_pursuers.clear();
        }
        if !self.escape_pursuers.is_empty() && self.escape_pursuers.is_disjoint(&pursuing) {
            self.escape_earned = true;
            self.latch(AchievementId::YouNeverSawUsLeave);
        }
        if pursuing.len() >= 3 {
            self.escape_pursuers.extend(pursuing);
        }
    }
    fn coverage(required: usize, achieved: usize) -> AchievementEvaluation {
        if required == 0 {
            AchievementEvaluation::NotApplicable
        } else if achieved == required {
            AchievementEvaluation::Earned
        } else {
            AchievementEvaluation::Failed
        }
    }

    pub fn configure_banners(&mut self, requirement: Option<(u32, u32)>) {
        for id in [
            AchievementId::NoBannersPurchased,
            AchievementId::AllBannersPurchased,
        ] {
            self.live_evaluations[id.index()] = Some(AchievementEvaluation::NotApplicable);
        }
        if let Some((purchased, total)) = requirement {
            assert!(
                total > 0 && purchased <= total,
                "invalid banner purchase evidence"
            );
            self.metrics.banners_purchased = purchased;
            self.metrics.purchasable_banners = total;
            self.live_evaluations[AchievementId::NoBannersPurchased.index()] =
                Some(if purchased == 0 {
                    AchievementEvaluation::Earned
                } else {
                    AchievementEvaluation::Failed
                });
            self.live_evaluations[AchievementId::AllBannersPurchased.index()] =
                Some(if purchased == total {
                    AchievementEvaluation::Earned
                } else {
                    AchievementEvaluation::Failed
                });
        }
    }

    /// Called only after a successful Pay deducts real money.
    pub fn record_beggar_payment(
        &mut self,
        beggar: crate::element::EntityId,
        exhausted: bool,
    ) -> Result<(), AchievementStateError> {
        self.ensure_not_finalized()?;
        self.paid_beggars.insert(beggar);
        if exhausted {
            self.pending_charity.insert(beggar);
        }
        Ok(())
    }

    /// Settle the paid response, never a scripted/unpaid reveal.
    pub fn record_beggar_response(
        &mut self,
        beggar: crate::element::EntityId,
        exhausted: bool,
        gave_info: bool,
    ) -> Result<(), AchievementStateError> {
        self.ensure_not_finalized()?;
        if self.paid_beggars.remove(&beggar) {
            if exhausted {
                self.exhausted_beggars.insert(beggar);
            }
            if self.pending_charity.remove(&beggar) && !gave_info {
                self.metrics.charitable_payments = self
                    .metrics
                    .charitable_payments
                    .checked_add(1)
                    .expect("charity count overflow");
            }
        }
        Ok(())
    }
    pub fn from_mission_start() -> Self {
        Self {
            tracking_provenance: AchievementTrackingProvenance::MissionStart,
            verifiable: AchievementSet::all(),
            live_evaluations: [Some(AchievementEvaluation::Failed); ACHIEVEMENT_COUNT],
            baseline_frame: 0,
            baseline_living_npcs: BTreeSet::new(),
            baseline_dead_npcs: BTreeSet::new(),
            encountered_hostiles: BTreeSet::new(),
            rich_civilians: BTreeSet::new(),
            rich_knockouts: BTreeSet::new(),
            beggars: BTreeSet::new(),
            exhausted_beggars: BTreeSet::new(),
            paid_beggars: BTreeSet::new(),
            pending_charity: BTreeSet::new(),
            dead_hostiles: BTreeSet::new(),
            processed_deaths: BTreeSet::new(),
            player_caused_deaths: BTreeSet::new(),
            npc_caused_deaths: BTreeSet::new(),
            hostile_observers: BTreeSet::new(),
            observed_player_characters: BTreeSet::new(),
            observation_pairs: BTreeSet::new(),
            npc_baselines: Default::default(),
            harmed_npc: false,
            contributors: BTreeSet::new(),
            party_health: Default::default(),
            party_hurt: false,
            knockouts: Default::default(),
            scarlet_knockouts: Default::default(),
            beer_by_tuck: Default::default(),
            beer_drinkers: Default::default(),
            wasp_targets: Default::default(),
            escape_pursuers: Default::default(),
            escape_earned: false,
            pending_stings: Default::default(),
            replaying_qa: false,
            named_party_participated: false,
            qa_execution: 0,
            qa_actors: Default::default(),
            qa_successes: Default::default(),
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
            rich_civilians: BTreeSet::new(),
            rich_knockouts: BTreeSet::new(),
            beggars: BTreeSet::new(),
            exhausted_beggars: BTreeSet::new(),
            paid_beggars: BTreeSet::new(),
            pending_charity: BTreeSet::new(),
            dead_hostiles: BTreeSet::new(),
            processed_deaths: BTreeSet::new(),
            player_caused_deaths: BTreeSet::new(),
            npc_caused_deaths: BTreeSet::new(),
            hostile_observers: BTreeSet::new(),
            observed_player_characters: BTreeSet::new(),
            observation_pairs: BTreeSet::new(),
            npc_baselines: Default::default(),
            harmed_npc: false,
            contributors: BTreeSet::new(),
            party_health: Default::default(),
            party_hurt: false,
            knockouts: Default::default(),
            scarlet_knockouts: Default::default(),
            beer_by_tuck: Default::default(),
            beer_drinkers: Default::default(),
            wasp_targets: Default::default(),
            escape_pursuers: Default::default(),
            escape_earned: false,
            pending_stings: Default::default(),
            replaying_qa: false,
            named_party_participated: false,
            qa_execution: 0,
            qa_actors: Default::default(),
            qa_successes: Default::default(),
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
    pub fn is_fresh_death(&self, victim: crate::element::EntityId) -> bool {
        !self.processed_deaths.contains(&victim)
    }

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

    /// Recompute body arrangements and mission population coverage.
    pub fn refresh_hostile_arrangement(
        &mut self,
        frame: u32,
        npcs: impl IntoIterator<Item = AchievementEntitySnapshot>,
    ) -> Result<(), AchievementStateError> {
        self.ensure_not_finalized()?;
        let mut body_counts = std::collections::BTreeMap::new();
        let mut bound_captives = 0;

        for npc in npcs {
            let baseline = self
                .npc_baselines
                .entry(npc.entity)
                .or_insert((npc.health, npc.out_of_order));
            if npc.health < baseline.0 || (npc.out_of_order && !baseline.1) {
                self.harmed_npc = true;
            }
            // Retain the high-water mark so healing cannot conceal later damage.
            baseline.0 = baseline.0.max(npc.health);
            if npc.hostile {
                if !self.baseline_dead_npcs.contains(&npc.entity) {
                    self.encountered_hostiles.insert(npc.entity);
                }
                if npc.dead {
                    self.dead_hostiles.insert(npc.entity);
                }
            }
            if npc.hostile
                && !npc.dead
                && npc.bound
                && npc.building.is_some()
                && self.knockouts.contains(&npc.entity)
            {
                bound_captives += 1;
            }
            if npc.rich_civilian {
                self.rich_civilians.insert(npc.entity);
                if npc.unconscious {
                    self.rich_knockouts.insert(npc.entity);
                }
            }
            if npc.beggar {
                self.beggars.insert(npc.entity);
            }
            if npc.out_of_order
                && let Some(building) = npc.building
            {
                *body_counts.entry(building).or_insert(0_u32) += 1;
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

        self.metrics.dead_enemies = self
            .encountered_hostiles
            .intersection(&self.dead_hostiles)
            .count()
            .try_into()
            .expect("enemy count overflow");
        self.metrics.rich_civilians = self
            .rich_civilians
            .len()
            .try_into()
            .expect("civilian count overflow");
        self.metrics.rich_civilians_knocked_out = self
            .rich_knockouts
            .len()
            .try_into()
            .expect("knockout count overflow");
        self.metrics.beggars = self
            .beggars
            .len()
            .try_into()
            .expect("beggar count overflow");
        let exhausted = self.beggars.intersection(&self.exhausted_beggars).count();
        self.metrics.beggars_exhausted = exhausted.try_into().expect("beggar count overflow");
        self.live_evaluations[AchievementId::Ruthless.index()] = Some(Self::coverage(
            self.encountered_hostiles.len(),
            self.metrics.dead_enemies as usize,
        ));
        self.live_evaluations[AchievementId::ImOffHome.index()] = Some(Self::coverage(
            self.rich_civilians.len(),
            self.rich_knockouts.len(),
        ));
        self.live_evaluations[AchievementId::AllBeggarInfo.index()] =
            Some(Self::coverage(self.beggars.len(), exhausted));
        self.live_evaluations[AchievementId::Charity.index()] =
            Some(if self.metrics.charitable_payments > 0 {
                AchievementEvaluation::Earned
            } else if self.beggars.is_empty() {
                AchievementEvaluation::NotApplicable
            } else {
                AchievementEvaluation::Failed
            });
        self.live_evaluations[AchievementId::StringTheory.index()] = Some(if bound_captives >= 3 {
            AchievementEvaluation::Earned
        } else {
            AchievementEvaluation::Failed
        });
        self.live_evaluations[AchievementId::DifferentKindOfScarlet.index()] = Some(
            if self.scarlet_knockouts.len() >= 6
                && self.live_evaluation(AchievementId::CleanHands)
                    == Some(AchievementEvaluation::Earned)
            {
                AchievementEvaluation::Earned
            } else {
                AchievementEvaluation::Failed
            },
        );
        self.live_evaluations[AchievementId::NotAScratch.index()] = Some(if self.party_hurt {
            AchievementEvaluation::Failed
        } else {
            AchievementEvaluation::Earned
        });
        self.live_evaluations[AchievementId::LeaveEveryoneStanding.index()] =
            Some(if !self.harmed_npc && self.observation_pairs.is_empty() {
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
#[derive(Default)]
pub enum AchievementRunKind {
    #[default]
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
        assert_eq!(AchievementId::Ruthless as u8, 4);
        for id in AchievementId::ALL {
            assert_eq!(AchievementId::from_stable_id(id.stable_id()), Some(id));
        }
        assert_eq!(AchievementId::from_stable_id(3), None);
        assert_eq!(serde_json::to_string(&AchievementId::Ghost).unwrap(), "1");
        assert_eq!(
            serde_json::from_str::<AchievementId>("4").unwrap(),
            AchievementId::Ruthless
        );
        assert!(serde_json::from_str::<AchievementId>("3").is_err());

        let set = AchievementSet::from_ids([AchievementId::Ruthless, AchievementId::CleanHands]);
        assert_eq!(
            set.iter().collect::<Vec<_>>(),
            vec![AchievementId::CleanHands, AchievementId::Ruthless]
        );
    }

    fn npc_snapshot(index: usize) -> AchievementEntitySnapshot {
        AchievementEntitySnapshot {
            entity: crate::element::EntityId::Soldier(crate::entity_id::SoldierId(index as u32)),
            hostile: true,
            dead: false,
            health: 100,
            rich_civilian: false,
            unconscious: false,
            bound: false,
            beggar: false,
            out_of_order: false,
            building: None,
        }
    }

    #[test]
    fn accepted_scopes_and_retired_bit_are_enforced() {
        assert_eq!(
            AchievementId::ALL
                .iter()
                .filter(|id| !id.campaign_only())
                .count(),
            10
        );
        assert_eq!(
            AchievementId::ALL
                .iter()
                .filter(|id| id.aggregation_policy() != AchievementAggregationPolicy::MissionOnly)
                .count(),
            16
        );
        assert_eq!(AchievementSet::all().0 & (1 << 3), 0);
    }

    #[test]
    fn rich_coverage_survives_waking_and_includes_new_arrivals() {
        let mut state = MissionAchievementState::default();
        let mut first = npc_snapshot(0);
        first.rich_civilian = true;
        state.refresh_hostile_arrangement(0, [first]).unwrap();
        first.unconscious = true;
        state.refresh_hostile_arrangement(1, [first]).unwrap();
        first.unconscious = false;
        state.refresh_hostile_arrangement(2, [first]).unwrap();
        assert_eq!(
            state.live_evaluation(AchievementId::ImOffHome),
            Some(AchievementEvaluation::Earned)
        );
        let mut second = npc_snapshot(1);
        second.rich_civilian = true;
        state
            .refresh_hostile_arrangement(3, [first, second])
            .unwrap();
        assert_eq!(
            state.live_evaluation(AchievementId::ImOffHome),
            Some(AchievementEvaluation::Failed)
        );
        assert_eq!(state.progress(3).metrics.rich_civilians_knocked_out, 1);
    }

    #[test]
    fn ruthless_requires_deaths_not_knockouts_and_ignores_initial_corpses() {
        let mut state = MissionAchievementState::default();
        let mut corpse = npc_snapshot(0);
        corpse.dead = true;
        let mut enemy = npc_snapshot(1);
        enemy.unconscious = true;
        state.initialize_mission_baseline(0, [(corpse.entity, true), (enemy.entity, false)]);
        state
            .refresh_hostile_arrangement(1, [corpse, enemy])
            .unwrap();
        assert_eq!(
            state.live_evaluation(AchievementId::Ruthless),
            Some(AchievementEvaluation::Failed)
        );
        enemy.dead = true;
        state
            .refresh_hostile_arrangement(2, [corpse, enemy])
            .unwrap();
        assert_eq!(
            state.live_evaluation(AchievementId::Ruthless),
            Some(AchievementEvaluation::Earned)
        );
        assert!(!state.is_fresh_death(corpse.entity));
        assert_eq!(state.progress(2).metrics.dead_enemies, 1);
    }

    #[test]
    fn last_hint_payment_is_not_charity_but_the_next_donation_is() {
        let mut state = MissionAchievementState::default();
        let mut beggar = npc_snapshot(0);
        beggar.beggar = true;
        state.record_beggar_payment(beggar.entity, false).unwrap();
        state
            .record_beggar_response(beggar.entity, true, true)
            .unwrap();
        state.refresh_hostile_arrangement(1, [beggar]).unwrap();
        assert_eq!(
            state.live_evaluation(AchievementId::AllBeggarInfo),
            Some(AchievementEvaluation::Earned)
        );
        assert_eq!(
            state.live_evaluation(AchievementId::Charity),
            Some(AchievementEvaluation::Failed)
        );
        state.record_beggar_payment(beggar.entity, true).unwrap();
        state
            .record_beggar_response(beggar.entity, true, false)
            .unwrap();
        state
            .record_beggar_response(beggar.entity, true, false)
            .unwrap();
        state.refresh_hostile_arrangement(2, [beggar]).unwrap();
        assert_eq!(state.progress(2).metrics.charitable_payments, 1);
        assert_eq!(
            state.live_evaluation(AchievementId::Charity),
            Some(AchievementEvaluation::Earned)
        );
    }

    #[test]
    fn banner_badges_distinguish_missing_mixed_zero_and_full_purchases() {
        let mut state = MissionAchievementState::default();
        for (requirement, expected) in [
            (None, [AchievementEvaluation::NotApplicable; 2]),
            (
                Some((0, 3)),
                [AchievementEvaluation::Earned, AchievementEvaluation::Failed],
            ),
            (Some((1, 3)), [AchievementEvaluation::Failed; 2]),
            (
                Some((3, 3)),
                [AchievementEvaluation::Failed, AchievementEvaluation::Earned],
            ),
        ] {
            state.configure_banners(requirement);
            assert_eq!(
                [
                    state
                        .live_evaluation(AchievementId::NoBannersPurchased)
                        .unwrap(),
                    state
                        .live_evaluation(AchievementId::AllBannersPurchased)
                        .unwrap()
                ],
                expected
            );
        }
    }

    #[test]
    fn quick_action_feat_requires_matching_targets_in_one_execution() {
        let mut state = MissionAchievementState::default();
        let ids = (0..6).map(|i| npc_snapshot(i).entity).collect::<Vec<_>>();
        for i in 0..3 {
            state.qa_actors.insert(ids[i], (1, ids[i + 3]));
        }
        state.record_qa_success(ids[0], ids[4]);
        assert!(state.qa_successes.is_empty());
        for i in 0..2 {
            state.record_qa_success(ids[i], ids[i + 3]);
        }
        assert_ne!(
            state.live_evaluation(AchievementId::OnMyMark),
            Some(AchievementEvaluation::Earned)
        );
        state.record_qa_success(ids[2], ids[5]);
        assert_eq!(
            state.live_evaluation(AchievementId::OnMyMark),
            Some(AchievementEvaluation::Earned)
        );
    }

    #[test]
    fn beer_and_wasps_require_distinct_victims_and_player_sources() {
        let mut state = MissionAchievementState::default();
        let ids = (0..7).map(|i| npc_snapshot(i).entity).collect::<Vec<_>>();
        state.beer_by_tuck.insert(ids[0]);
        state.wasp_targets.insert(ids[0], BTreeSet::new());
        state.wasp_targets.insert(ids[1], BTreeSet::new());
        for _ in 0..3 {
            state.record_beer_drunk(ids[2], ids[0]);
            state.record_wasp_sting(ids[0], ids[2]);
        }
        state.record_beer_drunk(ids[3], ids[6]);
        state.record_wasp_sting(ids[1], ids[3]);
        for id in [
            AchievementId::RoundOnTheFriar,
            AchievementId::SomethingInTheAir,
        ] {
            assert_ne!(
                state.live_evaluation(id),
                Some(AchievementEvaluation::Earned)
            );
        }
        for victim in [ids[3], ids[4]] {
            state.record_beer_drunk(victim, ids[0]);
            state.record_wasp_sting(ids[0], victim);
        }
        for id in [
            AchievementId::RoundOnTheFriar,
            AchievementId::SomethingInTheAir,
        ] {
            assert_eq!(
                state.live_evaluation(id),
                Some(AchievementEvaluation::Earned)
            );
        }
    }

    #[test]
    fn scarlet_requires_six_unique_knockouts_and_clean_hands_at_completion() {
        let mut state = MissionAchievementState::default();
        state.initialize_mission_baseline(0, []);
        for i in 0..5 {
            state.scarlet_knockouts.insert(npc_snapshot(i).entity);
        }
        state.refresh_hostile_arrangement(1, []).unwrap();
        assert_eq!(
            state.live_evaluation(AchievementId::DifferentKindOfScarlet),
            Some(AchievementEvaluation::Failed)
        );
        state.scarlet_knockouts.insert(npc_snapshot(5).entity);
        state.refresh_hostile_arrangement(2, []).unwrap();
        assert_eq!(
            state.live_evaluation(AchievementId::DifferentKindOfScarlet),
            Some(AchievementEvaluation::Earned)
        );
        state
            .record_npc_death(
                npc_snapshot(6).entity,
                AchievementDeathCause::PlayerControlled,
                false,
            )
            .unwrap();
        state.refresh_hostile_arrangement(3, []).unwrap();
        assert_eq!(
            state
                .finalize_success()
                .evaluation(AchievementId::DifferentKindOfScarlet),
            AchievementEvaluation::Failed
        );
    }

    #[test]
    fn escaped_pursuit_cannot_be_created_by_removing_the_pursuers() {
        let ids = (0..3)
            .map(|i| npc_snapshot(i).entity)
            .collect::<BTreeSet<_>>();
        let mut state = MissionAchievementState::default();
        state.refresh_pursuit(ids.clone(), ids.clone());
        state.refresh_pursuit(BTreeSet::new(), BTreeSet::new());
        assert!(!state.escape_earned);
        state.refresh_pursuit(ids.clone(), ids.clone());
        state.refresh_pursuit(BTreeSet::new(), ids);
        assert!(state.escape_earned);
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
            AchievementId::Ruthless.aggregation_policy(),
            AchievementAggregationPolicy::MissionOnly
        );
        assert_eq!(
            serde_json::to_string(&AchievementAggregationPolicy::AnyMissionOnce).unwrap(),
            "1"
        );
        assert_eq!(
            serde_json::from_str::<AchievementAggregationPolicy>("2").unwrap(),
            AchievementAggregationPolicy::MissionOnly
        );
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
    fn populated_entity_keyed_achievement_maps_roundtrip_without_changing_native_state() {
        use crate::element::EntityId;
        use crate::entity_id::{PcId, SoldierId};
        use std::collections::BTreeMap;

        let pc = EntityId::Pc(PcId(0));
        let soldier = EntityId::Soldier(SoldierId(0));
        let mut state = MissionAchievementState::from_mission_start();
        // Both variants at slot zero must survive as distinct map keys.
        state.npc_baselines = BTreeMap::from([(pc, (100, false)), (soldier, (40, true))]);
        state.party_health = BTreeMap::from([(pc, 90), (soldier, 35)]);
        state.wasp_targets = BTreeMap::from([
            (pc, BTreeSet::from([soldier])),
            (soldier, BTreeSet::from([pc])),
        ]);
        state.pending_stings = BTreeMap::from([(pc, soldier), (soldier, pc)]);
        state.qa_actors = BTreeMap::from([(pc, (0, soldier)), (soldier, (1, pc))]);
        state.qa_successes = BTreeMap::from([
            (0, BTreeMap::from([(pc, soldier), (soldier, pc)])),
            (1, BTreeMap::new()),
        ]);

        // Reproduce the pre-adapter failure without a second full build: the
        // unchanged raw map types cannot be written as JSON directly.
        for error in [
            serde_json::to_string(&state.npc_baselines).unwrap_err(),
            serde_json::to_string(&state.qa_successes).unwrap_err(),
        ] {
            assert_eq!(error.to_string(), "key must be a string");
        }

        let native = bitcode::encode(&state);
        let hash = robin_util::state_hash::compute(&state);
        let json = serde_json::to_string(&state).expect("populated maps must be valid save JSON");
        let value = serde_json::to_value(&state).expect("generic snapshot JSON must also work");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&json).unwrap(),
            value
        );
        let pc_key = serde_json::to_string(&pc).unwrap();
        let soldier_key = serde_json::to_string(&soldier).unwrap();
        for field in [
            "npc_baselines",
            "party_health",
            "wasp_targets",
            "pending_stings",
            "qa_actors",
        ] {
            let map = value[field]
                .as_object()
                .expect("entity map is a JSON object");
            assert_eq!(map.len(), 2, "{field}");
            assert!(map.contains_key(&pc_key), "{field} lost typed Pc(0)");
            assert!(
                map.contains_key(&soldier_key),
                "{field} lost typed Soldier(0)"
            );
        }
        let targets = value["qa_successes"]["0"]
            .as_object()
            .expect("nested target map");
        assert_eq!(targets.len(), 2);
        assert!(targets.contains_key(&pc_key));
        assert!(targets.contains_key(&soldier_key));
        assert_eq!(value["qa_successes"]["1"], serde_json::json!({}));

        for decoded in [
            serde_json::from_str::<MissionAchievementState>(&json).unwrap(),
            serde_json::from_value::<MissionAchievementState>(value).unwrap(),
            bitcode::decode::<MissionAchievementState>(&native).unwrap(),
        ] {
            assert_eq!(decoded, state);
            assert_eq!(bitcode::encode(&decoded), native);
            assert_eq!(robin_util::state_hash::compute(&decoded), hash);
            assert_eq!(serde_json::to_string(&decoded).unwrap(), json);
        }
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
    fn exact_building_tracker_latches_pile() {
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
                    dead: false,
                    health: 100,
                    rich_civilian: false,
                    unconscious: false,
                    bound: false,
                    beggar: false,
                    out_of_order: true,
                    building: Some(building),
                }),
            )
            .unwrap();
        assert_eq!(
            state.live_evaluation(AchievementId::PileOBones),
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
                        dead: false,
                        health: 100,
                        rich_civilian: false,
                        unconscious: false,
                        bound: false,
                        beggar: false,
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
    }

    #[test]
    fn pile_counts_non_hostile_npc_bodies() {
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
                    dead: false,
                    health: 100,
                    rich_civilian: false,
                    unconscious: false,
                    bound: false,
                    beggar: false,
                    out_of_order: true,
                    building: Some(building),
                })
                .chain(civilians.iter().copied().map(|entity| {
                    AchievementEntitySnapshot {
                        entity,
                        hostile: false,
                        dead: false,
                        health: 100,
                        rich_civilian: false,
                        unconscious: false,
                        bound: false,
                        beggar: false,
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
        assert_eq!(progress.metrics.encountered_hostiles, 1);
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
