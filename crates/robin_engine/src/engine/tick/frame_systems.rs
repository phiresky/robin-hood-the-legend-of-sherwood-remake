//! Existing tick phase implementations; scheduling remains in the parent tick spine.

use super::*;

impl EngineInner {
    /// Consume the host sound-manager update that completed after the
    /// preceding Original engine frame.
    ///
    /// Parity traces attach these between-frame resolutions to the following
    /// frame record: the original game records the engine frame before calling
    /// the sound update. Replay
    /// must therefore be able to run this boundary before applying the
    /// following frame's recorded input commands.
    pub(in crate::engine) fn hourglass_phase_sound_boundary(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) -> Result<(), String> {
        // The sound update completes after the preceding engine frame in the
        // Original. Its callbacks therefore finish before the next
        // simulation begins and must be the first mutation here.
        let cur_frame = self.control.frame_counter;
        drain_matured_exclamations(&mut self.feedback.sound_sim, cur_frame);
        // The original game handles sound completion inline while walking the pending
        // sound list. That callback may synchronously Think/Say and append a
        // request which a later resolution in this same boundary consumes.
        self.settle_npc_speech_completions(sim, assets);
        let replay_injected_resolutions = std::mem::take(
            &mut self
                .feedback
                .sound_sim
                .replay_injected_resolved_exclamations,
        );
        let resolutions = std::mem::take(&mut self.feedback.sound_sim.resolved_exclamations);
        for resolution in resolutions {
            self.debug_speech_lifecycle(
                resolution.actor_id,
                "resolution_enter",
                (
                    resolution.exclamation_id,
                    resolution.identifier,
                    resolution.duration_frames,
                ),
            );
            let pending = self
                .feedback
                .sound_sim
                .pending_exclamations
                .first()
                .cloned();
            let matches_pending = pending.as_ref().is_some_and(|pending| {
                (pending.actor_id, pending.exclamation_id)
                    == (resolution.actor_id, resolution.exclamation_id)
            });
            if matches_pending {
                let pending = pending.expect("matching pending exclamation disappeared");
                let expected_identifier =
                    (pending.profile_id & 0xFFFF_0000) | u32::from(pending.exclamation_id);
                if expected_identifier != resolution.identifier {
                    return Err(format!(
                        "sound manager resolved identifier {} for actor {}, but pending request expects {}",
                        resolution.identifier, resolution.actor_id, expected_identifier
                    ));
                }
                if self.control.ranked_simulation_policy().is_some() {
                    Self::validate_ranked_speech_resolution(assets, &pending, &resolution)?;
                }
                self.feedback.sound_sim.pending_exclamations.remove(0);
            } else if replay_injected_resolutions {
                // The host sound queue is not serialized in legacy saves.
                // Schema-16 records its concrete between-frame completions,
                // while Rust can independently reconstruct a different
                // logical request from adopted NPC state.  An authoritative
                // Original completion must not consume that unrelated Rust
                // FIFO entry; process its timing below exactly as in the
                // empty-pending case.
                tracing::warn!(
                    actor_id = resolution.actor_id,
                    exclamation_id = resolution.exclamation_id,
                    identifier = resolution.identifier,
                    duration_frames = resolution.duration_frames,
                    pending = ?self.feedback.sound_sim.pending_exclamations,
                    "replay injected an authoritative Original host exclamation that does not match the Rust logical FIFO"
                );
            } else if pending.is_some() {
                return Err(format!(
                    "sound-manager resolution order diverged for actor {} exclamation {}; pending FIFO: {:?}",
                    resolution.actor_id,
                    resolution.exclamation_id,
                    self.feedback.sound_sim.pending_exclamations
                ));
            } else {
                return Err(format!(
                    "live sound manager resolved exclamation {} for actor {} with no pending request",
                    resolution.exclamation_id, resolution.actor_id
                ));
            }
            if resolution.duration_frames == 0 {
                self.feedback
                    .sound_sim
                    .finished_exclamations
                    .push((resolution.actor_id, u32::from(resolution.exclamation_id)));
                self.settle_npc_speech_completions(sim, assets);
            } else {
                self.feedback.sound_sim.playing_exclamations.push(
                    crate::sound::PlayingExclamation {
                        actor_id: resolution.actor_id,
                        exclamation_id: u32::from(resolution.exclamation_id),
                        finish_frame: cur_frame + resolution.duration_frames,
                    },
                );
            }
        }
        Ok(())
    }

    /// Drain effects deferred by the preceding tick before any mission,
    /// entity, path, NPC, or sequence work observes this frame's state.
    ///
    /// The original game starts its simulation update with host/widget and
    /// mission-state work.
    /// These Rust-owned queues have no one-to-one original equivalent; their
    /// relative placement is retained from the pre-decomposition Rust tick.
    pub(in crate::engine) fn hourglass_phase_deferred_effects_start(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) -> bool {
        self.hourglass_phase_sound_boundary(sim, assets)
            .unwrap_or_else(|reason| panic!("internal sound boundary rejected: {reason}"));
        let cur_frame = self.control.frame_counter;
        // Drain deferred console-cheat / death reinforcement spawns and
        // scroll-reveal amulet spawns. Both used to live in
        // `Game::run_engine_tick` because they needed `&mut LevelAssets`
        // to load sprites; the two sprite families are now preloaded at
        // mission start (`preload_campaign_peasant_sprites`,
        // `preload_scroll_amulet_sprite`) so the spawn paths read the
        // scriptor cache via `&LevelAssets` and the whole flow lives
        // inside `perform_hourglass` — keeping the "sim mutation only
        // during perform_hourglass" invariant intact.
        self.drain_pending_reinforcements(sim, assets);
        self.drain_pending_scroll_amulets(sim, assets);
        self.drain_pending_hero_speeches(assets);
        self.drain_pending_hades_kills(sim, assets);
        self.drain_pending_concussion_side_effects(sim, assets);

        // Drain matured sound-source finishes.  Replaces the
        // `stop_sound_source` logic the Rust host used to run on
        // Audio-backend playback-completion events: for each scheduled
        // source whose sim-frame deadline has arrived, `Single` sources
        // flip to `active = false` and `Volatile` sources are deleted
        // from the manager.  `Delayed` / `Looped` never land in
        // `playing_sources` (Delayed re-rolls itself below; Looped
        // doesn't terminate on its own), so this drain only ever sees
        // Single/Volatile; still match exhaustively to fail loudly if
        // a kind ever leaks into the queue.
        let mut still_playing_sources = Vec::new();
        let mut source_deactivations: Vec<usize> = Vec::new();
        let mut source_deletions: Vec<usize> = Vec::new();
        let mut delayed_restarts: Vec<usize> = Vec::new();
        for p in self.feedback.sound_sim.playing_sources.drain(..) {
            if p.finish_frame > cur_frame {
                still_playing_sources.push(p);
                continue;
            }
            let Some(src) = self.feedback.sound_sim.sources.get(p.source_index as usize) else {
                // Slot already cleared (e.g. Destroy command ran this
                // tick); drop the stale entry silently.
                continue;
            };
            match src.source_kind {
                crate::sound_source::SoundSourceKind::Single => {
                    source_deactivations.push(p.source_index as usize);
                }
                crate::sound_source::SoundSourceKind::Volatile => {
                    source_deletions.push(p.source_index as usize);
                }
                crate::sound_source::SoundSourceKind::Delayed => {
                    delayed_restarts.push(p.source_index as usize);
                }
                crate::sound_source::SoundSourceKind::Looped => {
                    tracing::warn!(
                        source_index = p.source_index,
                        kind = ?src.source_kind,
                        "sound source scheduled finish fired for Looped/Delayed kind — \
                         should never happen (schedule_source_finish skips them)"
                    );
                }
            }
        }
        self.feedback.sound_sim.playing_sources = still_playing_sources;
        for idx in source_deactivations {
            if let Some(src) = self.feedback.sound_sim.sources.get_mut(idx) {
                src.active = false;
            }
        }
        for idx in source_deletions {
            self.feedback.sound_sim.sources.delete(idx);
        }
        for idx in delayed_restarts {
            let Some(src) = self.feedback.sound_sim.sources.get_mut(idx) else {
                continue;
            };
            if src.delay_stepping > 0 && src.max_delay > src.min_delay {
                let seed = (u64::from(cur_frame) << 32) ^ (u64::from(src.id) << 8) ^ idx as u64;
                let step = crate::sim_rng::with_auxiliary_seed(
                    crate::sim_rng::AuxiliaryRngSite::DelayedSoundTimer,
                    seed,
                    |rng| rng.u32(0..u32::from(src.delay_stepping)),
                ) as u16;
                let range = src.max_delay - src.min_delay;
                src.timer = (u32::from(step) * u32::from(range) / u32::from(src.delay_stepping))
                    as u16
                    + src.min_delay;
            } else {
                src.timer = src.min_delay;
            }
        }

        // PC-guarded state drives start/quit mission widget enable and
        // guard-portrait blinking.  The
        // widget-enable side is applied from `Game::run_engine_tick`
        // before `perform_hourglass` runs so both consumers see the
        // same value for this tick.  The guard-portrait blink is
        // rendered live by `ui_panel.rs` directly from
        // `mission.mission_won` + `PcData::guard`, so there's nothing
        // to do here for (b).

        self.is_pc_guarded()
    }

    /// Capture fighter state before the live owner walk. Runtime slot order,
    /// Original creation identity, and portrait priority are distinct; the
    /// subsequent owner coordinator preserves its explicit slot contract.
    pub(super) fn hourglass_phase_entities(&mut self) -> bool {
        // Snapshot pre-hourglass swordfight state so we can detect a
        // swordfight→non-swordfight transition across this tick and
        // raise the ignore-mouse-event bracket on the falling edge.
        // The per-element / sequence-manager hourglass passes below may
        // flip the selected PC out of `Swordfighting`; when that
        // happens mid-drag the in-flight drag must be suppressed so it
        // doesn't bleed into the next click-release action.
        let was_swordfighting = self.is_selected_pc_swordfighting();

        // The soldier update performs its specialized prelude before the NPC
        // update: apple smell,
        // primary-target tracking, and the reaction-time nearby-enemy test.
        // In particular, keep the target snap introduced by 24c43efde ahead
        // of view refresh without moving it into the base NPC phases.
        observe_npc_hourglass_phase(NpcHourglassPhase::SoldierPrelude);
        // Work runs at each soldier's live owner slot below.

        // First base-NPC update phase. Patrol history observes the actor before
        // the human-actor update
        // executes its movement/order work.
        observe_npc_hourglass_phase(NpcHourglassPhase::Patrol);
        // Work runs before the Human/Actor slices of each NPC owner below.

        // ── Element hourglass (per-element update) ───────────────
        observe_npc_hourglass_phase(NpcHourglassPhase::BaseHuman);
        // Human concussion healing runs synchronously in each owner's
        // pre-Actor hook below.
        // Concrete entity updates and their retain/remove results
        // execute in the live owner walk below; there is no legacy base pass.

        // ── PC selection outline fade ────────────────────────────
        // The hulk state-machine block runs during the per-element
        // refresh pass.
        self.refresh_pc_selection_hulk();
        self.refresh_tactical_selection_hulks();

        // Tick the cheat-teleport hulk-rebuild fade counter on every
        // PC.  Decrementing here (rather than from the per-PC render
        // path) lets rollback / replay see bit-identical state (the
        // counter is serde'd `PcData`).
        self.tick_pc_teleport_fades();

        was_swordfighting
    }

    /// Advance movement, animations, scripts, and the NPC-facing state that
    /// must be refreshed before the main AI pass.
    ///
    /// These responsibilities are distributed across
    /// individual entity updates inside the original creation-ordered loop.
    pub(super) fn hourglass_phase_entity_systems(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut CameraDisplayState,
        assets: &LevelAssets,
    ) -> Vec<crate::engine::movement::TerminalMovementOrderPop> {
        // Preserve the position each element exposed before the globally
        // batched movement pass. The original does not have this batch:
        // The NPC update calls the human-actor update
        // (and therefore the observer's own movement) before view refresh,
        // while actors with a later creation order have not run yet.
        let positions_before_movement = {
            let _detail = entity_system_detail_guard(EntitySystemDetail::BoundarySnapshot);
            let mut positions = EntitySlots::filled(self.world.entities.len(), None);
            for (entity_id, entity) in self.world.entities.occupied() {
                positions[entity_id] =
                    Some(crate::entities::BoundaryPosition::of(entity.element_data()));
            }
            positions
        };

        // ── Per-frame movement tick ─────────────────────────────
        // Actor movement runs later, inside the live legacy-slot owner walk.

        // `quit_swordfight_with_far_opponents` is called ONLY during
        // walking-with-sword movement, NOT for stationary entities.
        // Only check entities actively moving in sword state.
        // Owned by the selected sword-movement Execute arm in
        // `tick_entity_movement_owner`.

        // ── PC sword-walk pinch abort ───────────────────────────
        // During `WalkingWithSword` / `RunningWithSword`, after the
        // per-frame sprite motion the PC checks whether two opponents
        // are pinching its forward corridor and, if so, marks the
        // current sequence element `Impossible`.  Runs only on PCs in
        // sword movement with an active movement element and an
        // in-flight position delta (`is_moving_map()`).
        // `element_impossible` itself silently no-ops when the
        // element is `NonInterruptable`, which is the desired
        // behaviour.
        // Owned by the selected PC sword-movement Execute arm too.

        // ── Dispatch EventReachPoint to NPCs that just finished walking ──
        // Fires `Think(EVENT_REACHPOINT)` when a MOVE sequence
        // element terminates.

        // Separate Rust reconciliation boundary: the cited Original actor
        // Execute arms do not establish zone occupancy as owner-local work.
        // Fires EnterZone/ExitZone on zone scripts when occupancy changes.

        // ── Per-frame animation tick ────────────────────────────
        // Advance sprite animations for idle actors, FX, and other entities.
        // Supported moving actors are animated inside their live owner Execute arm.
        // Line-jump step advance runs inside each actor's own owner
        // envelope below, not as a batch ahead of the walk.

        // Every supported nonactor update now runs below at its
        // live legacy slot: mobile boundary first, then static owners, then
        // projectile/net dispatch.
        let terminal_movement_order_pops = self.tick_actor_owner_envelopes_with_display(
            sim,
            display,
            assets,
            &positions_before_movement,
        );
        // ── Corpse-intersection repulsion hook ────────────────────
        // Scan for lying↔non-lying posture transitions and fire
        // `update_intersecting_corpses` so stacked corpses get the
        // smaller repulsive radius and don't shove each other out
        // of their hitboxes.  Runs after animations have had a
        // chance to change postures this frame and before the next
        // frame's movement (which reads `small_repulsive_radius`
        // via `compute_repulsive_force`).
        {
            let _detail = entity_system_detail_guard(EntitySystemDetail::CorpseUpdates);
            self.process_corpse_intersection_updates();
        }

        // TODO(original-parity): the followed-target position oracle below
        // proves one movement/NPC-refresh interleaving, but the rest of this
        // system-oriented pass still lacks per-entity dispatch boundaries.
        // Keep those responsibilities batched until each consumer has the
        // mixed pre/post inputs required at an individual creation slot.

        finish_entity_system_detail_frame();
        terminal_movement_order_pops
    }

    /// Preserve the coarse NPC observations, validate closed owner boundaries,
    /// and decay screen remarks after the live owner pass. Position-snapshot
    /// reads stay inside `hourglass_phase_entity_systems`.
    pub(super) fn hourglass_phase_npcs(&mut self) {
        // Listen/object reveal and Target Heard are actor-owned Execute work.
        // ── Creation-ordered pre-detection boundary ──────────────
        // These observations remain coarse labels for the original nested
        // order. The coordinator below interleaves the actual operations per
        // NPC: own synchronous FITAGAIN + resurrection/eye apply, own body
        // broadcast, own view refresh, then that same NPC's detection refresh.
        observe_npc_hourglass_phase(NpcHourglassPhase::Broadcasts);

        observe_npc_hourglass_phase(NpcHourglassPhase::View);

        observe_npc_hourglass_phase(NpcHourglassPhase::Detection);
        // Production work already ran inside the live actor-owner walk in the
        // preceding EntitySystems phase. Keep these coarse observations for
        // the PA-016 tick-spine contract only.

        // The phase observations below retain the coarse PA-016 ordering
        // contract. Production work no longer runs here: PA-013 executes the
        // complete post-detection tail inside each NPC's creation slot before
        // the next NPC enters detection refresh.
        observe_npc_hourglass_phase(NpcHourglassPhase::Ambush);

        // ── Per-tick AILOCK_BUSY edge detector ─────────────────
        // Lock or unlock AILOCK_BUSY based on the live
        // `is_very_very_busy` predicate (posture or active PassDoor /
        // Fall element).  Runs after the view refresh.
        observe_npc_hourglass_phase(NpcHourglassPhase::Busy);

        // ── Stuck-on-ladder emergency counter ──────────────────
        // Bump per frame for non-script-locked NPCs on outdoor
        // ladders idling in CMD_WAIT/CMD_MOVE_WAITING; after 25
        // frames force a return to duty so the actor can self-recover.
        // Runs after the BUSY edge detector.
        observe_npc_hourglass_phase(NpcHourglassPhase::Ladder);

        // ── Locked-frame timer bumps ───────────────────────────
        // When any lock is held the entire update tail
        // short-circuits while the three timer ring-frames
        // (`when_does_timer_ring`, `when_does_macro_timer_ring`,
        // `emoticon_expiration_date`) tick forward by +1.  This both
        // keeps the relative timer offset stable across the lock
        // window and acts as the "skip the fire" gate for the
        // downstream macro-timer / EVENT_TIMER fire checks (which
        // compare against the live `frame_counter`).
        observe_npc_hourglass_phase(NpcHourglassPhase::LockGate);

        // The unlocked tail follows the original game's exact order:
        // Every-16-frame tasks, normal EVENT_TIMER, macro timer, then stimuli held
        // by a prior AI/script lock.
        observe_npc_hourglass_phase(NpcHourglassPhase::SixteenthFrame);

        observe_npc_hourglass_phase(NpcHourglassPhase::NormalTimer);

        // ── Macro-timer hourglass ──────────────────────────────
        // Poll the macro-specific timer each frame and, when it
        // rings, call `execute_next_macro_command` directly —
        // bypassing the stimulus queue so CMD_WAIT / CMD_BEND
        // resume cleanly. Any resulting movement-order / substate change
        // is visible to the queued-stimulus drain in the same frame.
        observe_npc_hourglass_phase(NpcHourglassPhase::MacroTimer);

        observe_npc_hourglass_phase(NpcHourglassPhase::QueuedStimuli);

        // Every engine-entered AI call closes its ordered owner-local
        // state-change/speech boundary before returning to effects/orders. Nothing
        // may survive into the obsolete post-NPC speech batch position.
        let frame_counter = self.control.frame_counter;
        for (npc_id, entity) in self.world.entities.npcs() {
            let leaked = entity
                .ai_controller()
                .map(|ai| ai.outbox.reentrant.owner_work.as_slice())
                .unwrap_or_default();
            assert!(
                leaked.is_empty(),
                "NPC {} leaked owner-local AI work past its update slot on frame {}: {leaked:?}",
                npc_id.index(),
                frame_counter,
            );
        }

        // ── HUD speech-log decay ────────────────────────────────
        // Decrement the per-remark display timer and evict expired
        // entries every frame regardless of `speech_display` so the
        // Vec does not grow unbounded when the overlay is off.
        self.tick_screen_remarks();

        // TODO(PA-013): pure-Rust handlers still enqueue until their AI borrow
        // returns, so arbitrary reads between speech/state-change statements cannot
        // yet observe the original game's fully synchronous engine/audio order.
    }

    /// Advance combat, projectiles, abilities, and other gameplay systems that
    /// consume the entity/sequence/NPC state established above.
    pub(in crate::engine) fn hourglass_phase_gameplay_systems(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        _display: &mut CameraDisplayState,
        assets: &LevelAssets,
    ) {
        // Active abilities, Listen/Heard, projectiles, and beggar simulation
        // already executed in their live owner slots.

        // Combat progression without a proven cross-subsystem ordering
        // discrepancy remains batched. Fallback-timed completions already
        // cleared at their owning actor slots above and are skipped here.
        self.tick_melee_combat(sim, assets);

        // Preserve only the terminal shoulder-climb sprite synchronization
        // before the motion latch is consumed. Carried transforms remain in
        // their established post-propagation phase below.
        abilities::sync_terminal_shoulder_animations(
            &mut self.world.entities,
            &self.world.original_creation_order_by_entity,
        );

        // ── Per-actor `Order::done` propagation ────────────────
        // Runs after every per-system sprite-advance tick this frame
        // (movement, jumps, animations, bow shots, melee, abilities),
        // each of which has already stashed its result on the sprite
        // via `Sprite::record_motion_state`.  The pass flips
        // `Order::done` on every actor whose sprite reported
        // `MotionState::Done`, then clears `last_motion_state` so the
        // next tick starts fresh.  Read by the postpone-race guard in
        // `EngineInner::engine_postpone`.
        self.propagate_done_to_current_orders();

        // Keep bodies carried by Little John positioned on the carrier and
        // drive their sprite animation synchronized with the carrier.
        abilities::sync_carried_positions(&mut self.world.entities, &assets.profile_manager);

        // TODO(original-parity): move further gameplay maintenance into the
        // ordered pass only when a concrete observable discrepancy is proven.
    }

    /// Apply work intentionally deferred until every entity, path, sequence,
    /// NPC, and gameplay-system update has completed.
    ///
    /// The original game performs the
    /// swordfight falling-edge check, titbit update, dead-selection scan, and
    /// anonymous timers after the sequence manager. Rust adds deterministic
    /// condolation, self-stimulus, and immediate-action drains.
    pub(super) fn hourglass_phase_deferred_effects_end(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        was_swordfighting: bool,
    ) {
        // ── Swordfight-drag IgnoreMouseEvent bracket ────────────
        // If the selected PC was swordfighting at entry to
        // `perform_hourglass` but is no longer swordfighting after
        // the per-element / sequence-manager hourglass, raise the
        // ignore-mouse-event bracket so a drag in flight when the
        // swordfight ended this tick is suppressed.  We push the
        // request as a side effect; the host gates it on
        // `InputState::is_dragging` in `apply_side_effects`.
        if was_swordfighting && !self.is_selected_pc_swordfighting() {
            self.feedback
                .pending_side_effects
                .pending_swordfight_drag_ignore = true;
        }

        // ── Titbit sync + per-frame update ──────────────────────
        // First, sync persistent titbits (emoticons, unconscious
        // stars, alert indicators) with current entity state.
        self.sync_titbits(assets);

        // Then run the titbit update to advance animations and
        // expire finished titbits.
        {
            let query = EntityTitbitQuery {
                sim,
                entities: &self.world.entities,
                sequence_manager: &self.orders.sequence_manager,
                follow_element: self.players.seats[0].follow_element,
            };
            self.feedback.titbit_manager.update(&query);
            // Refresh preparation: advance blink counter, sort by
            // display order using each supplier entity's Y position
            // as a stand-in (we don't compute display order yet).
            self.feedback.titbit_manager.prepare_refresh(|handle| {
                self.world
                    .entities
                    .id_at_legacy_slot(handle.0)
                    .and_then(|entity_id| self.world.entities.get(entity_id))
                    .map(|e| e.element_data().position_map().y)
            });
        }

        // ── Ground mark animation ────────────────────────────────
        // Advanced after `perform_hourglass_inner` by `ground_mark.tick`,
        // using the deterministic director view. That helper preserves the
        // original on-screen guard, so off-screen marks freeze. The renderer
        // remains read-only; see the wrapper at the start of this file.

        // Selection ring animation lives host-side now —
        // `Game::run_engine_tick` advances `host.selection_mark`
        // once per frame, gated on the same `should_run_hourglass`
        // check as this function, so pause / console still freeze
        // the ring.

        // ── Check selected PCs are still alive ───────────────────
        {
            let mut deselect = Vec::new();
            for &pc_id in &self.players.seats[0].selection {
                if let Some(entity) = self.world.entities.get(pc_id) {
                    let should_deselect = match entity {
                        Entity::Pc(pc) => pc.pc.life_points <= 0 || pc.human.unconscious,
                        _ => false,
                    };
                    if should_deselect {
                        deselect.push(pc_id);
                    }
                }
            }
            for pc_id in deselect {
                // Message forwarding synchronously routes
                // MSG_UNSELECT_CHARACTER to the engine/game receivers at
                // this exact point. Do not leave the authoritative selection
                // mutation in Rust's next-frame message queue.
                if self.is_sherwood(&assets.profile_manager)
                    && let Some(Entity::Pc(pc)) = self.get_entity_mut(pc_id)
                {
                    pc.pc.interface_hidden = true;
                }
                self.unselect_single_pc(pc_id);
                self.update_recording_after_selection_change();
                self.players.action_before_recording_macro = crate::profiles::Action::NoAction;
            }
        }

        // ── Anonymous timers ─────────────────────────────────────
        // The engine tick terminates a timer
        // element only when its timer property is *exactly* 1, and
        // otherwise decrements the `int` property. A timer recorded with 0
        // frames — e.g. `RecordTimer( Rand( 25 ) )` rolling a zero — therefore
        // counts down through negative values and NEVER terminates, stalling
        // its sequence level for the rest of the mission. Match that exactly:
        // an `expired if remaining <= 1` test would let the zero case fire a
        // frame later and advance a sequence the Original leaves parked.
        let mut expired: Vec<crate::sequence::SequenceElementRef> = Vec::new();
        self.orders.timer_elements.retain_mut(|timer| {
            if timer.remaining == 1 {
                expired.push(timer.element_ref);
                false
            } else {
                timer.remaining -= 1;
                true
            }
        });
        for r in expired {
            self.orders
                .sequence_manager
                .element_terminated(r.sequence_id, r.element_index);
        }

        // ── Post-timer removal-notification dispatch ──────────────
        // The pre-timer pass after `hourglass_phase_sequences` preserves the
        // original state-change, removal-notification, then readiness ordering for work
        // that completed before this scan. This second pass is still required:
        // timer expiry above can itself terminate an owned sequence element
        // and queue another card. Its continuation and immediate successors
        // drain below, after this frame's timer iteration has finished.
        self.dispatch_condolations(sim, assets);

        // ── Same-tick re-entrant stimulus dispatch ───────────────
        // The condolation drain calls `Think(EVENT_DONE)` /
        // `Think(EVENT_IMPOSSIBLE)` / etc. synchronously and
        // re-entrantly on the same tick — so e.g. a patrol Turn
        // that gets interrupted when enabling attentive mode
        // launches `ENTER_ATTENTIVE_MODE` during
        // Enemy-sighting processing fires its `EVENT_DONE`
        // *during that same* `EventView` Think, advancing
        // `SUBSTATE_ATTACKING_REACTIONTIME_TURNING` →
        // `REACTIONTIME` before the frame ends.  We can't nest
        // `&mut AiController` borrows mid-think, so
        // `send_condolation_card` queues the stimulus via
        // `fire_self_stimulus` (→ `pending_self_stimuli`).  Drain
        // that queue here — after `dispatch_condolations` has
        // populated it — so the redispatch happens on the same
        // tick as the condolation, keeping
        // `REACTIONTIME_TURNING → REACTIONTIME` timing correct.
        // Without this the substate waits for the full
        // 20-tick timer upper bound regardless of which
        // sequence actually completed.
        self.drain_pending_self_stimuli(sim, assets);

        // ── End-of-tick registration-inline drain ───────────────────
        // Anonymous timers run after the sequence-manager tick. Preserve
        // only work Original registration executes on that callback stack:
        // immediately executed commands and direct waiting-priority calls.
        // Ordinary successors stay queued for the next manager hourglass.
        self.drain_registration_inline_actions_sync(sim, assets);
    }
}
