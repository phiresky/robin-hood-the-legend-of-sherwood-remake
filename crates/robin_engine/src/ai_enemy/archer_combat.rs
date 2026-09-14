//! Bow launch adapter for the remaining borrowed beggar callback.

use super::EnemyAi;
use crate::ai::*;

impl EnemyAi {
    pub fn shoot_arrow_at(&mut self, enemy: HumanHandle, ctx: &AiContext) {
        // Asserts: is_archer() && remaining_arrows > 0.
        debug_assert!(self.is_archer_unit, "shoot_arrow_at called on non-archer");
        debug_assert!(
            ctx.remaining_arrows > 0,
            "shoot_arrow_at called with 0 arrows"
        );

        self.base.stop_all();

        // Set pending flag — the engine drains this after think() and
        // calls EngineInner::shoot_bow_at to launch the sequence element.
        self.base.outbox.actor.shoot_target = Some(AiEntityHandle::new(enemy));
    }
}
