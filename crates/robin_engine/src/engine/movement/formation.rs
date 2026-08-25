use super::*;

impl EngineInner {
    // ─── Order system ─────────────────────────────────────────────

    /// Snap a click/formation-slot point to the nearest authorized
    /// (walkable) position for a unit of the given size.
    ///
    /// Returns the adjusted point, or `None` if no walkable spot can be
    /// found near the click. Builds a move-box-sized bbox around the
    /// candidate point, pushes it away from any motion lines that would
    /// otherwise block the unit, then returns the box center.
    ///
    /// Without this snap, clicks that land on dynamic elements like
    /// drawbridges (whose surface lies just outside the static motion-area
    /// polygon) or even slightly inside an obstacle's bbox fail
    /// `object_position_authorized` and the pathfinder refuses to build
    /// a path — so the click appears to do nothing.
    ///
    /// `reference` is used as the "push toward" anchor — typically the
    /// raw click point passed alongside the per-PC formation slot.
    ///
    /// Callers must skip this snap when the click hits a Door/Drawbridge
    /// sector — the cross-sector gate A* path routes the PC through the
    /// door's entry point, which is the only walkable approach when the
    /// door sector itself isn't a motion area (e.g. a raised drawbridge).
    pub fn snap_click_to_walkable(
        &self,
        candidate: MapPoint,
        reference: MapPoint,
        layer: u16,
        half_diagonal_idx: usize,
    ) -> Option<MapPoint> {
        let hd = self
            .world
            .fast_grid
            .level
            .move_box_half_diagonals
            .get(half_diagonal_idx)
            .copied()?;
        let mut bbox = MapBBox::from_corners(
            MapPoint::new(candidate.x - hd.x, candidate.y - hd.y),
            MapPoint::new(candidate.x + hd.x, candidate.y + hd.y),
        );
        if self
            .world
            .fast_grid
            .find_authorized_position_toward(&mut bbox, reference, layer)
        {
            Some(bbox.center())
        } else {
            None
        }
    }

    /// Source-exact `PerformGroupMove` formation-slot authorization for one
    /// selected actor. Unlike the generic click snap, this must use the
    /// actor's live move box rather than pathfinder half-diagonal table entry
    /// zero; different PCs and adopted saves can carry different boxes.
    pub(in crate::engine) fn authorize_group_move_destination(
        &self,
        actor: EntityId,
        candidate: MapPoint,
        reference: MapPoint,
        layer: u16,
        is_lift: bool,
    ) -> Option<MapPoint> {
        let entity = self.get_entity(actor)?;
        let position = entity.position_iface();
        let mut bbox = group_move_candidate_box(
            *position.get_move_box_map(),
            *position.get_move_box(),
            entity.element_data().position_map(),
            candidate,
            is_lift,
        );
        if self
            .world
            .fast_grid
            .find_authorized_position_toward(&mut bbox, reference, layer)
        {
            Some(bbox.center())
        } else {
            None
        }
    }

    /// Issue movement orders for a group of selected PCs around a single
    /// click point.
    ///
    /// Uses the "mercenary" formation: each PC keeps its position
    /// relative to the group centroid and walks to the corresponding
    /// offset around `click_point`.  The marker for each PC is placed
    /// at *its own* resolved destination, not at the raw click point.
    ///
    /// Each per-PC formation slot is then snapped to a walkable spot via
    /// [`EngineInner::snap_click_to_walkable`].  This is what allows
    /// clicks on drawbridges and other dynamic elements to actually move
    /// PCs onto them — the raw click often lands just outside the
    /// walkable polygon, and the snap pulls it back inside.
    ///
    /// Uses mercenary formation for compact groups and circular dispatch
    /// for spread-out groups.
    pub(crate) fn perform_group_move(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        pc_ids: &[EntityId],
        click_point: MapPoint,
        run: bool,
        show_marker: bool,
        goal_override: Option<(crate::sector::SectorNumber, u16)>,
        goal_sector_index_override: Option<crate::fast_find_grid::SectorIndex>,
        door_route_override: Option<bool>,
        recorded_gate_routes: &[(EntityId, Vec<(u32, bool)>)],
        recorded_failed_gate_routes: &[EntityId],
    ) {
        self.perform_group_move_with_destinations(
            sim,
            assets,
            pc_ids,
            click_point,
            run,
            show_marker,
            goal_override,
            goal_sector_index_override,
            door_route_override,
            recorded_gate_routes,
            recorded_failed_gate_routes,
            None,
        );
    }

    /// Run the normal group-movement resolution while retaining explicit
    /// role-aware destinations. Allied formations use this rather than
    /// invoking [`Self::perform_group_move`] once per soldier, so the group
    /// shares the same click-sector resolution and slot-authorization pass as
    /// an ordinary multi-hero click.
    pub(in crate::engine) fn perform_group_move_to_slots(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        actor_ids: &[EntityId],
        click_point: MapPoint,
        destinations: &[MapPoint],
        run: bool,
        show_marker: bool,
    ) {
        assert_eq!(
            actor_ids.len(),
            destinations.len(),
            "explicit group-move destination count must match actor count"
        );
        self.perform_group_move_with_destinations(
            sim,
            assets,
            actor_ids,
            click_point,
            run,
            show_marker,
            None,
            None,
            None,
            &[],
            &[],
            Some(destinations),
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::engine) fn perform_group_move_with_destinations(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        pc_ids: &[EntityId],
        click_point: MapPoint,
        run: bool,
        show_marker: bool,
        goal_override: Option<(crate::sector::SectorNumber, u16)>,
        goal_sector_index_override: Option<crate::fast_find_grid::SectorIndex>,
        door_route_override: Option<bool>,
        recorded_gate_routes: &[(EntityId, Vec<(u32, bool)>)],
        recorded_failed_gate_routes: &[EntityId],
        explicit_destinations: Option<&[MapPoint]>,
    ) {
        if pc_ids.is_empty() {
            return;
        }

        // Preemption is handled downstream by `arbitrate_instruct`:
        // every same-sector PC gets a fresh `Command::Move` sequence
        // element launched via `launch_element` below, which reaches
        // `InstructOwner` on the next hourglass and drives the standard
        // priority-arbitration cascade.  A pending scroll/object pickup
        // (Seek + queued Take at `Normal`) vs a new Move at `Normal`
        // resolves to `InterruptCurrent`, which cleanly tears down both
        // the seek and its post-seek Take via the `NEXT_LEVEL` cascade.
        // Earlier fixes tried to short-circuit this with explicit
        // `stop_owner` calls, but `stop_owner` on a movement element
        // keeps the element InProgress "for transition", which left the
        // stale seek hanging when the same-sector shortcut was
        // direct-pathfinder rather than a proper Move element.

        // Collect each PC's effective route-source position, layer, and
        // sector. While a non-interruptible door pass is active, a newly
        // issued move cannot begin until that pass reaches its committed far
        // side. Original input dispatch observes that committed door side
        // when `PerformGroupMove` calls `AppendMoveToSequence`; using the
        // actor's still-visible near-side sector here would incorrectly
        // classify a return click as a same-sector Move and lose the reverse
        // gate traversal before the command is postponed.
        let positions: Vec<(
            EntityId,
            MapPoint,
            u16,
            crate::position_interface::SectorHandle,
        )> = pc_ids
            .iter()
            .map(|&pc_id| {
                let e = self
                    .get_entity(pc_id)
                    .unwrap_or_else(|| panic!("selected group-move actor {pc_id:?} is missing"));
                // Original passes the actor's complete live `RHSector*`
                // from GetSector() into PerformMove/AppendMoveToSequence
                // (RHengine.cpp:5407-5410, 5512-5515, 9941-10049). Restore
                // omitted legacy arena identity at this exact snapshot
                // boundary before same-sector classification or gate A*. A
                // selected live door remains authoritative and is resolved
                // first, while the actor may still display its old near-side
                // position.
                let (position, sector, layer) = group_move_route_source(
                    self,
                    pc_id,
                    e,
                    &self.script_domains.interactables.doors,
                );
                (pc_id, position, layer, sector)
            })
            .collect();
        if positions.is_empty() {
            return;
        }

        let src_layer = positions[0].2;
        let reference = positions[0].1;

        // ── Unified sector hit-test ──
        //
        // Top-down layer search reconstructs `mpSelectedSector`, whose sector
        // kind drives the door/lift/jump semantics below.  It is deliberately
        // independent of `goal_override`: Original `PerformGroupMove` can use
        // a patch's sector as `pSectorGoal` while `mpSelectedSector` remains
        // the coincident mouse-selection overlay (RHengine.cpp:5322-5337).
        // RecordGroupMove stores that patch-aware route goal, not necessarily
        // `mpSelectedSector`, so replay must preserve both identities.
        let hit = self
            .world
            .fast_grid
            .get_sector_screen(click_point, reference);
        let selected_grid_sector = hit
            .sector_idx
            .and_then(|i| self.world.fast_grid.level.sectors.get(usize::from(i)));
        let (is_lift_click, is_door_click_sector, is_jump_click) = selected_grid_sector
            .map(|sector| group_move_sector_kinds(sector.sector_type))
            .unwrap_or((false, false, false));
        let jump_underlying_sector = selected_grid_sector
            .filter(|sector| sector.sector_type.is_jump())
            .and_then(|sector| sector.underlying_sector)
            .and_then(|index| {
                self.world
                    .fast_grid
                    .level
                    .sectors
                    .get(usize::from(index))
                    .map(|sector| (sector.sector_number, index, sector.layer))
            });
        let clicked_sector_door_index = selected_grid_sector.and_then(|sector| sector.door_index);
        let clicked_polygon_door_index = self.scripts.mission.as_ref().and_then(|_| {
            door_click_polygon_at(&self.script_domains.interactables.doors, click_point)
        });
        let spatial_clicked_door_index = clicked_sector_door_index.or(clicked_polygon_door_index);
        let spatial_is_door_click = is_door_click_sector || spatial_clicked_door_index.is_some();
        let (clicked_door_index, is_door_click, bypass_formation_authorization) =
            group_move_door_selection(
                spatial_clicked_door_index,
                spatial_is_door_click,
                door_route_override,
            );
        let (route_goal_sector, route_goal_layer) =
            group_move_route_goal(goal_override, hit.sector, hit.layer);
        let route_goal_sector_index = resolve_group_move_route_goal_index(
            goal_override,
            goal_sector_index_override,
            hit.sector,
            hit.sector_idx,
            hit.layer,
            selected_grid_sector,
            &self.world.fast_grid.level,
        );

        let (
            goal_sector,
            effective_click,
            effective_layer,
            is_valid,
            is_lift_click,
            is_door_click,
            is_jump_click,
            clicked_jump_sector_idx,
            jump_underlying_sector,
            clicked_door_index,
        ) = if goal_override.is_some() {
            (
                route_goal_sector,
                click_point,
                route_goal_layer,
                true,
                is_lift_click,
                is_door_click,
                is_jump_click,
                if is_jump_click { hit.sector_idx } else { None },
                jump_underlying_sector,
                clicked_door_index,
            )
        } else {
            let is_valid = hit.is_valid_for_move(&self.world.fast_grid);

            // ── Door/Drawbridge click shortcut ──
            //
            // When the click hits a door sector, bypass the walkability
            // snap on formation slots.  Per-PC routing must also skip
            // `snap_click_to_walkable` so the destination stays in the
            // door sector and the gate-A* path routes through the
            // door's entry point (the door sector itself is not a
            // motion area).
            // Door index of the clicked door sector, if any.  Used to
            // route the per-PC gate search via `find_path_to_door` and
            // emit a `GoalShape::Door` terminal element.
            let (effective_click, effective_layer) = if is_valid || is_jump_click {
                (click_point, hit.layer)
            } else {
                let snapped = self.snap_to_nearest_walkable(assets, click_point, src_layer);
                (snapped.unwrap_or(click_point), src_layer)
            };
            (
                hit.sector,
                effective_click,
                effective_layer,
                is_valid,
                is_lift_click,
                is_door_click,
                is_jump_click,
                if is_jump_click { hit.sector_idx } else { None },
                jump_underlying_sector,
                clicked_door_index,
            )
        };

        // ── Compute formation slots around the click point ──
        //
        // If the group is compact enough, use mercenary formation
        // (preserve relative positions).  Otherwise use circular
        // dispatch (arrange in a circle around click).
        let pc_positions: Vec<MapPoint> = positions
            .iter()
            .map(|(pc_id, _, _, _)| {
                self.get_entity(*pc_id)
                    .unwrap_or_else(|| panic!("selected group-move actor {pc_id:?} is missing"))
                    .element_data()
                    .position_map()
            })
            .collect();
        let (mercenary_center, dests) = if let Some(destinations) = explicit_destinations {
            (None, destinations.to_vec())
        } else {
            let n = pc_positions.len() as f32;
            let mut cx = pc_positions.iter().map(|p| p.x).sum::<f32>();
            let mut cy = pc_positions.iter().map(|p| p.y).sum::<f32>();
            // Original multiplies the accumulated vector by the reciprocal;
            // preserve that operation rather than compiling this as two
            // divisions with potentially different rounding.
            let reciprocal = 1.0f32 / n;
            cx *= reciprocal;
            cy *= reciprocal;
            let max_sq_dist = pc_positions
                .iter()
                .map(|p| {
                    let dx = p.x - cx;
                    let dy = p.y - cy;
                    dx * dx + dy * dy
                })
                .fold(0.0f32, f32::max);
            if max_sq_dist <= GROUP_LIMIT_MAX * GROUP_LIMIT_MAX {
                (
                    Some(MapPoint::new(cx, cy)),
                    mercenary_formation_destinations(&pc_positions, effective_click),
                )
            } else {
                (
                    None,
                    circular_dispatch_destinations(&pc_positions, effective_click),
                )
            }
        };

        // ── Per-PC routing ──
        // For each PC, decide between:
        //   1. Same-sector: simple MOVE
        //   2. Cross-sector (door/lift): gate-A* sequence
        for ((pc_id, _, pc_src_layer, src_sector), formation_dest) in
            positions.iter().zip(dests.iter())
        {
            let owner_is_pc = self
                .get_entity(*pc_id)
                .unwrap_or_else(|| panic!("selected group-move actor {pc_id:?} is missing"))
                .is_pc();
            // Compact-group placement is authorized exactly once, before
            // PerformMove, using the box produced by Original's ordered
            // `box - center + click` translations. Reconstructing a point
            // first and then translating the box is algebraically equivalent
            // but changes f32 rounding at path-goal boundaries.
            let mercenary_dest;
            let dest = if let Some(center) = mercenary_center {
                let Some(entity) = self.get_entity(*pc_id) else {
                    panic!("selected group-move actor {pc_id:?} is missing");
                };
                let position = entity.position_iface();
                let live_move_box_map = *position.get_move_box_map();
                let upright_move_box = *position.get_move_box();
                let actor_position = entity.element_data().position_map();
                let mut bbox = group_move_mercenary_box(
                    live_move_box_map,
                    upright_move_box,
                    actor_position,
                    center,
                    effective_click,
                    is_lift_click,
                );
                let authorized = if bypass_formation_authorization {
                    true
                } else {
                    self.world.fast_grid.find_authorized_position_toward(
                        &mut bbox,
                        effective_click,
                        effective_layer,
                    )
                };
                if !authorized {
                    self.hero_speaking(
                        assets,
                        *pc_id,
                        crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                    );
                    continue;
                }
                mercenary_dest = bbox.center();
                &mercenary_dest
            } else {
                formation_dest
            };
            let mut pc_goal_sector = goal_sector;
            let mut pc_goal_sector_index = route_goal_sector_index;
            let mut pc_effective_layer = effective_layer;
            if is_jump_click {
                // PerformGroupMove authorizes each formation slot before
                // PerformMove tests whether the selected jump is usable.
                // Keep the raw click through the jump-sector hit test, then
                // apply that same move-box authorization here; the coarse
                // nearest-walkable fallback is not equivalent near a jump
                // landing boundary.
                let resolved_jump_dest = if mercenary_center.is_some() {
                    Some(*dest)
                } else {
                    self.authorize_group_move_destination(
                        *pc_id,
                        *dest,
                        effective_click,
                        pc_effective_layer,
                        is_lift_click,
                    )
                };
                let Some(resolved_jump_dest) = resolved_jump_dest else {
                    self.hero_speaking(
                        assets,
                        *pc_id,
                        crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                    );
                    continue;
                };
                let pc_pos = positions
                    .iter()
                    .find(|(id, _, _, _)| *id == *pc_id)
                    .map(|(_, p, _, _)| *p)
                    .unwrap_or(*dest);
                let source_line_idx = self
                    .get_nearest_jumpable_jump_line(
                        *pc_id,
                        u32::from(clicked_jump_sector_idx.unwrap_or_else(|| {
                            panic!("jump click missing selected jump sector index")
                        })),
                        pc_pos,
                        resolved_jump_dest,
                        true,
                        jump_underlying_sector.map(|(sector, _, _)| u16::from(sector)),
                    )
                    .and_then(crate::jump_line::JumpLineIndex::new);
                if let Some(source_line_idx) = source_line_idx {
                    let Some(source_line) = self
                        .world
                        .fast_grid
                        .level
                        .jump_lines
                        .get(usize::from(source_line_idx))
                        .cloned()
                    else {
                        panic!("line-jump source line {source_line_idx} is missing");
                    };
                    let Some(destination_line_idx) = source_line
                        .associated_line_index
                        .and_then(crate::jump_line::JumpLineIndex::new)
                    else {
                        panic!("line-jump source line {source_line_idx} has no associated line");
                    };
                    if self
                        .world
                        .fast_grid
                        .level
                        .jump_lines
                        .get(usize::from(destination_line_idx))
                        .is_none()
                    {
                        panic!(
                            "line-jump destination line {destination_line_idx} for source {source_line_idx} is missing"
                        );
                    }

                    let mut seq = build_line_jump_click_sequence(
                        *pc_id,
                        player_group_move_action(run),
                        source_line_idx,
                        &source_line,
                        destination_line_idx,
                        resolved_jump_dest,
                        pc_effective_layer,
                        1.0,
                    );
                    if owner_is_pc {
                        let speak = crate::sequence::SequenceElement::new(
                            4,
                            crate::element::Command::SpeakHeroReachDestination,
                            Some(*pc_id),
                        );
                        seq.append_element(speak);
                    }
                    self.append_posture_recovery(*pc_id, &mut seq);
                    self.launch_sequence(seq);
                    if show_marker && !is_door_click {
                        self.feedback.ground_mark.add_mark(
                            resolved_jump_dest.x,
                            resolved_jump_dest.y,
                            pc_effective_layer,
                        );
                    }
                    continue;
                } else if let Some((underlying_sector, underlying_index, underlying_layer)) =
                    jump_underlying_sector
                {
                    pc_goal_sector = Some(underlying_sector);
                    pc_goal_sector_index = Some(underlying_index);
                    pc_effective_layer = underlying_layer;
                    tracing::debug!(
                        actor = ?pc_id,
                        click_x = effective_click.x,
                        click_y = effective_click.y,
                        sector = %underlying_sector,
                        layer = underlying_layer,
                        "jump-sector click has no executable jump line; falling back to underlying motion sector"
                    );
                } else {
                    tracing::warn!(
                        actor = ?pc_id,
                        click_x = effective_click.x,
                        click_y = effective_click.y,
                        "jump-sector click has no executable jump line and no underlying motion sector"
                    );
                    self.hero_speaking(
                        assets,
                        *pc_id,
                        crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                    );
                    continue;
                };
            }

            // Same-sector or unknown goal sector: simple move
            if group_move_uses_simple_route(
                recorded_gate_routes.iter().any(|(actor, _)| actor == pc_id)
                    || recorded_failed_gate_routes
                        .iter()
                        .any(|actor| actor == pc_id),
                is_door_click,
                is_valid,
                pc_goal_sector,
                pc_goal_sector_index,
                pc_effective_layer,
                u16::from(*src_sector),
                src_sector.arena_index(),
                *pc_src_layer,
            ) {
                // Door clicks skip the walkable snap entirely.
                let snap_res = if bypass_formation_authorization || mercenary_center.is_some() {
                    Some(*dest)
                } else {
                    self.authorize_group_move_destination(
                        *pc_id,
                        *dest,
                        effective_click,
                        pc_effective_layer,
                        is_lift_click,
                    )
                };
                let snapped = match snap_res {
                    Some(pt) => pt,
                    None => {
                        // FindAuthorizedPosition failure on the
                        // mercenary/same-sector path fires
                        // HERO_UNABLE_TO_DO_SOMETHING and skips the
                        // move for this PC.
                        self.hero_speaking(
                            assets,
                            *pc_id,
                            crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                        );
                        continue;
                    }
                };
                // Launch a Move sequence element.  Going through the
                // sequence pipeline — rather than a direct
                // `pathfinder.add_request` shortcut — means the element
                // hits `arbitrate_instruct` when it transitions
                // Todo → InProgress next hourglass.  Any pending Seek +
                // post-seek Take (from a prior scroll-pickup click) at
                // Normal priority is interrupted by the new Normal Move
                // via the NEXT_LEVEL cascade, cleanly tearing down the
                // pickup so it doesn't replay at the new destination.
                let mut move_elem = crate::sequence::SequenceElement::new_movement(
                    1,
                    crate::element::Command::Move,
                    Some(*pc_id),
                    player_group_move_action(run),
                );
                if let crate::sequence::SequenceElementData::Movement {
                    destination, layer, ..
                } = &mut move_elem.data
                {
                    *destination = snapped;
                    *layer = pc_effective_layer;
                }

                // Append a `SpeakHeroReachDestination` element after
                // the move and cap the sequence with any
                // posture-cleanup sub-elements the PC needs (re-equip
                // bow, re-crouch, re-enter HelpingClimb / Beggar,
                // demote trailing ShootBow to ShootBowOnce).  The PC's
                // `Instruct` override terminates the Speak element on
                // dispatch and queues the HERO_DONE_COMMAND bark
                // (handled by `arbitrate_instruct`).
                let mut seq = crate::sequence::Sequence::new();
                seq.append_element(move_elem);
                if owner_is_pc {
                    append_arrival_speech(&mut seq, *pc_id);
                }
                self.append_posture_recovery(*pc_id, &mut seq);
                self.launch_sequence(seq);
                if show_marker && !is_door_click {
                    self.feedback
                        .ground_mark
                        .add_mark(snapped.x, snapped.y, pc_effective_layer);
                }
                continue;
            }

            if pc_goal_sector.is_none() && !is_door_click {
                tracing::warn!("skipping cross-sector move without resolved goal sector");
                continue;
            };

            // PerformGroupMove resolves every formation slot through
            // FindAuthorizedPosition before it builds a per-PC gate route.
            // This is also required for a single PC clicking a lift: the
            // authored click can be shifted slightly so the upright move box
            // fits inside the narrow wall/ladder rail.
            let resolved_dest = if bypass_formation_authorization || mercenary_center.is_some() {
                *dest
            } else {
                let Some(resolved) = self.authorize_group_move_destination(
                    *pc_id,
                    *dest,
                    effective_click,
                    pc_effective_layer,
                    is_lift_click,
                ) else {
                    self.hero_speaking(
                        assets,
                        *pc_id,
                        crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                    );
                    continue;
                };
                resolved
            };

            // Cross-sector: try gate A*
            let pc_pos_raw = positions
                .iter()
                .find(|(id, _, _, _)| *id == *pc_id)
                .map(|(_, p, _, _)| *p)
                .unwrap_or(*dest);

            // Source adaptation: if the PC is currently straddling a
            // gate, use the gate's far-side point / sector as the path
            // source.  Without this, the pathfinder starts from inside
            // the door sector, which is not a motion area and yields no
            // valid seed gates.
            let (door_handle, door_direction) = self
                .get_entity(*pc_id)
                .map(current_door_for_route_source)
                .unwrap_or((crate::position_interface::DoorHandle::NULL, false));
            let (pc_pos, path_src_sector, _path_src_layer) = {
                let adapted = self.scripts.mission.as_ref().and_then(|_| {
                    adapt_source_to_current_door_with_identity(
                        &self.script_domains.interactables.doors,
                        door_handle,
                        door_direction,
                    )
                });
                match adapted {
                    Some((adj, sector, layer)) => (adj, sector, layer),
                    None => (pc_pos_raw, *src_sector, *pc_src_layer),
                }
            };

            // Door-click routing: when the click lands on a door
            // sector with a known `door_index`, use
            // `find_path_to_door` and `GoalShape::Door` so the trailing
            // emission walks the PC up to the door's near-side (and
            // CHANGE_POSITION-teleports into buildings, turns the PC to
            // face the lock for lockpicks, etc.).
            let door_goal = if is_door_click {
                clicked_door_index
            } else {
                None
            };

            // PC authorisation for the gate A*.  Click-to-move never
            // sets `MoveFlags::MAP`, so `allow_leave_map = false` here.
            let pc_auth = self.get_entity(*pc_id).map(|e| e.actor_auth_info());
            let level = self.world.fast_grid.level.clone();
            let door_goal_info = door_goal.and_then(|door_idx| {
                self.scripts.mission.as_ref().and_then(|_| {
                    let path = crate::gate::find_path_into_door_with_sector_index(
                        &self.script_domains.interactables.doors,
                        (pc_pos.x, pc_pos.y),
                        u16::from(path_src_sector),
                        path_src_sector.arena_index(),
                        crate::gate::DoorIndex(door_idx),
                        pc_auth.as_ref(),
                        false,
                        &|sector| self.building_sector_is_authorized(sector),
                        &|sector| {
                            level
                                .sectors
                                .iter()
                                .find(|candidate| candidate.sector_number == sector)
                                .and_then(|candidate| candidate.lift_type)
                        },
                    )?;
                    let terminal = path
                        .last()
                        .copied()
                        .expect("path into a door must contain the goal door");
                    assert_eq!(
                        terminal.door_index,
                        crate::gate::DoorIndex(door_idx),
                        "path into door {door_idx} ended at {}",
                        terminal.door_index
                    );
                    let door = self
                        .script_domains
                        .interactables
                        .doors
                        .get(usize::from(terminal.door_index))
                        .expect("terminal door path index must resolve");
                    let (point, sector, layer) = if terminal.direct {
                        (door.point_out, door.sector_out, door.layer_out)
                    } else {
                        (door.point_in, door.sector_in, door.layer_in)
                    };
                    Some((door_idx, path, (point.x, point.y), u16::from(sector), layer))
                })
            });

            let door_far_side_is_building = door_goal_info.as_ref().map(|(_, _, _, sector, _)| {
                self.grid_sector_by_number(crate::sector::SectorNumber::new(*sector as i16))
                    .map(|gs| gs.sector_type.is_building())
                    .unwrap_or(false)
            });

            let mut recorded_routes_for_actor = recorded_gate_routes
                .iter()
                .filter(|(actor, _)| actor == pc_id);
            let recorded_gate_path = recorded_routes_for_actor.next().map(|(_, gates)| {
                assert!(
                    recorded_routes_for_actor.next().is_none(),
                    "recorded group move contains duplicate gate routes for {pc_id:?}"
                );
                assert!(
                    !gates.is_empty(),
                    "recorded successful gate route for {pc_id:?} is empty"
                );
                gates
                    .iter()
                    .map(|&(gate_id, direct)| {
                        let door_index = crate::gate::DoorIndex(gate_id);
                        self.script_domains
                            .interactables
                            .doors
                            .get(usize::from(door_index))
                            .unwrap_or_else(|| {
                                panic!(
                                    "recorded group-move gate {gate_id} for {pc_id:?} is absent from the Rust mission"
                                )
                            });
                        crate::gate::GatePathStep { door_index, direct }
                    })
                    .collect::<Vec<_>>()
            });
            let recorded_route_failed = recorded_failed_gate_routes
                .iter()
                .filter(|actor| *actor == pc_id)
                .count();
            let recorded_route_result =
                recorded_group_move_route_result(*pc_id, recorded_gate_path, recorded_route_failed);

            let path = if let Some(recorded) = recorded_route_result {
                recorded
            } else if door_goal_info.is_some() {
                door_goal_info.as_ref().map(|(_, p, _, _, _)| p.clone())
            } else {
                let Some(goal_sector) = pc_goal_sector else {
                    // This is the same failed route-construction outcome as
                    // FindPathIntoDoor / FindPathGates returning false in
                    // RHSequence::AppendMoveToSequence.  Original reports
                    // every such failure through the authoritative unable
                    // bark before abandoning the new sequence.
                    tracing::warn!(
                        actor = ?pc_id,
                        "skipping gate path without resolved goal sector"
                    );
                    self.hero_speaking(
                        assets,
                        *pc_id,
                        crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                    );
                    continue;
                };
                let level = self.world.fast_grid.level.clone();
                self.scripts.mission.as_ref().and_then(|_| {
                    find_group_move_gate_path(
                        &self.script_domains.interactables.doors,
                        *pc_id,
                        pc_pos,
                        path_src_sector,
                        resolved_dest,
                        goal_sector,
                        pc_goal_sector_index,
                        pc_effective_layer,
                        pc_auth.as_ref(),
                        &|sector| self.building_sector_is_authorized(sector),
                        &|sector| {
                            level
                                .sectors
                                .iter()
                                .find(|candidate| candidate.sector_number == sector)
                                .and_then(|candidate| candidate.lift_type)
                        },
                    )
                })
            };

            match path {
                Some(gate_steps) => {
                    tracing::info!(
                        "Gate A* from sector {} to sector {}: {} gates{}",
                        src_sector,
                        pc_goal_sector
                            .map(u16::from)
                            .unwrap_or_else(|| u16::from(*src_sector)),
                        gate_steps.len(),
                        if door_goal.is_some() {
                            " (door goal)"
                        } else {
                            ""
                        },
                    );
                    let goal_shape = if let Some((door_idx, _, pt, _sector, layer)) = door_goal_info
                    {
                        GoalShape::Door {
                            door_index: crate::gate::DoorIndex(door_idx),
                            far_side_point: MapPoint::new(pt.0, pt.1),
                            far_side_layer: layer,
                            far_side_is_building: door_far_side_is_building.unwrap_or(false),
                        }
                    } else {
                        GoalShape::Point {
                            point: resolved_dest,
                            tolerance: 0.0,
                        }
                    };
                    let _ = self.build_gate_movement_sequence(
                        sim,
                        *pc_id,
                        Some(path_src_sector),
                        gate_steps,
                        goal_shape,
                        pc_effective_layer,
                        player_group_move_action(run),
                        door_goal.is_none(),
                        1.0,
                        crate::sequence::MoveFlags::empty(),
                        Vec::new(),
                        Vec::new(),
                        owner_is_pc,
                        true,
                    );
                    if show_marker && !is_door_click {
                        self.feedback.ground_mark.add_mark(
                            resolved_dest.x,
                            resolved_dest.y,
                            pc_effective_layer,
                        );
                    }
                }
                None => {
                    // RHSequence::AppendMoveToSequence reports an
                    // unreachable cross-sector destination and returns
                    // without appending a direct MOVE when gate routing
                    // fails.
                    self.hero_speaking(
                        assets,
                        *pc_id,
                        crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                    );
                }
            }
        }

        // At the tail of group-move, if the click happened during
        // macro recording the messenger forwards `StopRecordingMacro`.
        // Routing through the messenger keeps the downstream
        // bookkeeping (QA HUD reset, macro-slot commit) consistent
        // with other stop points.
        if self.is_recording_macro() {
            self.orders.messenger.send(crate::messenger::Message::pc(
                crate::messenger::PcMessage::StopRecordingMacro,
                None,
            ));
        }
    }

    /// Search concentric rings for the nearest point inside a walkable
    /// motion area polygon on the given layer. Used when a click lands
    /// outside all sectors.
    pub(in crate::engine) fn snap_to_nearest_walkable(
        &self,
        assets: &LevelAssets,
        click: MapPoint,
        layer: u16,
    ) -> Option<MapPoint> {
        for radius_step in 1..=20u32 {
            let r = radius_step as f32 * 10.0;
            for dir in 0..16u32 {
                let angle = dir as f32 * std::f32::consts::FRAC_PI_8;
                let candidate = MapPoint::new(click.x + angle.sin() * r, click.y - angle.cos() * r);
                if assets
                    .pathfinder_graph
                    .find_area_at_point(layer as usize, candidate)
                    .is_some()
                {
                    return Some(candidate);
                }
            }
        }
        None
    }
}
