use super::*;
use crate::engine::TickCtx;
use crate::sequence::SequenceElementRef;

impl EngineInner {
    /// Extracted from the `ExecuteImmediateOwner` match arm in
    /// `perform_hourglass_inner`.  Dispatches the owner-immediate
    /// command group (Teleport, LockAi, UnlockAi, ReplaceAnim,
    /// RestoreAnim, Speak, StartMobile, StopMobile, ActivateMobile,
    /// DeactivateMobile, Unblip, owner-bound SendMessage).
    pub(super) fn dispatch_execute_immediate_owner(
        &mut self,
        tcx: TickCtx<'_>,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        owner: EntityId,
        elem_ref: SequenceElementRef,
    ) {
        let cmd = {
            let Some(e) = self.orders.sequence_manager.get_element_at(elem_ref) else {
                return;
            };
            e.command
        };
        match cmd {
            Command::StartMobile
            | Command::StopMobile
            | Command::ActivateMobile
            | Command::DeactivateMobile => {
                self.dispatch_mobile_immediate(tcx, active_scripts, owner, cmd, elem_ref);
            }
            Command::Unblip | Command::ReplaceAnim | Command::RestoreAnim => {
                self.dispatch_sprite_immediate(tcx, active_scripts, owner, cmd, elem_ref);
            }
            Command::Speak => {
                // NPC: `say_remark(speak_id, speak_flags)`.
                // PC:  `hero_speaking(speak_id, SPEECH_SCRIPT,
                //                     speak_variant)`.
                let (speak_id, speak_flags, speak_variant) = {
                    let elem = self.orders.sequence_manager.get_element_at(elem_ref);
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
                    self.element_terminated(tcx, active_scripts, elem_ref);
                    return;
                };
                let owner_is_pc = self.get_entity(owner).is_some_and(|e| e.is_pc());
                if owner_is_pc {
                    self.hero_speaking_script(
                        tcx.assets,
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
                        tcx,
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
                self.element_terminated(tcx, active_scripts, elem_ref);
            }
            Command::Teleport => self.execute_teleport(tcx, active_scripts, owner, elem_ref),
            Command::LockAi | Command::UnlockAi => {
                if self
                    .get_entity(owner)
                    .and_then(crate::element::Entity::ai_controller)
                    .is_some()
                {
                    if cmd == Command::LockAi {
                        self.execute_ai_script_lock_in_driver(tcx, owner, false, active_scripts)
                            .unwrap_or_else(|error| panic!("script lock failed: {error:?}"));
                    } else {
                        self.execute_ai_script_unlock(tcx, owner);
                    }
                }
                self.element_terminated(tcx, active_scripts, elem_ref);
            }
            _ => {
                self.element_terminated(tcx, active_scripts, elem_ref);
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
        tcx: TickCtx<'_>,
        elem_ref: SequenceElementRef,
    ) -> Option<(i32, i32, i32)> {
        // Check for SendMessage targeting the global script.
        let cmd = self
            .orders
            .sequence_manager
            .get_element_at(elem_ref)
            .map(|e| e.command);
        match cmd {
            Some(Command::SendMessage) => {
                // Ownerless SendMessage dispatches
                // `IEngineScript::ProcessMessage` (global).
                let (msg, arg1, arg2) = self.extract_message_properties(elem_ref);
                return Some((msg, arg1, arg2));
            }
            Some(command @ (Command::LockUser | Command::UnlockUser)) => {
                self.apply_script_user_lock(tcx, command);
                self.element_terminated(tcx, &mut Vec::new(), elem_ref);
            }
            Some(Command::Timer) => {
                let timer = self.timer_immediate_entry(elem_ref);
                self.add_timer(timer.remaining, timer.element_ref);
            }
            Some(Command::CameraJumpTo) => {
                // Terminate any pending camera sequence element,
                // snap the view to the requested point, invalidate
                // background, and terminate self.
                self.terminate_prev_camera_sequence_element(tcx);
                self.players.seats[0].follow_element = None;
                self.players.seats[0].locker_active = false;
                let point = self
                    .orders
                    .sequence_manager
                    .get_element_at(elem_ref)
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
                self.element_terminated(tcx, &mut Vec::new(), elem_ref);
            }
            Some(Command::CameraGoto) => {
                // Terminate any previous camera sequence element,
                // stash this one as the in-progress camera element,
                // and start a slide toward the target.
                // Fast-forward snaps instantly.
                self.terminate_prev_camera_sequence_element(tcx);
                self.players.seats[0].follow_element = None;
                self.players.seats[0].locker_active = false;
                let (point, speed) = {
                    let e = self.orders.sequence_manager.get_element_at(elem_ref);
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
                    self.element_terminated(tcx, &mut Vec::new(), elem_ref);
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
                        Some(crate::sequence::SequenceElementRef::new(
                            elem_ref.sequence_id,
                            elem_ref.element_index,
                        ));
                } else {
                    self.element_terminated(tcx, &mut Vec::new(), elem_ref);
                }
            }
            Some(Command::ZoomLevel) => {
                // Terminate any previous camera sequence element,
                // record the requested zoom factor, and latch this
                // element as the in-progress camera element until
                // the zoom transition finishes.
                self.terminate_prev_camera_sequence_element(tcx);
                let zoom = self
                    .orders
                    .sequence_manager
                    .get_element_at(elem_ref)
                    .and_then(|e| e.get_property(crate::sequence::Field::CameraZoomLevel))
                    .and_then(|v| match v {
                        crate::sequence::FieldValue::Float(f) => Some(*f),
                        _ => None,
                    });
                if let Some(z) = zoom {
                    self.feedback.cutscene_camera.desired_zoom_factor = z;
                    self.feedback.cutscene_camera.sequence_element =
                        Some(crate::sequence::SequenceElementRef::new(
                            elem_ref.sequence_id,
                            elem_ref.element_index,
                        ));
                } else {
                    self.element_terminated(tcx, &mut Vec::new(), elem_ref);
                }
            }
            Some(Command::LockCameraOn) => {
                // Terminate any previous camera sequence element,
                // start following the antagonist, drop any titbit
                // locks, and terminate self.
                self.terminate_prev_camera_sequence_element(tcx);
                let target = self
                    .orders
                    .sequence_manager
                    .get_element_at(elem_ref)
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
                self.element_terminated(tcx, &mut Vec::new(), elem_ref);
            }
            Some(Command::LockCameraStop) => {
                self.terminate_prev_camera_sequence_element(tcx);
                self.players.seats[0].follow_element = None;
                self.players.seats[0].locker_active = false;
                self.element_terminated(tcx, &mut Vec::new(), elem_ref);
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
                self.dispatch_presentation_command(tcx, &mut Vec::new(), command, elem_ref);
                if refreshes_during_dialogue || refreshes_during_popup {
                    // Dialogue display constructs a menu screen
                    // inline; accepted popup scroll backgrounds take the same
                    // path. Their constructor calls
                    // game refresh before returning to the
                    // sequence manager, hence before frame recording. Model only
                    // the simulation-bearing arrow portion here; resolved PC
                    // orientation is an explicit replay command.
                    self.refresh_arrows_for_presentation(tcx.sim);
                }
            }
            Some(Command::Freeze | Command::FreezeAll) => {
                self.dispatch_freeze_immediate(tcx, &mut Vec::new(), elem_ref);
            }
            Some(command @ (Command::CharacterAvailable | Command::ActionAvailable)) => {
                self.dispatch_availability_immediate(tcx, &mut Vec::new(), command, elem_ref);
            }
            Some(Command::OpenScroll) => {
                // Call `scroll_is_taken` on the scroll referenced
                // by `Scroll`, passing the PC from `ScrollReader`.
                // Opens the scroll and, if a script is bound,
                // dispatches its `IsTaken` handler.
                let (scroll_id, reader_id) = {
                    let elem = self.orders.sequence_manager.get_element_at(elem_ref);
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
                    self.scroll_is_taken(tcx, scroll, reader);
                } else {
                    tracing::warn!(
                        ?scroll_id,
                        ?reader_id,
                        "OpenScroll sequence command missing Scroll/ScrollReader property"
                    );
                }
                self.element_terminated(tcx, &mut Vec::new(), elem_ref);
            }
            _ => {
                // Unknown commands fall through without being
                // terminated.
            }
        }
        None
    }
}
