use super::*;

impl EngineInner {
    /// Drain each NPC's `pending_self_stimuli` queue and re-dispatch each
    /// stimulus through `think` on the same frame.  Matches
    /// `Think()`-from-within-handler calls (MYTALK callbacks from
    /// `say()`, deferred `EventDone` from `SendCondolationCard`, etc.)
    /// which in the original engine immediately re-enter the AI but in
    /// Rust are queued to avoid nested `&mut AiGlobalState` borrows.
    ///
    /// Called unconditionally each tick.  Each NPC is drained to a fixed
    /// point so a Think call that recursively fires another self-stimulus
    /// observes that stimulus in the originating frame, matching the
    /// original direct `Think(...)` call.
    pub(in crate::engine) fn drain_pending_self_stimuli(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        let npc_ids: Vec<_> = self.world.entities.ai_owner_ids().collect();
        for npc_id in npc_ids {
            self.drain_self_stimuli_for_npc(sim, npc_id, assets);
        }
    }

    /// Per-NPC half of [`Self::drain_pending_self_stimuli`] — drains the
    /// pending self-stimulus queue for a single NPC and re-dispatches
    /// each through `think`.  Called both from the global end-of-tick
    /// drain and from [`Self::dispatch_think_with_drain`] so the
    /// re-entrant `think(EVENT_DONE)` that `send_condolation_card`
    /// fires lands inside the same call stack as the outer think.
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    pub(in crate::engine) fn drain_self_stimuli_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) {
        self.drain_self_stimuli_for_npc_mode(sim, npc_id, assets, false, false);
    }

    /// Native `SetAIState` StartThink/EndThink recursion must remain
    /// owner-local: forecasting unrelated actors here would advance their
    /// authoritative BuildingExitGate RNG before the native returns.
    pub(in crate::engine) fn drain_self_stimuli_for_npc_without_forecast(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) {
        self.drain_self_stimuli_for_npc_mode(sim, npc_id, assets, true, false);
    }

    fn drain_self_stimuli_for_npc_mode(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
        owner_local_no_forecast: bool,
        defer_turn_instruction: bool,
    ) {
        const MAX_REENTRANT_STIMULI: usize = 111;
        let mut dispatched = 0usize;

        loop {
            let queued_stimulus = {
                let Some(entity) = self.world.entities.get_mut(npc_id) else {
                    return;
                };
                let Some(ai) = entity.ai_controller_mut() else {
                    return;
                };
                if ai.outbox.reentrant.self_stimuli.is_empty() {
                    break;
                }
                ai.outbox.reentrant.self_stimuli.remove(0)
            };

            dispatched += 1;
            if dispatched > MAX_REENTRANT_STIMULI {
                tracing::warn!(
                    npc = npc_id.index(),
                    "self-stimulus recursion exceeded the original 111-call guard"
                );
                // The cascade is being force-abandoned with events still
                // queued, so no innermost EndThink will unwind the open
                // ancestor frames (`open_end_think_frames`) — close them
                // here so the depth cannot leak across frames.
                if let Some(ai) = self
                    .world
                    .entities
                    .get_mut(npc_id)
                    .and_then(Entity::ai_controller_mut)
                {
                    let open = std::mem::take(&mut ai.open_end_think_frames);
                    if open > 0 {
                        ai.think_recursion_depth = ai.think_recursion_depth.saturating_sub(open);
                    }
                }
                break;
            }

            let scratch = if owner_local_no_forecast {
                self.build_owner_context_scratch_without_forecast(assets)
            } else {
                self.build_sim_scratch(sim, assets)
            };
            let frame = self.control.frame_counter;
            let in_uninterruptible_command = self.is_very_very_busy(npc_id);
            let ctx = {
                let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
                    panic!("re-entrant self-Think NPC {} disappeared", npc_id.index())
                });
                let building_sector = self.entity_building_sector(entity.element_data().sector());
                let mut ctx = build_ai_context_from_entity(
                    entity,
                    frame,
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
                ctx.in_uninterruptible_command = in_uninterruptible_command;
                ctx
            };
            let stimulus = crate::ai::Stimulus::from_queued_self(queued_stimulus);
            if owner_local_no_forecast {
                match self.world.entities.get(npc_id) {
                    Some(entity) if entity.enemy_ai().is_some() => {
                        let tick_data = self.build_npc_tick_data(sim, npc_id, &scratch, assets);
                        self.dispatch_filtered_stimulus_without_forecast(
                            sim, assets, npc_id, &stimulus, &ctx, &tick_data,
                        );
                    }
                    Some(entity) if entity.friendly_ai().is_some() => {
                        self.dispatch_filtered_friendly_stimulus_without_forecast(
                            sim, assets, npc_id, &stimulus, &ctx,
                        );
                    }
                    Some(other) => panic!(
                        "owner-local self-stimulus recipient {} has invalid kind {:?}",
                        npc_id.index(),
                        other.element_data().kind
                    ),
                    None => panic!(
                        "owner-local self-stimulus recipient {} disappeared",
                        npc_id.index()
                    ),
                };
            } else {
                let tick_data = self.build_npc_tick_data(sim, npc_id, &scratch, assets);
                self.dispatch_filtered_stimulus(sim, assets, npc_id, &stimulus, &ctx, &tick_data);
            }

            // This path deliberately uses the raw filtered dispatch to avoid
            // recursively entering the outer fixed-point drain. Preserve the
            // same immediate StartThink boundary as the top-level wrapper:
            // publish eye/resurrection writes before waypoint or sibling
            // self-stimulus work continues.
            self.tick_ai_pending_resurrection_and_eyes_for_npc(npc_id);

            // A recursive Think can itself reach an authored waypoint. The
            // Original runs ReachPoint on this same call stack, before the
            // recursive Think's generic effects are allowed to escape.
            self.dispatch_pending_waypoint_script_for_owner(sim, npc_id, assets);

            // Original Think calls execute their engine-facing side effects
            // before returning.  Close that window after every recursive
            // stimulus so a newly launched sequence participates in
            // arbitration before the next sibling stimulus is delivered.
            self.drain_pending_for_npc_mode(
                sim,
                npc_id,
                assets,
                owner_local_no_forecast,
                defer_turn_instruction,
            );
            self.launch_pending_orders_for_npc_mode(sim, assets, npc_id, defer_turn_instruction);
            let _ = self.drain_pending_move_requests_for_owner(sim, npc_id);
            self.surface_synchronous_completion_events_for_owner(npc_id);
            self.process_synchronous_reentrant_actions_for_mode(
                sim,
                npc_id,
                assets,
                defer_turn_instruction,
            );
            self.dispatch_condolations_for_npc(sim, npc_id, assets);
        }

        let finish_macro = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::ai_controller_mut)
            .is_some_and(|ai| {
                std::mem::take(&mut ai.outbox.reentrant.finish_macro_after_self_stimuli)
            });
        if finish_macro {
            self.world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "post-reentrant macro owner {} lost its AI controller",
                        npc_id.index()
                    )
                })
                .finish_patrol_macro();
        }
    }

    // ── Per-waypoint ReachPoint dispatch ──────────────────────────
    //
    // Drain `pending_waypoint_script_reach_point` on every NPC:
    // dispatch `ReachPoint(actor)` on the waypoint's bound VM, then
    // synchronously re-enter `think(EventAfterScriptGoOn)` unless the
    // script transitioned the NPC into `DefaultScriptDriven`.  Runs
    // `execute_waypoint_script`, including the `script_enabled` gate
    // and the recursive `think()` call.  If no script is bound for
    // the waypoint (class missing), the recursive `think` still fires
    // — the "script was a no-op" branch when the bound class doesn't
    // transition state.
    pub(in crate::engine) fn dispatch_pending_waypoint_scripts(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        let owners: Vec<_> = self
            .world
            .entities
            .npcs()
            .filter_map(|(npc_id, entity)| {
                entity
                    .ai_controller()
                    .and_then(|ai| ai.outbox.reentrant.waypoint_script_reach_point)
                    .map(|_| EntityId::from(npc_id))
            })
            .collect();
        for owner in owners {
            self.dispatch_pending_waypoint_script_for_owner(sim, owner, assets);
        }
    }

    /// Close one NPC's authored waypoint callback on the same owner-local
    /// stack that selected it. `ExecuteWaypointScript` in the Original calls
    /// `ReachPoint` and then `Think(EventAfterScriptGoOn)` directly.
    pub(in crate::engine) fn dispatch_pending_waypoint_script_for_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        let request = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(|entity| entity.ai_controller_mut())
            .and_then(|ai| ai.outbox.reentrant.waypoint_script_reach_point.take());
        let Some((path_idx, wp_idx)) = request else {
            return;
        };
        if !sim.config().script_enabled {
            return;
        }

        self.with_suspended_waypoint_think(npc_id, |engine| {
            engine
                .dispatch_waypoint_script_on_suspended_think(sim, npc_id, assets, path_idx, wp_idx);
        });
    }

    /// Keep the route-arrival Think logically live while Rust releases its AI
    /// borrow to enter the waypoint VM. The restoration is unwind-safe so a
    /// script panic cannot leak a fake recursion level into later AI work.
    fn with_suspended_waypoint_think<T>(
        &mut self,
        npc_id: EntityId,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> T {
        // ExecuteWaypointScript is called from inside the route-arrival
        // Think in the Original. Rust has to release the AI borrow before it
        // can enter the waypoint VM, but native AI calls made by ReachPoint
        // must still observe that suspended outer Think. In particular, a
        // close-point GoTo sets `already_on_point` for the enclosing EndThink
        // instead of immediately queueing a second EVENT_REACHPOINT. The
        // recursively entered EVENT_AFTER_SCRIPT_GO_ON resets that latch in
        // StartThink, exactly as the C++ call stack does.
        {
            let ai = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "waypoint-script owner {} lost its AI before ReachPoint",
                        npc_id.index()
                    )
                });
            ai.think_recursion_depth = ai
                .think_recursion_depth
                .checked_add(1)
                .expect("waypoint-script suspended Think depth overflow");
        }

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(self)));
        if let Some(ai) = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::ai_controller_mut)
        {
            // This is the EndThink of the outer route-arrival Think that was
            // suspended while the waypoint VM and its recursive
            // EventAfterScriptGoOn ran.  Merely restoring the depth strands
            // completion latches produced by the final recursive action
            // (for example AssignNewPatrolPath(-1) returning to a post at the
            // actor's current position).  Original consumes those latches
            // here and recursively dispatches the matching event.
            assert!(
                ai.end_think_completion_events(),
                "waypoint-script suspended Think unexpectedly hit the typed recursion fallback"
            );
        }
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    fn dispatch_waypoint_script_on_suspended_think(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
        path_idx: crate::ai::PathId,
        wp_idx: u8,
    ) {
        let actor_handle = crate::natives::ScriptHandleCodec::actor_handle(npc_id);
        tracing::trace!(
            frame = self.control.frame_counter,
            owner = npc_id.index(),
            path = ?path_idx,
            wp = wp_idx,
            "waypoint ReachPoint dispatch"
        );
        if let Err(error) = self.call_script_vm(
            sim,
            assets,
            ScriptVmKey::Waypoint(path_idx, wp_idx),
            "ReachPoint",
            &[actor_handle],
            crate::natives::ScriptCallFrame::default(),
        ) {
            tracing::warn!(
                "Waypoint ReachPoint (path {path_idx}, wp {wp_idx}, actor {actor_handle}): {error}"
            );
            debug_assert!(
                false,
                "Waypoint ReachPoint (path {path_idx}, wp {wp_idx}, actor {actor_handle}): {error}"
            );
        }

        // The script may have spawned or deactivated entities, so rebuild the
        // context only after its VM call returns.
        let scratch = self.build_sim_scratch(sim, assets);
        let frame = self.control.frame_counter;
        let is_forest_level = self.world.weather.is_forest_level;
        let ambiance = self.world.weather.ambiance;
        let standard_view_polygon_radius = self.ai.standard_view_polygon_radius;
        let script_driven = self
            .world
            .entities
            .get(npc_id)
            .and_then(Entity::ai_controller)
            .is_none_or(|ai| ai.current_substate == crate::ai::Substate::DefaultScriptDriven);
        if script_driven {
            return;
        }
        let ctx = {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            build_ai_context_from_entity(
                entity,
                frame,
                None,
                is_forest_level,
                ambiance,
                standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.world.fast_grid,
                &assets.hiking_paths,
                &assets.hiking_waypoint_sectors,
                &self.ai.global.all_soldier_handles,
                self.control.sim_config.difficulty,
            )
        };
        let stimulus = crate::ai::Stimulus::new(crate::ai::StimulusType::EventAfterScriptGoOn);
        let tick_data = self.build_npc_tick_data(sim, npc_id, &scratch, assets);
        self.dispatch_think_with_drain(sim, npc_id, &stimulus, &ctx, &tick_data, assets);
        self.dispatch_synchronous_owner_moves(sim, assets, npc_id, &mut Vec::new())
            .unwrap_or_else(|error| {
                panic!(
                    "waypoint-script owner {} synchronous Move dispatch failed: {error:?}",
                    npc_id.index()
                )
            });
    }

    /// Finish engine-facing work queued by a direct AI method that is not
    /// itself entered through `dispatch_think_with_drain` (ambush checks,
    /// ladder recovery, The16thFrame, and macro continuation). This remains
    /// owner-local and includes the shared SetState/Say FIFO before later
    /// effects, orders, condolations, and recursive self-stimuli.
    pub(in crate::engine) fn drain_direct_ai_owner_boundary(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        self.drain_direct_ai_owner_boundary_mode(sim, npc_id, assets, false, false);
    }

    pub(in crate::engine) fn drain_direct_ai_owner_boundary_without_forecast(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        self.drain_direct_ai_owner_boundary_mode(sim, npc_id, assets, true, false);
    }

    pub(in crate::engine) fn drain_direct_ai_owner_boundary_without_forecast_deferred_instruct(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        self.drain_direct_ai_owner_boundary_mode(sim, npc_id, assets, true, true);
    }

    /// Continue common route-arrival code after its virtual SetState barrier.
    ///
    /// This is a fresh engine-facing context because `FilterAIEvent` may have
    /// synchronously reassigned the patrol path or otherwise mutated the
    /// actor before Original resumes the caller after `SetState`.
    pub(in crate::engine) fn resume_goto_route_reach_point_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
        owner_boundary_positions: &[(u32, crate::ai::Position)],
    ) {
        let mut scratch = self.build_owner_context_scratch_without_forecast(assets);
        let views = std::sync::Arc::make_mut(&mut scratch.ai_entity_views);
        for &(handle, position) in owner_boundary_positions {
            if let Some(view) = views.get_mut(&handle) {
                view.position = position;
            }
        }
        let frame = self.control.frame_counter;
        let in_uninterruptible_command = self.is_very_very_busy(npc_id);
        let mut ctx = {
            let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
                panic!(
                    "route-arrival continuation owner {} disappeared",
                    npc_id.index()
                )
            });
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            let mut ctx = build_ai_context_from_entity(
                entity,
                frame,
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
            ctx.in_uninterruptible_command = in_uninterruptible_command;
            ctx
        };
        self.refresh_selected_default_wait_identity(npc_id, &mut ctx);
        self.world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap_or_else(|| {
                panic!(
                    "route-arrival continuation owner {} lost its AI",
                    npc_id.index()
                )
            })
            .resume_goto_route_reach_point(sim, &ctx);

        // Original calls InitializePatrol inline, immediately after the
        // virtual SetState callback returns. Delaying this to the next
        // RHArtificialIntelligence::Hourglass changes which side of the
        // formation equally-close members occupy because later legacy slots
        // have moved by then.
        self.initialize_patrol_for_npc_from_owner_views(assets, npc_id, &scratch.ai_entity_views);
    }

    /// Invoke the concrete Enemy/Friendly `ReturnToDuty` override requested
    /// by shared AI code. Enemy queues its patrol-initialization continuation
    /// on the same owner FIFO; Friendly applies its busy gate before entering
    /// the common tail. In both cases the caller observes the complete virtual
    /// call before later owner work runs.
    pub(in crate::engine) fn virtual_return_to_duty_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
        flags: crate::ai::DutyFlags,
        owner_position_override: Option<crate::ai::Position>,
        owner_boundary_positions: &[(u32, crate::ai::Position)],
    ) {
        // Work already behind this virtual call belongs to the caller after
        // ReturnToDuty returns. Detach it so work emitted by the override
        // (notably ResumeReturnToDutyAfterPatrolInit) stays nested ahead of
        // that caller tail instead of being appended after it.
        let later_owner_work = {
            let ai = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| {
                    panic!("virtual ReturnToDuty owner {} lost its AI", npc_id.index())
                });
            std::mem::take(&mut ai.outbox.reentrant.owner_work)
        };
        let mut scratch = self.build_owner_context_scratch_without_forecast(assets);
        let views = std::sync::Arc::make_mut(&mut scratch.ai_entity_views);
        for &(handle, position) in owner_boundary_positions {
            if let Some(view) = views.get_mut(&handle) {
                view.position = position;
            }
        }

        let frame = self.control.frame_counter;
        let in_uninterruptible_command = self.is_very_very_busy(npc_id);
        let mut ctx = {
            let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
                panic!("virtual ReturnToDuty owner {} disappeared", npc_id.index())
            });
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            let mut ctx = build_ai_context_from_entity(
                entity,
                frame,
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
            ctx.in_uninterruptible_command = in_uninterruptible_command;
            if let Some(owner_position) = owner_position_override {
                ctx.position = owner_position;
            }
            ctx
        };
        self.refresh_selected_default_wait_identity(npc_id, &mut ctx);
        let is_enemy = self
            .world
            .entities
            .get(npc_id)
            .is_some_and(|entity| entity.enemy_ai().is_some());
        if is_enemy {
            let tick = self.build_npc_tick_data(sim, npc_id, &scratch, assets);
            self.world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::enemy_ai_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "virtual ReturnToDuty enemy owner {} lost its AI",
                        npc_id.index()
                    )
                })
                .return_to_duty(sim, flags, &ctx, &tick);
        } else {
            self.world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::friendly_ai_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "virtual ReturnToDuty owner {} has neither Enemy nor Friendly AI",
                        npc_id.index()
                    )
                })
                .return_to_duty(sim, flags, &ctx);
        }

        self.world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap_or_else(|| {
                panic!(
                    "virtual ReturnToDuty owner {} lost its AI after override",
                    npc_id.index()
                )
            })
            .outbox
            .reentrant
            .owner_work
            .extend(later_owner_work);
    }

    /// Run Enemy `ReturnToDuty`'s synchronous patrol initialization and then
    /// resume the common tail at the same owner boundary.
    pub(in crate::engine) fn resume_return_to_duty_after_patrol_init_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
        flags: crate::ai::DutyFlags,
        high_recursion_failsafe: bool,
        owner_boundary_positions: &[(u32, crate::ai::Position)],
    ) {
        let mut scratch = self.build_owner_context_scratch_without_forecast(assets);
        let views = std::sync::Arc::make_mut(&mut scratch.ai_entity_views);
        for &(handle, position) in owner_boundary_positions {
            if let Some(view) = views.get_mut(&handle) {
                view.position = position;
            }
        }

        // This is an inline C++ call, not a future Hourglass request. It must
        // settle before ReturnToDutyCommonStuff checks `patrol_chief` and may
        // issue the reciprocal visibility query.
        self.initialize_patrol_for_npc_from_owner_views(assets, npc_id, &scratch.ai_entity_views);

        let frame = self.control.frame_counter;
        let in_uninterruptible_command = self.is_very_very_busy(npc_id);
        let mut ctx = {
            let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
                panic!(
                    "return-to-duty continuation owner {} disappeared",
                    npc_id.index()
                )
            });
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            let mut ctx = build_ai_context_from_entity(
                entity,
                frame,
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
            ctx.in_uninterruptible_command = in_uninterruptible_command;
            if let Some((_, owner_position)) = owner_boundary_positions
                .iter()
                .find(|(handle, _)| *handle == npc_id.index())
            {
                ctx.position = *owner_position;
            }
            ctx
        };
        self.refresh_selected_default_wait_identity(npc_id, &mut ctx);
        self.world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::enemy_ai_mut)
            .unwrap_or_else(|| {
                panic!(
                    "return-to-duty continuation owner {} lost its Enemy AI",
                    npc_id.index()
                )
            })
            .resume_return_to_duty_after_patrol_init(sim, flags, &ctx, high_recursion_failsafe);
    }

    /// Run `CMD_PATROL_START`'s inline patrol rebuild, then continue the
    /// waypoint macro at the same owner boundary.
    pub(in crate::engine) fn resume_macro_after_patrol_init_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
        owner_boundary_positions: &[(u32, crate::ai::Position)],
    ) {
        let mut scratch = self.build_owner_context_scratch_without_forecast(assets);
        let views = std::sync::Arc::make_mut(&mut scratch.ai_entity_views);
        for &(handle, position) in owner_boundary_positions {
            if let Some(view) = views.get_mut(&handle) {
                view.position = position;
            }
        }

        self.initialize_patrol_for_npc_from_owner_views(assets, npc_id, &scratch.ai_entity_views);

        let frame = self.control.frame_counter;
        let in_uninterruptible_command = self.is_very_very_busy(npc_id);
        let mut ctx = {
            let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
                panic!("patrol-start macro owner {} disappeared", npc_id.index())
            });
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            let mut ctx = build_ai_context_from_entity(
                entity,
                frame,
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
            ctx.in_uninterruptible_command = in_uninterruptible_command;
            ctx
        };
        self.refresh_selected_default_wait_identity(npc_id, &mut ctx);
        self.world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap_or_else(|| panic!("patrol-start macro owner {} lost its AI", npc_id.index()))
            .execute_next_macro_command(sim, &ctx);
    }

    /// Run Original's `InitializePatrol` at a captured owner boundary.
    ///
    /// All fields are resolved from the live world after `FilterAIEvent`.
    /// Original's inline call observes earlier legacy slots after their actor
    /// tick and later slots before theirs; Rust's entity table is at that same
    /// owner boundary. The views captured when the stimulus was queued can be
    /// older than that boundary and must not drive patrol ordering.
    pub(in crate::engine) fn initialize_patrol_for_npc_from_owner_views(
        &mut self,
        assets: &LevelAssets,
        chief_id: EntityId,
        views: &crate::ai_entity_view::AiEntityViewMap,
    ) {
        let theoretical = self
            .world
            .entities
            .get(chief_id)
            .and_then(Entity::ai_controller)
            .unwrap_or_else(|| {
                panic!(
                    "synchronous patrol initialization owner {} has no AI",
                    chief_id.index()
                )
            })
            .theoretical_patrol
            .clone();
        self.initialize_patrol_for_npc_over_members(assets, chief_id, views, &theoretical);
    }

    /// `InitializePatrol` restricted to an explicit slice of theoretical
    /// members. `AddPatrolMember` runs one initialization per appended
    /// member, so each of its passes sees only the prefix of the theoretical
    /// list that existed at that point.
    pub(in crate::engine) fn initialize_patrol_for_npc_over_members(
        &mut self,
        assets: &LevelAssets,
        chief_id: EntityId,
        _views: &crate::ai_entity_view::AiEntityViewMap,
        theoretical: &[EntityId],
    ) {
        #[derive(Clone, Copy)]
        struct PatrolSnap {
            position: crate::ai::Position,
            raw_position_world: crate::coordinates::WorldPoint3D,
            detection_position_world: crate::coordinates::WorldPoint3D,
            direction: u16,
            posture: crate::element::Posture,
            is_rider: bool,
            in_building: bool,
            ai_state: crate::ai::AiState,
            is_alive: bool,
            is_active: bool,
            is_civilian: bool,
            is_able_to_fight: bool,
        }

        let chief_entity = self.world.entities.get(chief_id).unwrap_or_else(|| {
            panic!(
                "synchronous patrol initialization owner {} disappeared",
                chief_id.index()
            )
        });
        // `InitializePatrol` admits members through IsDetecting360Degrees,
        // whose distance gate is the post-RefreshView real radius, not the
        // pre-factor base radius the growing cone animates towards.
        let chief_real_view_radius = chief_entity
            .ai_actor_data()
            .unwrap_or_else(|| {
                panic!(
                    "synchronous patrol initialization owner {} has no AI actor data",
                    chief_id.index()
                )
            })
            .view_radius;

        let live_views = build_entity_views_without_forecast(self);
        let snapshot = |id: EntityId| {
            let view = live_views.get(&id.index()).unwrap_or_else(|| {
                panic!(
                    "synchronous patrol initialization owner {} lacks live view for member {}",
                    chief_id.index(),
                    id.index()
                )
            });
            let entity = self.world.entities.get(id).unwrap_or_else(|| {
                panic!(
                    "synchronous patrol initialization owner {} references missing member {}",
                    chief_id.index(),
                    id.index()
                )
            });
            PatrolSnap {
                position: view.position,
                raw_position_world: entity.element_data().position(),
                detection_position_world: view.detection_position_world,
                direction: entity.element_data().direction() as u16,
                posture: entity.element_data().posture,
                is_rider: entity.soldier_data().is_some_and(|soldier| soldier.rider),
                in_building: self.entity_data_in_building_sector(entity.element_data()),
                ai_state: entity
                    .ai_controller()
                    .unwrap_or_else(|| {
                        panic!(
                            "patrol member {} referenced by owner {} has no AI controller",
                            id.index(),
                            chief_id.index()
                        )
                    })
                    .current_state,
                is_alive: !entity.is_dead(),
                is_active: entity.is_active(),
                is_civilian: entity.is_civilian(),
                is_able_to_fight: match entity {
                    crate::element::Entity::Soldier(soldier) => {
                        use crate::element::Human as _;
                        soldier.is_able_to_fight()
                    }
                    crate::element::Entity::Pc(pc) => {
                        use crate::element::Human as _;
                        pc.is_able_to_fight()
                    }
                    _ => false,
                },
            }
        };

        let chief_snap = snapshot(chief_id);
        let obstacles_owned = self.build_ai_sight_obstacles(assets);
        let obstacles = obstacles_owned.list();
        let mut patrol = Vec::new();
        let mut missed = Vec::new();

        for &member in theoretical {
            if member == chief_id {
                continue;
            }
            let snap = snapshot(member);
            // Original evaluates IsDetecting360Degrees first in the `&&`
            // chain. An active, outdoor member therefore emits its LOS query
            // even when its later AI-state / able-to-fight gate rejects it.
            // Pre-gating visibility on those later predicates loses the
            // chief-to-member prefix and lets ReturnToDutyCommonStuff's
            // reciprocal member queries appear first in the frame trace.
            let admit = patrol_member_admitted(
                chief_snap.is_active && snap.is_active,
                || {
                    patrol_member_visible_from_raw_world(
                        chief_snap.detection_position_world,
                        chief_snap.is_rider,
                        chief_real_view_radius,
                        chief_snap.in_building,
                        snap.detection_position_world,
                        snap.posture,
                        snap.is_rider,
                        snap.direction as i16,
                        snap.in_building,
                        obstacles,
                    )
                },
                snap.ai_state,
                snap.is_civilian,
                snap.is_able_to_fight,
            );
            if admit {
                patrol.push((member, snap));
            } else if snap.is_alive {
                missed.push(member);
            }
        }

        let square_distance = |snap: PatrolSnap| {
            // `SquareDistance` subtracts the actors' raw 3-D GetPosition
            // values. In particular, it does not use AI `Position(actor)`,
            // which teleports a door-passing actor to its gate endpoint.
            let dx = snap.raw_position_world.x - chief_snap.raw_position_world.x;
            let dy = (snap.raw_position_world.y - chief_snap.raw_position_world.y)
                * crate::position_interface::INVERSE_ASPECT_RATIO;
            let dz = snap.raw_position_world.z - chief_snap.raw_position_world.z;
            dx * dx + dy * dy + dz * dz
        };
        let mut sorted: Vec<(EntityId, PatrolSnap)> = Vec::with_capacity(patrol.len());
        for entry in patrol {
            let distance = square_distance(entry.1);
            let insert_at = sorted
                .iter()
                .position(|existing| {
                    patrol_distance_inserts_before(distance, square_distance(existing.1))
                })
                .unwrap_or(sorted.len());
            sorted.insert(insert_at, entry);
        }
        for pair_end in (1..sorted.len()).step_by(2) {
            let even = sorted[pair_end - 1].1.position;
            let odd = sorted[pair_end].1.position;
            let ex = even.x - chief_snap.position.x;
            let ey = even.y - chief_snap.position.y;
            let ox = odd.x - chief_snap.position.x;
            let oy = odd.y - chief_snap.position.y;
            if ex * oy - ey * ox < 0.0 {
                sorted.swap(pair_end - 1, pair_end);
            }
        }

        let patrol_ids: Vec<_> = sorted.into_iter().map(|(id, _)| id).collect();
        {
            let ai = self
                .world
                .entities
                .get_mut(chief_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "synchronous patrol initialization owner {} lost its AI",
                        chief_id.index()
                    )
                });
            ai.needs_patrol_reinit = false;
            ai.patrol = patrol_ids.clone();
            ai.missed_patrol_members = missed;
        }
        for member in patrol_ids {
            self.world
                .entities
                .get_mut(member)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "patrol member {} admitted by owner {} lost its AI",
                        member.index(),
                        chief_id.index()
                    )
                })
                .patrol_chief = Some(chief_id);
        }

        // TODO: share this sorting/admission core with the initialization
        // paths that run before the per-owner hourglass instead of retaining
        // the equivalent delayed-path implementation in patrol coordination.
    }

    pub(in crate::engine) fn drain_direct_ai_owner_boundary_mode(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
        owner_local_no_forecast: bool,
        defer_turn_instruction: bool,
    ) {
        self.drain_direct_ai_owner_boundary_mode_inner(
            sim,
            npc_id,
            assets,
            owner_local_no_forecast,
            defer_turn_instruction,
            true,
        );
    }

    /// SetState's pre-callback actor prefix is only a statement boundary
    /// inside the enclosing Think.  Its caller tail is temporarily detached
    /// by `drain_ai_owner_work_for_mode`, so there is no complete EndThink
    /// boundary to surface until that tail has been restored and executed.
    pub(in crate::engine) fn drain_direct_ai_owner_prefix_boundary_mode(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
        owner_local_no_forecast: bool,
        defer_turn_instruction: bool,
    ) {
        self.drain_direct_ai_owner_boundary_mode_inner(
            sim,
            npc_id,
            assets,
            owner_local_no_forecast,
            defer_turn_instruction,
            false,
        );
    }

    fn drain_direct_ai_owner_boundary_mode_inner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
        owner_local_no_forecast: bool,
        defer_turn_instruction: bool,
        surface_completion: bool,
    ) {
        // This entry point models one direct, synchronous member-call stack.
        // Cards that were already queued for other owners belong to their
        // established later Hourglass boundaries; nested helpers below still
        // use the global drain because cards they create on this stack are
        // causal and must close re-entrantly. Detach only the pre-existing
        // foreign backlog for the duration of the fixed point.
        let pending = self.orders.sequence_manager.drain_pending_condolations();
        let (owner_roots, foreign_backlog): (Vec<_>, Vec<_>) = pending
            .into_iter()
            .partition(|dispatch| dispatch.card.owner == npc_id);
        self.orders
            .sequence_manager
            .restore_pending_condolations(owner_roots);

        const MAX_ITERS: u32 = 8;
        for iter in 0..MAX_ITERS {
            self.drain_pending_for_npc_boundary_mode(
                sim,
                npc_id,
                assets,
                owner_local_no_forecast,
                defer_turn_instruction,
                surface_completion,
            );
            self.launch_pending_orders_for_npc_mode(sim, assets, npc_id, defer_turn_instruction);
            let _ = self.drain_pending_move_requests_for_owner(sim, npc_id);
            if surface_completion {
                self.surface_synchronous_completion_events_for_owner(npc_id);
            }
            self.process_synchronous_reentrant_actions_for_mode(
                sim,
                npc_id,
                assets,
                defer_turn_instruction,
            );
            // All foreign cards that predated this direct boundary are held
            // aside above. Any foreign-owner card visible here was therefore
            // produced causally on this call stack and must close now.
            self.dispatch_condolations_for_npc(sim, npc_id, assets);
            let has_self_stimuli = {
                let ai = self
                    .world
                    .entities
                    .get(npc_id)
                    .unwrap_or_else(|| panic!("direct-drain NPC {} disappeared", npc_id.index()))
                    .ai_controller()
                    .unwrap_or_else(|| {
                        panic!("direct-drain NPC {} has no AI controller", npc_id.index())
                    });
                !ai.outbox.reentrant.self_stimuli.is_empty()
            };
            if has_self_stimuli {
                self.drain_self_stimuli_for_npc_mode(
                    sim,
                    npc_id,
                    assets,
                    owner_local_no_forecast,
                    defer_turn_instruction,
                );
            }

            let still_pending = {
                let ai = self
                    .world
                    .entities
                    .get(npc_id)
                    .unwrap_or_else(|| panic!("direct-drain NPC {} disappeared", npc_id.index()))
                    .ai_controller()
                    .unwrap_or_else(|| {
                        panic!("direct-drain NPC {} has no AI controller", npc_id.index())
                    });
                ai.outbox.actor.has_boundary_work()
                    || !ai.outbox.reentrant.self_stimuli.is_empty()
                    || !ai.outbox.reentrant.owner_work.is_empty()
                    || ai.has_pending_synchronous_cross_npc_actions()
            };
            if !still_pending {
                break;
            }
            assert!(
                iter + 1 < MAX_ITERS,
                "direct AI drain for NPC {} did not stabilise after {MAX_ITERS} passes",
                npc_id.index()
            );
        }

        self.orders
            .sequence_manager
            .restore_pending_condolations(foreign_backlog);
    }

    /// Apply one AI `StopAll` prefix as a synchronous owner boundary.
    ///
    /// Existing cards for unrelated owners belong to their established
    /// Hourglass slots. Cards produced while draining this owner's queued
    /// SetState/StopAll work are causal, including cross-owner callbacks, and
    /// remain visible to the ordinary global condolence drain.
    pub(in crate::engine) fn drain_ai_owner_halt_boundary(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        npc_id: EntityId,
    ) {
        let pending = self.orders.sequence_manager.drain_pending_condolations();
        let (owner_roots, foreign_backlog): (Vec<_>, Vec<_>) = pending
            .into_iter()
            .partition(|dispatch| dispatch.card.owner == npc_id);
        self.orders
            .sequence_manager
            .restore_pending_condolations(owner_roots);

        self.drain_ai_owner_work_for(sim, assets, npc_id);
        self.apply_pending_ai_halt(npc_id);
        self.dispatch_condolations_for_npc(sim, npc_id, assets);

        self.orders
            .sequence_manager
            .restore_pending_condolations(foreign_backlog);
    }

    // ── The16thFrame — periodic AI tasks (staggered) ──────────────
    //
    // `the_16th_frame` runs every 16th frame from the NPC's
    // `hourglass`, staggered by NPC index so not all soldiers run on
    // the same frame.

    pub(in crate::engine) fn tick_periodic_ai_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        let current_frame = self.control.frame_counter;

        let entity = self
            .world
            .entities
            .get(npc_id)
            .unwrap_or_else(|| panic!("periodic NPC {} disappeared", npc_id.index()));

        // Exact original phase:
        //   (frame & 255) - ((register_number + 100) & 255)
        // with unsigned-byte wrap. Passing the full phase matters:
        // The16thFrame uses bits 4..5 to reduce some work to every
        // 64th frame, so substituting `frame % 16` ran that work 4x.
        let register_number = entity
            .ai_actor_data()
            .unwrap_or_else(|| panic!("periodic entity {} is not an AI owner", npc_id.index()))
            .register_number;
        let frame_phase = npc_hourglass_frame_phase(current_frame, u32::from(register_number));
        if (frame_phase & 15) != 0 {
            return;
        }

        if entity.is_dead() {
            return;
        }

        // `sequence_element_is_about_to_be_launched(self, NULL)`.
        // Civilians consume this entry-time value directly. Enemy
        // The16thFrame can synchronously register work during
        // RefreshArrowProtection, so its stuck suffix re-reads the live
        // manager after closing that authored prefix below.
        let sequence_null_about_to_launch = self
            .orders
            .sequence_manager
            .element_is_about_to_be_launched(npc_id, crate::element::Command::Null);

        // `command == Wait` — entity is idle.  Read the live
        // sequence-element command via `actor_command` rather
        // than `action_state == Waiting` so we don't get a
        // false-positive on `WaitTimer` (which sets `action_state
        // = Waiting` via the animation map but is not
        // `Command::Wait`) or a false-negative on the brief
        // window where a teardown nulls the sequence-element
        // before the next animation tick resets `action_state`.
        let actor_command = self.actor_command(npc_id);
        let is_idle = actor_command == crate::element::Command::Wait;
        let receiving_wasp_sting = actor_command == crate::element::Command::ReceiveWaspSting;
        // C++ `RHElementActor::GetAnimation()` returns `mpOrder->action`, not
        // the sprite row most recently performed. A transition may complete
        // during Actor::Execute and promote its successor before NPC
        // Hourglass reaches The16thFrame; in that window `Sprite::last_action`
        // still names the transition.
        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        // The16thFrame's only combat-context consumer is
        // RefreshArrowProtection. Original gathers its live fighter data
        // without ForecastDestinationForIA, so resolving door exits here
        // would consume unrelated BuildingExitGate RNG merely because an
        // idle soldier reached its staggered periodic slot.
        let tick_data = if entity.enemy_ai().is_some() {
            self.build_npc_tick_data_without_forecasts(sim, npc_id, &scratch, assets)
        } else {
            crate::ai::AiPerTickData::stub()
        };

        let building_sector = self
            .world
            .entities
            .get(npc_id)
            .map(|entity| self.entity_building_sector(entity.element_data().sector()))
            .unwrap_or_else(|| panic!("periodic NPC {} disappeared", npc_id.index()));
        let entity =
            self.world.entities.get(npc_id).unwrap_or_else(|| {
                panic!("periodic NPC {} disappeared before call", npc_id.index())
            });

        let mut ctx = build_ai_context_from_entity(
            entity,
            current_frame,
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
        self.refresh_selected_default_wait_identity(npc_id, &mut ctx);

        let entity =
            self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
                panic!("periodic NPC {} disappeared before call", npc_id.index())
            });

        match entity {
            Entity::Pc(_) | Entity::Soldier(_) => {
                let has_stuck_suffix = entity
                    .enemy_ai_mut()
                    .unwrap_or_else(|| {
                        panic!("periodic soldier {} has no enemy AI", npc_id.index())
                    })
                    .the_16th_frame_before_stuck(
                        sim,
                        frame_phase,
                        &ctx,
                        &self.ai.global,
                        &tick_data,
                        Some(&self.world.fast_grid),
                        is_idle,
                        receiving_wasp_sting,
                    );

                if has_stuck_suffix {
                    self.finish_enemy_periodic_stuck_suffix_after_refresh(
                        sim,
                        npc_id,
                        assets,
                        frame_phase,
                        &ctx,
                    );
                }
            }
            Entity::Civilian(c) => {
                c.npc
                    .ai_brain
                    .friendly_mut()
                    .unwrap_or_else(|| {
                        panic!("periodic civilian {} has no friendly AI", npc_id.index())
                    })
                    .the_16th_frame(
                        frame_phase,
                        &mut self.ai.global,
                        &ctx,
                        is_idle,
                        sequence_null_about_to_launch,
                    );
                // `tick_data` is only used for enemies; civilians
                // don't need it.
                let _ = &tick_data;
            }
            _ => unreachable!("post-detection owner must remain an AI actor"),
        }
        self.drain_direct_ai_owner_boundary_without_forecast(sim, npc_id, assets);
    }

    /// Close RefreshArrowProtection's synchronous prefix and resume the
    /// every-64-frame watchdog at its exact live manager-query boundary.
    pub(in crate::engine) fn finish_enemy_periodic_stuck_suffix_after_refresh(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
        frame_phase: u8,
        ctx: &crate::ai::AiContext,
    ) {
        // RefreshArrowProtection's GoTo/SetState calls are synchronous in
        // Original. Materialize their manager registrations before the
        // following live GetCommand/SequenceElementIsAboutToBeLaunched reads,
        // but retain the enclosing direct-call completion boundary until the
        // suffix has run.
        self.drain_direct_ai_owner_prefix_boundary_mode(sim, npc_id, assets, true, false);
        let actor_command = self.actor_command(npc_id);
        let post_refresh_stuck_command_active = matches!(
            actor_command,
            crate::element::Command::Wait
                | crate::element::Command::SwordstrikeSmalltalkLeft
                | crate::element::Command::SwordstrikeSmalltalkRight
                | crate::element::Command::ParrySmalltalkLeft
                | crate::element::Command::ParrySmalltalkRight
        );
        let post_refresh_sequence_about_to_launch = self
            .orders
            .sequence_manager
            .element_is_about_to_be_launched(npc_id, crate::element::Command::Null);
        self.world
            .entities
            .get_mut(npc_id)
            .unwrap_or_else(|| panic!("periodic AI owner {} disappeared", npc_id.index()))
            .enemy_ai_mut()
            .unwrap_or_else(|| panic!("periodic AI owner {} lost enemy AI", npc_id.index()))
            .the_16th_frame_after_refresh(
                frame_phase,
                ctx,
                post_refresh_stuck_command_active,
                post_refresh_sequence_about_to_launch,
            );
    }

    /// Civilian `RandomSpeech(ubFramePhase)` call from NPC Hourglass.
    /// It sits before the lock gate and only acts at exact phase zero.
    pub(in crate::engine) fn tick_civilian_random_speech_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        let current_frame = self.control.frame_counter;
        let entity = self
            .world
            .entities
            .get(npc_id)
            .unwrap_or_else(|| panic!("random-speech NPC {} disappeared", npc_id.index()));
        let Entity::Civilian(civilian) = entity else {
            return;
        };
        let register_number = civilian.npc.register_number;
        let frame_phase = npc_hourglass_frame_phase(current_frame, u32::from(register_number));
        let debug_creation_order = {
            let config = civilian_random_speech_debug_config();
            if !config.enabled || config.frame != current_frame {
                None
            } else {
                let creation_order = self.world.original_creation_order(npc_id);
                (config.creation_order == creation_order).then_some(creation_order)
            }
        };
        if let Some(creation_order) = debug_creation_order {
            let crate::element::AiBrain::Friendly(ai) = &civilian.npc.ai_brain else {
                panic!(
                    "random-speech civilian {} has non-friendly AI",
                    npc_id.index()
                )
            };
            eprintln!(
                "[CIVRANDSPEECH frame={current_frame} co={creation_order} owner={} phase=eligibility register={register_number} frame_phase={frame_phase} human_hourglass_continued=true active={} profile={} civilian_type={:?} is_beggar={} dont_talk={} current_remark={:?} remark_flags={} ai_locks={:?} script_locked={} will_call={} will_draw_gate={}]",
                npc_id.index(),
                civilian.element.active,
                civilian.civilian.civilian_profile_index.0,
                civilian.civilian.cached_civilian_type,
                civilian.civilian.cached_civilian_type == crate::profiles::CivilianType::Beggar,
                ai.beggar_dont_talk_counter,
                ai.base.current_remark,
                ai.base.current_remark_flags,
                ai.base.locks_flag_field,
                ai.base.script_locked,
                frame_phase == 0,
                frame_phase == 0
                    && civilian.civilian.cached_civilian_type
                        == crate::profiles::CivilianType::Beggar
                    && ai.beggar_dont_talk_counter == 0
                    && ai.base.current_remark == crate::ai::Remark::TheSoundOfSilence,
            );
        }
        if frame_phase != 0 {
            return;
        }

        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        let building_sector = self.entity_building_sector(entity.element_data().sector());
        let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
            panic!(
                "random-speech NPC {} disappeared before call",
                npc_id.index()
            )
        });
        let ctx = build_ai_context_from_entity(
            entity,
            current_frame,
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
        if let Some(creation_order) = debug_creation_order {
            let Entity::Civilian(civilian) = &*entity else {
                panic!(
                    "random-speech civilian {} changed entity kind before call",
                    npc_id.index()
                )
            };
            let crate::element::AiBrain::Friendly(ai) = &civilian.npc.ai_brain else {
                panic!("random-speech civilian {} changed AI kind", npc_id.index())
            };
            let source_animation = ctx
                .entity_view(ai.base.me)
                .map(|view| view.current_animation);
            eprintln!(
                "[CIVRANDSPEECH frame={current_frame} co={creation_order} owner={} phase=before_call source_animation={source_animation:?} source_is_weeping={} live_animation={:?} owner_work_count={} owner_work={:?}]",
                npc_id.index(),
                source_animation == Some(crate::order::OrderType::Weeping),
                civilian.element.sprite.last_action,
                ai.base.outbox.reentrant.owner_work.len(),
                ai.base.outbox.reentrant.owner_work,
            );
        }
        {
            entity
                .friendly_ai_mut()
                .unwrap_or_else(|| panic!("civilian {} has no friendly AI", npc_id.index()))
                .random_speech(sim, 0, &ctx);
        }
        if let Some(creation_order) = debug_creation_order {
            let Entity::Civilian(civilian) =
                self.world.entities.get(npc_id).unwrap_or_else(|| {
                    panic!("random-speech civilian {} disappeared", npc_id.index())
                })
            else {
                panic!(
                    "random-speech civilian {} changed entity kind",
                    npc_id.index()
                )
            };
            let crate::element::AiBrain::Friendly(ai) = &civilian.npc.ai_brain else {
                panic!("random-speech civilian {} changed AI kind", npc_id.index())
            };
            eprintln!(
                "[CIVRANDSPEECH frame={current_frame} co={creation_order} owner={} phase=after_call_before_drain dont_talk={} current_remark={:?} remark_flags={} live_animation={:?} owner_work_count={} owner_work={:?}]",
                npc_id.index(),
                ai.beggar_dont_talk_counter,
                ai.base.current_remark,
                ai.base.current_remark_flags,
                civilian.element.sprite.last_action,
                ai.base.outbox.reentrant.owner_work.len(),
                ai.base.outbox.reentrant.owner_work,
            );
        }
        // Original RandomSpeech calls Say synchronously before the following
        // NPC lock gate. Rust's AI borrow records Say in owner_work, so close
        // that same owner-local boundary here even when the lock gate will
        // short-circuit the remainder of Hourglass.
        self.drain_direct_ai_owner_boundary_without_forecast(sim, npc_id, assets);
        if let Some(creation_order) = debug_creation_order {
            let Entity::Civilian(civilian) = self.world.entities.get(npc_id).unwrap_or_else(|| {
                panic!(
                    "random-speech civilian {} disappeared after drain",
                    npc_id.index()
                )
            }) else {
                panic!(
                    "random-speech civilian {} changed entity kind after drain",
                    npc_id.index()
                )
            };
            let crate::element::AiBrain::Friendly(ai) = &civilian.npc.ai_brain else {
                panic!("random-speech civilian {} changed AI kind", npc_id.index())
            };
            eprintln!(
                "[CIVRANDSPEECH frame={current_frame} co={creation_order} owner={} phase=after_drain current_remark={:?} remark_flags={} live_animation={:?} owner_work_count={} owner_work={:?}]",
                npc_id.index(),
                ai.base.current_remark,
                ai.base.current_remark_flags,
                civilian.element.sprite.last_action,
                ai.base.outbox.reentrant.owner_work.len(),
                ai.base.outbox.reentrant.owner_work,
            );
        }
    }

    // ── RefreshAmbushPoints — per-frame ambush peek scan ─────────
    //
    // `refresh_ambush_points` runs every frame for each NPC from
    // `hourglass`.  Civilians have a no-op virtual stub, so this only
    // fires for enemies (soldiers).  The per-NPC method updates the
    // slot status vector and may transition the AI substate via
    // `check_ambush_point`.

    pub(in crate::engine) fn tick_refresh_ambush_points_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        if self.actors_frozen() {
            return;
        }
        if self.ai.global.ambush_points.is_empty() {
            return;
        }

        // Civilian RefreshAmbushPoints is the Original virtual no-op. Check
        // that before scratch construction, which can draw BuildingExitGate
        // RNG while forecasting unrelated door-passing actors.
        let owner = self
            .world
            .entities
            .get(npc_id)
            .unwrap_or_else(|| panic!("ambush-refresh NPC {} disappeared", npc_id.index()));
        if matches!(owner, Entity::Civilian(_)) {
            return;
        }
        assert!(
            owner.enemy_ai().is_some(),
            "soldier {} has no enemy AI for ambush refresh",
            npc_id.index()
        );
        let scratch = self.build_owner_context_scratch_without_forecast(assets);

        let frame = self.control.frame_counter;
        let is_forest_level = self.world.weather.is_forest_level;
        let ambiance = self.world.weather.ambiance;
        let standard_view_polygon_radius = self.ai.standard_view_polygon_radius;
        // Phase 1: read-only — gather context + eyes point + LOS scope.
        let (ctx, eyes) = {
            let entity = self
                .world
                .entities
                .get(npc_id)
                .unwrap_or_else(|| panic!("ambush-refresh NPC {} disappeared", npc_id.index()));
            assert!(
                entity.enemy_ai().is_some(),
                "soldier {} has no enemy AI for ambush refresh",
                npc_id.index()
            );
            let eyes = entity.compute_eyes_point(None).unwrap_or_else(|| {
                panic!(
                    "soldier {} has no eye point for ambush refresh",
                    npc_id.index()
                )
            });
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            let ctx = build_ai_context_from_entity(
                entity,
                frame,
                building_sector,
                is_forest_level,
                ambiance,
                standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.world.fast_grid,
                &assets.hiking_paths,
                &assets.hiking_waypoint_sectors,
                &self.ai.global.all_soldier_handles,
                self.control.sim_config.difficulty,
            );
            (ctx, eyes)
        };

        // Build the obstacle view from individual disjoint fields
        // so the borrow checker can split it from the mut borrow
        // on `self.world.entities` below.
        let sight_obstacles = crate::sight_obstacle::ObstacleList {
            static_obstacles: assets.static_sight_obstacles.as_slice(),
            dynamic_obstacles: &self.world.dynamic_sight_obstacles,
            static_active: &self.world.static_sight_obstacle_active,
        };
        let ambush_points = self.ai.global.ambush_points.as_slice();

        let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
            panic!(
                "ambush-refresh NPC {} disappeared before apply",
                npc_id.index()
            )
        });
        entity
            .enemy_ai_mut()
            .unwrap_or_else(|| panic!("soldier {} lost enemy AI", npc_id.index()))
            .refresh_ambush_points(&ctx, eyes, ambush_points, sight_obstacles);
        self.drain_direct_ai_owner_boundary_without_forecast(sim, npc_id, assets);
    }

    // ── Macro timer hourglass ────────────────────────────────────
    //
    // `hourglass` polls `macro_timer_is_running` each frame and, when
    // the timer has rung and the NPC is still in
    // `SUBSTATE_DEFAULT_INMACRO`, calls `execute_next_macro_command(sim, )`
    // directly — **bypassing** the Think stimulus dispatch so
    // CMD_WAIT / CMD_BEND resume without going through EVENT_TIMER.
    //
    // We iterate both soldier and civilian NPCs because civilians use
    // the common macro opcodes too (REVERSE_PATH, WAIT, GOTO_POINT,
    // FACE_TO, ...).
    pub(in crate::engine) fn tick_ai_macro_timer_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        let current_frame = self.control.frame_counter;

        // Read macro-timer state without holding a borrow. The original stops
        // an elapsed macro timer even outside DefaultInMacro; only execution
        // is substate-gated.
        let (fire, execute) = {
            let entity = self
                .world
                .entities
                .get(npc_id)
                .unwrap_or_else(|| panic!("macro-timer NPC {} disappeared", npc_id.index()));
            let ai = entity.ai_controller().unwrap_or_else(|| {
                panic!("macro-timer NPC {} has no AI controller", npc_id.index())
            });
            let fire = ai.macro_timer_is_running && ai.when_does_macro_timer_ring <= current_frame;
            (
                fire,
                fire && ai.current_substate == crate::ai::Substate::DefaultInMacro,
            )
        };
        if !fire {
            return;
        }

        let scratch = self.build_owner_context_scratch_without_forecast(assets);

        // Build the AI context before we take the mut AI borrow.
        let building_sector = self
            .world
            .entities
            .get(npc_id)
            .map(|entity| self.entity_building_sector(entity.element_data().sector()))
            .unwrap_or_else(|| panic!("macro-timer NPC {} disappeared", npc_id.index()));
        let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
            panic!(
                "macro-timer NPC {} disappeared before execute",
                npc_id.index()
            )
        });
        let mut ctx = build_ai_context_from_entity(
            entity,
            current_frame,
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
        self.refresh_selected_default_wait_identity(npc_id, &mut ctx);

        // Stop the timer and resume the macro VM.  `execute_next_
        // macro_command` may transition the substate (e.g. to
        // `DefaultEnroute` when the byte stream ends) — we don't
        // post-process beyond that; any downstream state changes
        // ride the normal think dispatch.
        let base = self
            .world
            .entities
            .get_mut(npc_id)
            .unwrap_or_else(|| {
                panic!(
                    "macro-timer NPC {} disappeared before execute",
                    npc_id.index()
                )
            })
            .ai_controller_mut()
            .unwrap_or_else(|| panic!("macro-timer NPC {} lost its AI controller", npc_id.index()));
        base.macro_timer_is_running = false;
        if execute {
            base.execute_next_macro_command(sim, &ctx);
        }
        self.drain_direct_ai_owner_boundary_without_forecast(sim, npc_id, assets);
    }

    // ── Locked-frame timer bumps ─────────────────────────────────
    //
    // `hourglass` short-circuits the post-Refresh tail when any lock
    // is held (`locks_flag_field > 0 || script_locked || frozen_all`)
    // but still bumps `when_does_timer_ring`,
    // `when_does_macro_timer_ring`, and `emoticon_expiration_date`
    // per locked frame.  Without this, the per-piece tick guards
    // skip everything (no bumps), so ring-times shift -N once the
    // lock clears — a script-locked civilian's EVENT_TIMER would
    // fire immediately on unlock instead of N frames later.
    //
    // The decision returned here is the one and only lock sample for this
    // owner suffix. Once it is false, later The16thFrame/Think side effects
    // may acquire locks or FrozenAll without suppressing the already-entered
    // normal timer, macro timer, or emoticon phases. Only the retained FIFO
    // intentionally samples AI/script locks again before every item.
    /// Original `GetDeafness()` call immediately after
    /// `RefreshAmbushPoints`. This runs for every non-frozen owner even when
    /// acoustic detection's staggered cadence did not open this frame.
    pub(in crate::engine) fn tick_npc_refresh_deafness_for_npc(&mut self, npc_id: EntityId) {
        if self.actors_frozen() {
            return;
        }
        let (position, elevation) = {
            let entity =
                self.world.entities.get(npc_id).unwrap_or_else(|| {
                    panic!("deafness-refresh NPC {} disappeared", npc_id.index())
                });
            assert!(
                entity.ai_actor_data().is_some(),
                "deafness-refresh owner {} has no AI data",
                npc_id.index()
            );
            (
                entity.element_data().position_map(),
                entity.element_data().position().z,
            )
        };
        let cover_volume = self
            .feedback
            .sound_sim
            .sources
            .max_noise_covering_volume_for_3d(position.x, position.y, elevation);
        let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
            panic!(
                "deafness-refresh NPC {} disappeared before apply",
                npc_id.index()
            )
        });
        entity
            .ai_actor_data_mut()
            .unwrap_or_else(|| panic!("deafness-refresh owner {} lost AI data", npc_id.index()))
            .get_deafness(self.control.frame_counter, cover_volume);
    }

    pub(in crate::engine) fn tick_npc_lock_gate_for_npc(&mut self, npc_id: EntityId) -> bool {
        let frozen = self.actors_frozen();
        let entity = self
            .world
            .entities
            .get_mut(npc_id)
            .unwrap_or_else(|| panic!("lock-gate NPC {} disappeared", npc_id.index()));
        let ai = entity
            .ai_controller_mut()
            .unwrap_or_else(|| panic!("lock-gate NPC {} has no AI controller", npc_id.index()));
        let locked = frozen || !ai.locks_flag_field.is_empty() || ai.script_locked;
        if locked {
            // C++ UDWORD `++` wraps. Saturation would pin a deadline forever
            // after one overflow and break the later elapsed checks.
            ai.when_does_timer_ring = ai.when_does_timer_ring.wrapping_add(1);
            ai.when_does_macro_timer_ring = ai.when_does_macro_timer_ring.wrapping_add(1);
            ai.emoticon_expiration_date = ai.emoticon_expiration_date.wrapping_add(1);
        }
        locked
    }

    pub(in crate::engine) fn tick_npc_emoticon_expiration_for_npc(&mut self, npc_id: EntityId) {
        let current_frame = self.control.frame_counter;
        let entity = self
            .world
            .entities
            .get_mut(npc_id)
            .unwrap_or_else(|| panic!("emoticon-expiry NPC {} disappeared", npc_id.index()));
        let ai = entity.ai_controller_mut().unwrap_or_else(|| {
            panic!(
                "emoticon-expiry NPC {} has no AI controller",
                npc_id.index()
            )
        });
        if ai.emoticon_has_expiration_date && ai.emoticon_expiration_date <= current_frame {
            ai.set_emoticon(crate::ai::EmoticonType::None);
            assert!(!ai.emoticon_has_expiration_date);
        }
    }

    // ── Stuck-on-ladder emergency counter ────────────────────────
    //
    // `hourglass` bumps `stuck_on_ladder_emergency_counter` every
    // frame an NPC is on a ladder in a non-building sector with
    // command `Wait`/`MoveWaiting` and not script-locked; otherwise
    // resets to 0.  After 25 frames it calls `force_return_to_duty()`
    // (== `return_to_duty(sim, )`) and resets the counter so
    // outdoor-ladder hangs self-recover.
    //
    // Note: this checks only `script_locked`, *not* `locks_flag_field`
    // — so the freshly-set BUSY lock from the edge detector earlier in
    // the same frame does not suppress this counter (the BUSY lock is
    // exactly what we want to escape from).
    pub(in crate::engine) fn tick_npc_stuck_on_ladder_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        // Snapshot the gating predicates without holding a borrow.
        let entity = self
            .world
            .entities
            .get(npc_id)
            .unwrap_or_else(|| panic!("ladder-tail NPC {} disappeared", npc_id.index()));
        let on_ladder = entity.element_data().posture == crate::element::Posture::OnLadder;
        let cmd = self.actor_command(npc_id);
        let in_wait_or_move_waiting = matches!(
            cmd,
            crate::element::Command::Wait | crate::element::Command::MoveWaiting
        );
        let script_locked = entity
            .ai_controller()
            .unwrap_or_else(|| panic!("ladder-tail NPC {} has no AI", npc_id.index()))
            .script_locked;
        let in_building = self.entity_data_in_building_sector(entity.element_data());
        let qualifies = on_ladder && in_wait_or_move_waiting && !script_locked && !in_building;

        // Bump or reset the counter; remember whether to fire.
        let trigger = {
            let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
                panic!(
                    "ladder-tail NPC {} disappeared before counter",
                    npc_id.index()
                )
            });
            let npc = entity
                .ai_actor_data_mut()
                .unwrap_or_else(|| panic!("ladder-tail owner {} has no AI data", npc_id.index()));
            if qualifies {
                npc.stuck_on_ladder_emergency_counter =
                    npc.stuck_on_ladder_emergency_counter.saturating_add(1);
                if npc.stuck_on_ladder_emergency_counter > 25 {
                    npc.stuck_on_ladder_emergency_counter = 0;
                    true
                } else {
                    false
                }
            } else {
                npc.stuck_on_ladder_emergency_counter = 0;
                false
            }
        };
        if !trigger {
            return;
        }

        // `force_return_to_duty == return_to_duty`.  Dispatch via
        // the AI subclass to mirror the virtual call.  Build the
        // ctx + tick data the way `tick_periodic_ai` does.
        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        let tick_data = self.build_npc_tick_data(sim, npc_id, &scratch, assets);
        let frame = self.control.frame_counter;
        let in_uninterruptible_command = self.is_very_very_busy(npc_id);
        let building_sector = self
            .world
            .entities
            .get(npc_id)
            .map(|entity| self.entity_building_sector(entity.element_data().sector()))
            .unwrap_or_else(|| panic!("ladder-tail NPC {} disappeared", npc_id.index()));
        let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
            panic!(
                "ladder-tail NPC {} disappeared before recovery",
                npc_id.index()
            )
        });
        let mut ctx = build_ai_context_from_entity(
            entity,
            frame,
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
        ctx.in_uninterruptible_command = in_uninterruptible_command;
        self.refresh_selected_default_wait_identity(npc_id, &mut ctx);
        let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
            panic!(
                "ladder-tail NPC {} disappeared before recovery",
                npc_id.index()
            )
        });
        if let Some(enemy) = entity.enemy_ai_mut() {
            enemy.return_to_duty(sim, crate::ai::DutyFlags::empty(), &ctx, &tick_data);
        } else if let Some(friendly) = entity.friendly_ai_mut() {
            friendly.return_to_duty(sim, crate::ai::DutyFlags::empty(), &ctx);
        } else {
            panic!(
                "ladder-tail owner {} has neither enemy nor friendly AI",
                npc_id.index()
            );
        }
        self.drain_direct_ai_owner_boundary_without_forecast(sim, npc_id, assets);
    }
}
