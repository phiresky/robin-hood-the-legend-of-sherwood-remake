//! Test entry point into the owner-scoped AI execution context.

use crate::element::EntityId;
use crate::engine::ai::AiOwnerCtx;
use crate::engine::{EngineInner, LevelAssets, TickCtx};
use crate::sim_rng::SimulationContext;

impl EngineInner {
    /// The AI execution context of `owner`, borrowing the caller's `sim` so
    /// its RNG stream keeps advancing across calls.
    pub(in crate::engine) fn ai_ctx<'a>(
        &'a mut self,
        sim: &'a SimulationContext,
        assets: &'a LevelAssets,
        owner: EntityId,
    ) -> AiOwnerCtx<'a> {
        AiOwnerCtx::new(self, TickCtx::new(sim, assets), owner)
    }
}
