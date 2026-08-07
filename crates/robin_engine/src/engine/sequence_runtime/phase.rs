use super::*;

impl EngineInner {
    /// Retry the front of a PC's legacy shoot list through the same
    /// Actor::Instruct admission stages used by the manager dispatcher.
    /// Returns the boolean result that `ProcessShootList` uses to decide
    /// whether to remove the retained pointer.
    pub(in crate::engine) fn instruct_held_shoot_bow(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        element_ref: crate::sequence::SequenceElementRef,
    ) -> bool {
        use crate::sequence::{SequencePriority, SequenceState};

        let seq_id = element_ref.sequence_id;
        let elem_idx = element_ref.element_index;
        let Some(element) = self.orders.sequence_manager.get_element(seq_id, elem_idx) else {
            panic!("shoot-list element {seq_id:?}/{elem_idx} disappeared");
        };
        assert_eq!(element.owner, Some(owner));
        assert_eq!(element.command, Command::ShootBow);
        if !matches!(
            element.state,
            SequenceState::Todo | SequenceState::Postponed | SequenceState::Terminated
        ) {
            panic!(
                "shoot-list element {seq_id:?}/{elem_idx} has invalid state {:?}",
                element.state
            );
        }

        if element.priority == SequencePriority::NotYetSet {
            let priority = {
                let resolver = Self::priority_resolver(&self.world.entities);
                resolver(element)
            };
            self.orders
                .sequence_manager
                .get_element_mut(seq_id, elem_idx)
                .expect("held shoot element vanished during priority resolution")
                .priority = priority;
        }

        self.stamp_element_transition_state(owner, seq_id, elem_idx);
        if self.non_interruptable_guard(owner, seq_id, elem_idx) {
            self.dispatch_condolations(sim, assets);
            return false;
        }
        if !self.generate_transition(sim, assets, owner, seq_id, elem_idx) {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            self.dispatch_condolations(sim, assets);
            return false;
        }
        if !self.arbitrate_held_shoot_instruct(seq_id, elem_idx) {
            self.dispatch_condolations(sim, assets);
            return false;
        }

        self.orders
            .sequence_manager
            .begin_instruct_callback(owner, seq_id, elem_idx);
        self.dispatch_condolations(sim, assets);
        self.orders
            .sequence_manager
            .end_instruct_callback(owner, seq_id, elem_idx);

        let target = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| match &element.data {
                crate::sequence::SequenceElementData::Interaction { antagonist } => *antagonist,
                _ => None,
            });
        let Some(target) = target else {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return true;
        };
        let ammo_count = self.get_bow_ammo_count(owner);
        if ammo_count == 0 {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return true;
        }

        let (bow_target, shoot_mode) = self.can_shoot_with_bow_at(assets, owner, target);
        if bow_target != super::input::BowTarget::Valid {
            let has_transition_orders = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .is_some_and(|element| !element.orders.is_empty());
            if has_transition_orders {
                self.orders
                    .sequence_manager
                    .element_in_progress(seq_id, elem_idx);
            } else {
                self.orders
                    .sequence_manager
                    .element_terminated(seq_id, elem_idx);
            }
            return true;
        }

        match bow_shot::begin_bow_shot(
            &mut self.world.entities,
            &mut self.orders.sequence_manager,
            owner,
            target,
            seq_id,
            elem_idx,
            false,
            ammo_count,
            Some(shoot_mode),
            &mut self.orders.next_order_id,
        ) {
            BeginShotResult::Started => self
                .orders
                .sequence_manager
                .element_in_progress(seq_id, elem_idx),
            BeginShotResult::Impossible => self
                .orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx),
        }
        true
    }

    /// Translate one Move/Seek at the exact `RHSequenceManager::Hourglass`
    /// FIFO position where its `Go()` action was emitted.
    pub(in crate::engine) fn dispatch_ordered_move_seek_instruct(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        sequence_id: crate::sequence::SequenceId,
        element_index: usize,
    ) -> OwnerActionBarrier {
        let Some((
            command,
            stored_destination,
            target_element,
            action,
            flags,
            tolerance,
            goal_sector,
            goal_layer,
        )) = self
            .orders
            .sequence_manager
            .get_element(sequence_id, element_index)
            .and_then(|element| match &element.data {
                crate::sequence::SequenceElementData::Movement {
                    destination,
                    element: target,
                    action,
                    flags,
                    tolerance,
                    sector,
                    layer,
                    ..
                } if matches!(element.command, Command::Move | Command::Seek) => Some((
                    element.command,
                    *destination,
                    *target,
                    *action,
                    *flags,
                    *tolerance,
                    *sector,
                    *layer,
                )),
                _ => None,
            })
        else {
            tracing::warn!(
                ?sequence_id,
                element_index,
                "Move/Seek action has invalid sequence-element data"
            );
            self.orders
                .sequence_manager
                .element_impossible(sequence_id, element_index);
            return OwnerActionBarrier::Skip;
        };

        let is_anonymous_archer_pc = self.get_entity(owner).is_some_and(|entity| {
            entity.is_pc()
                && entity.element_data().posture == crate::element_kinds::Posture::AnonymousArcher
        });
        if is_anonymous_archer_pc {
            tracing::trace!(
                ?owner,
                ?sequence_id,
                element_index,
                ?command,
                destination = ?stored_destination,
                target = ?target_element,
                ?flags,
                frame = self.control.frame_counter,
                "move instruct refused: anonymous archer",
            );
            self.hero_speaking(
                assets,
                owner,
                crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
            );
            self.orders
                .sequence_manager
                .element_impossible(sequence_id, element_index);
            return OwnerActionBarrier::Skip;
        }

        // Actor::Instruct disables anti-collision as soon as a MAP movement
        // is accepted, before Seek replacement or path translation.
        self.apply_map_move_instruction_side_effect(owner, sequence_id, element_index);

        let is_seek = command == Command::Seek;
        let destination = if is_seek {
            let post_seek = self
                .orders
                .sequence_manager
                .get_element_mut(sequence_id, element_index)
                .and_then(|element| match &mut element.data {
                    crate::sequence::SequenceElementData::Movement {
                        post_seek_sequence, ..
                    } => post_seek_sequence.take(),
                    _ => None,
                });
            if let Some(post_seek) = post_seek
                && let Some(actor) = self
                    .world
                    .entities
                    .get_mut(owner)
                    .and_then(|entity| entity.actor_data_mut())
            {
                actor.post_seek_sequence = Some(post_seek);
            }

            match target_element {
                Some(target) => {
                    if target == owner {
                        self.orders
                            .sequence_manager
                            .element_terminated(sequence_id, element_index);
                        self.start_post_seek_sequence(owner, None);
                        return OwnerActionBarrier::Skip;
                    }
                    // Translate(SEEK) initializes these actor fields before
                    // entering RefreshSeek. RefreshSeek can deliberately
                    // return without building orders while the target passes
                    // a door (or while both actors share a building), but the
                    // 25-frame legacy wait value and seek distance are already
                    // observable at that point.
                    let seek_distance = tolerance.max(4.0);
                    if let Some(actor) = self
                        .world
                        .entities
                        .get_mut(owner)
                        .and_then(|entity| entity.actor_data_mut())
                    {
                        actor.seek_distance = seek_distance;
                        actor.wait_time = 25;
                        actor.seek_refresh_wait = 25;
                    }
                    if self.try_handle_same_sector_actor_seek_wait(
                        owner,
                        sequence_id,
                        element_index,
                        target,
                        flags,
                    ) {
                        // Original resumes Translate after RefreshSeek's
                        // no-order return, rewrites SEEK to MOVE, and then
                        // Instruct immediately terminates the empty element.
                        // That selected MOVE condolence is authoritative for
                        // NPC EventReachPoint (notably return-to-post facing).
                        if let Some(element) = self
                            .orders
                            .sequence_manager
                            .get_element_mut(sequence_id, element_index)
                            .filter(|element| {
                                matches!(
                                    element.state,
                                    crate::sequence::SequenceState::Todo
                                        | crate::sequence::SequenceState::Postponed
                                )
                            })
                        {
                            element.command = Command::Move;
                            self.orders
                                .sequence_manager
                                .element_terminated(sequence_id, element_index);
                        }
                        return OwnerActionBarrier::Reach;
                    }
                    let target_position = self
                        .get_entity(target)
                        .unwrap_or_else(|| {
                            panic!(
                                "entity-target Seek owner {owner:?} requires missing target {target:?}"
                            )
                        })
                        .element_data()
                        .position_map();
                    if let Some(actor) = self
                        .world
                        .entities
                        .get_mut(owner)
                        .and_then(|entity| entity.actor_data_mut())
                    {
                        actor.seek_target = Some(target);
                        actor.last_seek_target_position = target_position;
                        // Original uses the single `mulWaitTime` field for
                        // both ordinary waits and the seek refresh countdown.
                        // Keep the split Rust fields identical at the launch
                        // boundary so a synchronously installed follow-up
                        // command observes TIME_SEEK_REFRESH too.
                        actor.wait_time = 25;
                        actor.seek_refresh_wait = 25;
                    }
                    if self.try_dispatch_cross_sector_entity_seek(
                        sim,
                        assets,
                        owner,
                        sequence_id,
                        element_index,
                        target,
                        action,
                        flags,
                        seek_distance,
                    ) {
                        return OwnerActionBarrier::Skip;
                    }
                    let Some(resolved) =
                        self.resolve_entity_seek(sim, assets, owner, target, flags, seek_distance)
                    else {
                        self.orders
                            .sequence_manager
                            .element_impossible(sequence_id, element_index);
                        return OwnerActionBarrier::Skip;
                    };
                    if let Some(crate::sequence::SequenceElementData::Movement {
                        destination,
                        tolerance,
                        speed_factor,
                        ..
                    }) = self
                        .orders
                        .sequence_manager
                        .get_element_mut(sequence_id, element_index)
                        .map(|element| &mut element.data)
                    {
                        *destination = resolved.destination;
                        *tolerance = resolved.tolerance;
                        *speed_factor = resolved.speed_factor;
                    }
                    resolved.destination
                }
                None => {
                    if let Some(actor) = self
                        .world
                        .entities
                        .get_mut(owner)
                        .and_then(|entity| entity.actor_data_mut())
                    {
                        actor.seek_target = None;
                        actor.last_seek_target_position = stored_destination;
                        actor.seek_distance = tolerance;
                    }
                    stored_destination
                }
            }
        } else {
            stored_destination
        };

        let owner_sector = self
            .get_entity(owner)
            .and_then(|entity| entity.element_data().sector());
        let owner_in_building = self.sector_is_building(owner_sector);
        let is_last_of_sequence = self
            .orders
            .sequence_manager
            .get_sequence(sequence_id)
            .map(|sequence| element_index + 1 >= sequence.elements.len())
            .unwrap_or(false);
        if owner_in_building && (!is_seek || !is_last_of_sequence) {
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
            return OwnerActionBarrier::Skip;
        }

        // Original `Translate(SEEK) -> RefreshSeek` does not flatten the
        // transient Seek into its concrete movement. It interrupts the
        // selected wrapper, then appends a freshly-built movement to the
        // sequence-manager's live FIFO. Keeping those as distinct elements is
        // required for faithful state/cascade ownership even when other
        // elements are already queued for this actor.
        // A point seek whose goal sector differs from the actor's own runs
        // the same gate expansion as any other cross-sector route: the
        // transient Seek is replaced by ASSERT_POSITION / gate approach /
        // PASS_DOOR legs and a trailing MOVE that keeps the SEEK flag, so the
        // post-seek interaction still fires on arrival.
        if is_seek
            && target_element.is_none()
            && self.try_dispatch_cross_sector_point_seek(
                sim,
                assets,
                owner,
                sequence_id,
                element_index,
                destination,
                goal_sector,
                goal_layer,
                action,
                flags,
                tolerance,
            )
        {
            return OwnerActionBarrier::Skip;
        }

        if is_seek {
            let Some(mut replacement_data) = self
                .orders
                .sequence_manager
                .get_element(sequence_id, element_index)
                .map(|element| element.data.clone())
            else {
                return OwnerActionBarrier::Skip;
            };
            if let crate::sequence::SequenceElementData::Movement { flags, .. } =
                &mut replacement_data
            {
                // Original Translate(SEEK) changes the command to MOVE and
                // adds RHMOVE_SEEK before RefreshSeek launches the concrete
                // movement. PerformSeek dispatch and its refresh countdown
                // are keyed by this flag, not by the now-replaced command.
                flags.insert(crate::sequence::MoveFlags::SEEK);
            }
            let mut replacement = crate::sequence::SequenceElement::new_movement(
                1,
                Command::Move,
                Some(owner),
                action,
            );
            replacement.data = replacement_data;
            self.relaunch_seek_replacement(owner, sequence_id, element_index, replacement);
            return OwnerActionBarrier::Skip;
        }

        self.dispatch_prepared_move_instruction(
            sim,
            assets,
            owner,
            sequence_id,
            element_index,
            destination,
            action,
        )
    }

    /// Launch and dispatch sequence elements after the ported base entity and
    /// actor-Hourglass work, including inline immediate-action cascades and
    /// message/target callbacks at their exact owner-dispatch positions.
    ///
    /// Original provenance: `original-code/RHengine.cpp:3726-3727` calls
    /// `RHSequenceManager::Hourglass` after the entity loop; its FIFO `Go()`
    /// drain is in `original-code/RHsequencemanager.cpp:931-943`.
    pub(in crate::engine) fn hourglass_phase_sequences(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        assets: &LevelAssets,
    ) {
        // An actor order can terminate during the preceding entity phase.
        // Original `SetState` closes its `SendCondolationCard` / `Ready`
        // stack immediately, so a postponed successor is registered before
        // `RHSequenceManager::Hourglass` starts and is instructed by that
        // same drain. Rust defers the callback to avoid re-entrant borrows;
        // close any such pre-existing stacks before collecting manager work.
        //
        // This deliberately does not process paths. A resumed Move/Seek is
        // translated below, after this frame's path phase, and its request
        // remains queued for the next frame just as in the Original.
        self.dispatch_condolations(sim, assets);

        // AI Think reached from an entity/NPC slot can launch GoTo after the
        // pre-entity order drain. Original registers that Move immediately,
        // so the sequence-manager Hourglass below still instructs it in this
        // frame. It reaches pathfinding only at next frame's earlier Paths
        // phase and therefore remains MoveWaiting meanwhile.
        self.drain_pending_move_requests(sim);

        // ── Sequence manager dispatch ────────────────────────────
        // Process pending sequence elements in the manager's emitted order.
        // Record the actual accepted Actor::Instruct boundaries. Looking at
        // the selected element after the drain is insufficient: translation
        // may validly produce no order and terminate the incoming element
        // synchronously, after Original has already written
        // mmotionState=IN_PROGRESS.
        let mut accepted_instruct_owners = Vec::new();
        let mut phase = SequencePhase::begin(&mut self.orders);

        // Dispatch each action at its exact FIFO position. In particular,
        // Move/Seek translation must not leap ahead of an earlier script
        // callback in this same batch.
        //
        // Pop actions one at a time and drain any synchronous
        // immediate-dispatch follow-ups produced by cascades inside
        // each action (e.g. an `element_terminated` whose
        // `signal_ready` re-registers the next element which happens
        // to be Speak / Teleport / etc.).  Successors land at the
        // front of the action queue, so they fire before the next
        // non-immediate action in the batch rather than waiting for
        // the next `Hourglass()`.
        while let Some(action) = phase.pop_action_after_registration(&mut self.orders) {
            // Translation selection never outlives the dispatch that
            // installed it; the arms below abandon the action early on many
            // rejection paths.
            self.orders.sequence_manager.set_translating_element(None);
            // Abandoning an action skips the rest of *that action's* work —
            // never the epilogue below. A rejected command can still have
            // terminated its element during translation, and the resulting
            // `SetState` card, its `Ready()` continuation, and the successor
            // element it registers all belong to this same manager drain.
            // Falling out of the whole loop instead would strand the
            // successor until the next frame and leave the actor orderless.
            'action: {
                match action {
                    crate::sequence::SequenceAction::InstructOwner {
                        owner,
                        sequence_id: seq_id,
                        element_index: elem_idx,
                    } => {
                        // PC::Instruct redirects a TO_JUMP Move from a rider to
                        // the carrier before Human/Actor Instruct sees it. This
                        // must sample the live posture here, not when the element
                        // was registered earlier in the frame.
                        let owner =
                            self.redirect_queued_move_to_jump_if_carried(owner, seq_id, elem_idx);
                        // Human::Instruct owns this guard, before
                        // Actor::Instruct resolves priority, stamps transition
                        // state, generates orders, or arbitrates. The action has
                        // already been detached from the manager FIFO by this
                        // phase, so retaining the exact ref is sufficient.
                        let command = self
                            .orders
                            .sequence_manager
                            .get_element(seq_id, elem_idx)
                            .map(|element| element.command);
                        if command
                            .is_some_and(|command| self.pc_should_hold_shoot_bow(owner, command))
                        {
                            self.queue_pc_shoot_bow(
                                owner,
                                crate::sequence::SequenceElementRef::new(seq_id, elem_idx),
                            );
                            break 'action;
                        }

                        // Every RHElementActor::Instruct call begins by asking
                        // the owner to DeterminePriority when the serialized
                        // element still carries NOT_YET_SET. Prebuilt and loaded
                        // sequences can reach this manager boundary without
                        // passing through the eager single-element launch
                        // wrappers, so resolve them here too. In particular an
                        // active PASS_DOOR must become NON_INTERRUPTABLE before a
                        // same-frame AI Move is instructed.
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

                        // Every RHElementActor::Instruct snapshots the actor's
                        // current posture and action state before the
                        // non-interruptable guard, transition generation, and
                        // ordinary priority arbitration. Freshly launched
                        // elements are eagerly stamped; a postponed element marks
                        // that snapshot Undefined when it is released because
                        // this is its second Instruct boundary.
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
                        // Original `RHElementActor::Instruct` handles a selected
                        // NON_INTERRUPTABLE element before GenerateTransition.
                        // This matters for commands arriving while a door pass
                        // temporarily owns a posture (Flying/OnWall) from which
                        // the incoming command cannot yet generate its ordinary
                        // posture transition. The command is postponed and only
                        // generates that transition after the door pass releases
                        // it. The guard can also interrupt an older postponed
                        // equal-priority command, so settle any resulting card at
                        // this exact Instruct boundary.
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
                        if !self.arbitrate_instruct(seq_id, elem_idx) {
                            // Abandon/Impossible calls SetState synchronously in
                            // Original too. Postpone produces no card, making this
                            // drain a no-op for that arm.
                            self.dispatch_condolations(sim, assets);
                            break 'action;
                        }
                        // Original priority arbitration interrupts/postpones the
                        // outgoing element through `SetState`, whose
                        // `SendCondolationCard` callback completes synchronously
                        // before `Instruct` continues into transition generation
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
                        self.orders
                            .sequence_manager
                            .end_instruct_callback(owner, seq_id, elem_idx);
                        // Skip elements whose state moved to terminal /
                        // interrupted while an earlier action in this batch
                        // arbitrated against them. Without this, the loop
                        // would try to dispatch a non-live element
                        // and hit `set_element_state: Terminated from
                        // illegal state Interrupted`.
                        let cmd = match self.orders.sequence_manager.get_element(seq_id, elem_idx) {
                            Some(e) => {
                                use crate::sequence::SequenceState;
                                if !matches!(
                                    e.state,
                                    SequenceState::Todo | SequenceState::Postponed
                                ) {
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
                        let elem = match self.orders.sequence_manager.get_element(seq_id, elem_idx)
                        {
                            Some(e) => e,
                            None => break 'action,
                        };
                        // Do not run a generic human validity check here.
                        // Original Human::Instruct delegates directly to
                        // Actor::Instruct after its dead/unconscious and repeated
                        // PC bow-shot guards. Commands that require live
                        // revalidation do so in their specific Execute
                        // initialization arm; WakeUp, for example, deliberately
                        // has no position-validity check during instruction.
                        match cmd {
                            Command::Move | Command::Seek => {
                                self.dispatch_ordered_move_seek_instruct(
                                    sim, assets, owner, seq_id, elem_idx,
                                );
                            }
                            Command::ShootBow | Command::ShootBowOnce => {
                                let shoot_once = cmd == Command::ShootBowOnce;
                                let antagonist = match &elem.data {
                                    crate::sequence::SequenceElementData::Interaction {
                                        antagonist,
                                    } => *antagonist,
                                    _ => None,
                                };
                                let target = match antagonist {
                                    Some(t) => t,
                                    None => {
                                        // No target — nothing we can do.
                                        self.orders
                                            .sequence_manager
                                            .element_impossible(seq_id, elem_idx);
                                        break 'action;
                                    }
                                };
                                // Original rejects zero ammo here only for PCs.
                                // Scripted NPC shots remain valid with an empty
                                // counter (the release build's later decrement
                                // saturates it at zero).
                                let ammo_count = self.get_bow_ammo_count(owner);
                                let owner_is_pc =
                                    self.get_entity(owner).is_some_and(|entity| entity.is_pc());
                                if owner_is_pc && ammo_count == 0 {
                                    self.orders
                                        .sequence_manager
                                        .element_impossible(seq_id, elem_idx);
                                    break 'action;
                                }

                                // Determine shoot mode via
                                // `can_shoot_with_bow_at` before
                                // beginning the shot.
                                let (bow_target, shoot_mode) =
                                    self.can_shoot_with_bow_at(assets, owner, target);
                                if bow_target != super::input::BowTarget::Valid {
                                    tracing::debug!(
                                        ?owner,
                                        ?target,
                                        ?bow_target,
                                        "ShootBow body rejected after preserving its transition prefix"
                                    );

                                    // Human::Instruct generates the action
                                    // transition before Human::Translate checks
                                    // CanShootWithBowAt. An out-of-range or
                                    // obstructed shot therefore still equips and
                                    // loads the bow, then completes normally with
                                    // no shoot-body orders. It is not an
                                    // Impossible element. This is visible for
                                    // scripted training shots whose target has
                                    // moved outside the configured bow range.
                                    let has_transition_orders = self
                                        .orders
                                        .sequence_manager
                                        .get_element(seq_id, elem_idx)
                                        .is_some_and(|element| !element.orders.is_empty());
                                    if has_transition_orders {
                                        self.orders
                                            .sequence_manager
                                            .element_in_progress(seq_id, elem_idx);
                                    } else {
                                        self.orders
                                            .sequence_manager
                                            .element_terminated(seq_id, elem_idx);
                                    }
                                    // Fall through to the shared mpOrder
                                    // publication: the rejected body still leaves
                                    // an accepted element whose transition prefix
                                    // becomes the actor's current order this same
                                    // frame.
                                } else {
                                    match bow_shot::begin_bow_shot(
                                        &mut self.world.entities,
                                        &mut self.orders.sequence_manager,
                                        owner,
                                        target,
                                        seq_id,
                                        elem_idx,
                                        shoot_once,
                                        ammo_count,
                                        Some(shoot_mode),
                                        &mut self.orders.next_order_id,
                                    ) {
                                        BeginShotResult::Started => {
                                            self.orders
                                                .sequence_manager
                                                .element_in_progress(seq_id, elem_idx);
                                        }
                                        BeginShotResult::Impossible => {
                                            self.orders
                                                .sequence_manager
                                                .element_impossible(seq_id, elem_idx);
                                        }
                                    }
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
                                if let crate::sequence::SequenceElementData::Movement {
                                    destination,
                                    layer: _,
                                    sector,
                                    direction,
                                    ..
                                } = &elem.data
                                {
                                    let dest = *destination;
                                    let tgt_sector = *sector;
                                    let tgt_direction = *direction;

                                    // Verify actor is in expected sector
                                    let actor_sector = self
                                        .get_entity(owner)
                                        .and_then(|e| e.element_data().sector());

                                    if tgt_sector.is_some() && actor_sector != tgt_sector {
                                        self.orders.sequence_manager.element_interrupted(
                                            seq_id,
                                            elem_idx,
                                            crate::sequence::CascadeFlags::NEXT_LEVEL,
                                        );
                                        break 'action;
                                    }

                                    self.finalize_special_move_position(
                                        assets,
                                        owner,
                                        super::special_motion::SpecialMovePosition::Map(dest),
                                        // The encoded topology is the expected
                                        // source, not a destination assignment.
                                        // C++ only changes the map point here.
                                        None,
                                        None,
                                        // C++ ChangePosition keeps the current
                                        // obstacle/plane and recomputes the 3D
                                        // position against it. This matters for
                                        // geometry-less building sectors, whose
                                        // elevation comes from the plane selected
                                        // while entering the building.
                                        None,
                                        "ChangePosition",
                                    );
                                    if let Some(entity) = self.world.entities.get_mut(owner) {
                                        // `SetDirectionInstantly` from the
                                        // element's direction field so a
                                        // ChangePosition can rotate the
                                        // actor in the same step.
                                        entity
                                            .element_data_mut()
                                            .set_direction_instantly(tgt_direction);
                                    }
                                }
                                self.orders
                                    .sequence_manager
                                    .element_terminated(seq_id, elem_idx);
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
                            // RHElementActor::Hourglass rather than this one-shot
                            // Instruct boundary.
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
                                let strike = match elem.command {
                                    Command::SwordstrikeThrustA => crate::weapons::SwordStrike::A,
                                    Command::SwordstrikeThrustB => crate::weapons::SwordStrike::B,
                                    Command::SwordstrikeThrustC => crate::weapons::SwordStrike::C,
                                    Command::SwordstrikeThrustD => crate::weapons::SwordStrike::D,
                                    Command::SwordstrikeThrustE => crate::weapons::SwordStrike::E,
                                    Command::SwordstrikeThrustF => crate::weapons::SwordStrike::F,
                                    Command::SwordstrikeThrustG => crate::weapons::SwordStrike::G,
                                    Command::SwordstrikeThrustH => crate::weapons::SwordStrike::H,
                                    Command::SwordstrikeThrustI => crate::weapons::SwordStrike::I,
                                    _ => unreachable!(),
                                };
                                let target = match &elem.data {
                                    crate::sequence::SequenceElementData::Interaction {
                                        antagonist,
                                    } => *antagonist,
                                    _ => None,
                                };
                                match target {
                                    Some(target_id) => {
                                        self.dispatch_sword_strike(
                                            sim, assets, owner, target_id, strike, seq_id, elem_idx,
                                        );
                                    }
                                    None => {
                                        self.orders
                                            .sequence_manager
                                            .element_impossible(seq_id, elem_idx);
                                    }
                                }
                            }

                            // ── Swordfight enter/quit ───────────────
                            Command::EnterSwordfight | Command::PrepareSwordfight => {
                                let opponent =
                                    match elem.get_property(crate::sequence::Field::Opponent) {
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
                                self.dispatch_quit_swordfight(sim, assets, owner, seq_id, elem_idx);
                            }

                            // ── Parry commands ──────────────────────
                            Command::ParrySword => {
                                self.dispatch_parry_sword(owner, false, seq_id, elem_idx);
                            }
                            Command::ParrySwordLow => {
                                self.dispatch_parry_sword(owner, true, seq_id, elem_idx);
                            }
                            Command::StopParrySword => {
                                self.dispatch_stop_parry(owner, seq_id, elem_idx);
                            }

                            // ── Damage reception commands ───────────
                            Command::ReceiveSwordDamage
                            | Command::ReceiveDamage
                            | Command::ReceiveArrowDamage
                            | Command::ReceiveStoneDamage
                            | Command::ReceiveHitDamage
                            | Command::ReceiveMobileDamage
                            | Command::ReceiveNet => {
                                self.dispatch_receive_damage(sim, assets, owner, seq_id, elem_idx);
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
                                let barrier = NpcAttentionCommandContext {
                                    entities: &mut self.world.entities,
                                    sequence_manager: &mut self.orders.sequence_manager,
                                    next_order_id: &mut self.orders.next_order_id,
                                }
                                .dispatch(owner, cmd, seq_id, elem_idx);
                                debug_assert_eq!(barrier, OwnerActionBarrier::Reach);
                            }

                            // ── Wasp sting ─────────────────────────
                            Command::ReceiveWaspSting => {
                                self.dispatch_receive_wasp_sting(
                                    sim, assets, owner, seq_id, elem_idx,
                                );
                            }

                            // ── Stealth posture commands ────────────
                            Command::CrouchDown
                            | Command::CrouchUp
                            | Command::EnterBeggar
                            | Command::LeaveBeggar
                            | Command::EnterHelpingClimb
                            | Command::LeaveHelpingClimb
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
                                let follow_up = crate::engine::melee::ShieldCommandContext::new(
                                    &mut self.world.entities,
                                    &mut self.orders.sequence_manager,
                                    &mut self.orders.next_order_id,
                                )
                                .dispatch(owner, cmd, seq_id, elem_idx);
                                if let Some(follow_up) = follow_up {
                                    // `RHElementActorPC::Translate(RAISE_SHIELD)`
                                    // launches this SEEK synchronously. Route it
                                    // through the full owned-element Instruct path
                                    // before the action-loop splice below.
                                    self.launch_element(follow_up);
                                }
                            }
                            // ── Bow equip / raise / lower ───────────
                            //
                            // C++ RHElementActorHuman::Translate appends
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
                                let antagonist = match &elem.data {
                                    crate::sequence::SequenceElementData::Interaction {
                                        antagonist,
                                    } => *antagonist,
                                    _ => None,
                                };
                                let posture_after = elem.posture_after_transition;
                                let Some(holder) = antagonist else {
                                    self.orders
                                        .sequence_manager
                                        .element_impossible(seq_id, elem_idx);
                                    break 'action;
                                };
                                let (is_holding, holder_protected) = self
                                    .get_entity(holder)
                                    .map(|e| {
                                        let h = e
                                            .actor_data()
                                            .map(|a| a.action_state.is_shield())
                                            .unwrap_or(false);
                                        let p = e.pc_data().and_then(|pc| pc.shield_protected);
                                        (h, p)
                                    })
                                    .unwrap_or((false, None));
                                if !is_holding || holder_protected.is_some() {
                                    self.orders.sequence_manager.element_interrupted(
                                        seq_id,
                                        elem_idx,
                                        crate::sequence::CascadeFlags::NEXT_LEVEL,
                                    );
                                    break 'action;
                                }
                                if posture_after != crate::element::Posture::Crouched {
                                    let id = self.orders.allocate_order_id();
                                    let mut order = crate::order::Order::new(
                                        crate::order::OrderType::TransitionCrouchingDown,
                                        0.0,
                                        0.0,
                                        id,
                                    );
                                    order.compute_direction = false;
                                    self.orders
                                        .sequence_manager
                                        .push_order_on(seq_id, elem_idx, order);
                                }
                                let id = self.orders.allocate_order_id();
                                let mut order = crate::order::Order::new(
                                    crate::order::OrderType::HidingBehindShield,
                                    0.0,
                                    0.0,
                                    id,
                                )
                                .with_antagonist(holder);
                                order.compute_direction = false;
                                self.orders
                                    .sequence_manager
                                    .push_order_on(seq_id, elem_idx, order);
                                self.orders
                                    .sequence_manager
                                    .element_in_progress(seq_id, elem_idx);
                            }

                            // ── Other sword-related commands ────────
                            Command::SwordstrikeDown => {
                                let antagonist = self
                                    .orders
                                    .sequence_manager
                                    .get_element(seq_id, elem_idx)
                                    .and_then(|elem| match &elem.data {
                                        crate::sequence::SequenceElementData::Interaction {
                                            antagonist,
                                        } => *antagonist,
                                        _ => None,
                                    });
                                let Some(target) = antagonist else {
                                    tracing::warn!(
                                        ?seq_id,
                                        elem_idx,
                                        "SwordstrikeDown missing antagonist"
                                    );
                                    self.orders
                                        .sequence_manager
                                        .element_impossible(seq_id, elem_idx);
                                    break 'action;
                                };
                                let (tx, ty, dir) =
                                    match (self.get_entity(owner), self.get_entity(target)) {
                                        (Some(owner_entity), Some(target_entity)) => {
                                            let owner_pos =
                                                owner_entity.element_data().position_map();
                                            let target_pos =
                                                target_entity.element_data().position_map();
                                            let dir =
                                                crate::position_interface::vector_to_sector_0_to_15(
                                                    target_pos.x - owner_pos.x,
                                                    target_pos.y - owner_pos.y,
                                                );
                                            (target_pos.x, target_pos.y, dir)
                                        }
                                        _ => {
                                            tracing::warn!(
                                                ?owner,
                                                ?target,
                                                "SwordstrikeDown owner or target missing"
                                            );
                                            self.orders
                                                .sequence_manager
                                                .element_impossible(seq_id, elem_idx);
                                            break 'action;
                                        }
                                    };
                                if let Some(entity) = self.world.entities.get_mut(owner) {
                                    entity.element_data_mut().set_direction_instantly(dir);
                                    if let Some(actor) = entity.actor_data_mut() {
                                        actor.clear_path();
                                    }
                                }
                                let mut order = crate::order::Order::new(
                                    crate::order::OrderType::StrikingDownSword,
                                    tx,
                                    ty,
                                    self.orders.allocate_order_id(),
                                )
                                .with_antagonist(target);
                                order.compute_direction = false;
                                self.orders
                                    .sequence_manager
                                    .push_order_on(seq_id, elem_idx, order);
                                self.orders
                                    .sequence_manager
                                    .element_in_progress(seq_id, elem_idx);
                            }
                            Command::GetKilledAtBottom => {
                                let killer = self
                                    .orders
                                    .sequence_manager
                                    .get_element(seq_id, elem_idx)
                                    .and_then(|elem| match elem.data {
                                        crate::sequence::SequenceElementData::Interaction {
                                            antagonist,
                                        } => antagonist,
                                        _ => None,
                                    });
                                let Some(victim) = self.world.entities.get_mut(owner) else {
                                    self.orders
                                        .sequence_manager
                                        .element_impossible(seq_id, elem_idx);
                                    break 'action;
                                };
                                let damage = victim
                                    .human_and_life_points_mut()
                                    .map(|(_, lp)| (*lp).max(0) as u16);
                                let Some(damage) = damage else {
                                    tracing::warn!(
                                        ?owner,
                                        ?killer,
                                        "GetKilledAtBottom owner is not a human"
                                    );
                                    self.orders
                                        .sequence_manager
                                        .element_impossible(seq_id, elem_idx);
                                    break 'action;
                                };
                                let max_life_points = match victim {
                                    crate::element::Entity::Pc(_) => crate::combat::LIFEPOINTS_PC,
                                    crate::element::Entity::Soldier(s) => {
                                        s.soldier.cached_max_life_points
                                    }
                                    crate::element::Entity::Civilian(_) => 100,
                                    _ => 100,
                                };
                                if let Some((_, lp)) = victim.human_and_life_points_mut() {
                                    crate::combat::get_wounded(
                                        lp,
                                        damage,
                                        false,
                                        max_life_points,
                                        false,
                                    );
                                }
                                let is_rider = matches!(
                                    victim,
                                    crate::element::Entity::Soldier(s) if s.soldier.rider
                                );
                                if is_rider {
                                    let anim = victim
                                        .actor_data()
                                        .map(|actor| {
                                            let action_state = actor.action_state;
                                            if action_state.is_sword()
                                                || action_state
                                                    == crate::element::ActionState::Menacing
                                            {
                                                crate::order::OrderType::DyingSword
                                            } else if action_state.is_bow() {
                                                crate::order::OrderType::DyingBow
                                            } else {
                                                crate::order::OrderType::DyingUpright
                                            }
                                        })
                                        .unwrap_or(crate::order::OrderType::DyingUpright);
                                    self.push_new_order(seq_id, elem_idx, anim, 0.0, 0.0);
                                    self.orders
                                        .sequence_manager
                                        .element_in_progress(seq_id, elem_idx);
                                } else {
                                    if victim.is_dead() {
                                        victim.set_posture(crate::element::Posture::DeadBack);
                                    }
                                    self.orders
                                        .sequence_manager
                                        .element_terminated(seq_id, elem_idx);
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
                                let target = match &elem.data {
                                    crate::sequence::SequenceElementData::Interaction {
                                        antagonist,
                                    } => *antagonist,
                                    _ => None,
                                };
                                match target {
                                    Some(target_id) => {
                                        match abilities::begin_carry(
                                            &mut self.world.entities,
                                            &mut self.orders.sequence_manager,
                                            owner,
                                            target_id,
                                            seq_id,
                                            elem_idx,
                                            &mut self.orders.next_order_id,
                                        ) {
                                            AbilityBeginResult::Started => {
                                                self.orders
                                                    .sequence_manager
                                                    .element_in_progress(seq_id, elem_idx);
                                                // Freezing the target and
                                                // starting its hulk belong to
                                                // the pickup order's first
                                                // Execute, not to translation:
                                                // the carrier's slot for this
                                                // frame has already run, so the
                                                // body keeps its own selected
                                                // order until the carrier
                                                // actually begins lifting it.
                                            }
                                            AbilityBeginResult::Impossible => {
                                                self.orders
                                                    .sequence_manager
                                                    .element_impossible(seq_id, elem_idx);
                                            }
                                        }
                                    }
                                    None => {
                                        self.orders
                                            .sequence_manager
                                            .element_impossible(seq_id, elem_idx);
                                    }
                                }
                            }
                            Command::DropCorpse => {
                                match abilities::begin_drop(
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
                                        // Drop-transition init twin of
                                        // the pickup building flash.
                                        let carried_id = self
                                            .get_entity(owner)
                                            .and_then(|e| e.pc_data())
                                            .and_then(|pc| pc.carried);
                                        if let Some(cid) = carried_id {
                                            // Re-freeze the carried on
                                            // drop init.  The victim is
                                            // normally already frozen
                                            // from the carry, but this
                                            // idempotently re-runs the
                                            // cascade-interrupt so any
                                            // element that slipped onto
                                            // the carried (e.g. a
                                            // script-driven
                                            // `ActionChange`) is
                                            // interrupted.
                                            self.actor_freeze_execution(cid);
                                            self.apply_carry_building_hulk(owner, cid);
                                        }
                                    }
                                    AbilityBeginResult::Impossible => {
                                        self.orders
                                            .sequence_manager
                                            .element_impossible(seq_id, elem_idx);
                                    }
                                }
                            }
                            Command::StrangleCmd => {
                                let Some(target) = self
                                    .orders
                                    .sequence_manager
                                    .get_element(seq_id, elem_idx)
                                    .and_then(|element| match &element.data {
                                        crate::sequence::SequenceElementData::Interaction {
                                            antagonist,
                                            ..
                                        } => *antagonist,
                                        _ => None,
                                    })
                                else {
                                    self.orders
                                        .sequence_manager
                                        .element_impossible(seq_id, elem_idx);
                                    break 'action;
                                };
                                match abilities::begin_strangle(
                                    &mut self.world.entities,
                                    &mut self.orders.sequence_manager,
                                    owner,
                                    target,
                                    seq_id,
                                    elem_idx,
                                    &mut self.orders.next_order_id,
                                ) {
                                    AbilityBeginResult::Impossible => self
                                        .orders
                                        .sequence_manager
                                        .element_impossible(seq_id, elem_idx),
                                    AbilityBeginResult::Started => {
                                        self.orders
                                            .sequence_manager
                                            .element_in_progress(seq_id, elem_idx);

                                        // Translate inserts the Strangling order before calling
                                        // a moving antagonist's Think(EVENT_STOP). Think and all
                                        // of its re-entrant effects finish in this stack frame,
                                        // before Perform's initialization acquires AILOCK_FREEZE.
                                        let moving = self
                                        .get_entity(target)
                                        .unwrap_or_else(|| {
                                            panic!(
                                                "strangle victim {target:?} vanished after translation for {seq_id:?}/{elem_idx}"
                                            )
                                        })
                                        .actor_data()
                                        .unwrap_or_else(|| {
                                            panic!(
                                                "strangle victim {target:?} lost actor state after translation"
                                            )
                                        })
                                        .action_state
                                        .is_moving();
                                        if moving {
                                            self.dispatch_synchronous_ai_think_preserving_detection_fifo(
                                            sim,
                                            target,
                                            assets,
                                            crate::ai::Stimulus::new(
                                                crate::ai::StimulusType::EventStop,
                                            ),
                                        );
                                        }
                                    }
                                }
                            }
                            Command::TieCmd
                            | Command::HealCmd
                            | Command::WhistleCmd
                            | Command::EatCmd
                            | Command::HitCmd
                            | Command::ReceivePurse
                            | Command::EnterListen
                            | Command::LeaveListen
                            | Command::ThrowNet
                            | Command::ThrowPurse
                            | Command::ThrowWaspNest
                            | Command::ThrowApple
                            | Command::ThrowStone => {
                                let ammo_available = match cmd {
                                    Command::HealCmd => {
                                        self.has_ammo(owner, crate::profiles::Action::Heal)
                                    }
                                    Command::EatCmd => self
                                        .get_entity(owner)
                                        .and_then(|entity| match entity {
                                            Entity::Pc(pc) => {
                                                self.pc_description_for_pc_data(&pc.pc)
                                            }
                                            _ => None,
                                        })
                                        .is_some_and(|description| {
                                            description
                                                .status
                                                .get_ammo(crate::profiles::Action::Eat)
                                                > 0
                                        }),
                                    Command::ThrowNet => {
                                        self.has_ammo(owner, crate::profiles::Action::Net)
                                    }
                                    Command::ThrowPurse => {
                                        self.has_ammo(owner, crate::profiles::Action::Purse)
                                    }
                                    Command::ThrowWaspNest => {
                                        self.has_ammo(owner, crate::profiles::Action::WaspNest)
                                    }
                                    Command::ThrowApple => {
                                        self.has_ammo(owner, crate::profiles::Action::Apple)
                                    }
                                    Command::ThrowStone => {
                                        self.has_ammo(owner, crate::profiles::Action::Stone)
                                    }
                                    _ => true,
                                };
                                let barrier = DirectAbilityCommandContext {
                                    entities: &mut self.world.entities,
                                    sequence_manager: &mut self.orders.sequence_manager,
                                    next_order_id: &mut self.orders.next_order_id,
                                    profiles: &assets.profile_manager,
                                }
                                .dispatch(
                                    owner,
                                    cmd,
                                    ammo_available,
                                    seq_id,
                                    elem_idx,
                                );
                                if barrier == OwnerActionBarrier::Skip {
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
                                // Owner is the climber, antagonist is the
                                // HelpingToClimb helper.
                                let helper = match &elem.data {
                                    crate::sequence::SequenceElementData::Interaction {
                                        antagonist,
                                    } => *antagonist,
                                    _ => None,
                                };
                                let Some(helper_id) = helper else {
                                    self.orders
                                        .sequence_manager
                                        .element_impossible(seq_id, elem_idx);
                                    break 'action;
                                };
                                // Disjoint-field obstacle list so the headroom
                                // ray-cast inside `begin_climb_on_shoulders`
                                // can run alongside the `&mut self.world.entities`
                                // borrow.
                                let obstacles = crate::sight_obstacle::ObstacleList {
                                    static_obstacles: assets.static_sight_obstacles.as_slice(),
                                    dynamic_obstacles: &self.world.dynamic_sight_obstacles,
                                    static_active: &self.world.static_sight_obstacle_active,
                                };
                                match abilities::begin_climb_on_shoulders(
                                    &mut self.world.entities,
                                    &mut self.orders.sequence_manager,
                                    owner,
                                    helper_id,
                                    seq_id,
                                    elem_idx,
                                    &mut self.orders.next_order_id,
                                    obstacles,
                                ) {
                                    crate::abilities::ClimbResult::Started => {
                                        self.orders
                                            .sequence_manager
                                            .element_in_progress(seq_id, elem_idx);
                                        // Helper is frozen for the
                                        // duration of the climb.
                                        self.actor_freeze_execution(helper_id);
                                    }
                                    crate::abilities::ClimbResult::Impossible => {
                                        self.orders
                                            .sequence_manager
                                            .element_impossible(seq_id, elem_idx);
                                    }
                                    crate::abilities::ClimbResult::NoHeadroom { helper_id } => {
                                        // Low ceiling → helper stands
                                        // back up (LeaveHelpingClimb) and
                                        // the climber's element is
                                        // Impossible.
                                        let leave_elem = crate::sequence::SequenceElement::new(
                                            1,
                                            crate::element::Command::LeaveHelpingClimb,
                                            Some(helper_id),
                                        );
                                        self.launch_element(leave_elem);
                                        self.orders
                                            .sequence_manager
                                            .element_impossible(seq_id, elem_idx);
                                    }
                                }
                            }
                            Command::Pay => {
                                // Validate campaign has enough ransom.
                                // The original aborts with the post-walk
                                // validity check if ransom dropped
                                // mid-sequence.  We pre-check on launch;
                                // a race where ransom becomes
                                // insufficient between the click and the
                                // animation is acceptable (next frame's
                                // completion handler would just not
                                // deduct — see PayDone branch).
                                let beggar = match &elem.data {
                                    crate::sequence::SequenceElementData::Interaction {
                                        antagonist,
                                    } => *antagonist,
                                    _ => None,
                                };
                                match beggar {
                                    Some(beggar_id) => {
                                        match abilities::begin_pay(
                                            &mut self.world.entities,
                                            &mut self.orders.sequence_manager,
                                            owner,
                                            beggar_id,
                                            seq_id,
                                            elem_idx,
                                            &mut self.orders.next_order_id,
                                        ) {
                                            AbilityBeginResult::Started => {
                                                // HERO_GIVE_MONEY speech
                                                // cue.
                                                self.hero_speaking(
                                                    assets,
                                                    owner,
                                                    crate::engine::melee::HERO_GIVE_MONEY,
                                                );
                                                self.orders
                                                    .sequence_manager
                                                    .element_in_progress(seq_id, elem_idx);
                                            }
                                            AbilityBeginResult::Impossible => {
                                                self.orders
                                                    .sequence_manager
                                                    .element_impossible(seq_id, elem_idx);
                                            }
                                        }
                                    }
                                    None => {
                                        self.orders
                                            .sequence_manager
                                            .element_impossible(seq_id, elem_idx);
                                    }
                                }
                            }
                            Command::DropAmmo => {
                                // Decrement the PC's ammo for the action,
                                // then either merge into an adjacent
                                // just-dropped bonus (same action,
                                // combined quantity ≤ 5) or spawn a fresh
                                // `ElementBonus` at the PC's position.
                                // We skip the TAKING animation frames
                                // (the original plays a taking animation
                                // during the drop) and apply the effect
                                // in one step — the observable result is
                                // the same: ammo goes down, a bonus
                                // appears.
                                //
                                // Merge gate: when the PC hasn't moved or
                                // turned since its last drop AND the
                                // previous bonus is still active AND same
                                // action AND combined quantity ≤
                                // `MAX_AMMO_PER_PILE`, the existing pile's
                                // quantity is bumped; otherwise a fresh
                                // bonus spawns. When the previous bonus is
                                // still active but the merge cap is reached
                                // (or it's a different action), the PC's
                                // facing rotates +1 sector so the next
                                // drop's "same direction" check fails and a
                                // fresh pile spawns again.
                                const MAX_AMMO_PER_PILE: u16 = 5;
                                let (action_id, amount) = match &elem.data {
                                    crate::sequence::SequenceElementData::Generic {
                                        properties,
                                    } => {
                                        let a = properties
                                            .get(&crate::sequence::Field::ActionId)
                                            .and_then(|v| match v {
                                                crate::sequence::FieldValue::Integer(n) => Some(*n),
                                                _ => None,
                                            });
                                        let q = properties
                                            .get(&crate::sequence::Field::Amount)
                                            .and_then(|v| match v {
                                                crate::sequence::FieldValue::Integer(n) => Some(*n),
                                                _ => None,
                                            });
                                        (a, q)
                                    }
                                    _ => (None, None),
                                };
                                let Some(action_id) = action_id else {
                                    self.orders
                                        .sequence_manager
                                        .element_impossible(seq_id, elem_idx);
                                    break 'action;
                                };
                                let requested = amount.unwrap_or(1) as u16;
                                let action = crate::profiles::Action::from_u32(action_id);
                                // `get_ammo` returns `u16::MAX` (0xFFFF)
                                // for actions without an ammo counter
                                // (pc_status.rs:368-386), so
                                // `!action_uses_ammo` is the equivalent
                                // sentinel test.  Treat this as terminate,
                                // not impossible.
                                if !crate::inventory::action_uses_ammo(action) {
                                    self.orders
                                        .sequence_manager
                                        .element_terminated(seq_id, elem_idx);
                                    break 'action;
                                }
                                // Refuse the drop when no walkable cell
                                // exists near the PC's hand: skip the
                                // `DROPPING_AMMO[_CROUCHED]` order and
                                // terminate.
                                if self.try_get_drop_position(owner).is_none() {
                                    self.orders
                                        .sequence_manager
                                        .element_terminated(seq_id, elem_idx);
                                    break 'action;
                                }
                                // Capture PC
                                // position/layer/sector/obstacle for the
                                // spawned bonus.
                                let pc_snap = self.get_entity(owner).map(|e| {
                                    let el = e.element_data();
                                    (
                                        el.position_map(),
                                        el.layer(),
                                        el.sector(),
                                        el.obstacle_index(),
                                        el.direction(),
                                        el.material(),
                                    )
                                });
                                let Some((pos, layer, sector, obstacle, direction, material)) =
                                    pc_snap
                                else {
                                    self.orders
                                        .sequence_manager
                                        .element_impossible(seq_id, elem_idx);
                                    break 'action;
                                };
                                // Decrement PC ammo, clamped to current
                                // count.
                                let status_idx = self.get_entity(owner).and_then(|e| match e {
                                    crate::element::Entity::Pc(pc) => {
                                        self.pc_description_index_for_pc_data(&pc.pc)
                                    }
                                    _ => None,
                                });
                                let Some(status_idx) = status_idx else {
                                    self.orders
                                        .sequence_manager
                                        .element_impossible(seq_id, elem_idx);
                                    break 'action;
                                };
                                let dropped = if let Some(campaign) =
                                    Some(&mut self.mission_domain.campaign)
                                    && let Some(pc_desc) = campaign.characters.get_mut(status_idx)
                                {
                                    let current = pc_desc.status.get_ammo(action);
                                    let take = requested.min(current);
                                    pc_desc.status.decrease_ammo(action, take);
                                    take
                                } else {
                                    0
                                };
                                if dropped == 0 {
                                    self.orders
                                        .sequence_manager
                                        .element_impossible(seq_id, elem_idx);
                                    break 'action;
                                }
                                // Auto-disable the action slot when ammo
                                // reaches 0.  `dropped` was clamped to the
                                // available amount so "now empty" is
                                // detectable by re-reading.
                                let now_empty = Some(&self.mission_domain.campaign)
                                    .and_then(|c| c.characters.get(status_idx))
                                    .map(|d| d.status.get_ammo(action) == 0)
                                    .unwrap_or(false);
                                if now_empty {
                                    self.disable_pc_action(assets, owner, action);
                                }

                                // Merge into the previously-dropped pile if
                                // PC hasn't moved/turned and the previous
                                // bonus is still alive and accepts more.
                                let prev = self.get_entity(owner).and_then(|e| match e {
                                    crate::element::Entity::Pc(pc) => Some((
                                        pc.pc.last_dropped_ammo,
                                        pc.pc.last_ammo_dropping_position,
                                        pc.pc.last_dropping_direction,
                                    )),
                                    _ => None,
                                });
                                let same_position_and_direction = prev
                                    .map(|(_, last_pos, last_dir)| {
                                        last_pos.x == pos.x
                                            && last_pos.y == pos.y
                                            && last_dir as i16 == direction
                                    })
                                    .unwrap_or(false);
                                // `prev_bonus_state`: Some((id, current_quantity, action))
                                // if a previous pile is still active.
                                let prev_bonus_state =
                                    prev.and_then(|(last, _, _)| last).and_then(|last_id| {
                                        self.get_entity(last_id).and_then(|e| match e {
                                            crate::element::Entity::Bonus(b)
                                                if b.element.active =>
                                            {
                                                Some((
                                                    last_id,
                                                    b.object.quantity,
                                                    b.object.associated_action,
                                                ))
                                            }
                                            _ => None,
                                        })
                                    });
                                let merged = if same_position_and_direction
                                    && let Some((last_id, prev_qty, prev_action)) = prev_bonus_state
                                    && prev_action == action
                                    && prev_qty + dropped <= MAX_AMMO_PER_PILE
                                {
                                    if let Some(crate::element::Entity::Bonus(b)) =
                                        self.world.entities.get_mut(last_id)
                                    {
                                        b.object.quantity = prev_qty + dropped;
                                    }
                                    tracing::debug!(
                                        pc = ?owner,
                                        ?action,
                                        dropped,
                                        bonus = ?last_id,
                                        new_qty = prev_qty + dropped,
                                        "DropAmmo: merged into previous bonus"
                                    );
                                    true
                                } else {
                                    false
                                };

                                // When the previous bonus is still alive
                                // but we couldn't merge into it (cap reached
                                // or different action), rotate the PC by
                                // +1 sector so the next drop spawns fresh.
                                // Only fires if the PC hadn't moved/turned
                                // — otherwise the merge gate would already
                                // have rejected next time.
                                let bumped_direction = if !merged
                                    && same_position_and_direction
                                    && prev_bonus_state.is_some()
                                {
                                    let new_dir = (direction + 1).rem_euclid(16);
                                    if let Some(entity) = self.world.entities.get_mut(owner) {
                                        entity.element_data_mut().set_direction_instantly(new_dir);
                                    }
                                    new_dir
                                } else {
                                    direction
                                };

                                let spawned_id = if !merged {
                                    // Spawn a fresh bonus at the PC's
                                    // position, refined via
                                    // `find_authorized_position` to nudge
                                    // it onto a walkable cell.
                                    let spawn_pos = {
                                        let mut b = crate::coordinates::MapBBox::new();
                                        b.expand_point(pos);
                                        if self
                                            .world
                                            .fast_grid
                                            .find_authorized_position_toward(&mut b, pos, layer)
                                        {
                                            b.center()
                                        } else {
                                            pos
                                        }
                                    };
                                    let object_type =
                                        crate::inventory::action_to_object_type(action);
                                    let mut bonus_element = crate::element::ElementData {
                                        kind: crate::element::ElementKind::ObjectBonus,
                                        active: true,
                                        // Bonus default: blipped iff this
                                        // isn't a forest level.
                                        blipped: !self.world.weather.is_forest_level,
                                        ..Default::default()
                                    };
                                    bonus_element.sprite.apply_placement(
                                    spawn_pos,
                                    layer,
                                    sector,
                                    bumped_direction,
                                    material,
                                    obstacle,
                                    crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                                        obstacle,
                                        assets.static_sight_obstacles.as_slice(),
                                    ),
                                );
                                    let bonus = crate::element::Entity::Bonus(
                                        crate::element::ElementBonus {
                                            element: bonus_element,
                                            object: crate::element::ObjectData {
                                                quantity: dropped,
                                                object_type,
                                                associated_action: action,
                                                ..Default::default()
                                            },
                                        },
                                    );
                                    let bonus_id = self.add_entity(bonus);
                                    tracing::debug!(
                                        pc = ?owner,
                                        ?action,
                                        dropped,
                                        ?bonus_id,
                                        "DropAmmo: decremented PC ammo and spawned bonus"
                                    );
                                    Some(bonus_id)
                                } else {
                                    None
                                };

                                // Stamp the per-PC drop trackers so the
                                // next drop's merge gate evaluates against
                                // this drop.
                                if let Some(crate::element::Entity::Pc(pc)) =
                                    self.world.entities.get_mut(owner)
                                {
                                    pc.pc.last_ammo_dropping_position = pos;
                                    pc.pc.last_dropping_direction = bumped_direction as u8;
                                    if let Some(new_id) = spawned_id {
                                        pc.pc.last_dropped_ammo = Some(new_id);
                                    }
                                }

                                self.orders
                                    .sequence_manager
                                    .element_terminated(seq_id, elem_idx);
                            }
                            // ── Drop ale bottle ───────────────────────
                            Command::DropAle => {
                                let order_type = match self.get_entity(owner) {
                                    Some(entity)
                                        if entity.element_data().posture
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
                            // `ExecuteImmediateEngine` arm at the bottom
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
                            // Instruct admission calls `generate_transition`
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
                                let id = required_unlock_door_id(
                                    self.orders.sequence_manager.get_element(seq_id, elem_idx),
                                    seq_id,
                                    elem_idx,
                                );
                                // Pick UnlockingDoor vs UnlockingTrap
                                // by door type.
                                let anim_type = match required_canonical_door(
                                    &self.script_domains.interactables.doors,
                                    id,
                                    "UnlockDoor dispatch",
                                )
                                .door_type
                                {
                                    crate::gate::DoorType::BuildingTrap => {
                                        crate::order::OrderType::UnlockingTrap
                                    }
                                    _ => crate::order::OrderType::UnlockingDoor,
                                };
                                tracing::debug!(
                                    door_id = %id,
                                    entity = ?owner,
                                    ?anim_type,
                                    "UnlockDoor: starting lockpick animation"
                                );
                                let order = crate::order::Order::new(
                                    anim_type,
                                    0.0,
                                    0.0,
                                    self.orders.allocate_order_id(),
                                )
                                .with_completion(
                                    crate::order::OrderCompletion::UnlockDoor { door_id: id },
                                );
                                self.orders
                                    .sequence_manager
                                    .push_order_on(seq_id, elem_idx, order);
                                self.orders
                                    .sequence_manager
                                    .element_in_progress(seq_id, elem_idx);
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
                            // PlayAnimFreeze / PlayAnimFrozen.  C++ translates these to
                            // PLAY_CUSTOM non-animations for actors, which
                            // then drive the stored RHFIELD_ANIMATION_ID.
                            // FX targets instead force the target sprite
                            // animation/progression immediately.
                            Command::PlayAnim
                            | Command::PlayAnimLoop
                            | Command::PlayAnimFreeze
                            | Command::PlayAnimFrozen => {
                                let animation =
                                    match elem.get_property(crate::sequence::Field::AnimationId) {
                                        Some(crate::sequence::FieldValue::Animation(anim)) => {
                                            Some(*anim)
                                        }
                                        Some(crate::sequence::FieldValue::Integer(v)) => {
                                            crate::order::OrderType::try_from(*v).ok()
                                        }
                                        _ => None,
                                    };
                                let barrier = TargetAnimationContext {
                                    entities: &mut self.world.entities,
                                    sequence_manager: &mut self.orders.sequence_manager,
                                    next_order_id: &mut self.orders.next_order_id,
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
                                .dispatch(cmd, target, seq_id, elem_idx);
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
                                    // run. Original Actor::Instruct nevertheless
                                    // writes mmotionState=IN_PROGRESS immediately
                                    // after Translate returns, before discovering
                                    // that GetCurrentOrder() is null and
                                    // terminating the accepted element. Preserve
                                    // that otherwise-invisible acceptance edge
                                    // before SetState clears the selected element.
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
                        if self
                            .world
                            .entities
                            .get(owner)
                            .is_some_and(|entity| entity.actor_data().is_some())
                        {
                            accepted_instruct_owners.push(owner);
                        }
                        // Accepted Actor::Instruct publishes the translated
                        // current order through mpOrder. Keep this write at the
                        // dispatch boundary rather than inferring it later from
                        // whichever element happens to be selected.
                        self.publish_selected_order_for_instruct_owner(owner);
                    }
                    crate::sequence::SequenceAction::ExecuteImmediateOwner {
                        owner,
                        sequence_id: seq_id,
                        element_index: elem_idx,
                    } => {
                        if let Some((handle, msg, arg1, arg2)) = self
                            .dispatch_execute_immediate_owner(sim, assets, owner, seq_id, elem_idx)
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
                        if let Some((msg, arg1, arg2)) = self.dispatch_engine_or_execute_immediate(
                            sim, display, assets, seq_id, elem_idx,
                        ) {
                            self.dispatch_sequence_messages(sim, assets, &[], &[(msg, arg1, arg2)]);
                            self.orders
                                .sequence_manager
                                .element_terminated(seq_id, elem_idx);
                        }
                    }
                }
            }

            // `SetState` calls the owner's SendCondolationCard and resumes at
            // `Ready()` before returning to this action loop. Closing that
            // boundary here lets an immediate next-level successor preempt
            // older actions already detached into `SequencePhase`.
            self.dispatch_condolations(sim, assets);
            // Keep Rust's translation identity through its deferred
            // SendCondolationCard bookkeeping, then release it. This mirrors
            // an actor pointer, not SequenceManager's launch list: pending-
            // command queries must not interpret this selection as queued.
            self.orders.sequence_manager.set_translating_element(None);

            // After-action live-FIFO continuation: re-entrant immediate/WAIT
            // work goes to the front, while newly registered normal work is
            // appended behind actions that were already waiting.
            phase.splice_registered_actions(&mut self.orders);
        }
        self.orders.sequence_manager.set_translating_element(None);

        for owner in accepted_instruct_owners {
            // RHElementActor::Instruct writes mmotionState=IN_PROGRESS after
            // an accepted element has survived translation and entered
            // INPROGRESS. AI work in the preceding derived NPC tail only
            // registers that element; the authoritative write therefore
            // belongs here, after SequenceManager::Hourglass has actually
            // dispatched InstructOwner.
            let actor = self
                .world
                .entities
                .get_mut(owner)
                .and_then(Entity::actor_data_mut)
                .expect("accepted InstructOwner lost its actor");
            actor.continuation.motion_state = crate::sprite::MotionState::InProgress;
        }

        // The redundant-EnterSwordfight retention above is only a bridge
        // across a re-entrant actor-Hourglass lazy Wait. If that Wait is
        // published, `publish_selected_order_as_installed` consumes the marker
        // and transfers the running sprite identity. Work first instructed by
        // SequenceManager::Hourglass is already past every actor slot, so an
        // unconsumed marker here means no replacement Wait exists this frame.
        // Original's interrupted Wait has cleared mpOrder in that case.
        for (_, entity) in self.world.entities.actors_mut() {
            let actor = entity
                .actor_data_mut()
                .expect("actor iterator yielded non-actor entity");
            let Some(retained_order_id) = actor.retained_waiting_sword_order_id else {
                continue;
            };
            if actor
                .installed_order
                .is_some_and(|order| order.order_id == retained_order_id)
            {
                actor.installed_order = None;
            }
            actor.retained_waiting_sword_order_id = None;
        }
    }
}
