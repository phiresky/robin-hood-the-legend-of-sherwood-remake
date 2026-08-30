use super::*;

impl EngineInner {
    fn required_cross_npc_enemy_mut(
        &mut self,
        target: u32,
        operation: &str,
    ) -> &mut crate::ai_enemy::EnemyAi {
        // Original combat-neighbour APIs take RHElementActorHuman pointers.
        // `HumanHandle` is the raw sparse element slot, not a SoldierId; an
        // AI-controlled hero therefore has to retain its ActorPc entity kind here.
        let target_id = self.expect_human_id_for_ai_handle(target, operation);
        self.world
            .entities
            .get_mut(target_id)
            .expect("validated cross-NPC human vanished")
            .enemy_ai_mut()
            .unwrap_or_else(|| {
                panic!("cross-NPC {operation} target human {target} has no enemy AI")
            })
    }

    /// Execute the complete Original `ClearPatrol` call made by the
    /// `RemoveAllSubordinates` script native.
    ///
    /// `RHArtificialIntelligence::ClearPatrol` clears each member's chief
    /// pointer and calls `ForceReturnToDuty` directly before clearing the
    /// chief's lists. `ForceReturnToDuty` is an inline virtual
    /// `ReturnToDuty()` call, not `Think(EVENT_RETURN_TO_DUTY)`: in
    /// particular, it bypasses `StartThink`'s script-lock refusal. Keep the
    /// direct duty transition, movement construction, and recursive callbacks
    /// inside this engine-owned script barrier while leaving ordinary owner
    /// instruction to the subsequent `RHSequenceManager::Hourglass`.
    pub(crate) fn script_remove_all_subordinates(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        chief: EntityId,
    ) {
        let members = self
            .world
            .entities
            .get(chief)
            .and_then(Entity::ai_controller)
            .unwrap_or_else(|| {
                panic!(
                    "RemoveAllSubordinates chief {} is not an NPC",
                    chief.index()
                )
            })
            .theoretical_patrol
            .clone();

        for member in members.iter().copied() {
            let should_return = {
                let ai = self
                    .world
                    .entities
                    .get_mut(member)
                    .and_then(Entity::ai_controller_mut)
                    .unwrap_or_else(|| {
                        panic!(
                            "RemoveAllSubordinates chief {} references missing NPC member {}",
                            chief.index(),
                            member.index()
                        )
                    });
                ai.patrol_chief = None;
                ai.current_state == crate::ai::AiState::Default
            };
            if !should_return {
                continue;
            }
            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let mut ctx = {
                let entity = self.world.entities.get(member).unwrap_or_else(|| {
                    panic!(
                        "RemoveAllSubordinates member {} vanished before ForceReturnToDuty",
                        member.index()
                    )
                });
                let building_sector = self.entity_building_sector(entity.element_data().sector());
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
            self.refresh_selected_default_wait_identity(member, &mut ctx);
            let tick_data = self.build_npc_tick_data(sim, member, &scratch, assets);
            {
                let entity = self.world.entities.get_mut(member).unwrap_or_else(|| {
                    panic!(
                        "RemoveAllSubordinates member {} vanished before ForceReturnToDuty",
                        member.index()
                    )
                });
                let npc = entity.ai_actor_data_mut().unwrap_or_else(|| {
                    panic!(
                        "RemoveAllSubordinates member {} has no AI actor data",
                        member.index()
                    )
                });
                match &mut npc.ai_brain {
                    crate::element::AiBrain::Enemy(ai) => {
                        ai.return_to_duty(sim, crate::ai::DutyFlags::empty(), &ctx, &tick_data)
                    }
                    crate::element::AiBrain::Friendly(ai) => {
                        ai.return_to_duty(sim, crate::ai::DutyFlags::empty(), &ctx)
                    }
                    crate::element::AiBrain::None => panic!(
                        "RemoveAllSubordinates member {} has no AI brain",
                        member.index()
                    ),
                }
            }

            {
                let ai = self
                    .world
                    .entities
                    .get_mut(member)
                    .and_then(Entity::ai_controller_mut)
                    .expect("RemoveAllSubordinates member lost its AI after ForceReturnToDuty");
                if let Some(crate::ai::AiOwnerWork::ResumeReturnToDutyAfterPatrolInit {
                    defer_clear_patrol_close_post,
                    ..
                }) = ai.outbox.reentrant.owner_work.last_mut()
                {
                    *defer_clear_patrol_close_post = true;
                }
            }
            self.drain_direct_ai_owner_boundary_without_forecast(sim, member, assets);
            self.drain_pending_move_requests_for_owner(sim, member);
        }

        self.world
            .entities
            .get_mut(chief)
            .and_then(Entity::ai_controller_mut)
            .expect("validated RemoveAllSubordinates chief vanished")
            .clear_patrol();
    }

    // ─── One-shot noise broadcast ──────────────────────────────────

    pub(crate) fn one_shot_noise_listener_ids(&self) -> Vec<EntityId> {
        let mut npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        // `RHEngine::GetNPC(i)` follows the Original registration array.
        // Rust's typed arena order is not authoritative after save adoption,
        // where static entities may be reused under restored creation ranks.
        npc_ids.sort_by_key(|&npc_id| self.world.original_creation_order(npc_id));
        npc_ids
    }

    pub(crate) fn one_shot_noise(
        &self,
        noise_type: crate::ai::NoiseType,
        origin: crate::coordinates::MapPoint,
        origin_layer: Option<crate::position_interface::Layer>,
        volume: u16,
        elevation: u16,
        source_entity: Option<EntityId>,
    ) -> crate::ai::Noise {
        use crate::ai::{Noise, NoiseType};

        let element_id = match noise_type {
            NoiseType::TapTapTap | NoiseType::ZingZing | NoiseType::Aaargh | NoiseType::Heeelp => {
                source_entity.map(|id| id.index() as u16).unwrap_or(0)
            }
            _ => 0,
        };

        // RHnoise keeps the complete RHposition supplied by the source,
        // including its motion-sector pointer. Delayed reactions later feed
        // that position through PositionToPoint3D, so dropping the sector
        // also drops authored elevation. Only inherit it when the supplied
        // source still describes this exact noise origin.
        let origin_sector = source_entity
            .and_then(|id| self.world.entities.get(id))
            .filter(|entity| {
                entity.element_data().position_map() == origin
                    && entity.element_data().optional_layer() == origin_layer
            })
            .and_then(|entity| entity.element_data().sector());

        Noise {
            origin: crate::ai::NoiseOrigin {
                x: origin.x,
                y: origin.y,
                sector: origin_sector,
                layer: origin_layer,
            },
            noise_type,
            volume,
            elevation,
            element_id,
        }
    }

    /// Compute one listener's live subjective copy of a one-shot noise.
    ///
    /// This deliberately mutates deafness at the listener slot. Original
    /// `Noise` calls `GetHearVolume` immediately before that listener's
    /// synchronous `Think`, so earlier listeners may alter world state before
    /// this method is called for the next registration-array entry.
    pub(crate) fn subjective_one_shot_noise_for(
        &mut self,
        npc_id: EntityId,
        noise: crate::ai::Noise,
    ) -> Option<crate::ai::Noise> {
        const HEARING_FACTOR: f32 = 1.0;

        let (npc_pos, npc_world) = {
            let entity = self.world.entities.get(npc_id)?;
            let include = match entity {
                Entity::Civilian(_) => true,
                Entity::Soldier(s) => s
                    .soldier
                    .cached_camp
                    .is_hostile_to(crate::element_kinds::Camp::Royalists),
                _ => false,
            };
            if !include {
                return None;
            }

            // Do not pre-filter inactive or unconscious NPCs. Original runs
            // GetHearVolume for every registered civilian/Lacklandist and
            // leaves refusal to StartThink, after the deafness read.
            let elem = entity.element_data();
            (elem.position_map(), elem.position())
        };

        let source_elev = noise.elevation as f32;
        let modified_volume = noise.volume as f32 * HEARING_FACTOR;
        // Original `GetHearVolume` subtracts the source point from the
        // listener's authoritative `GetPosition()` result. Do not rebuild Y
        // from `position_map + elevation`: a 3D-authored position projected
        // into map space can reconstruct one bit away, which is observable
        // when the positive remainder truncates to `UWORD` at volume 1.
        let dx = npc_world.x - noise.origin.x;
        let dy_world = npc_world.y - noise.origin.y - source_elev;
        let dz = npc_world.z - source_elev;

        // Original compares the full 3D points before range and deafness
        // work. A wounded or trapped source therefore cannot hear its own
        // AAARGH/HEEELP broadcast.
        if dx == 0.0 && dy_world == 0.0 && dz == 0.0 {
            return None;
        }

        let dy_stretched = dy_world * crate::position_interface::INVERSE_ASPECT_RATIO;
        if dx.abs().max(dy_stretched.abs()).max(dz.abs()) > modified_volume {
            return None;
        }

        let distance = (dx * dx + dy_stretched * dy_stretched + dz * dz).sqrt();
        // GetHearVolume returns before GetDeafness when the Euclidean
        // remainder is non-positive, even if the earlier max-norm range test
        // admitted the source.
        if modified_volume - distance <= 0.0 {
            return None;
        }

        let cover_volume = self
            .feedback
            .sound_sim
            .sources
            .max_noise_covering_volume_for_3d(npc_pos.x, npc_pos.y, npc_world.z);
        let frame = self.control.frame_counter;
        let deafness = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::ai_actor_data_mut)
            .unwrap_or_else(|| {
                panic!(
                    "one-shot noise listener {} lost its required AI actor state",
                    npc_id.index()
                )
            })
            .get_deafness(frame, cover_volume);

        let subjective = subjective_hear_volume(modified_volume, distance, deafness);
        (subjective != 0).then_some(crate::ai::Noise {
            volume: subjective,
            ..noise
        })
    }

    fn display_one_shot_noise(&mut self, noise: crate::ai::Noise) {
        // Original AddNoiseToDisplay runs only after every listener's Think.
        self.feedback
            .pending_side_effects
            .displayed_noises
            .push(noise);
    }

    /// Broadcast a one-shot noise and synchronously run each listener's new
    /// `EVENT_HEAR`, in Original NPC registration order.
    ///
    /// Original `RHElementActorNPC::Noise` calls `Think` inside the broadcast
    /// loop. Script natives and other in-frame callbacks therefore observe
    /// the listeners' RNG draws, state transitions, and launched sequences
    /// before returning.
    pub(in crate::engine) fn broadcast_noise_synchronously(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        noise_type: crate::ai::NoiseType,
        origin: crate::coordinates::MapPoint,
        origin_layer: Option<crate::position_interface::Layer>,
        volume: u16,
        elevation: u16,
        source_entity: Option<EntityId>,
    ) {
        use crate::ai::{Stimulus, StimulusType};

        let noise = self.one_shot_noise(
            noise_type,
            origin,
            origin_layer,
            volume,
            elevation,
            source_entity,
        );

        for npc_id in self.one_shot_noise_listener_ids() {
            let Some(subjective_noise) = self.subjective_one_shot_noise_for(npc_id, noise) else {
                continue;
            };
            let stimulus = Stimulus::with_noise(StimulusType::EventHear, subjective_noise);

            // Listener N sees every mutation produced by listener N-1. Only
            // the new EVENT_HEAR is dispatched: Original calls Think directly
            // and does not consume unrelated deferred stimuli here.
            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let ctx = {
                let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
                    panic!(
                        "one-shot noise listener {} disappeared before synchronous Think",
                        npc_id.index()
                    )
                });
                let building_sector = self.entity_building_sector(entity.element_data().sector());
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
            let tick_data = self.build_npc_tick_data(sim, npc_id, &scratch, assets);
            self.dispatch_think_with_drain_mode(
                sim, npc_id, &stimulus, &ctx, &tick_data, assets, false, false,
            );
        }
        self.display_one_shot_noise(noise);
    }

    // ── Cross-NPC action processing (phalanx coordination) ──────────
    //
    // After all AI think() calls, drain each NPC's pending cross-NPC
    // actions and apply them to the target NPCs. This covers:
    // - InstructGatherPosition + CALL_INSTRUCTION delivery
    // - BreakPhalanx propagation
    // - SendStimulus (e.g. CALL_COORDINATE to archers)
    // - SetLeft/RightCombatNeighbour for phalanx linking

    fn apply_update_left_combat_neighbour(
        &mut self,
        target: u32,
        old_left: Option<crate::ai::AiEntityHandle>,
        new_left: Option<crate::ai::AiEntityHandle>,
    ) {
        if let Some(old_left) = old_left {
            self.required_cross_npc_enemy_mut(old_left.get(), "unlink-old-left-neighbour")
                .right_combat_neighbour = None;
        }
        self.required_cross_npc_enemy_mut(target, "update-left-combat-neighbour")
            .left_combat_neighbour = new_left;
        if let Some(new_left) = new_left {
            let new_lefts_old_right = self
                .required_cross_npc_enemy_mut(new_left.get(), "inspect-new-left-neighbour")
                .right_combat_neighbour;
            if let Some(new_lefts_old_right) = new_lefts_old_right {
                self.required_cross_npc_enemy_mut(
                    new_lefts_old_right.get(),
                    "unlink-new-left-old-right-neighbour",
                )
                .left_combat_neighbour = None;
            }
            self.required_cross_npc_enemy_mut(new_left.get(), "link-new-left-neighbour")
                .right_combat_neighbour = Some(crate::ai::AiEntityHandle::new(target));
        }
    }

    fn apply_update_right_combat_neighbour(
        &mut self,
        target: u32,
        old_right: Option<crate::ai::AiEntityHandle>,
        new_right: Option<crate::ai::AiEntityHandle>,
    ) {
        if let Some(old_right) = old_right {
            self.required_cross_npc_enemy_mut(old_right.get(), "unlink-old-right-neighbour")
                .left_combat_neighbour = None;
        }
        self.required_cross_npc_enemy_mut(target, "update-right-combat-neighbour")
            .right_combat_neighbour = new_right;
        if let Some(new_right) = new_right {
            let new_rights_old_left = self
                .required_cross_npc_enemy_mut(new_right.get(), "inspect-new-right-neighbour")
                .left_combat_neighbour;
            if let Some(new_rights_old_left) = new_rights_old_left {
                self.required_cross_npc_enemy_mut(
                    new_rights_old_left.get(),
                    "unlink-new-right-old-left-neighbour",
                )
                .right_combat_neighbour = None;
            }
            self.required_cross_npc_enemy_mut(new_right.get(), "link-new-right-neighbour")
                .left_combat_neighbour = Some(crate::ai::AiEntityHandle::new(target));
        }
    }

    fn register_synchronizing_actor(&mut self, target: u32, actor: u32) {
        let target_id = self.expect_human_id_for_ai_handle(target, "register-synchronizing-actor");
        let entity = self
            .world
            .entities
            .get_mut(target_id)
            .expect("validated synchronization target vanished");
        let ai = entity.ai_controller_mut().unwrap_or_else(|| {
            panic!("synchronization target human {target} has no AI controller")
        });
        // RHArtificialIntelligence::RegisterSynchronizingActor is a direct,
        // unconditional InsertLast. In particular, the target can reach its
        // waypoint in a later element Hourglass slot in this same frame and
        // must observe this registration before dispatching EVENT_SYNC_CHARLY.
        ai.synchronizing_actors.push(actor);
    }

    pub(in crate::engine) fn process_pending_cross_npc_actions(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        // Close any direct Think calls left by a global owner-work/self-
        // stimulus fixed point before collecting genuinely deferred actions.
        // Iterate live owner slots in their stable order (PA-013).
        let ai_owner_ids: Vec<_> = self.world.entities.ai_owner_ids().collect();
        for owner_id in ai_owner_ids {
            self.process_synchronous_reentrant_actions_for(sim, owner_id, assets);
        }
        // Collect all pending actions first to avoid borrow issues.
        // Both enemy (soldier) and friendly (civilian) AIs can push
        // cross-NPC actions — e.g. civilians send `CALL_ALERT` /
        // `CALL_REPORT` to soldiers via `AiController` on their base.
        let mut all_actions: Vec<crate::ai::CrossNpcAction> = Vec::new();
        let ai_owner_ids: Vec<_> = self.world.entities.ai_owner_ids().collect();
        for owner_id in ai_owner_ids {
            if let Some(ai) = self
                .world
                .entities
                .get_mut(owner_id)
                .and_then(Entity::ai_controller_mut)
            {
                all_actions.extend(ai.take_pending_cross_npc_actions());
            }
        }

        if all_actions.is_empty() {
            // No cross-NPC actions to process, but still deliver any
            // self-stimuli queued last tick (EventDone from
            // `SendCondolationCard`, MYTALK callbacks, etc.).  This
            // drain used to live at the tail of this function, which
            // meant it was skipped entirely on ticks with no cross-NPC
            // actions — the common case — stranding queued stimuli
            // forever and hanging states like
            // `DefaultOnPostLookingSidewards` that wait on `EventDone`
            // to exit.
            self.drain_pending_self_stimuli(sim, assets);
            return;
        }

        // Building the full AI view resolves prepared building-exit forecasts
        // and therefore consumes authoritative RNG.  The original only builds
        // target context while actually delivering a cross-NPC action, so do
        // not speculate before the empty fast path above.
        let scratch = self.build_sim_scratch(sim, assets);
        let frame = self.control.frame_counter;

        for action in all_actions {
            match action {
                crate::ai::CrossNpcAction::RequestAlert { caller, target, .. } => {
                    panic!(
                        "result-bearing CALL_ALERT {caller}->{target} escaped its owner boundary"
                    )
                }
                crate::ai::CrossNpcAction::RequestThinkResult { caller, target, .. } => {
                    panic!(
                        "result-bearing Think request {caller}->{target} escaped its owner boundary"
                    )
                }
                crate::ai::CrossNpcAction::RequestPatrolDispatch { caller, chief, .. } => {
                    panic!(
                        "result-bearing patrol dispatch {caller}->{chief} escaped its owner boundary"
                    )
                }
                crate::ai::CrossNpcAction::FinalizeAlertSoldiers { caller, .. } => {
                    panic!(
                        "AlertSoldiers finalization for caller {caller} escaped its owner boundary"
                    )
                }
                crate::ai::CrossNpcAction::ResumeTowerGuardBattleDecisions { caller } => {
                    panic!(
                        "tower-guard battle continuation for caller {caller} escaped its owner boundary"
                    )
                }
                crate::ai::CrossNpcAction::ResumeAfterLookThere { caller, .. } => {
                    panic!(
                        "HeyFolksLookThere resume for caller {caller} escaped its owner boundary"
                    )
                }
                crate::ai::CrossNpcAction::BroadcastLookThere { caller, .. } => {
                    panic!(
                        "HeyFolksLookThere broadcast for caller {caller} escaped its owner boundary"
                    )
                }
                crate::ai::CrossNpcAction::RelayStimulusToPatrolMembers {
                    stimulus_type,
                    members,
                    ..
                } => {
                    panic!(
                        "whole-patrol {stimulus_type:?} broadcast to {} members escaped its owner boundary",
                        members.len()
                    )
                }
                crate::ai::CrossNpcAction::InstructGatherPosition {
                    target,
                    position,
                    direction,
                    call_instruction,
                    ..
                } => {
                    let target_id = self.expect_human_id_for_ai_handle(
                        target,
                        "deferred gather-instruction target",
                    );
                    tracing::trace!(
                        target: "robin_engine::ai_enemy::phalanx",
                        instructed = target,
                        frame = self.control.frame_counter,
                        ?position,
                        direction,
                        call_instruction,
                        "InstructGatherPosition"
                    );
                    if call_instruction && !self.soldier_stands_in_phalanx(target_id) {
                        continue;
                    }
                    let ctx = {
                        let entity = self
                            .world
                            .entities
                            .get_mut(target_id)
                            .expect("validated gather-instruction target vanished");
                        let ctx = build_ai_context_from_entity(
                            entity,
                            frame,
                            None,
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
                        let enemy_ai = entity.enemy_ai_mut().unwrap_or_else(|| {
                            panic!(
                                "deferred gather-instruction target human {target} has no EnemyAi"
                            )
                        });
                        enemy_ai.gather_position = position;
                        enemy_ai.gather_direction = direction;
                        enemy_ai.gather_position_instructed = true;
                        ctx
                    };
                    if !call_instruction {
                        continue;
                    }
                    // CrossNpcAction::InstructGatherPosition: target
                    // is an enemy soldier.  Build rich tick data so a
                    // subsequent think()-triggered BattleDecisions
                    // sees the target snapshot.
                    let tick_data = self.build_npc_tick_data(sim, target_id, &scratch, assets);
                    let stimulus = crate::ai::Stimulus::new(StimulusType::CallInstruction);
                    self.dispatch_filtered_stimulus(
                        sim, assets, target_id, &stimulus, &ctx, &tick_data,
                    );
                }

                crate::ai::CrossNpcAction::BreakPhalanx { target, .. } => {
                    panic!(
                        "synchronous break-phalanx target {target} escaped its source-owner boundary"
                    )
                }

                crate::ai::CrossNpcAction::SendStimulus {
                    target,
                    stimulus_type,
                    info,
                    fallback_to_sender,
                    to_whole_patrol,
                } => {
                    let target_id = self.entity_id_for_index(target);
                    let mut stimulus = crate::ai::Stimulus::new(stimulus_type);
                    stimulus.info = info;
                    stimulus.to_whole_patrol = to_whole_patrol;

                    let ctx = {
                        let Some(entity) = target_id
                            .and_then(|target_id| self.world.entities.get(target_id))
                            .filter(|entity| entity.ai_controller().is_some())
                        else {
                            // Target missing → try fallback directly below.
                            if let Some(sender) = fallback_to_sender
                                && let Some((sender_id, entity)) = self
                                    .entity_id_for_index(sender)
                                    .and_then(|sender_id| {
                                        self.world
                                            .entities
                                            .get(sender_id)
                                            .map(|entity| (sender_id, entity))
                                    })
                                    .filter(|(_, entity)| entity.ai_controller().is_some())
                            {
                                let ctx = build_ai_context_from_entity(
                                    entity,
                                    frame,
                                    None,
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
                                let fallback_tick =
                                    self.build_npc_tick_data(sim, sender_id, &scratch, assets);
                                self.dispatch_filtered_stimulus(
                                    sim,
                                    assets,
                                    sender_id,
                                    &stimulus,
                                    &ctx,
                                    &fallback_tick,
                                );
                            }
                            continue;
                        };
                        build_ai_context_from_entity(
                            entity,
                            frame,
                            None,
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
                    let target_id = self.expect_human_id_for_ai_handle(
                        target,
                        "validated deferred stimulus target",
                    );
                    // SendStimulus → enemy soldier target: the
                    // stimulus may be EVENT_VIEW / EVENT_REPORT /
                    // alert-forwarding which feeds BattleDecisions.
                    let tick_data = self.build_npc_tick_data(sim, target_id, &scratch, assets);
                    let handled = self.dispatch_filtered_stimulus(
                        sim, assets, target_id, &stimulus, &ctx, &tick_data,
                    );
                    // Fallback: if target couldn't handle the stimulus,
                    // redeliver to the sender (e.g. conversation chains).
                    if !handled && let Some(sender) = fallback_to_sender {
                        let Some(sender_id) = self.entity_id_for_index(sender) else {
                            continue;
                        };
                        let ctx2 = {
                            let Some(entity) = self
                                .world
                                .entities
                                .get(sender_id)
                                .filter(|entity| entity.ai_controller().is_some())
                            else {
                                continue;
                            };
                            build_ai_context_from_entity(
                                entity,
                                frame,
                                None,
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
                        let fallback_tick =
                            self.build_npc_tick_data(sim, sender_id, &scratch, assets);
                        self.dispatch_filtered_stimulus(
                            sim,
                            assets,
                            sender_id,
                            &stimulus,
                            &ctx2,
                            &fallback_tick,
                        );
                    }
                }

                crate::ai::CrossNpcAction::SetLeftCombatNeighbour { target, neighbour } => {
                    self.required_cross_npc_enemy_mut(target, "set-left-combat-neighbour")
                        .left_combat_neighbour = neighbour;
                }

                crate::ai::CrossNpcAction::SetRightCombatNeighbour { target, neighbour } => {
                    self.required_cross_npc_enemy_mut(target, "set-right-combat-neighbour")
                        .right_combat_neighbour = neighbour;
                }

                crate::ai::CrossNpcAction::SetArcherBehindMe { target, archer } => {
                    self.required_cross_npc_enemy_mut(target, "set-archer-behind")
                        .archer_behind_me = archer;
                }

                crate::ai::CrossNpcAction::SetShieldBearerBeforeMe {
                    target,
                    shield_bearer,
                } => {
                    self.required_cross_npc_enemy_mut(target, "set-shield-bearer")
                        .shield_bearer_before_me = shield_bearer;
                }

                // Full reciprocal update.  Four steps:
                //   1. clear old_left's right pointer
                //   2. store new_left on target's left pointer (caller
                //      may also have written it eagerly for immediate
                //      visibility)
                //   3. pre-clean new_left's existing right (recursive
                //      `update_right_combat_neighbour(NULL)`) — clear
                //      that-right's left pointer
                //   4. wire new_left's right back to target
                crate::ai::CrossNpcAction::UpdateLeftCombatNeighbour {
                    target,
                    old_left,
                    new_left,
                } => self.apply_update_left_combat_neighbour(target, old_left, new_left),

                // Same shape as `update_left_combat_neighbour`, for
                // the right side.
                crate::ai::CrossNpcAction::UpdateRightCombatNeighbour {
                    target,
                    old_right,
                    new_right,
                } => self.apply_update_right_combat_neighbour(target, old_right, new_right),

                crate::ai::CrossNpcAction::SetPrimaryTarget {
                    target,
                    primary_target,
                } => {
                    self.required_cross_npc_enemy_mut(target, "set-primary-target")
                        .base
                        .primary_target = primary_target;
                }

                crate::ai::CrossNpcAction::SetPhalanxThemList {
                    target,
                    them,
                    primary_target,
                } => self.process_synchronous_set_phalanx_them_list(target, them, primary_target),

                crate::ai::CrossNpcAction::Say { target, remark } => {
                    let target_id =
                        self.expect_human_id_for_ai_handle(target, "cross-NPC speech target");
                    self.required_cross_npc_enemy_mut(target, "cross-NPC speech target")
                        .base
                        .say(remark);
                    self.drain_ai_owner_work_for(sim, assets, target_id);
                }

                crate::ai::CrossNpcAction::SetLootedAfterMoneyFight { target, looted } => {
                    self.required_cross_npc_enemy_mut(target, "set-money-fight-looted")
                        .base
                        .looted_after_money_fight = looted;
                }

                // Legacy serialized pending work. Production no longer emits
                // this incomplete shape, but retaining the arm preserves old
                // save/checkpoint compatibility and later enum ordinals.
                crate::ai::CrossNpcAction::UpdateReport {
                    target,
                    report_type,
                    seek_position,
                } => {
                    self.required_cross_npc_enemy_mut(target, "update-report")
                        .base
                        .my_reconnaissance_report
                        .update(report_type, seek_position);
                }

                crate::ai::CrossNpcAction::ConsiderReport {
                    target,
                    report,
                    flags,
                } => {
                    let frame = self.control.frame_counter;
                    // Use the AiController-level helper: it merges
                    // the report AND queues the per-body
                    // `delete_detectable(body, DETECTABLE_BODY)`
                    // side effects.  The bare
                    // `ReconnaissanceReport::consider_report`
                    // skipped those side effects, leaving stale
                    // body detectables on the NPC after a peer
                    // report merge.
                    self.required_cross_npc_enemy_mut(target, "consider-report")
                        .base
                        .consider_report_merged_at_frame(
                            &report,
                            flags,
                            scratch.ai_entity_views.as_ref(),
                            frame,
                        );
                }

                crate::ai::CrossNpcAction::RegisterSynchronizingActor { target, actor } => {
                    self.register_synchronizing_actor(target, actor);
                }
                crate::ai::CrossNpcAction::ReportBackToOfficer { .. } => {
                    panic!("synchronous officer report leaked into deferred cross-NPC actions")
                }
            }
        }

        self.drain_pending_self_stimuli(sim, assets);
    }

    /// Dispatch `stimulus` to `npc_id` via
    /// [`Self::dispatch_filtered_stimulus`], then run a synchronous
    /// side-effect drain pass so handler side effects (LaunchSequence,
    /// SetAttentiveMode, Face, quit/enter swordfight, look-sidewards,
    /// …) and any condolations / re-entrant `Think(EVENT_DONE)` they
    /// trigger happen inside the same call stack as the outer Think —
    /// matching the original `think()`, where handlers invoke
    /// `launch_sequence`, `halt`, `face`, `set_attentive_mode` inline
    /// and `send_condolation_card` fires `think(EVENT_DONE)`
    /// re-entrantly.
    ///
    /// The loop re-runs the drain while the NPC keeps generating new
    /// pending side effects (e.g. one condolation's `EventDone` handler
    /// queues another sequence that is preempted in the next iteration),
    /// bounded at 8 iterations to guard against a pathological cascade.
    ///
    /// Returns `dispatch_filtered_stimulus`'s handled bool — unchanged
    /// by the drain pass.
    pub(in crate::engine) fn dispatch_think_with_drain(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        stimulus: &crate::ai::Stimulus,
        ctx: &crate::ai::AiContext,
        tick_data: &crate::ai::AiPerTickData,
        assets: &LevelAssets,
    ) -> bool {
        self.dispatch_think_with_drain_mode(
            sim, npc_id, stimulus, ctx, tick_data, assets, false, true,
        )
    }

    pub(in crate::engine) fn dispatch_think_with_drain_without_forecast(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        stimulus: &crate::ai::Stimulus,
        ctx: &crate::ai::AiContext,
        tick_data: &crate::ai::AiPerTickData,
        assets: &LevelAssets,
    ) -> bool {
        self.dispatch_think_with_drain_mode(
            sim, npc_id, stimulus, ctx, tick_data, assets, true, false,
        )
    }

    /// Owner-local Think before the current frame's SequenceManager hourglass.
    /// Keep standalone Turns registered but uninstructed just like the
    /// detection FIFO that originally produced a retained stimulus.
    pub(in crate::engine) fn dispatch_think_with_drain_without_forecast_deferred_turn(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        stimulus: &crate::ai::Stimulus,
        ctx: &crate::ai::AiContext,
        tick_data: &crate::ai::AiPerTickData,
        assets: &LevelAssets,
    ) -> bool {
        self.dispatch_think_with_drain_mode(
            sim, npc_id, stimulus, ctx, tick_data, assets, true, true,
        )
    }

    pub(super) fn dispatch_think_with_drain_mode(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        stimulus: &crate::ai::Stimulus,
        ctx: &crate::ai::AiContext,
        tick_data: &crate::ai::AiPerTickData,
        assets: &LevelAssets,
        owner_local_no_forecast: bool,
        defer_turn_instruction: bool,
    ) -> bool {
        // Original's ComputeViewRadius memo lives on the target surface, so a
        // synchronous IsDetecting inside Think must see a RefreshDetection
        // result produced earlier in this universal frame. Keep AiContext's
        // immutable-handler facade bounded exactly by the synchronous Think
        // call, then commit any newly computed surfaces before later callbacks.
        ctx.seed_view_radius_cache(&self.ai.view_radius_cache);
        let had_ai_at_entry = self
            .world
            .entities
            .get(npc_id)
            .and_then(Entity::ai_controller)
            .is_some();
        let handled = self.dispatch_filtered_stimulus_with_owner_mode(
            sim,
            assets,
            npc_id,
            stimulus,
            ctx,
            Some(tick_data),
            owner_local_no_forecast,
            defer_turn_instruction,
        );
        ctx.commit_view_radius_cache(&mut self.ai.view_radius_cache);

        // PCs can participate in direct swordfights but have no NPC AI
        // controller or AI-owned recovery effects to drain.
        if !had_ai_at_entry && matches!(self.world.entities.get(npc_id), Some(Entity::Pc(_))) {
            return handled;
        }

        // StartThink applies SetViewStatus synchronously for
        // LOSE_CONSCIOUSNESS, WASP, and NET. FITAGAIN can publish its
        // resurrection work at this same boundary. The typed AI records
        // those engine-owned writes while its controller is borrowed; commit
        // them immediately after Think returns, before waypoint callbacks or
        // any other pending/re-entrant work can observe stale NPC state.
        self.tick_ai_pending_resurrection_and_eyes_for_npc(npc_id);

        // `RHArtificialIntelligence::ExecuteWaypointScript` invokes the
        // waypoint VM directly from the active Think handler. Close that
        // authored callback before the generic post-Think effect drain:
        // script natives such as AssignPath recursively enter
        // EVENT_RETURN_TO_DUTY before later orders or condolations from the
        // outer handler can settle.
        self.dispatch_pending_waypoint_script_for_owner(sim, npc_id, assets);

        // EventViewStandardProcedure explicitly marks an accepted VIEW after
        // all StartThink and handler guards. Mirror that one-shot onto the
        // engine-owned AI actor record before draining its other synchronous
        // effects. Locked, frozen, script-filtered, and handler-rejected VIEWs
        // never set the flag.
        let mark_alerted = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap_or_else(|| {
                panic!(
                    "handled Think recipient {} lost its entity or AI controller before drain",
                    npc_id.index()
                )
            });
        let mark_alerted = std::mem::take(&mut mark_alerted.outbox.detection.mark_alerted);
        if mark_alerted {
            let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
                panic!(
                    "accepted EVENT_VIEW recipient {} disappeared after its synchronous Think",
                    npc_id.index()
                )
            });
            let ai_actor = entity.ai_actor_data_mut().unwrap_or_else(|| {
                panic!(
                    "accepted EVENT_VIEW recipient {} lost its AI actor data after synchronous Think",
                    npc_id.index()
                )
            });
            ai_actor.alerted = true;
        }

        const MAX_ITERS: u32 = 8;
        for iter in 0..MAX_ITERS {
            // Drain the per-NPC pending-flags pass (launches sequences,
            // commands, turn orders, attentive-mode transitions, etc.).
            self.drain_pending_for_npc_mode(
                sim,
                npc_id,
                assets,
                owner_local_no_forecast,
                defer_turn_instruction,
            );
            // `drain_pending_for_npc` launches the first order barrier in its
            // original position. Close the boundary again because later
            // effect application and civilian handlers share the same base
            // order outbox. Owner-local SetState notifications are also part
            // of this fixed point, so late script-seek callbacks cannot leak
            // into a global batch or strand in the outbox.
            self.launch_pending_orders_for_npc_mode(sim, assets, npc_id, defer_turn_instruction);
            let _ = self.drain_pending_move_requests_for_owner(sim, npc_id);
            self.surface_synchronous_completion_events_for_owner(npc_id);

            self.process_synchronous_reentrant_actions_for_mode(
                sim,
                npc_id,
                assets,
                defer_turn_instruction,
            );

            // Any condolations the drain above queued (sequences that
            // got preempted by the side effects) fire here — which may
            // push EventDone / EventImpossible into pending_self_stimuli.
            self.dispatch_condolations_for_npc(sim, npc_id, assets);

            // Re-enter Think for each self-stimulus (EventDone, MYTALK,
            // etc.).  This may queue more pending flags — loop again.
            let has_self_stimuli = {
                let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
                    panic!(
                        "handled Think recipient {} disappeared before self-stimulus recheck",
                        npc_id.index()
                    )
                });
                let ai = entity.ai_controller().unwrap_or_else(|| {
                    panic!(
                        "handled Think recipient {} lost its AI controller before self-stimulus recheck",
                        npc_id.index()
                    )
                });
                !ai.outbox.reentrant.self_stimuli.is_empty()
            };
            if has_self_stimuli {
                if owner_local_no_forecast {
                    self.drain_self_stimuli_for_npc_without_forecast(sim, npc_id, assets);
                } else {
                    self.drain_self_stimuli_for_npc(sim, npc_id, assets);
                }
            }

            // A re-entrant self stimulus can itself call another NPC. Close
            // those direct C++ call boundaries before deciding this owner has
            // stabilised; otherwise the result-bearing request can escape to
            // the global cross-action batch.
            self.process_synchronous_reentrant_actions_for_mode(
                sim,
                npc_id,
                assets,
                defer_turn_instruction,
            );

            let still_pending = {
                let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
                    panic!(
                        "handled Think recipient {} disappeared before fixed-point recheck",
                        npc_id.index()
                    )
                });
                let ai = entity.ai_controller().unwrap_or_else(|| {
                    panic!(
                        "handled Think recipient {} lost its AI controller before fixed-point recheck",
                        npc_id.index()
                    )
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
                "Think-drain NPC {} did not stabilise after {MAX_ITERS} passes",
                npc_id.index()
            );
        }

        handled
    }

    /// Deliver completion latches produced after the typed `EndThink` at the
    /// same logical boundary as the operation that produced them.
    ///
    /// Original `AppendMoveToSequence` performs path construction inline, so
    /// failures and already-at-destination results are visible to the
    /// enclosing `EndThink`. Rust may discover either result only after the
    /// controller borrow is released: path construction is engine-owned, and
    /// owner-work continuations such as patrol initialization resume outside
    /// the typed Think call. Surface all three `EndThink` latches before a
    /// sibling synchronous event can enter `StartThink` and clear them.
    pub(in crate::engine) fn surface_synchronous_completion_events_for_owner(
        &mut self,
        npc_id: EntityId,
    ) {
        let debug_decision_path = crate::ai_enemy::decision_path_debug_enabled()
            && crate::ai_enemy::decision_path_debug_matches_raw(
                self.control.frame_counter,
                npc_id.index(),
            );
        // A movement element's Impossible transition already owns a pending
        // synchronous SendCondolationCard callback. Original delivers that
        // callback directly from SetState before any enclosing stack can
        // return to EndThink. Rust's suspended card must therefore win this
        // boundary; surfacing the same latch first mislabels a genuine
        // movement condolation as an engine completion.
        let pending_couldnt_condolation = self
            .orders
            .sequence_manager
            .has_pending_couldnt_reachpoint_condolation(npc_id);
        // A preflight rejection can occur before Rust has materialized the
        // replacement movement element and its Impossible condolence card.
        // If the actor still has an authored movement selected, Original's
        // replacement arbitration reaches the movement SetState callback and
        // reports the rejection as EVENT_COULDNT_REACHPOINT from
        // SendCondolationCard. A nonmovement selection (notably the attentive
        // transition in the lift-entry continuation) instead belongs to the
        // engine-completion bridge.
        let selected_movement_owns_failure = self
            .orders
            .sequence_manager
            .actor_has_selected_movement(npc_id);
        let ai = self
            .world
            .entities
            .get_mut(npc_id)
            .unwrap_or_else(|| {
                panic!(
                    "synchronous move owner {} disappeared before path-result delivery",
                    npc_id.index()
                )
            })
            .ai_controller_mut()
            .unwrap_or_else(|| {
                panic!(
                    "synchronous move owner {} lost AI before path-result delivery",
                    npc_id.index()
                )
            });
        // Only an EndThink delivers these latches, so one whose operation ran
        // outside a Think is discarded exactly as the next Think entry would.
        // Dispatching a completion also re-enters Think, whose entry gate
        // clears all three latches before the nested handler runs, so a single
        // boundary surfaces at most one event even when several were set.
        let typed_tail_pending = ai.has_typed_completion_pending();
        let retain_couldnt_reachpoint = ai.completion_latch_inside_think
            && ai.couldnt_reachpoint
            && (typed_tail_pending || pending_couldnt_condolation);
        // Owner-work prefixes (notably Enemy SetState's synchronous callback)
        // can ask for a completion surface while the caller-tail GoTo is still
        // queued on the controller. Original cannot close that EndThink yet:
        // AppendMoveToSequence runs before control returns to EndThink, and its
        // path verdict belongs to the same open frame. Keep the deferred frame
        // alive until the engine has actually drained the queued intent.
        let engine_verdict_pending =
            ai.engine_deferred_end_think_frames != 0 && !ai.engine_completion_verdict_resolved;
        if debug_decision_path {
            eprintln!(
                "AIDECISION frame={} owner={} stage=surface_completion_enter inside_think={} couldnt={} already_on_point={} already_turned={} typed_tail_pending={} retain_couldnt={} engine_verdict_pending={} owner_work={:?}",
                self.control.frame_counter,
                npc_id.index(),
                ai.completion_latch_inside_think,
                ai.couldnt_reachpoint,
                ai.already_on_point,
                ai.already_turned,
                typed_tail_pending,
                retain_couldnt_reachpoint,
                engine_verdict_pending,
                ai.outbox.reentrant.owner_work,
            );
        }
        let event = if !ai.completion_latch_inside_think {
            None
        } else if retain_couldnt_reachpoint {
            None
        } else if ai.couldnt_reachpoint {
            Some(crate::ai::StimulusType::EventCouldntReachPoint)
        } else if ai.already_on_point {
            Some(crate::ai::StimulusType::EventReachPoint)
        } else if ai.already_turned {
            Some(crate::ai::StimulusType::EventDone)
        } else {
            None
        };
        if !retain_couldnt_reachpoint {
            ai.couldnt_reachpoint = false;
        }
        ai.already_on_point = false;
        ai.already_turned = false;
        if let Some(event) = event {
            ai.engine_completion_verdict_resolved = false;
            let origin = if selected_movement_owns_failure
                && event == crate::ai::StimulusType::EventCouldntReachPoint
            {
                crate::ai::SelfStimulusOrigin::Condolation
            } else {
                crate::ai::SelfStimulusOrigin::EngineCompletion
            };
            ai.outbox
                .reentrant
                .self_stimuli
                .push(crate::ai::QueuedSelfStimulus::new(event, origin));
        } else if !retain_couldnt_reachpoint && !typed_tail_pending && !engine_verdict_pending {
            // A successful engine-side authorization produces no recursive
            // completion event. This is the point where Original returns
            // through every EndThink frame that was kept live while Rust
            // released the AI borrow to build the path.
            ai.close_engine_deferred_end_think_frames();
        }
        if debug_decision_path {
            eprintln!(
                "AIDECISION frame={} owner={} stage=surface_completion_result event={event:?} couldnt={} already_on_point={} already_turned={} self_stimuli={:?} owner_work={:?}",
                self.control.frame_counter,
                npc_id.index(),
                ai.couldnt_reachpoint,
                ai.already_on_point,
                ai.already_turned,
                ai.outbox.reentrant.self_stimuli,
                ai.outbox.reentrant.owner_work,
            );
        }
    }

    pub(in crate::engine) fn process_synchronous_reentrant_actions_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) {
        self.process_synchronous_reentrant_actions_for_mode(sim, source_id, assets, false);
    }

    pub(super) fn process_synchronous_reentrant_actions_for_mode(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        assets: &LevelAssets,
        defer_turn_instruction: bool,
    ) {
        loop {
            let actions = self
                .world
                .entities
                .get_mut(source_id)
                .and_then(Entity::ai_controller_mut)
                .map(crate::ai::AiController::take_pending_synchronous_cross_npc_actions)
                .unwrap_or_else(|| {
                    panic!(
                        "synchronous action source {} has no AI controller",
                        source_id.index()
                    )
                });
            if actions.is_empty() {
                break;
            }

            let deferred = {
                let ai = self
                    .world
                    .entities
                    .get_mut(source_id)
                    .and_then(Entity::ai_controller_mut)
                    .unwrap_or_else(|| {
                        panic!(
                            "synchronous action source {} lost its AI controller",
                            source_id.index()
                        )
                    });
                std::mem::take(&mut ai.outbox.reentrant.cross_npc_actions)
            };
            let mut deferred = deferred;
            let mut alert_formation_targets = Vec::new();
            for action in actions {
                if let crate::ai::CrossNpcAction::RequestThinkResult {
                    target,
                    continuation:
                        crate::ai::ThinkResultContinuation::OfficerAlertedSoldier { .. }
                        | crate::ai::ThinkResultContinuation::OfficerCombatAlertedSoldier { .. },
                    ..
                } = &action
                {
                    alert_formation_targets.push(*target);
                }
                match action {
                    crate::ai::CrossNpcAction::InstructGatherPosition {
                        target,
                        position,
                        direction,
                        call_instruction,
                    } => {
                        // Alert formations queue their result requests before
                        // the sibling gather instructions. Remember those
                        // exact targets while draining this saved batch, then
                        // suppress an instruction if its direct Think result
                        // pruned it. Phalanx instructions have no preceding
                        // alert-result request and remain unconditional.
                        let alert_formation = alert_formation_targets.contains(&target);
                        let still_alerted = !alert_formation
                            || self
                                .world
                                .entities
                                .get(source_id)
                                .and_then(Entity::enemy_ai)
                                .unwrap_or_else(|| {
                                    panic!(
                                        "alert InstructGatherPosition source {source_id:?} is not an enemy soldier"
                                    )
                                })
                                .alerted_us
                                .contains(&target);
                        if still_alerted {
                            self.process_synchronous_gather_instruction(
                                sim,
                                target,
                                position,
                                direction,
                                call_instruction,
                                assets,
                            );
                        }
                    }
                    crate::ai::CrossNpcAction::BreakPhalanx {
                        target,
                        refresh_them_list,
                    } => {
                        // Original stores Think recursion depth in one static
                        // RHArtificialIntelligence byte, not on each NPC.
                        // A direct recursive BreakPhalanx call on a neighbour
                        // therefore observes the still-live source Think. The
                        // typed Rust handler has already returned through its
                        // controller-local EndThink before the engine can
                        // drain this action, so zero here still represents
                        // the one logical frame which emitted BreakPhalanx.
                        // A nonzero value represents an explicitly suspended
                        // outer frame and is already the global logical depth.
                        let logical_think_depth = self
                            .world
                            .entities
                            .get(source_id)
                            .and_then(Entity::ai_controller)
                            .unwrap_or_else(|| {
                                panic!(
                                    "break-phalanx source {} lost its AI controller",
                                    source_id.index()
                                )
                            })
                            .think_recursion_depth
                            .max(1);
                        self.process_synchronous_break_phalanx(
                            sim,
                            target,
                            refresh_them_list,
                            logical_think_depth,
                            target == source_id.index(),
                            assets,
                        )
                    }
                    crate::ai::CrossNpcAction::ConsiderReport {
                        target,
                        report,
                        flags,
                    } => {
                        self.process_synchronous_consider_report(sim, target, report, flags, assets)
                    }
                    crate::ai::CrossNpcAction::FinalizeAlertSoldiers {
                        caller,
                        use_formation,
                        failure,
                    } => self.process_synchronous_finalize_alert_soldiers(
                        sim,
                        source_id,
                        caller,
                        use_formation,
                        failure,
                        assets,
                    ),
                    crate::ai::CrossNpcAction::ResumeTowerGuardBattleDecisions { caller } => self
                        .process_synchronous_tower_guard_battle_decisions(
                            sim, source_id, caller, assets,
                        ),
                    crate::ai::CrossNpcAction::SetPhalanxThemList {
                        target,
                        them,
                        primary_target,
                    } => {
                        self.process_synchronous_set_phalanx_them_list(target, them, primary_target)
                    }
                    crate::ai::CrossNpcAction::ResumeAfterLookThere {
                        caller,
                        continuation,
                    } => self.process_synchronous_look_there_resume(
                        sim,
                        source_id,
                        caller,
                        continuation,
                        assets,
                    ),
                    crate::ai::CrossNpcAction::BroadcastLookThere {
                        caller,
                        position,
                        radius,
                        continuation,
                    } => self.process_synchronous_look_there_broadcast(
                        sim,
                        source_id,
                        caller,
                        position,
                        radius,
                        continuation,
                        assets,
                        defer_turn_instruction,
                    ),
                    crate::ai::CrossNpcAction::UpdateLeftCombatNeighbour {
                        target,
                        old_left,
                        new_left,
                    } => self.apply_update_left_combat_neighbour(target, old_left, new_left),
                    crate::ai::CrossNpcAction::UpdateRightCombatNeighbour {
                        target,
                        old_right,
                        new_right,
                    } => self.apply_update_right_combat_neighbour(target, old_right, new_right),
                    crate::ai::CrossNpcAction::SetLeftCombatNeighbour { target, neighbour } => {
                        self.required_cross_npc_enemy_mut(
                            target,
                            "synchronous left-neighbour setter",
                        )
                        .left_combat_neighbour = neighbour;
                    }
                    crate::ai::CrossNpcAction::SetRightCombatNeighbour { target, neighbour } => {
                        self.required_cross_npc_enemy_mut(
                            target,
                            "synchronous right-neighbour setter",
                        )
                        .right_combat_neighbour = neighbour;
                    }
                    crate::ai::CrossNpcAction::SetArcherBehindMe { target, archer } => {
                        self.required_cross_npc_enemy_mut(
                            target,
                            "synchronous archer-behind setter",
                        )
                        .archer_behind_me = archer;
                    }
                    crate::ai::CrossNpcAction::SetShieldBearerBeforeMe {
                        target,
                        shield_bearer,
                    } => {
                        self.required_cross_npc_enemy_mut(
                            target,
                            "synchronous shield-bearer setter",
                        )
                        .shield_bearer_before_me = shield_bearer;
                    }
                    crate::ai::CrossNpcAction::SetPrimaryTarget {
                        target,
                        primary_target,
                    } => {
                        // `PhalanxReinitializeThemList` assigns every member's
                        // `mpPrimaryTarget` inline while its recursion unwinds.
                        // Apply that direct setter before a later recursive
                        // `BreakPhalanx` lets the member choose its own target.
                        self.required_cross_npc_enemy_mut(
                            target,
                            "synchronous primary-target setter",
                        )
                        .base
                        .primary_target = primary_target;
                    }
                    crate::ai::CrossNpcAction::RegisterSynchronizingActor { target, actor } => {
                        self.register_synchronizing_actor(target, actor);
                    }
                    crate::ai::CrossNpcAction::SendStimulus { .. } => {
                        self.requeue_isolated_synchronous_action(source_id, action.clone());
                        self.process_synchronous_stimuli_for(
                            sim,
                            source_id,
                            assets,
                            defer_turn_instruction,
                        )
                    }
                    crate::ai::CrossNpcAction::RelayStimulusToPatrolMembers { .. } => {
                        self.requeue_isolated_synchronous_action(source_id, action.clone());
                        self.process_synchronous_patrol_member_relay_for(
                            sim,
                            source_id,
                            assets,
                            defer_turn_instruction,
                        )
                    }
                    crate::ai::CrossNpcAction::RequestPatrolDispatch { .. } => {
                        self.requeue_isolated_synchronous_action(source_id, action.clone());
                        self.process_synchronous_patrol_dispatch_requests_for(
                            sim,
                            source_id,
                            assets,
                            defer_turn_instruction,
                        )
                    }
                    crate::ai::CrossNpcAction::RequestAlert { .. } => {
                        self.requeue_isolated_synchronous_action(source_id, action.clone());
                        self.process_synchronous_alert_requests_for(sim, source_id, assets)
                    }
                    crate::ai::CrossNpcAction::RequestThinkResult { .. } => {
                        self.requeue_isolated_synchronous_action(source_id, action.clone());
                        self.process_synchronous_think_results_for(
                            sim,
                            source_id,
                            assets,
                            defer_turn_instruction,
                        )
                    }
                    crate::ai::CrossNpcAction::ReportBackToOfficer { .. } => {
                        self.requeue_isolated_synchronous_action(source_id, action.clone());
                        self.process_synchronous_officer_reports_for(sim, source_id, assets)
                    }
                    crate::ai::CrossNpcAction::Say { target, remark } => {
                        let target_id =
                            self.expect_human_id_for_ai_handle(target, "cross-NPC speech target");
                        self.required_cross_npc_enemy_mut(target, "cross-NPC speech target")
                            .base
                            .say(remark);
                        self.drain_ai_owner_work_for(sim, assets, target_id);
                    }
                    _ => unreachable!("ordered synchronous drain received deferred action"),
                }

                // Direct C++ calls are depth-first: if A emits C while B was
                // already queued, C closes before B. Isolate A's generated
                // work, recursively drain it, then continue the saved batch.
                self.process_synchronous_reentrant_actions_for_mode(
                    sim,
                    source_id,
                    assets,
                    defer_turn_instruction,
                );
                let ai = self
                    .world
                    .entities
                    .get_mut(source_id)
                    .and_then(Entity::ai_controller_mut)
                    .unwrap_or_else(|| {
                        panic!(
                            "synchronous action source {} lost its AI controller",
                            source_id.index()
                        )
                    });
                deferred.extend(std::mem::take(&mut ai.outbox.reentrant.cross_npc_actions));
            }
            let ai = self
                .world
                .entities
                .get_mut(source_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "synchronous action source {} lost its AI controller",
                        source_id.index()
                    )
                });
            ai.outbox.reentrant.cross_npc_actions = deferred;
        }
    }

    fn requeue_isolated_synchronous_action(
        &mut self,
        source_id: crate::element::EntityId,
        action: crate::ai::CrossNpcAction,
    ) {
        self.world
            .entities
            .get_mut(source_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap_or_else(|| {
                panic!(
                    "synchronous action source {} lost its AI controller",
                    source_id.index()
                )
            })
            .outbox
            .reentrant
            .cross_npc_actions
            .push(action);
    }

    /// Install the completed phalanx them-list and its head target on one
    /// member. This has to stay ordered against the `BreakPhalanx` batch:
    /// the assignment happens while the rebuild recursion unwinds, before
    /// any member's `BattleDecisions` prunes entries that can no longer
    /// fight.
    fn process_synchronous_set_phalanx_them_list(
        &mut self,
        target: u32,
        them: Vec<crate::ai::HumanHandle>,
        primary_target: Option<crate::ai::AiEntityHandle>,
    ) {
        let enemy_ai = self.required_cross_npc_enemy_mut(target, "install-phalanx-them-list");
        tracing::trace!(
            target: "robin_engine::ai_enemy::phalanx",
            member = target,
            ?them,
            ?primary_target,
            "phalanx them-list: installing on member"
        );
        enemy_ai.list_them = them;
        enemy_ai.base.primary_target = primary_target;
    }

    /// Execute one member of Original's recursive
    /// `RHArtificialMalignity::BreakPhalanx` call. Every neighbour clears its
    /// links and immediately runs `BattleDecisions`; delaying this to the
    /// global cross-NPC batch moves swordfight entry and panic RNG by a frame.
    fn process_synchronous_break_phalanx(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        target: u32,
        refresh_them_list: bool,
        logical_think_depth: u8,
        owns_end_think: bool,
        assets: &LevelAssets,
    ) {
        let target_id =
            self.expect_human_id_for_ai_handle(target, "cross-NPC break-phalanx target");
        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        let entity = self
            .world
            .entities
            .get(target_id)
            .expect("validated cross-NPC break-phalanx target vanished");
        assert!(
            entity.enemy_ai().is_some(),
            "cross-NPC break-phalanx target human {target} has no EnemyAi"
        );
        let mut ctx = build_ai_context_from_entity(
            entity,
            self.control.frame_counter,
            None,
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
        self.refresh_selected_default_wait_identity(target_id, &mut ctx);
        // This is a direct recursive `BreakPhalanx` call rather than a typed
        // Think dispatch, but its `PhalanxReinitializeThemList` performs live
        // `IsDetecting180Degrees` checks. Those checks share the Original's
        // surface-owned `ComputeViewRadius` memo with later RefreshDetection
        // in the same frame, so bracket the direct AI call with the same
        // cache handoff as the Think wrapper.
        ctx.seed_view_radius_cache(&self.ai.view_radius_cache);
        let tick_data = self.build_npc_tick_data(sim, target_id, &scratch, assets);
        let previous_target_depth = self.replace_cross_npc_logical_think_depth(
            target_id,
            logical_think_depth,
            "break-phalanx target",
        );
        let previous_target_completion_ownership = self
            .world
            .entities
            .get(target_id)
            .and_then(Entity::ai_controller)
            .expect("break-phalanx target lost its AI after depth projection")
            .completion_latch_inside_think;

        // Keep the borrowed static depth installed through the immediate
        // engine-side prefix. BattleDecisions can recursively break another
        // member while its orders settle, and that member must inherit the
        // same Original-global depth. BreakPhalanx itself owns no EndThink,
        // so restore the target's controller-local approximation raw after
        // the statement boundary instead of calling an EndThink helper.
        {
            let ai_global = &mut self.ai.global;
            let grid = &self.world.fast_grid;
            let Entity::Soldier(soldier) = self
                .world
                .entities
                .get_mut(target_id)
                .expect("cross-NPC break-phalanx target vanished during dispatch")
            else {
                panic!("cross-NPC break-phalanx target {target} stopped being a soldier")
            };
            let enemy_ai = soldier.npc.ai_brain.enemy_mut().unwrap_or_else(|| {
                panic!("cross-NPC break-phalanx target soldier {target} has no enemy AI")
            });
            enemy_ai.break_phalanx_from_neighbour(
                sim,
                ai_global,
                &ctx,
                &tick_data,
                Some(grid),
                refresh_them_list,
            );
        }
        ctx.commit_view_radius_cache(&mut self.ai.view_radius_cache);
        if owns_end_think {
            // The flattened BreakPhalanx batch ends with the initiating
            // member itself. Its BattleDecisions tail is still part of the
            // initiating Think and therefore closes that owner's EndThink.
            self.drain_direct_ai_owner_boundary_mode(sim, target_id, assets, true, false);
        } else {
            // Recursive neighbours execute under the same static depth but
            // own no EndThink of their own.
            self.drain_direct_ai_owner_prefix_boundary_mode(sim, target_id, assets, true, false);
        }
        self.replace_cross_npc_logical_think_depth(
            target_id,
            previous_target_depth,
            "break-phalanx target after prefix",
        );
        self.world
            .entities
            .get_mut(target_id)
            .and_then(Entity::ai_controller_mut)
            .expect("break-phalanx target lost its AI after prefix")
            .completion_latch_inside_think = previous_target_completion_ownership;
    }

    /// Temporarily project Original's static Think recursion byte onto the
    /// controller currently entered through a deferred direct C++ call.
    /// Returns the controller-local approximation so the caller can restore
    /// it raw; the direct method does not necessarily own an `EndThink`.
    pub(in crate::engine) fn replace_cross_npc_logical_think_depth(
        &mut self,
        target_id: EntityId,
        logical_think_depth: u8,
        operation: &str,
    ) -> u8 {
        let ai = self
            .world
            .entities
            .get_mut(target_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap_or_else(|| {
                panic!(
                    "cross-NPC {operation} {} lost its AI controller",
                    target_id.index()
                )
            });
        std::mem::replace(&mut ai.think_recursion_depth, logical_think_depth)
    }

    /// Resume the statement immediately following Original
    /// `TowerGuardCallAlert`. The alert routine directly enters every
    /// recipient's Think before returning, so rebuilding the caller context
    /// here is necessary: `BattleDecisions` can synchronously enter SeekArea,
    /// whose nearby-friend multiplier reads the recipients' new alert status.
    fn process_synchronous_tower_guard_battle_decisions(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        caller: u32,
        assets: &LevelAssets,
    ) {
        assert_eq!(
            source_id.index(),
            caller,
            "tower-guard battle continuation caller must be its owner"
        );
        begin_suspended_tower_guard_alert_think(
            self.world
                .entities
                .get_mut(source_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| panic!("tower-guard caller {caller} lost its AI")),
        );
        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        let building_sector = self
            .world
            .entities
            .get(source_id)
            .map(|entity| self.entity_building_sector(entity.element_data().sector()))
            .unwrap_or_else(|| panic!("tower-guard caller {caller} disappeared"));
        let mut ctx = {
            let entity = self
                .world
                .entities
                .get(source_id)
                .unwrap_or_else(|| panic!("tower-guard caller {caller} disappeared"));
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
        self.refresh_selected_default_wait_identity(source_id, &mut ctx);
        let tick = self.build_npc_tick_data(sim, source_id, &scratch, assets);
        let global = &mut self.ai.global;
        let grid = &self.world.fast_grid;
        self.world
            .entities
            .get_mut(source_id)
            .and_then(Entity::enemy_ai_mut)
            .unwrap_or_else(|| panic!("tower-guard caller {caller} lost its EnemyAi"))
            .battle_decisions(sim, global, &ctx, &tick, Some(grid));
        self.drain_direct_ai_owner_boundary_without_forecast(sim, source_id, assets);
        end_suspended_tower_guard_alert_think(
            self.world
                .entities
                .get_mut(source_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| panic!("tower-guard caller {caller} lost its AI")),
        );
        // EndThink can itself publish a completion event. Close that final
        // piece of the resumed Original call stack before returning to the
        // cross-NPC action dispatcher.
        self.drain_direct_ai_owner_boundary_without_forecast(sim, source_id, assets);
    }

    fn process_synchronous_consider_report(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        target: u32,
        report: crate::ai::ReconnaissanceReport,
        flags: u16,
        assets: &LevelAssets,
    ) {
        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        let frame = self.control.frame_counter;
        let target_id = self.expect_human_id_for_ai_handle(target, "ConsiderReport target");
        self.world
            .entities
            .get_mut(target_id)
            .and_then(Entity::enemy_ai_mut)
            .unwrap_or_else(|| panic!("ConsiderReport target human {target} has no EnemyAi"))
            .base
            .consider_report_merged_at_frame(
                &report,
                flags,
                scratch.ai_entity_views.as_ref(),
                frame,
            );
        self.drain_direct_ai_owner_boundary_without_forecast(sim, target_id, assets);
    }

    fn process_synchronous_finalize_alert_soldiers(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        caller: u32,
        use_formation: bool,
        failure: crate::ai::AlertSoldiersFailureContinuation,
        assets: &LevelAssets,
    ) {
        assert_eq!(
            source_id.index(),
            caller,
            "AlertSoldiers finalization caller must be its owner"
        );
        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        let building_sector = self
            .world
            .entities
            .get(source_id)
            .map(|entity| self.entity_building_sector(entity.element_data().sector()))
            .unwrap_or_else(|| panic!("AlertSoldiers caller {caller} disappeared"));
        let mut ctx = {
            let entity = self
                .world
                .entities
                .get(source_id)
                .unwrap_or_else(|| panic!("AlertSoldiers caller {caller} disappeared"));
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
        self.refresh_selected_default_wait_identity(source_id, &mut ctx);
        let tick = self.build_npc_tick_data(sim, source_id, &scratch, assets);
        let global = &mut self.ai.global;
        let grid = use_formation.then_some(&*self.world.fast_grid);
        self.world
            .entities
            .get_mut(source_id)
            .and_then(Entity::enemy_ai_mut)
            .unwrap_or_else(|| panic!("AlertSoldiers caller {caller} lost its EnemyAi"))
            .finalize_alert_soldiers(sim, failure, global, grid, &ctx, &tick);
        self.drain_direct_ai_owner_boundary_without_forecast(sim, source_id, assets);
    }

    fn process_synchronous_look_there_broadcast(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        caller: u32,
        position: crate::ai::Position,
        radius: u16,
        continuation: crate::ai::LookThereContinuation,
        assets: &LevelAssets,
        defer_turn_instruction: bool,
    ) {
        assert_eq!(
            source_id.index(),
            caller,
            "HeyFolksLookThere broadcast caller must be its owner"
        );
        let (caller_camp, caller_position) = self
            .world
            .entities
            .get(source_id)
            .map(|entity| (entity.camp(), entity.element_data().position()))
            .unwrap_or_else(|| panic!("HeyFolksLookThere caller {caller} disappeared"));
        let radius_sq = f32::from(radius) * f32::from(radius);
        let candidates = self.world.fighter_registry_order();

        for target_id in candidates {
            if target_id == source_id {
                continue;
            }
            let eligible = self
                .world
                .entities
                .get(target_id)
                .filter(|entity| {
                    matches!(entity, Entity::Soldier(_)) && entity.camp() == caller_camp
                })
                .and_then(Entity::enemy_ai)
                .is_some_and(|enemy| {
                    matches!(
                        enemy.base.current_state,
                        crate::ai::AiState::Default | crate::ai::AiState::Wondering
                    ) || (enemy.base.current_state == crate::ai::AiState::Seeking
                        && matches!(
                            enemy.base.current_substate,
                            crate::ai::Substate::SeekingJustWatching
                                | crate::ai::Substate::SeekingJustWatchingSidewards
                        ))
                });
            if !eligible {
                continue;
            }

            // Original evaluates the state and range together immediately
            // before the direct Think call. An earlier recipient may have
            // re-entered and changed this soldier since the loop began.
            let target_position = self
                .world
                .entities
                .get(target_id)
                .unwrap_or_else(|| {
                    panic!(
                        "HeyFolksLookThere target {} disappeared during the registry walk",
                        target_id.index()
                    )
                })
                .element_data()
                .position();
            let dx = target_position.x - caller_position.x;
            let dy = target_position.y - caller_position.y;
            let dz = target_position.z - caller_position.z;
            if !look_there_target_is_inside_radius(dx * dx + dy * dy + dz * dz, radius_sq) {
                continue;
            }

            let hint = crate::ai::Hint {
                seek_point: position,
                seek_flags: 0,
                who_tells_me: crate::ai::AiEntityHandle::new(caller),
            };
            self.world
                .entities
                .get_mut(source_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| panic!("HeyFolksLookThere caller {caller} lost its AI"))
                .outbox
                .reentrant
                .cross_npc_actions
                .push(crate::ai::CrossNpcAction::SendStimulus {
                    target: target_id.index(),
                    stimulus_type: crate::ai::StimulusType::CallLookThere,
                    info: crate::ai::StimulusInfo::Hint(hint),
                    fallback_to_sender: None,
                    to_whole_patrol: false,
                });
            self.process_synchronous_stimuli_for(sim, source_id, assets, defer_turn_instruction);
        }

        self.process_synchronous_look_there_resume(sim, source_id, caller, continuation, assets);
    }

    fn process_synchronous_look_there_resume(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        caller: u32,
        continuation: crate::ai::LookThereContinuation,
        assets: &LevelAssets,
    ) {
        assert_eq!(
            source_id.index(),
            caller,
            "HeyFolksLookThere resume caller must be its owner"
        );
        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        let building_sector = self
            .world
            .entities
            .get(source_id)
            .map(|entity| self.entity_building_sector(entity.element_data().sector()))
            .unwrap_or_else(|| panic!("HeyFolksLookThere caller {caller} disappeared"));
        let mut ctx = {
            let entity = self
                .world
                .entities
                .get(source_id)
                .unwrap_or_else(|| panic!("HeyFolksLookThere caller {caller} disappeared"));
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
        self.refresh_selected_default_wait_identity(source_id, &mut ctx);
        // The tail of `EVENT_VIEW` is what adopts the sighted enemy as the
        // primary target, so at this point the AI still carries whatever
        // target it had before the sighting. Reconstructing the per-tick
        // combat data off that stale handle leaves the enemy distances
        // unseeded, and `BattleDecisions` then reads an infinite
        // nearest-enemy distance and holds the soldier back in reserve
        // instead of engaging a target standing right next to it. Resolve
        // the tick data against the enemy the tail is about to adopt.
        let target_override = match continuation {
            crate::ai::LookThereContinuation::EventView { enemy, .. } => {
                self.entity_id_for_index(enemy)
            }
            _ => None,
        };
        let tick =
            self.build_npc_tick_data_for_target(sim, source_id, &scratch, assets, target_override);
        let global = &mut self.ai.global;
        let grid = &self.world.fast_grid;
        self.world
            .entities
            .get_mut(source_id)
            .and_then(Entity::enemy_ai_mut)
            .unwrap_or_else(|| panic!("HeyFolksLookThere caller {caller} lost its EnemyAi"))
            .resume_after_look_there(sim, continuation, global, Some(grid), &ctx, &tick);
        self.drain_direct_ai_owner_boundary_without_forecast(sim, source_id, assets);
    }

    /// Whether a soldier is still holding its place in a phalanx.
    ///
    /// The phalanx-correction loops re-read this for every member right before
    /// announcing the new slot, because an earlier member's `CALL_INSTRUCTION`
    /// can re-enter and pull later members out of the formation.
    fn soldier_stands_in_phalanx(&self, target_id: EntityId) -> bool {
        self.world
            .entities
            .get(target_id)
            .and_then(Entity::enemy_ai)
            .is_some_and(|enemy| {
                enemy.base.current_substate == crate::ai::Substate::AttackingPhalanx
            })
    }

    fn process_synchronous_gather_instruction(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        target: u32,
        position: crate::ai::Position,
        direction: u16,
        call_instruction: bool,
        assets: &LevelAssets,
    ) {
        let target_id = self.expect_human_id_for_ai_handle(target, "gather-instruction target");
        tracing::trace!(
            target: "robin_engine::ai_enemy::phalanx",
            instructed = target,
            frame = self.control.frame_counter,
            ?position,
            direction,
            call_instruction,
            "synchronous InstructGatherPosition"
        );
        if call_instruction && !self.soldier_stands_in_phalanx(target_id) {
            return;
        }
        let enemy = self
            .world
            .entities
            .get_mut(target_id)
            .and_then(Entity::enemy_ai_mut)
            .unwrap_or_else(|| {
                panic!("InstructGatherPosition target human {target} has no EnemyAi")
            });
        enemy.gather_position = position;
        enemy.gather_direction = direction;
        enemy.gather_position_instructed = true;
        if !call_instruction {
            return;
        }

        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        let building_sector = self
            .world
            .entities
            .get(target_id)
            .map(|entity| self.entity_building_sector(entity.element_data().sector()))
            .unwrap_or_else(|| panic!("gather-instruction target {target} disappeared"));
        let ctx = build_ai_context_from_entity(
            self.world
                .entities
                .get(target_id)
                .unwrap_or_else(|| panic!("gather-instruction target {target} disappeared")),
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
        let tick = self.build_npc_tick_data(sim, target_id, &scratch, assets);
        self.dispatch_think_with_drain_without_forecast(
            sim,
            target_id,
            &crate::ai::Stimulus::new(crate::ai::StimulusType::CallInstruction),
            &ctx,
            &tick,
            assets,
        );
    }

    fn process_synchronous_stimuli_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        assets: &LevelAssets,
        defer_turn_instruction: bool,
    ) {
        let actions = self
            .world
            .entities
            .get_mut(source_id)
            .and_then(Entity::ai_controller_mut)
            .map(crate::ai::AiController::take_pending_synchronous_stimuli)
            .unwrap_or_else(|| {
                panic!(
                    "synchronous stimulus source {} has no AI controller",
                    source_id.index()
                )
            });

        for action in actions {
            let crate::ai::CrossNpcAction::SendStimulus {
                target,
                stimulus_type,
                info,
                fallback_to_sender,
                to_whole_patrol,
            } = action
            else {
                unreachable!("synchronous-stimulus drain returned a different cross-NPC action")
            };
            let target_id = self.entity_id_for_index(target).unwrap_or_else(|| {
                panic!(
                    "synchronous {stimulus_type:?} from NPC {} references missing target {target}",
                    source_id.index()
                )
            });
            assert!(
                matches!(self.world.entities.get(target_id), Some(Entity::Soldier(_))),
                "synchronous {stimulus_type:?} target {target} is not a soldier"
            );

            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let building_sector = self
                .world
                .entities
                .get(target_id)
                .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                .unwrap_or_else(|| {
                    panic!("synchronous {stimulus_type:?} target {target} disappeared")
                });
            let ctx = {
                let entity = self.world.entities.get(target_id).unwrap_or_else(|| {
                    panic!("synchronous {stimulus_type:?} target {target} disappeared")
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
            let tick_data = self.build_npc_tick_data(sim, target_id, &scratch, assets);
            let mut stimulus = crate::ai::Stimulus::new(stimulus_type);
            stimulus.info = info;
            stimulus.to_whole_patrol = to_whole_patrol;
            tracing::trace!(
                target: "patrol_relay",
                source = source_id.index(),
                target,
                ?stimulus_type,
                to_whole_patrol,
                "synchronous SendStimulus drain"
            );
            let handled = self.dispatch_think_with_drain_mode(
                sim,
                target_id,
                &stimulus,
                &ctx,
                &tick_data,
                assets,
                true,
                defer_turn_instruction,
            );
            if !handled && let Some(sender) = fallback_to_sender {
                let sender_id = self.entity_id_for_index(sender).unwrap_or_else(|| {
                    panic!(
                        "synchronous {stimulus_type:?} fallback references missing sender {sender}"
                    )
                });
                let scratch = self.build_owner_context_scratch_without_forecast(assets);
                let building_sector = self
                    .world
                    .entities
                    .get(sender_id)
                    .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                    .unwrap_or_else(|| panic!("synchronous fallback sender {sender} disappeared"));
                let sender_ctx = {
                    let entity = self.world.entities.get(sender_id).unwrap_or_else(|| {
                        panic!("synchronous fallback sender {sender} disappeared")
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
                let sender_tick = self.build_npc_tick_data(sim, sender_id, &scratch, assets);
                self.dispatch_think_with_drain_mode(
                    sim,
                    sender_id,
                    &stimulus,
                    &sender_ctx,
                    &sender_tick,
                    assets,
                    true,
                    defer_turn_instruction,
                );
            }
        }
    }
}
