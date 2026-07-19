//! Immediate presentation commands and recorded sequence construction.

use super::*;

impl NativeContext<'_, '_> {
    pub(super) fn dispatch_sequences(&mut self, native: NativeFn, stack: &mut NativeStack) -> i32 {
        use NativeFn::*;

        match native {
            // --- camera / UI ---
            ScrollCameraTo => {
                let loc = stack.pop_i32();
                if Self::check_camera_location(loc, "ScrollCameraTo") {
                    self.engine.commands.push(EngineCommand::ScrollCameraTo {
                        location_handle: loc,
                        speed: 2.0,
                    });
                }
                0
            }
            ScrollCameraSlowlyTo => {
                let speed = f32::from_bits(stack.pop_i32() as u32);
                let loc = stack.pop_i32();
                if Self::check_camera_location(loc, "ScrollCameraSlowlyTo") {
                    self.engine.commands.push(EngineCommand::ScrollCameraTo {
                        location_handle: loc,
                        speed,
                    });
                }
                0
            }
            JumpCameraTo => {
                let loc = stack.pop_i32();
                if Self::check_camera_location(loc, "JumpCameraTo") {
                    self.engine.commands.push(EngineCommand::JumpCameraTo {
                        location_handle: loc,
                    });
                }
                0
            }
            SetZoomLevel => {
                let zoom_bits = stack.pop_i32();
                let zoom = f32::from_bits(zoom_bits as u32);
                if zoom != 0.5 && zoom != 1.0 && zoom != 2.0 {
                    tracing::warn!("Script Error: SetZoomLevel with invalid zoom {zoom}");
                } else {
                    self.engine
                        .commands
                        .push(EngineCommand::SetZoomLevel { zoom });
                }
                0
            }
            StartDialog => {
                let dialog_id = stack.pop_i32();
                self.engine
                    .commands
                    .push(EngineCommand::StartDialog { dialog_id });
                0
            }
            DisplayMap => {
                let show = stack.pop_i32();
                self.engine
                    .commands
                    .push(EngineCommand::DisplayMap { show: show != 0 });
                0
            }
            DisplayConsole => {
                self.engine.commands.push(EngineCommand::DisplayConsole);
                0
            }
            CustomizeMinimapDisplay => {
                let dot_type = stack.pop_i32();
                let actor_handle = stack.pop_i32();
                if actor_handle == 0 {
                    tracing::warn!("Script Error: CustomizeMinimapDisplay called with NULL actor");
                } else {
                    self.engine
                        .commands
                        .push(EngineCommand::CustomizeMinimapDisplay {
                            actor_handle,
                            dot_type,
                        });
                }
                0
            }
            DefineFlatTrajectoryZone => {
                let apex_height = stack.pop_i32();
                let location_handle = stack.pop_i32();
                self.engine
                    .commands
                    .push(EngineCommand::DefineFlatTrajectoryZone {
                        location_handle,
                        apex_height,
                    });
                0
            }
            AddShortBriefing => {
                let primary = stack.pop_i32();
                let id = stack.pop_i32();
                self.short_briefings
                    .as_mut()
                    .expect("AddShortBriefing requires live mission objectives")
                    .add(id as u32, primary != 0);
                0
            }
            DoneShortBriefing => {
                let id = stack.pop_i32();
                self.short_briefings
                    .as_mut()
                    .expect("DoneShortBriefing requires live mission objectives")
                    .mark_done(id as u32);
                0
            }
            ChooseVictoryDefeatText => {
                let id = stack.pop_i32();
                self.engine
                    .commands
                    .push(EngineCommand::ChooseVictoryDefeatText { id });
                0
            }
            DisplayPopupText => {
                let text_id = stack.pop_i32();
                self.engine
                    .commands
                    .push(EngineCommand::DisplayPopupText { text_id });
                0
            }
            DisplaySherwoodReport => {
                self.engine
                    .commands
                    .push(EngineCommand::DisplaySherwoodReport);
                0
            }
            FadeToBlack => {
                let speed = stack.pop_i32();
                self.engine
                    .commands
                    .push(EngineCommand::FadeToBlack { speed });
                0
            }
            SetOutlineDisplay => {
                let val = stack.pop_i32();
                let display = val != 0;
                if self.script_domains.mission_ui.outline_display != display {
                    self.script_domains.mission_ui.outline_display = display;
                    self.engine
                        .commands
                        .push(EngineCommand::SetOutlineDisplay { display });
                }
                0
            }
            GetOutlineDisplay => {
                if self.script_domains.mission_ui.outline_display {
                    1
                } else {
                    0
                }
            }
            SetViewRadius => {
                // C++ passes the script `int` to `SetStandardViewRadius(UWORD)`,
                // so retain the original narrowing conversion exactly.
                let radius = stack.pop_i32() as u16;
                **self
                    .standard_view_radius
                    .as_mut()
                    .expect("SetViewRadius requires live AI radius state") = radius;
                for (_, entity) in self.entities.npcs_mut() {
                    let npc = entity
                        .npc_data_mut()
                        .expect("NPC iterator yielded an entity without NPC data");
                    npc.view_radius_base = radius;
                    npc.view_radius_goal = radius;
                    npc.view_radius = radius;
                }
                0
            }
            PlayTrapJingle => {
                self.external.sound.push(SoundCommand::PlayJingle(
                    crate::sound::Jingle::TrapTriggered,
                ));
                0
            }

            // ═══════════════════════════════════════════════════════
            // Record / sequence — each creates a SequenceElement and
            // appends it to the current RecordingSession.
            // ═══════════════════════════════════════════════════════

            // --- Camera ---
            RecordScrollCameraTo => {
                let loc = stack.pop_i32();
                if !self.is_script_point(loc) {
                    tracing::warn!(
                        "Script Error: RecordScrollCameraTo wrong kind of location (handle {loc})"
                    );
                    return 0;
                }
                let (x, y) = match self.resolve_location_pos(loc) {
                    Some(p) => p,
                    None => {
                        tracing::warn!(
                            "Script Error: RecordScrollCameraTo unresolved location {loc}"
                        );
                        return 0;
                    }
                };
                let level = self.recording_level();
                let mut elem = SequenceElement::new_generic(level, Command::CameraGoto, None);
                elem.set_property(Field::CameraPoint, FieldValue::GeoPoint2D { x, y });
                // The CameraSpeed field must be an Integer (the
                // engine reader in tick.rs only accepts Integer);
                // a literal 0 means "default speed".
                elem.set_property(Field::CameraSpeed, FieldValue::Integer(0));
                self.record_element(elem)
            }
            RecordJumpCameraTo => {
                let loc = stack.pop_i32();
                if !self.is_script_point(loc) {
                    tracing::warn!(
                        "Script Error: RecordJumpCameraTo wrong kind of location (handle {loc})"
                    );
                    return 0;
                }
                let (x, y) = match self.resolve_location_pos(loc) {
                    Some(p) => p,
                    None => {
                        tracing::warn!(
                            "Script Error: RecordJumpCameraTo unresolved location {loc}"
                        );
                        return 0;
                    }
                };
                let level = self.recording_level();
                let mut elem = SequenceElement::new_generic(level, Command::CameraJumpTo, None);
                elem.set_property(Field::CameraPoint, FieldValue::GeoPoint2D { x, y });
                self.record_element(elem)
            }
            RecordSetZoom => {
                let zoom = stack.pop_i32();
                let zoom_f = f32::from_bits(zoom as u32);
                // Reject anything but 0.5 / 1.0 / 2.0.
                if zoom_f != 0.5 && zoom_f != 1.0 && zoom_f != 2.0 {
                    tracing::warn!(
                        "Script Error: Wanted zoom level is incorrect in RecordSetZoom (got {zoom_f})"
                    );
                    return 0;
                }
                let level = self.recording_level();
                let mut elem = SequenceElement::new_generic(level, Command::ZoomLevel, None);
                elem.set_property(Field::CameraZoomLevel, FieldValue::Float(zoom_f));
                self.record_element(elem)
            }
            RecordDisplayMap => {
                let show = stack.pop_i32();
                let level = self.recording_level();
                let mut elem = SequenceElement::new_generic(level, Command::DisplayMap, None);
                elem.set_property(Field::MapDisplay, FieldValue::Bool(show != 0));
                self.record_element(elem)
            }
            RecordMoveCameraTo => {
                let speed = stack.pop_i32();
                let loc = stack.pop_i32();
                if !self.is_script_point(loc) {
                    tracing::warn!(
                        "Script Error: RecordMoveCameraTo wrong kind of location (handle {loc})"
                    );
                    return 0;
                }
                let (x, y) = match self.resolve_location_pos(loc) {
                    Some(p) => p,
                    None => {
                        tracing::warn!(
                            "Script Error: RecordMoveCameraTo unresolved location {loc}"
                        );
                        return 0;
                    }
                };
                let level = self.recording_level();
                let mut elem = SequenceElement::new_generic(level, Command::CameraGoto, None);
                elem.set_property(Field::CameraPoint, FieldValue::GeoPoint2D { x, y });
                // CameraSpeed must be an Integer (the engine
                // reader in tick.rs only unwraps Integer).
                elem.set_property(Field::CameraSpeed, FieldValue::Integer(speed as u32));
                self.record_element(elem)
            }
            RecordLockCameraOn => {
                let actor = stack.pop_i32();
                // Reject non-actor handles with a warning + return 0.
                if !self.get_entity(actor).is_some_and(|e| e.is_actor()) {
                    tracing::warn!(
                        "Script Error: RecordLockCameraOn on illegal actor handle {actor}"
                    );
                    return 0;
                }
                let level = self.recording_level();
                // Interaction element with actor as antagonist, no owner.
                let elem = SequenceElement::new_interaction(
                    level,
                    Command::LockCameraOn,
                    None,
                    self.actor_id(actor),
                );
                self.record_element(elem)
            }
            RecordClearCameraLock => {
                let level = self.recording_level();
                let elem = SequenceElement::new(level, Command::LockCameraStop, None);
                self.record_element(elem)
            }

            // --- Dialog / UI ---
            RecordPlayDialog => {
                let dialog_id = stack.pop_i32();
                let level = self.recording_level();
                let mut elem = SequenceElement::new_generic(level, Command::PlayDialog, None);
                elem.set_property(Field::DialogId, FieldValue::Integer(dialog_id as u32));
                self.record_element(elem)
            }
            RecordDisplayPopupText => {
                let text_id = stack.pop_i32();
                let level = self.recording_level();
                let mut elem = SequenceElement::new_generic(level, Command::DisplayPopupText, None);
                elem.set_property(Field::PopupTextId, FieldValue::Integer(text_id as u32));
                self.record_element(elem)
            }

            // --- Action / character availability ---
            RecordActionAvailable => {
                let available = stack.pop_i32();
                let action_id = stack.pop_i32();
                let actor = stack.pop_i32();
                // Reject non-actor handles before recording.
                if !self.get_entity(actor).is_some_and(|e| e.is_actor()) {
                    tracing::warn!(
                        "Script Error: RecordActionAvailable on illegal actor handle {actor}"
                    );
                    return 0;
                }
                let level = self.recording_level();
                let mut elem = SequenceElement::new_generic(
                    level,
                    Command::ActionAvailable,
                    self.actor_id(actor),
                );
                elem.set_property(Field::ActionId, FieldValue::Integer(action_id as u32));
                elem.set_property(Field::ActionAvailable, FieldValue::Bool(available != 0));
                self.record_element(elem)
            }
            RecordCharacterAvailable => {
                let available = stack.pop_i32();
                let actor = stack.pop_i32();
                // Reject non-actor handles.
                if !self.get_entity(actor).is_some_and(|e| e.is_actor()) {
                    tracing::warn!(
                        "Script Error: RecordCharacterAvailable on illegal actor handle {actor}"
                    );
                    return 0;
                }
                let level = self.recording_level();
                let mut elem = SequenceElement::new_generic(
                    level,
                    Command::CharacterAvailable,
                    self.actor_id(actor),
                );
                elem.set_property(Field::CharacterAvailable, FieldValue::Bool(available != 0));
                self.record_element(elem)
            }

            // --- Messages ---
            RecordSendMessage => {
                let msg = stack.pop_i32();
                let actor = stack.pop_i32();
                // Reject non-actor, non-null handles with a
                // warning and no record.
                if actor != 0 && !self.is_actor_handle(actor) {
                    tracing::error!("Script Error : trying to send a message to non actor object.");
                    return 0;
                }
                let level = self.recording_level();
                let mut elem =
                    SequenceElement::new_generic(level, Command::SendMessage, self.actor_id(actor));
                elem.set_property(Field::Message, FieldValue::Integer(msg as u32));
                elem.set_property(Field::MessageArgument, FieldValue::Integer(0));
                elem.set_property(Field::MessageExtendedArgument, FieldValue::Integer(0));
                self.record_element(elem)
            }
            RecordSendMessageWithArguments => {
                let arg2 = stack.pop_i32();
                let arg1 = stack.pop_i32();
                let msg = stack.pop_i32();
                let actor = stack.pop_i32();
                // Same IsActor guard as RecordSendMessage.
                if actor != 0 && !self.is_actor_handle(actor) {
                    tracing::error!("Script Error : trying to send a message to non actor object.");
                    return 0;
                }
                let level = self.recording_level();
                let mut elem =
                    SequenceElement::new_generic(level, Command::SendMessage, self.actor_id(actor));
                elem.set_property(Field::Message, FieldValue::Integer(msg as u32));
                elem.set_property(Field::MessageArgument, FieldValue::Integer(arg1 as u32));
                elem.set_property(
                    Field::MessageExtendedArgument,
                    FieldValue::Integer(arg2 as u32),
                );
                self.record_element(elem)
            }

            // --- Movement ---
            RecordMove => {
                let style = stack.pop_i32();
                let loc = stack.pop_i32();
                let actor = stack.pop_i32();
                // Reject null actor, non-actor handle, null /
                // non-Point location, and any style outside 0..=3.
                if !self.is_actor_handle(actor) {
                    tracing::error!("Script Error in RecordMove: invalid actor handle {actor}");
                    return 0;
                }
                let Some((dx, dy)) = self.resolve_location_pos(loc) else {
                    tracing::error!(
                        "Script Error in RecordMove: illegal location handle {loc} (null or not a Point)"
                    );
                    return 0;
                };
                if !(0..=3).contains(&style) {
                    tracing::error!("Script Error in RecordMove: illegal movement style {style}");
                    return 0;
                }
                let dest_layer_sector = self.resolve_location_layer_sector(loc);
                // Chained Record* moves for the same actor start
                // from the previous target, not the actor's live
                // position.
                let origin = self.update_motion_start_position(actor, (dx, dy), dest_layer_sector);
                let action = Self::movement_style(style);
                let pre_record_size = self
                    .script_state
                    .sequence_recorder
                    .recording
                    .as_ref()
                    .map(|r| r.current_size())
                    .unwrap_or(0);
                // Expand the move into the sequence.
                let (goal_layer, goal_sector) = dest_layer_sector.unwrap_or((0, 0));
                let (sx, sy, src_layer, src_sector) =
                    origin.unwrap_or((dx, dy, goal_layer, goal_sector));
                self.append_move_to_sequence(
                    actor,
                    action,
                    (sx, sy),
                    src_sector,
                    src_layer,
                    (dx, dy),
                    goal_sector,
                    goal_layer,
                    None,
                    0.0,
                    MoveFlags::CALLED_BY_SCRIPT,
                    1.0,
                );
                // NONINTERRUPTABLE walks bump every just-added
                // element to Script priority.
                if matches!(style, 2 | 3)
                    && let Some(rec) = self.script_state.sequence_recorder.recording.as_mut()
                {
                    rec.bump_priority_from(
                        pre_record_size,
                        crate::sequence::SequencePriority::Script,
                    );
                }
                1
            }
            RecordMoveNear => {
                let tolerance = stack.pop_i32();
                let style = stack.pop_i32();
                let loc = stack.pop_i32();
                let actor = stack.pop_i32();
                // Same validation as RecordMove (we additionally
                // explicitly reject null actor handles, which the
                // legacy implementation would dereference).
                if !self.is_actor_handle(actor) {
                    tracing::error!("Script Error in RecordMoveNear: invalid actor handle {actor}");
                    return 0;
                }
                let Some((dx, dy)) = self.resolve_location_pos(loc) else {
                    tracing::error!(
                        "Script Error in RecordMoveNear: illegal location handle {loc} (null or not a Point)"
                    );
                    return 0;
                };
                if !(0..=3).contains(&style) {
                    tracing::error!(
                        "Script Error in RecordMoveNear: illegal movement style {style}"
                    );
                    return 0;
                }
                let dest_layer_sector = self.resolve_location_layer_sector(loc);
                let origin = self.update_motion_start_position(actor, (dx, dy), dest_layer_sector);
                let action = Self::movement_style(style);
                let pre_record_size = self
                    .script_state
                    .sequence_recorder
                    .recording
                    .as_ref()
                    .map(|r| r.current_size())
                    .unwrap_or(0);
                let (goal_layer, goal_sector) = dest_layer_sector.unwrap_or((0, 0));
                let (sx, sy, src_layer, src_sector) =
                    origin.unwrap_or((dx, dy, goal_layer, goal_sector));
                self.append_move_to_sequence(
                    actor,
                    action,
                    (sx, sy),
                    src_sector,
                    src_layer,
                    (dx, dy),
                    goal_sector,
                    goal_layer,
                    None,
                    tolerance as f32,
                    MoveFlags::CALLED_BY_SCRIPT,
                    1.0,
                );
                // NONINTERRUPTABLE near-walks bump every
                // just-added element to Preference priority (one
                // rung weaker than RecordMove's Script).
                if matches!(style, 2 | 3)
                    && let Some(rec) = self.script_state.sequence_recorder.recording.as_mut()
                {
                    rec.bump_priority_from(
                        pre_record_size,
                        crate::sequence::SequencePriority::Preference,
                    );
                }
                1
            }
            RecordMoveIntoBuilding => {
                // Validate the location is a Point, find the
                // nearest door within 300px of it, then synthesise
                // a point at the door's
                // (PointIn, LayerIn, SectorIn) and tail-call RecordMove.
                let style = stack.pop_i32();
                let loc = stack.pop_i32();
                let actor = stack.pop_i32();
                if !self.is_actor_handle(actor) {
                    tracing::error!(
                        "Script Error in RecordMoveIntoBuilding: invalid actor handle {actor}"
                    );
                    return 0;
                }
                let Some((lx, ly)) = self.resolve_location_pos(loc) else {
                    tracing::error!(
                        "Script Error in RecordMoveIntoBuilding: illegal location handle {loc} (null or not a Point)"
                    );
                    return 0;
                };
                if !(0..=3).contains(&style) {
                    tracing::error!(
                        "Script Error in RecordMoveIntoBuilding: illegal movement style {style}"
                    );
                    return 0;
                }

                // Find nearest door whose mid-point is within
                // 300px of the target; if none, return 0.
                let max_sq_dist = 300.0_f32 * 300.0;
                let mut best: Option<(f32, f32, f32, u16, u16)> = None;
                for door in &self.script_domains.interactables.doors {
                    let ddx = door.point_mid.x - lx;
                    let ddy = door.point_mid.y - ly;
                    let sq = ddx * ddx + ddy * ddy;
                    if sq < max_sq_dist && (best.is_none() || sq < best.unwrap().0) {
                        best = Some((
                            sq,
                            door.point_in.x,
                            door.point_in.y,
                            door.layer_in,
                            door.sector_in.0 as u16,
                        ));
                    }
                }
                let Some((_, ix, iy, door_layer, door_sector)) = best else {
                    tracing::error!(
                        "Script Error in RecordMoveIntoBuilding: no door within 300px of ({lx}, {ly})"
                    );
                    return 0;
                };

                // The original tail-calls RecordMove with a
                // synthesised point at the door's interior.  The
                // tail-call also runs `update_motion_start_position`
                // on the actor, so do that here too — chained
                // Record* see the door's interior as the new
                // motion target.
                let origin = self.update_motion_start_position(
                    actor,
                    (ix, iy),
                    Some((door_layer, door_sector)),
                );
                let action = Self::movement_style(style);
                let pre_record_size = self
                    .script_state
                    .sequence_recorder
                    .recording
                    .as_ref()
                    .map(|r| r.current_size())
                    .unwrap_or(0);
                // Drive the inner RecordMove tail call's
                // `append_move_to_sequence`.  The goal point uses
                // the door's interior (layer, sector).
                let (sx, sy, src_layer, src_sector) =
                    origin.unwrap_or((ix, iy, door_layer, door_sector));
                self.append_move_to_sequence(
                    actor,
                    action,
                    (sx, sy),
                    src_sector,
                    src_layer,
                    (ix, iy),
                    door_sector,
                    door_layer,
                    None,
                    0.0,
                    MoveFlags::CALLED_BY_SCRIPT,
                    1.0,
                );
                // Apply the same NONINTERRUPTABLE bump the inner
                // RecordMove would apply.
                if matches!(style, 2 | 3)
                    && let Some(rec) = self.script_state.sequence_recorder.recording.as_mut()
                {
                    rec.bump_priority_from(
                        pre_record_size,
                        crate::sequence::SequencePriority::Script,
                    );
                }
                1
            }
            RecordEnterGame => {
                // Immediately teleports the actor to a point just
                // outside the map edge (opposite of its facing
                // direction relative to the destination), then
                // records a single movement element from the
                // outside spawn point to the destination.
                let style = stack.pop_i32();
                let direction = stack.pop_i32();
                let loc = stack.pop_i32();
                let actor = stack.pop_i32();

                if !Self::validate_style(style, "RecordEnterGame") {
                    return 0;
                }
                if !self.actor_exists(actor) {
                    tracing::warn!("RecordEnterGame: invalid actor handle {actor}");
                    return 0;
                }
                let Some((dx, dy)) = self.resolve_location_pos(loc) else {
                    tracing::warn!("RecordEnterGame: illegal location handle {loc} (not a Point)");
                    return 0;
                };
                // Read layer + sector from the destination point
                // and apply them to the teleported actor.  Static
                // script locations carry that data; computed ones
                // do not, so we leave layer/sector untouched in
                // that case.
                let dest_layer_sector = self.resolve_location_layer_sector(loc);

                // `direction == -1` means "use actor's current direction".
                let actor_dir = self
                    .get_entity(actor)
                    .map(|e| e.element_data().direction())
                    .unwrap_or(0);
                let effective_dir = if direction == -1 {
                    actor_dir
                } else {
                    direction as i16
                };

                let (_border, (ox, oy)) = self.compute_border_point((dx, dy), effective_dir);

                // If the actor is already in `moving_actors`,
                // just refresh the cached destination and skip
                // the teleport / outside-spawn block.  Only the
                // first EnterGame call in a recording session
                // teleports the actor outside the map; subsequent
                // calls for the same actor only update the
                // bookkeeping target.
                let already_moving = self
                    .script_state
                    .sequence_recorder
                    .recording
                    .as_ref()
                    .is_some_and(|r| r.moving_actors.contains_key(&actor));

                if !already_moving {
                    // Immediate teleport: write position + layer
                    // + sector on the actor.  The native can only
                    // touch the `ElementData` copy — the 3D spawn
                    // elevation at `(ox, oy)` comes from the
                    // destination's projection-area top plane and
                    // needs fast-grid access, so that composition
                    // happens inside the queued
                    // `SetActorLocation` command below.  Here we
                    // only write the 2D spawn position so the VM
                    // observes the actor as outside the map on
                    // the next script read.  Also handles the
                    // `in_honolulu` wake-up case.
                    if let Some(entity) = self.get_entity_mut(actor) {
                        let ed = entity.element_data_mut();
                        if ed.in_honolulu {
                            ed.active = true;
                            ed.in_honolulu = false;
                        }
                        ed.set_position_map(crate::coordinates::MapPoint { x: ox, y: oy });
                        if let Some((layer, sector_num)) = dest_layer_sector {
                            ed.set_layer(layer);
                            ed.set_sector(crate::position_interface::SectorHandle::new(sector_num));
                        }
                        ed.update_grid_cell();
                    } else {
                        tracing::warn!("RecordEnterGame: invalid actor handle {actor}");
                        return 0;
                    }
                    self.engine.commands.push(EngineCommand::SetActorLocation {
                        actor_handle: actor,
                        x: ox,
                        y: oy,
                        dest_layer_sector,
                        // The engine handler probes the
                        // destination sector's top plane at
                        // `(dx, dy)` and stamps
                        // `(ox, oy + elev, elev)` as the 3D
                        // spawn — so the actor walks in at the
                        // same altitude as the destination's
                        // ground slope.
                        spawn_elevation_probe: Some((dx, dy)),
                    });
                }

                // Always refresh the cached destination on both
                // the insert and update paths.
                if let Some(rec) = self.script_state.sequence_recorder.recording.as_mut() {
                    let (layer, sector) = dest_layer_sector.unwrap_or((0, 0));
                    rec.moving_actors.insert(
                        actor,
                        crate::sequence::RecordingMotionTarget {
                            x: dx,
                            y: dy,
                            layer,
                            sector,
                        },
                    );
                }

                let level = self.recording_level();
                let action = Self::movement_style(style);
                let mut elem = SequenceElement::new_movement(
                    level,
                    Command::Move,
                    self.actor_id(actor),
                    action,
                );
                if let crate::sequence::SequenceElementData::Movement {
                    destination, flags, ..
                } = &mut elem.data
                {
                    destination.x = dx;
                    destination.y = dy;
                    *flags |= MoveFlags::CALLED_BY_SCRIPT | MoveFlags::MAP;
                    // No direction is passed — the enter-game walk
                    // computes it from the movement vector when
                    // dispatched.
                }
                let _ = self.record_element(elem);
                // Returns `false` (0) unconditionally; scripts
                // don't observe it.
                0
            }
            RecordLeaveGame => {
                // Records two sequential movement elements: first
                // a normal walk to the script point, then a
                // straight-line walk from that point to a spot
                // just outside the map edge (in the direction the
                // actor is heading).
                let style = stack.pop_i32();
                let direction = stack.pop_i32();
                let loc = stack.pop_i32();
                let actor = stack.pop_i32();

                if !Self::validate_style(style, "RecordLeaveGame") {
                    return 0;
                }
                if !self.actor_exists(actor) {
                    tracing::warn!("RecordLeaveGame: invalid actor handle {actor}");
                    return 0;
                }
                let Some((dx, dy)) = self.resolve_location_pos(loc) else {
                    tracing::warn!("RecordLeaveGame: illegal location handle {loc} (not a Point)");
                    return 0;
                };

                let actor_dir = self
                    .get_entity(actor)
                    .map(|e| e.element_data().direction())
                    .unwrap_or(0);
                let effective_dir = if direction == -1 {
                    actor_dir
                } else {
                    direction as i16
                };

                // `compute_border_point` wants the *opposite* of
                // the travel direction, so the exit edge is the
                // one the actor is walking towards — pass
                // `(direction + 8) & 15`.
                let opposite_dir = (effective_dir + 8) & 15;
                let (_border, (ox, oy)) = self.compute_border_point((dx, dy), opposite_dir);

                let action = Self::movement_style(style);

                // When `RecordEnterGame` already recorded a
                // destination for this actor in this session,
                // that destination becomes the *origin* of the
                // leave walk (not the actor's live position,
                // which the EnterGame teleport pinned to the
                // *outside* spawn point).
                let dest_layer_sector = self.resolve_location_layer_sector(loc);
                let origin = self.update_motion_start_position(actor, (dx, dy), dest_layer_sector);

                // Step 1: append_move_to_sequence(origin → script
                // point).  Cross-sector traversals expand into
                // ASSERT_POSITION + per-gate sub-elements;
                // same-sector goal collapses to a single MOVE.
                let (goal_layer, goal_sector) = dest_layer_sector.unwrap_or((0, 0));
                let (sx, sy, src_layer, src_sector) =
                    origin.unwrap_or((dx, dy, goal_layer, goal_sector));
                self.append_move_to_sequence(
                    actor,
                    action,
                    (sx, sy),
                    src_sector,
                    src_layer,
                    (dx, dy),
                    goal_sector,
                    goal_layer,
                    None,
                    0.0,
                    MoveFlags::CALLED_BY_SCRIPT,
                    1.0,
                );

                // Insert the two moves at adjacent sequence
                // levels so they execute sequentially rather
                // than concurrently.
                if let Some(rec) = self.script_state.sequence_recorder.recording.as_mut() {
                    rec.advance_level();
                }

                // Step 2: straight MOVE to the off-map exit
                // point (with the MAP flag).  This is a single
                // element, not a gate-expanded path.
                let level2 = self.recording_level();
                let mut elem2 = SequenceElement::new_movement(
                    level2,
                    Command::Move,
                    self.actor_id(actor),
                    action,
                );
                if let crate::sequence::SequenceElementData::Movement {
                    destination, flags, ..
                } = &mut elem2.data
                {
                    destination.x = ox;
                    destination.y = oy;
                    *flags |= MoveFlags::CALLED_BY_SCRIPT | MoveFlags::MAP;
                    // No direction is passed — the off-map leave
                    // walk derives facing from the move vector
                    // when dispatched.
                }
                let _ = self.record_element(elem2);
                // Returns `true` (1) unconditionally; scripts
                // don't observe the value.
                1
            }

            // --- Turn ---
            RecordTurnTo => {
                let loc = stack.pop_i32();
                let actor = stack.pop_i32();
                // Two hard preconditions: target must be an actor
                // and the location must be a point.  Reject
                // either miss with `false` rather than stashing
                // a raw integer under `CameraPoint`.
                if !self.is_actor_handle(actor) {
                    tracing::warn!("RecordTurnTo: illegal actor handle {actor}");
                    return 0;
                }
                let Some((x, y)) = self.resolve_location_pos(loc) else {
                    tracing::warn!("RecordTurnTo: illegal location handle {loc}");
                    return 0;
                };
                let level = self.recording_level();
                let mut elem =
                    SequenceElement::new_generic(level, Command::Turn, self.actor_id(actor));
                elem.set_property(Field::CameraPoint, FieldValue::GeoPoint2D { x, y });
                self.record_element(elem)
            }

            // --- Animation ---
            RecordPlayAnim => {
                let anim = stack.pop_i32();
                let actor = stack.pop_i32();
                // Reject null handles and anything that is
                // neither an actor nor an FX target.
                if actor == 0 || !self.is_actor_or_fx_target(actor) {
                    tracing::warn!(
                        "RecordPlayAnim: illegal actor handle {actor} (not actor/fx-target)"
                    );
                    return 0;
                }
                let level = self.recording_level();
                let mut elem =
                    SequenceElement::new_generic(level, Command::PlayAnim, self.actor_id(actor));
                elem.set_property(
                    Field::AnimationId,
                    FieldValue::Animation(anim_ordinal_to_order_type(anim, "RecordPlayAnim")),
                );
                self.record_element(elem)
            }
            RecordPlayAnimLoop => {
                let anim = stack.pop_i32();
                let actor = stack.pop_i32();
                // Uses the full ActorExists validator (no
                // FX-target branch like its siblings) before
                // constructing the element.
                if actor == 0 || !self.actor_exists(actor) {
                    tracing::warn!("RecordPlayAnimLoop: invalid actor handle {actor}");
                    return 0;
                }
                let level = self.recording_level();
                let mut elem = SequenceElement::new_generic(
                    level,
                    Command::PlayAnimLoop,
                    self.actor_id(actor),
                );
                elem.set_property(
                    Field::AnimationId,
                    FieldValue::Animation(anim_ordinal_to_order_type(anim, "RecordPlayAnimLoop")),
                );
                self.record_element(elem)
            }
            RecordPlayAnimFreeze => {
                let anim = stack.pop_i32();
                let actor = stack.pop_i32();
                // Omits the null-handle check but still requires
                // actor-or-FX-target.
                if !self.is_actor_or_fx_target(actor) {
                    tracing::warn!(
                        "RecordPlayAnimFreeze: illegal actor handle {actor} (not actor/fx-target)"
                    );
                    return 0;
                }
                let level = self.recording_level();
                let mut elem = SequenceElement::new_generic(
                    level,
                    Command::PlayAnimFreeze,
                    self.actor_id(actor),
                );
                elem.set_property(
                    Field::AnimationId,
                    FieldValue::Animation(anim_ordinal_to_order_type(anim, "RecordPlayAnimFreeze")),
                );
                self.record_element(elem)
            }
            RecordReplaceAnim => {
                let new_anim = stack.pop_i32();
                let old_anim = stack.pop_i32();
                let actor = stack.pop_i32();
                // Gates on `ActorExists && IsActor`.
                if !self.is_actor_handle(actor) {
                    tracing::warn!("RecordReplaceAnim: illegal actor handle {actor}");
                    return 0;
                }
                let level = self.recording_level();
                let mut elem =
                    SequenceElement::new_generic(level, Command::ReplaceAnim, self.actor_id(actor));
                elem.set_property(Field::OldAnimation, FieldValue::Integer(old_anim as u32));
                elem.set_property(Field::NewAnimation, FieldValue::Integer(new_anim as u32));
                self.record_element(elem)
            }
            RecordRestoreAnim => {
                let old_anim = stack.pop_i32();
                let actor = stack.pop_i32();
                // Gates on `IsActor` after the sequence-level check.
                if !self.is_actor_handle(actor) {
                    tracing::warn!("RecordRestoreAnim: illegal actor handle {actor}");
                    return 0;
                }
                let level = self.recording_level();
                let mut elem =
                    SequenceElement::new_generic(level, Command::RestoreAnim, self.actor_id(actor));
                elem.set_property(Field::OldAnimation, FieldValue::Integer(old_anim as u32));
                self.record_element(elem)
            }
            ResetAnim => {
                // NOT a Record function — directly resets the actor's
                // sprite to frame 0 of its current animation row.
                // Rejects !ActorExists || !IsFX with a warning +
                // false, else `reset_sprite_frame()` and true.
                let actor = stack.pop_i32();
                let is_fx = self.get_entity(actor).is_some_and(|e| e.is_fx());
                if !self.actor_exists(actor) || !is_fx {
                    tracing::error!("Script error (ResetAnim): invalid animation handle {actor}");
                    0
                } else {
                    self.simulation_barriers
                        .commands
                        .push(DeferredCommand::ResetSpriteFrame { actor });
                    1
                }
            }

            // --- Speech ---
            RecordSpeak => {
                let speak_id = stack.pop_i32();
                let actor = stack.pop_i32();
                // Validates `IsHuman` and bound-checks
                // `id < NUMBER_OF_REMARKS` before constructing the
                // element.
                if !self.get_entity(actor).is_some_and(|e| e.is_human()) {
                    tracing::warn!("RecordSpeak: illegal actor {actor} (not human)");
                    return 0;
                }
                let level = self.recording_level();
                let mut elem =
                    SequenceElement::new_generic(level, Command::Speak, self.actor_id(actor));
                elem.set_property(Field::SpeakId, FieldValue::Integer(speak_id as u32));
                // SpeakVariant = 0; SpeakFlags = SPEECH_SCRIPT |
                // SPEECH_ALWAYS.  The ALWAYS bit is load-bearing
                // — the speech pipeline uses it to bypass the
                // forbidden-remark and chorus filters (see ai.rs
                // / melee.rs consumers).
                elem.set_property(Field::SpeakVariant, FieldValue::Integer(0));
                const SPEECH_SCRIPT: u32 = 0x0004;
                const SPEECH_ALWAYS: u32 = 0x0008;
                elem.set_property(
                    Field::SpeakFlags,
                    FieldValue::Integer(SPEECH_SCRIPT | SPEECH_ALWAYS),
                );
                self.record_element(elem)
            }
            RecordSpeakPC => {
                let variant = stack.pop_i32();
                let speak_id = stack.pop_i32();
                let actor = stack.pop_i32();
                // Gates on `IsPC()`.
                if !self.get_entity(actor).is_some_and(|e| e.is_pc()) {
                    tracing::warn!("RecordSpeakPC: illegal actor {actor} (not PC)");
                    return 0;
                }
                let level = self.recording_level();
                let mut elem =
                    SequenceElement::new_generic(level, Command::Speak, self.actor_id(actor));
                elem.set_property(Field::SpeakId, FieldValue::Integer(speak_id as u32));
                elem.set_property(Field::SpeakVariant, FieldValue::Integer(variant as u32));
                self.record_element(elem)
            }

            // --- AI / user locks ---
            RecordLockAI => {
                let actor = stack.pop_i32();
                // Gates on `ActorExists && IsActor`.
                if !self.is_actor_handle(actor) {
                    tracing::warn!("RecordLockAI: illegal actor handle {actor}");
                    return 0;
                }
                let level = self.recording_level();
                let elem = SequenceElement::new(level, Command::LockAi, self.actor_id(actor));
                self.record_element(elem)
            }
            RecordUnlockAI => {
                let actor = stack.pop_i32();
                // Gates on `ActorExists && IsActor`.
                if !self.is_actor_handle(actor) {
                    tracing::warn!("RecordUnlockAI: illegal actor handle {actor}");
                    return 0;
                }
                let level = self.recording_level();
                let elem = SequenceElement::new(level, Command::UnlockAi, self.actor_id(actor));
                self.record_element(elem)
            }
            RecordLockUser => {
                let level = self.recording_level();
                let elem = SequenceElement::new(level, Command::LockUser, None);
                self.record_element(elem)
            }
            RecordUnLockUser => {
                let level = self.recording_level();
                let elem = SequenceElement::new(level, Command::UnlockUser, None);
                self.record_element(elem)
            }
            RecordFreezeAll => {
                let freeze = stack.pop_i32();
                let level = self.recording_level();
                let mut elem = SequenceElement::new_generic(level, Command::FreezeAll, None);
                elem.set_property(Field::Freeze, FieldValue::Bool(freeze != 0));
                self.record_element(elem)
            }

            // --- Timer ---
            RecordTimer => {
                let frames = stack.pop_i32();
                let level = self.recording_level();
                let mut elem = SequenceElement::new_generic(level, Command::Timer, None);
                elem.set_property(Field::Timer, FieldValue::Integer(frames as u32));
                self.record_element(elem)
            }

            // --- Seeking ---
            RecordSeekActor => {
                let distance = stack.pop_i32();
                let style = stack.pop_i32();
                let target = stack.pop_i32();
                let actor = stack.pop_i32();
                let level = self.recording_level();
                let action = Self::seek_style(style);
                let mut elem = SequenceElement::new_movement(
                    level,
                    Command::Seek,
                    self.actor_id(actor),
                    action,
                );
                if let crate::sequence::SequenceElementData::Movement {
                    element,
                    tolerance,
                    flags,
                    ..
                } = &mut elem.data
                {
                    *element = self.actor_id(target);
                    *tolerance = f32::from_bits(distance as u32);
                    *flags |= MoveFlags::SEEK;
                }
                self.record_element(elem)
            }
            RecordSeekActorMessage => {
                let msg_id = stack.pop_i32();
                let msg_actor = stack.pop_i32();
                let distance = stack.pop_i32();
                let style = stack.pop_i32();
                let target = stack.pop_i32();
                let actor = stack.pop_i32();
                // Rejects non-actor message-target handles.
                if msg_actor != 0 && !self.is_actor_handle(msg_actor) {
                    tracing::warn!("RecordSeekActorMessage: illegal msg_actor handle {msg_actor}");
                    return 0;
                }
                let level = self.recording_level();
                let action = Self::seek_message_style(style);
                let mut seek_elem = SequenceElement::new_movement(
                    level,
                    Command::Seek,
                    self.actor_id(actor),
                    action,
                );
                // Builds a single-element sub-sequence with a
                // SendMessage command at sub-level 1 and stashes
                // it on the seek element as the post-seek
                // sequence.  The post-seek sub-sequence fires
                // only on successful seek completion, not when
                // the seek is interrupted/aborted.
                let mut post_seek = crate::sequence::Sequence::new();
                post_seek
                    .append_element(self.build_send_message_element(1, msg_actor, msg_id, 0, 0));
                if let crate::sequence::SequenceElementData::Movement {
                    element,
                    tolerance,
                    flags,
                    post_seek_sequence,
                    ..
                } = &mut seek_elem.data
                {
                    *element = self.actor_id(target);
                    *tolerance = f32::from_bits(distance as u32);
                    *flags |= MoveFlags::SEEK;
                    *post_seek_sequence = Some(Box::new(post_seek));
                }
                self.record_element(seek_elem)
            }
            RecordSeekActorMessageWithArguments => {
                let arg2 = stack.pop_i32();
                let arg1 = stack.pop_i32();
                let msg_id = stack.pop_i32();
                let msg_actor = stack.pop_i32();
                let distance = stack.pop_i32();
                let style = stack.pop_i32();
                let target = stack.pop_i32();
                let actor = stack.pop_i32();
                // Rejects non-actor msg handles and `id < 1000`.
                if msg_actor != 0 && !self.is_actor_handle(msg_actor) {
                    tracing::warn!(
                        "RecordSeekActorMessageWithArguments: illegal msg_actor handle {msg_actor}"
                    );
                    return 0;
                }
                if msg_id < 1000 {
                    tracing::warn!(
                        "RecordSeekActorMessageWithArguments: ID for custom event is {msg_id}, must be >= 1000"
                    );
                    return 0;
                }
                let level = self.recording_level();
                let action = Self::seek_message_style(style);
                let mut seek_elem = SequenceElement::new_movement(
                    level,
                    Command::Seek,
                    self.actor_id(actor),
                    action,
                );
                // Same post-seek sub-sequence wiring as
                // `RecordSeekActorMessage` above.
                let mut post_seek = crate::sequence::Sequence::new();
                post_seek.append_element(
                    self.build_send_message_element(1, msg_actor, msg_id, arg1, arg2),
                );
                if let crate::sequence::SequenceElementData::Movement {
                    element,
                    tolerance,
                    flags,
                    post_seek_sequence,
                    ..
                } = &mut seek_elem.data
                {
                    *element = self.actor_id(target);
                    *tolerance = f32::from_bits(distance as u32);
                    *flags |= MoveFlags::SEEK;
                    *post_seek_sequence = Some(Box::new(post_seek));
                }
                self.record_element(seek_elem)
            }
            RecordStopSeek => {
                // Returns false (no-op).  The seek system handles
                // stopping internally.
                let _actor = stack.pop_i32();
                0
            }

            // --- RecordAction (polymorphic command dispatch) ---
            RecordAction => {
                let number = stack.pop_i32();
                let action_id = stack.pop_i32();
                let actor = stack.pop_i32();
                // Gates the whole dispatch on `ActorExists(actor)`.
                if !self.actor_exists(actor) {
                    tracing::warn!("RecordAction: invalid actor handle {actor}");
                    return 0;
                }
                let level = self.recording_level();
                let owner = self.actor_id(actor);
                // Antagonist lookup for SHOOT / ENTER_SF /
                // THRUST_*: `number` is a 0-based index into the
                // script-element array; we abort with `false`
                // when out of range. Convert that script index into
                // an actor handle before resolving it.
                let resolve_antagonist = |number: i32| -> Option<Option<EntityId>> {
                    if number < 0 || (number as usize) >= self.entities.len() {
                        None
                    } else {
                        // Slot may be None (a null antagonist
                        // is accepted once the bounds check
                        // passes).
                        Some(self.actor_id(Self::actor_handle_from_index(number as usize)))
                    }
                };
                // Script command constants.
                const WAIT: i32 = 0;
                const TURN: i32 = 1;
                const AIM: i32 = 2;
                const AIM_UP: i32 = 3;
                const SHOOT: i32 = 4;
                const ENTER_SF: i32 = 5;
                const LEAVE_SF: i32 = 6;
                const PARRY: i32 = 7;
                const THRUST_A: i32 = 8;
                const THRUST_B: i32 = 9;
                const THRUST_C: i32 = 10;
                const THRUST_D: i32 = 11;
                const THRUST_E: i32 = 12;
                const THRUST_F: i32 = 13;
                const THRUST_G: i32 = 14;
                const THRUST_H: i32 = 15;
                const THRUST_I: i32 = 16;
                const LOOK_LEFT: i32 = 17;
                const LOOK_RIGHT: i32 = 18;
                const UNEQUIP_BOW: i32 = 19;
                const CROUCH_DOWN: i32 = 20;

                let elem = match action_id {
                    WAIT => SequenceElement::new(level, Command::Wait, owner),
                    TURN => {
                        let mut e = SequenceElement::new_generic(level, Command::Turn, owner);
                        e.set_property(Field::Direction, FieldValue::Integer((number % 16) as u32));
                        e
                    }
                    AIM => SequenceElement::new(level, Command::EquipBow, owner),
                    AIM_UP => SequenceElement::new(level, Command::RaiseBow, owner),
                    SHOOT => {
                        let Some(antagonist) = resolve_antagonist(number) else {
                            tracing::warn!("RecordAction SHOOT: illegal antagonist index {number}");
                            return 0;
                        };
                        SequenceElement::new_interaction(
                            level,
                            Command::ShootBowOnce,
                            owner,
                            antagonist,
                        )
                    }
                    ENTER_SF => {
                        let Some(antagonist) = resolve_antagonist(number) else {
                            tracing::warn!(
                                "RecordAction ENTER_SF: illegal antagonist index {number}"
                            );
                            return 0;
                        };
                        let mut e =
                            SequenceElement::new_generic(level, Command::EnterSwordfight, owner);
                        // Store the opponent unconditionally
                        // after the bounds check; a null-slot
                        // antagonist still gets recorded.
                        if let Some(ant) = antagonist {
                            e.set_property(Field::Opponent, FieldValue::Element(ant));
                        }
                        e.set_property(Field::JumplineDestination, FieldValue::Integer(0));
                        e
                    }
                    LEAVE_SF => SequenceElement::new(level, Command::QuitSwordfight, owner),
                    PARRY => SequenceElement::new(level, Command::ParrySword, owner),
                    THRUST_A | THRUST_B | THRUST_C | THRUST_D | THRUST_E | THRUST_F | THRUST_G
                    | THRUST_H | THRUST_I => {
                        let cmd = match action_id {
                            THRUST_A => Command::SwordstrikeThrustA,
                            THRUST_B => Command::SwordstrikeThrustB,
                            THRUST_C => Command::SwordstrikeThrustC,
                            THRUST_D => Command::SwordstrikeThrustD,
                            THRUST_E => Command::SwordstrikeThrustE,
                            THRUST_F => Command::SwordstrikeThrustF,
                            THRUST_G => Command::SwordstrikeThrustG,
                            THRUST_H => Command::SwordstrikeThrustH,
                            _ => Command::SwordstrikeThrustI,
                        };
                        let Some(antagonist) = resolve_antagonist(number) else {
                            tracing::warn!(
                                "RecordAction THRUST: illegal antagonist index {number}"
                            );
                            return 0;
                        };
                        SequenceElement::new_interaction(level, cmd, owner, antagonist)
                    }
                    LOOK_LEFT => {
                        // Rejects non-soldiers.
                        if !self.get_entity(actor).is_some_and(|e| e.is_soldier()) {
                            tracing::warn!(
                                "RecordAction LOOK_LEFT: actor {actor} is not a soldier"
                            );
                            return 0;
                        }
                        SequenceElement::new(level, Command::LookLeft, owner)
                    }
                    LOOK_RIGHT => {
                        if !self.get_entity(actor).is_some_and(|e| e.is_soldier()) {
                            tracing::warn!(
                                "RecordAction LOOK_RIGHT: actor {actor} is not a soldier"
                            );
                            return 0;
                        }
                        SequenceElement::new(level, Command::LookRight, owner)
                    }
                    UNEQUIP_BOW => SequenceElement::new(level, Command::UnequipBow, owner),
                    CROUCH_DOWN => SequenceElement::new(level, Command::CrouchDown, owner),
                    _ => {
                        tracing::warn!("RecordAction: unknown script command ID {action_id}");
                        return 0;
                    }
                };
                self.record_element(elem)
            }

            // --- Corpse handling ---
            RecordTakeCorpse => {
                // Walk the taker to the corpse's position, then
                // run the TakeCorpse interaction.
                let style = stack.pop_i32();
                let corpse = stack.pop_i32();
                let actor = stack.pop_i32();
                // Gates on the taker being a PC with one of the
                // carry actions, and the corpse being an actor.
                if !self.is_pc_carrier(actor) {
                    tracing::warn!(
                        "RecordTakeCorpse: taker {actor} is not a PC with a carry action"
                    );
                    return 0;
                }
                if !self.is_actor_handle(corpse) {
                    tracing::warn!("RecordTakeCorpse: corpse {corpse} is not an actor");
                    return 0;
                }
                let level = self.recording_level();
                let action = Self::movement_style(style);

                // Walk the taker to the corpse position (no-op if the
                // corpse handle is invalid — the take element alone
                // still makes the sequence well-formed).
                if let Some(corpse_entity) = self.get_entity(corpse) {
                    let pos = corpse_entity.element_data().position_map();
                    let corpse_layer = corpse_entity.element_data().layer();
                    let corpse_sector = corpse_entity
                        .element_data()
                        .sector()
                        .map(u16::from)
                        .unwrap_or(0);
                    // Replays `update_motion_start_position` on
                    // the taker so a chained Record* sees the
                    // corpse as the new motion target.
                    let origin = self.update_motion_start_position(
                        actor,
                        (pos.x, pos.y),
                        Some((corpse_layer, corpse_sector)),
                    );
                    let (sx, sy, src_layer, src_sector) =
                        origin.unwrap_or((pos.x, pos.y, corpse_layer, corpse_sector));
                    // Tolerance is the per-animation stand-off
                    // for the carry-transition, matching the
                    // original GetActionDistance call.
                    let Some(animation) =
                        crate::engine::command_action_distance_animation(Command::TakeCorpse)
                    else {
                        tracing::warn!(
                            "RecordTakeCorpse: TakeCorpse has no action-distance animation"
                        );
                        return 0;
                    };
                    let Some(tolerance) = self.actor_action_distance(actor, animation) else {
                        return 0;
                    };
                    self.append_move_to_sequence(
                        actor,
                        action,
                        (sx, sy),
                        src_sector,
                        src_layer,
                        (pos.x, pos.y),
                        corpse_sector,
                        corpse_layer,
                        None,
                        tolerance,
                        MoveFlags::CALLED_BY_SCRIPT,
                        1.0,
                    );
                }

                // Take it.
                let elem = SequenceElement::new_interaction(
                    level,
                    Command::TakeCorpse,
                    self.actor_id(actor),
                    self.actor_id(corpse),
                );
                self.record_element(elem)
            }
            RecordLeaveCorpse => {
                let actor = stack.pop_i32();
                // Gates on `IsPC && (LittleJohnCarry || FarmerCarry)`.
                if !self.is_pc_carrier(actor) {
                    tracing::warn!(
                        "RecordLeaveCorpse: actor {actor} is not a PC with a carry action"
                    );
                    return 0;
                }
                let level = self.recording_level();
                let elem = SequenceElement::new(level, Command::DropCorpse, self.actor_id(actor));
                self.record_element(elem)
            }

            // --- Mobile elements ---
            RecordStartMobileElement
            | RecordStopMobileElement
            | RecordActivateMobileElement
            | RecordDeactivateMobileElement => {
                let mobile_index = stack.pop_i32();
                let Some(owner) = self.mobile_owner_id(mobile_index) else {
                    panic!("Record*MobileElement references missing mobile index {mobile_index}");
                };
                let command = match native {
                    RecordStartMobileElement => Command::StartMobile,
                    RecordStopMobileElement => Command::StopMobile,
                    RecordActivateMobileElement => Command::ActivateMobile,
                    RecordDeactivateMobileElement => Command::DeactivateMobile,
                    _ => unreachable!(),
                };
                let level = self.recording_level();
                self.record_element(SequenceElement::new(level, command, Some(owner)))
            }

            // --- Misc ---
            RecordUnBlip => {
                let actor = stack.pop_i32();
                // Gates on `ActorExists && IsActor`.
                if !self.is_actor_handle(actor) {
                    tracing::warn!("RecordUnBlip: illegal actor handle {actor}");
                    return 0;
                }
                let level = self.recording_level();
                let elem = SequenceElement::new(level, Command::Unblip, self.actor_id(actor));
                self.record_element(elem)
            }

            _ => self.dispatch_actors(native, stack),
        }
    }
}
