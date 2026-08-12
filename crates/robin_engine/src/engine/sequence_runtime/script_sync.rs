use super::*;

impl EngineInner {
    /// Close only the part of sequence registration that Original executes
    /// on the current script callback stack.
    ///
    /// Immediate commands and RHPRIORITY_WAIT successors recurse inline.
    /// Ordinary owner/engine commands remain queued for
    /// SequenceManager::Hourglass, even though the script VM has returned.
    pub(in crate::engine) fn drain_script_registration_inline_actions(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
    ) -> Result<(), crate::engine::script::ScriptDriverError> {
        while let Some(action) = self
            .orders
            .sequence_manager
            .pop_pending_registration_inline_action()
        {
            let failed_element = synchronous_action_element_ref(&action);
            let continuation = self
                .orders
                .sequence_manager
                .take_pending_synchronous_actions();
            let dispatch_result =
                self.dispatch_script_synchronous_action(sim, assets, action, active_scripts);
            let result = match dispatch_result {
                Ok(()) => {
                    self.drain_script_registration_inline_actions(sim, assets, active_scripts)
                }
                Err(mut error) => {
                    if !error.sequence_element_failed {
                        if let Some((sequence_id, element_index)) = failed_element {
                            self.orders
                                .sequence_manager
                                .element_impossible(sequence_id, element_index);
                        }
                        error.sequence_element_failed = true;
                    }
                    Err(error)
                }
            };
            self.orders
                .sequence_manager
                .restore_pending_synchronous_actions(continuation);
            result?;
        }
        Ok(())
    }

    /// Drain the sequence work emitted by a script-native callback as a
    /// recursive stack, not a flat FIFO. Each action temporarily detaches its
    /// older siblings; successors and nested callbacks therefore finish before
    /// control returns to the next sibling, matching `Go()` in the original.
    pub(in crate::engine) fn drain_script_synchronous_actions(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
    ) -> Result<(), crate::engine::script::ScriptDriverError> {
        while let Some(action) = self.orders.sequence_manager.pop_pending_immediate_action() {
            let failed_element = synchronous_action_element_ref(&action);
            let continuation = self
                .orders
                .sequence_manager
                .take_pending_synchronous_actions();
            let dispatch_result =
                self.dispatch_script_synchronous_action(sim, assets, action, active_scripts);
            let result = match dispatch_result {
                Ok(()) => self.drain_script_synchronous_actions(sim, assets, active_scripts),
                Err(mut error) => {
                    if !error.sequence_element_failed {
                        if let Some((sequence_id, element_index)) = failed_element {
                            self.orders
                                .sequence_manager
                                .element_impossible(sequence_id, element_index);
                        }
                        error.sequence_element_failed = true;
                    }
                    Err(error)
                }
            };
            self.orders
                .sequence_manager
                .restore_pending_synchronous_actions(continuation);
            result?;
        }
        Ok(())
    }

    /// Execute an action whose parent tail was detached by the yielding
    /// native. Child work drains against an empty queue; the parent tail is
    /// restored on every success and error path.
    pub(in crate::engine) fn drive_detached_sequence_operation(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        operation: crate::interp::SynchronousSequenceOperation,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
    ) -> Result<(), crate::engine::script::ScriptDriverError> {
        let failed_element = synchronous_action_element_ref(&operation.action);
        let dispatch_result =
            self.dispatch_script_synchronous_action(sim, assets, operation.action, active_scripts);
        let result = match dispatch_result {
            Ok(()) => self.drain_script_synchronous_actions(sim, assets, active_scripts),
            Err(mut error) => {
                if !error.sequence_element_failed {
                    if let Some((sequence_id, element_index)) = failed_element {
                        self.orders
                            .sequence_manager
                            .element_impossible(sequence_id, element_index);
                    }
                    error.sequence_element_failed = true;
                }
                Err(error)
            }
        };
        self.orders
            .sequence_manager
            .restore_pending_synchronous_actions(operation.continuation);
        result?;
        self.drain_script_synchronous_actions(sim, assets, active_scripts)
    }

    pub(in crate::engine) fn dispatch_script_synchronous_action(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        action: crate::sequence::SequenceAction,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
    ) -> Result<(), crate::engine::script::ScriptDriverError> {
        use crate::sequence::SequenceAction;

        match action {
            SequenceAction::InstructOwner {
                owner,
                sequence_id,
                element_index,
            } => {
                let needs_stamp = self
                    .orders
                    .sequence_manager
                    .get_element(sequence_id, element_index)
                    .is_some_and(|element| {
                        element.posture_after_transition == crate::element::Posture::Undefined
                    });
                if needs_stamp {
                    self.stamp_element_transition_state(owner, sequence_id, element_index);
                }
                if needs_stamp {
                    // RHElementActor::Instruct checks a selected
                    // NON_INTERRUPTABLE element before transition generation,
                    // then generates transitions before ordinary priority
                    // arbitration. WAIT-priority Go reaches this synchronous
                    // dispatcher directly at registration, so it must use the
                    // same admission order as the manager-Hourglass path.
                    if self.non_interruptable_guard(owner, sequence_id, element_index) {
                        return Ok(());
                    }
                    if !self.generate_transition(sim, assets, owner, sequence_id, element_index) {
                        self.orders
                            .sequence_manager
                            .element_impossible(sequence_id, element_index);
                        return Ok(());
                    }
                }
                let resolved_priority = {
                    let element = self
                        .orders
                        .sequence_manager
                        .get_element(sequence_id, element_index)
                        .ok_or_else(|| {
                            format!(
                                "missing synchronous owner element {sequence_id:?}/{element_index} before priority resolution"
                            )
                        })?;
                    let resolver = Self::priority_resolver(&self.world.entities);
                    resolver(element)
                };
                if let Some(element) = self
                    .orders
                    .sequence_manager
                    .get_element_mut(sequence_id, element_index)
                    && element.priority == crate::sequence::SequencePriority::NotYetSet
                {
                    // Actor::Instruct resolves priority after transition
                    // generation and before comparing against the selected
                    // element. Synchronous WAIT/native continuations must not
                    // reach arbitration with the NotYetSet fallback.
                    element.priority = resolved_priority;
                }
                if !self.arbitrate_instruct(sequence_id, element_index) {
                    return Ok(());
                }
                let command = self
                    .orders
                    .sequence_manager
                    .get_element(sequence_id, element_index)
                    .map(|element| element.command)
                    .ok_or_else(|| {
                        format!("missing synchronous owner element {sequence_id:?}/{element_index}")
                    })?;
                let barrier = if command == Command::Move {
                    self.dispatch_synchronous_move_instruct(
                        sim,
                        assets,
                        owner,
                        sequence_id,
                        element_index,
                    )?
                } else if command == Command::Seek {
                    // `SetState(TERMINATED)` calls Ready() and then
                    // StartPostponedSequenceElement() on the same C++ stack.
                    // A released Seek therefore reaches the ordinary owner
                    // Translate path before the outer condolence boundary
                    // returns. Reuse that complete Seek translation here,
                    // including live-target refresh and post-seek transfer.
                    self.dispatch_ordered_move_seek_instruct(
                        sim,
                        assets,
                        owner,
                        sequence_id,
                        element_index,
                    )
                } else if command == Command::QuitSwordfight {
                    // A GoTo issued from a sword action state is one
                    // Original sequence: QuitSwordfight followed by Move.
                    // Condolence and waypoint-script owner boundaries drive
                    // the first element synchronously, just like the normal
                    // hourglass dispatcher; the movement remains behind the
                    // lowering animation until this element completes.
                    self.dispatch_quit_swordfight(sim, assets, owner, sequence_id, element_index)
                } else if matches!(
                    command,
                    Command::EnterSwordfight | Command::PrepareSwordfight
                ) {
                    // `Ready() -> Go() -> Instruct()` is re-entrant in the
                    // Original. A terminal owner card may therefore promote
                    // an EnterSwordfight successor inside the same actor
                    // Hourglass slot; use the ordinary command dispatcher,
                    // but do not defer it to SequenceManager::Hourglass.
                    let opponent = self
                        .orders
                        .sequence_manager
                        .get_element(sequence_id, element_index)
                        .and_then(|element| element.get_property(crate::sequence::Field::Opponent))
                        .and_then(|value| match value {
                            crate::sequence::FieldValue::Element(id) => Some(*id),
                            _ => None,
                        });
                    self.dispatch_enter_swordfight(
                        sim,
                        assets,
                        owner,
                        opponent,
                        sequence_id,
                        element_index,
                    )
                } else if matches!(
                    command,
                    Command::SwordstrikeSmalltalkLeft
                        | Command::SwordstrikeSmalltalkRight
                        | Command::ParrySmalltalkLeft
                        | Command::ParrySmalltalkRight
                ) {
                    SmalltalkCommandContext {
                        entities: &self.world.entities,
                        sequence_manager: &mut self.orders.sequence_manager,
                        next_order_id: &mut self.orders.next_order_id,
                    }
                    .dispatch(owner, command, sequence_id, element_index)
                } else if command == Command::Provoke {
                    // A sword-movement TERMINATED callback registers this
                    // Wait-priority successor before DoNextOrder. Original
                    // Ready/Go/Instruct resumes it synchronously when the
                    // movement closes, so use the ordinary Provoke
                    // translator at this owner boundary.
                    self.dispatch_provoke(sim, assets, owner, sequence_id, element_index);
                    OwnerActionBarrier::Reach
                } else if command == Command::AssertPosition {
                    self.orders.sequence_manager.begin_instruct_callback(
                        owner,
                        sequence_id,
                        element_index,
                    );
                    let barrier = PositionAssertionContext {
                        entities: &self.world.entities,
                        sequence_manager: &mut self.orders.sequence_manager,
                    }
                    .dispatch(owner, sequence_id, element_index);
                    self.orders.sequence_manager.end_instruct_callback(
                        owner,
                        sequence_id,
                        element_index,
                    );
                    barrier
                } else if matches!(command, Command::Turn | Command::TurnFast) {
                    // A terminal condolation can synchronously resume a
                    // postponed Face Turn through Ready -> Go -> Instruct.
                    // Use the same translator as the ordinary hourglass path
                    // so its stored direction is applied only now, when the
                    // Turn has actually won arbitration.
                    TurnCommandContext {
                        entities: &mut self.world.entities,
                        sequence_manager: &mut self.orders.sequence_manager,
                        next_order_id: &mut self.orders.next_order_id,
                    }
                    .dispatch(owner, command, sequence_id, element_index)
                } else if matches!(
                    command,
                    Command::ParrySword | Command::ParrySwordLow | Command::StopParrySword
                ) {
                    match command {
                        Command::ParrySword => {
                            self.dispatch_parry_sword(owner, false, sequence_id, element_index)
                        }
                        Command::ParrySwordLow => {
                            self.dispatch_parry_sword(owner, true, sequence_id, element_index)
                        }
                        Command::StopParrySword => {
                            self.dispatch_stop_parry(owner, sequence_id, element_index)
                        }
                        _ => unreachable!(),
                    }
                } else if matches!(
                    command,
                    Command::LookLeft | Command::LookRight | Command::LeanOut
                ) {
                    // A completed attentive animation can release the next
                    // look booking through the same Ready -> Go -> Instruct
                    // stack. Reuse the normal translator so the actor's live
                    // attentive flag selects the correct alerted row.
                    NpcAttentionCommandContext {
                        entities: &mut self.world.entities,
                        sequence_manager: &mut self.orders.sequence_manager,
                        next_order_id: &mut self.orders.next_order_id,
                    }
                    .dispatch(owner, command, sequence_id, element_index)
                } else if matches!(
                    command,
                    Command::EnterAttentiveMode
                        | Command::LeaveAttentiveMode
                        | Command::LeaveAttentiveModeOfficer
                ) {
                    // A terminal owner card can synchronously resume an
                    // attentive-mode successor through Ready -> Go ->
                    // Instruct. Keep that re-entrant path on the same
                    // translator as the ordinary manager hourglass.
                    NpcAttentionCommandContext {
                        entities: &mut self.world.entities,
                        sequence_manager: &mut self.orders.sequence_manager,
                        next_order_id: &mut self.orders.next_order_id,
                    }
                    .dispatch(owner, command, sequence_id, element_index)
                } else if matches!(
                    command,
                    Command::Wait | Command::WaitTimer | Command::WaitFreeLift
                ) {
                    WaitCommandContext {
                        entities: &mut self.world.entities,
                        sequence_manager: &mut self.orders.sequence_manager,
                        next_order_id: &mut self.orders.next_order_id,
                        profiles: &assets.profile_manager,
                    }
                    .dispatch(owner, command, sequence_id, element_index)
                } else if matches!(
                    command,
                    Command::ReceiveSwordDamage
                        | Command::ReceiveDamage
                        | Command::ReceiveArrowDamage
                        | Command::ReceiveStoneDamage
                        | Command::ReceiveHitDamage
                        | Command::ReceiveMobileDamage
                        | Command::ReceiveNet
                ) {
                    // Ready()/StartPostponedSequenceElement() is re-entrant
                    // in Original. A damage element released by the
                    // terminating attack can therefore reach the victim's
                    // Instruct/Translate callback before the attacker's
                    // condolence stack returns. Use the same damage
                    // translator as the ordinary manager Hourglass path.
                    self.dispatch_receive_damage(sim, assets, owner, sequence_id, element_index)
                } else {
                    return Err(format!(
                        "unsupported synchronous owner command {command:?} at {sequence_id:?}/{element_index}"
                    )
                    .into());
                };
                if barrier == OwnerActionBarrier::Skip {
                    return Ok(());
                }
                // Accepted Actor::Instruct assigns mpOrder after Translate
                // and any zero-frame completion cascade has settled. Motion
                // is stamped from this accepted boundary rather than final
                // element state: an empty translated order list terminates
                // only after Original has written IN_PROGRESS.
                self.publish_selected_order_for_instruct_owner(owner);
                if let Some(actor) = self
                    .world
                    .entities
                    .get_mut(owner)
                    .and_then(Entity::actor_data_mut)
                {
                    actor.continuation.motion_state = crate::sprite::MotionState::InProgress;
                }
            }
            SequenceAction::EngineCommand {
                sequence_id,
                element_index,
            } => {
                let command = self
                    .orders
                    .sequence_manager
                    .get_element(sequence_id, element_index)
                    .map(|element| element.command)
                    .ok_or_else(|| {
                        format!(
                            "missing synchronous engine element {sequence_id:?}/{element_index}"
                        )
                    })?;
                return Err(format!(
                    "unsupported synchronous engine command {command:?} at {sequence_id:?}/{element_index}"
                )
                .into());
            }
            SequenceAction::ExecuteImmediateOwner {
                owner,
                sequence_id,
                element_index,
            } => {
                let command = self
                    .orders
                    .sequence_manager
                    .get_element(sequence_id, element_index)
                    .map(|element| element.command)
                    .ok_or_else(|| {
                        format!("missing immediate owner element {sequence_id:?}/{element_index}")
                    })?;
                if command == Command::SendMessage {
                    let (message, arg1, arg2) =
                        self.extract_message_properties(sequence_id, element_index);
                    let handle = crate::natives::ScriptHandleCodec::actor_handle(owner);
                    let frame = active_scripts
                        .last()
                        .map_or_else(crate::natives::ScriptCallFrame::default, |call| call.frame)
                        .with_script_this(handle);
                    let result = self.call_script_vm_inner(
                        sim,
                        assets,
                        crate::engine::ScriptVmKey::Actor(handle),
                        "ProcessMessage",
                        &[message, arg1, arg2],
                        frame,
                        active_scripts,
                    );
                    self.orders
                        .sequence_manager
                        .element_terminated(sequence_id, element_index);
                    // `RHElementActor::ExecuteImmediately` returns from
                    // ProcessMessage and immediately enters SetState, whose
                    // owner card and Ready() complete before the parent VM
                    // resumes. Keep the active call stack while closing it.
                    self.dispatch_condolations_in_script_driver(sim, assets, active_scripts)?;
                    result?;
                } else {
                    let message = self.dispatch_execute_immediate_owner(
                        sim,
                        assets,
                        owner,
                        sequence_id,
                        element_index,
                    );
                    debug_assert!(message.is_none());
                }
            }
            SequenceAction::ExecuteImmediateEngine {
                sequence_id,
                element_index,
            } => self.dispatch_script_immediate_engine(
                sim,
                assets,
                sequence_id,
                element_index,
                active_scripts,
            )?,
        }
        Ok(())
    }

    /// Apply the deterministic half of RHEngine/RHGame's MSG_LOCK_USER /
    /// MSG_UNLOCK_USER handling. Selection and current actions are part of
    /// simulation state; only physical input cleanup remains a host effect.
    pub(super) fn apply_script_user_lock(&mut self, assets: &LevelAssets, command: Command) {
        match command {
            Command::LockUser => {
                self.players.user_locked = true;
                self.feedback
                    .pending_side_effects
                    .invalidate_trajectory_preview = true;
                self.players.selection_before_user_lock = self.players.seats[0].selection.clone();
                for pc_id in self.players.seats[0].selection.clone() {
                    self.unselect_action(pc_id);
                    let pc = self
                        .get_entity_mut(pc_id)
                        .and_then(|entity| entity.pc_data_mut())
                        .expect("selected LockUser entity is not a PC");
                    pc.current_action = crate::profiles::Action::NoAction;
                }
                self.unselect_all_pcs(0);
            }
            Command::UnlockUser => {
                self.players.user_locked = false;
                for pc_id in self.players.selection_before_user_lock.clone() {
                    self.select_pc(assets, 0, pc_id, true, false);
                }
                self.feedback.pending_side_effects.pending_reset_input = true;
            }
            _ => unreachable!("apply_script_user_lock received {command:?}"),
        }
    }

    /// Translate one exact plain Move using the same path/outcome pipeline as
    /// the ordinary sequence hourglass. This is intentionally handle-based:
    /// the native boundary must not consume older same-owner Move elements.
    fn dispatch_synchronous_move_instruct(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        sequence_id: crate::sequence::SequenceId,
        element_index: usize,
    ) -> Result<OwnerActionBarrier, crate::engine::script::ScriptDriverError> {
        let (destination, move_action) = self
            .orders
            .sequence_manager
            .get_element(sequence_id, element_index)
            .and_then(|element| match &element.data {
                crate::sequence::SequenceElementData::Movement {
                    destination,
                    action,
                    ..
                } if element.command == Command::Move => Some((*destination, *action)),
                _ => None,
            })
            .ok_or_else(|| {
                format!("synchronous Move element {sequence_id:?}/{element_index} has invalid data")
            })?;

        if self.beggar_rejects_command(owner, Command::Move) {
            self.orders
                .sequence_manager
                .element_impossible(sequence_id, element_index);
            return Ok(OwnerActionBarrier::Skip);
        }
        let is_anonymous_archer_pc = self.get_entity(owner).is_some_and(|entity| {
            entity.is_pc()
                && entity.element_data().posture == crate::element_kinds::Posture::AnonymousArcher
        });
        if is_anonymous_archer_pc {
            self.hero_speaking(
                assets,
                owner,
                crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
            );
            self.orders
                .sequence_manager
                .element_impossible(sequence_id, element_index);
            return Ok(OwnerActionBarrier::Skip);
        }

        // Keep the synchronous native entry point on the same Instruct
        // boundary as the ordinary sequence-manager path.
        self.apply_map_move_instruction_side_effect(owner, sequence_id, element_index);

        let owner_sector = self
            .get_entity(owner)
            .and_then(|entity| entity.element_data().sector());
        if self.sector_is_building(owner_sector) {
            self.finalize_special_move_position(
                assets,
                owner,
                super::special_motion::SpecialMovePosition::Map(destination),
                None,
                None,
                Some(destination),
                "building interior move",
            );
            self.orders
                .sequence_manager
                .element_terminated(sequence_id, element_index);
            return Ok(OwnerActionBarrier::Skip);
        }

        Ok(self.dispatch_prepared_move_instruction(
            sim,
            assets,
            owner,
            sequence_id,
            element_index,
            destination,
            move_action,
        ))
    }

    fn dispatch_script_immediate_engine(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        sequence_id: crate::sequence::SequenceId,
        element_index: usize,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
    ) -> Result<(), crate::engine::script::ScriptDriverError> {
        let command = self
            .orders
            .sequence_manager
            .get_element(sequence_id, element_index)
            .map(|element| element.command)
            .ok_or_else(|| {
                format!("missing immediate engine element {sequence_id:?}/{element_index}")
            })?;
        match command {
            Command::SendMessage => {
                let (message, arg1, arg2) =
                    self.extract_message_properties(sequence_id, element_index);
                let frame = active_scripts
                    .last()
                    .map_or_else(crate::natives::ScriptCallFrame::default, |call| call.frame);
                let result = self.call_script_vm_inner(
                    sim,
                    assets,
                    crate::engine::ScriptVmKey::Global,
                    "ProcessMessage",
                    &[message, arg1, arg2],
                    frame,
                    active_scripts,
                );
                self.orders
                    .sequence_manager
                    .element_terminated(sequence_id, element_index);
                result?;
            }
            command @ (Command::LockUser | Command::UnlockUser) => {
                self.apply_script_user_lock(assets, command);
                self.orders
                    .sequence_manager
                    .element_terminated(sequence_id, element_index);
            }
            Command::Timer => {
                let timer = TimerImmediateContext {
                    sequence_manager: &self.orders.sequence_manager,
                }
                .entry(sequence_id, element_index);
                self.add_timer(timer.remaining, timer.element_ref);
            }
            Command::CameraJumpTo => {
                self.terminate_prev_camera_sequence_element();
                self.players.seats[0].follow_element = None;
                self.players.seats[0].locker_active = false;
                let point = self
                    .orders
                    .sequence_manager
                    .get_element(sequence_id, element_index)
                    .and_then(|element| {
                        read_sequence_map_point_property(
                            element,
                            crate::sequence::Field::CameraPoint,
                        )
                    });
                if let Some(position) = point {
                    self.feedback.cutscene_camera.view_position =
                        self.check_location_is_valid_for_camera(position);
                    self.feedback.pending_side_effects.invalidate_background = true;
                }
                self.orders
                    .sequence_manager
                    .element_terminated(sequence_id, element_index);
            }
            command @ (Command::CharacterAvailable | Command::ActionAvailable) => {
                AvailabilityImmediateContext {
                    entities: &mut self.world.entities,
                    messenger: &mut self.orders.messenger,
                    sequence_manager: &mut self.orders.sequence_manager,
                }
                .dispatch(command, sequence_id, element_index);
            }
            Command::OpenScroll => {
                let (scroll, reader) = {
                    let element = self
                        .orders
                        .sequence_manager
                        .get_element(sequence_id, element_index);
                    let scroll = element
                        .and_then(|element| element.get_property(crate::sequence::Field::Scroll))
                        .and_then(|value| match value {
                            crate::sequence::FieldValue::Element(value) => Some(*value),
                            _ => None,
                        });
                    let reader = element
                        .and_then(|element| {
                            element.get_property(crate::sequence::Field::ScrollReader)
                        })
                        .and_then(|value| match value {
                            crate::sequence::FieldValue::Element(value) => Some(*value),
                            _ => None,
                        });
                    (scroll, reader)
                };
                if let (Some(scroll), Some(reader)) = (scroll, reader) {
                    let result = self.scroll_is_taken_in_script_driver(
                        sim,
                        assets,
                        scroll,
                        reader,
                        active_scripts,
                    );
                    match result {
                        Ok(_) => {
                            self.orders
                                .sequence_manager
                                .element_terminated(sequence_id, element_index);
                        }
                        Err(error) if error.sequence_element_failed => {
                            // IsTaken dispatched successfully and a nested
                            // sequence element owns the failure. Match
                            // SendMessage: terminate this ancestor before
                            // propagating so only the actual child is
                            // Impossible.
                            self.orders
                                .sequence_manager
                                .element_terminated(sequence_id, element_index);
                            return Err(error);
                        }
                        Err(error) => {
                            // The OpenScroll/IsTaken dispatch itself failed.
                            // Leave it live so the outer action drain marks
                            // this element Impossible without advancing its
                            // sequence to a successor.
                            return Err(error);
                        }
                    }
                } else {
                    tracing::warn!(?scroll, ?reader, "OpenScroll missing properties");
                    self.orders
                        .sequence_manager
                        .element_terminated(sequence_id, element_index);
                }
            }
            other => {
                return Err(format!(
                    "non-immediate command {other:?} entered synchronous engine dispatcher"
                )
                .into());
            }
        }
        Ok(())
    }
}
