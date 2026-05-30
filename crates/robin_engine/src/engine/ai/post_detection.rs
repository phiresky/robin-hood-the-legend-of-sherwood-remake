//! Post-detection orchestration phases for `tick_enemy_ai`:
//! P4 (alert allies / log), P4b (out-of-view dispatch), P6 (pursuit
//! / approach / combat-stance), P6c (drain pending swordfight
//! requests), P6d (replay deferred stimuli).

use super::snapshots::{Detection, PcSnapshot};
use super::*;
use crate::element::{Entity, EntityId};
use crate::geo2d::{self};

impl EngineInner {
    /// P4 — fire `HeyFolksLookThere` + log on every fresh detection
    /// transition.  Alerts nearby idle soldiers when an NPC spots the
    /// PC.
    pub(super) fn tick_enemy_ai_alert_allies(&mut self, transitions: &[Detection]) {
        const VIEW_LOOK_THERE_RADIUS: f32 = 100.0;
        let alert_calls: Vec<(EntityId, geo2d::GeoPoint2D)> = transitions
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

    /// P4b — drain the per-detectable falling-edge OUTOFVIEW queue.
    ///
    /// When a detectable that was `seen_last_frame` is no longer
    /// `seen_now`, fire `EVENT_OUTOFVIEW` for that target.  No grace
    /// period — the event fires the moment LOS drops, and the AI's
    /// `think_unexpected_event` handler decides what to do based on
    /// state/substate.
    pub(super) fn tick_enemy_ai_dispatch_out_of_view(
        &mut self,
        out_of_view_dispatches: Vec<(EntityId, u32)>,
        pc_snapshots: &[PcSnapshot],
    ) {
        // Pre-resolve per-NPC building sectors — the mutable borrow
        // below blocks calling `entity_building_sector` inside the loop.
        let viewer_building_sectors: Vec<Option<crate::position_interface::SectorHandle>> =
            out_of_view_dispatches
                .iter()
                .map(|(npc_id, _)| {
                    self.get_entity(*npc_id)
                        .and_then(|e| self.entity_building_sector(e.element_data().sector()))
                })
                .collect();
        for ((npc_id, primary_target), _viewer_building_sector) in out_of_view_dispatches
            .into_iter()
            .zip(viewer_building_sectors)
        {
            if let Some(Some(Entity::Soldier(soldier))) = self.entities.get_mut(npc_id.0 as usize) {
                // `reinitialize_them_list` walks the enemy detectable
                // list and rebuilds `list_them` from entries with
                // `seen_now` true.  The dispatch reads the snapshot
                // out of `tick.enemy_sq_distances`, so every dispatch
                // site has to populate it — otherwise a default
                // `tick_data` clears `list_them` to empty and a
                // follow-up `battle_decisions` hits the no-enemies
                // fallback.  Build it here using the NPC's own
                // `detectable_lists` + PC snapshots so the OUTOFVIEW
                // default arm's `reinitialize_them_list` sees the
                // right "still-visible" subset.
                let enemy_idx = DetectableType::Enemy as usize;
                let mut visible_enemies: Vec<(u32, i32)> = Vec::new();
                for det in soldier.npc.detectable_lists[enemy_idx].iter() {
                    if !det.seen_now {
                        continue;
                    }
                    let Some(t_id) = det.element else { continue };
                    if let Some(pc) = pc_snapshots.iter().find(|p| p.id == t_id) {
                        let dx = pc.position.x - soldier.element.position_map().x;
                        let dy = (pc.position.y - soldier.element.position_map().y)
                            * crate::position_interface::INVERSE_ASPECT_RATIO;
                        visible_enemies.push((t_id.0, (dx * dx + dy * dy) as i32));
                    }
                }
                if let Some(enemy_ai) = soldier.npc.ai_brain.enemy_mut() {
                    let stimulus = crate::ai::Stimulus::with_human(
                        crate::ai::StimulusType::EventOutOfView,
                        primary_target,
                    );
                    let _ = &visible_enemies;
                    // Queue for post-detection drain — see EventHear
                    // site.  Enemy-distance tick_data is lost; the AI's
                    // `think_unexpected_event` OUTOFVIEW branch still
                    // transitions correctly from its own state.
                    enemy_ai.base.pending_stimuli.push(stimulus);
                }
            }
        }
    }

    /// P6 — pursuit / approach / combat stance.
    ///
    /// 6a) Reveal blipped NPCs that just committed a detection.
    /// 6b) Per-NPC EVENT_TIMER dispatch: fire `Think(EVENT_TIMER)`
    ///     through the filter gate so the AI state machine advances
    ///     (bored idle → DefaultBoredStandard, alerted →
    ///     ReconsiderEnemyApproach / ReconsiderSwordfight).
    /// 6.panic) For every enemy that just entered melee, apply the
    ///     combat-stance action_state, halt the PC target's movement,
    ///     and fire `nearby_civilians_panic`.
    pub(super) fn tick_enemy_ai_pursuit_approach(
        &mut self,
        assets: &LevelAssets,
        scratch: &SimScratch,
        transitions: Vec<Detection>,
    ) {
        let current_frame = self.frame_counter;

        // Collect enemies that transition INTO Swordfight this tick
        // so we can fire `nearby_civilians_panic` for them outside
        // the entity borrow scope.
        let mut panic_calls: Vec<EntityId> = Vec::new();

        // 6a. Newly committed detections — reveal blipped enemies who
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
        // `BattleDecisions` chain on `EVENT_TIMER`.  Section 6b polls
        // the timer and dispatches `EventTimer`, matching that flow.
        //
        // The facing snap and focus are already handled inside
        // `event_view_standard_procedure` (via `face_entity` +
        // `pending_focus`).
        for det in transitions {
            if let Some(Some(entity)) = self.entities.get_mut(det.enemy.0 as usize)
                && entity.element_data().blipped
            {
                tracing::debug!(
                    entity = det.enemy.0,
                    "reveal_blip: NPC revealed on detection commit"
                );
                entity.reveal_blip();
            }
        }

        // 6b. EVENT_TIMER dispatch.  For every enemy whose timer has
        // elapsed, stop the timer and fire `Think(EVENT_TIMER)` through
        // the filter gate so the AI state machine advances (bored idle
        // → `default_bored_standard_procedure`, alerted →
        // `reconsider_enemy_approach` / `reconsider_swordfight`).
        //
        // The wrap-around guard (`when_does_timer_ring > current_frame +
        // 1_000_000`) is an overflow-safety clause: wait times are
        // 1-600 frames so a ring-frame that "overshoots" by a million
        // always indicates an unsigned underflow, never a future tick.
        let npc_ids = self.npc_ids.clone();
        for npc_id in npc_ids {
            self.tick_enemy_ai_pursuit_approach_timer_for_npc(
                npc_id,
                assets,
                scratch,
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
                let Some(Some(Entity::Soldier(s))) = self.entities.get(enemy.0 as usize) else {
                    continue;
                };
                s.npc.ai_brain.base().map(|ai| EntityId(ai.primary_target))
            };

            // Set the soldier into combat stance.  Clearing
            // `active_movement` decouples the actor from any in-
            // progress Move element — the element itself gets
            // interrupted by the subsequent combat-sequence launch
            // via priority arbitration (same pattern used by every
            // ability teardown).
            if let Some(Some(Entity::Soldier(s))) = self.entities.get_mut(enemy.0 as usize) {
                s.actor.active_movement.clear();
                s.actor.action_state = crate::element::ActionState::WaitingSword;
            }

            // Stop the target PC's path so the soldier has a stable
            // melee anchor.
            if let Some(target_id) = target_id
                && target_id.0 != 0
                && let Some(Some(Entity::Pc(pc))) = self.entities.get_mut(target_id.0 as usize)
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

    /// P6 inner — per-NPC body of the timer-dispatch loop in
    /// [`Self::tick_enemy_ai_pursuit_approach`].  Carries the per-NPC
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
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.0))]
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
            let Some(Some(entity)) = self.entities.get(npc_id.0 as usize) else {
                return;
            };
            let Some(ai) = entity.ai_controller() else {
                return;
            };
            let fires = ai.timer_is_running
                && (ai.when_does_timer_ring <= current_frame
                    || ai.when_does_timer_ring > current_frame.saturating_add(1_000_000));
            // `primary_target == 0` means "no target selected" — the AI
            // hasn't seen a PC yet.  Treating 0 as an EntityId would
            // route target lookups to the first level entity.
            let tid = (ai.primary_target != 0).then_some(EntityId(ai.primary_target));
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
            let Some(Some(Entity::Soldier(soldier))) = self.entities.get(npc_id.0 as usize) else {
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
                .get(tid.0 as usize)
                .and_then(|s| s.as_ref())
                .map(|e| e.element_data().position_map().to_geo())
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
            let Some(Some(entity)) = self.entities.get_mut(npc_id.0 as usize) else {
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
            && let Some(Some(entity)) = self.entities.get(npc_id.0 as usize)
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
        let npc_ids = self.npc_ids.clone();
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
    pub(super) fn tick_enemy_ai_drain_pending_stimuli(
        &mut self,
        assets: &LevelAssets,
        scratch: &SimScratch,
    ) {
        let npc_ids = self.npc_ids.clone();
        for npc_id in npc_ids {
            self.tick_enemy_ai_drain_pending_stimuli_for_npc(npc_id, assets, scratch);
        }
    }

    /// P6d inner — per-NPC body of [`Self::tick_enemy_ai_drain_pending_stimuli`].
    /// Replays deferred stimuli for one NPC; carries the per-NPC tracing
    /// span so the `dispatch_think_with_drain` events emit with `npc=<id>`.
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.0))]
    fn tick_enemy_ai_drain_pending_stimuli_for_npc(
        &mut self,
        npc_id: EntityId,
        assets: &LevelAssets,
        scratch: &SimScratch,
    ) {
        let stimuli = {
            let Some(Some(entity)) = self.entities.get_mut(npc_id.0 as usize) else {
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
        for stimulus in stimuli {
            let in_uninterruptible_command = self.is_very_very_busy(npc_id);
            let ctx = {
                let Some(Some(entity)) = self.entities.get(npc_id.0 as usize) else {
                    break;
                };
                let entity_sector = entity.element_data().sector();
                let building_sector = self.entity_building_sector(entity_sector);
                let Some(Some(entity)) = self.entities.get(npc_id.0 as usize) else {
                    break;
                };
                let mut ctx = build_ai_context_from_entity(
                    entity,
                    self.frame_counter,
                    building_sector,
                    self.weather.is_forest_level,
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
            // pending_stimuli drain for this NPC — includes
            // deferred EVENT_VIEW / EVENT_HEAR / EVENT_SEES_SHADOW
            // from the detection pass.  The builder populates
            // primary-target metadata, friend-swap candidates,
            // and seeded enemy_sq_distances so the downstream
            // handlers (battle_decisions, filter_ai_event) see
            // real context instead of stub().
            let target_override = match stimulus.info {
                crate::ai::StimulusInfo::Human(handle)
                    if matches!(
                        stimulus.stimulus_type,
                        crate::ai::StimulusType::EventView
                            | crate::ai::StimulusType::EventSeesBeggar
                            | crate::ai::StimulusType::EventEnemyNear
                    ) =>
                {
                    Some(EntityId(handle))
                }
                _ => None,
            };
            let tick_data =
                self.build_npc_tick_data_for_target(npc_id, scratch, assets, target_override);
            self.dispatch_think_with_drain(npc_id, &stimulus, &ctx, &tick_data, assets);
        }
    }
}
