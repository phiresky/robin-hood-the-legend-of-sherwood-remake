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
    FinishSeek,
    ScanSleepingEnemies {
        observer_camp: crate::element::Camp,
    },
    ApproachSleepingEnemies {
        targets: Vec<super::HumanHandle>,
    },
    SearchCharlyTimer,
    TooProudOverviewRemark,
    FinalizeAlertSoldiers {
        restore_check_timer: bool,
    },
    AfterCombatInjury,
    BroadcastPatrol {
        stimulus: crate::ai::Stimulus,
        members: Vec<u32>,
    },
    Think {
        stimulus: crate::ai::Stimulus,
    },
    SwordfightInsult,
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
