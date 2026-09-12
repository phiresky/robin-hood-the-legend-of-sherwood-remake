use super::*;

impl EngineInner {
    /// Wrapper around the immediate-action helpers.
    ///
    /// Dispatches the immediate side effect synchronously rather
    /// than queuing it.  Used both by `perform_hourglass_inner`'s
    /// action loop and by
    /// [`Self::drain_pending_immediate_actions_sync`] to fire
    /// `pending_immediate_actions` queued by
    /// `register_element_to_go` outside the hourglass dispatch
    /// loop.
    fn dispatch_immediate_action(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        action: crate::sequence::SequenceAction,
    ) {
        match action {
            crate::sequence::SequenceAction::ExecuteImmediateOwner {
                owner,
                sequence_id,
                element_index,
            } => {
                if let Some((handle, msg, arg1, arg2)) = self.dispatch_execute_immediate_owner(
                    sim,
                    assets,
                    owner,
                    sequence_id,
                    element_index,
                ) {
                    self.dispatch_sequence_messages(sim, assets, &[(handle, msg, arg1, arg2)], &[]);
                    self.orders
                        .sequence_manager
                        .element_terminated(sequence_id, element_index);
                }
            }
            crate::sequence::SequenceAction::ExecuteImmediateEngine {
                sequence_id,
                element_index,
            } => {
                if let Some((msg, arg1, arg2)) = self.dispatch_engine_or_execute_immediate(
                    sim,
                    assets,
                    sequence_id,
                    element_index,
                ) {
                    self.dispatch_sequence_messages(sim, assets, &[], &[(msg, arg1, arg2)]);
                    self.orders
                        .sequence_manager
                        .element_terminated(sequence_id, element_index);
                }
            }
            other => panic!(
                "dispatch_immediate_action called with non-immediate variant: {:?}",
                other
            ),
        }
    }
    /// Synchronous drain of the complete sequence-registration stream.
    ///
    /// External entry points around the manager
    /// (`launch_sequence`, `launch_element`, `element_terminated`,
    /// `element_impossible`, `element_in_progress`,
    /// `element_interrupted`, `terminate_sequence`, `stop_owner`,
    /// `stop_pending_elements*`, `cancel_pending_move_commands`)
    /// can register elements via `register_element_to_go`, which in
    /// turn queues immediate `SequenceAction`s for the
    /// immediate command groups. Engine-side wrappers
    /// that have access to `&LevelAssets` call this helper after
    /// invoking such an entry point. Despite the legacy method name, it
    /// drains immediate commands and direct WAIT `Go()` successors as one
    /// ordered, depth-first registration stream.
    ///
    /// `SendMessage` invokes `ProcessMessage` at the action's exact position
    /// and terminates only after the callback returns, matching
    /// immediate actor and engine command execution.
    pub(crate) fn drain_pending_immediate_actions_sync(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        while let Some(action) = self.orders.sequence_manager.pop_pending_immediate_action() {
            // Work that was already registered is the caller's continuation.
            // Detach it so Ready() successors produced by this action drain
            // depth-first before an older sibling.
            let continuation = self
                .orders
                .sequence_manager
                .take_pending_synchronous_actions();
            match action {
                crate::sequence::SequenceAction::ExecuteImmediateOwner { .. }
                | crate::sequence::SequenceAction::ExecuteImmediateEngine { .. } => {
                    self.dispatch_immediate_action(sim, assets, action);
                }
                crate::sequence::SequenceAction::InstructOwner { .. }
                | crate::sequence::SequenceAction::EngineCommand { .. } => {
                    self.dispatch_script_synchronous_action(sim, assets, action, &mut Vec::new())
                        .unwrap_or_else(|error| {
                            panic!("synchronous sequence successor dispatch failed: {error:?}")
                        });
                }
            }
            self.dispatch_condolations(sim, assets);
            self.drain_pending_immediate_actions_sync(sim, assets);
            self.orders
                .sequence_manager
                .restore_pending_synchronous_actions(continuation);
        }
    }

    /// Drain only work that legacy sequence registration executes inline,
    /// leaving ordinary sequence-element registration work queued for
    /// the sequence-manager tick.
    ///
    /// Immediately executed commands and waiting-priority successors run on
    /// the registration stack. Other priorities cannot start until the next
    /// manager hourglass when registration occurs after its phase.
    pub(crate) fn drain_registration_inline_actions_sync(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        while let Some(action) = self
            .orders
            .sequence_manager
            .pop_pending_registration_inline_action()
        {
            let continuation = self
                .orders
                .sequence_manager
                .take_pending_synchronous_actions();
            match action {
                crate::sequence::SequenceAction::ExecuteImmediateOwner { .. }
                | crate::sequence::SequenceAction::ExecuteImmediateEngine { .. } => {
                    self.dispatch_immediate_action(sim, assets, action);
                }
                crate::sequence::SequenceAction::InstructOwner { .. }
                | crate::sequence::SequenceAction::EngineCommand { .. } => {
                    self.dispatch_script_synchronous_action(sim, assets, action, &mut Vec::new())
                        .unwrap_or_else(|error| {
                            panic!("inline WAIT successor dispatch failed: {error:?}")
                        });
                }
            }
            self.dispatch_condolations(sim, assets);
            self.drain_registration_inline_actions_sync(sim, assets);
            self.orders
                .sequence_manager
                .restore_pending_synchronous_actions(continuation);
        }
    }

    /// Extracted from the `ExecuteImmediateOwner` match arm in
    /// `perform_hourglass_inner`.  Dispatches the owner-immediate
    /// command group (Teleport, LockAi, UnlockAi, ReplaceAnim,
    /// RestoreAnim, Speak, StartMobile, StopMobile, ActivateMobile,
    /// DeactivateMobile, Unblip, owner-bound SendMessage).
    pub(super) fn dispatch_execute_immediate_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> Option<(i32, i32, i32, i32)> {
        let cmd = {
            let e = self.orders.sequence_manager.get_element(seq_id, elem_idx)?;
            e.command
        };
        match cmd {
            Command::StartMobile
            | Command::StopMobile
            | Command::ActivateMobile
            | Command::DeactivateMobile => {
                MobileImmediateContext {
                    entities: &mut self.world.entities,
                    mobiles: &mut self.world.mobile_elements,
                    sequence_manager: &mut self.orders.sequence_manager,
                }
                .dispatch(owner, cmd, seq_id, elem_idx);
            }
            Command::SendMessage => {
                // Dispatch ProcessMessage to the owner's per-actor
                // script.
                let (msg, arg1, arg2) = self.extract_message_properties(seq_id, elem_idx);
                let handle = crate::natives::ScriptHandleCodec::actor_handle(owner);
                return Some((handle, msg, arg1, arg2));
            }
            Command::Unblip | Command::ReplaceAnim | Command::RestoreAnim => {
                SpriteImmediateContext {
                    entities: &mut self.world.entities,
                    sequence_manager: &mut self.orders.sequence_manager,
                }
                .dispatch(owner, cmd, seq_id, elem_idx);
            }
            Command::Speak => {
                // NPC: `say_remark(speak_id, speak_flags)`.
                // PC:  `hero_speaking(speak_id, SPEECH_SCRIPT,
                //                     speak_variant)`.
                let (speak_id, speak_flags, speak_variant) = {
                    let elem = self.orders.sequence_manager.get_element(seq_id, elem_idx);
                    let id =
                        elem.and_then(|e| match e.get_property(crate::sequence::Field::SpeakId) {
                            Some(crate::sequence::FieldValue::Integer(v)) => Some(*v),
                            _ => None,
                        });
                    let flags = elem.and_then(|e| {
                        match e.get_property(crate::sequence::Field::SpeakFlags) {
                            Some(crate::sequence::FieldValue::Integer(v)) => Some(*v),
                            _ => None,
                        }
                    });
                    let variant = elem.and_then(|e| {
                        match e.get_property(crate::sequence::Field::SpeakVariant) {
                            Some(crate::sequence::FieldValue::Integer(v)) => Some(*v),
                            _ => None,
                        }
                    });
                    (id, flags, variant)
                };
                let Some(speak_id) = speak_id else {
                    tracing::warn!(?owner, "Speak: missing SpeakId property — terminating");
                    self.orders
                        .sequence_manager
                        .element_terminated(seq_id, elem_idx);
                    return None;
                };
                let owner_is_pc = self.get_entity(owner).is_some_and(|e| e.is_pc());
                if owner_is_pc {
                    self.hero_speaking_script(
                        assets,
                        owner,
                        speak_id as u16,
                        speak_variant.map(|v| v as i32),
                    );
                } else if let Ok(remark) = crate::ai::Remark::try_from(speak_id)
                    && let Some(entity) = self.world.entities.get_mut(owner)
                    && let Some(ai) = entity.npc_data_mut().and_then(|n| n.ai_brain.base_mut())
                {
                    let flags_bits = speak_flags.unwrap_or(0) as u16;
                    let flags = crate::ai::SpeechFlags::from_bits_truncate(flags_bits);
                    ai.say_with_flags(remark, flags);
                    self.drain_ai_owner_work_for(sim, assets, owner);
                } else {
                    tracing::warn!(
                        ?owner,
                        speak_id,
                        "Speak: invalid remark id or missing AI controller"
                    );
                }
                self.orders
                    .sequence_manager
                    .element_terminated(seq_id, elem_idx);
            }
            Command::Teleport => self.execute_teleport(assets, owner, seq_id, elem_idx),
            Command::LockAi | Command::UnlockAi => {
                let unconscious = self
                    .get_entity(owner)
                    .and_then(|entity| entity.human_data())
                    .is_some_and(|human| human.unconscious);
                let mut stop_for_lock = false;
                if let Some(ai) = self
                    .get_entity_mut(owner)
                    .and_then(crate::element::Entity::ai_controller_mut)
                {
                    if cmd == Command::LockAi {
                        // Immediate execution calls the owner directly; it
                        // does not pass through launch/instruction and therefore
                        // does not select the LockAi element. ScriptLockAI
                        // still sees the actor's outgoing command and calls
                        // Stop(Normal) synchronously before LockAi itself
                        // terminates.
                        //
                        // Suppress the controller's deferred halt and close
                        // that Stop explicitly below, at this exact immediate
                        // command boundary.
                        ai.script_lock(false, true);
                        stop_for_lock = true;
                    } else {
                        // The original game's script-driven AI unlock is deliberately not
                        // conditional on script-lock state. A repeated authored
                        // UnlockAi still clears detections and synchronously
                        // runs EVENT_RETURN_TO_DUTY, which can replace the
                        // actor's current movement with a fresh movement order.
                        ai.script_unlock(unconscious);
                    }
                }
                if stop_for_lock {
                    self.stop_owner(owner, crate::sequence::SequencePriority::Normal);
                    self.dispatch_condolations(sim, assets);
                }
                if cmd == Command::UnlockAi {
                    // ScriptUnlockAI calls Think(EVENT_RETURN_TO_DUTY)
                    // before returning. Finish the owner-local AI callback,
                    // but do not instruct any resulting movement here:
                    // Registering a sequence element to go queues
                    // that ordinary Move for its manager-update phase.
                    self.drain_direct_ai_owner_boundary_without_forecast(sim, owner, assets);
                    self.drain_pending_move_requests_for_owner(sim, owner);
                }
                self.orders
                    .sequence_manager
                    .element_terminated(seq_id, elem_idx);
            }
            _ => {
                self.orders
                    .sequence_manager
                    .element_terminated(seq_id, elem_idx);
            }
        }
        None
    }

    /// Stage A — extracted from the combined
    /// `EngineCommand` / `ExecuteImmediateEngine` match arm in
    /// `perform_hourglass_inner`.  Dispatches engine-side
    /// commands — both the immediate group (LockUser, UnlockUser,
    /// CameraJumpTo, Timer, ActionAvailable, CharacterAvailable,
    /// OpenScroll, ownerless SendMessage) and the non-immediate
    /// engine commands handled by the same switch (CameraGoto,
    /// ZoomLevel, LockCameraOn/Stop, DisplayMap, PlayDialog,
    /// DisplayPopupText, Freeze[All]).
    pub(super) fn dispatch_engine_or_execute_immediate(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> Option<(i32, i32, i32)> {
        // Check for SendMessage targeting the global script.
        let cmd = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|e| e.command);
        match cmd {
            Some(Command::SendMessage) => {
                // Ownerless SendMessage dispatches
                // `IEngineScript::ProcessMessage` (global).
                let (msg, arg1, arg2) = self.extract_message_properties(seq_id, elem_idx);
                return Some((msg, arg1, arg2));
            }
            Some(command @ (Command::LockUser | Command::UnlockUser)) => {
                self.apply_script_user_lock(assets, command);
                self.orders
                    .sequence_manager
                    .element_terminated(seq_id, elem_idx);
            }
            Some(Command::Timer) => {
                let timer = TimerImmediateContext {
                    sequence_manager: &self.orders.sequence_manager,
                }
                .entry(seq_id, elem_idx);
                self.add_timer(timer.remaining, timer.element_ref);
            }
            Some(Command::CameraJumpTo) => {
                // Terminate any pending camera sequence element,
                // snap the view to the requested point, invalidate
                // background, and terminate self.
                self.terminate_prev_camera_sequence_element();
                self.players.seats[0].follow_element = None;
                self.players.seats[0].locker_active = false;
                let point = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|e| {
                        read_sequence_map_point_property(e, crate::sequence::Field::CameraPoint)
                    });
                if let Some(pos) = point {
                    // Direct assignment via
                    // `check_location_is_valid_for_camera`, no
                    // separate clamp.
                    self.feedback.cutscene_camera.view_position =
                        self.check_location_is_valid_for_camera(pos);
                    self.feedback.pending_side_effects.invalidate_background = true;
                }
                self.orders
                    .sequence_manager
                    .element_terminated(seq_id, elem_idx);
            }
            Some(Command::CameraGoto) => {
                // Terminate any previous camera sequence element,
                // stash this one as the in-progress camera element,
                // and start a slide toward the target.
                // Fast-forward snaps instantly.
                self.terminate_prev_camera_sequence_element();
                self.players.seats[0].follow_element = None;
                self.players.seats[0].locker_active = false;
                let (point, speed) = {
                    let e = self.orders.sequence_manager.get_element(seq_id, elem_idx);
                    let p = e.and_then(|e| {
                        read_sequence_map_point_property(e, crate::sequence::Field::CameraPoint)
                    });
                    let s = e
                        .and_then(|e| e.get_property(crate::sequence::Field::CameraSpeed))
                        .and_then(|v| match v {
                            crate::sequence::FieldValue::Integer(n) => Some(*n as u16),
                            _ => None,
                        })
                        .unwrap_or(0);
                    (p, s)
                };
                if self.control.fast_forward {
                    if let Some(pos) = point {
                        self.feedback.cutscene_camera.view_position =
                            self.check_location_is_valid_for_camera(pos);
                    }
                    self.orders
                        .sequence_manager
                        .element_terminated(seq_id, elem_idx);
                } else if let Some(pos) = point {
                    // Store the raw script point as
                    // `camera_wanted`, store the centered+clamped
                    // result as `camera_slide`.
                    self.feedback.cutscene_camera.camera_wanted = pos;
                    self.feedback.cutscene_camera.camera_slide =
                        self.check_location_is_valid_for_camera(pos);
                    self.feedback.cutscene_camera.fixed_camera_speed = speed;
                    self.control.speed = 2.0;
                    self.control.speed_int = 0;
                    self.feedback.cutscene_camera.sequence_element =
                        Some(crate::sequence::SequenceElementRef::new(seq_id, elem_idx));
                } else {
                    self.orders
                        .sequence_manager
                        .element_terminated(seq_id, elem_idx);
                }
            }
            Some(Command::ZoomLevel) => {
                // Terminate any previous camera sequence element,
                // record the requested zoom factor, and latch this
                // element as the in-progress camera element until
                // the zoom transition finishes.
                self.terminate_prev_camera_sequence_element();
                let zoom = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|e| e.get_property(crate::sequence::Field::CameraZoomLevel))
                    .and_then(|v| match v {
                        crate::sequence::FieldValue::Float(f) => Some(*f),
                        _ => None,
                    });
                if let Some(z) = zoom {
                    self.feedback.cutscene_camera.desired_zoom_factor = z;
                    self.feedback.cutscene_camera.sequence_element =
                        Some(crate::sequence::SequenceElementRef::new(seq_id, elem_idx));
                } else {
                    self.orders
                        .sequence_manager
                        .element_terminated(seq_id, elem_idx);
                }
            }
            Some(Command::LockCameraOn) => {
                // Terminate any previous camera sequence element,
                // start following the antagonist, drop any titbit
                // locks, and terminate self.
                self.terminate_prev_camera_sequence_element();
                let target = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|e| match &e.data {
                        crate::sequence::SequenceElementData::Interaction { antagonist } => {
                            *antagonist
                        }
                        _ => None,
                    });
                if let Some(t) = target {
                    self.players.seats[0].follow_element = Some(t);
                    self.players.seats[0].locker_active = true;
                } else {
                    self.players.seats[0].follow_element = None;
                    self.players.seats[0].locker_active = false;
                }
                self.feedback.titbit_manager.remove_lock();
                self.orders
                    .sequence_manager
                    .element_terminated(seq_id, elem_idx);
            }
            Some(Command::LockCameraStop) => {
                self.terminate_prev_camera_sequence_element();
                self.players.seats[0].follow_element = None;
                self.players.seats[0].locker_active = false;
                self.orders
                    .sequence_manager
                    .element_terminated(seq_id, elem_idx);
            }
            Some(
                command @ (Command::DisplayMap | Command::PlayDialog | Command::DisplayPopupText),
            ) => {
                let refreshes_during_dialogue =
                    command == Command::PlayDialog && !self.control.fast_forward;
                // Popup-scroll display asks
                // the required background colorization before constructing its
                // menu screen. The first popup in a universal frame passes
                // mouse removal enabled, so background colorization synchronously
                // re-enters the game refresh; later popups in
                // that frame pass false and do not refresh.
                let refreshes_during_popup = command == Command::DisplayPopupText
                    && !self.control.fast_forward
                    && self.control.begin_popup_scroll_display();
                PresentationCommandContext {
                    fast_forward: self.control.fast_forward,
                    side_effects: &mut self.feedback.pending_side_effects,
                    messenger: &mut self.orders.messenger,
                    sequence_manager: &mut self.orders.sequence_manager,
                }
                .dispatch(command, seq_id, elem_idx);
                if refreshes_during_dialogue || refreshes_during_popup {
                    // Dialogue display constructs a menu screen
                    // inline; accepted popup scroll backgrounds take the same
                    // path. Their constructor calls
                    // game refresh before returning to the
                    // sequence manager, hence before frame recording. Model only
                    // the simulation-bearing arrow portion here; resolved PC
                    // orientation is an explicit replay command.
                    self.refresh_arrows_for_presentation(sim);
                }
            }
            Some(Command::Freeze | Command::FreezeAll) => {
                FreezeImmediateContext {
                    control: &mut self.control,
                    sequence_manager: &mut self.orders.sequence_manager,
                }
                .dispatch(seq_id, elem_idx);
            }
            Some(command @ (Command::CharacterAvailable | Command::ActionAvailable)) => {
                AvailabilityImmediateContext {
                    entities: &mut self.world.entities,
                    messenger: &mut self.orders.messenger,
                    sequence_manager: &mut self.orders.sequence_manager,
                }
                .dispatch(command, seq_id, elem_idx);
            }
            Some(Command::OpenScroll) => {
                // Call `scroll_is_taken` on the scroll referenced
                // by `Scroll`, passing the PC from `ScrollReader`.
                // Opens the scroll and, if a script is bound,
                // dispatches its `IsTaken` handler.
                let (scroll_id, reader_id) = {
                    let elem = self.orders.sequence_manager.get_element(seq_id, elem_idx);
                    let scroll = elem
                        .and_then(|e| e.get_property(crate::sequence::Field::Scroll))
                        .and_then(|v| match v {
                            crate::sequence::FieldValue::Element(id) => Some(*id),
                            _ => None,
                        });
                    let reader = elem
                        .and_then(|e| e.get_property(crate::sequence::Field::ScrollReader))
                        .and_then(|v| match v {
                            crate::sequence::FieldValue::Element(id) => Some(*id),
                            _ => None,
                        });
                    (scroll, reader)
                };
                if let (Some(scroll), Some(reader)) = (scroll_id, reader_id) {
                    self.scroll_is_taken(sim, assets, scroll, reader);
                } else {
                    tracing::warn!(
                        ?scroll_id,
                        ?reader_id,
                        "OpenScroll sequence command missing Scroll/ScrollReader property"
                    );
                }
                self.orders
                    .sequence_manager
                    .element_terminated(seq_id, elem_idx);
            }
            _ => {
                // Unknown commands fall through without being
                // terminated.
            }
        }
        None
    }
}
