//! Existing tick phase implementations; scheduling remains in the parent tick spine.

use super::*;

impl EngineInner {
    /// Run mission gates, the once-per-second script, clock advancement, and
    /// the tick's messenger drain. Returning a code short-circuits every later
    /// phase exactly where the monolithic implementation did.
    ///
    /// The original game performs
    /// mission/UI gates, script callbacks, counter advancement, lock checks,
    /// loss checks, and reinforcement notification in this order.
    pub(super) fn hourglass_phase_mission_and_messages(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut CameraDisplayState,
        assets: &LevelAssets,
        pc_guarded: bool,
        simulation_body_allowed: bool,
    ) -> Option<GameCode> {
        // ── Anti-chorus timer ────────────────────────────────────
        if self.control.chorus_timer > 0 {
            self.control.chorus_timer -= 1;
        }

        // ── First-time mission-won message ───────────────────────
        // Fire the mission-state banner ("leave mission now" / quit
        // mission popup) and disable the quit-mission widget once the
        // player has reached a guarded exit AND no PC is currently
        // being guarded (guarded PCs can't lead everyone out yet).
        // We signal both via `SideEffects.pending_mission_state_notice`;
        // the host flips the widget-enable flag and shows the popup.
        if self.mission_domain.state.mission_won_first_time && !pc_guarded {
            self.mission_domain.state.mission_won_first_time = false;
            self.feedback
                .pending_side_effects
                .pending_mission_state_notice = true;
        }

        // ── Check quit conditions ────────────────────────────────
        // Each of the three quit branches displays the full minimap.
        if self.mission_domain.state.quit_won {
            self.feedback
                .pending_side_effects
                .host_events
                .push(HostEvent::Minimap(MinimapHostEvent::DisplayMap {
                    show: false,
                    restore_position: true,
                }));
            self.finalize_mission_script(sim, assets, false);
            return Some(GameCode::LevelSucceeded);
        }
        if self.mission_domain.state.quit_lost {
            self.feedback
                .pending_side_effects
                .host_events
                .push(HostEvent::Minimap(MinimapHostEvent::DisplayMap {
                    show: false,
                    restore_position: true,
                }));
            self.quit_mission();
            return Some(GameCode::LevelFailed);
        }
        if self.mission_domain.state.quit_interrupted {
            self.feedback
                .pending_side_effects
                .host_events
                .push(HostEvent::Minimap(MinimapHostEvent::DisplayMap {
                    show: false,
                    restore_position: true,
                }));
            self.finalize_mission_script(sim, assets, true);
            return Some(GameCode::LevelInterrupted);
        }

        // ── Cheat display all dialogs/briefings ──────────────────
        // After the engine/host carve-out (Decision 9) level descriptors
        // live host-side.  `all_dialogues`, `all_popup_texts` and
        // `all_debriefings` are expanded by `game_session` after the
        // tick returns — it has the descriptor on hand and pushes every
        // registered ID straight onto the host-side pending queues.

        // ── Script tick (once per game-second) ──────────────────────
        // The main loop runs at 25 Hz (40 ms frame time), and the
        // script's Hourglass fires only when
        // `frame_counter % 25 == 0` — i.e. once per real second — with
        // the game-second index as its argument.
        if self.control.sim_config.script_enabled
            && self.control.frame_counter.is_multiple_of(FRAMES_PER_SECOND)
        {
            let game_seconds = self.control.frame_counter / FRAMES_PER_SECOND;

            if let Err(error) = self.call_script_vm(
                sim,
                assets,
                crate::engine::ScriptVmKey::Global,
                "Hourglass",
                &[game_seconds as i32],
                crate::natives::ScriptCallFrame::default(),
            ) {
                tracing::warn!("Script Hourglass error: {error}");
            }

            // Check victory/defeat conditions every 3 game-seconds
            // (or immediately if force_check was set by a native call).
            if game_seconds.is_multiple_of(VICTORY_CHECK_INTERVAL)
                || self.script_domains.mission_ui.force_check
            {
                self.script_domains.mission_ui.force_check = false;

                {
                    let victory_result = self.call_script_vm(
                        sim,
                        assets,
                        crate::engine::ScriptVmKey::Global,
                        "CheckVictoryCondition",
                        &[game_seconds as i32],
                        crate::natives::ScriptCallFrame::default(),
                    );
                    match victory_result {
                        Ok(1) => {
                            // Mission won!
                            if !self.mission_domain.state.mission_won {
                                // Don't show the "leave mission" message for
                                // ambush or tactical missions (they end immediately).
                                let show_window = !matches!(
                                    self.mission_type(&assets.profile_manager),
                                    Some(MissionType::Ambush | MissionType::Tactical)
                                );
                                self.win(show_window);
                            }
                        }
                        Ok(2) => {
                            // Script says mission lost
                            self.quit_mission();
                            return Some(GameCode::LevelFailed);
                        }
                        Ok(_) => {} // 0 or other = still in progress
                        Err(e) => {
                            tracing::warn!("Script CheckVictoryCondition error: {e}");
                        }
                    }
                }
            }
        }

        let runtime_features_can_advance = !display.background_transform.zoom_to_up
            && !display.background_transform.zoom_to_down
            && !self.engine_locked()
            && simulation_body_allowed;
        if runtime_features_can_advance && self.tick_mission_runtime_features(assets) {
            tracing::info!("authored mission time limit expired");
            self.mission_domain.state.quit_lost = true;
            self.quit_mission();
            return Some(GameCode::LevelFailed);
        }

        // ── Increment the Original universal frame counter ───────
        self.advance_mission_clock();

        // ── Skip logic if engine is locked (zoom, sequence, etc) ─
        if !runtime_features_can_advance {
            return Some(GameCode::LevelInProgress);
        }

        // ── Default lose condition check ─────────────────────────
        // Guarded by `ignore_default_loose`.
        // Missions that keep-an-NPC-alive (e.g. "protect the cart")
        // set this flag to true so the default "all PCs dead/guarded /
        // dead-PC / civilian-killed" loss checks are skipped; the
        // script's `CheckVictoryCondition` is the authority instead.
        let ignore_default_loose = self.control.sim_config.ignore_default_loose;
        if !ignore_default_loose {
            // The original game checks the PC's explicit
            // playable flag and guard state. Death paths are responsible
            // for clearing playability; do not substitute an HP/posture test.
            // A custom all-combatant battle has no player party whose defeat
            // can end the mission. Once an ordinary player party is present,
            // however, the original game tests every PC's playable/guarded state,
            // including rescued PCs that became playable.
            let has_player_party = self.world.pc_ids.iter().any(|&pc_id| {
                matches!(
                    self.world.entities.get(pc_id),
                    Some(Entity::Pc(pc))
                        if pc.pc.mission_role == crate::human_control::MissionRole::PlayerParty
                )
            });
            if has_player_party {
                let any_playable_and_free = self.world.pc_ids.iter().any(|&pc_id| {
                    if let Some(Entity::Pc(pc)) = self.world.entities.get(pc_id) {
                        let guarded = pc.pc.guard.is_some();
                        pc.pc.playable && !guarded
                    } else {
                        false
                    }
                });
                if !any_playable_and_free {
                    tracing::info!("No playable, unguarded PC remains; mission lost");
                    self.quit_mission();
                    return Some(GameCode::LevelFailed);
                }
            }

            // Check if a dead PC was flagged for mission failure
            if let Some(dead_id) = self.mission_domain.dead_pc.take() {
                if let Some(entity) = self.get_entity(dead_id) {
                    let pos = entity.element_data().position_map();
                    self.center_on_point(0, pos);
                }
                self.quit_mission();
                return Some(GameCode::LevelFailed);
            }

            // Check if any civilian was killed (not by accident) → mission failure
            let mut killed_civilian = None;
            for (npc_id, civilian) in self.world.entities.civilians() {
                if civilian.element.posture().is_dead() {
                    let npc_id: EntityId = npc_id.into();
                    // Check killed_by_accident via the civilian's human data
                    let accident = civilian.human.killed_by_accident;
                    if !accident {
                        killed_civilian = Some(npc_id);
                        break;
                    }
                }
            }
            if let Some(civ_id) = killed_civilian {
                if let Some(entity) = self.get_entity(civ_id) {
                    let pos = entity.element_data().position_map();
                    self.center_on_point(0, pos);
                }
                self.quit_mission();
                return Some(GameCode::LevelFailed);
            }
        }

        // ── Send reinforcement messages ──────────────────────────
        //
        // For every PC, decrement `time_till_reinforcement` and, the
        // tick it hits zero, enqueue a reinforcement spawn directly
        // (skipping the messenger round-trip the original used).
        // `drain_pending_reinforcements` already handles the
        // `&mut LevelAssets` needed for sprite loading, and the
        // intermediate message was never observed by anything else.
        let pc_ids_for_reinf: Vec<EntityId> = self.world.pc_ids.clone();
        for pc_id in pc_ids_for_reinf {
            let Some(Entity::Pc(pc)) = self.get_entity_mut(pc_id) else {
                continue;
            };
            let arrived = match pc.pc.time_till_reinforcement {
                0xFFFF_FFFF => false,
                0 => {
                    pc.pc.time_till_reinforcement = 0xFFFF_FFFF;
                    true
                }
                ref mut t => {
                    *t -= 1;
                    false
                }
            };
            if arrived {
                self.orders.pending_reinforcements.push(Some(pc_id));
            }
        }

        // ── Process messenger (engine-state messages) ────────────
        // Handle pending messages that mutate engine state. Other
        // messages (UI/mission flow) are left in the queue for their
        // respective consumers (UI layer, tests, etc.) to observe.
        // We only consume the ones that actually affect engine state.
        {
            // Message forwarding is synchronous and recursive:
            // a message emitted while handling another message completes
            // before the outer call resumes.  Keep host/UI-only messages for
            // their downstream consumer, but prepend newly emitted messages
            // to the remaining engine work so their observable state changes
            // happen depth-first in this frame.
            let mut messages: std::collections::VecDeque<_> = self.orders.messenger.drain().into();
            let mut downstream = std::collections::VecDeque::new();
            while let Some(msg) = messages.pop_front() {
                match msg.msg_type {
                    MessageType::Simple(SimpleMessage::LockAlt) => {
                        self.players.seats[0].is_lock_alt = true;
                    }
                    MessageType::Simple(SimpleMessage::UnlockAlt) => {
                        self.players.seats[0].is_lock_alt = false;
                    }
                    // Macro recording state machine.  The PC id is
                    // passed via the message: a present id targets one
                    // specific PC; an absent id arms every currently-
                    // selected PC.
                    MessageType::Pc(crate::messenger::PcMessage::StartRecordingMacro, pc) => {
                        let slot = self.players.qa_recording_slot;
                        let targets: Vec<crate::element::EntityId> = match pc {
                            Some(id) => vec![id],
                            None => self.players.seats[0].selection.clone(),
                        };
                        for pc_id in &targets {
                            self.players
                                .macro_store
                                .get_or_insert(*pc_id)
                                .begin_recording(slot);
                            let pc = self
                                .get_entity_mut(*pc_id)
                                .and_then(|entity| entity.pc_data_mut())
                                .unwrap_or_else(|| {
                                    panic!("quick-action recording target {pc_id:?} is not a PC")
                                });
                            pc.portrait.quick_icons[slot as usize] = Default::default();
                        }
                        self.players.qa_recording_for = targets;
                        // Snapshot the currently-armed action so the
                        // MSG_STOP_RECORDING_MACRO post-process can
                        // restore it.
                        self.players.action_before_recording_macro = self.get_selected_action();
                    }
                    MessageType::Pc(crate::messenger::PcMessage::StopRecordingMacro, _) => {
                        // Suppress the post-process restore unless
                        // something was actually recording.
                        let was_recording = !self.players.qa_recording_for.is_empty();
                        self.stop_recording_macro();

                        // Post-process: re-select the action that was
                        // armed before recording started.  Apply the
                        // saved action to each selected PC directly —
                        // we do not route MSG_SELECT_ACTION through
                        // the messenger drain.
                        if was_recording {
                            let restore = self.players.action_before_recording_macro;
                            self.players.action_before_recording_macro =
                                crate::profiles::Action::NoAction;
                            self.players.seats[0].selected_action = restore;
                            for id in self.players.seats[0].selection.clone() {
                                if let Some(entity) = self.get_entity_mut(id)
                                    && let Some(pc) = entity.pc_data_mut()
                                {
                                    pc.current_action = restore;
                                }
                            }
                            // Emit the message for script /
                            // edge-subscriber observation.
                            self.orders
                                .messenger
                                .send(crate::messenger::Message::pc_with_value(
                                    crate::messenger::PcMessage::SelectAction,
                                    None,
                                    restore as u32,
                                ));
                        }
                    }
                    MessageType::Pc(crate::messenger::PcMessage::UpdateRecordingMacro, _) => {
                        // When a recording is live, end it on PCs no
                        // longer selected and start it on any newly-
                        // selected PC — keeping the slot index stable
                        // across selection changes.
                        if !self.players.qa_recording_for.is_empty() {
                            let slot = self.players.qa_recording_slot;
                            let selected: Vec<crate::element::EntityId> =
                                self.players.seats[0].selection.clone();
                            // End on PCs that left the selection.
                            let current = self.players.qa_recording_for.clone();
                            for pc_id in &current {
                                if !selected.contains(pc_id)
                                    && let Some(state) = self.players.macro_store.get_mut(*pc_id)
                                {
                                    state.stop_recording();
                                }
                            }
                            // Start on PCs newly selected.
                            for pc_id in &selected {
                                if !current.contains(pc_id) {
                                    self.players
                                        .macro_store
                                        .get_or_insert(*pc_id)
                                        .begin_recording(slot);
                                }
                            }
                            self.players.qa_recording_for = selected;
                        }
                    }
                    MessageType::Pc(crate::messenger::PcMessage::SendReinforcement, pc) => {
                        // `MSG_SEND_REINFORCEMENT` plays the "new peasant
                        // called" jingle and sets the PC's cooldown to
                        // 100 ticks.  The cooldown poll in the tick
                        // above spawns the replacement when the counter
                        // hits zero.
                        if let Some(pc_id) = pc
                            && let Some(Entity::Pc(pc)) = self.get_entity_mut(pc_id)
                        {
                            pc.pc.time_till_reinforcement = 100;
                        }
                        self.feedback.pending_side_effects.sounds.push(
                            crate::engine::SoundCommand::Jingle(
                                crate::sound::Jingle::NewPeasantCalled,
                            ),
                        );
                    }
                    // PC-info hover popup is HQ-only (Sherwood) — go
                    // through `request_pc_info_overlay` so that gate
                    // is honored.
                    //
                    // UI-has-focus: another UI widget grabbed input
                    // focus — hide any live PC-info hover popup.
                    // Emitted from the minimap drag handler
                    // (commands.rs) and should be emitted from any
                    // future in-game widget that grabs focus.
                    //
                    // The Rust port keeps the mouse focus gate on
                    // host-owned `InputState`; `run_engine_tick_core`
                    // consumes the side effect below and clears that
                    // latch before later mouse dispatch can see it.
                    MessageType::Simple(crate::messenger::SimpleMessage::UiHasFocus) => {
                        self.request_pc_info_overlay(assets, None);
                        // Raise the host-side per-frame `ui_focus`
                        // latch; the host clears it at end of
                        // `update_mouse`.
                        self.feedback.pending_side_effects.ui_has_focus = true;
                    }
                    MessageType::Pc(crate::messenger::PcMessage::ShowPcInformation, pc) => {
                        self.request_pc_info_overlay(assets, pc);
                    }
                    MessageType::Pc(crate::messenger::PcMessage::HidePcInformation, _) => {
                        self.request_pc_info_overlay(assets, None);
                    }
                    // The four `SelectCharacter[Add][WithEcho]` arms
                    // all route through `select_pc` with the
                    // appropriate (multi-select, speak) flags.
                    MessageType::Pc(crate::messenger::PcMessage::SelectCharacter, Some(pc_id)) => {
                        // Tick messenger drains: ambient single-seat
                        // semantics; LOCAL seat for now.
                        self.select_pc(assets, 0, pc_id, false, false);
                        self.emit_character_selection_followups();
                    }
                    MessageType::Pc(
                        crate::messenger::PcMessage::SelectCharacterWithEcho,
                        Some(pc_id),
                    ) => {
                        self.select_pc(assets, 0, pc_id, false, true);
                        self.emit_character_selection_followups();
                    }
                    MessageType::Pc(
                        crate::messenger::PcMessage::SelectAddCharacter,
                        Some(pc_id),
                    ) => {
                        self.select_pc(assets, 0, pc_id, true, false);
                        self.emit_character_selection_followups();
                    }
                    MessageType::Pc(
                        crate::messenger::PcMessage::SelectAddCharacterWithEcho,
                        Some(pc_id),
                    ) => {
                        self.select_pc(assets, 0, pc_id, true, true);
                        self.emit_character_selection_followups();
                    }
                    // `pc == None` drops the whole selection;
                    // otherwise remove the specific PC.  Producers:
                    // `tick.rs:L4279` (dying / KO'd PC), `LockUser`,
                    // `DisableCharacter` (below).
                    MessageType::Pc(crate::messenger::PcMessage::UnselectCharacter, pc) => {
                        // Sherwood-only: on `pc == None`, mark every
                        // PC's interface hidden; otherwise hide just
                        // that PC's.  Engine side clears the selection
                        // list separately.
                        if self.is_sherwood(&assets.profile_manager) {
                            match pc {
                                None => {
                                    let ids = self.world.pc_ids.clone();
                                    for id in ids {
                                        if let Some(crate::element::Entity::Pc(pc)) =
                                            self.get_entity_mut(id)
                                        {
                                            pc.pc.interface_hidden = true;
                                        }
                                    }
                                }
                                Some(pc_id) => {
                                    if let Some(crate::element::Entity::Pc(pc)) =
                                        self.get_entity_mut(pc_id)
                                    {
                                        pc.pc.interface_hidden = true;
                                    }
                                }
                            }
                        }
                        match pc {
                            None => self.unselect_all_pcs(0),
                            Some(pc_id) => self.unselect_single_pc(pc_id),
                        }
                        self.emit_character_selection_followups();
                    }
                    // The engine drops the PC from the selection and
                    // (outside Sherwood) removes the portrait.  The
                    // portrait strip in Rust immediate-mode renders
                    // from `pc_ids` filtered by `pc.playable`, so the
                    // "portrait disappears" side effect is covered by
                    // the native already writing `pc.playable = false`
                    // at `natives/mod.rs:1546`.  Here we only need the
                    // selection-drop plus the Sherwood interface flag.
                    MessageType::Pc(crate::messenger::PcMessage::DisableCharacter, pc) => {
                        if let Some(pc_id) = pc {
                            self.unselect_single_pc(pc_id);
                            // Net effect: flip the interface-hidden
                            // flag only when we are NOT in Sherwood.
                            // Previously the gate was inverted; the
                            // effect was masked because
                            // `interface_hidden` is not read by the
                            // HUD path, but parity still matters for
                            // the `STATUS PC` cheat and future HUD
                            // wiring.
                            if !self.is_sherwood(&assets.profile_manager)
                                && let Some(crate::element::Entity::Pc(pc)) =
                                    self.get_entity_mut(pc_id)
                            {
                                pc.pc.interface_hidden = true;
                            }
                        }
                    }
                    // The portrait widget is re-added only outside
                    // Sherwood.  In Rust, the live HUD reads
                    // `pc.interface_hidden`; clear it whenever the
                    // portrait would have been re-added.  Sherwood
                    // also gets the same clear so the HUD panel
                    // re-shows the PC when re-activated mid-Sherwood.
                    MessageType::Pc(crate::messenger::PcMessage::EnableCharacter, pc) => {
                        if let Some(pc_id) = pc
                            && let Some(crate::element::Entity::Pc(pc)) = self.get_entity_mut(pc_id)
                        {
                            pc.pc.interface_hidden = false;
                        }
                    }
                    // After a modal (dialogue, popup, Sherwood report)
                    // closes, zero the cached mouse/keyboard state,
                    // clear the rubber-band selection and
                    // pending-drag / click suppression flags, and drop
                    // pressed-key edges queued during the modal.  The
                    // Rust equivalents live host-side across two
                    // InputState groups: ThreadedInput pressed-key
                    // cache (`pending_reset_input`) and the
                    // rubber-band / click-suppression flags
                    // (`reset_input`).
                    MessageType::Simple(crate::messenger::SimpleMessage::ResetInput) => {
                        self.feedback.pending_side_effects.pending_reset_input = true;
                        self.feedback.pending_side_effects.reset_input = true;
                        // Clear the alt-lock latch along with the
                        // modifier cache; without this, an alt-lock
                        // toggled before a console-hide / task-switch
                        // / save-load / unlock-user would persist
                        // past the reset.
                        self.players.seats[0].is_lock_alt = false;
                    }
                    // Ctrl-press saves the current action on every
                    // selected PC so the follow-on move command can
                    // run without the action overriding it (and the
                    // action is restored on ctrl-release).  Emitted
                    // by the host input layer when
                    // `GameAction::KeyControl` fires.
                    MessageType::Simple(crate::messenger::SimpleMessage::KeyControl) => {
                        self.save_action_for_selected_pcs(0);
                    }
                    // `LockUser` / `UnlockUser` flip `user_locked`.
                    // Scripts already set it directly via
                    // `Command::LockUser` (see tick.rs sequence-manager
                    // handler), but wiring the messenger variants
                    // keeps any non-script producer in sync with the
                    // engine-side flag that gates mouse events in
                    // `handle_mouse_input`.  Unlock also raises the
                    // `pending_reset_input` side-effect so held-key
                    // edges from the locked period are dropped.
                    MessageType::Simple(crate::messenger::SimpleMessage::LockUser) => {
                        self.players.user_locked = true;
                    }
                    MessageType::Simple(crate::messenger::SimpleMessage::UnlockUser) => {
                        self.players.user_locked = false;
                        self.feedback.pending_side_effects.pending_reset_input = true;
                    }
                    // After hiding the console or switching task,
                    // emit `MSG_RESET_INPUT` so the held-key edges
                    // and modifier latches don't bleed across the
                    // task boundary.
                    MessageType::Simple(crate::messenger::SimpleMessage::HideConsole)
                    | MessageType::Simple(crate::messenger::SimpleMessage::SwitchTask) => {
                        self.feedback.pending_side_effects.pending_reset_input = true;
                        self.feedback.pending_side_effects.reset_input = true;
                        // Same `is_lock_alt` clear as the explicit
                        // `ResetInput` arm above.
                        self.players.seats[0].is_lock_alt = false;
                    }
                    // `SelectActionSimple` and `DisableAction` both
                    // clear the aim-trajectory preview so a dropped /
                    // replaced action doesn't leave a stale trajectory
                    // overlay on screen.  `valid_trajectory` lives on
                    // `host` in the Rust split, so raise the
                    // side-effect flag.
                    MessageType::Pc(crate::messenger::PcMessage::SelectActionSimple, _)
                    | MessageType::Pc(crate::messenger::PcMessage::DisableAction, _) => {
                        if matches!(
                            msg.msg_type,
                            MessageType::Pc(crate::messenger::PcMessage::SelectActionSimple, _)
                        ) {
                            self.players.seats[0].selected_action =
                                crate::profiles::Action::try_from(msg.value).unwrap_or_else(|_| {
                                    panic!(
                                        "MSG_SELECT_ACTION_SIMPLE carried invalid action {}",
                                        msg.value
                                    )
                                });
                        }
                        self.feedback
                            .pending_side_effects
                            .invalidate_trajectory_preview = true;
                    }
                    // A macro fizzled on a PC's QA slot, so arm the
                    // per-slot titbit blink strobe.  Typed `pc` slot
                    // carries the PC id; `msg.value` is the QA slot
                    // index.  A `None` PC is treated as a no-op with
                    // a warning (the producer must always set one).
                    MessageType::Pc(crate::messenger::PcMessage::FizzleMacro, pc) => {
                        let slot = msg.value as usize;
                        match pc {
                            None => tracing::warn!(
                                "FizzleMacro received with no PC; \
                                 producer must set the PC id"
                            ),
                            Some(pc_id) => {
                                self.feedback.pending_side_effects.host_events.push(
                                    HostEvent::MacroUi(MacroUiHostEvent::BlinkQa { pc_id, slot }),
                                );
                            }
                        }
                    }
                    // `QaFocus` flashes the macro titbit for the
                    // focused QA slot.  Typed `pc` slot carries the
                    // PC (None = all PCs); `msg.value` encodes the
                    // slot index.
                    MessageType::Pc(crate::messenger::PcMessage::QaFocus, pc) => {
                        let slot = msg.value as usize;
                        match pc {
                            None => {
                                let pc_ids = self.world.pc_ids.clone();
                                for pc_id in pc_ids {
                                    self.set_blinking_for_slot(pc_id, slot);
                                }
                            }
                            Some(pc_id) => self.set_blinking_for_slot(pc_id, slot),
                        }
                    }
                    // Bulk-flip `disabled_actions_temp` on a specific
                    // PC (`Some(pc_id)`) or every selected PC
                    // (`None`).
                    MessageType::Pc(crate::messenger::PcMessage::DisableAllActionsTemp, pc) => {
                        // Tick messenger drain: ambient single-seat
                        // semantics; LOCAL seat for now.
                        self.apply_disable_all_actions_temp(0, pc);
                    }
                    MessageType::Pc(crate::messenger::PcMessage::EnableAllActionsTemp, pc) => {
                        self.apply_enable_all_actions_temp(assets, 0, pc);
                    }
                    // Other messages are consumed by downstream systems
                    // (UI layer, mission flow). Re-enqueue so those
                    // consumers can still observe them.
                    _ => downstream.push_back(msg),
                }

                // Preserve the send order of recursive calls while placing
                // them ahead of pre-existing sibling messages.
                for nested in self.orders.messenger.drain().into_iter().rev() {
                    messages.push_front(nested);
                }
            }
            for msg in downstream {
                self.orders.messenger.send(msg);
            }
        }

        None
    }

    /// Promote queued NPC intents before entity refresh and sequence dispatch.
    ///
    /// NPC AI is primarily reached through each NPC's
    /// the per-entity update in the original entity loop. The Rust pre-pass is an
    /// architectural split; its exact parity remains audited separately.
    pub(super) fn hourglass_phase_npc_orders(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        self.tick_tactical_control(sim, assets);

        // ── Sequence manager cleanup ─────────────────────────────
        // Run every 256 frames (or every frame in debug).
        if self.control.frame_counter.is_multiple_of(256) {
            // The human shoot list stores raw sequence-element references. A
            // retail save can retain a terminal pointer past Friday cleanup;
            // the allocation then remains readable as stale legacy state.
            // Keep the Rust backing sequence alive while that explicit pointer
            // emulation exists, rather than turning the next shoot-list update
            // call into a missing-element panic.
            let retained_shoot_sequences = self
                .world
                .entities
                .occupied()
                .filter_map(|(_, entity)| entity.human_data())
                .flat_map(|human| {
                    human
                        .pending_shoots
                        .iter()
                        .map(|element_ref| element_ref.sequence_id)
                })
                .collect::<std::collections::BTreeSet<_>>();
            self.orders
                .sequence_manager
                .friday_evening_cleanup_preserving(&retained_shoot_sequences);
        }

        // ── Process pending AI orders ─────────────────────────────
        //
        // AI Move intents collected by `launch_pending_orders_for_npc`
        // route through `launch_ai_move`, which just enqueues into
        // `pending_move_requests` (dedup-per-actor).  The drain below
        // promotes one Move sequence element per unique actor this
        // tick — absorbing redundant re-fires that would otherwise
        // launch a fresh Move each frame and `InterruptCurrent` the
        // in-flight one. A*-requiring elements enter the frame-paced
        // path-request queue advanced by the following `Paths` phase.
        self.process_pending_ai_orders(sim, assets);
        self.drain_pending_move_requests(sim);

        // ── Dispatch per-waypoint ReachPoint scripts ─────────────
        // When the AI reaches a scripted waypoint it queues the
        // dispatch on `pending_waypoint_script_reach_point`; we drain
        // the queue here, call `ReachPoint(actor)` on the waypoint's
        // VM, and push `EventAfterScriptGoOn` as a self-stimulus
        // unless the script pulled the NPC into `DefaultScriptDriven`.
        // Runs before `process_pending_cross_npc_actions` so the
        // self-stimulus drain at the end of that pass picks up the
        // `EventAfterScriptGoOn` in the same tick.
        self.dispatch_pending_waypoint_scripts(sim, assets);

        // ── Process cross-NPC actions (phalanx coordination) ────
        self.process_pending_cross_npc_actions(sim, assets);

        // ── Process AI animation orders ─────────────────────────
        // Drain Pointing/RaisingShield/etc orders from NPC order queues
        // and start them as active_ai_anim. EventDone fires when the
        // sprite animation completes (detected in tick_actor_animation_for).
        self.process_animation_orders();

        // TODO(original-parity): determine which queued NPC-order effects must
        // remain inside an individual NPC's creation-ordered update.
    }

    pub(super) fn advance_mission_clock(&mut self) {
        self.control.frame_counter += 1;
        if self.control.frame_counter.is_multiple_of(FRAMES_PER_SECOND)
            && let Some(campaign) = Some(&mut self.mission_domain.campaign)
        {
            campaign.add_value(crate::campaign::CampaignValue::MissionLength, 1);
        }
    }
}
