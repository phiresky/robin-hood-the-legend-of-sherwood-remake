use super::*;

impl EngineInner {
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
        self.tick_enemy_ai_inner(sim, assets, None);
    }

    /// Production NPC coordinator for the pre-detection portion of
    /// `RHElementActorNPC::Hourglass`.
    ///
    /// Each NPC consumes only its own body/recovery work and refreshes its
    /// own view immediately before its creation-ordered `RefreshDetection`.
    /// The direct `tick_enemy_ai` entry point remains detection-only for
    /// focused tests that construct already-refreshed vision state.
    #[cfg(test)]
    pub(in crate::engine) fn tick_enemy_ai_with_creation_ordered_prelude(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
    ) {
        self.tick_enemy_ai_inner(sim, assets, Some(positions_before_movement));
    }

    /// Prepare the shared, RNG-free portion of the fused owner pass.
    pub(in crate::engine) fn prepare_npc_owner_pass(
        &mut self,
        _sim: &crate::sim_rng::SimulationContext,
        _assets: &LevelAssets,
    ) -> PreparedNpcOwnerPass {
        self.ai.global.same_frame_target_claims.clear();
        if !self.ai.global.primary_target_multiplicity_initialized {
            let initial = self.tick_enemy_ai_build_primary_target_multiplicity();
            self.ai.global.primary_target_multiplicity_scratch = initial
                .into_iter()
                .map(|(target, count)| (target.index(), count))
                .collect();
            self.ai.global.primary_target_multiplicity_initialized = true;
        }
        PreparedNpcOwnerPass {
            world: None,
            entity_views: PreparedAiEntityViewCache::default(),
        }
    }

    /// Run one NPC's complete post-human envelope using live inputs sampled at
    /// this legacy slot. No later owner's view or forecast is constructed.
    pub(in crate::engine) fn tick_npc_owner_pass(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
        prepared: &mut PreparedNpcOwnerPass,
        npc_id: EntityId,
    ) {
        self.debug_refresh_view_lifecycle("npc_tail_enter", npc_id, None);
        let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
            panic!(
                "NPC owner {} disappeared before its fused legacy-slot envelope",
                npc_id.index()
            )
        });
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

        if prepared.world.is_none() {
            prepared.world =
                Some(self.tick_enemy_ai_build_world_view(
                    assets,
                    Some((npc_id, positions_before_movement)),
                ));
        }
        let world = prepared
            .world
            .as_ref()
            .expect("prepared NPC owner pass lost its tactical world view");
        self.tick_enemy_ai_refresh_detection(
            sim,
            assets,
            world,
            Some(positions_before_movement),
            Some(npc_id),
            false,
            Some(&mut prepared.entity_views),
        );
    }

    pub(in crate::engine) fn finish_npc_owner_pass(&mut self) {
        self.ai.global.same_frame_target_claims.clear();
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
        positions_before_movement: Option<&EntitySlots<Option<crate::entities::BoundaryPosition>>>,
    ) {
        if self.actors_frozen() {
            // Frozen-all skips patrol/view/detection/ambush/deafness but the
            // original still enters each NPC's busy/ladder/speech/lock gate,
            // where all three deadlines are extended before returning.
            if positions_before_movement.is_some() {
                let npc_ids: Vec<_> = self.world.entities.ai_owner_ids().collect();
                for npc_id in npc_ids {
                    self.tick_npc_post_detection_tail_for_npc(sim, npc_id, assets);
                }
            }
            return;
        }
        self.ai.global.same_frame_target_claims.clear();
        if !self.ai.global.primary_target_multiplicity_initialized {
            let initial = self.tick_enemy_ai_build_primary_target_multiplicity();
            self.ai.global.primary_target_multiplicity_scratch = initial
                .into_iter()
                .map(|(target, count)| (target.index(), count))
                .collect();
            self.ai.global.primary_target_multiplicity_initialized = true;
        }

        // ── 1. Build one immutable per-tick AI world view. ────────
        // Snapshot construction does not dispatch behavior. The phase calls
        // below remain in the original soldier/NPC Hourglass order.
        let world = self.tick_enemy_ai_build_world_view(assets, None);

        // ── 2a. Listen/object blip work. ────────────────────────
        // NPC-owned SeesBlip remains inside its creation-ordered
        // RefreshDetection slot below.
        let pc_ids = self.world.pc_ids.clone();
        for pc_id in pc_ids {
            self.tick_enemy_ai_blip_detection(sim, assets, pc_id);
        }

        // ── 3. Creation-ordered per-NPC prelude + RefreshDetection. ───
        // Production first consumes the current NPC's inform/recovery outbox
        // and refreshes its view. Acoustic detection + synchronous EVENT_HEAR,
        // Enemy detection, volatile target rebuild, non-Enemy detectable
        // buckets, and the resulting FIFO Think dispatches then all finish for
        // that NPC before the next creation slot starts.
        self.tick_enemy_ai_refresh_detection(
            sim,
            assets,
            &world,
            positions_before_movement,
            None,
            positions_before_movement.is_some(),
            None,
        );

        // Focused detection tests retain the legacy fallback drains for
        // stimuli injected outside the production owner coordinator. Every
        // production Think now drains its own effects/stimuli synchronously;
        // re-running 6c/6d globally here would re-batch owner work.
        #[cfg(test)]
        if positions_before_movement.is_none() {
            self.tick_enemy_ai_drain_swordfight_requests(sim, assets);
            self.tick_enemy_ai_drain_pending_stimuli(sim, assets);
        }
        self.ai.global.same_frame_target_claims.clear();

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
    #[cfg(test)]
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    pub(in crate::engine) fn drain_pending_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) {
        self.drain_pending_for_npc_mode(sim, npc_id, assets, false, false);
    }

    pub(in crate::engine) fn drain_pending_for_npc_mode(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
        owner_local_no_forecast: bool,
        defer_turn_instruction: bool,
    ) {
        self.drain_pending_for_npc_boundary_mode(
            sim,
            npc_id,
            assets,
            owner_local_no_forecast,
            defer_turn_instruction,
            true,
        );
    }

    pub(in crate::engine) fn drain_pending_for_npc_boundary_mode(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
        owner_local_no_forecast: bool,
        defer_turn_instruction: bool,
        surface_completion: bool,
    ) {
        // Direct engine-owned AI calls also enter this drain. Close the
        // SetState callback boundary before consuming halt/effect/order work.
        self.drain_ai_owner_work_for_boundary_mode(
            sim,
            assets,
            npc_id,
            owner_local_no_forecast,
            defer_turn_instruction,
            surface_completion,
        );
        self.drain_patrol_direction_broadcast_for(sim, npc_id, assets);

        // Direct SetDirection calls made before StopAll must update the goal
        // before the halt/transition barrier. They do not create a standalone
        // Turn element; the subsequently selected action performs the turn.
        let direction_goal = {
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                return;
            };
            let Some(ai) = entity.ai_controller_mut() else {
                return;
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
                return;
            };
            let Some(ai) = entity.ai_controller_mut() else {
                return;
            };
            ai.outbox.actor.take_halt_count()
        };
        if halt_count != 0 {
            // `StopAll` calls `Stop(PREFERENCE)` synchronously before the
            // handler continues into SetState/SetAttentiveMode and other
            // replacement work. Deliver the halt condolence at that same
            // barrier: actor-base cleanup must clear a selected movement's
            // cached goal before a newly launched attentive transition can
            // become the selected element. `from_halt` suppresses the NPC
            // EventDone/Impossible callbacks while retaining that base
            // selected-element cleanup.
            for _ in 0..halt_count {
                self.halt_actor(npc_id);
                self.dispatch_condolations_for_npc(sim, npc_id, assets);
            }
        }

        // The halt application above is a real same-frame barrier: only now
        // take the prefixes that the original `go_to` path launches next.
        let preemption = {
            let entity = self
                .world
                .entities
                .get_mut(npc_id)
                .unwrap_or_else(|| panic!("pending-drain NPC {} disappeared", npc_id.index()));
            let ai = entity.ai_controller_mut().unwrap_or_else(|| {
                panic!("pending-drain NPC {} has no AI controller", npc_id.index())
            });
            ai.outbox.actor.take_movement_prefixes()
        };

        // Take exactly the channels read at the first post-Think barrier.
        // Later barrier groups remain live so re-entrant sequence work can
        // still enqueue effects that this pass observes at their Original
        // application point.
        let (effects, finish_lost_enemy_overview) = {
            let entity = self
                .world
                .entities
                .get_mut(npc_id)
                .unwrap_or_else(|| panic!("pending-drain NPC {} disappeared", npc_id.index()));
            let ai = entity.ai_controller_mut().unwrap_or_else(|| {
                panic!("pending-drain NPC {} has no AI controller", npc_id.index())
            });
            (
                ai.outbox.actor.take_core(),
                ai.outbox.actor.take_lost_enemy_overview_after_quit(),
            )
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

        // EndSwordfight launches an explicit QUIT_SWORDFIGHT element.  Do
        // not tear down the relationship directly here: the command owns
        // both that teardown and the visible lowering-sword transition, and
        // LaunchSequenceElement arbitrates it synchronously in the Original.
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
            // LaunchSequenceElement reaches Instruct synchronously. If the
            // quit replaces a selected command, its SendCondolationCard
            // callback therefore re-enters Think before EndSwordfight
            // returns to its caller.
            self.dispatch_condolations_for_npc(sim, npc_id, assets);
        }

        // Process stop_menace — the explicit `STOP_MENACE` element
        // prepend in `go_to`.  Launching a `Command::StopMenace`
        // element here lets the per-element dispatch in `tick.rs`
        // queue `TRANSITION_MENACING_WAITING_SWORD` then
        // `TRANSITION_LOWERING_SWORD` before the move that
        // `launch_pending_orders_for_npc` is about to launch starts.
        if preemption.stop_menace {
            let elem = crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::StopMenace,
                Some(npc_id),
            );
            self.launch_element(elem);
        }

        // Process lower_shield — the explicit `LOWER_SHIELD` element
        // prepend in `go_to`.  Launching a `Command::LowerShield`
        // element here lets `dispatch_lower_shield` queue the
        // `LoweringShield` order so the shield arm completes before
        // `launch_pending_orders_for_npc` runs the move.
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
            let target_id = self.expect_human_id_for_ai_handle(target_handle, "AI stop_target");
            // Original BeginSwordfight queries these members at this exact
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
        // `SetState(ATTACKING, REACTIONTIME)` before BattleDecisions reaches
        // BeginSwordfight. SetState synchronously registers
        // ENTER_ATTENTIVE_MODE; BeginSwordfight registers
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
                // Original SetState's SetAttentiveMode call has now launched
                // its transition and written will-be-attentive. The special
                // event handler's following ForgetAttentiveMode call wins the
                // final flag state without clearing forced-attentive.
                enemy.attentive = false;
                enemy.will_be_attentive = false;
            }
        }

        // Process enter_swordfight.  Two shapes:
        //   * Engage(target) — engagement against a specific opponent.
        //     Original `BeginSwordfight` launches ENTER_SWORDFIGHT; it
        //     does not call `EnterSwordFight` directly from Think.  Keep
        //     relationship and animation changes behind that owner boundary.
        //   * Direct(target) — EVENT_GOTHIT's direct EnterSwordFight call.
        //     This immediately updates both opponent lists and, when the
        //     attacker is not already swordfighting, authors the reciprocal
        //     ENTER_SWORDFIGHT element with the attacker as owner.
        //   * Rebalance(target) — ReconsiderSwordfight's direct
        //     `RHElementActorHuman::EnterSwordFight` call. This updates the
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
                    // Original AI explicitly stores null in RHFIELD_OPPONENT
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
                    let target_id = self
                        .expect_human_id_for_ai_handle(target_handle, "AI enter_swordfight target");
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
                    // `RHArtificialMalignity::BeginSwordfight` performs
                    // `StopAll()` before registering ENTER_SWORDFIGHT.
                    // `Stop(PREFERENCE)` does not itself run the selected
                    // movement's condolence callback, so the sprite retains
                    // its last movement goal while the sword transition takes
                    // ownership.
                    self.launch_element(elem);
                }
                crate::ai::EnterSwordfightRequest::Direct(target_handle) => {
                    let target_id = self.expect_human_id_for_ai_handle(
                        target_handle,
                        "AI direct swordfight target",
                    );
                    self.direct_enter_swordfight(sim, assets, npc_id, target_id);
                }
                crate::ai::EnterSwordfightRequest::Rebalance(target_handle) => {
                    let target_id = self.expect_human_id_for_ai_handle(
                        target_handle,
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
                            .primary_target = crate::ai::AiEntityHandle::new(target_handle);
                    }
                }
            }
        }

        // Process set_as_new_principal_opponent.
        if let Some(opponent_handle) = effects.set_principal {
            let opponent_id =
                self.expect_human_id_for_ai_handle(opponent_handle, "AI principal opponent");
            self.set_as_new_principal_opponent(assets, npc_id, opponent_id);
        }

        // Process friend primary-target swaps.  The reference calls
        // `friend.set_primary_target(primary_target)` directly on the
        // other soldier when the swap heuristic fires — for every
        // improving friend in the pass, not just the last one — so we
        // apply the whole queue here after the owner's AI tick ran.
        let debug_primary_swap = crate::ai_enemy::primary_swap_debug_enabled();
        for (friend_id, new_target) in effects.friend_primary_target_swaps {
            let friend = self.world.entities.get_mut(friend_id).unwrap_or_else(|| {
                panic!(
                    "pending-drain NPC {} primary-target friend {} disappeared",
                    npc_id.index(),
                    friend_id.index()
                )
            });
            let Entity::Soldier(friend) = friend else {
                panic!(
                    "pending-drain NPC {} primary-target friend {} is not a soldier",
                    npc_id.index(),
                    friend_id.index()
                );
            };
            let friend_ai = friend.npc.ai_brain.base_mut().unwrap_or_else(|| {
                panic!(
                    "pending-drain NPC {} primary-target friend {} has no AI",
                    npc_id.index(),
                    friend_id.index()
                )
            });
            if debug_primary_swap
                && crate::ai_enemy::primary_swap_debug_matches(
                    self.control.frame_counter,
                    npc_id.index(),
                )
            {
                eprintln!(
                    "[PRIMARY_SWAP frame={} owner={} phase=friend_swap_apply friend={:?} old_target={:?} new_target={}]",
                    self.control.frame_counter,
                    npc_id.index(),
                    friend_id,
                    friend_ai.primary_target,
                    new_target,
                );
            }
            friend_ai.primary_target = crate::ai::AiEntityHandle::new(new_target);
        }

        // Process pending bow shot.
        if let Some(target_handle) = effects.shoot_target {
            let target_id = self.expect_human_id_for_ai_handle(target_handle, "AI bow target");
            self.shoot_bow_at(assets, npc_id, target_id);
        }

        // Process pending focus / focus_point / unfocus — the
        // `focus(primary_target)` / `focus(position&)` / `focus(NULL)`
        // calls.  Each explicit channel "consumes" the primary_target
        // edge by stamping `last_synced_focus_target = primary_target`,
        // so `refresh_npc_views` sees no edge and does not auto-revert
        // the explicit focus state next tick.  This is what makes
        // patterns like rider-charge passing (`focus(NULL)` while
        // `primary_target` stays set) and `battle_decisions` entry
        // honour the synchronous ordering even though the channel
        // itself is deferred.
        let mut focus_channel_fired = false;
        if let Some(target_handle) = effects.focus {
            // Original `RHElementActorNPC::Focus` accepts an arbitrary
            // `RHElement*`. Object-handling AI legitimately focuses bonuses
            // such as ale bottles, so preserve the element kind while still
            // treating a missing raw slot as corrupted state.
            let target_id = self.expect_entity_id_for_index(target_handle, "AI focus target");
            let npc = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_actor_data_mut)
                .unwrap_or_else(|| panic!("pending-drain owner {} lost AI data", npc_id.index()));
            crate::ai_vision::focus_entity(npc, target_id);
            focus_channel_fired = true;
        }

        if let Some(point) = effects.focus_point {
            // Original `Focus(RHposition&)` first calls
            // `PositionToPoint3D(posTarget, false)` and stores that point's
            // world X/Y in `starePoint`.
            let point_3d =
                self.position_to_point_3d(assets, point.sector, point.level, point.x, point.y);
            let npc = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_actor_data_mut)
                .unwrap_or_else(|| panic!("pending-drain owner {} lost AI data", npc_id.index()));
            crate::ai_vision::focus_point(
                npc,
                crate::coordinates::GroundPoint::new(point_3d.x, point_3d.y),
            );
            focus_channel_fired = true;
        }

        if effects.unfocus {
            let npc = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_actor_data_mut)
                .unwrap_or_else(|| panic!("pending-drain owner {} lost AI data", npc_id.index()));
            crate::ai_vision::unfocus(npc);
            focus_channel_fired = true;
        }

        if focus_channel_fired {
            let ai = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| {
                    panic!("pending-drain owner {} lost AI after focus", npc_id.index())
                });
            ai.last_synced_focus_target = ai.primary_target.map(crate::ai::AiEntityHandle::get);
        }

        // Process pending SlowlyOpenEyes — `slowly_open_eyes` sets
        // `view_radius = 5`, points `view_radius_goal` at the engine's
        // standard view radius, switches `eye_status` to
        // `ViewconeGrow`, and marks `view_transition`.  The
        // `ViewconeGrow` branch of `refresh_view` then ramps the cone
        // back open at 8 units/frame.
        if effects.slowly_open_eyes {
            let standard = self.ai.standard_view_polygon_radius;
            let npc = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_actor_data_mut)
                .unwrap_or_else(|| panic!("pending-drain owner {} lost AI data", npc_id.index()));
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

        // Preserve the authored boundary on either side of SetState's
        // attentive-mode call. Face normally launches before attentive mode;
        // Face authored after SetState is held until that transition has
        // launched, matching the two distinct C++ statement orders.
        let orders_after_attentive = {
            let ai = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "pending-drain owner {} lost AI before Face split",
                        npc_id.index()
                    )
                });
            let orders = std::mem::take(&mut ai.outbox.actor.orders);
            let (before, after) = orders
                .into_iter()
                .partition(|intent| !intent.after_attentive_mode);
            ai.outbox.actor.orders = before;
            after
        };
        self.launch_pending_orders_for_npc_mode_after_halt(
            sim,
            assets,
            npc_id,
            defer_turn_instruction,
            halt_count != 0,
        );
        // Original GoTo constructs and launches its movement sequence inline
        // inside the AI call. Promote this owner's queued intent now so path
        // topology and any construction-time RNG are observed at this exact
        // Think boundary. The returned sequence actions remain registered for
        // the later SequenceManager::Hourglass instruction phase.
        let _ = self.drain_pending_move_requests_for_owner(sim, npc_id);

        if !orders_after_attentive.is_empty() {
            let ai = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "pending-drain owner {} lost AI after attentive mode",
                        npc_id.index()
                    )
                });
            ai.outbox.actor.orders.extend(orders_after_attentive);
            // EnterAttentiveMode is registered but remains Todo until the
            // sequence-manager instruction phase. Register following Turns
            // behind it as well so that phase arbitrates the two elements in
            // authored FIFO order instead of eagerly instructing the Turn
            // past the still-Todo attentive barrier.
            self.launch_pending_orders_for_npc_mode_after_halt(sim, assets, npc_id, true, true);
            // `SetState(Default, ...)` calls SetAttentiveMode and then the
            // caller immediately executes GoTo in Original. The attentive
            // element must be registered first, but GoTo still constructs
            // its route synchronously at this owner boundary (including any
            // building-exit rand draws). Merely promoting the held intent
            // here leaves construction to the later global drain and shifts
            // those draws by a frame.
            let _ = self.drain_pending_move_requests_for_owner(sim, npc_id);
        }

        // Process pending `SetGuardedPC` — `set_guarded_pc`.  The AI
        // wrote its own `guarded_pc` field already; here we flip the
        // reciprocal `pc.guard` on the old and new target PCs.
        let guard_delta = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap_or_else(|| panic!("guard-delta owner {} lost its AI", npc_id.index()))
            .outbox
            .actor
            .set_guarded_pc
            .take();
        if let Some(guard_delta) = guard_delta {
            // Clear `pc.guard` on the old target
            // (`guarded_pc.set_guard(NULL)`).
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
        let reported_updates = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::ai_controller_mut)
            .map(|ai| std::mem::take(&mut ai.outbox.actor.set_reported_to_officer))
            .unwrap_or_else(|| panic!("report-update owner {} lost its AI", npc_id.index()));
        for (target_handle, value) in reported_updates {
            let target_id =
                self.expect_human_id_for_ai_handle(target_handle, "set-reported-to-officer target");
            self.world
                .entities
                .get_mut(target_id)
                .and_then(Entity::enemy_ai_mut)
                .unwrap_or_else(|| {
                    panic!("set-reported-to-officer target human {target_handle} has no EnemyAi")
                })
                .reported_to_officer = value;
        }

        // Process pending bow-ammo refill — the
        // `set_ammo_amount(BOW, MAX_NPC_ARROWS)` call inside
        // `fleeing_run_for_arrow_reserves`.
        {
            let refill = {
                let ai = self
                    .world
                    .entities
                    .get_mut(npc_id)
                    .and_then(Entity::ai_controller_mut)
                    .unwrap_or_else(|| panic!("bow-ammo owner {} lost its AI", npc_id.index()));
                std::mem::take(&mut ai.outbox.actor.refill_bow_ammo)
            };
            if refill {
                self.world
                    .entities
                    .get_mut(npc_id)
                    .and_then(Entity::ai_actor_data_mut)
                    .unwrap_or_else(|| {
                        panic!(
                            "bow-ammo refill owner {} lost AI actor data",
                            npc_id.index()
                        )
                    })
                    .number_of_arrows = crate::parameters_ai::MAX_NPC_ARROWS as u16;
            }
        }

        // Process the ordered archery-reservation release — the
        // `set_my_archery_sector(NULL)` call queued from
        // `EnemyAi::set_state` when the soldier leaves an archer-wait
        // substate.  Decrement the owner counter on the current
        // archery sector and clear the index.  The companion
        // typed effect carries the prior shooting
        // point's `(sector, point)` so we can also run the
        // `set_my_shooting_point(NULL)` `set_owner(NULL)` write here —
        // the AI layer already cleared its own `my_shooting_point`
        // field synchronously in `set_state`.
        {
            let release = if let Some(enemy) = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::enemy_ai_mut)
            {
                let effect = enemy.base.outbox.actor.take_archery_reservation_release();
                let sector = if effect.release_sector {
                    enemy.my_archery_sector.take()
                } else {
                    None
                };
                (sector, effect.shooting_point)
            } else {
                (None, None)
            };
            if let (_, Some(point)) = release
                && let Some(sector) = self
                    .ai
                    .global
                    .archery_sectors
                    .get_mut(point.sector_index as usize)
                && let Some(pt) = sector.points.get_mut(usize::from(point.point_index))
            {
                pt.owner = None;
            }
            if let (Some(idx), _) = release
                && let Some(sector) = self.ai.global.archery_sectors.get_mut(idx as usize)
            {
                sector.decrement_owner_counter();
            }
        }

        // Process pending UnalertAllNearCharlySeekers — walks all
        // soldier NPCs in the same camp and for each candidate that
        //   - is alive / active / not the seeker / not the charly,
        //   - passes the rank/antagonist guard
        //     `(seeker_rank == OFFICER || cs != antagonist)`,
        //   - and detects either charly or self within 180°,
        // dispatches `CALL_CHARLY_IS_BACK` carrying charly's handle.
        // The pending field's payload selects either self or an
        // explicit Charly handle.
        let unalert = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap_or_else(|| panic!("unalert owner {} lost its AI", npc_id.index()))
            .outbox
            .actor
            .take_unalert_near_charly_seekers();
        if let Some((target_charly, my_antagonist)) = unalert {
            let my_rank = self
                .get_entity(npc_id)
                .and_then(Entity::enemy_ai)
                .map(|enemy| enemy.soldier_profile_rank)
                .unwrap_or(crate::profiles::ProfileRank::None);
            let charly_handle = match target_charly {
                crate::ai::CharlySeekerTarget::SelfNpc => npc_id.index(),
                crate::ai::CharlySeekerTarget::Npc(handle) => handle,
            };
            if self
                .world
                .entities
                .id_at_legacy_slot(charly_handle)
                .and_then(|charly_id| self.world.entities.get(charly_id))
                .is_some()
            {
                let charly_is_self = charly_handle == npc_id.index();
                for other_id in self.world.entities.npc_ids().collect::<Vec<_>>() {
                    if other_id == npc_id {
                        continue;
                    }
                    if other_id.index() == charly_handle {
                        continue;
                    }
                    // Rank/antagonist guard:
                    //   `rank == Officer || other != antagonist`.
                    if my_rank != crate::profiles::ProfileRank::Officer
                        && other_id.index() == my_antagonist
                    {
                        continue;
                    }
                    let eligible = {
                        let Some(Entity::Soldier(os)) = self.world.entities.get(other_id) else {
                            continue;
                        };
                        // `IsDetecting180Degrees` checks only the raw active
                        // flag for the viewer and target. Dead or unconscious
                        // soldiers are not filtered by the outer Original
                        // `UnalertAllNearCharlySeekers` walk.
                        os.element.active
                    };
                    if !eligible {
                        continue;
                    }
                    // Original evaluates the full visibility predicate in
                    // this exact short-circuit order. Besides the cone gate,
                    // each surviving arm samples ComputeViewRadius and runs
                    // opaque LOS, both of which are observable and may affect
                    // whether CALL_CHARLY_IS_BACK is delivered.
                    let scratch = self.build_sim_scratch(sim, assets);
                    let other_ctx = {
                        let Some(entity) = self.world.entities.get(other_id) else {
                            continue;
                        };
                        let building_sector =
                            self.entity_building_sector(entity.element_data().sector());
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
                    other_ctx.seed_view_radius_cache(&self.ai.view_radius_cache);
                    let detects_charly = self
                        .world
                        .entities
                        .get(other_id)
                        .and_then(Entity::enemy_ai)
                        .unwrap_or_else(|| {
                            panic!(
                                "UnalertAllNearCharlySeekers candidate {} lost EnemyAi",
                                other_id.index()
                            )
                        })
                        .is_detecting_180_degrees(charly_handle, &other_ctx);
                    let detects_me_branch = !detects_charly
                        && !charly_is_self
                        && self
                            .world
                            .entities
                            .get(other_id)
                            .and_then(Entity::enemy_ai)
                            .unwrap_or_else(|| {
                                panic!(
                                    "UnalertAllNearCharlySeekers candidate {} lost EnemyAi",
                                    other_id.index()
                                )
                            })
                            .is_detecting_180_degrees(npc_id.index(), &other_ctx);
                    other_ctx.commit_view_radius_cache(&mut self.ai.view_radius_cache);
                    if !(detects_charly || detects_me_branch) {
                        continue;
                    }
                    let stimulus = crate::ai::Stimulus::with_human(
                        crate::ai::StimulusType::CallCharlyIsBack,
                        charly_handle,
                    );
                    // The preceding drain work and earlier Charly recipients
                    // may have synchronously changed entity state. The
                    // recipient context above was built at this exact Think
                    // boundary and is also the one used for dispatch.
                    let tick_data = self.build_npc_tick_data(sim, other_id, &scratch, assets);
                    self.dispatch_think_with_drain_without_forecast_deferred_turn(
                        sim, other_id, &stimulus, &other_ctx, &tick_data, assets,
                    );
                }
            }
        }

        // Process pending launch commands — create and launch
        // sequence elements for commands the AI wants to execute.
        for cmd in effects.launch_commands {
            let elem = crate::sequence::SequenceElement::new(1, cmd, Some(npc_id));
            let mut sequence = crate::sequence::Sequence::new();
            sequence.append_element(elem);
            // AI helpers call LaunchSequenceElement, which registers an
            // ordinary owned command with SequenceManager. It does not run
            // the actor's Instruct/arbitration inline at the AI call site.
            self.launch_sequence(sequence);
        }

        // Sequence commands the AI wants to launch on *another*
        // entity (e.g. soldier forcing a beggar to stand up).
        // Equivalent to a `launch_sequence_element(cmd,
        // other_actor)` call as used by the enemy beggar-identify
        // cascade.
        for (target_handle, cmd) in effects.launch_on_target {
            let target_id = self.expect_human_id_for_ai_handle(target_handle, "AI command target");
            let elem = crate::sequence::SequenceElement::new(1, cmd, Some(target_id));
            self.launch_element(elem);
        }

        // Cross-actor speech authored directly after a target command. The
        // beggar-identification path, for example, launches SHOW_FACE and
        // then synchronously calls Say on that civilian. Queue through the
        // target AI's ordinary owner-work path so all Say gates, sound
        // requests, and automatic forbids remain authoritative.
        for (target_handle, remark) in effects.say_on_target {
            let target_id = self.expect_human_id_for_ai_handle(target_handle, "AI speech target");
            let target = self.world.entities.get_mut(target_id).unwrap_or_else(|| {
                panic!(
                    "AI speech target {} disappeared before Say",
                    target_id.index()
                )
            });
            target
                .ai_controller_mut()
                .unwrap_or_else(|| {
                    panic!(
                        "AI speech target {} has no AI controller",
                        target_id.index()
                    )
                })
                .say(remark);
            self.drain_ai_owner_work_for(sim, assets, target_id);
        }

        // Full sequences the AI wants to launch verbatim — the
        // `launch_sequence(SEQ_INFO, sequence)` calls inside AI
        // handlers (e.g. the officer's turn/gather/point alert
        // sequence). `RHSequence::Launch` calls
        // `RegisterSequenceElementToGo` while the AI handler is still on the
        // stack, and `ExecutedImmediately` dispatches engine commands inline.
        // Close that exact boundary for each sequence: batching the drain
        // until after the NPC tail lets a Timer/LockUser successor escape the
        // owner's legacy Hourglass slot.
        for seq in effects.launch_sequences {
            self.launch_sequence(seq);
            self.drain_script_synchronous_actions(sim, assets, &mut Vec::new())
                .unwrap_or_else(|error| {
                    panic!(
                        "AI owner {} failed to drain a synchronously launched sequence: {error:?}",
                        npc_id.index()
                    )
                });
        }

        if effects.raise_shield_immediately {
            let entity = self
                .world
                .entities
                .get_mut(npc_id)
                .unwrap_or_else(|| panic!("instant shield owner {npc_id:?} disappeared"));
            entity.set_posture(crate::element::Posture::Upright);
            entity
                .actor_data_mut()
                .expect("instant shield owner is not an actor")
                .action_state = crate::element::ActionState::HoldingShield;
            self.refresh_retained_shield_obstacle(assets, npc_id);
        }

        if effects.refresh_shield {
            self.refresh_retained_shield_obstacle(assets, npc_id);
        }

        // Process pending LookSidewards — build a one- or two-element
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
            // `look_sidewards` calls `focus(NULL)` before allocating
            // the sequence so the soldier's gaze drops its lock for
            // the head-turn animation.  Centralise it here instead of
            // patching every caller.
            let npc = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_actor_data_mut)
                .unwrap_or_else(|| {
                    panic!("pending-drain owner {} lost AI actor data", npc_id.index())
                });
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
            let entity =
                self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
                    panic!("pending-drain AI owner {} disappeared", npc_id.index())
                });
            let ai_actor = entity.ai_actor_data_mut().unwrap_or_else(|| {
                panic!("pending-drain owner {} lost AI actor data", npc_id.index())
            });
            match ai_actor.ai_brain.base_mut() {
                Some(ai) => std::mem::take(&mut ai.outbox.actor.delete_beggar_for_all_npc),
                None => Vec::new(),
            }
        };
        for beggar_id in delete_beggar_requests {
            self.delete_beggar_detectable_for_all_npc(beggar_id);
        }

        // Process pending detectable modifications.
        if !effects.add_detectables.is_empty()
            || !effects.append_detectables.is_empty()
            || !effects.delete_detectables.is_empty()
            || !effects.delete_detectable_entities.is_empty()
        {
            let debug_consider_report = crate::ai::consider_report_debug_matches(
                self.control.frame_counter,
                npc_id.index(),
            );
            if debug_consider_report {
                eprintln!(
                    "CONSIDERREPORT {{\"stage\":\"drain_start\",\"frame\":{},\"owner\":{},\"pending_entity_deletes\":{:?}}}",
                    self.control.frame_counter,
                    npc_id.index(),
                    effects.delete_detectable_entities,
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
                    .chain(effects.add_detectables.iter().map(|(target, _)| *target))
                    .chain(effects.append_detectables.iter().map(|(target, _)| *target))
                    .chain(
                        effects
                            .delete_detectable_entities
                            .iter()
                            .map(|(target, _)| *target),
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
                        let target_creation_order = self.original_static_creation_order(target_id);
                        detection::detectable_mutation_debug_target_matches(
                            target_id.index(),
                            target_creation_order,
                        )
                        .then_some((target_id, target_creation_order))
                    })
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            // Resolve target classification for each ENEMY-arm push
            // so the `add_detectable` filter can run.  Resolved
            // up-front to avoid borrowing `self.world.entities` mutably
            // while we read target metadata from it.
            use crate::element::DetectableType;
            let enemy_target_info: Vec<Option<(bool, bool, crate::element_kinds::Camp, bool)>> =
                effects
                    .add_detectables
                    .iter()
                    .map(|(eid, dt)| {
                        if *dt != DetectableType::Enemy {
                            return None;
                        }
                        let target = self.get_entity(*eid).unwrap_or_else(|| {
                            panic!(
                                "pending-drain owner {} detectable target {} disappeared",
                                npc_id.index(),
                                eid.index()
                            )
                        });
                        Some((
                            target.is_pc(),
                            target.is_soldier(),
                            target.camp(),
                            target.is_human(),
                        ))
                    })
                    .collect();

            let (npc_camp, npc_uses_enemy_combat_ai) = {
                let owner = self.world.entities.get(npc_id).unwrap_or_else(|| {
                    panic!("pending-drain owner {} disappeared", npc_id.index())
                });
                (owner.camp(), owner.enemy_ai().is_some())
            };
            let npc = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_actor_data_mut)
                .unwrap_or_else(|| {
                    panic!("pending-drain owner {} lost AI actor data", npc_id.index())
                });
            // Delete all detectables of specified types.
            for det_type in &effects.delete_detectables {
                let idx = *det_type as usize;
                assert!(
                    idx < npc.detectable_lists.len(),
                    "pending-drain owner {} has no {:?} detectable list",
                    npc_id.index(),
                    det_type
                );
                let (mutation_length_before, presence_before) = if mutation_targets.is_empty() {
                    (0, Vec::new())
                } else {
                    (
                        npc.detectable_lists[idx].len(),
                        mutation_targets
                            .iter()
                            .map(|(target_id, target_creation_order)| {
                                (
                                    *target_id,
                                    *target_creation_order,
                                    npc.detectable_lists[idx]
                                        .iter()
                                        .any(|detectable| detectable.element == Some(*target_id)),
                                )
                            })
                            .collect::<Vec<_>>(),
                    )
                };
                npc.detectable_lists[idx].clear();
                for (target_id, target_creation_order, present_before) in presence_before {
                    if present_before {
                        detection::debug_detectable_mutation_event(
                            "delete_all",
                            "pending_effects.delete_detectables",
                            self.control.frame_counter,
                            npc_id.index(),
                            mutation_owner_creation_order,
                            idx,
                            target_id.index(),
                            target_creation_order,
                            true,
                            false,
                            mutation_length_before,
                            0,
                        );
                    }
                }
            }
            // Per-entity deletes: `delete_detectable(entity, type)`
            // drops a single (element, type) entry, leaving
            // siblings of the same type alone.
            for (entity_id, det_type) in &effects.delete_detectable_entities {
                let idx = *det_type as usize;
                assert!(
                    idx < npc.detectable_lists.len(),
                    "pending-drain owner {} has no {:?} detectable list",
                    npc_id.index(),
                    det_type
                );
                let mutation_target_creation_order = mutation_targets
                    .iter()
                    .find(|(target_id, _)| target_id == entity_id)
                    .map(|(_, target_creation_order)| *target_creation_order);
                let mutation_before = mutation_target_creation_order.map(|_| {
                    (
                        npc.detectable_lists[idx].len(),
                        npc.detectable_lists[idx]
                            .iter()
                            .any(|detectable| detectable.element == Some(*entity_id)),
                    )
                });
                npc.delete_detectable(*entity_id, *det_type);
                if let (Some(target_creation_order), Some((before, present_before))) =
                    (mutation_target_creation_order, mutation_before)
                {
                    let present_after = npc.detectable_lists[idx]
                        .iter()
                        .any(|detectable| detectable.element == Some(*entity_id));
                    detection::debug_detectable_mutation_event(
                        "delete",
                        "pending_effects.delete_detectable_entities",
                        self.control.frame_counter,
                        npc_id.index(),
                        mutation_owner_creation_order,
                        idx,
                        entity_id.index(),
                        target_creation_order,
                        present_before,
                        present_after,
                        before,
                        npc.detectable_lists[idx].len(),
                    );
                }
            }
            if debug_consider_report {
                let body_ids = npc.detectable_lists[DetectableType::Body as usize]
                    .iter()
                    .map(|detectable| detectable.element.map(EntityId::index))
                    .collect::<Vec<_>>();
                eprintln!(
                    "CONSIDERREPORT {{\"stage\":\"drain_after_delete\",\"frame\":{},\"owner\":{},\"body_ids\":{:?}}}",
                    self.control.frame_counter,
                    npc_id.index(),
                    body_ids,
                );
            }
            // Add new detectables.
            for ((entity_id, det_type), tgt) in
                effects.add_detectables.iter().zip(enemy_target_info.iter())
            {
                let idx = *det_type as usize;
                assert!(
                    idx < npc.detectable_lists.len(),
                    "pending-drain owner {} has no {:?} detectable list",
                    npc_id.index(),
                    det_type
                );
                // ENEMY-arm filter — drop pushes that fail the
                // per-NPC camp/rank arm so a Royalist soldier
                // never tracks a PC and a Lacklandist civilian
                // never tracks a Royalist soldier.
                if *det_type == DetectableType::Enemy {
                    let Some((tgt_pc, tgt_soldier, tgt_camp, tgt_human)) = *tgt else {
                        continue;
                    };
                    if !tgt_human {
                        continue;
                    }
                    if !crate::ai_detectable_filter::should_add_enemy_detectable(
                        npc_camp,
                        npc_uses_enemy_combat_ai,
                        tgt_pc,
                        tgt_soldier,
                        tgt_camp,
                    ) {
                        continue;
                    }
                }
                let mutation_target_creation_order = mutation_targets
                    .iter()
                    .find(|(target_id, _)| target_id == entity_id)
                    .map(|(_, target_creation_order)| *target_creation_order);
                let mutation_before = mutation_target_creation_order.map(|_| {
                    (
                        npc.detectable_lists[idx].len(),
                        npc.detectable_lists[idx]
                            .iter()
                            .any(|detectable| detectable.element == Some(*entity_id)),
                    )
                });
                append_detectable(&mut npc.detectable_lists[idx], *entity_id, *det_type, false);
                if let (Some(target_creation_order), Some((before, present_before))) =
                    (mutation_target_creation_order, mutation_before)
                {
                    let present_after = npc.detectable_lists[idx]
                        .iter()
                        .any(|detectable| detectable.element == Some(*entity_id));
                    detection::debug_detectable_mutation_event(
                        "add",
                        "pending_effects.add_detectables",
                        self.control.frame_counter,
                        npc_id.index(),
                        mutation_owner_creation_order,
                        idx,
                        entity_id.index(),
                        target_creation_order,
                        present_before,
                        present_after,
                        before,
                        npc.detectable_lists[idx].len(),
                    );
                }
            }
            // Preserve the release-build behavior of selected direct
            // AddDetectable calls. Original's uniqueness check is an assert,
            // so it disappears in the shipped build and the append still
            // occurs when the entry is already present.
            for (entity_id, det_type) in &effects.append_detectables {
                let idx = *det_type as usize;
                assert!(
                    idx < npc.detectable_lists.len(),
                    "pending-drain owner {} has no {:?} detectable list",
                    npc_id.index(),
                    det_type
                );
                let mutation_target_creation_order = mutation_targets
                    .iter()
                    .find(|(target_id, _)| target_id == entity_id)
                    .map(|(_, target_creation_order)| *target_creation_order);
                let mutation_before = mutation_target_creation_order.map(|_| {
                    (
                        npc.detectable_lists[idx].len(),
                        npc.detectable_lists[idx]
                            .iter()
                            .any(|detectable| detectable.element == Some(*entity_id)),
                    )
                });
                append_detectable(&mut npc.detectable_lists[idx], *entity_id, *det_type, true);
                if let (Some(target_creation_order), Some((before, present_before))) =
                    (mutation_target_creation_order, mutation_before)
                {
                    let present_after = npc.detectable_lists[idx]
                        .iter()
                        .any(|detectable| detectable.element == Some(*entity_id));
                    detection::debug_detectable_mutation_event(
                        "append",
                        "pending_effects.append_detectables",
                        self.control.frame_counter,
                        npc_id.index(),
                        mutation_owner_creation_order,
                        idx,
                        entity_id.index(),
                        target_creation_order,
                        present_before,
                        present_after,
                        before,
                        npc.detectable_lists[idx].len(),
                    );
                }
            }
        }

        // Process pending `ForgetAllNearbyCoins` request — the first
        // half of `forget_all_nearby_coins`: walk the
        // `DETECTABLE_OBJECT` list and drop every coin entry whose
        // referenced element is within Chebyshev 500 of `pos`.  The
        // second half (`other_seen_money.clear()`) is performed
        // synchronously on the AI side in
        // `EnemyAi::forget_all_nearby_coins`.
        let forget_pos = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap_or_else(|| panic!("pending-drain owner {} lost its AI", npc_id.index()))
            .outbox
            .actor
            .forget_nearby_coins
            .take();
        if let Some(pos) = forget_pos {
            use crate::element::DetectableType;
            use crate::element_kinds::ObjectType;
            const NEARBY_COIN_DISTANCE: f32 = 500.0;
            let det_idx = DetectableType::Object as usize;
            // Snapshot the candidate element ids first so we can read
            // `entities` immutably while iterating, then mutate the
            // detectable list in a second pass.
            let mut to_remove: Vec<crate::element::EntityId> = Vec::new();
            if let Some(ai_actor) = self
                .world
                .entities
                .get(npc_id)
                .and_then(Entity::ai_actor_data)
                && det_idx < ai_actor.detectable_lists.len()
            {
                for det in &ai_actor.detectable_lists[det_idx] {
                    let Some(elem_id) = det.element else {
                        continue;
                    };
                    let Some(elem) = self.world.entities.get(elem_id) else {
                        continue;
                    };
                    let Some(obj) = elem.object_data() else {
                        continue;
                    };
                    if obj.object_type != ObjectType::Coin {
                        continue;
                    }
                    let elem_pos = elem.element_data().position_map();
                    let dx = (elem_pos.x - pos.x).abs();
                    let dy = (elem_pos.y - pos.y).abs();
                    if dx.max(dy) < NEARBY_COIN_DISTANCE {
                        to_remove.push(elem_id);
                    }
                }
            }
            if !to_remove.is_empty()
                && let Some(ai_actor) = self
                    .world
                    .entities
                    .get_mut(npc_id)
                    .and_then(Entity::ai_actor_data_mut)
                && det_idx < ai_actor.detectable_lists.len()
            {
                ai_actor.detectable_lists[det_idx]
                    .retain(|d| d.element.is_none_or(|id| !to_remove.contains(&id)));
            }
        }

        // Process pending SetPosture request.  Like the
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

        // Process pending BlinkEnemy(NULL) request — clear the
        // seen_now / seen_last_frame flags on every enemy detectable
        // so the next detection pass treats anyone still in the cone
        // as a "first-seen" edge and re-issues EVENT_VIEW.
        let blink_all = {
            let entity = self
                .world
                .entities
                .get_mut(npc_id)
                .unwrap_or_else(|| panic!("pending-drain NPC {} disappeared", npc_id.index()));
            let ai = entity
                .ai_controller_mut()
                .unwrap_or_else(|| panic!("pending-drain NPC {} lost its AI", npc_id.index()));
            std::mem::take(&mut ai.outbox.actor.blink_all_enemies)
        };
        if blink_all {
            // BlinkEnemy is defined on RHElementActorNPC, not the soldier
            // subclass. ScriptGoOn therefore reaches this path for both
            // soldiers and civilians.
            let npc = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_actor_data_mut)
                .unwrap_or_else(|| {
                    panic!("pending-drain owner {} lost AI actor data", npc_id.index())
                });
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
        // Process pending `EnemyInHouseAlert` request.
        //
        // Orchestrator walks the building's occupant list, sorts by
        // camp, dispatches `panic()` to civilians, and calls
        // `init_battle_before_door` on the outnumbered side.  Both
        // the panic side-effect and the door-battle orchestration
        // (`init_battle_before_door` + `send_before_door_to_fight`
        // in `engine/soldier_helpers.rs`) are wired below.
        let in_house_alert = {
            let ai = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| panic!("pending-drain owner {} lost its AI", npc_id.index()));
            std::mem::take(&mut ai.outbox.actor.enemy_in_house_alert)
        };
        if in_house_alert {
            self.dispatch_enemy_in_house_alert(sim, npc_id, assets);
        }

        // Drain any pending panic request from the enemy AI — the
        // analogue of the civilian-side drain that runs inside
        // `nearby_civilians_panic`.  Without this, an EnemyAi that
        // pushes a `PanicRequest` (e.g. from the fleeing arm of
        // `think_alerting_event(sim, EVENT_VIEW)` outdoors) stays wedged
        // in `FleeingPanic` with no door picked.
        let observe_after_panic = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::ai_controller_mut)
            .is_some_and(|ai| std::mem::take(&mut ai.outbox.actor.observe_after_panic));
        let has_begin_panic = self
            .world
            .entities
            .get(npc_id)
            .and_then(Entity::ai_controller)
            .is_some_and(|ai| ai.outbox.actor.begin_panic.is_some());
        if has_begin_panic {
            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let entity = self
                .world
                .entities
                .get(npc_id)
                .unwrap_or_else(|| panic!("pending-drain NPC {} disappeared", npc_id.index()));
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            let mut ctx = build_ai_context_from_entity(
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
            self.refresh_selected_default_wait_identity(npc_id, &mut ctx);
            self.process_pending_begin_panic_for(sim, assets, npc_id, &ctx);
        }

        if observe_after_panic {
            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let entity = self
                .world
                .entities
                .get(npc_id)
                .unwrap_or_else(|| panic!("panic continuation owner {npc_id:?} disappeared"));
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            let mut ctx = build_ai_context_from_entity(
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
            self.refresh_selected_default_wait_identity(npc_id, &mut ctx);
            let tick = self.build_npc_tick_data_without_forecasts(sim, npc_id, &scratch, assets);
            let grid = &self.world.fast_grid;
            self.world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::enemy_ai_mut)
                .unwrap_or_else(|| panic!("panic continuation owner {npc_id:?} has no enemy AI"))
                .observe_after_synchronous_panic(sim, &ctx, &tick, Some(grid));
            // The resumed source tail contains SetState/Focus/GoTo calls.
            // Close their owner-local callbacks and actor effects before the
            // enclosing synchronous Panic continuation returns.
            self.drain_pending_for_npc_mode(
                sim,
                npc_id,
                assets,
                owner_local_no_forecast,
                defer_turn_instruction,
            );
        }

        let has_panic_seek_fallback = self
            .world
            .entities
            .get(npc_id)
            .and_then(Entity::ai_controller)
            .is_some_and(|ai| ai.outbox.actor.panic_seek_fallback);
        if has_panic_seek_fallback {
            // BeginPanic above can mutate the owner and world; do not reuse
            // its context snapshot at this later synchronous boundary.
            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let entity = self
                .world
                .entities
                .get(npc_id)
                .unwrap_or_else(|| panic!("pending-drain NPC {} disappeared", npc_id.index()));
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            let mut ctx = build_ai_context_from_entity(
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
            self.refresh_selected_default_wait_identity(npc_id, &mut ctx);
            self.process_pending_panic_seek_fallback_for(sim, assets, npc_id, &ctx);
        }

        // Drain any pending script-driven SeekArea request.  Matches
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
            // The panic boundaries above may have synchronously changed the
            // world. Script SeekArea gets a fresh owner snapshot and tick
            // data at its own Original call boundary.
            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let entity = self
                .world
                .entities
                .get(npc_id)
                .unwrap_or_else(|| panic!("pending-drain NPC {} disappeared", npc_id.index()));
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            let mut ctx = build_ai_context_from_entity(
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
            self.refresh_selected_default_wait_identity(npc_id, &mut ctx);
            let tick_for_seek =
                self.build_npc_tick_data_without_forecasts(sim, npc_id, &scratch, assets);
            self.process_pending_script_seek_area_for(sim, assets, npc_id, &ctx, &tick_for_seek);
        }

        if finish_lost_enemy_overview {
            // EndSwordfight's explicit sequence launch above has now
            // interrupted the old command and delivered its nested
            // condolence. Resume the outer EVENT_OUTOFVIEW handler at the
            // following GetBattleOverview statement with a fresh live view.
            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let entity = self
                .world
                .entities
                .get(npc_id)
                .unwrap_or_else(|| panic!("lost-enemy overview owner {npc_id:?} disappeared"));
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            let mut ctx = build_ai_context_from_entity(
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
            self.refresh_selected_default_wait_identity(npc_id, &mut ctx);
            let tick = self.build_npc_tick_data_without_forecasts(sim, npc_id, &scratch, assets);
            self.world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::enemy_ai_mut)
                .unwrap_or_else(|| panic!("lost-enemy overview owner {npc_id:?} has no enemy AI"))
                .get_battle_overview(0, &ctx, &tick);
            self.drain_pending_for_npc_mode(
                sim,
                npc_id,
                assets,
                owner_local_no_forecast,
                defer_turn_instruction,
            );
        }
    }
}
