use super::*;

impl EngineInner {
    /// Extracted from the `ExecuteImmediateOwner` match arm in
    /// `perform_hourglass_inner`.  Dispatches the owner-immediate
    /// command group (Teleport, LockAi, UnlockAi, ReplaceAnim,
    /// RestoreAnim, Speak, StartMobile, StopMobile, ActivateMobile,
    /// DeactivateMobile, Unblip, owner-bound SendMessage).
    pub(super) fn dispatch_execute_immediate_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let cmd = {
            let Some(e) = self.orders.sequence_manager.get_element(seq_id, elem_idx) else {
                return;
            };
            e.command
        };
        match cmd {
            Command::StartMobile
            | Command::StopMobile
            | Command::ActivateMobile
            | Command::DeactivateMobile => {
                self.dispatch_mobile_immediate(
                    sim,
                    assets,
                    active_scripts,
                    owner,
                    cmd,
                    seq_id,
                    elem_idx,
                );
            }
            Command::Unblip | Command::ReplaceAnim | Command::RestoreAnim => {
                self.dispatch_sprite_immediate(
                    sim,
                    assets,
                    active_scripts,
                    owner,
                    cmd,
                    seq_id,
                    elem_idx,
                );
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
                    self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
                    return;
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
                    && self
                        .world
                        .entities
                        .get(owner)
                        .and_then(Entity::ai_controller)
                        .is_some()
                {
                    let flags_bits = speak_flags.unwrap_or(0) as u16;
                    let flags = crate::ai::SpeechFlags::from_bits_truncate(flags_bits);
                    self.execute_ai_speech(
                        sim,
                        assets,
                        owner,
                        crate::ai::AiSpeechAttempt {
                            remark,
                            flags: flags.bits(),
                        },
                    );
                } else {
                    tracing::warn!(
                        ?owner,
                        speak_id,
                        "Speak: invalid remark id or missing AI controller"
                    );
                }
                self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
            }
            Command::Teleport => {
                self.execute_teleport(sim, assets, active_scripts, owner, seq_id, elem_idx)
            }
            Command::LockAi | Command::UnlockAi => {
                if self
                    .get_entity(owner)
                    .and_then(crate::element::Entity::ai_controller)
                    .is_some()
                {
                    if cmd == Command::LockAi {
                        self.execute_ai_script_lock_in_driver(
                            sim,
                            assets,
                            owner,
                            false,
                            active_scripts,
                        )
                        .unwrap_or_else(|error| panic!("script lock failed: {error:?}"));
                    } else {
                        self.execute_ai_script_unlock(sim, assets, owner);
                    }
                }
                self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
            }
            _ => {
                self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
            }
        }
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
                self.apply_script_user_lock(sim, assets, command);
                self.element_terminated(sim, assets, &mut Vec::new(), seq_id, elem_idx);
            }
            Some(Command::Timer) => {
                let timer = self.timer_immediate_entry(seq_id, elem_idx);
                self.add_timer(timer.remaining, timer.element_ref);
            }
            Some(Command::CameraJumpTo) => {
                // Terminate any pending camera sequence element,
                // snap the view to the requested point, invalidate
                // background, and terminate self.
                self.terminate_prev_camera_sequence_element(sim, assets);
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
                }
                self.element_terminated(sim, assets, &mut Vec::new(), seq_id, elem_idx);
            }
            Some(Command::CameraGoto) => {
                // Terminate any previous camera sequence element,
                // stash this one as the in-progress camera element,
                // and start a slide toward the target.
                // Fast-forward snaps instantly.
                self.terminate_prev_camera_sequence_element(sim, assets);
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
                    self.element_terminated(sim, assets, &mut Vec::new(), seq_id, elem_idx);
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
                    self.element_terminated(sim, assets, &mut Vec::new(), seq_id, elem_idx);
                }
            }
            Some(Command::ZoomLevel) => {
                // Terminate any previous camera sequence element,
                // record the requested zoom factor, and latch this
                // element as the in-progress camera element until
                // the zoom transition finishes.
                self.terminate_prev_camera_sequence_element(sim, assets);
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
                    self.element_terminated(sim, assets, &mut Vec::new(), seq_id, elem_idx);
                }
            }
            Some(Command::LockCameraOn) => {
                // Terminate any previous camera sequence element,
                // start following the antagonist, drop any titbit
                // locks, and terminate self.
                self.terminate_prev_camera_sequence_element(sim, assets);
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
                self.element_terminated(sim, assets, &mut Vec::new(), seq_id, elem_idx);
            }
            Some(Command::LockCameraStop) => {
                self.terminate_prev_camera_sequence_element(sim, assets);
                self.players.seats[0].follow_element = None;
                self.players.seats[0].locker_active = false;
                self.element_terminated(sim, assets, &mut Vec::new(), seq_id, elem_idx);
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
                self.dispatch_presentation_command(
                    sim,
                    assets,
                    &mut Vec::new(),
                    command,
                    seq_id,
                    elem_idx,
                );
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
                self.dispatch_freeze_immediate(sim, assets, &mut Vec::new(), seq_id, elem_idx);
            }
            Some(command @ (Command::CharacterAvailable | Command::ActionAvailable)) => {
                self.dispatch_availability_immediate(
                    sim,
                    assets,
                    &mut Vec::new(),
                    command,
                    seq_id,
                    elem_idx,
                );
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
                self.element_terminated(sim, assets, &mut Vec::new(), seq_id, elem_idx);
            }
            _ => {
                // Unknown commands fall through without being
                // terminated.
            }
        }
        None
    }
}
