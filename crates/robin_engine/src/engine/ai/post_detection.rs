//! Post-detection orchestration phases for `tick_enemy_ai`:
//! P4 (alert allies / log), P6 (pursuit / approach / combat-stance), P6c
//! (drain pending swordfight requests), P6d (replay deferred stimuli).

use super::snapshots::Detection;
use super::*;

/// Final scan aggregate attached to the contiguous Enemy stimulus block queued
/// by `RefreshDetection`. The absolute queue start preserves FIFO order. Live
/// context and target-dependent combat fields are rebuilt for each Think; only
/// fields whose value belongs to the completed detection scan are copied from
/// this aggregate.
pub(super) struct PendingEnemyDetectionTickData {
    pub(super) queue_start: usize,
    pub(super) stimuli: Vec<crate::ai::Stimulus>,
    pub(super) tick_data: crate::ai::AiPerTickData,
    matched: usize,
}

fn overlay_final_detection_scan(
    live: &mut crate::ai::AiPerTickData,
    aggregate: &crate::ai::AiPerTickData,
) {
    // TODO(parity): original ReinitializeThemList can re-walk a detectable
    // list changed synchronously by a script between two Think calls. This
    // aggregate intentionally freezes RefreshDetection's completed scan; live
    // mid-FIFO detectable-list mutation needs a separate script-boundary test.
    live.enemy_sq_distances = aggregate.enemy_sq_distances.clone();
    live.min_sq_enemy_distance = aggregate.min_sq_enemy_distance;
    live.personally_visible_enemies = aggregate.personally_visible_enemies;
    live.unconscious_enemies = aggregate.unconscious_enemies.clone();
    live.nearby_sleeping_enemies = aggregate.nearby_sleeping_enemies.clone();
    live.seen_last_frame_enemies = aggregate.seen_last_frame_enemies.clone();

    // These are also products of RefreshDetection's completed detectable-list
    // walk rather than properties of the stimulus target.
    live.visible_seeking_friends = aggregate.visible_seeking_friends;
    live.friend_seek_clears_help_flag = aggregate.friend_seek_clears_help_flag;
    live.camp_ko_money_fighters = aggregate.camp_ko_money_fighters.clone();
}

impl PendingEnemyDetectionTickData {
    pub(super) fn new(
        queue_start: usize,
        stimuli: Vec<crate::ai::Stimulus>,
        tick_data: crate::ai::AiPerTickData,
    ) -> Self {
        Self {
            queue_start,
            stimuli,
            tick_data,
            matched: 0,
        }
    }
}

fn take_enemy_detection_tick_data(
    queue_index: usize,
    stimulus: &crate::ai::Stimulus,
    pending: &mut Option<PendingEnemyDetectionTickData>,
) -> Option<crate::ai::AiPerTickData> {
    let override_data = pending.as_mut()?;
    let Some(offset) = queue_index.checked_sub(override_data.queue_start) else {
        return None;
    };
    let Some(expected) = override_data.stimuli.get(offset) else {
        return None;
    };
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
    Some(override_data.tick_data.clone())
}
use crate::coordinates::MapPoint;
use crate::element::{Entity, EntityId};

impl EngineInner {
    /// P4 — fire `HeyFolksLookThere` + log on every fresh detection
    /// transition.  Alerts nearby idle soldiers when an NPC spots the
    /// PC.
    pub(super) fn tick_enemy_ai_alert_allies(&mut self, transitions: &[Detection]) {
        const VIEW_LOOK_THERE_RADIUS: f32 = 100.0;
        let alert_calls: Vec<(EntityId, MapPoint)> = transitions
            .iter()
            .filter(|d| d.newly_alerted)
            .map(|d| (d.enemy, d.target_pos))
            .collect();
        for det in transitions {
            if det.newly_alerted {
                tracing::info!(
                    enemy = ?det.enemy,
                    target = ?det.target,
                    "Enemy AI: spotted PC, transitioning to Attacking"
                );
            }
        }
        for (enemy, pos) in alert_calls {
            self.hey_folks_look_there(enemy, pos, VIEW_LOOK_THERE_RADIUS);
        }
    }

    /// Finalize newly committed detections by revealing blipped NPCs.
    pub(super) fn tick_enemy_ai_commit_detection_transitions(
        &mut self,
        transitions: Vec<Detection>,
    ) {
        // Newly committed detections — reveal blipped enemies who
        // just saw the player.
        //
        // This block previously also called `reconsider_enemy_approach`
        // for every fresh detection, which immediately pushed the NPC
        // into `AttackingRunningToEnemy` and bypassed the reaction-time
        // pause that `event_view_standard_procedure` just set up
        // (`AttackingReactiontimeTurning` + `LaunchTimer(20)`).
        // `event_view_standard_procedure` does NOT call
        // `reconsider_enemy_approach` after detection — it lets the
        // state machine advance through the
        // `AttackingReactiontimeTurning` → `AttackingReactiontime` →
        // `BattleDecisions` chain on the later `EVENT_TIMER` phase.
        //
        // The facing snap and focus are already handled inside
        // `event_view_standard_procedure` (via `face_entity` +
        // `pending_focus`).
        for det in transitions {
            if let Some(entity) = self.entities.get_mut(det.enemy)
                && entity.element_data().blipped
            {
                tracing::debug!(
                    entity = det.enemy.index(),
                    "reveal_blip: NPC revealed on detection commit"
                );
                entity.reveal_blip();
            }
        }
    }

    /// Normal-timer phase from `RHElementActorNPC::Hourglass`.
    ///
    /// Runs after `The16thFrame` and before the macro timer. For every
    /// unlocked NPC whose timer elapsed, stop it and dispatch
    /// `Think(EVENT_TIMER)`. Soldiers that enter swordfight receive the
    /// original post-dispatch combat-stance and civilian-panic effects.
    pub(crate) fn tick_ai_normal_timers(&mut self, assets: &LevelAssets) {
        if self.actors_frozen() || self.ai_global.freeze {
            return;
        }

        let scratch = self.build_sim_scratch(assets);
        let current_frame = self.frame_counter;
        let mut panic_calls: Vec<EntityId> = Vec::new();

        // EVENT_TIMER dispatch. For every NPC whose timer has
        // elapsed, stop the timer and fire `Think(EVENT_TIMER)` through
        // the filter gate so the AI state machine advances (bored idle
        // → `default_bored_standard_procedure`, alerted →
        // `reconsider_enemy_approach` / `reconsider_swordfight`).
        //
        // The wrap-around guard (`when_does_timer_ring > current_frame +
        // 1_000_000`) is an overflow-safety clause: wait times are
        // 1-600 frames so a ring-frame that "overshoots" by a million
        // always indicates an unsigned underflow, never a future tick.
        let npc_ids: Vec<_> = self.entities.npc_ids().collect();
        for npc_id in npc_ids {
            self.tick_enemy_ai_pursuit_approach_timer_for_npc(
                npc_id,
                assets,
                &scratch,
                current_frame,
                &mut panic_calls,
            );
        }

        // For every enemy that just entered melee:
        //   1. Apply the soldier's combat-stance action_state
        //      (so the WaitingSword sprite plays).
        //   2. Stop and freeze the primary target if they're a PC
        //      (target->stop() if moving), so the swordfight has a
        //      stable position.
        //   3. `nearby_civilians_panic` — bystanders flee.
        for enemy in panic_calls {
            // Look up the soldier's primary target.
            let target_id = {
                let Some(Entity::Soldier(s)) = self.entities.get(enemy) else {
                    continue;
                };
                s.npc
                    .ai_brain
                    .base()
                    .map(|ai| EntityId::Pc(crate::entity_id::PcId(ai.primary_target)))
            };

            // Set the soldier into combat stance.  Clearing
            // `active_movement` decouples the actor from any in-
            // progress Move element — the element itself gets
            // interrupted by the subsequent combat-sequence launch
            // via priority arbitration (same pattern used by every
            // ability teardown).
            if let Some(Entity::Soldier(s)) = self.entities.get_mut(enemy) {
                s.actor.active_movement.clear();
                s.actor.action_state = crate::element::ActionState::WaitingSword;
            }

            // Stop the target PC's path so the soldier has a stable
            // melee anchor.
            if let Some(target_id) = target_id
                && target_id.index() != 0
                && let Some(Entity::Pc(pc)) = self.entities.get_mut(target_id)
            {
                pc.actor.active_movement.clear();
                // Don't force the PC into WaitingSword — that's
                // controlled by the player input layer.  Just
                // halt their current movement.
            }

            // Civilian panic.
            self.nearby_civilians_panic(assets, enemy);
        }
    }

    /// Per-NPC body of [`Self::tick_ai_normal_timers`]. Carries the per-NPC
    /// tracing span for `Think(EVENT_TIMER)` dispatches.
    ///
    /// Handles both soldiers (enemy AI) and civilians (friendly AI).
    /// `Think(EVENT_TIMER)` fires for every NPC whose timer has
    /// elapsed regardless of subclass; civilians use `LaunchTimer`
    /// from `WonderingCivilianAdmiringHero` /
    /// `WonderingCivilianEnemyReactiontime` and would otherwise stick
    /// in those substates indefinitely.  The soldier-only pre-dispatch
    /// facing snap and post-dispatch swordfight-entry detection are
    /// gated on `Entity::Soldier`.
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    fn tick_enemy_ai_pursuit_approach_timer_for_npc(
        &mut self,
        npc_id: EntityId,
        assets: &LevelAssets,
        scratch: &SimScratch,
        current_frame: u32,
        panic_calls: &mut Vec<EntityId>,
    ) {
        // Snapshot the state we need (immut borrow).  `ai_controller`
        // returns the base controller for both soldiers and civilians.
        let (timer_fires, alerted, target_id, enemy_pos, is_soldier) = {
            let Some(entity) = self.entities.get(npc_id) else {
                return;
            };
            let Some(ai) = entity.ai_controller() else {
                return;
            };
            let unlocked = ai.locks_flag_field.is_empty() && !ai.script_locked;
            let fires = unlocked
                && ai.timer_is_running
                && (ai.when_does_timer_ring <= current_frame
                    || ai.when_does_timer_ring > current_frame.saturating_add(1_000_000));
            // `primary_target == 0` means "no target selected" — the AI
            // hasn't seen a PC yet.  Treating 0 as an EntityId would
            // route target lookups to the first level entity.
            let tid = (ai.primary_target != 0)
                .then_some(EntityId::Pc(crate::entity_id::PcId(ai.primary_target)));
            let alerted = match entity {
                Entity::Soldier(s) => s.npc.alerted,
                _ => false,
            };
            (
                fires,
                alerted,
                tid,
                entity.element_data().position_map(),
                matches!(entity, Entity::Soldier(_)),
            )
        };
        if !timer_fires {
            return;
        }
        // Soldier-only swordfight entry tracking — civilians never
        // transition into `AttackingSwordfight` so the post-dispatch
        // panic_calls push is gated on this snapshot too.
        let in_swordfight = if is_soldier {
            let Some(Entity::Soldier(soldier)) = self.entities.get(npc_id) else {
                return;
            };
            soldier.npc.ai_substate() == crate::ai::Substate::AttackingSwordfight
        } else {
            false
        };

        // Pre-dispatch facing snap: only when the AI is alerted
        // and has a live target.  Surfaces the primary-target
        // facing through a pre-dispatch snap alongside the
        // `AiPerTickData` the builder assembles below.
        let face_dir = target_id.and_then(|tid| {
            self.entities
                .get(tid)
                .map(|e| e.element_data().position_map())
                .map(|tp| {
                    crate::position_interface::vector_to_sector_0_to_15_iso(
                        tp.x - enemy_pos.x,
                        tp.y - enemy_pos.y,
                    )
                })
        });

        // Build the rich tick data from the centralized builder
        // — covers primary target metadata, friend-swap
        // candidates, avenger-on-roof wait position, and seeded
        // enemy_sq_distances.  Matches (and supersedes) the
        // bespoke hand-roll this block used to do.
        let tick_data = self.build_npc_tick_data(npc_id, scratch, assets);

        // Build ctx and stop the timer under a single mut borrow.
        let in_uninterruptible_command = self.is_very_very_busy(npc_id);
        let ctx = {
            let Some(entity) = self.entities.get_mut(npc_id) else {
                return;
            };
            // Only snap facing when the AI is alerted and has a
            // target — idle soldiers keep whatever direction their
            // look-sidewards cascade left them in.
            if alerted && let Some(fd) = face_dir {
                entity.element_data_mut().set_direction_instantly(fd);
            }
            let mut ctx = build_ai_context_from_entity(
                entity,
                current_frame,
                None,
                self.weather.is_forest_level,
                self.weather.ambiance,
                self.standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.fast_grid,
                &assets.hiking_paths,
                &self.ai_global.all_soldier_handles,
            );
            ctx.in_uninterruptible_command = in_uninterruptible_command;
            ctx.enter_swordfight_pending = self
                .sequence_manager
                .element_is_about_to_be_launched(npc_id, crate::element::Command::EnterSwordfight);
            // Clear `timer_is_running` before dispatching
            // `Think(EVENT_TIMER)`.
            let Some(ai) = entity.ai_controller_mut() else {
                return;
            };
            ai.timer_is_running = false;
            ctx
        };

        let timer_stimulus = crate::ai::Stimulus::new(crate::ai::StimulusType::EventTimer);
        self.dispatch_think_with_drain(npc_id, &timer_stimulus, &ctx, &tick_data, assets);

        // Post-think: detect swordfight entry so the caller can fire
        // `nearby_civilians_panic` + combat-stance bookkeeping below.
        // Civilians never enter `AttackingSwordfight`, so this check
        // can stay gated on the Soldier-only `enemy_ai()` accessor.
        if !in_swordfight
            && let Some(entity) = self.entities.get(npc_id)
            && let Some(ai) = entity.enemy_ai()
            && ai.base.current_substate == crate::ai::Substate::AttackingSwordfight
        {
            panic_calls.push(npc_id);
        }
    }

    /// P6c — drain `pending_*` AI swordfight / order flags for every NPC.
    /// AI decisions set flags on `AiController`; we consume them here
    /// after all think calls are done, since they require engine-side
    /// entity mutations (opponent lists, sequences).
    pub(super) fn tick_enemy_ai_drain_swordfight_requests(&mut self, assets: &LevelAssets) {
        let npc_ids: Vec<_> = self.entities.npc_ids().collect();
        for npc_id in npc_ids {
            self.drain_pending_for_npc(npc_id, assets);
        }
    }

    /// P6d — replay deferred `pending_stimuli` for every NPC.
    ///
    /// Combat events (EVENT_GOOD_STRIKE, EVENT_LETHAL_STRIKE,
    /// EVENT_ENTER_SWORDFIGHT, etc.) are queued on
    /// `AiController::pending_stimuli` by `dispatch_ai_stimulus()`
    /// during the combat tick.  We defer them to avoid re-entrant
    /// borrow issues, then replay them now.
    pub(super) fn tick_enemy_ai_drain_pending_stimuli(&mut self, assets: &LevelAssets) {
        let npc_ids: Vec<_> = self.entities.npc_ids().collect();
        for npc_id in npc_ids {
            self.tick_enemy_ai_drain_pending_stimuli_for_npc(npc_id, assets, None);
        }
    }

    /// P6d inner — per-NPC body of [`Self::tick_enemy_ai_drain_pending_stimuli`].
    /// Replays deferred stimuli for one NPC; carries the per-NPC tracing
    /// span so the `dispatch_think_with_drain` events emit with `npc=<id>`.
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    pub(super) fn tick_enemy_ai_drain_pending_stimuli_for_npc(
        &mut self,
        npc_id: EntityId,
        assets: &LevelAssets,
        mut enemy_detection_tick_data: Option<PendingEnemyDetectionTickData>,
    ) {
        let stimuli = {
            let Some(entity) = self.entities.get_mut(npc_id) else {
                return;
            };
            let Some(ai) = entity.ai_controller_mut() else {
                return;
            };
            std::mem::take(&mut ai.pending_stimuli)
        };
        if stimuli.is_empty() {
            return;
        }
        for (queue_index, stimulus) in stimuli.into_iter().enumerate() {
            // Original Think is a synchronous boundary. Its EndThink (and any
            // recursive event it launches) finishes before the next queued
            // stimulus starts, so every entry must observe mutations made by
            // its predecessor rather than the tick-start entity-view map.
            let scratch = self.build_sim_scratch(assets);
            let in_uninterruptible_command = self.is_very_very_busy(npc_id);
            let ctx = {
                let Some(entity) = self.entities.get(npc_id) else {
                    break;
                };
                let entity_sector = entity.element_data().sector();
                let building_sector = self.entity_building_sector(entity_sector);
                let Some(entity) = self.entities.get(npc_id) else {
                    break;
                };
                let mut ctx = build_ai_context_from_entity(
                    entity,
                    self.frame_counter,
                    building_sector,
                    self.weather.is_forest_level,
                    self.weather.ambiance,
                    self.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.fast_grid,
                    &assets.hiking_paths,
                    &self.ai_global.all_soldier_handles,
                );
                ctx.in_uninterruptible_command = in_uninterruptible_command;
                if let crate::ai::StimulusInfo::Human(handle) = stimulus.info {
                    let view = ctx.entity_view(handle).unwrap_or_else(|| {
                        panic!(
                            "queued {:?} for NPC {} references missing entity {}",
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
            // The Enemy VIEW / OUTOFVIEW block retains the completed scan
            // aggregate, but all tactical and target-specific inputs are
            // rebuilt from the live world for this exact stimulus.
            let detection_aggregate = take_enemy_detection_tick_data(
                queue_index,
                &stimulus,
                &mut enemy_detection_tick_data,
            );
            let tick_data = if let Some(aggregate) = detection_aggregate {
                let target_id = match stimulus.info {
                    crate::ai::StimulusInfo::Human(handle) => {
                        self.entity_id_for_index(handle).unwrap_or_else(|| {
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
                    self.build_npc_tick_data_for_target(npc_id, &scratch, assets, Some(target_id));
                overlay_final_detection_scan(&mut live, &aggregate);
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
                        Some(self.entity_id_for_index(handle).unwrap_or_else(|| {
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
                self.build_npc_tick_data_for_target(npc_id, &scratch, assets, target_override)
            };
            self.dispatch_think_with_drain(npc_id, &stimulus, &ctx, &tick_data, assets);
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
    /// `RHElementActorNPC::Hourglass`, after both timer kinds.
    pub(crate) fn tick_ai_queued_stimuli(&mut self, assets: &LevelAssets) {
        if self.actors_frozen() || self.ai_global.freeze {
            return;
        }

        let scratch = self.build_sim_scratch(assets);
        let npc_ids: Vec<_> = self.entities.npc_ids().collect();
        for npc_id in npc_ids {
            loop {
                let stimulus = {
                    let Some(entity) = self.entities.get_mut(npc_id) else {
                        break;
                    };
                    let Some(ai) = entity.ai_controller_mut() else {
                        break;
                    };
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

                let in_uninterruptible_command = self.is_very_very_busy(npc_id);
                let ctx = {
                    let Some(entity) = self.entities.get(npc_id) else {
                        break;
                    };
                    let building_sector =
                        self.entity_building_sector(entity.element_data().sector());
                    let mut ctx = build_ai_context_from_entity(
                        entity,
                        self.frame_counter,
                        building_sector,
                        self.weather.is_forest_level,
                        self.weather.ambiance,
                        self.standard_view_polygon_radius,
                        &scratch.ai_entity_views,
                        &scratch.ai_sight_obstacles,
                        &self.fast_grid,
                        &assets.hiking_paths,
                        &self.ai_global.all_soldier_handles,
                    );
                    ctx.in_uninterruptible_command = in_uninterruptible_command;
                    ctx
                };
                let target_override = match stimulus.info {
                    crate::ai::StimulusInfo::Human(handle)
                        if matches!(
                            stimulus.stimulus_type,
                            crate::ai::StimulusType::EventView
                                | crate::ai::StimulusType::EventSeesBeggar
                                | crate::ai::StimulusType::EventEnemyNear
                        ) =>
                    {
                        self.entity_id_for_index(handle)
                    }
                    _ => None,
                };
                let tick_data =
                    self.build_npc_tick_data_for_target(npc_id, &scratch, assets, target_override);
                self.dispatch_think_with_drain(npc_id, &stimulus, &ctx, &tick_data, assets);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enemy_detection_tick_data_override_matches_the_exact_fifo_block() {
        let mut full_tick_data = crate::ai::AiPerTickData::stub();
        full_tick_data.personally_visible_enemies = 7;
        full_tick_data.us_battle_points = 321;
        let shadow = crate::ai::Stimulus::with_position(
            crate::ai::StimulusType::EventSeesShadow,
            crate::ai::Position::default(),
        );
        let view = crate::ai::Stimulus::with_human(crate::ai::StimulusType::EventView, 42);
        let out_of_view =
            crate::ai::Stimulus::with_human(crate::ai::StimulusType::EventOutOfView, 77);
        let mut pending = Some(PendingEnemyDetectionTickData::new(
            1,
            vec![view, out_of_view],
            full_tick_data,
        ));

        assert!(take_enemy_detection_tick_data(0, &shadow, &mut pending).is_none());
        let selected = take_enemy_detection_tick_data(1, &view, &mut pending)
            .expect("exact EVENT_VIEW queue entry keeps detection-built input");
        assert_eq!(selected.personally_visible_enemies, 7);
        assert_eq!(selected.us_battle_points, 321);
        let selected = take_enemy_detection_tick_data(2, &out_of_view, &mut pending)
            .expect("exact EVENT_OUTOFVIEW queue entry keeps detection-built input");
        assert_eq!(selected.personally_visible_enemies, 7);
        assert_eq!(
            pending.as_ref().expect("block remains for audit").matched,
            2
        );
    }
}
