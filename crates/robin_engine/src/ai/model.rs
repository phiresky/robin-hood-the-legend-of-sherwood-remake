use super::*;
use crate::coordinates::MapVec;

// AI State
// ---------------------------------------------------------------------------

/// Top-level AI state.
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u32)]
pub enum AiState {
    Sleeping = 0,
    #[default]
    Default = 1,
    Wondering = 2,
    Seeking = 3,
    Attacking = 4,
    Menacing = 5,
    Fleeing = 6,
}

/// State codes used in the event system / scripts. Matches the `#define
/// AISTATE_*` constants from the header.
impl AiState {
    pub const SCRIPT_DRIVEN: u32 = 7;

    /// Translate an internal `STATE_*` engine enum to the script-visible
    /// `AISTATE_*` constant emitted by the `GetAIState` script native. The internal
    /// and script numeric spaces coincide for Sleeping/Default/Wondering/Seeking
    /// but differ for Attacking/Menacing/Fleeing.
    pub fn to_script_code(self) -> i32 {
        match self {
            Self::Sleeping => 0,  // AISTATE_SLEEPING
            Self::Default => 1,   // AISTATE_DEFAULT
            Self::Wondering => 2, // AISTATE_WONDERING
            Self::Seeking => 3,   // AISTATE_SEEKING
            Self::Menacing => 4,  // AISTATE_MENACING
            Self::Fleeing => 5,   // AISTATE_FLEEING
            Self::Attacking => 6, // AISTATE_ATTACKING
        }
    }

    /// AI event code for script `FilterAIEvent` state-change notifications.
    pub fn state_change_event_code(self) -> i32 {
        match self {
            Self::Sleeping => 100,
            Self::Default => 101,
            Self::Wondering => 102,
            Self::Seeking => 103,
            Self::Attacking => 104,
            Self::Menacing => 105,
            Self::Fleeing => 106,
        }
    }
}

// ── AI event codes for FilterAIEvent ────────────────────────────────
//
// Used by the per-actor script `FilterAIEvent` callback which can block
// stimulus processing (early gate) or is notified of state changes (late
// notification).

/// Map a stimulus type to its AI event code for `FilterAIEvent`.
///
/// Returns `Some(code)` for stimuli with a defined event mapping and
/// `None` for types that the original passes to `FilterAIEvent` as `-2`.
/// The mapping covers event codes 0–52.
///
/// Maps the original game's stimulus values to AI event values.
pub fn stimulus_to_ai_event_code(st: StimulusType) -> Option<i32> {
    match st {
        // Perception events (0–14)
        StimulusType::EventView => Some(0),
        StimulusType::EventOutOfView => Some(1),
        StimulusType::EventHear => Some(2),
        StimulusType::EventReachPoint => Some(3),
        StimulusType::EventCouldntReachPoint => Some(4),
        StimulusType::EventDone => Some(5),
        StimulusType::EventImpossible => Some(6),
        StimulusType::EventTimer => Some(7),
        StimulusType::EventSeesBody => Some(8),
        StimulusType::EventSeesObject => Some(9),
        StimulusType::EventSeesSoldier => Some(10),
        StimulusType::EventSeesFriendInTrouble => Some(11),
        StimulusType::EventFitAgain => Some(12),
        StimulusType::EventGotHit => Some(13),
        StimulusType::EventLoseConsciousness => Some(14),
        // Extended perception / combat events (15–32)
        StimulusType::EventMissesCharly => Some(15),
        StimulusType::EventObjectAway => Some(16),
        StimulusType::EventSeesCharly => Some(17),
        StimulusType::EventSyncCharly => Some(18),
        StimulusType::EventAfterScriptGoOn => Some(19),
        StimulusType::EventReturnToDuty => Some(20),
        StimulusType::EventPanic => Some(21),
        StimulusType::EventEnterSwordfight => Some(22),
        StimulusType::EventQuitSwordfight => Some(23),
        StimulusType::EventSwordStrike => Some(24),
        StimulusType::EventWasp => Some(25),
        StimulusType::EventWaspAway => Some(26),
        StimulusType::EventApple => Some(27),
        StimulusType::EventNet => Some(28),
        StimulusType::EventNetAway => Some(29),
        StimulusType::EventSeesBeggar => Some(30),
        StimulusType::EventGetArrow => Some(31),
        StimulusType::EventSeesBrawl => Some(32),
        // Inter-NPC calls (33–48)
        StimulusType::CallAlert => Some(33),
        StimulusType::CallCombatAlert => Some(34),
        StimulusType::CallFinishBrawl => Some(35),
        StimulusType::CallHey => Some(36),
        StimulusType::CallTowerGuardAlert => Some(37),
        StimulusType::CallTowerGuardCallsMe => Some(38),
        StimulusType::CallHint => Some(39),
        StimulusType::CallInstruction => Some(40),
        StimulusType::CallLookThere => Some(41),
        StimulusType::CallCoordinate => Some(42),
        StimulusType::CallReport => Some(43),
        StimulusType::CallGoToOfficer => Some(44),
        StimulusType::CallMrOfficerIAmBack => Some(45),
        StimulusType::CallCharlyIsBack => Some(46),
        StimulusType::CallPatrolCoordinate => Some(47),
        StimulusType::CallYouJustWait => Some(48),
        // Chase / combat / patrol events (49–52)
        StimulusType::EventAppleChaseNear => Some(49),
        StimulusType::EventDoorCombat => Some(50),
        StimulusType::EventGaloppLoopEnd => Some(51),
        StimulusType::EventSeesShadow => Some(52),
        // Original-game stimuli with no public AI event mapping. The decision tick's
        // default switch arm assigns -2 before calling FilterAIEvent.
        StimulusType::EventPcShotAtMe
        | StimulusType::EventArrowLaunched
        | StimulusType::EventStone
        | StimulusType::EventAdversaryWeak
        | StimulusType::EventAfterCombatInjury
        | StimulusType::CallCleanUpAfterBrawl
        | StimulusType::EventMyTalk0
        | StimulusType::EventMyTalk1
        | StimulusType::EventMyTalk2
        | StimulusType::EventMyTalk3
        | StimulusType::CallYourTalk0
        | StimulusType::CallYourTalk1
        | StimulusType::CallYourTalk2
        | StimulusType::CallYourTalk3
        | StimulusType::EventGoodStrike
        | StimulusType::EventLethalStrike
        | StimulusType::EventEnemyNear
        | StimulusType::EventStop
        | StimulusType::ForceBattleDecision
        | StimulusType::NoEvent => None,
    }
}

// ---------------------------------------------------------------------------
// AI Substate — massive enum
// ---------------------------------------------------------------------------

/// Fine-grained substate within an [`AiState`]. Implemented as a giant
/// flat enum with sentinel markers for each state group.
///
/// The numeric layout is preserved so savegame compatibility is possible
/// if needed.
#[derive(
    Default,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
    strum_macros::IntoStaticStr,
)]
#[repr(u32)]
#[allow(non_camel_case_types)]
// preserve original naming for clarity
// Overlay/log names (`Substate::log_string`). Variants whose original name
// does not follow the plain SCREAMING-KEBAB split carry an explicit
// `serialize`; `log_string_tests` pins every variant to the legacy table.
#[strum(serialize_all = "SCREAMING-KEBAB-CASE", prefix = "SUBSTATE-")]
pub enum Substate {
    // -- Sleeping substates --
    #[default]
    StartSleepingSubstates = 0,

    SleepingForever,
    SleepingUnconscious,
    SleepingNapping,
    SleepingAwakening,

    EndSleepingSubstates,

    // -- Default substates --
    StartDefaultSubstates,

    #[strum(serialize = "DEFAULT-GOTOPOST")]
    DefaultGotoPost,
    #[strum(serialize = "DEFAULT-GOTOPOST-TURN")]
    DefaultGotoPostTurn,
    #[strum(serialize = "DEFAULT-GOTOROUTE")]
    DefaultGotoRoute,
    #[strum(serialize = "DEFAULT-GOTOROUTE-TURN")]
    DefaultGotoRouteTurn,
    #[strum(serialize = "DEFAULT-ONPOST")]
    DefaultOnPost,
    #[strum(serialize = "DEFAULT-ONPOST-LOOKING-SIDEWARDS")]
    DefaultOnPostLookingSidewards,
    DefaultEnroute,
    DefaultScriptDriven,
    #[strum(serialize = "DEFAULT-INMACRO")]
    DefaultInMacro,
    #[strum(serialize = "DEFAULT-INMACRO-WAITING-FOR-DONE")]
    DefaultInMacroWaitingForDone,
    DefaultHomeSweetHome,
    DefaultLookingOfficerForAdvice,
    DefaultLookingForCharly,
    DefaultLookingSidewardsForCharly,
    DefaultDetectedCharly,
    DefaultSynchronizing,
    DefaultPatrolEnroute,
    DefaultPatrolEnrouteWaiting,
    DefaultLookingShadow,
    DefaultChildApproachedWhistling,

    EndDefaultSubstates,

    // -- Wondering substates --
    StartWonderingSubstates,

    WonderingWatching,
    #[strum(serialize = "WONDERING-LOOKING-1")]
    WonderingLooking1,
    #[strum(serialize = "WONDERING-LOOKING-1-SIDEWARDS")]
    WonderingLooking1Sidewards,
    #[strum(serialize = "WONDERING-LOOKING-2")]
    WonderingLooking2,
    #[strum(serialize = "WONDERING-LOOKING-2-SIDEWARDS")]
    WonderingLooking2Sidewards,
    #[strum(serialize = "WONDERING-LOOKING-3")]
    WonderingLooking3,
    #[strum(serialize = "WONDERING-LOOKING-3-SIDEWARDS")]
    WonderingLooking3Sidewards,
    WonderingWaspInArmour,
    WonderingAppleReactiontime,
    WonderingAppleChasingChild,
    WonderingAppleChasingChildWaiting,
    WonderingAppleChasingChildEnd,
    WonderingMoneyReactiontime,
    WonderingApproachingMoney,
    WonderingRunningForMoney,
    WonderingTakingMoney,
    WonderingBrawlReactiontime,
    WonderingBrawlApproaching,
    WonderingBrawlHitting,
    #[strum(serialize = "WONDERING-BRAWL-GOTHIT")]
    WonderingBrawlGotHit,
    WonderingBrawlRecovering,
    WonderingWatchingForMoreMoney,
    WonderingApproachingToLoot,
    WonderingLooting,
    WonderingAleReactiontime,
    WonderingApproachingAle,
    WonderingDrinkingAle,
    WonderingAleAway,
    WonderingWatchingTowerGuard,
    WonderingUnderNet,
    WonderingCivilianAdmiringHero,
    WonderingCivilianEnemyReactiontime,
    WonderingCivilianBodyReactiontime,
    WonderingOfficerSeeingBrawl,
    WonderingOfficerApproachingBrawl,
    WonderingOfficerFinishingBrawl,
    WonderingSoldierLookingOfficerWhoFinishedBrawl,
    WonderingHeardWhistling,
    WonderingWatchingWhistling,
    WonderingChildApproachingWhistling,

    EndWonderingSubstates,

    // -- Seeking substates --
    StartSeekingSubstates,

    SeekingHeardstepsReactiontime,
    SeekingHeardsteps,
    SeekingSeekpoint,
    SeekingSeekpointWatching,
    SeekingSeekpointWatchingSidewards,
    SeekingSeekpointPassedAmbushPointLeft,
    SeekingSeekpointPassedAmbushPointRight,
    SeekingSeekpointCheckingAmbushPoint,
    SeekingSeekpointApproachingBeggar,
    #[strum(serialize = "SEEKING-SEEKPOINT-IDENTIFYING-BEGGAR-1")]
    SeekingSeekpointIdentifyingBeggar1,
    #[strum(serialize = "SEEKING-SEEKPOINT-IDENTIFYING-BEGGAR-2")]
    SeekingSeekpointIdentifyingBeggar2,
    SeekingJustWatching,
    SeekingJustWatchingSidewards,
    SeekingKnightWatchingTowerGuard,
    SeekingOfficerCallSoldier,
    SeekingOfficerWaitForSoldier,
    SeekingOfficerInstructSoldier,
    SeekingOfficerWaitForInstructedSoldier,
    SeekingOfficerGetReportFromSoldier,
    SeekingOfficerGetAlertingReportFromSoldier,
    SeekingSoldierCalledByOfficer,
    SeekingSoldierGoToOfficer,
    SeekingSoldierGetInstructedByOfficer,
    SeekingSoldierReturnToOfficer,
    SeekingSoldierGiveReportToOfficer,
    SeekingSoldierGiveAlertingReportToOfficerStart,
    SeekingSoldierGiveAlertingReportToOfficerPoint,
    SeekingSoldierGiveAlertingReportToOfficerEnd,
    SeekingOfficerCallGroup,
    SeekingOfficerWaitForGroup,
    SeekingOfficerWaitInsideHouseToInstructGroup,
    SeekingOfficerLeavingHouseToInstructGroup,
    SeekingOfficerInstructGroup,
    SeekingOfficerInstructGroupPointing,
    SeekingOfficerWaitForInstructedGroup,
    SeekingGroupCalledByOfficer,
    SeekingGroupGoToOfficer,
    SeekingGroupGetInstructedByOfficer,
    SeekingBodyReactiontime,
    SeekingBody,
    SeekingNet,
    SeekingTakingNet,
    SeekingBodyLookingDeadBody,
    #[strum(serialize = "SEEKING-BODY-AWAKENING-SLEEPER")]
    SeekingBodyAwakeningSleeperr,
    #[strum(serialize = "SEEKING-OFFICER-LOOKING-FOR-SOLDIERS-1")]
    SeekingOfficerLookingForSoldiers1,
    #[strum(serialize = "SEEKING-OFFICER-LOOKING-FOR-SOLDIERS-1-SIDEWARDS")]
    SeekingOfficerLookingForSoldiers1Sidewards,
    #[strum(serialize = "SEEKING-OFFICER-LOOKING-FOR-SOLDIERS-2")]
    SeekingOfficerLookingForSoldiers2,
    #[strum(serialize = "SEEKING-OFFICER-LOOKING-FOR-SOLDIERS-2-SIDEWARDS")]
    SeekingOfficerLookingForSoldiers2Sidewards,
    #[strum(serialize = "SEEKING-OFFICER-LOOKING-FOR-SOLDIERS-3")]
    SeekingOfficerLookingForSoldiers3,
    #[strum(serialize = "SEEKING-OFFICER-LOOKING-FOR-SOLDIERS-3-SIDEWARDS")]
    SeekingOfficerLookingForSoldiers3Sidewards,
    SeekingRunningToOfficer,
    SeekingRunningToOfficerSeen,
    SeekingOfficerWaitForAlertingSoldier,
    SeekingArrowReactiontime,
    SeekingArrow,
    SeekingArrowJustWatching,
    SeekingArrowJustWatchingSidewards,
    SeekingCharly,
    SeekingCharlyWatching,
    SeekingDetectedCharly,
    SeekingSendCharlyToOfficer,
    SeekingLookingResurrectedCharly,
    SeekingCharlySentToOfficer,
    SeekingCharlyGoToOfficer,
    SeekingCharlyGoToOfficerSeen,
    SeekingCharlyGetLectureByOfficer,
    SeekingOfficerWaitForCharly,
    SeekingOfficerLectureCharly,
    SeekingOfficerLectureCharlyPointing,
    SeekingCombatAlertReactiontime,
    SeekingCombatAlert,
    SeekingCivilianRunningToSoldier,
    SeekingCivilianRunningToSoldierSeen,
    SeekingCivilianGiveAlertingReportToSoldierStart,
    SeekingCivilianGiveAlertingReportToSoldierPoint,
    SeekingCivilianGiveAlertingReportToSoldierEnd,
    SeekingWaitForAlertingCivilian,
    SeekingGetReportFromCivilian,
    SeekingGetAlertingReportFromCivilian,

    EndSeekingSubstates,

    // -- Attacking substates --
    StartAttackingSubstates,

    AttackingReactiontimeTurning,
    AttackingReactiontime,
    AttackingReactiontimeRunning,
    AttackingRunningToEnemy,
    AttackingWalkingToEnemy,
    AttackingChargingEnemy,
    AttackingOverviewLookLeft,
    AttackingOverviewLookRight,
    AttackingSwordfight,
    /// Original numeric substate retained even though the current combat
    /// implementation also tracks the pending strike sequence explicitly.
    /// Omitting it shifts every subsequent legacy substate discriminant.
    AttackingSwordfightSpecialStrike,
    AttackingSwordfightParade,
    AttackingQuittingSwordfight,
    AttackingReserve,
    AttackingReserveOverview,
    AttackingApproachToObserve,
    AttackingObserve,
    AttackingObserveAndMove,
    AttackingGotHit,
    AttackingGotHitStandingUp,
    AttackingHitting,
    AttackingApproachingNewEnemy,
    AttackingMovingAroundOldEnemy,
    AttackingApproachingSleepingEnemy,
    AttackingKillingSleepingEnemy,
    AttackingBowShooting,
    AttackingBowLoading,
    AttackingBowAiming,
    AttackingBowObserving,
    AttackingBowObservingLoading,
    AttackingArcherRetireFromCombat,
    AttackingArcherRetireFromCombatTurn,
    AttackingProtectingWithShield,
    AttackingAdvancingWithShield,
    AttackingBowRunningBehindShieldBearer,
    AttackingBowCorrectingPosition,
    AttackingPhalanx,
    AttackingRunningToPhalanx,
    AttackingOfficerGivingOrders,
    AttackingOfficerGivingOrdersWaiting,
    AttackingTooProudToAttack,
    AttackingTooProudToAttackOverview,
    AttackingTooProudToAttackRetire,
    AttackingTooProudToAttackRetireTurn,
    AttackingTooProudToAttackApproach,
    AttackingTowerGuardAlert,
    AttackingTowerGuardObserve,
    AttackingArcherRunOnShootingPath,
    AttackingArcherRunOnShootingPathFinalSprint,
    AttackingArcherRunOnShootingPathTurn,
    #[strum(serialize = "ATTACKING-ARCHER-WAIT-ON-ACHERY-PATH")]
    AttackingArcherWaitOnArcheryPath,
    #[strum(serialize = "ATTACKING-ARCHER-WAIT-ON-ACHERY-PATH-BENDING")]
    AttackingArcherWaitOnArcheryPathBending,
    AttackingDoorFightDelay,
    AttackingDoorFightLeaving,
    AttackingDoorFightTurning,
    AttackingDoorFightWaiting,
    AttackingRiderChargingApproachingBlindly,
    AttackingRiderChargingApproaching,
    AttackingRiderChargingPassing,
    AttackingRiderChargingGettingDistance,
    AttackingRiderChargingReturning,
    AttackingReactiontimeBending,
    AttackingArcherWaitOnBendPoint,

    AttackingDummyBehaviour,

    EndAttackingSubstates,

    // -- Menacing substates --
    StartMenacingSubstates,

    MenacingPcInComa,

    EndMenacingSubstates,

    // -- Fleeing substates --
    StartFleeingSubstates,

    FleeingRunToHide,
    FleeingRunToDoor,
    FleeingHiding,
    FleeingRunForArrowReserves,
    FleeingPanic,
    FleeingChildChased,
    FleeingChildChasedSupplementalRuns,
    FleeingChildChasedEnd,
    FleeingChildFriendChased,
    FleeingRunToAlertSoldiers,
    FleeingRetireFromCombat,
    FleeingRetireFromCombatTurn,
    FleeingMerryManRunToLeaveMap,
    FleeingMerryManLeaveMap,

    EndFleeingSubstates,

    // -- Additional substates (added later, outside main groups) --
    BeginAdditionalSubstates,

    AttackingSwordfightStepBack,
    WonderingAppleSauceInTheVisor,
    DefaultPatrolEnrouteRunning,
    #[strum(serialize = "DEFAULT-GOTOCHIEF")]
    DefaultGotoChief,
    DefaultPatrolChiefReturnToPatrol,
    WonderingApproachingBrawlVictim,
    WonderingAwakenBrawlVictim,
    WonderingOfficerFinishingBrawlWaiting,
    AttackingReturnToOtherPcAfterMenacing,
    #[strum(serialize = "SEEKING-CHARLY-GET-LECTURE-BY-OFFICER-2")]
    SeekingCharlyGetLectureByOfficer2,
    AttackingRunningToLadder,
    AttackingWaitingAtLadder,
    SeekingHeardstepsPreReactiontime,
    AttackingLastReserve,
    AttackingRunToAvengerOnRoof,
    AttackingWaitForAvengerOnRoof,
    SeekingGotStopEvent,
    SeekingGetAlertingReportFromCivilianLook,

    NumberOfSubstates,

    /// Sentinel — no substate.
    None = 0xFFFF_FFFF,
}

impl Substate {
    /// Return the top-level AI state that owns this numeric substate.
    ///
    /// The original game's state changes enforce these numeric family boundaries in
    /// debug builds. Additional substates live after the contiguous family
    /// ranges, so they are mapped explicitly here rather than inferred from
    /// their names.
    pub const fn ai_state_family(self) -> Option<AiState> {
        let raw = self as u32;
        if raw > Self::StartSleepingSubstates as u32 && raw < Self::EndSleepingSubstates as u32 {
            return Some(AiState::Sleeping);
        }
        if raw > Self::StartDefaultSubstates as u32 && raw < Self::EndDefaultSubstates as u32 {
            return Some(AiState::Default);
        }
        if raw > Self::StartWonderingSubstates as u32 && raw < Self::EndWonderingSubstates as u32 {
            return Some(AiState::Wondering);
        }
        if raw > Self::StartSeekingSubstates as u32 && raw < Self::EndSeekingSubstates as u32 {
            return Some(AiState::Seeking);
        }
        if raw > Self::StartAttackingSubstates as u32 && raw < Self::EndAttackingSubstates as u32 {
            return Some(AiState::Attacking);
        }
        if raw > Self::StartMenacingSubstates as u32 && raw < Self::EndMenacingSubstates as u32 {
            return Some(AiState::Menacing);
        }
        if raw > Self::StartFleeingSubstates as u32 && raw < Self::EndFleeingSubstates as u32 {
            return Some(AiState::Fleeing);
        }

        match self {
            Self::AttackingSwordfightStepBack
            | Self::AttackingReturnToOtherPcAfterMenacing
            | Self::AttackingRunningToLadder
            | Self::AttackingWaitingAtLadder
            | Self::AttackingLastReserve
            | Self::AttackingRunToAvengerOnRoof
            | Self::AttackingWaitForAvengerOnRoof => Some(AiState::Attacking),
            Self::WonderingAppleSauceInTheVisor
            | Self::WonderingApproachingBrawlVictim
            | Self::WonderingAwakenBrawlVictim
            | Self::WonderingOfficerFinishingBrawlWaiting => Some(AiState::Wondering),
            Self::DefaultPatrolEnrouteRunning
            | Self::DefaultGotoChief
            | Self::DefaultPatrolChiefReturnToPatrol => Some(AiState::Default),
            Self::SeekingCharlyGetLectureByOfficer2
            | Self::SeekingHeardstepsPreReactiontime
            | Self::SeekingGotStopEvent
            | Self::SeekingGetAlertingReportFromCivilianLook => Some(AiState::Seeking),
            _ => None,
        }
    }

    pub fn log_string_from_u16(raw: u16) -> &'static str {
        Self::try_from(u32::from(raw))
            .ok()
            .and_then(Self::log_string)
            .unwrap_or("SUBSTATE-???")
    }

    /// Overlay/log name; `None` for the group markers, the roof-avenger
    /// substates and the sentinels, which the original overlay never printed.
    pub fn log_string(self) -> Option<&'static str> {
        use Substate::*;

        match self {
            StartSleepingSubstates
            | EndSleepingSubstates
            | StartDefaultSubstates
            | EndDefaultSubstates
            | StartWonderingSubstates
            | EndWonderingSubstates
            | StartSeekingSubstates
            | EndSeekingSubstates
            | StartAttackingSubstates
            | EndAttackingSubstates
            | StartMenacingSubstates
            | EndMenacingSubstates
            | StartFleeingSubstates
            | EndFleeingSubstates
            | BeginAdditionalSubstates
            | AttackingRunToAvengerOnRoof
            | AttackingWaitForAvengerOnRoof
            | NumberOfSubstates
            | None => std::option::Option::None,
            other => Some(other.into()),
        }
    }

    /// Returns `true` if this substate is in the "seek area" group.
    pub fn is_seek_area(self) -> bool {
        matches!(
            self,
            Self::SeekingSeekpoint
                | Self::SeekingSeekpointWatching
                | Self::SeekingSeekpointWatchingSidewards
                | Self::SeekingSeekpointPassedAmbushPointLeft
                | Self::SeekingSeekpointPassedAmbushPointRight
                | Self::SeekingSeekpointCheckingAmbushPoint
                | Self::SeekingSeekpointApproachingBeggar
                | Self::SeekingSeekpointIdentifyingBeggar1
                | Self::SeekingSeekpointIdentifyingBeggar2
        )
    }

    /// Returns `true` if this is any swordfight substate.
    pub fn is_any_swordfight(self) -> bool {
        matches!(
            self,
            Self::AttackingRunningToEnemy
                | Self::AttackingWalkingToEnemy
                | Self::AttackingChargingEnemy
                | Self::AttackingSwordfight
                | Self::AttackingSwordfightSpecialStrike
                | Self::AttackingSwordfightParade
                | Self::AttackingApproachingNewEnemy
                | Self::AttackingSwordfightStepBack
                | Self::AttackingMovingAroundOldEnemy
        )
    }

    /// Returns `true` if this is an active swordfight substate.
    pub fn is_real_swordfight(self) -> bool {
        matches!(
            self,
            Self::AttackingSwordfight
                | Self::AttackingSwordfightSpecialStrike
                | Self::AttackingSwordfightParade
                | Self::AttackingApproachingNewEnemy
                | Self::AttackingSwordfightStepBack
                | Self::AttackingMovingAroundOldEnemy
        )
    }

    /// Any money-taking substate.
    pub fn is_take_money(self) -> bool {
        matches!(
            self,
            Self::WonderingMoneyReactiontime
                | Self::WonderingApproachingMoney
                | Self::WonderingRunningForMoney
                | Self::WonderingTakingMoney
        )
    }

    /// Any money-fight substate.
    pub fn is_fight_for_money(self) -> bool {
        matches!(
            self,
            Self::WonderingBrawlReactiontime
                | Self::WonderingBrawlApproaching
                | Self::WonderingBrawlHitting
                | Self::WonderingBrawlGotHit
                | Self::WonderingBrawlRecovering
                | Self::WonderingApproachingToLoot
                | Self::WonderingLooting
                | Self::WonderingWatchingForMoreMoney
        )
    }

    /// Any ale-taking substate.
    pub fn is_take_ale(self) -> bool {
        matches!(
            self,
            Self::WonderingAleReactiontime
                | Self::WonderingApproachingAle
                | Self::WonderingDrinkingAle
                | Self::WonderingAleAway
        )
    }
}

// ---------------------------------------------------------------------------
// Stored enum words
// ---------------------------------------------------------------------------

/// An AI enum whose original-game representation is a 32-bit enum word.
pub trait OriginalEnumWord: Copy + TryFrom<u32> {
    fn to_word(self) -> u32;
}

impl OriginalEnumWord for AiState {
    fn to_word(self) -> u32 {
        self as u32
    }
}

impl OriginalEnumWord for Substate {
    fn to_word(self) -> u32 {
        self as u32
    }
}

/// Raw serialized storage for an enum word of type `T`.
///
/// Legacy saves (and the original game's indeterminate initialization) can
/// carry words that are not valid `T` discriminants, so the raw `i32` is kept
/// verbatim. Every wire format is exactly that of the bare `i32`: the
/// `StateHash` impl delegates to it, serde is transparent, and the bitcode
/// derive encodes the single `i32` column (`PhantomData` encodes nothing) —
/// pinned by `stored_enum_word_wire_matches_raw_i32` in `ai/persisted/tests.rs`.
/// `Debug` also prints the bare `i32`.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
#[serde(transparent)]
pub struct StoredEnumWord<T> {
    raw: i32,
    #[serde(skip)]
    _enum: std::marker::PhantomData<T>,
}

impl<T> StoredEnumWord<T> {
    /// Preserve an arbitrary stored word (legacy saves, tests).
    pub const fn from_raw(raw: i32) -> Self {
        Self {
            raw,
            _enum: std::marker::PhantomData,
        }
    }

    /// The raw stored word, for wire projections.
    pub const fn raw(self) -> i32 {
        self.raw
    }
}

impl<T: OriginalEnumWord> StoredEnumWord<T> {
    pub fn new(value: T) -> Self {
        Self::from_raw(value.to_word() as i32)
    }

    /// Decode the stored word, panicking (naming `field`) when it is not a
    /// valid `T` — a live read of an indeterminate word is an invariant bug.
    #[track_caller]
    pub fn get(self, field: &'static str) -> T {
        T::try_from(self.raw as u32).unwrap_or_else(|_| {
            panic!(
                "live {field} contains invalid original-game enum word {}",
                self.raw
            )
        })
    }
}

impl<T> Default for StoredEnumWord<T> {
    fn default() -> Self {
        Self::from_raw(0)
    }
}

impl<T> std::fmt::Debug for StoredEnumWord<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.raw, f)
    }
}

impl<T> robin_util::state_hash::StateHash for StoredEnumWord<T> {
    fn state_hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Byte-identical to the former bare `i32` field (`write_i32`).
        robin_util::state_hash::StateHash::state_hash(&self.raw, state);
    }
}

// ---------------------------------------------------------------------------
// Emoticon type
// ---------------------------------------------------------------------------

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u32)]
pub enum EmoticonType {
    #[default]
    None = 0,
    GrowingQuestionMark,
    QuestionMark,
    XMark,
    Zzz,
    Cloud,
    Sun,
    Thunderstorm,
    Drunken,
}

// ---------------------------------------------------------------------------
// Probability distribution
// ---------------------------------------------------------------------------

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u32)]
pub enum ProbabilityDistribution {
    Rectangle = 0,
    Gauss,
    GaussHighVariance,
    Dirac,
}

// ---------------------------------------------------------------------------
// Stimulus types (events / calls)
// ---------------------------------------------------------------------------

/// The type of stimulus that can trigger an AI reaction.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
    strum_macros::IntoStaticStr,
)]
#[repr(u32)]
// Overlay/log names (`StimulusType::log_string`); explicit `serialize`
// entries keep the original's joined words and digit spellings.
#[strum(serialize_all = "SCREAMING-KEBAB-CASE")]
pub enum StimulusType {
    // -- Perception events --
    EventView = 0,
    #[strum(serialize = "EVENT-OUTOFVIEW")]
    EventOutOfView,
    EventHear,
    #[strum(serialize = "EVENT-REACHPOINT")]
    EventReachPoint,
    #[strum(serialize = "EVENT-COULDNT-REACHPOINT")]
    EventCouldntReachPoint,
    EventDone,
    EventImpossible,
    EventTimer,
    EventPcShotAtMe,
    #[strum(serialize = "EVENT-SEESBODY")]
    EventSeesBody,
    #[strum(serialize = "EVENT-SEESOBJECT")]
    EventSeesObject,
    EventSeesSoldier,
    #[strum(serialize = "EVENT-SEESFRIENDINTROUBLE")]
    EventSeesFriendInTrouble,
    #[strum(serialize = "EVENT-FITAGAIN")]
    EventFitAgain,
    #[strum(serialize = "EVENT-GOTHIT")]
    EventGotHit,
    EventLoseConsciousness,
    EventMissesCharly,
    EventObjectAway,
    EventSeesCharly,
    EventSyncCharly,
    EventAfterScriptGoOn,
    EventReturnToDuty,
    EventPanic,
    EventEnterSwordfight,
    EventQuitSwordfight,
    #[strum(serialize = "EVENT-SWORDSTRIKE")]
    EventSwordStrike,
    EventWasp,
    EventWaspAway,
    EventApple,
    EventNet,
    EventNetAway,
    EventSeesBeggar,
    EventGetArrow,
    EventSeesBrawl,
    // -- Calls (inter-NPC communication) --
    CallAlert,
    CallCombatAlert,
    CallHey,
    CallHint,
    CallInstruction,
    #[strum(serialize = "CALL-LOOKTHERE")]
    CallLookThere,
    CallCoordinate,
    CallReport,
    CallGoToOfficer,
    CallMrOfficerIAmBack,
    CallCharlyIsBack,
    CallPatrolCoordinate,
    CallTowerGuardAlert,
    CallTowerGuardCallsMe,
    CallFinishBrawl,
    CallYouJustWait,
    EventAppleChaseNear,
    EventDoorCombat,
    EventGaloppLoopEnd,
    EventSeesShadow,
    EventArrowLaunched,
    EventStone,
    EventAdversaryWeak,
    EventAfterCombatInjury,
    CallCleanUpAfterBrawl,
    #[strum(serialize = "EVENT-MYTALK-1")]
    EventMyTalk1,
    #[strum(serialize = "EVENT-MYTALK-2")]
    EventMyTalk2,
    #[strum(serialize = "EVENT-MYTALK-3")]
    EventMyTalk3,
    #[strum(serialize = "CALL-YOURTALK-1")]
    CallYourTalk1,
    #[strum(serialize = "CALL-YOURTALK-2")]
    CallYourTalk2,
    #[strum(serialize = "CALL-YOURTALK-3")]
    CallYourTalk3,
    EventGoodStrike,
    EventLethalStrike,
    EventEnemyNear,
    #[strum(serialize = "EVENT-MYTALK-0")]
    EventMyTalk0,
    #[strum(serialize = "CALL-YOURTALK-0")]
    CallYourTalk0,
    EventStop,
    NoEvent,
    /// Script-triggered: force AI to run battle_decisions(sim, ) immediately.
    ForceBattleDecision,
}

impl StimulusType {
    /// The "expected" stimulus class: completion events and officer/talk
    /// calls an actor is waiting for, as opposed to unsolicited perception.
    pub fn is_expected_class(self) -> bool {
        matches!(
            self,
            Self::EventReachPoint
                | Self::EventDone
                | Self::EventTimer
                | Self::EventSyncCharly
                | Self::CallCoordinate
                | Self::CallInstruction
                | Self::CallReport
                | Self::EventGaloppLoopEnd
                | Self::EventMyTalk0
                | Self::EventMyTalk1
                | Self::EventMyTalk2
                | Self::EventMyTalk3
                | Self::CallYourTalk0
                | Self::CallYourTalk1
                | Self::CallYourTalk2
                | Self::CallYourTalk3
        )
    }

    pub fn log_string_from_u16(raw: u16) -> &'static str {
        Self::try_from(u32::from(raw))
            .ok()
            .and_then(Self::log_string)
            .unwrap_or("EVENT-???")
    }

    /// Overlay/log name; `None` for the two internal pseudo-stimuli the
    /// original overlay never printed.
    pub fn log_string(self) -> Option<&'static str> {
        match self {
            StimulusType::NoEvent | StimulusType::ForceBattleDecision => None,
            other => Some(other.into()),
        }
    }
}

// ---------------------------------------------------------------------------
// Remark types
// ---------------------------------------------------------------------------

/// Speech/remark that an NPC can make.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    num_enum::TryFromPrimitive,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u32)]
pub enum Remark {
    SeesBody = 0,
    AwakensSleeperr,
    BahIlBougePus,
    SeesEnemy,
    HuntsEnemy,
    StartsCombat,
    ProvokesCombat,
    GoodStrikeCombat,
    CombatInsult,
    Warcry,
    KilledAdversary,
    Cassos,
    CallsOfficer,
    TellsOfficerBody,
    TellsOfficerEnemy,
    TellsOfficerOther,
    TellsOfficerCharlyAway,
    TellsOfficerWhere,
    AwaitsOrders,
    TellsOfficerNothing,
    CharlyDefendsHimself,
    MissesCharly,
    DidntFindCharly,
    FoundCharly,
    SendsCharlyToOfficer,
    WaspSting,
    UnderNet,
    SeesFriendUnderNet,
    Arrow,
    Wounded,
    Dies,
    Strangled,
    TiedUp,
    SeesObject,
    AleYes,
    AleNo,
    Drunken,
    HitByApple,
    ChasesChild,
    CaughtChild,
    GoldYes,
    GoldNo,
    GoldBrawl,
    SearchingSoldierGold,
    SearchingSoldierNothing,
    EndsSearch,
    Panic,
    HearsNoise,
    ControlsBeggar,
    MenacesPcInComa,
    BadExcuse,
    CryAlert,
    ShieldBearerCovers,
    ShieldBearersLineFormation,
    ArchersBehindShieldBearers,
    ProudDontFight,
    ProudFinallyFight,
    OfficerSeesBrawl,
    OfficerEndsBrawl,
    OfficerStopsPatrol,
    OfficerStartsPatrol,
    OfficerComplains,
    OfficerAsksWhatsup,
    OfficerAsksWhere,
    OfficerEndsConversation,
    OfficerCallsSoldier,
    OfficerSendsOutSoldier,
    OfficerCallsGroup,
    OfficerSendsOutGroup,
    OfficerSendsOutGroupForCharly,
    OfficerRebukesCharly,
    OfficerRebukesCharlyEnd,
    OfficerGivesAttackOrder,
    OutOfAmmunition,
    SpecialAction,
    AdmiresObjectScript,
    MissesObjectScript,
    GiveOrReceiveOrder,

    // -- Civilian remarks --
    CivSeesBody,
    CivSeesDeadBody,
    CivCallsSoldier,
    CivDenunciates,
    CivAdmiresRobin,
    CivPanic,
    CivWounded,
    CivDies,
    CivThanx,
    CivCries,
    CivBeerYes,
    CivBeerNo,
    CivSeesSoldiersUnderNet,
    CivUnderNet,
    CivApple,
    CivWasps,
    CivWhistling,
    CivSeesBrawl,
    CivGoldYes,
    CivGoldNo,
    CivBeggarBegging,
    CivBeggarGivesInfo,
    CivBeggarWantsMore,
    CivBeggarGivesLastInfo,
    CivBeggarThanx,
    CivBeggarIdentifiesHimself,
    CivChildCaughtBySoldier,
    CivChildChasedBySoldier,

    // -- VIP remarks --
    VipProudDontFight,
    VipProudFinallyFight,
    VipStartsCombat,
    VipWounded,
    VipDies,
    VipGoodStrikeCombat,
    VipWarcry,
    VipVictory,
    VipSpeaksToHimself,
    VipAleNo,
    VipNetNo,
    VipAppleNo,
    VipWaspsNo,
    VipGoldNo,

    NumberOfRemarks,
    /// Sentinel — no remark.
    TheSoundOfSilence,
}

impl Remark {
    /// First civilian remark variant.
    pub const FIRST_CIVILIAN: Self = Self::CivSeesBody;
    /// First VIP remark variant.
    pub const FIRST_VIP: Self = Self::VipProudDontFight;

    pub fn log_string_from_u16(raw: u16) -> &'static str {
        Self::try_from(u32::from(raw))
            .map(Self::speech)
            .unwrap_or(" ........... ")
    }

    /// Returns the NPC's actual French speech line for this remark.
    ///
    /// Strings are kept verbatim, including trailing-tab and trailing-space
    /// quirks (some lines pad with tabs to reserve display width). Variants
    /// without a dedicated arm — `NumberOfRemarks`, `TheSoundOfSilence` —
    /// fall through to the default arm.
    pub fn speech(self) -> &'static str {
        match self {
            Remark::SeesBody => "Ca va?",
            Remark::AwakensSleeperr => "Leve-toi!",
            Remark::BahIlBougePus => "Il est mort!",
            Remark::SeesEnemy => "Declinez votre identite! ",
            Remark::HuntsEnemy => "Halte!",
            Remark::StartsCombat => "Defends-toi !",
            Remark::ProvokesCombat => "Allez, viens!",
            Remark::GoodStrikeCombat => "Hahaaaaa!",
            Remark::CombatInsult => "Gibier de Potence!",
            Remark::Warcry => "A l'assaut!",
            Remark::KilledAdversary => "Un de moins!",
            Remark::Cassos => "Il est trop fort !",
            Remark::CallsOfficer => "Sire!",
            Remark::TellsOfficerBody => "Sire, un cadavre, Sire!",
            Remark::TellsOfficerEnemy => "Sire, des ennemis, Sire!",
            Remark::TellsOfficerOther => "Sire, un probleme, Sire !",
            Remark::TellsOfficerCharlyAway => "Sire, un garde manque a l'appel, Sire!",
            Remark::TellsOfficerWhere => "Sire, la-bas, Sire!",
            Remark::AwaitsOrders => "Sire, A vos ordres, Sire!",
            Remark::TellsOfficerNothing => "Sire, il n'y a rien, Sire!",
            Remark::CharlyDefendsHimself => "Sire, je...",
            Remark::MissesCharly => "O\u{FFFD} est-il?",
            Remark::DidntFindCharly => "Je ne le trouve pas!",
            Remark::FoundCharly => "O\u{FFFD} etais-tu?  ",
            Remark::SendsCharlyToOfficer => "L'officier te demande!\t\t\t\t\t\t\t\t\t\t\t\t\t\t",
            Remark::WaspSting => "Bon sang de guepe!\t\t\t\t\t\t\t\t\t\t\t\t\t\t",
            Remark::UnderNet => "Au secours! Sortez-moi d'ici!\t\t\t\t\t\t\t\t\t\t\t\t\t\t",
            Remark::SeesFriendUnderNet => "Aidons-les!",
            Remark::Arrow => "Qu'est-ce ?",
            Remark::Wounded => "Ouille!",
            Remark::Dies => "Ahhhh...",
            Remark::Strangled => " Alagrll mmf rgh",
            Remark::TiedUp => "Mohfefour!",
            Remark::SeesObject => "Qu'est-ce que c'est?",
            Remark::AleYes => "Hmm! Ca c'est gentil!",
            Remark::AleNo => "On ne boit pas en service !",
            Remark::Drunken => " HUPS On ne boit pas HUPS pendant le s... HUPS service!",
            Remark::HitByApple => "Qui a lance ca?",
            Remark::ChasesChild => "Encore ces gamins!",
            Remark::CaughtChild => "Tu vas voir, chenapan !",
            Remark::GoldYes => "Ah, de l'or!",
            Remark::GoldNo => "Cet argent ne m'appartient pas!",
            Remark::GoldBrawl => "Eh! C'est a moi!",
            Remark::SearchingSoldierGold => "Ah! C'est donc lui qui l'avait!",
            Remark::SearchingSoldierNothing => "C'est pas lui...",
            Remark::EndsSearch => "Il faut que je retourne a mon poste...",
            Remark::Panic => "Allons chercher des secours!",
            Remark::HearsNoise => "Qui va la?...",
            Remark::ControlsBeggar => "Controle!",
            Remark::MenacesPcInComa => "J'en tiens un!",
            Remark::BadExcuse => "Sire, il vous a insulte, Sire!",
            Remark::CryAlert => "Alerte!!! Alerte!!!",
            Remark::ShieldBearerCovers => {
                "A couvert! Ils ont des arcs!\t\t\t\t\t\t\t\t\t\t\t\t\t\t"
            }
            Remark::ShieldBearersLineFormation => "En ligne!",
            Remark::ArchersBehindShieldBearers => "Les archers, derriere!",
            Remark::ProudDontFight => "Montrez-moi ce que vous savez faire!",
            Remark::ProudFinallyFight => "Je vais vous montrer moi...",
            Remark::OfficerSeesBrawl => "Qu'est-ce qu'ils font, encore?",
            Remark::OfficerEndsBrawl => "Hkhmmmm!\t\t\t\t\t\t\t\t\t\t\t\t\t\t",
            Remark::OfficerStopsPatrol => "Halte !",
            Remark::OfficerStartsPatrol => "En avant, marche !",
            Remark::OfficerComplains => "Bande d'incapables !",
            Remark::OfficerAsksWhatsup => "Qu' y a-t-il, Soldat?",
            Remark::OfficerAsksWhere => "O\u{FFFD} ?",
            Remark::OfficerEndsConversation => "Rompez!",
            Remark::OfficerCallsSoldier => "Soldat!",
            Remark::OfficerSendsOutSoldier => "Va voir par la",
            Remark::OfficerCallsGroup => "A moi, la garde!",
            Remark::OfficerSendsOutGroup => "Examinez les alentours! Execution!",
            Remark::OfficerSendsOutGroupForCharly => "Trouvez-moi ce tire au flanc! Execution!",
            Remark::OfficerRebukesCharly => "Alors? On quitte son poste?",
            Remark::OfficerRebukesCharlyEnd => "Tu me feras trois jours!",
            Remark::OfficerGivesAttackOrder => "Soldats! A l'attaaaaque!!!",
            Remark::OutOfAmmunition => "J'ai plus de fleches!\t\t\t\t\t\t\t\t\t\t\t\t\t\t",
            Remark::SpecialAction => "hahaha",
            Remark::AdmiresObjectScript => "Alors ca ressemble a ca?",
            Remark::MissesObjectScript => "Bon sang! Il a disparu!",
            Remark::GiveOrReceiveOrder => "J'y vais!",

            Remark::CivSeesBody => "Oh, le pauvre!",
            Remark::CivSeesDeadBody => "Mais il est mort!",
            Remark::CivCallsSoldier => "Eh, le garde! ",
            Remark::CivDenunciates => "Y sont passes par la!",
            Remark::CivAdmiresRobin => "Qu'il est beau!",
            Remark::CivPanic => "A l'aide!",
            Remark::CivWounded => "Pitie!",
            Remark::CivDies => "hennnfff",
            Remark::CivThanx => "Oh, merci, merci",
            Remark::CivCries => "C'est affreux, affreux",
            Remark::CivBeerYes => "Une bonne chopine, ca rechauffe...",
            Remark::CivBeerNo => "Non, ca me ferait perdre la tete...",
            Remark::CivSeesSoldiersUnderNet => "Tiens? Elle a fini par en attraper un?",
            Remark::CivUnderNet => "Mais qui a fait ca?",
            Remark::CivApple => "Oh, le vilain petit garcon!",
            Remark::CivWasps => "Au secours, des guepes!",
            Remark::CivWhistling => "Arretes, mon mari va t'entendre!",
            Remark::CivSeesBrawl => "Quelle bande de brutes!",
            Remark::CivGoldYes => "Oh! Quelle chance!",
            Remark::CivGoldNo => "L'argent ne fait pas le bonheur...",
            Remark::CivBeggarBegging => "L'aumone, mon bon seigneur, l'aumone!",
            Remark::CivBeggarGivesInfo => "Merci bien! Je vais vous dire...",
            Remark::CivBeggarWantsMore => "Encore quelques sous, monseigneur?",
            Remark::CivBeggarGivesLastInfo => "Mon dernier conseil...",
            Remark::CivBeggarThanx => "Oh, merci!",
            Remark::CivBeggarIdentifiesHimself => "Voila, voila",
            Remark::CivChildCaughtBySoldier => "C'etait pas moi",
            Remark::CivChildChasedBySoldier => "Tu m'attraperas pas!",

            Remark::VipProudDontFight => "Qu'on l'echarpe!",
            Remark::VipProudFinallyFight => "Ahhh! Poussez-vous, bande d'incapables!",
            Remark::VipStartsCombat => "Je vais t'ecraser!",
            Remark::VipWounded => "Argh!",
            Remark::VipDies => "Noir tout est si  noir",
            Remark::VipGoodStrikeCombat => "Ca fait mal, hein?",
            Remark::VipWarcry => "Je ne vais pas te tuer tout de suite...",
            Remark::VipVictory => "Pff trop facile",
            Remark::VipSpeaksToHimself => "Une bataille! Qu'on me donne une bataille!",
            Remark::VipAleNo => "De la biere Tiede! Je ferait fouetter cet impudent!",
            Remark::VipNetNo => "Ah! Quelle idee grotesque!",
            Remark::VipAppleNo => "Une pomme? J'ai demande du CHEVREUIL que diable!",
            Remark::VipWaspsNo => "Des guepes? Hmm C'est une idee...",
            Remark::VipGoldNo => "Hmm Si un serviteur la ramasse, je le ferais fouetter..",

            Remark::NumberOfRemarks | Remark::TheSoundOfSilence => " ........... ",
        }
    }
}

impl std::fmt::Display for Remark {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.speech())
    }
}

// ---------------------------------------------------------------------------
// Question (decision-making queries)
// ---------------------------------------------------------------------------

/// Questions the AI asks itself to make behavior decisions.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u32)]
pub enum Question {
    ShallIFollowSteps = 0,
    ShallIStayOnMyPost,
    ShallIFollowLostEnemy,
    ShallIFollowHint,
    ShallIHelpFriendInTrouble,
    ShallIRun,
    ShallITakeAle,
    ShallITakeMoney,
    ShallIReactOnApple,
    ShallIFightForMoney,
    ShallISeekBeforeAlertingOfficer,
    ShallISeekBeforeAlertingSoldiers,
    ShallISendOutSoldier,
    ShallILookWhistle,
    ShallIFollowWhistle,
    HasTheNewTaskPriority,
}

// ---------------------------------------------------------------------------
// Battle decision
// ---------------------------------------------------------------------------

/// Battle-time tactical decisions.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
    Default,
    strum_macros::IntoStaticStr,
)]
#[repr(u32)]
#[strum(serialize_all = "SCREAMING-KEBAB-CASE", prefix = "DECISION-")]
pub enum Decision {
    #[default]
    None = 0,
    PredecisionOffensive,
    PredecisionDefensive,
    Cassos,
    Fight,
    Observe,
    Reserve,
    AlertSoldiers,
    RunAndAlertSoldiers,
    Menace,
    Shoot,
    ArcherStepBack,
    #[strum(serialize = "LOOK-4-HELP")]
    LookForHelp,
    #[strum(serialize = "LOOK-4-HELP-IF-NOBODY-ELSE-DOES")]
    LookForHelpIfNobodyElseDoes,
    CoverBehindShieldBearer,
    TooProudToAttack,
    TowerGuardAlert,
    TowerGuardObserve,
    ArcherObserve,
    RunToArcheryPoint,
    RunForNewArrows,
    LastReserve,
}

impl Decision {
    pub fn log_string_from_u16(raw: u16) -> &'static str {
        Self::try_from(u32::from(raw))
            .ok()
            .and_then(Self::log_string)
            .unwrap_or("DECISION-???")
    }

    /// Overlay/log name; `None` for the undecided and pre-decision markers,
    /// which the original overlay never printed.
    pub fn log_string(self) -> Option<&'static str> {
        match self {
            Decision::None | Decision::PredecisionOffensive | Decision::PredecisionDefensive => {
                None
            }
            other => Some(other.into()),
        }
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum AlertSoldiersFailureContinuation {
    None,
    ReturnToDuty,
    SeekBody { center: Position, radius: u16 },
    SeekMissedCharly { center: Position },
}

/// Patrol-path assignment variants — the three call shapes (sentinel
/// `-1`, sentinel `-2`, valid index) collapse to these semantic cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatrolAssignment {
    /// Sentinel `-1` / null pointer — drop the path, leave
    /// `likes_to_sit_around = false`.
    ClearPath,
    /// Sentinel `-2` / `(void*)-1` — drop the path but set
    /// `likes_to_sit_around = true`.
    ClearPathSitAround,
    /// Valid-index branch of patrol-path assignment by 16-bit index (waypoint-macro
    /// opcodes `CMD_CHANGE_WAY` / `CMD_STAY_HERE`). Clears both
    /// `likes_to_sit_around` and `special_action`.
    Index(PathId),
    /// Valid-reference branch of assigning a new hiking patrol path — the
    /// `AssignPath` script native. Unlike the index
    /// index-based path, while the original game's reference-based path
    /// only clears
    /// the likes-to-sit-around flag; an NPC authored with a Special/leisure
    /// initial action keeps its special-action flag while walking the
    /// scripted route, which later disables movement's already-on-point
    /// shortcut when it returns to duty.
    ScriptWay(PathId),
}

// ---------------------------------------------------------------------------
// Look direction
// ---------------------------------------------------------------------------

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u32)]
pub enum LookDirection {
    Left = 0,
    Right,
    LeftRight,
    RightLeft,
    Down,
}

// ---------------------------------------------------------------------------
// Log line type (debug AI log)
// ---------------------------------------------------------------------------

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u32)]
pub enum LogLineType {
    Event = 0,
    EventRefused,
    ChangeState,
    BattleDecision,
    Speak,
    SpeakImpossible,
    SpeakFinished,
    Timer,
}

/// A single AI log entry for debug display.
#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct LogLine {
    pub line_type: LogLineType,
    pub info: u16,
    pub frame: u32,
}

// ---------------------------------------------------------------------------
// Simple shared data types
// ---------------------------------------------------------------------------

/// Noise type.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u32)]
pub enum NoiseType {
    Plouf = 0,
    Bonk,
    Zonk,
    TapTapTap,
    ArfArf,
    Tirili,
    PutPut,
    Aaargh,
    Heeelp,
    Pling,
    Pfiiit,
    Logs,
    Drawbridge,
    ZingZing,
    Off,
    /// An intentionally thrown object impact. Appended after every Original
    /// ordinal so legacy enum values remain stable.
    Distraction,
}

/// Spatial origin of a noise. One-shot effects may deliberately have no
/// world layer (for example a crumpled net launched without a landing
/// surface), so absence is represented structurally rather than by layer
/// `0xffff`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct NoiseOrigin {
    pub x: f32,
    pub y: f32,
    pub sector: Option<crate::position_interface::SectorHandle>,
    pub layer: Option<crate::position_interface::Layer>,
}

impl NoiseOrigin {
    pub fn from_position(position: Position) -> Self {
        Self {
            x: position.x,
            y: position.y,
            sector: position.sector,
            layer: crate::position_interface::Layer::new(position.level),
        }
    }

    pub fn position(self) -> Option<Position> {
        self.layer.map(|layer| Position {
            x: self.x,
            y: self.y,
            sector: self.sector,
            level: layer.get(),
        })
    }

    /// Recreate the complete original-game position, including its authored
    /// `0xffff` no-layer sentinel. Projectile impacts can legitimately carry
    /// that sentinel together with a null sector; Original stores the raw
    /// position in AI state and projects it at ground level when facing it.
    pub fn legacy_position(self) -> Position {
        Position {
            x: self.x,
            y: self.y,
            sector: self.sector,
            level: self
                .layer
                .map_or(u16::MAX, crate::position_interface::Layer::get),
        }
    }
}

/// A noise event with origin, type, volume, and elevation.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct Noise {
    pub origin: NoiseOrigin,
    pub noise_type: NoiseType,
    pub volume: u16,
    pub elevation: u16,
    pub element_id: u16,
}

/// Detection level of a PC by an NPC.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u32)]
pub enum Detection {
    None = 0,
    Unrecognized,
    Recognized,
    /// Internally used by AI.
    Killed,
}

/// Global alert level.
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u32)]
pub enum AlertLevel {
    #[default]
    Green = 0,
    Yellow,
    Red,
}

/// NPC attitude toward PCs / the world.
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u32)]
pub enum Attitude {
    Friendly = 0,
    Neutral,
    #[default]
    Suspicious,
    Nervous,
    Hostile,
}

/// View cone configuration.
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u32)]
pub enum ViewCone {
    #[default]
    Commandoslike = 0,
    Patrol,
    QuickSearch,
    GetOverview,
    QuickOverview,
    SlowOverview,
    GattlingOverview,
    LookDown,
    LookTo,
    LookToOrCommandoslikeDependingOnIq,
    LookForward,
    Focus,
    GattlingFocus,
    Idle,
    Slow,
    LongRange,
    Sniper,
    SceneOfTheCrime,
    Valium,
}

/// Curiosity trigger type.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u32)]
pub enum Curiosity {
    Shot = 0,
    Dynamite,
    Siesta,
    Steps,
    Cards,
    Watch,
    Whistle,
    // Curiosity count — use Curiosity::COUNT
}

impl Curiosity {
    pub const COUNT: usize = 7;
}

/// Type of target (PC, NPC, or scarecrow).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u32)]
pub enum TargetType {
    Pc = 0,
    Npc,
    Scarecrow,
}

/// Report type for reconnaissance reports.
#[derive(
    Default,
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
#[repr(u32)]
pub enum ReportType {
    #[default]
    Nothing = 0,
    Noise,
    Body,
    MissedCharly,
    DeadBody,
    Enemy,
}

// ---------------------------------------------------------------------------
// Stimulus info — typed payload for stimuli
// ---------------------------------------------------------------------------

/// Hint passed between NPCs (e.g. "look over there").
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct Hint {
    pub seek_point: Position,
    pub seek_flags: u16,
    pub who_tells_me: AiEntityHandle,
}

/// Info about a stolen object.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct StolenObject {
    pub object: AiEntityHandle,
    pub thief: AiEntityHandle,
}

/// Info about a friend in trouble.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct CombatInfo {
    pub actor_npc: AiEntityHandle,
    pub enemy_position: Position,
}

/// Info about a door combat event.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct DoorCombatInfo {
    pub delay: u16,
    pub goal: Position,
    pub direction: u16,
    /// The original game's pre-door combat dispatch explicitly permits no adversary.
    /// Slot zero is a live human, so only `None` represents that null pointer.
    #[serde(with = "optional_ai_handle")]
    pub adversary: Option<AiEntityHandle>,
}

#[cfg(test)]
mod log_string_tests;
#[cfg(test)]
mod nullable_stimulus_reference_tests;

/// The payload of a [`Stimulus`].
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum StimulusInfo {
    #[default]
    None,
    Noise(Noise),
    Position(Position),
    Human(AiEntityHandle),
    Hint(Hint),
    Object(AiEntityHandle),
    Stolen(StolenObject),
    Combat(CombatInfo),
    DoorCombat(DoorCombatInfo),
    Index(u16),
    /// Exact invalid stimulus type storage from an old native save.
    ///
    /// The original game did not initialize this type field by default. Such a
    /// stimulus reaches the default/no-event dispatch path if it was queued,
    /// but retaining the raw word keeps the imported state inspectable.
    LegacyInvalidType(i32),
}

impl StimulusInfo {
    /// Direct object/human dispatch requires a live target. Other payloads
    /// describe events (noise, theft, combat) and retain their historical
    /// identities and positions even when an originating actor disappears.
    pub(crate) fn live_target(&self) -> Option<AiEntityHandle> {
        match self {
            Self::Human(handle) | Self::Object(handle) => Some(*handle),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Stimulus
// ---------------------------------------------------------------------------

/// An event or call that is dispatched to an NPC's AI for processing.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct Stimulus {
    pub stimulus_type: StimulusType,
    pub info: StimulusInfo,
    /// Optional original-game stimulus-owner reference. This is independent of the
    /// actor currently processing the stimulus and is initialized empty.
    #[serde(with = "optional_ai_handle")]
    pub owner: Option<AiEntityHandle>,
    pub to_whole_patrol: bool,
}

impl robin_util::state_hash::StateHash for Stimulus {
    fn state_hash<H: std::hash::Hasher>(&self, state: &mut H) {
        robin_util::state_hash::StateHash::state_hash(&self.stimulus_type, state);
        robin_util::state_hash::StateHash::state_hash(&self.info, state);
        robin_util::state_hash::StateHash::state_hash(&self.owner, state);
        robin_util::state_hash::StateHash::state_hash(&self.to_whole_patrol, state);
    }
}

impl Stimulus {
    pub fn new(stimulus_type: StimulusType) -> Self {
        Self {
            stimulus_type,
            info: StimulusInfo::None,
            owner: None,
            to_whole_patrol: false,
        }
    }

    pub fn with_noise(stimulus_type: StimulusType, noise: Noise) -> Self {
        Self {
            stimulus_type,
            info: StimulusInfo::Noise(noise),
            owner: None,
            to_whole_patrol: false,
        }
    }

    pub fn with_position(stimulus_type: StimulusType, pos: Position) -> Self {
        Self {
            stimulus_type,
            info: StimulusInfo::Position(pos),
            owner: None,
            to_whole_patrol: false,
        }
    }

    pub fn with_human(stimulus_type: StimulusType, human: HumanHandle) -> Self {
        Self {
            stimulus_type,
            info: StimulusInfo::Human(AiEntityHandle::new(human)),
            owner: None,
            to_whole_patrol: false,
        }
    }

    pub fn with_door_combat(stimulus_type: StimulusType, dc: DoorCombatInfo) -> Self {
        Self {
            stimulus_type,
            info: StimulusInfo::DoorCombat(dc),
            owner: None,
            to_whole_patrol: false,
        }
    }

    /// Returns `true` if two stimuli have the same type and equivalent info.
    pub fn is_similar(&self, other: &Self) -> bool {
        if self.stimulus_type != other.stimulus_type {
            return false;
        }
        match (&self.info, &other.info) {
            (StimulusInfo::None, StimulusInfo::None) => true,
            (StimulusInfo::Noise(a), StimulusInfo::Noise(b)) => {
                a.origin.x == b.origin.x && a.origin.y == b.origin.y && a.noise_type == b.noise_type
            }
            (StimulusInfo::Position(a), StimulusInfo::Position(b)) => a.x == b.x && a.y == b.y,
            (StimulusInfo::Human(a), StimulusInfo::Human(b)) => a == b,
            (StimulusInfo::Hint(a), StimulusInfo::Hint(b)) => {
                a.seek_point.x == b.seek_point.x
                    && a.seek_point.y == b.seek_point.y
                    && a.seek_flags == b.seek_flags
            }
            (StimulusInfo::Object(a), StimulusInfo::Object(b)) => a == b,
            (StimulusInfo::Stolen(a), StimulusInfo::Stolen(b)) => {
                a.object == b.object && a.thief == b.thief
            }
            (StimulusInfo::Combat(a), StimulusInfo::Combat(b)) => {
                a.enemy_position.x == b.enemy_position.x
                    && a.enemy_position.y == b.enemy_position.y
                    && a.actor_npc == b.actor_npc
            }
            (StimulusInfo::DoorCombat(a), StimulusInfo::DoorCombat(b)) => {
                a.goal.x == b.goal.x && a.goal.y == b.goal.y
            }
            (StimulusInfo::Index(a), StimulusInfo::Index(b)) => a == b,
            (StimulusInfo::LegacyInvalidType(a), StimulusInfo::LegacyInvalidType(b)) => a == b,
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Screen remark (HUD display)
// ---------------------------------------------------------------------------

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ScreenRemark {
    pub timer: u16,
    pub prefix: String,
    pub remark: Remark,
}

/// A forbidden remark entry — prevents the same line from being repeated.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ForbiddenRemark {
    pub remark: Remark,
    pub flags: u16,
    pub speech_id: u32,
    pub guy_index: u16,
    pub bad_guy: bool,
    pub forbidden_till_frame: u32,
}

// ---------------------------------------------------------------------------
// Reconnaissance report
// ---------------------------------------------------------------------------

#[derive(
    Default,
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ReconnaissanceReport {
    pub seek_position: Position,
    pub report_type: ReportType,
    pub seen_bodies: Vec<HumanHandle>,
    #[serde(with = "optional_ai_handle")]
    pub charly: Option<AiEntityHandle>,
    pub charly_seen: bool,
}

impl ReconnaissanceReport {
    pub fn reset(&mut self) {
        self.seen_bodies.clear();
        self.report_type = ReportType::Nothing;
        self.charly = None;
    }

    pub fn update(&mut self, new_type: ReportType, new_position: Position) {
        if self.report_type <= new_type {
            self.report_type = new_type;
            self.seek_position = new_position;
        }
    }

    /// Full report merging.
    ///
    /// `flags` is a bitmask:
    /// - `REPORT_UPDATE_BODIES` (1): merge seen_bodies from `other`
    /// - `REPORT_UPDATE_CHARLY` (2): copy charly handle if we don't have one
    /// - `REPORT_UPDATE_TYPE` (4): update report type and seek position
    pub fn add_seen_body(&mut self, body: HumanHandle) {
        self.seen_bodies.push(body);
    }

    pub fn is_body_seen(&self, body: HumanHandle) -> bool {
        self.seen_bodies.contains(&body)
    }
}

// ---------------------------------------------------------------------------
// Seek point
// ---------------------------------------------------------------------------

/// A point of interest that NPCs can investigate during seek-area sweeps.
///
/// Interest decays over time after examination: the `frame_when_full_interest`
/// field tracks when the point will be "fresh" again (100% interest).
/// Multiple NPCs avoid investigating the same point simultaneously via
/// the `locked` flag.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SeekPoint {
    pub position: Position,
    /// Frame at which interest will be 100% again.
    pub frame_when_full_interest: u32,
    /// Compass directions (0–15) to look from this point.
    pub directions: Vec<u16>,
    /// Last calculated interest value (0–100).
    pub last_calculated_interest: u8,
    /// Whether a soldier is currently investigating this point.
    pub locked: bool,
    /// Unique ID. Global seek points use their array index; personal
    /// seek points use sentinel values (1111, 2222).
    pub id: u16,
}

impl SeekPoint {
    /// Create a new seek point from a direction.
    ///
    /// We initialise `last_calculated_interest = 100` (full interest) as a
    /// safe, deterministic starting value; in the happy path
    /// `calculate_interest()` overwrites it before any reader inspects it.
    pub fn from_direction(dir: &SeekPointDirection) -> Self {
        Self {
            position: dir.position,
            directions: vec![dir.direction],
            frame_when_full_interest: 0,
            last_calculated_interest: 100,
            locked: false,
            id: 0,
        }
    }

    /// Create a seek point at a position with random directions.
    ///
    /// Uses `sim_rng` for deterministic RNG (port-wide choice) and
    /// initialises `last_calculated_interest = 100` — see `from_direction`
    /// above.
    pub fn from_position(sim: &crate::sim_rng::SimulationContext, pos: Position) -> Self {
        let directions = match crate::sim_rng::u8(
            sim,
            crate::sim_rng::RngSite::SeekPointDirectionPattern,
            0..4,
        ) {
            0 => vec![0, 3, 7, 11],
            1 => vec![2, 5, 10, 14],
            2 => vec![2, 7, 13],
            _ => vec![4, 10, 15],
        };
        Self {
            position: pos,
            directions,
            frame_when_full_interest: 0,
            last_calculated_interest: 100,
            locked: false,
            id: 0,
        }
    }

    /// Calculate interest based on elapsed time since last examination.
    /// Returns 0–100.
    pub fn calculate_interest(&mut self, current_frame: u32) -> u8 {
        let relative = current_frame as i32 - self.frame_when_full_interest as i32;
        self.last_calculated_interest = if relative >= 0 {
            100
        } else if relative <= -(crate::parameters_ai::SEEK_POINT_TIME_TO_REGAIN_FULL_INTEREST) {
            0
        } else {
            (100 + (100 * relative) / crate::parameters_ai::SEEK_POINT_TIME_TO_REGAIN_FULL_INTEREST)
                as u8
        };
        self.last_calculated_interest
    }

    /// Decrease interest (push full-interest frame further into the future).
    pub fn subtract_interest(&mut self, value: u8, current_frame: u32) {
        if self.frame_when_full_interest < current_frame {
            self.frame_when_full_interest = current_frame;
        }
        self.frame_when_full_interest += value as u32
            * crate::parameters_ai::SEEK_POINT_TIME_TO_REGAIN_1_PERCENT_OF_INTEREST as u32;
        let max =
            current_frame + crate::parameters_ai::SEEK_POINT_TIME_TO_REGAIN_FULL_INTEREST as u32;
        if self.frame_when_full_interest > max {
            self.frame_when_full_interest = max;
        }
    }

    /// Try to merge a nearby direction into this seek point.
    /// Returns `true` if the direction was close enough and was added.
    pub fn add_if_near(&mut self, dir: &SeekPointDirection) -> bool {
        let dx = (dir.position.x - self.position.x).abs();
        let dy = (dir.position.y - self.position.y).abs();
        let max_norm = dx.max(dy);
        if max_norm <= crate::parameters_ai::SEEK_POINT_UNIFY_TOLERANCE as f32 {
            // The original game stores these in a unique integer array: a near
            // duplicate is handled successfully, but is not inserted again.
            if !self.directions.contains(&dir.direction) {
                self.directions.push(dir.direction);
            }
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod seek_point_tests;

/// A seek-point direction from the level file (position + facing).
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SeekPointDirection {
    pub position: Position,
    pub direction: u16,
}

// ---------------------------------------------------------------------------
// Ambush point
// ---------------------------------------------------------------------------

/// A tactical ambush point that NPCs check while patrolling.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct AmbushPoint {
    pub position: Position,
    pub direction: u16,
    /// 3D anchor point — the 2D `position` lifted to eye height (z + 32).
    /// Used by the sight-polygon anchor for stealth / hide-in-ambush
    /// queries.
    pub position_3d: crate::coordinates::WorldPoint3D,
    /// Unique ambush-point ID assigned during AI initialization. Used by AI
    /// scripts that reference ambush points by index.
    pub id: u16,
}

/// Half-size of the ambush-containment box along the X axis.
pub const AMBUSH_BOX_HALF_SIZE: f32 = 100.0;

impl AmbushPoint {
    /// True iff `sector` and `level` match the ambush point's stored
    /// position and the 2D `point` lies inside the ambush containment
    /// box centred on `position` with half-diagonal
    /// `(AMBUSH_BOX_HALF_SIZE, AMBUSH_BOX_HALF_SIZE * ASPECT_RATIO)`.
    pub fn is_near(
        &self,
        point: crate::coordinates::MapPoint,
        level: u16,
        sector: Option<crate::position_interface::SectorHandle>,
    ) -> bool {
        if self.position.level != level || self.position.sector != sector {
            return false;
        }
        let dx = (point.x - self.position.x).abs();
        let dy = (point.y - self.position.y).abs();
        dx <= AMBUSH_BOX_HALF_SIZE
            && dy <= AMBUSH_BOX_HALF_SIZE * crate::position_interface::ASPECT_RATIO
    }
}

// ---------------------------------------------------------------------------
// Archery sector
// ---------------------------------------------------------------------------

/// A waypoint along an archery path (entry point or shooting point).
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct PointArchery {
    pub position: Position,
    pub direction: u16,
    /// True if this is a shooting position (not just a path waypoint).
    pub is_shooting_point: bool,
    /// Sector number of this point — used for sector-change distance
    /// penalty (compared against [`crate::position_interface::SectorHandle`]
    /// via [`crate::sector::SectorNumber`] u16 conversion).
    pub sector_index: crate::sector::SectorNumber,
    /// Entity of the archer occupying this point, or `None` if free.
    pub owner: Option<crate::entity_id::EntityId>,
}

/// An archery sector where archers can set up, with ordered waypoints
/// leading to shooting positions.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SectorArchery {
    pub points: Vec<PointArchery>,
    /// Polygon vertices for the `is_inside` check (f32 coords).
    pub polygon: Vec<(f32, f32)>,
    /// Layer / level this archery sector belongs to.
    pub layer: u16,
    /// Index of the first shooting point in `points`.  `None` when the
    /// sector has no shooting points.
    pub index_first_shooting_point: Option<crate::sector::ArcheryPointIdx>,
    /// Index of the last shooting point in `points`.  `None` when the
    /// sector has no shooting points.
    pub index_last_shooting_point: Option<crate::sector::ArcheryPointIdx>,
    /// Total number of shooting points.
    pub num_shooting_points: u16,
    /// Number of archers currently assigned to this sector.
    pub num_owners: u16,
}

impl SectorArchery {
    pub fn is_full(&self) -> bool {
        self.num_owners >= self.num_shooting_points
    }

    /// Bump the sector-level archer count; asserts the sector isn't
    /// already full (the caller must have checked `!is_full()` before
    /// picking this sector, as `choose_good_shooting_point` does).
    pub fn increment_owner_counter(&mut self) {
        assert!(!self.is_full(), "archery sector is full");
        self.num_owners += 1;
    }

    pub fn decrement_owner_counter(&mut self) {
        assert!(self.num_owners > 0, "archery sector has no owners");
        self.num_owners -= 1;
    }

    /// Point-in-polygon test for the archery sector boundary.
    pub fn is_inside(&self, pos: &Position, layer: u16) -> bool {
        if self.layer != layer {
            return false;
        }
        let (px, py) = (pos.x, pos.y);
        let n = self.polygon.len();
        if n < 3 {
            return false;
        }
        // Ray-casting algorithm
        let mut inside = false;
        let mut j = n - 1;
        for i in 0..n {
            let (xi, yi) = self.polygon[i];
            let (xj, yj) = self.polygon[j];
            if ((yi > py) != (yj > py)) && (px < (xj - xi) * (py - yi) / (yj - yi) + xi) {
                inside = !inside;
            }
            j = i;
        }
        inside
    }
}

// ---------------------------------------------------------------------------
// Repulsive point (scripts add these to repel NPCs from an area)
// ---------------------------------------------------------------------------

/// A point that NPCs try to avoid during pathfinding.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RepulsivePoint {
    pub id: i32,
    pub position: Position,
    /// Inner radius — strong repulsion zone.
    pub radius: f32,
    /// Total outer action radius, including `radius`, matching
    /// Repulsive-point action radius.
    pub action_radius: f32,
    /// Linear falloff coefficients serialized for a repulsive point.
    pub force_a: f32,
    pub force_b: f32,
    /// Serialized action-field geometry. Static script points are total
    /// circles in the Original, but retaining these fields is required for
    /// lossless save adoption.
    pub concave: bool,
    pub limit_left: MapVec,
    pub limit_right: MapVec,
    /// Flags (affects PCs, soldiers, etc.).
    pub flags: i32,
}

impl RepulsivePoint {
    /// Construct the total-circle point produced by the Original's
    /// Fast-grid static repulsive-point insertion.
    pub fn new(
        id: i32,
        position: Position,
        radius: f32,
        action_radius_input: f32,
        flags: i32,
    ) -> Self {
        let (action_radius, radius, force_a, force_b) =
            crate::rhline::repulsive_set_force(radius, action_radius_input);
        Self {
            id,
            position,
            radius,
            action_radius,
            force_a,
            force_b,
            concave: false,
            limit_left: MapVec::ZERO,
            limit_right: MapVec::ZERO,
            flags,
        }
    }
}

// ---------------------------------------------------------------------------
// Door info for seek-area door checks
// ---------------------------------------------------------------------------

/// Minimal door info cached on AiGlobalState for searches behind doors.
/// Populated at level load from the canonical interactable door table.
/// Serialized with `AiGlobalState`; includes cached authorization data that
/// should match the exact door state at the save point.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct DoorSeekInfo {
    /// Index into the canonical interactable door array. Carried so AI
    /// helpers (e.g. `RunAndAlertSoldiers`) can stash a door reference
    /// onto the NPC.
    pub door_index: crate::gate::DoorIndex,
    pub door_type: crate::gate::DoorType,
    pub point_out: MapPoint,
    pub position_in: Position,
    pub sector_out: u16,
    /// Exact arena half of the original game's outside-door sector identity.
    /// Current Rust snapshots must carry this field explicitly; older Rust
    /// layouts are rejected by their outer schema version.
    #[serde(deserialize_with = "Option::deserialize")]
    pub sector_out_index: Option<crate::fast_find_grid::SectorIndex>,
    /// Sector on the inside of the door (the building).
    pub sector_in: u16,
    /// Layer (z-level) on the outside of the door. Used by
    /// running soldier alerts for the layer-mismatch malus in the
    /// weighted-distance scoring.
    pub layer_out: u16,
    /// Cached static door authorization for a non-rider NPC
    /// soldier entering in the direct (outside→inside) direction with
    /// building capacity available. Runtime capacity and rider state are
    /// applied by [`Self::is_npc_villain_authorized_direct`].
    pub npc_villain_authorized_direct: bool,
}

impl DoorSeekInfo {
    /// Complete the cached static authorization with the two live gates from
    /// Door authorization: destination-building capacity and rider
    /// state.
    #[inline]
    pub fn is_npc_villain_authorized_direct(
        &self,
        building_has_capacity: bool,
        actor_is_rider: bool,
    ) -> bool {
        self.npc_villain_authorized_direct && building_has_capacity && !actor_is_rider
    }
}

#[cfg(test)]
mod door_seek_schema_tests;

/// Build the static authorization cached by [`DoorSeekInfo`].
///
/// The search for an enemy behind a door has already narrowed the actor to an NPC
/// soldier and supplies the live capacity/rider gates at use time. Calling
/// the shared door authorization implementation here keeps the remaining
/// building-type, active-state, and villain-lock gates aligned with
/// original-game door authorization.
pub(crate) fn cache_npc_villain_authorized_direct(door: &crate::gate::Door) -> bool {
    let actor = crate::gate::ActorAuthInfo {
        kind: crate::element_kinds::ElementKind::ActorSoldier,
        pc_auth_bit: 0,
        has_lockpick: false,
        has_climb: false,
        has_jump: false,
        is_rider: false,
        posture: crate::element::Posture::Upright,
    };

    door.door_type == crate::gate::DoorType::Building
        && door.is_actor_authorized(true, &actor, true, false)
}

// ---------------------------------------------------------------------------
