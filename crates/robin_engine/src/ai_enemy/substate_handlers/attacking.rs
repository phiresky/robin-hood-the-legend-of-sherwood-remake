//! Combat remark continuations.
use super::*;

impl EnemyAi {
    pub(crate) fn swordfight_insult_after_reconsider(&mut self) {
        if self.base.current_substate == Substate::AttackingSwordfight {
            if self.pending_sword_strike_consideration {
                self.pending_combat_insult_after_strike_consideration = true;
            } else {
                self.base.say(Remark::CombatInsult);
            }
        }
    }
}
