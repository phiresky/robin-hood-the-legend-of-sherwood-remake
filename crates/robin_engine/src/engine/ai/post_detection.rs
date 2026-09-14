//! Creation-ordered post-detection NPC update tail, plus test-only legacy
//! drains used by focused detection seams.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum NpcPostDetectionTailPhase {
    Ambush,
    Deafness,
    Busy,
    Ladder,
    RandomSpeech,
    LockGate,
    SixteenthFrame,
    NormalTimer,
    MacroTimer,
    Emoticon,
    QueuedStimuli,
}

#[cfg(test)]
thread_local! {
    static NPC_POST_DETECTION_TAIL_TRACE: crate::engine::test_support::Probe<(EntityId, NpcPostDetectionTailPhase)> =
        const { crate::engine::test_support::Probe::new() };
}

#[cfg(test)]
fn observe_npc_post_detection_tail_phase(npc_id: EntityId, phase: NpcPostDetectionTailPhase) {
    NPC_POST_DETECTION_TAIL_TRACE.with(|trace| trace.record((npc_id, phase)));
}

#[cfg(not(test))]
#[inline(always)]
fn observe_npc_post_detection_tail_phase(_npc_id: EntityId, _phase: NpcPostDetectionTailPhase) {}

#[cfg(test)]
pub(crate) fn capture_npc_post_detection_tail_phases<T>(
    f: impl FnOnce() -> T,
) -> (T, Vec<(EntityId, NpcPostDetectionTailPhase)>) {
    NPC_POST_DETECTION_TAIL_TRACE.with(|trace| trace.capture(f))
}

use crate::element::EntityId;

impl EngineInner {
    #[inline(never)]
    fn bored_owner_boundary_debug(&self, npc_id: EntityId, phase: &str) {
        let frame = self.control.frame_counter;
        let owner = npc_id.index();
        // Keep the disabled path ahead of all diagnostic-only world and queue reads.
        if !crate::ai::AiController::bored_boundary_debug_matches(frame, owner) {
            return;
        }
        let command = self.actor_command(npc_id);
        let ai = self.world.entities.expect_ai_controller(
            npc_id,
            format_args!("BORED_BOUNDARY owner {owner} during {phase}"),
        );
        eprintln!(
            "BORED_BOUNDARY frame={} owner={} phase={} command={:?} state={:?} substate={:?} timer_running={} timer_deadline={} self_stimuli={} owner_work={} orders={}",
            frame,
            owner,
            phase,
            command,
            ai.current_state,
            ai.current_substate,
            ai.timer_is_running,
            ai.when_does_timer_ring,
            ai.outbox.reentrant.self_stimuli.len(),
            ai.outbox.reentrant.owner_work.len(),
            ai.outbox.actor.orders.len(),
        );
    }
}

impl EngineInner {
    /// Creation-ordered tail of the original-game NPC update.
    ///
    /// This is entered immediately after the owner's complete
    /// detection FIFO and returns before the next NPC creation slot.
    /// Preserve the original game's ordering.
    pub(crate) fn tick_npc_post_detection_tail_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        self.bored_owner_boundary_debug(npc_id, "entry");
        let entity = self.expect_entity(npc_id, "creation-ordered post-detection owner");
        assert!(
            entity.ai_actor_data().is_some(),
            "post-detection owner {} has no AI actor data",
            npc_id.index()
        );
        assert!(
            entity.ai_controller().is_some(),
            "post-detection NPC {} has no AI controller",
            npc_id.index()
        );

        observe_npc_post_detection_tail_phase(npc_id, NpcPostDetectionTailPhase::Ambush);
        self.tick_refresh_ambush_points_for_npc(sim, npc_id, assets);

        observe_npc_post_detection_tail_phase(npc_id, NpcPostDetectionTailPhase::Deafness);
        self.tick_npc_refresh_deafness_for_npc(npc_id);

        observe_npc_post_detection_tail_phase(npc_id, NpcPostDetectionTailPhase::Busy);
        self.tick_npc_busy_edge_detect_for_npc(npc_id);

        observe_npc_post_detection_tail_phase(npc_id, NpcPostDetectionTailPhase::Ladder);
        self.tick_npc_stuck_on_ladder_for_npc(sim, npc_id, assets);

        observe_npc_post_detection_tail_phase(npc_id, NpcPostDetectionTailPhase::RandomSpeech);
        self.tick_civilian_random_speech_for_npc(sim, npc_id, assets);

        observe_npc_post_detection_tail_phase(npc_id, NpcPostDetectionTailPhase::LockGate);
        if self.tick_npc_lock_gate_for_npc(npc_id) {
            self.bored_owner_boundary_debug(npc_id, "lock_gate_return");
            return;
        }

        observe_npc_post_detection_tail_phase(npc_id, NpcPostDetectionTailPhase::SixteenthFrame);
        self.tick_periodic_ai_for_npc(sim, npc_id, assets);
        self.bored_owner_boundary_debug(npc_id, "after_periodic");

        observe_npc_post_detection_tail_phase(npc_id, NpcPostDetectionTailPhase::NormalTimer);
        self.tick_ai_normal_timer_for_npc(sim, npc_id, assets);
        self.bored_owner_boundary_debug(npc_id, "after_normal_timer");

        observe_npc_post_detection_tail_phase(npc_id, NpcPostDetectionTailPhase::MacroTimer);
        self.tick_ai_macro_timer_for_npc(sim, npc_id, assets);

        observe_npc_post_detection_tail_phase(npc_id, NpcPostDetectionTailPhase::Emoticon);
        self.tick_npc_emoticon_expiration_for_npc(npc_id);

        observe_npc_post_detection_tail_phase(npc_id, NpcPostDetectionTailPhase::QueuedStimuli);
        self.tick_ai_queued_stimuli_for_npc(sim, npc_id, assets);
        self.bored_owner_boundary_debug(npc_id, "exit");
    }

    /// Per-owner normal-timer phase. Carries the owner span for the
    /// synchronous `Think(EVENT_TIMER)` dispatch.
    ///
    /// Handles both soldiers (enemy AI) and civilians (friendly AI).
    /// `Think(EVENT_TIMER)` fires for every NPC whose timer has
    /// elapsed regardless of actor kind; civilians use timer launch
    /// from `WonderingCivilianAdmiringHero` /
    /// `WonderingCivilianEnemyReactiontime` and would otherwise stick
    /// in those substates indefinitely.
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    pub(crate) fn tick_ai_normal_timer_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        let current_frame = self.control.frame_counter;
        // Snapshot the state we need (immut borrow).  `ai_controller`
        // returns the base controller for both soldiers and civilians.
        let timer_fires = {
            let ai = self
                .world
                .entities
                .expect_ai_controller(npc_id, format_args!("normal-timer NPC"));

            ai.timer_is_running
                && (ai.when_does_timer_ring <= current_frame
                    || ai.when_does_timer_ring > current_frame.wrapping_add(1_000_000))
        };
        if !timer_fires {
            return;
        }
        self.world
            .entities
            .expect_ai_controller_mut(npc_id, format_args!("normal-timer NPC before Think"))
            .timer_is_running = false;
        let timer_stimulus = crate::ai::Stimulus::new(crate::ai::StimulusType::EventTimer);
        self.execute_ai_callback(sim, assets, npc_id, &timer_stimulus);
    }

    /// P6c — drain `pending_*` AI swordfight / order flags for every NPC.
    /// AI decisions set flags on `AiController`; we consume them here
    /// after all think calls are done, since they require engine-side
    /// entity mutations (opponent lists, sequences).
    #[cfg(test)]
    pub(super) fn tick_enemy_ai_drain_swordfight_requests(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        let npc_ids: Vec<_> = self.world.entities.ai_owner_ids().collect();
        for npc_id in npc_ids {
            self.drain_pending_for_npc(sim, npc_id, assets);
        }
    }

    /// P6d — replay deferred `pending_stimuli` for every NPC.
    ///
    /// Combat events (EVENT_GOOD_STRIKE, EVENT_LETHAL_STRIKE,
    /// EVENT_ENTER_SWORDFIGHT, etc.) are queued on
    /// `AiController::outbox.detection.stimuli` by `dispatch_ai_stimulus()`
    /// during the combat tick.  We defer them to avoid re-entrant
    /// borrow issues, then replay them now.
    #[cfg(test)]
    pub(super) fn tick_enemy_ai_drain_pending_stimuli(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        let npc_ids: Vec<_> = self.world.entities.ai_owner_ids().collect();
        for npc_id in npc_ids {
            self.tick_enemy_ai_drain_pending_stimuli_for_npc(sim, npc_id, assets);
        }
    }

    /// Run the base-actor `Execute` combat-injury Think synchronously without
    /// stealing older work from the NPC's ordinary deferred stimulus FIFO.
    /// Any stimuli emitted by the Think are restored behind that older work.
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    pub(in crate::engine) fn dispatch_combat_injury_think_for_actor_hourglass(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        self.dispatch_synchronous_ai_think_preserving_detection_fifo(
            sim,
            npc_id,
            assets,
            crate::ai::Stimulus::new(crate::ai::StimulusType::EventAfterCombatInjury),
        );
    }

    /// Run one legacy synchronous NPC Think while preserving older deferred
    /// detection stimuli ahead of anything emitted by that Think.
    pub(in crate::engine) fn dispatch_synchronous_ai_think_preserving_detection_fifo(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
        stimulus: crate::ai::Stimulus,
    ) {
        let mut preexisting = {
            let ai = self.world.entities.expect_ai_controller_mut(
                npc_id,
                format_args!("synchronous Think before detaching its stimulus FIFO"),
            );
            std::mem::take(&mut ai.outbox.detection.stimuli)
        };

        self.dispatch_ai_stimulus(npc_id, stimulus);
        self.tick_enemy_ai_drain_pending_stimuli_for_npc(sim, npc_id, assets);

        // This FIFO was detached during the synchronous call, so deletion's
        // owner hooks could not reach it. Do not restore newly stale targets.
        preexisting.retain(|stimulus| {
            stimulus
                .info
                .live_target()
                .is_none_or(|target| self.world.entities.get_legacy_slot(target.get()).is_some())
        });
        let ai = self.world.entities.expect_ai_controller_mut(
            npc_id,
            format_args!("synchronous Think before restoring its stimulus FIFO"),
        );
        preexisting.append(&mut ai.outbox.detection.stimuli);
        ai.outbox.detection.stimuli = preexisting;
    }

    /// P6d inner — per-NPC body of [`Self::tick_enemy_ai_drain_pending_stimuli`].
    /// Replays deferred stimuli for one NPC; carries the per-NPC tracing
    /// span so the `dispatch_think_with_drain` events emit with `npc=<id>`.
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    pub(in crate::engine) fn tick_enemy_ai_drain_pending_stimuli_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        let stimuli = {
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                return;
            };
            let Some(ai) = entity.ai_controller_mut() else {
                return;
            };
            std::mem::take(&mut ai.outbox.detection.stimuli)
        };
        if stimuli.is_empty() {
            return;
        }
        for (queue_index, stimulus) in stimuli.into_iter().enumerate() {
            self.debug_building_exit_wait_event_view(npc_id, queue_index, &stimulus);
            tracing::trace!(
                npc = npc_id.index(),
                queue_index,
                stimulus_type = ?stimulus.stimulus_type,
                stimulus_info = ?stimulus.info,
                "dispatching detection-refresh stimulus"
            );
            if stimulus
                .info
                .live_target()
                .is_some_and(|target| self.world.entities.get_legacy_slot(target.get()).is_none())
            {
                tracing::warn!(npc = npc_id.index(), info = ?stimulus.info,
                    "dropping detached detection stimulus after its target left the live world");
                continue;
            }
            if self.world.entities.get(npc_id).is_none() {
                break;
            }
            let target_override = match stimulus.info {
                crate::ai::StimulusInfo::Human(handle)
                    if matches!(
                        stimulus.stimulus_type,
                        crate::ai::StimulusType::EventView
                            | crate::ai::StimulusType::EventOutOfView
                            | crate::ai::StimulusType::EventSeesBeggar
                            | crate::ai::StimulusType::EventEnemyNear
                    ) =>
                {
                    Some(self.expect_entity_id_for_index(handle.get(), "queued detection target"))
                }
                _ => None,
            };
            // Production reaches this FIFO from detection refresh in the NPC
            // tail, after the actor's Execute slot has already run. Face/Turn
            // side effects are synchronous as sequence registration, but the
            // newly registered standalone Turn is not instructed until the
            // later sequence-manager tick boundary. Focused/global
            // detection entry points have no owner-slot boundary to preserve.
            let trace_shadow_delivery = matches!(
                stimulus.stimulus_type,
                crate::ai::StimulusType::EventSeesShadow
            );
            if trace_shadow_delivery {
                let npc = self.world.entities.expect_ai_actor_data(
                    npc_id,
                    format_args!("shadow-event receiver before Think"),
                );
                let ai = self.world.entities.expect_ai_controller(
                    npc_id,
                    format_args!("shadow-event receiver before Think"),
                );
                tracing::trace!(
                    target: "shadow_delivery",
                    frame = self.control.frame_counter,
                    phase = "before",
                    receiver = ?npc_id,
                    receiver_index = npc_id.index(),
                    queue_index,
                    stimulus_info = ?stimulus.info,
                    to_whole_patrol = stimulus.to_whole_patrol,
                    state = ?ai.current_state,
                    substate = ?ai.current_substate,
                    patrol_chief = ?ai.patrol_chief,
                    patrol_members = ?ai.patrol,
                    detection_suspects = ?npc.detection_suspects,
                    maximal_detection_suspect = npc.maximal_detection_suspect,
                    maximal_visibility = ai.max_visibility,
                    "delivering shadow event to AI"
                );
            }
            self.dispatch_think_with_drain(sim, npc_id, &stimulus, target_override, assets);
            if trace_shadow_delivery {
                let npc = self.world.entities.expect_ai_actor_data(
                    npc_id,
                    format_args!("shadow-event receiver after Think"),
                );
                let ai = self.world.entities.expect_ai_controller(
                    npc_id,
                    format_args!("shadow-event receiver after Think"),
                );
                tracing::trace!(
                    target: "shadow_delivery",
                    frame = self.control.frame_counter,
                    phase = "after",
                    receiver = ?npc_id,
                    receiver_index = npc_id.index(),
                    queue_index,
                    stimulus_info = ?stimulus.info,
                    to_whole_patrol = stimulus.to_whole_patrol,
                    state = ?ai.current_state,
                    substate = ?ai.current_substate,
                    patrol_chief = ?ai.patrol_chief,
                    patrol_members = ?ai.patrol,
                    detection_suspects = ?npc.detection_suspects,
                    maximal_detection_suspect = npc.maximal_detection_suspect,
                    maximal_visibility = ai.max_visibility,
                    "finished shadow-event AI delivery"
                );
            }
        }
    }

    /// Drain stimuli retained by `start_think` while an NPC was AI- or
    /// script-locked. This is the final unlocked phase of
    /// the original-game NPC update, after both timer kinds.
    #[cfg(test)]
    pub(crate) fn tick_ai_queued_stimuli(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        if self.actors_frozen() {
            return;
        }

        let npc_ids: Vec<_> = self.world.entities.ai_owner_ids().collect();
        for npc_id in npc_ids {
            self.tick_ai_queued_stimuli_for_npc(sim, npc_id, assets);
        }
    }

    pub(crate) fn tick_ai_queued_stimuli_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        self.tick_ai_queued_stimuli_for_npc_limit(sim, npc_id, assets, None);
    }

    pub(crate) fn tick_one_ai_queued_stimulus_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        self.tick_ai_queued_stimuli_for_npc_limit(sim, npc_id, assets, Some(1));
    }

    fn tick_ai_queued_stimuli_for_npc_limit(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
        limit: Option<usize>,
    ) {
        let mut processed = 0usize;
        loop {
            let stimulus = {
                let ai = self
                    .world
                    .entities
                    .expect_ai_controller_mut(npc_id, format_args!("retained-FIFO NPC"));
                // A previous queued Think may acquire a new lock. The
                // original loop stops immediately and preserves the rest.
                if !ai.locks_flag_field.is_empty() || ai.script_locked {
                    break;
                }
                if ai.stimulus_queue.is_empty() {
                    break;
                }
                ai.stimulus_queue.remove(0)
            };

            let target_override = match stimulus.info {
                crate::ai::StimulusInfo::Human(handle)
                    if matches!(
                        stimulus.stimulus_type,
                        crate::ai::StimulusType::EventView
                            | crate::ai::StimulusType::EventOutOfView
                            | crate::ai::StimulusType::EventSeesBeggar
                            | crate::ai::StimulusType::EventEnemyNear
                    ) =>
                {
                    Some(self.entity_id_for_index(handle.get()).unwrap_or_else(|| {
                        panic!(
                            "retained {:?} for NPC {} references missing entity {}",
                            stimulus.stimulus_type,
                            npc_id.index(),
                            handle
                        )
                    }))
                }
                _ => None,
            };
            self.dispatch_think_with_drain(sim, npc_id, &stimulus, target_override, assets);
            processed += 1;
            if limit.is_some_and(|limit| processed >= limit) {
                return;
            }
        }
    }
}
