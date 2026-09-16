use super::*;

#[cfg(test)]
mod resumed_instruction_tests {
    use super::*;

    #[test]
    fn resumed_run_restamps_sword_state_before_selecting_movement_animation() {
        use crate::coordinates::MapPoint;
        use crate::element::{ActionState, Posture};
        use crate::order::OrderType;
        use crate::sequence::{
            MoveFlags, SequenceAction, SequenceElement, SequenceElementData, SequencePriority,
        };

        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        engine.world.fast_grid_mut().size_map(128, 128);
        engine.world.fast_grid_mut().allocate_layers(1);
        let sector_index = engine.world.fast_grid_mut().add_sector(
            crate::engine::test_support::square_sector(
                1,
                0,
                MapPoint::new(0.0, 0.0),
                MapPoint::new(1000.0, 1000.0),
            ),
            0,
        );
        let sector = crate::position_interface::SectorHandle::new(1)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(sector_index).unwrap());
        let owner = engine.add_test_entity(crate::engine::test_support::actors::make_test_pc(
            Posture::Upright,
        ));
        let entity = engine.get_entity_mut(owner).unwrap();
        entity
            .element_data_mut()
            .set_position(crate::coordinates::WorldPoint3D::new(100.0, 100.0, 0.0));
        entity.element_data_mut().set_sector(Some(sector));
        entity
            .position_iface_mut()
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -4.0, -4.0, 4.0, 4.0,
            ));
        entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);

        let mut movement =
            SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::RunningUpright);
        movement.priority = SequencePriority::Normal;
        movement.posture_after_transition = Posture::Upright;
        movement.action_state_after_transition = ActionState::Waiting;
        let SequenceElementData::Movement {
            destination, flags, ..
        } = &mut movement.data
        else {
            unreachable!()
        };
        *destination = MapPoint::new(120.0, 100.0);
        flags.insert(MoveFlags::NO_TRANSITIONS);
        let sim = crate::sim_rng::test_context();
        let sequence = engine.launch_element(&sim, &assets, movement);
        engine.postpone_element(&sim, &assets, &mut Vec::new(), sequence, 0);

        engine
            .dispatch_script_synchronous_action(
                &sim,
                &assets,
                SequenceAction::InstructOwner {
                    owner,
                    sequence_id: sequence,
                    element_index: 0,
                },
                &mut Vec::new(),
            )
            .unwrap();

        let movement = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap();
        assert_eq!(
            movement.action_state_after_transition,
            ActionState::WaitingSword
        );
        assert_eq!(
            movement.current_order().unwrap().order_type,
            OrderType::RunningWithSword
        );
        assert_eq!(
            engine.actor_order_type(owner),
            Some(OrderType::RunningWithSword)
        );
    }
}

impl EngineInner {
    pub(crate) fn dispatch_script_synchronous_action(
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
                self.instruct_owner(
                    sim,
                    assets,
                    active_scripts,
                    owner,
                    sequence_id,
                    element_index,
                );
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
                    self.element_terminated(
                        sim,
                        assets,
                        active_scripts,
                        sequence_id,
                        element_index,
                    );
                    // Immediate original-game actor execution returns from
                    // message processing and immediately enters state change, whose
                    // owner card and Ready() complete before the parent VM
                    // resumes. Keep the active call stack while closing it.

                    result?;
                } else {
                    self.dispatch_execute_immediate_owner(
                        sim,
                        assets,
                        active_scripts,
                        owner,
                        sequence_id,
                        element_index,
                    );
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

    /// Apply lock-user selection and action dispatch. Selection and current actions are part of
    /// simulation state; only physical input cleanup remains a host effect.
    pub(super) fn apply_script_user_lock(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        command: Command,
    ) {
        match command {
            Command::LockUser => {
                self.players.user_locked = true;
                self.feedback
                    .pending_side_effects
                    .request_signal(crate::engine::HostSignal::InvalidateTrajectoryPreview);
                self.players.selection_before_user_lock = self.players.seats[0].selection.clone();
                if let Some(pc_id) = self.players.seats[0].selection.first().copied() {
                    self.set_pc_action_from_message(
                        sim,
                        assets,
                        0,
                        pc_id,
                        crate::profiles::Action::NoAction,
                    );
                }
                self.unselect_all_pcs(0);
            }
            Command::UnlockUser => {
                self.players.user_locked = false;
                let selected_count = self.players.selection_before_user_lock.len();
                for index in 0..selected_count {
                    let pc_id = self.players.selection_before_user_lock[index];
                    self.select_pc(sim, assets, 0, pc_id, true, false);
                }
                self.feedback
                    .pending_side_effects
                    .request_signal(crate::engine::HostSignal::ResetInput);
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
                self.element_terminated(sim, assets, active_scripts, sequence_id, element_index);
                result?;
            }
            command @ (Command::LockUser | Command::UnlockUser) => {
                self.apply_script_user_lock(sim, assets, command);
                self.element_terminated(sim, assets, active_scripts, sequence_id, element_index);
            }
            Command::Timer => {
                let timer = self.timer_immediate_entry(sequence_id, element_index);
                self.add_timer(timer.remaining, timer.element_ref);
            }
            Command::CameraJumpTo => {
                self.terminate_prev_camera_sequence_element(sim, assets);
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
                }
                self.element_terminated(sim, assets, active_scripts, sequence_id, element_index);
            }
            command @ (Command::CharacterAvailable | Command::ActionAvailable) => {
                self.dispatch_availability_immediate(
                    sim,
                    assets,
                    active_scripts,
                    command,
                    sequence_id,
                    element_index,
                );
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
                            self.element_terminated(
                                sim,
                                assets,
                                active_scripts,
                                sequence_id,
                                element_index,
                            );
                        }
                        Err(error) if error.sequence_element_failed => {
                            // IsTaken dispatched successfully and a nested
                            // sequence element owns the failure. Match
                            // SendMessage: terminate this ancestor before
                            // propagating so only the actual child is
                            // Impossible.
                            self.element_terminated(
                                sim,
                                assets,
                                active_scripts,
                                sequence_id,
                                element_index,
                            );
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
                    self.element_terminated(
                        sim,
                        assets,
                        active_scripts,
                        sequence_id,
                        element_index,
                    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_lock_during_recording_clears_action_without_unequipping_live_bow() {
        use crate::element::{ActionState, ActorPc, ElementData, Entity, Posture};
        use crate::profiles::Action;

        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(Entity::Pc(ActorPc {
            element: ElementData::from_initial_posture(Posture::Upright),
            actor: crate::element::ActorData {
                action_state: ActionState::AimingWithBow,
                ..Default::default()
            },
            human: Default::default(),
            pc: crate::element::PcData {
                current_action: Action::Bow,
                ..Default::default()
            },
        }));
        engine.players.seats[0].selection.push(owner);
        engine.players.seats[0].selected_action = Action::Bow;
        engine
            .players
            .macro_store
            .get_or_insert(owner)
            .begin_recording(0);
        engine.players.qa_recording_for.push(owner);

        engine.apply_script_user_lock(
            &crate::sim_rng::test_context(),
            &LevelAssets::default(),
            Command::LockUser,
        );

        assert!(engine.players.user_locked);
        assert_eq!(engine.players.selection_before_user_lock, [owner]);
        assert!(engine.players.seats[0].selection.is_empty());
        assert_eq!(engine.players.seats[0].selected_action, Action::NoAction);
        let pc = engine.get_entity(owner).unwrap();
        assert_eq!(pc.pc_data().unwrap().current_action, Action::NoAction);
        assert_eq!(
            pc.actor_data().unwrap().action_state,
            ActionState::AimingWithBow
        );
        assert!(
            !engine
                .orders
                .sequence_manager
                .queued_element_exists(owner, Command::UnequipBow)
        );
        assert!(
            engine
                .feedback
                .pending_side_effects
                .has_signal(crate::engine::HostSignal::CancelMultiSelection)
        );
    }
}
