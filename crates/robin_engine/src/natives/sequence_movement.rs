use super::*;

impl NativeContext<'_, '_> {
    /// Walks the gate path from `(source_sector, source)` to
    /// `(goal_sector, goal)` and appends the corresponding sub-elements
    /// to the active recording session (ASSERT_POSITION leader,
    /// per-gate approach + PASS_DOOR / JUMP / CHANGE_POSITION +
    /// post-pass ASSERT_POSITION, optional trailing MOVE).  Returns
    /// `false` when there is no path between the sectors or when
    /// called outside an active recording session; `true` otherwise
    /// (including the same-sector fast path).
    ///
    /// Side effects (seed `ASSERT_POSITION` against the source sector,
    /// choose move-after-last-door, raise `TO_JUMP` until past the
    /// first jump gate, lockpick short-circuit, `SEEK` building-interior
    /// trailing MOVE) are driven from script domains plus the canonical grid
    /// and entity owners borrowed by this native resume.
    ///
    /// `victim` is the SEEK target, passed straight through onto the
    /// trailing MOVE element's `element` field.
    pub(super) fn append_move_to_sequence(&mut self, request: SequenceMoveRequest) -> bool {
        let SequenceMoveRequest {
            actor_handle,
            action,
            source:
                SequenceMovePoint {
                    position: mut source,
                    sector: mut source_sector,
                    layer: mut source_layer,
                },
            goal:
                SequenceMovePoint {
                    position: goal,
                    sector: goal_sector,
                    layer: goal_layer,
                },
            victim,
            tolerance,
            initial_flags,
            speed_factor,
        } = request;
        use crate::element::Command;
        use crate::gate::{
            find_path_gates_with_sector_indices, find_path_into_door_with_sector_index,
        };
        use crate::position_interface::SectorHandle;
        use crate::sequence::{Field, FieldValue, MoveFlags, SequenceElement, SequenceElementData};

        debug_assert!(
            !initial_flags.contains(MoveFlags::STRAIGHT),
            "movement-sequence construction: STRAIGHT flag must be clear"
        );

        if self.script_state.sequence_recorder.is_none() {
            return false;
        }

        let owner = self.actor_id(actor_handle);
        let to_pt = |(x, y): (f32, f32)| crate::coordinates::MapPoint { x, y };

        // The original game rewrites the movement source when the
        // actor is currently straddling a gate, as reported by its target.
        // Do this before the same-sector fast path and path lookup so
        // recorded/script movement starts from the gate's far side.
        if let Some((door_handle, door_direction)) = self
            .get_entity(actor_handle)
            .and_then(crate::engine::current_door_for_route_source)
            && let Some((adapted_source, adapted_sector, adapted_layer)) =
                crate::engine::adapt_source_to_current_door_with_identity(
                    &self.script_domains.interactables.doors,
                    door_handle,
                    door_direction,
                )
        {
            source = (adapted_source.x, adapted_source.y);
            source_sector = adapted_sector;
            source_layer = adapted_layer;
        }

        source_sector = resolve_script_position_sector(
            &self.fast_grid.level,
            &self.script_domains.interactables.doors,
            source_sector,
            to_pt(source),
            source_layer,
        );
        let goal_sector = resolve_script_position_sector(
            &self.fast_grid.level,
            &self.script_domains.interactables.doors,
            goal_sector,
            to_pt(goal),
            goal_layer,
        );

        // The root emission uses the current recording level. All following
        // gate and trailing movement elements are recorded as child steps.

        // ── Same-sector fast path ──
        if script_sector_identities_match(source_sector, goal_sector) {
            let mut elem = SequenceElement::new_movement(0, Command::Move, owner, action);
            if let SequenceElementData::Movement {
                destination,
                element,
                tolerance: tol,
                flags,
                speed_factor: sf,
                layer,
                ..
            } = &mut elem.data
            {
                *destination = to_pt(goal);
                *element = victim;
                *tol = tolerance;
                *flags = initial_flags;
                *sf = speed_factor;
                *layer = goal_layer;
            }
            self.record_seq_step(elem, true);
            return true;
        }

        // ── Cross-sector ASSERT_POSITION leader ──
        let mut leader = SequenceElement::new_movement(0, Command::AssertPosition, owner, action);
        if let SequenceElementData::Movement {
            sector,
            element,
            speed_factor: sf,
            ..
        } = &mut leader.data
        {
            *sector = Some(source_sector);
            *element = owner;
            *sf = speed_factor;
        }
        self.record_seq_step(leader, true);

        // ── Find the gate path ──
        let auth = self.get_entity(actor_handle).map(|e| e.actor_auth_info());
        let allow_leave_map = initial_flags.contains(MoveFlags::MAP);
        let goal_is_door_sector = self
            .sector_kind_handle(goal_sector)
            .is_some_and(|sector| sector.sector_type.is_door());

        let path_opt = if goal_is_door_sector {
            self.door_index_for_goal_sector(goal_sector.get(), goal)
                .and_then(|door_idx| {
                    find_path_into_door_with_sector_index(
                        &self.script_domains.interactables.doors,
                        source,
                        source_sector.get(),
                        source_sector.arena_index(),
                        door_idx,
                        auth.as_ref(),
                        allow_leave_map,
                        &|sector| self.building_sector_is_authorized(sector),
                        &|sector| self.sector_lift_type(sector),
                    )
                })
        } else {
            find_path_gates_with_sector_indices(
                &self.script_domains.interactables.doors,
                source,
                source_sector.get(),
                source_sector.arena_index(),
                goal,
                goal_sector.get(),
                goal_sector.arena_index(),
                auth.as_ref(),
                allow_leave_map,
                &|sector| self.building_sector_is_authorized(sector),
                &|sector| self.sector_lift_type(sector),
            )
        };

        let Some(gate_steps) = path_opt else {
            // PC speaks HERO_UNABLE_TO_DO_SOMETHING and returns false.
            // The hero-speaking side effect requires engine-side state
            // (sound, hud); queue an EngineCommand so the engine fires
            // the bark on drain.
            if let Some(pc_id) = self
                .get_entity(actor_handle)
                .filter(|e| e.is_pc())
                .and_then(|_| self.actor_id(actor_handle))
            {
                self.emit_engine(EngineCommand::HeroSpeak {
                    pc_id,
                    expression: crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                });
            }
            tracing::debug!(
                actor = actor_handle,
                from_sector = source_sector.get(),
                to_sector = goal_sector.get(),
                "movement-sequence construction: no gate path"
            );
            return false;
        };

        let move_after_last_door = !goal_is_door_sector;

        // First-jump gate index — controls TO_JUMP flag.
        let first_jump = gate_steps.iter().enumerate().find_map(|(i, step)| {
            self.script_domains
                .interactables
                .doors
                .get(usize::from(step.door_index))
                .filter(|d| d.is_jump())
                .map(|_| i)
        });

        // Snapshot per-gate data into a local struct so the per-gate
        // emission loop can run without re-borrowing `self.script_domains.interactables.doors`.

        let gate_shots: Vec<GateShot> = gate_steps
            .iter()
            .map(|step| {
                let door = script_gate_path_door(&self.script_domains.interactables.doors, *step);
                let (entry, exit, entry_layer, exit_layer, new_sector_number, new_sector_index) =
                    if step.direct {
                        (
                            door.point_out,
                            door.point_in,
                            door.layer_out,
                            door.layer_in,
                            u16::from(door.sector_in),
                            door.sector_in_index,
                        )
                    } else {
                        (
                            door.point_in,
                            door.point_out,
                            door.layer_in,
                            door.layer_out,
                            u16::from(door.sector_out),
                            door.sector_out_index,
                        )
                    };
                let mut new_sector = SectorHandle::new(new_sector_number)
                    .expect("script-recorded door endpoint uses null sector sentinel");
                if let Some(index) = new_sector_index {
                    new_sector = new_sector.with_arena_index(index);
                }
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
                // Sequence handling keeps the caller's action on
                // gate approach, WAIT_FREE_LIFT, PASS_DOOR, and
                // post-pass asserts. Door-specific action-pair queries
                // are documented but inactive in the original game.
                let (entry_action, door_action) = (action, action);
                GateShot {
                    door_index: step.door_index,
                    direct: step.direct,
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
                }
            })
            .collect();

        let has_lockpick = self
            .get_entity(actor_handle)
            .map(|e| e.actor_auth_info().has_lockpick)
            .unwrap_or(false);

        // Track the "previous" sector so each gate emission knows
        // what it's coming *from*.  After the first gate this is the
        // previous gate's `new_sector`.
        let mut prev_sector = source_sector;

        // Snapshot of the recording size at entry — used to skip the
        // 50-frame wait on the first gate of a building-source
        // emission.
        let first_gate_size = self
            .script_state
            .sequence_recorder
            .as_ref()
            .expect("movement expansion requires the recording session checked at entry")
            .current_size();

        let mut ended_early = false;
        let mut last_new_sector = source_sector;

        let flags_at = |gate_idx: usize| -> MoveFlags {
            match first_jump {
                Some(j) if gate_idx <= j => initial_flags | MoveFlags::TO_JUMP,
                _ => initial_flags,
            }
        };

        let gate_context = GateEmissionContext {
            owner,
            victim,
            speed_factor,
            has_lockpick,
            first_gate_size,
        };
        for (gate_idx, shot) in gate_shots.iter().enumerate() {
            let complete =
                self.emit_gate_step(shot, prev_sector, flags_at(gate_idx), &gate_context);
            last_new_sector = shot.new_sector;
            if !complete {
                ended_early = true;
                break;
            }
            prev_sector = shot.new_sector;
        }

        // ── Trailing emission ──
        if !ended_early {
            let last_into_building = self
                .sector_kind_handle(last_new_sector)
                .is_some_and(|sector| sector.sector_type.is_building());

            // Trailing MOVE to the goal unless we landed inside a
            // building or `move_after_last_door=false`.
            if move_after_last_door && !last_into_building {
                let mut m = SequenceElement::new_movement(0, Command::Move, owner, action);
                if let SequenceElementData::Movement {
                    destination,
                    element,
                    tolerance: tol,
                    flags,
                    speed_factor: sf,
                    layer,
                    ..
                } = &mut m.data
                {
                    *destination = to_pt(goal);
                    *element = victim;
                    *tol = tolerance;
                    *flags = initial_flags;
                    *sf = speed_factor;
                    *layer = goal_layer;
                }
                self.record_seq_step(m, false);
            }

            // SEEK + last sector is building → trailing MOVE back to
            // the last gate's `point_in` so the seeker doesn't get
            // stuck at the interior teleport spot.
            if last_into_building
                && initial_flags.contains(MoveFlags::SEEK)
                && let Some(last_shot) = gate_shots.last()
            {
                let point_in = self
                    .script_domains
                    .interactables
                    .doors
                    .get(usize::from(last_shot.door_index))
                    .map(|d| d.point_in)
                    .unwrap_or(last_shot.exit);
                let mut m = SequenceElement::new_movement(0, Command::Move, owner, action);
                if let SequenceElementData::Movement {
                    destination,
                    element,
                    tolerance: tol,
                    flags,
                    speed_factor: sf,
                    layer,
                    ..
                } = &mut m.data
                {
                    *destination = point_in;
                    *element = victim;
                    *tol = tolerance;
                    *flags = initial_flags;
                    *sf = speed_factor;
                    *layer = goal_layer;
                }
                self.record_seq_step(m, false);
            }
        }

        true
    }
}

impl NativeContext<'_, '_> {
    /// Emit one ordered gate transition. False ends the route at lockpicking.
    fn emit_gate_step(
        &mut self,
        shot: &GateShot,
        prev_sector: crate::position_interface::SectorHandle,
        gate_flags: crate::sequence::MoveFlags,
        context: &GateEmissionContext,
    ) -> bool {
        use crate::element::Command;
        use crate::sequence::{Field, FieldValue, MoveFlags, SequenceElement, SequenceElementData};
        let GateEmissionContext {
            owner,
            victim,
            speed_factor,
            has_lockpick,
            first_gate_size,
        } = *context;

        // ── Gate approach ──
        //
        // The original game approaches every gate
        // before splitting into door handling or the jump command.
        let old_is_building = self
            .sector_kind_handle(prev_sector)
            .is_some_and(|sector| sector.sector_type.is_building());
        let entry_action = shot.entry_action;
        let door_action = shot.door_action;

        if old_is_building {
            let cur_size = self
                .script_state
                .sequence_recorder
                .as_ref()
                .expect("movement expansion requires the recording session checked at entry")
                .current_size();
            if cur_size != first_gate_size {
                let mut w = SequenceElement::new_generic(0, Command::WaitTimer, owner);
                w.set_property(Field::Timer, FieldValue::Integer(50));
                self.record_seq_step(w, false);
            }
            // Random 0..30: source uses `rand() & 15 + rand() & 15`.
            // Script recording receives the engine's explicit simulation
            // context, so this consumes the same deterministic stream as
            // runtime gate routing.
            let r: u32 = crate::sim_rng::u32(
                self.simulation,
                crate::sim_rng::RngSite::SequenceRecordingBuildingExitWait,
                0..16,
            ) + crate::sim_rng::u32(
                self.simulation,
                crate::sim_rng::RngSite::SequenceRecordingBuildingExitWait,
                0..16,
            );
            let mut w = SequenceElement::new_generic(0, Command::WaitTimer, owner);
            w.set_property(Field::Timer, FieldValue::Integer(r));
            self.record_seq_step(w, false);

            // CHANGE_POSITION teleport.
            let dx = shot.exit.x - shot.entry.x;
            let dy = shot.exit.y - shot.entry.y;
            let dir = crate::position_interface::vector_to_sector_0_to_15(dx, dy);
            let mut cp =
                SequenceElement::new_movement(0, Command::ChangePosition, owner, entry_action);
            if let SequenceElementData::Movement {
                destination,
                layer,
                sector,
                flags,
                direction,
                speed_factor: sf,
                ..
            } = &mut cp.data
            {
                *destination = shot.entry;
                *layer = shot.entry_layer;
                *sector = Some(prev_sector);
                *flags = gate_flags;
                *direction = dir;
                *sf = speed_factor;
            }
            self.record_seq_step(cp, false);
        } else {
            // MOVE to gate entry + ASSERT_POSITION.
            let mut m = SequenceElement::new_movement(0, Command::Move, owner, entry_action);
            if let SequenceElementData::Movement {
                destination,
                element,
                tolerance: tol,
                flags,
                speed_factor: sf,
                ..
            } = &mut m.data
            {
                *destination = shot.entry;
                *element = victim;
                *tol = 0.0;
                *flags = gate_flags;
                *sf = speed_factor;
            }
            self.record_seq_step(m, false);

            let mut ap =
                SequenceElement::new_movement(0, Command::AssertPosition, owner, entry_action);
            if let SequenceElementData::Movement {
                destination,
                element,
                tolerance: tol,
                speed_factor: sf,
                ..
            } = &mut ap.data
            {
                *destination = shot.entry;
                *element = owner;
                *tol = 10.0;
                *sf = speed_factor;
            }
            self.record_seq_step(ap, false);
        }

        if shot.is_jump {
            // ── Jump gate ──
            let (src, dst) = match (shot.jump_line_src, shot.jump_line_dst) {
                (Some(s), Some(d)) => (s, d),
                _ => {
                    tracing::warn!(
                        gate = %shot.door_index,
                        "Jump gate missing jump_line indices; skipping"
                    );
                    return true;
                }
            };
            let mut jump_elem = SequenceElement::new_generic(0, Command::JumpCmd, owner);
            jump_elem.set_property(Field::JumplineSource, FieldValue::LineId(src));
            jump_elem.set_property(Field::JumplineDestination, FieldValue::LineId(dst));
            self.record_seq_step(jump_elem, false);
            return true;
        }

        // ── Lockpick branch ──
        if shot.is_locked_pc_unlockable && has_lockpick {
            let cam_pt = if shot.direct { shot.exit } else { shot.entry };
            let mut turn = SequenceElement::new_generic(0, Command::Turn, owner);
            turn.set_property(
                Field::CameraPoint,
                FieldValue::GeoPoint2D {
                    x: cam_pt.x,
                    y: cam_pt.y,
                },
            );
            self.record_seq_step(turn, false);

            let mut unlock = SequenceElement::new_generic(0, Command::UnlockDoor, owner);
            unlock.set_property(Field::Door, FieldValue::DoorId(shot.door_index));
            self.record_seq_step(unlock, false);
            return false;
        }

        // ── Ladder-lift wait ──
        if self.sector_is_ladder_lift(shot.new_sector.get()) {
            let mut wait =
                SequenceElement::new_movement(0, Command::WaitFreeLift, owner, door_action);
            if let SequenceElementData::Movement {
                sector,
                gate_id,
                speed_factor: sf,
                ..
            } = &mut wait.data
            {
                *sector = Some(shot.new_sector);
                *gate_id = Some(shot.door_index);
                *sf = speed_factor;
            }
            self.record_seq_step(wait, false);
        }

        // ── PASS_DOOR ──
        let mut pass = SequenceElement::new_movement(0, Command::PassDoor, owner, door_action);
        if let SequenceElementData::Movement {
            destination,
            layer,
            gate_id,
            flags,
            direction,
            speed_factor: sf,
            ..
        } = &mut pass.data
        {
            *destination = shot.exit;
            *layer = shot.exit_layer;
            *gate_id = Some(shot.door_index);
            // Original-game door-passage initialization uses default flags
            // and only attaches the gate. Gate assignment preserves
            // the path-local direct-gate value in the stored direction;
            // AI position reads it while the
            // PassDoor is selected.
            *flags = MoveFlags::empty();
            *direction = i16::from(shot.direct);
            *sf = speed_factor;
        }
        self.record_seq_step(pass, false);

        // ── ASSERT post-pass ──
        let mut ap = SequenceElement::new_movement(0, Command::AssertPosition, owner, door_action);
        if let SequenceElementData::Movement {
            destination,
            element,
            tolerance: tol,
            speed_factor: sf,
            ..
        } = &mut ap.data
        {
            *destination = shot.exit;
            *element = owner;
            *tol = 10.0;
            *sf = speed_factor;
        }
        self.record_seq_step(ap, false);

        true
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct SequenceMovePoint {
    pub(super) position: (f32, f32),
    pub(super) sector: crate::position_interface::SectorHandle,
    pub(super) layer: u16,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct SequenceMoveRequest {
    pub(super) actor_handle: i32,
    pub(super) action: OrderType,
    pub(super) source: SequenceMovePoint,
    pub(super) goal: SequenceMovePoint,
    pub(super) victim: Option<EntityId>,
    pub(super) tolerance: f32,
    pub(super) initial_flags: crate::sequence::MoveFlags,
    pub(super) speed_factor: f32,
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct GateEmissionContext {
    owner: Option<EntityId>,
    victim: Option<EntityId>,
    speed_factor: f32,
    has_lockpick: bool,
    first_gate_size: usize,
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct GateShot {
    door_index: crate::gate::DoorIndex,
    direct: bool,
    entry: crate::coordinates::MapPoint,
    exit: crate::coordinates::MapPoint,
    entry_layer: u16,
    exit_layer: u16,
    new_sector: crate::position_interface::SectorHandle,
    is_jump: bool,
    jump_line_src: Option<crate::jump_line::JumpLineIndex>,
    jump_line_dst: Option<crate::jump_line::JumpLineIndex>,
    is_locked_pc_unlockable: bool,
    entry_action: OrderType,
    door_action: OrderType,
}
