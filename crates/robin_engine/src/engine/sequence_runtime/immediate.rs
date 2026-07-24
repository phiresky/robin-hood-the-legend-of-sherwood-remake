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
        display: &mut HostDisplayState,
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
                    display,
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
    /// `ExecutedImmediately()` command groups.  Engine-side wrappers
    /// that have access to `&LevelAssets` call this helper after
    /// invoking such an entry point. Despite the legacy method name, it
    /// drains immediate commands and direct WAIT `Go()` successors as one
    /// ordered, depth-first registration stream.
    ///
    /// `SendMessage` invokes `ProcessMessage` at the action's exact position
    /// and terminates only after the callback returns, matching
    /// `RHElementActor::ExecuteImmediately` / `RHEngine::PerformExecuteCommand`.
    pub(crate) fn drain_pending_immediate_actions_sync(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
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
                    self.dispatch_immediate_action(sim, display, assets, action);
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
            self.drain_pending_immediate_actions_sync(sim, display, assets);
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
        let cmd = match self.orders.sequence_manager.get_element(seq_id, elem_idx) {
            Some(e) => e.command,
            None => return None,
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
            Command::Teleport => {
                // Read destination + layer + sector off the
                // movement element, snap the actor there, and spawn
                // the two 5-star bursts (old → new) at feet-to-eyes.
                // The element's `sector` field is ignored; sector +
                // layer are re-derived from the destination via
                // `get_sector_screen_accessible`.  Only the
                // destination point is read off the element here;
                // `dest_layer` is kept as a fallback for the
                // new-side star burst when the validation step
                // gives up.
                let (dest, dest_layer) = {
                    let elem = self.orders.sequence_manager.get_element(seq_id, elem_idx);
                    match elem.map(|e| &e.data) {
                        Some(crate::sequence::SequenceElementData::Movement {
                            destination,
                            layer,
                            ..
                        }) => (Some(*destination), Some(*layer)),
                        _ => (None, None),
                    }
                };
                if let Some(dest) = dest {
                    self.mission_domain.cheat_used_flags |= 0x0000_0001; // CHEAT_TELEPORT

                    // `stop_owner` cleans up any in-flight
                    // movement / active element before the teleport
                    // so the actor doesn't resume pathing toward
                    // its old destination on the next tick.
                    self.stop_owner(owner, crate::sequence::SequencePriority::Normal);

                    // Snapshot old position & whether this is a PC
                    // before any mutation; also capture eyes/feet
                    // points for the old-position star burst.
                    let (old_pos, old_feet, old_eyes, is_pc) = {
                        let entity = match self.get_entity(owner) {
                            Some(e) => e,
                            None => {
                                self.orders
                                    .sequence_manager
                                    .element_terminated(seq_id, elem_idx);
                                return None;
                            }
                        };
                        let ed = entity.element_data();
                        let feet = entity.compute_feet_point();
                        let eyes = entity.compute_eyes_point(None);
                        (
                            ed.position_map(),
                            feet,
                            eyes,
                            matches!(entity, crate::element::Entity::Pc(_)),
                        )
                    };

                    let zero_teleport = (dest.x - old_pos.x).abs() < f32::EPSILON
                        && (dest.y - old_pos.y).abs() < f32::EPSILON;

                    // Helper: emit 5 UnconsciousStar titbits from
                    // feet → eyes with the canonical phases.
                    let emit_stars = |mgr: &mut crate::titbit::TitbitManager,
                                      feet: crate::coordinates::WorldPoint3D,
                                      eyes: crate::coordinates::WorldPoint3D,
                                      layer: u16| {
                        let feet = crate::coordinates::WorldPoint3D {
                            x: feet.x,
                            y: feet.y,
                            z: feet.z,
                        };
                        let eyes = crate::coordinates::WorldPoint3D {
                            x: eyes.x,
                            y: eyes.y,
                            z: eyes.z,
                        };
                        let inc = crate::coordinates::WorldPoint3D {
                            x: (eyes.x - feet.x) * 0.25,
                            y: (eyes.y - feet.y) * 0.25,
                            z: (eyes.z - feet.z) * 0.25,
                        };
                        let mut p = crate::coordinates::WorldPoint3D {
                            x: feet.x - 4.0,
                            y: feet.y - 4.0,
                            z: feet.z,
                        };
                        for &phase in &[4u16, 12, 20, 12, 4] {
                            mgr.add_titbit(
                                p,
                                layer,
                                crate::titbit::TitbitKind::UnconsciousStar,
                                crate::titbit::ElementHandle::INVALID,
                                phase,
                                crate::titbit::ElementHandle::INVALID,
                                false,
                                crate::titbit::INVALID_ID,
                                false,
                                None,
                                None,
                            );
                            p.x += inc.x;
                            p.y += inc.y;
                            p.z += inc.z;
                        }
                    };

                    // The old-position star burst is gated by
                    // `bstars = !set_teleport_stuff(position_map, 20)`.
                    // `set_teleport_stuff(pt_old, 20)`:
                    //   ret = (teleport_counter > 0);
                    //   if position_before_teleport == position_map:
                    //       return ret  // already snapshot, leave counter
                    //   position_before_teleport = pt_old;
                    //   max_teleport_counter = teleport_counter = 20;
                    //   return ret;
                    // `bstars` is `true` only when no prior
                    // teleport-fade is active — a re-teleport
                    // during the 20-frame fade window suppresses
                    // the second star burst.  The render-side
                    // hulk-rebuild that consumes `teleport_counter`
                    // lives in `game_render.rs::render_entities_gpu`.
                    const TELEPORT_FADE_FRAMES: u16 = 20;
                    let mut bstars = true;
                    if is_pc
                        && let Some(entity) = self.world.entities.get_mut(owner)
                        && let Some(pc) = entity.pc_data_mut()
                    {
                        let breturn = pc.teleport_counter > 0;
                        if pc.position_before_teleport.x == old_pos.x
                            && pc.position_before_teleport.y == old_pos.y
                        {
                            // Already snapshot at this position — keep
                            // the existing counter, return prior state.
                        } else {
                            pc.position_before_teleport = old_pos;
                            pc.max_teleport_counter = TELEPORT_FADE_FRAMES;
                            pc.teleport_counter = TELEPORT_FADE_FRAMES;
                        }
                        bstars = !breturn;
                    }
                    if is_pc
                        && !zero_teleport
                        && bstars
                        && let (Some(f), Some(e)) = (old_feet, old_eyes)
                    {
                        emit_stars(
                            &mut self.feedback.titbit_manager,
                            f,
                            e,
                            dest_layer.unwrap_or(0),
                        );
                    }

                    // Probe the destination sector via
                    // `get_sector_screen_accessible`, then nudge
                    // the actor's move-box onto a walkable cell
                    // with `find_authorized_position_toward`.
                    // When either step fails the entire apply
                    // block is skipped — the actor stays put but
                    // the new-position star burst still fires.
                    let probe = self.world.fast_grid.get_sector_screen_accessible(dest);
                    let move_box = self
                        .get_entity(owner)
                        .map(|e| *e.position_iface().get_move_box());
                    let validated =
                        if let (Some(_sector_idx), Some(sector_number), Some(move_box)) =
                            (probe.sector_idx, probe.sector, move_box)
                        {
                            let mut box_at = move_box.translated(dest);
                            if self.world.fast_grid.find_authorized_position_toward(
                                &mut box_at,
                                dest,
                                probe.layer,
                            ) {
                                let dest_pt = box_at.center();
                                let sector_handle = crate::position_interface::SectorHandle::new(
                                    u16::from(sector_number),
                                );
                                Some((dest_pt, probe.layer, sector_handle, sector_number))
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                    let final_dest_layer = if let Some(v) = validated.as_ref() {
                        Some(v.1)
                    } else {
                        dest_layer
                    };

                    if let Some((
                        final_dest,
                        final_layer,
                        final_sector_handle,
                        final_sector_number,
                    )) = validated
                    {
                        // Apply new position + layer/sector and
                        // re-resolve projection/material through the
                        // same finalization path used by jump and
                        // door/lift transitions.
                        self.finalize_special_move_position(
                            assets,
                            owner,
                            super::special_motion::SpecialMovePosition::Map(final_dest),
                            Some(final_layer),
                            Some(u16::from(final_sector_number)),
                            Some(final_dest),
                            "script teleport",
                        );

                        if let Some(entity) = self.world.entities.get_mut(owner) {
                            entity.element_data_mut().set_sector(final_sector_handle);
                        }

                        // Landing in a lift sector snaps posture
                        // / action-state: LIFT_LADDER →
                        // (OnLadder, Waiting); LIFT_WALL →
                        // (OnWall, Waiting); LIFT_STAIRS leaves
                        // it alone.
                        if final_sector_handle.is_some() {
                            let lift = self.get_sector_lift_type(final_sector_number);
                            match lift {
                                Some(crate::sector::LiftType::Ladder) => {
                                    if let Some(entity) = self.world.entities.get_mut(owner) {
                                        entity.set_posture(crate::element::Posture::OnLadder);
                                        if let Some(actor) = entity.actor_data_mut() {
                                            actor.action_state =
                                                crate::element::ActionState::Waiting;
                                        }
                                    }
                                }
                                Some(crate::sector::LiftType::Wall) => {
                                    if let Some(entity) = self.world.entities.get_mut(owner) {
                                        entity.set_posture(crate::element::Posture::OnWall);
                                        if let Some(actor) = entity.actor_data_mut() {
                                            actor.action_state =
                                                crate::element::ActionState::Waiting;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }

                        // If this PC carries another PC or is
                        // being carried, copy the new position /
                        // layer / sector onto the partner so the
                        // carry link stays synced after the
                        // teleport.  Route partner snaps through the
                        // same finalizer so obstacle/material are
                        // refreshed too.
                        if is_pc {
                            let (carried, carrier) = self
                                .get_entity(owner)
                                .map(|e| {
                                    let pc = e.pc_data();
                                    let human = e.human_data();
                                    (pc.and_then(|pc| pc.carried), human.and_then(|h| h.carrier))
                                })
                                .unwrap_or((None, None));
                            for partner in [carried, carrier].into_iter().flatten() {
                                self.finalize_special_move_position(
                                    assets,
                                    partner,
                                    super::special_motion::SpecialMovePosition::Map(final_dest),
                                    Some(final_layer),
                                    Some(u16::from(final_sector_number)),
                                    Some(final_dest),
                                    "script teleport carry partner",
                                );
                                if let Some(partner_entity) = self.get_entity_mut(partner) {
                                    partner_entity
                                        .element_data_mut()
                                        .set_sector(final_sector_handle);
                                }
                            }
                        }
                    }

                    // After a layer/sector swap, refresh
                    // `update_opponents_jump_lines` for both the
                    // teleporter and any carry partner that was
                    // synced above.
                    self.update_opponents_jump_lines(assets, owner);
                    if is_pc {
                        let (carried, carrier) = self
                            .get_entity(owner)
                            .map(|e| {
                                let pc = e.pc_data();
                                let human = e.human_data();
                                (pc.and_then(|pc| pc.carried), human.and_then(|h| h.carrier))
                            })
                            .unwrap_or((None, None));
                        for partner in [carried, carrier].into_iter().flatten() {
                            self.update_opponents_jump_lines(assets, partner);
                        }
                    }

                    // New-position star burst after the snap.
                    // Gated by `is_pc && !zero_teleport &&
                    // bstars` — the same hulk-fade suppression
                    // as the old-side burst.  Fires regardless
                    // of whether the position write happened.
                    if is_pc && !zero_teleport && bstars {
                        let (new_feet, new_eyes) = match self.get_entity(owner) {
                            Some(e) => (e.compute_feet_point(), e.compute_eyes_point(None)),
                            None => (None, None),
                        };
                        if let (Some(f), Some(e)) = (new_feet, new_eyes) {
                            emit_stars(
                                &mut self.feedback.titbit_manager,
                                f,
                                e,
                                final_dest_layer.unwrap_or(0),
                            );
                        }
                    }
                }
                self.orders
                    .sequence_manager
                    .element_terminated(seq_id, elem_idx);
                // `actor_wait` parks the actor in a low-priority
                // idle element after the teleport so the AI
                // re-enters its default loop instead of resuming
                // whatever command was running before.
                self.actor_wait(owner);
            }
            Command::LockAi | Command::UnlockAi => {
                let from_lockai_command = self
                    .orders
                    .sequence_manager
                    .current_element_for_actor(owner)
                    .is_some_and(|(current_seq, current_idx)| {
                        self.orders
                            .sequence_manager
                            .get_element(current_seq, current_idx)
                            .is_some_and(|element| element.command == Command::LockAi)
                    });
                let unconscious = self
                    .get_entity(owner)
                    .and_then(|entity| entity.human_data())
                    .is_some_and(|human| human.unconscious);
                if let Some(ai) = self
                    .get_entity_mut(owner)
                    .and_then(crate::element::Entity::ai_controller_mut)
                {
                    if cmd == Command::LockAi {
                        // ScriptLockAI calls actor.Stop(NORMAL) while
                        // the previously selected command is still
                        // current. Suppress the controller's deferred
                        // halt and perform that stop synchronously
                        // below, before this zero-frame LockAI
                        // terminates and registers its successor.
                        ai.script_lock(false, true);
                    } else if ai.script_locked {
                        ai.script_unlock(unconscious);
                    }
                }
                if cmd == Command::LockAi && !from_lockai_command {
                    self.stop_owner(owner, crate::sequence::SequencePriority::Normal);
                    self.dispatch_condolations(sim, assets);
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
        display: &mut HostDisplayState,
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
                PresentationCommandContext {
                    display,
                    fast_forward: self.control.fast_forward,
                    side_effects: &mut self.feedback.pending_side_effects,
                    messenger: &mut self.orders.messenger,
                    sequence_manager: &mut self.orders.sequence_manager,
                }
                .dispatch(command, seq_id, elem_idx);
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
