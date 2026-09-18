use super::*;
use crate::ai::Substate;
use crate::engine::TickCtx;

impl EngineInner {
    pub(in crate::engine) fn stop_ai_owner(&mut self, tcx: TickCtx<'_>, owner: EntityId) {
        AiOwnerCtx::new(self, tcx, owner).stop_ai_owner()
    }
}

impl AiOwnerCtx<'_> {
    /// Finish preceding operations, clear checkpoint bookkeeping, then halt.
    /// Macro interruption uses the substate left by the halt's callbacks.
    pub(in crate::engine) fn stop_ai_owner(&mut self) {
        let substate = self
            .engine
            .ai(self.owner, "stop owner checkpoint")
            .current_substate;
        if matches!(
            substate,
            Substate::DefaultLookingForCharly | Substate::DefaultLookingSidewardsForCharly
        ) {
            self.engine
                .execute_ai_set_checkpoint_charly(self.owner, None);
        }

        self.engine
            .halt_actor(TickCtx::new(self.sim, self.assets), self.owner);

        let ai = self.engine.ai_mut(self.owner, "stop owner after halt");
        if !matches!(
            ai.current_substate,
            Substate::DefaultLookingForCharly
                | Substate::DefaultLookingSidewardsForCharly
                | Substate::SeekingGroupGetInstructedByOfficer
        ) {
            self.engine.execute_ai_break_macro(self.owner);
        }
    }
}
