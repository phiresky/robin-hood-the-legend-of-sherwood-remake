//! Existing tick phase implementations; scheduling remains in the parent tick spine.

use super::*;
use crate::engine::TickCtx;
use crate::sequence::SequenceElementRef;

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
        tcx: TickCtx<'_>,
        execution: Option<&crate::ranked_resim::RankedExecutionContext>,
    ) -> Result<(), String> {
        // The sound update completes after the preceding engine frame in the
        // Original. Its callbacks therefore finish before the next
        // simulation begins and must be the first mutation here.
        let cur_frame = self.control.frame_counter;
        drain_matured_exclamations(&mut self.feedback.sound_sim, cur_frame);
        // The original game handles sound completion inline while walking the pending
        // sound list. That callback may synchronously Think/Say and append a
        // request which a later resolution in this same boundary consumes.
        self.settle_npc_speech_completions(tcx);
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
                if let Some(execution) = execution {
                    execution.validate_speech_resolution(tcx.assets, &pending, &resolution)?;
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
                self.settle_npc_speech_completions(tcx);
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

    /// Finish sound deadlines before mission and entity work observes the frame.
    pub(in crate::engine) fn hourglass_phase_deferred_effects_start(
        &mut self,
        tcx: TickCtx<'_>,
        execution: Option<&crate::ranked_resim::RankedExecutionContext>,
    ) -> bool {
        self.hourglass_phase_sound_boundary(tcx, execution)
            .unwrap_or_else(|reason| panic!("internal sound boundary rejected: {reason}"));
        let cur_frame = self.control.frame_counter;
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

    /// Capture selection state and advance presentation counters before owners run.
    pub(super) fn hourglass_phase_entities(&mut self) -> bool {
        // Detect a swordfight ending during owner or sequence execution so an
        // in-flight drag cannot leak into the next click-release action.
        let was_swordfighting = self.is_selected_pc_swordfighting();
        self.refresh_pc_selection_hulk();
        self.refresh_tactical_selection_hulks();
        self.tick_pc_teleport_fades();
        was_swordfighting
    }

    /// Execute each live owner completely before advancing to the next slot.
    pub(super) fn hourglass_phase_entity_systems(&mut self, tcx: TickCtx<'_>) {
        self.tick_actor_owner_envelopes(tcx);

        // Close posture writes made outside an actor's own update. Owner-local
        // posture transitions already publish before the next owner runs.
        {
            let _detail = entity_system_detail_guard(EntitySystemDetail::CorpseUpdates);
            self.process_corpse_intersection_updates();
        }
        finish_entity_system_detail_frame();

        // Remarks decay once after all owners, including while hidden.
        self.tick_screen_remarks();
    }

    /// Advance combat, projectiles, abilities, and other gameplay systems that
    /// consume the entity/sequence/NPC state established above.
    pub(in crate::engine) fn hourglass_phase_gameplay_systems(
        &mut self,
        tcx: TickCtx<'_>,
        _display: &mut CameraDisplayState,
    ) {
        // Active abilities, Listen/Heard, projectiles, and beggar simulation
        // already executed in their live owner slots.

        // Combat progression without a proven cross-subsystem ordering
        // discrepancy remains batched. Fallback-timed completions already
        // cleared at their owning actor slots above and are skipped here.
        self.tick_melee_combat(tcx);

        // Order completion was published inside each actor's Execute boundary.
        // Clear presentation motion edges only after their frame consumers.
        for (_, entity) in self.world.entities.occupied_mut() {
            entity.element_data_mut().sprite.last_motion_state = None;
        }

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
        tcx: TickCtx<'_>,
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
                .request_signal(crate::engine::HostSignal::IgnoreSwordfightDrag);
        }

        // ── Titbit sync + per-frame update ──────────────────────
        // First, sync persistent titbits (emoticons, unconscious
        // stars, alert indicators) with current entity state.
        self.sync_titbits(tcx.assets);

        // Then run the titbit update to advance animations and
        // expire finished titbits.
        {
            let query = EntityTitbitQuery {
                sim: tcx.sim,
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
                if self.is_sherwood(&tcx.assets.profile_manager)
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
            self.element_terminated(
                tcx,
                &mut Vec::new(),
                SequenceElementRef::new(r.sequence_id, r.element_index),
            );
        }
    }
}
