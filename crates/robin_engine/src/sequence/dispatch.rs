//! Registration dispatch and the live manager FIFO.
use super::*;
use crate::engine::TickCtx;

impl SequenceManager {
    pub(crate) fn start_sequence_level(&mut self, id: SequenceId) -> Vec<usize> {
        self.sequences
            .get_mut(&id)
            .expect("starting missing sequence")
            .next_elements_go()
    }
    pub(super) fn immediate_action_for(
        seq_id: SequenceId,
        elem_idx: usize,
        elem: &SequenceElement,
    ) -> Option<SequenceAction> {
        match elem.command {
            // Owner-only group: must dispatch to owner.
            Command::Teleport
            | Command::LockAi
            | Command::UnlockAi
            | Command::ReplaceAnim
            | Command::RestoreAnim
            | Command::Speak
            | Command::StartMobile
            | Command::StopMobile
            | Command::ActivateMobile
            | Command::DeactivateMobile
            | Command::Unblip => Some(SequenceAction::ExecuteImmediateOwner {
                owner: elem.owner?,
                sequence_id: seq_id,
                element_index: elem_idx,
            }),
            // Engine-only group: dispatch to engine regardless of owner.
            Command::LockUser
            | Command::UnlockUser
            | Command::CameraJumpTo
            | Command::Timer
            | Command::ActionAvailable
            | Command::CharacterAvailable
            | Command::OpenScroll => Some(SequenceAction::ExecuteImmediateEngine {
                sequence_id: seq_id,
                element_index: elem_idx,
            }),
            // SendMessage: owner if present, else engine.
            Command::SendMessage => Some(match elem.owner {
                Some(owner) => SequenceAction::ExecuteImmediateOwner {
                    owner,
                    sequence_id: seq_id,
                    element_index: elem_idx,
                },
                None => SequenceAction::ExecuteImmediateEngine {
                    sequence_id: seq_id,
                    element_index: elem_idx,
                },
            }),
            _ => None,
        }
    }

    pub(crate) fn pop_next_hourglass_action(&mut self) -> Option<SequenceAction> {
        self.pop_deferred_hourglass_action()
    }

    pub(super) fn pop_deferred_hourglass_action(&mut self) -> Option<SequenceAction> {
        loop {
            let (seq_id, elem_idx) = self.elements_to_go.pop_front()?;
            // Validate the sequence still exists
            let Some(seq) = self.sequences.get(&seq_id) else {
                continue;
            };
            if elem_idx >= seq.elements.len() {
                continue;
            }

            let elem = &seq.elements[elem_idx];

            // Only process elements that are still Todo or Postponed
            match elem.state {
                SequenceState::Todo | SequenceState::Postponed => {}
                _ => continue,
            }

            if elem.executed_immediately() {
                if let Some(action) = Self::immediate_action_for(seq_id, elem_idx, elem) {
                    return Some(action);
                } else {
                    tracing::warn!(
                        ?seq_id,
                        elem_idx,
                        command = ?elem.command,
                        owner = ?elem.owner,
                        "owner-only immediate command has no owner — terminating"
                    );
                    return Some(SequenceAction::EngineCommand {
                        sequence_id: seq_id,
                        element_index: elem_idx,
                    });
                }
            } else if let Some(owner) = elem.owner {
                return Some(SequenceAction::InstructOwner {
                    owner,
                    sequence_id: seq_id,
                    element_index: elem_idx,
                });
            } else {
                return Some(SequenceAction::EngineCommand {
                    sequence_id: seq_id,
                    element_index: elem_idx,
                });
            }
        }
    }
}

impl crate::engine::EngineInner {
    pub(crate) fn launch_sequence_inline(
        &mut self,
        tcx: TickCtx<'_>,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        sequence: Sequence,
    ) -> Result<SequenceId, crate::engine::script::ScriptDriverError> {
        let id = self.orders.sequence_manager.insert_sequence(sequence);
        self.start_sequence_inline(tcx, active_scripts, id)?;
        Ok(id)
    }

    pub(crate) fn start_sequence_inline(
        &mut self,
        tcx: TickCtx<'_>,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        id: SequenceId,
    ) -> Result<(), crate::engine::script::ScriptDriverError> {
        let level = self.orders.sequence_manager.start_sequence_level(id);
        self.register_sequence_level(tcx, active_scripts, id, level)
    }

    pub(crate) fn launch_element_inline(
        &mut self,
        tcx: TickCtx<'_>,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        mut element: SequenceElement,
    ) -> Result<SequenceId, crate::engine::script::ScriptDriverError> {
        element.command_level = 1;
        let mut sequence = Sequence::new();
        sequence.append_element(element);
        self.launch_sequence_inline(tcx, active_scripts, sequence)
    }

    /// The level boundary stays fixed across callbacks; each sibling is read
    /// again only after the preceding sibling has completed its registration.
    pub(crate) fn register_sequence_level(
        &mut self,
        tcx: TickCtx<'_>,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        sequence_id: SequenceId,
        level: Vec<usize>,
    ) -> Result<(), crate::engine::script::ScriptDriverError> {
        for element_index in level {
            self.register_sequence_element(tcx, active_scripts, sequence_id, element_index, true)?;
        }
        Ok(())
    }

    pub(crate) fn register_sequence_element(
        &mut self,
        tcx: TickCtx<'_>,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        sequence_id: SequenceId,
        element_index: usize,
        wait_inline: bool,
    ) -> Result<(), crate::engine::script::ScriptDriverError> {
        let Some(element) = self
            .orders
            .sequence_manager
            .get_element(sequence_id, element_index)
        else {
            return Ok(());
        };
        if !matches!(
            element.state,
            SequenceState::Todo | SequenceState::Postponed
        ) {
            return Ok(());
        }
        let action = if wait_inline && element.priority == SequencePriority::Wait {
            match element.owner {
                Some(owner) => SequenceAction::InstructOwner {
                    owner,
                    sequence_id,
                    element_index,
                },
                None => SequenceAction::EngineCommand {
                    sequence_id,
                    element_index,
                },
            }
        } else if element.executed_immediately() {
            SequenceManager::immediate_action_for(sequence_id, element_index, element)
                .unwrap_or_else(|| {
                    panic!(
                        "owner-only immediate command {:?} has no owner",
                        element.command
                    )
                })
        } else {
            self.orders
                .sequence_manager
                .elements_to_go
                .push_back((sequence_id, element_index));
            return Ok(());
        };
        if let Err(mut error) = self.dispatch_script_synchronous_action(tcx, action, active_scripts)
        {
            if !error.sequence_element_failed {
                self.element_impossible(tcx, active_scripts, sequence_id, element_index);
                error.sequence_element_failed = true;
            }
            return Err(error);
        }
        Ok(())
    }
}
