use super::*;

impl EngineInner {
    /// Retry the front of a PC's legacy shoot list through the same
    /// Actor-instruction admission stages used by the manager dispatcher.
    /// Returns the boolean result that shoot-list processing uses to decide
    /// whether to remove the retained pointer.
    pub(in crate::engine) fn instruct_held_shoot_bow(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        element_ref: crate::sequence::SequenceElementRef,
    ) -> bool {
        use crate::sequence::SequenceState;

        let seq_id = element_ref.sequence_id;
        let elem_idx = element_ref.element_index;
        let Some(element) = self.orders.sequence_manager.get_element(seq_id, elem_idx) else {
            panic!("shoot-list element {seq_id:?}/{elem_idx} disappeared");
        };
        assert_eq!(element.owner, Some(owner));
        assert_eq!(element.command, Command::ShootBow);
        self.stamp_element_transition_state(owner, seq_id, elem_idx);
        if self.non_interruptable_guard(sim, assets, owner, seq_id, elem_idx) {
            return false;
        }
        if !self.generate_transition(sim, assets, owner, seq_id, elem_idx) {
            self.element_impossible(sim, assets, &mut Vec::new(), seq_id, elem_idx);

            return false;
        }
        // Actor instruction checks the element state again after transition
        // generation. A retained element that became Terminated, Impossible,
        // or Interrupted must be rejected before priority arbitration or
        // translation. Shoot-list processing consequently keeps the reference
        // and the actor continues its already-started bow Wait
        // order instead of restarting that order on this frame.
        if self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .is_some_and(|element| {
                matches!(
                    element.state,
                    SequenceState::Terminated
                        | SequenceState::Impossible
                        | SequenceState::Interrupted
                )
            })
        {
            return false;
        }
        // Shoot-list processing re-enters ordinary actor instruction handling
        // body. Priority resolution therefore runs after transition generation
        // and its terminal-state guard, before priority arbitration. The
        // retained element deliberately kept NotYetSet while it was waiting
        // in mShootList; do not let that sentinel become the live shot's
        // interruption priority once it is finally admitted.
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
        if !self.arbitrate_held_shoot_instruct(sim, assets, &mut Vec::new(), seq_id, elem_idx) {
            // Priority arbitration's POSTPONE_NEW outcome has handled this instruction:
            // Original returns true even though translation does not run yet.
            // Shoot-list processing must consequently remove the retained
            // pointer; the sequence manager owns the postponed element from
            // here and will re-register it when its blocker finishes.
            return self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .is_some_and(|element| element.state == SequenceState::Postponed);
        }

        self.orders
            .sequence_manager
            .begin_instruct_callback(owner, seq_id, elem_idx);

        let still_selected = self
            .orders
            .sequence_manager
            .end_instruct_callback(owner, seq_id, elem_idx);
        if !still_selected {
            return false;
        }

        let target = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| match &element.data {
                crate::sequence::SequenceElementData::Interaction { antagonist } => *antagonist,
                _ => None,
            });
        let Some(target) = target else {
            self.element_impossible(sim, assets, &mut Vec::new(), seq_id, elem_idx);
            return true;
        };
        let ammo_count = self.get_bow_ammo_count(owner);
        if ammo_count == 0 {
            self.element_impossible(sim, assets, &mut Vec::new(), seq_id, elem_idx);
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
                self.element_in_progress(sim, assets, &mut Vec::new(), seq_id, elem_idx);
            } else {
                // Actor instruction handling writes IN_PROGRESS before publishing the
                // translated current order. An accepted shot whose body and
                // transition are both empty therefore retains that one-frame
                // motion edge even though the null order immediately
                // terminates and detaches the element.
                self.world
                    .entities
                    .get_mut(owner)
                    .and_then(Entity::actor_data_mut)
                    .expect("accepted empty held ShootBow lost its actor")
                    .continuation
                    .motion_state = crate::sprite::MotionState::InProgress;
                self.element_terminated(sim, assets, &mut Vec::new(), seq_id, elem_idx);
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
            BeginShotResult::Started => {
                self.element_in_progress(sim, assets, &mut Vec::new(), seq_id, elem_idx)
            }
            BeginShotResult::Impossible => {
                self.element_impossible(sim, assets, &mut Vec::new(), seq_id, elem_idx)
            }
        }
        true
    }

    /// Translate one Move/Seek at the exact sequence-processing
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
            recorded_gate_path,
            point_seek_route_provenance,
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
                    element.recorded_gate_path.clone(),
                    element.point_seek_route_provenance,
                )),
                _ => None,
            })
        else {
            tracing::warn!(
                ?sequence_id,
                element_index,
                "Move/Seek action has invalid sequence-element data"
            );
            self.element_impossible(sim, assets, &mut Vec::new(), sequence_id, element_index);
            return OwnerActionBarrier::Skip;
        };

        // The one SEEK exception to MOVE fallthrough is a self target.
        // Original terminates it and launches its post-seek sequence before
        // reaching the move command's source-extraction arm
        // used by ordinary movement.
        if command == Command::Seek && target_element == Some(owner) {
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
            self.element_terminated(sim, assets, &mut Vec::new(), sequence_id, element_index);
            self.start_post_seek_sequence(sim, assets, owner, None);
            return OwnerActionBarrier::Skip;
        }

        // Original-game owner instruction lets SEEK fall through the
        // move-command arm's source extraction before any seek-specific
        // handling. In particular this must
        // precede seek-refresh/cross-sector lowering: that lowering can consume
        // the wrapper without ever reaching ordinary path dispatch.
        if !self.extract_move_instruction_owner(owner) {
            self.element_impossible(sim, assets, &mut Vec::new(), sequence_id, element_index);
            return OwnerActionBarrier::Skip;
        }

        let is_anonymous_archer_pc = self.get_entity(owner).is_some_and(|entity| {
            entity.is_pc()
                && entity.element_data().posture() == crate::element_kinds::Posture::AnonymousArcher
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
            self.element_impossible(sim, assets, &mut Vec::new(), sequence_id, element_index);
            return OwnerActionBarrier::Skip;
        }

        // Actor instruction handling disables anti-collision as soon as a MAP movement
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
                    // Translate(SEEK) initializes these actor fields before
                    // entering seek refresh. Seek refresh can deliberately
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
                        // Actor SEEK translation selects entity
                        // mode before seek refresh
                        // in the point-seek mode. It deliberately
                        // leaves the serialized point sector and layer alone;
                        // The seek-to-point flag alone selects which metadata is live.
                        actor.continuation.seek_to_point = false;
                        actor.seek_distance = seek_distance;
                        actor.wait_time = 25;
                        actor.seek_refresh_wait = 25;
                    }
                    if self.try_handle_same_sector_actor_seek_wait(
                        sim,
                        assets,
                        crate::engine::refresh_seek::EntitySeekRequest {
                            owner,
                            sequence_id,
                            element_index,
                            target,
                            action,
                            flags,
                        },
                    ) {
                        // Original resumes translation after seek refresh's
                        // early return and rewrites SEEK to MOVE. The return
                        // only leaves the movement body empty: transition
                        // generation has already run before priority
                        // arbitration and may have populated the order list
                        // during instruction arbitration. In that
                        // case actor instruction handling installs the transition and
                        // leaves the Move IN_PROGRESS while the target passes
                        // its door. Only a genuinely orderless element takes
                        // the selected-element-clear / TERMINATED path
                        // which clears the active element and terminates.
                        let retained_transition = if let Some(element) = self
                            .orders
                            .sequence_manager
                            .get_element_mut(sequence_id, element_index)
                            .filter(|element| {
                                matches!(
                                    element.state,
                                    crate::sequence::SequenceState::Todo
                                        | crate::sequence::SequenceState::Postponed
                                )
                            }) {
                            element.command = Command::Move;
                            !element.orders.is_empty()
                        } else {
                            false
                        };
                        self.world
                            .entities
                            .get_mut(owner)
                            .and_then(Entity::actor_data_mut)
                            .expect("accepted same-sector Seek lost its actor")
                            .continuation
                            .motion_state = crate::sprite::MotionState::InProgress;
                        if retained_transition {
                            self.element_in_progress(
                                sim,
                                assets,
                                &mut Vec::new(),
                                sequence_id,
                                element_index,
                            );
                        } else {
                            self.orders.sequence_manager.set_translating_element(None);
                            self.element_terminated(
                                sim,
                                assets,
                                &mut Vec::new(),
                                sequence_id,
                                element_index,
                            );
                        }
                        return OwnerActionBarrier::Reach;
                    }
                    let target_position = self
                        .world
                        .entities
                        .expect_entity(
                            target,
                            format_args!("entity-target Seek owner {owner:?} target"),
                        )
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
                        // The original game uses a single wait-time field for
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
                        crate::engine::refresh_seek::EntitySeekRequest {
                            owner,
                            sequence_id,
                            element_index,
                            target,
                            action,
                            flags,
                        },
                        seek_distance,
                    ) {
                        return OwnerActionBarrier::Skip;
                    }
                    let Some(resolved) =
                        self.resolve_entity_seek(sim, assets, owner, target, flags, seek_distance)
                    else {
                        self.element_impossible(
                            sim,
                            assets,
                            &mut Vec::new(),
                            sequence_id,
                            element_index,
                        );
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
                        // Point-mode seeking reads these actor-owned
                        // fields, not the transient movement wrapper
                        // stored on the actor.
                        actor.continuation.seek_to_point = true;
                        actor.continuation.seek_layer = goal_layer;
                        actor.continuation.seek_sector =
                            goal_sector.map(crate::actor_state::ActorSeekSector::Position);
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
        if owner_in_building && !is_seek {
            // Original's standard-Move building branch distinguishes an
            // ordinary interior move from the final Move generated by a
            // Seek route. The latter must stay selected on a non-animation
            // RefreshingSeek order; testing only the source command loses
            // this distinction because seek refresh has already rewritten
            // SEEK to MOVE and retained the semantic marker in `flags`.
            if !flags.contains(crate::sequence::MoveFlags::SEEK) || !is_last_of_sequence {
                self.finalize_special_move_position(
                    assets,
                    owner,
                    super::special_motion::SpecialMovePosition::Map(destination),
                    None,
                    None,
                    // RHelementactor.cpp:3726 retains the installed plane
                    // for hidden interior motion; buildings have no floor
                    // projection polygon to query at this destination.
                    None,
                    "building interior move",
                );
                self.element_terminated(sim, assets, &mut Vec::new(), sequence_id, element_index);
                return OwnerActionBarrier::Skip;
            }

            let has_post_seek = self
                .get_entity(owner)
                .and_then(|entity| entity.actor_data())
                .is_some_and(|actor| actor.post_seek_sequence.is_some());
            if has_post_seek && target_element.is_none() {
                self.start_post_seek_sequence(
                    sim,
                    assets,
                    owner,
                    Some((sequence_id, element_index)),
                );
                return OwnerActionBarrier::Skip;
            }

            let order_id = self.orders.allocate_order_id();
            self.orders.sequence_manager.push_order_on(
                sequence_id,
                element_index,
                crate::order::Order::new(
                    crate::order::OrderType::RefreshingSeek,
                    destination.x,
                    destination.y,
                    order_id,
                ),
            );
            self.element_in_progress(sim, assets, &mut Vec::new(), sequence_id, element_index);
            return OwnerActionBarrier::Reach;
        }

        // The original game's seek translation and refresh do not flatten the
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
                crate::engine::refresh_seek::PointSeekRequest {
                    owner,
                    sequence_id,
                    element_index,
                    destination,
                    goal_sector,
                    goal_layer,
                    action,
                    flags,
                    seek_distance: tolerance,
                    recorded_gate_path,
                    route_provenance: point_seek_route_provenance,
                },
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
                // The original game's seek translation changes the command to movement and
                // enables seeking before seek refresh launches the concrete
                // movement. Seeking dispatch and its refresh countdown
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
            self.relaunch_seek_replacement(
                sim,
                assets,
                owner,
                sequence_id,
                element_index,
                replacement,
            );
            return OwnerActionBarrier::Skip;
        }

        self.dispatch_prepared_move_instruction(
            sim,
            assets,
            &mut Vec::new(),
            owner,
            sequence_id,
            element_index,
            destination,
            action,
        )
    }

    /// Launch and dispatch sequence elements after the shared entity and
    /// actor-update work, including synchronous immediate-action cascades and
    /// message/target callbacks at their exact owner-dispatch positions.
    ///
    /// The original game updates the sequence manager after the entity loop
    /// and drains its FIFO there.
    pub(in crate::engine) fn hourglass_phase_sequences_authoritative(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        // Release cross-actor shoulder-climb dependencies from canonical
        // gameplay state rather than the optional UI action callback. The
        // helping transition publishes HelpingToClimb on its DONE edge, but
        // Original retains EnterHelpingClimb through the animation's
        // TERMINATED edge. Re-register only after that element has left the
        // helper's in-progress slot. This lets the climb translate in the
        // termination frame's manager FIFO and a low-headroom
        // LeaveHelpingClimb install immediately, matching retail.
        let ready_helpers: Vec<_> = self
            .world
            .pc_ids
            .iter()
            .copied()
            .filter(|&pc_id| {
                let posture_ready = self.get_entity(pc_id).is_some_and(|entity| {
                    entity.element_data().posture() == crate::element::Posture::HelpingToClimb
                });
                let entry_still_in_progress = self
                    .orders
                    .sequence_manager
                    .current_element_for_actor(pc_id)
                    .and_then(|(sequence_id, element_index)| {
                        self.orders
                            .sequence_manager
                            .get_element(sequence_id, element_index)
                    })
                    .is_some_and(|element| {
                        element.command == Command::EnterHelpingClimb
                            && element.state == crate::sequence::SequenceState::InProgress
                    });
                posture_ready && !entry_still_in_progress
            })
            .collect();
        for helper in ready_helpers {
            self.orders
                .sequence_manager
                .resume_postponed_climbs_for_helper(helper);
        }

        // Pop one live FIFO entry only after its predecessor and every
        // synchronous successor callback have returned.
        while let Some(action) = self.orders.sequence_manager.pop_next_hourglass_action() {
            // Translation selection never outlives the dispatch that
            // installed it; the arms below abandon the action early on many
            // rejection paths.
            self.orders.sequence_manager.set_translating_element(None);
            // Abandoning an action skips the rest of *that action's* work —
            // never the epilogue below. A rejected command can still have
            // terminated its element during translation, and the resulting
            // state-change notification, its readiness continuation, and the successor
            // element it registers all belong to this same manager drain.
            // Falling out of the whole loop instead would strand the
            // successor until the next frame and leave the actor orderless.
            self.dispatch_sequence_phase_action(sim, assets, action);

            self.orders.sequence_manager.set_translating_element(None);
        }
        self.orders.sequence_manager.set_translating_element(None);

        // The redundant-EnterSwordfight retention above is only a bridge
        // across a re-entrant actor-update lazy Wait. If that Wait is
        // published, `publish_selected_order_as_installed` consumes the marker
        // and transfers the running sprite identity. Work first instructed by
        // The sequence-manager tick is already past every actor slot, so an
        // unconsumed marker here means no replacement Wait exists this frame.
        // The original game's interrupted Wait has cleared the actor order in that case.
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

    #[cfg(test)]
    pub(in crate::engine) fn hourglass_phase_sequences(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        assets: &LevelAssets,
    ) {
        let camera = self.feedback.cutscene_camera.display.clone();
        self.hourglass_phase_sequences_authoritative(sim, assets);
        self.feedback.cutscene_camera.display = camera;
        let mut input = InputState::default();
        for event in self
            .feedback
            .pending_side_effects
            .host_events
            .iter()
            .cloned()
        {
            display.apply_host_event(&mut input, event);
        }
    }
}
