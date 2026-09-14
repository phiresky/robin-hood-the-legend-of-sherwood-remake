//! Transient entry points for live officer coordination.

use super::{EnemyAi, ThinkEnv};
use crate::ai::*;

impl EnemyAi {
    pub(crate) fn alert_soldiers(
        &mut self,
        center: Position,
        flags: u16,
        _env: ThinkEnv<'_>,
        failure: AlertSoldiersFailureContinuation,
    ) -> crate::ai::AiFlow<bool> {
        Err(crate::ai::DutyCall {
            flags: crate::ai::DutyFlags::empty(),
            think_result: false,
            tail: crate::ai::DutyTail::AlertSoldiers {
                center,
                flags,
                failure,
            },
            after: Vec::new(),
        })
    }

    pub(crate) fn alert_officer(
        &mut self,
        caller: crate::ai::OfficerAlertCaller,
    ) -> crate::ai::AiFlow<()> {
        Err(crate::ai::DutyCall {
            flags: crate::ai::DutyFlags::empty(),
            think_result: false,
            tail: crate::ai::DutyTail::AlertOfficer { caller },
            after: Vec::new(),
        })
    }

    pub(crate) fn officer_look_for_soldier(&mut self, reason: ReportType) -> crate::ai::AiFlow<()> {
        Err(crate::ai::DutyCall {
            flags: crate::ai::DutyFlags::empty(),
            think_result: false,
            tail: crate::ai::DutyTail::OfficerLookForSoldier { reason },
            after: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests;
