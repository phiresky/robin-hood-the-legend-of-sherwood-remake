//! Borrowed decision handlers return engine calls before decision completion.

use serde::{Deserialize, Serialize};

use super::DutyFlags;

pub(crate) type AiFlow<T> = Result<T, DutyCall>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DutyCall {
    pub flags: DutyFlags,
    /// Boolean result of the enclosing Think after the engine call completes.
    pub think_result: bool,
    pub tail: DutyTail,
    /// Enclosing caller statements, executed after the innermost tail returns.
    pub after: Vec<DutyTail>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum DutyTail {
    None,
    PanicSegment {
        stimulus: super::StimulusType,
    },
    BodyReaction {
        operation: BodyReaction,
    },
    AlertOfficer {
        caller: OfficerAlertCaller,
    },
    TowerGuardAlert {
        center: super::Position,
    },
    OfficerLookForSoldier {
        reason: super::ReportType,
    },
    BattleDecisions,
    BattleOverview {
        flags: u16,
    },
    ReconsiderEnemyApproach {
        reachpoint: bool,
    },
    AttackEnemy {
        target: super::HumanHandle,
    },
    RiderAttack {
        fallback: RiderAttackFallback,
    },
    ReconsiderSwordfight {
        enemy_weak: bool,
    },
    ReconsiderSwordfightObservation,
    CommandSoldiersToAttack {
        center: super::Position,
    },
    AlertSoldiers {
        center: super::Position,
        flags: u16,
        failure: super::AlertSoldiersFailureContinuation,
    },
    MoneyFight {
        operation: MoneyFightOperation,
    },
    FinishSeek,
    SeekArea {
        center: super::Position,
        standard_radius: u16,
        flags: crate::ai_enemy::SeekFlags,
        seek_direction: u16,
    },
    SeekNextPoint,
    ScanSleepingEnemies {
        observer_camp: crate::element::Camp,
    },
    ApproachSleepingEnemies {
        targets: Vec<super::HumanHandle>,
    },
    SearchCharlyTimer,
    TooProudOverviewRemark,
    AfterCombatInjury,
    GotHitViewStatus,
    DispatchPatrol {
        stimulus: super::Stimulus,
    },
    Think {
        stimulus: crate::ai::Stimulus,
    },
    SwordfightInsult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum BodyReaction {
    Seen { body: super::HumanHandle },
    ReactionTimer,
    Arrival,
    BodyTimer,
    DeadBodyTimer,
    SleeperTimer,
    NetDone,
    Unreachable,
    Examine { body: super::HumanHandle },
    DeadBodyAlert { center: super::Position },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum RiderAttackFallback {
    Approach,
    BattleDecisions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum OfficerAlertCaller {
    Ignore,
    ReturnToDuty,
    SeekBody {
        center: super::Position,
        radius: u16,
    },
    SeekMissedCharly {
        center: super::Position,
    },
    TowerGuardCalled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum MoneyFightOperation {
    FinishBrawl,
    CleanUpAfterBrawl,
    CollectOrLootAfterLook,
    StolenMoney {
        object: super::AiEntityHandle,
        thief: super::AiEntityHandle,
    },
    FinishHitAfterOfficer,
    RecoverBrawl,
    AwakeNextVictim,
}

impl DutyCall {
    pub(crate) fn new(flags: DutyFlags, think_result: bool) -> Self {
        Self {
            flags,
            think_result,
            tail: DutyTail::None,
            after: Vec::new(),
        }
    }

    pub(crate) fn with_think_result(mut self, result: bool) -> Self {
        self.think_result = result;
        self
    }

    pub(crate) fn then(mut self, tail: DutyTail) -> Self {
        self.after.push(tail);
        self
    }
}
