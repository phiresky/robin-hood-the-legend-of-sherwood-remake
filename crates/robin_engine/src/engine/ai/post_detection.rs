//! Creation-ordered post-detection NPC update tail.

use super::*;
use crate::engine::TickCtx;

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
        let ai = self.entities().expect_ai_controller(
            npc_id,
            format_args!("BORED_BOUNDARY owner {owner} during {phase}"),
        );
        eprintln!(
            "BORED_BOUNDARY frame={} owner={} phase={} command={:?} state={:?} substate={:?} timer_running={} timer_deadline={}",
            frame,
            owner,
            phase,
            command,
            ai.current_state,
            ai.current_substate,
            ai.timer_is_running,
            ai.when_does_timer_ring,
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
        tcx: TickCtx<'_>,
        npc_id: EntityId,
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
        self.tick_refresh_ambush_points_for_npc(tcx, npc_id);

        observe_npc_post_detection_tail_phase(npc_id, NpcPostDetectionTailPhase::Deafness);
        self.tick_npc_refresh_deafness_for_npc(npc_id);

        observe_npc_post_detection_tail_phase(npc_id, NpcPostDetectionTailPhase::Busy);
        self.tick_npc_busy_edge_detect_for_npc(npc_id);

        observe_npc_post_detection_tail_phase(npc_id, NpcPostDetectionTailPhase::Ladder);
        self.tick_npc_stuck_on_ladder_for_npc(tcx, npc_id);

        observe_npc_post_detection_tail_phase(npc_id, NpcPostDetectionTailPhase::RandomSpeech);
        self.tick_civilian_random_speech_for_npc(tcx, npc_id);

        observe_npc_post_detection_tail_phase(npc_id, NpcPostDetectionTailPhase::LockGate);
        if self.tick_npc_lock_gate_for_npc(npc_id) {
            self.bored_owner_boundary_debug(npc_id, "lock_gate_return");
            return;
        }

        observe_npc_post_detection_tail_phase(npc_id, NpcPostDetectionTailPhase::SixteenthFrame);
        self.tick_periodic_ai_for_npc(tcx, npc_id);
        self.bored_owner_boundary_debug(npc_id, "after_periodic");

        observe_npc_post_detection_tail_phase(npc_id, NpcPostDetectionTailPhase::NormalTimer);
        self.tick_ai_normal_timer_for_npc(tcx, npc_id);
        self.bored_owner_boundary_debug(npc_id, "after_normal_timer");

        observe_npc_post_detection_tail_phase(npc_id, NpcPostDetectionTailPhase::MacroTimer);
        self.tick_ai_macro_timer_for_npc(tcx, npc_id);

        observe_npc_post_detection_tail_phase(npc_id, NpcPostDetectionTailPhase::Emoticon);
        self.tick_npc_emoticon_expiration_for_npc(npc_id);

        observe_npc_post_detection_tail_phase(npc_id, NpcPostDetectionTailPhase::QueuedStimuli);
        self.tick_ai_queued_stimuli_for_npc(tcx, npc_id);
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
    pub(crate) fn tick_ai_normal_timer_for_npc(&mut self, tcx: TickCtx<'_>, npc_id: EntityId) {
        let current_frame = self.control.frame_counter;
        // Snapshot the state we need (immut borrow).  `ai_controller`
        // returns the base controller for both soldiers and civilians.
        let timer_fires = {
            let ai = self.ai(npc_id, "normal-timer NPC");

            ai.timer_is_running
                && (ai.when_does_timer_ring <= current_frame
                    || ai.when_does_timer_ring > current_frame.wrapping_add(1_000_000))
        };
        if !timer_fires {
            return;
        }
        self.ai_mut(npc_id, "normal-timer NPC before Think")
            .timer_is_running = false;
        let timer_stimulus = crate::ai::Stimulus::new(crate::ai::StimulusType::EventTimer);
        self.execute_ai_callback(tcx, npc_id, &timer_stimulus);
    }

    /// Deliver the actor's combat-injury callback at its current execution slot.
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    pub(in crate::engine) fn dispatch_combat_injury_think_for_actor_hourglass(
        &mut self,
        tcx: TickCtx<'_>,
        npc_id: EntityId,
    ) {
        self.execute_ai_callback(
            tcx,
            npc_id,
            &crate::ai::Stimulus::new(crate::ai::StimulusType::EventAfterCombatInjury),
        );
    }

    /// Deliver the local optical scan batch after every detectable bucket finishes.
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    pub(in crate::engine) fn dispatch_optical_stimuli(
        &mut self,
        tcx: TickCtx<'_>,
        npc_id: EntityId,
        stimuli: Vec<crate::ai::Stimulus>,
    ) {
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
                .is_some_and(|target| self.entities().get_legacy_slot(target.get()).is_none())
            {
                tracing::warn!(npc = npc_id.index(), info = ?stimulus.info,
                    "dropping detached detection stimulus after its target left the live world");
                continue;
            }
            if self.entities().get(npc_id).is_none() {
                break;
            }
            match stimulus.info {
                crate::ai::StimulusInfo::Human(handle)
                    if matches!(
                        stimulus.stimulus_type,
                        crate::ai::StimulusType::EventView
                            | crate::ai::StimulusType::EventOutOfView
                            | crate::ai::StimulusType::EventSeesBeggar
                            | crate::ai::StimulusType::EventEnemyNear
                    ) =>
                {
                    self.expect_entity_id_for_index(handle.get(), "queued detection target");
                }
                _ => {}
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
                let npc = self.ai_actor(npc_id, "shadow-event receiver before Think");
                let ai = self.ai(npc_id, "shadow-event receiver before Think");
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
            self.dispatch_think_with_drain(tcx, npc_id, &stimulus);
            if trace_shadow_delivery {
                let npc = self.ai_actor(npc_id, "shadow-event receiver after Think");
                let ai = self.ai(npc_id, "shadow-event receiver after Think");
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
    pub(crate) fn tick_ai_queued_stimuli(&mut self, tcx: TickCtx<'_>) {
        if self.actors_frozen() {
            return;
        }

        let npc_ids: Vec<_> = self.entities().ai_owner_ids().collect();
        for npc_id in npc_ids {
            self.tick_ai_queued_stimuli_for_npc(tcx, npc_id);
        }
    }

    pub(crate) fn tick_ai_queued_stimuli_for_npc(&mut self, tcx: TickCtx<'_>, npc_id: EntityId) {
        loop {
            let stimulus = {
                let ai = self.ai_mut(npc_id, "retained-FIFO NPC");
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

            match stimulus.info {
                crate::ai::StimulusInfo::Human(handle)
                    if matches!(
                        stimulus.stimulus_type,
                        crate::ai::StimulusType::EventView
                            | crate::ai::StimulusType::EventOutOfView
                            | crate::ai::StimulusType::EventSeesBeggar
                            | crate::ai::StimulusType::EventEnemyNear
                    ) =>
                {
                    self.entity_id_for_index(handle.get()).unwrap_or_else(|| {
                        panic!(
                            "retained {:?} for NPC {} references missing entity {}",
                            stimulus.stimulus_type,
                            npc_id.index(),
                            handle
                        )
                    });
                }
                _ => {}
            };
            self.dispatch_think_with_drain(tcx, npc_id, &stimulus);
        }
    }
}
