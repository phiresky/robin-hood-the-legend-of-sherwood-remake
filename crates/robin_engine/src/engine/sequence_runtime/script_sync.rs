use super::*;

impl EngineInner {
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
                if !self.arbitrate_instruct(sequence_id, element_index) {
                    return Ok(());
                }
                // `launch_element_for_owner` already stamped and generated a
                // transition. A recorded WAIT reaches us with Undefined and
                // needs the same one-time work as the normal hourglass path.
                if needs_stamp && !self.generate_transition(owner, sequence_id, element_index) {
                    self.orders
                        .sequence_manager
                        .element_impossible(sequence_id, element_index);
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
                if matches!(
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
                    .dispatch(owner, command, sequence_id, element_index);
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
                    .dispatch(owner, command, sequence_id, element_index);
                } else {
                    return Err(format!(
                        "unsupported synchronous owner command {command:?} at {sequence_id:?}/{element_index}"
                    )
                    .into());
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
                    result?;
                } else {
                    let mut messages = Vec::new();
                    self.dispatch_execute_immediate_owner(
                        assets,
                        owner,
                        sequence_id,
                        element_index,
                        &mut messages,
                    );
                    debug_assert!(messages.is_empty());
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
