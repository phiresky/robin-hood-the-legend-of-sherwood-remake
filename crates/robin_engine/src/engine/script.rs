//! Script/GameHost wiring, mission script management, campaign integration.

use super::scroll_reveal::ScrollStatus;
use super::*;
use crate::ai::AiStateChangeSource;
use crate::campaign::{Campaign, CampaignValue};
use crate::messenger::{Message, MessageType, SimpleMessage};
use crate::profiles::{MissionLocation, MissionProfile};

impl EngineInner {
    /// Refresh entity-active map in GameHost from the real entity storage.
    /// Called before script execution so IsAnimationActive reads live state.
    /// Also populates PC/NPC snapshot state used by misc native functions.
    pub(super) fn refresh_game_host_entity_state(&mut self) {
        // Collect data first, then write to host (avoids borrow issues).
        let ambiance = self.weather.ambiance;
        let is_forest_level = self.weather.is_forest_level;
        let mut entity_active_map: Vec<(i32, bool)> = Vec::with_capacity(self.entities.len());
        let mut pc_handles = Vec::with_capacity(self.pc_ids.len());
        let mut robin_handle: i32 = 0;
        let mut pc_profile_map: Vec<(i32, crate::profiles::CharacterProfileIdx)> =
            Vec::with_capacity(self.pc_ids.len());
        let mut any_civilian_dead = false;
        let mut any_enemy_dead = false;
        let mut overall_enemy_alert: i32 = 0;
        let mut overall_civilian_alert: i32 = 0;

        // Sound-source liveness snapshot — destroyed slots must be
        // distinguishable from valid indices so `GetSoundSourceScript`
        // can reject destroyed-but-preserved-index without conflating
        // them with live ones.
        let sound_source_alive: Vec<bool> = (0..self.sound_sim.sources.num_sources())
            .map(|i| self.sound_sim.sources.get(i).is_some())
            .collect();

        for (id, fx) in self.entities.fxs() {
            let handle = crate::natives::GameHost::actor_handle(id);
            entity_active_map.push((handle, fx.element.active));
        }
        for (id, target) in self.entities.targets() {
            let handle = crate::natives::GameHost::actor_handle(id);
            entity_active_map.push((handle, target.element.active));
        }

        for (id, pc) in self.entities.pcs() {
            let handle = crate::natives::GameHost::actor_handle(id);
            pc_handles.push(handle);
            pc_profile_map.push((handle, pc.pc.profile_index));
            if pc.pc.robin {
                robin_handle = handle;
            }
        }

        // NPC aggregate state.  Alert sourced from the NPC's
        // `current_music_alert_status`, which `SetAlertStatus`
        // writes independently of the AI state machine.  Do not
        // collapse with `AiState` — the two fields drift.
        for (_, entity) in self.entities.npcs() {
            let dead = entity.is_dead();
            let alert = entity
                .ai_controller()
                .map(|ai| ai.current_music_alert_status as i32)
                .unwrap_or(0);
            // Civilian filter is `!is_soldier()` (all non-soldier
            // NPCs), not a narrower civilian category.
            if entity.is_soldier() {
                if dead {
                    any_enemy_dead = true;
                }
                if !dead && alert > overall_enemy_alert {
                    overall_enemy_alert = alert;
                }
            } else {
                if dead {
                    any_civilian_dead = true;
                }
                if !dead && alert > overall_civilian_alert {
                    overall_civilian_alert = alert;
                }
            }
        }

        // Selected PCs
        let selected_pc_handles: Vec<i32> = self.seats[0]
            .selection
            .iter()
            .map(|&id| crate::natives::GameHost::actor_handle(id))
            .collect();

        // Live animation snapshot per handle.  Actors source from the
        // front order on their current sequence element; objects source
        // from `ObjectData::animation`. Used by natives like
        // `GetCurrentAction` that must return the animation enum value
        // for the current frame.
        let mut current_animations: Vec<(i32, crate::order::OrderType)> =
            Vec::with_capacity(self.entities.len());
        for (entity_id, _) in self.entities.actors() {
            let handle = crate::natives::GameHost::actor_handle(entity_id);
            if let Some((_, _, order)) = self.sequence_manager.current_order_for_actor(entity_id) {
                current_animations.push((handle, order.order_type));
            }
        }
        for (entity_id, entity) in self.entities.objects() {
            let handle = crate::natives::GameHost::actor_handle(entity_id);
            if let Some(obj) = entity.object_data() {
                current_animations.push((handle, obj.animation));
            }
        }

        // Write to host
        let script = match self.mission_script.as_mut() {
            Some(s) => s,
            None => return,
        };
        let game_host = match script.game_host_mut() {
            Some(h) => h,
            None => return,
        };

        game_host.entity_active.clear();
        for (h, active) in entity_active_map {
            game_host.entity_active.insert(h, active);
        }

        game_host.current_animations.clear();
        for (h, anim) in current_animations {
            game_host.current_animations.insert(h, anim);
        }

        game_host.pc_handles = pc_handles;
        game_host.selected_pc_handles = selected_pc_handles;
        game_host.robin_handle = robin_handle;
        game_host.pc_profile_map.clear();
        for (h, pi) in pc_profile_map {
            game_host.pc_profile_map.insert(h, pi);
        }
        game_host.any_civilian_dead = any_civilian_dead;
        game_host.any_enemy_dead = any_enemy_dead;
        game_host.overall_enemy_alert = overall_enemy_alert;
        game_host.overall_civilian_alert = overall_civilian_alert;
        game_host.sound_source_alive = sound_source_alive;
        game_host.ambiance = ambiance;
        game_host.is_forest_level = is_forest_level;
    }

    /// Populate PC authorisation bits in GameHost from spawned PC entities.
    pub(super) fn refresh_game_host_pc_auth_bits(&mut self) {
        let mut bits: Vec<(i32, u16)> = Vec::new();
        let mut pc_bit_idx = 0u16;
        for (id, _) in self.entities.pcs() {
            let handle = crate::natives::GameHost::actor_handle(id);
            let bit = 1u16 << pc_bit_idx;
            bits.push((handle, bit));
            pc_bit_idx += 1;
        }
        if let Some(ref mut script) = self.mission_script
            && let Some(game_host) = script.game_host_mut()
        {
            for (handle, bit) in bits {
                game_host.pc_auth_bits.insert(handle, bit);
            }
        }
    }

    /// Copy level-static data (script location table, counts, map bbox,
    /// source count) into `GameHost` once at script init.  These fields
    /// never change during mission play — they're built at load and
    /// frozen thereafter, so we avoid re-cloning the location Vecs on
    /// every script call.
    pub(super) fn install_script_static_data_into_game_host(&mut self, assets: &LevelAssets) {
        let Some(ref mut script) = self.mission_script else {
            return;
        };
        let Some(game_host) = script.game_host_mut() else {
            return;
        };
        // Profile manager is immutable for the mission lifetime — install
        // once and let it ride through every campaign swap-in/swap-out.
        // (Previously natives read profiles off the swapped-in `Campaign`,
        // but the field is now an explicit parameter passed alongside.)
        game_host.profile_manager = assets.profile_manager.clone().into();
        crate::natives::set_script_sight_obstacles(crate::sight_obstacle::SharedSightObstacles {
            static_obstacles: assets.static_sight_obstacles.clone(),
            dynamic_obstacles: std::sync::Arc::new(self.dynamic_sight_obstacles.clone()),
            static_active: std::sync::Arc::new(self.static_sight_obstacle_active.clone()),
        });
        game_host.script_location_count = assets.script_location_count;
        game_host.script_point_count = assets.script_point_count;
        game_host
            .location_positions
            .clone_from(&assets.script_location_positions);
        game_host
            .location_layers
            .clone_from(&assets.script_location_layers);
        game_host
            .location_sectors
            .clone_from(&assets.script_location_sectors);
        game_host.script_building_count = assets.script_building_count;
        game_host.script_hiking_path_count = assets.script_hiking_path_count;
        game_host.hiking_paths = assets.hiking_paths.clone().into();
        game_host.sound_source_count = self.sound_sim.sources.num_sources();
        // Map bounding box, needed by RecordEnterGame / RecordLeaveGame
        // to compute map-edge spawn/exit points.
        game_host.map_bbox = self.fast_grid.level.map_bbox;
        // Sector type/lift/door snapshot — needed by record-time
        // `append_move_to_sequence` to handle the building / ladder /
        // door-goal branches without holding a back-reference to
        // FastFindGrid.
        game_host.sector_kinds.clear();
        for gs in &self.fast_grid.level.sectors {
            let key = u16::from(gs.sector_number);
            game_host.sector_kinds.insert(
                key,
                crate::natives::SectorKindInfo {
                    is_building: gs.sector_type.is_building(),
                    is_ladder_lift: gs.lift_type == Some(crate::sector::LiftType::Ladder),
                    lift_type: gs.lift_type,
                    is_door: gs.sector_type.is_door(),
                },
            );
        }
        // Zone-polygon geometry — lets the IsInside native recompute
        // per call without touching the engine's grid (the
        // "works after teleports" path).  Index here matches `zone_idx`
        // in `script_zone_data` and the per-zone ordering of
        // `assets.script_zone_grid_indices`.
        game_host.script_zone_polygons.clear();
        game_host
            .script_zone_polygons
            .reserve(assets.script_zone_grid_indices.len());
        for &grid_idx in &assets.script_zone_grid_indices {
            let gs = &self.fast_grid.level.sectors[grid_idx as usize];
            game_host
                .script_zone_polygons
                .push(crate::natives::ScriptZonePolygon {
                    layer: gs.layer,
                    bounding_box: gs.bounding_box,
                    points: gs.points.clone(),
                });
        }
    }

    /// Apply changes from GameHost back to the engine after a script call.
    /// Syncs entity active state, processes sound commands, and checks
    /// whether patches invalidated the background.
    pub(crate) fn sync_game_host_post_script(&mut self, assets: &LevelAssets) {
        // We need to take the script out briefly so we can mutably borrow
        // both GameHost fields and engine fields simultaneously.
        let mut script = match self.mission_script.take() {
            Some(s) => s,
            None => return,
        };

        let mut engine_commands = Vec::new();
        // Deferred commands that must run AFTER `self.mission_script` is put
        // back (currently `ProcessPatchEffects`, which looks patches up via
        // `self.mission_script`). Populated inside the host block below.
        let mut post_script: Vec<crate::natives::DeferredCommand> = Vec::new();
        // RHScript::SendMessage launches a standalone RHCOMMAND_SEND_MESSAGE
        // sequence element. Keep requests in native-call order and launch
        // them once the mission script is installed again so their immediate
        // ProcessMessage callback can re-enter the script system this frame.
        let mut script_messages: Vec<(i32, i32, i32, i32)> = Vec::new();

        if let Some(game_host) = script.game_host_mut() {
            // ── Entity active state → real entities ──
            for (&handle, &active) in &game_host.entity_active {
                if let Some(entity_id) = self.entity_id_for_actor_handle(handle)
                    && let Some(entity) = self.entities.get_mut(entity_id)
                    && (entity.kind().is_fx() || entity.kind().is_fx_target())
                {
                    entity.element_data_mut().active = active;
                }
            }

            // ── Sound commands ──
            // Commands that don't need an AudioBackend are processed now.
            // The remaining ones are queued for main_entry to flush.
            for cmd in game_host.sound_commands.drain(..) {
                match cmd {
                    crate::natives::SoundCommand::SuspendAll => {
                        // SuspendAllSoundSources stops the audio
                        // channels but the paired `ResumeAll` must be
                        // able to restart every source that was active
                        // at suspend time.  We clear `active` so the
                        // hourglass stops channels, but first stash the
                        // active set on `sound_sim` so `ResumeAll` can
                        // restore it.
                        let mut stashed: Vec<u32> = Vec::new();
                        for i in 0..self.sound_sim.sources.num_sources() {
                            if let Some(src) = self.sound_sim.sources.get_mut(i)
                                && src.active
                            {
                                stashed.push(i as u32);
                                src.active = false;
                            }
                        }
                        self.sound_sim.suspended_active_sources = stashed;
                        self.sound_sim.playing_sources.clear();
                    }
                    crate::natives::SoundCommand::ResumeAll => {
                        // Restore `active` on every source that was
                        // active at the last suspend — preserves the
                        // active flag across suspend/resume.
                        let stashed = std::mem::take(&mut self.sound_sim.suspended_active_sources);
                        for idx in stashed {
                            if let Some(src) = self.sound_sim.sources.get_mut(idx as usize) {
                                src.active = true;
                            }
                        }
                        let pos = self.cutscene_camera.view_position;
                        let zoom = self.cutscene_camera.zoom_factor;
                        self.pending_side_effects.sounds.push(
                            super::SoundCommand::ResumeAllSources {
                                position: pos,
                                zoom,
                            },
                        );
                        // For every still-active `Single` / `Volatile`
                        // source that's being resumed, re-arm the
                        // deterministic finish so the drain in
                        // `perform_hourglass` applies the same
                        // transition the host used to drive from
                        // `stop_sound_source`.
                        schedule_source_finishes_for_all_active(
                            &mut self.sound_sim,
                            &assets.source_durations,
                            self.frame_counter,
                        );
                    }
                    crate::natives::SoundCommand::Activate(h) => {
                        // Mark active sim-side (participates in rollback hash),
                        // then emit the side-effect so the host audio backend
                        // picks up the source and starts a channel.  Symmetric
                        // with the Deactivate path below.
                        if let Some(idx) = crate::natives::GameHost::sound_source_index(h) {
                            // Re-activation cancels any previously
                            // scheduled finish so we don't prematurely
                            // kill a freshly-restarted source.
                            self.sound_sim
                                .playing_sources
                                .retain(|p| p.source_index as usize != idx);
                            if let Some(src) = self.sound_sim.sources.get_mut(idx) {
                                src.active = true;
                                schedule_source_finish(
                                    &src.source_kind,
                                    src.id,
                                    idx,
                                    self.frame_counter,
                                    &assets.source_durations,
                                    &mut self.sound_sim.playing_sources,
                                );
                            }
                            self.pending_side_effects
                                .sounds
                                .push(super::SoundCommand::ActivateSource(idx));
                        }
                    }
                    crate::natives::SoundCommand::Deactivate(h) => {
                        // Mark inactive; hourglass will stop the channel.
                        // Drop any pending scheduled finish — the source
                        // is no longer playing and a stale `finish_frame`
                        // would fire as a no-op on an already-inactive
                        // source, but clearing it keeps the queue small
                        // and unambiguous across rollback snapshots.
                        if let Some(idx) = crate::natives::GameHost::sound_source_index(h) {
                            if let Some(src) = self.sound_sim.sources.get_mut(idx) {
                                src.active = false;
                            }
                            self.sound_sim
                                .playing_sources
                                .retain(|p| p.source_index as usize != idx);
                        }
                    }
                    crate::natives::SoundCommand::Destroy(h) => {
                        if let Some(idx) = crate::natives::GameHost::sound_source_index(h) {
                            if let Some(src) = self.sound_sim.sources.get_mut(idx) {
                                src.active = false;
                            }
                            self.sound_sim.sources.delete(idx);
                            self.sound_sim
                                .playing_sources
                                .retain(|p| p.source_index as usize != idx);
                        }
                    }
                }
            }

            // ── Background invalidation ──
            if game_host.background_invalidated {
                self.pending_side_effects.invalidate_background = true;
                game_host.background_invalidated = false;
            }

            // ── ForceCheckVictory ──
            if game_host.force_check {
                self.force_check = true;
                game_host.force_check = false;
            }

            // ── Camera / UI commands ──
            engine_commands = game_host.drain_commands();

            // ── Completed sequences (from Record*/Thanx) ──
            for seq in game_host.take_completed_sequences() {
                self.launch_sequence(seq);
            }

            // ── Deferred game-logic commands ──
            // NB: `ProcessPatchEffects` reads `self.mission_script`, which is
            // currently taken out of `self` — so that arm is deferred to
            // `post_script` and processed after the script is put back.
            for cmd in game_host.deferred_commands.drain(..) {
                match cmd {
                    crate::natives::DeferredCommand::SendMessage {
                        actor,
                        message,
                        arg1,
                        arg2,
                    } => script_messages.push((actor, message, arg1, arg2)),
                    crate::natives::DeferredCommand::SelectPC { actor, select } => {
                        // Scripted scene: targets the LOCAL seat.
                        if actor == 0 {
                            // NULL actor → select/deselect all
                            if select {
                                self.select_all_pcs(assets, 0);
                            } else {
                                self.unselect_all_pcs(0);
                            }
                        } else if let Some(id) = self.entity_id_for_actor_handle(actor) {
                            if select {
                                // Script-path SelectPC uses `speak=false`
                                // — script already owns the sound flow.
                                self.select_pc(assets, 0, id, true, false);
                            } else {
                                self.seats[0].selection.retain(|&x| x != id);
                            }
                        }
                    }
                    crate::natives::DeferredCommand::StopActor { actor } => {
                        if let Some(id) = self.entity_id_for_actor_handle(actor) {
                            self.stop_owner(id, crate::sequence::SequencePriority::Script);
                        }
                    }
                    crate::natives::DeferredCommand::FreezeAll { freeze } => {
                        self.freeze_all = freeze;
                    }
                    crate::natives::DeferredCommand::HandleDeath { actor } => {
                        if let Some(id) = self.entity_id_for_actor_handle(actor) {
                            self.handle_death(assets, id);
                        }
                    }
                    crate::natives::DeferredCommand::SpawnDamageNumber { actor, damage } => {
                        if let Some(id) = self.entity_id_for_actor_handle(actor) {
                            self.add_damage_number(id, damage);
                        }
                    }
                    crate::natives::DeferredCommand::PcSayOuchForLifeDrop { actor, damage } => {
                        if let Some(id) = self.entity_id_for_actor_handle(actor) {
                            self.say_ouch(assets, id, Some(damage));
                        }
                    }
                    crate::natives::DeferredCommand::SetScriptedLifePoints { actor, amount } => {
                        if let Some(id) = self.entity_id_for_actor_handle(actor) {
                            self.apply_scripted_life_points(assets, id, amount);
                        }
                    }
                    crate::natives::DeferredCommand::SetScriptedConcussion {
                        actor,
                        amount,
                        force_value,
                    } => {
                        if let Some(id) = self.entity_id_for_actor_handle(actor) {
                            // Clamp negative `i32` from the script stack to 0
                            // before casting; `combat::set_concussion` clamps
                            // the upper bound to `CONCUSSION_MAX`.
                            let value = amount.max(0).min(u16::MAX as i32) as u16;
                            self.apply_concussion(assets, id, value, force_value);
                        }
                    }
                    crate::natives::DeferredCommand::QuitSwordfight { actor } => {
                        if let Some(id) = self.entity_id_for_actor_handle(actor) {
                            self.quit_swordfight(assets, id);
                        }
                    }
                    crate::natives::DeferredCommand::RemoveUnconsciousStars { actor } => {
                        // The titbit is only dropped when the actor is *not*
                        // currently unconscious — `remove_unconscious_stars_if`
                        // takes `is_still_unconscious` and short-circuits
                        // otherwise.  Read the live human-data flag now.
                        if let Some(id) = self.entity_id_for_actor_handle(actor)
                            && let Some(entity) = self.entities.get(id)
                        {
                            let still_unconscious =
                                entity.human_data().is_some_and(|h| h.unconscious);
                            self.titbit_manager.remove_unconscious_stars_if(
                                crate::titbit::ElementHandle(id.index()),
                                still_unconscious,
                            );
                        }
                    }
                    crate::natives::DeferredCommand::SetPlayable { actor, playable } => {
                        // PC playable state (pc.playable) was already set on
                        // the entity by the native call. Forward
                        // MSG_ENABLE/DISABLE_CHARACTER to the messenger
                        // carrying the actor's entity id so the handler
                        // can drop the PC from the selection and update
                        // Sherwood interface-hidden state.
                        let msg_type = if playable {
                            crate::messenger::PcMessage::EnableCharacter
                        } else {
                            crate::messenger::PcMessage::DisableCharacter
                        };
                        let pc_id = self.entity_id_for_actor_handle(actor);
                        self.messenger.send(Message::pc(msg_type, pc_id));
                        tracing::debug!("SetPlayable: actor {actor} → playable={playable}");
                    }
                    crate::natives::DeferredCommand::ScriptLockAI { actor, send_back } => {
                        // Script-lock an NPC's AI. Two callers:
                        //   - SetActorLocation honolulu path (NPC sent
                        //     to a null location); always passes
                        //     `send_back=false`.
                        //   - LockAI script native; `send_back` is the
                        //     remember-events arg.
                        // ScriptLockAI suppresses `Stop()` only when the
                        // actor's current command is already `LockAi`.
                        // We implement that by peeking the sequence
                        // manager for the actor's in-flight command.
                        if let Some(owner) = self.entity_id_for_actor_handle(actor) {
                            let from_lockai_command = self
                                .sequence_manager
                                .current_element_for_actor(owner)
                                .and_then(|(seq_id, elem_idx)| {
                                    self.sequence_manager.get_element(seq_id, elem_idx)
                                })
                                .is_some_and(|elem| {
                                    elem.command == crate::element::Command::LockAi
                                });
                            if let Some(entity) = self.entities.get_mut(owner)
                                && let Some(ai) = entity.ai_controller_mut()
                            {
                                ai.script_lock(send_back, from_lockai_command);
                            }
                        }
                        tracing::debug!("ScriptLockAI: actor {actor}, send_back={send_back}");
                    }
                    cmd @ crate::natives::DeferredCommand::ProcessPatchEffects { .. } => {
                        post_script.push(cmd);
                    }
                    crate::natives::DeferredCommand::PutActorInBuilding { actor, building } => {
                        self.put_actor_in_building(actor, building);
                    }
                    crate::natives::DeferredCommand::ResetSpriteFrame { actor } => {
                        // Rewind the actor's sprite to frame 0 of its current row.
                        if let Some(id) = self.entity_id_for_actor_handle(actor)
                            && let Some(entity) = self.entities.get_mut(id)
                        {
                            entity.sprite_mut().reset_sprite_frame(false);
                        }
                    }
                    crate::natives::DeferredCommand::ClearAllQuickActionSlots { actor } => {
                        // Per-slot `SetQuickActionSequence(0, 0, i, 0xFFFFFFFF)`
                        // loop: drops QA titbits + clears macro_store slot.
                        if let Some(pc_id) = self.entity_id_for_actor_handle(actor) {
                            for slot in 0..crate::macro_store::NUMBER_OF_QA_MEMORY as u8 {
                                self.remove_quick_action_titbits_for(pc_id, slot);
                                if let Some(state) = self.macro_store.get_mut(pc_id) {
                                    state.clear_slot(slot as usize);
                                }
                            }
                        }
                    }
                    crate::natives::DeferredCommand::LaunchWait { actor } => {
                        // Build a fresh `SequenceElement(1, Wait, owner)`
                        // at `Wait` priority and hand it to the sequence
                        // manager so the instruct arbitration displaces
                        // any lower-or-equal-priority sequence the actor
                        // was running.  Called from `SetActorPosture`,
                        // `SetActorActionState` (every arm), etc., right
                        // after the script stamps the new posture/action-state.
                        if let Some(owner) = self.entity_id_for_actor_handle(actor) {
                            let mut elem = crate::sequence::SequenceElement::new(
                                1,
                                crate::element::Command::Wait,
                                Some(owner),
                            );
                            elem.priority = crate::sequence::SequencePriority::Wait;
                            self.sequence_manager.launch_element(elem);
                        } else {
                            tracing::warn!("LaunchWait: invalid actor handle {actor}");
                        }
                    }
                    crate::natives::DeferredCommand::StopActorAtPriority { actor, priority } => {
                        // `Stop(priority)` invoked outside the StopActor
                        // native; currently driven by `SetActorPosture` ID_KO
                        // at `Injury` priority.  Routes through the engine's
                        // wrapper so movement/path-request teardown stays
                        // in sync with the sequence-manager stop.
                        if let Some(id) = self.entity_id_for_actor_handle(actor) {
                            self.stop_owner(id, priority);
                        }
                    }
                    crate::natives::DeferredCommand::BroadcastLoseConsciousness { actor } => {
                        // `Think(EVENT_LOSE_CONSCIOUSNESS) +
                        // BroadcastBodyDetectable()` invoked from
                        // `SetActorPosture` ID_KO/ID_TIED arms.  Both are
                        // NPC-only (guarded by `is_npc()`); we no-op when
                        // the entity has no AI controller.
                        if let Some(id) = self.entity_id_for_actor_handle(actor) {
                            // Queue stimulus first so the AI's next think
                            // tick observes the "lose consciousness" event
                            // before the detect-me broadcast lands on
                            // friends — ordering matters here.
                            if let Some(entity) = self.entities.get_mut(id)
                                && let Some(ai) = entity.ai_controller_mut()
                            {
                                ai.pending_stimuli.push(crate::ai::Stimulus::new(
                                    crate::ai::StimulusType::EventLoseConsciousness,
                                ));
                            }
                            // Only NPCs broadcast their body — guard via
                            // `is_npc()` to avoid touching a PC or non-actor
                            // slot.
                            if let Some(entity) = self.entities.get(id)
                                && entity.is_npc()
                            {
                                self.broadcast_body_detectable(id);
                            }
                        }
                    }
                    crate::natives::DeferredCommand::BroadcastResurrection { actor } => {
                        // From the `SetActorPosture` ID_UPRIGHT/LYING NPC
                        // branch.  The engine-side `broadcast_resurrection`
                        // walks every other NPC and clears the resurrected
                        // NPC from their `DETECTABLE_BODY` list.
                        if let Some(id) = self.entity_id_for_actor_handle(actor)
                            && let Some(entity) = self.entities.get(id)
                            && entity.is_npc()
                        {
                            self.broadcast_resurrection(id);
                        }
                    }
                    crate::natives::DeferredCommand::AddHiddenTitbitForActor { actor } => {
                        // From the `SetActorPosture` ID_ANONYMOUS_ARCHER
                        // arm: add a HIDDEN titbit for the actor.  The
                        // script bypasses the stealth-command transition
                        // that normally adds the HIDDEN titbit
                        // (`engine/tick.rs:5318`), so we replicate the
                        // add here.  Phase resolution (`HiddenCharacter`)
                        // requires a PC profile; for an NPC the original
                        // would deref a non-PC as PC (UB), so we guard
                        // and log instead — script callers in shipping
                        // levels only target PCs.
                        let Some(id) = self.entity_id_for_actor_handle(actor) else {
                            continue;
                        };
                        let Some(entity) = self.entities.get(id) else {
                            continue;
                        };
                        let phase = if let crate::element::Entity::Pc(pc) = entity {
                            let profile = assets
                                .profile_manager
                                .get_character(pc.pc.profile_index)
                                .unwrap_or_else(|| {
                                    panic!(
                                        "AddHiddenTitbitForActor: PC {} has unknown profile_index {}",
                                        id.index(), pc.pc.profile_index
                                    )
                                });
                            crate::titbit::HiddenCharacter::for_pc(pc.pc.robin, &profile.filename)
                                .to_phase()
                        } else {
                            tracing::warn!(
                                "AddHiddenTitbitForActor: actor {actor} is not a PC; \
                                 skipping HIDDEN titbit (original would deref non-PC as PC)"
                            );
                            continue;
                        };
                        let handle = crate::titbit::ElementHandle(id.index());
                        self.titbit_manager.add_titbit(
                            crate::coordinates::WorldPoint3D::default(),
                            0,
                            crate::titbit::TitbitKind::Hidden,
                            handle,
                            phase,
                            handle,
                            false,
                            0,
                            true,
                            None,
                            None,
                        );
                    }
                    crate::natives::DeferredCommand::RelaunchPathAtNewSpeed { actor } => {
                        // From the `SetPathWalkingFlags` relaunch tail:
                        // re-issue GoTo at the freshly-changed walking
                        // flags so the speed change takes effect
                        // mid-segment instead of waiting for the next
                        // waypoint pickup.
                        if let Some(id) = self.entity_id_for_actor_handle(actor) {
                            self.relaunch_path_at_new_speed(assets, id);
                        }
                    }
                    crate::natives::DeferredCommand::SetPatrolShouldRun {
                        actor: _,
                        should_run: _,
                    } => {
                        // Spellforge `SetPatrolShouldRun` — no engine
                        // handler yet. The patrol walk/run toggle
                        // lives on the patrol descriptor; wiring it
                        // up is tracked alongside the Lua mission
                        // bring-up.
                        // TODO: stamp `patrol.should_run` and re-issue
                        // the in-flight GoTo via the existing
                        // `RelaunchPathAtNewSpeed` flow.
                    }
                }
            }
        }

        // Put the script back before applying engine commands (which may
        // need to look up entities, etc.) and before post-script deferred
        // commands that read `self.mission_script`.
        self.mission_script = Some(script);

        for cmd in post_script {
            match cmd {
                crate::natives::DeferredCommand::ProcessPatchEffects {
                    patch_index,
                    effects,
                } => {
                    self.process_patch_effects(assets, patch_index, effects);
                }
                _ => unreachable!("only ProcessPatchEffects is deferred post-script"),
            }
        }

        for (actor, message, arg1, arg2) in script_messages {
            self.launch_script_send_message(assets, actor, message, arg1, arg2);
        }

        if !engine_commands.is_empty() {
            self.apply_host_commands(assets, engine_commands);
        }
    }

    /// Launch and synchronously execute the one-element sequence created by
    /// `RHScript::SendMessage[WithArguments]`.
    ///
    /// The original route is `LaunchSequenceElement` →
    /// `RegisterSequenceElementToGo` → `ExecutedImmediately`. The last
    /// step calls `ExecuteImmediately` directly, so an owner-bound message
    /// deliberately bypasses `Instruct` priority contention and leaves the
    /// actor's current sequence untouched. `ProcessMessage` runs before the
    /// SendMessage element changes from `Todo` to `Terminated`.
    fn launch_script_send_message(
        &mut self,
        assets: &LevelAssets,
        actor: i32,
        message: i32,
        arg1: i32,
        arg2: i32,
    ) {
        let owner = if actor == 0 {
            None
        } else {
            let Some(owner) = self.entity_id_for_actor_handle(actor) else {
                tracing::warn!(
                    actor,
                    message,
                    "SendMessage target disappeared before sequence launch"
                );
                return;
            };
            Some(owner)
        };

        let mut element = crate::sequence::SequenceElement::new_generic(
            1,
            crate::element::Command::SendMessage,
            owner,
        );
        element.set_property(
            crate::sequence::Field::Message,
            crate::sequence::FieldValue::Integer(message as u32),
        );
        element.set_property(
            crate::sequence::Field::MessageArgument,
            crate::sequence::FieldValue::Integer(arg1 as u32),
        );
        element.set_property(
            crate::sequence::Field::MessageExtendedArgument,
            crate::sequence::FieldValue::Integer(arg2 as u32),
        );
        let mut sequence = crate::sequence::Sequence::new();
        sequence.append_element(element);

        // Use LaunchSequence rather than the owned LaunchElement/Instruct
        // wrapper. RHCOMMAND_SEND_MESSAGE is in ExecutedImmediately(), so
        // the original never arbitrates it against the actor's current
        // element.
        let sequence_id = self.launch_sequence(sequence);
        let action = self
            .sequence_manager
            .take_pending_immediate_action_for(sequence_id, 0)
            .unwrap_or_else(|| {
                panic!(
                    "SendMessage sequence {:?} did not register its immediate action",
                    sequence_id
                )
            });

        match action {
            crate::sequence::SequenceAction::ExecuteImmediateOwner {
                owner: action_owner,
                sequence_id: action_sequence_id,
                element_index: 0,
            } => {
                assert_eq!(Some(action_owner), owner);
                assert_eq!(action_sequence_id, sequence_id);
                self.dispatch_sequence_messages(assets, &[(actor, message, arg1, arg2)], &[]);
            }
            crate::sequence::SequenceAction::ExecuteImmediateEngine {
                sequence_id: action_sequence_id,
                element_index: 0,
            } => {
                assert!(owner.is_none());
                assert_eq!(action_sequence_id, sequence_id);
                self.dispatch_sequence_messages(assets, &[], &[(message, arg1, arg2)]);
            }
            other => panic!(
                "SendMessage sequence {:?} registered unexpected action {:?}",
                sequence_id, other
            ),
        }

        // RHEngine/RHElementActor set RHSEQ_TERMINATED only after
        // ProcessMessage returns.
        self.sequence_manager.element_terminated(sequence_id, 0);
    }

    /// Load a mission script from the level directory.
    ///
    /// Looks up the pre-decoded script program in
    /// `assets.mission_script_programs` and installs it into
    /// `self.mission_script`.
    pub(crate) fn load_mission_script(&mut self, assets: &LevelAssets, scb_path: &std::path::Path) {
        let stem = scb_path.file_stem().and_then(|s| s.to_str());
        let program = stem.and_then(|name| {
            assets
                .mission_script_programs
                .get(name)
                .map(std::sync::Arc::clone)
        });
        let result = if let (Some(name), Some(program)) = (stem, program) {
            tracing::info!(
                "Mission script {}: loaded from LevelAssets",
                scb_path.display()
            );
            MissionScript::from_program(name.to_owned(), program)
        } else {
            Err(format!(
                "no mission script registered for {}",
                scb_path.display()
            ))
        };
        match result {
            Ok(script) => {
                tracing::info!(
                    "Loaded mission script: {} ({} classes)",
                    scb_path.display(),
                    script.manager.class_count(),
                );
                self.mission_script = Some(script);
            }
            Err(e) => {
                tracing::warn!("Could not load mission script {}: {e}", scb_path.display());
            }
        }
    }

    /// Initialize the loaded mission script.
    ///
    /// Three-phase init:
    /// 1. **Per-waypoint binding** — for each waypoint in `hiking_paths`
    ///    with a script class, bind it and run `IWaypointScript::Initialize()`.
    /// 2. **Per-actor Initialize** — for each entity with a `script_class`,
    ///    create a temporary `ScriptInstance` bound to that class and call
    ///    its `Initialize()`.  Runs during entity loading.
    /// 3. **Global StartUp::Initialize(seed)** — the main mission script init.
    ///
    /// Called from `Engine::new` once the level loader has populated
    /// `assets.hiking_paths`.
    pub(crate) fn initialize_mission_script_with(
        &mut self,
        assets: &LevelAssets,
        seed: i32,
        hiking_paths: &[crate::level_data::RawHikingPath],
    ) {
        self.refresh_game_host_entity_state();
        self.install_script_static_data_into_game_host(assets);

        // Collect per-actor script classes before swapping entities into GameHost.
        // Each actor with a script_class gets IActorScript::Initialize()
        // called during loading (before StartUp::Initialize).
        let per_actor_scripts: Vec<(i32, String)> = self
            .entities
            .actors()
            .filter_map(|(entity_id, entity)| {
                let script_class = &entity.actor_data()?.script_class;
                if script_class.is_empty() {
                    return None;
                }
                Some((
                    crate::natives::GameHost::actor_handle(entity_id),
                    script_class.clone(),
                ))
            })
            .collect();

        // Same collection pass for FX targets — each target with a
        // non-empty `script_class` gets its own `ScriptInstance`.
        // Each target carries its own VM and `Initialize()` runs
        // during `InitializeFromMissionStream`.
        let per_target_scripts: Vec<(i32, String)> = self
            .entities
            .targets()
            .filter_map(|(entity_id, target)| {
                if target.target.script_class.is_empty() {
                    return None;
                }
                Some((
                    crate::natives::GameHost::actor_handle(entity_id),
                    target.target.script_class.clone(),
                ))
            })
            .collect();

        // Scrolls also carry their own VMs — bind the class during
        // `InitializeFromMissionStream` and walk the list calling
        // `IScrollScript::Initialize()`.
        let per_scroll_scripts: Vec<(i32, String)> = self
            .entities
            .scrolls()
            .filter_map(|(entity_id, scroll)| {
                if scroll.script_class.is_empty() {
                    return None;
                }
                Some((
                    crate::natives::GameHost::actor_handle(entity_id),
                    scroll.script_class.clone(),
                ))
            })
            .collect();

        if let Some(ref mut script) = self.mission_script {
            if let Some(game_host) = script.game_host_mut() {
                game_host.frame_counter = self.frame_counter;
            }
            script.swap_engine_state(
                &mut self.entities,
                &mut self.ai_global,
                &mut self.fast_grid,
                &mut self.campaign,
                &mut self.mission_stat,
            );

            // ── Phase 1: Per-actor Initialize ──
            // Each actor's script class gets a ScriptInstance that persists for the
            // actor's lifetime — the heap (member variables) survives across calls
            // to Initialize, ActionChange, HandleEvent, FilterAIEvent, ProcessMessage.
            // The GameHost is transferred in/out for each call (it lives on the
            // global StartUp instance between calls). All the host-transfer and
            // Initialize dispatch plumbing lives in `MissionScript::bind_actor`.
            let mut init_count = 0u32;
            for (handle, class_name) in &per_actor_scripts {
                if script.bind_actor(*handle, class_name) {
                    init_count += 1;
                }
            }
            if init_count > 0 {
                tracing::info!(
                    "Ran per-actor Initialize on {init_count} entities \
                     ({} instances persisted)",
                    script.actor_instances.len()
                );
            }

            // ── Phase 1b: Per-target Initialize ──
            // Run `IElementTargetScript::Initialize()` during
            // `InitializeFromMissionStream`.
            let mut target_init_count = 0u32;
            for (handle, class_name) in &per_target_scripts {
                if script.bind_target(*handle, class_name) {
                    target_init_count += 1;
                }
            }
            if target_init_count > 0 {
                tracing::info!(
                    "Ran per-target Initialize on {target_init_count} targets \
                     ({} instances persisted)",
                    script.target_instances.len()
                );
            }

            // ── Phase 1c: Per-scroll Initialize ──
            // Walk every scroll and run `IScrollScript::Initialize()`
            // on the bound class.
            let mut scroll_init_count = 0u32;
            for (handle, class_name) in &per_scroll_scripts {
                if script.bind_scroll(*handle, class_name) {
                    scroll_init_count += 1;
                }
            }
            if scroll_init_count > 0 {
                tracing::info!(
                    "Ran per-scroll Initialize on {scroll_init_count} scrolls \
                     ({} instances persisted)",
                    script.scroll_instances.len()
                );
            }

            // ── Phase 1d: Per-waypoint Initialize ──
            // For each scripted waypoint, call `Bind(class)` +
            // `IWaypointScript::Initialize()` during mission load.
            // Each waypoint is its own VM instance so the heap
            // persists across traversals.
            let mut wp_init_count = 0u32;
            for (path_idx, path) in hiking_paths.iter().enumerate() {
                for (wp_idx, wp) in path.waypoints.iter().enumerate() {
                    let crate::level_data::WaypointCommand::Script(ref class_name) = wp.command
                    else {
                        continue;
                    };
                    if class_name.is_empty() {
                        continue;
                    }
                    let Some(pid) = crate::ai::PathId::new(path_idx as u16) else {
                        continue;
                    };
                    if script.bind_waypoint(pid, wp_idx as u8, class_name) {
                        wp_init_count += 1;
                    }
                }
            }
            if wp_init_count > 0 {
                tracing::info!(
                    "Ran per-waypoint Initialize on {wp_init_count} waypoints \
                     ({} instances persisted)",
                    script.waypoint_instances.len()
                );
            }

            // ── Phase 2: Global StartUp::Initialize(seed) ──
            let startup_result = MissionScript::with_game_host_attached(
                &mut script.game_host,
                &mut script.instance,
                |instance, host| {
                    instance.push_param(seed);
                    instance.call_function_limited_with_host(
                        &mut script.manager,
                        "Initialize",
                        100_000,
                        host,
                    )
                },
            );
            match startup_result {
                Ok(ret) => tracing::info!("Script StartUp::Initialize returned {ret}"),
                Err(crate::script_manager::ScriptError::Vm(
                    crate::interp::StopReason::StepLimit,
                )) => {
                    tracing::warn!("Script StartUp::Initialize hit step limit (100K)");
                }
                Err(e) => tracing::warn!("Script StartUp::Initialize failed: {e}"),
            }
            script.swap_engine_state(
                &mut self.entities,
                &mut self.ai_global,
                &mut self.fast_grid,
                &mut self.campaign,
                &mut self.mission_stat,
            );
        }
        self.sync_game_host_post_script(assets);

        // ── Mark AiControllers whose bound class overrides FilterAIEvent ──
        // Read by cascade `think()` sites in ai_enemy.rs to decide
        // whether to warn about the "would re-filter here, didn't"
        // divergence.  Unscripted NPCs leave the flag at its default
        // `false` and stay silent.  Entities have just been swapped
        // back, so this iteration sees the real engine state.
        if let Some(script) = self.mission_script.as_ref() {
            let scripted_actors: Vec<i32> = script.actor_instances.keys().copied().collect();
            for handle in scripted_actors {
                let has_override = script.actor_has_function(handle, "FilterAIEvent");
                if !has_override {
                    continue;
                }
                let Some(id) = self.entity_id_for_actor_handle(handle) else {
                    continue;
                };
                if let Some(entity) = self.entities.get_mut(id)
                    && let Some(ai) = entity.ai_controller_mut()
                {
                    ai.has_script_filter_override = true;
                }
            }
        }

        // ── Phase 3: Zone script Initialize ──
        self.initialize_zone_scripts();

        // ── Phase 3b: Apply SectorProduction registrations from StartUp::Initialize.
        // RegisterAsProductionSector / AddProductionPoint queue into GameHost; the
        // engine drains them here so the zone-occupant step (Phase 4) can emit
        // SetWorkicon for initial occupants.
        self.apply_production_registrations(assets);

        // ── Phase 4: Populate initial zone occupants ──
        self.initialize_zone_occupants(assets);
    }

    /// Finalize the mission script (called on mission end).
    /// `abandoned` is true if the player quit/interrupted.
    pub(crate) fn finalize_mission_script(&mut self, abandoned: bool) {
        if let Some(ref mut script) = self.mission_script {
            if let Some(game_host) = script.game_host_mut() {
                game_host.frame_counter = self.frame_counter;
            }
            script.swap_engine_state(
                &mut self.entities,
                &mut self.ai_global,
                &mut self.fast_grid,
                &mut self.campaign,
                &mut self.mission_stat,
            );
            if let Err(e) = script.finalize(abandoned) {
                tracing::warn!("Script Finalize failed: {e}");
            }
            script.swap_engine_state(
                &mut self.entities,
                &mut self.ai_global,
                &mut self.fast_grid,
                &mut self.campaign,
                &mut self.mission_stat,
            );
        }
    }

    // ─── Per-actor script event dispatch ───────────────────────────

    /// Check all scripted actors for animation changes and dispatch
    /// `ActionChange(newAction, oldAction)` to their per-actor scripts.
    ///
    /// Calls `ActionChange` when the current animation differs from
    /// `old_action`.  Called once per frame from `perform_hourglass`,
    /// after all animation updates.
    pub(crate) fn dispatch_actor_action_changes(&mut self, assets: &LevelAssets) {
        if self.mission_script.is_none() {
            return;
        }

        // Phase 1: Collect actors whose animation changed.
        // Current animation = front order of the actor's current
        // in-progress sequence element.
        let mut changes = Vec::new();
        for (entity_id, entity) in self.entities.actors() {
            let Some(actor) = entity.actor_data() else {
                continue;
            };
            if actor.script_class.is_empty() {
                continue;
            }

            let current_anim = self
                .sequence_manager
                .current_order_for_actor(entity_id)
                .map(|(_, _, o)| o.order_type)
                .unwrap_or(crate::order::OrderType::WaitingUpright);

            if current_anim != actor.old_action {
                changes.push((entity_id, current_anim, actor.old_action));
            }
        }
        // Apply old_action updates in a second pass (the peek loop
        // above only reads self.entities to avoid conflicting with the
        // sequence_manager borrow).
        for &(entity_id, new_anim, _) in &changes {
            if let Some(entity) = self.entities.get_mut(entity_id)
                && let Some(actor) = entity.actor_data_mut()
            {
                actor.old_action = new_anim;
            }
        }
        let changes: Vec<(i32, i32, i32)> = changes
            .into_iter()
            .map(|(entity_id, new_anim, old_anim)| {
                (
                    crate::natives::GameHost::actor_handle(entity_id),
                    new_anim as i32,
                    old_anim as i32,
                )
            })
            .collect();

        if changes.is_empty() {
            return;
        }

        // Phase 2: Dispatch to scripts with engine state swapped in.
        self.refresh_game_host_entity_state();
        let script = self.mission_script.as_mut().unwrap();
        script.swap_engine_state(
            &mut self.entities,
            &mut self.ai_global,
            &mut self.fast_grid,
            &mut self.campaign,
            &mut self.mission_stat,
        );

        for (handle, new_anim, old_anim) in &changes {
            if let Err(e) =
                script.call_actor_function(*handle, "ActionChange", &[*new_anim, *old_anim])
            {
                tracing::warn!("ActionChange (handle {handle}): {e}");
            }
        }

        script.swap_engine_state(
            &mut self.entities,
            &mut self.ai_global,
            &mut self.fast_grid,
            &mut self.campaign,
            &mut self.mission_stat,
        );
        self.sync_game_host_post_script(assets);
    }

    /// Per-frame scroll script `Hourglass(0)` dispatch.
    ///
    /// For every active scroll with a bound script (i.e.
    /// `scroll_instances.contains_key(handle)`), increment
    /// `script_hourglass_timeout` and, when it reaches
    /// `SCRIPT_HOURGLASS_TIMEOUT = 25`, fire `IScrollScript::Hourglass(0)`
    /// with the `SetScrollExecutingScript` bracket provided by
    /// [`MissionScript::call_scroll_function`], then reset the counter.
    ///
    /// Sprite frame advance for scrolls lives in the generic animation
    /// tick; this function only handles the per-25-tick script
    /// callback dispatched alongside the frame advance.
    pub(crate) fn dispatch_scroll_hourglasses(&mut self, assets: &LevelAssets) {
        const SCRIPT_HOURGLASS_TIMEOUT: u32 = 25;

        if self.mission_script.is_none() {
            return;
        }

        // Phase 1: bump timers; collect handles whose script is due to
        // fire this frame.  The mutable walk can't borrow `mission_script`
        // (it gets swapped into the script manager below), so the list
        // of ready-to-fire scrolls is captured first.
        let mut ready: Vec<i32> = Vec::new();
        for (id, s) in self.entities.scrolls_mut() {
            if !s.element.active {
                continue;
            }
            let handle = crate::natives::GameHost::actor_handle(id);
            let has_script = self
                .mission_script
                .as_ref()
                .is_some_and(|ms| ms.scroll_instances.contains_key(&handle));
            if !has_script {
                continue;
            }
            s.script_hourglass_timeout += 1;
            if s.script_hourglass_timeout >= SCRIPT_HOURGLASS_TIMEOUT {
                s.script_hourglass_timeout = 0;
                ready.push(handle);
            }
        }

        if ready.is_empty() {
            return;
        }

        // Phase 2: dispatch with engine state swapped in.
        self.refresh_game_host_entity_state();
        let script = self.mission_script.as_mut().unwrap();
        script.swap_engine_state(
            &mut self.entities,
            &mut self.ai_global,
            &mut self.fast_grid,
            &mut self.campaign,
            &mut self.mission_stat,
        );

        // Per-scroll `Hourglass` is distinct from the engine script
        // `Hourglass(seconds)`.  The shipped scroll path passes zero;
        // preserve that literal script ABI.
        for handle in &ready {
            if let Err(e) = script.call_scroll_function(*handle, "Hourglass", &[0]) {
                tracing::warn!("Scroll Hourglass (handle {handle}): {e}");
            }
        }

        script.swap_engine_state(
            &mut self.entities,
            &mut self.ai_global,
            &mut self.fast_grid,
            &mut self.campaign,
            &mut self.mission_stat,
        );
        self.sync_game_host_post_script(assets);
    }

    /// Dispatch `IScrollScript::IsTaken(pc)` for a scroll being picked up.
    ///
    ///   1. Flip the scroll's sprite to `BonusThree` (the "opened
    ///      scroll" pose).
    ///   2. Call the bound script's `IsTaken(pc)` inside the
    ///      `SetScrollExecutingScript` / `ResetScrollExecutingScript`
    ///      bracket (provided by [`MissionScript::call_scroll_function`]).
    ///   3. If the script returns non-zero, mark the scroll `Taken`
    ///      and return `true`.  Otherwise `false` — the scroll keeps
    ///      the `Opened` visual but stays in-world.
    ///
    /// Scrolls without a bound script return `false` with no status
    /// change.
    ///
    /// NB: the scroll-pickup pipeline itself (PC ↔ scroll proximity,
    /// `Action::TakeScroll` dispatch) is not yet ported; this helper
    /// exists so whatever wires that up next can fire the
    /// script-bracketed `IsTaken` dispatch with a single call.
    pub fn scroll_is_taken(
        &mut self,
        assets: &LevelAssets,
        scroll_id: crate::element::EntityId,
        pc_id: crate::element::EntityId,
    ) -> bool {
        use crate::element::Entity;
        use ScrollStatus;

        let handle = crate::natives::GameHost::actor_handle(scroll_id);

        // Step 1 — always flip to the "opened" pose, even if there's no
        // script.  Set status to Opened and force the sprite animation
        // *before* the script-bound check.
        if let Some(Entity::Scroll(s)) = self.get_entity_mut(scroll_id) {
            let dir = s.element.direction() as u16;
            s.element
                .sprite
                .force_animation(crate::order::OrderType::BonusThree, dir);
        } else {
            tracing::warn!(?scroll_id, "scroll_is_taken: entity is not a scroll");
            return false;
        }
        self.set_scroll_status(scroll_id, ScrollStatus::Opened);

        // Step 2 — if no script is bound, return false immediately,
        // leaving the status at Opened.
        let has_script = self
            .mission_script
            .as_ref()
            .is_some_and(|ms| ms.scroll_instances.contains_key(&handle));
        if !has_script {
            return false;
        }

        // Step 3 — dispatch via the SetScrollExecutingScript bracket.
        let pc_handle = crate::natives::GameHost::actor_handle(pc_id);
        self.refresh_game_host_entity_state();
        let script = self.mission_script.as_mut().unwrap();
        script.swap_engine_state(
            &mut self.entities,
            &mut self.ai_global,
            &mut self.fast_grid,
            &mut self.campaign,
            &mut self.mission_stat,
        );
        let result = script.call_scroll_function(handle, "IsTaken", &[pc_handle]);
        script.swap_engine_state(
            &mut self.entities,
            &mut self.ai_global,
            &mut self.fast_grid,
            &mut self.campaign,
            &mut self.mission_stat,
        );
        self.sync_game_host_post_script(assets);

        let accepted = match result {
            Ok(v) => v != 0,
            Err(e) => {
                tracing::warn!("Scroll IsTaken (handle {handle}): {e}");
                false
            }
        };

        if accepted {
            // Flip the status to `Taken` and refresh the minimap dot
            // on a successful take.
            self.set_scroll_status(scroll_id, ScrollStatus::Taken);
        }
        accepted
    }

    // ─── Zone script system ───────────────────────────────────────

    /// Initialize per-zone script instances and call `Initialize()` on each.
    ///
    /// Creates `ScriptInstance`s for each script zone that has a `script_class`,
    /// runs `Initialize()`, and stores them in `MissionScript::zone_instances`.
    /// Called during mission init, after script sectors are registered on the grid.
    pub(crate) fn initialize_zone_scripts(&mut self) {
        let script = match self.mission_script.as_mut() {
            Some(s) => s,
            None => return,
        };

        let mut init_count = 0u32;
        for (zone_idx, zone_data) in self.script_zone_data.iter().enumerate() {
            let class_name = match &zone_data.script_class_name {
                Some(name) => name.clone(),
                None => continue,
            };

            let class_idx = match script.manager.find_class(&class_name) {
                Some(idx) => idx,
                None => {
                    // The original fires a fatal "Structural error in RHD,
                    // a Sector has got a script reference that does not
                    // exist!" — we escalate to `error!` rather than
                    // panicking so authoring breakage is loud without
                    // killing the engine outright.
                    tracing::error!(
                        "Structural error in RHD: zone {zone_idx} references script class \
                         '{class_name}' which does not exist in the SCB — zone will run unbound"
                    );
                    continue;
                }
            };

            let mut zone_inst = script.manager.create_instance_idx(class_idx);

            MissionScript::with_game_host_attached(
                &mut script.game_host,
                &mut zone_inst,
                |zone_inst, host| {
                    if zone_inst.has_function(&script.manager, "Initialize") {
                        match zone_inst.call_function_limited_with_host(
                            &mut script.manager,
                            "Initialize",
                            10_000,
                            host,
                        ) {
                            Ok(ret) => {
                                tracing::debug!(
                                    "Zone Init '{class_name}' (zone {zone_idx}) → {ret}"
                                );
                                init_count += 1;
                            }
                            Err(crate::script_manager::ScriptError::Vm(
                                crate::interp::StopReason::StepLimit,
                            )) => {
                                init_count += 1;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Zone Init '{class_name}' (zone {zone_idx}) failed: {e}"
                                )
                            }
                        }
                    }
                },
            );
            script.zone_instances.insert(zone_idx, zone_inst);
        }

        if init_count > 0 {
            tracing::info!(
                "Initialized {init_count} zone scripts ({} instances persisted)",
                script.zone_instances.len()
            );
        }
    }

    /// Scan all actors against all script-zone polygons and return the
    /// `(zone_idx, entity_idx, handle)` tuples for every actor that lies
    /// inside a zone.  Pure read helper — no state is mutated.
    ///
    /// Implements the "scan candidates → IsReallyInside" half of zone
    /// occupant initialization.  We walk every zone linearly rather
    /// than consulting a spatial index — same observable result
    /// (`contains_point` is bbox + polygon point-in-test), just
    /// O(actors × zones) rather than the per-cell narrowing the
    /// original used.  Documented perf gap.
    fn scan_zone_occupant_entries(
        &self,
        assets: &LevelAssets,
    ) -> Vec<(usize, crate::entity_id::EntityId, i32)> {
        let mut entries: Vec<(usize, crate::entity_id::EntityId, i32)> = Vec::new();
        if assets.script_zone_grid_indices.is_empty() {
            return entries;
        }
        for (actor_id, entity) in self.entities.actors() {
            let entity_id = actor_id.into();
            let ed = entity.element_data();
            // `in_honolulu` stands in for the `IsInside(GetBoxMap())`
            // reject — honolulu actors are parked off-map.  The extra
            // `!active` guard is a deliberate divergence; see the
            // `InitializeScriptSectorOccupants` parity entry.
            if !ed.active || ed.in_honolulu {
                continue;
            }
            let pos = ed.position_map();
            let layer = ed.layer();
            let handle = crate::natives::GameHost::actor_handle(actor_id);

            for (zone_idx, &grid_idx) in assets.script_zone_grid_indices.iter().enumerate() {
                // Skip zones that `DefineFlatTrajectoryZone` converted
                // into apex sectors — once converted, the SECTOR_SCRIPT
                // flag is dropped so the engine stops scanning them.
                if self
                    .script_zone_data
                    .get(zone_idx)
                    .is_some_and(|z| z.transformed_to_apex)
                {
                    continue;
                }
                let gs = &self.fast_grid.level.sectors[grid_idx as usize];
                if gs.layer == layer && gs.contains_point(pos) {
                    entries.push((zone_idx, entity_id, handle));
                }
            }
        }
        // Carried-recursion: a PC entering a zone also recursively
        // enters its carried actor.  The polygon scan above normally
        // catches a sync'd carried, but when the carried is excluded
        // (in_honolulu / inactive at the moment of carry) we still
        // need it represented in the zone's occupants so the silent-
        // init path puts the carried in the right lists.
        let primary_len = entries.len();
        for i in 0..primary_len {
            let (zone_idx, eidx, _) = entries[i];
            let Some(entity) = self.entities.get(eidx) else {
                continue;
            };
            let Some(carried_id) = entity.pc_data().and_then(|pc| pc.carried) else {
                continue;
            };
            if entries
                .iter()
                .any(|&(z, e, _)| z == zone_idx && e == carried_id)
            {
                continue;
            }
            let carried_h = crate::natives::GameHost::actor_handle(carried_id);
            entries.push((zone_idx, carried_id, carried_h));
        }
        entries
    }

    /// Silent occupant population: pushes each entry into its zone's
    /// occupant list and applies the production work-icon, **without**
    /// firing any zone `EnterZone` script.  Matches the bare
    /// `AddOccupant` list-push semantics that never trigger scripts.
    fn apply_zone_occupant_entries(
        &mut self,
        entries: &[(usize, crate::entity_id::EntityId, i32)],
    ) {
        for &(zone_idx, entity_idx, _) in entries {
            self.script_zone_data[zone_idx].enter(entity_idx);
            let pt = self.script_zone_data[zone_idx].production_sector_type;
            if pt != crate::sector_production::Type::Unknown {
                self.apply_production_work_icon(entity_idx, pt, true);
            }
        }
    }

    /// Bulk-clear occupant lists on every script zone.  Iterates
    /// script-sector objects and calls `RemoveAllOccupants` — no
    /// scripts fire.  Used by the post-mission Sherwood-entry refresh
    /// path, where occupant lists must be wiped before re-scanning
    /// against teleported positions.
    pub(crate) fn empty_all_script_sectors(&mut self) {
        for zone in &mut self.script_zone_data {
            zone.remove_all_occupants();
        }
    }

    /// Clear every zone's occupant list and silently re-scan actor
    /// positions to rebuild it.  No `EnterZone` scripts fire.  Used
    /// to reconcile zone membership after post-mission teleports.
    pub(crate) fn refresh_zone_occupants_silent(&mut self, assets: &LevelAssets) {
        self.empty_all_script_sectors();
        if assets.script_zone_grid_indices.is_empty() {
            return;
        }
        let entries = self.scan_zone_occupant_entries(assets);
        self.apply_zone_occupant_entries(&entries);
    }

    /// Populate initial zone occupants by checking all actor positions
    /// against zone polygons, and fire `EnterZone` for each.
    ///
    /// Called once after zone scripts and actor scripts are initialized.
    ///
    /// Divergence kept by design: `AddOccupant` is a pure list push —
    /// it does **not** fire `EnterZone`.  This function additionally
    /// dispatches `EnterZone` at init so zone scripts see their
    /// starting occupants; removing this would silently change the
    /// first-frame observable behaviour of every scripted level and
    /// can't be safely done without a full mission-script playthrough.
    /// The refresh path (`refresh_zone_occupants_silent`) uses the
    /// silent helpers and skips the dispatch.
    pub(crate) fn initialize_zone_occupants(&mut self, assets: &LevelAssets) {
        if assets.script_zone_grid_indices.is_empty() {
            return;
        }

        let entries = self.scan_zone_occupant_entries(assets);
        if entries.is_empty() {
            return;
        }

        self.apply_zone_occupant_entries(&entries);

        // Phase 3: Dispatch EnterZone to zone scripts.
        let script = match self.mission_script.as_mut() {
            Some(s) => s,
            None => return,
        };
        script.swap_engine_state(
            &mut self.entities,
            &mut self.ai_global,
            &mut self.fast_grid,
            &mut self.campaign,
            &mut self.mission_stat,
        );

        for &(zone_idx, _, handle) in &entries {
            if let Err(e) = script.call_zone_function(zone_idx, "EnterZone", &[handle]) {
                tracing::warn!("Zone {zone_idx} EnterZone (actor {handle}): {e}");
            }
        }

        script.swap_engine_state(
            &mut self.entities,
            &mut self.ai_global,
            &mut self.fast_grid,
            &mut self.campaign,
            &mut self.mission_stat,
        );
        self.sync_game_host_post_script(assets);

        tracing::info!(
            "Initialized {} zone occupant entries across {} zones",
            entries.len(),
            assets.script_zone_grid_indices.len()
        );
    }

    /// Per-frame zone occupant update: detect actors entering/leaving zones.
    ///
    /// For each actor that might have moved, checks against all script zone
    /// polygons. Fires `EnterZone(actor)` / `ExitZone(actor)` on the zone
    /// script when occupancy changes.
    ///
    /// Called once per frame from `perform_hourglass`, after movement tick.
    pub(crate) fn tick_zone_occupants(&mut self, assets: &LevelAssets) {
        if assets.script_zone_grid_indices.is_empty() || self.mission_script.is_none() {
            return;
        }

        // Phase 1: Collect enter/exit events by comparing current positions
        // with zone occupant lists.
        let mut enter_events: Vec<(usize, crate::entity_id::EntityId, i32)> = Vec::new();
        let mut exit_events: Vec<(usize, crate::entity_id::EntityId, i32)> = Vec::new();

        for (actor_id, entity) in self.entities.actors() {
            let eidx = actor_id.into();
            let ed = entity.element_data();
            let active = ed.active && !ed.in_honolulu;
            let pos = ed.position_map();
            let layer = ed.layer();
            let handle = crate::natives::GameHost::actor_handle(actor_id);

            for (zone_idx, &grid_idx) in assets.script_zone_grid_indices.iter().enumerate() {
                // Skip apex-converted zones — see scan_zone_occupant_entries note.
                if self.script_zone_data[zone_idx].transformed_to_apex {
                    continue;
                }
                let gs = &self.fast_grid.level.sectors[grid_idx as usize];
                let was_inside = self.script_zone_data[zone_idx].is_inside(eidx);
                let is_inside = active && gs.layer == layer && gs.contains_point(pos);

                if is_inside && !was_inside {
                    enter_events.push((zone_idx, eidx, handle));
                } else if !is_inside && was_inside {
                    exit_events.push((zone_idx, eidx, handle));
                }
            }
        }

        // Carried-recursion: a PC that enters or leaves a zone takes
        // its carried actor with it, regardless of whether the carried
        // element's own scan would catch the transition (it won't if
        // the carried is `in_honolulu` while held).  Synthesize the
        // missing event without double-firing for carried entries that
        // the scan already produced.
        let primary_enter_len = enter_events.len();
        for i in 0..primary_enter_len {
            let (zone_idx, eidx, _) = enter_events[i];
            let Some(entity) = self.entities.get(eidx) else {
                continue;
            };
            let Some(carried_id) = entity.pc_data().and_then(|pc| pc.carried) else {
                continue;
            };
            if self.script_zone_data[zone_idx].is_inside(carried_id) {
                continue;
            }
            if enter_events
                .iter()
                .any(|&(z, e, _)| z == zone_idx && e == carried_id)
            {
                continue;
            }
            let carried_h = crate::natives::GameHost::actor_handle(carried_id);
            enter_events.push((zone_idx, carried_id, carried_h));
        }
        let primary_exit_len = exit_events.len();
        for i in 0..primary_exit_len {
            let (zone_idx, eidx, _) = exit_events[i];
            let Some(entity) = self.entities.get(eidx) else {
                continue;
            };
            let Some(carried_id) = entity.pc_data().and_then(|pc| pc.carried) else {
                continue;
            };
            if !self.script_zone_data[zone_idx].is_inside(carried_id) {
                continue;
            }
            if exit_events
                .iter()
                .any(|&(z, e, _)| z == zone_idx && e == carried_id)
            {
                continue;
            }
            let carried_h = crate::natives::GameHost::actor_handle(carried_id);
            exit_events.push((zone_idx, carried_id, carried_h));
        }

        if enter_events.is_empty() && exit_events.is_empty() {
            return;
        }

        // Phase 2: Update occupant lists and apply production work icons.
        for &(zone_idx, entity_idx, _) in &enter_events {
            self.script_zone_data[zone_idx].enter(entity_idx);
            let pt = self.script_zone_data[zone_idx].production_sector_type;
            if pt != crate::sector_production::Type::Unknown {
                self.apply_production_work_icon(entity_idx, pt, true);
            }
        }
        for &(zone_idx, entity_idx, _) in &exit_events {
            self.script_zone_data[zone_idx].leave(entity_idx);
            let pt = self.script_zone_data[zone_idx].production_sector_type;
            if pt != crate::sector_production::Type::Unknown {
                self.apply_production_work_icon(entity_idx, pt, false);
            }
        }

        // Phase 3: Dispatch to zone scripts.
        self.refresh_game_host_entity_state();
        let script = self.mission_script.as_mut().unwrap();
        script.swap_engine_state(
            &mut self.entities,
            &mut self.ai_global,
            &mut self.fast_grid,
            &mut self.campaign,
            &mut self.mission_stat,
        );

        for &(zone_idx, _, handle) in &enter_events {
            if let Err(e) = script.call_zone_function(zone_idx, "EnterZone", &[handle]) {
                tracing::warn!("Zone {zone_idx} EnterZone (actor {handle}): {e}");
            }
        }
        for &(zone_idx, _, handle) in &exit_events {
            if let Err(e) = script.call_zone_function(zone_idx, "ExitZone", &[handle]) {
                tracing::warn!("Zone {zone_idx} ExitZone (actor {handle}): {e}");
            }
        }

        script.swap_engine_state(
            &mut self.entities,
            &mut self.ai_global,
            &mut self.fast_grid,
            &mut self.campaign,
            &mut self.mission_stat,
        );
        self.sync_game_host_post_script(assets);
    }

    // ─── Production-sector wiring ────────────────────────────────

    /// Drain `production_registrations` and `production_points` from the
    /// GameHost into engine state.  Sets the `production_sector_type` on each
    /// referenced script zone sector, and pushes a per-sector
    /// `sector_production::Point` into the matching campaign SectorProduction.
    ///
    /// `RegisterAsProductionSector` sets the sector's production type;
    /// `AddProductionPoint` pushes onto the per-type points list.
    pub(super) fn apply_production_registrations(&mut self, assets: &LevelAssets) {
        // Points come before sectors in the script-location payload
        // layout, so a sector's zone index is `location_index - points_count`.
        let points_count = assets
            .script_location_positions
            .len()
            .saturating_sub(self.script_zone_data.len());

        let Some(ref mut script) = self.mission_script else {
            return;
        };
        let game_host = match script.game_host_mut() {
            Some(h) => h,
            None => return,
        };

        let registrations: Vec<(i32, i32, i32)> =
            std::mem::take(&mut game_host.production_registrations);
        let points: Vec<(i32, i32)> = std::mem::take(&mut game_host.production_points);

        for (prod_type, loc_handle, speed) in registrations {
            let prod_type_enum = match crate::sector_production::Type::from_script_i32(prod_type) {
                Some(t) => t,
                None => {
                    tracing::warn!("RegisterAsProductionSector: bad type {prod_type} — ignored");
                    continue;
                }
            };
            let Some(loc_idx) = crate::natives::GameHost::location_index(loc_handle) else {
                tracing::warn!("RegisterAsProductionSector: invalid location handle {loc_handle}");
                continue;
            };
            if loc_idx < points_count {
                tracing::warn!(
                    "RegisterAsProductionSector: location {loc_handle} is not a script zone sector"
                );
                continue;
            }
            let zone_idx = loc_idx - points_count;
            if zone_idx >= self.script_zone_data.len() {
                tracing::warn!("RegisterAsProductionSector: zone {zone_idx} out of range");
                continue;
            }
            self.script_zone_data[zone_idx].production_sector_type = prod_type_enum;

            // Attach to the campaign's SectorProduction so its `speed` is set.
            if let Some(campaign) = self.campaign.as_mut()
                && (prod_type as usize) < campaign.production_sectors.len()
            {
                let prod = &mut campaign.production_sectors[prod_type as usize];
                prod.speed = speed.max(0) as u16;
                prod.prod_type = prod_type_enum;
            }
        }

        for (prod_type, loc_handle) in points {
            let prod_type_enum = match crate::sector_production::Type::from_script_i32(prod_type) {
                Some(t) => t,
                None => continue,
            };
            let Some(loc_idx) = crate::natives::GameHost::location_index(loc_handle) else {
                continue;
            };
            if loc_idx >= assets.script_location_positions.len() {
                continue;
            }
            let (x, y) = assets.script_location_positions[loc_idx];
            let layer = assets.script_location_layers[loc_idx];
            let sector = assets.script_location_sectors[loc_idx];
            // GetProjectionArea(point) → GetObstacleIndex.
            let obstacle = self
                .get_projection_area_index(
                    assets,
                    sector,
                    layer,
                    crate::coordinates::MapPoint::new(x, y),
                )
                .unwrap_or(0xFFFF);
            if let Some(campaign) = self.campaign.as_mut()
                && (prod_type as usize) < campaign.production_sectors.len()
            {
                let prod = &mut campaign.production_sectors[prod_type as usize];
                prod.prod_type = prod_type_enum;
                prod.production_points
                    .push(crate::sector_production::Point {
                        x,
                        y,
                        layer,
                        sector,
                        obstacle,
                    });
            }
        }
    }

    /// Set a PC's work icon when entering/leaving a script sector with a
    /// production type.
    pub(super) fn apply_production_work_icon(
        &mut self,
        entity_id: EntityId,
        production_type: crate::sector_production::Type,
        entering: bool,
    ) {
        use crate::element::WorkIcon;
        use crate::sector_production::Type as PT;

        let Some(entity) = self.entities.get_mut(entity_id) else {
            return;
        };
        let crate::engine::Entity::Pc(pc) = entity else {
            return;
        };

        if entering {
            // Map production type onto the WorkIcon enum. Relic / Unknown have
            // no icon (work icons cover types 0..11; Relic=12 falls through
            // at the call site).
            let icon = match production_type {
                PT::MakeArrow => WorkIcon::Arrows,
                PT::MakePurse => WorkIcon::Purses,
                PT::MakeStone => WorkIcon::Stones,
                PT::MakeApple => WorkIcon::Apples,
                PT::MakeAle => WorkIcon::Beer,
                PT::MakeLamblegg => WorkIcon::Legs,
                PT::MakePlant => WorkIcon::Plants,
                PT::MakeNet => WorkIcon::Nets,
                PT::MakeWaspNest => WorkIcon::Wasps,
                PT::TrainBow => WorkIcon::BowTraining,
                PT::TrainHandToHand => WorkIcon::SwordTraining,
                PT::Heal => WorkIcon::Regeneration,
                PT::Relic | PT::Unknown => return,
            };
            pc.pc.work_icon = icon;
        } else {
            pc.pc.work_icon = WorkIcon::None;
        }
    }

    // ─── Sequence SendMessage → ProcessMessage dispatch ──────────

    /// Extract message properties from a generic sequence element.
    /// Returns `(message, argument, extended_argument)`.
    pub(super) fn extract_message_properties(
        &self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> (i32, i32, i32) {
        use crate::sequence::{Field, FieldValue};
        let elem = match self.sequence_manager.get_element(seq_id, elem_idx) {
            Some(e) => e,
            None => return (0, 0, 0),
        };
        let msg = match elem.get_property(Field::Message) {
            Some(FieldValue::Integer(v)) => *v as i32,
            _ => 0,
        };
        let arg1 = match elem.get_property(Field::MessageArgument) {
            Some(FieldValue::Integer(v)) => *v as i32,
            _ => 0,
        };
        let arg2 = match elem.get_property(Field::MessageExtendedArgument) {
            Some(FieldValue::Integer(v)) => *v as i32,
            _ => 0,
        };
        (msg, arg1, arg2)
    }

    /// Dispatch deferred `ProcessMessage` calls from sequence SendMessage
    /// elements.
    ///
    /// Per-actor messages go to the actor's script `ProcessMessage(msg, arg1, arg2)`.
    /// EngineInner-level messages (ownerless) go to the global StartUp script's
    /// `ProcessMessage`.
    ///
    /// Routes through `IEngineScript::ProcessMessage` /
    /// `IActorScript::ProcessMessage`.
    pub(super) fn dispatch_sequence_messages(
        &mut self,
        assets: &LevelAssets,
        per_actor: &[(i32, i32, i32, i32)],
        engine_level: &[(i32, i32, i32)],
    ) {
        self.refresh_game_host_entity_state();
        if let Some(ref mut script) = self.mission_script {
            script.swap_engine_state(
                &mut self.entities,
                &mut self.ai_global,
                &mut self.fast_grid,
                &mut self.campaign,
                &mut self.mission_stat,
            );

            // Per-actor ProcessMessage
            for &(handle, msg, arg1, arg2) in per_actor {
                if let Err(e) =
                    script.call_actor_function(handle, "ProcessMessage", &[msg, arg1, arg2])
                {
                    tracing::warn!("Sequence ProcessMessage (actor {handle}, msg {msg}): {e}");
                }
            }

            // EngineInner-level ProcessMessage → global StartUp script
            for &(msg, arg1, arg2) in engine_level {
                if script
                    .instance
                    .has_function(&script.manager, "ProcessMessage")
                {
                    let result = MissionScript::with_game_host_attached(
                        &mut script.game_host,
                        &mut script.instance,
                        |instance, host| {
                            instance.push_param(msg);
                            instance.push_param(arg1);
                            instance.push_param(arg2);
                            instance.call_function_with_host(
                                &mut script.manager,
                                "ProcessMessage",
                                host,
                            )
                        },
                    );
                    match result {
                        Ok(_) => {}
                        Err(e) => tracing::warn!("EngineInner ProcessMessage(msg {msg}): {e}"),
                    }
                }
            }

            script.swap_engine_state(
                &mut self.entities,
                &mut self.ai_global,
                &mut self.fast_grid,
                &mut self.campaign,
                &mut self.mission_stat,
            );
        }
        self.sync_game_host_post_script(assets);
    }

    /// Dispatch deferred `IElementTargetScript::ActivatedBy*(pPC)` calls.
    ///
    /// Each entry is `(target_handle, pc_handle, method_name)`.  Sets
    /// `script_this` to the target then calls the relevant
    /// `IElementTargetScript::ActivatedBy*` method pointer on the
    /// target's own VM.  Missing methods on the bound class are silent
    /// no-ops, matching script-runtime behaviour for classes that
    /// don't override every callback.
    ///
    /// The original gates dispatch on a global script-enabled flag
    /// (`--NOSCRIPT` CLI option).  We don't plumb that flag through
    /// to the runtime (same situation as `ActivatedByListenable` in
    /// `engine/ai.rs`), so script dispatch is effectively always on.
    /// The "is class instantiated" check is implicit:
    /// `call_target_function` returns `Ok(0)` when no `ScriptInstance`
    /// is bound for the target.
    pub(super) fn dispatch_target_activations(
        &mut self,
        assets: &LevelAssets,
        calls: &[(i32, i32, &str)],
    ) {
        if calls.is_empty() {
            return;
        }
        self.refresh_game_host_entity_state();
        if let Some(ref mut script) = self.mission_script {
            script.swap_engine_state(
                &mut self.entities,
                &mut self.ai_global,
                &mut self.fast_grid,
                &mut self.campaign,
                &mut self.mission_stat,
            );
            for &(target_handle, pc_handle, fn_name) in calls {
                if let Err(e) = script.call_target_function(target_handle, fn_name, &[pc_handle]) {
                    tracing::warn!("{fn_name} (target {target_handle}): {e}");
                }
            }
            script.swap_engine_state(
                &mut self.entities,
                &mut self.ai_global,
                &mut self.fast_grid,
                &mut self.campaign,
                &mut self.mission_stat,
            );
        }
        self.sync_game_host_post_script(assets);
    }

    /// Send a one-shot engine-level `ProcessMessage` to the global
    /// StartUp script.
    ///
    /// Used e.g. by the Sherwood `GoToExit` button (msg=1000).  Thin
    /// wrapper over the existing `dispatch_sequence_messages`
    /// engine-level path.
    pub(crate) fn dispatch_startup_message(
        &mut self,
        assets: &LevelAssets,
        msg: i32,
        arg1: i32,
        arg2: i32,
    ) {
        self.dispatch_sequence_messages(assets, &[], &[(msg, arg1, arg2)]);
    }

    // ─── AI event filter precompute ─────────────────────────────

    /// Run the per-actor `FilterAIEvent` for a stimulus about to be
    /// dispatched to `handle` (opaque script actor handle).
    ///
    /// Returns `true` if `think()` should proceed, `false` if the
    /// script blocked the stimulus.  Implements the early-gate:
    ///
    /// ```text
    /// SetScriptThis(self);
    /// ok = (FilterAIEvent(stimulus_actor, event_code) != 0);
    /// SetScriptThis(prev);
    /// if (!ok) { register_log(LOG_EVENT_REFUSED, 0); return false; }
    /// ```
    ///
    /// Callers must invoke this *before* acquiring a `&mut` borrow on
    /// the target entity, since the script call needs `self.entities`
    /// via [`MissionScript::swap_engine_state`].  The function is a
    /// no-op (returns `true`) for:
    ///  - Actors with no script instance or no `FilterAIEvent`
    ///    override (the base-class `FilterAIEvent` returns 1 / allow).
    ///  - Script VM errors — logged and treated as allow so a
    ///    script bug never blocks AI progress.
    ///
    /// Source actor is extracted from `stimulus.info`: `Human(h)` becomes
    /// a script actor handle; other info variants become 0 (originally NULL).
    pub fn filter_stimulus(
        &mut self,
        assets: &LevelAssets,
        handle: i32,
        stimulus: &crate::ai::Stimulus,
    ) -> bool {
        // Original: RHArtificialIntelligence::StartThink assigns -2 in the
        // default switch arm and still calls FilterAIEvent for scripted NPCs.
        let code = crate::ai::stimulus_to_ai_event_code(stimulus.stimulus_type).unwrap_or(-2);

        let source = match stimulus.info {
            crate::ai::StimulusInfo::Human(h) => crate::natives::GameHost::actor_handle(
                crate::element::EntityId::Soldier(crate::entity_id::SoldierId(h)),
            ),
            _ => 0,
        };

        // Fast-paths that skip the swap+call machinery.
        let has_override = match self.mission_script.as_ref() {
            Some(s) => s.actor_has_function(handle, "FilterAIEvent"),
            None => return true,
        };
        if !has_override {
            return true;
        }

        // Full script call.  Matches the swap pattern used elsewhere
        // (e.g., `dispatch_ai_state_change_notifications`) — entities,
        // ai_global, and campaign flow through the GameHost so native
        // functions (`GetRobin`, `IsActorEqual`, `SetGlobal`, etc.)
        // read and write live engine state.
        self.refresh_game_host_entity_state();
        let script = self.mission_script.as_mut().expect("checked above");
        script.swap_engine_state(
            &mut self.entities,
            &mut self.ai_global,
            &mut self.fast_grid,
            &mut self.campaign,
            &mut self.mission_stat,
        );
        let result = script.call_actor_function(handle, "FilterAIEvent", &[source, code]);
        script.swap_engine_state(
            &mut self.entities,
            &mut self.ai_global,
            &mut self.fast_grid,
            &mut self.campaign,
            &mut self.mission_stat,
        );
        self.sync_game_host_post_script(assets);

        match result {
            Ok(v) => v != 0,
            Err(e) => {
                tracing::warn!(
                    "FilterAIEvent(handle={handle}, source={source}, code={code}) failed: {e} — allowing"
                );
                true
            }
        }
    }

    /// Run [`filter_stimulus`](Self::filter_stimulus) on `stimulus` for
    /// the AI on `entity_id`, and dispatch to `think()` if the filter
    /// allows it.  Returns `think()`'s handled-bool — returns `false`
    /// when the filter blocks.  Also returns `false` when the entity
    /// has no AI controller (nothing to think with).
    ///
    /// This is the canonical entry point for engine-layer stimulus
    /// dispatch — every external stimulus (detection pass, command
    /// completion, reach-point, etc.) should route through here so
    /// `FilterAIEvent` fires live with the actual source.
    ///
    /// Cascades — `self.think(&other_stimulus, ...)` calls inside
    /// `EnemyAi::think` / `FriendlyAi::think` — intentionally do *not*
    /// go through this path.  `think()` doesn't have engine access;
    /// routing cascades through a deferred queue would break the
    /// synchronous-within-tick semantics the script runtime relies on.
    /// Audit of the shipped `fullgame` `.scb` content confirmed no
    /// script filters any cascade-emitted stimulus, so the divergence
    /// is harmless for shipped content.  A warning is logged in
    /// `EnemyAi::think_*` cascades if this assumption ever breaks.
    pub(crate) fn dispatch_filtered_stimulus(
        &mut self,
        assets: &LevelAssets,
        entity_id: crate::element::EntityId,
        stimulus: &crate::ai::Stimulus,
        ctx: &crate::ai::AiContext,
        tick_data: &crate::ai::AiPerTickData,
    ) -> bool {
        let handle = crate::natives::GameHost::actor_handle(entity_id);
        if !self.filter_stimulus(assets, handle, stimulus) {
            return false;
        }
        // Hoist the door slice off `mission_script.game_host()`
        // before grabbing the mutable entity borrow — the friendly
        // AI's `alert_soldier` needs it for the
        // `ALERTFLAG_CHECK_DOOR_PATH` retry.
        let doors_ptr = self
            .mission_script
            .as_ref()
            .and_then(|ms| ms.game_host())
            .map(|gh| gh.doors.as_slice());
        let ai_global = &mut self.ai_global;
        let Some(entity) = self.entities.get_mut(entity_id) else {
            return false;
        };
        if let Some(enemy_ai) = entity.enemy_ai_mut() {
            enemy_ai.think(stimulus, ai_global, ctx, tick_data, Some(&self.fast_grid))
        } else if let Some(friendly_ai) = entity.friendly_ai_mut() {
            friendly_ai.think(
                stimulus,
                ai_global,
                ctx,
                tick_data,
                Some(&self.fast_grid),
                doors_ptr,
            )
        } else {
            false
        }
    }

    /// Dispatch `FilterAIEvent` state-change notifications for NPCs
    /// whose AI state changed this frame.
    ///
    /// Called after the AI tick. `SetState()` calls
    /// `FilterAIEvent(source, AI_STATE_CHANGE_TO_*)` for notification
    /// (return value ignored).
    ///
    /// Each `set_state` queues a tuple onto
    /// `AiBase::pending_state_change_notifications` synchronously.
    /// We drain those queues in slot order here so multiple
    /// transitions inside a single `think()` (e.g.
    /// `Default → Wondering → Attacking`) each fire their own
    /// notification — synchronous per-substate behaviour.
    pub(crate) fn dispatch_ai_state_change_notifications(&mut self, assets: &LevelAssets) {
        if self.mission_script.is_none() {
            return;
        }

        // Collect state changes: (npc_handle, source_handle, state_change_code).
        let mut notifications: Vec<(i32, i32, i32)> = Vec::new();
        for (id, entity) in self.entities.npcs_mut() {
            let Some(actor) = entity.actor_data() else {
                continue;
            };
            let is_scripted = !actor.script_class.is_empty();
            let Some(ai) = entity.ai_controller_mut() else {
                continue;
            };
            // Always drain — even unscripted actors should not
            // accumulate stale entries for the next tick.
            let drained = std::mem::take(&mut ai.pending_state_change_notifications);
            if !is_scripted {
                continue;
            }
            let handle = crate::natives::GameHost::actor_handle(id);
            for (state, source_kind) in drained {
                let code = state.state_change_event_code();
                let source = match source_kind {
                    AiStateChangeSource::SelfActor => handle,
                    AiStateChangeSource::Null => 0,
                    AiStateChangeSource::Human(h) => crate::natives::GameHost::actor_handle(
                        crate::element::EntityId::Soldier(crate::entity_id::SoldierId(h)),
                    ),
                };
                notifications.push((handle, source, code));
            }
        }

        if notifications.is_empty() {
            return;
        }

        self.refresh_game_host_entity_state();
        let script = self.mission_script.as_mut().unwrap();
        script.swap_engine_state(
            &mut self.entities,
            &mut self.ai_global,
            &mut self.fast_grid,
            &mut self.campaign,
            &mut self.mission_stat,
        );

        for (handle, source, code) in &notifications {
            // Return value ignored — notification only.
            let _ = script.call_actor_function(*handle, "FilterAIEvent", &[*source, *code]);
        }

        script.swap_engine_state(
            &mut self.entities,
            &mut self.ai_global,
            &mut self.fast_grid,
            &mut self.campaign,
            &mut self.mission_stat,
        );
        self.sync_game_host_post_script(assets);
    }

    // ─── Campaign integration ────────────────────────────────────

    /// Initialize the engine for the campaign's current mission.
    ///
    /// The campaign must already be stored in `self.campaign`.
    /// Pulls the mission name, proto-level filename, and mission type
    /// from the campaign state, then delegates to `initialize_from_mission`.
    ///
    /// Called from `Engine::new` when `EngineArgs::level` is set.
    pub(crate) fn initialize_from_campaign(
        &mut self,
        assets: &mut LevelAssets,
        pending: &mut PendingLevelData,
        loaded: crate::level_data::LoadedLevel,
        level_directory: &str,
        bg_pixel_dims: (f32, f32),
        progress: &mut dyn FnMut(f32),
    ) -> Result<(), EngineError> {
        let campaign = self
            .campaign
            .as_ref()
            .expect("initialize_from_campaign: campaign not set on engine");
        let idx = campaign
            .current_mission_idx
            .expect("initialize_from_campaign: no current mission set");
        let profile = campaign.missions[idx].profile(&assets.profile_manager);
        let mission_filename = profile.mission_filename.clone();
        let proto_level_filename = profile.proto_level_filename.clone();
        let location = profile.location;

        self.initialize_from_mission(
            assets,
            pending,
            &mission_filename,
            &proto_level_filename,
            loaded,
            level_directory,
            bg_pixel_dims,
            progress,
        )?;

        // Set mission-specific engine state from the profile
        self.weather.is_forest_level = location == MissionLocation::Sherwood;

        Ok(())
    }

    /// Sync the post-mission soldier counts into the campaign's running
    /// totals.  `LIVING_SOLDIERS_VALUE` and `DEAD_SOLDIERS_VALUE` are
    /// accumulated only at mission end.  Money and score are NOT
    /// synced here: they are credited continuously during gameplay
    /// through `EngineInner::add_campaign_value`'s side effects
    /// (the RANSOM/SCORE branches of `Campaign::add_value`), so
    /// re-adding them at mission end would double-count.
    pub fn sync_stats_to_campaign(&self, campaign: &mut Campaign) {
        campaign.add_value(
            CampaignValue::LivingSoldiers,
            self.mission_stat.living_soldier_count as i32,
        );
        campaign.add_value(
            CampaignValue::DeadSoldiers,
            self.mission_stat
                .total_soldier_count
                .saturating_sub(self.mission_stat.living_soldier_count) as i32,
        );
    }

    /// Get the current mission's static profile from the campaign.
    ///
    /// Returns `None` if no current mission is set in the campaign.
    pub fn current_mission_profile<'a>(
        &self,
        campaign: &'a Campaign,
        profiles: &'a crate::profiles::ProfileManager,
    ) -> Option<&'a MissionProfile> {
        campaign
            .current_mission_idx
            .and_then(|idx| campaign.missions.get(idx))
            .map(|m| m.profile(profiles))
    }

    /// Check whether this is a Sherwood (HQ) mission based on the campaign.
    pub fn is_sherwood_mission(
        &self,
        campaign: &Campaign,
        profiles: &crate::profiles::ProfileManager,
    ) -> bool {
        self.current_mission_profile(campaign, profiles)
            .is_some_and(|p| p.location == MissionLocation::Sherwood)
    }

    // ─── Script command processing ──────────────────────────────

    /// Resolve a script location handle to a map position.
    /// Script locations are points and sectors from the SCRIPT chunk,
    /// **not** entity handles. Handle 0 = null.
    fn resolve_location_position(
        assets: &LevelAssets,
        handle: i32,
    ) -> Option<crate::coordinates::MapPoint> {
        let idx = crate::natives::GameHost::location_index(handle)?;
        assets
            .script_location_positions
            .get(idx)
            .map(|&(x, y)| crate::coordinates::MapPoint::new(x, y))
    }

    /// Process all deferred commands from script native calls.
    /// Called after each script tick (Hourglass / CheckVictoryCondition).
    pub(crate) fn apply_host_commands(
        &mut self,
        assets: &LevelAssets,
        commands: Vec<crate::natives::EngineCommand>,
    ) {
        use crate::natives::EngineCommand;

        for cmd in commands {
            match cmd {
                EngineCommand::ScrollCameraTo {
                    location_handle,
                    speed,
                } => {
                    // Store the raw script point in `camera_wanted` so
                    // resize/zoom can re-derive the slide target later,
                    // and the centered+clamped result in `camera_slide`.
                    if let Some(pos) = Self::resolve_location_position(assets, location_handle) {
                        self.cutscene_camera.camera_wanted = pos;
                        self.cutscene_camera.camera_slide =
                            self.check_location_is_valid_for_camera(pos);
                        self.speed = speed;
                    } else {
                        tracing::warn!(
                            "ScrollCameraTo: could not resolve location handle {location_handle}"
                        );
                    }
                }
                EngineCommand::JumpCameraTo { location_handle } => {
                    // Snap the view to the script point and invalidate
                    // background validity so the next frame redraws.
                    if let Some(pos) = Self::resolve_location_position(assets, location_handle) {
                        self.cutscene_camera.view_position =
                            self.check_location_is_valid_for_camera(pos);
                        self.pending_side_effects.invalidate_background = true;
                    } else {
                        tracing::warn!(
                            "JumpCameraTo: could not resolve location handle {location_handle}"
                        );
                    }
                }
                EngineCommand::SetZoomLevel { zoom } => {
                    // `SetZoomLevel` only assigns the desired zoom; the
                    // `mechanized_zoom` flag flips later when the
                    // zoom-update loop notices `desired != current`.
                    // Guard the flag so a no-op `SetZoomLevel` at the
                    // current zoom doesn't prematurely flip it.
                    self.cutscene_camera.desired_zoom_factor = zoom;
                    if zoom != self.cutscene_camera.zoom_factor {
                        self.cutscene_camera.mechanized_zoom = true;
                    }
                }
                EngineCommand::StartDialog { dialog_id } => {
                    tracing::debug!("StartDialog({dialog_id}): queued for game session");
                    self.pending_side_effects.pending_dialogues.push(dialog_id);
                    self.messenger
                        .send(Message::new(MessageType::Simple(SimpleMessage::ResetInput)));
                }
                EngineCommand::DisplayMap { show } => {
                    self.pending_side_effects
                        .pending_minimap_display_maps
                        .push((show, false));
                }
                EngineCommand::DisplayConsole => {
                    tracing::debug!("DisplayConsole: queued for UI system");
                    self.pending_side_effects.pending_show_console = true;
                    self.messenger.send(Message::new(MessageType::Simple(
                        SimpleMessage::DisplayConsole,
                    )));
                }
                EngineCommand::CustomizeMinimapDisplay {
                    actor_handle,
                    dot_type,
                } => {
                    // Validate the dot code against the known
                    // CUSTOM_DOT_* whitelist, gate the `_MULTI` variants
                    // on `is_human()` (codes 111/222/333), and overwrite
                    // the PC / Villain / Civilian outline colour slots
                    // for the codes that select a class.
                    use crate::element_kinds::OutlineColorName;
                    use crate::element_kinds::outline_colors;
                    use crate::minimap::CustomDot;
                    let Some(id) = self.entity_id_for_actor_handle(actor_handle) else {
                        tracing::warn!(
                            "CustomizeMinimapDisplay: invalid actor handle {actor_handle}"
                        );
                        continue;
                    };
                    let Some(entity) = self.get_entity_mut(id) else {
                        tracing::warn!(
                            "CustomizeMinimapDisplay: invalid actor handle {actor_handle}"
                        );
                        continue;
                    };
                    // Match a fixed whitelist of CUSTOM_DOT_* values.
                    // Any other code → log + skip both the dot update
                    // and the outline-colour write.
                    let dot_val = dot_type as u16;
                    let dot = match dot_val {
                        0 => Some(CustomDot::Invisible),
                        1 => Some(CustomDot::NotCustomized),
                        100 => Some(CustomDot::Pc),
                        101 => Some(CustomDot::PcLying),
                        102 => Some(CustomDot::PcDead),
                        111 => Some(CustomDot::PcMulti),
                        200 => Some(CustomDot::Villain),
                        201 => Some(CustomDot::VillainLying),
                        202 => Some(CustomDot::VillainDead),
                        222 => Some(CustomDot::VillainMulti),
                        300 => Some(CustomDot::Civilian),
                        301 => Some(CustomDot::CivilianLying),
                        302 => Some(CustomDot::CivilianDead),
                        333 => Some(CustomDot::CivilianMulti),
                        666 => Some(CustomDot::Animal),
                        500 => Some(CustomDot::Item),
                        _ => None,
                    };
                    let Some(dot) = dot else {
                        tracing::warn!(
                            "Script Error: Trying to customize minimap display with illegal dot ID ({:#x}).",
                            dot_val
                        );
                        continue;
                    };
                    // `_MULTI` codes require an is_human() target;
                    // log + early return otherwise.
                    let is_multi = matches!(
                        dot,
                        CustomDot::PcMulti | CustomDot::VillainMulti | CustomDot::CivilianMulti
                    );
                    if is_multi && !entity.is_human() {
                        tracing::warn!(
                            "Script Error: Minimap display codes 111, 222, 333 only valid for humans."
                        );
                        continue;
                    }
                    entity.element_data_mut().custom_minimap_dot = dot_val;
                    // Second switch — overwrite outline colour slots
                    // for PC / Villain / Civilian variants.  The
                    // `_DEAD` / `_LYING` / `_MULTI` variants also fall
                    // into these palette groups.
                    let palette = match dot {
                        CustomDot::Pc
                        | CustomDot::PcLying
                        | CustomDot::PcDead
                        | CustomDot::PcMulti => Some((
                            outline_colors::pc_default(),
                            outline_colors::pc_hidden(),
                            outline_colors::pc_target(),
                        )),
                        CustomDot::Villain
                        | CustomDot::VillainLying
                        | CustomDot::VillainDead
                        | CustomDot::VillainMulti => Some((
                            outline_colors::npc_evil_default(),
                            outline_colors::npc_evil_hidden(),
                            outline_colors::npc_evil_target(),
                        )),
                        CustomDot::Civilian
                        | CustomDot::CivilianLying
                        | CustomDot::CivilianDead
                        | CustomDot::CivilianMulti => Some((
                            outline_colors::npc_good_default(),
                            outline_colors::npc_good_hidden(),
                            outline_colors::npc_good_target(),
                        )),
                        _ => None,
                    };
                    if let Some((default, hidden, target)) = palette {
                        let colors = &mut entity.element_data_mut().outline_colors;
                        colors[OutlineColorName::Default as usize] = default;
                        colors[OutlineColorName::Hidden as usize] = hidden;
                        colors[OutlineColorName::Target as usize] = target;
                    }
                }
                EngineCommand::DefineFlatTrajectoryZone {
                    location_handle,
                    apex_height,
                } => {
                    // Resolve the location handle to the matching script
                    // zone index and transform its script sector into
                    // an apex sector.
                    //
                    // Script-location payload indices are laid out as
                    // `[script_points..., script_sectors...]`; the sector
                    // slice starts at `script_location_count - script_zone_data.len()`.
                    let points_count = assets
                        .script_location_positions
                        .len()
                        .saturating_sub(self.script_zone_data.len());
                    let Some(loc_idx) = crate::natives::GameHost::location_index(location_handle)
                    else {
                        tracing::warn!(
                            "DefineFlatTrajectoryZone(loc={location_handle}): invalid location handle"
                        );
                        continue;
                    };
                    if loc_idx < points_count || loc_idx >= assets.script_location_positions.len() {
                        tracing::warn!(
                            "DefineFlatTrajectoryZone(loc={location_handle}): handle is not a script zone sector"
                        );
                        continue;
                    } else {
                        let zone_idx = loc_idx - points_count;
                        if let Some(zone) = self.script_zone_data.get_mut(zone_idx) {
                            if zone.script_associated {
                                tracing::warn!(
                                    "DefineFlatTrajectoryZone(loc={location_handle}): \
                                     cannot convert script-associated sector to apex"
                                );
                            } else {
                                zone.transform_into_apex(apex_height as f32);
                                // Flip the APEX flag on the corresponding
                                // grid sector so `is_apex()` queries see it.
                                // The flag lives on the runtime overlay (not
                                // the static sector_type) so the geometry
                                // arena stays purely level-loaded.
                                if let Some(&grid_idx) =
                                    assets.script_zone_grid_indices.get(zone_idx)
                                {
                                    self.fast_grid.or_sector_type_overlay(
                                        grid_idx,
                                        crate::sector::SectorType::APEX,
                                    );
                                }
                            }
                        } else {
                            tracing::warn!(
                                "DefineFlatTrajectoryZone(loc={location_handle}): zone {zone_idx} out of range"
                            );
                        }
                    }
                }
                EngineCommand::AddShortBriefing { id, primary } => {
                    self.short_briefings.add(id as u32, primary);
                }
                EngineCommand::DoneShortBriefing { id } => {
                    self.short_briefings.mark_done(id as u32);
                }
                EngineCommand::ChooseVictoryDefeatText { id } => {
                    self.mission.victory_defeat_id = id as u32;
                }
                EngineCommand::DisplayPopupText { text_id } => {
                    tracing::debug!("DisplayPopupText({text_id}): queued for UI system");
                    self.pending_side_effects.pending_popup_texts.push(text_id);
                    self.messenger
                        .send(Message::new(MessageType::Simple(SimpleMessage::ResetInput)));
                }
                EngineCommand::DisplaySherwoodReport => {
                    tracing::debug!("DisplaySherwoodReport: queued for UI system");
                    self.pending_side_effects.pending_sherwood_report = true;
                    self.messenger
                        .send(Message::new(MessageType::Simple(SimpleMessage::ResetInput)));
                }
                EngineCommand::FadeToBlack { speed } => {
                    // The original `FadeToBlack` runs `2 * speed`
                    // iterations of a per-pixel-scale ramp, each
                    // followed by a present.  No engine update happens
                    // between iterations, so the game is genuinely
                    // frozen for the duration of the fade.  We split
                    // that into:
                    //   - `pending_side_effects.fade_to_black`: per-pixel
                    //     ramp drained by the host renderer (alpha-blend
                    //     overlay matching `current_alpha`).
                    //   - `fade_freeze_frames_remaining`: presentation
                    //     countdown read before the hourglass wrapper
                    //     touches any game clock or timer. The trigger
                    //     tick presents frame one, leaving `2*speed - 1`
                    //     frozen presentation frames. This is the only
                    //     blocking native in the entire script API
                    //     (verified across all shipped `.scb` files;
                    //     called once total, in `H04_Lei_VL`
                    //     `ProcessMessage(11)`), so a per-engine freeze
                    //     countdown beats generic VM yield/resume infra.
                    let s = speed.max(0) as u32;
                    let total_frames = s.saturating_mul(2);
                    self.pending_side_effects.fade_to_black = Some(if s == 0 {
                        None
                    } else {
                        Some(crate::engine::types::FadeToBlack {
                            speed: s,
                            frames_remaining: total_frames,
                        })
                    });
                    self.fade_freeze_frames_remaining = total_frames.saturating_sub(1);
                }
                EngineCommand::SetOutlineDisplay { display: show } => {
                    // Forward `MSG_SWITCH_MASKED_DISPLAY` when the
                    // state actually changes.  The rendering side
                    // (`game_render.rs:814` et al.) already reads
                    // `host.input.draw_hidden` to switch entities into
                    // the masked/outline draw mode.
                    self.pending_side_effects.set_draw_hidden = Some(show);
                }
                EngineCommand::SetViewRadius { radius } => {
                    self.standard_view_polygon_radius = radius as u16;
                    self.propagate_view_radius();
                }
                EngineCommand::PlayJingle(jingle) => {
                    self.pending_side_effects
                        .sounds
                        .push(super::SoundCommand::Jingle(jingle));
                }
                EngineCommand::SetActorLocation {
                    actor_handle,
                    x,
                    y,
                    dest_layer_sector,
                    spawn_elevation_probe,
                } => {
                    // SetPositionMap → SetLayer/SetSector →
                    // SetObstacle(GetProjectionArea) → ComputePositionAll.
                    // The native already wrote `position_map` and
                    // (for static script destinations) `layer` /
                    // `sector`; here we refresh the position interface,
                    // the grid cell, and — when a new floor landed the
                    // actor on a different projection-area obstacle —
                    // re-bind obstacle/material too.
                    let Some(id) = self.entity_id_for_actor_handle(actor_handle) else {
                        tracing::warn!("SetActorLocation: invalid actor handle {actor_handle}");
                        continue;
                    };
                    let Some(entity) = self.entities.get_mut(id) else {
                        tracing::warn!("SetActorLocation: actor {actor_handle} missing entity");
                        continue;
                    };
                    let pt = crate::coordinates::MapPoint { x, y };
                    if entity.actor_data().is_none() {
                        // Non-actor entities don't need the full actor
                        // reproject dance; refresh the basic grid.
                        entity.element_data_mut().set_position_map(pt);
                        entity.element_data_mut().update_grid_cell();
                        continue;
                    }
                    let pi = entity.position_iface_mut();
                    pi.set_map_position(pt);
                    let ed = entity.element_data_mut();
                    ed.set_position_map(pt);
                    ed.update_grid_cell();

                    // Motion-area validation: check the destination
                    // sector after the position/layer/sector writes
                    // but before obstacle refresh / display-order /
                    // spawn-elevation — on failure log
                    // `VERBOTEN SCRIPT : Character not lying on motion
                    // area (%f,%f) !` and return, leaving the partial
                    // state writes in place.  Required ordering: if
                    // the destination sector isn't a motion area,
                    // skip the rest.
                    if let Some((_layer, sector_num)) = dest_layer_sector {
                        let sector_handle = crate::sector::SectorNumber::new(sector_num as i16);
                        let valid = self
                            .grid_sector_by_number(sector_handle)
                            .map(|gs| gs.sector_type.is_motion() && gs.sector_type.is_area())
                            .unwrap_or(false);
                        if !valid {
                            tracing::warn!(
                                "VERBOTEN SCRIPT : Character not lying on motion area ({}, {}) !",
                                pt.x,
                                pt.y,
                            );
                            continue;
                        }
                    }

                    // ComputeDisplayOrder(NULL, true) — passing a null
                    // reference element zeroes any stale
                    // `display_order_ref` so a teleported actor that
                    // had been carried/attached doesn't keep its prior
                    // z-sort anchor.
                    let Some(entity) = self.entities.get_mut(id) else {
                        continue;
                    };
                    let sprite = entity.sprite_mut();
                    sprite.display_order_ref = None;
                    sprite.behind_display_order_ref = false;

                    // Projection-area refresh: if the native told us the
                    // destination's layer/sector, look up the new
                    // projection area and stamp its obstacle + material
                    // on the actor.  Computed (non-static) locations
                    // don't carry layer/sector so the refresh is
                    // skipped — the obstacle only gets rebound when
                    // the destination was a real script point or
                    // script sector.
                    if let Some((layer, sector_num)) = dest_layer_sector {
                        let new_obstacle =
                            self.get_projection_area_index(assets, sector_num, layer, pt);
                        let new_material = new_obstacle.and_then(|oi| {
                            self.sight_obstacles(assets).get(oi as usize).map(|obs| {
                                crate::element::GameMaterial::from_u32(obs.material as u32)
                            })
                        });
                        let new_obstacle_handle =
                            new_obstacle.and_then(crate::position_interface::ObstacleHandle::new);
                        let plane = crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                            new_obstacle_handle,
                            assets.static_sight_obstacles.as_slice(),
                        );
                        if let Some(entity) = self.entities.get_mut(id) {
                            let ed = entity.element_data_mut();
                            ed.set_obstacle_index(new_obstacle_handle, plane);
                            if let Some(mat) = new_material {
                                ed.set_material(mat);
                            }
                        }
                    }

                    // Spawn-elevation compose (RecordEnterGame path):
                    //     elevation = position_to_point_3d(destination).z;
                    //     origin.y = outside.y + elevation;
                    //     origin.z = elevation;
                    //     set_position(origin);
                    // When `spawn_elevation_probe` is `Some((dx, dy))` we
                    // evaluate the destination sector's top plane at the
                    // *inside* probe point and overwrite the actor's 3D
                    // position so the outside-of-map spawn sits at the
                    // same altitude as where it's about to walk to.  The
                    // earlier `set_position_map` call derived Z from the
                    // actor's stale cached plane — acceptable for
                    // ordinary SetActorLocation but wrong for an
                    // outside-of-map enter-game spawn.
                    if let (Some((layer, sector_num)), Some((probe_x, probe_y))) =
                        (dest_layer_sector, spawn_elevation_probe)
                    {
                        let handle = crate::position_interface::SectorHandle::new(sector_num);
                        let elev = self
                            .position_to_point_3d(assets, handle, layer, probe_x, probe_y)
                            .z;
                        if let Some(entity) = self.entities.get_mut(id) {
                            // `set_position` writes the 3D point and
                            // calls `recompute_from_3d`, which rederives
                            // `position_map` / sprite / move_box from
                            // the new `(x, y + elev, elev)` — preserving
                            // the iso invariant `map.y = position.y -
                            // position.z`.  The earlier
                            // `set_position_map(x, y)` above routed
                            // through the actor's stale cached plane at
                            // a 2D point that's outside the map; this
                            // pass corrects both Z and map-Y from the
                            // destination's projection-area top plane.
                            let pi = entity.position_iface_mut();
                            pi.set_position(crate::coordinates::WorldPoint3D {
                                x,
                                y: y + elev,
                                z: elev,
                            });
                            entity.element_data_mut().update_grid_cell();
                        }
                    }
                }
                EngineCommand::Win { show_window } => {
                    self.win(show_window);
                }
                EngineCommand::SetScrollStatus {
                    scroll_handle,
                    status,
                } => {
                    // Set scroll status: write status, run minimap-dot
                    // update, force animation `BonusThree` when entering
                    // Opened.  The native pre-validates handle/type/
                    // range, so the script handle is an actor handle
                    // for a scroll entity and `status` is in 0..=3.
                    let Some(eid) = self.entity_id_for_actor_handle(scroll_handle) else {
                        continue;
                    };
                    let st = ScrollStatus::from_i32(status);
                    self.set_scroll_status(eid, st);
                    if matches!(st, crate::engine::scroll_reveal::ScrollStatus::Opened)
                        && let Some(entity) = self.get_entity_mut(eid)
                        && let Some(obj) = entity.object_data_mut()
                    {
                        obj.animation = crate::order::OrderType::BonusThree;
                    }
                }
                EngineCommand::ScriptMakePCCrouched { actor_handle } => {
                    // Validate the handle is a PC, then delegate to
                    // `actor_make_crouched`, which either rewrites an
                    // in-flight movement sequence to its crouched
                    // variant or launches a brand-new
                    // `Command::CrouchDown` so the actor plays the
                    // crouch-down animation.
                    let Some(eid) = self.entity_id_for_actor_handle(actor_handle) else {
                        tracing::error!(
                            "Script Error: The Actor in MakePCCrouched is invalid (handle {actor_handle})"
                        );
                        continue;
                    };
                    if !matches!(self.get_entity(eid), Some(crate::element::Entity::Pc(_))) {
                        tracing::error!(
                            "Script Error: The Actor in MakePCCrouched is invalid (handle {actor_handle})"
                        );
                        continue;
                    }
                    self.actor_make_crouched(eid);
                }
                EngineCommand::MarkPc { actor_handle } => {
                    // Resolve the script handle to an EntityId and route
                    // it to the host via pending_side_effects.  The sim
                    // can't draw, so it hands the ID off to the host's
                    // outline pass, which flashes the outline for one
                    // frame.
                    if let Some(eid) = self.entity_id_for_actor_handle(actor_handle) {
                        if matches!(self.get_entity(eid), Some(crate::element::Entity::Pc(_))) {
                            self.pending_side_effects.pending_mark_pc_ids.push(eid);
                        } else {
                            tracing::warn!(
                                "MarkPc: handle {actor_handle} does not resolve to a PC"
                            );
                        }
                    }
                }
                EngineCommand::UpdateInformationBars => {
                    // The original `UpdateInformationBars` does two
                    // things:
                    //   (a) tears down and rebuilds the blazon bar
                    //       vs. the mission-requirements widget based
                    //       on `ProduceBlazons()` and the next-mission
                    //       profile type.
                    //   (b) calls `UpdateBlazonStatus()` on the blazon
                    //       bar so its counter matches the current
                    //       human-status / mission-stat values.
                    //
                    // Our HUD (see `game_render.rs`, `hud_text.rs`,
                    // `ui_panel.rs`) is immediate-mode: every frame
                    // re-reads mission + campaign + money state
                    // directly from the engine, campaign, and
                    // mission-stat it already has in scope.  There are
                    // no cached widget instances to recreate, and
                    // money / blazon counters do not cache their
                    // displayed value.  Therefore (b) is a no-op —
                    // the next frame will render the updated counters
                    // automatically.
                    //
                    // For (a), the blazon-bar and mission-requirements
                    // widgets are data-computation modules (see
                    // `widget/blazon_bar.rs`, `widget/requirements.rs`)
                    // that the immediate-mode HUD reads per-frame.
                    // Nothing to cache on the engine side: derive the
                    // states here so the log/trace reflects what the
                    // next HUD frame will show.
                    if let Some(campaign) = self.campaign.as_ref() {
                        // `Game::is_men_to_blazon_conversion` is mirrored
                        // onto `GameHost::men_to_blazon_conversion_mode`
                        // (the `SetMenToBlazonConversionMode` setter
                        // writes both; see `Game::set_men_to_blazon_conversion`).
                        // Read the host copy here so the blazon bar can
                        // switch to next-mission targeting during
                        // conversion mode without needing a `&Game`
                        // borrow at the engine tick.
                        let (men_to_blazon, blinking) = self
                            .mission_script
                            .as_ref()
                            .and_then(|s| s.game_host())
                            .map(|h| (h.men_to_blazon_conversion_mode, h.active_blinking_blazons()))
                            .unwrap_or((false, 0));
                        let bb = crate::widget_state::blazon_bar::build_blazon_bar_state(
                            campaign,
                            &assets.profile_manager,
                            men_to_blazon,
                            blinking,
                        );
                        let mission_team: Vec<crate::profiles::CharacterProfileIdx> =
                            campaign.mission_team_profile_indices();
                        let selected: Vec<crate::profiles::CharacterProfileIdx> = self.seats[0]
                            .selection
                            .iter()
                            .filter_map(|&id| match self.get_entity(id)? {
                                crate::element::Entity::Pc(pc) => Some(pc.pc.profile_index),
                                _ => None,
                            })
                            .collect();
                        let req = campaign.next_mission_idx.and_then(|idx| {
                            crate::widget_state::requirements::build_requirements_state(
                                campaign,
                                &assets.profile_manager,
                                idx,
                                &mission_team,
                                &selected,
                            )
                        });
                        tracing::debug!(
                            ?bb,
                            req_slots = req.as_ref().map(|r| r.slots.len()),
                            "UpdateInformationBars: recomputed HUD states"
                        );
                    } else {
                        tracing::debug!("UpdateInformationBars: no campaign — HUD states skipped");
                    }
                }
                EngineCommand::HeroSpeak { pc_id, expression } => {
                    self.hero_speaking(assets, pc_id, expression);
                }
                EngineCommand::ActivateDoorMouseSector {
                    door_handle,
                    active,
                } => {
                    // Flip the door's clickable polygon sector active
                    // state for hit-testing.  The register pass at
                    // `level_loading.rs:L4980` stores the door's
                    // `door_index` on each grid sector; we scan for a
                    // matching sector and drive `set_sector_active`.
                    let Some(door_idx) = crate::natives::GameHost::door_index(door_handle) else {
                        tracing::warn!(
                            "ActivateDoorMouseSector: invalid door handle {door_handle}"
                        );
                        continue;
                    };
                    let target = door_idx as u32;
                    let sector_idx = self
                        .fast_grid
                        .level
                        .sectors
                        .iter()
                        .position(|s| s.door_index == Some(target))
                        .map(|i| i as u32);
                    if let Some(idx) = sector_idx {
                        self.fast_grid.set_sector_active(idx, active);
                    } else {
                        tracing::warn!(
                            "ActivateDoorMouseSector: no grid sector registered for door {door_handle}"
                        );
                    }
                }
                EngineCommand::MakeNoise {
                    noise_type,
                    x,
                    y,
                    layer,
                } => {
                    // Delegate to the shared broadcast path so scripted
                    // noises get the same AI dispatch and debug overlay
                    // as gameplay-triggered broadcasts.
                    use crate::parameters_ai;
                    let volume = match noise_type {
                        crate::ai::NoiseType::Logs => parameters_ai::NOISE_VOLUME_LOGS,
                        crate::ai::NoiseType::Drawbridge => parameters_ai::NOISE_VOLUME_DRAWBRIDGE,
                        // Unexpected — the native arm already rejects
                        // anything other than LOGS/DRAWBRIDGE.  Keep a
                        // sensible floor so a future arm extension
                        // doesn't silently broadcast zero-volume noise.
                        _ => parameters_ai::NOISE_VOLUME_PLOUF,
                    } as u16;
                    // Scripted noises (LOGS / DRAWBRIDGE) don't carry an
                    // elevation through the EngineCommand — these
                    // always broadcast at elevation 0.
                    self.broadcast_noise(
                        noise_type,
                        crate::coordinates::MapPoint::new(x, y),
                        layer,
                        volume,
                        0,
                        None,
                    );
                }
            }
        }
    }

    /// Apply the positioning side of `PutActorInBuilding`:
    /// SetActive(false) (we use `hidden_in_building`), move to the
    /// building's special layer + sector, teleport onto the first gate's
    /// `point_in`, and DisableAllActionsTemp for PCs.
    fn put_actor_in_building(&mut self, actor: i32, building: i32) {
        let Some(actor_id) = self.entity_id_for_actor_handle(actor) else {
            tracing::warn!("PutActorInBuilding: invalid actor handle {actor}");
            return;
        };
        let Some(bld_idx) = crate::natives::GameHost::building_index(building) else {
            tracing::warn!("PutActorInBuilding: invalid building handle {building}");
            return;
        };

        // Look up the first gate's `point_in` and the building's sector
        // number. Sector number comes from the grid sector tagged
        // `building_index == bld_idx` (populated at level load).
        let (gate_point_in, sector_num) = {
            let Some(ref script) = self.mission_script else {
                return;
            };
            let Some(game_host) = script.game_host() else {
                return;
            };
            let gate_handle = game_host
                .building_gates
                .get(bld_idx)
                .and_then(|g| g.first())
                .copied();
            let point_in = gate_handle
                .and_then(crate::natives::GameHost::door_index)
                .and_then(|di| game_host.doors.get(di))
                .map(|d| d.point_in);
            let sn = self.fast_grid.level.sectors.iter().find_map(|gs| {
                if gs.building_index == crate::sector::BuildingIdx::new(bld_idx as u16) {
                    Some(gs.sector_number)
                } else {
                    None
                }
            });
            (point_in, sn)
        };

        let Some(point_in) = gate_point_in else {
            tracing::warn!(
                "PutActorInBuilding: building {building} has no gates — cannot position actor"
            );
            return;
        };
        let Some(sector_num) = sector_num else {
            tracing::warn!(
                "PutActorInBuilding: building {building} has no grid sector — cannot position actor"
            );
            return;
        };

        let special_layer = self.fast_grid.level.special_layer;

        let is_pc;
        let carried_handle: Option<i32>;
        if let Some(entity) = self.entities.get_mut(actor_id) {
            let elem = entity.element_data_mut();
            elem.hidden_in_building = true;
            elem.set_layer(special_layer);
            elem.set_sector(crate::position_interface::SectorHandle::new(u16::from(
                sector_num,
            )));
            elem.set_position_map(point_in);
            elem.update_grid_cell();
            // After `SetPositionMap` on the gate's point-in, re-derive
            // the sprite-space and 3D positions from the new map
            // position so the renderer / display-order pipeline picks
            // up the teleport on the first post-script frame instead
            // of mis-framing.
            if entity.actor_data().is_some() {
                let pi = entity.position_iface_mut();
                pi.set_map_position(point_in);
            }
            is_pc = entity.pc_data().is_some();
            carried_handle = entity
                .pc_data()
                .and_then(|pc| pc.carried)
                .map(crate::natives::GameHost::actor_handle);
            if is_pc && let Some(pc) = entity.pc_data_mut() {
                // DisableAllActionsTemp gates the
                // disabled_actions_temp loop on `playable` so a
                // non-playable PC kept inside the building doesn't
                // accumulate stale temp-disable flags.
                pc.disable_all_actions_temp();
            }
        } else {
            tracing::warn!("PutActorInBuilding: entity {actor:?} missing");
            return;
        }

        if is_pc {
            // Forward MSG_DISABLE_ALL_ACTIONS — counterpart to
            // DisableAllActionsTemp.
            self.messenger.send(Message::pc(
                crate::messenger::PcMessage::DisableAllActionsTemp,
                None,
            ));

            // When the entering actor is a PC,
            // (a) recursively enter its carried actor, and
            // (b) re-enable existing occupants who are dead/unconscious
            //     and not being carried — their corpses should render
            //     inside the building.
            if let Some(carried_h) = carried_handle
                && carried_h != 0
            {
                if let Some(carried_id) = self.entity_id_for_actor_handle(carried_h)
                    && let Some(carried_entity) = self.entities.get_mut(carried_id)
                {
                    let elem = carried_entity.element_data_mut();
                    elem.hidden_in_building = true;
                    elem.set_layer(special_layer);
                    elem.set_sector(crate::position_interface::SectorHandle::new(u16::from(
                        sector_num,
                    )));
                    elem.set_position_map(point_in);
                    elem.update_grid_cell();
                    if carried_entity.actor_data().is_some() {
                        let pi = carried_entity.position_iface_mut();
                        pi.set_map_position(point_in);
                    }
                }
                // Push the carried into the occupants list.
                if let Some(ref mut script) = self.mission_script
                    && let Some(gh) = script.game_host_mut()
                {
                    if bld_idx >= gh.building_occupants.len() {
                        gh.building_occupants.resize(bld_idx + 1, Vec::new());
                    }
                    gh.building_occupants[bld_idx].push(carried_h);
                    gh.actor_building.insert(carried_h, building);
                }
            }

            // Re-enable corpses already inside the building: walk the
            // occupants list and SetActive(true) on humans that are
            // (is_dead || unconscious) && carrier.is_none().
            let occupants: Vec<i32> = self
                .mission_script
                .as_ref()
                .and_then(|s| s.game_host())
                .and_then(|gh| gh.building_occupants.get(bld_idx))
                .cloned()
                .unwrap_or_default();
            for occ_h in occupants {
                let Some(occ_id) = self.entity_id_for_actor_handle(occ_h) else {
                    continue;
                };
                let Some(occ) = self.entities.get_mut(occ_id) else {
                    continue;
                };
                let Some(hd) = occ.human_data() else { continue };
                let is_dead_or_ko = occ.is_dead() || hd.unconscious;
                let has_carrier = hd.carrier.is_some();
                if is_dead_or_ko && !has_carrier {
                    occ.element_data_mut().hidden_in_building = false;
                }
            }
        }

        tracing::debug!(
            "PutActorInBuilding: actor={actor} building={building} \
             → layer={special_layer}, sector={sector_num}, pos=({:.1},{:.1})",
            point_in.x,
            point_in.y,
        );
    }
}

#[cfg(test)]
mod script_context_tests {
    use super::*;
    use crate::scb::{ClassEntry, SCB_VERSION, ScbFile};

    fn empty_mission_script() -> MissionScript {
        let startup = ClassEntry {
            source_file: "script_context_test.scs".into(),
            class_name: "StartUp".into(),
            size_of_member_variables: 0,
            member_variables: Vec::new(),
            functions: Vec::new(),
            quads: Vec::new(),
        };
        MissionScript::from_scb(ScbFile {
            version: SCB_VERSION,
            classes: vec![startup],
        })
        .expect("minimal StartUp script must load")
    }

    #[test]
    fn external_this_actor_success_restores_callback_and_parked_state() {
        let mut engine = EngineInner::new();
        let mut script = empty_mission_script();
        script.game_host.script_this = 41;
        script.game_host.entities.push(None);
        engine.mission_script = Some(script);

        let result =
            engine.call_external_native_with_this(&LevelAssets::new(), "ThisActor", &[], Some(99));

        assert_eq!(result, Ok(99));
        assert!(engine.entities.is_empty());
        let host = engine
            .mission_script
            .as_ref()
            .expect("script remains installed")
            .game_host()
            .expect("mission script always owns its game host");
        assert_eq!(host.script_this, 41);
        assert_eq!(host.entities.len(), 1);
    }

    fn fail_inside_script_context(
        script: &mut MissionScript,
        entities: &mut crate::entities::Entities,
        ai_global: &mut crate::ai::AiGlobalState,
        fast_grid: &mut crate::fast_find_grid::FastFindGrid,
        campaign: &mut Option<crate::campaign::Campaign>,
        mission_stat: &mut crate::mission_stat::MissionStat,
    ) -> Result<(), &'static str> {
        let mut context = script.script_context(
            entities,
            ai_global,
            fast_grid,
            campaign,
            mission_stat,
            Some(99),
        );
        assert_eq!(context.game_host_mut().script_this, 99);
        Err("simulated native error")
    }

    #[test]
    fn script_context_error_restores_callback_and_engine_ownership() {
        let mut script = empty_mission_script();
        script.game_host.script_this = 41;
        script.game_host.entities.push(None);
        let mut engine = EngineInner::new();

        let result = fail_inside_script_context(
            &mut script,
            &mut engine.entities,
            &mut engine.ai_global,
            &mut engine.fast_grid,
            &mut engine.campaign,
            &mut engine.mission_stat,
        );

        assert_eq!(result, Err("simulated native error"));
        assert!(engine.entities.is_empty());
        assert_eq!(script.game_host.script_this, 41);
        assert_eq!(script.game_host.entities.len(), 1);
    }

    #[test]
    fn external_native_early_returns_without_touching_callback_state() {
        let mut engine = EngineInner::new();
        let mut script = empty_mission_script();
        script.game_host.script_this = 41;
        script.game_host.entities.push(None);
        engine.mission_script = Some(script);

        let result = engine.call_external_native_with_this(
            &LevelAssets::new(),
            "NotAnOriginalNative",
            &[],
            Some(99),
        );

        assert_eq!(result, Err("unknown native: NotAnOriginalNative".into()));
        assert!(engine.entities.is_empty());
        let host = &engine
            .mission_script
            .as_ref()
            .expect("script remains installed")
            .game_host;
        assert_eq!(host.script_this, 41);
        assert_eq!(host.entities.len(), 1);
    }
}

/// Schedule a finish for a freshly-activated source if its kind is
/// `Single` or `Volatile` — the two kinds that terminate on their own.
/// `Looped` never ends; `Delayed` runs its own sim-side re-roll in
/// `perform_hourglass` and isn't scheduled here.
///
/// A missing duration means the original cache lookup would return a
/// zero-length sample and complete it in the sound hourglass. Schedule
/// that same zero-length result and warn rather than inventing a duration.
fn schedule_source_finish(
    kind: &crate::sound_source::SoundSourceKind,
    sample_id: u32,
    source_index: usize,
    cur_frame: u32,
    durations: &super::SourceDurations,
    playing_sources: &mut Vec<crate::sound::PlayingSource>,
) {
    use crate::sound_source::SoundSourceKind;
    match kind {
        SoundSourceKind::Single | SoundSourceKind::Volatile => {
            let duration = durations.get(&sample_id).copied().unwrap_or_else(|| {
                tracing::warn!(
                    sample_id,
                    "sound source missing from source_durations table; \
                     scheduling zero-length completion"
                );
                0
            });
            playing_sources.push(crate::sound::PlayingSource {
                source_index: source_index as u32,
                finish_frame: cur_frame + duration,
            });
        }
        SoundSourceKind::Looped | SoundSourceKind::Delayed => {}
    }
}

#[cfg(test)]
mod sound_completion_tests {
    use super::*;
    use crate::sound::PlayingSource;
    use crate::sound_source::SoundSourceKind;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    #[test]
    fn source_finish_uses_exact_metadata_duration() {
        let durations = Arc::new(BTreeMap::from([(0x1234, 9)]));
        let mut playing = Vec::<PlayingSource>::new();

        schedule_source_finish(
            &SoundSourceKind::Single,
            0x1234,
            4,
            100,
            &durations,
            &mut playing,
        );

        assert_eq!(playing.len(), 1);
        assert_eq!(playing[0].source_index, 4);
        assert_eq!(playing[0].finish_frame, 109);
    }

    #[test]
    fn missing_source_duration_schedules_zero_length_completion() {
        let durations = Arc::new(BTreeMap::new());
        let mut playing = Vec::<PlayingSource>::new();

        schedule_source_finish(
            &SoundSourceKind::Volatile,
            0x5678,
            7,
            100,
            &durations,
            &mut playing,
        );

        assert_eq!(playing.len(), 1);
        assert_eq!(playing[0].source_index, 7);
        assert_eq!(
            playing[0].finish_frame, 100,
            "missing samples complete at the next drain, never after a fabricated 75 frames"
        );
    }
}

/// Walk every active source in `sound_sim.sources` and schedule a
/// fresh finish for the `Single` / `Volatile` ones.  Called from the
/// `ResumeAll` dispatch so a script-triggered suspend/resume
/// round-trip produces the same kind-specific termination the host
/// used to drive via SDL_mixer playback completion.
fn schedule_source_finishes_for_all_active(
    sound_sim: &mut crate::sound::SoundSimState,
    durations: &super::SourceDurations,
    cur_frame: u32,
) {
    for i in 0..sound_sim.sources.num_sources() {
        let Some(src) = sound_sim.sources.get(i) else {
            continue;
        };
        if !src.active {
            continue;
        }
        let kind = src.source_kind;
        let id = src.id;
        // Re-arming duplicates would stack a second finish on top of
        // any existing entry, so cancel first.
        sound_sim
            .playing_sources
            .retain(|p| p.source_index as usize != i);
        schedule_source_finish(
            &kind,
            id,
            i,
            cur_frame,
            durations,
            &mut sound_sim.playing_sources,
        );
    }
}

impl EngineInner {
    /// Dispatch a single native function from outside the script VM
    /// (HTTP-RPC, debug console, etc.).
    ///
    /// Goes through the same swap-engine-state / `sync_game_host_post_script`
    /// dance script callbacks use, so any side-effect commands the
    /// native queues (camera, dialog, sequence Start/Thanx, sound,
    /// deferred game-logic) are drained as if a script had made the
    /// call.
    ///
    /// `args` are pushed onto a fresh `NativeStack` in script-source
    /// order (i.e. `args[0]` is the first argument to the native, and
    /// will be popped *last* — matches the `Param`/`Pop` LIFO contract).
    ///
    /// When `this_actor` is `Some`, the GameHost's `script_this` field
    /// is overridden for the duration of the call and restored after,
    /// matching the `set_script_this` bracket scripted dispatches use.
    /// Pass `None` to leave `script_this` unchanged.
    pub fn call_external_native(
        &mut self,
        assets: &LevelAssets,
        native_name: &str,
        args: &[i32],
    ) -> Result<i32, String> {
        self.call_external_native_with_this(assets, native_name, args, None)
    }

    /// Like [`Self::call_external_native`], but with an explicit
    /// `this_actor` override applied to `GameHost::script_this` for the
    /// duration of the dispatch. Restored after — even on early returns.
    pub fn call_external_native_with_this(
        &mut self,
        assets: &LevelAssets,
        native_name: &str,
        args: &[i32],
        this_actor: Option<i32>,
    ) -> Result<i32, String> {
        use crate::interp::NativeStack;
        use crate::natives::NativeFn;

        // Resolve name -> index. The enum implements `IntoStaticStr`
        // (one-way), so reverse lookup is a small linear scan over the
        // ~291 known indices. Comparison is case-insensitive — script
        // source uses CamelCase but JSON callers may not match exactly.
        let mut found_index: Option<u32> = None;
        for i in 0u32..512 {
            if let Ok(n) = NativeFn::try_from(i) {
                let s: &'static str = n.into();
                if s.eq_ignore_ascii_case(native_name) {
                    found_index = Some(i);
                    break;
                }
            }
        }
        let Some(index) = found_index else {
            return Err(format!("unknown native: {native_name}"));
        };

        if self.mission_script.is_none() {
            return Err("no mission script loaded (no mission running)".into());
        }

        // Mirror the in-script dispatch transaction. ScriptContext owns the
        // restoration bracket, so errors or future early returns cannot leave
        // engine state parked on GameHost.
        self.refresh_game_host_entity_state();
        let script = self
            .mission_script
            .as_mut()
            .expect("mission_script presence checked above");

        let return_value = {
            let mut context = script.script_context(
                &mut self.entities,
                &mut self.ai_global,
                &mut self.fast_grid,
                &mut self.campaign,
                &mut self.mission_stat,
                this_actor,
            );
            let game_host = context.game_host_mut();
            let mut stack = NativeStack::default();
            for &a in args {
                stack.push_i32(a);
            }
            let ret = <crate::natives::GameHost as crate::interp::HostFunctions>::call(
                game_host, index, &mut stack,
            );
            ret
        };

        self.sync_game_host_post_script(assets);

        Ok(return_value)
    }
}
