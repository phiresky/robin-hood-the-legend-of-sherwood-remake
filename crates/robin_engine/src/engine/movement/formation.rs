use super::*;
use crate::engine::TickCtx;

/// One actor's fully resolved movement quick action at click time.
///
/// The destination is the actor's authorized formation slot, while `route`
/// retains the exact sector identity that Original passes to the per-actor
/// movement call with recording enabled.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::engine) struct PlannedRecordedGroupMove {
    pub actor: EntityId,
    pub destination: MapPoint,
    pub route: crate::macro_store::RecordedQaMoveRoute,
}

/// A formation slot can fail the same move-box authorization as a live move.
/// Keep that outcome explicit so automatic capture can reject only that actor
/// without manufacturing a destination or mutating the simulation to play an
/// unable bark.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::engine) enum PlannedRecordedGroupMoveOutcome {
    Resolved(PlannedRecordedGroupMove),
    Unauthorized { actor: EntityId },
}

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

    /// Exact group-movement formation-slot authorization for one
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

    /// Authorize actor-indexed circle points before Original reverses the
    /// successful candidate list and performs shared nearest-slot assignment.
    fn authorized_circular_group_destinations(
        &self,
        pc_ids: &[EntityId],
        click: MapPoint,
        layer: u16,
        is_lift: bool,
        bypass_authorization: bool,
    ) -> (Vec<MapPoint>, Vec<bool>) {
        let raw_offsets = circular_dispatch_offsets(pc_ids.len());
        let mut eligible = vec![false; pc_ids.len()];
        let mut candidates = Vec::with_capacity(pc_ids.len());
        for (index, (&actor, offset)) in pc_ids.iter().zip(raw_offsets).enumerate() {
            let entity = self
                .get_entity(actor)
                .unwrap_or_else(|| panic!("selected group-move actor {actor:?} is missing"));
            let position = entity.position_iface();
            let actor_position = entity.element_data().position_map();
            // Preserve Original's bbox operation order exactly:
            // `box + [actor] + click + rotated - actor`. Collapsing these
            // translations changes the authorized center by an ULP.
            let mut bbox = if is_lift {
                position
                    .get_move_box()
                    .translated(actor_position)
                    .translated(MapVec::new(click.x, click.y))
                    .translated(offset)
                    .translated(MapVec::new(-actor_position.x, -actor_position.y))
            } else {
                position
                    .get_move_box_map()
                    .translated(MapVec::new(click.x, click.y))
                    .translated(offset)
                    .translated(MapVec::new(-actor_position.x, -actor_position.y))
            };
            let authorized = bypass_authorization
                || self
                    .world
                    .fast_grid
                    .find_authorized_position_toward(&mut bbox, click, layer);
            if authorized {
                eligible[index] = true;
                // Original-game front insertion reverses actor-indexed candidates.
                candidates.insert(0, bbox.center());
            }
        }
        (candidates, eligible)
    }

    /// Resolve a group click into per-PC quick-action movement records without
    /// launching orders, adding markers, speaking, or touching either QA
    /// store.
    ///
    /// The original game's group movement first computes and authorizes every
    /// mercenary/circular formation slot, then executes movement separately
    /// for each PC. The quick-action recording arm retains
    /// that actor-specific destination and exact goal sector/layer instead of
    /// launching movement. Automatic Shift queue
    /// capture uses this read-only boundary rather than arming the manual
    /// recorder or applying the nested live `GroupMove`.
    pub(in crate::engine) fn plan_recorded_group_move(
        &self,
        assets: &LevelAssets,
        pc_ids: &[EntityId],
        click_point: MapPoint,
        goal_override: Option<(crate::sector::SectorNumber, u16)>,
        goal_sector_index_override: Option<crate::fast_find_grid::SectorIndex>,
        door_route_override: Option<bool>,
    ) -> Vec<PlannedRecordedGroupMoveOutcome> {
        if pc_ids.is_empty() {
            return Vec::new();
        }

        let plan = self
            .group_move_click_plan(
                assets,
                pc_ids,
                click_point,
                goal_override,
                goal_sector_index_override,
                door_route_override,
                &[],
                &[],
            )
            .expect("nonempty group has a route-source position");
        let mut formation = self.group_move_formation_slots(pc_ids, None, &plan);
        let mut circular_destinations = vec![None; pc_ids.len()];
        if let Some((candidates, eligible)) = formation.circular_candidates.take() {
            dispatch_circular_candidates(
                &mut circular_destinations,
                candidates,
                &eligible,
                |_, index| {
                    self.expect_entity(pc_ids[index], "group-move actor")
                        .element_data()
                        .position_map()
                },
                |destinations, index, destination, contested| {
                    destinations[index] = Some((destination, contested));
                },
            );
        }

        pc_ids.iter().enumerate().map(|(index, &actor)| {
            let resolved = if formation.circular_destinations_pre_authorized {
                circular_destinations[index]
            } else {
                self.resolve_group_move_destination(&plan, &formation, index).map(|destination| (destination, false))
            };
            let Some((destination, contested)) = resolved else {
                return PlannedRecordedGroupMoveOutcome::Unauthorized { actor };
            };
            // Recording retains the jump's underlying sector and the formation
            // destination; live execution may instead approach a jump line.
            let route = if plan.is_jump_click {
                let (sector, sector_index, layer) = plan.jump_underlying_sector.unwrap_or_else(|| {
                    panic!("recorded jump group move for {actor:?} has no underlying goal sector")
                });
                recorded_qa_move_route(sector, sector_index, layer)
            } else if contested && let Some(index) = plan.selected_sector_index {
                let sector = &self.world.fast_grid.level.sectors[usize::from(index)];
                recorded_qa_move_route(sector.sector_number, index, plan.effective_layer)
            } else {
                recorded_qa_move_route(
                    plan.goal_sector.expect("recorded group move has no resolved goal sector"),
                    plan.route_goal_sector_index.expect("recorded group move has no exact goal-sector identity"),
                    plan.effective_layer,
                )
            };
            PlannedRecordedGroupMoveOutcome::Resolved(PlannedRecordedGroupMove { actor, destination, route })
        }).collect()
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
        tcx: TickCtx<'_>,
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
            tcx,
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
        tcx: TickCtx<'_>,
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
            tcx,
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

    pub(in crate::engine) fn perform_group_move_with_destinations(
        &mut self,
        tcx: TickCtx<'_>,
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
        let Some(plan) = self.group_move_click_plan(
            tcx.assets,
            pc_ids,
            click_point,
            goal_override,
            goal_sector_index_override,
            door_route_override,
            recorded_gate_routes,
            recorded_failed_gate_routes,
        ) else {
            return;
        };
        let mut formation = self.group_move_formation_slots(pc_ids, explicit_destinations, &plan);
        let circular_candidates = formation.circular_candidates.take();
        let ctx = GroupMoveRouteCtx {
            plan,
            formation,
            run,
            show_marker,
            recorded_gate_routes,
            recorded_failed_gate_routes,
        };

        // ── Per-PC routing ──
        // For each PC, decide between:
        //   1. Same-sector: simple MOVE
        //   2. Cross-sector (door/lift): gate-A* sequence
        if let Some((candidates, eligible)) = circular_candidates {
            // Speech only changes sound feedback, speech prohibitions, and
            // the chorus timer. It cannot affect another slot's geometry;
            // retain selection-order rejection before the first move runs.
            for (index, &authorized) in eligible.iter().enumerate() {
                if !authorized {
                    self.hero_speaking(
                        tcx.assets,
                        pc_ids[index],
                        crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                    );
                }
            }
            dispatch_circular_candidates(
                self,
                candidates,
                &eligible,
                |engine, index| {
                    engine
                        .expect_entity(pc_ids[index], "group-move actor")
                        .element_data()
                        .position_map()
                },
                |engine, index, destination, contested| {
                    let mut pc = engine.group_move_pc_route(&ctx, index, destination);
                    if contested && let Some(index) = ctx.plan.selected_sector_index {
                        let sector = &engine.world.fast_grid.level.sectors[usize::from(index)];
                        pc.pc_goal_sector = Some(sector.sector_number);
                        pc.pc_goal_sector_index = Some(index);
                    }
                    engine.dispatch_group_move_pc(tcx, &ctx, pc);
                },
            );
        } else {
            for dispatch_index in 0..pc_ids.len() {
                if let Some(pc) = self.group_move_pc_destination(tcx.assets, &ctx, dispatch_index) {
                    self.dispatch_group_move_pc(tcx, &ctx, pc);
                }
            }
        }

        // At the tail of group-move, if the click happened during
        // macro recording the messenger forwards `StopRecordingMacro`.
        // Routing through the messenger keeps the downstream
        // bookkeeping (QA HUD reset, macro-slot commit) consistent
        // with other stop points.
        if self.is_recording_macro() {
            self.forward_message(
                tcx,
                crate::messenger::Message::pc(
                    crate::messenger::PcMessage::StopRecordingMacro,
                    None,
                ),
            );
        }
    }

    fn dispatch_group_move_pc(
        &mut self,
        tcx: TickCtx<'_>,
        ctx: &GroupMoveRouteCtx<'_>,
        mut pc: GroupMovePcRoute,
    ) {
        if ctx.plan.is_jump_click && self.group_move_pc_jump(tcx, ctx, &mut pc).is_break() {
            return;
        }
        if self.group_move_pc_simple_route(tcx, ctx, &pc).is_break() {
            return;
        }
        if let Some(source) = self.group_move_pc_gate_source(tcx.assets, ctx, &pc) {
            self.group_move_pc_gate_route(tcx, ctx, &pc, source);
        }
    }

    /// Resolve the shared click using the first selected actor as its anchor.
    fn group_move_click_plan<'a>(
        &self,
        assets: &LevelAssets,
        pc_ids: &'a [EntityId],
        click_point: MapPoint,
        goal_override: Option<(crate::sector::SectorNumber, u16)>,
        goal_sector_index_override: Option<crate::fast_find_grid::SectorIndex>,
        door_route_override: Option<bool>,
        recorded_gate_routes: &[(EntityId, Vec<(u32, bool)>)],
        recorded_failed_gate_routes: &[EntityId],
    ) -> Option<GroupMoveClickPlan<'a>> {
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
        // when group movement constructs movement sequences; using the
        // actor's still-visible near-side sector here would incorrectly
        // classify a return click as a same-sector Move and lose the reverse
        // gate traversal before the command is postponed.
        let route_source = |pc_id| {
            let e = self
                .get_entity(pc_id)
                .unwrap_or_else(|| panic!("selected group-move actor {pc_id:?} is missing"));
            // The original game passes the actor's complete live sector reference
            // from sector lookup into movement / movement-sequence construction
            // for both formation paths. Restore
            // omitted legacy arena identity at this exact snapshot
            // boundary before same-sector classification or gate A*. A
            // selected live door remains authoritative and is resolved
            // first, while the actor may still display its old near-side
            // position.
            let (position, sector, layer) =
                group_move_route_source(self, pc_id, e, &self.script_domains.interactables.doors);
            (position, sector, layer)
        };
        let (reference, _, src_layer) = route_source(*pc_ids.first()?);

        // ── Unified sector hit-test ──
        //
        // Top-down layer search reconstructs the selected sector, whose sector
        // kind drives the door/lift/jump semantics below.  It is deliberately
        // independent of `goal_override`: original-game group movement can use
        // a patch's sector as the goal sector while the selected sector remains
        // the coincident mouse-selection overlay.
        // RecordGroupMove stores that patch-aware route goal, not necessarily
        // the selected sector, so replay must preserve both identities.
        let hit = self
            .world
            .fast_grid
            .get_sector_screen(click_point, reference);
        let selected_grid_sector = hit
            .sector_idx
            .and_then(|i| self.world.fast_grid.level.sectors.get(usize::from(i)));
        let GroupMoveClick {
            is_lift_click,
            is_jump_click,
            jump_underlying_sector,
            clicked_door_index,
            is_door_click,
            bypass_formation_authorization,
        } = self.classify_group_move_click(
            selected_grid_sector,
            click_point,
            goal_override,
            goal_sector_index_override,
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
        let all_source_arenas_match_spatial = hit.sector_idx.is_some()
            && pc_ids
                .iter()
                .all(|&actor| route_source(actor).1.arena_index() == hit.sector_idx);
        let retained_jump_falls_back_to_spatial = goal_override.is_some_and(|(goal, layer)| {
            retained_jump_goal_uses_underlying_sector(
                &self.world.fast_grid.level,
                goal,
                layer,
                hit.sector_idx,
            )
        });
        let legacy_collapsed_simple_route = legacy_unmapped_jump_goal_matches_spatial_source(
            assets.navigation.legacy_grid_topology.as_ref(),
            LegacyUnmappedJumpGoal {
                recorded_goal: goal_override,
                exact_goal_index: goal_sector_index_override,
                has_recorded_route_outcome: !recorded_gate_routes.is_empty()
                    || !recorded_failed_gate_routes.is_empty(),
                recorded_door_route: door_route_override,
                is_door_click: is_door_click,
                is_jump_click: is_jump_click,
                is_lift_click: is_lift_click,
                is_valid: hit.is_valid_for_move(&self.world.fast_grid),
                selected_sector_index: hit.sector_idx,
                selected_layer: hit.layer,
                retained_jump_falls_back_to_spatial: retained_jump_falls_back_to_spatial,
                all_source_arenas_match_spatial: all_source_arenas_match_spatial,
            },
        );

        let is_valid = goal_override.is_some() || hit.is_valid_for_move(&self.world.fast_grid);
        let (effective_click, effective_layer) = if goal_override.is_some() {
            (click_point, route_goal_layer)
        } else if is_valid || is_jump_click {
            (click_point, hit.layer)
        } else {
            (
                self.snap_to_nearest_walkable(assets, click_point, src_layer)
                    .unwrap_or(click_point),
                src_layer,
            )
        };
        Some(GroupMoveClickPlan {
            actor_ids: pc_ids,
            goal_sector: route_goal_sector,
            route_goal_sector_index,
            selected_sector_index: hit.sector_idx,
            effective_click,
            effective_layer,
            is_valid,
            is_lift_click,
            is_door_click,
            is_jump_click,
            clicked_jump_sector_idx: is_jump_click.then_some(hit.sector_idx).flatten(),
            jump_underlying_sector,
            clicked_door_index,
            bypass_formation_authorization,
            legacy_collapsed_simple_route,
        })
    }

    /// Formation slots around the click point: explicit destinations,
    /// mercenary formation, or authorized circular dispatch.
    fn group_move_formation_slots<'a>(
        &self,
        pc_ids: &[EntityId],
        explicit_destinations: Option<&'a [MapPoint]>,
        plan: &GroupMoveClickPlan,
    ) -> GroupMoveFormation<'a> {
        let GroupMoveClickPlan {
            effective_click,
            effective_layer,
            is_lift_click,
            bypass_formation_authorization,
            ..
        } = *plan;

        // ── Compute formation slots around the click point ──
        //
        // If the group is compact enough, use mercenary formation
        // (preserve relative positions).  Otherwise use circular
        // dispatch (arrange in a circle around click).
        let pc_positions: Vec<MapPoint> = pc_ids
            .iter()
            .map(|pc_id| {
                self.get_entity(*pc_id)
                    .unwrap_or_else(|| panic!("selected group-move actor {pc_id:?} is missing"))
                    .element_data()
                    .position_map()
            })
            .collect();
        let mut circular_destinations_pre_authorized = false;
        let mut circular_candidates = None;
        let (mercenary_center, dests) = if let Some(destinations) = explicit_destinations {
            (None, destinations)
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
            if uses_mercenary_group_formation(&pc_positions) {
                (Some(MapPoint::new(cx, cy)), &[][..])
            } else {
                circular_destinations_pre_authorized = true;
                circular_candidates = Some(self.authorized_circular_group_destinations(
                    pc_ids,
                    effective_click,
                    effective_layer,
                    is_lift_click,
                    bypass_formation_authorization,
                ));
                (None, &[][..])
            }
        };
        GroupMoveFormation {
            mercenary_center,
            dests,
            circular_candidates,
            circular_destinations_pre_authorized,
        }
    }

    /// Per-PC formation destination and compact-group move-box
    /// authorization. `None` means this PC is skipped (the loop continues).
    fn group_move_pc_destination(
        &mut self,
        assets: &LevelAssets,
        ctx: &GroupMoveRouteCtx<'_>,
        dispatch_index: usize,
    ) -> Option<GroupMovePcRoute> {
        let plan = &ctx.plan;
        let pc_id = plan.actor_ids[dispatch_index];
        let Some(dest) = self.resolve_group_move_destination(plan, &ctx.formation, dispatch_index)
        else {
            self.hero_speaking(
                assets,
                pc_id,
                crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
            );
            return None;
        };
        Some(self.group_move_pc_route(ctx, dispatch_index, dest))
    }

    fn group_move_pc_route(
        &self,
        ctx: &GroupMoveRouteCtx<'_>,
        dispatch_index: usize,
        dest: MapPoint,
    ) -> GroupMovePcRoute {
        let plan = &ctx.plan;
        let pc_id = plan.actor_ids[dispatch_index];
        let entity = self.expect_entity(pc_id, "group-move actor");
        let (position, sector, layer) = group_move_route_source(
            self,
            pc_id,
            entity,
            &self.script_domains.interactables.doors,
        );
        GroupMovePcRoute {
            source: (pc_id, position, layer, sector),
            dest,
            owner_is_pc: self
                .get_entity(pc_id)
                .expect("selected group-move actor is missing")
                .is_pc(),
            pc_goal_sector: plan.goal_sector,
            pc_goal_sector_index: plan.route_goal_sector_index,
            pc_effective_layer: plan.effective_layer,
        }
    }

    /// Resolve compact slots from the actor's live box at dispatch time.
    /// Circular slots were authorized together before dispatch began.
    fn resolve_group_move_destination(
        &self,
        plan: &GroupMoveClickPlan,
        formation: &GroupMoveFormation,
        dispatch_index: usize,
    ) -> Option<MapPoint> {
        let Some(center) = formation.mercenary_center else {
            return Some(formation.dests[dispatch_index]);
        };
        let actor = plan.actor_ids[dispatch_index];
        let entity = self
            .get_entity(actor)
            .expect("selected group-move actor is missing");
        let position = entity.position_iface();
        let mut bbox = group_move_mercenary_box(
            *position.get_move_box_map(),
            *position.get_move_box(),
            entity.element_data().position_map(),
            center,
            plan.effective_click,
            plan.is_lift_click,
        );
        let authorized = plan.bypass_formation_authorization
            || self.world.fast_grid.find_authorized_position_toward(
                &mut bbox,
                plan.effective_click,
                plan.effective_layer,
            );
        authorized.then(|| bbox.center())
    }

    /// Jump-sector click: authorize the slot, then either record the QA
    /// seek, launch the line-jump approach, or fall back to the underlying
    /// motion sector. `Break` means this PC is done (the loop continues);
    /// `Continue` proceeds to the simple/gate routing with the (possibly
    /// replaced) goal sector and layer written back into `pc`.
    fn group_move_pc_jump(
        &mut self,
        tcx: TickCtx<'_>,
        ctx: &GroupMoveRouteCtx<'_>,
        pc: &mut GroupMovePcRoute,
    ) -> std::ops::ControlFlow<()> {
        let GroupMoveRouteCtx {
            plan:
                GroupMoveClickPlan {
                    effective_click,
                    is_lift_click,
                    is_door_click,
                    clicked_jump_sector_idx,
                    jump_underlying_sector,
                    ..
                },
            formation:
                GroupMoveFormation {
                    mercenary_center,
                    circular_destinations_pre_authorized,
                    ..
                },
            run,
            show_marker,
            ..
        } = *ctx;
        let (pc_id, pc_pos, _, src_sector) = &pc.source;
        let dest = &pc.dest;
        // The only path that falls through to the write-back below assigns
        // both goal-sector locals first; every other path returns `Break`.
        let pc_goal_sector;
        let pc_goal_sector_index;
        let mut pc_effective_layer = pc.pc_effective_layer;
        // Group movement authorizes each formation slot before
        // movement execution tests whether the selected jump is usable.
        // Keep the raw click through the jump-sector hit test, then
        // apply that same move-box authorization here; the coarse
        // nearest-walkable fallback is not equivalent near a jump
        // landing boundary.
        let resolved_jump_dest =
            if mercenary_center.is_some() || circular_destinations_pre_authorized {
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
                tcx.assets,
                *pc_id,
                crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
            );
            return std::ops::ControlFlow::Break(());
        };
        if self.players.qa_recording_for.contains(pc_id) {
            // Original's QA branch deliberately disables executable
            // jump-line construction and records a coordinate SEEK
            // against the selected jump's underlying sector.
            let (sector, sector_index, layer) = jump_underlying_sector.unwrap_or_else(|| {
                panic!("recorded jump group move for {pc_id:?} has no underlying goal sector")
            });
            self.record_resolved_group_move_step(
                *pc_id,
                resolved_jump_dest,
                run,
                recorded_qa_move_route(sector, sector_index, layer),
                tcx.assets,
            );
            return std::ops::ControlFlow::Break(());
        }
        let pc_pos = *pc_pos;
        let source_line_idx = self
            .get_nearest_jumpable_jump_line(
                *pc_id,
                u32::from(
                    clicked_jump_sector_idx
                        .unwrap_or_else(|| panic!("jump click missing selected jump sector index")),
                ),
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

            let source_line_sector_idx = source_line.sector_index.unwrap_or_else(|| {
                panic!("line-jump source line {source_line_idx} has no home sector")
            });
            let source_line_sector = self
                        .world
                        .fast_grid
                        .level
                        .sectors
                        .get(usize::from(source_line_sector_idx))
                        .unwrap_or_else(|| {
                            panic!(
                                "line-jump source line {source_line_idx} references missing sector index {}",
                                u32::from(source_line_sector_idx)
                            )
                        });
            let source_line_sector_number = source_line_sector.sector_number;
            let source_line_midpoint = source_line.get_middle_point();
            let approach_owner = line_jump_approach_owner(self, *pc_id);
            let mut tail_sequence = crate::sequence::Sequence::new();
            for element in build_line_jump_click_tail(
                *pc_id,
                player_group_move_action(run),
                source_line_idx,
                destination_line_idx,
                resolved_jump_dest,
                pc_effective_layer,
                1.0,
            ) {
                tail_sequence.append_element(element);
            }
            // Posture recovery remains owned by the selected PC
            // even when Original substitutes its carrier solely for
            // the routed source-line approach.
            self.append_posture_recovery(*pc_id, &mut tail_sequence);
            let tail = tail_sequence.elements;

            // The original game delegates the source-line approach
            // to line-movement sequence construction. A cross-sector approach
            // must therefore find a gate path and emit the complete
            // AssertPosition/gate route before the explicit jump and
            // post-jump click tail.  The old Rust path emitted one
            // direct LINE|TO_JUMP move here, which crossed active
            // motion blockers and queued an A* request absent from the
            // Original lifecycle.
            let source_and_line_are_same_sector =
                src_sector.arena_index() == Some(source_line_sector_idx);
            let gate_path = if source_and_line_are_same_sector {
                Some(Vec::new())
            } else {
                let approach_auth = self
                    .get_entity(approach_owner)
                    .map(|entity| entity.actor_auth_info());
                let level = &self.world.fast_grid.level;
                self.scripts.mission.as_ref().and_then(|_| {
                    find_group_move_gate_path(
                        &self.script_domains.interactables.doors,
                        approach_owner,
                        pc_pos,
                        *src_sector,
                        source_line_midpoint,
                        source_line_sector_number,
                        Some(source_line_sector_idx),
                        source_line.layer,
                        approach_auth.as_ref(),
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
            let Some(gate_path) = gate_path else {
                self.hero_speaking(
                    tcx.assets,
                    approach_owner,
                    crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                );
                return std::ops::ControlFlow::Break(());
            };
            self.launch_gate_movement_sequence(
                tcx,
                &mut Vec::new(),
                crate::engine::movement::GateRouteRequest {
                    entity_id: approach_owner,
                    source_sector: (!source_and_line_are_same_sector).then_some(*src_sector),
                    gate_path: gate_path,
                    goal: GoalShape::Line {
                        line_index: source_line_idx,
                        midpoint: source_line_midpoint,
                        tolerance: 0.0,
                    },
                    goal_layer: source_line.layer,
                    base_action: player_group_move_action(run),
                    move_after_last_door: true,
                    speed_factor: 1.0,
                    initial_flags: crate::sequence::MoveFlags::empty(),
                    prefix_elements: Vec::new(),
                    tail_elements: tail,
                    append_arrival_speech: false,
                    append_recovery: false,
                },
            )
            .unwrap_or_else(|| {
                panic!("line-jump route for {pc_id:?} could not build its movement sequence")
            });
            if show_marker && !is_door_click {
                self.feedback.ground_mark.add_mark(
                    resolved_jump_dest.x,
                    resolved_jump_dest.y,
                    pc_effective_layer,
                );
            }
            return std::ops::ControlFlow::Break(());
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
                tcx.assets,
                *pc_id,
                crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
            );
            return std::ops::ControlFlow::Break(());
        };
        pc.pc_goal_sector = pc_goal_sector;
        pc.pc_goal_sector_index = pc_goal_sector_index;
        pc.pc_effective_layer = pc_effective_layer;
        std::ops::ControlFlow::Continue(())
    }

    /// Same-sector (or unknown goal sector) simple MOVE. `Break` means this
    /// PC is done (the loop continues); `Continue` falls through to gate
    /// routing.
    fn group_move_pc_simple_route(
        &mut self,
        tcx: TickCtx<'_>,
        ctx: &GroupMoveRouteCtx<'_>,
        pc: &GroupMovePcRoute,
    ) -> std::ops::ControlFlow<()> {
        let GroupMoveRouteCtx {
            plan:
                GroupMoveClickPlan {
                    effective_click,
                    is_valid,
                    is_lift_click,
                    is_door_click,
                    bypass_formation_authorization,
                    legacy_collapsed_simple_route,
                    ..
                },
            formation:
                GroupMoveFormation {
                    mercenary_center,
                    circular_destinations_pre_authorized,
                    ..
                },
            run,
            show_marker,
            recorded_gate_routes,
            recorded_failed_gate_routes,
        } = *ctx;
        let (pc_id, _, pc_src_layer, src_sector) = &pc.source;
        let GroupMovePcRoute {
            ref dest,
            owner_is_pc,
            pc_goal_sector,
            pc_goal_sector_index,
            pc_effective_layer,
            ..
        } = *pc;
        let has_recorded_route_outcome = |pc_id: EntityId| {
            recorded_gate_routes
                .iter()
                .any(|(actor, _)| *actor == pc_id)
                || recorded_failed_gate_routes.contains(&pc_id)
        };
        // Same-sector or unknown goal sector: simple move
        if group_move_uses_simple_route(
            has_recorded_route_outcome(*pc_id),
            is_door_click,
            is_valid,
            pc_goal_sector,
            pc_goal_sector_index,
            pc_effective_layer,
            u16::from(*src_sector),
            src_sector.arena_index(),
            *pc_src_layer,
        ) || legacy_collapsed_simple_route
        {
            // Door clicks skip the walkable snap entirely.
            let snap_res = if bypass_formation_authorization
                || mercenary_center.is_some()
                || circular_destinations_pre_authorized
            {
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
                    // Failure to find an authorized position on the
                    // mercenary/same-sector path fires
                    // HERO_UNABLE_TO_DO_SOMETHING and skips the
                    // move for this PC.
                    self.hero_speaking(
                        tcx.assets,
                        *pc_id,
                        crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                    );
                    return std::ops::ControlFlow::Break(());
                }
            };
            if self.players.qa_recording_for.contains(pc_id) {
                self.record_resolved_group_move_step(
                        *pc_id,
                        snapped,
                        run,
                        recorded_qa_move_route(
                            pc_goal_sector.unwrap_or_else(|| {
                                panic!(
                                    "recorded group move for {pc_id:?} has no resolved goal sector"
                                )
                            }),
                            pc_goal_sector_index.unwrap_or_else(|| {
                                panic!(
                                    "recorded group move for {pc_id:?} has no exact goal-sector identity"
                                )
                            }),
                            pc_effective_layer,
                        ),
                        tcx.assets,
                    );
                return std::ops::ControlFlow::Break(());
            }
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
            // instruction handler terminates the Speak element on
            // dispatch and queues the HERO_DONE_COMMAND bark
            // (handled by `arbitrate_instruct`).
            let mut seq = crate::sequence::Sequence::new();
            seq.append_element(move_elem);
            if owner_is_pc {
                append_arrival_speech(&mut seq, *pc_id);
            }
            self.append_posture_recovery(*pc_id, &mut seq);
            self.launch_sequence(tcx, seq);
            if show_marker && !is_door_click {
                self.feedback
                    .ground_mark
                    .add_mark(snapped.x, snapped.y, pc_effective_layer);
            }
            return std::ops::ControlFlow::Break(());
        }
        std::ops::ControlFlow::Continue(())
    }

    /// Cross-sector prelude: resolved-goal check, slot authorization, QA
    /// recording, and door-straddle source adaptation. `None` means this PC
    /// is done (the loop continues).
    fn group_move_pc_gate_source(
        &mut self,
        assets: &LevelAssets,
        ctx: &GroupMoveRouteCtx<'_>,
        pc: &GroupMovePcRoute,
    ) -> Option<GroupMoveGateSource> {
        let GroupMoveRouteCtx {
            plan:
                GroupMoveClickPlan {
                    effective_click,
                    is_lift_click,
                    is_door_click,
                    bypass_formation_authorization,
                    ..
                },
            formation: GroupMoveFormation {
                mercenary_center, ..
            },
            run,
            ..
        } = *ctx;
        let (pc_id, source_position, pc_src_layer, src_sector) = &pc.source;
        let GroupMovePcRoute {
            ref dest,
            pc_goal_sector,
            pc_goal_sector_index,
            pc_effective_layer,
            ..
        } = *pc;

        if pc_goal_sector.is_none() && !is_door_click {
            tracing::warn!("skipping cross-sector move without resolved goal sector");
            return None;
        };

        // Group movement resolves every formation slot through
        // authorized-position search before it builds a per-PC gate route.
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
                return None;
            };
            resolved
        };

        if self.players.qa_recording_for.contains(pc_id) {
            self.record_resolved_group_move_step(
                *pc_id,
                resolved_dest,
                run,
                recorded_qa_move_route(
                    pc_goal_sector.unwrap_or_else(|| {
                        panic!("recorded group move for {pc_id:?} has no resolved goal sector")
                    }),
                    pc_goal_sector_index.unwrap_or_else(|| {
                        panic!(
                            "recorded group move for {pc_id:?} has no exact goal-sector identity"
                        )
                    }),
                    pc_effective_layer,
                ),
                assets,
            );
            return None;
        }

        // Cross-sector: try gate A*
        let pc_pos_raw = *source_position;

        // Source adaptation: if the PC is currently straddling a
        // gate, use the gate's far-side point / sector as the path
        // source.  Without this, the pathfinder starts from inside
        // the door sector, which is not a motion area and yields no
        // valid seed gates.
        let door_source = self
            .get_entity(*pc_id)
            .and_then(current_door_for_route_source);
        let (pc_pos, path_src_sector, _path_src_layer) = {
            let adapted = self.scripts.mission.as_ref().and_then(|_| {
                door_source.and_then(|(door_handle, door_direction)| {
                    adapt_source_to_current_door_with_identity(
                        &self.script_domains.interactables.doors,
                        door_handle,
                        door_direction,
                    )
                })
            });
            match adapted {
                Some((adj, sector, layer)) => (adj, sector, layer),
                None => (pc_pos_raw, *src_sector, *pc_src_layer),
            }
        };
        Some(GroupMoveGateSource {
            resolved_dest,
            pc_pos,
            path_src_sector,
        })
    }

    /// Door-goal resolution, recorded/searched gate path, and the gate
    /// movement order launch (or the unable bark when no route exists).
    fn group_move_pc_gate_route(
        &mut self,
        tcx: TickCtx<'_>,
        ctx: &GroupMoveRouteCtx<'_>,
        pc: &GroupMovePcRoute,
        source: GroupMoveGateSource,
    ) {
        let GroupMoveRouteCtx {
            plan:
                GroupMoveClickPlan {
                    is_door_click,
                    clicked_door_index,
                    ..
                },
            run,
            show_marker,
            recorded_gate_routes,
            recorded_failed_gate_routes,
            ..
        } = *ctx;
        let (pc_id, _, _, src_sector) = &pc.source;
        let GroupMovePcRoute {
            owner_is_pc,
            pc_goal_sector,
            pc_goal_sector_index,
            pc_effective_layer,
            ..
        } = *pc;
        let GroupMoveGateSource {
            resolved_dest,
            pc_pos,
            path_src_sector,
        } = source;

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
        let level = &self.world.fast_grid.level;
        let mut door_goal_info = door_goal.and_then(|door_idx| {
            self.scripts.mission.as_ref().and_then(|_| {
                let path = crate::gate::find_path_into_door_with_sector_index(
                    &self.script_domains.interactables.doors,
                    (pc_pos.x, pc_pos.y),
                    u16::from(path_src_sector),
                    path_src_sector.arena_index(),
                    crate::gate::DoorIndex::new(door_idx).expect("valid door index"),
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
                    crate::gate::DoorIndex::new(door_idx).expect("valid door index"),
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
                        let door_index = crate::gate::DoorIndex::new(gate_id).expect("valid door index");
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
            door_goal_info
                .as_mut()
                .map(|(_, path, _, _, _)| std::mem::take(path))
        } else {
            let Some(goal_sector) = pc_goal_sector else {
                // This is the same failed route-construction outcome as
                // a failed door-entry or gate-path search during
                // movement-sequence construction. The original game reports
                // every such failure through the authoritative unable
                // bark before abandoning the new sequence.
                tracing::warn!(
                    actor = ?pc_id,
                    "skipping gate path without resolved goal sector"
                );
                self.hero_speaking(
                    tcx.assets,
                    *pc_id,
                    crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                );
                return;
            };
            let level = &self.world.fast_grid.level;
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
                let goal_shape = if let Some((door_idx, _, pt, _sector, layer)) = door_goal_info {
                    GoalShape::Door {
                        door_index: crate::gate::DoorIndex::new(door_idx)
                            .expect("valid door index"),
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
                self.launch_gate_movement_order(
                    tcx,
                    &mut Vec::new(),
                    crate::engine::movement::GateRouteRequest {
                        entity_id: *pc_id,
                        source_sector: Some(path_src_sector),
                        gate_path: gate_steps,
                        goal: goal_shape,
                        goal_layer: pc_effective_layer,
                        base_action: player_group_move_action(run),
                        move_after_last_door: door_goal.is_none(),
                        speed_factor: 1.0,
                        initial_flags: crate::sequence::MoveFlags::empty(),
                        prefix_elements: Vec::new(),
                        tail_elements: Vec::new(),
                        append_arrival_speech: owner_is_pc,
                        append_recovery: true,
                    },
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
                // Original-game movement-sequence construction reports an
                // unreachable cross-sector destination and returns
                // without appending a direct MOVE when gate routing
                // fails.
                self.hero_speaking(
                    tcx.assets,
                    *pc_id,
                    crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                );
            }
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
                    .navigation
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

fn recorded_qa_move_route(
    sector: crate::sector::SectorNumber,
    sector_index: crate::fast_find_grid::SectorIndex,
    layer: u16,
) -> crate::macro_store::RecordedQaMoveRoute {
    crate::macro_store::RecordedQaMoveRoute {
        goal_sector: sector,
        goal_sector_index: sector_index,
        goal_layer: layer,
    }
}

/// Shared click classification produced by
/// [`EngineInner::group_move_click_plan`] and consumed by the per-PC routing
/// phases of [`EngineInner::perform_group_move_with_destinations`].
/// Transient per-call state (no serde: `SectorHandle` carries no serde derive
/// and this is never persisted).
struct GroupMoveClickPlan<'a> {
    actor_ids: &'a [EntityId],
    goal_sector: Option<crate::sector::SectorNumber>,
    route_goal_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    selected_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    effective_click: MapPoint,
    effective_layer: u16,
    is_valid: bool,
    is_lift_click: bool,
    is_door_click: bool,
    is_jump_click: bool,
    clicked_jump_sector_idx: Option<crate::fast_find_grid::SectorIndex>,
    jump_underlying_sector: Option<(
        crate::sector::SectorNumber,
        crate::fast_find_grid::SectorIndex,
        u16,
    )>,
    clicked_door_index: Option<u32>,
    bypass_formation_authorization: bool,
    legacy_collapsed_simple_route: bool,
}

/// Formation slots and dispatch order for one group move.
struct GroupMoveFormation<'a> {
    mercenary_center: Option<MapPoint>,
    dests: &'a [MapPoint],
    circular_candidates: Option<(Vec<MapPoint>, Vec<bool>)>,
    circular_destinations_pre_authorized: bool,
}

/// Everything the per-PC routing phases of
/// [`EngineInner::perform_group_move_with_destinations`] read. No serde: it
/// borrows the caller's recorded-route slices and is never persisted.
struct GroupMoveRouteCtx<'a> {
    plan: GroupMoveClickPlan<'a>,
    formation: GroupMoveFormation<'a>,
    run: bool,
    show_marker: bool,
    recorded_gate_routes: &'a [(EntityId, Vec<(u32, bool)>)],
    recorded_failed_gate_routes: &'a [EntityId],
}

/// One PC's resolved formation destination and route goal, flowing between
/// the per-PC routing phases (the jump phase may replace the goal).
struct GroupMovePcRoute {
    source: (
        EntityId,
        MapPoint,
        u16,
        crate::position_interface::SectorHandle,
    ),
    dest: MapPoint,
    owner_is_pc: bool,
    pc_goal_sector: Option<crate::sector::SectorNumber>,
    pc_goal_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    pc_effective_layer: u16,
}

/// Authorized gate-route destination and adapted path source for one PC.
struct GroupMoveGateSource {
    resolved_dest: MapPoint,
    pc_pos: MapPoint,
    path_src_sector: crate::position_interface::SectorHandle,
}

struct GroupMoveClick {
    is_lift_click: bool,
    is_jump_click: bool,
    jump_underlying_sector: Option<(
        crate::sector::SectorNumber,
        crate::fast_find_grid::SectorIndex,
        u16,
    )>,
    clicked_door_index: Option<u32>,
    is_door_click: bool,
    bypass_formation_authorization: bool,
}

impl EngineInner {
    fn classify_group_move_click(
        &self,
        selected_grid_sector: Option<&crate::fast_find_grid::GridSector>,
        click_point: MapPoint,
        goal_override: Option<(crate::sector::SectorNumber, u16)>,
        goal_sector_index_override: Option<crate::fast_find_grid::SectorIndex>,
        door_route_override: Option<bool>,
    ) -> GroupMoveClick {
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
        let exact_recorded_goal_is_non_door = goal_override.is_some()
            && goal_sector_index_override
                .and_then(|index| self.world.fast_grid.level.sectors.get(usize::from(index)))
                .is_some_and(|sector| !sector.sector_type.is_door());
        let suppress_spatial_door = group_move_masks_spatial_door_for_recorded_goal(
            exact_recorded_goal_is_non_door,
            door_route_override,
        );
        let spatial_clicked_door_index = (!suppress_spatial_door)
            .then(|| clicked_sector_door_index.or(clicked_polygon_door_index))
            .flatten();
        let spatial_is_door_click = !suppress_spatial_door
            && (is_door_click_sector || spatial_clicked_door_index.is_some());
        let (clicked_door_index, is_door_click, bypass_formation_authorization) =
            group_move_door_selection(
                spatial_clicked_door_index,
                spatial_is_door_click,
                door_route_override,
            );

        GroupMoveClick {
            is_lift_click,
            is_jump_click,
            jump_underlying_sector,
            clicked_door_index,
            is_door_click,
            bypass_formation_authorization,
        }
    }
}

#[cfg(test)]
mod shared_resolution_tests {
    use super::*;

    #[test]
    fn rejected_circular_slots_speak_in_selection_order_without_launching_moves() {
        use crate::coordinates::MoveBox;
        use crate::element::Posture;
        use crate::engine::test_support::actors::TestActor;

        let mut engine = EngineInner::new();
        engine.control.sim_config.amount_of_speaking = 8;
        engine.world.fast_grid_mut().size_map(16, 16);
        engine.world.fast_grid_mut().allocate_layers(1);
        // Both circle slots straddle this wall. The click lies on the wall,
        // so there is no authorized normal-side push toward the click.
        engine.world.fast_grid_mut().add_line(
            crate::fast_find_grid::GridLine::new(
                MapPoint::new(400.0, 300.0),
                MapPoint::new(400.0, 500.0),
                true,
            ),
            0,
        );
        let mut assets = LevelAssets::new();
        std::sync::Arc::make_mut(&mut assets.profile_manager)
            .characters
            .push(Default::default());
        let actors: Vec<_> = [100.0, 300.0]
            .into_iter()
            .map(|x| {
                let mut actor = TestActor::pc(Posture::Upright).build();
                actor
                    .element_data_mut()
                    .set_position_map(MapPoint::new(x, 100.0));
                actor
                    .element_data_mut()
                    .set_sector(crate::position_interface::SectorHandle::new(1));
                actor
                    .position_iface_mut()
                    .set_move_box(MoveBox::from_coords(-2.0, -2.0, 2.0, 2.0));
                actor
                    .position_iface_mut()
                    .set_map_position(MapPoint::new(x, 100.0));
                engine.add_test_entity(actor)
            })
            .collect();
        let click = MapPoint::new(400.0, 400.0);
        let goal = Some((crate::sector::SectorNumber::new(1), 0));
        let recorded = engine.plan_recorded_group_move(&assets, &actors, click, goal, None, None);
        assert!(recorded.iter().all(|outcome| matches!(
            outcome,
            PlannedRecordedGroupMoveOutcome::Unauthorized { .. }
        )));
        assert!(engine.feedback.sound_sim.pending_exclamations.is_empty());
        engine.perform_group_move(
            TickCtx::new(&crate::sim_rng::SimulationContext::with_seed(1), &assets),
            &actors,
            click,
            false,
            false,
            goal,
            None,
            None,
            &[],
            &[],
        );
        let speeches = &engine.feedback.sound_sim.pending_exclamations;
        assert_eq!(
            speeches.len(),
            1,
            "the first rejected actor suppresses later barks through the chorus timer"
        );
        assert_eq!(speeches[0].actor_id, actors[0].index());
        assert_eq!(
            speeches[0].exclamation_id,
            crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .next()
                .is_none()
        );
    }

    #[test]
    fn circular_equal_distance_claimants_keep_selection_order() {
        let mut dispatched = Vec::new();
        dispatch_circular_candidates(
            &mut dispatched,
            vec![MapPoint::new(0.0, 0.0), MapPoint::new(100.0, 0.0)],
            &[true, true],
            |_, index| MapPoint::new([-10.0, 10.0][index], 0.0),
            |dispatched, index, destination, _| dispatched.push((index, destination.x)),
        );
        assert_eq!(dispatched, [(0, 0.0), (1, 100.0)]);
    }

    #[test]
    fn circular_zero_distance_conflict_waits_for_another_candidate_to_dispatch() {
        let mut state = ([0.0, 0.0, 9.0], Vec::new());
        dispatch_circular_candidates(
            &mut state,
            [0.0, 10.0, 20.0].map(|x| MapPoint::new(x, 0.0)).to_vec(),
            &[true; 3],
            |(positions, _), index| MapPoint::new(positions[index], 0.0),
            |(positions, dispatched), index, destination, _| {
                dispatched.push((index, destination.x));
                if index == 2 {
                    positions[0] = -2.0;
                }
            },
        );
        assert_eq!(state.1, [(2, 10.0), (0, 0.0), (1, 20.0)]);
    }

    #[test]
    fn circular_contested_removal_resumes_at_second_remaining_candidate() {
        let mut dispatched = Vec::new();
        dispatch_circular_candidates(
            &mut dispatched,
            [0.0, 10.0, 20.0, 30.0]
                .map(|x| MapPoint::new(x, 0.0))
                .to_vec(),
            &[true; 4],
            |_, index| MapPoint::new([-2.0, -1.0, 10.0, 30.0][index], 0.0),
            |dispatched, index, destination, _| dispatched.push((index, destination.x)),
        );
        assert_eq!(dispatched, [(0, 0.0), (3, 30.0), (1, 10.0), (2, 20.0)]);
    }

    #[test]
    fn circular_dispatch_rereads_positions_and_retains_unclaimed_slot_history() {
        let mut state = (
            [-2.0, -1.0, 10.0, 30.0].map(|x| MapPoint::new(x, 0.0)),
            Vec::new(),
        );
        dispatch_circular_candidates(
            &mut state,
            [0.0, 10.0, 20.0, 30.0]
                .map(|x| MapPoint::new(x, 0.0))
                .to_vec(),
            &[true; 4],
            |(positions, _), index| positions[index],
            |(positions, dispatched), index, destination, contested| {
                dispatched.push((index, destination.x, contested));
                if index == 0 {
                    positions[1].x = 19.0;
                    positions[2].x = 21.0;
                }
            },
        );
        assert_eq!(
            state.1,
            [
                (0, 0.0, true),
                (3, 30.0, false),
                // The skipped slot retains actor 2's sole claim even after
                // both remaining actors now prefer the slot at x = 20.
                (2, 10.0, false),
                (1, 20.0, true),
            ]
        );
    }

    #[test]
    fn recorded_jump_slots_match_live_resolution_and_keep_circle_dispatch_order() {
        use crate::coordinates::MoveBox;
        use crate::element::Posture;
        use crate::engine::test_support::actors::TestActor;
        use crate::sector::{SectorNumber, SectorType};

        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(16, 16);
        engine.world.fast_grid_mut().allocate_layers(1);
        let mut sector = crate::fast_find_grid::GridSector {
            points: vec![
                MapPoint::new(0.0, 0.0),
                MapPoint::new(800.0, 0.0),
                MapPoint::new(800.0, 800.0),
                MapPoint::new(0.0, 800.0),
            ],
            bounding_box: MapBBox::from_corners(
                MapPoint::new(0.0, 0.0),
                MapPoint::new(800.0, 800.0),
            ),
            sector_type: SectorType::MOUSE | SectorType::MOTION | SectorType::AREA,
            layer: 0,
            sector_number: SectorNumber::new(1),
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
        };
        let underlying = crate::fast_find_grid::SectorIndex::new(
            engine.world.fast_grid_mut().add_sector(sector.clone(), 0),
        )
        .unwrap();
        sector.sector_number = SectorNumber::new(2);
        sector.sector_type = SectorType::MOUSE | SectorType::JUMP;
        sector.underlying_sector = Some(underlying);
        engine
            .world
            .fast_grid_mut()
            .level_mut()
            .jump_lines
            .push(crate::jump_line::JumpLine::new(
                MapPoint::new(380.0, 400.0),
                MapPoint::new(420.0, 400.0),
                0.0,
                0.0,
            ));
        sector
            .jump_line_indices
            .push(crate::jump_line::JumpLineIndex::new(0).unwrap());
        engine.world.fast_grid_mut().add_sector(sector, 0);
        let actors: Vec<_> = [300.0, 500.0]
            .into_iter()
            .map(|y| {
                let mut actor = TestActor::pc(Posture::Upright).build();
                actor
                    .element_data_mut()
                    .set_position_map(MapPoint::new(400.0, y));
                actor.element_data_mut().set_sector(Some(
                    crate::position_interface::SectorHandle::new(1)
                        .unwrap()
                        .with_arena_index(underlying),
                ));
                actor
                    .position_iface_mut()
                    .set_move_box(MoveBox::from_coords(-2.0, -2.0, 2.0, 2.0));
                actor
                    .position_iface_mut()
                    .set_map_position(MapPoint::new(400.0, y));
                engine.add_test_entity(actor)
            })
            .collect();
        let assets = LevelAssets::new();
        let click = MapPoint::new(400.0, 400.0);
        let goal = Some((SectorNumber::new(1), 0));
        let plan = engine
            .group_move_click_plan(
                &assets,
                &actors,
                click,
                goal,
                Some(underlying),
                Some(false),
                &[],
                &[],
            )
            .unwrap();
        assert!(plan.is_jump_click);
        let mut formation = engine.group_move_formation_slots(&actors, None, &plan);
        let (candidates, eligible) = formation.circular_candidates.take().unwrap();
        let mut dispatched = Vec::new();
        dispatch_circular_candidates(
            &mut dispatched,
            candidates,
            &eligible,
            |_, index| {
                engine
                    .expect_entity(actors[index], "formation actor")
                    .element_data()
                    .position_map()
            },
            |dispatched, index, destination, _| dispatched.push((index, destination)),
        );
        assert_eq!(
            dispatched
                .iter()
                .map(|(index, _)| *index)
                .collect::<Vec<_>>(),
            vec![1, 0]
        );
        let ctx = GroupMoveRouteCtx {
            plan,
            formation,
            run: false,
            show_marker: false,
            recorded_gate_routes: &[],
            recorded_failed_gate_routes: &[],
        };
        let recorded = engine.plan_recorded_group_move(
            &assets,
            &actors,
            click,
            goal,
            Some(underlying),
            Some(false),
        );
        for (index, outcome) in recorded.into_iter().enumerate() {
            let PlannedRecordedGroupMoveOutcome::Resolved(recorded) = outcome else {
                panic!("authorized circle slot must resolve");
            };
            let destination = dispatched
                .iter()
                .find(|(actor, _)| *actor == index)
                .unwrap()
                .1;
            let live = engine.group_move_pc_route(&ctx, index, destination);
            assert_eq!(recorded.actor, actors[index]);
            assert_eq!(recorded.destination.x.to_bits(), live.dest.x.to_bits());
            assert_eq!(recorded.destination.y.to_bits(), live.dest.y.to_bits());
            assert_eq!(recorded.route.goal_sector, SectorNumber::new(1));
            assert_eq!(recorded.route.goal_sector_index, underlying);
            assert_eq!(recorded.route.goal_layer, 0);
        }
        assert!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .next()
                .is_none()
        );
    }

    #[test]
    fn compact_dispatch_reads_live_boxes_while_circle_keeps_authorized_slots() {
        use crate::coordinates::MoveBox;
        use crate::element::Posture;
        use crate::engine::test_support::actors::TestActor;
        for (spacing, compact) in [(30.0, true), (130.0, false)] {
            let mut engine = EngineInner::new();
            let actors: Vec<_> = [100.0, 100.0 + spacing, 100.0 + 2.0 * spacing]
                .into_iter()
                .map(|x| {
                    let mut actor = TestActor::pc(Posture::Upright).build();
                    actor
                        .element_data_mut()
                        .set_position_map(MapPoint::new(x, 100.0));
                    actor
                        .element_data_mut()
                        .set_sector(crate::position_interface::SectorHandle::new(1));
                    actor
                        .position_iface_mut()
                        .set_move_box(MoveBox::from_coords(-2.0, -2.0, 2.0, 2.0));
                    actor
                        .position_iface_mut()
                        .set_map_position(MapPoint::new(x, 100.0));
                    engine.add_test_entity(actor)
                })
                .collect();
            let mut plan = engine
                .group_move_click_plan(
                    &LevelAssets::new(),
                    &actors,
                    MapPoint::new(400.0, 400.0),
                    Some((crate::sector::SectorNumber::new(1), 0)),
                    None,
                    None,
                    &[],
                    &[],
                )
                .unwrap();
            // Bypass authorization to isolate live slot sampling from geometry.
            plan.bypass_formation_authorization = true;
            let mut formation = engine.group_move_formation_slots(&actors, None, &plan);
            assert_eq!(formation.mercenary_center.is_some(), compact);
            let captured_circle = formation.circular_candidates.take();
            let resolve = |engine: &EngineInner| {
                if let Some((candidates, eligible)) = &captured_circle {
                    let positions: Vec<_> = actors
                        .iter()
                        .map(|&actor| {
                            engine
                                .expect_entity(actor, "formation actor")
                                .element_data()
                                .position_map()
                        })
                        .collect();
                    let mut destination = None;
                    dispatch_circular_candidates(
                        &mut destination,
                        candidates.clone(),
                        eligible,
                        |_, index| positions[index],
                        |destination, index, point, _| {
                            if index == 1 {
                                *destination = Some(point);
                            }
                        },
                    );
                    destination.unwrap()
                } else {
                    engine
                        .resolve_group_move_destination(&plan, &formation, 1)
                        .unwrap()
                }
            };
            let before = resolve(&engine);
            // A previous actor's synchronous move may alter the next actor's
            // live box before that actor is dispatched.
            engine
                .ent_mut(actors[1])
                .position_iface_mut()
                .set_move_box(MoveBox::from_coords(8.0, -2.0, 12.0, 2.0));
            engine
                .ent_mut(actors[1])
                .position_iface_mut()
                .set_map_position(MapPoint::new(100.0 + spacing, 100.0));
            let after = resolve(&engine);
            assert_eq!(after.y.to_bits(), before.y.to_bits());
            assert_eq!(after.x, before.x + if compact { 10.0 } else { 0.0 });
            if compact {
                assert!(formation.dests.is_empty());
            } else {
                assert_eq!(captured_circle.as_ref().unwrap().0.len(), 3);
            }
        }
    }
}
