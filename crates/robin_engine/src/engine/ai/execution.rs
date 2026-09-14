//! Actor-ID based AI call orchestration. Actor borrows end between admission,
//! handler execution, and completion.

use super::*;
use crate::ai::{AiState, Stimulus, Substate};
use crate::sim_rng::SimulationContext;

#[cfg(test)]
mod after_script_tests {
    use super::*;

    #[test]
    fn retained_callbacks_skip_duplicate_markers_and_finish_the_outer_think() {
        let (mut engine, assets, owner, _) =
            super::super::battle_decision_observation_tests::fixture(false);
        let ai = engine
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("retained events fixture"));
        ai.current_state = AiState::Wondering;
        ai.current_substate = Substate::WonderingOfficerSeeingBrawl;
        ai.stimulus_queue = vec![
            Stimulus::new(StimulusType::EventAfterScriptGoOn),
            Stimulus::new(StimulusType::NoEvent),
            Stimulus::new(StimulusType::EventAfterScriptGoOn),
        ];
        assert!(!engine.execute_ai_callback(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            &Stimulus::new(StimulusType::EventAfterScriptGoOn)
        ));
        let ai = engine
            .world
            .entities
            .expect_ai_controller(owner, format_args!("retained events result"));
        assert!(ai.stimulus_queue.is_empty());
        assert_eq!(ai.current_substate, Substate::WonderingOfficerSeeingBrawl);
        assert!(engine.ai.think_call_stack.is_empty());
    }
}

impl EngineInner {
    fn execute_ai_after_script(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        loop {
            let stimulus = {
                let ai = self
                    .world
                    .entities
                    .expect_ai_controller_mut(owner, format_args!("AfterScript retained event"));
                if ai.stimulus_queue.is_empty() {
                    break;
                }
                if !ai.locks_flag_field.is_empty() || ai.script_locked {
                    return;
                }
                ai.stimulus_queue.remove(0)
            };
            if stimulus.stimulus_type != StimulusType::EventAfterScriptGoOn {
                let target = match stimulus.info {
                    crate::ai::StimulusInfo::Human(handle)
                        if matches!(
                            stimulus.stimulus_type,
                            StimulusType::EventView
                                | StimulusType::EventOutOfView
                                | StimulusType::EventSeesBeggar
                                | StimulusType::EventEnemyNear
                        ) =>
                    {
                        Some(
                            self.entity_id_for_index(handle.get())
                                .expect("retained event target"),
                        )
                    }
                    _ => None,
                };
                self.execute_ai_callback_for_target(sim, assets, owner, &stimulus, target);
            }
        }
        let ai = self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("AfterScript route"));
        if ai.current_state != AiState::Default {
            return;
        }
        if ai
            .patrol_path
            .as_ref()
            .and_then(|path| path.current_waypoint(&assets.navigation.hiking_paths))
            .is_none()
        {
            self.execute_ai_return_to_duty(sim, assets, owner, crate::ai::DutyFlags::empty());
            return;
        }
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("AfterScript route advance"))
            .patrol_path
            .as_mut()
            .expect("AfterScript path")
            .advance();
        self.duty_set_state(
            sim,
            assets,
            owner,
            AiState::Default,
            Substate::DefaultEnroute,
        );
        let ai = self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("AfterScript route callback"));
        let path = ai
            .patrol_path
            .as_ref()
            .expect("AfterScript path after callback");
        let waypoint = path
            .current_waypoint(&assets.navigation.hiking_paths)
            .expect("AfterScript waypoint after callback");
        let destination = crate::ai::Position {
            x: waypoint.x as f32,
            y: waypoint.y as f32,
            sector: assets.navigation.hiking_waypoint_sector(
                usize::from(path.hiking_path_index),
                usize::from(path.current_waypoint_index),
                waypoint.sector,
            ),
            level: waypoint.level,
        };
        let flags = ai.default_path_walking_flags;
        self.duty_go_to(sim, assets, owner, destination, flags);
    }

    pub(in crate::engine) fn begin_ai_think_before_filter(
        &mut self,
        assets: &LevelAssets,
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
        if stimulus.stimulus_type == crate::ai::StimulusType::EventLoseConsciousness {
            self.execute_ai_set_alert_status(
                assets,
                owner,
                crate::ai::AlertLevel::Green,
                crate::ai::AlertFlags::empty(),
            );
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
        self.execute_specialized_ai_duty(sim, assets, owner, flags);
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
        let body_reaction = if enemy_owner {
            use crate::ai::{BodyReaction, Substate};
            let substate = self
                .world
                .entities
                .expect_ai_controller(owner, format_args!("body decision"))
                .current_substate;
            match (substate, stimulus.stimulus_type) {
                (Substate::SeekingBodyReactiontime, StimulusType::EventTimer) => {
                    Some(BodyReaction::ReactionTimer)
                }
                (Substate::SeekingBody, StimulusType::EventTimer) => Some(BodyReaction::BodyTimer),
                (Substate::SeekingBody, StimulusType::EventReachPoint) => {
                    Some(BodyReaction::Arrival)
                }
                (Substate::SeekingBody, StimulusType::EventCouldntReachPoint) => {
                    Some(BodyReaction::Unreachable)
                }
                (Substate::SeekingBodyLookingDeadBody, StimulusType::EventTimer) => {
                    Some(BodyReaction::DeadBodyTimer)
                }
                (Substate::SeekingBodyAwakeningSleeperr, StimulusType::EventTimer) => {
                    Some(BodyReaction::SleeperTimer)
                }
                (Substate::SeekingTakingNet, StimulusType::EventDone) => {
                    Some(BodyReaction::NetDone)
                }
                _ => None,
            }
        } else {
            None
        };
        let panic_segment = matches!(
            stimulus.stimulus_type,
            StimulusType::EventReachPoint | StimulusType::EventCouldntReachPoint
        ) && self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("panic dispatch"))
            .current_substate
            == crate::ai::Substate::FleeingPanic;
        let handled = if panic_segment {
            self.execute_ai_common_fleeing_event(sim, assets, owner, stimulus)
                .expect("panic segment must be handled by the common fleeing dispatcher")
        } else if let Some(operation) = body_reaction {
            self.execute_ai_body_reaction(sim, assets, owner, operation);
            false
        } else if enemy_owner && stimulus.stimulus_type == StimulusType::EventSeesBody {
            let state = self
                .world
                .entities
                .expect_ai_controller(owner, format_args!("body sighting"))
                .current_state;
            if matches!(
                state,
                crate::ai::AiState::Sleeping
                    | crate::ai::AiState::Default
                    | crate::ai::AiState::Wondering
                    | crate::ai::AiState::Seeking
            ) && let crate::ai::StimulusInfo::Human(body) = stimulus.info
                && !self.dispatch_live_stimulus_to_patrol(sim, assets, owner, stimulus)
            {
                self.execute_ai_body_reaction(
                    sim,
                    assets,
                    owner,
                    crate::ai::BodyReaction::Seen { body: body.get() },
                );
            }
            false
        } else if stimulus.stimulus_type == StimulusType::EventReturnToDuty {
            self.execute_ai_return_to_duty(sim, assets, owner, crate::ai::DutyFlags::empty());
            false
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
            false
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
            false
        } else if enemy_owner
            && self.execute_ai_shield_expected_event(sim, assets, owner, stimulus.stimulus_type)
        {
            false
        } else if enemy_owner
            && self.execute_ai_archery_expected_event(sim, assets, owner, stimulus.stimulus_type)
        {
            false
        } else if enemy_owner
            && (self.execute_ai_combat_unexpected_event(sim, assets, owner, stimulus)
                || self.execute_ai_combat_expected_event(
                    sim,
                    assets,
                    owner,
                    stimulus.stimulus_type,
                ))
        {
            false
        } else if enemy_owner
            && let Some(handled) =
                self.execute_ai_officer_rendezvous_event(sim, assets, owner, stimulus)
        {
            handled
        } else if enemy_owner
            && let Some(handled) = self.execute_ai_wondering_event(sim, assets, owner, stimulus)
        {
            handled
        } else if enemy_owner
            && let Some(handled) = self.execute_ai_seeking_event(sim, assets, owner, stimulus)
        {
            handled
        } else if enemy_owner
            && let Some(handled) = self.execute_ai_money_event(sim, assets, owner, stimulus)
        {
            handled
        } else if enemy_owner && stimulus.stimulus_type == StimulusType::CallPatrolCoordinate {
            self.execute_ai_coordinate_patrol(sim, assets, owner, &stimulus.info);
            false
        } else if let Some(handled) = self.execute_friendly_callback(sim, assets, owner, stimulus) {
            handled
        } else if let Some(handled) =
            self.execute_enemy_report_callback(sim, assets, owner, stimulus)
        {
            handled
        } else if enemy_owner {
            self.execute_ai_enemy_event(sim, assets, owner, stimulus)
        } else {
            self.execute_friendly_remaining_event(sim, assets, owner, stimulus)
        };
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
        let frame = self.control.frame_counter;
        let original_creation_order = Some(self.world.original_creation_order(owner));
        let admitted = if self
            .world
            .entities
            .expect_entity(owner, format_args!("Think admission"))
            .enemy_ai()
            .is_some()
        {
            self.begin_enemy_think(sim, assets, owner, stimulus)
        } else {
            self.begin_friendly_think(sim, assets, owner, stimulus)
        };
        if !admitted {
            self.execute_ai_end_think(sim, assets, owner);
            return true;
        }

        let handled = if stimulus.stimulus_type == StimulusType::EventAfterScriptGoOn {
            self.execute_ai_after_script(sim, assets, owner);
            false
        } else {
            self.execute_ai_handler_body(sim, assets, owner, stimulus, target)
        };
        self.execute_ai_end_think(sim, assets, owner);
        if let Some(enemy) = self.world.entities.get(owner).and_then(Entity::enemy_ai) {
            enemy.base.debug_macro_lifecycle_at(
                frame,
                original_creation_order,
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
