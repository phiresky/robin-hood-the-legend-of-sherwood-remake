//! Script/GameHost wiring, mission script management, campaign integration.

use super::scroll_reveal::ScrollStatus;
use super::*;
use crate::ai::AiStateChangeSource;
use crate::campaign::{Campaign, CampaignValue};
use crate::messenger::{Message, MessageType, SimpleMessage};
use crate::profiles::{MissionLocation, MissionProfile};

/// Script-originated effects removed from the VM adapter before processing.
///
/// Draining first ends the `MissionScript` borrow. Effect handlers may then
/// synchronously re-enter script dispatch while the canonical VM remains in
/// `ScriptRuntime`; no engine state or script owner is parked elsewhere.
#[derive(Default)]
struct PendingScriptEffects {
    sound: Vec<crate::natives::SoundCommand>,
    engine: Vec<crate::natives::EngineCommand>,
    completed_sequences: Vec<crate::sequence::Sequence>,
    deferred: Vec<crate::natives::DeferredCommand>,
}

impl PendingScriptEffects {
    fn drain(script: &mut MissionScript) -> Self {
        let game_host = &mut script.game_host;
        Self {
            sound: std::mem::take(&mut game_host.sound_commands),
            engine: game_host.drain_commands(),
            completed_sequences: game_host.take_completed_sequences(),
            deferred: std::mem::take(&mut game_host.deferred_commands),
        }
    }
}

impl EngineInner {
    /// Canonical entry boundary for global, actor, zone, target, scroll, and
    /// waypoint script callbacks. The VM and every native capability are
    /// disjoint borrows of their sole owners; nothing is removed from
    /// `EngineInner`, including while nested callbacks resume the outer VM.
    pub(super) fn with_script_session<R>(
        &mut self,
        assets: &LevelAssets,
        callback: impl FnOnce(
            &mut MissionScript,
            &mut crate::engine::ScriptDomains,
            &crate::natives::NativeSessionCapabilities<'_>,
        ) -> R,
    ) -> Option<R> {
        self.refresh_script_sight_bindings();
        self.scripts.assert_native_attachments_ready();
        let result = {
            let EngineInner {
                mission_domain,
                control,
                ai,
                world,
                script_domains,
                orders,
                scripts,
                players,
                feedback,
            } = self;
            let script = scripts.mission.as_mut()?;
            script.assert_no_active_call_frames();
            let campaign = &mut mission_domain.campaign;
            let capabilities = crate::natives::NativeSessionCapabilities::new(
                &mut world.entities,
                &mut ai.global,
                &mut world.fast_grid,
            )
            .with_queries(
                &orders.sequence_manager,
                &players.seats[0].selection,
                &feedback.sound_sim.sources,
                &world.weather,
                &control.frame_counter,
            )
            .with_campaign(campaign, &mut mission_domain.mission_stat);
            let result = callback(script, script_domains, &capabilities);
            script.assert_no_active_call_frames();
            result
        };
        self.drain_script_effects(assets);
        Some(result)
    }

    /// Normalize campaign/custom values from the legacy GameHost save shape
    /// into their canonical campaign/entity owners. Contradictory duplicate
    /// values are corrupt and must not be resolved by choosing one silently.
    pub(super) fn migrate_legacy_script_custom_values(&mut self) {
        let Some(legacy) = self
            .scripts
            .mission
            .as_mut()
            .and_then(|script| script.legacy_custom_values.take())
        else {
            return;
        };

        if let Some(parked) = legacy.parked_campaign {
            let parked_value = serde_json::to_value(&parked)
                .expect("serialize legacy parked campaign for comparison");
            let canonical_value = serde_json::to_value(&self.mission_domain.campaign)
                .expect("serialize canonical campaign for comparison");
            assert_eq!(
                parked_value, canonical_value,
                "legacy GameHost campaign contradicts canonical engine campaign"
            );
        }

        if !legacy.campaign.is_empty() {
            let campaign = Some(&mut self.mission_domain.campaign)
                .expect("legacy script campaign values require an active campaign");
            for (index, value) in legacy.campaign {
                let slot = CampaignValue::custom(index).unwrap_or_else(|| {
                    panic!("legacy script save has invalid campaign custom-value index {index}")
                });
                let canonical = campaign.values[slot];
                assert!(
                    canonical == 0 || canonical == value,
                    "legacy script campaign value {index} contradicts canonical value: legacy={value}, canonical={canonical}"
                );
                campaign.values[slot] = value;
            }
        }

        for ((actor_handle, index), value) in legacy.npc {
            assert!(
                (0..crate::element_kinds::NpcCustomValue::COUNT as i32).contains(&index),
                "legacy script save has invalid NPC custom-value index {index} for actor {actor_handle}"
            );
            let entity_id = self
                .entity_id_for_actor_handle(actor_handle)
                .unwrap_or_else(|| {
                    panic!("legacy script save references missing NPC actor {actor_handle}")
                });
            let npc = self
                .world
                .entities
                .get_mut(entity_id)
                .and_then(|entity| entity.npc_data_mut())
                .unwrap_or_else(|| {
                    panic!("legacy script save references non-NPC actor {actor_handle}")
                });
            let canonical = npc.custom_values[index as usize];
            assert!(
                canonical == 0 || canonical == value,
                "legacy script NPC value ({actor_handle}, {index}) contradicts canonical value: legacy={value}, canonical={canonical}"
            );
            npc.custom_values[index as usize] = value;
        }
    }

    /// Reattach sight-obstacle arrays rebuilt by live world mutations.
    pub(super) fn refresh_script_sight_bindings(&mut self) {
        self.scripts.refresh_sight_bindings(
            &self.world.dynamic_sight_obstacles,
            &self.world.static_sight_obstacle_active,
        );
    }

    /// Attach immutable level data to the script-native dispatcher.
    ///
    /// The dispatcher borrows this object for each VM resume. It is not part
    /// of simulation state and is reattached after save/snapshot decode.
    pub(super) fn attach_script_bindings(&mut self, assets: &LevelAssets) {
        self.scripts.attach_native_capabilities(
            assets,
            &self.world.dynamic_sight_obstacles,
            &self.world.static_sight_obstacle_active,
        );
    }

    /// Drain and apply script-originated effects after a callback batch.
    ///
    /// The queue batch is removed under a short `ScriptRuntime` borrow before
    /// any effect is executed. Handlers can therefore re-enter the same live
    /// VM synchronously without a take/restore ownership transaction.
    pub(crate) fn drain_script_effects(&mut self, assets: &LevelAssets) {
        let effects = match self.scripts.mission.as_mut() {
            Some(script) => PendingScriptEffects::drain(script),
            None => return,
        };
        let PendingScriptEffects {
            sound,
            engine: engine_commands,
            completed_sequences,
            deferred,
        } = effects;

        // Commands whose handlers can synchronously call the mission VM are
        // kept until the first-pass state effects have completed.
        let mut post_script: Vec<crate::natives::DeferredCommand> = Vec::new();
        // RHScript::SendMessage launches a standalone RHCOMMAND_SEND_MESSAGE
        // sequence element. Keep requests in native-call order and launch
        // them once the mission script is installed again so their immediate
        // ProcessMessage callback can re-enter the script system this frame.
        let mut script_messages: Vec<(i32, i32, i32, i32)> = Vec::new();

        // ── Sound commands ──
        // Commands that don't need an AudioBackend are processed now.
        // The remaining ones are queued for main_entry to flush.
        for cmd in sound {
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
                    for i in 0..self.feedback.sound_sim.sources.num_sources() {
                        if let Some(src) = self.feedback.sound_sim.sources.get_mut(i)
                            && src.active
                        {
                            stashed.push(i as u32);
                            src.active = false;
                        }
                    }
                    self.feedback.sound_sim.suspended_active_sources = stashed;
                    self.feedback.sound_sim.playing_sources.clear();
                }
                crate::natives::SoundCommand::ResumeAll => {
                    // Restore `active` on every source that was
                    // active at the last suspend — preserves the
                    // active flag across suspend/resume.
                    let stashed =
                        std::mem::take(&mut self.feedback.sound_sim.suspended_active_sources);
                    for idx in stashed {
                        if let Some(src) = self.feedback.sound_sim.sources.get_mut(idx as usize) {
                            src.active = true;
                        }
                    }
                    let pos = self.feedback.cutscene_camera.view_position;
                    let zoom = self.feedback.cutscene_camera.zoom_factor;
                    self.feedback.pending_side_effects.sounds.push(
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
                        &mut self.feedback.sound_sim,
                        &assets.source_durations,
                        self.control.frame_counter,
                    );
                }
                crate::natives::SoundCommand::Activate(h) => {
                    // Mark active sim-side (participates in rollback hash),
                    // then emit the side-effect so the host audio backend
                    // picks up the source and starts a channel.  Symmetric
                    // with the Deactivate path below.
                    if let Some(idx) = crate::natives::ScriptHandleCodec::sound_source_index(h) {
                        // Re-activation cancels any previously
                        // scheduled finish so we don't prematurely
                        // kill a freshly-restarted source.
                        self.feedback
                            .sound_sim
                            .playing_sources
                            .retain(|p| p.source_index as usize != idx);
                        if let Some(src) = self.feedback.sound_sim.sources.get_mut(idx) {
                            src.active = true;
                            schedule_source_finish(
                                &src.source_kind,
                                src.id,
                                idx,
                                self.control.frame_counter,
                                &assets.source_durations,
                                &mut self.feedback.sound_sim.playing_sources,
                            );
                        }
                        self.feedback
                            .pending_side_effects
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
                    if let Some(idx) = crate::natives::ScriptHandleCodec::sound_source_index(h) {
                        if let Some(src) = self.feedback.sound_sim.sources.get_mut(idx) {
                            src.active = false;
                        }
                        self.feedback
                            .sound_sim
                            .playing_sources
                            .retain(|p| p.source_index as usize != idx);
                    }
                }
                crate::natives::SoundCommand::Destroy(h) => {
                    if let Some(idx) = crate::natives::ScriptHandleCodec::sound_source_index(h) {
                        if let Some(src) = self.feedback.sound_sim.sources.get_mut(idx) {
                            src.active = false;
                        }
                        self.feedback.sound_sim.sources.delete(idx);
                        self.feedback
                            .sound_sim
                            .playing_sources
                            .retain(|p| p.source_index as usize != idx);
                    }
                }
            }
        }

        // ── Completed sequences (from Record*/Thanx) ──
        for seq in completed_sequences {
            self.launch_sequence(seq);
        }

        // ── Deferred game-logic commands ──
        // Re-entrant handlers are deferred until the first-pass effects
        // have released all temporary borrows.
        for cmd in deferred {
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
                            self.players.seats[0].selection.retain(|&x| x != id);
                        }
                    }
                }
                crate::natives::DeferredCommand::StopActor { actor } => {
                    if let Some(id) = self.entity_id_for_actor_handle(actor) {
                        self.stop_owner(id, crate::sequence::SequencePriority::Script);
                    }
                }
                crate::natives::DeferredCommand::FreezeAll { freeze } => {
                    self.set_actors_frozen(freeze);
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
                        && let Some(entity) = self.world.entities.get(id)
                    {
                        let still_unconscious = entity.human_data().is_some_and(|h| h.unconscious);
                        self.feedback.titbit_manager.remove_unconscious_stars_if(
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
                    self.orders.messenger.send(Message::pc(msg_type, pc_id));
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
                            .orders
                            .sequence_manager
                            .current_element_for_actor(owner)
                            .and_then(|(seq_id, elem_idx)| {
                                self.orders.sequence_manager.get_element(seq_id, elem_idx)
                            })
                            .is_some_and(|elem| elem.command == crate::element::Command::LockAi);
                        if let Some(entity) = self.world.entities.get_mut(owner)
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
                        && let Some(entity) = self.world.entities.get_mut(id)
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
                            if let Some(state) = self.players.macro_store.get_mut(pc_id) {
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
                        self.orders.sequence_manager.launch_element(elem);
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
                        if let Some(entity) = self.world.entities.get_mut(id)
                            && let Some(ai) = entity.ai_controller_mut()
                        {
                            ai.pending_stimuli.push(crate::ai::Stimulus::new(
                                crate::ai::StimulusType::EventLoseConsciousness,
                            ));
                        }
                        // Only NPCs broadcast their body — guard via
                        // `is_npc()` to avoid touching a PC or non-actor
                        // slot.
                        if let Some(entity) = self.world.entities.get(id)
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
                        && let Some(entity) = self.world.entities.get(id)
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
                    let Some(entity) = self.world.entities.get(id) else {
                        continue;
                    };
                    let phase = if let crate::element::Entity::Pc(pc) = entity {
                        let profile = assets
                            .profile_manager
                            .get_character(pc.pc.profile_index)
                            .unwrap_or_else(|| {
                                panic!(
                                    "AddHiddenTitbitForActor: PC {} has unknown profile_index {}",
                                    id.index(),
                                    pc.pc.profile_index
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
                    self.feedback.titbit_manager.add_titbit(
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
    pub(super) fn launch_script_send_message(
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
            .orders
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
        self.orders
            .sequence_manager
            .element_terminated(sequence_id, 0);
    }

    /// Load a mission script from the level directory.
    ///
    /// Looks up the pre-decoded script program in
    /// `assets.mission_script_programs` and installs it into
    /// `self.scripts.mission`.
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
                self.scripts.install_mission(script);
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
        self.refresh_script_sight_bindings();
        self.attach_script_bindings(assets);

        // Collect per-actor script classes before borrowing canonical entities.
        // Each actor with a script_class gets IActorScript::Initialize()
        // called during loading (before StartUp::Initialize).
        let per_actor_scripts: Vec<(i32, String)> = self
            .world
            .entities
            .actors()
            .filter_map(|(entity_id, entity)| {
                let script_class = &entity.actor_data()?.script_class;
                if script_class.is_empty() {
                    return None;
                }
                Some((
                    crate::natives::ScriptHandleCodec::actor_handle(entity_id),
                    script_class.clone(),
                ))
            })
            .collect();

        // Same collection pass for FX targets — each target with a
        // non-empty `script_class` gets its own `ScriptInstance`.
        // Each target carries its own VM and `Initialize()` runs
        // during `InitializeFromMissionStream`.
        let per_target_scripts: Vec<(i32, String)> = self
            .world
            .entities
            .targets()
            .filter_map(|(entity_id, target)| {
                if target.target.script_class.is_empty() {
                    return None;
                }
                Some((
                    crate::natives::ScriptHandleCodec::actor_handle(entity_id),
                    target.target.script_class.clone(),
                ))
            })
            .collect();

        // Scrolls also carry their own VMs — bind the class during
        // `InitializeFromMissionStream` and walk the list calling
        // `IScrollScript::Initialize()`.
        let per_scroll_scripts: Vec<(i32, String)> = self
            .world
            .entities
            .scrolls()
            .filter_map(|(entity_id, scroll)| {
                if scroll.script_class.is_empty() {
                    return None;
                }
                Some((
                    crate::natives::ScriptHandleCodec::actor_handle(entity_id),
                    scroll.script_class.clone(),
                ))
            })
            .collect();

        let _ = self.with_script_session(assets, |script, script_domains, capabilities| {
            // ── Phase 1: Per-actor Initialize ──
            // Each actor's script class gets a ScriptInstance that persists for the
            // actor's lifetime — the heap (member variables) survives across calls
            // to Initialize, ActionChange, HandleEvent, FilterAIEvent, ProcessMessage.
            // Each VM receives a short-lived native context over the same
            // canonical capability bundle.
            let mut init_count = 0u32;
            for (handle, class_name) in &per_actor_scripts {
                if script.bind_actor(*handle, class_name, script_domains, capabilities) {
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
                if script.bind_target(*handle, class_name, script_domains, capabilities) {
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
                if script.bind_scroll(*handle, class_name, script_domains, capabilities) {
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
                    if script.bind_waypoint(
                        pid,
                        wp_idx as u8,
                        class_name,
                        script_domains,
                        capabilities,
                    ) {
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
            let frame = crate::natives::ScriptCallFrame::default();
            let startup_result = script.with_call_frame(frame, |script| {
                MissionScript::with_game_host_attached(
                    &mut script.game_host,
                    &mut script.state,
                    script_domains,
                    &script.bindings,
                    capabilities,
                    frame,
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
                )
            });
            match startup_result {
                Ok(ret) => tracing::info!("Script StartUp::Initialize returned {ret}"),
                Err(crate::script_manager::ScriptError::Vm(
                    crate::interp::StopReason::StepLimit,
                )) => {
                    tracing::warn!("Script StartUp::Initialize hit step limit (100K)");
                }
                Err(e) => tracing::warn!("Script StartUp::Initialize failed: {e}"),
            }
        });

        // ── Mark AiControllers whose bound class overrides FilterAIEvent ──
        // Read by cascade `think()` sites in ai_enemy.rs to decide
        // whether to warn about the "would re-filter here, didn't"
        // divergence.  Unscripted NPCs leave the flag at its default
        // `false` and stay silent. This iteration reads the canonical engine
        // entity store directly.
        if let Some(script) = self.scripts.mission.as_ref() {
            let scripted_actors: Vec<i32> = script.actor_instances.keys().copied().collect();
            for handle in scripted_actors {
                let has_override = script.actor_has_function(handle, "FilterAIEvent");
                if !has_override {
                    continue;
                }
                let Some(id) = self.entity_id_for_actor_handle(handle) else {
                    continue;
                };
                if let Some(entity) = self.world.entities.get_mut(id)
                    && let Some(ai) = entity.ai_controller_mut()
                {
                    ai.has_script_filter_override = true;
                }
            }
        }

        // ── Phase 3: Zone script Initialize ──
        self.initialize_zone_scripts(assets);

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
    pub(crate) fn finalize_mission_script(&mut self, assets: &LevelAssets, abandoned: bool) {
        let _ = self.with_script_session(assets, |script, script_domains, capabilities| {
            if let Err(e) = script.finalize(abandoned, script_domains, capabilities) {
                tracing::warn!("Script Finalize failed: {e}");
            }
        });
    }

    // ─── Per-actor script event dispatch ───────────────────────────

    /// Check all scripted actors for animation changes and dispatch
    /// `ActionChange(newAction, oldAction)` to their per-actor scripts.
    ///
    /// Calls `ActionChange` when the current animation differs from
    /// `old_action`.  Called once per frame from `perform_hourglass`,
    /// after all animation updates.
    pub(crate) fn dispatch_actor_action_changes(&mut self, assets: &LevelAssets) {
        if self.scripts.mission.is_none() {
            return;
        }

        // Phase 1: Collect actors whose animation changed.
        // Current animation = front order of the actor's current
        // in-progress sequence element.
        let mut changes = Vec::new();
        for (entity_id, entity) in self.world.entities.actors() {
            let Some(actor) = entity.actor_data() else {
                continue;
            };
            if actor.script_class.is_empty() {
                continue;
            }

            let current_anim = self
                .orders
                .sequence_manager
                .current_order_for_actor(entity_id)
                .map(|(_, _, o)| o.order_type)
                .unwrap_or(crate::order::OrderType::WaitingUpright);

            if current_anim != actor.old_action {
                changes.push((entity_id, current_anim, actor.old_action));
            }
        }
        // Apply old_action updates in a second pass (the peek loop
        // above only reads self.world.entities to avoid conflicting with the
        // sequence_manager borrow).
        for &(entity_id, new_anim, _) in &changes {
            if let Some(entity) = self.world.entities.get_mut(entity_id)
                && let Some(actor) = entity.actor_data_mut()
            {
                actor.old_action = new_anim;
            }
        }
        let changes: Vec<(i32, i32, i32)> = changes
            .into_iter()
            .map(|(entity_id, new_anim, old_anim)| {
                (
                    crate::natives::ScriptHandleCodec::actor_handle(entity_id),
                    new_anim as i32,
                    old_anim as i32,
                )
            })
            .collect();

        if changes.is_empty() {
            return;
        }

        // Phase 2: Dispatch to scripts in collection order.
        let _ = self.with_script_session(assets, |script, script_domains, capabilities| {
            for (handle, new_anim, old_anim) in &changes {
                if let Err(e) = script.call_actor_function(
                    *handle,
                    "ActionChange",
                    &[*new_anim, *old_anim],
                    script_domains,
                    capabilities,
                ) {
                    tracing::warn!("ActionChange (handle {handle}): {e}");
                }
            }
        });
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

        if self.scripts.mission.is_none() {
            return;
        }

        // Phase 1: bump timers; collect handles whose script is due to
        // fire this frame. The mutable walk can't also borrow the mission
        // script, so the list of ready-to-fire scrolls is captured first.
        let mut ready: Vec<i32> = Vec::new();
        for (id, s) in self.world.entities.scrolls_mut() {
            if !s.element.active {
                continue;
            }
            let handle = crate::natives::ScriptHandleCodec::actor_handle(id);
            let has_script = self
                .scripts
                .mission
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

        // Phase 2: dispatch in scroll slot order. Per-scroll `Hourglass` is
        // distinct from the engine callback and passes the literal zero.
        let _ = self.with_script_session(assets, |script, script_domains, capabilities| {
            for handle in &ready {
                if let Err(e) = script.call_scroll_function(
                    *handle,
                    "Hourglass",
                    &[0],
                    script_domains,
                    capabilities,
                ) {
                    tracing::warn!("Scroll Hourglass (handle {handle}): {e}");
                }
            }
        });
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

        let handle = crate::natives::ScriptHandleCodec::actor_handle(scroll_id);

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
            .scripts
            .mission
            .as_ref()
            .is_some_and(|ms| ms.scroll_instances.contains_key(&handle));
        if !has_script {
            return false;
        }

        // Step 3 — dispatch via the SetScrollExecutingScript bracket.
        let pc_handle = crate::natives::ScriptHandleCodec::actor_handle(pc_id);
        let result = self
            .with_script_session(assets, |script, script_domains, capabilities| {
                script.call_scroll_function(
                    handle,
                    "IsTaken",
                    &[pc_handle],
                    script_domains,
                    capabilities,
                )
            })
            .expect("bound scroll script disappeared before IsTaken dispatch");

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
    pub(crate) fn initialize_zone_scripts(&mut self, assets: &LevelAssets) {
        let classes: Vec<(usize, String)> = self
            .script_domains
            .zones
            .scripts
            .iter()
            .enumerate()
            .filter_map(|(zone_idx, zone_data)| {
                zone_data
                    .script_class_name
                    .as_ref()
                    .map(|name| (zone_idx, name.clone()))
            })
            .collect();

        let _ = self.with_script_session(assets, |script, script_domains, capabilities| {
            let mut init_count = 0u32;
            for (zone_idx, class_name) in classes {
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

                let frame = crate::natives::ScriptCallFrame::default();
                script.with_call_frame(frame, |script| {
                    MissionScript::with_game_host_attached(
                        &mut script.game_host,
                        &mut script.state,
                        script_domains,
                        &script.bindings,
                        capabilities,
                        frame,
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
                });
                script.zone_instances.insert(zone_idx, zone_inst);
            }

            if init_count > 0 {
                tracing::info!(
                    "Initialized {init_count} zone scripts ({} instances persisted)",
                    script.zone_instances.len()
                );
            }
        });
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
        for (actor_id, entity) in self.world.entities.actors() {
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
            let handle = crate::natives::ScriptHandleCodec::actor_handle(actor_id);

            for (zone_idx, &grid_idx) in assets.script_zone_grid_indices.iter().enumerate() {
                // Skip zones that `DefineFlatTrajectoryZone` converted
                // into apex sectors — once converted, the SECTOR_SCRIPT
                // flag is dropped so the engine stops scanning them.
                if self
                    .script_domains
                    .zones
                    .scripts
                    .get(zone_idx)
                    .is_some_and(|z| z.transformed_to_apex)
                {
                    continue;
                }
                let gs = &self.world.fast_grid.level.sectors[grid_idx as usize];
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
            let Some(entity) = self.world.entities.get(eidx) else {
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
            let carried_h = crate::natives::ScriptHandleCodec::actor_handle(carried_id);
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
            self.script_domains.zones.scripts[zone_idx].enter(entity_idx);
            let pt = self.script_domains.zones.scripts[zone_idx].production_sector_type;
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
        for zone in &mut self.script_domains.zones.scripts {
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
        let _ = self.with_script_session(assets, |script, script_domains, capabilities| {
            for &(zone_idx, _, handle) in &entries {
                if let Err(e) = script.call_zone_function(
                    zone_idx,
                    "EnterZone",
                    &[handle],
                    script_domains,
                    capabilities,
                ) {
                    tracing::warn!("Zone {zone_idx} EnterZone (actor {handle}): {e}");
                }
            }
        });

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
        if assets.script_zone_grid_indices.is_empty() || self.scripts.mission.is_none() {
            return;
        }

        // Phase 1: Collect enter/exit events by comparing current positions
        // with zone occupant lists.
        let mut enter_events: Vec<(usize, crate::entity_id::EntityId, i32)> = Vec::new();
        let mut exit_events: Vec<(usize, crate::entity_id::EntityId, i32)> = Vec::new();

        for (actor_id, entity) in self.world.entities.actors() {
            let eidx = actor_id.into();
            let ed = entity.element_data();
            let active = ed.active && !ed.in_honolulu;
            let pos = ed.position_map();
            let layer = ed.layer();
            let handle = crate::natives::ScriptHandleCodec::actor_handle(actor_id);

            for (zone_idx, &grid_idx) in assets.script_zone_grid_indices.iter().enumerate() {
                // Skip apex-converted zones — see scan_zone_occupant_entries note.
                if self.script_domains.zones.scripts[zone_idx].transformed_to_apex {
                    continue;
                }
                let gs = &self.world.fast_grid.level.sectors[grid_idx as usize];
                let was_inside = self.script_domains.zones.scripts[zone_idx].is_inside(eidx);
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
            let Some(entity) = self.world.entities.get(eidx) else {
                continue;
            };
            let Some(carried_id) = entity.pc_data().and_then(|pc| pc.carried) else {
                continue;
            };
            if self.script_domains.zones.scripts[zone_idx].is_inside(carried_id) {
                continue;
            }
            if enter_events
                .iter()
                .any(|&(z, e, _)| z == zone_idx && e == carried_id)
            {
                continue;
            }
            let carried_h = crate::natives::ScriptHandleCodec::actor_handle(carried_id);
            enter_events.push((zone_idx, carried_id, carried_h));
        }
        let primary_exit_len = exit_events.len();
        for i in 0..primary_exit_len {
            let (zone_idx, eidx, _) = exit_events[i];
            let Some(entity) = self.world.entities.get(eidx) else {
                continue;
            };
            let Some(carried_id) = entity.pc_data().and_then(|pc| pc.carried) else {
                continue;
            };
            if !self.script_domains.zones.scripts[zone_idx].is_inside(carried_id) {
                continue;
            }
            if exit_events
                .iter()
                .any(|&(z, e, _)| z == zone_idx && e == carried_id)
            {
                continue;
            }
            let carried_h = crate::natives::ScriptHandleCodec::actor_handle(carried_id);
            exit_events.push((zone_idx, carried_id, carried_h));
        }

        if enter_events.is_empty() && exit_events.is_empty() {
            return;
        }

        // Phase 2: Update occupant lists and apply production work icons.
        for &(zone_idx, entity_idx, _) in &enter_events {
            self.script_domains.zones.scripts[zone_idx].enter(entity_idx);
            let pt = self.script_domains.zones.scripts[zone_idx].production_sector_type;
            if pt != crate::sector_production::Type::Unknown {
                self.apply_production_work_icon(entity_idx, pt, true);
            }
        }
        for &(zone_idx, entity_idx, _) in &exit_events {
            self.script_domains.zones.scripts[zone_idx].leave(entity_idx);
            let pt = self.script_domains.zones.scripts[zone_idx].production_sector_type;
            if pt != crate::sector_production::Type::Unknown {
                self.apply_production_work_icon(entity_idx, pt, false);
            }
        }

        // Phase 3: Dispatch enters before exits, preserving the original
        // batch order used by this port.
        let _ = self.with_script_session(assets, |script, script_domains, capabilities| {
            for &(zone_idx, _, handle) in &enter_events {
                if let Err(e) = script.call_zone_function(
                    zone_idx,
                    "EnterZone",
                    &[handle],
                    script_domains,
                    capabilities,
                ) {
                    tracing::warn!("Zone {zone_idx} EnterZone (actor {handle}): {e}");
                }
            }
            for &(zone_idx, _, handle) in &exit_events {
                if let Err(e) = script.call_zone_function(
                    zone_idx,
                    "ExitZone",
                    &[handle],
                    script_domains,
                    capabilities,
                ) {
                    tracing::warn!("Zone {zone_idx} ExitZone (actor {handle}): {e}");
                }
            }
        });
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
            .saturating_sub(self.script_domains.zones.scripts.len());

        let Some(ref mut script) = self.scripts.mission else {
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
            let Some(loc_idx) = crate::natives::ScriptHandleCodec::location_index(loc_handle)
            else {
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
            if zone_idx >= self.script_domains.zones.scripts.len() {
                tracing::warn!("RegisterAsProductionSector: zone {zone_idx} out of range");
                continue;
            }
            self.script_domains.zones.scripts[zone_idx].production_sector_type = prod_type_enum;

            // Attach to the campaign's SectorProduction so its `speed` is set.
            if let Some(campaign) = Some(&mut self.mission_domain.campaign)
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
            let Some(loc_idx) = crate::natives::ScriptHandleCodec::location_index(loc_handle)
            else {
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
            if let Some(campaign) = Some(&mut self.mission_domain.campaign)
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

        let Some(entity) = self.world.entities.get_mut(entity_id) else {
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
        let elem = match self.orders.sequence_manager.get_element(seq_id, elem_idx) {
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
        let _ = self.with_script_session(assets, |script, script_domains, capabilities| {
            // Per-actor ProcessMessage
            for &(handle, msg, arg1, arg2) in per_actor {
                if let Err(e) = script.call_actor_function(
                    handle,
                    "ProcessMessage",
                    &[msg, arg1, arg2],
                    script_domains,
                    capabilities,
                ) {
                    tracing::warn!("Sequence ProcessMessage (actor {handle}, msg {msg}): {e}");
                }
            }

            // EngineInner-level ProcessMessage → global StartUp script
            for &(msg, arg1, arg2) in engine_level {
                if script
                    .instance
                    .has_function(&script.manager, "ProcessMessage")
                {
                    let frame = crate::natives::ScriptCallFrame::default();
                    let result = script.with_call_frame(frame, |script| {
                        MissionScript::with_game_host_attached(
                            &mut script.game_host,
                            &mut script.state,
                            script_domains,
                            &script.bindings,
                            capabilities,
                            frame,
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
                        )
                    });
                    match result {
                        Ok(_) => {}
                        Err(e) => tracing::warn!("EngineInner ProcessMessage(msg {msg}): {e}"),
                    }
                }
            }
        });
    }

    /// Dispatch deferred `IElementTargetScript::ActivatedBy*(pPC)` calls.
    ///
    /// Each entry is `(target_handle, pc_handle, method_name)`. Binds
    /// `ThisActor` to the target then calls the relevant
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
        let _ = self.with_script_session(assets, |script, script_domains, capabilities| {
            for &(target_handle, pc_handle, fn_name) in calls {
                if let Err(e) = script.call_target_function(
                    target_handle,
                    fn_name,
                    &[pc_handle],
                    script_domains,
                    capabilities,
                ) {
                    tracing::warn!("{fn_name} (target {target_handle}): {e}");
                }
            }
        });
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
    /// the target entity, since the script session leases
    /// `self.world.entities` for the callback. The function is a
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
            crate::ai::StimulusInfo::Human(h) => crate::natives::ScriptHandleCodec::actor_handle(
                crate::element::EntityId::Soldier(crate::entity_id::SoldierId(h)),
            ),
            _ => 0,
        };

        // Fast paths that skip script dispatch.
        let has_override = match self.scripts.mission.as_ref() {
            Some(s) => s.actor_has_function(handle, "FilterAIEvent"),
            None => return true,
        };
        if !has_override {
            return true;
        }

        let result = self
            .with_script_session(assets, |script, script_domains, capabilities| {
                script.call_actor_function(
                    handle,
                    "FilterAIEvent",
                    &[source, code],
                    script_domains,
                    capabilities,
                )
            })
            .expect("checked mission-script presence above");

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
        let handle = crate::natives::ScriptHandleCodec::actor_handle(entity_id);
        if !self.filter_stimulus(assets, handle, stimulus) {
            return false;
        }
        // Hoist the door slice off `mission_script.game_host()`
        // before grabbing the mutable entity borrow — the friendly
        // AI's `alert_soldier` needs it for the
        // `ALERTFLAG_CHECK_DOOR_PATH` retry.
        let doors_ptr = self
            .scripts
            .mission
            .as_ref()
            .and_then(|ms| ms.game_host())
            .map(|_| self.script_domains.interactables.doors.as_slice());
        let ai_global = &mut self.ai.global;
        let Some(entity) = self.world.entities.get_mut(entity_id) else {
            return false;
        };
        if let Some(enemy_ai) = entity.enemy_ai_mut() {
            enemy_ai.think(
                stimulus,
                ai_global,
                ctx,
                tick_data,
                Some(&self.world.fast_grid),
            )
        } else if let Some(friendly_ai) = entity.friendly_ai_mut() {
            friendly_ai.think(
                stimulus,
                ai_global,
                ctx,
                tick_data,
                Some(&self.world.fast_grid),
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
        if self.scripts.mission.is_none() {
            return;
        }

        // Collect state changes: (npc_handle, source_handle, state_change_code).
        let mut notifications: Vec<(i32, i32, i32)> = Vec::new();
        for (id, entity) in self.world.entities.npcs_mut() {
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
            let handle = crate::natives::ScriptHandleCodec::actor_handle(id);
            for (state, source_kind) in drained {
                let code = state.state_change_event_code();
                let source = match source_kind {
                    AiStateChangeSource::SelfActor => handle,
                    AiStateChangeSource::Null => 0,
                    AiStateChangeSource::Human(h) => {
                        crate::natives::ScriptHandleCodec::actor_handle(
                            crate::element::EntityId::Soldier(crate::entity_id::SoldierId(h)),
                        )
                    }
                };
                notifications.push((handle, source, code));
            }
        }

        if notifications.is_empty() {
            return;
        }

        let _ = self.with_script_session(assets, |script, script_domains, capabilities| {
            for (handle, source, code) in &notifications {
                // Return value ignored — notification only.
                let _ = script.call_actor_function(
                    *handle,
                    "FilterAIEvent",
                    &[*source, *code],
                    script_domains,
                    capabilities,
                );
            }
        });
    }

    // ─── Campaign integration ────────────────────────────────────

    /// Initialize the engine for the campaign's current mission.
    ///
    /// The campaign must already be stored in `self.mission_domain.campaign`.
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
        let campaign = Some(&self.mission_domain.campaign)
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
        self.world.weather.is_forest_level = location == MissionLocation::Sherwood;

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
            self.mission_domain.mission_stat.living_soldier_count as i32,
        );
        campaign.add_value(
            CampaignValue::DeadSoldiers,
            self.mission_domain
                .mission_stat
                .total_soldier_count
                .saturating_sub(self.mission_domain.mission_stat.living_soldier_count)
                as i32,
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
        let idx = crate::natives::ScriptHandleCodec::location_index(handle)?;
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
                        self.feedback.cutscene_camera.camera_wanted = pos;
                        self.feedback.cutscene_camera.camera_slide =
                            self.check_location_is_valid_for_camera(pos);
                        self.control.speed = speed;
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
                        self.feedback.cutscene_camera.view_position =
                            self.check_location_is_valid_for_camera(pos);
                        self.feedback.pending_side_effects.invalidate_background = true;
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
                    self.feedback.cutscene_camera.desired_zoom_factor = zoom;
                    if zoom != self.feedback.cutscene_camera.zoom_factor {
                        self.feedback.cutscene_camera.mechanized_zoom = true;
                    }
                }
                EngineCommand::StartDialog { dialog_id } => {
                    tracing::debug!("StartDialog({dialog_id}): queued for game session");
                    self.feedback
                        .pending_side_effects
                        .pending_dialogues
                        .push(dialog_id);
                    self.orders
                        .messenger
                        .send(Message::new(MessageType::Simple(SimpleMessage::ResetInput)));
                }
                EngineCommand::DisplayMap { show } => {
                    self.feedback
                        .pending_side_effects
                        .pending_minimap_display_maps
                        .push((show, false));
                }
                EngineCommand::DisplayConsole => {
                    tracing::debug!("DisplayConsole: queued for UI system");
                    self.feedback.pending_side_effects.pending_show_console = true;
                    self.orders.messenger.send(Message::new(MessageType::Simple(
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
                        .saturating_sub(self.script_domains.zones.scripts.len());
                    let Some(loc_idx) =
                        crate::natives::ScriptHandleCodec::location_index(location_handle)
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
                        if let Some(zone) = self.script_domains.zones.scripts.get_mut(zone_idx) {
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
                                    self.world.fast_grid.or_sector_type_overlay(
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
                    self.mission_domain.short_briefings.add(id as u32, primary);
                }
                EngineCommand::DoneShortBriefing { id } => {
                    self.mission_domain.short_briefings.mark_done(id as u32);
                }
                EngineCommand::ChooseVictoryDefeatText { id } => {
                    self.mission_domain.state.victory_defeat_id = id as u32;
                }
                EngineCommand::DisplayPopupText { text_id } => {
                    tracing::debug!("DisplayPopupText({text_id}): queued for UI system");
                    self.feedback
                        .pending_side_effects
                        .pending_popup_texts
                        .push(text_id);
                    self.orders
                        .messenger
                        .send(Message::new(MessageType::Simple(SimpleMessage::ResetInput)));
                }
                EngineCommand::DisplaySherwoodReport => {
                    tracing::debug!("DisplaySherwoodReport: queued for UI system");
                    self.feedback.pending_side_effects.pending_sherwood_report = true;
                    self.orders
                        .messenger
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
                    self.feedback.pending_side_effects.fade_to_black = Some(if s == 0 {
                        None
                    } else {
                        Some(crate::engine::types::FadeToBlack {
                            speed: s,
                            frames_remaining: total_frames,
                        })
                    });
                    self.set_fade_freeze_frames_remaining(total_frames.saturating_sub(1));
                }
                EngineCommand::SetOutlineDisplay { display: show } => {
                    // Forward `MSG_SWITCH_MASKED_DISPLAY` when the
                    // state actually changes.  The rendering side
                    // (`game_render.rs:814` et al.) already reads
                    // `host.input.draw_hidden` to switch entities into
                    // the masked/outline draw mode.
                    self.feedback.pending_side_effects.set_draw_hidden = Some(show);
                }
                EngineCommand::SetViewRadius { radius } => {
                    self.ai.standard_view_polygon_radius = radius as u16;
                    self.propagate_view_radius();
                }
                EngineCommand::PlayJingle(jingle) => {
                    self.feedback
                        .pending_side_effects
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
                    let Some(entity) = self.world.entities.get_mut(id) else {
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
                    let Some(entity) = self.world.entities.get_mut(id) else {
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
                        if let Some(entity) = self.world.entities.get_mut(id) {
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
                        if let Some(entity) = self.world.entities.get_mut(id) {
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
                EngineCommand::SetMobileActive {
                    mobile_index,
                    active,
                } => {
                    let mobile = self
                        .world
                        .mobile_elements
                        .get_mut(usize::from(mobile_index))
                        .unwrap_or_else(|| {
                            panic!("SetMobileActive references missing mobile {mobile_index}")
                        });
                    mobile.set_active(active);
                    let sprite_ids = mobile.sprite_ids.clone();
                    for sprite_id in sprite_ids {
                        let fx = self
                            .world
                            .entities
                            .get_mut(sprite_id)
                            .and_then(crate::element::Entity::as_fx_mut)
                            .unwrap_or_else(|| {
                                panic!(
                                    "mobile {mobile_index} child {sprite_id} is missing or non-FX"
                                )
                            });
                        fx.element.active = active;
                    }
                }
                EngineCommand::MarkPc { actor_handle } => {
                    // Resolve the script handle to an EntityId and route
                    // it to the host via pending_side_effects.  The sim
                    // can't draw, so it hands the ID off to the host's
                    // outline pass, which flashes the outline for one
                    // frame.
                    if let Some(eid) = self.entity_id_for_actor_handle(actor_handle) {
                        if matches!(self.get_entity(eid), Some(crate::element::Entity::Pc(_))) {
                            self.feedback
                                .pending_side_effects
                                .pending_mark_pc_ids
                                .push(eid);
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
                    if let Some(campaign) = Some(&self.mission_domain.campaign) {
                        // `Game::is_men_to_blazon_conversion` is reflected in
                        // the engine-owned mission UI domain by the
                        // `SetMenToBlazonConversionMode` player command.
                        // Read that state here so the blazon bar can
                        // switch to next-mission targeting during
                        // conversion mode without needing a `&Game`
                        // borrow at the engine tick.
                        let men_to_blazon =
                            self.script_domains.mission_ui.men_to_blazon_conversion_mode;
                        let blinking = self
                            .script_domains
                            .mission_ui
                            .active_blinking_blazons(self.control.frame_counter);
                        let bb = crate::widget_state::blazon_bar::build_blazon_bar_state(
                            campaign,
                            &assets.profile_manager,
                            men_to_blazon,
                            blinking,
                        );
                        let mission_team: Vec<crate::profiles::CharacterProfileIdx> =
                            campaign.mission_team_profile_indices();
                        let selected: Vec<crate::profiles::CharacterProfileIdx> =
                            self.players.seats[0]
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
        let Some(bld_idx) = crate::natives::ScriptHandleCodec::building_index(building) else {
            tracing::warn!("PutActorInBuilding: invalid building handle {building}");
            return;
        };

        // Look up the first gate's `point_in` and the building's sector
        // number. Sector number comes from the grid sector tagged
        // `building_index == bld_idx` (populated at level load).
        let (gate_point_in, sector_num) = {
            let Some(ref script) = self.scripts.mission else {
                return;
            };
            let Some(_game_host) = script.game_host() else {
                return;
            };
            let gate_handle = self
                .script_domains
                .buildings
                .gates
                .get(bld_idx)
                .and_then(|g| g.first())
                .copied();
            let point_in = gate_handle
                .and_then(crate::natives::ScriptHandleCodec::door_index)
                .and_then(|di| self.script_domains.interactables.doors.get(di))
                .map(|d| d.point_in);
            let sn = self.world.fast_grid.level.sectors.iter().find_map(|gs| {
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

        let special_layer = self.world.fast_grid.level.special_layer;

        let is_pc;
        let carried_handle: Option<i32>;
        if let Some(entity) = self.world.entities.get_mut(actor_id) {
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
                .map(crate::natives::ScriptHandleCodec::actor_handle);
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
            self.orders.messenger.send(Message::pc(
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
                    && let Some(carried_entity) = self.world.entities.get_mut(carried_id)
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
                if bld_idx >= self.script_domains.buildings.occupants.len() {
                    self.script_domains
                        .buildings
                        .occupants
                        .resize(bld_idx + 1, Vec::new());
                }
                self.script_domains.buildings.occupants[bld_idx].push(carried_h);
                self.script_domains
                    .buildings
                    .actor_building
                    .insert(carried_h, building);
            }

            // Re-enable corpses already inside the building: walk the
            // occupants list and SetActive(true) on humans that are
            // (is_dead || unconscious) && carrier.is_none().
            let occupants: Vec<i32> = self
                .script_domains
                .buildings
                .occupants
                .get(bld_idx)
                .cloned()
                .unwrap_or_default();
            for occ_h in occupants {
                let Some(occ_id) = self.entity_id_for_actor_handle(occ_h) else {
                    continue;
                };
                let Some(occ) = self.world.entities.get_mut(occ_id) else {
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
    fn mission_script_v6_snapshot_round_trips_state_and_reattaches_program() {
        let mut script = empty_mission_script();
        script.state.globals.insert(7, 91);
        script
            .state
            .computed_locations
            .push(crate::natives::ComputedScriptLocation {
                position: (12.5, -8.0),
                layer_sector: Some((2, 44)),
            });
        script.state.sequence_recorder.sequence_id = 3;
        script.state.sequence_recorder.recording = Some(crate::sequence::RecordingSession::new());
        let location_positions = std::sync::Arc::new(vec![(12.0, 34.0)]);
        script.attach_bindings(crate::natives::AttachedScriptBindings {
            script_location_count: 1,
            location_positions: location_positions.clone(),
            ..Default::default()
        });

        let hash_before = robin_util::state_hash::compute(&script);
        let program = script.manager.program.clone();
        let json = serde_json::to_string(&script).expect("serialize v6 MissionScript");
        let value: serde_json::Value = serde_json::from_str(&json).expect("parse snapshot JSON");
        assert_eq!(value["snapshot_version"], 6);
        assert!(value["game_host"].get("campaign").is_none());
        assert!(value["game_host"].get("mission_stat").is_none());
        assert!(value["game_host"].get("engine_domains").is_none());
        assert!(value["game_host"].get("script_this").is_none());
        assert!(value["game_host"].get("current_scroll").is_none());
        assert!(value["game_host"].get("nested_call_depth").is_none());
        assert!(value["game_host"].get("globals").is_none());
        assert!(value["game_host"].get("computed_locations").is_none());
        assert!(value["game_host"].get("recording").is_none());
        assert!(value["game_host"].get("entities").is_none());
        assert!(value["game_host"].get("ai_global").is_none());
        assert!(value["game_host"].get("fast_grid").is_none());
        assert!(value["game_host"].get("background_invalidated").is_none());
        assert!(value.get("bindings").is_none());

        let mut decoded: MissionScript =
            serde_json::from_str(&json).expect("deserialize v6 MissionScript");
        assert_eq!(decoded.bindings.script_location_count, 0);
        decoded.attach_program(program);
        decoded.attach_bindings(crate::natives::AttachedScriptBindings {
            script_location_count: 1,
            location_positions: location_positions.clone(),
            ..Default::default()
        });
        assert!(std::sync::Arc::ptr_eq(
            &decoded.bindings.location_positions,
            &location_positions
        ));
        assert_eq!(decoded.state.globals.get(&7), Some(&91));
        assert_eq!(decoded.state.computed_locations.len(), 1);
        assert!(decoded.state.sequence_recorder.recording.is_some());
        assert_eq!(robin_util::state_hash::compute(&decoded), hash_before);
    }

    #[test]
    fn active_call_frame_rejects_snapshot_and_unwinds() {
        let mut script = empty_mission_script();
        let hash_before = robin_util::state_hash::compute(&script);
        let error = script.with_call_frame(
            crate::natives::ScriptCallFrame::scroll(17).with_script_this(41),
            |script| {
                assert_eq!(robin_util::state_hash::compute(script), hash_before);
                serde_json::to_string(script).expect_err("active callback is not snapshot-safe")
            },
        );

        assert!(error.to_string().contains("active script callback"));
        assert_eq!(script.active_call_frame_count(), 0);
    }

    #[test]
    fn legacy_game_host_background_invalidation_is_accepted_then_omitted() {
        let script = empty_mission_script();
        let mut snapshot = serde_json::to_value(script).expect("serialize current MissionScript");
        snapshot["game_host"]["background_invalidated"] = serde_json::json!(true);

        let restored: MissionScript =
            serde_json::from_value(snapshot).expect("decode legacy background invalidation flag");
        let normalized =
            serde_json::to_value(restored).expect("serialize normalized MissionScript");

        assert!(
            normalized["game_host"]
                .get("background_invalidated")
                .is_none()
        );
    }

    #[test]
    fn patch_background_effects_invalidate_canonical_side_effects_immediately() {
        let mut engine = EngineInner::new();
        engine.scripts.mission = Some(empty_mission_script());
        engine
            .script_domains
            .interactables
            .patches
            .push(crate::patch::Patch {
                integrate_in_background: true,
                ..Default::default()
            });
        let patch_index = crate::patch::PatchIndex::new(0).expect("zero is a valid patch index");

        engine.process_patch_effects(
            &LevelAssets::default(),
            patch_index,
            vec![crate::patch::PatchEffect::SwapBackground { applied: true }],
        );
        assert!(engine.feedback.pending_side_effects.invalidate_background);

        engine.feedback.pending_side_effects.invalidate_background = false;
        engine.process_patch_effects(
            &LevelAssets::default(),
            patch_index,
            vec![crate::patch::PatchEffect::RestoreBackground],
        );
        assert!(engine.feedback.pending_side_effects.invalidate_background);
    }

    #[test]
    fn v4_callframe_branch_parked_transient_game_host_values_are_ignored() {
        let script = empty_mission_script();
        let mut snapshot = serde_json::to_value(&script).expect("serialize current snapshot");
        snapshot["snapshot_version"] = serde_json::json!(4);
        snapshot["game_host"]["script_this"] = serde_json::json!(41);
        snapshot["game_host"]["current_scroll"] = serde_json::json!(17);
        snapshot["game_host"]["nested_call_depth"] = serde_json::json!(3);

        let decoded: MissionScript =
            serde_json::from_value(snapshot).expect("legacy parked callback values are accepted");
        let normalized = serde_json::to_value(decoded).expect("serialize normalized snapshot");
        assert!(normalized["game_host"].get("script_this").is_none());
        assert!(normalized["game_host"].get("current_scroll").is_none());
        assert!(normalized["game_host"].get("nested_call_depth").is_none());
    }

    #[test]
    fn v5_parked_ai_global_deserializes_only_through_legacy_dto() {
        let script = empty_mission_script();
        let mut snapshot = serde_json::to_value(&script).expect("serialize current snapshot");
        snapshot["snapshot_version"] = serde_json::json!(5);
        let mut parked = crate::ai::AiGlobalState::default();
        parked.golden_eye_mode = true;
        parked.next_repulsive_point_id = 77;
        snapshot["game_host"]["ai_global"] =
            serde_json::to_value(parked).expect("serialize legacy parked AI mirror");

        let decoded: MissionScript =
            serde_json::from_value(snapshot).expect("legacy parked AI mirror remains loadable");
        let normalized = serde_json::to_value(decoded).expect("serialize normalized snapshot");
        assert_eq!(normalized["snapshot_version"], 6);
        assert!(normalized["game_host"].get("ai_global").is_none());
    }

    #[test]
    fn legacy_game_host_script_state_normalizes_once() {
        let mut script = empty_mission_script();
        script.state.globals.insert(4, 55);
        script
            .state
            .computed_locations
            .push(crate::natives::ComputedScriptLocation {
                position: (1.25, 9.5),
                layer_sector: Some((3, 12)),
            });
        script.state.sequence_recorder.sequence_id = 2;

        let mut snapshot = serde_json::to_value(&script).expect("serialize current snapshot");
        let root = snapshot.as_object_mut().expect("MissionScript JSON object");
        root.remove("snapshot_version");
        let state = root.remove("state").expect("current ScriptState field");
        let state = state.as_object().expect("ScriptState object");
        let computed = state["computed_locations"]
            .as_array()
            .expect("computed location array");
        let positions = computed
            .iter()
            .map(|location| location["position"].clone())
            .collect();
        let layers = computed
            .iter()
            .map(|location| location["layer_sector"].clone())
            .collect();
        let recorder = state["sequence_recorder"]
            .as_object()
            .expect("sequence recorder object");
        let host = root["game_host"].as_object_mut().expect("GameHost object");
        host.insert("globals".into(), state["globals"].clone());
        host.insert(
            "computed_locations".into(),
            serde_json::Value::Array(positions),
        );
        host.insert(
            "computed_location_layers".into(),
            serde_json::Value::Array(layers),
        );
        host.insert("recording".into(), recorder["recording"].clone());
        host.insert("sequence_id".into(), recorder["sequence_id"].clone());

        let decoded: MissionScript =
            serde_json::from_value(snapshot).expect("normalize legacy MissionScript");
        assert_eq!(decoded.state.globals.get(&4), Some(&55));
        assert_eq!(decoded.state.computed_locations[0].position, (1.25, 9.5));
        assert_eq!(
            decoded.state.computed_locations[0].layer_sector,
            Some((3, 12))
        );
        assert_eq!(decoded.state.sequence_recorder.sequence_id, 2);
    }

    #[test]
    fn contradictory_new_and_legacy_script_state_is_rejected() {
        let script = empty_mission_script();
        let mut snapshot = serde_json::to_value(&script).expect("serialize current snapshot");
        let root = snapshot.as_object_mut().expect("MissionScript JSON object");
        let host = root["game_host"].as_object_mut().expect("GameHost object");
        host.insert("globals".into(), serde_json::json!({"9": 1}));
        host.insert("computed_locations".into(), serde_json::json!([]));
        host.insert("computed_location_layers".into(), serde_json::json!([]));
        host.insert("recording".into(), serde_json::Value::Null);
        host.insert("sequence_id".into(), serde_json::json!(0));

        let error = serde_json::from_value::<MissionScript>(snapshot)
            .expect_err("contradictory ScriptState must fail");
        assert!(error.to_string().contains("contradictory"), "{error}");
    }

    #[test]
    fn legacy_custom_campaign_value_migrates_once() {
        let mut engine = EngineInner::new();
        engine.mission_domain.campaign = Some(crate::campaign::Campaign::default());
        let mut script = empty_mission_script();
        script.legacy_custom_values = Some(crate::engine::types::LegacyScriptCustomValues {
            parked_campaign: None,
            campaign: std::collections::BTreeMap::from([(7, 42)]),
            npc: std::collections::BTreeMap::new(),
        });
        engine.scripts.mission = Some(script);

        engine.migrate_legacy_script_custom_values();

        let slot = CampaignValue::custom(7).unwrap();
        assert_eq!(engine.campaign().unwrap().values[slot], 42);
        assert!(
            engine
                .scripts
                .mission
                .as_ref()
                .unwrap()
                .legacy_custom_values
                .is_none()
        );
        engine.migrate_legacy_script_custom_values();
        assert_eq!(engine.campaign().unwrap().values[slot], 42);
    }

    #[test]
    fn v2_game_host_custom_values_are_preserved_for_migration() {
        let script = empty_mission_script();
        let mut snapshot = serde_json::to_value(&script).expect("serialize current snapshot");
        snapshot["snapshot_version"] = serde_json::json!(2);
        snapshot["game_host"]["campaign_values"] = serde_json::json!({"7": 42});

        let decoded: MissionScript =
            serde_json::from_value(snapshot).expect("deserialize v2 custom values");
        let legacy = decoded
            .legacy_custom_values
            .expect("v2 custom values must survive until engine attachment");
        assert_eq!(legacy.campaign.get(&7), Some(&42));
    }

    #[test]
    fn v4_campaign_branch_parked_campaign_migrates_to_engine_owner() {
        let script = empty_mission_script();
        let mut parked = crate::campaign::Campaign::default();
        parked.values[CampaignValue::Custom20] = 0x8b_20_26;
        let mut snapshot = serde_json::to_value(&script).expect("serialize current snapshot");
        snapshot["snapshot_version"] = serde_json::json!(4);
        snapshot["game_host"]["campaign"] =
            serde_json::to_value(&parked).expect("serialize legacy parked campaign");

        let decoded: MissionScript =
            serde_json::from_value(snapshot).expect("decode v4 parked campaign");
        let mut engine = EngineInner::new();
        assert!(engine.mission_domain.campaign.is_none());
        engine.scripts.mission = Some(decoded);

        engine.migrate_legacy_script_custom_values();

        assert_eq!(
            engine
                .campaign()
                .expect("legacy campaign becomes canonical")
                .values[CampaignValue::Custom20],
            0x8b_20_26
        );
    }

    #[test]
    fn legacy_engine_save_moves_game_host_campaign_to_canonical_owner() {
        let mut engine = EngineInner::new();
        let mut campaign = crate::campaign::Campaign::default();
        campaign.values[CampaignValue::Custom19] = 0x19_08_25;
        engine.mission_domain.campaign = Some(campaign);
        engine.scripts.mission = Some(empty_mission_script());

        let mut snapshot = serde_json::to_value(&engine).expect("serialize current engine");
        snapshot["mission_script"]["snapshot_version"] = serde_json::json!(3);
        let parked_campaign = std::mem::take(&mut snapshot["campaign"]);
        snapshot["mission_script"]["game_host"]["campaign"] = parked_campaign;
        snapshot["mission_script"]["game_host"]["mission_stat"] =
            serde_json::to_value(crate::mission_stat::MissionStat::default())
                .expect("serialize legacy parked mission stats");

        let mut restored: EngineInner =
            serde_json::from_value(snapshot).expect("decode legacy engine save");
        assert!(restored.mission_domain.campaign.is_none());
        restored.migrate_legacy_script_custom_values();

        assert_eq!(
            restored
                .campaign()
                .expect("parked campaign migrated")
                .values[CampaignValue::Custom19],
            0x19_08_25
        );
    }

    #[test]
    fn v5_game_host_native_owners_move_to_empty_canonical_engine_owners() {
        let mut engine = EngineInner::new();
        engine.world.entities.push(None);
        engine.ai.global.golden_eye_mode = true;
        engine.world.fast_grid.sector_active.push(false);
        engine.scripts.mission = Some(empty_mission_script());

        let mut snapshot = serde_json::to_value(&engine).expect("serialize current engine");
        snapshot["mission_script"]["snapshot_version"] = serde_json::json!(5);
        let parked_entities = std::mem::take(&mut snapshot["entities"]);
        let parked_ai = std::mem::replace(
            &mut snapshot["ai_global"],
            serde_json::to_value(crate::ai::AiGlobalState::default()).unwrap(),
        );
        let parked_grid = std::mem::replace(
            &mut snapshot["fast_grid"],
            serde_json::to_value(crate::fast_find_grid::FastFindGrid::default()).unwrap(),
        );
        snapshot["entities"] = serde_json::json!([]);
        snapshot["mission_script"]["game_host"]["entities"] = parked_entities;
        snapshot["mission_script"]["game_host"]["ai_global"] = parked_ai;
        snapshot["mission_script"]["game_host"]["fast_grid"] = parked_grid;

        let restored: EngineInner =
            serde_json::from_value(snapshot).expect("decode v5 parked native owners");
        assert_eq!(restored.world.entities.len(), 1);
        assert!(restored.ai.global.golden_eye_mode);
        assert_eq!(restored.world.fast_grid.sector_active, [false]);
        let host = &restored
            .scripts
            .mission
            .as_ref()
            .expect("mission script survives migration")
            .game_host;
        let host = serde_json::to_value(host).expect("serialize normalized GameHost");
        assert!(host.get("entities").is_none());
        assert!(host.get("ai_global").is_none());
        assert!(host.get("fast_grid").is_none());
    }

    #[test]
    fn contradictory_legacy_game_host_entities_are_rejected() {
        let mut engine = EngineInner::new();
        engine.world.entities.push(None);
        engine.scripts.mission = Some(empty_mission_script());

        let mut snapshot = serde_json::to_value(&engine).expect("serialize current engine");
        snapshot["mission_script"]["snapshot_version"] = serde_json::json!(4);
        snapshot["mission_script"]["game_host"]["entities"] = serde_json::json!([null, null]);

        let error = match serde_json::from_value::<EngineInner>(snapshot) {
            Ok(_) => panic!("contradictory entity owners must be rejected"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("contradictory"), "{error}");
    }

    #[test]
    #[should_panic(expected = "legacy GameHost campaign contradicts canonical engine campaign")]
    fn contradictory_legacy_game_host_campaign_is_rejected() {
        let mut canonical = crate::campaign::Campaign::default();
        canonical.values[CampaignValue::Custom20] = 1;
        let mut parked = canonical.clone();
        parked.values[CampaignValue::Custom20] = 2;

        let mut engine = EngineInner::new();
        engine.mission_domain.campaign = Some(canonical);
        let mut script = empty_mission_script();
        script.legacy_custom_values = Some(crate::engine::types::LegacyScriptCustomValues {
            parked_campaign: Some(parked),
            campaign: std::collections::BTreeMap::new(),
            npc: std::collections::BTreeMap::new(),
        });
        engine.scripts.mission = Some(script);

        engine.migrate_legacy_script_custom_values();
    }

    #[test]
    #[should_panic(expected = "active mission entered a script call without its campaign")]
    fn script_session_does_not_default_missing_required_campaign() {
        let mut engine = EngineInner::new();
        engine.scripts.mission = Some(empty_mission_script());
        let assets = LevelAssets::new();
        engine.attach_script_bindings(&assets);

        let _ = engine.with_script_session(&assets, |_script, _, _capabilities| ());
    }

    #[test]
    fn legacy_game_host_interactables_migrate_to_engine_domains_once() {
        let mut engine = EngineInner::new();
        engine.scripts.mission = Some(empty_mission_script());
        engine
            .script_domains
            .interactables
            .doors
            .push(crate::gate::Door {
                locked_pc: true,
                ..Default::default()
            });
        engine
            .script_domains
            .interactables
            .patches
            .push(crate::patch::Patch {
                active: true,
                applied: true,
                initially_active: true,
                ..Default::default()
            });

        let mut snapshot = serde_json::to_value(&engine).expect("serialize current engine");
        let interactables = snapshot["script_domains"]["interactables"]
            .as_object()
            .cloned()
            .expect("current interactable domain");
        snapshot["mission_script"]["game_host"]["doors"] = interactables["doors"].clone();
        snapshot["mission_script"]["game_host"]["patches"] = interactables["patches"].clone();
        snapshot["script_domains"]["interactables"]["doors"] = serde_json::json!([]);
        snapshot["script_domains"]["interactables"]["patches"] = serde_json::json!([]);

        let restored: EngineInner =
            serde_json::from_value(snapshot).expect("normalize legacy interactables");
        assert_eq!(restored.script_domains.interactables.doors.len(), 1);
        assert!(restored.script_domains.interactables.doors[0].locked_pc);
        assert_eq!(restored.script_domains.interactables.patches.len(), 1);
        assert!(restored.script_domains.interactables.patches[0].applied);
        let game_host = &restored
            .scripts
            .mission
            .as_ref()
            .expect("mission script survives migration")
            .game_host;
        assert!(
            serde_json::to_value(game_host)
                .expect("serialize migrated GameHost")
                .get("engine_domains")
                .is_none(),
            "legacy storage must be consumed instead of retained as a mirror"
        );
    }

    #[test]
    fn legacy_game_host_mission_ui_migrates_without_overwriting_force_check() {
        let mut engine = EngineInner::new();
        engine.scripts.mission = Some(empty_mission_script());
        engine.script_domains.mission_ui.outline_display = true;
        engine.script_domains.mission_ui.force_check = true;
        engine
            .script_domains
            .mission_ui
            .men_to_blazon_conversion_mode = true;
        engine
            .script_domains
            .mission_ui
            .set_blinking_blazons(3, 100);

        let mut snapshot = serde_json::to_value(&engine).expect("serialize current engine");
        let ui = snapshot["script_domains"]["mission_ui"]
            .as_object()
            .cloned()
            .expect("current mission UI domain");
        let host = snapshot["mission_script"]["game_host"]
            .as_object_mut()
            .expect("legacy GameHost object");
        host.insert("force_check".into(), serde_json::json!(false));
        for field in [
            "outline_display",
            "men_to_blazon_conversion_mode",
            "blinking_blazons",
            "blink_expire_frame",
        ] {
            host.insert(field.into(), ui[field].clone());
        }
        snapshot["script_domains"]["mission_ui"] =
            serde_json::to_value(crate::engine::state::MissionUiState::default())
                .expect("serialize default mission UI");

        let restored: EngineInner =
            serde_json::from_value(snapshot).expect("normalize legacy mission UI");
        let ui = &restored.script_domains.mission_ui;
        assert!(ui.outline_display);
        assert!(
            ui.force_check,
            "top-level legacy force_check remains authoritative"
        );
        assert!(ui.men_to_blazon_conversion_mode);
        assert_eq!(ui.blinking_blazons, 3);
        assert_eq!(ui.blink_expire_frame, 150);
        let game_host = &restored
            .scripts
            .mission
            .as_ref()
            .expect("mission script survives migration")
            .game_host;
        assert!(
            serde_json::to_value(game_host)
                .expect("serialize migrated GameHost")
                .get("engine_domains")
                .is_none(),
            "legacy UI storage must be consumed instead of retained as a mirror"
        );
    }

    #[test]
    #[should_panic(expected = "contradicts canonical value")]
    fn contradictory_legacy_custom_campaign_value_is_rejected() {
        let mut engine = EngineInner::new();
        let slot = CampaignValue::custom(7).unwrap();
        let mut campaign = crate::campaign::Campaign::default();
        campaign.values[slot] = 41;
        engine.mission_domain.campaign = Some(campaign);
        let mut script = empty_mission_script();
        script.legacy_custom_values = Some(crate::engine::types::LegacyScriptCustomValues {
            parked_campaign: None,
            campaign: std::collections::BTreeMap::from([(7, 42)]),
            npc: std::collections::BTreeMap::new(),
        });
        engine.scripts.mission = Some(script);

        engine.migrate_legacy_script_custom_values();
    }

    #[test]
    fn external_this_actor_success_keeps_canonical_entity_ownership() {
        let mut engine = EngineInner::new();
        engine.mission_domain.campaign = Some(crate::campaign::Campaign::default());
        engine.world.entities.push(None);
        engine.scripts.mission = Some(empty_mission_script());
        engine.attach_script_bindings(&LevelAssets::new());

        let result =
            engine.call_external_native_with_this(&LevelAssets::new(), "ThisActor", &[], Some(99));

        assert_eq!(result, Ok(99));
        assert_eq!(engine.world.entities.len(), 1);
        let script = engine
            .scripts
            .mission
            .as_ref()
            .expect("script remains installed");
        assert_eq!(script.active_call_frame_count(), 0);
    }

    #[test]
    fn native_mutation_writes_the_canonical_script_domains_in_place() {
        use crate::interp::{HostFunctions, NativeStack};
        use crate::natives::NativeFn;

        let mut engine = EngineInner::new();
        engine.mission_domain.campaign = Some(crate::campaign::Campaign::default());
        engine.scripts.mission = Some(empty_mission_script());
        engine
            .script_domains
            .interactables
            .doors
            .push(crate::gate::Door {
                locked_pc: true,
                ..Default::default()
            });
        let assets = LevelAssets::new();
        engine.attach_script_bindings(&assets);
        let canonical_domains = std::ptr::addr_of_mut!(engine.script_domains);
        let canonical_entities = std::ptr::from_ref(&engine.world.entities);
        let door = crate::natives::ScriptHandleCodec::door_handle_from_index(0);

        let result = engine.with_script_session(&assets, |script, script_domains, capabilities| {
            assert_eq!(
                std::ptr::from_mut(script_domains),
                canonical_domains,
                "the native capability must borrow EngineInner's allocation"
            );
            assert_eq!(
                capabilities.entities_owner_ptr(),
                canonical_entities,
                "the entity capability must borrow EngineInner's canonical allocation"
            );
            let mut stack = NativeStack::default();
            stack.push_i32(door);
            stack.push_i32(0);
            let mut context = crate::natives::NativeContext::with_bindings(
                &mut script.game_host,
                &mut script.state,
                script_domains,
                &script.bindings,
                capabilities,
            );
            HostFunctions::call(&mut context, NativeFn::SetDoorLockedPC as u32, &mut stack)
                .expect_return("SetDoorLockedPC is synchronous")
        });

        assert_eq!(result, Some(0));
        assert!(!engine.script_domains.interactables.doors[0].locked_pc);
    }

    #[test]
    fn native_ai_mutation_writes_engine_inner_directly() {
        use crate::interp::{HostFunctions, NativeStack};
        use crate::natives::NativeFn;

        let mut engine = EngineInner::new();
        engine.mission_domain.campaign = Some(crate::campaign::Campaign::default());
        engine.scripts.mission = Some(empty_mission_script());
        engine.ai.global.next_repulsive_point_id = 9;
        engine
            .ai
            .global
            .repulsive_points
            .push(crate::ai::RepulsivePoint {
                id: 8,
                position: crate::ai::Position::default(),
                radius: 10.0,
                action_radius: 20.0,
                flags: 0,
            });
        let assets = LevelAssets::new();
        engine.attach_script_bindings(&assets);
        let canonical_ai_global = std::ptr::addr_of_mut!(engine.ai.global);

        let result = engine.with_script_session(&assets, |script, script_domains, queries| {
            let mut context = crate::natives::NativeContext::with_bindings(
                &mut script.game_host,
                &mut script.state,
                script_domains,
                &script.bindings,
                queries,
            );
            assert_eq!(
                std::ptr::from_mut(context.ai_global_mut()),
                canonical_ai_global,
                "the native capability must borrow EngineInner's AI allocation"
            );
            let mut stack = NativeStack::default();
            stack.push_i32(8);
            HostFunctions::call(
                &mut context,
                NativeFn::DeleteRepulsivePoint as u32,
                &mut stack,
            )
            .expect_return("DeleteRepulsivePoint is synchronous")
        });

        assert_eq!(result, Some(0));
        assert!(engine.ai.global.repulsive_points.is_empty());
        assert_eq!(engine.ai.global.next_repulsive_point_id, 9);
    }

    #[test]
    #[should_panic(expected = "native dispatch requires live level attachments")]
    fn external_native_rejects_a_detached_live_script() {
        let mut engine = EngineInner::new();
        engine.scripts.mission = Some(empty_mission_script());

        let _ =
            engine.call_external_native_with_this(&LevelAssets::new(), "ThisActor", &[], Some(99));
    }

    #[test]
    fn script_session_normal_return_restores_state_and_hash() {
        let mut engine = EngineInner::new();
        engine.mission_domain.campaign = Some(crate::campaign::Campaign::default());
        engine.world.entities.push(None);
        engine.scripts.mission = Some(empty_mission_script());
        let assets = LevelAssets::new();
        engine.attach_script_bindings(&assets);
        let hash_before = robin_util::state_hash::compute(&engine);
        let canonical_entities = std::ptr::from_ref(&engine.world.entities);

        let result = engine.with_script_session(&assets, |script, _, capabilities| {
            assert_eq!(capabilities.entities_owner_ptr(), canonical_entities);
            script.with_call_frame(crate::natives::ScriptCallFrame::actor(99), |script| {
                assert_eq!(script.active_call_frame_count(), 1);
                73
            })
        });

        assert_eq!(result, Some(73));
        assert_eq!(engine.world.entities.len(), 1);
        let script = engine.scripts.mission.as_ref().unwrap();
        assert_eq!(script.active_call_frame_count(), 0);
        assert_eq!(robin_util::state_hash::compute(&engine), hash_before);
    }

    #[test]
    fn script_callback_error_keeps_canonical_owners_in_place() {
        let mut engine = EngineInner::new();
        engine.mission_domain.campaign = Some(crate::campaign::Campaign::default());
        engine.world.entities.push(None);
        engine.scripts.mission = Some(empty_mission_script());
        let assets = LevelAssets::new();
        engine.attach_script_bindings(&assets);

        let result: Result<(), &'static str> = engine
            .with_script_session(&assets, |script, _, _capabilities| {
                script.with_call_frame(crate::natives::ScriptCallFrame::actor(99), |_| {
                    Err("simulated script error")
                })
            })
            .unwrap();

        assert_eq!(result, Err("simulated script error"));
        assert_eq!(engine.world.entities.len(), 1);
        let script = engine.scripts.mission.as_ref().unwrap();
        assert_eq!(script.active_call_frame_count(), 0);
    }

    #[test]
    #[should_panic(expected = "simulated script panic")]
    fn script_callback_unwind_keeps_canonical_owners_in_place() {
        struct VerifyRestoredOnUnwind(*const EngineInner);

        impl Drop for VerifyRestoredOnUnwind {
            fn drop(&mut self) {
                // SAFETY: the pointer targets the engine local below, which
                // outlives this verifier. All callback capability borrows have
                // ended before unwinding reaches this Drop implementation.
                let engine = unsafe { &*self.0 };
                assert_eq!(engine.world.entities.len(), 1);
                let script = engine.scripts.mission.as_ref().unwrap();
                assert_eq!(script.active_call_frame_count(), 0);
                assert!(
                    engine.script_domains.mission_ui.outline_display,
                    "canonical domain mutation survives callback unwind"
                );
                assert!(
                    engine.ai.global.golden_eye_mode,
                    "canonical AI-global mutation survives callback unwind"
                );
            }
        }

        let mut engine = EngineInner::new();
        engine.mission_domain.campaign = Some(crate::campaign::Campaign::default());
        engine.world.entities.push(None);
        engine.scripts.mission = Some(empty_mission_script());
        let assets = LevelAssets::new();
        engine.attach_script_bindings(&assets);
        let _verify = VerifyRestoredOnUnwind(&engine);

        let _ = engine.with_script_session(&assets, |script, script_domains, capabilities| {
            script_domains.mission_ui.outline_display = true;
            {
                let mut context = crate::natives::NativeContext::with_bindings(
                    &mut script.game_host,
                    &mut script.state,
                    script_domains,
                    &script.bindings,
                    capabilities,
                );
                context.ai_global_mut().golden_eye_mode = true;
            }
            script.with_call_frame(
                crate::natives::ScriptCallFrame::scroll(100).with_script_this(99),
                |_| panic!("simulated script panic"),
            );
        });
    }

    #[test]
    fn external_native_early_returns_without_touching_callback_state() {
        let mut engine = EngineInner::new();
        engine.world.entities.push(None);
        engine.scripts.mission = Some(empty_mission_script());

        let result = engine.call_external_native_with_this(
            &LevelAssets::new(),
            "NotAnOriginalNative",
            &[],
            Some(99),
        );

        assert_eq!(result, Err("unknown native: NotAnOriginalNative".into()));
        assert_eq!(engine.world.entities.len(), 1);
        let script = engine
            .scripts
            .mission
            .as_ref()
            .expect("script remains installed");
        assert_eq!(script.active_call_frame_count(), 0);
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
/// used to drive via audio-backend playback completion.
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
    /// Goes through the same disjoint-owner boundary script callbacks use,
    /// so any side-effect commands the
    /// native queues (camera, dialog, sequence Start/Thanx, sound,
    /// deferred game-logic) are drained as if a script had made the
    /// call.
    ///
    /// `args` are pushed onto a fresh `NativeStack` in script-source
    /// order (i.e. `args[0]` is the first argument to the native, and
    /// will be popped *last* — matches the `Param`/`Pop` LIFO contract).
    ///
    /// When `this_actor` is `Some`, the standalone frame binds `ThisActor`
    /// for the duration of the call. Pass `None` for a receiver-free frame.
    pub fn call_external_native(
        &mut self,
        assets: &LevelAssets,
        native_name: &str,
        args: &[i32],
    ) -> Result<i32, String> {
        self.call_external_native_with_this(assets, native_name, args, None)
    }

    /// Like [`Self::call_external_native`], but with an explicit
    /// `ThisActor` receiver installed in the transient call frame.
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

        if self.scripts.mission.is_none() {
            return Err("no mission script loaded (no mission running)".into());
        }

        self.with_script_session(assets, |script, script_domains, capabilities| {
            let frame = this_actor.map_or_else(
                crate::natives::ScriptCallFrame::default,
                crate::natives::ScriptCallFrame::actor,
            );
            script.with_call_frame(frame, |script| {
                let mut stack = NativeStack::default();
                for &a in args {
                    stack.push_i32(a);
                }

                let outcome = {
                    let mut native_context = crate::natives::NativeContext::with_call_frame(
                        &mut script.game_host,
                        &mut script.state,
                        script_domains,
                        &script.bindings,
                        capabilities,
                        frame,
                    );
                    crate::interp::HostFunctions::call(&mut native_context, index, &mut stack)
                };

                match outcome {
                    crate::interp::NativeCallOutcome::Return(value) => Ok(value),
                    crate::interp::NativeCallOutcome::PendingNestedCall(call) => Err(format!(
                        "native {native_name} requires nested script dispatch and cannot be invoked through the standalone native adapter: {call:?}"
                    )),
                }
            })
        })
        .expect("mission-script presence checked above")
    }
}
