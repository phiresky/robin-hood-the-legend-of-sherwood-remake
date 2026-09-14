use super::*;

impl EngineInner {
    pub(in crate::engine) fn execute_ai_dispatch_patrol_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        mut stimulus: crate::ai::Stimulus,
    ) {
        if !self.dispatch_live_stimulus_to_patrol(sim, assets, owner, &stimulus) {
            stimulus.to_whole_patrol = true;
            self.resume_local_patrol_stimulus(sim, assets, owner, &stimulus);
        }
    }

    fn resume_local_patrol_stimulus(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &crate::ai::Stimulus,
    ) {
        let target = match stimulus.info {
            crate::ai::StimulusInfo::Human(handle)
                if matches!(
                    stimulus.stimulus_type,
                    crate::ai::StimulusType::EventView
                        | crate::ai::StimulusType::EventOutOfView
                        | crate::ai::StimulusType::EventSeesBeggar
                        | crate::ai::StimulusType::EventEnemyNear
                ) =>
            {
                Some(self.expect_entity_id_for_index(handle.get(), "patrol detection target"))
            }
            _ => None,
        };
        self.execute_ai_handler_body(sim, assets, owner, stimulus, target);
    }

    pub(in crate::engine) fn dispatch_live_stimulus_to_patrol(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &crate::ai::Stimulus,
    ) -> bool {
        use crate::ai::{AiState, StimulusType, Substate};

        if stimulus.to_whole_patrol {
            return false;
        }
        let ai = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("patrol dispatch owner"));
        if matches!(
            stimulus.stimulus_type,
            StimulusType::EventSeesObject | StimulusType::EventHear | StimulusType::EventSeesBody
        ) && ai
            .last_stimulus_dispatched_to_patrol
            .as_ref()
            .is_some_and(|last| last.is_similar(stimulus))
        {
            return true;
        }
        match ai.base.current_state {
            AiState::Default
                if ai.base.current_substate != Substate::DefaultPatrolEnrouteRunning => {}
            AiState::Wondering => {}
            _ => return false,
        }
        if let Some(chief) = ai.base.patrol_chief {
            if matches!(
                self.world
                    .entities
                    .expect_entity(chief, format_args!("patrol chief")),
                Entity::Soldier(_)
            ) && self.patrol_member_visible(assets, owner, chief)
            {
                return self.dispatch_live_stimulus_to_patrol(sim, assets, chief, stimulus);
            }
        }

        let ai = self
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("patrol dispatch owner"));
        ai.last_stimulus_dispatched_to_patrol = Some(*stimulus);
        if ai.base.patrol.is_empty() {
            return false;
        }
        // This call intentionally retains membership before recursively
        // processing the chief, which can rebuild the live patrol list.
        let members = ai.base.patrol.iter().map(|member| member.index()).collect();
        let mut forwarded = *stimulus;
        forwarded.to_whole_patrol = true;
        self.execute_ai_patrol_broadcast(sim, assets, owner, forwarded, members);
        true
    }

    pub(in crate::engine) fn execute_ai_patrol_broadcast(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        source_id: EntityId,
        stimulus: crate::ai::Stimulus,
        members: Vec<u32>,
    ) {
        self.execute_ai_callback(sim, assets, source_id, &stimulus);
        self.execute_ai_patrol_member_broadcast(sim, assets, source_id, &stimulus, members);
    }

    fn execute_ai_patrol_member_broadcast(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        source_id: EntityId,
        stimulus: &crate::ai::Stimulus,
        members: Vec<u32>,
    ) {
        for member in members {
            let member_id = self.entity_id_for_index(member).unwrap_or_else(|| {
                panic!(
                    "patrol broadcast from chief {} references missing member {member}",
                    source_id.index()
                )
            });

            let detected = matches!(
                self.world
                    .entities
                    .expect_entity(member_id, format_args!("patrol broadcast member")),
                Entity::Soldier(_)
            ) && self.patrol_member_visible(assets, source_id, member_id);
            tracing::trace!(
                target: "patrol_relay",
                chief = source_id.index(),
                member,
                stimulus_type = ?stimulus.stimulus_type,
                detected,
                "patrol broadcast member gate"
            );
            if !detected {
                continue;
            }

            self.execute_ai_callback(sim, assets, member_id, stimulus);
        }
    }

    pub(in crate::engine) fn process_synchronous_think_results_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) {
        let requests = {
            let ai = self
                .world
                .entities
                .expect_ai_controller_mut(source_id, format_args!("Think-result source"));
            let mut requests = Vec::new();
            let mut deferred = Vec::new();
            for action in ai.outbox.reentrant.cross_npc_actions.drain(..) {
                if matches!(action, crate::ai::CrossNpcAction::RequestThinkResult { .. }) {
                    requests.push(action);
                } else {
                    deferred.push(action);
                }
            }
            ai.outbox.reentrant.cross_npc_actions = deferred;
            requests
        };

        for request in requests {
            let crate::ai::CrossNpcAction::RequestThinkResult {
                target,
                caller,
                stimulus_type,
                info,
                continuation,
            } = request
            else {
                unreachable!("Think-result drain returned a different action")
            };
            assert_eq!(
                source_id.index(),
                caller,
                "Think-result caller must be its owner"
            );
            let target_id = self.entity_id_for_index(target).unwrap_or_else(|| {
                panic!(
                    "synchronous {stimulus_type:?} from NPC {caller} references missing target {target}"
                )
            });
            if !matches!(self.world.entities.get(target_id), Some(Entity::Soldier(s)) if s.npc.ai_brain.enemy().is_some())
            {
                panic!(
                    "synchronous {stimulus_type:?} from enemy NPC {caller} requires enemy-soldier target {target}"
                );
            }
            let mut stimulus = crate::ai::Stimulus::new(stimulus_type);
            stimulus.info = info;
            let accepted = self.execute_ai_callback(sim, assets, target_id, &stimulus);

            let flow = self
                .world
                .entities
                .expect_enemy_ai_mut(
                    source_id,
                    format_args!("Think-result caller {caller} lost its EnemyAi"),
                )
                .resolve_think_result(self.control.frame_counter, accepted, target, continuation);
            match flow {
                Err(call) => {
                    self.execute_ai_duty_call(sim, assets, source_id, call);
                }
                Ok(true) => {
                    self.drain_direct_ai_owner_boundary(sim, source_id, assets);
                    let destination = self
                        .world
                        .entities
                        .expect_enemy_ai(source_id, format_args!("pointing officer"))
                        .officers_position;
                    self.duty_point_to(sim, assets, source_id, destination);
                }
                Ok(false) => {}
            }

            // The continuation is the caller's original-game stack frame resuming
            // immediately after target event processing returned. The officer's
            // single-soldier call can reject into returning to duty, whose
            // state changes and following movement synchronously publish actor work as
            // well as owner callbacks. Close that exact caller stack through
            // the full owner-local fixed point. Other result continuations
            // retain their narrower owner-work boundary because their outer
            // loops still own subsequent member calls.
            if matches!(
                continuation,
                crate::ai::ThinkResultContinuation::OfficerCalledSoldier
            ) {
                self.drain_direct_ai_owner_boundary(sim, source_id, assets);
            } else {
                self.drain_ai_owner_work_for(sim, assets, source_id);
            }
        }
    }

    pub(super) fn process_synchronous_alert_requests_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) {
        let requests = self
            .world
            .entities
            .expect_ai_controller_mut(source_id, format_args!("CALL_ALERT source"))
            .take_pending_alert_requests();

        for request in requests {
            let crate::ai::CrossNpcAction::RequestAlert { target, caller } = request else {
                unreachable!("alert-request drain returned a deferred action")
            };
            assert_eq!(
                source_id.index(),
                caller,
                "CALL_ALERT request caller must be its owner"
            );

            let target_id = self.entity_id_for_index(target).unwrap_or_else(|| {
                panic!(
                    "synchronous CALL_ALERT from NPC {caller} references missing target {target}"
                )
            });
            assert!(
                matches!(self.world.entities.get(target_id), Some(Entity::Soldier(_))),
                "synchronous CALL_ALERT target {target} is not a soldier"
            );
            let accepted = self.execute_ai_callback(
                sim,
                assets,
                target_id,
                &crate::ai::Stimulus::with_human(crate::ai::StimulusType::CallAlert, caller),
            );

            if !accepted {
                self.execute_ai_return_to_duty(
                    sim,
                    assets,
                    source_id,
                    crate::ai::DutyFlags::empty(),
                );
                continue;
            }
            self.duty_set_state(
                sim,
                assets,
                source_id,
                crate::ai::AiState::Seeking,
                crate::ai::Substate::SeekingRunningToOfficerSeen,
            );
            self.world
                .entities
                .expect_ai_controller_mut(source_id, format_args!("soldier alert caller"))
                .say_with_flags(
                    crate::ai::Remark::CallsOfficer,
                    crate::ai::SpeechFlags::MYTALK_0,
                );
            self.drain_direct_ai_owner_boundary(sim, source_id, assets);
            let officer = self
                .world
                .entities
                .expect_ai_controller(source_id, format_args!("soldier alert caller"))
                .antagonist
                .expect("accepted soldier alert requires target officer");
            let officer_id =
                self.expect_human_id_for_ai_handle(officer.get(), "accepted soldier alert officer");
            let officer = self.expect_entity(officer_id, "accepted soldier alert forecast");
            let passing_door =
                selected_actor_is_passing_door(&self.orders.sequence_manager, officer_id);
            let input = extract_exact_forecast_input(self, officer, passing_door)
                .expect("officer forecast requires actor");
            let destination = crate::ai::forecast_destination_for_ia(
                sim,
                &input,
                &self.script_domains.interactables.doors,
                &self.world.fast_grid.level.sectors,
                &self.world.fast_grid.level.sector_number_map,
            )
            .position;
            self.duty_go_near(
                sim,
                assets,
                source_id,
                destination,
                crate::parameters_ai::AI_TALK_DISTANCE,
                crate::ai::GotoFlags::RUN,
            );
            self.world
                .entities
                .expect_ai_controller_mut(source_id, format_args!("soldier alert timer"))
                .launch_timer(20, self.control.frame_counter);
        }
    }

    pub(super) fn process_synchronous_officer_reports_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) {
        let reports = self
            .world
            .entities
            .expect_ai_controller_mut(source_id, format_args!("officer-report source"))
            .take_pending_officer_reports();

        for report in reports {
            let crate::ai::CrossNpcAction::ReportBackToOfficer { officer, charly } = report else {
                unreachable!("take_pending_officer_reports returned a deferred action")
            };
            assert_eq!(
                source_id.index(),
                charly,
                "officer report source must be the reporting Charly"
            );

            let officer_id =
                self.expect_human_id_for_ai_handle(officer, "reporting Charly's officer");
            let officer_stimulus = crate::ai::Stimulus::with_human(
                crate::ai::StimulusType::CallMrOfficerIAmBack,
                charly,
            );
            let accepted = self.execute_ai_callback(sim, assets, officer_id, &officer_stimulus);

            let charly_id = self.expect_human_id_for_ai_handle(charly, "officer-report Charly");
            let enemy = self
                .world
                .entities
                .expect_enemy_ai_mut(charly_id, format_args!("reporting Charly {charly}"));
            let flow = enemy.resolve_charly_officer_report(self.control.frame_counter, accepted);
            if let Err(call) = flow {
                self.execute_ai_duty_call(sim, assets, charly_id, call);
            }
        }
    }
}
