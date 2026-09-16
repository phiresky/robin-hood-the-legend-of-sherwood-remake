use super::*;

impl EngineInner {
    pub(in crate::engine) fn launch_live_ai_turn(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        direction: i16,
        fast: bool,
    ) {
        self.halt_actor(sim, assets, owner);
        self.launch_turn_sequence_deferred_no_transitions(
            owner,
            if fast {
                crate::element::Command::TurnFast
            } else {
                crate::element::Command::Turn
            },
            Some(direction),
            0.0,
            0.0,
        );
    }
    /// Emit one `AIDECISION` line; `stage` holds the stage-specific payload.
    #[inline(never)]
    fn trace_ai_decision(frame: u32, owner: EntityId, stage: std::fmt::Arguments<'_>) {
        eprintln!("AIDECISION frame={} owner={} {stage}", frame, owner.index());
    }

    #[inline(never)]
    fn trace_post_seek_frozen_in_tolerance(&self, owner: EntityId, selected: impl std::fmt::Debug) {
        eprintln!(
            "[POST_SEEK frame={} owner={owner:?} stage=frozen_in_tolerance selected={:?} actors_frozen={}]",
            self.control.frame_counter,
            selected,
            self.actors_frozen(),
        );
    }

    #[inline(never)]
    fn trace_post_seek_frozen_launch_done(
        &self,
        owner: EntityId,
        launched: impl std::fmt::Display,
    ) {
        eprintln!(
            "[POST_SEEK frame={} owner={owner:?} stage=frozen_launch_done launched={launched} current={:?}]",
            self.control.frame_counter,
            self.world.entities.current_element_for_actor(owner),
        );
    }

    /// Complete movement eligibility checks for callers that still queue requests.
    /// Live engine movement has already completed these checks before admission.

    /// Resolve the requested destination before proximity completion is considered.
    pub(in crate::engine) fn resolve_ai_accessible_destination(
        &mut self,
        owner: EntityId,
        destination: &mut crate::ai::Position,
    ) -> bool {
        let move_box = *self
            .expect_entity(owner, "accessible movement owner")
            .position_iface()
            .get_move_box();
        let mut bbox = if move_box.is_somewhere() {
            MapBBox::from_corners(
                MapPoint::new(
                    move_box.x_min() + destination.x,
                    move_box.y_min() + destination.y,
                ),
                MapPoint::new(
                    move_box.x_max() + destination.x,
                    move_box.y_max() + destination.y,
                ),
            )
        } else {
            MapBBox::new()
        };
        if !self
            .world
            .fast_grid
            .find_authorized_position(&mut bbox, destination.level)
        {
            self.set_ai_couldnt_reachpoint(owner);
            return false;
        }
        let center = bbox.center();
        destination.x = center.x;
        destination.y = center.y;
        true
    }

    /// Check the admitted destination before stopping the outgoing movement.
    pub(in crate::engine) fn authorize_ai_destination(
        &mut self,
        owner: EntityId,
        destination: crate::ai::Position,
        check_bounds: bool,
        ask_obstacle: bool,
    ) -> bool {
        let size = self.feedback.cutscene_camera.level_size;
        if check_bounds
            && (size.x > 0.0 && destination.x >= size.x || size.y > 0.0 && destination.y >= size.y)
        {
            self.set_ai_couldnt_reachpoint(owner);
            return false;
        }
        if ask_obstacle {
            let position = self.live_ai_position(owner);
            let entity = self.expect_entity(owner, "straight movement owner");
            if !self.world.fast_grid.is_straight_movement_authorized(
                MapPoint::new(position.x, position.y),
                MapPoint::new(destination.x, destination.y),
                entity.element_data().layer(),
                entity.position_iface().get_move_box(),
            ) {
                self.set_ai_couldnt_reachpoint(owner);
                return false;
            }
        }
        true
    }
    /// Set `AiController::couldnt_reachpoint = true` on the entity, used
    /// by the preliminary movement checks to surface a same-frame failure to
    /// the AI's stuck-retry / fallback logic.
    #[track_caller]
    pub(in crate::engine) fn set_ai_couldnt_reachpoint(&mut self, entity_id: EntityId) {
        let debug_decision_path = crate::ai_enemy::decision_path_debug_enabled()
            && crate::ai_enemy::decision_path_debug_matches_raw(
                self.control.frame_counter,
                entity_id.index(),
            );
        if debug_decision_path {
            Self::trace_ai_decision(
                self.control.frame_counter,
                entity_id,
                format_args!(
                    "stage=set_couldnt_reachpoint caller={}",
                    std::panic::Location::caller()
                ),
            );
        }
        let ai = self
            .world
            .entities
            .expect_ai_controller_mut(entity_id, format_args!("AI movement failure owner"));
        ai.couldnt_reachpoint = true;
    }

    /// Construct and register movement at the caller's current statement.
    pub(in crate::engine) fn launch_ai_move(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        destination: crate::ai::Position,
        goto: crate::ai::GotoFlags,
        speed: f32,
    ) -> Option<crate::sequence::SequenceId> {
        use crate::ai::GotoFlags;
        if !self.tactical_allows_combat_movement(entity_id) {
            return None;
        }
        let actor = self.expect_entity(entity_id, "live movement owner");
        let action_state = actor.actor_data().expect("movement actor").action_state;
        let move_tolerance = if goto.contains(GotoFlags::NEAR) {
            actor
                .ai_controller()
                .expect("movement AI")
                .stop_before_end_of_path_distance as f32
        } else {
            0.0
        };
        let quit_swordfight = !goto.contains(GotoFlags::SWORD) && action_state.is_sword();
        let enter_swordfight = goto.contains(GotoFlags::SWORD) && !action_state.is_sword();
        let stop_menace = !goto.contains(GotoFlags::SWORD)
            && !action_state.is_sword()
            && action_state == crate::element::ActionState::Menacing;
        let lower_shield = action_state.is_shield();
        let action = if goto.contains(GotoFlags::STRAFE) {
            OrderType::WalkingUpright
        } else if goto.contains(GotoFlags::RIDER_CHARGE_HIT) {
            OrderType::RiderCharging
        } else if goto.contains(GotoFlags::RUN) {
            OrderType::RunningUpright
        } else {
            OrderType::WalkingUpright
        };
        let mut move_flags = crate::sequence::MoveFlags::empty();
        if goto.contains(GotoFlags::BACK) {
            move_flags |= crate::sequence::MoveFlags::REVERSED;
        }
        if goto.contains(GotoFlags::RIDER_CHARGE) {
            move_flags |= crate::sequence::MoveFlags::RIDER_CHARGE;
        }
        if goto.contains(GotoFlags::SWORD) {
            move_flags |= crate::sequence::MoveFlags::FORCE_SWORD_MOVEMENT;
        }
        if goto.contains(GotoFlags::STRAIGHT) {
            move_flags |= crate::sequence::MoveFlags::STRAIGHT;
        }
        if goto.contains(GotoFlags::DONT_STOP) {
            move_flags |= crate::sequence::MoveFlags::NO_TRANSITIONS;
        }
        let selected = self
            .world
            .entities
            .current_element_for_actor(entity_id)
            .and_then(|(seq, index)| self.orders.sequence_manager.get_element(seq, index));
        let was_computing_path =
            selected.is_some_and(|element| element.command == crate::element::Command::MoveWaiting);
        let (raw_source, raw_sector, raw_layer, door_source) = {
            let entity = self.expect_entity(entity_id, "AI movement source actor before enqueue");
            let element = entity.element_data();
            let door_source = current_door_for_route_source(entity);
            (
                element.position_map(),
                // The original game snapshots the actor's AI position, including its
                // exact sector reference when AI movement builds the sequence. A
                // legacy-loaded actor may retain only the public sector
                // number on ElementData, while the live AI Position can
                // recover the unique arena object from point + layer.
                // Keep that exact source identity here: mixing a
                // number-only source with an exact goal cannot enter the
                // indexed gate graph and spuriously reports
                // EVENT_COULDNT_REACHPOINT.
                super::ai::ai_view_position_sector(self, element),
                element.layer(),
                door_source,
            )
        };
        // Choose the simple Move from raw actor topology before adapting
        // the source of a cross-sector route to its selected door.
        let goal_layer = destination.level;
        let goal_sector = destination.sector.or(raw_sector);
        let raw_sector_index = raw_sector.and_then(|sector| sector.arena_index());
        let goal_sector_index = goal_sector.and_then(|sector| sector.arena_index());
        // The original game compares the two sector references directly. Both
        // identities now come from authored/copy provenance; coordinates
        // are never queried to guess an overlapping polygon.
        let source_target_sector_identity_differs = match (raw_sector_index, goal_sector_index) {
            (Some(source), Some(goal)) => source != goal,
            _ => false,
        };
        let crosses_raw_topology = goal_layer != raw_layer
            || goal_sector != raw_sector
            || source_target_sector_identity_differs;
        let adapted_source = crosses_raw_topology
            .then(|| {
                self.scripts.mission.as_ref().and_then(|_| {
                    door_source.and_then(|(door_handle, door_direction)| {
                        adapt_source_to_current_door(
                            &self.script_domains.interactables.doors,
                            door_handle,
                            door_direction,
                        )
                    })
                })
            })
            .flatten();
        let adapted_sector_index = adapted_source.and_then(|_| {
            let (door_handle, door_direction) = door_source?;
            self.script_domains
                .interactables
                .doors
                .get(usize::from(door_handle))
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
                        adapted_sector_index.map_or(handle, |index| handle.with_arena_index(index))
                    }),
                    layer,
                )
            })
            .unwrap_or((raw_source, raw_sector, raw_layer));

        let dest = destination.map_point();
        let source_layer = layer;
        let source_sector = sector;
        let source_sector_index = sector.and_then(|sector| sector.arena_index());
        let crosses_topology = crosses_raw_topology;
        let launched = (|| {
            if crosses_topology {
                let Some(source_sector) = source_sector else {
                    tracing::warn!(?entity_id, "cross-sector AI movement has no source sector");
                    self.set_ai_couldnt_reachpoint(entity_id);
                    return None;
                };
                let Some(goal_sector) = goal_sector else {
                    tracing::warn!(
                        ?entity_id,
                        "cross-sector AI movement has no destination sector"
                    );
                    self.set_ai_couldnt_reachpoint(entity_id);
                    return None;
                };
                let auth = self
                    .get_entity(entity_id)
                    .map(|entity| entity.actor_auth_info());
                let level = self.world.fast_grid.level.clone();
                // Movement construction treats a door sector as a door-identity
                // goal. A sector-only gate-path search cannot represent that
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
                    // RHSequence::AppendMoveToSequence returns false when FindPathGates
                    // rejects a candidate. AI escape/defense searches deliberately
                    // try several destinations, so this is an ordinary negative
                    // routing result, not corrupt or missing topology. Keep the
                    // failure verdict and diagnostic for the caller retry logic.
                    tracing::debug!(
                        ?entity_id,
                        source_sector = u16::from(source_sector),
                        source_layer,
                        goal_sector = u16::from(goal_sector),
                        goal_layer,
                        "cross-sector AI movement has no gate route"
                    );
                    self.set_ai_couldnt_reachpoint(entity_id);
                    return None;
                };
                let route_identity_differs = match (source_sector_index, goal_sector_index) {
                    (Some(source), Some(goal)) => source != goal,
                    _ => source_sector != goal_sector,
                };
                // The original game can only authorize a cross-sector
                // movement construction with at least one gate. If our compact
                // sector handles compare equal while retained pointer provenance
                // says they differ, its empty same-number result is failure, not
                // a direct Move. In particular, do not replace the actor's
                // existing sequence in this case.
                if gate_path.is_empty() && route_identity_differs {
                    tracing::warn!(
                        ?entity_id,
                        source_sector = u16::from(source_sector),
                        goal_sector = u16::from(goal_sector),
                        identity_differs = route_identity_differs,
                        "cross-sector AI movement resolved to an empty gate route"
                    );
                    self.set_ai_couldnt_reachpoint(entity_id);
                    return None;
                }
                let mut prefix = Vec::new();
                if quit_swordfight {
                    prefix.push(crate::sequence::SequenceElement::new(
                        1,
                        crate::element::Command::QuitSwordfight,
                        Some(entity_id),
                    ));
                }
                if enter_swordfight {
                    prefix.push(Self::goto_enter_swordfight_element(
                        prefix.len() as u16 + 1,
                        entity_id,
                    ));
                }
                if stop_menace {
                    prefix.push(crate::sequence::SequenceElement::new(
                        prefix.len() as u16 + 1,
                        crate::element::Command::StopMenace,
                        Some(entity_id),
                    ));
                }
                if lower_shield {
                    prefix.push(crate::sequence::SequenceElement::new(
                        prefix.len() as u16 + 1,
                        crate::element::Command::LowerShield,
                        Some(entity_id),
                    ));
                }
                let tail = self.ai_special_action_tail(
                    entity_id,
                    goto.contains(crate::ai::GotoFlags::SPECIAL_ACTION),
                );
                let goal = door_goal.map_or(
                    GoalShape::Point {
                        point: dest,
                        tolerance: move_tolerance,
                    },
                    |door_index| GoalShape::Door {
                        door_index,
                        // These fields only serve the move-after-last-door
                        // variant. Movement construction sets that false for a
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
                    tolerance = move_tolerance,
                    speed_factor = speed,
                    quit_swordfight = quit_swordfight,
                    stop_menace = stop_menace,
                    door_goal = ?door_goal,
                    gate_path = ?gate_path,
                    "about to build cross-sector AI movement sequence"
                );
                // Movement requests choose route construction from the actor's
                // raw sector, but construction then adapts an in-flight
                // door source before deciding whether to emit its leading
                // AssertPosition. The adapted endpoint can already
                // be the goal sector.  In that case Original emits only the
                // trailing Move; retaining Some(source_sector) here would invent
                // an AssertPosition and postpone the real Move one manager-FIFO
                // position behind later actors.
                let route_assert_sector = route_identity_differs.then_some(source_sector);
                return self.launch_gate_movement_sequence(
                    sim,
                    assets,
                    &mut Vec::new(),
                    crate::engine::movement::GateRouteRequest {
                        entity_id: entity_id,
                        source_sector: route_assert_sector,
                        gate_path: gate_path,
                        goal: goal,
                        goal_layer: goal_layer,
                        base_action: action,
                        move_after_last_door: door_goal.is_none(),
                        speed_factor: speed,
                        initial_flags: move_flags,
                        prefix_elements: prefix,
                        tail_elements: tail,
                        append_arrival_speech: false,
                        append_recovery: false,
                    },
                );
            }

            let move_level = 1
                + u16::from(quit_swordfight)
                + u16::from(enter_swordfight)
                + u16::from(stop_menace)
                + u16::from(lower_shield);
            let mut elem = crate::sequence::SequenceElement::new_movement(
                move_level,
                crate::element::Command::Move,
                Some(entity_id),
                action,
            );
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
                *tolerance = move_tolerance;
                *element = None;
                *speed_factor = speed;
            }

            // Register normal movement for instruction by the sequence manager.
            let mut sequence = crate::sequence::Sequence::new();
            if quit_swordfight {
                sequence.append_element(crate::sequence::SequenceElement::new(
                    1,
                    crate::element::Command::QuitSwordfight,
                    Some(entity_id),
                ));
            }
            if enter_swordfight {
                let level = sequence
                    .last()
                    .map_or(1, |element| element.command_level.saturating_add(1));
                sequence.append_element(Self::goto_enter_swordfight_element(level, entity_id));
            }
            if stop_menace {
                sequence.append_element(crate::sequence::SequenceElement::new(
                    sequence
                        .last()
                        .map_or(1, |element| element.command_level.saturating_add(1)),
                    crate::element::Command::StopMenace,
                    Some(entity_id),
                ));
            }
            if lower_shield {
                sequence.append_element(crate::sequence::SequenceElement::new(
                    sequence
                        .last()
                        .map_or(1, |element| element.command_level.saturating_add(1)),
                    crate::element::Command::LowerShield,
                    Some(entity_id),
                ));
            }
            sequence.append_element(elem);
            for mut tail in self.ai_special_action_tail(
                entity_id,
                goto.contains(crate::ai::GotoFlags::SPECIAL_ACTION),
            ) {
                tail.command_level = sequence
                    .last()
                    .map_or(1, |element| element.command_level.saturating_add(1));
                sequence.append_element(tail);
            }
            let sequence_id = self.launch_sequence(sim, assets, sequence);

            tracing::trace!(
                entity = ?entity_id,
                dest_x = dest.x,
                dest_y = dest.y,
                ?action,
                move_flags = move_flags.bits(),
                "AI movement launched via sequence element"
            );
            Some(sequence_id)
        })();
        if launched.is_some() && was_computing_path {
            self.halt_actor(sim, assets, entity_id);
            None
        } else {
            launched
        }
    }
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
    /// AI movement with the special-action flag.
    ///
    /// These are part of the movement sequence, not follow-up AI work. That
    /// distinction keeps the Move from being the last real action: its
    /// condolence must not emit EventReachPoint until the final SitDown /
    /// EnterLeisure element terminates.
    pub(in crate::engine) fn ai_special_action_tail(
        &self,
        entity_id: EntityId,
        append: bool,
    ) -> Vec<crate::sequence::SequenceElement> {
        if !append {
            return Vec::new();
        }
        let ai = self.world.entities.expect_ai_controller(
            entity_id,
            format_args!("GOTO_SPECIAL_ACTION movement owner"),
        );
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
                    order_action,
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
                // TODO(parity): the original game narrows the building trap's inside
                // building sector to lift sector in the decorative ladder
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

            // Point-seeking takes the unconditional turn/motion branch.
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

            let target_entity = self.world.entities.expect_entity(
                seek_target,
                format_args!("globally frozen seek owner {owner:?} target"),
            );
            let target_position = target_entity.element_data().position_map();
            let target_sector = target_entity.element_data().sector();
            let use_point = flags.contains(crate::sequence::MoveFlags::USE_POINT);
            let point = if use_point {
                target_entity
                    .current_gameplay_point_map()
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
                        self.trace_post_seek_frozen_in_tolerance(
                            owner,
                            (selected.seq_id, selected.elem_idx, selected.order_id),
                        );
                    }
                    let launched = self.start_post_seek_sequence(
                        sim,
                        assets,
                        &mut Vec::new(),
                        owner,
                        Some((selected.seq_id, selected.elem_idx)),
                    );
                    if debug_post_seek_handoff_enabled() {
                        self.trace_post_seek_frozen_launch_done(owner, launched);
                    }
                    return order_action;
                }
                // Frozen action processing leaves the sprite untouched, but
                // seeking still renews the order before aging its wait scalar.
                let (element, next_order_id) = self
                    .orders
                    .element_with_order_ids_mut(selected.seq_id, selected.elem_idx)
                    .expect("globally frozen seek lost its selected element");
                let order = element
                    .orders
                    .front_mut()
                    .expect("globally frozen seek lost its selected order");
                assert_eq!(order.order_id, selected.order_id);
                order.reseed_id(crate::order::alloc_order_id(next_order_id));
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
            // execution. If it did not replace the seek, seeking ages the
            // counter and turns before frozen motion processing returns.
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
        self.expect_entity_mut(owner, "globally frozen movement owner")
            .position_iface_mut()
            .turn();
        order_action
    }
}

#[cfg(test)]
mod exact_ai_goto_source_tests {
    use super::*;
    use crate::coordinates::MapPoint;
    use crate::element::{ActorSoldier, AiBrain, ElementData, ElementKind, Entity, Posture};
    use crate::engine::test_support::square_sector;
    use crate::fast_find_grid::SectorIndex;
    use crate::gate::{Door, GatePathStep};
    use crate::position_interface::{DoorHandle, SectorHandle};
    use crate::sector::SectorNumber;

    fn minimal_mission() -> crate::engine::MissionScript {
        use crate::scb::{ClassEntry, Function};
        use crate::vm::{Opcode, Quad};

        crate::engine::MissionScript::from_scb(crate::scb::ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![ClassEntry {
                source_file: "queued_goto_door_test.scs".into(),
                class_name: crate::engine::test_support::asm::STARTUP_CLASS.into(),
                size_of_member_variables: 0,
                member_variables: Vec::new(),
                functions: vec![Function {
                    name: "Initialize".into(),
                    address: 0,
                    num_parameters: 0,
                    size_of_return_value: 0,
                    size_of_parameters: 0,
                    size_of_volatile: 0,
                    size_of_temporary: 0,
                }],
                quads: vec![
                    Quad {
                        operation: Opcode::BeginFunction as u8,
                        operands: [0; 8],
                    },
                    Quad {
                        operation: Opcode::Return as u8,
                        operands: [0; 8],
                    },
                ],
            }],
        })
        .expect("minimal mission")
    }

    #[test]
    fn live_move_preserves_reverse_and_strafe_action_flags() {
        use crate::ai::GotoFlags;
        for (flags, expected_action, reversed) in [
            (GotoFlags::BACK, OrderType::WalkingUpright, true),
            (
                GotoFlags::STRAFE | GotoFlags::RUN | GotoFlags::RIDER_CHARGE_HIT,
                OrderType::WalkingUpright,
                false,
            ),
            (GotoFlags::RUN, OrderType::RunningUpright, false),
        ] {
            let mut engine = EngineInner::new();
            let owner =
                engine.add_test_entity(crate::engine::test_support::actors::make_test_ai_soldier(
                    crate::element::Camp::Lacklandists,
                ));
            let destination = crate::ai::Position {
                x: 100.0,
                y: 100.0,
                sector: None,
                level: 0,
            };
            let sequence = engine
                .launch_ai_move(
                    &crate::sim_rng::test_context(),
                    &LevelAssets::new(),
                    owner,
                    destination,
                    flags,
                    1.0,
                )
                .expect("admitted movement registers a sequence");
            let element = engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap();
            let crate::sequence::SequenceElementData::Movement { action, flags, .. } =
                &element.data
            else {
                panic!("movement must be first element");
            };
            assert_eq!(*action, expected_action);
            assert_eq!(
                flags.contains(crate::sequence::MoveFlags::REVERSED),
                reversed
            );
            assert_eq!(element.state, crate::sequence::SequenceState::Todo);
            assert!(
                engine
                    .orders
                    .sequence_manager
                    .current_order_for_actor(&engine.world.entities, owner)
                    .is_none()
            );
        }
    }

    #[test]
    fn door_transit_construction_keeps_raw_branch_and_adapted_route_identities_distinct() {
        let mut engine = EngineInner::new();
        engine.scripts.mission = Some(minimal_mission());

        engine.world.fast_grid_mut().size_map(16, 16);
        engine.world.fast_grid_mut().allocate_layers(3);
        let raw_index = SectorIndex::new(engine.world.fast_grid_mut().add_sector(
            square_sector(
                40,
                2,
                MapPoint::new(80.0, 180.0),
                MapPoint::new(130.0, 220.0),
            ),
            2,
        ))
        .unwrap();
        let endpoint_index = SectorIndex::new(engine.world.fast_grid_mut().add_sector(
            square_sector(
                41,
                2,
                MapPoint::new(130.0, 180.0),
                MapPoint::new(200.0, 220.0),
            ),
            2,
        ))
        .unwrap();
        let raw_sector = SectorHandle::new(40).unwrap().with_arena_index(raw_index);
        let endpoint_sector = SectorHandle::new(41)
            .unwrap()
            .with_arena_index(endpoint_index);
        engine.script_domains.interactables.doors.push(Door {
            point_out: MapPoint::new(100.0, 200.0),
            point_in: MapPoint::new(140.0, 200.0),
            sector_out: SectorNumber::new(40),
            sector_in: SectorNumber::new(41),
            sector_out_index: Some(raw_index),
            sector_in_index: Some(endpoint_index),
            layer_out: 2,
            layer_in: 2,
            ..Door::default()
        });

        let mut soldier = ActorSoldier {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
                initial_element.kind = ElementKind::ActorSoldier;
                initial_element
            },
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: crate::element::SoldierData {
                cached_camp: crate::element::Camp::Lacklandists,
                ..Default::default()
            },
        };
        soldier.npc.ai_brain = AiBrain::Enemy(Box::default());
        soldier
            .element
            .set_position_map(MapPoint::new(110.0, 200.0));
        soldier.element.set_sector(Some(raw_sector));
        soldier.element.set_layer(2);
        let owner = engine.add_test_entity(Entity::Soldier(soldier));
        let position = engine.get_entity_mut(owner).unwrap().position_iface_mut();
        position.set_sector_topology(Some(raw_sector), Some(raw_index));
        position.set_door(
            DoorHandle::new(0).expect("zero is a valid door index"),
            true,
        );

        let destination = crate::ai::Position {
            x: 180.0,
            y: 200.0,
            sector: Some(endpoint_sector),
            level: 2,
        };
        let sequence_id = engine
            .launch_ai_move(
                &crate::sim_rng::test_context(),
                &LevelAssets::new(),
                owner,
                destination,
                crate::ai::GotoFlags::RUN,
                0.0,
            )
            .expect("adapted movement must register its sequence inline");

        let sequence = engine
            .orders
            .sequence_manager
            .get_sequence(sequence_id)
            .expect("launched adapted movement sequence");
        assert_eq!(sequence.elements.len(), 1);
        assert_eq!(sequence.elements[0].command, crate::element::Command::Move);
        let crate::sequence::SequenceElementData::Movement {
            destination,
            layer,
            sector,
            flags,
            tolerance,
            action,
            ..
        } = &sequence.elements[0].data
        else {
            panic!("adapted movement request did not produce a movement element")
        };
        assert_eq!(*destination, MapPoint::new(180.0, 200.0));
        assert_eq!(*layer, 2);
        assert_eq!(
            *sector, None,
            "the movement sequence's trailing point Move does not retain a sector field"
        );
        assert_eq!(*flags, crate::sequence::MoveFlags::empty());
        assert_eq!(*tolerance, 0.0);
        assert_eq!(*action, crate::order::OrderType::RunningUpright);
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
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
                initial_element.kind = ElementKind::ActorSoldier;
                initial_element
            },
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: crate::element::SoldierData {
                cached_camp: crate::element::Camp::Lacklandists,
                ..Default::default()
            },
        };
        soldier.npc.ai_brain = AiBrain::Enemy(Box::default());
        soldier
            .element
            .set_position_map(MapPoint::new(630.0, 1408.0));
        soldier.element.set_layer(4);
        // Legacy adoption retained the public sector but not its exact sector reference.
        soldier.element.set_sector(SectorHandle::new(104));
        let owner = engine.add_test_entity(Entity::Soldier(soldier));

        let position = engine.live_ai_position(owner);
        assert_eq!((position.x, position.y), (630.0, 1408.0));
        assert_eq!(
            position.sector.and_then(|sector| sector.arena_index()),
            SectorIndex::new(source)
        );

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
            (position.x, position.y),
            position.sector.unwrap().get(),
            position.sector.and_then(|sector| sector.arena_index()),
            (531.231, 1268.2043),
            89,
            SectorIndex::new(goal),
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
                    door_index: crate::gate::DoorIndex::new(0).expect("valid door index"),
                    direct: true,
                },
                GatePathStep {
                    door_index: crate::gate::DoorIndex::new(1).expect("valid door index"),
                    direct: false,
                },
            ]
        );
    }
}
