use super::*;

impl EngineInner {
    /// Translate one Move/Seek at the exact sequence-processing
    /// FIFO position where its `Go()` action was emitted.
    pub(in crate::engine) fn dispatch_ordered_move_seek_instruct(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        owner: EntityId,
        sequence_id: crate::sequence::SequenceId,
        element_index: usize,
    ) {
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
            self.element_impossible(sim, assets, active_scripts, sequence_id, element_index);
            return;
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
            self.element_terminated(sim, assets, active_scripts, sequence_id, element_index);
            self.start_post_seek_sequence(sim, assets, active_scripts, owner, None);
            return;
        }

        // Original-game owner instruction lets SEEK fall through the
        // move-command arm's source extraction before any seek-specific
        // handling. In particular this must
        // precede seek-refresh/cross-sector lowering: that lowering can consume
        // the wrapper without ever reaching ordinary path dispatch.
        if !self.extract_move_instruction_owner(owner) {
            self.element_impossible(sim, assets, active_scripts, sequence_id, element_index);
            return;
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
            self.element_impossible(sim, assets, active_scripts, sequence_id, element_index);
            return;
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
                        active_scripts,
                        crate::engine::refresh_seek::EntitySeekRequest {
                            owner,
                            sequence_id,
                            element_index,
                            target,
                            action,
                            flags,
                        },
                    ) {
                        if self.current_sequence_element_for_actor(owner)
                            != Some((sequence_id, element_index))
                        {
                            return;
                        }
                        self.orders
                            .sequence_manager
                            .get_element_mut(sequence_id, element_index)
                            .expect("translated Seek element disappeared")
                            .command = Command::Move;
                        return;
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
                        active_scripts,
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
                        return;
                    }
                    let Some(resolved) =
                        self.resolve_entity_seek(sim, assets, owner, target, flags, seek_distance)
                    else {
                        self.element_impossible(
                            sim,
                            assets,
                            active_scripts,
                            sequence_id,
                            element_index,
                        );
                        return;
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
                self.element_terminated(sim, assets, active_scripts, sequence_id, element_index);
                return;
            }

            let has_post_seek = self
                .get_entity(owner)
                .and_then(|entity| entity.actor_data())
                .is_some_and(|actor| actor.post_seek_sequence.is_some());
            if has_post_seek && target_element.is_none() {
                self.start_post_seek_sequence(
                    sim,
                    assets,
                    active_scripts,
                    owner,
                    Some((sequence_id, element_index)),
                );
                return;
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

            return;
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
                active_scripts,
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
            return;
        }

        if is_seek {
            let Some(mut replacement_data) = self
                .orders
                .sequence_manager
                .get_element(sequence_id, element_index)
                .map(|element| element.data.clone())
            else {
                return;
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
                active_scripts,
                owner,
                sequence_id,
                element_index,
                replacement,
            );
            return;
        }

        self.dispatch_prepared_move_instruction(
            sim,
            assets,
            active_scripts,
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
                    .world
                    .entities
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
            // Abandoning an action skips the rest of *that action's* work —
            // never the epilogue below. A rejected command can still have
            // terminated its element during translation, and the resulting
            // state-change notification, its readiness continuation, and the successor
            // element it registers all belong to this same manager drain.
            // Falling out of the whole loop instead would strand the
            // successor until the next frame and leave the actor orderless.
            self.dispatch_sequence_phase_action(sim, assets, action);
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
