//! Patch effect processing.
//!
//! When a patch is applied, reset, or finalized, the `Patch` state machine
//! produces a `Vec<PatchEffect>`. This module implements the engine-side
//! execution of those effects: toggling sight obstacles, grid sectors/lines,
//! pathfinder state, FX entity animations, background invalidation, and
//! door rights.

use super::*;
use crate::patch::{PatchAnimation, PatchEffect};

/// Snapshot of the patch-level data needed to process effects.
/// Extracted once before iterating effects to avoid repeated borrows.
struct PatchContext {
    door_indices: Vec<u32>,
    old_sight_obstacle_indices: Vec<crate::sight_obstacle::SightObstacleIndex>,
    new_sight_obstacle_indices: Vec<crate::sight_obstacle::SightObstacleIndex>,
    old_sector_indices: Vec<u32>,
    new_sector_indices: Vec<u32>,
    old_line_indices: Vec<crate::fast_find_grid::LineIndex>,
    new_line_indices: Vec<crate::fast_find_grid::LineIndex>,
    old_mask_indices: Vec<crate::mask::MaskIndex>,
    new_mask_indices: Vec<crate::mask::MaskIndex>,
    use_changing_obstacles: bool,
    pathfinder_layer: u16,
    pathfinder_sector: u16,
    pathfinder_changing_obstacles: u32,
    /// Actor script handle for the patch's FX animation entity, if any.
    animation_entity_handle: Option<i32>,
    /// Whether this patch's final frame should be baked into the background.
    integrate_in_background: bool,
}

impl EngineInner {
    /// Process a list of patch effects produced by `Patch::apply()`,
    /// `Patch::apply_final()`, or `Patch::force_reset()`.
    ///
    /// This is the central dispatch for all patch side effects. Called from:
    /// - `apply_door_patch` (door_pass.rs) — when an actor passes through a door
    /// - Deferred command processing (script.rs) — for script ApplyPatch/ResetPatch
    pub(crate) fn process_patch_effects(
        &mut self,
        assets: &LevelAssets,
        patch_index: crate::patch::PatchIndex,
        effects: Vec<PatchEffect>,
    ) {
        if effects.is_empty() {
            return;
        }

        // Snapshot patch data to avoid holding borrows across effect processing.
        let ctx = match self.snapshot_patch_context(patch_index) {
            Some(ctx) => ctx,
            None => {
                tracing::warn!(%patch_index, "process_patch_effects: patch not found");
                return;
            }
        };

        for effect in effects {
            match effect {
                PatchEffect::SwapDoors => {
                    self.execute_swap_doors(&ctx);
                }
                PatchEffect::SwapBackground { applied } => {
                    // Skip SwapBackground entirely if the patch isn't
                    // configured to bake into the background.
                    if !ctx.integrate_in_background {
                        continue;
                    }

                    if let Some(handle) = ctx.animation_entity_handle
                        && let Some(entity_id) = crate::natives::GameHost::actor_index(handle)
                            .map(|i| crate::element::EntityId::from_raw(i as u32))
                    {
                        if applied {
                            // Bake the last transition frame into the
                            // map surface; the engine queue picks up
                            // the baked sprite on the next drain.
                            // NOTE: sprite state is already at the
                            // transition-last frame when
                            // `SwapBackground { applied: true }` fires
                            // from `Patch::apply_final`, so no
                            // separate force-frame step is needed.
                            self.queue_blit_fx_to_map(entity_id);
                        } else {
                            // Reverse: undo the blit via the saved
                            // rectangle.
                            self.queue_restore_fx_bg(entity_id);
                        }
                    }

                    self.pending_side_effects.invalidate_background = true;
                    if let Some(ref mut script) = self.mission_script
                        && let Some(game_host) = script.game_host_mut()
                    {
                        game_host.background_invalidated = true;
                    }
                }
                PatchEffect::SwapObjects {
                    applied,
                    forced_reset,
                } => {
                    self.execute_swap_objects(assets, &ctx, applied, forced_reset);
                }
                PatchEffect::StartAnimation { anim, reverse } => {
                    self.execute_start_animation(&ctx, anim, reverse);
                }
                PatchEffect::DeactivateAnimation => {
                    self.execute_deactivate_animation(&ctx);
                }
                PatchEffect::RestoreBackground => {
                    // Queue a restore for the patch's FX entity; the
                    // drain will replay the saved rectangle and
                    // re-compose affected mask textures.
                    if let Some(handle) = ctx.animation_entity_handle
                        && let Some(entity_id) = crate::natives::GameHost::actor_index(handle)
                            .map(|i| crate::element::EntityId::from_raw(i as u32))
                    {
                        self.queue_restore_fx_bg(entity_id);
                    }
                    self.pending_side_effects.invalidate_background = true;
                    if let Some(ref mut script) = self.mission_script
                        && let Some(game_host) = script.game_host_mut()
                    {
                        game_host.background_invalidated = true;
                    }
                }
            }
        }
    }

    /// Extract patch context data from GameHost (snapshot to avoid borrow issues).
    fn snapshot_patch_context(
        &mut self,
        patch_index: crate::patch::PatchIndex,
    ) -> Option<PatchContext> {
        let game_host = self.mission_script.as_mut()?.game_host_mut()?;
        let patch = game_host.patches.get(usize::from(patch_index))?;

        let animation_entity_handle = game_host
            .patch_animation_entities
            .get(usize::from(patch_index))
            .copied()
            .flatten();

        Some(PatchContext {
            door_indices: patch.door_indices.clone(),
            old_sight_obstacle_indices: patch.old_sight_obstacle_indices.clone(),
            new_sight_obstacle_indices: patch.new_sight_obstacle_indices.clone(),
            old_sector_indices: patch.old_sector_indices.clone(),
            new_sector_indices: patch.new_sector_indices.clone(),
            old_line_indices: patch.old_line_indices.clone(),
            new_line_indices: patch.new_line_indices.clone(),
            old_mask_indices: patch.old_mask_indices.clone(),
            new_mask_indices: patch.new_mask_indices.clone(),
            use_changing_obstacles: patch.use_changing_obstacles,
            pathfinder_layer: patch.pathfinder_layer,
            pathfinder_sector: patch.pathfinder_sector,
            pathfinder_changing_obstacles: patch.pathfinder_changing_obstacles,
            animation_entity_handle,
            integrate_in_background: patch.integrate_in_background,
        })
    }

    /// Execute SwapDoors: call `swap_rights_patch()` on each door in the patch.
    fn execute_swap_doors(&mut self, ctx: &PatchContext) {
        if ctx.door_indices.is_empty() {
            return;
        }
        if let Some(ref mut script) = self.mission_script
            && let Some(game_host) = script.game_host_mut()
        {
            for &di in &ctx.door_indices {
                if let Some(door) = game_host.doors.get_mut(di as usize) {
                    door.swap_rights_patch();
                }
            }
        }
    }

    /// Execute SwapObjects: toggle masks, sight obstacles, sectors, lines,
    /// and pathfinder state.
    fn execute_swap_objects(
        &mut self,
        assets: &LevelAssets,
        ctx: &PatchContext,
        applied: bool,
        forced_reset: bool,
    ) {
        // Toggle sight obstacles
        for &idx in &ctx.old_sight_obstacle_indices {
            self.set_sight_obstacle_active(u32::from(idx), !applied);
        }
        for &idx in &ctx.new_sight_obstacle_indices {
            self.set_sight_obstacle_active(u32::from(idx), applied);
        }

        // Toggle grid sectors
        for &idx in &ctx.old_sector_indices {
            self.fast_grid.set_sector_active(idx, !applied);
        }
        for &idx in &ctx.new_sector_indices {
            self.fast_grid.set_sector_active(idx, applied);
        }

        // Toggle grid lines
        for &idx in &ctx.old_line_indices {
            self.fast_grid.set_line_active(idx, !applied);
        }
        for &idx in &ctx.new_line_indices {
            self.fast_grid.set_line_active(idx, applied);
        }

        // Toggle sprite-occlusion masks.
        for &idx in &ctx.old_mask_indices {
            self.fast_grid.set_mask_active(idx, !applied);
        }
        for &idx in &ctx.new_mask_indices {
            self.fast_grid.set_mask_active(idx, applied);
        }

        // Pathfinder obstacle state change.  The stream-deserialised
        // `pathfinder_sector` is a cumulative obstacle count, not an
        // area index — `convert_sector` maps it to the correct graph
        // area (identity only when every area has zero obstacles).
        //
        // When `!forced_reset`, also:
        //   - collect the list of obstacle sectors that just became active,
        //   - iterate actors in the affected layer/sector, invalidate
        //     their paths, and if any appeared obstacle intersects the
        //     actor's move box, flag them unreachable + queue a lethal
        //     1000-damage sequence element.
        if ctx.use_changing_obstacles {
            let area = self
                .pathfinder
                .try_convert_sector(assets.pathfinder_graph.as_ref(), ctx.pathfinder_sector)
                .unwrap_or_else(|| {
                    panic!(
                        "patch_effects: ConvertSector failed — no area mapping \
                         for pathfinder_sector={} (layer={})",
                        ctx.pathfinder_sector, ctx.pathfinder_layer
                    )
                });
            let mut appeared = Vec::new();
            let mut line_toggles = Vec::new();
            self.pathfinder.toggle_obstacle_state(
                assets.pathfinder_graph.as_ref(),
                ctx.pathfinder_layer as usize,
                area as usize,
                ctx.pathfinder_changing_obstacles as u16,
                &mut appeared,
                &mut line_toggles,
            );

            // Apply grid-line toggles from motion-obstacle activation
            // changes.
            for (line_idx, active) in line_toggles {
                self.fast_grid.set_line_active(line_idx, active);
            }

            if !forced_reset {
                self.invalidate_paths_and_kill_crushed(
                    assets,
                    ctx.pathfinder_layer,
                    ctx.pathfinder_sector,
                    &appeared,
                );
            }
        }
    }

    /// Re-translate active Move/Seek paths for actors in the patch's
    /// affected (layer, sector) and kill anyone crushed by a freshly-
    /// appeared motion obstacle.
    ///
    /// Algorithm:
    /// ```text
    /// for each actor in entities:
    ///     if actor.layer == layer && actor.sector == grid.GetSector(sector):
    ///         invalidate_movements(actor);          // re-submit current path
    ///         for each obstacle in appeared:
    ///             if obstacle.box.intersects(move_box)
    ///                 && obstacle.polygon.intersects(move_box):
    ///                 actor.unreachable = true;
    ///                 launch_damage(actor, 1000, 1000);
    /// ```
    ///
    /// `invalidate_movements` only acts when the actor has an active
    /// Move/Seek `InProgress` element with non-empty orders (the path
    /// has already been found and the actor is walking along it): it
    /// clears the order list and re-runs path dispatch.  On
    /// re-translate success the new orders replace the cleared ones;
    /// on failure the element slides into `MOVE_WAITING` via
    /// `failed_path_requests` and times out after 100 frames.
    ///
    /// The two-stage test is intentional: a cheap bbox-vs-bbox
    /// pre-filter, followed by a polygon-vs-bbox narrow test against
    /// the obstacle's polygon vertices (carried through the
    /// `AppearedObstacle` payload).
    fn invalidate_paths_and_kill_crushed(
        &mut self,
        assets: &LevelAssets,
        layer: u16,
        sector: u16,
        appeared: &[crate::pathfinder::AppearedObstacle],
    ) {
        // Phase 1: collect targets + whether each is crushed.  Borrows
        // self immutably; mutations happen in phase 2.
        //
        // Every same-sector actor gets its in-progress path
        // re-translated *before* the move box is read — an actor
        // without a current move box still has its path re-translated.
        // So the move-box check only gates the `crushed` computation,
        // not target inclusion.
        let targets: Vec<(EntityId, bool)> = self
            .entities_iter_with_id()
            .filter_map(|(id, entity)| {
                if !entity.is_actor() {
                    return None;
                }
                let element = entity.element_data();
                if element.layer() != layer {
                    return None;
                }
                if element.sector() != crate::position_interface::SectorHandle::new(sector) {
                    return None;
                }
                let pi = entity.position_iface();
                let move_box_map = *pi.get_move_box_map();
                let crushed = move_box_map.is_somewhere()
                    && appeared.iter().any(|obs| {
                        obs.bounding_box.is_somewhere()
                            && obs.bounding_box.intersects_bbox(&move_box_map)
                            && crate::geo2d::polygon_vertices_intersect_bbox(
                                &obs.polygon,
                                &move_box_map,
                            )
                    });
                Some((id, crushed))
            })
            .collect();

        for (id, crushed) in targets {
            // Invalidate movement: only acts on a Move/Seek InProgress
            // element with non-empty orders.  Snapshot dest + action
            // off the element, clear the orders, then re-run
            // `try_dispatch_move_path` to re-submit the path request.
            let retranslate = self
                .get_entity(id)
                .and_then(|e| e.actor_data())
                .map(|a| a.active_movement)
                .filter(|am| am.is_active())
                .and_then(|am| {
                    let seq_id = am.sequence_id?;
                    let elem_idx = am.element_index;
                    let elem = self.sequence_manager.get_element(seq_id, elem_idx)?;
                    if elem.owner != Some(id) {
                        return None;
                    }
                    if !matches!(elem.state, crate::sequence::SequenceState::InProgress) {
                        return None;
                    }
                    if !matches!(
                        elem.command,
                        crate::element::Command::Move | crate::element::Command::Seek
                    ) {
                        return None;
                    }
                    if elem.orders.is_empty() {
                        return None;
                    }
                    let (dest, action) = match &elem.data {
                        crate::sequence::SequenceElementData::Movement {
                            destination,
                            element: seek_target,
                            action,
                            ..
                        } => {
                            let pt = if elem.command == crate::element::Command::Seek {
                                let tgt = (*seek_target)?;
                                let te = self.get_entity(tgt)?;
                                te.element_data().position_map()
                            } else {
                                *destination
                            };
                            (pt, *action)
                        }
                        _ => return None,
                    };
                    Some((seq_id, elem_idx, dest, action))
                });

            if let Some((seq_id, elem_idx, dest, action)) = retranslate {
                // Clear orders and the actor's active-movement link
                // so `try_dispatch_move_path` can re-establish them
                // from a clean slate.
                if let Some(elem) = self.sequence_manager.get_element_mut(seq_id, elem_idx) {
                    elem.orders.clear();
                }
                if let Some(entity) = self.get_entity_mut(id)
                    && let Some(actor) = entity.actor_data_mut()
                {
                    actor.active_movement.clear();
                }
                match self.try_dispatch_move_path(assets, id, seq_id, elem_idx, dest, action) {
                    crate::engine::movement::MovePathOutcome::Success => {}
                    crate::engine::movement::MovePathOutcome::ActorGone => {
                        self.sequence_manager.element_impossible(seq_id, elem_idx);
                    }
                    crate::engine::movement::MovePathOutcome::Failed => {
                        // Re-translate failed — slide into MOVE_WAITING.
                        self.failed_path_requests.push(
                            crate::engine::movement::FailedPathRequest {
                                owner: id,
                                seq_id,
                                elem_idx,
                                first_fail_frame: self.frame_counter,
                            },
                        );
                    }
                }
            }

            if crushed {
                if let Some(entity) = self.get_entity_mut(id) {
                    entity.element_data_mut().unreachable = true;
                }
                self.launch_damage(id, 1000, 1000);
            }
        }
    }

    /// Execute StartAnimation: activate the patch's FX entity and set its
    /// animation row.
    fn execute_start_animation(&mut self, ctx: &PatchContext, anim: PatchAnimation, reverse: bool) {
        let handle = match ctx.animation_entity_handle {
            Some(h) => h,
            None => return,
        };

        // Map `PatchAnimation` to an `OrderType` so the sprite's
        // current conversion table can resolve the actual animation row
        // via `row_for_action`.  These are not raw row indices.
        let action = match anim {
            PatchAnimation::Initial => crate::order::OrderType::PATCH_INITIAL,
            PatchAnimation::Transition => crate::order::OrderType::PATCH_TRANSITION,
            PatchAnimation::Final => crate::order::OrderType::PATCH_FINAL,
        };

        // Activate the entity and set the animation frame.
        let Some(entity_idx) = crate::natives::GameHost::actor_index(handle) else {
            tracing::warn!(handle, "patch_effects: invalid animation entity handle");
            return;
        };
        if let Some(Some(entity)) = self.entities.get_mut(entity_idx) {
            entity.element_data_mut().active = true;
            {
                let sprite = entity.sprite_mut();
                let Some(row) = sprite.row_for_action(action) else {
                    tracing::warn!(
                        handle,
                        ?anim,
                        ?action,
                        profile = %sprite.frame_profile_name,
                        "patch_effects: StartAnimation on sprite without this animation — skipping"
                    );
                    return;
                };
                sprite.current_row = row;
                if reverse {
                    // Start at last frame for reverse playback.
                    let last_frame = sprite.num_frames_for_row(row).saturating_sub(1);
                    sprite.current_frame = last_frame;
                } else {
                    sprite.current_frame = 0;
                }
                sprite.frame_count = 0;
            }
        }

        // Also update entity_active on GameHost so script queries see it.
        if let Some(ref mut script) = self.mission_script
            && let Some(game_host) = script.game_host_mut()
        {
            game_host.entity_active.insert(handle, true);
        }

        tracing::trace!(handle, ?anim, "patch_effects: StartAnimation");
    }

    /// Execute DeactivateAnimation: deactivate the patch's FX entity.
    fn execute_deactivate_animation(&mut self, ctx: &PatchContext) {
        let handle = match ctx.animation_entity_handle {
            Some(h) => h,
            None => return,
        };

        let Some(entity_idx) = crate::natives::GameHost::actor_index(handle) else {
            tracing::warn!(handle, "patch_effects: invalid animation entity handle");
            return;
        };
        if let Some(Some(entity)) = self.entities.get_mut(entity_idx) {
            entity.element_data_mut().active = false;
        }

        // Also update entity_active on GameHost.
        if let Some(ref mut script) = self.mission_script
            && let Some(game_host) = script.game_host_mut()
        {
            game_host.entity_active.insert(handle, false);
        }

        tracing::trace!(handle, "patch_effects: DeactivateAnimation");
    }

    /// Queue a persistent background decal insert for this FX entity.
    /// Consumed later by the host-side drain after `perform_hourglass`
    /// returns its `SideEffects` (see `robin_rs::blit_to_map`).
    pub(crate) fn queue_blit_fx_to_map(&mut self, entity_id: crate::element::EntityId) {
        let decal = self.snapshot_patch_transition_decal(entity_id);
        self.pending_side_effects
            .bg_blits
            .push(super::PendingBgBlit {
                entity_id,
                restore_only: false,
                decal,
            });
    }

    /// Queue a persistent background decal removal for this FX entity.
    /// Consumed later by the host-side drain.
    pub(crate) fn queue_restore_fx_bg(&mut self, entity_id: crate::element::EntityId) {
        self.pending_side_effects
            .bg_blits
            .push(super::PendingBgBlit {
                entity_id,
                restore_only: true,
                decal: None,
            });
    }

    fn snapshot_patch_transition_decal(
        &self,
        entity_id: crate::element::EntityId,
    ) -> Option<super::PendingBgBlitDecal> {
        let entity = match self.get_entity(entity_id) {
            Some(e) => e,
            None => {
                tracing::warn!("blit_to_map: FX entity {:?} missing", entity_id);
                return None;
            }
        };

        if !entity.kind().is_fx_base() {
            tracing::warn!(
                ?entity_id,
                kind = ?entity.kind(),
                "blit_to_map: patch background blit requested for non-FX entity"
            );
            return None;
        }

        let elem = entity.element_data();
        if elem.position().z != 0.0 || !elem.active {
            return None;
        }

        let sprite = &elem.sprite;
        let Some(row) = sprite.row_for_action(crate::order::OrderType::PATCH_TRANSITION) else {
            tracing::warn!(
                ?entity_id,
                profile = %sprite.frame_profile_name,
                "blit_to_map: patch FX sprite has no transition animation"
            );
            return None;
        };
        let frame = sprite.num_frames_for_row(row).saturating_sub(1);
        let scripts = sprite.current_scripts_opt()?;
        let script = scripts.get(row as usize)?;
        let &bank_id = script.frame_ids.get(frame as usize)?;
        let offset = script.offsets.get(frame as usize).copied()?;

        let center = sprite.center;
        let dst_x = ((elem.position_map().x - center.x).floor() + offset.x).floor() as i32;
        let dst_y = ((elem.position_map().y - center.y).floor() + offset.y).floor() as i32;

        Some(super::PendingBgBlitDecal {
            bank_id,
            dst_x,
            dst_y,
            shadow_color: self.weather.night_color,
        })
    }
}
