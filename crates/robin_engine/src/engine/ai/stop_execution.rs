use super::*;
use crate::ai::Substate;
use crate::sim_rng::SimulationContext;

impl EngineInner {
    /// Finish preceding operations, clear checkpoint bookkeeping, then halt.
    /// Macro interruption uses the substate left by the halt's callbacks.
    pub(in crate::engine) fn stop_ai_owner(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let substate = self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("stop owner checkpoint"))
            .current_substate;
        if matches!(
            substate,
            Substate::DefaultLookingForCharly | Substate::DefaultLookingSidewardsForCharly
        ) {
            self.execute_ai_set_checkpoint_charly(owner, None);
        }

        self.halt_actor(sim, assets, owner);

        let ai = self
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("stop owner after halt"));
        if !matches!(
            ai.current_substate,
            Substate::DefaultLookingForCharly
                | Substate::DefaultLookingSidewardsForCharly
                | Substate::SeekingGroupGetInstructedByOfficer
        ) {
            self.execute_ai_break_macro(owner);
        }
    }
}
