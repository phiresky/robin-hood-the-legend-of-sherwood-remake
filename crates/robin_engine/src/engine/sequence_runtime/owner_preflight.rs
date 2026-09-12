use super::*;

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct PreparedOwnerInstruction {
    pub(super) owner: EntityId,
    pub(super) cmd: Command,
    pub(super) trace_path_owner: bool,
    pub(super) satisfied_enter_swordfight_order: Option<crate::element::InstalledActorOrder>,
}

impl EngineInner {
    /// Admission and synchronous arbitration complete before translation.
    /// A rejected instruction still returns to the phase's common condolence
    /// and live-FIFO continuation, including every re-entrant successor.
    pub(super) fn prepare_owner_instruction(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> Option<PreparedOwnerInstruction> {
        self.debug_patrol_turn_instruct(owner, seq_id, elem_idx);
        // Player instruction handling redirects a TO_JUMP Move from a rider to
        // the carrier before human/actor instruction handling sees it. This
        // must sample the live posture here, not when the element
        // was registered earlier in the frame.
        let owner = self.redirect_queued_move_to_jump_if_carried(owner, seq_id, elem_idx);
        // Human instruction handling owns this guard, before
        // actor instruction handling resolves priority and stamps transition
        // state, generates orders, or arbitrates. The action has
        // already been detached from the manager FIFO by this
        // phase, so retaining the exact ref is sufficient.
        let command = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|element| element.command);
        // Human instruction handling rejects dead, unconscious, and
        // net-stuck owners before delegating to actor instruction handling.
        // In particular, a postponed ordinary command released
        // after its owner falls unconscious must not allocate a
        // doomed stand-up transition first.
        if command.is_some_and(|command| self.human_instruct_rejects_command(owner, command)) {
            if let Some(actor) = self
                .world
                .entities
                .get_mut(owner)
                .and_then(crate::element::Entity::actor_data_mut)
            {
                actor.execution_frozen = false;
            }
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return None;
        }
        if command.is_some_and(|command| self.pc_should_hold_shoot_bow(owner, command)) {
            self.queue_pc_shoot_bow(
                owner,
                crate::sequence::SequenceElementRef::new(seq_id, elem_idx),
            );
            return None;
        }

        // RHElementActor::Instruct terminates NULL before transition
        // generation or priority arbitration. MakeUpright deliberately
        // cancels queued CROUCH_DOWN elements by rewriting them to NULL;
        // they still need their normal termination/readiness cascade.
        if command == Some(Command::Null) {
            self.world
                .entities
                .get_mut(owner)
                .and_then(crate::element::Entity::actor_data_mut)
                .expect("NULL actor instruction requires its actor owner")
                .execution_frozen = false;
            self.orders
                .sequence_manager
                .element_terminated(seq_id, elem_idx);
            self.dispatch_condolations(sim, assets);
            return None;
        }

        // Every actor instruction snapshots the actor's
        // current posture and action state before the
        // non-interruptable guard, transition generation, and
        // ordinary priority arbitration. Freshly launched
        // elements are eagerly stamped; a postponed element marks
        // that snapshot Undefined when it is released because
        // this is its second instruction boundary.
        let needs_transition = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .is_some_and(|element| {
                matches!(
                    element.state,
                    crate::sequence::SequenceState::Todo
                        | crate::sequence::SequenceState::Postponed
                ) && element.posture_after_transition == crate::element::Posture::Undefined
            });
        if needs_transition {
            self.stamp_element_transition_state(owner, seq_id, elem_idx);
        }
        // Original-game actor instruction handles a selected
        // NON_INTERRUPTABLE element before transition generation.
        // This matters for commands arriving while a door pass
        // temporarily owns a posture (Flying/OnWall) from which
        // the incoming command cannot yet generate its ordinary
        // posture transition. The command is postponed and only
        // generates that transition after the door pass releases
        // it. The guard can also interrupt an older postponed
        // equal-priority command, so settle any resulting card at
        // this exact instruction boundary.
        if self.non_interruptable_guard(owner, seq_id, elem_idx) {
            self.dispatch_condolations(sim, assets);
            return None;
        }
        // Outside that special arm, Original generates the
        // incoming element's transition orders before normal
        // priority comparison with the selected element.
        if needs_transition && !self.generate_transition(sim, assets, owner, seq_id, elem_idx) {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            self.dispatch_condolations(sim, assets);
            return None;
        }
        if self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .is_some_and(|element| {
                matches!(
                    element.state,
                    crate::sequence::SequenceState::Terminated
                        | crate::sequence::SequenceState::Impossible
                        | crate::sequence::SequenceState::Interrupted
                )
            })
        {
            // Original-game actor instruction handling returns immediately when
            // Transition generation made the incoming element
            // terminal. The state change's removal callback is
            // synchronous there, so close that boundary before
            // leaving without priority resolution or arbitration.
            self.dispatch_condolations(sim, assets);
            return None;
        }
        let resolved_priority = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .filter(|element| element.priority == crate::sequence::SequencePriority::NotYetSet)
            .map(|element| {
                let resolver = Self::priority_resolver(&self.world.entities);
                resolver(element)
            });
        if let Some(priority) = resolved_priority
            && let Some(element) = self
                .orders
                .sequence_manager
                .get_element_mut(seq_id, elem_idx)
        {
            element.priority = priority;
        }
        // A shoulder-climb click can be recorded while its
        // target still owns the non-interruptible
        // EnterHelpingClimb transition. Original retains the
        // climber's interaction until that target-side entry
        // element completes; only then does Translate inspect
        // shoulder-carry eligibility and, for low headroom, launch the
        // helper's LeaveHelpingClimb recovery
        // for leaving the helping-climb state.
        //
        // This is a cross-actor dependency, not ordinary
        // owner-priority arbitration. Keep the incoming
        // element Postponed without attaching it to the
        // climber's selected-element chain. The canonical
        // helper-state scan above re-registers it after the
        // entry element's TERMINATED edge.
        if command == Some(Command::ClimbUpOnShoulders) {
            let helper = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .and_then(|element| match &element.data {
                    crate::sequence::SequenceElementData::Interaction { antagonist } => *antagonist,
                    _ => None,
                });
            let helper_is_entering = helper.is_some_and(|helper| {
                let posture_ready = self.get_entity(helper).is_some_and(|entity| {
                    entity.element_data().posture() == crate::element::Posture::HelpingToClimb
                });
                !posture_ready
                    && self
                        .orders
                        .sequence_manager
                        .current_element_for_actor(helper)
                        .and_then(|(helper_sequence, helper_index)| {
                            self.orders
                                .sequence_manager
                                .get_element(helper_sequence, helper_index)
                        })
                        .is_some_and(|element| {
                            element.command == Command::EnterHelpingClimb
                                && element.state == crate::sequence::SequenceState::InProgress
                        })
            });
            if helper_is_entering {
                self.orders
                    .sequence_manager
                    .postpone_element(seq_id, elem_idx);
                return None;
            }
            if let Some(helper_id) = helper
                && let Some(helper_position) = self
                    .get_entity(helper_id)
                    .filter(|entity| {
                        entity.element_data().posture() == crate::element::Posture::HelpingToClimb
                    })
                    .map(|entity| entity.position_iface().get_position())
            {
                let obstacles = crate::sight_obstacle::ObstacleList {
                    static_obstacles: assets.environment.static_sight_obstacles.as_slice(),
                    dynamic_obstacles: &self.world.dynamic_sight_obstacles,
                    static_active: &self.world.static_sight_obstacle_active,
                };
                if !abilities::can_carry_on_shoulders(helper_position, obstacles) {
                    // This retained interaction is a
                    // cross-actor validity retry. Retail
                    // resolves low headroom without replacing
                    // the climber's live idle element; only the
                    // helper receives the compensating leave.
                    self.launch_element(crate::sequence::SequenceElement::new(
                        1,
                        Command::LeaveHelpingClimb,
                        Some(helper_id),
                    ));
                    self.orders
                        .sequence_manager
                        .element_impossible(seq_id, elem_idx);
                    return None;
                }
            }
        }
        // A cross-sector player route can reach this FIFO
        // immediately after the preceding movement published
        // its bow-equipping recovery. The original game translates the movement
        // first, then lets the fresh recovery postpone it. Its
        // queued path is therefore cancelled as the retained
        // head and reports an invalid completion next frame
        // through normal path processing. Preserve that
        // real request lifecycle instead of postponing an
        // untranslated Move and fabricating a recorder event.
        if let Some((blocker_seq, blocker_idx)) =
            self.fresh_recovery_blocker_after_route_assert(owner, seq_id, elem_idx)
        {
            self.orders.sequence_manager.set_translating_element(Some((
                owner,
                crate::sequence::SequenceElementRef::new(seq_id, elem_idx),
            )));
            let barrier =
                self.dispatch_ordered_move_seek_instruct(sim, assets, owner, seq_id, elem_idx);
            self.orders.sequence_manager.set_translating_element(None);
            if barrier == OwnerActionBarrier::Reach {
                self.engine_postpone(blocker_seq, blocker_idx, seq_id, elem_idx);
            }
            self.dispatch_condolations(sim, assets);
            return None;
        }
        // A redundant EnterSwordfight still replaces and
        // terminates the selected Wait element, but Original's
        // actor keeps driving the already installed
        // WaitingSword order until the fresh idle is published
        // on the following frame. Preserve only that stable
        // order; arbitration and its synchronous EventDone
        // callbacks must continue to observe the ordinary
        // replacement lifecycle.
        let satisfied_enter_swordfight_order = (self.control.frame_counter > 0
            && command == Some(crate::element::Command::EnterSwordfight))
        .then(|| {
            self.orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .and_then(|element| element.get_property(crate::sequence::Field::Opponent))
                .and_then(|value| match value {
                    crate::sequence::FieldValue::Element(opponent) => Some(*opponent),
                    _ => None,
                })
                .and_then(|opponent| {
                    self.get_entity(owner)
                        .and_then(|entity| entity.human_data())
                        .filter(|human| {
                            human.opponents.contains(&opponent)
                                && self
                                    .current_sequence_element_for_actor(owner)
                                    .and_then(|(sequence, index)| {
                                        self.orders.sequence_manager.get_element(sequence, index)
                                    })
                                    .is_some_and(|element| {
                                        element.command == crate::element::Command::Wait
                                            && element.current_order().is_some_and(|order| {
                                                order.order_type
                                                    == crate::order::OrderType::WaitingSword
                                            })
                                    })
                        })
                        .and_then(|_| {
                            self.get_entity(owner)
                                .and_then(|entity| entity.actor_data())
                                .and_then(|actor| actor.installed_order)
                        })
                })
        })
        .flatten();
        let trace_reactive_topology = matches!(
            command,
            Some(crate::element::Command::ParrySword)
                | Some(crate::element::Command::ReceiveSwordDamage)
        );
        if trace_reactive_topology {
            self.trace_reactive_sword_topology(
                "before_instruct_arbitration",
                owner,
                Some((seq_id, elem_idx)),
            );
        }
        let trace_path_owner = matches!(
            command,
            Some(crate::element::Command::Move) | Some(crate::element::Command::Seek)
        );
        if trace_path_owner {
            self.trace_path_owner_lifecycle(
                "before_instruct_arbitration",
                owner,
                Some((seq_id, elem_idx)),
            );
        }
        let arbitration_accepted = self.arbitrate_instruct(seq_id, elem_idx);
        if trace_path_owner {
            self.trace_path_owner_lifecycle(
                if arbitration_accepted {
                    "after_instruct_arbitration_accepted"
                } else {
                    "after_instruct_arbitration_rejected"
                },
                owner,
                Some((seq_id, elem_idx)),
            );
        }
        if trace_reactive_topology {
            self.trace_reactive_sword_topology(
                if arbitration_accepted {
                    "after_instruct_arbitration_accepted"
                } else {
                    "after_instruct_arbitration_rejected"
                },
                owner,
                Some((seq_id, elem_idx)),
            );
        }
        if !arbitration_accepted {
            // Abandonment/impossibility changes state synchronously in
            // Original too. Postpone produces no card, making this
            // drain a no-op for that arm.
            self.dispatch_condolations(sim, assets);
            return None;
        }
        // Original priority arbitration interrupts/postpones the
        // outgoing element through state change, whose
        // removal-notification callback completes synchronously
        // before instruction handling continues into transition generation
        // and command translation for the incoming element.
        //
        // `SequenceManager` queues that callback to avoid
        // re-entrant borrows, so close the same stack boundary
        // here. In particular, an interrupted combat action's
        // EventDone/Reconsider RNG must run before incoming
        // damage translation and its damage/provoke RNG.
        self.orders
            .sequence_manager
            .begin_instruct_callback(owner, seq_id, elem_idx);
        self.dispatch_condolations(sim, assets);
        let still_selected = self
            .orders
            .sequence_manager
            .end_instruct_callback(owner, seq_id, elem_idx);
        if !still_selected {
            // A recursive instruction accepted replacement work
            // while the outgoing element's condolence callback
            // ran. The original game's post-callback identity check sees
            // that the selected sequence element no longer names this
            // incoming element and returns before Translate.
            return None;
        }
        if trace_path_owner {
            self.trace_path_owner_lifecycle(
                "after_instruct_callback",
                owner,
                Some((seq_id, elem_idx)),
            );
        }
        // Skip elements whose state moved to terminal /
        // interrupted while an earlier action in this batch
        // arbitrated against them. Without this, the loop
        // would try to dispatch a non-live element
        // and hit `set_element_state: Terminated from
        // illegal state Interrupted`.
        let cmd = match self.orders.sequence_manager.get_element(seq_id, elem_idx) {
            Some(e) => {
                use crate::sequence::SequenceState;
                if !matches!(e.state, SequenceState::Todo | SequenceState::Postponed) {
                    return None;
                }
                e.command
            }
            None => return None,
        };
        // Beggar-command filter: reject anything other
        // than RECEIVE_PURSE / BEGGAR_SHOW_FACE / WAIT on
        // beggar civilians.
        if self.beggar_rejects_command(owner, cmd) {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return None;
        }
        // Posture transitions (leave-disguise, stand-up, …)
        // are handled before command dispatch at this ordered
        // InstructOwner admission boundary. A direct prebuilt-
        // order lowering may already have performed that work,
        // which is why `needs_transition` gates it above.
        //
        // The accepted element stays the actor's selection for
        // the whole translation. Translation bodies that
        // terminate or interrupt the element on the spot then
        // send their condolence card while still selected, and
        // that card is what clears the actor's movement goal.
        self.orders.sequence_manager.set_translating_element(Some((
            owner,
            crate::sequence::SequenceElementRef::new(seq_id, elem_idx),
        )));

        Some(PreparedOwnerInstruction {
            owner,
            cmd,
            trace_path_owner,
            satisfied_enter_swordfight_order,
        })
    }
}
