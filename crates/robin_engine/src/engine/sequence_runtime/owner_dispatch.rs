use super::*;

impl EngineInner {
    pub(super) fn dispatch_sequence_phase_action(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        action: crate::sequence::SequenceAction,
        accepted_instruct_owners: &mut Vec<EntityId>,
    ) {
        'action: {
            match action {
                crate::sequence::SequenceAction::InstructOwner {
                    owner,
                    sequence_id: seq_id,
                    element_index: elem_idx,
                } => {
                    self.debug_patrol_turn_instruct(owner, seq_id, elem_idx);
                    // Player instruction handling redirects a TO_JUMP Move from a rider to
                    // the carrier before human/actor instruction handling sees it. This
                    // must sample the live posture here, not when the element
                    // was registered earlier in the frame.
                    let owner =
                        self.redirect_queued_move_to_jump_if_carried(owner, seq_id, elem_idx);
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
                    if command
                        .is_some_and(|command| self.human_instruct_rejects_command(owner, command))
                    {
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
                        break 'action;
                    }
                    if command.is_some_and(|command| self.pc_should_hold_shoot_bow(owner, command))
                    {
                        self.queue_pc_shoot_bow(
                            owner,
                            crate::sequence::SequenceElementRef::new(seq_id, elem_idx),
                        );
                        break 'action;
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
                        break 'action;
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
                            ) && element.posture_after_transition
                                == crate::element::Posture::Undefined
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
                        break 'action;
                    }
                    // Outside that special arm, Original generates the
                    // incoming element's transition orders before normal
                    // priority comparison with the selected element.
                    if needs_transition
                        && !self.generate_transition(sim, assets, owner, seq_id, elem_idx)
                    {
                        self.orders
                            .sequence_manager
                            .element_impossible(seq_id, elem_idx);
                        self.dispatch_condolations(sim, assets);
                        break 'action;
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
                        break 'action;
                    }
                    let resolved_priority = self
                        .orders
                        .sequence_manager
                        .get_element(seq_id, elem_idx)
                        .filter(|element| {
                            element.priority == crate::sequence::SequencePriority::NotYetSet
                        })
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
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
                                _ => None,
                            });
                        let helper_is_entering = helper.is_some_and(|helper| {
                            let posture_ready = self.get_entity(helper).is_some_and(|entity| {
                                entity.element_data().posture()
                                    == crate::element::Posture::HelpingToClimb
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
                                            && element.state
                                                == crate::sequence::SequenceState::InProgress
                                    })
                        });
                        if helper_is_entering {
                            self.orders
                                .sequence_manager
                                .postpone_element(seq_id, elem_idx);
                            break 'action;
                        }
                        if let Some(helper_id) = helper
                            && let Some(helper_position) = self
                                .get_entity(helper_id)
                                .filter(|entity| {
                                    entity.element_data().posture()
                                        == crate::element::Posture::HelpingToClimb
                                })
                                .map(|entity| entity.position_iface().get_position())
                        {
                            let obstacles = crate::sight_obstacle::ObstacleList {
                                static_obstacles: assets
                                    .environment
                                    .static_sight_obstacles
                                    .as_slice(),
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
                                break 'action;
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
                        let barrier = self.dispatch_ordered_move_seek_instruct(
                            sim, assets, owner, seq_id, elem_idx,
                        );
                        self.orders.sequence_manager.set_translating_element(None);
                        if barrier == OwnerActionBarrier::Reach {
                            self.engine_postpone(blocker_seq, blocker_idx, seq_id, elem_idx);
                        }
                        self.dispatch_condolations(sim, assets);
                        break 'action;
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
                                .and_then(|element| {
                                    element.get_property(crate::sequence::Field::Opponent)
                                })
                                .and_then(|value| match value {
                                    crate::sequence::FieldValue::Element(opponent) => {
                                        Some(*opponent)
                                    }
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
                                                        self.orders
                                                            .sequence_manager
                                                            .get_element(sequence, index)
                                                    })
                                                    .is_some_and(|element| {
                                                        element.command
                                                            == crate::element::Command::Wait
                                                            && element
                                                                .current_order()
                                                                .is_some_and(|order| {
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
                        break 'action;
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
                        break 'action;
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
                                break 'action;
                            }
                            e.command
                        }
                        None => break 'action,
                    };
                    // Beggar-command filter: reject anything other
                    // than RECEIVE_PURSE / BEGGAR_SHOW_FACE / WAIT on
                    // beggar civilians.
                    if self.beggar_rejects_command(owner, cmd) {
                        self.orders
                            .sequence_manager
                            .element_impossible(seq_id, elem_idx);
                        break 'action;
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
                    // Re-borrow element for data access.
                    let elem = match self.orders.sequence_manager.get_element(seq_id, elem_idx) {
                        Some(e) => e,
                        None => break 'action,
                    };
                    // Do not run a generic human validity check here.
                    // Original-game human instruction handling delegates directly to
                    // base actor handling after its dead/unconscious and repeated
                    // PC bow-shot guards. Commands that require live
                    // revalidation do so in their specific Execute
                    // initialization arm; WakeUp, for example, deliberately
                    // has no position-validity check during instruction.
                    match cmd {
                        Command::Move | Command::Seek => {
                            let barrier = self.dispatch_ordered_move_seek_instruct(
                                sim, assets, owner, seq_id, elem_idx,
                            );
                            if barrier == OwnerActionBarrier::Skip {
                                break 'action;
                            }
                        }
                        Command::ShootBow | Command::ShootBowOnce => {
                            if self.instruct_shoot_bow(assets, owner, seq_id, elem_idx, cmd)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        Command::PassDoor => {
                            let barrier = crate::engine::door_pass::PassDoorLaunchContext::new(
                                self.script_domains.interactables.doors.as_slice(),
                                &mut self.world.entities,
                                &self.world.fast_grid,
                                &mut self.orders.sequence_manager,
                                &mut self.orders.next_order_id,
                            )
                            .dispatch(owner, seq_id, elem_idx);
                            if barrier
                                == crate::engine::door_pass::PassDoorLaunchBarrier::SkipSplice
                            {
                                break 'action;
                            }
                        }
                        // ── CHANGE_POSITION ────────────────────────
                        // Instant teleport to a new position.
                        Command::ChangePosition => {
                            if self.instruct_change_position(assets, owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        // ── ASSERT_POSITION ────────────────────────
                        // Check actor is at expected position/sector.
                        Command::AssertPosition => {
                            let barrier = PositionAssertionContext {
                                entities: &self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                            }
                            .dispatch(owner, seq_id, elem_idx);
                            debug_assert_eq!(barrier, OwnerActionBarrier::Skip);
                            break 'action;
                        }
                        // ── WAIT_FREE_LIFT ──────────────────────
                        // Translation is identical to WAIT: book the
                        // stationary actor order and enter InProgress. The
                        // live actor-slot coordinator rechecks/reserves the
                        // lift after each actual Execute, matching
                        // actor updating rather than this one-shot
                        // instruction boundary.
                        Command::WaitFreeLift => {
                            WaitCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                                profiles: &assets.profile_manager,
                            }
                            .dispatch(
                                owner,
                                Command::WaitFreeLift,
                                seq_id,
                                elem_idx,
                            );
                        }
                        // ── Sword strike commands ────────────────
                        Command::SwordstrikeThrustA
                        | Command::SwordstrikeThrustB
                        | Command::SwordstrikeThrustC
                        | Command::SwordstrikeThrustD
                        | Command::SwordstrikeThrustE
                        | Command::SwordstrikeThrustF
                        | Command::SwordstrikeThrustG
                        | Command::SwordstrikeThrustH
                        | Command::SwordstrikeThrustI => {
                            if self.instruct_swordstrike_thrust_a(assets, owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }

                        // ── Swordfight enter/quit ───────────────
                        Command::EnterSwordfight | Command::PrepareSwordfight => {
                            let opponent = match elem.get_property(crate::sequence::Field::Opponent)
                            {
                                Some(crate::sequence::FieldValue::Element(id)) => Some(*id),
                                _ => None,
                            };
                            let barrier = self.dispatch_enter_swordfight(
                                sim, assets, owner, opponent, seq_id, elem_idx,
                            );
                            if barrier == OwnerActionBarrier::Skip {
                                self.dispatch_condolations(sim, assets);
                                if let Some(retained_order) = satisfied_enter_swordfight_order {
                                    let entity = self
                                        .get_entity_mut(owner)
                                        .expect("satisfied EnterSwordfight owner disappeared");
                                    let actor = entity.actor_data_mut().unwrap();
                                    actor.installed_order = Some(retained_order);
                                    actor.retained_waiting_sword_order_id =
                                        Some(retained_order.order_id);
                                }
                                break 'action;
                            }
                        }
                        Command::QuitSwordfight => {
                            if self.dispatch_quit_swordfight(sim, assets, owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }

                        // ── Parry commands ──────────────────────
                        Command::ParrySword => {
                            if self.dispatch_parry_sword(owner, false, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        Command::ParrySwordLow => {
                            if self.dispatch_parry_sword(owner, true, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        Command::StopParrySword => {
                            if self.dispatch_stop_parry(owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }

                        // ── Damage reception commands ───────────
                        Command::ReceiveSwordDamage
                        | Command::ReceiveDamage
                        | Command::ReceiveArrowDamage
                        | Command::ReceiveStoneDamage
                        | Command::ReceiveHitDamage
                        | Command::ReceiveMobileDamage
                        | Command::ReceiveNet => {
                            if self.dispatch_receive_damage(sim, assets, owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }

                        // ── Shoulder-fall sub-sequence ──────────
                        // Launched by `translate_shoulder_damage` on
                        // the carrier/carried partner when shoulder-
                        // damage lands on the other side of the carry.
                        Command::Fall => {
                            self.dispatch_fall(owner, seq_id, elem_idx);
                        }

                        // ── NPC head-turn / lean-out commands ────
                        // Insert a Looking{Left,Right}[Alerted] or
                        // TransitionWaitingAlertedLeaningOut order on
                        // the actor's queue, then stay in-progress
                        // until the sprite reaches DONE.  Terminating
                        // the element immediately (as the code did
                        // before) let `LOOK_LEFT_RIGHT` sequences
                        // advance to the second command before the
                        // first animation ran, so the second booking
                        // overwrote the first and only one of the
                        // two head turns played.
                        Command::LookLeft | Command::LookRight | Command::LeanOut => {
                            let barrier = NpcAttentionCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                            debug_assert_eq!(barrier, OwnerActionBarrier::Reach);
                        }

                        // ── Attentive-mode transitions ───────────
                        Command::EnterAttentiveMode
                        | Command::LeaveAttentiveMode
                        | Command::LeaveAttentiveModeOfficer => {
                            self.trace_attentive_owner_handoff(
                                "translate_before",
                                owner,
                                Some((seq_id, elem_idx)),
                                format_args!("before attentive translator"),
                            );
                            let barrier = NpcAttentionCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                            self.trace_attentive_owner_handoff(
                                "translate_after",
                                owner,
                                Some((seq_id, elem_idx)),
                                format_args!(
                                    "{}",
                                    match barrier {
                                        OwnerActionBarrier::Reach => {
                                            "attentive translator queued transition"
                                        }
                                        OwnerActionBarrier::Skip => {
                                            "attentive translator terminalized inline"
                                        }
                                    }
                                ),
                            );
                            if barrier == OwnerActionBarrier::Skip {
                                break 'action;
                            }
                        }

                        // ── Wasp sting ─────────────────────────
                        Command::ReceiveWaspSting => {
                            self.dispatch_receive_wasp_sting(sim, assets, owner, seq_id, elem_idx);
                        }

                        // ── Stealth posture commands ────────────
                        Command::CrouchDown
                        | Command::CrouchUp
                        | Command::EnterBeggar
                        | Command::LeaveBeggar
                        | Command::EnterHelpingClimb
                        | Command::LeaveHelpingClimb
                        | Command::EnterCloak
                        | Command::LeaveSpy
                        | Command::LeaveTree => {
                            if cmd == Command::EnterBeggar {
                                // "To avoid beggar & run bug": the beggar
                                // entry stops the actor from inside its own
                                // translation, so the stop runs after this
                                // element has already taken over and pushed
                                // whatever it replaced into its postponed
                                // slot. Walking that slot is the point — a
                                // move the beggar entry displaced is
                                // interrupted here and never resumes. The
                                // element is not the actor's selection yet on
                                // this side, so root the stop at it directly.
                                let resolver = Self::priority_resolver(&self.world.entities);
                                self.orders.sequence_manager.stop_owner_from_root(
                                    owner,
                                    Some((seq_id, elem_idx)),
                                    crate::sequence::SequencePriority::Normal,
                                    &resolver,
                                );
                            }
                            let barrier = StealthCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                                titbit_manager: &mut self.feedback.titbit_manager,
                                profiles: &assets.profile_manager,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                            debug_assert_eq!(barrier, OwnerActionBarrier::Reach);
                        }

                        // ── Shield commands ─────────────────────
                        Command::RaiseShield
                        | Command::RaiseShieldInstantly
                        | Command::LowerShield
                        | Command::ParryShield => {
                            if self.instruct_raise_shield(assets, owner, seq_id, elem_idx, cmd)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        // ── Bow equip / raise / lower ───────────
                        //
                        // The original game's actor translation appends
                        // these bow animation orders from the command
                        // body itself. Some command profiles may have
                        // already queued transition orders before
                        // translate; when they have not, push the
                        // command's own orders here.
                        Command::EquipBow
                        | Command::EquipBowDown
                        | Command::UnequipBow
                        | Command::RaiseBow
                        | Command::LowerBow => {
                            let barrier = BowTransitionContext {
                                entities: &self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                break 'action;
                            }
                        }
                        // ── Hide behind shield ──────────────────
                        //
                        // 1. Holder must be holding-shield (HOLDING/
                        //    MOVING/PARRYING) AND not currently
                        //    protecting anyone.  Otherwise → INTERRUPTED
                        //    (note: this is stricter than the
                        //    validity gate, which permits
                        //    `holder.shield_protected == self`).
                        // 2. If the element's posture-after-transition is
                        //    not Crouched, prepend a TRANSITION_CROUCHING_DOWN
                        //    order so the actor crouches before hiding.
                        // 3. Push the HIDING_BEHIND_SHIELD non-animation
                        //    order with the shield holder as antagonist.
                        Command::HideBehindShield => {
                            if self.instruct_hide_behind_shield(seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }

                        // ── Other sword-related commands ────────
                        Command::SwordstrikeDown => {
                            if self.instruct_swordstrike_down(owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        Command::GetKilledAtBottom => {
                            if self
                                .instruct_get_killed_at_bottom(sim, assets, owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        // SwordstrikeTired pushes a `BeingWeakSword`
                        // animation order; the order is consumed by
                        // `do_next_order` and (on a soldier)
                        // `apply_combat_injury_side_effect`
                        // dispatches `EventAfterCombatInjury` so the
                        // AI can resume the fight.
                        Command::SwordstrikeTired => {
                            if self.get_entity(owner).is_some() {
                                self.push_new_order(
                                    seq_id,
                                    elem_idx,
                                    crate::order::OrderType::BeingWeakSword,
                                    0.0,
                                    0.0,
                                );
                                self.orders
                                    .sequence_manager
                                    .element_in_progress(seq_id, elem_idx);
                            } else {
                                self.orders
                                    .sequence_manager
                                    .element_terminated(seq_id, elem_idx);
                            }
                        }
                        // ── Smalltalk strikes / parries (Wait priority) ─
                        // WAIT-priority launch is synchronous. Use the same
                        // narrow translator from both the normal sequence
                        // phase and owner-local WaitingSword callbacks.
                        Command::SwordstrikeSmalltalkLeft
                        | Command::SwordstrikeSmalltalkRight
                        | Command::ParrySmalltalkLeft
                        | Command::ParrySmalltalkRight => {
                            SmalltalkCommandContext {
                                entities: &self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                        }
                        // ── Provoke (taunt) ─────────────────────
                        // Say `ProvokesCombat` and queue a `Provoking`
                        // animation order (with `compute_direction =
                        // false`).  The animation is consumed via
                        // `active_ai_anim` tied to the sequence
                        // element; its START hook in
                        // `melee::process_pc_combat_anim_speech`
                        // fires `HERO_PROVOKE_OPPONENT` for PCs.
                        Command::Provoke => {
                            self.dispatch_provoke(sim, assets, owner, seq_id, elem_idx);
                        }
                        Command::Fainted
                        | Command::Recover
                        | Command::StandUp
                        | Command::WakeUp
                        | Command::Knee => {
                            let barrier = RecoveryCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                break 'action;
                            }
                        }

                        // ── Ability commands ─────────────────────
                        Command::TakeCorpse => {
                            if self.instruct_take_corpse(owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        Command::DropCorpse => {
                            if self.instruct_drop_corpse(owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        Command::HitCmd | Command::StrangleCmd => {
                            if self.instruct_hit_cmd(sim, assets, owner, seq_id, elem_idx, cmd)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        Command::TieCmd
                        | Command::Untie
                        | Command::HealCmd
                        | Command::WhistleCmd
                        | Command::EatCmd
                        | Command::ReceivePurse
                        | Command::EnterListen
                        | Command::LeaveListen
                        | Command::ThrowNet
                        | Command::ThrowPurse
                        | Command::ThrowWaspNest
                        | Command::ThrowApple
                        | Command::ThrowStone => {
                            if self.instruct_tie_cmd(assets, owner, seq_id, elem_idx, cmd)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        Command::ClimbDownFromShoulders => {
                            // Owner is the climber; the carrier
                            // (helper) is read from the climber's
                            // `human.carrier` back-reference latched
                            // at climb-up time.
                            let carrier_id = self
                                .get_entity(owner)
                                .and_then(|e| e.human_data())
                                .and_then(|h| h.carrier);
                            match abilities::begin_climb_down_from_shoulders(
                                &mut self.world.entities,
                                &mut self.orders.sequence_manager,
                                owner,
                                seq_id,
                                elem_idx,
                                &mut self.orders.next_order_id,
                            ) {
                                AbilityBeginResult::Started => {
                                    self.orders
                                        .sequence_manager
                                        .element_in_progress(seq_id, elem_idx);
                                    // Helper is frozen for the
                                    // duration of the climb-down so
                                    // it can't acquire a fresh
                                    // sequence element while playing
                                    // the sync'd
                                    // TRANSITION_HELPING_CLIMBING_DOWN.
                                    if let Some(helper_id) = carrier_id {
                                        self.actor_freeze_execution(helper_id);
                                    }
                                }
                                AbilityBeginResult::Impossible => {
                                    self.orders
                                        .sequence_manager
                                        .element_impossible(seq_id, elem_idx);
                                }
                            }
                        }
                        Command::ClimbUpOnShoulders => {
                            if self.instruct_climb_up_on_shoulders(assets, owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        Command::Pay => {
                            if self.instruct_pay(owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        Command::DropAmmo => {
                            if self.instruct_drop_ammo(assets, owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        // ── Drop ale bottle ───────────────────────
                        Command::DropAle => {
                            let order_type = match self.get_entity(owner) {
                                Some(entity)
                                    if entity.element_data().posture()
                                        == crate::element::Posture::Crouched =>
                                {
                                    crate::order::OrderType::DroppingAleCrouched
                                }
                                Some(_) => crate::order::OrderType::DroppingAle,
                                None => {
                                    self.orders
                                        .sequence_manager
                                        .element_impossible(seq_id, elem_idx);
                                    break 'action;
                                }
                            };
                            self.push_new_order(seq_id, elem_idx, order_type, 0.0, 0.0);
                            self.orders
                                .sequence_manager
                                .element_in_progress(seq_id, elem_idx);
                        }
                        // ── Turn ───────────────────────────────
                        // Rotate the actor to face the `CameraPoint`
                        // property (or `Direction` property if no
                        // point), then push a single `Turning` order.
                        // The element terminates when the animation's
                        // sprite reports completion.  TURN and
                        // TURN_FAST share an identical body — both
                        // read CameraPoint / Direction from the
                        // element and push Turning onto the order
                        // queue; only Upright posture is legal.
                        Command::Turn | Command::TurnFast => {
                            let barrier = TurnCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                break 'action;
                            }
                        }

                        // Face the element's antagonist, then push
                        // Turning.  Carried by
                        // `SequenceElementData::Interaction`.
                        Command::TurnElement => {
                            let barrier = TurnCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                break 'action;
                            }
                        }

                        // Owner-ful Freeze pushes a `Freezing` order
                        // onto the element.  The engine-side
                        // immediate engine-execution arm at the bottom
                        // of this file handles non-owner Freeze
                        // (which collapses into FreezeAll).
                        Command::Freeze => {
                            let barrier = TurnCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                break 'action;
                            }
                        }

                        // ── Point / GatherSoldiers ─────────────
                        // Each pushes a single one-shot animation
                        // order (`Pointing` / `GatheringSoldiers`)
                        // with `compute_direction = false`.  Point
                        // reads `Direction` and sets the actor's
                        // facing before the anim; GatherSoldiers has
                        // no direction.  Both terminate the sequence
                        // element on animation completion, wired via
                        // `AiAnimCompletion::SequenceElement`.
                        Command::Point | Command::GatherSoldiers => {
                            let barrier = TurnCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                break 'action;
                            }
                        }

                        // ── Wait (soldier-specific override) ───
                        //   - attentive + upright + waiting + alive →
                        //     WAITING_ALERTED
                        //   - leaning out with AimingWithBow{,Down} →
                        //     AIMING_WITH_BOW_LEANING_OUT
                        //   - leaning out otherwise → LEANING_OUT
                        //   - anything else → fall through to NPC
                        //     base (not dispatched here — terminates,
                        //     which matches the existing catch-all).
                        // WAIT_TIMER additionally records `wait_time`
                        // from the element's Timer property.
                        // WAIT_FREE_LIFT is translated by the identical
                        // stationary-order path above, then rechecked by its
                        // owner after Execute.
                        Command::Wait | Command::WaitTimer => {
                            let barrier = WaitCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                                profiles: &assets.profile_manager,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                break 'action;
                            }
                        }
                        // ── NPC-specific one-shot anims ────────
                        // Each command appends one animation order
                        // with `compute_direction = false`, so we
                        // book it through `active_ai_anim` and bind
                        // sequence termination to its DONE — matching
                        // the existing `Point` arm above.  Posture
                        // flips (Upright→Sitting / Upright→Leisure)
                        // are handled by the animation-completion
                        // side effects in `animation.rs`.
                        //
                        // Instruction admission calls `generate_transition`
                        // before this command body is reached. For these NPC
                        // commands the transition flags match legacy behavior,
                        // so any needed leave-action/posture orders have
                        // already been queued ahead of the command's own
                        // animation.
                        Command::SitDown | Command::BeggarShowFace | Command::EnterLeisure => {
                            let barrier = NpcStateCommandContext {
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(cmd, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                break 'action;
                            }
                        }
                        // ── Menace / Sleep transitions ─────────
                        // Each pushes a fixed sequence of transition
                        // orders with `compute_direction = false`.
                        // The animation system's DONE/TERMINATED
                        // hooks in `animation.rs` flip posture /
                        // action_state appropriately when each order
                        // finishes. The sequence element remains selected
                        // and InProgress until its final order completes.
                        Command::StartMenace
                        | Command::StopMenace
                        | Command::StopSleep
                        | Command::LowerBowLeanOut
                        | Command::RaiseBowLeanOut => {
                            let barrier = NpcStateCommandContext {
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(cmd, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                break 'action;
                            }
                        }
                        // ── DrinkAle / Take ────────────────────
                        // DrinkAle / Take push a single interaction
                        // order whose animation (DRINKING_ALE /
                        // TAKING) references the antagonist (bottle /
                        // purse / coin).  The corresponding Execute
                        // handlers hide / remove the antagonist on
                        // DONE and bump money / blood-alcohol on
                        // TERMINATED.  Book through `active_ai_anim`
                        // with the antagonist threaded along so the
                        // `apply_soldier_execute_side_effects`
                        // handler picks up the target.
                        Command::DrinkAle | Command::Take => {
                            ObjectInteractionCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                        }

                        // ── UnlockDoor ─────────────────────────
                        // The PC pushes a single `UnlockingDoor`
                        // order (or `UnlockingTrap` when the door is
                        // a building-trap) and the door's `locked_pc`
                        // flag flips off when the lockpick animation
                        // finishes.  We book the anim via
                        // `active_ai_anim` + `UnlockDoor` completion
                        // so the flag flip + element termination
                        // happen on animation end.  Target door is
                        // read from the `Field::Door` property set
                        // by `build_gate_movement_sequence`.
                        Command::UnlockDoor => {
                            if self.instruct_unlock_door(owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }

                        // ── Jump ────────────────────────────────
                        // Build a step list covering the run-up,
                        // airborne trajectory, and landing
                        // transitions, then drive the actor through
                        // them via `tick_active_jump_for`.  If the jump
                        // can't be installed (missing data) the
                        // element is terminated so the sequence
                        // doesn't stall.
                        Command::JumpCmd => {
                            if self.start_jump(sim, assets, owner, seq_id, elem_idx) {
                                self.orders
                                    .sequence_manager
                                    .element_in_progress(seq_id, elem_idx);
                            } else {
                                tracing::warn!(
                                    entity = ?owner,
                                    seq = ?seq_id,
                                    elem = elem_idx,
                                    "Jump: failed to install ActiveJump — terminating element"
                                );
                                self.orders
                                    .sequence_manager
                                    .element_terminated(seq_id, elem_idx);
                            }
                        }

                        Command::ActivateApple
                        | Command::ActivateArrow
                        | Command::ActivateHandle
                        | Command::ActivateHeal
                        | Command::ActivateLever
                        | Command::ActivateMoney
                        | Command::ActivateSearch
                        | Command::ActivateStone
                        | Command::ActivateSword => {
                            let antagonist = match &elem.data {
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
                                _ => None,
                            };
                            let (target_handle, pc_handle, method) = TargetActivationContext {
                                entities: &self.world.entities,
                            }
                            .dispatch(owner, cmd, antagonist);
                            let key = crate::engine::ScriptVmKey::Target(target_handle);
                            let is_instantiated = self
                                .scripts
                                .mission
                                .as_ref()
                                .is_some_and(|script| script.has_script_vm(key));
                            if is_instantiated
                                && let Err(error) = self.call_script_vm(
                                    sim,
                                    assets,
                                    key,
                                    method,
                                    &[pc_handle],
                                    crate::natives::ScriptCallFrame::actor(target_handle),
                                )
                            {
                                tracing::warn!("{method} (target {target_handle}): {error}");
                            }
                            self.orders
                                .sequence_manager
                                .element_terminated(seq_id, elem_idx);
                        }

                        // Script-recorded PlayAnim / PlayAnimLoop /
                        // PlayAnimFreeze / PlayAnimFrozen. The original game translates these to
                        // PLAY_CUSTOM non-animations for actors, which
                        // then drive the stored animation identifier.
                        // FX targets instead force the target sprite
                        // animation/progression immediately.
                        Command::PlayAnim
                        | Command::PlayAnimLoop
                        | Command::PlayAnimFreeze
                        | Command::PlayAnimFrozen => {
                            let animation = match elem
                                .get_property(crate::sequence::Field::AnimationId)
                            {
                                Some(crate::sequence::FieldValue::Animation(anim)) => Some(*anim),
                                Some(crate::sequence::FieldValue::Integer(v)) => {
                                    crate::order::OrderType::try_from(*v).ok()
                                }
                                _ => None,
                            };
                            let preserve_trigger_visual = self.control.sim_config.reversible_background_patches
                                && self.script_domains.interactables.patches.iter().any(|patch| {
                                    patch.repeat_activation.as_ref().is_some_and(|(handle, _)| {
                                        *handle == crate::natives::ScriptHandleCodec::actor_handle(owner)
                                    })
                                });
                            let barrier = TargetAnimationContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                                preserve_trigger_visual,
                            }
                            .dispatch_play_animation(owner, cmd, animation, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                break 'action;
                            }
                        }

                        // PC-side target interaction commands.  Each
                        // enqueues a per-command animation order on
                        // the PC (USING_LEVER / HITTING_TARGET /
                        // HANDLING_TARGET / TAKING_TARGET /
                        // SEARCHING), and on DONE the engine launches
                        // the corresponding `Activate*` interaction
                        // element on the target antagonist.
                        //
                        // The order driver plays the PC order first;
                        // `apply_pc_target_interaction_side_effect`
                        // launches the target activation when that
                        // order reports `MotionState::Done`.
                        Command::HitTarget
                        | Command::HandleTarget
                        | Command::UseLever
                        | Command::TakeTarget
                        | Command::SearchCmd => {
                            let target = match &elem.data {
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
                                _ => None,
                            };
                            let barrier = TargetInteractionContext {
                                entities: &self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(owner, cmd, target, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                break 'action;
                            }
                        }

                        // Internal carrier for a pre-built animation order.
                        // `launch_single_order_sequence_stamped` normally
                        // promotes these synchronously, but a postponed
                        // carrier returns here as Todo when its blocker
                        // completes.  The order is already attached; keep the
                        // element alive so the actor animation driver can
                        // consume it instead of dropping the visible action.
                        Command::Generic => {
                            if elem.orders.is_empty() {
                                // The carrier's animation may have completed
                                // while this element was postponed behind a
                                // higher-priority command.  There is no
                                // command-specific Translate body left to
                                // run. Original-game actor instruction handling nevertheless
                                // writes mmotionState=IN_PROGRESS immediately
                                // after Translate returns, before discovering
                                // that the current order is null and
                                // terminating the accepted element. Preserve
                                // that otherwise-invisible acceptance edge
                                // before the state change clears the selected element.
                                self.world
                                    .entities
                                    .get_mut(owner)
                                    .and_then(Entity::actor_data_mut)
                                    .expect("accepted empty Generic lost its actor")
                                    .continuation
                                    .motion_state = crate::sprite::MotionState::InProgress;
                                self.orders.sequence_manager.set_translating_element(None);
                                self.orders
                                    .sequence_manager
                                    .element_terminated(seq_id, elem_idx);
                            } else {
                                self.orders
                                    .sequence_manager
                                    .element_in_progress(seq_id, elem_idx);
                            }
                        }

                        _ => {
                            // Dispatch for remaining owner-instructed
                            // commands will be added per-command;
                            // marking terminated here keeps the
                            // sequence ticking.  Warn so unhandled
                            // commands don't silently vanish (the
                            // Seek-vs-Move bug hid here for months
                            // because the element just terminated
                            // without any log — Seek needed dispatch
                            // through the Move path and this default
                            // arm swallowed it).
                            tracing::warn!(
                                ?cmd,
                                ?owner,
                                ?seq_id,
                                elem_idx,
                                "InstructOwner: no dispatch for command; terminating element"
                            );
                            self.orders.sequence_manager.set_translating_element(None);
                            self.orders
                                .sequence_manager
                                .element_terminated(seq_id, elem_idx);
                        }
                    }
                    // Accepted actor instruction handling publishes the translated
                    // current order through the actor-order field. Keep this write at the
                    // dispatch boundary rather than inferring it later from
                    // whichever element happens to be selected.
                    if self
                        .world
                        .entities
                        .get(owner)
                        .is_some_and(|entity| entity.actor_data().is_some())
                    {
                        self.publish_instructed_order_as_installed(owner, seq_id, elem_idx);
                    }
                    if trace_path_owner {
                        self.trace_path_owner_lifecycle(
                            "after_instruct_translation",
                            owner,
                            Some((seq_id, elem_idx)),
                        );
                    }
                    // The original game returns immediately when translation completed the
                    // element re-entrantly and changed the selected sequence element;
                    // that path deliberately does not overwrite the
                    // actor's preceding motion latch with IN_PROGRESS.
                    // Only an element that survived translation and was
                    // promoted to INPROGRESS reaches the common write
                    // below. Command-specific empty-order paths publish
                    // their required edge at the dispatch site above.
                    if self
                        .orders
                        .sequence_manager
                        .get_element(seq_id, elem_idx)
                        .is_some_and(|element| {
                            element.state == crate::sequence::SequenceState::InProgress
                        })
                        && self
                            .world
                            .entities
                            .get(owner)
                            .is_some_and(|entity| entity.actor_data().is_some())
                    {
                        accepted_instruct_owners.push(owner);
                    }
                }
                crate::sequence::SequenceAction::ExecuteImmediateOwner {
                    owner,
                    sequence_id: seq_id,
                    element_index: elem_idx,
                } => {
                    if let Some((handle, msg, arg1, arg2)) =
                        self.dispatch_execute_immediate_owner(sim, assets, owner, seq_id, elem_idx)
                    {
                        self.dispatch_sequence_messages(
                            sim,
                            assets,
                            &[(handle, msg, arg1, arg2)],
                            &[],
                        );
                        self.orders
                            .sequence_manager
                            .element_terminated(seq_id, elem_idx);
                    }
                }
                crate::sequence::SequenceAction::EngineCommand {
                    sequence_id: seq_id,
                    element_index: elem_idx,
                }
                | crate::sequence::SequenceAction::ExecuteImmediateEngine {
                    sequence_id: seq_id,
                    element_index: elem_idx,
                } => {
                    if let Some((msg, arg1, arg2)) =
                        self.dispatch_engine_or_execute_immediate(sim, assets, seq_id, elem_idx)
                    {
                        self.dispatch_sequence_messages(sim, assets, &[], &[(msg, arg1, arg2)]);
                        self.orders
                            .sequence_manager
                            .element_terminated(seq_id, elem_idx);
                    }
                }
            }
        }
    }
}
