//! Actor-ID based AI call orchestration. Actor borrows end between admission,
//! handler execution, and completion.

use super::*;

impl EngineInner {
    pub(in crate::engine) fn execute_ai_return_to_duty(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        flags: crate::ai::DutyFlags,
    ) {
        self.drain_direct_ai_owner_prefix_boundary(sim, owner, assets);
        self.virtual_return_to_duty_for_npc(sim, owner, assets, flags);
        self.drain_direct_ai_owner_prefix_boundary(sim, owner, assets);
    }

    /// Execute a nested actor decision to completion before its caller resumes.
    pub(in crate::engine) fn execute_ai_callback(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &crate::ai::Stimulus,
    ) -> bool {
        let scratch = self.build_sim_scratch(assets);
        let mut ctx = self.ai_context_for(owner, self.control.frame_counter, &scratch, assets);
        ctx.in_uninterruptible_command = self.is_very_very_busy(owner);
        let tick = self
            .world
            .entities
            .expect_entity(owner, format_args!("nested Think owner"))
            .enemy_ai()
            .is_some()
            .then(|| self.build_npc_tick_data(sim, owner, assets));
        self.dispatch_think_with_drain(sim, owner, stimulus, &ctx, tick.as_ref(), assets)
    }

    pub(in crate::engine) fn execute_ai_think(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
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

        let handled = if let Some(handled) =
            self.execute_friendly_callback(sim, assets, owner, stimulus, ctx)
        {
            handled
        } else if let Some(handled) =
            self.execute_enemy_report_callback(sim, assets, owner, stimulus, ctx, enemy_tick)
        {
            handled
        } else {
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
        // Macro entry is a synchronous statement inside Think. Settle its
        // preceding notifications before execution, retaining completion latches
        // for the enclosing EndThink below.
        let has_macro_call = self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("Think macro entry"))
            .outbox
            .reentrant
            .owner_work
            .iter()
            .any(|work| matches!(work, crate::ai::AiOwnerWork::RunMacro));
        if has_macro_call {
            self.drain_ai_owner_work_for_boundary(
                sim,
                assets,
                owner,
                super::CompletionBoundary::StatementPrefix,
            );
        }
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
