use super::*;

impl EngineInner {
    /// Build a movement sequence that traverses a gate path from
    /// `find_path_gates` and ends at `goal` on `goal_layer`.
    ///
    /// Unifies the three goal shapes (point, door, line) — callers
    /// pick via [`GoalShape`].
    ///
    /// For each gate in the path the emitted sub-elements depend on
    /// the gate's type:
    ///
    /// * **Jump gates** emit a single `Jump` element carrying the
    ///   source / destination `JumpLine` indices.  The tick handler
    ///   consumes those via [`EngineInner::start_jump`].
    /// * **Building doors** (previous sector `is_building()` true)
    ///   emit `WaitTimer(50)` (skipped on the first gate),
    ///   `WaitTimer(rand & 15 + rand & 15)`, and `ChangePosition` to
    ///   the gate's outside point.  The wait + teleport drives the
    ///   "actor walks inside the building and re-emerges" illusion.
    /// * **Regular doors** emit `Move` to the gate's entry point
    ///   followed by `AssertPosition` that the actor reached it.
    ///
    /// After the approach sub-elements, the door itself is crossed:
    ///
    /// * A **locked PC door** that the PC can pick (`unlockable` +
    ///   `has_lockpick`) emits `Turn` toward the lock then
    ///   `UnlockDoor` and *returns* — the door is expected to re-issue
    ///   the move once the lockpick animation terminates.
    /// * Ladder-lift sectors interpose a `WaitFreeLift` before
    ///   `PassDoor` so the climber waits for the ladder to free up.
    /// * All other doors emit `PassDoor` + `AssertPosition`.
    ///
    /// Trailing emission depends on [`GoalShape`]:
    ///
    /// * **Point goal** — emit a plain `Move` to the goal point
    ///   unless the last gate dropped the actor into a building
    ///   sector.  Skipped entirely when `move_after_last_door` is
    ///   `false` (the "walk up to the door" variant).
    /// * **Door goal** — emit the building CHANGE_POSITION or plain
    ///   MOVE to the far-side point of the goal door, then optionally
    ///   TURN + UNLOCK_DOOR for PC-lockable goal doors.
    /// * **Line goal** — emit a plain `Move` to the line's midpoint
    ///   carrying `MoveFlags::LINE` and the line id so the actor's
    ///   arrival check snaps to line tolerance.  Intermediate gate
    ///   moves never carry `MoveFlags::LINE`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_gate_movement_sequence(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        entity_id: EntityId,
        source_sector: Option<crate::position_interface::SectorHandle>,
        gate_path: Vec<crate::gate::GatePathStep>,
        goal: GoalShape,
        goal_layer: u16,
        base_action: OrderType,
        move_after_last_door: bool,
        speed_factor: f32,
        initial_flags: crate::sequence::MoveFlags,
        prefix_elements: Vec<crate::sequence::SequenceElement>,
        tail_elements: Vec<crate::sequence::SequenceElement>,
        append_arrival_speech: bool,
        append_recovery: bool,
    ) -> Option<crate::sequence::SequenceId> {
        use crate::element::Command;
        use crate::sequence::{
            Field, FieldValue, MoveFlags, Sequence, SequenceElement, SequenceElementData,
        };

        // Determine first jump gate.  Every gate *before* the first
        // jump gets the `TO_JUMP` flag so its movement element sets
        // the actor up for the jump.  Only the Point and Door goal
        // variants apply this flag-mutation; the Line variant passes
        // the input flags to every gate unmodified, so suppress the
        // OR for `GoalShape::Line`.
        let apply_to_jump = !matches!(goal, GoalShape::Line { .. });
        let first_jump: Option<usize> = if apply_to_jump {
            gate_path.iter().enumerate().find_map(|(i, step)| {
                let is_jump = self
                    .scripts
                    .mission
                    .as_ref()
                    .and_then(|_| {
                        self.script_domains
                            .interactables
                            .doors
                            .get(usize::from(step.door_index))
                    })
                    .map(|d| d.is_jump())
                    .unwrap_or(false);
                if is_jump { Some(i) } else { None }
            })
        } else {
            None
        };

        let flags_at = |gate_idx: usize| -> MoveFlags {
            match first_jump {
                Some(j) if gate_idx <= j => initial_flags | MoveFlags::TO_JUMP,
                _ => initial_flags,
            }
        };

        // Snapshot the canonical gate data in one short borrow so the main
        // loop can call grid and sequence helpers on `self` without fighting
        // the borrow checker.
        #[derive(Clone, Copy)]
        struct GateShot {
            door_index: crate::gate::DoorIndex,
            direct: bool,
            // Exact sector on the source side of this gate.  This is the
            // route's retained equivalent of Original's `pOldSector` when a
            // legacy/compatibility caller supplies only the public number.
            old_sector: crate::position_interface::SectorHandle,
            // Geometry used by the emitted sub-elements.
            entry: MapPoint,
            exit: MapPoint,
            entry_layer: u16,
            exit_layer: u16,
            // Where the actor ends up *after* crossing.
            new_sector: crate::position_interface::SectorHandle,
            // Gate typing.
            is_jump: bool,
            jump_line_src: Option<crate::jump_line::JumpLineIndex>,
            jump_line_dst: Option<crate::jump_line::JumpLineIndex>,
            // Door typing (only meaningful when !is_jump).
            is_locked_pc_unlockable: bool,
            // Original RHsequence.cpp keeps the caller's action on
            // gate approach, WAIT_FREE_LIFT, PASS_DOOR, and post-pass
            // asserts.  Door-specific GetAction1/2 calls exist in
            // original-code but are commented out at execution time.
            entry_action: OrderType,
            door_action: OrderType,
        }

        let gate_shots = {
            self.scripts.mission.as_ref()?;
            let shots: Vec<GateShot> = gate_path
                .iter()
                .filter_map(|step| {
                    let door = self
                        .script_domains
                        .interactables
                        .doors
                        .get(usize::from(step.door_index))?;
                    let (
                        entry,
                        exit,
                        entry_layer,
                        exit_layer,
                        old_sector_number,
                        old_sector_index,
                        new_sector_number,
                        new_sector_index,
                    ) = if step.direct {
                        (
                            door.point_out,
                            door.point_in,
                            door.layer_out,
                            door.layer_in,
                            u16::from(door.sector_out),
                            door.sector_out_index,
                            u16::from(door.sector_in),
                            door.sector_in_index,
                        )
                    } else {
                        (
                            door.point_in,
                            door.point_out,
                            door.layer_in,
                            door.layer_out,
                            u16::from(door.sector_in),
                            door.sector_in_index,
                            u16::from(door.sector_out),
                            door.sector_out_index,
                        )
                    };
                    let old_sector = crate::position_interface::SectorHandle::new(
                        old_sector_number,
                    )
                    .map(|handle| {
                        old_sector_index.map_or(handle, |index| handle.with_arena_index(index))
                    })?;
                    let new_sector = crate::position_interface::SectorHandle::new(
                        new_sector_number,
                    )
                    .map(|handle| {
                        new_sector_index.map_or(handle, |index| handle.with_arena_index(index))
                    })?;
                    let is_jump = door.is_jump();
                    let (jump_src, jump_dst) = if is_jump {
                        let (s, d) = if step.direct {
                            (door.jump_line_out, door.jump_line_in)
                        } else {
                            (door.jump_line_in, door.jump_line_out)
                        };
                        (
                            s.and_then(crate::jump_line::JumpLineIndex::new),
                            d.and_then(crate::jump_line::JumpLineIndex::new),
                        )
                    } else {
                        (None, None)
                    };
                    let is_locked_pc_unlockable = !is_jump && door.locked_pc && door.unlockable;
                    let (entry_action, door_action) = (base_action, base_action);
                    Some(GateShot {
                        door_index: step.door_index,
                        direct: step.direct,
                        old_sector,
                        entry,
                        exit,
                        entry_layer,
                        exit_layer,
                        new_sector,
                        is_jump,
                        jump_line_src: jump_src,
                        jump_line_dst: jump_dst,
                        is_locked_pc_unlockable,
                        entry_action,
                        door_action,
                    })
                })
                .collect();
            shots
        }; // host borrow dropped here

        // Does the entity have the lockpick contextual action?
        // Needed to choose the lockpick sub-element branch.
        let has_lockpick = self
            .expect_entity(entity_id, "gate-route lockpick check")
            .actor_auth_info()
            .has_lockpick;

        // Original carries exact sector pointers through pOldSector. Public
        // numbers are not unique, so consult retained arena identity first;
        // the number map is only the compatibility fallback for old saves.
        let is_building_sector =
            |this: &Self, sector: crate::position_interface::SectorHandle| -> bool {
                route_sector_by_exact_handle(this, sector)
                    .map(|gs| gs.sector_type.is_building())
                    .unwrap_or(false)
            };

        let is_ladder_lift =
            |this: &Self, sector: crate::position_interface::SectorHandle| -> bool {
                route_sector_by_exact_handle(this, sector)
                    .and_then(|gs| gs.lift_type)
                    .map(|lt| lt == crate::sector::LiftType::Ladder)
                    .unwrap_or(false)
            };

        let mut seq = Sequence::new();
        let mut level: u16 = 1;

        for mut elem in prefix_elements {
            elem.command_level = level;
            seq.append_element(elem);
            level += 1;
        }

        // Track the "previous" sector so each gate knows what it's
        // coming *from*.  After the first gate, this is the
        // previous gate's `new_sector`.
        // Original carries the (possibly door-adapted) `pSectorSource`
        // argument through both the leading assertion and `pOldSector`.
        // Do not reconstruct it from the first gate: authored/loaded gate
        // sides can contain the invalid-sector sentinel even when the route
        // source is a valid ordinary motion sector.
        assert!(
            gate_shots.is_empty() || source_sector.is_some(),
            "gate movement route for {entity_id:?} has no source sector"
        );
        let mut prev_sector = source_sector;

        // Cross-sector source-sector sanity assert.  When the goal
        // sector differs from the source, prepend an `AssertPosition`
        // against the source sector so the actor's location is
        // re-validated right before the gate walk begins; if the actor
        // was nudged out between scheduling and dispatch the sequence
        // aborts gracefully instead of following a stale path.  This
        // unified builder is only invoked for cross-sector traversals
        // (callers handle same-sector inline), so emit unconditionally.
        if let Some(source_sector) = source_sector {
            let mut leading_ap = SequenceElement::new_movement(
                level,
                Command::AssertPosition,
                Some(entity_id),
                base_action,
            );
            leading_ap.data = SequenceElementData::Movement {
                destination: crate::coordinates::MapPoint::default(),
                layer: 0,
                sector: Some(source_sector),
                gate_id: None,
                line_id: None,
                element: Some(entity_id),
                flags: MoveFlags::empty(),
                // Original leading cross-sector ASSERT_POSITION uses
                // the constructor default tolerance (0).  Gate-entry
                // and post-pass asserts explicitly pass 10.
                tolerance: 0.0,
                direction: 0,
                action: base_action,
                speed_factor,
                post_seek_sequence: None,
            };
            seq.append_element(leading_ap);
            level += 1;
        }

        let entity_goal = match goal {
            GoalShape::Seek {
                target, tolerance, ..
            } => Some((target, tolerance, true)),
            GoalShape::Target {
                target, tolerance, ..
            } => Some((target, tolerance, false)),
            _ => None,
        };

        // Goal-point used for the trailing MOVE (if any).  For Point
        // goals this is the caller's point; for Line goals it's the
        // line's midpoint; for Door goals it's the approach point on
        // the near side of the goal door.
        let goal_point = goal.goal_point();

        // Tracks whether a lockpick branch on an intermediate gate
        // terminated the sequence early.
        let mut ended_early = false;

        // Element count captured after the leading AssertPosition (if
        // any) and used by the building-source branch to skip the
        // 50-frame WaitTimer on the first gate's emission.
        let first_gate_element_count = seq.elements.len();

        for (gate_idx, shot) in gate_shots.iter().enumerate() {
            let gate_flags = flags_at(gate_idx);
            // -------- Gate approach branch --------
            //
            // Doors and jumps both first move to the gate's source
            // point (or CHANGE_POSITION out of a building), matching
            // original AppendMoveToSequence.  The door-vs-jump split
            // happens after this approach.
            // `FindPath*` retains exact sector pointers on every gate. The
            // first retained gate's agreeing source side is therefore the
            // exact identity of Original's initial `pOldSector`. This remains
            // authoritative even when a restored/spatial source handle has
            // an arena index: overlapping sectors can give that position the
            // opposite alias (building vs ordinary) while the selected route
            // still proves which sector pointer FindPath used. Never recover
            // through a disagreeing public number, and after the first gate
            // keep using the previous gate's exact `pNewSector` equivalent.
            let old_sector_for_classification = prev_sector.map(|sector| {
                if gate_idx == 0 && u16::from(sector) == u16::from(shot.old_sector) {
                    shot.old_sector
                } else {
                    sector
                }
            });
            let old_is_building = old_sector_for_classification
                .map(|s| is_building_sector(self, s))
                .unwrap_or(false);
            tracing::trace!(
                entity = ?entity_id,
                gate_idx,
                door = ?shot.door_index,
                direct = shot.direct,
                ?prev_sector,
                new_sector = ?shot.new_sector,
                old_is_building,
                is_jump = shot.is_jump,
                "gate-traversal sequence emits a gate"
            );

            // Original sequence construction uses the caller's action
            // for both approach and door-pass sub-elements.
            let entry_action = shot.entry_action;
            let door_action = shot.door_action;

            if old_is_building {
                // When the previous sector is a building, the actor
                // "walks inside" by waiting out a timer then
                // teleporting to the gate's outside point.  Two
                // WaitTimer elements: the 50-frame one is only added
                // when there was already a prior gate-emitted element
                // (so the very first gate skips it).
                let wait_command = if matches!(goal, GoalShape::Line { .. }) {
                    Command::Wait
                } else {
                    Command::WaitTimer
                };
                if seq.elements.len() != first_gate_element_count {
                    let mut w = SequenceElement::new_generic(level, wait_command, Some(entity_id));
                    w.set_property(Field::Timer, FieldValue::Integer(50));
                    seq.append_element(w);
                    level += 1;
                }
                // Original: `RHSequence::AppendMoveToSequence` in
                // `original-code/RHsequence.cpp:484` sums two `rand() & 15`
                // draws for this building-exit wait.
                let r = building_exit_wait_frames(sim);
                let mut w = SequenceElement::new_generic(level, wait_command, Some(entity_id));
                w.set_property(Field::Timer, FieldValue::Integer(r));
                seq.append_element(w);
                level += 1;

                // CHANGE_POSITION — instant teleport to the gate's
                // "outside" point (the `entry` in our direction).
                // Compute a 0..15 direction from (exit - entry) so
                // the sprite is facing the exit.  We stuff that into
                // the element's direction field for the tick handler
                // to apply.
                let dx = shot.exit.x - shot.entry.x;
                let dy = shot.exit.y - shot.entry.y;
                let dir = crate::position_interface::vector_to_sector_0_to_15(dx, dy);
                let mut cp = SequenceElement::new_movement(
                    level,
                    Command::ChangePosition,
                    Some(entity_id),
                    entry_action,
                );
                cp.data = SequenceElementData::Movement {
                    destination: shot.entry,
                    layer: shot.entry_layer,
                    // Assert actor is still in the building sector
                    // before teleporting.  Building teleport is an
                    // in-sector position change, not a door-pass, so
                    // no gate ref is attached.
                    sector: prev_sector,
                    gate_id: None,
                    line_id: None,
                    element: None,
                    flags: gate_flags,
                    tolerance: 0.0,
                    direction: dir,
                    action: entry_action,
                    speed_factor,
                    post_seek_sequence: None,
                };
                seq.append_element(cp);
                level += 1;
            } else {
                // MOVE to gate entry point on the source side.
                let gate_seek_target = entity_goal.map(|(target, _, _)| target);
                let mut m = SequenceElement::new_movement(
                    level,
                    Command::Move,
                    Some(entity_id),
                    entry_action,
                );
                m.data = SequenceElementData::Movement {
                    destination: shot.entry,
                    layer: 0,
                    sector: None,
                    // Original gate-approach MOVE uses the plain
                    // point+victim constructor and does not SetGate;
                    // only WAIT_FREE_LIFT/PASS_DOOR carry the gate.
                    gate_id: None,
                    line_id: None,
                    element: gate_seek_target,
                    flags: gate_flags,
                    // Original AppendMoveToSequence passes the
                    // seek victim through to gate-approach moves but
                    // uses tolerance 0; fTolerance belongs to the
                    // final goal/seek move.
                    tolerance: 0.0,
                    direction: 0,
                    action: entry_action,
                    speed_factor,
                    post_seek_sequence: None,
                };
                seq.append_element(m);
                level += 1;

                // ASSERT_POSITION that the actor actually reached
                // the gate.  Tolerance is 10.
                let mut ap = SequenceElement::new_movement(
                    level,
                    Command::AssertPosition,
                    Some(entity_id),
                    entry_action,
                );
                ap.data = SequenceElementData::Movement {
                    destination: shot.entry,
                    layer: 0,
                    sector: None,
                    gate_id: None,
                    line_id: None,
                    element: Some(entity_id),
                    flags: MoveFlags::empty(),
                    tolerance: 10.0,
                    direction: 0,
                    action: entry_action,
                    speed_factor,
                    post_seek_sequence: None,
                };
                seq.append_element(ap);
                level += 1;
            }

            // -------- Jump gate branch --------
            //
            // After the approach/assert above, a jump gate emits a
            // single `Jump` generic element carrying the source and
            // destination jump-line indices.  The tick handler
            // consumes these in `start_jump`.
            if shot.is_jump {
                let (src, dst) = match (shot.jump_line_src, shot.jump_line_dst) {
                    (Some(s), Some(d)) => (s, d),
                    _ => {
                        tracing::warn!(
                            gate = %shot.door_index,
                            "Jump gate missing jump_line indices; skipping jump element"
                        );
                        prev_sector = Some(shot.new_sector);
                        continue;
                    }
                };
                let mut jump_elem =
                    SequenceElement::new_generic(level, Command::JumpCmd, Some(entity_id));
                jump_elem.set_property(Field::JumplineSource, FieldValue::LineId(src));
                jump_elem.set_property(Field::JumplineDestination, FieldValue::LineId(dst));
                seq.append_element(jump_elem);
                level += 1;
                prev_sector = Some(shot.new_sector);
                continue;
            }

            // -------- Lockpick branch --------
            //
            // When the door is PC-locked and the PC has the lockpick
            // action, the sequence terminates after TURN + UNLOCK_DOOR
            // — the unlock animation flips `locked_pc` off and the
            // caller re-issues the move command to resume the path.
            if shot.is_locked_pc_unlockable && has_lockpick {
                // Original uses `mbDirect ? pointIn : pointOut`, which is
                // the path-local exit for either traversal direction, so the
                // sprite faces the lock while picking it.
                let camera_pt = shot.exit;
                let mut turn = SequenceElement::new_generic(level, Command::Turn, Some(entity_id));
                turn.set_property(
                    Field::CameraPoint,
                    FieldValue::GeoPoint2D {
                        x: camera_pt.x,
                        y: camera_pt.y,
                    },
                );
                seq.append_element(turn);
                level += 1;

                // UNLOCK_DOOR — the tick handler reads the door id
                // from `Field::Door` and picks UnlockingDoor vs
                // UnlockingTrap from the door table on its own.
                let mut unlock =
                    SequenceElement::new_generic(level, Command::UnlockDoor, Some(entity_id));
                unlock.set_property(Field::Door, FieldValue::DoorId(shot.door_index));
                seq.append_element(unlock);
                level += 1;

                // Early return — the lockpick animation will re-issue
                // the move once it terminates.
                ended_early = true;
                break;
            }

            // -------- Ladder lift wait --------
            if is_ladder_lift(self, shot.new_sector) {
                let mut wait = SequenceElement::new_movement(
                    level,
                    Command::WaitFreeLift,
                    Some(entity_id),
                    door_action,
                );
                wait.data = SequenceElementData::Movement {
                    destination: crate::coordinates::MapPoint::default(),
                    layer: 0,
                    sector: Some(shot.new_sector),
                    gate_id: Some(shot.door_index),
                    line_id: None,
                    element: None,
                    flags: MoveFlags::empty(),
                    tolerance: 0.0,
                    direction: 0,
                    action: door_action,
                    speed_factor,
                    post_seek_sequence: None,
                };
                seq.append_element(wait);
                level += 1;
            }

            // -------- PASS_DOOR + post-pass assert --------
            let mut pass = SequenceElement::new_movement(
                level,
                Command::PassDoor,
                Some(entity_id),
                door_action,
            );
            pass.data = SequenceElementData::Movement {
                destination: shot.exit,
                layer: shot.exit_layer,
                sector: None,
                gate_id: Some(shot.door_index),
                line_id: None,
                element: None,
                // Original PASS_DOOR constructor uses default flags
                // and only attaches the gate via SetGate.
                flags: MoveFlags::empty(),
                tolerance: 0.0,
                // The Original gate carries path-local `RHGate::mbDirect`
                // while constructing PassDoor. AI::Position then reads the
                // selected movement element's direction to commit the actor
                // to the side it is entering. Materialize that traversal
                // direction instead of leaving Rust's element at its default.
                direction: i16::from(shot.direct),
                action: door_action,
                speed_factor,
                post_seek_sequence: None,
            };
            seq.append_element(pass);
            level += 1;

            // ASSERT_POSITION that the actor reached the exit point.
            let mut ap = SequenceElement::new_movement(
                level,
                Command::AssertPosition,
                Some(entity_id),
                door_action,
            );
            ap.data = SequenceElementData::Movement {
                destination: shot.exit,
                layer: 0,
                sector: None,
                gate_id: None,
                line_id: None,
                element: Some(entity_id),
                flags: MoveFlags::empty(),
                tolerance: 10.0,
                direction: 0,
                action: door_action,
                speed_factor,
                post_seek_sequence: None,
            };
            seq.append_element(ap);
            level += 1;

            prev_sector = Some(shot.new_sector);
        }

        // Clear TO_JUMP once we're past the last jump gate — the
        // trailing MOVE uses `initial_flags` unmodified.
        let trailing_flags = initial_flags;

        // Trailing emission.  Three goal shapes, three branches:
        //
        // * Point: emit MOVE to `goal_point`, subject to
        //   `move_after_last_door` and the building-sector
        //   short-circuit.
        // * Door: handle the goal-door's approach / CHANGE_POSITION
        //   into building / PC-lockpick tail.
        // * Line: emit MOVE with `MoveFlags::LINE` + `line_id` so
        //   arrival snaps to the line's tolerance window.
        if !ended_early {
            let last_into_building = prev_sector
                .map(|s| is_building_sector(self, s))
                .unwrap_or(false);

            match goal {
                GoalShape::Point { .. } | GoalShape::Seek { .. } | GoalShape::Target { .. } => {
                    if move_after_last_door && !last_into_building {
                        let (seek_target, seek_tolerance, seek_flags) = match goal {
                            GoalShape::Seek {
                                target, tolerance, ..
                            } => (Some(target), tolerance, trailing_flags | MoveFlags::SEEK),
                            GoalShape::Target {
                                target, tolerance, ..
                            } => (Some(target), tolerance, trailing_flags),
                            GoalShape::Point { tolerance, .. } => (None, tolerance, trailing_flags),
                            _ => unreachable!("point/entity trailing branch"),
                        };
                        let mut final_move = SequenceElement::new_movement(
                            level,
                            Command::Move,
                            Some(entity_id),
                            base_action,
                        );
                        final_move.data = SequenceElementData::Movement {
                            destination: goal_point,
                            layer: goal_layer,
                            sector: None,
                            gate_id: None,
                            line_id: None,
                            element: seek_target,
                            flags: seek_flags,
                            tolerance: seek_tolerance,
                            direction: 0,
                            action: base_action,
                            speed_factor,
                            post_seek_sequence: None,
                        };
                        seq.append_element(final_move);
                        level += 1;
                    }

                    // When SEEK is set and the last gate landed us
                    // inside a building sector, emit a trailing MOVE
                    // back to the last gate's `point_in` so the actor
                    // doesn't get stuck at the interior teleport point.
                    if last_into_building
                        && initial_flags.contains(MoveFlags::SEEK)
                        && let Some(last_shot) = gate_shots.last()
                    {
                        let (seek_target, seek_tolerance, seek_flags) = entity_goal
                            .map(|(target, tolerance, is_seek)| {
                                (
                                    Some(target),
                                    tolerance,
                                    if is_seek {
                                        trailing_flags | MoveFlags::SEEK
                                    } else {
                                        trailing_flags
                                    },
                                )
                            })
                            .unwrap_or((None, 0.0, trailing_flags));
                        let point_in = {
                            self.scripts
                                .mission
                                .as_ref()
                                .and_then(|_| {
                                    self.script_domains
                                        .interactables
                                        .doors
                                        .get(usize::from(last_shot.door_index))
                                })
                                .map(|d| d.point_in)
                                .unwrap_or(last_shot.exit)
                        };
                        let mut seek_move = SequenceElement::new_movement(
                            level,
                            Command::Move,
                            Some(entity_id),
                            base_action,
                        );
                        seek_move.data = SequenceElementData::Movement {
                            destination: point_in,
                            layer: goal_layer,
                            sector: None,
                            gate_id: None,
                            line_id: None,
                            element: seek_target,
                            flags: seek_flags,
                            tolerance: seek_tolerance,
                            direction: 0,
                            action: base_action,
                            speed_factor,
                            post_seek_sequence: None,
                        };
                        seq.append_element(seek_move);
                    }
                }
                GoalShape::Line {
                    line_index,
                    tolerance,
                    ..
                } => {
                    // Emit `Move` to the line goal with
                    // `MoveFlags::LINE` and the line id.  When the
                    // last gate landed in a building, bail out without
                    // emitting.
                    if !last_into_building {
                        let mut final_move = SequenceElement::new_movement(
                            level,
                            Command::Move,
                            Some(entity_id),
                            base_action,
                        );
                        final_move.data = SequenceElementData::Movement {
                            destination: goal_point,
                            layer: goal_layer,
                            sector: None,
                            gate_id: None,
                            line_id: Some(line_index),
                            element: None,
                            flags: trailing_flags | MoveFlags::LINE,
                            tolerance,
                            direction: 0,
                            action: base_action,
                            speed_factor,
                            post_seek_sequence: None,
                        };
                        seq.append_element(final_move);
                        // AppendMoveToLineToSequence advances uwCount after
                        // the terminal LINE movement.  The caller's explicit
                        // JumpCmd must be on the following command level, not
                        // dispatched concurrently with line arrival.
                        level += 1;
                    }
                }
                GoalShape::Door {
                    door_index,
                    far_side_point,
                    far_side_layer,
                    far_side_is_building,
                } => {
                    // Hoist the goal door's PC-lockable lookup so the
                    // trailing lockpick tail fires regardless of which
                    // trailing branch was taken.  The lockpick tail is
                    // unconditional on the goal door's PC-locked flag,
                    // not on which branch (building vs non-building)
                    // was selected.
                    let goal_door_pc_lockable = {
                        self.scripts
                            .mission
                            .as_ref()
                            .and_then(|_| {
                                self.script_domains
                                    .interactables
                                    .doors
                                    .get(usize::from(door_index))
                            })
                            .map(|d| d.locked_pc && d.unlockable)
                            .unwrap_or(false)
                    };
                    if !move_after_last_door {
                        // "Stop at the door" variant — caller set
                        // `move_after_last_door=false` to skip the
                        // trailing MOVE.  The gate-path includes the
                        // goal door as the last gate, so the loop
                        // already emitted approach + PASS_DOOR for it.
                        // Nothing to emit here.
                    } else if far_side_is_building {
                        // Random 0..30 frames wait + CHANGE_POSITION
                        // teleport into the building interior. Original:
                        // `original-code/RHsequence.cpp:905`. The
                        // direction stuffed on the element is the
                        // door's `point_out - point_in` sector-index.
                        let r = building_exit_wait_frames(sim);
                        let mut wait = SequenceElement::new_generic(
                            level,
                            Command::WaitTimer,
                            Some(entity_id),
                        );
                        wait.set_property(Field::Timer, FieldValue::Integer(r));
                        seq.append_element(wait);
                        level += 1;

                        let (dx, dy) = {
                            let d = self.scripts.mission.as_ref().and_then(|_| {
                                self.script_domains
                                    .interactables
                                    .doors
                                    .get(usize::from(door_index))
                            });
                            match d {
                                Some(d) => {
                                    (d.point_out.x - d.point_in.x, d.point_out.y - d.point_in.y)
                                }
                                None => (0.0, 0.0),
                            }
                        };
                        let dir = vector_to_sector_0_to_15(dx, dy);
                        let mut cp = SequenceElement::new_movement(
                            level,
                            Command::ChangePosition,
                            Some(entity_id),
                            base_action,
                        );
                        cp.data = SequenceElementData::Movement {
                            destination: far_side_point,
                            layer: far_side_layer,
                            sector: prev_sector,
                            // No gate ref on the building-interior
                            // CHANGE_POSITION (it is an in-sector
                            // teleport).
                            gate_id: None,
                            line_id: None,
                            element: None,
                            flags: trailing_flags,
                            tolerance: 0.0,
                            direction: dir,
                            action: base_action,
                            speed_factor,
                            post_seek_sequence: None,
                        };
                        seq.append_element(cp);
                        level += 1;
                    } else {
                        // Plain MOVE to the goal door's far-side
                        // point.  No `last_into_building` guard here —
                        // the trailing MOVE fires unconditionally for
                        // non-building goal doors.
                        let mut final_move = SequenceElement::new_movement(
                            level,
                            Command::Move,
                            Some(entity_id),
                            base_action,
                        );
                        final_move.data = SequenceElementData::Movement {
                            destination: far_side_point,
                            layer: far_side_layer,
                            sector: None,
                            // Original AppendMoveToDoorToSequence's
                            // trailing goal move is a plain MOVE to
                            // ptGoal; it does not call SetGate.
                            gate_id: None,
                            line_id: None,
                            element: None,
                            flags: trailing_flags,
                            tolerance: 0.0,
                            direction: 0,
                            action: base_action,
                            speed_factor,
                            post_seek_sequence: None,
                        };
                        seq.append_element(final_move);
                        level += 1;
                    }

                    // After the trailing MOVE / CHANGE_POSITION, if
                    // the goal door is PC-lockable and the actor has
                    // lockpick, emit TURN toward the lock +
                    // UNLOCK_DOOR.  The goal door is *not* included in
                    // `gate_path` for the door-goal case
                    // (`find_path_to_door` pops it), so the in-loop
                    // lockpick branch didn't fire for it — this is
                    // where the "walk up to door and pick it" finale
                    // is emitted.
                    if goal_door_pc_lockable && has_lockpick {
                        let (cam_pt, direct) = {
                            let d = self.scripts.mission.as_ref().and_then(|_| {
                                self.script_domains
                                    .interactables
                                    .doors
                                    .get(usize::from(door_index))
                            });
                            // Use the path-direction the gate was
                            // approached in.  When the goal door was
                            // excluded from `gate_path` the caller
                            // signals that direction implicitly via
                            // `far_side_point` — it matches the
                            // door's near-side endpoint for the
                            // approach side.  Recover the direction
                            // by comparing endpoints.
                            let direct = d
                                .map(|d| {
                                    let dx = far_side_point.x - d.point_out.x;
                                    let dy = far_side_point.y - d.point_out.y;
                                    (dx * dx + dy * dy) < 1e-4
                                })
                                .unwrap_or(true);
                            let cam = d
                                .map(|d| if direct { d.point_in } else { d.point_out })
                                .unwrap_or(far_side_point);
                            (cam, direct)
                        };
                        let _ = direct;
                        let mut turn =
                            SequenceElement::new_generic(level, Command::Turn, Some(entity_id));
                        turn.set_property(
                            Field::CameraPoint,
                            FieldValue::GeoPoint2D {
                                x: cam_pt.x,
                                y: cam_pt.y,
                            },
                        );
                        seq.append_element(turn);
                        level += 1;

                        let mut unlock = SequenceElement::new_generic(
                            level,
                            Command::UnlockDoor,
                            Some(entity_id),
                        );
                        unlock.set_property(Field::Door, FieldValue::DoorId(door_index));
                        seq.append_element(unlock);
                    }
                }
            }
        }

        for mut elem in tail_elements {
            elem.command_level = level;
            seq.append_element(elem);
            level += 1;
        }

        // Append a `SpeakHeroReachDestination` element at the tail of
        // the gate-movement sequence so the PC barks the "I have
        // arrived" line once the destination is reached. Original
        // `PerformMove` passes the incremented `uwCount` left by
        // `AppendMoveToSequence`, so speech is the next command level,
        // after the final movement has completed. The PC's `Instruct`
        // override terminates it on dispatch and queues
        // `HeroDoneCommand` via `arbitrate_instruct`.
        if append_arrival_speech && !seq.is_empty() {
            let speak_level = seq
                .last()
                .map(|element| element.command_level.saturating_add(1))
                .unwrap_or(level);
            let speak = SequenceElement::new(
                speak_level,
                Command::SpeakHeroReachDestination,
                Some(entity_id),
            );
            seq.append_element(speak);
        }

        // Append posture-recovery sub-elements right after the Speak
        // element so a PC mid-bow-aim / crouched / helping-climb /
        // simulating-beggar ends the order in a neutral posture
        // instead of frozen in their pre-move state.  Only fires for
        // PCs; `append_posture_recovery` bails on non-PC entities.
        if append_recovery {
            self.append_posture_recovery(entity_id, &mut seq);
        }

        let seq_id = self.launch_sequence(seq);
        tracing::trace!(
            entity = ?entity_id,
            ?seq_id,
            gates = gate_path.len(),
            early = ended_early,
            goal = ?goal,
            move_after_last_door,
            "Launched gate-traversal movement sequence"
        );

        // Destination markers are emitted by player group-move callers
        // only; AI/pathfinding callers use this helper without dropping
        // a ground mark.
        Some(seq_id)
    }

    /// Append posture-cleanup sub-elements at the tail of a PC move
    /// sequence so the PC ends the order in a neutral posture rather
    /// than frozen in the pre-move state.
    ///
    /// Covers:
    ///
    /// * **Shoot-bow drain** — if the sequence currently ends with a
    ///   `Command::ShootBow` element *and* the PC is no longer aiming,
    ///   demote that trailing element to `Command::ShootBowOnce` so the
    ///   queued shot fires exactly once before the walk resumes.
    /// * **Upright + bow-aim** → append `EQUIP_BOW` (re-arms the bow so
    ///   the aim state is re-entered after the walk).
    /// * **Crouched + last command ≠ CrouchUp** → append `CROUCH_DOWN`
    ///   so the PC re-crouches at the destination.
    /// * **HelpingToClimb** → append `ENTER_HELPING_CLIMB`.
    /// * **SimulatingBeggar** → append `ENTER_BEGGAR`.
    ///
    /// When the input sequence ends in SEEK, recovery is appended to
    /// that movement element's post-seek sub-sequence so it fires only
    /// on successful seek completion, not on seek abort.
    pub(crate) fn append_posture_recovery(
        &self,
        pc_id: EntityId,
        sequence: &mut crate::sequence::Sequence,
    ) {
        use crate::element::Command;
        let entity = self
            .get_entity(pc_id)
            .unwrap_or_else(|| panic!("posture-recovery owner {pc_id:?} disappeared"));
        if !entity.is_pc() {
            return;
        }
        let posture = entity.element_data().posture;
        let actor = entity
            .actor_data()
            .unwrap_or_else(|| panic!("posture-recovery PC {pc_id:?} has no actor state"));
        let action_state = actor.action_state;

        // Drill into the SEEK element's post-seek sub-sequence when the
        // inbound sequence ends with `Command::Seek`. Promote the flat
        // continuation temporarily, reuse the ordinary append logic, then
        // validate and store it as a one-level continuation again.
        if sequence
            .last()
            .is_some_and(|last| last.command == Command::Seek)
        {
            let last_elem = sequence
                .elements
                .last_mut()
                .expect("Sequence::last() returned Some above");
            let crate::sequence::SequenceElementData::Movement {
                post_seek_sequence, ..
            } = &mut last_elem.data
            else {
                panic!("Seek sequence element does not contain movement data");
            };
            let mut continuation = post_seek_sequence
                .take()
                .map(crate::sequence::PostSeekSequence::into_sequence)
                .unwrap_or_default();
            self.append_posture_recovery(pc_id, &mut continuation);
            *post_seek_sequence = Some(continuation.into_post_seek());
            return;
        }

        let (level, last_command) = match sequence.last() {
            None => (1u16, None),
            Some(last) => (last.command_level.saturating_add(1), Some(last.command)),
        };

        // "Shoot once then stop".
        if last_command == Some(Command::ShootBow) && !action_state.is_bow() {
            if let Some(last_mut) = sequence.elements.last_mut() {
                last_mut.command = Command::ShootBowOnce;
            }
            return;
        }

        match posture {
            crate::element::Posture::Upright if action_state.is_bow() => {
                sequence.append_element(crate::sequence::SequenceElement::new(
                    level,
                    Command::EquipBow,
                    Some(pc_id),
                ));
            }
            crate::element::Posture::Crouched if last_command != Some(Command::CrouchUp) => {
                sequence.append_element(crate::sequence::SequenceElement::new(
                    level,
                    Command::CrouchDown,
                    Some(pc_id),
                ));
            }
            crate::element::Posture::HelpingToClimb => {
                sequence.append_element(crate::sequence::SequenceElement::new(
                    level,
                    Command::EnterHelpingClimb,
                    Some(pc_id),
                ));
            }
            crate::element::Posture::SimulatingBeggar => {
                sequence.append_element(crate::sequence::SequenceElement::new(
                    level,
                    Command::EnterBeggar,
                    Some(pc_id),
                ));
            }
            _ => {}
        }
    }
}
