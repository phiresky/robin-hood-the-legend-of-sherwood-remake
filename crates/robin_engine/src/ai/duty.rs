//! Typed arguments for live enemy decision operations.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum EnemyRecovery {
    FitAgain,
    WaspAway,
    NetAway,
    Stop,
    Apple { position: super::Position },
    Stone { position: super::Position },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum EnemyObservation {
    Noise { noise: super::Noise },
    LookThere { position: super::Position },
    TowerGuardAlert { hint: super::Hint },
    TowerGuardCalls { hint: super::Hint },
    CombatAlert { position: super::Position },
    ArcherEnemy { target: super::HumanHandle },
    Enemy { target: super::HumanHandle },
    Charly { target: super::HumanHandle },
    Shadow { position: super::Position },
    Object { target: super::ObjectHandle },
    Arrow { origin: super::Position },
    ArrowReaction,
    AleReaction,
    AleApproach { arrived: bool },
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
