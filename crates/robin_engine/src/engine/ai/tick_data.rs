use super::*;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum AiObservationKind {
    Current,
}

/// Non-authoritative measurement. Enable with
/// `RUST_LOG=robin_engine::ai_view_build=debug`. Counts describe actual rebuilt
/// views and overlay lengths; they deliberately do not pretend to measure
/// heap allocations or recursively owned bytes.
fn observation_build_started() -> Option<web_time::Instant> {
    tracing::enabled!(target: "robin_engine::ai_view_build", tracing::Level::DEBUG)
        .then(web_time::Instant::now)
}

fn observe_view_build(
    started: Option<web_time::Instant>,
    frame: u32,
    mode: AiObservationKind,
    entity_slots: usize,
    rebuilt_views: usize,
    scratch: &SimScratch,
) {
    if let Some(started) = started {
        tracing::debug!(
            target: "robin_engine::ai_view_build",
            frame,
            ?mode,
            entity_slots,
            rebuilt_views,
            view_count = scratch.ai_entity_views.entities.len(),
            dynamic_obstacles = scratch.ai_sight_obstacles.dynamic_obstacles.len(),
            static_overlay_entries = scratch.ai_sight_obstacles.static_active.len(),
            elapsed_ns = started.elapsed().as_nanos() as u64,
            "AI observation prepared"
        );
    }
}

impl EngineInner {
    #[inline(never)]
    pub(in crate::engine) fn debug_building_exit_wait_event_view(
        &self,
        owner: EntityId,
        queue_index: usize,
        stimulus: &crate::ai::Stimulus,
    ) {
        if !building_exit_wait_owner_debug_enabled()
            || stimulus.stimulus_type != crate::ai::StimulusType::EventView
        {
            return;
        }
        let (target_handle, target_creation_order) = match stimulus.info {
            crate::ai::StimulusInfo::Human(handle) => (
                Some(handle),
                self.entity_id_for_index(handle.get())
                    .map(|target| self.world.original_creation_order(target)),
            ),
            _ => (None, None),
        };
        eprintln!(
            "BEXITWAIT {{\"event\":\"queued_event_view\",\"frame\":{},\"owner\":{:?},\"owner_creation_order\":{},\"queue_index\":{queue_index},\"target_handle\":{target_handle:?},\"target_creation_order\":{target_creation_order:?}}}",
            self.control.frame_counter,
            owner,
            self.world.original_creation_order(owner),
        );
    }

    #[inline(never)]
    pub(in crate::engine) fn debug_building_exit_wait_pc_route(
        &self,
        owner: EntityId,
        source_sector: crate::position_interface::SectorHandle,
        goal_sector: crate::position_interface::SectorHandle,
    ) {
        if !building_exit_wait_owner_debug_enabled() {
            return;
        }
        eprintln!(
            "BEXITWAIT {{\"event\":\"pc_door_fight_route\",\"frame\":{},\"owner\":{:?},\"owner_creation_order\":{},\"source_sector\":{},\"goal_sector\":{}}}",
            self.control.frame_counter,
            owner,
            self.world.original_creation_order(owner),
            source_sector.get(),
            goal_sector.get(),
        );
    }

    #[inline(never)]
    pub(in crate::engine) fn debug_refresh_view_lifecycle(
        &self,
        stage: &str,
        npc_id: EntityId,
        derived_tail_order_type: Option<crate::order::OrderType>,
    ) {
        let gate = refresh_view_lifecycle_debug_gate();
        if !gate.enabled()
            || self.control.frame_counter < gate.filter(0).unwrap_or(0)
            || self.control.frame_counter > gate.filter(1).unwrap_or(u32::MAX)
        {
            return;
        }
        let creation_order = self.world.original_creation_order(npc_id);
        if !gate.matches([None, None, Some(creation_order)]) {
            return;
        }
        let entity = self
            .world
            .entities
            .expect_entity(npc_id, format_args!("RVLIFE owner at stage {stage}"));
        let Some(npc) = entity.ai_actor_data() else {
            return;
        };
        let actor = entity.actor_data().unwrap_or_else(|| {
            panic!(
                "RVLIFE owner {} is not an actor at stage {stage}",
                npc_id.index()
            )
        });
        let human = entity.human_data().unwrap_or_else(|| {
            panic!(
                "RVLIFE owner {} is not human at stage {stage}",
                npc_id.index()
            )
        });
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let installed_order = actor
            .installed_order
            .map(|order| order.order_type as u32)
            .map_or(-1_i64, i64::from);
        let derived_tail_order = derived_tail_order_type
            .map(|order| order as u32)
            .map_or(-1_i64, i64::from);
        let direction = entity.element_data().direction();
        let view_direction = npc.view_direction;
        let left_side = npc.view_left_side;
        let right_side = npc.view_right_side;
        let half_aperture = npc.real_half_aperture;
        let angle = npc.view_angle;
        let angle_step = npc.view_angle_step;
        eprintln!(
            "RVLIFE {{\"engine\":\"rust\",\"seq\":{sequence},\"stage\":{stage:?},\"frame\":{},\"owner_slot\":{},\"creation_order\":{creation_order},\"eye_status\":{},\"alpha_start\":{},\"radius_goal\":{},\"radius_step\":{},\"radius\":{},\"active\":{},\"unconscious\":{},\"tied\":{},\"dead\":{},\"frozen_all\":{},\"installed_order\":{installed_order},\"derived_tail_order\":{derived_tail_order},\"motion_state\":{},\"execution_frozen\":{},\"direction\":{direction},\"direction_old\":{},\"view_transition\":{},\"angle_bits\":{},\"angle_step_bits\":{},\"real_half_aperture_bits\":{},\"view_direction_bits\":[{},{}],\"left_side_bits\":[{},{}],\"right_side_bits\":[{},{}]}}",
            self.control.frame_counter,
            npc_id.index(),
            npc.eye_status as u8,
            npc.view_alpha_start,
            npc.view_radius_goal,
            npc.view_radius_step,
            npc.view_radius,
            entity.element_data().active,
            human.unconscious,
            entity.element_data().posture() == crate::element::Posture::Tied,
            entity.is_dead(),
            self.actors_frozen(),
            actor.continuation.motion_state as u8,
            actor.execution_frozen,
            npc.direction_old,
            npc.view_transition,
            angle.to_bits(),
            angle_step.to_bits(),
            half_aperture.to_bits(),
            view_direction[0].to_bits(),
            view_direction[1].to_bits(),
            left_side[0].to_bits(),
            left_side[1].to_bits(),
            right_side[0].to_bits(),
            right_side[1].to_bits(),
        );
    }

    /// Refresh the original actor's selected wait-element identity
    /// on a context immediately before an AI call that may project deferred
    /// Halt effects. The legacy ownership decoder proves that only Wait and
    /// historical Freeze elements may occupy that pointer.
    pub(in crate::engine) fn refresh_selected_default_wait_identity(
        &self,
        entity_id: EntityId,
        ctx: &mut crate::ai::AiContext,
    ) {
        let selected = self
            .orders
            .sequence_manager
            .current_element_for_actor(entity_id)
            .and_then(|(sequence_id, element_index)| {
                self.orders
                    .sequence_manager
                    .get_element(sequence_id, element_index)
            });
        ctx.self_selected_element_is_default_wait = Some(selected.is_some_and(|element| {
            matches!(
                element.command,
                crate::element::Command::Wait | crate::element::Command::Freeze
            )
        }));
        ctx.self_selected_element_priority = Some(selected.map(|element| element.priority));
    }

    /// Resolve an AI `HumanHandle` back through the original sparse element
    /// table without inventing an entity kind.  AI still stores these handles
    /// as raw slots, so a target can be a PC, soldier, or civilian.
    pub(in crate::engine) fn expect_human_id_for_ai_handle(
        &self,
        handle: crate::ai::HumanHandle,
        context: &str,
    ) -> EntityId {
        let id = self.expect_entity_id_for_index(handle, context);
        assert!(
            self.world
                .entities
                .get(id)
                .is_some_and(crate::element::Entity::is_human),
            "{context}: entity in raw slot {handle} is not human"
        );
        id
    }

    pub(super) fn build_ai_sight_obstacles(
        &self,
        assets: &LevelAssets,
    ) -> crate::sight_obstacle::SharedSightObstacles {
        crate::sight_obstacle::SharedSightObstacles {
            static_obstacles: assets.environment.static_sight_obstacles.clone(),
            dynamic_obstacles: std::sync::Arc::new(self.world.dynamic_sight_obstacles.clone()),
            static_active: std::sync::Arc::new(self.world.static_sight_obstacle_active.clone()),
        }
    }

    pub(crate) fn build_sim_scratch(&self, assets: &LevelAssets) -> SimScratch {
        self.build_ai_observation(assets)
    }

    fn build_ai_observation(&self, assets: &LevelAssets) -> SimScratch {
        let started = observation_build_started();
        let views = build_entity_views(self);
        let rebuilt = views.len();
        let scratch = SimScratch {
            ai_entity_views: self.share_ai_entity_views(views),
            ai_sight_obstacles: self.build_ai_sight_obstacles(assets),
        };
        observe_view_build(
            started,
            self.control.frame_counter,
            AiObservationKind::Current,
            self.world.entities.len(),
            rebuilt,
            &scratch,
        );
        scratch
    }

    pub(super) fn building_authorizations_for_ai_views(
        &self,
    ) -> std::collections::HashMap<crate::sector::SectorNumber, bool> {
        self.script_domains
            .interactables
            .doors
            .iter()
            .filter(|door| {
                matches!(
                    door.door_type,
                    crate::gate::DoorType::Building | crate::gate::DoorType::BuildingTrap
                )
            })
            .map(|door| {
                (
                    door.sector_in,
                    self.building_sector_is_authorized(door.sector_in),
                )
            })
            .collect()
    }

    pub(super) fn share_ai_entity_views(&self, entities: AiEntityViewMap) -> SharedAiEntityViews {
        let building_authorizations = self.building_authorizations_for_ai_views();
        std::sync::Arc::new(AiEntityViews {
            entities,
            unavailable_entities: self
                .world
                .entities
                .occupied()
                .filter_map(|(id, entity)| {
                    crate::ai_entity_view::entity_view_unavailable(entity)
                        .map(|reason| (id.index(), reason))
                })
                .collect(),
            building_authorizations,
            diplomacy: self.mission_domain.diplomacy.clone(),
        })
    }

    /// Prepare combat inputs for a dispatch outside the detection pass.
    /// Target metadata and registry inputs reflect the current owner boundary;
    /// the detection FIFO separately retains its completed camp scan.
    pub(in crate::engine) fn build_npc_tick_data(
        &self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) -> crate::ai::AiPerTickData {
        self.build_npc_tick_data_for_target_mode(sim, npc_id, assets, None, true)
    }

    pub(in crate::engine) fn build_npc_tick_data_for_target(
        &self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
        target_override: Option<crate::element::EntityId>,
    ) -> crate::ai::AiPerTickData {
        self.build_npc_tick_data_for_target_mode(sim, npc_id, assets, target_override, true)
    }

    pub(in crate::engine) fn build_npc_tick_data_without_forecasts(
        &self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) -> crate::ai::AiPerTickData {
        match self.world.entities.get(npc_id) {
            Some(entity) if entity.enemy_ai().is_some() => {}
            Some(entity) if entity.ai_controller().is_some() => panic!(
                "owner-local tick context owner {} requires Enemy AI",
                npc_id.index()
            ),
            Some(other) => panic!(
                "owner-local tick context owner {} has invalid entity kind {:?}",
                npc_id.index(),
                other.element_data().kind
            ),
            None => panic!(
                "owner-local tick context owner {} disappeared",
                npc_id.index()
            ),
        }
        self.build_npc_tick_data_for_target_mode(sim, npc_id, assets, None, false)
    }

    fn build_npc_tick_data_for_target_mode(
        &self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
        target_override: Option<crate::element::EntityId>,
        build_forecasts: bool,
    ) -> crate::ai::AiPerTickData {
        use crate::ai::AiPerTickData;

        // Pull the minimum we need from the NPC: its position, camp,
        // primary target handle, and the `couldnt_reachpoint` flag
        // (drives avenger-on-roof computation).
        let Some(entity) = self.world.entities.get(npc_id) else {
            return AiPerTickData::stub();
        };
        let Some(ai_actor) = entity.ai_actor_data() else {
            return AiPerTickData::stub();
        };
        let Some(ai) = ai_actor.ai_brain.base() else {
            return AiPerTickData::stub();
        };
        let Some(enemy_ai) = ai_actor.ai_brain.enemy() else {
            return AiPerTickData::stub();
        };
        let primary_target_handle = target_override
            .map(|id| crate::ai::AiEntityHandle::new(id.index()))
            .or(ai.primary_target);
        let target_id = target_override.or_else(|| {
            primary_target_handle.and_then(|handle| self.entity_id_for_index(handle.get()))
        });
        let my_camp = entity.camp();
        let me_pos = entity.element_data().position_map();
        let me_layer = entity.element_data().layer();
        let couldnt_reachpoint = enemy_ai.base.couldnt_reachpoint;
        // A failed lift-entry approach is surfaced only after the tick snapshot
        // for its EventCouldntReachPoint has started construction. Original
        // computes the roof-avenger waiting position synchronously inside that
        // decision. Preserve the same lookup window from the exact authored
        // 30-frame RunningToLadder timer even though the staged failure latch
        // is not live yet.
        let pending_lift_completion = {
            let enemy = enemy_ai;
            enemy.base.current_substate == crate::ai::Substate::AttackingRunningToLadder
                && enemy.base.timer_is_running
                && enemy.base.substate_at_last_timer_launch
                    == crate::ai::Substate::AttackingRunningToLadder
                && enemy.base.when_does_timer_ring == self.frame_counter().saturating_add(30)
        };

        let mut tick = AiPerTickData::stub();
        tick.fix_hard_reaction_times = sim.config().fix_hard_reaction_times;
        tick.owner_live_position = Some(crate::ai::Position {
            x: me_pos.x,
            y: me_pos.y,
            sector: entity.element_data().sector(),
            level: me_layer,
        });
        tick.primary_target_snapshot_handle = primary_target_handle;
        let enemy_idx = DetectableType::Enemy as usize;
        tick.profile_manager = Some(assets.profile_manager.clone());
        // Area search scans the live global NPC register at the call site.
        // Despite the old local name "visible friends", the Original applies
        // no visibility, camp, layer, posture, or AI-state filter here: every
        // other soldier with alert status above green and raw map-space
        // distance below 500 contributes to the point-count multiplier.
        // Build this for every AI decision boundary, not only detection refresh,
        // because timer/report callbacks also start area searches synchronously.
        let doors = self.script_domains.interactables.doors.as_slice();
        // Swordfight reconsideration refreshes `primary_target` from the actor's
        // principal opponent before it forecasts a lost opponent.  That
        // principal can differ from the AI member captured above, so retain
        // prepared forecasts by detectable handle as well as in the
        // primary-target convenience slot.  Detection dispatch rebuilds this
        // list from live positions for each synchronous delivery.
        if build_forecasts {
            for detectable in &ai_actor.detectable_lists[enemy_idx] {
                let Some(target_id) = detectable.element else {
                    continue;
                };
                let target = self.world.entities.expect_entity(
                    target_id,
                    format_args!("NPC {} Enemy detectable actor", npc_id.index()),
                );
                let input = extract_exact_forecast_input(
                    self,
                    target,
                    selected_actor_is_passing_door(&self.orders.sequence_manager, target_id),
                )
                .unwrap_or_else(|| {
                    panic!(
                        "NPC {} requires a destination forecast for non-actor {}",
                        npc_id.index(),
                        target_id.index()
                    )
                });
                tick.enemy_detectable_forecasts.push((
                    target_id.index(),
                    crate::ai::prepare_forecast_destination_for_ia(
                        &input,
                        doors,
                        &self.world.fast_grid.level.sectors,
                        &self.world.fast_grid.level.sector_number_map,
                    ),
                ));
            }
        }
        tick.camp_soldiers = self.build_camp_soldier_tick_infos(npc_id, my_camp);
        // Populate the remaining borrowed tactical inputs here so
        // off-detection dispatch sites (timer events, reach-point
        // events, panic, patrol, cross-NPC actions, pending-stimuli
        // drain, …) see the same fighter view that the in-detection
        // builder produces.  Without this, AI predicates that consume
        // `tick.nearby_fighters` (rider charge target lookup,
        // phalanx encirclement, nearby archers needing protection,
        // phalanx geometry, friendly-presence polygon checks)
        // observe an empty list outside swordfight substates.
        tick.nearby_fighters = self.build_nearby_fighters_for(npc_id, assets);
        tick.fighter_registry = self.build_fighter_snapshots_for(npc_id, assets, None);

        if let Some(chief_id) = ai.patrol_chief {
            let chief_ai = self.world.entities.expect_ai_controller(
                chief_id,
                format_args!("enemy tick context owner {} patrol chief", npc_id.index()),
            );
            tick.patrol_chief_state = chief_ai.current_state;
        }

        // Avenger-on-roof wait positions — computed for a live failure latch
        // or the exact pending lift completion described above. Decision arms
        // re-pick their target from the personal enemy list (even from a null
        // pre-think target), so compute one wait position per candidate handle
        // plus the current target; consumers resolve their own live handle at
        // use time.
        if couldnt_reachpoint || pending_lift_completion {
            assert!(
                self.scripts.mission.is_some(),
                "AI roof recovery requires an installed mission script"
            );
            let doors_slice = self.script_domains.interactables.doors.as_slice();
            let mut candidates: Vec<crate::ai::HumanHandle> = Vec::new();
            candidates.extend(enemy_ai.list_them.iter().copied());
            if let Some(primary_target_handle) = primary_target_handle {
                candidates.push(primary_target_handle.get());
            }
            candidates.retain(|&h| h != 0);
            candidates.dedup();
            for handle in candidates {
                if tick
                    .avenger_on_roof_wait_positions
                    .iter()
                    .any(|(h, _)| *h == handle)
                {
                    continue;
                }
                let Some(candidate_id) = self.entity_id_for_index(handle) else {
                    continue;
                };
                if let Some(wait) = precompute_avenger_on_roof_wait_position(
                    &self.world.entities,
                    doors_slice,
                    &self.orders.sequence_manager,
                    npc_id,
                    candidate_id,
                    |element| super::ai_view_position_sector(self, element),
                    &|sector| self.building_sector_is_authorized(sector),
                    &|sector| self.get_sector_lift_type(sector),
                ) {
                    tick.avenger_on_roof_wait_positions.push((handle, wait));
                }
            }
        }

        let Some(target_id) = target_id else {
            return tick;
        };

        if build_forecasts
            && let Some(target_entity) = self.world.entities.get(target_id)
            && let Some(input) = extract_exact_forecast_input(
                self,
                target_entity,
                selected_actor_is_passing_door(&self.orders.sequence_manager, target_id),
            )
        {
            let doors = self.script_domains.interactables.doors.as_slice();
            tick.primary_target_forecast = Some(crate::ai::prepare_forecast_destination_for_ia(
                &input,
                doors,
                &self.world.fast_grid.level.sectors,
                &self.world.fast_grid.level.sector_number_map,
            ));
        }

        tick
    }

    fn build_camp_soldier_tick_infos(
        &self,
        npc_id: crate::element::EntityId,
        my_camp: crate::element::Camp,
    ) -> Vec<crate::ai_enemy::CampSoldierInfo> {
        let mut camp_soldiers =
            Vec::with_capacity(self.world.entities.soldiers().count().saturating_sub(1));
        for &handle in self.ai.global.all_soldier_handles.iter() {
            let other_id = crate::entity_id::SoldierId(handle);
            if EntityId::Soldier(other_id) == npc_id {
                continue;
            }
            let Some(Entity::Soldier(s)) = self.world.entities.get(EntityId::Soldier(other_id))
            else {
                // Validate the current typed occupant: Original's camp array
                // order survives removals, while recycled non-soldier slots
                // must not enter the ordered union.
                continue;
            };
            // Camp soldier counting includes unconscious and inactive
            // soldiers. Individual Original consumers apply their own gates:
            // Alertable-soldier collection retains everyone of the allowed
            // rank, while nearest-fighter selection rejects dead, unconscious, and
            // inactive candidates.
            if !self.camps_are_allied(s.soldier.cached_camp, my_camp) {
                continue;
            }
            let able_to_fight = crate::element::Human::is_able_to_fight(s);
            let alive_and_conscious = s.npc.life_points > 0 && !s.human.unconscious;
            let Some(enemy_ai) = s.npc.ai_brain.enemy() else {
                continue;
            };
            let in_building = self.entity_data_in_building_sector(&s.element);
            let position = s.element.position_map();
            // Snapshot the soldier's `DETECTABLE_BODY` list — handles of
            // corpses they have not yet reacted to.  Snapshotting the
            // data here lets AI predicates run off `tick.camp_soldiers`
            // alone instead of poking at the live detectable list.
            let detectable_body_idx = crate::element::DetectableType::Body as usize;
            let detectable_bodies = s
                .npc
                .detectable_lists
                .get(detectable_body_idx)
                .map(|list| {
                    let mut bodies = Vec::with_capacity(list.len());
                    bodies.extend(list.iter().filter_map(|d| d.element.map(|e| e.index())));
                    bodies
                })
                .unwrap_or_default();
            let cs_position = crate::ai::Position {
                x: position.x,
                y: position.y,
                sector: s.element.sector(),
                level: s.element.layer(),
            };
            let eye_blind = s.npc.eye_status.is_blind();
            camp_soldiers.push(crate::ai_enemy::CampSoldierInfo {
                handle: other_id.index(),
                active: s.element.active,
                position: cs_position,
                position_world: s.element.position(),
                direction: s.element.direction() as u16,
                rank: enemy_ai.soldier_profile_rank,
                ai_state: s.npc.ai_state(),
                ai_substate: s.npc.ai_substate(),
                is_able_to_fight: able_to_fight,
                is_dead: s.npc.life_points <= 0,
                knocked_out_in_money_fight: enemy_ai.base.knocked_out_in_money_fight,
                primary_target: enemy_ai.base.primary_target,
                pride: enemy_ai.soldier_profile_pride,
                is_able_to_help: crate::ai_enemy::soldier_is_able_to_help_state(
                    alive_and_conscious,
                    s.npc.ai_state(),
                    s.npc.ai_substate(),
                ),
                script_locked: enemy_ai.base.script_locked,
                ai_lock_frozen: enemy_ai
                    .base
                    .locks_flag_field
                    .contains(crate::ai::AiLockFlags::FREEZE),
                layer: s.element.layer(),
                alert_soldiers_point: enemy_ai.base.alert_soldiers_point,
                patrol_chief: enemy_ai.base.patrol_chief,
                antagonist: enemy_ai.base.antagonist,
                detected_body: enemy_ai.base.detected_body,
                blood_alcohol: enemy_ai.base.blood_alcohol,
                duty_flag: enemy_ai.soldier_profile_duty,
                is_tower_guard: enemy_ai.tower_guard,
                company_number: enemy_ai.company_number,
                in_building,
                detectable_bodies,
                current_task_priority: enemy_ai.current_task_priority,
                minimal_task_priority: enemy_ai.minimal_task_priority,
                view_direction: s.npc.view_direction,
                view_radius: s.npc.view_radius,
                real_half_aperture: s.npc.real_half_aperture,
                eye_blind,
            });
        }
        camp_soldiers
    }

    /// Build a `nearby_fighters` snapshot list for one enemy NPC.
    ///
    /// Walks the entity store directly, the same scan-the-global-fighter-
    /// registry approach used by swordfight reconsideration. Filters
    /// non-self entries to the same 500-unit Chebyshev radius the
    /// detection-pass builder applies.
    ///
    /// Returns an empty Vec for non-enemy soldiers — civilians and
    /// PCs don't consume `nearby_fighters`.
    pub(in crate::engine) fn build_nearby_fighters_for(
        &self,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) -> Vec<crate::ai_enemy::FighterSnapshot> {
        self.build_fighter_snapshots_for(npc_id, assets, Some(500.0))
    }

    #[cfg(test)]
    pub(in crate::engine) fn build_full_fighter_registry_for_test(
        &self,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) -> Vec<crate::ai_enemy::FighterSnapshot> {
        self.build_fighter_snapshots_for(npc_id, assets, None)
    }

    fn build_fighter_snapshots_for(
        &self,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
        max_distance: Option<f32>,
    ) -> Vec<crate::ai_enemy::FighterSnapshot> {
        use crate::ai::Position;
        use crate::ai_enemy::FighterSnapshot;
        use crate::element::Posture;

        let owner = self.expect_entity(npc_id, "fighter snapshot owner");
        let Some(enemy_ai) = owner.enemy_ai() else {
            // This registry is an enemy-brain capability. The public nearby
            // query also accepts civilians and PCs, for whom it is inapplicable.
            return Vec::new();
        };
        let doors = self.script_domains.interactables.doors.as_slice();
        let fighter_position = |id: crate::element::EntityId| {
            resolve_ai_position_with(
                &self.world.entities,
                doors,
                &self.orders.sequence_manager,
                id,
                |position_id| {
                    let element = self
                        .expect_entity(position_id, "fighter snapshot position owner")
                        .element_data();
                    Position {
                        x: element.position_map().x,
                        y: element.position_map().y,
                        // Element positioning carries the exact sector reference.
                        // Legacy-loaded actors can retain only its public
                        // number on ElementData, so recover the arena identity
                        // before combat-position proposals copy this snapshot
                        // into a cross-door movement destination.
                        sector: super::ai_view_position_sector(self, element),
                        level: element.layer(),
                    }
                },
            )
            .effective
        };
        let me_position = fighter_position(npc_id);
        let me_pos_pt = crate::coordinates::MapPoint::new(me_position.x, me_position.y);
        let me_elevation = owner.element_data().position().z;
        let my_camp = owner.camp();
        let me_handle = enemy_ai.base.me;

        // Build a friendly soldier snapshot for `handle` (which may be self).
        // The fighter registry retains inactive and out-of-order soldiers.
        // Nearby-fighter collection inserts self before applying combat
        // readiness checks to the remaining registry entries.
        let build_soldier = |handle: u32, require_able: bool| -> Option<FighterSnapshot> {
            let s = self.world.entities.get_soldier(SoldierId(handle))?;
            let is_able_to_fight = s.is_able_to_fight();
            if require_able && !is_able_to_fight {
                return None;
            }
            let position = fighter_position(EntityId::Soldier(SoldierId(handle)));
            // the element position — no door-transit or carrier
            // substitution. Squared-distance range gates read
            // this, not the AI `Position()` result above.
            let raw_position = Position {
                x: s.element.position_map().x,
                y: s.element.position_map().y,
                sector: s.element.sector(),
                level: s.element.layer(),
            };
            let enemy_ai_other = s
                .npc
                .ai_brain
                .enemy()
                .unwrap_or_else(|| panic!("active soldier {handle} has no EnemyAi brain"));
            let (soldier_profile, fighting_ability, _) =
                self.soldier_profile_facts(assets, s, EntityId::Soldier(SoldierId(handle)));
            let hth_id = enemy_ai_other.hth_weapon_id;
            let hth_profile = assets
                .profile_manager
                .get_hth_weapon(hth_id)
                .unwrap_or_else(|| {
                    panic!("soldier {handle} requires missing HtH weapon profile {hth_id}")
                });
            let (sword_range_default, sword_range_maximal, sword_range_uber) = (
                hth_profile.distance[crate::weapons::WeaponDistance::Default as usize],
                hth_profile.distance[crate::weapons::WeaponDistance::Maximal as usize],
                hth_profile.distance[crate::weapons::WeaponDistance::Uber as usize],
            );
            let in_recovery = self.actor_is_in_sword_recovery(EntityId::Soldier(SoldierId(handle)));
            // The seek position is complete and carries its own
            // level; it is never re-levelled from the soldier's current
            // element layer. Positioning behind the shield bearer
            // in the original game copies
            // that level into the cover point for the straight-movement
            // check, so a shield bearer running from
            // one layer to a phalanx slot on another must keep the slot's
            // level here.
            let seek_position = enemy_ai_other.base.seek_position;
            let opponent_handles: Vec<u32> =
                s.human.opponents.iter().map(|id| id.index()).collect();
            let number_of_opponents = opponent_handles.len().min(u16::MAX as usize) as u16;
            let is_friendly = self.camps_are_allied(s.soldier.cached_camp, my_camp);
            Some(FighterSnapshot {
                handle,
                position: Position {
                    x: position.x,
                    y: position.y,
                    // The original game's entity-position query copies the complete
                    // saved position, including its authoritative sector
                    // pointer.  Combat helpers later copy this position
                    // when deriving destinations (notably the archer
                    // cover point behind a stationary shield bearer), so
                    // discarding the sector here turns an otherwise valid
                    // same-sector movement into EVENT_COULDNT_REACHPOINT.
                    sector: position.sector,
                    level: position.level,
                },
                raw_position,
                direction: s.element.direction() as u16,
                is_friendly,
                is_swordfighting: !s.human.opponents.is_empty(),
                is_able_to_fight,
                is_tied: s.element.posture() == Posture::Tied,
                is_unconscious: s.human.unconscious,
                is_dead: s.npc.life_points <= 0,
                is_carried: s.human.carrier.is_some(),
                is_pc: false,
                is_soldier: true,
                rank: enemy_ai_other.soldier_profile_rank,
                primary_target: enemy_ai_other.base.primary_target,
                principal_opponent: s
                    .human
                    .opponents
                    .first()
                    .map(|id| crate::ai::AiEntityHandle::new(id.index())),
                number_of_opponents,
                opponent_handles,
                sword_range_default,
                sword_range_maximal,
                sword_range_uber,
                fighting_ability,
                is_vip: soldier_profile.vip,
                soldier_profile_pride: enemy_ai_other.soldier_profile_pride,
                is_robin: false,
                is_in_recovery_animation: in_recovery,
                in_sword_action_state: s.actor.action_state.is_sword(),
                elevation: s.element.position().z,
                seek_position,
                current_substate: s.npc.ai_substate(),
                hth_weapon_id: hth_id,
                action_state: s.actor.action_state,
            })
        };

        // Build a PC snapshot for `handle`. Campaign PCs normally belong to
        // the Royalist camp; custom-mission PCs retain their authored
        // allegiance so each retinue recognizes its champion as a friend.
        let build_pc = |handle: u32, require_able: bool| -> Option<FighterSnapshot> {
            let pc = self.world.entities.get_pc(PcId(handle))?;
            let is_dead = pc.pc.life_points <= 0;
            let is_unconscious = pc.human.unconscious;
            let is_able_to_fight = pc.element.active
                && !is_dead
                && !is_unconscious
                && !matches!(pc.element.posture(), Posture::Tree | Posture::Spy);
            if require_able && !is_able_to_fight {
                return None;
            }
            let is_carried = pc.human.carrier.is_some();
            let position = fighter_position(EntityId::Pc(PcId(handle)));
            // the element position — see the soldier branch.
            let raw_position = Position {
                x: pc.element.position_map().x,
                y: pc.element.position_map().y,
                sector: pc.element.sector(),
                level: pc.element.layer(),
            };
            let character = assets
                .profile_manager
                .get_character(pc.pc.profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "PC {handle} requires missing character profile {}",
                        u32::from(pc.pc.profile_index)
                    )
                });
            let hth_id = character.hth_weapon_id;
            let fighting_ability = character.fighting;
            let hth_profile = assets
                .profile_manager
                .get_hth_weapon(hth_id)
                .unwrap_or_else(|| {
                    panic!("PC {handle} requires missing HtH weapon profile {hth_id}")
                });
            let (sword_range_default, sword_range_maximal, sword_range_uber) = (
                hth_profile.distance[crate::weapons::WeaponDistance::Default as usize],
                hth_profile.distance[crate::weapons::WeaponDistance::Maximal as usize],
                hth_profile.distance[crate::weapons::WeaponDistance::Uber as usize],
            );
            let in_recovery =
                !is_able_to_fight || self.actor_is_in_sword_recovery(EntityId::Pc(PcId(handle)));
            let opponent_handles: Vec<u32> =
                pc.human.opponents.iter().map(|id| id.index()).collect();
            let number_of_opponents = opponent_handles.len().min(u16::MAX as usize) as u16;
            let live_position = pc.element.position_map();
            let pc_seek_position = Position {
                x: live_position.x,
                y: live_position.y,
                sector: pc.element.sector(),
                level: pc.element.layer(),
            };
            Some(FighterSnapshot {
                handle,
                position,
                raw_position,
                direction: pc.element.direction() as u16,
                is_friendly: self.camps_are_allied(pc.pc.cached_camp, my_camp),
                is_swordfighting: !pc.human.opponents.is_empty(),
                is_able_to_fight,
                is_tied: pc.element.posture() == Posture::Tied,
                is_unconscious,
                is_dead,
                is_carried,
                is_pc: true,
                is_soldier: false,
                rank: pc
                    .pc
                    .ai
                    .as_deref()
                    .and_then(|ai| ai.ai_brain.enemy())
                    .map(|ai| ai.soldier_profile_rank)
                    .unwrap_or(crate::profiles::ProfileRank::None),
                primary_target: pc
                    .pc
                    .ai
                    .as_deref()
                    .and_then(|ai| ai.ai_brain.enemy())
                    .and_then(|ai| ai.base.primary_target)
                    .or_else(|| {
                        pc.pc
                            .melee_target
                            .map(|id| crate::ai::AiEntityHandle::new(id.index()))
                    }),
                principal_opponent: pc
                    .human
                    .opponents
                    .first()
                    .map(|id| crate::ai::AiEntityHandle::new(id.index())),
                number_of_opponents,
                opponent_handles,
                sword_range_default,
                sword_range_maximal,
                sword_range_uber,
                fighting_ability,
                is_vip: character.vip,
                soldier_profile_pride: 0,
                is_robin: pc.pc.robin,
                is_in_recovery_animation: in_recovery,
                in_sword_action_state: pc.actor.action_state.is_sword(),
                elevation: pc.element.sprite.position_iface.get_elevation(),
                seek_position: pc_seek_position,
                current_substate: pc
                    .pc
                    .ai
                    .as_deref()
                    .map(crate::element::AiActorData::ai_substate)
                    .unwrap_or_default(),
                hth_weapon_id: hth_id,
                action_state: pc.actor.action_state,
            })
        };

        let mut out: Vec<FighterSnapshot> = Vec::with_capacity(1 + self.world.pc_ids.len() + 4);

        // Self entry first — no radius filter (the AI is at distance 0).
        let self_snapshot = match npc_id {
            EntityId::Soldier(_) => build_soldier(me_handle, false),
            EntityId::Pc(_) => build_pc(me_handle, false),
            _ => None,
        };
        out.push(self_snapshot.unwrap_or_else(|| {
            panic!("enemy AI self {me_handle} is absent from the fighter registry")
        }));

        // Walk the registration order so each camp's fighter order matches
        // Original's append-only registry even when PCs and soldiers are
        // interleaved. Friendly scans still put the current actor first, as
        // nearby-fighter collection does explicitly.
        for id in self.world.fighter_registry_order() {
            if id == npc_id {
                continue;
            }
            let Some(entity) = self.world.entities.get(id) else {
                continue;
            };
            let (position, elevation, snapshot) = match entity {
                Entity::Soldier(soldier) => (
                    fighter_position(id),
                    soldier.element.position().z,
                    // Radius-limited snapshots model
                    // nearby-fighter collection and therefore exclude
                    // unable fighters. The complete registry is the backing
                    // store for already-held original-game references, which remain
                    // dereferenceable while their owner decides how to prune
                    // them.
                    build_soldier(id.index(), max_distance.is_some()),
                ),
                Entity::Pc(pc) => (
                    fighter_position(id),
                    pc.element.sprite.position_iface.get_elevation(),
                    build_pc(id.index(), max_distance.is_some()),
                ),
                _ => continue,
            };
            // Maximum-norm distance subtracts full world positions before
            // stretching Y, so the elevation enters twice: once as the
            // projection offset baked into map Y and once as its own
            // component. Comparing raw map coordinates instead pushed
            // fighters standing a layer above or below out of every
            // consideration radius built on this snapshot.
            let world = crate::coordinates::GroundPoint::from_map_and_z(
                crate::coordinates::MapPoint::new(position.x, position.y),
                elevation,
            );
            let me_world = crate::coordinates::GroundPoint::from_map_and_z(me_pos_pt, me_elevation);
            let dx = world.x - me_world.x;
            let dy = (world.y - me_world.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
            let dz = elevation - me_elevation;
            if max_distance.is_some_and(|radius| dx.abs().max(dy.abs()).max(dz.abs()) > radius) {
                continue;
            }
            if let Some(snapshot) = snapshot {
                out.push(snapshot);
            }
        }

        out
    }
}

#[cfg(test)]
mod observation_tests {
    use super::*;

    #[test]
    fn observation_metrics_preserve_views_hash_and_rng() {
        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        engine.add_test_entity(crate::engine::tests::scenarios::make_test_ai_soldier(
            crate::element::Camp::Lacklandists,
        ));
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        let hash = crate::replay::state_hash(&engine);
        let sim = engine.control.simulation_context();
        let seed = sim.seed();
        let plain = engine.build_sim_scratch(&assets);
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(std::io::sink)
            .finish();
        let measured =
            tracing::subscriber::with_default(subscriber, || engine.build_sim_scratch(&assets));
        assert_eq!(
            serde_json::to_value(&plain.ai_entity_views.entities).unwrap(),
            serde_json::to_value(&measured.ai_entity_views.entities).unwrap()
        );
        assert_eq!(crate::replay::state_hash(&engine), hash);
        assert_eq!(sim.seed(), seed);
    }
}
