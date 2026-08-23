use super::*;

impl EngineInner {
    // ─── Elevation-line crossing ──────────────────────────────────

    /// Find a projection-area sight obstacle on `layer` whose
    /// screen-space plane contains `pos`.
    ///
    /// Used by the elevation-line emergency fallbacks: iterate plane
    /// sectors in the spatial bucket at `(pos, layer 0)`, then keep
    /// the one whose attached sight obstacle's layer matches and whose
    /// screen-space sector plane contains the position.  We don't carry
    /// a plane-sector registry yet — but every plane sector wraps a
    /// single projection-area obstacle, so iterating projection-area
    /// obstacles directly gives the same answer.
    pub(in crate::engine) fn find_plane_obstacle_at(
        &self,
        assets: &LevelAssets,
        layer: u16,
        pos: MapPoint,
    ) -> Option<u16> {
        self.find_plane_obstacle_split(assets, layer, pos, pos)
    }

    /// Asymmetric variant used by the second-emergency probe in
    /// `cross_elevation_line`.  The bounding-box check is evaluated
    /// at the 2-units-ahead probe but the polygon containment check
    /// is at the actor's *current* map position.  In a band where the
    /// probe has left the current polygon but the actor has not, the
    /// old polygon is accepted.  Use `bbox_at` = probe and
    /// `polygon_at` = current map position to capture that.
    pub(in crate::engine) fn find_plane_obstacle_split(
        &self,
        assets: &LevelAssets,
        layer: u16,
        bbox_at: MapPoint,
        polygon_at: MapPoint,
    ) -> Option<u16> {
        for (oi, obs) in self.sight_obstacles(assets).iter_indexed() {
            if !obs.is_projection_area() {
                continue;
            }
            if obs.layer != layer {
                continue;
            }
            if !obs.box_projection.contains_point(bbox_at) {
                continue;
            }
            if !obs.contains_point_projection(polygon_at) {
                continue;
            }
            return Some(oi as u16);
        }
        None
    }

    pub(in crate::engine) fn crossed_elevation_obstacle(
        current: Option<u16>,
        left: Option<u16>,
        right: Option<u16>,
    ) -> Option<Option<u16>> {
        if current == left {
            Some(right)
        } else if current == right {
            Some(left)
        } else {
            None
        }
    }

    /// Elevation-bond crossing for a shipped mobile master. The C++ mobile
    /// owns the obstacle pointer, then propagates it to every masked child.
    /// Its multi-line branch accidentally iterates a zero counter; preserve
    /// that release behavior and only cross when this tick sees exactly one
    /// unique, non-origin elevation line.
    pub(in crate::engine) fn check_mobile_line_crossing(
        &mut self,
        assets: &LevelAssets,
        mobile_index: usize,
    ) {
        let (old_pos, new_pos, layer, increment, current) = {
            let mobile = self
                .world
                .mobile_elements
                .get(mobile_index)
                .unwrap_or_else(|| panic!("missing mobile {mobile_index}"));
            (
                mobile.old_position,
                mobile.position,
                mobile.layer,
                mobile.increment,
                mobile.obstacle,
            )
        };
        #[cfg(test)]
        LAST_MOBILE_CROSSING_INCREMENT.with(|observed| observed.set(Some(increment)));
        if old_pos == new_pos {
            return;
        }

        let mut indices = self
            .world
            .fast_grid
            .get_crossing_elevation_line_indices(layer, old_pos, new_pos);
        indices.dedup_by(|left_idx, right_idx| {
            let left = &self.world.fast_grid.level.lines[usize::from(*left_idx)];
            let right = &self.world.fast_grid.level.lines[usize::from(*right_idx)];
            (left.a == right.a && left.b == right.b) || (left.a == right.b && left.b == right.a)
        });
        indices.retain(|idx| {
            let line = &self.world.fast_grid.level.lines[usize::from(*idx)];
            let vector = line.b - line.a;
            let from_a = old_pos - line.a;
            vector.x * from_a.y - vector.y * from_a.x != 0.0
        });
        if indices.len() != 1 {
            return;
        }

        let line = &self.world.fast_grid.level.lines[usize::from(indices[0])];
        let mut next = Self::crossed_elevation_obstacle(
            current,
            line.left_obstacle_index,
            line.right_obstacle_index,
        )
        .flatten();
        if current != line.left_obstacle_index && current != line.right_obstacle_index {
            next = self.find_plane_obstacle_at(assets, layer, new_pos);
            if next.is_none() && increment != MapVec::ZERO {
                let probe = new_pos + increment.scale(2.0);
                next = self.find_plane_obstacle_split(assets, layer, probe, new_pos);
            }
            if next.is_none() {
                tracing::debug!(
                    mobile_index,
                    ?current,
                    left = ?line.left_obstacle_index,
                    right = ?line.right_obstacle_index,
                    "mobile crossed an illegal elevation bond with no projection-area fallback"
                );
                return;
            }
        }

        let sprite_ids = {
            let mobile = &mut self.world.mobile_elements[mobile_index];
            mobile.obstacle = next;
            mobile.sprite_ids.clone()
        };
        for sprite_id in sprite_ids {
            self.set_obstacle_and_material(assets, sprite_id, next);
        }
    }

    pub(in crate::engine) fn expand_move_box_for_command_extraction(bbox: MapBBox) -> MapBBox {
        if bbox.is_somewhere() {
            MapBBox::from_coords(
                bbox.x_min() - 0.5,
                bbox.y_min() - 0.5,
                bbox.x_max() + 0.5,
                bbox.y_max() + 0.5,
            )
        } else {
            bbox
        }
    }

    /// Apply the source-position extraction performed at the start of
    /// `RHElementActor::InstructOwner(RHCOMMAND_MOVE)`.
    ///
    /// `RHCOMMAND_SEEK` falls through that same arm in the Original, so this
    /// must run before RefreshSeek resolves an entity target or constructs a
    /// cross-sector route.  Keeping it at the later path-dispatch boundary
    /// skips the correction whenever Seek is consumed while building its
    /// replacement sequence.
    pub(in crate::engine) fn extract_move_instruction_owner(&mut self, owner: EntityId) -> bool {
        let (entity_layer, pf_idx, move_box_map) = {
            let entity =
                self.world.entities.get(owner).unwrap_or_else(|| {
                    panic!("RHCOMMAND_MOVE extraction owner {owner:?} disappeared")
                });
            let pi = entity.position_iface();
            let pf_idx = {
                let index = pi.get_pathfinder_index();
                if index == u16::MAX { 0 } else { index }
            };
            (
                entity.element_data().layer(),
                pf_idx,
                *pi.get_move_box_map(),
            )
        };

        if self
            .world
            .fast_grid
            .is_position_authorized(&move_box_map, entity_layer)
        {
            return true;
        }

        let capture_extraction = crate::movement_diagnostics::parity_movement_capture_active();
        let original_box = move_box_map;
        let mut box_element = Self::expand_move_box_for_command_extraction(move_box_map);
        let expanded_box = box_element;
        let expanded_motion_lines = capture_extraction
            .then(|| {
                self.world
                    .fast_grid
                    .get_active_motion_line_indices(entity_layer, &expanded_box)
                    .into_iter()
                    .map(|index| usize::from(index) as u32)
                    .collect()
            })
            .unwrap_or_default();
        let authorized = self
            .world
            .fast_grid
            .find_authorized_position(&mut box_element, entity_layer);
        let authorized_box = box_element;
        let authorized_motion_lines = capture_extraction
            .then(|| {
                self.world
                    .fast_grid
                    .get_active_motion_line_indices(entity_layer, &authorized_box)
                    .into_iter()
                    .map(|index| usize::from(index) as u32)
                    .collect()
            })
            .unwrap_or_default();
        let center = authorized.then(|| authorized_box.center());
        if let Some(center) = center {
            let source = MapPoint::new(center.x, center.y);
            let entity = self.get_entity_mut(owner).unwrap_or_else(|| {
                panic!("RHCOMMAND_MOVE extraction owner {owner:?} disappeared after lookup")
            });
            entity.position_iface_mut().set_map_position(source);
            let elem = entity.element_data_mut();
            elem.set_position_map(source);
            elem.update_grid_cell();
        }
        let corrected_position = authorized.then(|| {
            self.get_entity(owner)
                .unwrap_or_else(|| {
                    panic!("RHCOMMAND_MOVE extraction owner {owner:?} disappeared after correction")
                })
                .element_data()
                .position_map()
        });
        let sector = self
            .get_entity(owner)
            .and_then(|entity| entity.element_data().sector())
            .map(u16::from);
        if capture_extraction {
            crate::movement_diagnostics::record_parity_move_box_extraction(
                crate::movement_diagnostics::ParityMoveBoxExtraction {
                    entity: owner,
                    layer: entity_layer,
                    sector,
                    pathfinder_area: pf_idx,
                    original_box: original_box.into(),
                    expanded_box: expanded_box.into(),
                    expanded_motion_lines,
                    authorized,
                    authorized_box: authorized_box.into(),
                    authorized_motion_lines,
                    authorized_center: center.map(Into::into),
                    corrected_position: corrected_position.map(Into::into),
                },
            );
        }

        true
    }

    /// Swap an actor's sight-obstacle pointer to the opposite side of
    /// an elevation line it just crossed and update the footstep
    /// material + 3D plane projection.
    ///
    /// Given the line's stored left/right obstacle indices, flip the
    /// actor's `obstacle_index` to the other side.  The new obstacle
    /// is then routed through `set_obstacle_and_material` so the
    /// actor picks up the new top-plane and footstep material
    /// immediately.  Finally the sprite is reprojected from the new
    /// map position onto the new plane.
    ///
    /// When the actor's current obstacle matches neither side
    /// ("illegal bond crossing"), two emergency fallbacks run: walk
    /// the plane-sector registry at the actor's position for a
    /// containing plane, then if that misses retry at `pos + 2 *
    /// increment_map`.  Both are reproduced via
    /// [`Self::find_plane_obstacle_at`].
    ///
    /// `new_pos` is the actor's post-move map position. `increment_map`
    /// is a unit vector in the movement direction (used by the second
    /// emergency probe).
    pub(in crate::engine) fn cross_elevation_line(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        line_idx: crate::fast_find_grid::LineIndex,
        new_pos: MapPoint,
        increment_map: MapVec,
    ) {
        let line = match self.world.fast_grid.level.lines.get(usize::from(line_idx)) {
            Some(l) if l.is_elevation => l,
            _ => return,
        };
        let left = line.left_obstacle_index;
        let right = line.right_obstacle_index;

        let (current, layer) = match self.world.entities.get(entity_id) {
            Some(e) => (
                e.element_data().obstacle_index().map(u16::from),
                e.element_data().layer(),
            ),
            None => return,
        };

        let mut next: Option<u16>;
        let mut found = true;

        if let Some(crossed) = Self::crossed_elevation_obstacle(current, left, right) {
            // legacy implementation compares raw obstacle pointers here, so NULL is a
            // valid side of an elevation line and must cross to the
            // opposite side instead of falling into the emergency path.
            next = crossed;
        } else {
            // "VERBOTEN: Illegal bond crossing" — current obstacle
            // matches neither side.  Walk projection-area obstacles
            // for one containing the actor's current position on its
            // layer.
            tracing::debug!(
                entity = ?entity_id,
                ?current,
                ?left,
                ?right,
                "cross_elevation_line: obstacle pointer doesn't match either side (illegal bond crossing)"
            );
            next = self.find_plane_obstacle_at(assets, layer, new_pos);
            if next.is_none() {
                // "STRENG VERBOTEN" — second emergency: probe two
                // map units ahead in the movement direction.  Gated
                // on a real direction (non-zero `increment_map`) —
                // when `check_for_line_crossing` early-returns on a
                // zero-length step the probe never reaches us, but
                // if a future caller wires this with an unfilled
                // increment we skip the second emergency rather than
                // probing in the wrong direction.
                let increment_computed =
                    increment_map.x.abs() > 1e-9 || increment_map.y.abs() > 1e-9;
                if increment_computed {
                    let probe = MapPoint::new(
                        new_pos.x + 2.0 * increment_map.x,
                        new_pos.y + 2.0 * increment_map.y,
                    );
                    tracing::debug!(
                        entity = ?entity_id,
                        "cross_elevation_line: second emergency, probing 2 units ahead at ({:.1}, {:.1})",
                        probe.x,
                        probe.y,
                    );
                    // Asymmetric predicate: bbox at the probe point,
                    // polygon containment at the actor's current
                    // (post-move) position.
                    next = self.find_plane_obstacle_split(assets, layer, probe, new_pos);
                }
                if next.is_none() {
                    // "ABSOLUT VERBOTEN" — give up; leave the actor's
                    // obstacle alone and skip the reprojection.
                    tracing::debug!(
                        entity = ?entity_id,
                        "cross_elevation_line: no projection area found at ({:.1}, {:.1})",
                        new_pos.x,
                        new_pos.y,
                    );
                    found = false;
                }
            }
        }

        if !found {
            return;
        }

        // Apply the new obstacle: updates element_data.obstacle_index,
        // element_data.material (footstep sounds), and the actor's
        // PositionInterface (obstacle, top-plane, material).
        self.set_obstacle_and_material(assets, entity_id, next);

        // Reproject the sprite onto the new plane.  Per-frame
        // movement updates `element_data.position_map` directly
        // without touching `position_iface`, so seed
        // `position_iface.position_map` from the freshly moved
        // position before recomputing 3D.
        if let Some(entity) = self.get_entity_mut(entity_id) {
            let pi = entity.position_iface_mut();
            pi.set_map_position(crate::coordinates::MapPoint {
                x: new_pos.x,
                y: new_pos.y,
            });
        }
    }

    /// Per-tick line-crossing dispatch for a moving actor.
    ///
    /// Restricted to elevation-line crossings.  For each elevation
    /// line the actor's `(old_pos, new_pos)` segment crosses on its
    /// current layer, we swap the actor's obstacle pointer via
    /// `cross_elevation_line`.  When multiple elevation lines are
    /// crossed in one tick, they are bubble-sorted by obstacle
    /// continuity so consecutive `cross_elevation_line` calls walk an
    /// actual chain of adjacent obstacles.
    ///
    /// Returns `true` if any elevation line was crossed — callers can
    /// use that to fire the human-specific `UpdateRoll` follow-up.
    pub(in crate::engine) fn check_for_line_crossing(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        old_pos: MapPoint,
        new_pos: MapPoint,
        layer: u16,
    ) -> bool {
        // Early-out: exact same position means no crossing at all.
        if (old_pos.x - new_pos.x).abs() < 1e-4 && (old_pos.y - new_pos.y).abs() < 1e-4 {
            return false;
        }

        let indices = self
            .world
            .fast_grid
            .get_crossing_elevation_line_indices(layer, old_pos, new_pos);
        self.check_for_elevation_line_crossing_indices(
            assets, entity_id, old_pos, new_pos, layer, indices,
        )
    }

    /// Dispatch an already-filtered elevation subset from Actor's unified
    /// `LINE_CROSS` list. This keeps Original's candidate count and callback
    /// set on the same boundary.
    pub(in crate::engine) fn check_for_elevation_line_crossing_indices(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        old_pos: MapPoint,
        new_pos: MapPoint,
        layer: u16,
        mut indices: Vec<crate::fast_find_grid::LineIndex>,
    ) -> bool {
        tracing::trace!(
            target: "robin_engine::elevation_crossing",
            ?entity_id,
            layer,
            old_x = old_pos.x,
            old_y = old_pos.y,
            new_x = new_pos.x,
            new_y = new_pos.y,
            crossing_count = indices.len(),
            "queried elevation crossings"
        );
        if indices.is_empty() {
            return false;
        }

        // Read the actor's current obstacle — used as the seed for the
        // sort when multiple lines are crossed.
        let mut current_obstacle = match self.world.entities.get(entity_id) {
            Some(e) => e.element_data().obstacle_index().map(u16::from),
            None => return false,
        };

        // Bubble-sort elevation lines by obstacle continuity.  Each
        // iteration picks the next line whose left or right side
        // matches the running `current_obstacle`, swaps it into
        // place, and advances the running obstacle.  If no line
        // matches we stop sorting — later indices will still be
        // dispatched in whatever order they came out of the grid.
        let n = indices.len();
        if n > 1 {
            for i in 0..n.saturating_sub(1) {
                let mut matched = false;
                for j in i..n {
                    let line = match self
                        .world
                        .fast_grid
                        .level
                        .lines
                        .get(usize::from(indices[j]))
                    {
                        Some(l) => l,
                        None => continue,
                    };
                    if line.left_obstacle_index == current_obstacle {
                        current_obstacle = line.right_obstacle_index;
                        indices.swap(i, j);
                        matched = true;
                        break;
                    }
                    if line.right_obstacle_index == current_obstacle {
                        current_obstacle = line.left_obstacle_index;
                        indices.swap(i, j);
                        matched = true;
                        break;
                    }
                }
                if !matched {
                    break;
                }
            }
        }

        // Compute the unit movement vector for the second-emergency
        // probe inside `cross_elevation_line`.
        let increment_map = {
            let dx = new_pos.x - old_pos.x;
            let dy = new_pos.y - old_pos.y;
            let len = (dx * dx + dy * dy).sqrt();
            if len > 1e-6 {
                MapVec::new(dx / len, dy / len)
            } else {
                MapVec::ZERO
            }
        };

        // Dispatch the swaps in order.
        for &idx in &indices {
            self.cross_elevation_line(assets, entity_id, idx, new_pos, increment_map);
        }

        true
    }

    /// Dispatch the Original actor `CheckForLineCrossing` non-elevation tail.
    ///
    /// Patch, script, and sound lines share one candidate list and one stable
    /// distance sort. For each line the Original checks those flags in that
    /// order, so callbacks from different boundary kinds remain interleaved
    /// by the actor's travel order rather than grouped by kind.
    pub(in crate::engine) fn check_for_non_elevation_line_crossing(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        old_pos: MapPoint,
        new_pos: MapPoint,
        layer: u16,
    ) {
        if old_pos == new_pos {
            return;
        }
        let indices = self
            .world
            .fast_grid
            .get_actor_non_elevation_crossing_line_indices(layer, old_pos, new_pos);
        self.check_for_non_elevation_line_crossing_indices(
            sim, assets, entity_id, old_pos, new_pos, indices,
        );
    }

    pub(in crate::engine) fn check_for_non_elevation_line_crossing_indices(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        old_pos: MapPoint,
        new_pos: MapPoint,
        indices: Vec<crate::fast_find_grid::LineIndex>,
    ) {
        if indices.is_empty() {
            return;
        }

        let movement = crate::geo2d::segment(old_pos.to_geo(), new_pos.to_geo());
        let mut crossed: Vec<(f32, crate::fast_find_grid::LineIndex)> = indices
            .into_iter()
            .filter_map(|line_index| {
                let line = &self.world.fast_grid.level.lines[usize::from(line_index)];
                let old_dx = old_pos.x - line.a.x;
                let old_dy = old_pos.y - line.a.y;
                let line_dx = line.b.x - line.a.x;
                let line_dy = line.b.y - line.a.y;
                if line_dx * old_dy - line_dy * old_dx == 0.0 {
                    return None;
                }
                let point = crate::geo2d::segment_intersection(movement, line.segment()).point()?;
                let dx = point.x - old_pos.x;
                let dy = point.y - old_pos.y;
                Some((dx * dx + dy * dy, line_index))
            })
            .collect();
        crossed.sort_by(|(left, _), (right, _)| left.total_cmp(right));

        let is_pc = self
            .get_entity(entity_id)
            .unwrap_or_else(|| panic!("line-crossing actor {entity_id} is missing"))
            .is_pc();
        for (_, line_index) in crossed {
            let (is_patch, is_script, is_sound) = {
                let line = &self.world.fast_grid.level.lines[usize::from(line_index)];
                (line.is_patch, line.is_script, line.is_sound)
            };
            if is_patch && is_pc {
                self.dispatch_patch_line_crossing(sim, assets, entity_id, new_pos, line_index);
            }
            if is_script {
                self.dispatch_script_line_crossing(sim, assets, entity_id, new_pos, line_index);
            }
            if is_sound {
                self.dispatch_sound_line_crossing(assets, entity_id, new_pos, line_index);
            }
        }
    }

    /// Close Original `RHElementActor::Hourglass`'s post-`Execute`
    /// `CheckForLineCrossing` boundary for an actor whose selected Execute arm
    /// moved it without going through the movement owner.
    ///
    /// Original collects every `LINE_CROSS` candidate once. With exactly one
    /// line it recomputes the actor increment only when that line is an
    /// elevation bond; with multiple lines it unconditionally runs the shared
    /// recompute block, even when every candidate is non-elevation. Keep that
    /// observable branch shape: corpse placement can cross coincident sound
    /// and script/patch boundaries while initializing a generic dying order.
    pub(in crate::engine) fn dispatch_actor_post_execute_line_crossing(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        entry_compute_direction: Option<bool>,
    ) {
        #[cfg(test)]
        observe_post_execute_crossing(self, entity_id);
        let (old_pos, new_pos, layer, posture, is_carried, is_human) = {
            let entity =
                self.world.entities.get(entity_id).unwrap_or_else(|| {
                    panic!("post-Execute crossing owner {entity_id:?} is missing")
                });
            (
                entity.position_iface().old_map_position(),
                entity.element_data().position_map(),
                entity.element_data().layer(),
                entity.element_data().posture,
                entity
                    .human_data()
                    .is_some_and(|human| human.carrier.is_some()),
                entity.is_human(),
            )
        };
        if old_pos == new_pos
            || !actor_line_crossing_eligible(
                posture,
                is_carried,
                self.world.fast_grid.level.map_bbox.contains_point(new_pos),
            )
        {
            return;
        }

        // Original obtains one SBListUnique<RHLine*> for LINE_CROSS, then
        // filters it once. Keep this exact list stable across callbacks: an
        // Enter/Leave may change line activity but cannot retroactively change
        // this Hourglass's branch or dispatch set.
        let crossing_indices = self
            .world
            .fast_grid
            .get_actor_crossing_line_indices(layer, old_pos, new_pos);
        let crossing_count = crossing_indices.len();
        if crossing_count == 0 {
            return;
        }
        let elevation_indices = crossing_indices
            .iter()
            .copied()
            .filter(|&line_index| {
                self.world.fast_grid.level.lines[usize::from(line_index)].is_elevation
            })
            .collect::<Vec<_>>();
        // In Original's single-line arm every flag on that one RHLine is
        // dispatched. In the multi-line arm elevation lines are grouped at
        // the front and excluded from the later patch/script/sound loop.
        let callback_indices = crossing_indices
            .into_iter()
            .filter(|&line_index| {
                crossing_count == 1
                    || !self.world.fast_grid.level.lines[usize::from(line_index)].is_elevation
            })
            .collect::<Vec<_>>();

        let crossed_elevation = self.check_for_elevation_line_crossing_indices(
            assets,
            entity_id,
            old_pos,
            new_pos,
            layer,
            elevation_indices,
        );
        if crossed_elevation || crossing_count > 1 {
            if is_human {
                self.update_roll_after_crossing(assets, entity_id);
            }
            // `mpOrder` is the entry-latched pointer in Original. Execute may
            // already have exhausted the Rust order deque, but Actor does not
            // call DoNextOrder until after this crossing boundary.
            if let Some(compute_direction) = entry_compute_direction
                && let Some(entity) = self.world.entities.get_mut(entity_id)
            {
                // Preserve PositionInterface's cached-computation contract:
                // Original calls ComputeIncrementAll here without forcibly
                // clearing its flags. Elevation crossing/reprojection may
                // have invalidated them; an otherwise cached vector remains
                // authoritative.
                entity
                    .position_iface_mut()
                    .compute_increment_all(compute_direction);
            }
        }

        self.check_for_non_elevation_line_crossing_indices(
            sim,
            assets,
            entity_id,
            old_pos,
            new_pos,
            callback_indices,
        );
    }

    /// Per-tick `LINE_PATCH` crossing dispatch for a PC.
    ///
    /// On crossing a LINE_PATCH line:
    ///
    /// ```text
    ///   if patch is active:
    ///       if patch.apply_sector contains GetPositionMap():
    ///           patch.Enter(actor)
    ///           if !patch.is_applied: patch.Apply()
    ///       else:
    ///           patch.Leave(actor)
    ///           if patch.is_applied && patch.any_occupant().is_none():
    ///               patch.Apply()
    /// ```
    ///
    /// Uses the PC's `new_pos` as the post-move probe.  `inside` means
    /// the PC just entered the apply polygon, `outside` means the PC
    /// just left it.  Patch state machine, FX entity, sight obstacles,
    /// grid sectors, and door rights are updated via
    /// `process_patch_effects`.
    pub(in crate::engine) fn dispatch_patch_line_crossing(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        new_pos: MapPoint,
        line_index: crate::fast_find_grid::LineIndex,
    ) {
        let occupant = crate::patch::OccupantId(entity_id.index());

        // `Patch::enter` / `leave` recurse onto the actor's carried
        // entity when the actor is a PC and is currently carrying
        // someone.  Resolve that here once (same entity for every
        // crossed patch this tick) so each per-patch Enter/Leave can
        // mirror the recursion. The combined dispatcher gates this arm to
        // PCs, matching CheckForLineCrossing.
        let carried_occupant = self
            .get_entity(entity_id)
            .and_then(|e| match e {
                crate::element::Entity::Pc(pc) => pc.pc.carried,
                _ => None,
            })
            .map(|cid| crate::patch::OccupantId(cid.index()));

        let patch_index = self.world.fast_grid.level.lines[usize::from(line_index)]
            .patch_index
            .unwrap_or_else(|| panic!("LINE_PATCH {line_index:?} has no owning patch"));
        // Snapshot the apply-sector polygon test result + active state before
        // mutating the patch, preserving is_active → is_inside → Enter/Leave
        // → conditional Apply.
        let patch_usize = patch_index.get() as usize;
        let Some(patch) = self.script_domains.interactables.patches.get(patch_usize) else {
            return;
        };
        if !patch.is_active() {
            return;
        }
        let Some(apply_sector_idx) = patch.apply_sector_index else {
            tracing::warn!(
                patch = %patch_index,
                "LINE_PATCH crossing on patch with no apply sector — skipping",
            );
            return;
        };
        let Some(apply_sector) = self
            .world
            .fast_grid
            .level
            .sectors
            .get(apply_sector_idx as usize)
        else {
            return;
        };
        let inside_apply = apply_sector.contains_point(new_pos);

        let effects = {
            let Some(patch) = self
                .script_domains
                .interactables
                .patches
                .get_mut(patch_usize)
            else {
                return;
            };
            if inside_apply {
                patch.enter(occupant);
                if let Some(carried) = carried_occupant {
                    patch.enter(carried);
                }
                if !patch.is_applied() {
                    patch.apply()
                } else {
                    Vec::new()
                }
            } else {
                patch.leave(occupant);
                if let Some(carried) = carried_occupant {
                    patch.leave(carried);
                }
                if patch.is_applied() && patch.any_occupant().is_none() {
                    patch.apply()
                } else {
                    Vec::new()
                }
            }
        };

        if !effects.is_empty() {
            self.process_patch_effects(sim, assets, patch_index, effects);
        }
    }

    /// Per-tick LINE_SOUND crossing dispatch for a moving actor.
    ///
    /// When the actor's `(old_pos, new_pos)` segment crosses one or
    /// more active LINE_SOUND grid lines on its current layer,
    /// refresh `actor.material` from the new ground material via
    /// [`MaterialSectors::material_at`] (which combines the
    /// "is-inside material polygon" test with the obstacle /
    /// default-material fallback in a single call).
    ///
    /// Updates both `ElementData::material` (read by footstep sound
    /// playback) and the actor's `PositionInterface` material so
    /// subsequent reads match.
    pub(in crate::engine) fn dispatch_sound_line_crossing(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        new_pos: MapPoint,
        line_index: crate::fast_find_grid::LineIndex,
    ) {
        let obstacle_material = self
            .get_entity(entity_id)
            .and_then(|e| e.element_data().obstacle_index())
            .map(|handle| {
                let idx: usize = handle.into();
                self.sight_obstacles(assets)
                    .get(idx)
                    .unwrap_or_else(|| {
                        panic!(
                            "entity {} references missing sight obstacle {idx}",
                            entity_id.index()
                        )
                    })
                    .material
            })
            .map(|raw| crate::element::GameMaterial::from_u32(raw as u32))
            .unwrap_or(assets.material_sectors.default_material);

        let line = &self.world.fast_grid.level.lines[usize::from(line_index)];
        let raw_index = line
            .sound_material_sector_index
            .unwrap_or_else(|| panic!("LINE_SOUND {line_index:?} has no owning material sector"));
        let sector = assets
            .all_material_sectors
            .get(usize::from(raw_index))
            .and_then(Option::as_ref)
            .unwrap_or_else(|| {
                panic!("LINE_SOUND {line_index:?} references missing material sector {raw_index}")
            });
        let new_material = if sector.contains(new_pos) {
            sector.material
        } else {
            obstacle_material
        };

        if let Some(entity) = self.get_entity_mut(entity_id) {
            let prev = entity.element_data().material();
            if prev != new_material {
                entity.element_data_mut().set_material(new_material);
                let pi = entity.position_iface_mut();
                pi.set_material(new_material);
                tracing::trace!(
                    ?entity_id,
                    ?prev,
                    ?new_material,
                    ?line_index,
                    "dispatch_sound_line_crossing: refreshed material"
                );
            }
        }
    }
}
