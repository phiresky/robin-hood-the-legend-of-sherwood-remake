//! Actor-ID based AI call orchestration. Actor borrows end between admission,
//! handler execution, and completion.

use super::*;

impl EngineInner {
    pub(crate) fn ai_admission(&self, owner: EntityId) -> crate::ai::AiAdmission {
        let entity = self
            .world
            .entities
            .expect_entity(owner, format_args!("decision admission"));
        crate::ai::AiAdmission {
            frame: self.control.frame_counter,
            original_creation_order: Some(self.world.original_creation_order(owner)),
            think_depth: self.ai_think_depth(),
            in_building: self
                .entity_building_sector(entity.element_data().sector())
                .is_some(),
            self_is_rider: matches!(entity, Entity::Soldier(s) if s.soldier.rider),
            self_is_dead: entity.is_dead(),
            self_is_unconscious: entity.is_unconscious(),
            posture: entity.element_data().posture(),
            position: self.live_ai_position(owner),
        }
    }

    pub(in crate::engine) fn begin_ai_think_before_filter(
        &mut self,
        owner: EntityId,
        stimulus: &crate::ai::Stimulus,
    ) -> bool {
        use crate::ai::AiRole;
        if self
            .world
            .entities
            .get(owner)
            .and_then(Entity::ai_controller)
            .is_none()
        {
            return false;
        }
        let frame = self.control.frame_counter;
        let ai = self
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("decision event log"));
        ai.cached_frame = frame;
        ai.register_log_line(crate::ai::LogLineType::Event, stimulus.stimulus_type as u16);
        self.enter_ai_think_frame(owner);
        let entity = self
            .world
            .entities
            .expect_entity_mut(owner, format_args!("decision pre-filter"));
        if let Some(enemy) = entity.enemy_ai_mut() {
            enemy.start_think_pre_filter(stimulus);
        } else {
            entity
                .friendly_ai_mut()
                .expect("decision owner has no brain")
                .start_think_pre_filter(stimulus);
        }
        true
    }

    pub(in crate::engine) fn enter_ai_think_frame(&mut self, owner: EntityId) {
        self.ai_think_depth()
            .checked_add(1)
            .expect("think recursion depth overflow");
        self.ai.think_call_stack.push(owner);
    }

    pub(crate) fn ai_think_depth(&self) -> u8 {
        u8::try_from(self.ai.think_call_stack.len()).expect("think recursion depth overflow")
    }

    pub(in crate::engine) fn execute_ai_duty_call(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        call: crate::ai::DutyCall,
    ) -> bool {
        match call.tail {
            crate::ai::DutyTail::SelectShotTarget {
                old_substate,
                cover_shield_bearer,
            } => {
                self.execute_ai_select_shot_target(
                    sim,
                    assets,
                    owner,
                    old_substate,
                    cover_shield_bearer,
                );
            }
            crate::ai::DutyTail::BattleOverview { flags } => {
                self.execute_ai_get_battle_overview(sim, assets, owner, flags);
            }
            crate::ai::DutyTail::ReconsiderEnemyApproach { reachpoint } => {
                self.execute_ai_reconsider_enemy_approach(sim, assets, owner, reachpoint);
            }
            crate::ai::DutyTail::AttackEnemy { target } => {
                self.execute_ai_attack_enemy(sim, assets, owner, target);
            }
            crate::ai::DutyTail::RiderAttack { fallback } => {
                if !self.execute_ai_maybe_make_rider_attack(sim, assets, owner) {
                    match fallback {
                        crate::ai::RiderAttackFallback::Approach => {
                            self.duty_set_state(
                                sim,
                                assets,
                                owner,
                                crate::ai::AiState::Attacking,
                                crate::ai::Substate::AttackingRunningToEnemy,
                            );
                            self.execute_ai_reconsider_enemy_approach(sim, assets, owner, true);
                        }
                        crate::ai::RiderAttackFallback::BattleDecisions => {
                            self.execute_battle_decisions(sim, assets, owner);
                        }
                    }
                }
            }
            crate::ai::DutyTail::RunAndAlertSoldiers { center } => {
                self.execute_ai_run_and_alert_soldiers_decision(sim, assets, owner, center);
            }
            crate::ai::DutyTail::OfficerLookForSoldier { reason } => {
                self.execute_ai_officer_look_for_soldier(sim, assets, owner, reason);
            }
            crate::ai::DutyTail::TowerGuardAlert { center } => {
                self.execute_ai_tower_guard_alert(sim, assets, owner, center);
            }
            crate::ai::DutyTail::AlertOfficer { caller } => {
                self.execute_ai_alert_officer_for_caller(sim, assets, owner, caller);
            }
            crate::ai::DutyTail::BattleDecisions => {
                self.execute_battle_decisions(sim, assets, owner);
            }
            crate::ai::DutyTail::ReconsiderSwordfight { enemy_weak } => {
                self.execute_reconsider_swordfight(sim, assets, owner, enemy_weak);
            }
            crate::ai::DutyTail::ReconsiderSwordfightObservation => {
                self.execute_reconsider_swordfight_observation(sim, assets, owner);
            }
            crate::ai::DutyTail::CommandSoldiersToAttack { center } => {
                self.execute_ai_combat_alert_decision(sim, assets, owner, center);
            }
            crate::ai::DutyTail::AlertSoldiers {
                center,
                flags,
                failure,
            } => {
                self.execute_ai_alert_soldiers_with_failure(
                    sim, assets, owner, center, flags, failure,
                );
            }
            crate::ai::DutyTail::MoneyFight { operation } => {
                self.execute_money_fight(sim, assets, owner, operation);
            }
            crate::ai::DutyTail::None => {
                self.execute_ai_return_to_duty(sim, assets, owner, call.flags);
            }
            crate::ai::DutyTail::FinishSeek => {
                self.execute_ai_return_to_duty(sim, assets, owner, call.flags);
                self.execute_finish_exhausted_search(sim, assets, owner);
            }
            crate::ai::DutyTail::SeekArea {
                center,
                standard_radius,
                flags,
                seek_direction,
            } => {
                self.execute_ai_seek_area(
                    sim,
                    assets,
                    owner,
                    center,
                    standard_radius,
                    flags,
                    seek_direction,
                );
            }
            crate::ai::DutyTail::SeekNextPoint => {
                self.execute_ai_seek_next_point(sim, assets, owner);
            }
            crate::ai::DutyTail::ScanSleepingEnemies { observer_camp } => {
                self.execute_kill_nearby_sleeping_enemies(sim, assets, owner, observer_camp);
            }
            crate::ai::DutyTail::ApproachSleepingEnemies { targets } => {
                self.execute_approach_sleeping_enemies(sim, assets, owner, targets);
            }
            crate::ai::DutyTail::DispatchPatrol { stimulus } => {
                self.execute_ai_dispatch_patrol_event(sim, assets, owner, stimulus);
            }
            crate::ai::DutyTail::Think { stimulus } => {
                self.execute_ai_callback(sim, assets, owner, &stimulus);
            }
            crate::ai::DutyTail::SearchCharlyTimer
            | crate::ai::DutyTail::TooProudOverviewRemark
            | crate::ai::DutyTail::SwordfightInsult
            | crate::ai::DutyTail::GotHitViewStatus
            | crate::ai::DutyTail::FinishBattleFightAfterAttack
            | crate::ai::DutyTail::AfterCombatInjury => {
                panic!("caller tail used as a duty operation");
            }
        }
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
        for tail in call.after {
            match tail {
                crate::ai::DutyTail::FinishBattleFightAfterAttack => {
                    self.finish_ai_battle_fight_after_attack(sim, assets, owner);
                }
                crate::ai::DutyTail::GotHitViewStatus => {
                    let npc = self
                        .world
                        .entities
                        .expect_ai_actor_data_mut(owner, format_args!("hit eye-status tail"));
                    crate::ai_vision::set_view_status(
                        npc,
                        crate::element::EyeStatus::DieOrGetUnconscious,
                    );
                }
                crate::ai::DutyTail::SearchCharlyTimer => {
                    self.world
                        .entities
                        .expect_ai_controller_mut(owner, format_args!("search timer tail"))
                        .launch_timer(
                            crate::parameters_ai::AI_CHECKFOR_TIME_INTERVAL as u32,
                            self.control.frame_counter,
                        );
                }
                crate::ai::DutyTail::TooProudOverviewRemark => {
                    self.world
                        .entities
                        .expect_entity_mut(owner, format_args!("combat remark tail"))
                        .enemy_ai_mut()
                        .expect("combat remark owner has no enemy brain")
                        .too_proud_overview_finally_fight_remark();
                }
                crate::ai::DutyTail::SwordfightInsult => {
                    self.world
                        .entities
                        .expect_enemy_ai_mut(owner, format_args!("swordfight remark tail"))
                        .swordfight_insult_after_reconsider();
                }
                crate::ai::DutyTail::AfterCombatInjury => {
                    self.world
                        .entities
                        .expect_enemy_ai_mut(owner, format_args!("combat injury tail"))
                        .finish_after_combat_injury();
                }
                _ => panic!("duty operation used as a caller tail"),
            }
            self.drain_direct_ai_owner_boundary(sim, owner, assets);
        }
        call.think_result
    }

    /// Finish one decision frame, returning from each nested call before
    /// inspecting the next movement-completion latch.
    pub(in crate::engine) fn execute_ai_end_think(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        assert_eq!(
            self.ai.think_call_stack.last(),
            Some(&owner),
            "decision completion has no matching active frame"
        );
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
        for event in [
            StimulusType::EventCouldntReachPoint,
            StimulusType::EventReachPoint,
            StimulusType::EventDone,
        ] {
            let (pending, depth) = {
                let ai = self
                    .world
                    .entities
                    .expect_ai_controller_mut(owner, format_args!("decision completion phase"));
                let pending = match event {
                    StimulusType::EventCouldntReachPoint => {
                        std::mem::take(&mut ai.couldnt_reachpoint)
                    }
                    StimulusType::EventReachPoint => std::mem::take(&mut ai.already_on_point),
                    StimulusType::EventDone => std::mem::take(&mut ai.already_turned),
                    _ => unreachable!(),
                };
                (pending, self.ai.think_call_stack.len())
            };
            if pending {
                if depth < 100 {
                    self.execute_ai_callback(sim, assets, owner, &crate::ai::Stimulus::new(event));
                } else if depth < 111 {
                    self.execute_ai_return_to_duty(
                        sim,
                        assets,
                        owner,
                        crate::ai::DutyFlags::empty(),
                    );
                }
            }
        }
        assert_eq!(
            self.ai.think_call_stack.pop(),
            Some(owner),
            "decision completion returned out of call order"
        );
    }

    pub(crate) fn execute_ai_return_to_duty(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        flags: crate::ai::DutyFlags,
    ) {
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
        self.execute_specialized_ai_duty(sim, assets, owner, flags);
        self.drain_direct_ai_owner_boundary(sim, owner, assets);
    }

    /// Execute a nested actor decision to completion before its caller resumes.
    pub(crate) fn execute_ai_callback(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &crate::ai::Stimulus,
    ) -> bool {
        self.execute_ai_callback_for_target(sim, assets, owner, stimulus, None)
    }

    pub(in crate::engine) fn execute_ai_callback_for_target(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &crate::ai::Stimulus,
        target: Option<EntityId>,
    ) -> bool {
        self.dispatch_think_with_drain(sim, owner, stimulus, target, assets)
    }

    /// Run the body of an already admitted decision without entering a new frame.
    pub(in crate::engine) fn execute_ai_handler_body(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &crate::ai::Stimulus,
        target: Option<EntityId>,
    ) -> bool {
        let enemy_owner = self
            .world
            .entities
            .expect_entity(owner, format_args!("admitted decision owner"))
            .enemy_ai()
            .is_some();
        let handled = if stimulus.stimulus_type == StimulusType::EventReturnToDuty {
            Err(crate::ai::DutyCall::new(
                crate::ai::DutyFlags::empty(),
                false,
            ))
        } else if enemy_owner
            && matches!(
                stimulus.stimulus_type,
                StimulusType::EventTimer | StimulusType::CallInstruction
            )
            && self
                .world
                .entities
                .expect_ai_controller(owner, format_args!("phalanx timer"))
                .current_substate
                == crate::ai::Substate::AttackingPhalanx
        {
            if stimulus.stimulus_type == StimulusType::EventTimer {
                self.execute_ai_phalanx_timer(sim, assets, owner);
            } else {
                self.execute_ai_phalanx_instruction(sim, assets, owner);
            }
            Ok(false)
        } else if enemy_owner
            && stimulus.stimulus_type == StimulusType::EventTimer
            && self
                .world
                .entities
                .expect_ai_controller(owner, format_args!("shield timer"))
                .current_substate
                == crate::ai::Substate::AttackingAdvancingWithShield
        {
            self.execute_ai_advancing_shield_timer(sim, assets, owner);
            Ok(false)
        } else if enemy_owner && stimulus.stimulus_type == StimulusType::CallPatrolCoordinate {
            self.execute_ai_coordinate_patrol(sim, assets, owner, &stimulus.info);
            Ok(false)
        } else if let Some(handled) = self.execute_friendly_callback(sim, assets, owner, stimulus) {
            Ok(handled)
        } else if let Some(handled) =
            self.execute_enemy_report_callback(sim, assets, owner, stimulus)
        {
            Ok(handled)
        } else {
            let scratch = self.build_sim_scratch(assets);
            let mut ctx = self.ai_context_for(owner, self.control.frame_counter, &scratch, assets);
            if let crate::ai::StimulusInfo::Human(handle) = stimulus.info {
                let view = ctx.entity_view(handle.get()).unwrap_or_else(|| {
                    panic!("decision stimulus references missing human {handle}")
                });
                ctx.antagonist = Some(crate::ai::AntagonistInfo {
                    position: view.position,
                    camp: view.camp,
                    is_swordfighting: view.is_swordfighting,
                    is_pc: view.is_pc,
                    is_robin: view.is_robin,
                    is_vip: view.is_vip,
                    in_building: view.in_building,
                });
            }
            ctx.in_uninterruptible_command = self.is_very_very_busy(owner);
            ctx.seed_view_radius_cache(&self.ai.view_radius_cache);
            ctx.enter_swordfight_pending = self
                .orders
                .sequence_manager
                .element_is_about_to_be_launched_or_postponed_by_current(
                    owner,
                    crate::element::Command::EnterSwordfight,
                );
            self.refresh_selected_default_wait_identity(owner, &mut ctx);
            let enemy_tick = enemy_owner
                .then(|| self.build_npc_tick_data_for_target(sim, owner, assets, target));
            let entity = self
                .world
                .entities
                .expect_entity_mut(owner, format_args!("Think handler"));
            let result = if let Some(enemy) = entity.enemy_ai_mut() {
                enemy.think_body(
                    crate::ai_enemy::ThinkEnv::new(
                        sim,
                        &ctx,
                        enemy_tick
                            .as_ref()
                            .expect("enemy Think requires tactical data"),
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
                        &ctx,
                        Some(&self.world.fast_grid),
                        Some(self.script_domains.interactables.doors.as_slice()),
                    )
            };
            ctx.commit_view_radius_cache(&mut self.ai.view_radius_cache);
            result
        };
        let handled = match handled {
            Ok(handled) => handled,
            Err(call) => self.execute_ai_duty_call(sim, assets, owner, call),
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
            self.drain_pending_for_npc(sim, owner, assets);
        }
        handled
    }

    pub(in crate::engine) fn execute_ai_think(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &crate::ai::Stimulus,
        target: Option<EntityId>,
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
        let admission = self.ai_admission(owner);
        let admitted = {
            let entity = self
                .world
                .entities
                .expect_entity_mut(owner, format_args!("Think admission"));
            if let Some(enemy) = entity.enemy_ai_mut() {
                enemy.begin_think(&admission, stimulus, &mut self.ai.global)
            } else {
                entity
                    .friendly_ai_mut()
                    .expect("Think owner has no brain")
                    .begin_think(sim, stimulus, &mut self.ai.global, &admission)
            }
        };
        if !admitted {
            self.execute_ai_end_think(sim, assets, owner);
            return true;
        }

        let handled = self.execute_ai_handler_body(sim, assets, owner, stimulus, target);
        let suspended = stimulus.stimulus_type == StimulusType::EventAfterScriptGoOn
            && self
                .world
                .entities
                .expect_ai_controller(owner, format_args!("Think completion"))
                .outbox
                .reentrant
                .engine_drains_after_script_go_on;
        if !suspended {
            self.execute_ai_end_think(sim, assets, owner);
        }
        if let Some(enemy) = self.world.entities.get(owner).and_then(Entity::enemy_ai) {
            enemy.base.debug_macro_lifecycle_at(
                admission.frame,
                admission.original_creation_order,
                "think_return",
                stimulus.stimulus_type,
            );
        }
        handled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::AiRole;
    use crate::engine::test_support::actors::make_test_ai_soldier;

    fn enter(engine: &mut EngineInner, owner: EntityId) {
        engine.enter_ai_think_frame(owner);
        engine
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("test decision owner"))
            .start_think_pre_filter(&crate::ai::Stimulus::new(StimulusType::EventTimer));
    }

    #[test]
    fn nested_actor_completion_keeps_its_callers_frame_open() {
        let mut engine = EngineInner::new();
        let first = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Royalists));
        let second = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Royalists));
        enter(&mut engine, first);
        enter(&mut engine, second);
        assert_eq!(engine.ai_think_depth(), 2);
        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::default();
        engine.execute_ai_end_think(&sim, &assets, second);
        assert_eq!(engine.ai.think_call_stack, vec![first]);
        assert_eq!(engine.ai_think_depth(), 1);
        engine.execute_ai_end_think(&sim, &assets, first);
        assert!(engine.ai.think_call_stack.is_empty());
    }

    #[test]
    fn completion_rechecks_sibling_latches_after_nested_admission() {
        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Royalists));
        enter(&mut engine, owner);
        let ai = engine
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("completion fixture"));
        ai.current_state = crate::ai::AiState::Sleeping;
        ai.current_substate = crate::ai::Substate::SleepingUnconscious;
        ai.couldnt_reachpoint = true;
        ai.already_on_point = true;
        ai.already_turned = true;
        engine.execute_ai_end_think(
            &crate::sim_rng::test_context(),
            &LevelAssets::default(),
            owner,
        );
        let ai = engine
            .world
            .entities
            .expect_ai_controller(owner, format_args!("completed fixture"));
        let events: Vec<_> = ai
            .ai_log
            .iter()
            .filter(|line| line.line_type == crate::ai::LogLineType::Event)
            .map(|line| line.info)
            .collect();
        assert_eq!(events, vec![StimulusType::EventCouldntReachPoint as u16]);
        assert!(!ai.couldnt_reachpoint && !ai.already_on_point && !ai.already_turned);
        assert!(engine.ai.think_call_stack.is_empty());
    }

    #[test]
    fn completion_depth_limit_counts_other_actor_frames() {
        let mut engine = EngineInner::new();
        let caller = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Royalists));
        let owner = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Royalists));
        for _ in 0..110 {
            enter(&mut engine, caller);
        }
        enter(&mut engine, owner);
        let ai = engine
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("deep completion"));
        ai.couldnt_reachpoint = true;
        ai.already_on_point = true;
        ai.already_turned = true;
        engine.execute_ai_end_think(
            &crate::sim_rng::test_context(),
            &LevelAssets::default(),
            owner,
        );
        let ai = engine
            .world
            .entities
            .expect_ai_controller(owner, format_args!("deep completion result"));
        assert!(
            ai.ai_log.is_empty(),
            "depth-limited completion must not call Think"
        );
        assert!(!ai.couldnt_reachpoint && !ai.already_on_point && !ai.already_turned);
        assert_eq!(engine.ai.think_call_stack.len(), 110);
    }

    #[test]
    fn recursive_same_actor_completion_returns_one_frame_at_a_time() {
        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Royalists));
        enter(&mut engine, owner);
        enter(&mut engine, owner);
        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::default();
        engine.execute_ai_end_think(&sim, &assets, owner);
        assert_eq!(engine.ai.think_call_stack, vec![owner]);
        assert_eq!(engine.ai_think_depth(), 1);
        engine.execute_ai_end_think(&sim, &assets, owner);
        assert!(engine.ai.think_call_stack.is_empty());
    }
}
