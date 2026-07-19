use super::*;

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
    /// `AISTATE_*` constant emitted by `Script::GetAIState`. The internal
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
/// Returns `Some(code)` for stimuli that `StartThink`'s big switch maps and
/// `None` for types that the original passes to `FilterAIEvent` as `-2`.
/// The mapping covers event codes 0–52.
///
/// Original: `RHArtificialIntelligence::StartThink` in
/// `original-code/RHartificialintelligence.cpp`.
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
        // Original stimuli with no public AI event-code mapping. StartThink's
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

fn pascal_debug_name_to_hyphen_upper<T: std::fmt::Debug>(value: T) -> String {
    let name = format!("{value:?}");
    let mut out = String::with_capacity(name.len() + 8);
    let mut prev: Option<char> = None;
    let mut chars = name.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch.is_uppercase() {
            let split_before = prev.is_some_and(|p| {
                p.is_lowercase()
                    || p.is_ascii_digit()
                    || chars.peek().is_some_and(|next| next.is_lowercase()) && p.is_uppercase()
            });
            if split_before {
                out.push('-');
            }
        } else if ch.is_ascii_digit() && prev.is_some_and(|p| !p.is_ascii_digit()) {
            out.push('-');
        }

        for upper in ch.to_uppercase() {
            out.push(upper);
        }
        prev = Some(ch);
    }

    out
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
)]
#[repr(u32)]
#[allow(non_camel_case_types)] // preserve original naming for clarity
pub enum Substate {
    // -- Sleeping substates --
    StartSleepingSubstates = 0,

    SleepingForever,
    SleepingUnconscious,
    SleepingNapping,
    SleepingAwakening,

    EndSleepingSubstates,

    // -- Default substates --
    StartDefaultSubstates,

    DefaultGotoPost,
    DefaultGotoPostTurn,
    DefaultGotoRoute,
    DefaultGotoRouteTurn,
    DefaultOnPost,
    DefaultOnPostLookingSidewards,
    DefaultEnroute,
    DefaultScriptDriven,
    DefaultInMacro,
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
    WonderingLooking1,
    WonderingLooking1Sidewards,
    WonderingLooking2,
    WonderingLooking2Sidewards,
    WonderingLooking3,
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
    SeekingSeekpointIdentifyingBeggar1,
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
    SeekingBodyAwakeningSleeperr,
    SeekingOfficerLookingForSoldiers1,
    SeekingOfficerLookingForSoldiers1Sidewards,
    SeekingOfficerLookingForSoldiers2,
    SeekingOfficerLookingForSoldiers2Sidewards,
    SeekingOfficerLookingForSoldiers3,
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
    // NOTE: a `SUBSTATE_ATTACKING_SWORDFIGHT_SPECIAL_STRIKE` variant
    // intentionally does NOT exist between `AttackingSwordfight` and
    // `AttackingSwordfightParade`. Such a substate would duplicate
    // information already owned by the sequence manager (the pending
    // strike sequence), leaving two sources of truth that could wedge out
    // of sync when the sequence was interrupted before firing EVENT_DONE.
    // The "in the middle of a special strike" condition is derived from
    // `EnemyAi::pending_special_strike`, which is tied to the sequence's
    // lifetime via per-tick reconciliation in
    // `engine/melee.rs::tick_enemy_sword_attacks`. The NPC stays in
    // `AttackingSwordfight` for the whole strike.
    AttackingSwordfight,
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
    AttackingArcherWaitOnArcheryPath,
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
    DefaultGotoChief,
    DefaultPatrolChiefReturnToPatrol,
    WonderingApproachingBrawlVictim,
    WonderingAwakenBrawlVictim,
    WonderingOfficerFinishingBrawlWaiting,
    AttackingReturnToOtherPcAfterMenacing,
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
    /// The original `SetState` asserts these numeric family boundaries in
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

    pub fn log_string_from_u16(raw: u16) -> String {
        Self::try_from(u32::from(raw))
            .ok()
            .and_then(Self::log_string)
            .unwrap_or_else(|| "SUBSTATE-???".to_string())
    }

    pub fn log_string(self) -> Option<String> {
        use Substate::*;

        let text = match self {
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
            | None => return std::option::Option::None,

            DefaultGotoPost => "SUBSTATE-DEFAULT-GOTOPOST".to_string(),
            DefaultGotoPostTurn => "SUBSTATE-DEFAULT-GOTOPOST-TURN".to_string(),
            DefaultGotoRoute => "SUBSTATE-DEFAULT-GOTOROUTE".to_string(),
            DefaultGotoRouteTurn => "SUBSTATE-DEFAULT-GOTOROUTE-TURN".to_string(),
            DefaultGotoChief => "SUBSTATE-DEFAULT-GOTOCHIEF".to_string(),
            DefaultOnPost => "SUBSTATE-DEFAULT-ONPOST".to_string(),
            DefaultOnPostLookingSidewards => {
                "SUBSTATE-DEFAULT-ONPOST-LOOKING-SIDEWARDS".to_string()
            }
            DefaultInMacro => "SUBSTATE-DEFAULT-INMACRO".to_string(),
            DefaultInMacroWaitingForDone => "SUBSTATE-DEFAULT-INMACRO-WAITING-FOR-DONE".to_string(),
            WonderingBrawlGotHit => "SUBSTATE-WONDERING-BRAWL-GOTHIT".to_string(),
            SeekingBodyAwakeningSleeperr => "SUBSTATE-SEEKING-BODY-AWAKENING-SLEEPER".to_string(),
            AttackingSwordfight => "SUBSTATE-ATTACKING-SWORDFIGHT".to_string(),
            AttackingSwordfightParade => "SUBSTATE-ATTACKING-SWORDFIGHT-PARADE".to_string(),
            AttackingQuittingSwordfight => "SUBSTATE-ATTACKING-QUITTING-SWORDFIGHT".to_string(),
            AttackingSwordfightStepBack => "SUBSTATE-ATTACKING-SWORDFIGHT-STEP-BACK".to_string(),
            AttackingArcherWaitOnArcheryPath => {
                "SUBSTATE-ATTACKING-ARCHER-WAIT-ON-ACHERY-PATH".to_string()
            }
            AttackingArcherWaitOnArcheryPathBending => {
                "SUBSTATE-ATTACKING-ARCHER-WAIT-ON-ACHERY-PATH-BENDING".to_string()
            }
            other => format!("SUBSTATE-{}", pascal_debug_name_to_hyphen_upper(other)),
        };

        Some(text)
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
)]
#[repr(u32)]
pub enum StimulusType {
    // -- Perception events --
    EventView = 0,
    EventOutOfView,
    EventHear,
    EventReachPoint,
    EventCouldntReachPoint,
    EventDone,
    EventImpossible,
    EventTimer,
    EventPcShotAtMe,
    EventSeesBody,
    EventSeesObject,
    EventSeesSoldier,
    EventSeesFriendInTrouble,
    EventFitAgain,
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
    EventMyTalk1,
    EventMyTalk2,
    EventMyTalk3,
    CallYourTalk1,
    CallYourTalk2,
    CallYourTalk3,
    EventGoodStrike,
    EventLethalStrike,
    EventEnemyNear,
    EventMyTalk0,
    CallYourTalk0,
    EventStop,
    NoEvent,
    /// Script-triggered: force AI to run battle_decisions(sim, ) immediately.
    ForceBattleDecision,
}

impl StimulusType {
    pub fn log_string_from_u16(raw: u16) -> &'static str {
        Self::try_from(u32::from(raw))
            .ok()
            .and_then(Self::log_string)
            .unwrap_or("EVENT-???")
    }

    pub fn log_string(self) -> Option<&'static str> {
        Some(match self {
            StimulusType::EventView => "EVENT-VIEW",
            StimulusType::EventOutOfView => "EVENT-OUTOFVIEW",
            StimulusType::EventHear => "EVENT-HEAR",
            StimulusType::EventReachPoint => "EVENT-REACHPOINT",
            StimulusType::EventCouldntReachPoint => "EVENT-COULDNT-REACHPOINT",
            StimulusType::EventDone => "EVENT-DONE",
            StimulusType::EventImpossible => "EVENT-IMPOSSIBLE",
            StimulusType::EventTimer => "EVENT-TIMER",
            StimulusType::EventPcShotAtMe => "EVENT-PC-SHOT-AT-ME",
            StimulusType::EventSeesBody => "EVENT-SEESBODY",
            StimulusType::EventSeesObject => "EVENT-SEESOBJECT",
            StimulusType::EventSeesSoldier => "EVENT-SEES-SOLDIER",
            StimulusType::EventSeesFriendInTrouble => "EVENT-SEESFRIENDINTROUBLE",
            StimulusType::EventFitAgain => "EVENT-FITAGAIN",
            StimulusType::EventGotHit => "EVENT-GOTHIT",
            StimulusType::EventLoseConsciousness => "EVENT-LOSE-CONSCIOUSNESS",
            StimulusType::EventMissesCharly => "EVENT-MISSES-CHARLY",
            StimulusType::EventObjectAway => "EVENT-OBJECT-AWAY",
            StimulusType::EventSeesCharly => "EVENT-SEES-CHARLY",
            StimulusType::EventSyncCharly => "EVENT-SYNC-CHARLY",
            StimulusType::EventAfterScriptGoOn => "EVENT-AFTER-SCRIPT-GO-ON",
            StimulusType::EventReturnToDuty => "EVENT-RETURN-TO-DUTY",
            StimulusType::EventPanic => "EVENT-PANIC",
            StimulusType::EventEnterSwordfight => "EVENT-ENTER-SWORDFIGHT",
            StimulusType::EventQuitSwordfight => "EVENT-QUIT-SWORDFIGHT",
            StimulusType::EventSwordStrike => "EVENT-SWORDSTRIKE",
            StimulusType::EventWasp => "EVENT-WASP",
            StimulusType::EventWaspAway => "EVENT-WASP-AWAY",
            StimulusType::EventApple => "EVENT-APPLE",
            StimulusType::EventNet => "EVENT-NET",
            StimulusType::EventNetAway => "EVENT-NET-AWAY",
            StimulusType::EventSeesBeggar => "EVENT-SEES-BEGGAR",
            StimulusType::EventGetArrow => "EVENT-GET-ARROW",
            StimulusType::EventSeesBrawl => "EVENT-SEES-BRAWL",
            StimulusType::CallAlert => "CALL-ALERT",
            StimulusType::CallCombatAlert => "CALL-COMBAT-ALERT",
            StimulusType::CallHey => "CALL-HEY",
            StimulusType::CallHint => "CALL-HINT",
            StimulusType::CallInstruction => "CALL-INSTRUCTION",
            StimulusType::CallLookThere => "CALL-LOOKTHERE",
            StimulusType::CallCoordinate => "CALL-COORDINATE",
            StimulusType::CallReport => "CALL-REPORT",
            StimulusType::CallGoToOfficer => "CALL-GO-TO-OFFICER",
            StimulusType::CallMrOfficerIAmBack => "CALL-MR-OFFICER-I-AM-BACK",
            StimulusType::CallCharlyIsBack => "CALL-CHARLY-IS-BACK",
            StimulusType::CallPatrolCoordinate => "CALL-PATROL-COORDINATE",
            StimulusType::CallTowerGuardAlert => "CALL-TOWER-GUARD-ALERT",
            StimulusType::CallTowerGuardCallsMe => "CALL-TOWER-GUARD-CALLS-ME",
            StimulusType::CallFinishBrawl => "CALL-FINISH-BRAWL",
            StimulusType::CallYouJustWait => "CALL-YOU-JUST-WAIT",
            StimulusType::EventAppleChaseNear => "EVENT-APPLE-CHASE-NEAR",
            StimulusType::EventDoorCombat => "EVENT-DOOR-COMBAT",
            StimulusType::EventGaloppLoopEnd => "EVENT-GALOPP-LOOP-END",
            StimulusType::EventSeesShadow => "EVENT-SEES-SHADOW",
            StimulusType::EventArrowLaunched => "EVENT-ARROW-LAUNCHED",
            StimulusType::EventStone => "EVENT-STONE",
            StimulusType::EventAdversaryWeak => "EVENT-ADVERSARY-WEAK",
            StimulusType::EventAfterCombatInjury => "EVENT-AFTER-COMBAT-INJURY",
            StimulusType::CallCleanUpAfterBrawl => "CALL-CLEAN-UP-AFTER-BRAWL",
            StimulusType::EventMyTalk1 => "EVENT-MYTALK-1",
            StimulusType::EventMyTalk2 => "EVENT-MYTALK-2",
            StimulusType::EventMyTalk3 => "EVENT-MYTALK-3",
            StimulusType::CallYourTalk1 => "CALL-YOURTALK-1",
            StimulusType::CallYourTalk2 => "CALL-YOURTALK-2",
            StimulusType::CallYourTalk3 => "CALL-YOURTALK-3",
            StimulusType::EventGoodStrike => "EVENT-GOOD-STRIKE",
            StimulusType::EventLethalStrike => "EVENT-LETHAL-STRIKE",
            StimulusType::EventEnemyNear => "EVENT-ENEMY-NEAR",
            StimulusType::EventMyTalk0 => "EVENT-MYTALK-0",
            StimulusType::CallYourTalk0 => "CALL-YOURTALK-0",
            StimulusType::EventStop => "EVENT-STOP",
            StimulusType::NoEvent | StimulusType::ForceBattleDecision => return None,
        })
    }
}

/// Classification of stimulus types into processing categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StimulusCategory {
    /// Expected events (timer, reachpoint, etc.) — drive state progression.
    Expected,
    /// Unexpected events — interruptions that may change behavior.
    Unexpected,
    /// Alerting events — high-priority perception events.
    Alerting,
    /// Return to duty — special handling.
    ReturnToDuty,
    /// Ignored by this AI type.
    Ignored,
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
    robin_state_hash_derive::StateHash,
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
)]
#[repr(u32)]
pub enum Decision {
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
    LookForHelp,
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

    pub fn log_string(self) -> Option<&'static str> {
        Some(match self {
            Decision::None | Decision::PredecisionOffensive | Decision::PredecisionDefensive => {
                return None;
            }
            Decision::Cassos => "DECISION-CASSOS",
            Decision::Fight => "DECISION-FIGHT",
            Decision::Observe => "DECISION-OBSERVE",
            Decision::Reserve => "DECISION-RESERVE",
            Decision::AlertSoldiers => "DECISION-ALERT-SOLDIERS",
            Decision::RunAndAlertSoldiers => "DECISION-RUN-AND-ALERT-SOLDIERS",
            Decision::Menace => "DECISION-MENACE",
            Decision::Shoot => "DECISION-SHOOT",
            Decision::ArcherStepBack => "DECISION-ARCHER-STEP-BACK",
            Decision::LookForHelp => "DECISION-LOOK-4-HELP",
            Decision::LookForHelpIfNobodyElseDoes => "DECISION-LOOK-4-HELP-IF-NOBODY-ELSE-DOES",
            Decision::CoverBehindShieldBearer => "DECISION-COVER-BEHIND-SHIELD-BEARER",
            Decision::TooProudToAttack => "DECISION-TOO-PROUD-TO-ATTACK",
            Decision::TowerGuardAlert => "DECISION-TOWER-GUARD-ALERT",
            Decision::TowerGuardObserve => "DECISION-TOWER-GUARD-OBSERVE",
            Decision::ArcherObserve => "DECISION-ARCHER-OBSERVE",
            Decision::RunToArcheryPoint => "DECISION-RUN-TO-ARCHERY-POINT",
            Decision::RunForNewArrows => "DECISION-RUN-FOR-NEW-ARROWS",
            Decision::LastReserve => "DECISION-LAST-RESERVE",
        })
    }
}

// ---------------------------------------------------------------------------
// Cross-NPC actions (phalanx coordination, stimulus forwarding)
// ---------------------------------------------------------------------------

/// Actions that one NPC's AI emits to affect another NPC. The engine
/// drains these after each think() and applies them to the targets.
/// Used for patterns like calling `InstructGatherPosition` then
/// delivering `CALL_INSTRUCTION`, and recursive `BreakPhalanx`.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub enum CrossNpcAction {
    /// Set gather position on target NPC, then deliver `CALL_INSTRUCTION`.
    InstructGatherPosition {
        target: NpcHandle,
        position: Position,
        direction: u16,
    },
    /// Propagate break-phalanx to target: clear their combat neighbours,
    /// set `phalanx_aborted = true`, and trigger `BattleDecisions`.
    BreakPhalanx { target: NpcHandle },
    /// Deliver a stimulus to the target NPC (e.g. `CALL_COORDINATE`).
    SendStimulus {
        target: NpcHandle,
        stimulus_type: StimulusType,
        /// Optional payload (position, human handle, etc.).  Defaults to
        /// `StimulusInfo::None` for stimuli that carry no data.
        info: StimulusInfo,
        /// When set, if the target's `think()` returns `false` (stimulus
        /// not handled), redeliver the stimulus to this NPC instead. Used
        /// in conversation chains to fall back to the original sender when
        /// the receiver doesn't handle the call.
        fallback_to_sender: Option<NpcHandle>,
        /// Propagated `Stimulus::to_whole_patrol` flag — set when a patrol
        /// chief broadcasts a stimulus to subordinates. Receivers must
        /// restore this flag when rebuilding the `Stimulus`, otherwise
        /// `dispatch_stimulus_to_whole_patrol` fails to early-exit on the
        /// member side and re-delegates back to the chief, producing an
        /// unbounded chief↔member ping-pong loop.
        to_whole_patrol: bool,
    },
    /// Set the target NPC's left combat neighbour link (one-way).
    /// Bare setter, no reciprocal cleanup. Use
    /// [`Self::UpdateLeftCombatNeighbour`] for the full semantics
    /// (reciprocal cleanup).
    SetLeftCombatNeighbour {
        target: NpcHandle,
        neighbour: HumanHandle,
    },
    /// Set the target NPC's right combat neighbour link (one-way).
    SetRightCombatNeighbour {
        target: NpcHandle,
        neighbour: HumanHandle,
    },
    /// Full reciprocal update of `target`'s left combat neighbour. Four steps:
    ///   1. Clear `old_left`'s right pointer (if non-zero).
    ///   2. Store `new_left` on `target`'s left pointer.
    ///   3. Pre-clean `new_left`'s existing right (and that-right's left).
    ///   4. Wire `new_left`'s right pointer back to `target`.
    ///
    /// `old_left` is captured at push time so the drain doesn't depend on
    /// `target`'s current state being unmodified.
    UpdateLeftCombatNeighbour {
        target: NpcHandle,
        old_left: HumanHandle,
        new_left: HumanHandle,
    },
    /// Mirror of [`Self::UpdateLeftCombatNeighbour`] for the right side.
    UpdateRightCombatNeighbour {
        target: NpcHandle,
        old_right: HumanHandle,
        new_right: HumanHandle,
    },
    /// Propagate primary target to a phalanx member during
    /// `ReconsiderPhalanx`'s phalanx-member walk.
    SetPrimaryTarget {
        target: NpcHandle,
        primary_target: HumanHandle,
    },
    /// Make the target NPC say a remark.
    Say { target: NpcHandle, remark: Remark },
    /// Write `AiBase::looted_after_money_fight` on a target soldier.
    /// Money-fight looters set this as soon as they reserve a KO'd victim
    /// so other scanners skip the same body.
    SetLootedAfterMoneyFight { target: NpcHandle, looted: bool },
    /// Update the target NPC's reconnaissance report type and seek position
    /// — shares the officer's report back to the soldier after
    /// `GetReportFromSoldier`.
    UpdateReport {
        target: NpcHandle,
        report_type: ReportType,
        seek_position: Position,
    },
    /// Merge the officer's reconnaissance report into the target soldier's
    /// report. Broadcast inside `AlertSoldiers` so newly alerted soldiers
    /// pick up the officer's charly handle and report type before they run
    /// into the group.
    ConsiderReport {
        target: NpcHandle,
        /// Cloned from the caller's own `ReconnaissanceReport` at the
        /// time the alert was dispatched.
        report: ReconnaissanceReport,
        /// Merge-mask passed to [`ReconnaissanceReport::consider_report`]
        /// (e.g. `UPDATE_CHARLY | UPDATE_TYPE = 2|4 = 6`).
        flags: u16,
    },
    /// Push `actor` onto `target`'s `synchronizing_actors` list. Used by
    /// `EventSeesCharlyStandardProcedure` when the reuniting soldier
    /// still needs to wait at the sync waypoint for its macro friend.
    RegisterSynchronizingActor { target: NpcHandle, actor: NpcHandle },
    /// Synchronously deliver `CALL_MR_OFFICER_I_AM_BACK` and feed the
    /// target officer's actual `Think` return value back into Charly's
    /// state machine before the originating dispatch completes.
    ReportBackToOfficer {
        officer: NpcHandle,
        charly: NpcHandle,
    },
}

// ---------------------------------------------------------------------------
// Panic request (queued by AI, applied by engine)
// ---------------------------------------------------------------------------

/// Queued `Panic()` request on an [`AiController`].
///
/// The AI layer sets this field when a fleeing stimulus kicks in; the
/// engine consumes it at post-think time and performs the door lookup
/// against `ai_global.door_seek_infos` (which the AI layer doesn't
/// see on its call stack).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct PanicRequest {
    /// Point to flee *away from*.  `None` means undirected panic — the
    /// engine picks any reachable door and runs in random directions.
    pub center: Option<Position>,
    /// Number of run segments the NPC should execute after the initial
    /// door fallback fails.
    pub runs: u8,
    /// Alert level the drain should install on state entry (default
    /// `ALERT_RED`).
    pub alert: AlertLevel,
    /// `true` when the caller was not already in `FleeingPanic` /
    /// `FleeingRunToDoor` at the time the request was queued. Lets the
    /// drain suppress repeated state changes / Say() / `EventReachPoint`
    /// dispatches when we're already mid-panic.
    pub is_new_panic: bool,
}

/// Pending request for a script-driven `SeekArea` entry, set from
/// `SetAIState(actor, STATE_SEEKING)` script natives. The engine
/// consumes it post-think by dispatching into `EnemyAi::seek_area`
/// (soldier-only).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct ScriptSeekAreaRequest {
    /// Seek center — typically the NPC's current position.
    pub center: Position,
    /// Radius passed to `SeekArea` (`AI_SCRIPT_SEEK_RADIUS`).
    pub radius: u16,
}

/// Variants of `AssignNewPatrolPath` — the three call shapes (sentinel
/// `-1`, sentinel `-2`, valid index) collapse to these semantic cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatrolAssignment {
    /// Sentinel `-1` / null pointer — drop the path, leave
    /// `likes_to_sit_around = false`.
    ClearPath,
    /// Sentinel `-2` / `(void*)-1` — drop the path but set
    /// `likes_to_sit_around = true`.
    ClearPathSitAround,
    /// Valid-index branch.
    Index(PathId),
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
#[derive(Debug, Clone, Copy, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
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
}

/// A noise event with origin, type, volume, and elevation.
#[derive(
    Debug, Clone, Copy, PartialEq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct Noise {
    pub origin: Position,
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
    // NumberOfCuriosities — use Curiosity::COUNT
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
)]
#[repr(u32)]
pub enum TargetType {
    Pc = 0,
    Npc,
    Scarecrow,
}

/// Report type for reconnaissance reports.
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
)]
#[repr(u32)]
pub enum ReportType {
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
    Debug, Clone, Copy, PartialEq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct Hint {
    pub seek_point: Position,
    pub seek_flags: u16,
    pub who_tells_me: NpcHandle,
}

/// Info about a stolen object.
#[derive(
    Debug, Clone, Copy, PartialEq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct StolenObject {
    pub object: ObjectHandle,
    pub thief: NpcHandle,
}

/// Info about a friend in trouble.
#[derive(
    Debug, Clone, Copy, PartialEq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct CombatInfo {
    pub actor_npc: NpcHandle,
    pub enemy_position: Position,
}

/// Info about a door combat event.
#[derive(
    Debug, Clone, Copy, PartialEq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct DoorCombatInfo {
    pub delay: u16,
    pub goal: Position,
    pub direction: u16,
    pub adversary: HumanHandle,
}

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
)]
pub enum StimulusInfo {
    #[default]
    None,
    Noise(Noise),
    Position(Position),
    Human(HumanHandle),
    Hint(Hint),
    Object(ObjectHandle),
    Stolen(StolenObject),
    Combat(CombatInfo),
    DoorCombat(DoorCombatInfo),
    Index(u16),
}

// ---------------------------------------------------------------------------
// Stimulus
// ---------------------------------------------------------------------------

/// An event or call that is dispatched to an NPC's AI for processing.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct Stimulus {
    pub stimulus_type: StimulusType,
    pub info: StimulusInfo,
    pub owner: NpcHandle,
    pub to_whole_patrol: bool,
}

impl Stimulus {
    pub fn new(stimulus_type: StimulusType) -> Self {
        Self {
            stimulus_type,
            info: StimulusInfo::None,
            owner: 0,
            to_whole_patrol: false,
        }
    }

    pub fn with_noise(stimulus_type: StimulusType, noise: Noise) -> Self {
        Self {
            stimulus_type,
            info: StimulusInfo::Noise(noise),
            owner: 0,
            to_whole_patrol: false,
        }
    }

    pub fn with_position(stimulus_type: StimulusType, pos: Position) -> Self {
        Self {
            stimulus_type,
            info: StimulusInfo::Position(pos),
            owner: 0,
            to_whole_patrol: false,
        }
    }

    pub fn with_human(stimulus_type: StimulusType, human: HumanHandle) -> Self {
        Self {
            stimulus_type,
            info: StimulusInfo::Human(human),
            owner: 0,
            to_whole_patrol: false,
        }
    }

    pub fn with_door_combat(stimulus_type: StimulusType, dc: DoorCombatInfo) -> Self {
        Self {
            stimulus_type,
            info: StimulusInfo::DoorCombat(dc),
            owner: 0,
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
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Screen remark (HUD display)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct ScreenRemark {
    pub timer: u16,
    pub prefix: String,
    pub remark: Remark,
}

/// A forbidden remark entry — prevents the same line from being repeated.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
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

#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct ReconnaissanceReport {
    pub seek_position: Position,
    pub report_type: ReportType,
    pub seen_bodies: Vec<HumanHandle>,
    pub charly: NpcHandle,
    pub charly_seen: bool,
}

impl Default for ReconnaissanceReport {
    fn default() -> Self {
        Self {
            seek_position: Position::default(),
            report_type: ReportType::Nothing,
            seen_bodies: Vec::new(),
            charly: 0,
            charly_seen: false,
        }
    }
}

impl ReconnaissanceReport {
    pub fn reset(&mut self) {
        self.seen_bodies.clear();
        self.report_type = ReportType::Nothing;
        self.charly = 0;
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
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
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
            // Unconditionally append the incoming direction — duplicates
            // are intentional, they bias the seek sweep toward
            // repeatedly-hinted compass directions.
            self.directions.push(dir.direction);
            true
        } else {
            false
        }
    }
}

/// A seek-point direction from the level file (position + facing).
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SeekPointDirection {
    pub position: Position,
    pub direction: u16,
}

// ---------------------------------------------------------------------------
// Ambush point
// ---------------------------------------------------------------------------

/// A tactical ambush point that NPCs check while patrolling.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct AmbushPoint {
    pub position: Position,
    pub direction: u16,
    /// 3D anchor point — the 2D `position` lifted to eye height (z + 32).
    /// Used by the sight-polygon anchor for stealth / hide-in-ambush
    /// queries.
    pub position_3d: crate::coordinates::WorldPoint3D,
    /// Unique ambush-point ID assigned at `InitAI()` time. Used by AI
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
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
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
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
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
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct RepulsivePoint {
    pub id: i32,
    pub position: Position,
    /// Inner radius — strong repulsion zone.
    pub radius: f32,
    /// Outer radius — weaker repulsion zone.
    pub action_radius: f32,
    /// Flags (affects PCs, soldiers, etc.).
    pub flags: i32,
}

// ---------------------------------------------------------------------------
// Door info for seek-area door checks
// ---------------------------------------------------------------------------

/// Minimal door info cached on AiGlobalState for `FindDoorEnemyCouldBeBehind`.
/// Populated at level load from the canonical interactable door table.
/// Serialized with `AiGlobalState`; includes cached authorization data that
/// should match the exact door state at the save point.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct DoorSeekInfo {
    /// Index into the canonical interactable door array. Carried so AI
    /// helpers (e.g. `RunAndAlertSoldiers`) can stash a door reference
    /// onto the NPC.
    pub door_index: crate::gate::DoorIndex,
    pub door_type: crate::gate::DoorType,
    pub point_out: MapPoint,
    pub position_in: Position,
    pub sector_out: u16,
    /// Sector on the inside of the door (the building).
    pub sector_in: u16,
    /// Layer (z-level) on the outside of the door. Used by
    /// `RunAndAlertSoldiers` for the layer-mismatch malus in the
    /// weighted-distance scoring.
    pub layer_out: u16,
    /// Cached static portion of `IsActorAutorized` for a non-rider NPC
    /// soldier entering in the direct (outside→inside) direction with
    /// building capacity available. Runtime capacity and rider state are
    /// applied by [`Self::is_npc_villain_authorized_direct`].
    pub npc_villain_authorized_direct: bool,
}

impl DoorSeekInfo {
    /// Complete the cached static authorization with the two live gates from
    /// `RHDoor::IsActorAutorized`: destination-building capacity and rider
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

/// Build the static authorization cached by [`DoorSeekInfo`].
///
/// `FindDoorEnemyCouldBeBehind` has already narrowed the actor to an NPC
/// soldier and supplies the live capacity/rider gates at use time. Calling
/// the shared door authorization implementation here keeps the remaining
/// building-type, active-state, and villain-lock gates aligned with
/// `RHDoor::IsActorAutorized(true, mpMe, false)`.
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
