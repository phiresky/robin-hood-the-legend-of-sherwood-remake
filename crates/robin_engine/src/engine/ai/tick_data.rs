use super::*;

impl EngineInner {
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

    pub(in crate::engine) fn debug_refresh_view_lifecycle(
        &self,
        stage: &str,
        npc_id: EntityId,
        derived_tail_order_type: Option<crate::order::OrderType>,
    ) {
        let config = refresh_view_lifecycle_debug_config();
        if !config.enabled
            || self.control.frame_counter < config.from_frame
            || self.control.frame_counter > config.through_frame
        {
            return;
        }
        let creation_order = self.world.original_creation_order(npc_id);
        if config
            .creation_order
            .is_some_and(|expected| creation_order != expected)
        {
            return;
        }
        let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
            panic!(
                "RVLIFE owner {} disappeared at stage {stage}",
                npc_id.index()
            )
        });
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
            entity.element_data().posture == crate::element::Posture::Tied,
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

    /// Refresh the Original actor's selected `mpWaitSequenceElement` identity
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

    /// Position visible at one Original legacy owner boundary. Rust movement
    /// is globally batched, so later slots are projected back to the preserved
    /// pre-movement oracle. Callback-spawned slots are absent from that oracle
    /// and correctly retain their current (never-moved) position.
    pub(in crate::engine) fn position_at_owner_boundary(
        &self,
        target: EntityId,
        owner: EntityId,
        positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
        owner_actor_complete: bool,
    ) -> MapPoint {
        self.boundary_position(
            target,
            owner,
            positions_before_movement,
            owner_actor_complete,
        )
        .map
    }

    /// The same boundary choice, keeping both stored coordinate spaces. Direct
    /// actor geometry (`ComputeEyesPoint` / `ComputeDetectionPoint`) reads the
    /// 3D position, which is not recoverable from the map projection without
    /// rounding.
    pub(in crate::engine) fn boundary_position(
        &self,
        target: EntityId,
        owner: EntityId,
        positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
        owner_actor_complete: bool,
    ) -> crate::entities::BoundaryPosition {
        let current = crate::entities::BoundaryPosition::of(
            self.world
                .entities
                .get(target)
                .unwrap_or_else(|| {
                    panic!(
                        "owner {} requires position for missing entity {}",
                        owner.index(),
                        target.index()
                    )
                })
                .element_data(),
        );
        let target_has_not_moved = self.world.original_creation_order(target)
            > self.world.original_creation_order(owner)
            || (!owner_actor_complete && target.index() == owner.index());
        if target_has_not_moved {
            positions_before_movement
                .get(target)
                .copied()
                .flatten()
                .unwrap_or(current)
        } else {
            current
        }
    }

    pub(super) fn build_ai_sight_obstacles(
        &self,
        assets: &LevelAssets,
    ) -> crate::sight_obstacle::SharedSightObstacles {
        crate::sight_obstacle::SharedSightObstacles {
            static_obstacles: assets.static_sight_obstacles.clone(),
            dynamic_obstacles: std::sync::Arc::new(self.world.dynamic_sight_obstacles.clone()),
            static_active: std::sync::Arc::new(self.world.static_sight_obstacle_active.clone()),
        }
    }

    pub(crate) fn build_sim_scratch(
        &self,
        _sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) -> SimScratch {
        SimScratch {
            ai_entity_views: self.share_ai_entity_views(build_entity_views(self)),
            ai_sight_obstacles: self.build_ai_sight_obstacles(assets),
        }
    }

    pub(in crate::engine) fn build_cached_detection_scratch(
        &self,
        assets: &LevelAssets,
        cache: &mut PreparedAiEntityViewCache,
    ) -> SimScratch {
        let _rebuilt = refresh_prepared_entity_views(self, cache);
        SimScratch {
            ai_entity_views: std::sync::Arc::clone(
                cache
                    .views
                    .as_ref()
                    .expect("prepared AI entity-view cache was not initialized"),
            ),
            ai_sight_obstacles: self.build_ai_sight_obstacles(assets),
        }
    }

    pub(crate) fn build_owner_context_scratch_without_forecast(
        &self,
        assets: &LevelAssets,
    ) -> SimScratch {
        SimScratch {
            ai_entity_views: self.share_ai_entity_views(build_entity_views_without_forecast(self)),
            ai_sight_obstacles: self.build_ai_sight_obstacles(assets),
        }
    }

    pub(crate) fn build_owner_context_scratch_at_slot_without_forecast(
        &self,
        assets: &LevelAssets,
        owner: EntityId,
        positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
        owner_actor_complete: bool,
    ) -> SimScratch {
        let mut views = build_entity_views_without_forecast(self);
        for (target, _) in self.world.entities.occupied() {
            let boundary = self.boundary_position(
                target,
                owner,
                positions_before_movement,
                owner_actor_complete,
            );
            // The initial view builder already applies
            // `RHArtificialIntelligence::Position(actor)`'s committed
            // gate-side override. Creation-slot projection is for live map
            // positions and must not replace that AI-specific value.
            // Direct geometry is different: ComputeDetectionPoint starts from
            // literal GetPosition even during a door pass, so always stamp its
            // map/world pair with the owner-boundary values.
            let passing_door = self
                .world
                .entities
                .get(target)
                .and_then(Entity::actor_data)
                .is_some_and(|actor| actor.active_door_pass.is_some());
            if let Some(view) = views.get_mut(&target.index()) {
                view.detection_position = boundary.map;
                view.detection_position_world = boundary.world;
                if !passing_door {
                    view.position.x = boundary.map.x;
                    view.position.y = boundary.map.y;
                }
            }
        }
        SimScratch {
            ai_entity_views: self.share_ai_entity_views(views),
            ai_sight_obstacles: self.build_ai_sight_obstacles(assets),
        }
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
            building_authorizations,
            diplomacy: self.mission_domain.diplomacy.clone(),
        })
    }

    /// Build a per-NPC [`AiPerTickData`] snapshot on demand, outside
    /// the main detection pass.
    ///
    /// The detection pass (see the builder at `engine/ai.rs:4319`)
    /// assembles a full-fidelity `AiPerTickData` with camp soldiers,
    /// nearby fighters, battle points, multiplicity, etc. — but it
    /// only runs once per frame per NPC.  Off-detection dispatch sites
    /// (timer events, reach-point events, panic, patrol, cross-NPC
    /// actions, civilian EventView...) previously called
    /// `AiPerTickData::stub()`, losing every field that matters for
    /// `battle_decisions` and swordfight tactics.  The symptom: a
    /// soldier with a valid `primary_target` but empty
    /// `enemy_sq_distances` bails to `return_to_duty`, producing the
    /// Reactiontime/Default ping-pong.
    ///
    /// This builder fills in everything that can be cheaply computed
    /// from the live entity store without re-running the detection
    /// loop: same-camp soldier snapshots for alert coordination,
    /// primary target metadata (position, posture, animation,
    /// carrier, destination forecast, table-swordfight jump line),
    /// `primary_target_is_pc`, friend-swap candidates for
    /// `ReconsiderEnemyApproach`, the avenger-on-the-roof wait
    /// position, and a single-target seed for
    /// `enemy_sq_distances` / `min_sq_enemy_distance` so
    /// `battle_decisions` doesn't see an empty list when a valid
    /// `primary_target` exists. Fields that truly require the full detection
    /// scan (`unconscious_enemies`, `nearby_sleeping_enemies`, final visible
    /// enemy distances/latches, ...) remain empty; RefreshDetection overlays
    /// those scan products when it uses this builder for a queued stimulus.
    ///
    /// Returns a stub for non-enemy-soldier entities (civilians, PCs,
    /// beggar/animal NPCs); their AI paths don't consult the combat
    /// tick fields, so the stub is adequate.
    pub(in crate::engine) fn build_npc_tick_data(
        &self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        scratch: &SimScratch,
        assets: &LevelAssets,
    ) -> crate::ai::AiPerTickData {
        self.build_npc_tick_data_for_target_mode(sim, npc_id, scratch, assets, None, true)
    }

    pub(in crate::engine) fn build_npc_tick_data_for_target(
        &self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        scratch: &SimScratch,
        assets: &LevelAssets,
        target_override: Option<crate::element::EntityId>,
    ) -> crate::ai::AiPerTickData {
        self.build_npc_tick_data_for_target_mode(
            sim,
            npc_id,
            scratch,
            assets,
            target_override,
            true,
        )
    }

    pub(in crate::engine) fn build_npc_tick_data_without_forecasts(
        &self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        scratch: &SimScratch,
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
        self.build_npc_tick_data_for_target_mode(sim, npc_id, scratch, assets, None, false)
    }

    /// Build the typed live value consumed by Friendly AI. The narrow type
    /// has no stub/default fields for future handlers to read accidentally.
    pub(in crate::engine) fn build_friendly_tick_data_without_forecasts(
        &self,
        npc_id: crate::element::EntityId,
    ) -> crate::ai_friendly::FriendlyPerTickData {
        let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
            panic!(
                "owner-local friendly tick context owner {} disappeared",
                npc_id.index()
            )
        });
        let Entity::Civilian(civilian) = entity else {
            panic!(
                "owner-local friendly tick context owner {} is not a Civilian",
                npc_id.index()
            )
        };
        let ai = civilian.npc.ai_brain.friendly().unwrap_or_else(|| {
            panic!(
                "owner-local friendly tick context owner {} requires Friendly AI",
                npc_id.index()
            )
        });

        if let Some(chief_id) = ai.base.patrol_chief {
            let chief = self.world.entities.get(chief_id).unwrap_or_else(|| {
                panic!(
                    "owner-local friendly tick context owner {} has stale patrol chief {}",
                    npc_id.index(),
                    chief_id.index()
                )
            });
            let chief_ai = chief.ai_controller().unwrap_or_else(|| {
                panic!(
                    "owner-local friendly tick context patrol chief {} has no AI",
                    chief_id.index()
                )
            });
            let point = chief.element_data().position_map();
            crate::ai_friendly::FriendlyPerTickData::with_patrol_chief(
                crate::ai::Position {
                    x: point.x,
                    y: point.y,
                    sector: chief.element_data().sector(),
                    level: chief.element_data().layer(),
                },
                chief_ai.current_state,
            )
        } else {
            crate::ai_friendly::FriendlyPerTickData::without_patrol_chief()
        }
    }

    fn build_npc_tick_data_for_target_mode(
        &self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        scratch: &SimScratch,
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
        let me_handle = ai.me;
        let me_pos = entity.element_data().position_map();
        let me_layer = entity.element_data().layer();
        let couldnt_reachpoint = enemy_ai.base.couldnt_reachpoint;
        // A failed lift-entry GoNear is surfaced only after the tick snapshot
        // for its EventCouldntReachPoint has started construction. Original
        // computes GetAvengerOnTheRoofWaitPosition synchronously inside that
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
        let enemy_idx = DetectableType::Enemy as usize;
        tick.seen_last_frame_enemies =
            seen_last_frame_detectable_handles(&ai_actor.detectable_lists[enemy_idx]);
        tick.primary_target_snapshot_handle = primary_target_handle;
        tick.profile_manager = Some(assets.profile_manager.clone());
        // `SeekArea` scans the live global NPC register at the call site.
        // Despite the old local name "visible friends", the Original applies
        // no visibility, camp, layer, posture, or AI-state filter here: every
        // other soldier with alert status above green and raw map-space
        // distance below 500 contributes to the point-count multiplier.
        // Build this for every Think boundary, not only RefreshDetection,
        // because timer/report callbacks also enter SeekArea synchronously.
        let doors = self.script_domains.interactables.doors.as_slice();
        // ReconsiderSwordfight refreshes `primary_target` from the actor's
        // principal opponent before it forecasts a lost opponent.  That
        // principal can differ from the AI member captured above, so retain
        // prepared forecasts by detectable handle as well as in the
        // primary-target convenience slot.  Detection dispatch rebuilds this
        // list with owner-boundary positions; timer/sequence dispatches still
        // need the live variants populated here.
        if build_forecasts {
            for detectable in &ai_actor.detectable_lists[enemy_idx] {
                let Some(target_id) = detectable.element else {
                    continue;
                };
                let target = self.world.entities.get(target_id).unwrap_or_else(|| {
                    panic!(
                        "NPC {} has Enemy detectable for missing actor {}",
                        npc_id.index(),
                        target_id.index()
                    )
                });
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
        let frame = self.frame_counter();
        // Keep the diagnostic entirely absent from the disabled path. In
        // particular, do not resolve Original identity for an observation
        // that production does not otherwise need here.
        let seek_area_debug_enabled = seek_area_owner_position_debug_enabled();
        let creation_order = if seek_area_debug_enabled {
            self.world.original_creation_order(npc_id)
        } else {
            0
        };
        let seek_area_debug = seek_area_debug_enabled
            && seek_area_owner_position_debug_matches(frame, creation_order);
        if seek_area_debug {
            let owner_selected_door =
                selected_pass_door_movement(&self.orders.sequence_manager, npc_id);
            let owner_effective_position = seek_area_friend_position_map(
                me_pos,
                owner_selected_door.map(|(door_index, direction)| (door_index, direction != 0)),
                doors,
            );
            eprintln!(
                "SEEKAREA {{\"event\":\"owner_position\",\"frame\":{},\"owner\":{:?},\"owner_creation_order\":{},\"raw\":[{},{}],\"effective\":[{},{}],\"selected_door\":{:?}}}",
                frame,
                npc_id,
                creation_order,
                me_pos.x,
                me_pos.y,
                owner_effective_position.x,
                owner_effective_position.y,
                owner_selected_door,
            );
        }
        for (other_id, other) in self.world.entities.soldiers() {
            if other_id == npc_id {
                continue;
            }
            let Some(other_ai) = other.npc.ai_brain.enemy() else {
                continue;
            };
            // Original `RHElementActorNPC::GetAlertStatus()` reads
            // `mViewParameters.ubAlertStatus`, not the independently tracked
            // music alert. Forced-attentive and music-only transitions can
            // deliberately make those values differ.
            let alert_status = other_ai.base.view_alert_status;
            let friend_raw_position = other.element.position_map();
            if alert_status == crate::ai::AlertLevel::Green {
                if seek_area_debug {
                    let friend_selected_door = selected_pass_door_movement(
                        &self.orders.sequence_manager,
                        EntityId::Soldier(other_id),
                    );
                    let friend_effective_position = seek_area_friend_position_map(
                        friend_raw_position,
                        friend_selected_door
                            .map(|(door_index, direction)| (door_index, direction != 0)),
                        doors,
                    );
                    eprintln!(
                        "SEEKAREA {{\"event\":\"friend_contribution\",\"frame\":{},\"owner_creation_order\":{},\"friend\":{:?},\"friend_creation_order\":{},\"alert\":{:?},\"raw\":[{},{}],\"effective\":[{},{}],\"selected_door\":{:?},\"contributes\":false,\"reason\":\"green\"}}",
                        frame,
                        creation_order,
                        other_id,
                        self.world
                            .original_creation_order(EntityId::Soldier(other_id)),
                        alert_status,
                        friend_raw_position.x,
                        friend_raw_position.y,
                        friend_effective_position.x,
                        friend_effective_position.y,
                        friend_selected_door,
                    );
                }
                continue;
            }
            let friend_seeks_with_help = other_ai.base.current_substate.is_seek_area()
                && other_ai
                    .seek_flags
                    .contains(crate::ai_enemy::SeekFlags::LOOK_FOR_HELP_AFTER);
            let contribution = seek_area_friend_contribution(
                &self.orders.sequence_manager,
                EntityId::Soldier(other_id),
                npc_id,
                me_pos,
                friend_raw_position,
                doors,
                friend_seeks_with_help,
            );
            if seek_area_debug {
                let friend_selected_door = selected_pass_door_movement(
                    &self.orders.sequence_manager,
                    EntityId::Soldier(other_id),
                );
                let friend_effective_position = seek_area_friend_position_map(
                    friend_raw_position,
                    friend_selected_door
                        .map(|(door_index, direction)| (door_index, direction != 0)),
                    doors,
                );
                eprintln!(
                    "SEEKAREA {{\"event\":\"friend_contribution\",\"frame\":{},\"owner_creation_order\":{},\"friend\":{:?},\"friend_creation_order\":{},\"alert\":{:?},\"raw\":[{},{}],\"effective\":[{},{}],\"selected_door\":{:?},\"contributes\":{},\"clears_help\":{}}}",
                    frame,
                    creation_order,
                    other_id,
                    self.world
                        .original_creation_order(EntityId::Soldier(other_id)),
                    alert_status,
                    friend_raw_position.x,
                    friend_raw_position.y,
                    friend_effective_position.x,
                    friend_effective_position.y,
                    friend_selected_door,
                    contribution.is_some(),
                    contribution.unwrap_or(false),
                );
            }
            let Some(clears_help) = contribution else {
                continue;
            };
            tick.visible_seeking_friends += 1;
            if clears_help {
                tick.friend_seek_clears_help_flag = true;
            }
        }
        if seek_area_debug {
            eprintln!(
                "SEEKAREA {{\"event\":\"friend_summary\",\"frame\":{},\"owner_creation_order\":{},\"visible_friends\":{},\"clears_help\":{}}}",
                frame,
                creation_order,
                tick.visible_seeking_friends,
                tick.friend_seek_clears_help_flag,
            );
        }
        tick.camp_soldiers =
            self.build_camp_soldier_tick_infos(npc_id, my_camp, scratch, build_forecasts);
        // Sequence/timer callbacks run outside RefreshDetection, but Original
        // still walks the live camp registry from those Think boundaries.
        // Keep the parallel KO list live as well: money-fight victim scans
        // must not inherit an empty `AiPerTickData::stub()` field merely
        // because their EVENT_DONE came from sequence completion.
        tick.camp_unconscious_soldiers = self
            .ai
            .global
            .all_soldier_handles
            .iter()
            .filter_map(|&handle| {
                if handle == npc_id.index() {
                    return None;
                }
                // The authored handle list is the Original camp-array order.
                // Resolve its current typed occupant so a removed soldier's
                // recycled slot cannot turn a civilian/object into a soldier.
                let current_id =
                    crate::element::EntityId::Soldier(crate::entity_id::SoldierId(handle));
                let Some(crate::element::Entity::Soldier(soldier)) =
                    self.world.entities.get(current_id)
                else {
                    return None;
                };
                if !self.camps_are_allied(soldier.soldier.cached_camp, my_camp)
                    || !soldier.element.active
                    || soldier.npc.life_points <= 0
                    || !soldier.human.unconscious
                {
                    return None;
                }
                let knocked_out_in_money_fight = soldier
                    .npc
                    .ai_brain
                    .base()
                    .map(|ai| ai.knocked_out_in_money_fight)
                    .unwrap_or(false);
                Some(crate::ai_enemy::CampUnconsciousSoldierInfo {
                    handle,
                    knocked_out_in_money_fight,
                })
            })
            .collect();
        tick.alert_soldier_candidates = self.build_alert_soldier_candidates(npc_id);
        if build_forecasts
            && let Some(missed_handle) = enemy_ai.missed_pc
            && let Some(missed_id) = self.entity_id_for_index(missed_handle.get())
            && let Some(missed_entity) = self.world.entities.get(missed_id)
            && let Some(input) = extract_exact_forecast_input(
                self,
                missed_entity,
                selected_actor_is_passing_door(&self.orders.sequence_manager, missed_id),
            )
        {
            let doors = self.script_domains.interactables.doors.as_slice();
            tick.missed_pc_forecast = Some(crate::ai::prepare_forecast_destination_for_ia(
                &input,
                doors,
                &self.world.fast_grid.level.sectors,
                &self.world.fast_grid.level.sector_number_map,
            ));
            tick.missed_pc_forecast_handle = enemy_ai.missed_pc;
            tick.missed_pc_is_pc = matches!(missed_entity, Entity::Pc(_));
        }
        // `fill_list_with_all_near_fighters` walks the global fighter
        // registry on every call.  Populate `nearby_fighters` here so
        // off-detection dispatch sites (timer events, reach-point
        // events, panic, patrol, cross-NPC actions, pending-stimuli
        // drain, …) see the same fighter view that the in-detection
        // builder produces.  Without this, AI predicates that consume
        // `tick.nearby_fighters` (rider charge target lookup,
        // PhalanxIsEncercledByEnemies, NumberOfNearbyArchersWhoNeedProtection,
        // ReconsiderPhalanx geometry, IsAnyFriendInThisPolygon)
        // observe an empty list outside swordfight substates.
        tick.nearby_fighters =
            self.build_nearby_fighters_for(npc_id, assets, &scratch.ai_sight_obstacles);
        tick.fighter_registry = self.build_fighter_snapshots_for(npc_id, assets, None);
        tick.reconsider_swordfight_observation_fighters = tick
            .fighter_registry
            .iter()
            .map(|fighter| {
                let id = self.entity_id_for_index(fighter.handle).unwrap_or_else(|| {
                    panic!(
                        "NPC {} observation fighter {} disappeared from the registry",
                        npc_id.index(),
                        fighter.handle
                    )
                });
                let raw_world_position = self
                    .world
                    .entities
                    .get(id)
                    .unwrap_or_else(|| {
                        panic!(
                            "NPC {} observation fighter {} has no live entity",
                            npc_id.index(),
                            fighter.handle
                        )
                    })
                    .element_data()
                    .position();
                crate::ai::ReconsiderSwordfightObservationFighter {
                    handle: fighter.handle,
                    raw_world_position,
                    is_friendly: fighter.is_friendly,
                    is_able_to_fight: fighter.is_able_to_fight,
                    is_soldier: fighter.is_soldier,
                    primary_target: fighter.primary_target,
                    current_substate: fighter.current_substate,
                }
            })
            .collect();
        if reconsider_observation_debug_enabled() {
            let creation_order = self.world.original_creation_order(npc_id);
            if reconsider_observation_debug_matches(frame, creation_order, me_handle) {
                eprintln!(
                    "RECONSIDER {{\"event\":\"snapshot_begin\",\"frame\":{},\"owner\":{},\"owner_creation_order\":{},\"owner_state\":{:?},\"owner_substate\":{:?},\"registry_len\":{}}}",
                    frame,
                    me_handle,
                    creation_order,
                    ai_actor.ai_state(),
                    ai_actor.ai_substate(),
                    tick.reconsider_swordfight_observation_fighters.len(),
                );
                for (ordinal, fighter) in tick
                    .reconsider_swordfight_observation_fighters
                    .iter()
                    .enumerate()
                {
                    let fighter_id =
                        self.entity_id_for_index(fighter.handle).unwrap_or_else(|| {
                            panic!(
                                "RECONSIDER owner {npc_id:?} cannot resolve fighter {}",
                                fighter.handle
                            )
                        });
                    let entity = self.world.entities.get(fighter_id).unwrap_or_else(|| {
                        panic!("RECONSIDER owner {npc_id:?} cannot read fighter {fighter_id:?}")
                    });
                    let current_sequence = self
                        .orders
                        .sequence_manager
                        .current_element_for_actor(fighter_id)
                        .and_then(|(sequence_id, element_index)| {
                            self.orders
                                .sequence_manager
                                .get_element(sequence_id, element_index)
                                .map(|element| {
                                    (sequence_id, element_index, element.command, element.state)
                                })
                        });
                    let eligibility = match entity {
                        Entity::Soldier(other) => Some((
                            other.npc.life_points <= 0,
                            other.human.unconscious,
                            other.element.posture == crate::element::Posture::Tied,
                            other.human.carrier.is_some(),
                            other.element.active,
                            other.npc.ai_state(),
                            other.npc.ai_substate(),
                        )),
                        _ => None,
                    };
                    eprintln!(
                        "RECONSIDER {{\"event\":\"snapshot_fighter\",\"frame\":{},\"owner_creation_order\":{},\"ordinal\":{},\"fighter\":{},\"fighter_creation_order\":{},\"friendly\":{},\"able\":{},\"raw\":[{},{},{}],\"soldier_eligibility\":{:?},\"current_sequence\":{:?}}}",
                        frame,
                        creation_order,
                        ordinal,
                        fighter.handle,
                        self.world.original_creation_order(fighter_id),
                        fighter.is_friendly,
                        fighter.is_able_to_fight,
                        fighter.raw_world_position.x,
                        fighter.raw_world_position.y,
                        fighter.raw_world_position.z,
                        eligibility,
                        current_sequence,
                    );
                }
            }
        }
        // KillNearbySleepingEnemies walks the live opposing-camp fighter
        // registry synchronously at the BattleDecisions boundary. Populate
        // this from the same live registry for every Think, including timer
        // events that do not own a RefreshDetection VIEW/OUTOFVIEW aggregate.
        // Restricting this field to the detection aggregate leaves the final
        // fallback blind whenever no optical stimulus was queued.
        tick.nearby_sleeping_enemies =
            sleeping_enemy_candidates_from_fighter_registry(&tick.fighter_registry);
        tick.reconsider_swordfight_enemies = tick
            .fighter_registry
            .iter()
            .filter(|fighter| !fighter.is_friendly)
            .cloned()
            .collect();
        tick.reconsider_swordfight_friends = self.build_reconsider_swordfight_friends_for(npc_id);

        // Phalanx right-chain "them" snapshots — consumed by
        // `PhalanxReinitializeThemList` so the leftmost member can
        // union each neighbour's enemies via the recursion.  Always
        // populated; empty when this NPC has no right neighbour.
        tick.phalanx_member_them_lists = self.build_phalanx_member_them_lists(npc_id);

        // Patrol-chief data is live per-dispatch context, not specific to
        // CALL_PATROL_COORDINATE. ReturnToDuty can synchronously enter
        // DefaultGotoChief and immediately receive EventReachPoint; that
        // handler faces this position on the same Think stack. Leaving the
        // general builder's stub origin here made every such member face
        // sector 15 even though the preceding GoNear used the real chief.
        if let Some(chief_id) = ai.patrol_chief {
            let chief = self.world.entities.get(chief_id).unwrap_or_else(|| {
                panic!(
                    "enemy tick context owner {} has stale patrol chief {}",
                    npc_id.index(),
                    chief_id.index()
                )
            });
            let chief_ai = chief.ai_controller().unwrap_or_else(|| {
                panic!(
                    "enemy tick context patrol chief {} has no AI",
                    chief_id.index()
                )
            });
            // CoordinatePatrol subtracts `Position(mpPatrolChief)`, not the
            // chief's literal sprite position
            // (`original-code/RHartificialintelligence.cpp:7845-7847`).
            // `Position` substitutes the committed gate side while an actor's
            // selected element is PassDoor (:4365-4378); this matters while
            // the chief is still interpolating along the door rail.
            tick.patrol_chief_position = super::resolve_ai_position_with(
                &self.world.entities,
                self.script_domains.interactables.doors.as_slice(),
                &self.orders.sequence_manager,
                chief_id,
                |position_id| {
                    let element = self
                        .world
                        .entities
                        .get(position_id)
                        .unwrap_or_else(|| {
                            panic!("patrol-chief position owner {position_id:?} disappeared")
                        })
                        .element_data();
                    crate::ai::Position {
                        x: element.position_map().x,
                        y: element.position_map().y,
                        sector: element.sector(),
                        level: element.layer(),
                    }
                },
            )
            .effective;
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
            // No target selected — primary-target fields stay None,
            // enemy_sq_distances stays empty.  Friend-swap still
            // scans the other soldiers; the helper handles the
            // empty-target case.
            tick.friend_swap_candidates = build_friend_swap_candidates(
                &self.world.entities,
                &self.mission_domain.diplomacy,
                doors,
                &self.orders.sequence_manager,
                npc_id,
                my_camp,
                |element| super::ai_view_position_sector(self, element),
            );
            return tick;
        };

        // Primary target metadata (position, posture, animation,
        // carrier) from the live entity store.
        let target_meta = lookup_primary_target_metadata(self, target_id);

        if let Some((pos, posture, anim, carrier_pos, carrier_handle)) = target_meta {
            tick.primary_target_position = Some(pos);
            let target = self
                .world
                .entities
                .get(target_id)
                .unwrap_or_else(|| panic!("resolved primary target {target_id:?} disappeared"));
            let element = target.element_data();
            tick.primary_target_live_position = Some(crate::ai::Position {
                x: element.position_map().x,
                y: element.position_map().y,
                sector: element.sector(),
                level: element.layer(),
            });
            tick.primary_target_posture = Some(posture);
            tick.primary_target_animation = anim;
            tick.primary_target_carrier_position = carrier_pos;
            tick.primary_target_carrier_handle = carrier_handle;

            // Seed enemy_sq_distances from the primary target so
            // `battle_decisions` sees a non-empty list when the
            // soldier has a valid target — same rationale as the
            // timer-dispatch seed at line 5916.
            let dx = pos.x - me_pos.x;
            let dy = (pos.y - me_pos.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
            let sq = (dx * dx + dy * dy) as i32;
            tick.enemy_sq_distances.push((target_id.index(), sq));
            tick.min_sq_enemy_distance = sq;
        }

        // primary_target_is_pc: look up the target's entity variant.
        tick.primary_target_is_pc =
            matches!(self.world.entities.get(target_id), Some(Entity::Pc(_)));
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
        tick.primary_target_jump_line = crate::engine::melee::is_table_swordfight_needed(
            &self.world.entities,
            &self.world.fast_grid,
            &assets.profile_manager,
            npc_id,
            target_id,
        );
        tick.primary_target_multiplicity = self
            .ai
            .global
            .primary_target_multiplicity_scratch
            .iter()
            .map(|(&target, &count)| (target, count))
            .collect();

        {
            let my_company = enemy_ai.company_number;
            let my_pride = enemy_ai.soldier_profile_pride;
            tick.us_battle_points = 100 + my_pride as u32;

            let self_to_target_sq = tick.primary_target_position.map(|target_pos| {
                let dx = target_pos.x - me_pos.x;
                let dy =
                    (target_pos.y - me_pos.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
                dx * dx + dy * dy
            });

            for friend in &tick.nearby_fighters {
                if !friend.is_friendly || friend.handle == me_handle || !friend.is_able_to_fight {
                    continue;
                }

                if friend.is_pc {
                    tick.us_battle_points += 100;
                    if my_company > 0 {
                        tick.friends_lower_company = tick.friends_lower_company.saturating_add(1);
                    }
                    continue;
                }

                if !matches!(
                    friend.ai_state,
                    crate::ai::AiState::Default
                        | crate::ai::AiState::Wondering
                        | crate::ai::AiState::Seeking
                        | crate::ai::AiState::Attacking
                ) {
                    continue;
                }

                let friend_company = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == friend.handle)
                    .map(|cs| cs.company_number)
                    .unwrap_or(u16::MAX);
                if my_company > friend_company
                    && (ai.current_substate == crate::ai::Substate::AttackingReactiontime
                        || friend.ai_state == crate::ai::AiState::Attacking)
                {
                    tick.friends_lower_company = tick.friends_lower_company.saturating_add(1);
                }

                if my_pride > friend.soldier_profile_pride {
                    tick.soldiers_lower_pride = true;
                }
                tick.us_battle_points += 100 + friend.soldier_profile_pride as u32;

                if friend.rank == crate::profiles::ProfileRank::Soldier {
                    tick.simple_soldiers_near = true;
                }
                if friend.rank == crate::profiles::ProfileRank::Officer {
                    tick.has_officer_nearby = true;
                }

                if friend.ai_state == crate::ai::AiState::Attacking
                    && friend.primary_target.is_some()
                {
                    if crate::ai_enemy::is_any_swordfight_substate(friend.current_substate) {
                        tick.friends_nearer_to_enemy =
                            tick.friends_nearer_to_enemy.saturating_add(1);
                    } else if let Some(self_sq) = self_to_target_sq {
                        let Some(target_pos) = tick.primary_target_position else {
                            continue;
                        };
                        let dx = friend.position.x - target_pos.x;
                        // Original deliberately compares two differently
                        // shaped distances here: `SquareDistance(primary)`
                        // stretches the owner's Y delta, while the friend's
                        // raw `RHposition` delta uses an unstretched
                        // `SquareNorm()`.
                        let dy = friend.position.y - target_pos.y;
                        if dx * dx + dy * dy < self_sq {
                            tick.friends_nearer_to_enemy =
                                tick.friends_nearer_to_enemy.saturating_add(1);
                        }
                    }
                }
            }

            for &(attacker, target) in &self.ai.global.same_frame_target_claims {
                if attacker == me_handle || target == 0 {
                    continue;
                }
                let attacker_id = EntityId::Soldier(SoldierId(attacker));
                let Some(Entity::Soldier(s)) = self.world.entities.get(attacker_id) else {
                    continue;
                };
                if !self.camps_are_allied(s.soldier.cached_camp, my_camp)
                    || !s.element.active
                    || s.human.unconscious
                    || s.npc.life_points <= 0
                {
                    continue;
                }
                if Some(crate::ai::AiEntityHandle::new(target)) == primary_target_handle {
                    tick.friends_nearer_to_enemy = tick.friends_nearer_to_enemy.saturating_add(1);
                }
            }
        }

        // Friend-swap candidates for ReconsiderEnemyApproach.
        tick.friend_swap_candidates = build_friend_swap_candidates(
            &self.world.entities,
            &self.mission_domain.diplomacy,
            doors,
            &self.orders.sequence_manager,
            npc_id,
            my_camp,
            |element| super::ai_view_position_sector(self, element),
        );

        // Stashed-exit-door snapshot for the AlertSoldiers indoor
        // branch and the merry-man flee path.  Always populated
        // whenever the AI has stashed a door (irrespective of
        // in-building status), so paths that reach the door's
        // point_out through a sequence of substates still see the
        // cached geometry.  No fallback when no door is stashed.
        let stashed = ai.my_door_index;
        if stashed.is_some() {
            assert!(
                self.scripts.mission.is_some(),
                "stashed AI exit-door state requires an installed mission script"
            );
            let doors_slice = self.script_domains.interactables.doors.as_slice();
            tick.my_exit_door = build_my_exit_door_info(stashed, doors_slice);
        }

        tick
    }

    /// Rank-soldier NPCs of *every* camp, in NPC registry order.
    ///
    /// `CommandSoldiersToAttack` walks the whole NPC array and gates each
    /// entry on rank, body state and the candidate's own 360° detection of
    /// the officer — no camp test anywhere. Feeding it the same-camp
    /// snapshot dropped the opposing camp's soldiers from both the alert
    /// broadcast and the observable detection-call stream.
    fn build_alert_soldier_candidates(
        &self,
        npc_id: crate::element::EntityId,
    ) -> Vec<crate::ai_enemy::AlertSoldierCandidate> {
        let mut candidates = Vec::new();
        for other_id in self.world.entities.npc_ids() {
            if other_id == npc_id {
                continue;
            }
            let Some(entity) = self.world.entities.get(other_id) else {
                continue;
            };
            let crate::element::Entity::Soldier(s) = entity else {
                continue;
            };
            let Some(enemy_ai) = s.npc.ai_brain.enemy() else {
                continue;
            };
            if enemy_ai.soldier_profile_rank != crate::profiles::ProfileRank::Soldier
                || !crate::element::Human::is_able_to_fight(s)
            {
                continue;
            }
            let position = s.element.position_map();
            candidates.push(crate::ai_enemy::AlertSoldierCandidate {
                handle: other_id.index(),
                position: crate::ai::Position {
                    x: position.x,
                    y: position.y,
                    sector: s.element.sector(),
                    level: s.element.layer(),
                },
                elevation: s.element.sprite.position_iface.get_elevation(),
                is_rider: s.soldier.rider,
                view_radius: s.npc.view_radius,
                in_building: self.entity_data_in_building_sector(&s.element),
            });
        }
        candidates
    }

    fn build_camp_soldier_tick_infos(
        &self,
        npc_id: crate::element::EntityId,
        my_camp: crate::element::Camp,
        _scratch: &SimScratch,
        forecast_destinations: bool,
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
            // GetNumberOfSoldiers(camp) includes unconscious and inactive
            // soldiers. Individual Original consumers apply their own gates:
            // CreateListOfSoldiersYouCanAlert retains everyone of the allowed
            // rank, while GetNearestFighter rejects dead, unconscious, and
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
            let forecast_destination = if forecast_destinations {
                // Missing scripts are a recoverable developer-data load path;
                // `init_ai` warns once before these per-NPC snapshots are built.
                let doors = self
                    .scripts
                    .mission
                    .as_ref()
                    .map(|_| self.script_domains.interactables.doors.as_slice())
                    .unwrap_or(&[]);
                let pos_now = s.element.position_map();
                let live_door = s.element.sprite.position_iface.get_door();
                let door_pass = selected_actor_is_passing_door(
                    &self.orders.sequence_manager,
                    EntityId::Soldier(other_id),
                )
                .then_some(live_door)
                .flatten()
                .map(|door| (door, s.actor.passing_door_directly));
                let input = crate::ai::ForecastInput {
                    position_map_x: pos_now.x,
                    position_map_y: pos_now.y,
                    sector: s.element.sector().map(u16::from).unwrap_or(0),
                    sector_handle: s.element.sector(),
                    layer: s.element.layer(),
                    direction: s.element.direction() as u16,
                    forecasted_movement_z: s
                        .element
                        .sprite
                        .position_iface
                        .get_forecasted_movement()
                        .z,
                    door_pass,
                    passing_door_directly: s.actor.passing_door_directly,
                };
                Some(crate::ai::prepare_forecast_destination_for_ia(
                    &input,
                    doors,
                    &self.world.fast_grid.level.sectors,
                    &self.world.fast_grid.level.sector_number_map,
                ))
            } else {
                None
            };
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
                report_type: enemy_ai.base.my_reconnaissance_report.report_type,
                report_seek_position: enemy_ai.base.my_reconnaissance_report.seek_position,
                report_seen_bodies: enemy_ai.base.my_reconnaissance_report.seen_bodies.clone(),
                report_charly: enemy_ai.base.my_reconnaissance_report.charly,
                alert_soldiers_point: enemy_ai.base.alert_soldiers_point,
                patrol_chief: enemy_ai.base.patrol_chief,
                antagonist: enemy_ai.base.antagonist,
                detected_body: enemy_ai.base.detected_body,
                blood_alcohol: enemy_ai.base.blood_alcohol,
                duty_flag: enemy_ai.soldier_profile_duty,
                is_tower_guard: enemy_ai.tower_guard,
                company_number: enemy_ai.company_number,
                in_building,
                forecast_destination,
                detectable_bodies,
                seek_position: enemy_ai.base.seek_position,
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
        _sight_obstacles: &crate::sight_obstacle::SharedSightObstacles,
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

        let Some(owner) = self.world.entities.get(npc_id) else {
            return Vec::new();
        };
        let Some(enemy_ai) = owner.enemy_ai() else {
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
                        .world
                        .entities
                        .get(position_id)
                        .unwrap_or_else(|| {
                            panic!("fighter snapshot owner {position_id:?} disappeared")
                        })
                        .element_data();
                    Position {
                        x: element.position_map().x,
                        y: element.position_map().y,
                        // `Position(element)` carries the exact `RHSector*`.
                        // Legacy-loaded actors can retain only its public
                        // number on ElementData, so recover the arena identity
                        // before combat-position proposals copy this snapshot
                        // into a cross-door GoTo destination.
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
        // The original fighter registry retains inactive and out-of-order
        // soldiers, and `FillListWithAllNearFighters` inserts self before it
        // applies `IsAbleToFight` to the remaining registry entries.
        let build_soldier = |handle: u32, require_able: bool| -> Option<FighterSnapshot> {
            let s = self.world.entities.get_soldier(SoldierId(handle))?;
            let is_able_to_fight = s.is_able_to_fight();
            if require_able && !is_able_to_fight {
                return None;
            }
            let position = fighter_position(EntityId::Soldier(SoldierId(handle)));
            // `RHElement::GetPosition()` — no door-transit or carrier
            // substitution. Range gates phrased as `SquareDistance` read
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
            let soldier_profile = assets
                .profile_manager
                .get_soldier(s.soldier.soldier_profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "soldier {handle} requires missing soldier profile {}",
                        u32::from(s.soldier.soldier_profile_index)
                    )
                });
            let has_formation = soldier_profile.formation;
            let fighting_ability = {
                let base = soldier_profile.fighting;
                if self.camps_are_hostile(s.soldier.cached_camp, Camp::Royalists) {
                    let diff = self.control.sim_config.difficulty;
                    diff.rules().enemy_fighting(base, 100)
                } else {
                    base
                }
            };
            let bow_profile = if soldier_profile.shooting_weapon_id == 0 {
                None
            } else {
                Some(
                    assets
                        .profile_manager
                        .get_bow(soldier_profile.shooting_weapon_id)
                        .unwrap_or_else(|| {
                            panic!(
                                "soldier {handle} requires missing bow profile {}",
                                soldier_profile.shooting_weapon_id
                            )
                        }),
                )
            };
            let is_archer_unit = snapshots::is_archer_from_bow(bow_profile);
            let bow_max_range = bow_profile
                .map(|bow| {
                    if bow.has_long_shoot {
                        bow.long_shoot.range
                    } else {
                        bow.normal_shoot.range
                    }
                })
                .unwrap_or(0);
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
            let weapon_is_shield = hth_profile.shield;
            let has_shield_anim = s
                .element
                .sprite
                .has_animation(crate::order::OrderType::WaitingShield);
            let is_shield_bearer = weapon_is_shield && has_shield_anim;
            let in_recovery = self.actor_is_in_sword_recovery(EntityId::Soldier(SoldierId(handle)));
            // `mposSeekPosition` is an `RHposition` and carries its own
            // `uwLevel`; it is never re-levelled from the soldier's current
            // element layer. `ComputePositionBehindMyShieldBearer`
            // (`original-code/RHartificialmalignity.cpp:18005-18011`) copies
            // that level into the cover point and hands it to
            // `IsStraightMovementAutorized`, so a shield bearer running from
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
                    // `Position(entity)` in Original copies the complete
                    // RHposition, including its authoritative sector
                    // pointer.  Combat helpers later copy this position
                    // when deriving destinations (notably the archer
                    // cover point behind a stationary shield bearer), so
                    // discarding the sector here turns an otherwise valid
                    // same-sector GoTo into EVENT_COULDNT_REACHPOINT.
                    sector: position.sector,
                    level: position.level,
                },
                raw_position,
                direction: s.element.direction() as u16,
                is_friendly,
                is_swordfighting: !s.human.opponents.is_empty(),
                is_able_to_fight,
                is_tied: s.element.posture == Posture::Tied,
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
                has_formation,
                is_shield_bearer,
                is_archer_unit,
                is_tower_guard: enemy_ai_other.tower_guard,
                is_vip: soldier_profile.vip,
                soldier_profile_pride: enemy_ai_other.soldier_profile_pride,
                is_robin: false,
                left_combat_neighbour: enemy_ai_other.left_combat_neighbour,
                right_combat_neighbour: enemy_ai_other.right_combat_neighbour,
                is_in_recovery_animation: in_recovery,
                in_sword_action_state: s.actor.action_state.is_sword(),
                elevation: s.element.position().z,
                seek_position,
                current_substate: s.npc.ai_substate() as u32,
                archer_behind_me: enemy_ai_other.archer_behind_me,
                ai_state: s.npc.ai_state(),
                shield_bearer_before_me: enemy_ai_other.shield_bearer_before_me,
                hth_weapon_id: hth_id,
                action_state: s.actor.action_state,
                shield_bearer_direction: enemy_ai_other.shield_bearer_direction,
                shield_bearer_seek_position: seek_position,
                bow_max_range,
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
                && !matches!(pc.element.posture, Posture::Tree | Posture::Spy);
            if require_able && !is_able_to_fight {
                return None;
            }
            let is_carried = pc.human.carrier.is_some();
            let position = fighter_position(EntityId::Pc(PcId(handle)));
            // `RHElement::GetPosition()` — see the soldier branch.
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
                is_tied: pc.element.posture == Posture::Tied,
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
                has_formation: false,
                is_shield_bearer: false,
                is_archer_unit: false,
                is_tower_guard: false,
                is_vip: character.vip,
                soldier_profile_pride: 0,
                is_robin: pc.pc.robin,
                left_combat_neighbour: None,
                right_combat_neighbour: None,
                is_in_recovery_animation: in_recovery,
                in_sword_action_state: pc.actor.action_state.is_sword(),
                elevation: pc.element.sprite.position_iface.get_elevation(),
                seek_position: pc_seek_position,
                current_substate: pc
                    .pc
                    .ai
                    .as_deref()
                    .map(|ai| ai.ai_substate() as u32)
                    .unwrap_or(0),
                archer_behind_me: None,
                ai_state: pc
                    .pc
                    .ai
                    .as_deref()
                    .map(crate::element::AiActorData::ai_state)
                    .unwrap_or_default(),
                shield_bearer_before_me: None,
                hth_weapon_id: hth_id,
                action_state: pc.actor.action_state,
                shield_bearer_direction: 0,
                shield_bearer_seek_position: pc_seek_position,
                bow_max_range: 0,
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
        // interleaved. Friendly scans still put `mpMe` first, as
        // `FillListWithAllNearFighters` does explicitly.
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
                    // FillListWithAllNearFighters and therefore exclude
                    // unable fighters. The complete registry is the backing
                    // store for already-held Original pointers, which remain
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
            // `MaxNormDistance` subtracts full world positions before
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

    /// Build the friendly half of Original `ReconsiderSwordfight`'s
    /// per-call camp-fighter scan.
    ///
    /// This cannot reuse `build_nearby_fighters_for`: that shared cache uses
    /// projected map positions and filters non-self soldiers through
    /// `IsAbleToFight`, while `ReconsiderSwordfight` walks every fighter in
    /// `marrayFighters[myCamp]`, tests only `IsSwordfighting`, and computes
    /// `MaxNormDistance` from full 3D world positions.
    fn build_reconsider_swordfight_friends_for(
        &self,
        npc_id: crate::element::EntityId,
    ) -> Vec<crate::ai::ReconsiderSwordfightFriend> {
        use crate::ai::ReconsiderSwordfightFriend;

        let Some(me) = self.world.entities.get(npc_id) else {
            return Vec::new();
        };
        let Some(me_ai) = me.enemy_ai() else {
            return Vec::new();
        };
        let me_world = me.element_data().position();
        let my_camp = me.camp();
        let radius = crate::parameters_ai::MAX_SWORDFIGHT_CONSIDERATION_RADIUS as u16;
        let mut out = Vec::new();

        // Walk the registration order, matching the Original's append-only
        // camp fighter arrays (including the PC/soldier interleaving
        // established during level creation).
        for id in self.world.fighter_registry_order() {
            let Some(entity) = self.world.entities.get(id) else {
                continue;
            };
            let Some(opponents) = entity.human_data().map(|human| &human.opponents) else {
                continue;
            };
            let handle = id.index();
            let world = entity.element_data().position();
            let same_camp = self.camps_are_allied(entity.camp(), my_camp);
            if handle == me_ai.base.me || !same_camp || opponents.is_empty() {
                continue;
            }

            let dx = (world.x - me_world.x).abs();
            let dy =
                ((world.y - me_world.y) * crate::position_interface::INVERSE_ASPECT_RATIO).abs();
            let dz = (world.z - me_world.z).abs();
            // Original explicitly casts the FLOAT result to UWORD before
            // comparing it with MAX_SWORDFIGHT_CONSIDERATION_RADIUS.
            let max_norm_distance = dx.max(dy).max(dz) as u16;
            if max_norm_distance >= radius {
                continue;
            }
            out.push(ReconsiderSwordfightFriend {
                handle,
                max_norm_distance,
                number_of_opponents: opponents.len().min(u16::MAX as usize) as u16,
            });
        }
        out
    }

    /// Snapshot every right-chain phalanx member (including self) with
    /// their live viewer state, persistent `list_them`, and current
    /// detectable-enemy list. `PhalanxReinitializeThemList` replays the
    /// original recursion over this data without borrowing sibling AI
    /// brains while one member is mutable.
    pub(in crate::engine) fn build_phalanx_member_them_lists(
        &self,
        npc_id: crate::element::EntityId,
    ) -> Vec<crate::ai::PhalanxMemberThemList> {
        use crate::ai::{PhalanxEnemySnapshot, PhalanxMemberThemList, Position};
        use crate::element::Human;
        let Some(owner) = self.world.entities.get(npc_id) else {
            return Vec::new();
        };
        let Some(enemy_ai) = owner.enemy_ai() else {
            return Vec::new();
        };

        let snapshot_enemy = |handle: u32, member_camp: Camp| -> PhalanxEnemySnapshot {
            let entity_id = self.entity_id_for_index(handle).unwrap_or_else(|| {
                panic!("phalanx member references missing enemy handle {handle}")
            });
            let entity = self.world.entities.get(entity_id).unwrap_or_else(|| {
                panic!("phalanx enemy handle {handle} resolved to a vacant entity slot")
            });
            let human = entity
                .human_data()
                .unwrap_or_else(|| panic!("phalanx enemy handle {handle} is not a human entity"));
            let element = entity.element_data();
            let map = element.position_map();
            let able_to_fight = match entity {
                Entity::Pc(pc) => pc.is_able_to_fight(),
                Entity::Soldier(soldier) => soldier.is_able_to_fight(),
                Entity::Civilian(civilian) => civilian.is_able_to_fight(),
                _ => unreachable!("human_data returned Some for a non-human entity"),
            };
            PhalanxEnemySnapshot {
                handle,
                position: Position {
                    x: map.x,
                    y: map.y,
                    sector: element.sector(),
                    level: element.layer(),
                },
                world_position: entity.position_iface().get_position(),
                direction: element.direction() as u16,
                posture: element.posture,
                elevation: entity.position_iface().get_elevation(),
                is_rider: entity.soldier_data().is_some_and(|data| data.rider),
                active: element.active,
                able_to_fight,
                dead: entity.is_dead(),
                unconscious: human.unconscious,
                friend: self.camps_are_allied(entity.camp(), member_camp),
                in_building: self.entity_data_in_building_sector(element),
                obstacle: element.obstacle_index(),
            }
        };

        let mut out: Vec<PhalanxMemberThemList> = Vec::new();
        let mut current = enemy_ai.base.me;
        // Cap at 16 like the consumer's right-chain walk; phalanxes are
        // small and the cap guards against any cycle in cached neighbour
        // links.
        for _ in 0..16 {
            if current == 0 {
                break;
            }
            let member_id = self.expect_human_id_for_ai_handle(current, "phalanx member");
            let member = self
                .world
                .entities
                .get(member_id)
                .expect("validated phalanx member vanished");
            let ai_actor = member
                .ai_actor_data()
                .unwrap_or_else(|| panic!("phalanx member human {current} has no AI actor data"));
            let neighbour_ai = member
                .enemy_ai()
                .unwrap_or_else(|| panic!("phalanx member human {current} has no EnemyAi"));
            let element = member.element_data();
            let pos = element.position_map();
            let member_camp = member.camp();
            let current_them_list = neighbour_ai
                .list_them
                .iter()
                .map(|&handle| snapshot_enemy(handle, member_camp))
                .collect();
            let enemy_list = ai_actor
                .detectable_lists
                .get(crate::element::DetectableType::Enemy as usize)
                .unwrap_or_else(|| panic!("phalanx member {current} has no detectable-enemy list"));
            let detectable_enemies = enemy_list
                .iter()
                .map(|detectable| {
                    let entity_id = detectable.element.unwrap_or_else(|| {
                        panic!("phalanx member {current} has a null detectable enemy")
                    });
                    snapshot_enemy(entity_id.index(), member_camp)
                })
                .collect();
            out.push(PhalanxMemberThemList {
                handle: current,
                entity: self.entity_id_for_index(current).unwrap_or_else(|| {
                    panic!("phalanx member handle {current} is absent from the entity table")
                }),
                current_them_list,
                detectable_enemies,
                position: Position {
                    x: pos.x,
                    y: pos.y,
                    sector: element.sector(),
                    level: element.layer(),
                },
                world_position: member.position_iface().get_position(),
                direction: element.direction() as u16,
                posture: element.posture,
                elevation: member.position_iface().get_elevation(),
                is_rider: member.soldier_data().is_some_and(|soldier| soldier.rider),
                active: element.active,
                in_building: self.entity_data_in_building_sector(element),
                view_radius: ai_actor.view_radius,
                view_direction: ai_actor.view_direction,
                real_half_aperture: ai_actor.real_half_aperture,
                sq_view_radius: (ai_actor.view_radius as f32) * (ai_actor.view_radius as f32),
            });
            let next = neighbour_ai.right_combat_neighbour;
            let Some(next) = next else {
                break;
            };
            if next.get() == current {
                break;
            }
            current = next.get();
        }
        out
    }
}
