use super::*;

impl EngineInner {
    /// Walk a patrol chief's members, delivering the whole-patrol stimulus to
    /// each one that the chief currently 360-degree detects.
    ///
    /// The detection gate is evaluated inside the loop, immediately before the
    /// member's own `think`, so a member whose visibility changed because an
    /// earlier member's dispatch moved somebody is judged on the state that
    /// dispatch left behind.
    pub(super) fn process_synchronous_patrol_member_relay_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        assets: &LevelAssets,
        defer_turn_instruction: bool,
    ) {
        let relays = self
            .world
            .entities
            .get_mut(source_id)
            .and_then(Entity::ai_controller_mut)
            .map(crate::ai::AiController::take_pending_patrol_member_relays)
            .unwrap_or_else(|| {
                panic!(
                    "patrol broadcast source {} has no AI controller",
                    source_id.index()
                )
            });

        for relay in relays {
            let crate::ai::CrossNpcAction::RelayStimulusToPatrolMembers {
                members,
                stimulus_type,
                info,
            } = relay
            else {
                unreachable!("patrol-broadcast drain returned a different cross-NPC action")
            };
            let mut stimulus = crate::ai::Stimulus::new(stimulus_type);
            stimulus.info = info;
            stimulus.to_whole_patrol = true;

            for member in members {
                let member_id = self.entity_id_for_index(member).unwrap_or_else(|| {
                    panic!(
                        "patrol broadcast from chief {} references missing member {member}",
                        source_id.index()
                    )
                });

                let scratch = self.build_owner_context_scratch_without_forecast(assets);
                let detected = {
                    let building_sector = self
                        .world
                        .entities
                        .get(source_id)
                        .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                        .unwrap_or_else(|| {
                            panic!("patrol broadcast chief {} disappeared", source_id.index())
                        });
                    let entity = self.world.entities.get(source_id).unwrap_or_else(|| {
                        panic!("patrol broadcast chief {} disappeared", source_id.index())
                    });
                    let chief_ctx = build_ai_context_from_entity(
                        entity,
                        self.control.frame_counter,
                        building_sector,
                        self.world.weather.is_forest_level,
                        self.world.weather.ambiance,
                        self.ai.standard_view_polygon_radius,
                        &scratch.ai_entity_views,
                        &scratch.ai_sight_obstacles,
                        &self.world.fast_grid,
                        &assets.hiking_paths,
                        &assets.hiking_waypoint_sectors,
                        &self.ai.global.all_soldier_handles,
                        self.control.sim_config.difficulty,
                    );
                    let chief_ai = entity.enemy_ai().unwrap_or_else(|| {
                        panic!(
                            "patrol broadcast chief {} is not an enemy soldier",
                            source_id.index()
                        )
                    });
                    chief_ai.detects_patrol_member_360(member, &chief_ctx)
                };
                tracing::trace!(
                    target: "patrol_relay",
                    chief = source_id.index(),
                    member,
                    ?stimulus_type,
                    detected,
                    "patrol broadcast member gate"
                );
                if !detected {
                    continue;
                }

                let building_sector = self
                    .world
                    .entities
                    .get(member_id)
                    .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                    .unwrap_or_else(|| panic!("patrol broadcast member {member} disappeared"));
                let ctx = {
                    let entity =
                        self.world.entities.get(member_id).unwrap_or_else(|| {
                            panic!("patrol broadcast member {member} disappeared")
                        });
                    build_ai_context_from_entity(
                        entity,
                        self.control.frame_counter,
                        building_sector,
                        self.world.weather.is_forest_level,
                        self.world.weather.ambiance,
                        self.ai.standard_view_polygon_radius,
                        &scratch.ai_entity_views,
                        &scratch.ai_sight_obstacles,
                        &self.world.fast_grid,
                        &assets.hiking_paths,
                        &assets.hiking_waypoint_sectors,
                        &self.ai.global.all_soldier_handles,
                        self.control.sim_config.difficulty,
                    )
                };
                let tick_data = self.build_npc_tick_data(sim, member_id, &scratch, assets);
                self.dispatch_think_with_drain_mode(
                    sim,
                    member_id,
                    &stimulus,
                    &ctx,
                    &tick_data,
                    assets,
                    true,
                    defer_turn_instruction,
                );
            }
        }
    }

    /// Invoke a patrol chief's dispatch routine exactly as the subordinate's
    /// direct C++ call does. The routine's return value, rather than a
    /// prediction from the chief's state, decides whether the subordinate
    /// resumes its local handler.
    pub(super) fn process_synchronous_patrol_dispatch_requests_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        assets: &LevelAssets,
        defer_turn_instruction: bool,
    ) {
        let requests = self
            .world
            .entities
            .get_mut(source_id)
            .and_then(Entity::ai_controller_mut)
            .map(crate::ai::AiController::take_pending_patrol_dispatch_requests)
            .unwrap_or_else(|| {
                panic!(
                    "patrol-dispatch source {} has no AI controller",
                    source_id.index()
                )
            });

        for request in requests {
            let crate::ai::CrossNpcAction::RequestPatrolDispatch {
                chief,
                caller,
                stimulus_type,
                info,
            } = request
            else {
                unreachable!("patrol-dispatch drain returned a different action")
            };
            assert_eq!(
                source_id.index(),
                caller,
                "patrol-dispatch caller must be its owner"
            );
            let chief_id = self.entity_id_for_index(chief).unwrap_or_else(|| {
                panic!(
                    "synchronous patrol dispatch from NPC {caller} references missing chief {chief}"
                )
            });
            if !matches!(self.world.entities.get(chief_id), Some(Entity::Soldier(s)) if s.npc.ai_brain.enemy().is_some())
            {
                panic!("patrol chief {chief} is not an enemy soldier");
            }

            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let chief_building_sector = self
                .world
                .entities
                .get(chief_id)
                .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                .unwrap_or_else(|| panic!("patrol chief {chief} disappeared"));
            let mut chief_ctx = {
                let entity = self
                    .world
                    .entities
                    .get(chief_id)
                    .unwrap_or_else(|| panic!("patrol chief {chief} disappeared"));
                build_ai_context_from_entity(
                    entity,
                    self.control.frame_counter,
                    chief_building_sector,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &assets.hiking_waypoint_sectors,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                )
            };
            self.refresh_selected_default_wait_identity(chief_id, &mut chief_ctx);
            let chief_tick = self.build_npc_tick_data(sim, chief_id, &scratch, assets);
            let mut stimulus = crate::ai::Stimulus::new(stimulus_type);
            stimulus.info = info;
            chief_ctx.seed_view_radius_cache(&self.ai.view_radius_cache);
            let dispatched = {
                let global = &mut self.ai.global;
                let grid = &self.world.fast_grid;
                self.world
                    .entities
                    .get_mut(chief_id)
                    .and_then(Entity::enemy_ai_mut)
                    .unwrap_or_else(|| panic!("patrol chief {chief} lost its EnemyAi"))
                    .dispatch_stimulus_to_whole_patrol(
                        sim,
                        &stimulus,
                        global,
                        &chief_ctx,
                        &chief_tick,
                        Some(grid),
                    )
            };
            chief_ctx.commit_view_radius_cache(&mut self.ai.view_radius_cache);

            // A successful chief routine can recursively Think and queue the
            // member walk. Close those effects before the direct call returns.
            if dispatched {
                self.drain_direct_ai_owner_boundary_mode(
                    sim,
                    chief_id,
                    assets,
                    true,
                    defer_turn_instruction,
                );
                continue;
            }

            // The caller's outer handler resumes after the chief returned
            // false. Re-enter with the patrol flag set solely as a recursion
            // guard; no other handler observes that flag.
            let caller_scratch = self.build_owner_context_scratch_without_forecast(assets);
            let caller_building_sector = self
                .world
                .entities
                .get(source_id)
                .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                .unwrap_or_else(|| panic!("patrol-dispatch caller {caller} disappeared"));
            let caller_ctx = {
                let entity = self
                    .world
                    .entities
                    .get(source_id)
                    .unwrap_or_else(|| panic!("patrol-dispatch caller {caller} disappeared"));
                build_ai_context_from_entity(
                    entity,
                    self.control.frame_counter,
                    caller_building_sector,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &caller_scratch.ai_entity_views,
                    &caller_scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &assets.hiking_waypoint_sectors,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                )
            };
            let caller_tick = self.build_npc_tick_data(sim, source_id, &caller_scratch, assets);
            stimulus.to_whole_patrol = true;
            self.dispatch_think_with_drain_mode(
                sim,
                source_id,
                &stimulus,
                &caller_ctx,
                &caller_tick,
                assets,
                true,
                defer_turn_instruction,
            );
        }
    }

    pub(in crate::engine) fn process_synchronous_think_results_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        assets: &LevelAssets,
        defer_turn_instruction: bool,
    ) {
        let requests = self
            .world
            .entities
            .get_mut(source_id)
            .and_then(Entity::ai_controller_mut)
            .map(|ai| {
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
            })
            .unwrap_or_else(|| {
                panic!(
                    "Think-result source {} has no AI controller",
                    source_id.index()
                )
            });

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
            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let target_building_sector = self
                .world
                .entities
                .get(target_id)
                .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                .unwrap_or_else(|| panic!("{stimulus_type:?} target {target} disappeared"));
            let target_ctx = {
                let entity = self
                    .world
                    .entities
                    .get(target_id)
                    .unwrap_or_else(|| panic!("{stimulus_type:?} target {target} disappeared"));
                build_ai_context_from_entity(
                    entity,
                    self.control.frame_counter,
                    target_building_sector,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &assets.hiking_waypoint_sectors,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                )
            };
            let target_tick = self.build_npc_tick_data(sim, target_id, &scratch, assets);
            let mut stimulus = crate::ai::Stimulus::new(stimulus_type);
            stimulus.info = info;
            let accepted = self.dispatch_think_with_drain_without_forecast(
                sim,
                target_id,
                &stimulus,
                &target_ctx,
                &target_tick,
                assets,
            );

            let source_scratch = self.build_owner_context_scratch_without_forecast(assets);
            let source_building_sector = self
                .world
                .entities
                .get(source_id)
                .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                .unwrap_or_else(|| panic!("Think-result caller {caller} disappeared"));
            let mut source_ctx = {
                let entity = self
                    .world
                    .entities
                    .get(source_id)
                    .unwrap_or_else(|| panic!("Think-result caller {caller} disappeared"));
                build_ai_context_from_entity(
                    entity,
                    self.control.frame_counter,
                    source_building_sector,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &source_scratch.ai_entity_views,
                    &source_scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &assets.hiking_waypoint_sectors,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                )
            };
            self.refresh_selected_default_wait_identity(source_id, &mut source_ctx);
            let source_tick = self.build_npc_tick_data(sim, source_id, &source_scratch, assets);
            let global = &mut self.ai.global;
            let grid = &self.world.fast_grid;
            self.world
                .entities
                .get_mut(source_id)
                .and_then(Entity::enemy_ai_mut)
                .unwrap_or_else(|| panic!("Think-result caller {caller} lost its EnemyAi"))
                .resolve_think_result(
                    sim,
                    accepted,
                    target,
                    continuation,
                    global,
                    Some(grid),
                    &source_ctx,
                    &source_tick,
                );

            // The continuation is the caller's C++ stack frame resuming
            // immediately after `target->Think(...)` returned. The officer's
            // single-soldier call can reject into ReturnToDuty, whose virtual
            // SetState and following GoTo synchronously publish actor work as
            // well as owner callbacks. Close that exact caller stack through
            // the full owner-local fixed point. Other result continuations
            // retain their narrower owner-work boundary because their outer
            // loops still own subsequent member calls.
            if matches!(
                continuation,
                crate::ai::ThinkResultContinuation::OfficerCalledSoldier
            ) {
                self.drain_direct_ai_owner_boundary_mode(
                    sim,
                    source_id,
                    assets,
                    true,
                    defer_turn_instruction,
                );
            } else {
                self.drain_ai_owner_work_for_mode(
                    sim,
                    assets,
                    source_id,
                    true,
                    defer_turn_instruction,
                );
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
            .get_mut(source_id)
            .and_then(Entity::ai_controller_mut)
            .map(crate::ai::AiController::take_pending_alert_requests)
            .unwrap_or_else(|| {
                panic!(
                    "CALL_ALERT source {} has no AI controller",
                    source_id.index()
                )
            });

        for request in requests {
            let crate::ai::CrossNpcAction::RequestAlert {
                target,
                caller,
                continuation,
            } = request
            else {
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
            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let target_building_sector = self
                .world
                .entities
                .get(target_id)
                .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                .unwrap_or_else(|| panic!("CALL_ALERT target {target} disappeared"));
            let target_ctx = {
                let entity = self
                    .world
                    .entities
                    .get(target_id)
                    .unwrap_or_else(|| panic!("CALL_ALERT target {target} disappeared"));
                build_ai_context_from_entity(
                    entity,
                    self.control.frame_counter,
                    target_building_sector,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &assets.hiking_waypoint_sectors,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                )
            };
            let target_tick = self.build_npc_tick_data(sim, target_id, &scratch, assets);
            let accepted = self.dispatch_think_with_drain_without_forecast(
                sim,
                target_id,
                &crate::ai::Stimulus::with_human(crate::ai::StimulusType::CallAlert, caller),
                &target_ctx,
                &target_tick,
                assets,
            );

            let source_scratch = self.build_owner_context_scratch_without_forecast(assets);
            let source_building_sector = self
                .world
                .entities
                .get(source_id)
                .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                .unwrap_or_else(|| panic!("CALL_ALERT caller {caller} disappeared"));
            let mut source_ctx = {
                let entity = self
                    .world
                    .entities
                    .get(source_id)
                    .unwrap_or_else(|| panic!("CALL_ALERT caller {caller} disappeared"));
                build_ai_context_from_entity(
                    entity,
                    self.control.frame_counter,
                    source_building_sector,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &source_scratch.ai_entity_views,
                    &source_scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &assets.hiking_waypoint_sectors,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                )
            };
            self.refresh_selected_default_wait_identity(source_id, &mut source_ctx);
            let source_tick = self.build_npc_tick_data(sim, source_id, &source_scratch, assets);
            match continuation {
                crate::ai::AlertContinuation::CivilianReachedSoldier
                | crate::ai::AlertContinuation::CivilianSawSoldier => self
                    .world
                    .entities
                    .get_mut(source_id)
                    .and_then(Entity::friendly_ai_mut)
                    .unwrap_or_else(|| {
                        panic!("civilian CALL_ALERT caller {caller} lost its FriendlyAi")
                    })
                    .resolve_alert_request(sim, accepted, continuation, &source_ctx),
                crate::ai::AlertContinuation::SoldierSawOfficer => self
                    .world
                    .entities
                    .get_mut(source_id)
                    .and_then(Entity::enemy_ai_mut)
                    .unwrap_or_else(|| {
                        panic!("soldier CALL_ALERT caller {caller} lost its EnemyAi")
                    })
                    .resolve_alert_request(sim, accepted, continuation, &source_ctx, &source_tick),
            }
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
            .get_mut(source_id)
            .and_then(Entity::ai_controller_mut)
            .map(crate::ai::AiController::take_pending_officer_reports)
            .unwrap_or_else(|| {
                panic!(
                    "officer-report source {} has no AI controller",
                    source_id.index()
                )
            });

        for report in reports {
            let crate::ai::CrossNpcAction::ReportBackToOfficer { officer, charly } = report else {
                unreachable!("take_pending_officer_reports returned a deferred action")
            };
            assert_eq!(
                source_id.index(),
                charly,
                "officer report source must be the reporting Charly"
            );

            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let officer_id =
                self.expect_human_id_for_ai_handle(officer, "reporting Charly's officer");
            let officer_building_sector = self
                .world
                .entities
                .get(officer_id)
                .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                .unwrap_or_else(|| panic!("reporting Charly requires missing officer {officer}"));
            let officer_ctx = {
                let entity = self.world.entities.get(officer_id).unwrap_or_else(|| {
                    panic!("reporting Charly requires missing officer {officer}")
                });
                build_ai_context_from_entity(
                    entity,
                    self.control.frame_counter,
                    officer_building_sector,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &assets.hiking_waypoint_sectors,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                )
            };
            let officer_tick = self.build_npc_tick_data(sim, officer_id, &scratch, assets);
            let officer_stimulus = crate::ai::Stimulus::with_human(
                crate::ai::StimulusType::CallMrOfficerIAmBack,
                charly,
            );
            let accepted = self.dispatch_think_with_drain_without_forecast(
                sim,
                officer_id,
                &officer_stimulus,
                &officer_ctx,
                &officer_tick,
                assets,
            );

            let charly_id = self.expect_human_id_for_ai_handle(charly, "officer-report Charly");
            let charly_building_sector = self
                .world
                .entities
                .get(charly_id)
                .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                .unwrap_or_else(|| panic!("officer response requires missing Charly {charly}"));
            let mut charly_ctx = {
                let entity =
                    self.world.entities.get(charly_id).unwrap_or_else(|| {
                        panic!("officer response requires missing Charly {charly}")
                    });
                build_ai_context_from_entity(
                    entity,
                    self.control.frame_counter,
                    charly_building_sector,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &assets.hiking_waypoint_sectors,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                )
            };
            self.refresh_selected_default_wait_identity(charly_id, &mut charly_ctx);
            let charly_tick = self.build_npc_tick_data(sim, charly_id, &scratch, assets);
            let entity = self
                .world
                .entities
                .get_mut(charly_id)
                .unwrap_or_else(|| panic!("officer response requires missing Charly {charly}"));
            let enemy = entity
                .enemy_ai_mut()
                .unwrap_or_else(|| panic!("reporting Charly {charly} requires enemy AI"));
            enemy.resolve_charly_officer_report(sim, accepted, &charly_ctx, &charly_tick);
        }
    }
}
