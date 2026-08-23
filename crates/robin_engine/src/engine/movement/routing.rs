use super::*;

impl EngineInner {
    /// Enqueue an AI-initiated Move intent for this actor.
    ///
    /// Per-actor dedup: only one pending request per actor exists in
    /// the queue at any time — a later call for the same `entity_id`
    /// overwrites the earlier entry.  The actual Move element launch
    /// happens in `drain_pending_move_requests` at a deterministic
    /// point in the hourglass.
    ///
    /// This queue absorbs high-frequency AI re-fires (patrol macro-
    /// GoTo, pursuit re-pathfind) that would otherwise each spawn a
    /// fresh `Command::Move` element and `InterruptCurrent` the
    /// previous one at the same Normal priority, preventing the actor
    /// from ever completing a startup transition or making waypoint
    /// progress.
    ///
    /// Once drained, the Move element is launched via the sequence
    /// pipeline (`launch_element_for_owner` → `arbitrate_instruct` →
    /// `InstructOwner` dispatch), giving the move:
    /// * Priority arbitration — the element can be postponed behind
    ///   an in-flight `ENTER_ATTENTIVE_MODE`
    ///   (`PostponeEverythingButInjuries`) so the alerted-pose
    ///   transition finishes before the move starts.
    /// * System #16 — failed-path-impossible actually reaches the
    ///   owner via the Move's `element_impossible` condolation.
    /// * `post_process_path` on path arrival (see `tick.rs` Move
    ///   dispatch) inserts the startup-transition animation via the
    ///   normal pipeline.
    ///
    /// Run the AI `GoTo` pre-flight gates for an AI movement intent.
    /// Returns `true` if the intent should proceed to launch, `false`
    /// if it was rejected (in which case `couldnt_reachpoint` has been
    /// set on the AI controller and the caller should drop
    /// the intent).
    ///
    /// `intent.find_accessible` runs
    /// `FastFindGrid::find_authorized_position` against the actor's
    /// `MoveBox + (target_x, target_y)` and rewrites the intent target
    /// to the snapped centre on success.
    ///
    /// `intent.ask_obstacle` runs
    /// `FastFindGrid::is_straight_movement_authorized` from the
    /// actor's current position to the destination.  Only meaningful
    /// for straight moves (gated on `compute_direction == false`).
    pub(in crate::engine) fn preflight_ai_goto(
        &mut self,
        entity_id: EntityId,
        intent: &mut crate::order::AiOrderIntent,
    ) -> bool {
        let debug_decision_path = crate::ai_enemy::decision_path_debug_enabled()
            && crate::ai_enemy::decision_path_debug_matches_raw(
                self.control.frame_counter,
                entity_id.index(),
            );
        if debug_decision_path {
            eprintln!(
                "AIDECISION frame={} owner={} stage=preflight_enter order={:?} target=({:08x},{:08x}) move_flags={} tolerance_bits={:08x} no_halt={} reverse={} find_accessible={} ask_obstacle={} compute_direction={}",
                self.control.frame_counter,
                entity_id.index(),
                intent.order_type,
                intent.target_x.to_bits(),
                intent.target_y.to_bits(),
                intent.move_flags,
                intent.tolerance.to_bits(),
                intent.no_halt,
                intent.reverse,
                intent.find_accessible,
                intent.ask_obstacle,
                intent.compute_direction,
            );
        }
        // Upper-bound check.  `AiController::go_to` already rejects
        // `target_x <= 0 || target_y <= 0` before pushing the intent;
        // the engine drain owns the `>= GetLevelSize()` half because
        // `level_size` lives on the shared cutscene camera, not on
        // `AiContext`. Direct RHMOVE_MAP elements are the exception: the
        // merry-man exit path intentionally targets a reinforcement door's
        // PointOut beyond the playable map and Actor::Instruct admits MAP
        // movement without the ordinary reachable-position gate.
        let move_flags =
            crate::sequence::MoveFlags::from_bits_truncate(u32::from(intent.move_flags));
        if !move_flags.contains(crate::sequence::MoveFlags::MAP) {
            let level_w = self.feedback.cutscene_camera.level_size.x;
            let level_h = self.feedback.cutscene_camera.level_size.y;
            if level_w > 0.0 && intent.target_x >= level_w
                || level_h > 0.0 && intent.target_y >= level_h
            {
                self.set_ai_couldnt_reachpoint(entity_id);
                if debug_decision_path {
                    eprintln!(
                        "AIDECISION frame={} owner={} stage=preflight_result result=reject_upper_bound level=({:08x},{:08x}) target=({:08x},{:08x})",
                        self.control.frame_counter,
                        entity_id.index(),
                        level_w.to_bits(),
                        level_h.to_bits(),
                        intent.target_x.to_bits(),
                        intent.target_y.to_bits(),
                    );
                }
                return false;
            }
        }

        if !intent.find_accessible && !intent.ask_obstacle {
            if debug_decision_path {
                eprintln!(
                    "AIDECISION frame={} owner={} stage=preflight_result result=accepted_no_checks",
                    self.control.frame_counter,
                    entity_id.index(),
                );
            }
            return true;
        }

        let (move_box, layer, position) = {
            let entity = self
                .get_entity(entity_id)
                .unwrap_or_else(|| panic!("AI movement preflight owner {entity_id:?} disappeared"));
            let pi = entity.position_iface();
            let pm = pi.map_position();
            (*pi.get_move_box(), entity.element_data().layer(), pm)
        };

        // Snap destination to the nearest authorised position when
        // `find_accessible` is set.  Translate the move box to the
        // requested destination and ask the grid.  On success rewrite
        // the intent target to the box centre.
        if intent.find_accessible {
            let dest = MapPoint::new(intent.target_x, intent.target_y);
            let mut bbox = if move_box.is_somewhere() {
                MapBBox::from_corners(
                    MapPoint::new(move_box.x_min() + dest.x, move_box.y_min() + dest.y),
                    MapPoint::new(move_box.x_max() + dest.x, move_box.y_max() + dest.y),
                )
            } else {
                MapBBox::new()
            };
            if !self
                .world
                .fast_grid
                .find_authorized_position(&mut bbox, layer)
            {
                self.set_ai_couldnt_reachpoint(entity_id);
                if debug_decision_path {
                    eprintln!(
                        "AIDECISION frame={} owner={} stage=preflight_result result=reject_find_accessible target=({:08x},{:08x}) layer={} move_box={:?}",
                        self.control.frame_counter,
                        entity_id.index(),
                        intent.target_x.to_bits(),
                        intent.target_y.to_bits(),
                        layer,
                        move_box,
                    );
                }
                return false;
            }
            let centre = bbox.center();
            intent.target_x = centre.x;
            intent.target_y = centre.y;
        }

        // Pre-flight straight movement.  Only meaningful for straight
        // moves (gated on `compute_direction == false`); when
        // `ask_obstacle` is set without straight-mode the check is
        // silently skipped rather than asserting.
        if intent.ask_obstacle && !intent.compute_direction {
            let dest = MapPoint::new(intent.target_x, intent.target_y);
            if !self
                .world
                .fast_grid
                .is_straight_movement_authorized(position, dest, layer, &move_box)
            {
                self.set_ai_couldnt_reachpoint(entity_id);
                if debug_decision_path {
                    eprintln!(
                        "AIDECISION frame={} owner={} stage=preflight_result result=reject_straight from=({:08x},{:08x}) target=({:08x},{:08x}) layer={} move_box={:?}",
                        self.control.frame_counter,
                        entity_id.index(),
                        position.x.to_bits(),
                        position.y.to_bits(),
                        intent.target_x.to_bits(),
                        intent.target_y.to_bits(),
                        layer,
                        move_box,
                    );
                }
                return false;
            }
        }

        if debug_decision_path {
            eprintln!(
                "AIDECISION frame={} owner={} stage=preflight_result result=accepted target=({:08x},{:08x})",
                self.control.frame_counter,
                entity_id.index(),
                intent.target_x.to_bits(),
                intent.target_y.to_bits(),
            );
        }
        true
    }

    /// Set `AiController::couldnt_reachpoint = true` on the entity, used
    /// by the GoTo pre-flight gates to surface a same-frame failure to
    /// the AI's stuck-retry / fallback logic.
    #[track_caller]
    pub(in crate::engine) fn set_ai_couldnt_reachpoint(&mut self, entity_id: EntityId) {
        let debug_decision_path = crate::ai_enemy::decision_path_debug_enabled()
            && crate::ai_enemy::decision_path_debug_matches_raw(
                self.control.frame_counter,
                entity_id.index(),
            );
        if debug_decision_path {
            eprintln!(
                "AIDECISION frame={} owner={} stage=set_couldnt_reachpoint caller={}",
                self.control.frame_counter,
                entity_id.index(),
                std::panic::Location::caller(),
            );
        }
        let entity = self
            .world
            .entities
            .get_mut(entity_id)
            .unwrap_or_else(|| panic!("AI movement failure owner {entity_id:?} disappeared"));
        let ai = entity.ai_controller_mut().unwrap_or_else(|| {
            panic!("AI movement failure owner {entity_id:?} has no AI controller")
        });
        ai.couldnt_reachpoint = true;
    }

    /// Mark the engine-owned result of an AI order as available to the
    /// deferred EndThink surface.  Original builds/authorizes the order
    /// inline; Rust must not interpret an earlier nested drain with no result
    /// as a successful authorization.
    pub(in crate::engine) fn resolve_ai_engine_completion_verdict(&mut self, entity_id: EntityId) {
        let entity = self.world.entities.get_mut(entity_id).unwrap_or_else(|| {
            panic!("AI order owner {entity_id:?} disappeared before its engine verdict")
        });
        let ai = entity.ai_controller_mut().unwrap_or_else(|| {
            panic!("AI order owner {entity_id:?} lost its controller before its engine verdict")
        });
        ai.resolve_engine_completion_verdict();
    }

    pub(in crate::engine) fn launch_ai_move(
        &mut self,
        entity_id: EntityId,
        intent: &crate::order::AiOrderIntent,
    ) {
        // One AI think can legitimately emit two distinct `GoTo` intents for
        // the same actor (`RHArtificialMalignity::ReconsiderSwordfightObservation`
        // runs its defensive step-back `GoTo` and then deliberately falls
        // through — `original-code/RHartificialmalignity.cpp:15502-15519` has
        // no `return` — into the attack block's `GoNear`). Each
        // `RHArtificialIntelligence::GoTo` builds its own `RHSequence` and
        // hands it to `RHSequenceManager::LaunchSequence`
        // (`original-code/RHartificialintelligence.cpp:2453,2594`), so both
        // land on `mlistSequenceElementsToGo` and both are instructed, in
        // launch order, by the sequence-manager hourglass
        // (`original-code/RHsequencemanager.cpp:938-952`). Nothing in `GoTo`
        // discards the earlier one: its pre-launch halt is the dead
        // `uwFlags & GOTO_NOHALT == 0` gate, which C++ precedence evaluates as
        // `uwFlags & 0` (`original-code/RHartificialintelligence.cpp:2423`).
        //
        // So keep every intent, in FIFO order. An explicit
        // `Halt`/`StopAll` still invalidates the queued ones, because
        // `halt_actor` drops this actor's pending intents at exactly that
        // boundary — that is Original's
        // `StopNotYetLaunchedSequenceElements`.
        let mut intent = intent.clone();
        if intent.defer_instruction && intent.not_before_frame.is_none() {
            intent.not_before_frame = Some(self.control.frame_counter.saturating_add(1));
        }
        if intent.source_position.is_none() {
            let (raw_source, raw_sector, raw_layer, door_handle, door_direction) = {
                let entity = self.get_entity(entity_id).unwrap_or_else(|| {
                    panic!("AI GoTo source actor {entity_id:?} disappeared before enqueue")
                });
                let element = entity.element_data();
                let (door_handle, door_direction) = current_door_for_route_source(entity);
                (
                    element.position_map(),
                    // Original snapshots `Position(mpMe)`, including its
                    // exact `RHSector*`, when GoTo builds the sequence. A
                    // legacy-loaded actor may retain only the public sector
                    // number on ElementData, while the live AI Position can
                    // recover the unique arena object from point + layer.
                    // Keep that exact source identity here: mixing a
                    // number-only source with an exact goal cannot enter the
                    // indexed gate graph and spuriously reports
                    // EVENT_COULDNT_REACHPOINT.
                    super::ai::ai_view_position_sector(self, element),
                    element.layer(),
                    door_handle,
                    door_direction,
                )
            };
            // Original chooses the simple same-topology Move from the raw
            // actor topology. Only the cross-topology branch calls
            // AppendMoveToSequence, which adapts an actor already committed
            // to a selected door onto its far side. Snapshot that exact
            // call-time decision rather than the actor's potentially
            // different door state at the later drain.
            let goal_layer = intent.target_layer.unwrap_or(raw_layer);
            let goal_sector = intent.target_sector.or(raw_sector);
            let raw_sector_index = raw_sector.and_then(|sector| sector.arena_index());
            let goal_sector_index = intent
                .target_sector_index
                .or_else(|| goal_sector.and_then(|sector| sector.arena_index()));
            // Original compares the two `RHSector*` values directly. Both
            // identities now come from authored/copy provenance; coordinates
            // are never queried to guess an overlapping polygon.
            let source_target_sector_identity_differs = match (raw_sector_index, goal_sector_index)
            {
                (Some(source), Some(goal)) => source != goal,
                _ => false,
            };
            intent.source_target_sector_identity_differs |= source_target_sector_identity_differs;
            let crosses_raw_topology = goal_layer != raw_layer
                || goal_sector != raw_sector
                || source_target_sector_identity_differs;
            let adapted_source = crosses_raw_topology
                .then(|| {
                    self.scripts.mission.as_ref().and_then(|_| {
                        adapt_source_to_current_door(
                            &self.script_domains.interactables.doors,
                            door_handle,
                            door_direction,
                        )
                    })
                })
                .flatten();
            let adapted_sector_index = adapted_source.and_then(|_| {
                self.script_domains
                    .interactables
                    .doors
                    .get(usize::try_from(door_handle.0).expect("door handle exceeds runtime usize"))
                    .and_then(|door| {
                        if door_direction {
                            door.sector_in_index
                        } else {
                            door.sector_out_index
                        }
                    })
            });
            let (source, sector, layer) = adapted_source
                .map(|(point, sector, layer)| {
                    (
                        point,
                        crate::position_interface::SectorHandle::new(sector).map(|handle| {
                            adapted_sector_index
                                .map_or(handle, |index| handle.with_arena_index(index))
                        }),
                        layer,
                    )
                })
                .unwrap_or((raw_source, raw_sector, raw_layer));
            intent.source_position = Some(source);
            intent.source_sector = sector;
            intent.source_sector_index = sector.and_then(|sector| sector.arena_index());
            intent.target_sector_index = goal_sector_index;
            intent.source_layer = Some(layer);
        }
        self.orders.pending_move_requests.push((entity_id, intent));
    }

    /// Drain the pending-move-request queue and launch a Move
    /// sequence element for each.  Runs once per tick from the
    /// hourglass pipeline.  Determinism: requests drain in FIFO order
    /// of enqueue (a `Vec` with `retain`+`push` on launch preserves
    /// this).
    pub(in crate::engine) fn drain_pending_move_requests(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
    ) {
        let requests = std::mem::take(&mut self.orders.pending_move_requests);
        let mut deferred = Vec::new();
        for (entity_id, intent) in requests {
            if intent
                .not_before_frame
                .is_some_and(|frame| frame > self.control.frame_counter)
            {
                deferred.push((entity_id, intent));
                continue;
            }
            let launched = self.do_launch_ai_move(sim, entity_id, &intent);
            if launched.is_some() && intent.halt_after_launch_for_path_waiter {
                self.halt_actor(entity_id);
            }
            self.resolve_ai_engine_completion_verdict(entity_id);
        }
        // Work authored while draining may already have appended newer
        // intents. The retained older FIFO prefix stays ahead of those.
        if !deferred.is_empty() {
            deferred.append(&mut self.orders.pending_move_requests);
            self.orders.pending_move_requests = deferred;
        }
    }

    /// Launch only one owner's pending AI Move at a synchronous owner
    /// boundary. Requests belonging to other creation slots retain their FIFO
    /// positions for the normal tick drain.
    pub(in crate::engine) fn drain_pending_move_requests_for_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        owner: EntityId,
    ) -> Vec<crate::sequence::SequenceId> {
        let requests = std::mem::take(&mut self.orders.pending_move_requests);
        let mut owner_requests = Vec::new();
        let mut remaining = Vec::with_capacity(requests.len());
        for request @ (entity_id, _) in requests {
            // A continuation that resumes an Original call stack after its
            // authored manager boundary may construct a movement now but must
            // leave its instruction to the next ordinary drain. The global
            // `drain_pending_move_requests` intentionally ignores this marker
            // when that boundary arrives.
            if entity_id == owner
                && !request
                    .1
                    .not_before_frame
                    .is_some_and(|frame| frame > self.control.frame_counter)
            {
                owner_requests.push(request);
            } else {
                remaining.push(request);
            }
        }
        self.orders.pending_move_requests = remaining;
        let mut launched = Vec::new();
        for (_, intent) in owner_requests {
            if let Some(sequence_id) = self.do_launch_ai_move(sim, owner, &intent) {
                if intent.halt_after_launch_for_path_waiter {
                    // RHArtificialIntelligence::GoTo checks
                    // `mpMe->IsComputingPath()` after LaunchSequence. In this
                    // recursive path-waiter case it remains true, so Halt
                    // synchronously interrupts the newly registered Move
                    // before SequenceManager::Hourglass can instruct it.
                    self.halt_actor(owner);
                } else {
                    launched.push(sequence_id);
                }
            }
            self.resolve_ai_engine_completion_verdict(owner);
        }
        launched
    }

    /// Actually build and launch the Move sequence element for an AI
    /// intent.  Split out from `launch_ai_move` so the enqueue side
    /// can be cheap (push into a Vec) and the heavier work (resolve
    /// entity state, build element, run arbitration + path) only
    /// happens once per actor per tick at drain time.
    pub(in crate::engine) fn do_launch_ai_move(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        entity_id: EntityId,
        intent: &crate::order::AiOrderIntent,
    ) -> Option<crate::sequence::SequenceId> {
        let dest = crate::coordinates::MapPoint {
            x: intent.target_x,
            y: intent.target_y,
        };
        let (raw_source, raw_source_layer, raw_source_sector, door_handle, door_direction) = {
            let Some(entity) = self.get_entity(entity_id) else {
                tracing::warn!("do_launch_ai_move: entity {:?} not found", entity_id);
                return None;
            };
            let ed = entity.element_data();
            let (door_handle, door_direction) = current_door_for_route_source(entity);
            (
                ed.position_map(),
                ed.layer(),
                ed.sector(),
                door_handle,
                door_direction,
            )
        };
        // RHSequence::AppendMoveToSequence adapts a source that is currently
        // crossing a gate to the committed far side before comparing sectors
        // or searching the gate graph.
        let (source, source_layer, source_sector) = if let Some(source) = intent.source_position {
            (
                source,
                intent.source_layer.unwrap_or_else(|| {
                    panic!("AI GoTo for {entity_id:?} captured a source position without a layer")
                }),
                intent.source_sector,
            )
        } else {
            // Backward-compatible fallback for old serialized intents that
            // predate the enqueue-time topology snapshot.
            self.scripts
                .mission
                .as_ref()
                .and_then(|_| {
                    adapt_source_to_current_door(
                        &self.script_domains.interactables.doors,
                        door_handle,
                        door_direction,
                    )
                })
                .map(|(point, sector, layer)| {
                    (
                        point,
                        layer,
                        crate::position_interface::SectorHandle::new(sector),
                    )
                })
                .unwrap_or((raw_source, raw_source_layer, raw_source_sector))
        };
        let goal_layer = intent.target_layer.unwrap_or(source_layer);
        let goal_sector = intent.target_sector.or(source_sector);
        let source_sector_index = intent
            .source_sector_index
            .or_else(|| source_sector.and_then(|sector| sector.arena_index()));
        let goal_sector_index = intent
            .target_sector_index
            .or_else(|| goal_sector.and_then(|sector| sector.arena_index()));
        let move_flags =
            crate::sequence::MoveFlags::from_bits_truncate(u32::from(intent.move_flags));

        let action = intent.order_type;
        // A layer transition requires gate routing even when the numeric
        // sector handle happens to remain the same.
        let exact_identity_differs = match (source_sector_index, goal_sector_index) {
            (Some(source), Some(goal)) => source != goal,
            _ => intent.source_target_sector_identity_differs,
        };
        let crosses_topology =
            goal_layer != source_layer || goal_sector != source_sector || exact_identity_differs;
        if crosses_topology {
            let Some(source_sector) = source_sector else {
                tracing::warn!(?entity_id, "cross-sector AI GoTo has no source sector");
                self.set_ai_couldnt_reachpoint(entity_id);
                return None;
            };
            let Some(goal_sector) = goal_sector else {
                tracing::warn!(?entity_id, "cross-sector AI GoTo has no destination sector");
                self.set_ai_couldnt_reachpoint(entity_id);
                return None;
            };
            let auth = self
                .get_entity(entity_id)
                .map(|entity| entity.actor_auth_info());
            let level = self.world.fast_grid.level.clone();
            // AppendMoveToSequence treats a door sector as a door-identity
            // goal. A sector-only FindPathGates search cannot represent that
            // terminal condition because a door sector is not an ordinary
            // motion area.
            let door_goal = ai_move_goal_door(self, goal_sector, goal_sector_index);
            let gate_path = self.scripts.mission.as_ref().and_then(|_| {
                if let Some(door_index) = door_goal {
                    crate::gate::find_path_into_door_with_sector_index(
                        &self.script_domains.interactables.doors,
                        (source.x, source.y),
                        u16::from(source_sector),
                        source_sector_index,
                        door_index,
                        auth.as_ref(),
                        move_flags.contains(crate::sequence::MoveFlags::MAP),
                        &|sector| self.building_sector_is_authorized(sector),
                        &|sector| {
                            level
                                .sectors
                                .iter()
                                .find(|candidate| candidate.sector_number == sector)
                                .and_then(|candidate| candidate.lift_type)
                        },
                    )
                } else {
                    crate::gate::find_path_gates_with_sector_indices(
                        &self.script_domains.interactables.doors,
                        (source.x, source.y),
                        u16::from(source_sector),
                        source_sector_index,
                        (dest.x, dest.y),
                        u16::from(goal_sector),
                        goal_sector_index,
                        auth.as_ref(),
                        move_flags.contains(crate::sequence::MoveFlags::MAP),
                        &|sector| self.building_sector_is_authorized(sector),
                        &|sector| {
                            level
                                .sectors
                                .iter()
                                .find(|candidate| candidate.sector_number == sector)
                                .and_then(|candidate| candidate.lift_type)
                        },
                    )
                }
            });
            let Some(gate_path) = gate_path else {
                tracing::warn!(
                    ?entity_id,
                    source_sector = u16::from(source_sector),
                    source_layer,
                    goal_sector = u16::from(goal_sector),
                    goal_layer,
                    "cross-sector AI GoTo has no gate route"
                );
                self.set_ai_couldnt_reachpoint(entity_id);
                return None;
            };
            // Original FindPathGates can only authorize a cross-sector
            // AppendMoveToSequence with at least one gate. If our compact
            // sector handles compare equal while retained pointer provenance
            // says they differ, its empty same-number result is failure, not
            // a direct Move. In particular, do not replace the actor's
            // existing sequence in this case.
            if gate_path.is_empty() && exact_identity_differs {
                tracing::warn!(
                    ?entity_id,
                    source_sector = u16::from(source_sector),
                    goal_sector = u16::from(goal_sector),
                    identity_differs = exact_identity_differs,
                    "cross-sector AI GoTo resolved to an empty gate route"
                );
                self.set_ai_couldnt_reachpoint(entity_id);
                return None;
            }
            let mut prefix = Vec::new();
            if intent.quit_swordfight_before_move {
                prefix.push(crate::sequence::SequenceElement::new(
                    1,
                    crate::element::Command::QuitSwordfight,
                    Some(entity_id),
                ));
            }
            if intent.enter_swordfight_before_move {
                prefix.push(Self::goto_enter_swordfight_element(
                    prefix.len() as u16 + 1,
                    entity_id,
                ));
            }
            if intent.stop_menace_before_move {
                prefix.push(crate::sequence::SequenceElement::new(
                    prefix.len() as u16 + 1,
                    crate::element::Command::StopMenace,
                    Some(entity_id),
                ));
            }
            if intent.lower_shield_before_move {
                prefix.push(crate::sequence::SequenceElement::new(
                    prefix.len() as u16 + 1,
                    crate::element::Command::LowerShield,
                    Some(entity_id),
                ));
            }
            let tail = self.ai_special_action_tail(entity_id, intent);
            let goal = door_goal.map_or(
                GoalShape::Point {
                    point: dest,
                    tolerance: intent.tolerance,
                },
                |door_index| GoalShape::Door {
                    door_index,
                    // These fields only serve the move-after-last-door
                    // variant. AppendMoveToSequence sets that false for a
                    // door-sector goal because the gate path is inclusive.
                    far_side_point: dest,
                    far_side_layer: goal_layer,
                    far_side_is_building: false,
                },
            );
            tracing::debug!(
                target: "parity_rng_owner",
                frame = self.control.frame_counter,
                owner = ?entity_id,
                caller = "do_launch_ai_move",
                source_x = source.x,
                source_y = source.y,
                source_layer,
                source_sector = u16::from(source_sector),
                goal_x = dest.x,
                goal_y = dest.y,
                goal_layer,
                goal_sector = u16::from(goal_sector),
                action = ?action,
                move_flags = move_flags.bits(),
                tolerance = intent.tolerance,
                speed_factor = intent.speed_factor,
                quit_swordfight = intent.quit_swordfight_before_move,
                stop_menace = intent.stop_menace_before_move,
                door_goal = ?door_goal,
                gate_path = ?gate_path,
                "about to build cross-sector AI GoTo sequence"
            );
            return self.build_gate_movement_sequence(
                sim,
                entity_id,
                Some(source_sector),
                gate_path,
                goal,
                goal_layer,
                action,
                door_goal.is_none(),
                intent.speed_factor,
                move_flags,
                prefix,
                tail,
                false,
                false,
            );
        }

        let move_level = 1
            + u16::from(intent.quit_swordfight_before_move)
            + u16::from(intent.enter_swordfight_before_move)
            + u16::from(intent.stop_menace_before_move)
            + u16::from(intent.lower_shield_before_move);
        let mut elem = crate::sequence::SequenceElement::new_movement(
            move_level,
            crate::element::Command::Move,
            Some(entity_id),
            action,
        );
        elem.retained_movement_goal = intent.retained_movement_goal;
        if let crate::sequence::SequenceElementData::Movement {
            destination,
            layer: elem_layer,
            sector: elem_sector,
            flags,
            tolerance,
            element,
            speed_factor,
            ..
        } = &mut elem.data
        {
            *destination = dest;
            *elem_layer = goal_layer;
            *elem_sector = goal_sector;
            *flags = move_flags;
            *tolerance = intent.tolerance;
            *element = intent.antagonist;
            *speed_factor = intent.speed_factor;
        }

        // Promotion creates the Original sequence-manager work item but does
        // not run the owner's Instruct yet. Normal frame and patrol drains
        // leave it queued until SequenceManager::Hourglass; script-native and
        // condolence call sites that require re-entrant dispatch explicitly
        // take this exact deferred action immediately after this returns.
        let mut sequence = crate::sequence::Sequence::new();
        if intent.quit_swordfight_before_move {
            sequence.append_element(crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::QuitSwordfight,
                Some(entity_id),
            ));
        }
        if intent.enter_swordfight_before_move {
            let level = sequence
                .last()
                .map_or(1, |element| element.command_level.saturating_add(1));
            sequence.append_element(Self::goto_enter_swordfight_element(level, entity_id));
        }
        if intent.stop_menace_before_move {
            sequence.append_element(crate::sequence::SequenceElement::new(
                sequence
                    .last()
                    .map_or(1, |element| element.command_level.saturating_add(1)),
                crate::element::Command::StopMenace,
                Some(entity_id),
            ));
        }
        if intent.lower_shield_before_move {
            sequence.append_element(crate::sequence::SequenceElement::new(
                sequence
                    .last()
                    .map_or(1, |element| element.command_level.saturating_add(1)),
                crate::element::Command::LowerShield,
                Some(entity_id),
            ));
        }
        sequence.append_element(elem);
        for mut tail in self.ai_special_action_tail(entity_id, intent) {
            tail.command_level = sequence
                .last()
                .map_or(1, |element| element.command_level.saturating_add(1));
            sequence.append_element(tail);
        }
        let sequence_id = self.launch_sequence(sequence);

        tracing::trace!(
            entity = ?entity_id,
            dest_x = dest.x,
            dest_y = dest.y,
            ?action,
            move_flags = intent.move_flags,
            "AI movement launched via sequence element"
        );
        Some(sequence_id)
    }

    /// Test the gate portion of `AppendMoveToSequence` before registering an
    /// AI move that will immediately be cancelled by GoTo's legacy
    /// `IsComputingPath` tail.
    ///
    /// The Original constructs a cross-topology sequence synchronously. If
    /// gate construction fails, `GoTo` publishes `mbCouldntReachpoint` before
    /// it notices and halts the outgoing `MOVE_WAITING` element
    /// (`RHartificialintelligence.cpp:2538-2580,2614-2620`). Rust normally
    /// defers construction through `pending_move_requests`; that tail halt
    /// would otherwise erase the raw intent before the failure can be seen.
    pub(in crate::engine) fn ai_move_gate_route_is_authorized(
        &self,
        entity_id: EntityId,
        intent: &crate::order::AiOrderIntent,
    ) -> bool {
        let entity = self.get_entity(entity_id).unwrap_or_else(|| {
            panic!("AI gate-route authorization owner {entity_id:?} disappeared")
        });
        let ed = entity.element_data();
        let (door_handle, door_direction) = current_door_for_route_source(entity);
        let raw_source = ed.position_map();
        let raw_layer = ed.layer();
        let raw_sector = ed.sector();
        let raw_sector_index = raw_sector.and_then(|sector| sector.arena_index());
        let (source, source_layer, source_sector, source_sector_index) = if let Some(source) =
            intent.source_position
        {
            (
                source,
                intent.source_layer.unwrap_or_else(|| {
                    panic!("AI GoTo for {entity_id:?} captured a source position without a layer")
                }),
                intent.source_sector,
                intent
                    .source_sector_index
                    .or_else(|| intent.source_sector.and_then(|sector| sector.arena_index())),
            )
        } else {
            // AppendMoveToSequence only adapts a live door source after the
            // raw RHSector* comparison selected the cross-topology branch.
            // Keep this preflight on the same source identity as the later
            // launch; otherwise it can authorize a different gate graph.
            let raw_goal_layer = intent.target_layer.unwrap_or(raw_layer);
            let raw_goal_sector = intent.target_sector.or(raw_sector);
            let raw_goal_sector_index = intent
                .target_sector_index
                .or_else(|| raw_goal_sector.and_then(|sector| sector.arena_index()));
            let raw_identity_differs = match (raw_sector_index, raw_goal_sector_index) {
                (Some(source), Some(goal)) => source != goal,
                _ => intent.source_target_sector_identity_differs,
            };
            let crosses_raw_topology = raw_goal_layer != raw_layer
                || raw_goal_sector != raw_sector
                || raw_identity_differs;
            crosses_raw_topology
                .then(|| {
                    self.scripts.mission.as_ref().and_then(|_| {
                        adapt_source_to_current_door_with_identity(
                            &self.script_domains.interactables.doors,
                            door_handle,
                            door_direction,
                        )
                    })
                })
                .flatten()
                .map(|(point, sector, layer)| (point, layer, Some(sector), sector.arena_index()))
                .unwrap_or((raw_source, raw_layer, raw_sector, raw_sector_index))
        };
        let goal_layer = intent.target_layer.unwrap_or(source_layer);
        let goal_sector = intent.target_sector.or(source_sector);
        let goal_sector_index = intent
            .target_sector_index
            .or_else(|| goal_sector.and_then(|sector| sector.arena_index()));
        let exact_identity_differs = match (source_sector_index, goal_sector_index) {
            (Some(source), Some(goal)) => source != goal,
            _ => intent.source_target_sector_identity_differs,
        };
        if goal_layer == source_layer && goal_sector == source_sector && !exact_identity_differs {
            return true;
        }
        let (Some(source_sector), Some(goal_sector), Some(_)) =
            (source_sector, goal_sector, self.scripts.mission.as_ref())
        else {
            return false;
        };
        let auth = entity.actor_auth_info();
        let level = &self.world.fast_grid.level;
        let move_flags =
            crate::sequence::MoveFlags::from_bits_truncate(u32::from(intent.move_flags));
        let door_goal = ai_move_goal_door(self, goal_sector, goal_sector_index);
        let goal = (intent.target_x, intent.target_y);
        find_ai_move_gate_path(
            &self.script_domains.interactables.doors,
            source,
            source_sector,
            source_sector_index,
            MapPoint::new(goal.0, goal.1),
            goal_sector,
            goal_sector_index,
            door_goal,
            Some(&auth),
            move_flags.contains(crate::sequence::MoveFlags::MAP),
            &|sector| self.building_sector_is_authorized(sector),
            &|sector| {
                level
                    .sectors
                    .iter()
                    .find(|candidate| candidate.sector_number == sector)
                    .and_then(|candidate| candidate.lift_type)
            },
        )
        .is_some()
    }

    /// The raise-sword element `RHArtificialIntelligence::GoTo` inserts into
    /// the movement's own sequence for `GOTO_SWORD` when the actor is not
    /// already in a sword action state
    /// (`original-code/RHartificialintelligence.cpp:2486-2495`): a generic
    /// `ENTER_SWORDFIGHT` with a null opponent, no jump-line destination and
    /// `SWORDFIGHT_PREPARED` cleared.
    pub(in crate::engine) fn goto_enter_swordfight_element(
        command_level: u16,
        entity_id: EntityId,
    ) -> crate::sequence::SequenceElement {
        let mut element = crate::sequence::SequenceElement::new_generic(
            command_level,
            crate::element::Command::EnterSwordfight,
            Some(entity_id),
        );
        element.set_property(
            crate::sequence::Field::Opponent,
            crate::sequence::FieldValue::Integer(0),
        );
        element.set_property(
            crate::sequence::Field::JumplineDestination,
            crate::sequence::FieldValue::Integer(0),
        );
        element.set_property(
            crate::sequence::Field::SwordfightPrepared,
            crate::sequence::FieldValue::Bool(false),
        );
        element
    }

    /// Build the exact tail authored by
    /// `RHArtificialIntelligence::GoTo(..., GOTO_SPECIAL_ACTION)`.
    ///
    /// These are part of the movement sequence, not follow-up AI work. That
    /// distinction keeps the Move from being the last real action: its
    /// condolence must not emit EventReachPoint until the final SitDown /
    /// EnterLeisure element terminates.
    pub(in crate::engine) fn ai_special_action_tail(
        &self,
        entity_id: EntityId,
        intent: &crate::order::AiOrderIntent,
    ) -> Vec<crate::sequence::SequenceElement> {
        if !intent.append_special_action_tail {
            return Vec::new();
        }
        let ai = self
            .world
            .entities
            .get(entity_id)
            .and_then(|entity| entity.ai_controller())
            .unwrap_or_else(|| {
                panic!("GOTO_SPECIAL_ACTION movement owner {entity_id:?} lost its AI controller")
            });
        let direction = ai.initial_view_direction;

        let mut turn = crate::sequence::SequenceElement::new_generic(
            0,
            crate::element::Command::Turn,
            Some(entity_id),
        );
        turn.set_property(
            crate::sequence::Field::Direction,
            crate::sequence::FieldValue::Integer(u32::from(direction)),
        );
        let posture_command = if ai.special_action {
            crate::element::Command::EnterLeisure
        } else {
            crate::element::Command::SitDown
        };
        vec![
            turn,
            crate::sequence::SequenceElement::new(0, posture_command, Some(entity_id)),
        ]
    }

    /// Execute the selected movement arm for one live actor owner.
    ///
    /// Mutable inputs are sampled at this owner's legacy slot. Movement and
    /// its Execute-arm callbacks, completion, and condolence continuation are
    /// applied synchronously before this function returns; no owner result
    /// vectors escape to a later global dispatch pass.
    pub(in crate::engine) fn turn_globally_frozen_climb_owner(
        &mut self,
        owner: EntityId,
        selected: MovementOwnerSelection,
    ) {
        let order_action = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .and_then(|element| element.current_order())
            .filter(|order| order.order_id == selected.order_id)
            .map(|order| order.order_type)
            .expect("globally frozen movement owner lost its selected order");
        let (action, door_index, current_sector, execute_order_initialising, position) = self
            .world
            .entities
            .get(owner)
            .and_then(|entity| {
                let actor = entity.actor_data()?;
                Some((
                    actor
                        .active_door_pass
                        .as_ref()
                        .map_or(order_action, |pass| pass.current_action),
                    actor.active_door_pass.as_ref().map(|pass| pass.door_index),
                    entity.element_data().sector(),
                    actor.execute_order_initialising,
                    entity.element_data().position_map(),
                ))
            })
            .unwrap_or_else(|| panic!("globally frozen movement owner {owner:?} is not an actor"));
        let Some(expected_lift_type) = climb_lift_type(action) else {
            return;
        };

        let selected_order = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .and_then(|element| element.current_order())
            .filter(|order| order.order_id == selected.order_id)
            .expect("globally frozen climb owner lost its selected order");
        let lift_direction = if let Some(door_index) = door_index {
            let door = self
                .script_domains
                .interactables
                .doors
                .get(usize::from(door_index))
                .unwrap_or_else(|| {
                    panic!(
                        "globally frozen climb owner {owner:?} references missing door {door_index}"
                    )
                });
            if door.door_type == crate::gate::DoorType::BuildingTrap
                && action == OrderType::ClimbingLadderDown
                && selected_order.reverse
                && position == MapPoint::new(selected_order.target_x, selected_order.target_y)
            {
                // TODO(parity): Original casts the BuildingTrap's inside
                // RHSectorBuilding to RHSectorLift in the decorative ladder
                // Execute arm. Three shipped traces consistently expose zero
                // from that invalid release-build read. Preserve that narrow
                // compatibility result without applying it to real ladders or
                // to a decorative row which still has distance to travel.
                None
            } else if !door_type_uses_lift_climb_direction(door.door_type) {
                // Building-trap passes deliberately contain a decorative
                // ClimbingLadderDown order even though their inside sector is
                // a building. It skips only the lift-facing setup; the climb
                // Execute arm still calls Turn below while sprites are frozen.
                None
            } else {
                Some(door.sector_in)
            }
        } else {
            let sector = current_sector.unwrap_or_else(|| {
                panic!("globally frozen climb owner {owner:?} has no lift sector")
            });
            Some(crate::sector::SectorNumber::new(i16::from(sector)))
        }
        .map(|sector_number| {
            let lift = self
                .grid_sector_by_number(sector_number)
                .unwrap_or_else(|| {
                    panic!(
                        "globally frozen climb owner {owner:?} references missing lift sector {sector_number}"
                    )
                });
            assert_eq!(
                lift.lift_type,
                Some(expected_lift_type),
                "globally frozen climb owner {owner:?} action {action:?} requires {expected_lift_type:?}, found {:?}",
                lift.lift_type
            );
            if action == OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel {
                (lift.lift_direction + 8) & 15
            } else {
                lift.lift_direction
            }
        });
        let lift_direction = if execute_order_initialising
            && door_index.is_some_and(|door_index| {
                self.script_domains
                    .interactables
                    .doors
                    .get(usize::from(door_index))
                    .is_some_and(|door| {
                        door.door_type == crate::gate::DoorType::BuildingTrap
                            && action == OrderType::ClimbingLadderDown
                            && selected_order.reverse
                            && position
                                == MapPoint::new(selected_order.target_x, selected_order.target_y)
                    })
            }) {
            Some(0)
        } else {
            lift_direction
        };
        let turns = if is_fast_climb_action(action) { 2 } else { 1 };
        let entity = self
            .world
            .entities
            .get_mut(owner)
            .expect("globally frozen climb owner disappeared after canonical lookup");
        if execute_order_initialising && let Some(direction) = lift_direction {
            entity.element_data_mut().set_direction_goal(direction);
        }
        for _ in 0..turns {
            entity.element_data_mut().sprite.position_iface.turn();
        }
    }

    pub(in crate::engine) fn execute_globally_frozen_pre_motion_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        selected: MovementOwnerSelection,
    ) -> OrderType {
        let (order_action, flags, target, destination) = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .and_then(|element| {
                let (flags, target, destination) = match &element.data {
                    crate::sequence::SequenceElementData::Movement {
                        flags,
                        element,
                        destination,
                        ..
                    } => (*flags, *element, *destination),
                    _ => return None,
                };
                element
                    .current_order()
                    .filter(|order| order.order_id == selected.order_id)
                    .map(|order| (order.order_type, flags, target, destination))
            })
            .expect("globally frozen movement owner lost its selected order");
        if climb_lift_type(order_action).is_some() {
            self.turn_globally_frozen_climb_owner(owner, selected);
            return order_action;
        }

        if flags.contains(crate::sequence::MoveFlags::SEEK) {
            let (owner_position, owner_sector, seek_target, seek_distance, has_post_seek) = self
                .world
                .entities
                .get(owner)
                .and_then(|entity| {
                    let actor = entity.actor_data()?;
                    Some((
                        entity.element_data().position_map(),
                        entity.element_data().sector(),
                        actor.seek_target,
                        actor.seek_distance,
                        actor.post_seek_sequence.is_some(),
                    ))
                })
                .unwrap_or_else(|| panic!("globally frozen seek owner {owner:?} is not an actor"));

            // `mbSeekToPoint` takes the unconditional Turn/PerformMotion arm.
            // An entity seek whose target was cleared instead returns
            // TERMINATED before touching either the wait counter or facing.
            let Some(seek_target) = seek_target else {
                if target.is_none() {
                    self.world
                        .entities
                        .get_mut(owner)
                        .expect("globally frozen point-seek owner disappeared")
                        .position_iface_mut()
                        .turn();
                }
                return order_action;
            };
            assert_eq!(
                target,
                Some(seek_target),
                "globally frozen seek owner {owner:?} has inconsistent actor/element targets"
            );

            let target_entity = self.world.entities.get(seek_target).unwrap_or_else(|| {
                panic!(
                    "globally frozen seek owner {owner:?} references missing target {seek_target:?}"
                )
            });
            let target_position = target_entity.element_data().position_map();
            let target_sector = target_entity.element_data().sector();
            let use_point = flags.contains(crate::sequence::MoveFlags::USE_POINT);
            let point = if use_point {
                target_entity
                    .cxx_current_point_map()
                    .filter(|point| *point != target_position)
                    .unwrap_or(target_position)
            } else {
                target_position
            };
            let delta = if flags.contains(crate::sequence::MoveFlags::SEEK_SHIELD) {
                assert!(
                    self.world
                        .entities
                        .get(owner)
                        .is_some_and(crate::element::Entity::is_pc),
                    "SEEK_SHIELD owner {owner:?} is not a PC"
                );
                destination - owner_position
            } else {
                point - owner_position
            };
            let dy = if flags.contains(crate::sequence::MoveFlags::DIRECTIONAL_TOLERANCE) {
                delta.y * 1.743_446_8
            } else {
                delta.y
            };
            let in_tolerance = owner_sector == target_sector
                && delta.x * delta.x + dy * dy < seek_distance * seek_distance * 1.1025;

            if in_tolerance {
                if has_post_seek {
                    if debug_post_seek_handoff_enabled() {
                        eprintln!(
                            "[POST_SEEK frame={} owner={owner:?} stage=frozen_in_tolerance selected={:?} actors_frozen={}]",
                            self.control.frame_counter,
                            (selected.seq_id, selected.elem_idx, selected.order_id),
                            self.actors_frozen(),
                        );
                    }
                    let launched = self.start_post_seek_sequence(
                        sim,
                        assets,
                        owner,
                        Some((selected.seq_id, selected.elem_idx)),
                    );
                    if debug_post_seek_handoff_enabled() {
                        eprintln!(
                            "[POST_SEEK frame={} owner={owner:?} stage=frozen_launch_done launched={launched} current={:?}]",
                            self.control.frame_counter,
                            self.orders
                                .sequence_manager
                                .current_element_for_actor(owner),
                        );
                    }
                    return order_action;
                }
                // PerformAction(FROZEN) returns before sprite motion, then
                // PerformSeek ages the shared unsigned wait scalar.
                let actor = self
                    .world
                    .entities
                    .get_mut(owner)
                    .and_then(|entity| entity.actor_data_mut())
                    .expect("globally frozen seek owner lost actor data");
                actor.seek_refresh_wait = age_seek_refresh_wait(actor.seek_refresh_wait);
                actor.wait_time = actor.seek_refresh_wait;
                return order_action;
            }

            // The moved-target refresh test runs in
            // `tick_refresh_seek_for_owner` immediately before this owner
            // Execute. If it did not replace the seek, PerformSeek ages the
            // counter and turns before frozen PerformMotion returns.
            let entity = self
                .world
                .entities
                .get_mut(owner)
                .expect("globally frozen seek owner disappeared before Turn");
            let actor = entity
                .actor_data_mut()
                .expect("globally frozen seek owner lost actor data before Turn");
            actor.seek_refresh_wait = age_seek_refresh_wait(actor.seek_refresh_wait);
            actor.wait_time = actor.seek_refresh_wait;
            entity.position_iface_mut().turn();
            return order_action;
        }

        if !order_turns_before_motion(order_action) {
            return order_action;
        }
        self.world
            .entities
            .get_mut(owner)
            .unwrap_or_else(|| panic!("globally frozen movement owner {owner:?} disappeared"))
            .position_iface_mut()
            .turn();
        order_action
    }
}

#[cfg(test)]
mod exact_ai_goto_source_tests {
    use super::*;
    use crate::coordinates::{MapBBox, MapPoint};
    use crate::element::{ActorSoldier, AiBrain, ElementData, ElementKind, Entity, Posture};
    use crate::fast_find_grid::{GridSector, SectorIndex};
    use crate::gate::{Door, GatePathStep};
    use crate::position_interface::SectorHandle;
    use crate::sector::{SectorNumber, SectorType};

    fn square_sector(number: i16, layer: u16, min: MapPoint, max: MapPoint) -> GridSector {
        GridSector {
            points: vec![
                min,
                MapPoint::new(max.x, min.y),
                max,
                MapPoint::new(min.x, max.y),
            ],
            bounding_box: MapBBox::from_coords(min.x, min.y, max.x, max.y),
            sector_type: SectorType::MOTION | SectorType::AREA,
            layer,
            sector_number: SectorNumber::new(number),
            door_index: None,
            lift_type: None,
            lift_direction: 0,
            force_crouched: false,
            building_index: None,
            low_exit_point: None,
            high_exit_point: None,
            lowest_door_index: None,
            jump_line_indices: Vec::new(),
            gate_indices: Vec::new(),
            underlying_sector: None,
        }
    }

    #[test]
    fn arrow_reaction_goto_recovers_exact_source_before_indexed_gate_search() {
        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(32, 32);
        engine.world.fast_grid_mut().allocate_layers(5);
        let wrong_source = engine.world.fast_grid_mut().add_sector(
            square_sector(
                104,
                4,
                MapPoint::new(100.0, 100.0),
                MapPoint::new(200.0, 200.0),
            ),
            4,
        );
        let source = engine.world.fast_grid_mut().add_sector(
            square_sector(
                104,
                4,
                MapPoint::new(600.0, 1350.0),
                MapPoint::new(700.0, 1450.0),
            ),
            4,
        );
        let middle = engine.world.fast_grid_mut().add_sector(
            square_sector(
                99,
                3,
                MapPoint::new(250.0, 1150.0),
                MapPoint::new(350.0, 1450.0),
            ),
            3,
        );
        let goal = engine.world.fast_grid_mut().add_sector(
            square_sector(
                89,
                2,
                MapPoint::new(500.0, 1200.0),
                MapPoint::new(600.0, 1350.0),
            ),
            2,
        );
        assert_ne!(wrong_source, source);

        let mut soldier = ActorSoldier {
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                posture: Posture::Upright,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: Default::default(),
        };
        soldier.npc.ai_brain = AiBrain::Enemy(Box::default());
        soldier
            .element
            .set_position_map(MapPoint::new(630.0, 1408.0));
        soldier.element.set_layer(4);
        // Legacy adoption retained the public sector but not RHSector*.
        soldier.element.set_sector(SectorHandle::new(104));
        let owner = engine.add_entity(Entity::Soldier(soldier));

        let mut intent = crate::order::AiOrderIntent::new(
            crate::order::OrderType::RunningUpright,
            531.231,
            1268.2043,
        );
        intent.target_layer = Some(2);
        intent.target_sector = SectorHandle::new(89)
            .map(|sector| sector.with_arena_index(SectorIndex::new(goal).unwrap()));
        intent.target_sector_index = SectorIndex::new(goal);
        engine.launch_ai_move(owner, &intent);

        let [(_, captured)] = engine.orders.pending_move_requests.as_slice() else {
            panic!("arrow reaction GoTo must enqueue exactly one movement")
        };
        assert_eq!(captured.source_position, Some(MapPoint::new(630.0, 1408.0)));
        assert_eq!(captured.source_sector_index, SectorIndex::new(source));

        let mut doors = vec![
            Door {
                active: true,
                sector_out: SectorNumber::new(104),
                sector_in: SectorNumber::new(99),
                sector_out_index: SectorIndex::new(source),
                sector_in_index: SectorIndex::new(middle),
                point_out: MapPoint::new(273.0, 1195.0),
                point_in: MapPoint::new(280.0, 1221.0),
                ..Door::default()
            },
            Door {
                active: true,
                sector_out: SectorNumber::new(89),
                sector_in: SectorNumber::new(99),
                sector_out_index: SectorIndex::new(goal),
                sector_in_index: SectorIndex::new(middle),
                point_out: MapPoint::new(322.0, 1426.0),
                point_in: MapPoint::new(314.0, 1392.0),
                ..Door::default()
            },
        ];
        crate::gate::build_gate_links(&mut doors);
        let route = crate::gate::find_path_gates_with_sector_indices(
            &doors,
            (
                captured.source_position.unwrap().x,
                captured.source_position.unwrap().y,
            ),
            captured.source_sector.unwrap().get(),
            captured.source_sector_index,
            (captured.target_x, captured.target_y),
            captured.target_sector.unwrap().get(),
            captured.target_sector_index,
            None,
            false,
            &|_| true,
            &|_| None,
        )
        .expect("exact arrow-reaction source must enter the two-door gate route");
        assert_eq!(
            route,
            [
                GatePathStep {
                    door_index: crate::gate::DoorIndex(0),
                    direct: true,
                },
                GatePathStep {
                    door_index: crate::gate::DoorIndex(1),
                    direct: false,
                },
            ]
        );
    }
}
