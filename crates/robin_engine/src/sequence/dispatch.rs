//! Sequence manager dispatch responsibilities.
use super::*;

impl SequenceManager {
    // ─── Element dispatch registration ──────────────────────────

    /// Run the original game's per-element registration loop.
    ///
    /// The loop body is *not* a batch: waiting-priority elements execute
    /// and registering an element to go executes the
    /// immediate command group inline, all before the next sibling is even
    /// looked at.  A composite such as `[LockAi(a), Turn(b), Turn(a)]` relies
    /// on that: `LockAi` -> `ScriptLockAI` -> actor stopping ->
    /// Stopping not-yet-launched sequence elements only walks
    /// the pending sequence-element list, which does not yet contain `Turn(a)`.
    ///
    /// Rust dispatches those commands from the engine, so registration stops
    /// as soon as one element owes synchronous work and the remaining loop
    /// iterations are parked directly behind that work as
    /// [`PendingSyncEntry::Register`] entries.  Elements whose registration
    /// only appends to `elements_to_go` have no observable effect on their
    /// siblings, so those iterations still run straight through.
    pub(super) fn register_level_elements_to_go(&mut self, seq_id: SequenceId, to_go: Vec<usize>) {
        let mut remaining = to_go.into_iter();
        while let Some(elem_idx) = remaining.next() {
            let before = self.pending_synchronous_actions.len();
            self.register_one_element_to_go(seq_id, elem_idx);
            if self.pending_synchronous_actions.len() > before {
                for deferred in remaining {
                    self.pending_synchronous_actions
                        .push_back(PendingSyncEntry::Register {
                            sequence_id: seq_id,
                            element_index: deferred,
                        });
                }
                return;
            }
        }
    }

    /// One iteration of that loop: the `RHSEQ_INTERRUPTED` guard plus the
    /// wait-priority split, both read at iteration time.
    pub(super) fn register_one_element_to_go(&mut self, seq_id: SequenceId, elem_idx: usize) {
        let Some(element) = self.get_element(seq_id, elem_idx) else {
            tracing::trace!(
                ?seq_id,
                elem_idx,
                "next_elements_go: element disappeared before its registration ran"
            );
            return;
        };
        if element.state == SequenceState::Interrupted {
            return;
        }
        if element.priority == SequencePriority::Wait {
            self.register_wait_element_to_go(seq_id, elem_idx);
        } else {
            self.register_element_to_go(seq_id, elem_idx);
        }
    }

    /// Resume the parked successor-registration iterations that sit at the
    /// front of the synchronous buffer, stopping again as soon as one of them
    /// owes inline work.
    pub(super) fn settle_leading_registrations(&mut self) {
        while self
            .pending_synchronous_actions
            .front()
            .is_some_and(PendingSyncEntry::is_register)
        {
            self.perform_registration_at(0);
        }
    }

    /// Run the parked loop iteration at `index`, splicing whatever it
    /// registers into exactly that position so ordering is preserved.
    pub(super) fn perform_registration_at(&mut self, index: usize) {
        let entry = self
            .pending_synchronous_actions
            .remove(index)
            .expect("perform_registration_at index out of range");
        let PendingSyncEntry::Register {
            sequence_id,
            element_index,
        } = entry
        else {
            panic!("perform_registration_at called on a dispatchable action: {entry:?}");
        };
        let tail = self.pending_synchronous_actions.split_off(index);
        self.register_one_element_to_go(sequence_id, element_index);
        self.pending_synchronous_actions.extend(tail);
    }

    /// Register an element for deferred dispatch.
    ///
    /// If the element's command is in the `executed_immediately()`
    /// group, the element is *not* queued — instead, the corresponding
    /// `SequenceAction` is pushed onto
    /// [`pending_synchronous_actions`](Self::pending_synchronous_actions)
    /// for synchronous engine-side dispatch.  Non-immediate elements
    /// land on `elements_to_go` for the next `hourglass` pass.
    ///
    /// Engine-side wrappers around external entry points
    /// (`launch_sequence`, `launch_element`, `element_terminated`,
    /// `element_impossible`, `element_in_progress`,
    /// `element_interrupted`, `terminate_sequence`, `stop_owner`,
    /// `stop_pending_elements*`, `cancel_pending_move_commands`)
    /// drain pending immediate actions after each call so the
    /// immediate side effect fires this same frame: registration =
    /// dispatch.  The hourglass-internal cascade callsites in
    /// [`Self::process_effects`] need no extra drain — `hourglass`
    /// itself folds the queue into the action stream it returns.
    ///
    /// Terminal-state elements are silently skipped — only `Todo` /
    /// `Postponed` elements actually dispatch.  This situation arises
    /// legitimately when a preemption cascade lands an element into
    /// Terminated before [`Sequence::next_elements_go`] iterates over
    /// it: that iterator only filters `Interrupted`, not Terminated /
    /// Impossible.
    pub(super) fn register_element_to_go(&mut self, seq_id: SequenceId, elem_idx: usize) {
        let Some(seq) = self.sequences.get(&seq_id) else {
            return;
        };
        let Some(elem) = seq.elements.get(elem_idx) else {
            return;
        };

        if matches!(
            elem.state,
            SequenceState::Terminated | SequenceState::Impossible | SequenceState::Interrupted
        ) {
            tracing::trace!(
                ?seq_id,
                elem_idx,
                state = ?elem.state,
                command = ?elem.command,
                owner = ?elem.owner,
                "register_element_to_go: skipping terminal-state element"
            );
            return;
        }

        if elem.executed_immediately() {
            // `executed_immediately()` is a pure predicate; the matching
            // `SequenceAction` is queued here for the engine-side
            // dispatcher to drain inline.
            if let Some(action) = Self::immediate_action_for(seq_id, elem_idx, elem) {
                self.pending_synchronous_actions
                    .push_back(PendingSyncEntry::Action(action));
            } else {
                tracing::error!(
                    ?seq_id,
                    elem_idx,
                    command = ?elem.command,
                    owner = ?elem.owner,
                    "register_element_to_go: executed_immediately() = true but no \
                     immediate-action mapping — terminating element"
                );
                // Fall through to `elements_to_go` so the hourglass
                // diagnostic arm logs and terminates.  The element is
                // deliberately never put on `pending_synchronous_actions`
                // because we have no action to fire.
                self.elements_to_go.push_back((seq_id, elem_idx));
            }
            return;
        }

        self.elements_to_go.push_back((seq_id, elem_idx));
    }

    /// Emit the `Go()` action for a WAIT-priority element at registration
    /// time instead of placing it behind the next manager hourglass.
    ///
    /// Sequence advancement calls
    /// wait-priority command directly. Other priorities take the non-immediate
    /// path and append to the manager FIFO.
    pub(super) fn register_wait_element_to_go(&mut self, seq_id: SequenceId, elem_idx: usize) {
        let seq = self
            .sequences
            .get(&seq_id)
            .unwrap_or_else(|| panic!("register_wait_element_to_go: missing sequence {seq_id:?}"));
        let elem = seq.elements.get(elem_idx).unwrap_or_else(|| {
            panic!("register_wait_element_to_go: missing element ({seq_id:?}, {elem_idx})")
        });

        if !matches!(elem.state, SequenceState::Todo | SequenceState::Postponed) {
            tracing::trace!(
                ?seq_id,
                elem_idx,
                state = ?elem.state,
                command = ?elem.command,
                owner = ?elem.owner,
                "register_wait_element_to_go: Go is a no-op for non-live element"
            );
            return;
        }

        // Starting a sequence element bypasses the immediate-execution test and
        // routes solely by owner presence.
        let action = if let Some(owner) = elem.owner {
            SequenceAction::InstructOwner {
                owner,
                sequence_id: seq_id,
                element_index: elem_idx,
            }
        } else {
            SequenceAction::EngineCommand {
                sequence_id: seq_id,
                element_index: elem_idx,
            }
        };
        self.pending_synchronous_actions
            .push_back(PendingSyncEntry::Action(action));
    }

    /// Build the `SequenceAction` for an immediate-dispatch element.
    ///
    /// 3-way switch routed by command group, not by owner-presence:
    /// owner-only commands always dispatch to the owner, engine-only
    /// commands always dispatch to the engine regardless of owner,
    /// and `SendMessage` picks owner if non-null else engine.
    ///
    /// Returns `None` for owner-only commands launched without an
    /// owner — the caller logs and terminates the element so we don't
    /// silently drop the side effect.
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

    /// Drain pending immediate-dispatch actions accumulated
    /// since the last call.  Engine-side wrappers around external entry
    /// points call this after invoking `launch_sequence`,
    /// `launch_element`, `element_terminated`, `element_impossible`,
    /// `element_in_progress`, `element_interrupted`,
    /// `terminate_sequence`, `stop_owner`, `stop_pending_elements*`,
    /// or `cancel_pending_move_commands` so any immediate command that
    /// was registered fires this same frame: registration = dispatch.
    ///
    /// `hourglass` already folds this queue into its returned action
    /// stream, so callers inside the hourglass dispatch loop need not
    /// drain separately.
    pub fn take_pending_immediate_actions(&mut self) -> Vec<SequenceAction> {
        let mut immediate = Vec::new();
        let mut retained = VecDeque::new();
        loop {
            self.settle_leading_registrations();
            let Some(entry) = self.pending_synchronous_actions.pop_front() else {
                break;
            };
            match entry {
                PendingSyncEntry::Action(
                    action @ (SequenceAction::ExecuteImmediateOwner { .. }
                    | SequenceAction::ExecuteImmediateEngine { .. }),
                ) => immediate.push(action),
                other => retained.push_back(other),
            }
        }
        self.pending_synchronous_actions = retained;
        immediate
    }

    /// Pop the next synchronous action without disturbing the remainder.
    /// Script-native sequence launch uses this to stop exactly at a re-entrant
    /// SendMessage callback, then continue in order before the outer VM resumes.
    pub fn pop_pending_immediate_action(&mut self) -> Option<SequenceAction> {
        self.settle_leading_registrations();
        match self.pending_synchronous_actions.pop_front()? {
            PendingSyncEntry::Action(action) => Some(action),
            entry @ PendingSyncEntry::Register { .. } => {
                panic!("settle_leading_registrations left a parked registration: {entry:?}")
            }
        }
    }

    /// Remove the first action that Original registration executes inline,
    /// while leaving ordinary manager-update work queued in order.
    ///
    /// This is used at callbacks that occur outside the sequence-manager tick
    /// (notably director completion during Draw and anonymous-timer expiry
    /// after the manager phase). Immediate commands and
    /// Waiting-priority successors run inline; other priorities stay queued.
    ///
    /// Parked successor registrations are never left behind: any registration
    /// still queued after the inline scan runs here and the scan repeats.
    pub fn pop_pending_registration_inline_action(&mut self) -> Option<SequenceAction> {
        loop {
            self.settle_leading_registrations();
            let parked = self
                .pending_synchronous_actions
                .iter()
                .position(PendingSyncEntry::is_register);
            let scan_limit = parked.unwrap_or(self.pending_synchronous_actions.len());
            let inline = self
                .pending_synchronous_actions
                .iter()
                .take(scan_limit)
                .position(|entry| match entry.as_action() {
                    Some(
                        SequenceAction::ExecuteImmediateOwner { .. }
                        | SequenceAction::ExecuteImmediateEngine { .. },
                    ) => true,
                    Some(
                        SequenceAction::InstructOwner {
                            sequence_id,
                            element_index,
                            ..
                        }
                        | SequenceAction::EngineCommand {
                            sequence_id,
                            element_index,
                        },
                    ) => self
                        .get_element(*sequence_id, *element_index)
                        .is_some_and(|element| element.priority == SequencePriority::Wait),
                    None => false,
                });
            if let Some(index) = inline {
                return match self.pending_synchronous_actions.remove(index)? {
                    PendingSyncEntry::Action(action) => Some(action),
                    entry @ PendingSyncEntry::Register { .. } => {
                        panic!("inline scan selected a parked registration: {entry:?}")
                    }
                };
            }
            let index = parked?;
            self.perform_registration_at(index);
        }
    }

    pub fn next_pending_immediate_action(&mut self) -> Option<&SequenceAction> {
        self.settle_leading_registrations();
        self.pending_synchronous_actions
            .front()
            .and_then(PendingSyncEntry::as_action)
    }

    /// Drain the complete ordered stream emitted synchronously by sequence
    /// registration: direct waiting-priority actions interleaved with
    /// immediate actions.
    ///
    /// The engine action loop uses this after every callback.  If that
    /// callback completes an element and advances to a waiting-priority
    /// successor, the successor is inserted at the front of the remaining
    /// work before an older sibling action runs.
    /// Parked [`PendingSyncEntry::Register`] iterations travel with the
    /// continuation and resume only once the callback returns.
    pub fn take_pending_synchronous_actions(&mut self) -> Vec<PendingSyncEntry> {
        self.pending_synchronous_actions.drain(..).collect()
    }

    /// Drain only the settled head of the synchronous buffer, leaving any
    /// parked successor-registration iteration queued.
    ///
    /// The manager-hourglass action loop uses this: it must dispatch the
    /// action that a registration produced before the loop's next iteration
    /// registers the following sibling.
    pub fn take_settled_synchronous_actions(&mut self) -> Vec<SequenceAction> {
        let mut actions = Vec::new();
        self.settle_leading_registrations();
        while let Some(PendingSyncEntry::Action(_)) = self.pending_synchronous_actions.front() {
            let Some(PendingSyncEntry::Action(action)) =
                self.pending_synchronous_actions.pop_front()
            else {
                unreachable!("front was just observed to be an action");
            };
            actions.push(action);
        }
        actions
    }

    /// Restore a parent callback's detached synchronous continuation after a
    /// nested callback has fully returned. Any actions still produced by the
    /// child stay in front, matching the original game's recursive evaluation order.
    pub fn restore_pending_synchronous_actions(&mut self, continuation: Vec<PendingSyncEntry>) {
        self.pending_synchronous_actions.extend(continuation);
    }

    /// Append owner/engine instruction actions to the deferred manager FIFO.
    ///
    /// This is used when a synchronous `Go()` registration was created while
    /// an older instruction for the same owner was already waiting in
    /// `elements_to_go`. Keeping the newer action in the synchronous queue
    /// would let it jump ahead of that older registration; appending its
    /// element identity here preserves registration order without moving any
    /// unrelated manager entries.
    pub fn append_actions_to_deferred_fifo(&mut self, actions: Vec<SequenceAction>) {
        for action in actions {
            let target = match action {
                SequenceAction::InstructOwner {
                    sequence_id,
                    element_index,
                    ..
                }
                | SequenceAction::EngineCommand {
                    sequence_id,
                    element_index,
                } => (sequence_id, element_index),
                immediate => panic!(
                    "cannot defer immediate sequence action behind manager FIFO: {immediate:?}"
                ),
            };
            self.elements_to_go.push_back(target);
        }
    }

    /// `true` iff there is at least one immediate-dispatch action awaiting
    /// drain, ignoring direct WAIT `Go()` actions in the same stream.
    pub fn has_pending_immediate_actions(&self) -> bool {
        self.pending_synchronous_actions.iter().any(|entry| {
            matches!(
                entry.as_action(),
                Some(
                    SequenceAction::ExecuteImmediateOwner { .. }
                        | SequenceAction::ExecuteImmediateEngine { .. }
                )
            )
        })
    }

    // ─── Per-frame processing ───────────────────────────────────

    /// Process all pending sequence elements for this frame.
    /// Returns actions the engine must dispatch.
    ///
    /// Drains both the deferred `elements_to_go` queue and the
    /// synchronous registration buffer (populated by
    /// [`Self::register_element_to_go`] and
    /// [`Self::register_wait_element_to_go`]). Cascade callsites in
    /// [`Self::process_effects`] re-register elements during the loop —
    /// any new synchronous actions land on that buffer and are
    /// drained here this same frame.
    pub fn hourglass(&mut self) -> Vec<SequenceAction> {
        let mut actions = Vec::new();
        while let Some(action) = self.pop_next_hourglass_action() {
            actions.push(action);
        }
        actions
    }

    /// Pop exactly one action from the live manager FIFO. Original
    /// sequence-manager processing removes one element, starts it, and
    /// only then loops. Keeping the remaining elements registered makes them
    /// observable to callbacks querying pending sequence elements.
    pub(crate) fn pop_next_hourglass_action(&mut self) -> Option<SequenceAction> {
        // Waiting-priority and immediate actions run at registration, before
        // deferred non-WAIT work reaches the manager hourglass.
        self.settle_leading_registrations();
        match self.pending_synchronous_actions.pop_front() {
            Some(PendingSyncEntry::Action(action)) => Some(action),
            Some(entry @ PendingSyncEntry::Register { .. }) => {
                panic!("settle_leading_registrations left a parked registration: {entry:?}")
            }
            None => self.pop_deferred_hourglass_action(),
        }
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

            // The `register_element_to_go` path routes immediate
            // commands directly to `pending_synchronous_actions`, so
            // anything coming out of `elements_to_go` should normally
            // be non-immediate. WAIT-priority elements also bypass this
            // queue via `pending_synchronous_actions`.
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
                    self.element_terminated(seq_id, elem_idx);
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

    /// Drain normal-priority work registered while an engine-side update
    /// action was executing. Original appends this work to the live manager
    /// FIFO, after actions that were already waiting.
    pub fn take_pending_deferred_actions(&mut self) -> Vec<SequenceAction> {
        let mut actions = Vec::new();
        while let Some(action) = self.pop_deferred_hourglass_action() {
            actions.push(action);
        }
        actions
    }

    /// Promote one exact deferred element produced inside a synchronous
    /// native boundary. No other same-owner work is inspected or reordered.
    pub fn take_deferred_owner_action(
        &mut self,
        owner: EntityId,
        sequence_id: SequenceId,
        element_index: usize,
    ) -> Result<Option<SequenceAction>, String> {
        let element = self
            .get_element(sequence_id, element_index)
            .ok_or_else(|| format!("missing deferred element {sequence_id:?}/{element_index}"))?;
        if element.owner != Some(owner) {
            return Err(format!(
                "deferred element {sequence_id:?}/{element_index} belongs to {:?}, expected {owner:?}",
                element.owner
            ));
        }
        if !matches!(
            element.state,
            SequenceState::Todo | SequenceState::Postponed
        ) {
            if let Some(position) = self
                .elements_to_go
                .iter()
                .position(|handle| *handle == (sequence_id, element_index))
            {
                self.elements_to_go.remove(position);
            }
            return Ok(None);
        }
        if element.executed_immediately() {
            return Err(format!(
                "deferred element {sequence_id:?}/{element_index} unexpectedly executes immediately"
            ));
        }
        let position = self
            .elements_to_go
            .iter()
            .position(|handle| *handle == (sequence_id, element_index))
            .ok_or_else(|| {
                format!(
                    "live deferred element {sequence_id:?}/{element_index} is absent from elements_to_go"
                )
            })?;
        self.elements_to_go.remove(position);
        Ok(Some(SequenceAction::InstructOwner {
            owner,
            sequence_id,
            element_index,
        }))
    }

    /// Detach one exact live element from the ordinary manager FIFO without
    /// changing any of its instruction-time state.
    ///
    /// Human-actor instruction uses this shape for repeated PC bow
    /// shots: the sequence element has already been registered and reached
    /// the human, but is retained in the shoot list before actor instruction handling can
    /// resolve priority, stamp transition state, or translate the command.
    #[cfg(test)]
    pub(crate) fn hold_deferred_element(&mut self, sequence_id: SequenceId, element_index: usize) {
        let element = self
            .get_element(sequence_id, element_index)
            .unwrap_or_else(|| panic!("missing held element {sequence_id:?}/{element_index}"));
        assert_eq!(
            element.state,
            SequenceState::Todo,
            "held element {sequence_id:?}/{element_index} must still be Todo"
        );
        let position = self
            .elements_to_go
            .iter()
            .position(|handle| *handle == (sequence_id, element_index))
            .unwrap_or_else(|| {
                panic!("held element {sequence_id:?}/{element_index} is absent from elements_to_go")
            });
        self.elements_to_go.remove(position);
    }

    /// Remove and return this owner's deferred actions through an exact
    /// target, preserving their relative manager-FIFO order.
    ///
    /// Termination registers the finishing sequence's newly-ready
    /// elements before postponed-element startup registers a released
    /// cross-sequence successor.  A synchronous owner boundary must therefore
    /// not pluck that successor out of the middle of `elements_to_go`: doing
    /// so lets the old sequence run after the replacement and interrupt it.
    /// Foreign-owner entries remain in place.
    pub fn take_deferred_owner_actions_through(
        &mut self,
        owner: EntityId,
        target_sequence_id: SequenceId,
        target_element_index: usize,
    ) -> Result<Vec<SequenceAction>, String> {
        let target = (target_sequence_id, target_element_index);
        let target_element = self
            .get_element(target_sequence_id, target_element_index)
            .ok_or_else(|| {
                format!("missing deferred target {target_sequence_id:?}/{target_element_index}")
            })?;
        if target_element.owner != Some(owner) {
            return Err(format!(
                "deferred target {target_sequence_id:?}/{target_element_index} belongs to {:?}, expected {owner:?}",
                target_element.owner
            ));
        }
        if !matches!(
            target_element.state,
            SequenceState::Todo | SequenceState::Postponed
        ) {
            if let Some(position) = self
                .elements_to_go
                .iter()
                .position(|handle| *handle == target)
            {
                self.elements_to_go.remove(position);
            }
            return Ok(Vec::new());
        }
        if target_element.executed_immediately() {
            return Err(format!(
                "deferred target {target_sequence_id:?}/{target_element_index} unexpectedly executes immediately"
            ));
        }
        let target_position = self
            .elements_to_go
            .iter()
            .position(|handle| *handle == target)
            .ok_or_else(|| {
                format!(
                    "live deferred target {target_sequence_id:?}/{target_element_index} is absent from elements_to_go"
                )
            })?;

        let handles: Vec<_> = self
            .elements_to_go
            .iter()
            .take(target_position + 1)
            .copied()
            .filter(|(sequence_id, element_index)| {
                self.get_element(*sequence_id, *element_index)
                    .is_some_and(|element| element.owner == Some(owner))
            })
            .collect();

        if !handles.contains(&target) {
            return Err(format!(
                "deferred target {target_sequence_id:?}/{target_element_index} does not belong to {owner:?}"
            ));
        }

        let mut actions = Vec::with_capacity(handles.len());
        for (sequence_id, element_index) in handles {
            if let Some(action) =
                self.take_deferred_owner_action(owner, sequence_id, element_index)?
            {
                actions.push(action);
            }
        }
        Ok(actions)
    }
}
