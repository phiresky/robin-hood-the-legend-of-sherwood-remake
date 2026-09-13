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

/// The only lists retained from detection; all other tactical fields are read
/// live at delivery. Keeping this type narrow prevents rebuilding and cloning
/// a whole tactical world merely to discard it at the next Think.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(in crate::engine) struct EnemyDetectionAggregate {
    pub(super) nearby_sleeping_enemies: Vec<crate::ai::SleepingEnemyInfo>,
    pub(super) camp_unconscious_soldiers: Vec<crate::ai_enemy::CampUnconsciousSoldierInfo>,
}

/// Attach the retained lists to the exact contiguous Enemy stimulus block.
/// Queue identity remains valid even when an earlier callback deletes a target.
#[derive(serde::Serialize, serde::Deserialize)]
pub(in crate::engine) struct PendingEnemyDetection {
    pub(super) queue_start: usize,
    pub(super) stimuli: Vec<crate::ai::Stimulus>,
    pub(super) aggregate: EnemyDetectionAggregate,
    matched: usize,
}

fn overlay_final_detection_scan(
    live: &mut crate::ai::AiPerTickData,
    aggregate: &EnemyDetectionAggregate,
) {
    // Enemy list products are deliberately not copied here. Original
    // Enemy-list rebuilding re-walks the live detectable list during every
    // Think, including mutations made synchronously by a preceding queued
    // Think/script. `overlay_live_enemy_detection_scan_for_think` rebuilds
    // those fields at the exact FIFO boundary.
    live.nearby_sleeping_enemies = aggregate.nearby_sleeping_enemies.clone();

    // These are also products of the completed detectable-list
    // walk rather than properties of the stimulus target.
    live.camp_unconscious_soldiers = aggregate.camp_unconscious_soldiers.clone();
}

fn enemy_detection_handles(
    detectables: &[crate::element::Detectable],
    npc_id: EntityId,
) -> (Vec<EntityId>, Vec<EntityId>) {
    let mut visible = Vec::new();
    let mut latched = Vec::new();
    for detectable in detectables {
        if !detectable.seen_now && !detectable.seen_last_frame {
            continue;
        }
        let target = detectable.element.unwrap_or_else(|| {
            panic!(
                "visible/latched Enemy detectable for NPC {} has no target",
                npc_id.index()
            )
        });
        if detectable.seen_now {
            visible.push(target);
        }
        if detectable.seen_last_frame {
            latched.push(target);
        }
    }
    (visible, latched)
}

impl PendingEnemyDetection {
    pub(super) fn new(
        queue_start: usize,
        stimuli: Vec<crate::ai::Stimulus>,
        aggregate: EnemyDetectionAggregate,
    ) -> Self {
        Self {
            queue_start,
            stimuli,
            aggregate,
            matched: 0,
        }
    }
}

fn take_enemy_detection_aggregate<'a>(
    queue_index: usize,
    stimulus: &crate::ai::Stimulus,
    pending: &'a mut Option<PendingEnemyDetection>,
) -> Option<&'a EnemyDetectionAggregate> {
    let override_data = pending.as_mut()?;
    let offset = queue_index.checked_sub(override_data.queue_start)?;
    let expected = override_data.stimuli.get(offset)?;
    assert_eq!(
        stimulus.stimulus_type, expected.stimulus_type,
        "Enemy detection tick-data block no longer points at its queued stimulus type"
    );
    assert_eq!(
        stimulus.info, expected.info,
        "Enemy detection tick-data block no longer points at its queued stimulus target"
    );
    assert_eq!(
        stimulus.owner, expected.owner,
        "Enemy detection tick-data block no longer points at its queued stimulus owner"
    );
    assert_eq!(
        stimulus.to_whole_patrol, expected.to_whole_patrol,
        "Enemy detection tick-data block no longer points at its queued patrol routing"
    );
    override_data.matched += 1;
    Some(&override_data.aggregate)
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
        // Every synchronous Think boundary receives a fresh RNG-free view of
        // the live world. Forecast alternatives are prepared below and only
        // the handler that consumes one resolves it.
        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        // Build the rich tick data from the centralized builder
        // — covers primary target metadata, friend-swap
        // candidates, avenger-on-roof wait position, and seeded
        // enemy_sq_distances.  Matches (and supersedes) the
        // bespoke hand-roll this block used to do.
        let tick_data = self.build_npc_tick_data(sim, npc_id, assets);

        // Build ctx and stop the timer under a single mut borrow.
        let in_uninterruptible_command = self.is_very_very_busy(npc_id);
        let building_sector = self
            .world
            .entities
            .get(npc_id)
            .map(|entity| self.entity_building_sector(entity.element_data().sector()))
            .unwrap_or_else(|| panic!("normal-timer NPC {} disappeared", npc_id.index()));
        let ctx = {
            let entity = self.expect_entity(npc_id, "normal-timer NPC before Think");
            let mut ctx = self.ai_context_from_entity(
                entity,
                current_frame,
                building_sector,
                &scratch,
                assets,
            );
            ctx.in_uninterruptible_command = in_uninterruptible_command;
            ctx.enter_swordfight_pending = self
                .orders
                .sequence_manager
                .element_is_about_to_be_launched_or_postponed_by_current(
                    npc_id,
                    crate::element::Command::EnterSwordfight,
                );
            // Clear `timer_is_running` before dispatching
            // `Think(EVENT_TIMER)`.
            let ai = self.world.entities.expect_ai_controller_mut(
                npc_id,
                format_args!(
                    "normal-timer NPC {} lost its AI controller before Think",
                    npc_id.index()
                ),
            );
            ai.timer_is_running = false;
            ctx
        };

        let timer_stimulus = crate::ai::Stimulus::new(crate::ai::StimulusType::EventTimer);
        self.dispatch_think_with_drain_without_forecast(
            sim,
            npc_id,
            &timer_stimulus,
            &ctx,
            &tick_data,
            assets,
        );
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
            self.tick_enemy_ai_drain_pending_stimuli_for_npc(sim, npc_id, assets, None);
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
        self.tick_enemy_ai_drain_pending_stimuli_for_npc(sim, npc_id, assets, None);

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
        mut enemy_detection_tick_data: Option<PendingEnemyDetection>,
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
            // Consume the matching retained scan record even if a preceding
            // synchronous stimulus removed this target and delivery below is
            // consequently skipped.
            let detection_aggregate = take_enemy_detection_aggregate(
                queue_index,
                &stimulus,
                &mut enemy_detection_tick_data,
            );
            if stimulus
                .info
                .live_target()
                .is_some_and(|target| self.world.entities.get_legacy_slot(target.get()).is_none())
            {
                // Consume the scan record first: pruning this local batch
                // would shift the absolute queue indices used by the scan.
                tracing::warn!(npc = npc_id.index(), info = ?stimulus.info,
                    "dropping detached detection stimulus after its target left the live world");
                continue;
            }
            // The original game's decision tick is a synchronous boundary. Its completion (and any
            // recursive event it launches) finishes before the next queued
            // stimulus starts, so every entry must observe mutations made by
            // its predecessor rather than the tick-start entity-view map.
            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            if let crate::ai::StimulusInfo::Human(handle) = stimulus.info
                && !scratch.ai_entity_views.contains_key(&handle.get())
            {
                // A preceding synchronous stimulus can kill/remove this
                // target before the next queued detection stimulus runs.
                // Entity removal prunes the controller-owned queues, but this
                // batch has already been taken off the controller. Revalidate
                // each delivery after its predecessor's synchronous callbacks.
                tracing::warn!(
                    npc = npc_id.index(),
                    target = handle.get(),
                    stimulus_type = ?stimulus.stimulus_type,
                    "dropping queued detection stimulus after its target left the live world"
                );
                continue;
            }
            let in_uninterruptible_command = self.is_very_very_busy(npc_id);
            let mut ctx = {
                let Some(entity) = self.world.entities.get(npc_id) else {
                    break;
                };
                let entity_sector = entity.element_data().sector();
                let building_sector = self.entity_building_sector(entity_sector);
                let Some(entity) = self.world.entities.get(npc_id) else {
                    break;
                };
                let mut ctx = self.ai_context_from_entity(
                    entity,
                    self.control.frame_counter,
                    building_sector,
                    &scratch,
                    assets,
                );
                ctx.in_uninterruptible_command = in_uninterruptible_command;
                if let crate::ai::StimulusInfo::Human(handle) = stimulus.info {
                    let Some(view) = ctx.entity_view(handle.get()) else {
                        tracing::warn!(
                            npc = npc_id.index(),
                            target = handle.get(),
                            stimulus_type = ?stimulus.stimulus_type,
                            "dropping queued detection stimulus after its target left the typed live view"
                        );
                        continue;
                    };
                    ctx.antagonist = Some(crate::ai::AntagonistInfo {
                        position: view.position,
                        camp: view.camp,
                        is_swordfighting: view.is_swordfighting,
                        is_pc: view.is_pc,
                        is_robin: view.is_robin,
                        is_vip: view.is_vip,
                        in_building: view.in_building,
                    });
                }
                ctx
            };
            self.refresh_selected_default_wait_identity(npc_id, &mut ctx);
            // The Enemy VIEW / OUTOFVIEW block retains the completed scan
            // aggregate, but all tactical and target-specific inputs are
            // rebuilt from the live world for this exact stimulus.
            let mut tick_data = if let Some(aggregate) = detection_aggregate {
                let target_id = match stimulus.info {
                    crate::ai::StimulusInfo::Human(handle) => {
                        self.entity_id_for_index(handle.get()).unwrap_or_else(|| {
                            panic!(
                                "Enemy detection {:?} for NPC {} references missing entity {}",
                                stimulus.stimulus_type,
                                npc_id.index(),
                                handle
                            )
                        })
                    }
                    _ => panic!(
                        "Enemy detection {:?} for NPC {} has no human target",
                        stimulus.stimulus_type,
                        npc_id.index()
                    ),
                };
                let mut live =
                    self.build_npc_tick_data_for_target(sim, npc_id, assets, Some(target_id));
                overlay_final_detection_scan(&mut live, aggregate);

                live
            } else {
                let target_override = match stimulus.info {
                    crate::ai::StimulusInfo::Human(handle)
                        if matches!(
                            stimulus.stimulus_type,
                            crate::ai::StimulusType::EventView
                                | crate::ai::StimulusType::EventSeesBeggar
                                | crate::ai::StimulusType::EventEnemyNear
                        ) =>
                    {
                        Some(self.entity_id_for_index(handle.get()).unwrap_or_else(|| {
                            panic!(
                                "queued {:?} for NPC {} references missing entity {}",
                                stimulus.stimulus_type,
                                npc_id.index(),
                                handle
                            )
                        }))
                    }
                    _ => None,
                };
                self.build_npc_tick_data_for_target(sim, npc_id, assets, target_override)
            };
            // Detection delivery forecasts the actor's actual primary target,
            // which can differ from this stimulus's tactical target override.
            self.prepare_detection_forecasts_for_owner(npc_id, &mut tick_data);
            if matches!(
                stimulus.stimulus_type,
                crate::ai::StimulusType::EventView | crate::ai::StimulusType::EventOutOfView
            ) {
                // Only one queued Enemy stimulus owns the completed scan
                // aggregate, but every synchronous VIEW/OUTOFVIEW Think reads
                // the authoritative live detectable list. Rebuilding only the
                // aggregate-owning entry lets a later falling-edge event
                // resurrect geometrically visible enemies whose `seen_now`
                // latch has already been cleared.
                self.overlay_live_enemy_detection_scan_for_think(npc_id, &scratch, &mut tick_data);
            }
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
            self.dispatch_think_with_drain_mode(
                sim,
                npc_id,
                &stimulus,
                &ctx,
                &tick_data,
                assets,
                crate::engine::ai::OwnerBoundaryPolicy::Current,
            );
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
        if let Some(override_data) = enemy_detection_tick_data {
            assert_eq!(
                override_data.matched,
                override_data.stimuli.len(),
                "Enemy detection tick-data block did not match every queued stimulus"
            );
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

            // Every retained Think is a fresh synchronous boundary. An
            // earlier replay may mutate positions, latches, or targets
            // consumed by the next retained stimulus.
            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let in_uninterruptible_command = self.is_very_very_busy(npc_id);
            let ctx = {
                let entity = self.expect_entity(npc_id, "retained-FIFO NPC before Think");
                let building_sector = self.entity_building_sector(entity.element_data().sector());
                let mut ctx = self.ai_context_from_entity(
                    entity,
                    self.control.frame_counter,
                    building_sector,
                    &scratch,
                    assets,
                );
                ctx.in_uninterruptible_command = in_uninterruptible_command;
                if let crate::ai::StimulusInfo::Human(handle) = stimulus.info {
                    let view = ctx.entity_view(handle.get()).unwrap_or_else(|| {
                        panic!(
                            "retained {:?} for NPC {} references missing entity {}",
                            stimulus.stimulus_type,
                            npc_id.index(),
                            handle
                        )
                    });
                    ctx.antagonist = Some(crate::ai::AntagonistInfo {
                        position: view.position,
                        camp: view.camp,
                        is_swordfighting: view.is_swordfighting,
                        is_pc: view.is_pc,
                        is_robin: view.is_robin,
                        is_vip: view.is_vip,
                        in_building: view.in_building,
                    });
                }
                ctx
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
            let mut tick_data =
                self.build_npc_tick_data_for_target(sim, npc_id, assets, target_override);
            if matches!(
                stimulus.stimulus_type,
                crate::ai::StimulusType::EventView | crate::ai::StimulusType::EventOutOfView
            ) {
                self.overlay_live_enemy_detection_scan_for_think(npc_id, &scratch, &mut tick_data);
                // A retained OUTOFVIEW can still reach the lost-enemy body,
                // which forecasts the destination of the human the stimulus
                // carries — not necessarily the current primary target. The
                // generic tick builder only prepares primary/missed
                // forecasts, so add the per-detectable ones the handler
                // indexes by handle.
                self.prepare_detection_forecasts_for_owner(npc_id, &mut tick_data);
            }
            self.dispatch_think_with_drain_without_forecast(
                sim, npc_id, &stimulus, &ctx, &tick_data, assets,
            );
            processed += 1;
            if limit.is_some_and(|limit| processed >= limit) {
                return;
            }
        }
    }

    /// Rebuild the Enemy-list products that queued VIEW/OUTOFVIEW
    /// handlers read when rebuilding enemy lists at every AI decision boundary.
    /// This covers both the immediate detection FIFO and retained
    /// stimuli replayed later: in either case read `seen_now`
    /// from the live detectable list, not a frozen scan aggregate.
    fn overlay_live_enemy_detection_scan_for_think(
        &self,
        npc_id: EntityId,
        scratch: &SimScratch,
        tick_data: &mut crate::ai::AiPerTickData,
    ) {
        let (observer_position, visible_targets, latched_targets) = {
            let entity = self.expect_entity(npc_id, "NPC before live Enemy-list reconstruction");
            let npc = self.world.entities.expect_ai_actor_data(
                npc_id,
                format_args!("NPC before live Enemy-list reconstruction"),
            );
            let enemy_idx = crate::element::DetectableType::Enemy as usize;
            let (visible_targets, latched_targets) =
                enemy_detection_handles(&npc.detectable_lists[enemy_idx], npc_id);
            (
                super::detection::human_eye_point_for_visibility(entity).0,
                visible_targets,
                latched_targets,
            )
        };

        tick_data.enemy_sq_distances.clear();
        tick_data.min_sq_enemy_distance = i32::MAX;
        tick_data.personally_visible_enemies = 0;
        tick_data.unconscious_enemies.clear();
        tick_data.seen_last_frame_enemies = latched_targets
            .iter()
            .map(|target| target.index())
            .collect();

        for target_id in visible_targets {
            let target = scratch
                .ai_entity_views
                .get(&target_id.index())
                .unwrap_or_else(|| {
                    panic!(
                        "latched Enemy target {} for NPC {} is absent from queued replay views",
                        target_id.index(),
                        npc_id.index()
                    )
                });
            if target.is_unconscious {
                if !target.is_carried {
                    tick_data
                        .unconscious_enemies
                        .push(crate::ai::SleepingEnemyInfo {
                            handle: target_id.index(),
                            position: target.position,
                            is_pc: target.is_pc,
                            is_robin: target.is_robin,
                            is_vip: target.is_vip,
                        });
                }
                continue;
            }
            let dx = target.position.x - observer_position.x;
            let dy = (target.position.y - observer_position.y)
                * crate::position_interface::INVERSE_ASPECT_RATIO;
            let sq_distance = (dx * dx + dy * dy) as i32;
            tick_data
                .enemy_sq_distances
                .push((target_id.index(), sq_distance));
            tick_data.min_sq_enemy_distance = tick_data.min_sq_enemy_distance.min(sq_distance);
        }
        tick_data.personally_visible_enemies = tick_data.enemy_sq_distances.len() as u16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enemy_detectable(
        handle: u32,
        seen_now: bool,
        seen_last_frame: bool,
    ) -> crate::element::Detectable {
        crate::element::Detectable {
            element: Some(EntityId::Soldier(crate::entity_id::SoldierId(handle))),
            detectable_type: crate::element::DetectableType::Enemy,
            seen_now,
            seen_last_frame,
            ..Default::default()
        }
    }

    #[test]
    fn reinitialize_them_inputs_use_seen_now_and_preserve_detectable_order() {
        let detectables = vec![
            enemy_detectable(9, true, false),
            enemy_detectable(4, false, true),
            enemy_detectable(7, true, true),
            enemy_detectable(2, false, false),
        ];
        let npc_id = EntityId::Soldier(crate::entity_id::SoldierId(1));

        let (visible, latched) = enemy_detection_handles(&detectables, npc_id);

        assert_eq!(
            visible.iter().map(|id| id.index()).collect::<Vec<_>>(),
            vec![9, 7],
            "enemy-list rebuilding must consume live visibility in list order"
        );
        assert_eq!(
            latched.iter().map(|id| id.index()).collect::<Vec<_>>(),
            vec![4, 7],
            "arrow-protection latches remain distinct from live visibility"
        );
    }

    #[test]
    fn final_scan_overlay_does_not_refreeze_live_enemy_list_products() {
        let mut live = crate::ai::AiPerTickData::stub();
        live.enemy_sq_distances = vec![(9, 81)];
        live.min_sq_enemy_distance = 81;
        live.personally_visible_enemies = 1;
        live.unconscious_enemies = vec![crate::ai::SleepingEnemyInfo {
            handle: 7,
            position: crate::ai::Position::default(),
            is_pc: false,
            is_robin: false,
            is_vip: false,
        }];

        let mut aggregate = EnemyDetectionAggregate::default();
        aggregate.nearby_sleeping_enemies = vec![crate::ai::SleepingEnemyInfo {
            handle: 3,
            position: crate::ai::Position::default(),
            is_pc: false,
            is_robin: false,
            is_vip: false,
        }];

        overlay_final_detection_scan(&mut live, &aggregate);

        assert_eq!(live.enemy_sq_distances, vec![(9, 81)]);
        assert_eq!(live.min_sq_enemy_distance, 81);
        assert_eq!(live.personally_visible_enemies, 1);
        assert_eq!(live.unconscious_enemies[0].handle, 7);
        assert_eq!(live.nearby_sleeping_enemies[0].handle, 3);
    }

    #[test]
    fn enemy_detection_tick_data_override_matches_the_exact_fifo_block() {
        let aggregate = EnemyDetectionAggregate {
            nearby_sleeping_enemies: vec![crate::ai::SleepingEnemyInfo {
                handle: 7,
                position: crate::ai::Position::default(),
                is_pc: true,
                is_robin: false,
                is_vip: false,
            }],
            camp_unconscious_soldiers: vec![crate::ai_enemy::CampUnconsciousSoldierInfo {
                handle: 321,
                knocked_out_in_money_fight: true,
            }],
        };
        let shadow = crate::ai::Stimulus::with_position(
            crate::ai::StimulusType::EventSeesShadow,
            crate::ai::Position::default(),
        );
        let view = crate::ai::Stimulus::with_human(crate::ai::StimulusType::EventView, 42);
        let out_of_view =
            crate::ai::Stimulus::with_human(crate::ai::StimulusType::EventOutOfView, 77);
        let mut pending = Some(PendingEnemyDetection::new(
            1,
            vec![view, out_of_view],
            aggregate,
        ));

        assert!(take_enemy_detection_aggregate(0, &shadow, &mut pending).is_none());
        let selected = take_enemy_detection_aggregate(1, &view, &mut pending)
            .expect("exact EVENT_VIEW queue entry keeps detection-built input");
        assert_eq!(selected.nearby_sleeping_enemies[0].handle, 7);
        assert_eq!(selected.camp_unconscious_soldiers[0].handle, 321);
        let selected = take_enemy_detection_aggregate(2, &out_of_view, &mut pending)
            .expect("exact EVENT_OUTOFVIEW queue entry keeps detection-built input");
        assert_eq!(selected.nearby_sleeping_enemies[0].handle, 7);
        assert_eq!(
            pending.as_ref().expect("block remains for audit").matched,
            2
        );
    }

    #[test]
    fn event_view_tick_data_override_is_one_shot_at_exact_fifo_index() {
        let aggregate = EnemyDetectionAggregate {
            nearby_sleeping_enemies: vec![crate::ai::SleepingEnemyInfo {
                handle: 7,
                position: crate::ai::Position::default(),
                is_pc: true,
                is_robin: false,
                is_vip: false,
            }],
            camp_unconscious_soldiers: vec![crate::ai_enemy::CampUnconsciousSoldierInfo {
                handle: 321,
                knocked_out_in_money_fight: true,
            }],
        };
        let shadow = crate::ai::Stimulus::with_position(
            crate::ai::StimulusType::EventSeesShadow,
            crate::ai::Position::default(),
        );
        let view = crate::ai::Stimulus::with_human(crate::ai::StimulusType::EventView, 42);
        let mut pending = Some(PendingEnemyDetection::new(1, vec![view], aggregate));

        assert!(take_enemy_detection_aggregate(0, &shadow, &mut pending).is_none());
        let selected = take_enemy_detection_aggregate(1, &view, &mut pending)
            .expect("exact EVENT_VIEW queue entry keeps detection-built input");
        assert_eq!(selected.nearby_sleeping_enemies[0].handle, 7);
        assert_eq!(selected.camp_unconscious_soldiers[0].handle, 321);
        assert_eq!(
            pending.as_ref().expect("block remains for audit").matched,
            1
        );
        assert!(take_enemy_detection_aggregate(2, &view, &mut pending).is_none());
    }
}
