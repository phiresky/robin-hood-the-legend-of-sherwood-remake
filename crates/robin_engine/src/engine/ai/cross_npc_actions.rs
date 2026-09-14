use super::*;

impl EngineInner {
    fn required_cross_npc_enemy_mut(
        &mut self,
        target: u32,
        operation: &str,
    ) -> &mut crate::ai_enemy::EnemyAi {
        // Original-game combat-neighbor operations take human-actor references.
        // `HumanHandle` is the raw sparse element slot, not a SoldierId; an
        // AI-controlled hero therefore has to retain its ActorPc entity kind here.
        let target_id = self.expect_human_id_for_ai_handle(target, operation);
        self.world.entities.expect_enemy_ai_mut(
            target_id,
            format_args!("cross-NPC {operation} target human {target}"),
        )
    }

    /// Execute the complete original-game patrol clearing made by the
    /// `RemoveAllSubordinates` script native.
    ///
    /// Clearing an AI patrol clears each member's chief
    /// reference and forces a return to duty before clearing the chief's
    /// lists. This is a direct return, not an `EVENT_RETURN_TO_DUTY` decision: in
    /// particular, it bypasses decision-tick admission's script-lock refusal. Keep the
    /// direct duty transition, movement construction, and recursive callbacks
    /// inside this engine-owned script barrier while leaving ordinary owner
    /// instruction to subsequent sequence processing.
    pub(crate) fn script_remove_all_subordinates(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        chief: EntityId,
    ) {
        let members = self
            .world
            .entities
            .expect_ai_controller(chief, format_args!("RemoveAllSubordinates chief"))
            .theoretical_patrol
            .clone();

        for member in members.iter().copied() {
            let should_return = {
                let ai = self.world.entities.expect_ai_controller_mut(
                    member,
                    format_args!(
                        "RemoveAllSubordinates chief {} references missing NPC member {}",
                        chief.index(),
                        member.index()
                    ),
                );
                ai.patrol_chief = None;
                ai.current_state == crate::ai::AiState::Default
            };
            if !should_return {
                continue;
            }
            self.execute_ai_return_to_duty(sim, assets, member, crate::ai::DutyFlags::empty());
            // A forced duty call does not close a Think frame. Keep its
            // close-post latch available for the actor's actual completion.
            self.drain_direct_ai_owner_boundary(sim, member, assets);
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
        // NPC lookup follows the original-game registration array.
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

        // The noise record keeps the complete position supplied by the source,
        // including its motion-sector pointer. Delayed reactions later feed
        // that position through world-point conversion, so dropping the sector
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
    /// Noise handling computes heard volume immediately before that listener's
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
                Entity::Soldier(s) => self.camps_are_hostile(
                    s.soldier.cached_camp,
                    crate::element_kinds::Camp::Royalists,
                ),
                _ => false,
            };
            if !include {
                return None;
            }

            // Do not pre-filter inactive or unconscious NPCs. Original runs
            // heard-volume calculation for every registered civilian/Lacklandist and
            // leaves refusal to decision-tick admission, after the deafness read.
            let elem = entity.element_data();
            (elem.position_map(), elem.position())
        };

        let source_elev = noise.elevation as f32;
        let modified_volume = noise.volume as f32 * HEARING_FACTOR;
        // The original game's hearing-volume calculation subtracts the source point from the
        // listener's authoritative world position. Do not rebuild Y
        // from `position_map + elevation`: a 3D-authored position projected
        // into map space can reconstruct one bit away, which is observable
        // when the positive remainder truncates to 16 bits at volume 1.
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
        // Heard-volume calculation returns before checking deafness when the Euclidean
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
            .expect_ai_actor_data_mut(
                npc_id,
                format_args!(
                    "one-shot noise listener {} lost its required AI actor state",
                    npc_id.index()
                ),
            )
            .get_deafness(frame, cover_volume);

        let subjective = subjective_hear_volume(modified_volume, distance, deafness);
        (subjective != 0).then_some(crate::ai::Noise {
            volume: subjective,
            ..noise
        })
    }

    fn display_one_shot_noise(&mut self, noise: crate::ai::Noise) {
        // The original game displays noise only after every listener's AI update.
        self.feedback
            .pending_side_effects
            .displayed_noises
            .push(noise);
    }

    /// Broadcast a one-shot noise and synchronously run each listener's new
    /// hearing event, in original-game NPC registration order.
    ///
    /// Original-game NPC noise handling invokes AI inside the broadcast
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

            // Each listener observes all mutations from the preceding call.
            self.execute_ai_callback(sim, assets, npc_id, &stimulus);
        }
        self.display_one_shot_noise(noise);
    }

    // ── Cross-NPC action processing (phalanx coordination) ──────────
    //
    // After all AI think() calls, drain each NPC's pending cross-NPC
    // actions and apply them to the target NPCs. This covers:
    // - SendStimulus (e.g. CALL_COORDINATE to archers)
    // - left/right combat-neighbor assignment for phalanx linking

    pub(super) fn apply_update_left_combat_neighbour(
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

    pub(super) fn apply_update_right_combat_neighbour(
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
        let ai = self.world.entities.expect_ai_controller_mut(
            target_id,
            format_args!("synchronization target human {target}"),
        );
        // Registering a synchronizing AI actor is a direct,
        // unconditional append. In particular, the target can reach its
        // waypoint in a later element update slot in this same frame and
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
            // removal notifications, MYTALK callbacks, etc.). This
            // drain used to live at the tail of this function, which
            // meant it was skipped entirely on ticks with no cross-NPC
            // actions — the common case — stranding queued stimuli
            // forever and hanging states like
            // `DefaultOnPostLookingSidewards` that wait on `EventDone`
            // to exit.
            self.drain_pending_self_stimuli(sim, assets);
            return;
        }

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
                crate::ai::CrossNpcAction::ResumeAfterLookThere { caller, .. } => {
                    panic!("look-there resume for caller {caller} escaped its owner boundary")
                }
                crate::ai::CrossNpcAction::BroadcastLookThere { caller, .. } => {
                    panic!("look-there broadcast for caller {caller} escaped its owner boundary")
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

                    let handled = target_id
                        .filter(|id| {
                            self.world
                                .entities
                                .get(*id)
                                .and_then(Entity::ai_controller)
                                .is_some()
                        })
                        .is_some_and(|id| {
                            self.dispatch_filtered_stimulus(sim, assets, id, &stimulus, None)
                        });
                    // Fallback: if target couldn't handle the stimulus,
                    // redeliver to the sender (e.g. conversation chains).
                    if !handled && let Some(sender) = fallback_to_sender {
                        let Some(sender_id) = self.entity_id_for_index(sender) else {
                            continue;
                        };
                        if self
                            .world
                            .entities
                            .get(sender_id)
                            .and_then(Entity::ai_controller)
                            .is_some()
                        {
                            self.dispatch_filtered_stimulus(
                                sim, assets, sender_id, &stimulus, None,
                            );
                        }
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
                //      clearing the right combat neighbour) — clear
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

                crate::ai::CrossNpcAction::ConsiderReport { target, .. } => {
                    panic!("report transfer to {target} escaped its owner boundary");
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
    /// side-effect drain pass so handler side effects (sequence launch,
    /// attentive-mode changes, facing, quitting/entering swordfight, looking sideways,
    /// …) and any completion notifications / re-entrant `EVENT_DONE` they
    /// trigger happen synchronously before the outer AI response completes.
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
        target: Option<EntityId>,
        assets: &LevelAssets,
    ) -> bool {
        let had_ai_at_entry = self
            .world
            .entities
            .get(npc_id)
            .and_then(Entity::ai_controller)
            .is_some();
        let handled = self.dispatch_filtered_stimulus_inner(sim, assets, npc_id, stimulus, target);

        // PCs can participate in direct swordfights but have no NPC AI
        // controller or AI-owned recovery effects to drain.
        if !had_ai_at_entry && matches!(self.world.entities.get(npc_id), Some(Entity::Pc(_))) {
            return handled;
        }

        // Decision-tick admission applies view status synchronously for
        // LOSE_CONSCIOUSNESS, WASP, and NET. FITAGAIN can publish its
        // resurrection work at this same boundary. The typed AI records
        // those engine-owned writes while its controller is borrowed; commit
        // them immediately after Think returns, before waypoint callbacks or
        // any other pending/re-entrant work can observe stale NPC state.
        self.tick_ai_pending_resurrection_and_eyes_for_npc(npc_id);

        // AI waypoint-script execution invokes the
        // waypoint VM directly from the active Think handler. Close that
        // authored callback before the generic post-Think effect drain:
        // script natives such as AssignPath recursively enter
        // EVENT_RETURN_TO_DUTY before later orders or condolations from the
        // outer handler can settle.
        self.dispatch_pending_waypoint_script_for_owner(sim, npc_id, assets);

        // Enemy-sighting processing explicitly marks an accepted VIEW after
        // all decision-tick admission and handler guards. Mirror that one-shot onto the
        // engine-owned AI actor record before draining its other synchronous
        // effects. Locked, frozen, script-filtered, and handler-rejected VIEWs
        // never set the flag.
        let mark_alerted = self.world.entities.expect_ai_controller_mut(
            npc_id,
            format_args!(
                "handled Think recipient {} lost its entity or AI controller before drain",
                npc_id.index()
            ),
        );
        let mark_alerted = std::mem::take(&mut mark_alerted.outbox.detection.mark_alerted);
        if mark_alerted {
            let ai_actor = self.world.entities.expect_ai_actor_data_mut(
                npc_id,
                format_args!("accepted EVENT_VIEW recipient after its synchronous Think"),
            );
            ai_actor.alerted = true;
        }

        const MAX_ITERS: u32 = 8;
        for iter in 0..MAX_ITERS {
            // Drain the per-NPC pending-flags pass (launches sequences,
            // commands, turn orders, attentive-mode transitions, etc.).
            self.drain_pending_for_npc(sim, npc_id, assets);
            // `drain_pending_for_npc` launches the first order barrier in its
            // original position. Close the boundary again because later
            // effect application and civilian handlers share the same base
            // order outbox. Owner-local state-change notifications are also part
            // of this fixed point, so late script-seek callbacks cannot leak
            // into a global batch or strand in the outbox.
            self.launch_pending_orders_for_npc(sim, assets, npc_id);

            self.process_synchronous_reentrant_actions_for(sim, npc_id, assets);

            // Any condolations the drain above queued (sequences that
            // got preempted by the side effects) fire here — which may
            // push EventDone / EventImpossible into pending_self_stimuli.
            self.dispatch_condolations(sim, assets);

            // Re-enter Think for each self-stimulus (EventDone, MYTALK,
            // etc.).  This may queue more pending flags — loop again.
            let has_self_stimuli = {
                let ai = self.world.entities.expect_ai_controller(
                    npc_id,
                    format_args!("handled Think recipient before self-stimulus recheck"),
                );
                !ai.outbox.reentrant.self_stimuli.is_empty()
            };
            if has_self_stimuli {
                self.drain_self_stimuli_for_npc(sim, npc_id, assets);
            }

            // A re-entrant self stimulus can itself call another NPC. Close
            // those direct original-game call boundaries before deciding this owner has
            // stabilised; otherwise the result-bearing request can escape to
            // the global cross-action batch.
            self.process_synchronous_reentrant_actions_for(sim, npc_id, assets);

            let still_pending = {
                let ai = self.world.entities.expect_ai_controller(
                    npc_id,
                    format_args!("handled Think recipient before fixed-point recheck"),
                );
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

    pub(in crate::engine) fn process_synchronous_reentrant_actions_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) {
        loop {
            let actions = self
                .world
                .entities
                .expect_ai_controller_mut(source_id, format_args!("synchronous action source"))
                .take_pending_synchronous_cross_npc_actions();
            if actions.is_empty() {
                break;
            }

            let deferred = {
                let ai = self.world.entities.expect_ai_controller_mut(
                    source_id,
                    format_args!(
                        "synchronous action source {} lost its AI controller",
                        source_id.index()
                    ),
                );
                std::mem::take(&mut ai.outbox.reentrant.cross_npc_actions)
            };
            let mut deferred = deferred;
            for action in actions {
                match action {
                    crate::ai::CrossNpcAction::ConsiderReport { target, flags } => {
                        let target_id =
                            self.expect_human_id_for_ai_handle(target, "report transfer target");
                        self.consider_live_ai_report(sim, assets, target_id, source_id, flags);
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
                    crate::ai::CrossNpcAction::RegisterSynchronizingActor { target, actor } => {
                        self.register_synchronizing_actor(target, actor);
                    }
                    crate::ai::CrossNpcAction::SendStimulus { .. } => {
                        self.requeue_isolated_synchronous_action(source_id, action.clone());
                        self.process_synchronous_stimuli_for(sim, source_id, assets)
                    }
                    crate::ai::CrossNpcAction::RequestAlert { .. } => {
                        self.requeue_isolated_synchronous_action(source_id, action.clone());
                        self.process_synchronous_alert_requests_for(sim, source_id, assets)
                    }
                    crate::ai::CrossNpcAction::RequestThinkResult { .. } => {
                        self.requeue_isolated_synchronous_action(source_id, action.clone());
                        self.process_synchronous_think_results_for(sim, source_id, assets)
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

                // Direct original-game calls are depth-first: if A emits C while B was
                // already queued, C closes before B. Isolate A's generated
                // work, recursively drain it, then continue the saved batch.
                self.process_synchronous_reentrant_actions_for(sim, source_id, assets);
                let ai = self.world.entities.expect_ai_controller_mut(
                    source_id,
                    format_args!(
                        "synchronous action source {} lost its AI controller",
                        source_id.index()
                    ),
                );
                deferred.extend(std::mem::take(&mut ai.outbox.reentrant.cross_npc_actions));
            }
            let ai = self.world.entities.expect_ai_controller_mut(
                source_id,
                format_args!(
                    "synchronous action source {} lost its AI controller",
                    source_id.index()
                ),
            );
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
            .expect_ai_controller_mut(
                source_id,
                format_args!(
                    "synchronous action source {} lost its AI controller",
                    source_id.index()
                ),
            )
            .outbox
            .reentrant
            .cross_npc_actions
            .push(action);
    }

    /// Resume the statement immediately following Original
    /// tower-guard alerts. The alert routine directly enters every
    /// recipient's Think before returning, so rebuilding the caller context
    /// here is necessary: battle planning can synchronously start an area search,
    /// whose nearby-friend multiplier reads the recipients' new alert status.

    fn process_synchronous_look_there_broadcast(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: EntityId,
        caller: u32,
        position: crate::ai::Position,
        radius: u16,
        continuation: crate::ai::LookThereContinuation,
        assets: &LevelAssets,
    ) {
        assert_eq!(
            source_id.index(),
            caller,
            "look-there caller must be its owner"
        );
        self.execute_ai_look_there(sim, assets, source_id, position, radius);
        self.process_synchronous_look_there_resume(sim, source_id, caller, continuation, assets);
    }

    pub(super) fn execute_ai_look_there(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        source_id: EntityId,
        position: crate::ai::Position,
        radius: u16,
    ) {
        let camp = self.expect_entity(source_id, "look-there caller").camp();
        let count = self.ai.global.all_soldier_handles.len();
        let radius_squared = f32::from(radius).powi(2);
        let stimulus = crate::ai::Stimulus {
            info: crate::ai::StimulusInfo::Hint(crate::ai::Hint {
                seek_point: position,
                seek_flags: 0,
                who_tells_me: crate::ai::AiEntityHandle::new(source_id.index()),
            }),
            ..crate::ai::Stimulus::new(crate::ai::StimulusType::CallLookThere)
        };
        for index in 0..count {
            let handle = *self
                .ai
                .global
                .all_soldier_handles
                .get(index)
                .expect("look-there soldier registry shortened during callback");
            let target_id = EntityId::Soldier(crate::entity_id::SoldierId(handle));
            if target_id == source_id {
                continue;
            }
            let entity = self.expect_entity(target_id, "look-there soldier");
            if !self.camps_are_allied(entity.camp(), camp) {
                continue;
            }
            let ai = entity
                .enemy_ai()
                .expect("look-there soldier requires enemy AI");
            if !matches!(
                ai.base.current_state,
                crate::ai::AiState::Default | crate::ai::AiState::Wondering
            ) && !(ai.base.current_state == crate::ai::AiState::Seeking
                && matches!(
                    ai.base.current_substate,
                    crate::ai::Substate::SeekingJustWatching
                        | crate::ai::Substate::SeekingJustWatchingSidewards
                ))
            {
                continue;
            }
            let target = entity.element_data().position();
            let caller = self
                .expect_entity(source_id, "look-there range caller")
                .element_data()
                .position();
            let dx = target.x - caller.x;
            let dy = target.y - caller.y;
            let dz = target.z - caller.z;
            if look_there_target_is_inside_radius(dx * dx + dy * dy + dz * dz, radius_squared) {
                self.execute_ai_callback(sim, assets, target_id, &stimulus);
            }
        }
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
            "look-there resume caller must be its owner"
        );
        let scratch = self.build_sim_scratch(assets);
        let building_sector = self
            .world
            .entities
            .get(source_id)
            .map(|entity| self.entity_building_sector(entity.element_data().sector()))
            .unwrap_or_else(|| panic!("look-there caller {caller} disappeared"));
        let mut ctx = {
            let entity = self.expect_entity(source_id, "look-there caller");
            self.ai_context_from_entity(
                entity,
                self.control.frame_counter,
                building_sector,
                &scratch,
                assets,
            )
        };
        self.refresh_selected_default_wait_identity(source_id, &mut ctx);
        // The tail of `EVENT_VIEW` is what adopts the sighted enemy as the
        // primary target, so at this point the AI still carries whatever
        // target it had before the sighting. Reconstructing the per-tick
        // combat data off that stale handle leaves the enemy distances
        // unseeded, and battle planning then reads an infinite
        // nearest-enemy distance and holds the soldier back in reserve
        // instead of engaging a target standing right next to it. Resolve
        // the tick data against the enemy the tail is about to adopt.
        let target_override = match continuation {
            crate::ai::LookThereContinuation::EventView { enemy, .. } => {
                self.entity_id_for_index(enemy)
            }
            _ => None,
        };
        let tick = self.build_npc_tick_data_for_target(sim, source_id, assets, target_override);
        let global = &mut self.ai.global;
        let grid = &self.world.fast_grid;
        let flow = self
            .world
            .entities
            .expect_enemy_ai_mut(
                source_id,
                format_args!("look-there caller {caller} lost its EnemyAi"),
            )
            .resume_after_look_there(
                crate::ai_enemy::ThinkEnv::new(sim, &ctx, &tick, Some(grid)),
                continuation,
                global,
            );
        if let Err(call) = flow {
            self.execute_ai_duty_call(sim, assets, source_id, call);
        }
        self.drain_direct_ai_owner_boundary(sim, source_id, assets);
    }

    fn process_synchronous_stimuli_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) {
        let actions = self
            .world
            .entities
            .expect_ai_controller_mut(source_id, format_args!("synchronous stimulus source"))
            .take_pending_synchronous_stimuli();

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
            let handled = self.dispatch_think_with_drain(sim, target_id, &stimulus, None, assets);
            if !handled && let Some(sender) = fallback_to_sender {
                let sender_id = self.entity_id_for_index(sender).unwrap_or_else(|| {
                    panic!(
                        "synchronous {stimulus_type:?} fallback references missing sender {sender}"
                    )
                });
                self.dispatch_think_with_drain(sim, sender_id, &stimulus, None, assets);
            }
        }
    }
}
