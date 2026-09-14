//! Post-strike bookkeeping retained by live combat callbacks.
use super::EnemyAi;
use crate::ai::*;

impl EnemyAi {
    pub(crate) fn finish_after_combat_injury(&mut self) {
        if self.base.current_substate == Substate::AttackingSwordfight {
            if self.pending_sword_strike_consideration {
                self.pending_combat_insult_after_strike_consideration = true;
            } else {
                self.base.say(Remark::CombatInsult);
            }
        }
    }
}
