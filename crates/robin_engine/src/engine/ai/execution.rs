//! Actor-ID based AI call orchestration. Actor borrows end between admission,
//! handler execution, and completion.

use super::*;

impl EngineInner {
    pub(in crate::engine) fn execute_ai_think(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        owner: EntityId,
        stimulus: &crate::ai::Stimulus,
        ctx: &AiContext,
        enemy_tick: Option<&AiPerTickData>,
        friendly_tick: Option<&crate::ai_friendly::FriendlyPerTickData>,
    ) -> bool {
        // Direct human interactions can address a PC without an AI brain.
        if self
            .world
            .entities
            .get(owner)
            .and_then(Entity::ai_controller)
            .is_none()
        {
            return false;
        }
        let admitted = {
            let entity = self
                .world
                .entities
                .expect_entity_mut(owner, format_args!("Think admission"));
            if let Some(enemy) = entity.enemy_ai_mut() {
                enemy.begin_think(
                    crate::ai_enemy::ThinkEnv::new(
                        sim,
                        ctx,
                        enemy_tick.expect("enemy Think requires tactical data"),
                        Some(&self.world.fast_grid),
                    ),
                    stimulus,
                    &mut self.ai.global,
                )
            } else {
                entity
                    .friendly_ai_mut()
                    .expect("Think owner has no brain")
                    .begin_think(sim, stimulus, &mut self.ai.global, ctx)
            }
        };
        if !admitted {
            return true;
        }

        let handled = {
            let entity = self
                .world
                .entities
                .expect_entity_mut(owner, format_args!("Think handler"));
            if let Some(enemy) = entity.enemy_ai_mut() {
                enemy.think_body(
                    crate::ai_enemy::ThinkEnv::new(
                        sim,
                        ctx,
                        enemy_tick.expect("enemy Think requires tactical data"),
                        Some(&self.world.fast_grid),
                    ),
                    stimulus,
                    &mut self.ai.global,
                )
            } else {
                entity
                    .friendly_ai_mut()
                    .expect("Think owner has no brain")
                    .think_body(
                        sim,
                        stimulus,
                        &mut self.ai.global,
                        ctx,
                        friendly_tick.expect("friendly Think requires tactical data"),
                        Some(&self.world.fast_grid),
                        Some(self.script_domains.interactables.doors.as_slice()),
                    )
            }
        };
        let suspended = stimulus.stimulus_type == StimulusType::EventAfterScriptGoOn
            && self
                .world
                .entities
                .expect_ai_controller(owner, format_args!("Think completion"))
                .outbox
                .reentrant
                .engine_drains_after_script_go_on;
        if !suspended {
            let entity = self
                .world
                .entities
                .expect_entity_mut(owner, format_args!("Think completion"));
            if let Some(enemy) = entity.enemy_ai_mut() {
                enemy.end_think(crate::ai_enemy::ThinkEnv::new(
                    sim,
                    ctx,
                    enemy_tick.expect("enemy Think requires tactical data"),
                    Some(&self.world.fast_grid),
                ));
            } else {
                entity
                    .friendly_ai_mut()
                    .expect("Think owner has no brain")
                    .end_think(sim, ctx);
            }
        }
        if let Some(enemy) = self.world.entities.get(owner).and_then(Entity::enemy_ai) {
            enemy
                .base
                .debug_macro_lifecycle(ctx, "think_return", stimulus.stimulus_type);
        }
        handled
    }
}
