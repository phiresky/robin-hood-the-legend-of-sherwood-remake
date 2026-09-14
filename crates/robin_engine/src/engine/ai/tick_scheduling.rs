use super::*;

impl EngineInner {
    #[inline(never)]
    fn trace_consider_report_drain_start(
        frame: u32,
        npc_id: EntityId,
        pending_mutations: &impl std::fmt::Debug,
    ) {
        eprintln!(
            "CONSIDERREPORT {{\"stage\":\"drain_start\",\"frame\":{},\"owner\":{},\"pending_mutations\":{:?}}}",
            frame,
            npc_id.index(),
            pending_mutations,
        );
    }

    #[inline(never)]
    fn trace_consider_report_drain_end(
        frame: u32,
        npc_id: EntityId,
        bodies: &[crate::element::Detectable],
    ) {
        let body_ids = bodies
            .iter()
            .map(|detectable| detectable.element.map(EntityId::index))
            .collect::<Vec<_>>();
        eprintln!(
            "CONSIDERREPORT {{\"stage\":\"drain_end\",\"frame\":{},\"owner\":{},\"body_ids\":{:?}}}",
            frame,
            npc_id.index(),
            body_ids
        );
    }

    #[cfg(test)]
    pub(in crate::engine) fn tick_enemy_ai(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        // This detection-only test seam predates the production owner walk.
        // Preserve its contract that all PCs have refreshed their noise before
        // the first NPC is evaluated.
        let pc_ids = self.world.pc_ids.clone();
        for pc_id in pc_ids {
            self.refresh_pc_produced_noise_for(pc_id);
        }
        let run_detection = !self.actors_frozen();
        self.tick_enemy_ai_inner(sim, assets, false);
        // This detection-only test driver explicitly closes its synthetic
        // frame. Production drains belong to each creation-ordered owner.
        if run_detection {
            self.tick_enemy_ai_drain_swordfight_requests(sim, assets);
            self.tick_enemy_ai_drain_pending_stimuli(sim, assets);
        }
    }

    /// Legacy test coordinator for complete NPC updates without actor movement.
    ///
    /// Each NPC consumes only its own body/recovery work and refreshes its
    /// own view immediately before its creation-ordered detection refresh.
    /// The direct `tick_enemy_ai` entry point remains detection-only for
    /// focused tests that construct already-refreshed vision state.
    #[cfg(test)]
    pub(in crate::engine) fn tick_enemy_ai_with_creation_ordered_prelude(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        self.tick_enemy_ai_inner(sim, assets, true);
    }

    /// Initialize transient actor counters before the fused owner pass.
    pub(in crate::engine) fn prepare_npc_owner_pass(&mut self) {
        if !self.ai.global.primary_target_multiplicity_initialized {
            // The human actor's primary-target multiplicity is temporary initialization state and
            // is explicitly absent from the save stream. Loading a save into
            // newly-created actors therefore starts every counter at zero,
            // even if restored AI state already describes a swordfight.
            self.ai.global.primary_target_multiplicity_scratch.clear();
            self.ai.global.primary_target_multiplicity_initialized = true;
        }
    }

    /// Run one NPC's complete post-human envelope using live inputs sampled at
    /// this legacy slot. No later owner's view or forecast is constructed.
    pub(in crate::engine) fn tick_npc_owner_pass(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        shield_links_need_refresh: &mut bool,
        npc_id: EntityId,
    ) {
        self.debug_refresh_view_lifecycle("npc_tail_enter", npc_id, None);
        let entity = self.expect_entity(npc_id, "NPC owner before its fused legacy-slot envelope");
        assert!(
            entity.ai_actor_data().is_some(),
            "fused AI owner {} has no AI actor data",
            npc_id.index()
        );
        // FrozenAll is volatile script state. Sample it at the consuming NPC
        // slot rather than caching it before earlier owners run callbacks.
        if self.actors_frozen() {
            self.debug_refresh_view_lifecycle("npc_tail_frozen_skip", npc_id, None);
            self.tick_npc_post_detection_tail_for_npc(sim, npc_id, assets);
            return;
        }

        if *shield_links_need_refresh {
            self.refresh_archer_shield_links();
            *shield_links_need_refresh = false;
        }
        self.tick_inform_my_friends_for_npc(npc_id);
        self.refresh_npc_view_for_npc(npc_id);
        self.tick_enemy_ai_refresh_detection(sim, assets, npc_id);
        self.tick_npc_post_detection_tail_for_npc(sim, npc_id, assets);
    }

    pub(in crate::engine) fn tick_enemy_ai_blip_detection_for_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> bool {
        self.tick_enemy_ai_blip_detection(sim, assets, owner)
    }

    #[cfg(test)]
    fn tick_enemy_ai_inner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        run_owner_envelope: bool,
    ) {
        if self.actors_frozen() {
            // Frozen-all skips patrol/view/detection/ambush/deafness but the
            // original still enters each NPC's busy/ladder/speech/lock gate,
            // where all three deadlines are extended before returning.
            if run_owner_envelope {
                let npc_ids: Vec<_> = self.world.entities.ai_owner_ids().collect();
                for npc_id in npc_ids {
                    self.tick_npc_post_detection_tail_for_npc(sim, npc_id, assets);
                }
            }
            return;
        }
        if !self.ai.global.primary_target_multiplicity_initialized {
            self.ai.global.primary_target_multiplicity_scratch.clear();
            self.ai.global.primary_target_multiplicity_initialized = true;
        }

        self.refresh_archer_shield_links();

        // ── 2a. Listen/object blip work. ────────────────────────
        // NPC-owned SeesBlip remains inside its creation-ordered
        // detection-refresh slot below.
        let pc_ids = self.world.pc_ids.clone();
        for pc_id in pc_ids {
            self.tick_enemy_ai_blip_detection(sim, assets, pc_id);
        }

        // Test drivers explicitly choose either a complete NPC envelope or
        // detection alone. Geometry arguments never select scheduling phases.
        let owners: Vec<_> = self.world.entities.ai_owner_ids().collect();
        for npc_id in owners {
            if run_owner_envelope {
                if self.dispatch_pending_fit_again_for_npc(sim, npc_id, assets) {
                    self.tick_ai_pending_resurrection_and_eyes_for_npc(npc_id);
                    self.apply_wake_redetection_blinks(npc_id);
                }
                self.tick_inform_my_friends_for_npc(npc_id);
                self.refresh_npc_view_for_npc(npc_id);
            }
            self.tick_enemy_ai_refresh_detection(sim, assets, npc_id);
            if run_owner_envelope {
                self.tick_npc_post_detection_tail_for_npc(sim, npc_id, assets);
            }
        }

        // Sword strikes are launched by `engine::melee::tick_enemy_sword_attacks`.
        // Keep this AI pass to target selection, pursuit, and swordfight
        // requests; applying direct damage here would bypass the
        // wait-timer + interaction sequence timing.
    }

    /// Per-NPC drain for all `pending_*` flags on [`AiController`] that
    /// mutate engine state (launch sequences / orders, toggle attentive
    /// mode, fire cross-NPC stimuli, etc.).  Extracted from the global
    /// post-Think drain loop so the same body can also run synchronously
    /// right after each [`Self::dispatch_filtered_stimulus`] call via
    /// [`Self::dispatch_think_with_drain`] — matching `think()`
    /// semantics where handler side effects (`launch_sequence`,
    /// `set_attentive_mode`, `face`, …) are immediate.
    pub(in crate::engine) fn drain_pending_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) {
        let Some(mut drain) = self.drain_pending_owner_prelude(sim, npc_id, assets) else {
            return;
        };
        self.drain_pending_swordfight_effects(sim, npc_id, assets, &mut drain);
        self.drain_pending_focus_and_orders(sim, npc_id, assets, &mut drain);
        self.drain_pending_guard_and_archery(npc_id, &drain);
        self.drain_pending_launches(sim, npc_id, assets, &mut drain);
        self.drain_pending_detectable_mutations(npc_id, &drain);
        self.drain_pending_coins_posture_and_alerts(sim, npc_id, assets, &drain);
        self.drain_pending_panic_and_search(sim, npc_id, assets);
    }

    /// Direction goal, halt barrier, and the first post-Think
    /// channel takes. `None` means the owner or its AI controller vanished
    /// before the halt barrier; the drain then stops.
    fn drain_pending_owner_prelude(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) -> Option<PendingDrainBarrier> {
        self.drain_patrol_direction_broadcast_for(sim, npc_id, assets);

        // Direct direction assignments made before stopping must update the goal
        // before the halt/transition barrier. They do not create a standalone
        // Turn element; the subsequently selected action performs the turn.
        let direction_goal = {
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                return None;
            };
            let Some(ai) = entity.ai_controller_mut() else {
                return None;
            };
            ai.outbox.actor.take_direction_goal()
        };
        if let Some(direction_goal) = direction_goal
            && let Some(entity) = self.world.entities.get_mut(npc_id)
        {
            entity.position_iface_mut().set_direction(
                crate::position_interface::Direction::from_raw(direction_goal as i32),
            );
        }

        // Drain pending_halt FIRST so the actor's in-progress sequence
        // (typically a Move element while running toward the target) is
        // torn down before any subsequent intent (e.g.
        // `pending_enter_swordfight`) launches a new sequence.
        // `begin_swordfight` / `break_macro` callers call
        // `stop_all() → halt() → stop(PREFERENCE)` inline before
        // `launch_sequence_element(EnterSwordfight)`.
        //
        // Without this ordering, `enter_swordfight`'s
        // `pathfinder.cancel_requests_for` (a no-op post-refactor) and
        // local `clear_path` leave the orphaned Move sequence in
        // InProgress state.  An in-flight path response then
        // `try_dispatch_move_path`s onto the actor a few ticks later,
        // restoring `active_movement` and re-driving the run animation
        // — the visual "stuck in running pose" symptom.
        let halt_count = {
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                return None;
            };
            let Some(ai) = entity.ai_controller_mut() else {
                return None;
            };
            ai.outbox.actor.take_halt_count()
        };
        if halt_count != 0 {
            // Stop-all applies preference-based stopping synchronously before the
            // handler continues into state/attentive-mode changes and other
            // replacement work. Deliver the halt condolence at that same
            // barrier: actor-base cleanup must clear a selected movement's
            // cached goal before a newly launched attentive transition can
            // become the selected element. `from_halt` suppresses the NPC
            // EventDone/Impossible callbacks while retaining that base
            // selected-element cleanup.
            for _ in 0..halt_count {
                self.halt_actor(npc_id);
                self.dispatch_condolations(sim, assets);
            }
        }

        // The halt application above is a real same-frame barrier: only now
        // take the prefixes that the original `go_to` path launches next.
        let preemption = {
            let ai = self
                .world
                .entities
                .expect_ai_controller_mut(npc_id, format_args!("pending-drain NPC"));
            ai.outbox.actor.take_movement_prefixes()
        };

        // Take exactly the channels read at the first post-Think barrier.
        // Later barrier groups remain live so re-entrant sequence work can
        // still enqueue effects that this pass observes at their Original
        // application point.
        let effects = {
            let ai = self
                .world
                .entities
                .expect_ai_controller_mut(npc_id, format_args!("pending-drain NPC"));
            ai.outbox.actor.take_core()
        };
        assert!(
            effects.enter_swordfight_jump_line.is_none()
                || matches!(
                    effects.enter_swordfight,
                    Some(crate::ai::EnterSwordfightRequest::Engage(_))
                ),
            "pending-drain owner {} queued a swordfight jump line without an engagement",
            npc_id.index()
        );
        Some(PendingDrainBarrier {
            preemption,
            effects,
        })
    }

    /// Swordfight exit, movement preemption prefixes, target stop,
    /// attentive-mode requests and swordfight entry.
    fn drain_pending_swordfight_effects(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
        drain: &mut PendingDrainBarrier,
    ) {
        let PendingDrainBarrier {
            preemption,
            effects,
            ..
        } = drain;

        // Swordfight exit launches an explicit QUIT_SWORDFIGHT element. Do
        // not tear down the relationship directly here: the command owns
        // both that teardown and the visible lowering-sword transition, and
        // Sequence-element launch arbitrates it synchronously in the original game.
        let retry_quit_swordfight = effects.retry_quit_swordfight
            && self
                .current_sequence_element_for_actor(npc_id)
                .and_then(|(sequence, index)| {
                    self.orders.sequence_manager.get_element(sequence, index)
                })
                .is_none_or(|element| element.command != crate::element::Command::QuitSwordfight);
        if effects.quit_swordfight || retry_quit_swordfight {
            self.launch_element(crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::QuitSwordfight,
                Some(npc_id),
            ));
            // Sequence-element launch reaches instruction synchronously. If the
            // quit replaces a selected command, its removal-notification
            // callback therefore re-enters the decision tick before swordfight exit
            // returns to its caller.
            self.dispatch_condolations(sim, assets);
        }

        // Apply an explicit stop-menacing request.
        if preemption.stop_menace {
            let elem = crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::StopMenace,
                Some(npc_id),
            );
            self.launch_element(elem);
        }

        // Apply an explicit shield-lowering request.
        if preemption.lower_shield {
            let elem = crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::LowerShield,
                Some(npc_id),
            );
            self.launch_element(elem);
        }

        // Process pending `stop()` on a different entity — the
        // `primary_target.stop()` call inside `begin_swordfight`.  The
        // default `stop()` uses `Normal` priority.  Drained before
        // `enter_swordfight` so the target's in-flight Move element is
        // torn down before the engine-side ENTER_SWORDFIGHT sequence
        // runs.
        if let Some(target_handle) = effects.stop_target {
            let target_id =
                self.expect_human_id_for_ai_handle(target_handle.get(), "AI stop_target");
            // The original game queries these members at this exact swordfight-start point
            // point in the live entity walk. Do not use the AI tick snapshot:
            // an earlier-created target may have completed a movement-start
            // transition since that snapshot was built.
            let should_stop = self
                .get_entity(target_id)
                .and_then(|entity| {
                    Some((
                        entity.human_data()?.opponents.is_empty(),
                        entity.actor_data()?.action_state.is_moving(),
                    ))
                })
                .unwrap_or_else(|| {
                    panic!(
                        "AI stop_target {} did not resolve to a human actor",
                        target_id.index()
                    )
                });
            if should_stop.0 && should_stop.1 {
                self.stop_owner(target_id, crate::sequence::SequencePriority::Normal);
            }
        }

        // The near-enemy EventView path calls
        // entering attacking / reaction-time state before battle decisions reach
        // swordfight entry. The state change synchronously registers
        // ENTER_ATTENTIVE_MODE; swordfight entry registers
        // ENTER_SWORDFIGHT afterward, so the attentive lean-forward element
        // is authoritative and the fight waits behind it. Rust batches both
        // effects in one outbox; preserve that authored order instead of
        // draining the core swordfight channel first.
        let attentive_requests = {
            self.world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_controller_mut)
                .map(|base| base.outbox.actor.take_attentive_modes())
                .unwrap_or_default()
        };
        for request in attentive_requests {
            self.set_soldier_attentive_mode_from(
                npc_id,
                request.target,
                request.fast_officer_variant,
                crate::engine::soldier_helpers::AttentiveModeCaller::AiOwnerEffect,
            );
            if request.forget_after
                && let Some(enemy) = self
                    .world
                    .entities
                    .get_mut(npc_id)
                    .and_then(Entity::enemy_ai_mut)
            {
                // The original game's state change has now launched attentive mode
                // its transition and written will-be-attentive. The special
                // event handler's subsequent attentive-mode exit wins the
                // final flag state without clearing forced-attentive.
                enemy.attentive = false;
                enemy.will_be_attentive = false;
            }
        }

        // Process enter_swordfight.  Two shapes:
        //   * Engage(target) — engagement against a specific opponent.
        //     The original game launches swordfight entry when combat begins; it
        //     does not enter swordfight directly during AI processing. Keep
        //     relationship and animation changes behind that owner boundary.
        //   * Direct(target) — EVENT_GOTHIT's direct swordfight entry.
        //     This immediately updates both opponent lists and, when the
        //     attacker is not already swordfighting, authors the reciprocal
        //     ENTER_SWORDFIGHT element with the attacker as owner.
        //   * Rebalance(target) — swordfight reconsideration's direct
        //     enter-swordfight action. This updates the
        //     relationship without authoring a recursive command/EventDone.
        //   * RaiseSword — sword pose without engagement, launched as its
        //     own sequence. `AttackingApproachToObserve` and
        //     menace-effect-of-hit need a sword pose held without an
        //     active fight. `go_to`'s `GOTO_SWORD` arm does NOT come
        //     through here: Original inserts its raise-sword element into
        //     the movement's own sequence, so it travels on the movement
        //     intent as `enter_swordfight_before_move` instead.
        if let Some(request) = effects.enter_swordfight {
            match request {
                crate::ai::EnterSwordfightRequest::RaiseSword => {
                    let mut elem = crate::sequence::SequenceElement::new_generic(
                        1,
                        crate::element::Command::EnterSwordfight,
                        Some(npc_id),
                    );
                    // The original-game AI explicitly clears the opponent field
                    // for the raise-sword-only form.  Preserve that
                    // distinction from a malformed element which omitted the
                    // required property altogether.
                    elem.set_property(
                        crate::sequence::Field::Opponent,
                        crate::sequence::FieldValue::Integer(0),
                    );
                    elem.set_property(
                        crate::sequence::Field::JumplineDestination,
                        crate::sequence::FieldValue::Integer(0),
                    );
                    self.launch_element(elem);
                }
                crate::ai::EnterSwordfightRequest::Engage(target_handle) => {
                    let target_id = self.expect_human_id_for_ai_handle(
                        target_handle.get(),
                        "AI enter_swordfight target",
                    );
                    let mut elem = crate::sequence::SequenceElement::new_generic(
                        1,
                        crate::element::Command::EnterSwordfight,
                        Some(npc_id),
                    );
                    elem.set_property(
                        crate::sequence::Field::Opponent,
                        crate::sequence::FieldValue::Element(target_id),
                    );
                    if let Some(jump_line) = effects
                        .enter_swordfight_jump_line
                        .and_then(crate::jump_line::JumpLineIndex::new)
                    {
                        elem.set_property(
                            crate::sequence::Field::JumplineDestination,
                            crate::sequence::FieldValue::LineId(jump_line),
                        );
                    } else {
                        elem.set_property(
                            crate::sequence::Field::JumplineDestination,
                            crate::sequence::FieldValue::Integer(0),
                        );
                    }
                    // Original-game swordfight entry performs
                    // stop-all before registering ENTER_SWORDFIGHT.
                    // `Stop(PREFERENCE)` does not itself run the selected
                    // movement's condolence callback, so the sprite retains
                    // its last movement goal while the sword transition takes
                    // ownership.
                    self.launch_element(elem);
                }
                crate::ai::EnterSwordfightRequest::Direct(target_handle) => {
                    let target_id = self.expect_human_id_for_ai_handle(
                        target_handle.get(),
                        "AI direct swordfight target",
                    );
                    self.direct_enter_swordfight(sim, assets, npc_id, target_id);
                }
                crate::ai::EnterSwordfightRequest::Rebalance(target_handle) => {
                    let target_id = self.expect_human_id_for_ai_handle(
                        target_handle.get(),
                        "AI reconsider swordfight target",
                    );
                    if self.direct_enter_swordfight(sim, assets, npc_id, target_id) {
                        self.world
                            .entities
                            .get_mut(npc_id)
                            .expect("successful AI swordfight rebalance owner disappeared")
                            .enemy_ai_mut()
                            .expect("successful AI swordfight rebalance owner lost enemy AI")
                            .base
                            .primary_target = Some(target_handle);
                    }
                }
            }
        }
    }

    /// Principal opponent, bow shot, focus, eye reopening, and instant direction.
    fn drain_pending_focus_and_orders(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
        drain: &mut PendingDrainBarrier,
    ) {
        let PendingDrainBarrier { effects, .. } = drain;

        // Process set_as_new_principal_opponent.
        if let Some(opponent_handle) = effects.set_principal {
            let opponent_id =
                self.expect_human_id_for_ai_handle(opponent_handle.get(), "AI principal opponent");
            self.set_as_new_principal_opponent(assets, npc_id, opponent_id);
        }

        // Process pending focus / focus_point / unfocus — the
        // focus by primary target, position, or no target
        // calls.  Each explicit channel "consumes" the primary_target
        // edge by stamping `last_synced_focus_target = primary_target`,
        // so `refresh_npc_views` sees no edge and does not auto-revert
        // the explicit focus state next tick.  This is what makes
        // patterns like rider-charge passing (clearing focus while
        // `primary_target` stays set) and `battle_decisions` entry
        // honour the synchronous ordering even though the channel
        // itself is deferred.
        let mut focus_channel_fired = false;
        if let Some(target_handle) = effects.focus {
            // Original-game NPC focus accepts an arbitrary
            // any element. Object-handling AI legitimately focuses bonuses
            // such as ale bottles, so preserve the element kind while still
            // treating a missing raw slot as corrupted state.
            let target_id = self.expect_entity_id_for_index(target_handle.get(), "AI focus target");
            let npc = self.world.entities.expect_ai_actor_data_mut(
                npc_id,
                format_args!("pending-drain owner {} lost AI data", npc_id.index()),
            );
            crate::ai_vision::focus_entity(npc, target_id);
            focus_channel_fired = true;
        }

        if let Some(point) = effects.focus_point {
            // The original game's position focus first calls
            // world-point conversion without elevation adjustment and stores that point's
            // world X/Y in `starePoint`.
            let point_3d =
                self.position_to_point_3d(assets, point.sector, point.level, point.x, point.y);
            let npc = self.world.entities.expect_ai_actor_data_mut(
                npc_id,
                format_args!("pending-drain owner {} lost AI data", npc_id.index()),
            );
            crate::ai_vision::focus_point(
                npc,
                crate::coordinates::GroundPoint::new(point_3d.x, point_3d.y),
            );
            focus_channel_fired = true;
        }

        if effects.unfocus {
            let npc = self.world.entities.expect_ai_actor_data_mut(
                npc_id,
                format_args!("pending-drain owner {} lost AI data", npc_id.index()),
            );
            crate::ai_vision::unfocus(npc);
            focus_channel_fired = true;
        }

        if focus_channel_fired {
            let ai = self.world.entities.expect_ai_controller_mut(
                npc_id,
                format_args!("pending-drain owner {} lost AI after focus", npc_id.index()),
            );
            ai.last_synced_focus_target = ai.primary_target;
        }

        // Process pending gradual eye reopening — `slowly_open_eyes` sets
        // `view_radius = 5`, points `view_radius_goal` at the engine's
        // standard view radius, switches `eye_status` to
        // `ViewconeGrow`, and marks `view_transition`.  The
        // `ViewconeGrow` branch of `refresh_view` then ramps the cone
        // back open at 8 units/frame.
        if effects.slowly_open_eyes {
            let standard = self.ai.standard_view_polygon_radius;
            let npc = self.world.entities.expect_ai_actor_data_mut(
                npc_id,
                format_args!("pending-drain owner {} lost AI data", npc_id.index()),
            );
            npc.view_transition = true;
            npc.view_radius = 5;
            npc.view_radius_base = 5;
            npc.view_radius_goal = standard;
            npc.eye_status = crate::element::EyeStatus::ViewconeGrow;
        }

        // Process pending set_direction_instantly.
        if let Some(dir) = effects.set_direction_instantly
            && let Some(entity) = self.world.entities.get_mut(npc_id)
        {
            entity.position_iface_mut().set_direction_instantly(
                crate::position_interface::Direction::from_raw(dir as i32),
            );
        }
    }

    /// Guarded-PC reciprocity, deactivation, reported-to-officer writes,
    /// bow-ammo refill and archery-reservation release.
    fn drain_pending_guard_and_archery(
        &mut self,
        npc_id: crate::element::EntityId,
        drain: &PendingDrainBarrier,
    ) {
        let PendingDrainBarrier { effects, .. } = drain;

        // Process pending guarded-PC assignment — `set_guarded_pc`. The AI
        // wrote its own `guarded_pc` field already; here we flip the
        // reciprocal `pc.guard` on the old and new target PCs.
        let guard_delta = self
            .world
            .entities
            .expect_ai_controller_mut(
                npc_id,
                format_args!("guard-delta owner {} lost its AI", npc_id.index()),
            )
            .outbox
            .actor
            .set_guarded_pc
            .take();
        if let Some(guard_delta) = guard_delta {
            // Clear `pc.guard` on the old target
            // by clearing the guarded PC's guard.
            if let Some(old_pc) = guard_delta.old {
                let old_pc_id = EntityId::Pc(old_pc);
                match self.world.entities.get_mut(old_pc_id) {
                    Some(Entity::Pc(pc)) => pc.pc.guard = None,
                    Some(entity) => tracing::warn!(
                        npc = ?npc_id,
                        target = ?old_pc_id,
                        actual_kind = ?entity.kind(),
                        "guarded-PC clear target has the wrong entity kind"
                    ),
                    None => tracing::warn!(
                        npc = ?npc_id,
                        target = ?old_pc_id,
                        "guarded-PC clear target does not exist"
                    ),
                }
            }
            // Set `pc.guard` on the new target
            // (`guarded_pc.set_guard(self)`).  Asserts `is_in_coma()`
            // on the PC; the only caller already gates on the coma
            // check in the `AttackingApproachingSleepingEnemy`
            // handler, so skip the redundant debug_assert here.
            if let Some(new_pc) = guard_delta.new {
                let new_pc_id = EntityId::Pc(new_pc);
                match self.world.entities.get_mut(new_pc_id) {
                    Some(Entity::Pc(pc)) => pc.pc.guard = Some(npc_id),
                    Some(entity) => tracing::warn!(
                        npc = ?npc_id,
                        target = ?new_pc_id,
                        actual_kind = ?entity.kind(),
                        "guarded-PC set target has the wrong entity kind"
                    ),
                    None => tracing::warn!(
                        npc = ?npc_id,
                        target = ?new_pc_id,
                        "guarded-PC set target does not exist"
                    ),
                }
            }
        }

        // Process pending entity deactivation (merry man leaving map).
        // Equivalent to `set_active(false)`.
        if effects.deactivate
            && let Some(entity) = self.world.entities.get_mut(npc_id)
        {
            entity.element_data_mut().active = false;
            tracing::debug!(
                npc = npc_id.index(),
                "Deactivated entity (merry man left map)"
            );
        }

        // Process pending `set_reported_to_officer(flag)` — the
        // `charly.set_reported_to_officer(false)` call inside
        // `missed_charly_alert`.  Writes the other NPC's
        // `EnemyAi::reported_to_officer` flag.
        let reported_updates = std::mem::take(
            &mut self
                .world
                .entities
                .expect_ai_controller_mut(npc_id, format_args!("report-update owner"))
                .outbox
                .actor
                .set_reported_to_officer,
        );
        for (target_handle, value) in reported_updates {
            let target_id = self.expect_human_id_for_ai_handle(
                target_handle.get(),
                "set-reported-to-officer target",
            );
            self.world
                .entities
                .expect_enemy_ai_mut(
                    target_id,
                    format_args!(
                        "set-reported-to-officer target human {target_handle} has no EnemyAi"
                    ),
                )
                .reported_to_officer = value;
        }
    }

    /// Nearby-searcher stand-down (`CALL_CHARLY_IS_BACK` delivery).

    /// Sequence/element launches, cross-actor speech, shield refresh,
    /// sideways looks, and beggar detectable stripping.
    fn drain_pending_launches(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
        drain: &mut PendingDrainBarrier,
    ) {
        let PendingDrainBarrier { effects, .. } = drain;

        // Process pending launch commands — create and launch
        // sequence elements for commands the AI wants to execute.
        for cmd in std::mem::take(&mut effects.launch_commands) {
            let elem = crate::sequence::SequenceElement::new(1, cmd, Some(npc_id));
            let mut sequence = crate::sequence::Sequence::new();
            sequence.append_element(elem);
            // AI helpers launch sequence elements, which registers an
            // ordinary owned command with SequenceManager. It does not run
            // the actor's instruction/arbitration synchronously at the AI call site.
            self.launch_sequence(sequence);
        }

        // Full sequences the AI wants to launch verbatim — the
        // `launch_sequence(SEQ_INFO, sequence)` calls inside AI
        // handlers (e.g. the officer's turn/gather/point alert
        // sequence). Original-game sequence launch calls
        // sequence registration while the AI handler is still on the
        // stack, and immediate execution dispatches engine commands inline.
        // Close that exact boundary for each sequence: batching the drain
        // until after the NPC tail lets a Timer/LockUser successor escape the
        // owner's legacy update slot.
        for seq in std::mem::take(&mut effects.launch_sequences) {
            self.launch_sequence(seq);
            self.drain_script_synchronous_actions(sim, assets, &mut Vec::new())
                .unwrap_or_else(|error| {
                    panic!(
                        "AI owner {} failed to drain a synchronously launched sequence: {error:?}",
                        npc_id.index()
                    )
                });
        }

        if effects.refresh_shield {
            self.refresh_retained_shield_obstacle(assets, npc_id);
        }

        // Process pending sideways looks — build a one- or two-element
        // sequence of LookLeft / LookRight / LeanOut commands and
        // launch it.
        if let Some(dir) = effects.look_sidewards {
            use crate::ai::LookDirection;
            use crate::element::Command;
            let cmds: &[Command] = match dir {
                LookDirection::Left => &[Command::LookLeft],
                LookDirection::Right => &[Command::LookRight],
                LookDirection::LeftRight => &[Command::LookLeft, Command::LookRight],
                LookDirection::RightLeft => &[Command::LookRight, Command::LookLeft],
                LookDirection::Down => &[Command::LeanOut],
            };
            tracing::trace!(
                npc = npc_id.index(),
                ?dir,
                ?cmds,
                "launching look-sidewards sequence"
            );
            // looking sideways clears focus before allocating
            // the sequence so the soldier's gaze drops its lock for
            // the head-turn animation.  Centralise it here instead of
            // patching every caller.
            let npc = self.world.entities.expect_ai_actor_data_mut(
                npc_id,
                format_args!("pending-drain owner {} lost AI actor data", npc_id.index()),
            );
            crate::ai_vision::unfocus(npc);
            let mut seq = crate::sequence::Sequence::new();
            for (i, cmd) in cmds.iter().enumerate() {
                let elem =
                    crate::sequence::SequenceElement::new((i as u16) + 1, *cmd, Some(npc_id));
                seq.append_element(elem);
            }
            self.launch_sequence(seq);
        }

        // Process pending "strip beggar from every NPC" requests:
        //   delete_detectable_for_all_npc(stimulus.human, BEGGAR);
        // Fired from the `EventSeesBeggar` handler in `ai_enemy.rs`
        // once a seek-area soldier has claimed the PC-beggar via
        // `beggars_to_control`, so every other soldier's BEGGAR list
        // drops the PC and stops firing duplicate `EventSeesBeggar`
        // stimuli on subsequent frames.
        let delete_beggar_requests: Vec<EntityId> = {
            let ai_actor = self
                .world
                .entities
                .expect_ai_actor_data_mut(npc_id, format_args!("pending-drain AI owner"));
            match ai_actor.ai_brain.base_mut() {
                Some(ai) => std::mem::take(&mut ai.outbox.actor.delete_beggar_for_all_npc),
                None => Vec::new(),
            }
        };
        for beggar_id in delete_beggar_requests {
            self.delete_beggar_detectable_for_all_npc(beggar_id);
        }
    }

    /// Detectable-list mutations with their DETMUT / consider-report traces.
    fn drain_pending_detectable_mutations(
        &mut self,
        npc_id: crate::element::EntityId,
        drain: &PendingDrainBarrier,
    ) {
        let PendingDrainBarrier { effects, .. } = drain;

        // Apply detectable mutations in statement order without introducing a
        // new owner/reentrant barrier. Classification is read before borrowing
        // the owner mutably; these operations only modify its detectable lists.
        if !effects.detectable_mutations.is_empty() {
            use crate::ai::DetectableMutation;
            use crate::element::DetectableType;
            if crate::ai::consider_report_debug_matches(self.control.frame_counter, npc_id.index())
            {
                Self::trace_consider_report_drain_start(
                    self.control.frame_counter,
                    npc_id,
                    &effects.detectable_mutations,
                );
            }
            let mutation_debug_enabled = detection::detectable_mutation_debug_enabled();
            let mutation_owner_creation_order = if mutation_debug_enabled
                && detection::detectable_mutation_debug_owner_slot_matches(npc_id.index())
            {
                self.original_static_creation_order(npc_id)
            } else {
                0
            };
            let mutation_targets = if mutation_debug_enabled
                && detection::detectable_mutation_debug_owner_matches(
                    npc_id.index(),
                    mutation_owner_creation_order,
                ) {
                let target_ids = self
                    .world
                    .entities
                    .get(npc_id)
                    .and_then(Entity::ai_actor_data)
                    .expect("DETMUT pending-effect owner lost AI actor data")
                    .detectable_lists
                    .iter()
                    .flatten()
                    .filter_map(|detectable| detectable.element)
                    .chain(
                        effects
                            .detectable_mutations
                            .iter()
                            .filter_map(|mutation| mutation.target()),
                    )
                    .collect::<std::collections::BTreeSet<_>>();
                target_ids
                    .into_iter()
                    .filter_map(|target_id| {
                        if !detection::detectable_mutation_debug_target_slot_matches(
                            target_id.index(),
                        ) {
                            return None;
                        }
                        let creation_order = self.original_static_creation_order(target_id);
                        detection::detectable_mutation_debug_target_matches(
                            target_id.index(),
                            creation_order,
                        )
                        .then_some((target_id, creation_order))
                    })
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let enemy_target_info = effects
                .detectable_mutations
                .iter()
                .map(|mutation| {
                    let DetectableMutation::Add(target_id, DetectableType::Enemy) = *mutation
                    else {
                        return None;
                    };
                    let target = self.get_entity(target_id).unwrap_or_else(|| {
                        panic!(
                            "pending-drain owner {} detectable target {} disappeared",
                            npc_id.index(),
                            target_id.index()
                        )
                    });
                    Some((
                        target.is_pc(),
                        target.is_soldier(),
                        target.camp(),
                        target.is_human(),
                    ))
                })
                .collect::<Vec<_>>();
            let (npc_camp, npc_uses_enemy_combat_ai) = {
                let owner = self.expect_entity(npc_id, "pending-drain owner");
                (owner.camp(), owner.enemy_ai().is_some())
            };
            let npc = self.world.entities.expect_ai_actor_data_mut(
                npc_id,
                format_args!("pending-drain owner {} lost AI actor data", npc_id.index()),
            );
            for (mutation, target_info) in
                effects.detectable_mutations.iter().zip(enemy_target_info)
            {
                let kind = mutation.detectable_type();
                let idx = kind as usize;
                assert!(
                    idx < npc.detectable_lists.len(),
                    "pending-drain owner {} has no {:?} detectable list",
                    npc_id.index(),
                    kind
                );
                if matches!(mutation, DetectableMutation::Add(_, DetectableType::Enemy)) {
                    let (pc, soldier, camp, human) = target_info
                        .expect("Enemy add target was classified before borrowing the owner");
                    if !human
                        || !crate::ai_detectable_filter::should_add_enemy_detectable_with(
                            &self.mission_domain.diplomacy,
                            npc_camp,
                            npc_uses_enemy_combat_ai,
                            pc,
                            soldier,
                            camp,
                        )
                    {
                        continue;
                    }
                }
                let before = npc.detectable_lists[idx].len();
                let tracked = mutation_targets
                    .iter()
                    .filter(|(target, _)| {
                        mutation.target().is_none_or(|selected| selected == *target)
                    })
                    .map(|(target, creation_order)| {
                        (
                            *target,
                            *creation_order,
                            npc.detectable_lists[idx]
                                .iter()
                                .any(|entry| entry.element == Some(*target)),
                        )
                    })
                    .collect::<Vec<_>>();
                let (event, source) = match *mutation {
                    DetectableMutation::Add(target, _) => {
                        append_detectable(&mut npc.detectable_lists[idx], target, kind, false);
                        ("add", "pending_effects.add_detectables")
                    }
                    DetectableMutation::Append(target, _) => {
                        append_detectable(&mut npc.detectable_lists[idx], target, kind, true);
                        ("append", "pending_effects.append_detectables")
                    }
                    DetectableMutation::DeleteType(_) => {
                        npc.detectable_lists[idx].clear();
                        ("delete_all", "pending_effects.delete_detectables")
                    }
                    DetectableMutation::DeleteEntity(target, _) => {
                        npc.delete_detectable(target, kind);
                        ("delete", "pending_effects.delete_detectable_entities")
                    }
                };
                for (target, creation_order, present_before) in tracked {
                    if matches!(mutation, DetectableMutation::DeleteType(_)) && !present_before {
                        continue;
                    }
                    let present_after = npc.detectable_lists[idx]
                        .iter()
                        .any(|entry| entry.element == Some(target));
                    detection::debug_detectable_mutation_event(
                        event,
                        source,
                        self.control.frame_counter,
                        npc_id.index(),
                        mutation_owner_creation_order,
                        idx,
                        target.index(),
                        creation_order,
                        present_before,
                        present_after,
                        before,
                        npc.detectable_lists[idx].len(),
                    );
                }
            }
            if crate::ai::consider_report_debug_matches(self.control.frame_counter, npc_id.index())
            {
                Self::trace_consider_report_drain_end(
                    self.control.frame_counter,
                    npc_id,
                    &npc.detectable_lists[DetectableType::Body as usize],
                );
            }
        }
    }

    /// Posture change, enemy blink reset, and the
    /// indoor enemy alert.
    fn drain_pending_coins_posture_and_alerts(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
        drain: &PendingDrainBarrier,
    ) {
        let PendingDrainBarrier { effects, .. } = drain;

        // Process pending posture-change request. Like the
        // `set_posture(Sitting/Leisure)` calls in the reference.
        // The move-box recomputation in
        // `PositionInterface::set_posture` is skipped here because
        // the engine stores posture on the element-data struct and
        // the move box is reshaped lazily elsewhere — this matches
        // every other posture write in the codebase (e.g.
        // `abilities.rs` `CarryingCorpse`, `melee.rs` knock-out
        // paths).
        if let Some(p) = effects.posture
            && let Some(entity) = self.world.entities.get_mut(npc_id)
        {
            entity.set_posture(p);
        }

        // Process the pending request to clear the blinking enemy — clear the
        // seen_now / seen_last_frame flags on every enemy detectable
        // so the next detection pass treats anyone still in the cone
        // as a "first-seen" edge and re-issues EVENT_VIEW.
        let blink_all = {
            let ai = self
                .world
                .entities
                .expect_ai_controller_mut(npc_id, format_args!("pending-drain NPC"));
            std::mem::take(&mut ai.outbox.actor.blink_all_enemies)
        };
        if blink_all {
            // Enemy blinking belongs to NPC actors, not the soldier
            // subclass. ScriptGoOn therefore reaches this path for both
            // soldiers and civilians.
            let npc = self.world.entities.expect_ai_actor_data_mut(
                npc_id,
                format_args!("pending-drain owner {} lost AI actor data", npc_id.index()),
            );
            let idx = crate::element::DetectableType::Enemy as usize;
            let list = npc.detectable_lists.get_mut(idx).unwrap_or_else(|| {
                panic!(
                    "pending-drain owner {} has no enemy detectable list",
                    npc_id.index()
                )
            });
            for det in list.iter_mut() {
                det.seen_now = false;
                det.seen_last_frame = false;
            }
        }
        // Process the pending indoor enemy alert.
        //
        // Orchestrator walks the building's occupant list, sorts by
        // camp, dispatches `panic()` to civilians, and calls
        // `init_battle_before_door` on the outnumbered side.  Both
        // the panic side-effect and the door-battle orchestration
        // (`init_battle_before_door` + `send_before_door_to_fight`
        // in `engine/soldier_helpers.rs`) are wired below.
        let in_house_alert = {
            let ai = self.world.entities.expect_ai_controller_mut(
                npc_id,
                format_args!("pending-drain owner {} lost its AI", npc_id.index()),
            );
            std::mem::take(&mut ai.outbox.actor.enemy_in_house_alert)
        };
        if in_house_alert {
            self.dispatch_enemy_in_house_alert(sim, npc_id, assets);
        }
    }

    /// Complete pending panic and script-driven area searches.
    fn drain_pending_panic_and_search(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) {
        // Drain any pending panic request from the enemy AI — the
        // analogue of the civilian-side drain that runs inside
        // `nearby_civilians_panic`.  Without this, an EnemyAi that
        // pushes a `PanicRequest` (e.g. from the fleeing arm of
        // `think_alerting_event(sim, EVENT_VIEW)` outdoors) stays wedged
        // in `FleeingPanic` with no door picked.
        let has_begin_panic = self
            .world
            .entities
            .get(npc_id)
            .and_then(Entity::ai_controller)
            .is_some_and(|ai| ai.outbox.actor.begin_panic.is_some());
        if has_begin_panic {
            self.process_pending_begin_panic_for(sim, assets, npc_id);
        }

        // Drain any pending script-driven area-search request. Matches
        // the immediate `start_think(NO_EVENT); seek_area(sim, ...);
        // end_think(sim, )` block inside `set_ai_state(STATE_SEEKING)`.
        //
        // Only pay the surrounding battle-context cost when the
        // request exists. Keep the cheap pre-check here so the common
        // drain pass does not rebuild full per-NPC tick data for
        // every soldier just to discover
        // `pending_script_seek_area == None`.
        let has_script_seek = self
            .world
            .entities
            .get(npc_id)
            .and_then(|entity| entity.ai_controller())
            .is_some_and(|ai| ai.outbox.actor.script_seek_area.is_some());
        if has_script_seek {
            self.process_pending_script_seek_area_for(sim, assets, npc_id);
        }
    }
}

/// Locals taken at the first post-Think barrier of
/// [`EngineInner::drain_pending_for_npc`] that its later phases
/// consume. Transient per-drain state (no serde: the effect channels it holds
/// are runtime-only and never persisted).
struct PendingDrainBarrier {
    preemption: crate::ai::AiActorPreemptionEffects,
    effects: crate::ai::AiActorCoreEffects,
}
