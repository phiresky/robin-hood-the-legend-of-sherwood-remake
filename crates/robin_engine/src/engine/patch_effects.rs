//! Direct patch transitions and their terrain, actor, animation, and render updates.

use super::movement::MovePathOutcome;
use super::*;
use crate::order::OrderType;

fn initialize_patch_animation(
    sprite: &mut crate::sprite::Sprite,
    action: crate::order::OrderType,
    reverse: bool,
) -> Option<u16> {
    let row = sprite.row_for_action(action)?;
    sprite.current_row = row;
    // Original-game patch application forces animation followed by
    // `ResetSpriteFrame`, whose 0xffff counter sentinel makes the first
    // update wrap to zero without consuming the first authored tick.
    sprite.reset_sprite_frame(reverse);
    Some(row)
}

impl EngineInner {
    pub(crate) fn apply_patch(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        index: crate::patch::PatchIndex,
    ) {
        let i = usize::from(index);
        let patch = &self.script_domains.interactables.patches[i];
        if patch.animated && patch.in_transition {
            self.execute_deactivate_animation(index);
            self.apply_patch_final(sim, assets, index, false);
        }
        let patch = &self.script_domains.interactables.patches[i];
        if patch.applied {
            if patch.definitive {
                return;
            }
            self.swap_patch_background(index, false);
        }
        let patch = &self.script_domains.interactables.patches[i];
        if patch.animation_flags.transition_valid {
            let reverse = patch.applied;
            self.execute_start_animation(index, OrderType::PATCH_TRANSITION, reverse);
            self.script_domains.interactables.patches[i].in_transition = true;
        } else {
            self.execute_deactivate_animation(index);
            self.apply_patch_final(sim, assets, index, false);
        }
    }

    pub(crate) fn apply_patch_final(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        index: crate::patch::PatchIndex,
        forced_reset: bool,
    ) {
        let i = usize::from(index);
        if self.script_domains.interactables.patches[i].definitive {
            self.script_domains.interactables.patches[i].active = false;
        }
        self.execute_swap_doors(index);
        let patch = &self.script_domains.interactables.patches[i];
        if patch.applied {
            if !patch.definitive {
                self.script_domains.interactables.patches[i].applied = false;
                if self.script_domains.interactables.patches[i]
                    .animation_flags
                    .start_valid
                {
                    self.execute_start_animation(index, OrderType::PATCH_INITIAL, false);
                } else {
                    self.execute_deactivate_animation(index);
                }
                self.execute_swap_objects(sim, assets, index, false, forced_reset);
            }
        } else {
            self.script_domains.interactables.patches[i].applied = true;
            self.swap_patch_background(index, true);
            self.execute_swap_objects(sim, assets, index, true, forced_reset);
            if self.script_domains.interactables.patches[i]
                .animation_flags
                .end_valid
            {
                self.execute_start_animation(index, OrderType::PATCH_FINAL, false);
            } else {
                self.execute_deactivate_animation(index);
            }
        }
    }

    pub(crate) fn reset_patch(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        index: crate::patch::PatchIndex,
    ) {
        let i = usize::from(index);
        if self.script_domains.interactables.patches[i].applied {
            self.swap_patch_background(index, false);
            if !self.script_domains.interactables.patches[i].in_transition {
                self.execute_swap_doors(index);
            }
        }
        let patch = &mut self.script_domains.interactables.patches[i];
        patch.applied = false;
        patch.active = patch.initially_active;
        if patch.animated {
            self.restore_patch_background(index);
            if self.script_domains.interactables.patches[i]
                .animation_flags
                .start_valid
            {
                self.execute_start_animation(index, OrderType::PATCH_INITIAL, false);
            } else {
                self.execute_deactivate_animation(index);
            }
            self.script_domains.interactables.patches[i].in_transition = false;
        }
        self.execute_swap_objects(sim, assets, index, false, true);
    }

    fn patch_animation_handle(&self, index: crate::patch::PatchIndex) -> Option<i32> {
        self.scripts
            .mission
            .as_ref()?
            .bindings
            .patch_animation_entities
            .get(usize::from(index))
            .copied()
            .flatten()
    }

    fn restore_patch_background(&mut self, index: crate::patch::PatchIndex) {
        if let Some(handle) = self.patch_animation_handle(index)
            && let Some(entity) = self.entity_id_for_actor_handle(handle)
        {
            self.queue_restore_fx_bg(entity);
        }
        self.feedback.pending_side_effects.invalidate_background = true;
    }

    fn swap_patch_background(&mut self, index: crate::patch::PatchIndex, applied: bool) {
        if !self.script_domains.interactables.patches[usize::from(index)].integrate_in_background {
            return;
        }
        if applied {
            if let Some(handle) = self.patch_animation_handle(index)
                && let Some(entity) = self.entity_id_for_actor_handle(handle)
            {
                self.queue_blit_fx_to_map(entity);
            }
            self.feedback.pending_side_effects.invalidate_background = true;
        } else {
            self.restore_patch_background(index);
        }
    }

    /// Execute SwapDoors: call `swap_rights_patch()` on each door in the patch.
    fn execute_swap_doors(&mut self, index: crate::patch::PatchIndex) {
        let interactables = &mut self.script_domains.interactables;
        for &di in &interactables.patches[usize::from(index)].door_indices {
            if let Some(door) = interactables.doors.get_mut(di as usize) {
                door.swap_rights_patch();
            }
        }
    }

    /// Execute SwapObjects: toggle masks, sight obstacles, sectors, lines,
    /// and pathfinder state.
    fn execute_swap_objects(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        index: crate::patch::PatchIndex,
        applied: bool,
        forced_reset: bool,
    ) {
        for offset in 0..self.script_domains.interactables.patches[usize::from(index)]
            .old_mask_indices
            .len()
        {
            let idx = self.script_domains.interactables.patches[usize::from(index)]
                .old_mask_indices[offset];
            self.world.fast_grid_mut().set_mask_active(idx, !applied);
        }
        for offset in 0..self.script_domains.interactables.patches[usize::from(index)]
            .new_mask_indices
            .len()
        {
            let idx = self.script_domains.interactables.patches[usize::from(index)]
                .new_mask_indices[offset];
            self.world.fast_grid_mut().set_mask_active(idx, applied);
        }
        for offset in 0..self.script_domains.interactables.patches[usize::from(index)]
            .old_sight_obstacle_indices
            .len()
        {
            let idx = self.script_domains.interactables.patches[usize::from(index)]
                .old_sight_obstacle_indices[offset];
            self.set_sight_obstacle_active(u32::from(idx), !applied);
        }
        for offset in 0..self.script_domains.interactables.patches[usize::from(index)]
            .new_sight_obstacle_indices
            .len()
        {
            let idx = self.script_domains.interactables.patches[usize::from(index)]
                .new_sight_obstacle_indices[offset];
            self.set_sight_obstacle_active(u32::from(idx), applied);
        }
        for offset in 0..self.script_domains.interactables.patches[usize::from(index)]
            .old_sector_indices
            .len()
        {
            let idx = self.script_domains.interactables.patches[usize::from(index)]
                .old_sector_indices[offset];
            self.world.fast_grid_mut().set_sector_active(idx, !applied);
        }
        for offset in 0..self.script_domains.interactables.patches[usize::from(index)]
            .new_sector_indices
            .len()
        {
            let idx = self.script_domains.interactables.patches[usize::from(index)]
                .new_sector_indices[offset];
            self.world.fast_grid_mut().set_sector_active(idx, applied);
        }
        for offset in 0..self.script_domains.interactables.patches[usize::from(index)]
            .old_line_indices
            .len()
        {
            let idx = self.script_domains.interactables.patches[usize::from(index)]
                .old_line_indices[offset];
            self.world.fast_grid_mut().set_line_active(idx, !applied);
        }
        for offset in 0..self.script_domains.interactables.patches[usize::from(index)]
            .new_line_indices
            .len()
        {
            let idx = self.script_domains.interactables.patches[usize::from(index)]
                .new_line_indices[offset];
            self.world.fast_grid_mut().set_line_active(idx, applied);
        }
        let patch = &self.script_domains.interactables.patches[usize::from(index)];
        let (
            use_changing_obstacles,
            pathfinder_layer,
            pathfinder_sector,
            pathfinder_changing_obstacles,
        ) = (
            patch.use_changing_obstacles,
            patch.pathfinder_layer,
            patch.pathfinder_sector,
            patch.pathfinder_changing_obstacles,
        );

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
        if use_changing_obstacles {
            let area = self
                .world
                .pathfinder
                .try_convert_sector(
                    assets.navigation.pathfinder_graph.as_ref(),
                    pathfinder_sector,
                )
                .unwrap_or_else(|| {
                    panic!(
                        "patch_effects: ConvertSector failed — no area mapping \
                         for pathfinder_sector={} (layer={})",
                        pathfinder_sector, pathfinder_layer
                    )
                });
            let appeared = self.world.pathfinder.toggle_obstacle_state(
                assets.navigation.pathfinder_graph.as_ref(),
                std::sync::Arc::make_mut(&mut self.world.fast_grid),
                pathfinder_layer as usize,
                area as usize,
                pathfinder_changing_obstacles as u16,
            );

            if !forced_reset {
                self.invalidate_paths_and_kill_crushed(
                    sim,
                    assets,
                    pathfinder_layer,
                    pathfinder_sector,
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
    ///     if actor.layer == layer && actor.sector == resolved_sector:
    ///         invalidate_movements(actor);          // re-submit current path
    ///         for each obstacle in appeared:
    ///             if obstacle.box.intersects(move_box)
    ///                 && obstacle.polygon.intersects(move_box):
    ///                 actor.unreachable = true;
    ///                 launch_damage(actor, 1000, 1000);
    /// ```
    ///
    /// Movement invalidation retranslates the selected MoveOk element:
    /// it clears the order list and re-runs path dispatch.  On
    /// re-translate success the new orders replace the cleared ones;
    /// on failure the element slides into `MOVE_WAITING` via
    /// `failed_path_requests` and times out after 100 frames.
    ///
    /// The two-stage test is intentional: a cheap bbox-vs-bbox
    /// pre-filter, followed by a polygon-vs-bbox narrow test against
    /// the obstacle's live sector geometry. Only appeared membership is
    /// retained across movement retranslation and damage callbacks.
    fn invalidate_paths_and_kill_crushed(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        layer: u16,
        sector: u16,
        appeared: &[crate::fast_find_grid::SectorIndex],
    ) {
        let actor_slots = self.world.entities.len();
        for slot in 0..actor_slots {
            let Some((id, entity)) = self.world.entities.get_legacy_slot(slot as u32) else {
                continue;
            };
            if entity.actor_data().is_none()
                || entity.element_data().layer() != layer
                || entity.element_data().sector()
                    != crate::position_interface::SectorHandle::new(sector)
            {
                continue;
            }
            // Read destination and action from the selected movement,
            // clear its orders, then re-run
            // `try_dispatch_move_path` to re-submit the path request.
            let retranslate =
                self.current_sequence_element_for_actor(id)
                    .and_then(|(seq_id, elem_idx)| {
                        let elem = self.orders.sequence_manager.get_element(seq_id, elem_idx)?;
                        if elem.command != crate::element::Command::MoveOk {
                            return None;
                        }
                        let (dest, action) = match &elem.data {
                            crate::sequence::SequenceElementData::Movement {
                                destination,
                                element: seek_target,
                                action,
                                flags,
                                ..
                            } => {
                                let pt = if flags.contains(crate::sequence::MoveFlags::SEEK) {
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
                crate::movement_diagnostics::record_parity_late_movement_retranslation(id);
                // Rebuild the selected movement's orders from a clean slate.
                if let Some(elem) = self
                    .orders
                    .sequence_manager
                    .get_element_mut(seq_id, elem_idx)
                {
                    elem.command = crate::element::Command::Move;
                    elem.orders.clear();
                }
                // The original game retranslates the
                // selected movement. The
                // MOVE arm begins by extracting an unauthorized actor from
                // its obstacle before it tests direct reachability or queues
                // a path request. Ordinary
                // manager-driven translation already enters through
                // `extract_move_instruction_owner`; this synchronous patch
                // retranslation must cross the same boundary as well.
                //
                // Skipping it leaves the actor at the obstructed point and
                // lets only the pathfinder's later request processing
                // unexpanded-box recovery adjust the request source.  That
                // differs observably: the actor is not moved, the corrected
                // source is different, and first-point selection becomes enabled.
                if !self.extract_move_instruction_owner(id) {
                    self.element_impossible(sim, assets, &mut Vec::new(), seq_id, elem_idx);
                } else {
                    match self
                        .try_dispatch_move_path(sim, assets, id, seq_id, elem_idx, dest, action)
                    {
                        MovePathOutcome::Success | MovePathOutcome::Pending => {
                            // Corrected original-game movement invalidation refreshes
                            // actor order from the retranslated selected element. The
                            // old order storage has been deleted, so retaining the
                            // previous installed snapshot would reproduce its
                            // former dangling-pointer allocator dependence.
                            let installed_order = self
                                .orders
                                .sequence_manager
                                .current_order_for_actor(&self.world.entities, id)
                                .filter(|(live_seq, live_idx, _)| {
                                    *live_seq == seq_id && *live_idx == elem_idx
                                })
                                .map(|(_, _, order)| crate::element::InstalledActorOrder {
                                    order_id: order.order_id,
                                    order_type: order.order_type,
                                });
                            self.get_entity_mut(id)
                                .and_then(crate::element::Entity::actor_data_mut)
                                .expect("retranslated movement owner lost actor data")
                                .installed_order = installed_order;
                        }
                        MovePathOutcome::ActorGone | MovePathOutcome::Refused => {
                            self.element_impossible(sim, assets, &mut Vec::new(), seq_id, elem_idx);
                        }
                        MovePathOutcome::Failed => {
                            // Source extraction failure already performed the
                            // the original game's stop-and-wait effects and never enters the
                            // failed-A* timeout list.
                        }
                    }
                }
            }

            let Some(entity) = self.get_entity(id) else {
                continue;
            };
            let move_box = *entity.position_iface().get_move_box_map();
            for sector_index in appeared {
                let obstacle = &self.world.fast_grid.level.sectors[sector_index.get() as usize];
                if move_box.is_somewhere()
                    && obstacle.bounding_box.is_somewhere()
                    && obstacle.bounding_box.intersects_bbox(&move_box)
                    && obstacle.intersects_bbox(&move_box)
                {
                    if let Some(entity) = self.get_entity_mut(id) {
                        entity.element_data_mut().unreachable = true;
                    }
                    self.launch_damage(sim, assets, id, 1000, 1000);
                }
            }
        }
    }

    /// Execute StartAnimation: activate the patch's FX entity and set its
    /// animation row.
    fn execute_start_animation(
        &mut self,
        index: crate::patch::PatchIndex,
        action: OrderType,
        reverse: bool,
    ) {
        let handle = match self.patch_animation_handle(index) {
            Some(h) => h,
            None => return,
        };

        // Activate the entity and set the animation frame.
        let Some(entity_id) = self.entity_id_for_actor_handle(handle) else {
            tracing::warn!(handle, "patch_effects: invalid animation entity handle");
            return;
        };
        if let Some(entity) = self.world.entities.get_mut(entity_id) {
            entity.element_data_mut().active = true;
            {
                let sprite = entity.sprite_mut();
                let Some(_row) = initialize_patch_animation(sprite, action, reverse) else {
                    tracing::warn!(
                        handle,
                        ?action,
                        profile = %sprite.frame_profile_name,
                        "patch_effects: StartAnimation on sprite without this animation — skipping"
                    );
                    return;
                };
            }
        }

        tracing::trace!(handle, ?action, "patch_effects: StartAnimation");
    }

    /// Execute DeactivateAnimation: deactivate the patch's FX entity.
    fn execute_deactivate_animation(&mut self, index: crate::patch::PatchIndex) {
        let handle = match self.patch_animation_handle(index) {
            Some(h) => h,
            None => return,
        };

        let Some(entity_id) = self.entity_id_for_actor_handle(handle) else {
            tracing::warn!(handle, "patch_effects: invalid animation entity handle");
            return;
        };
        if let Some(entity) = self.world.entities.get_mut(entity_id) {
            entity.element_data_mut().active = false;
        }

        tracing::trace!(handle, "patch_effects: DeactivateAnimation");
    }

    /// Queue a persistent background decal insert for this FX entity.
    /// Consumed later by the host-side drain after `perform_hourglass`
    /// returns its `SideEffects` (see `robin_rs::blit_to_map`).
    pub(crate) fn queue_blit_fx_to_map(&mut self, entity_id: crate::element::EntityId) {
        let decal = self.snapshot_patch_transition_decal(entity_id);
        self.feedback
            .pending_side_effects
            .host_effects
            .background_blits
            .push(super::PendingBgBlit {
                entity_id,
                restore_only: false,
                decal,
            });
    }

    /// Queue a persistent background decal removal for this FX entity.
    /// Consumed later by the host-side drain.
    pub(crate) fn queue_restore_fx_bg(&mut self, entity_id: crate::element::EntityId) {
        self.feedback
            .pending_side_effects
            .host_effects
            .background_blits
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
        // The patch state machine deactivates FX without a transition
        // animation before emitting SwapBackground. The decal is an
        // explicit snapshot of the transition row, not the entity's current
        // live frame, so inactive state must not discard it.
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
            shadow_color: self.world.weather.night_color,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinates::{MapPoint, SpriteAnchor, SpriteFrameOffset, WorldPoint3D};
    use crate::element::{
        ActorData, ActorPc, ElementData, ElementFx, ElementKind, Entity, FxData, HumanData, PcData,
        Posture,
    };
    use crate::fast_find_grid::GridLine;
    use crate::order::{Order, OrderType};
    use crate::position_interface::SectorHandle;
    use crate::sequence::{SequenceElement, SequencePriority};
    use crate::sprite::Sprite;
    use crate::sprite_script::SpriteScript;

    fn patch_fixture(animated: bool, definitive: bool) -> (EngineInner, crate::patch::PatchIndex) {
        let mut engine = EngineInner::new();
        let old = engine.world.fast_grid_mut().add_line(
            GridLine::new(MapPoint::new(0.0, 0.0), MapPoint::new(10.0, 0.0), true),
            0,
        );
        let new = engine.world.fast_grid_mut().add_line(
            GridLine::new(MapPoint::new(0.0, 10.0), MapPoint::new(10.0, 10.0), true),
            0,
        );
        engine.world.fast_grid_mut().set_line_active(new, false);
        engine
            .script_domains
            .interactables
            .doors
            .push(crate::gate::Door {
                locked_pc: false,
                locked_pc_after_patch: true,
                ..Default::default()
            });
        engine
            .script_domains
            .interactables
            .patches
            .push(crate::patch::Patch {
                active: true,
                initially_active: true,
                definitive,
                animation_flags: crate::patch::AnimationFlags {
                    start_valid: animated,
                    transition_valid: animated,
                    end_valid: animated,
                },
                door_indices: vec![0],
                old_line_indices: vec![old],
                new_line_indices: vec![new],
                ..Default::default()
            });
        (engine, crate::patch::PatchIndex::new(0).unwrap())
    }

    fn assert_patch_terrain(engine: &EngineInner, applied: bool) {
        let patch = &engine.script_domains.interactables.patches[0];
        assert_eq!(patch.applied, applied);
        assert_eq!(
            engine
                .world
                .fast_grid
                .is_line_active(patch.old_line_indices[0]),
            !applied
        );
        assert_eq!(
            engine
                .world
                .fast_grid
                .is_line_active(patch.new_line_indices[0]),
            applied
        );
        assert_eq!(
            engine.script_domains.interactables.doors[0].locked_pc,
            applied
        );
    }

    #[test]
    fn patch_transitions_update_canonical_terrain_and_door_rights() {
        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::default();
        for animated in [false, true] {
            for definitive in [false, true] {
                let (mut engine, index) = patch_fixture(animated, definitive);
                engine.apply_patch(&sim, &assets, index);
                if animated {
                    assert_patch_terrain(&engine, false);
                    assert!(engine.script_domains.interactables.patches[0].in_transition);
                    engine.finish_patch_transition_for(&sim, &assets, index);
                }
                assert_patch_terrain(&engine, true);
                assert_eq!(
                    engine.script_domains.interactables.patches[0].active,
                    !definitive
                );
                engine.apply_patch(&sim, &assets, index);
                if animated && !definitive {
                    assert_patch_terrain(&engine, true);
                    engine.finish_patch_transition_for(&sim, &assets, index);
                }
                assert_patch_terrain(&engine, definitive);
                engine.reset_patch(&sim, &assets, index);
                assert_patch_terrain(&engine, false);
                assert!(engine.script_domains.interactables.patches[0].active);
                assert!(!engine.script_domains.interactables.patches[0].in_transition);
            }
        }
    }

    #[test]
    fn interrupted_patch_transition_finishes_before_starting_reverse() {
        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::default();
        let (mut engine, index) = patch_fixture(true, false);
        engine.apply_patch(&sim, &assets, index);
        engine.apply_patch(&sim, &assets, index);
        assert_patch_terrain(&engine, true);
        assert!(engine.script_domains.interactables.patches[0].in_transition);
        engine.finish_patch_transition_for(&sim, &assets, index);
        assert_patch_terrain(&engine, false);
    }

    #[test]
    fn reset_during_forward_transition_does_not_toggle_doors() {
        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::default();
        let (mut engine, index) = patch_fixture(true, false);
        engine.apply_patch(&sim, &assets, index);
        engine.reset_patch(&sim, &assets, index);
        assert_patch_terrain(&engine, false);
        assert!(!engine.script_domains.interactables.patches[0].in_transition);
    }

    #[test]
    fn configured_background_reversal_survives_save_between_transitions() {
        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::default();
        let (mut engine, index) = patch_fixture(true, true);
        let patch = &mut engine.script_domains.interactables.patches[0];
        patch.configure_background_reversal(true);
        patch.repeat_activation = Some((123, "ActivatedBySword".into()));
        for _ in 0..2 {
            engine.apply_patch(&sim, &assets, index);
            engine.finish_patch_transition_for(&sim, &assets, index);
            assert_patch_terrain(&engine, true);
            let patch = &mut engine.script_domains.interactables.patches[0];
            *patch = serde_json::from_str(&serde_json::to_string(patch).unwrap()).unwrap();
            assert_eq!(
                patch.repeat_activation,
                Some((123, "ActivatedBySword".into()))
            );
            engine.apply_patch(&sim, &assets, index);
            engine.finish_patch_transition_for(&sim, &assets, index);
            assert_patch_terrain(&engine, false);
            assert!(engine.script_domains.interactables.patches[0].active);
        }
    }

    #[test]
    fn patch_animation_reset_preserves_original_first_tick_sentinel() {
        let mut conversion = crate::engine::test_support::unmapped_conversion();
        conversion[OrderType::PATCH_TRANSITION as usize] = 0;
        let script = SpriteScript {
            frame_ids: vec![11, 22],
            delays: vec![1, 1],
            ..Default::default()
        };
        let mut sprite = Sprite::new(
            std::sync::Arc::new(vec![script]),
            std::sync::Arc::new(conversion),
        );

        assert_eq!(
            initialize_patch_animation(&mut sprite, OrderType::PATCH_TRANSITION, false),
            Some(0)
        );
        assert_eq!((sprite.current_frame, sprite.frame_count), (0, u16::MAX));

        assert!(!sprite.increment_frame(
            &crate::sim_rng::test_context(),
            crate::sprite::FrameProgression::Default,
        ));
        assert_eq!((sprite.current_frame, sprite.frame_count), (0, 0));

        assert_eq!(
            initialize_patch_animation(&mut sprite, OrderType::PATCH_TRANSITION, true),
            Some(0)
        );
        assert_eq!((sprite.current_frame, sprite.frame_count), (1, u16::MAX));
    }

    #[test]
    fn inactive_patch_fx_still_snapshots_explicit_transition_frame() {
        let mut conversion = crate::engine::test_support::unmapped_conversion();
        conversion[OrderType::PATCH_TRANSITION as usize] = 0;
        let script = SpriteScript {
            frame_ids: vec![11, 22],
            delays: vec![0; 2],
            offsets: vec![
                SpriteFrameOffset::new(0.0, 0.0),
                SpriteFrameOffset::new(3.0, 4.0),
            ],
            ..Default::default()
        };
        let mut sprite = Sprite::new(
            std::sync::Arc::new(vec![script]),
            std::sync::Arc::new(conversion),
        );
        sprite.center = SpriteAnchor::new(5.0, 6.0);
        sprite
            .position_iface
            .set_map_position(MapPoint::new(100.0, 200.0));
        sprite.current_frame = 1;

        let mut engine = EngineInner::new();
        let entity_id = engine.add_test_entity(Entity::Fx(ElementFx {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::Fx;
                initial_element.active = false;
                initial_element.sprite = sprite;
                initial_element
            },
            fx: FxData::default(),
        }));

        let decal = engine
            .snapshot_patch_transition_decal(entity_id)
            .expect("inactive patch FX has an authored transition-frame decal");
        assert_eq!(decal.bank_id, 22);
        assert_eq!(decal.dst_x, 98);
        assert_eq!(decal.dst_y, 198);
    }

    #[test]
    fn elevated_patch_fx_snapshots_at_projected_position() {
        let mut conversion = crate::engine::test_support::unmapped_conversion();
        conversion[OrderType::PATCH_TRANSITION as usize] = 0;
        let script = SpriteScript {
            frame_ids: vec![11, 22],
            delays: vec![0; 2],
            offsets: vec![
                SpriteFrameOffset::new(0.0, 0.0),
                SpriteFrameOffset::new(3.0, 4.0),
            ],
            ..Default::default()
        };
        let mut sprite = Sprite::new(
            std::sync::Arc::new(vec![script]),
            std::sync::Arc::new(conversion),
        );
        sprite.center = SpriteAnchor::new(5.0, 6.0);
        // Original-game effect rendering uses the sprite position for
        // normal patch blits regardless of elevation. World (100, 220, 20)
        // projects to the same map anchor (100, 200) as the ground case.
        sprite
            .position_iface
            .set_position(WorldPoint3D::new(100.0, 220.0, 20.0));
        sprite.current_frame = 1;

        let mut engine = EngineInner::new();
        let entity_id = engine.add_test_entity(Entity::Fx(ElementFx {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::Fx;
                initial_element.active = true;
                initial_element.sprite = sprite;
                initial_element
            },
            fx: FxData::default(),
        }));

        let decal = engine
            .snapshot_patch_transition_decal(entity_id)
            .expect("elevated patch FX has an authored transition-frame decal");
        assert_eq!(decal.bank_id, 22);
        assert_eq!(decal.dst_x, 98);
        assert_eq!(decal.dst_y, 198);
    }

    #[test]
    fn patch_invalidation_extracts_owner_before_retranslating_move() {
        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(4, 4);
        engine.world.fast_grid_mut().allocate_layers(1);
        engine.world.fast_grid_mut().add_line(
            GridLine::new(MapPoint::new(0.0, 128.0), MapPoint::new(256.0, 128.0), true),
            0,
        );

        let start = MapPoint::new(130.0, 130.0);
        let destination = MapPoint::new(220.0, 220.0);
        let mut element = {
            let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
            initial_element.kind = ElementKind::ActorPc;
            initial_element.active = true;
            initial_element
        };
        element.set_position_map(start);
        element.set_layer(0);
        element.set_sector(SectorHandle::new(1));
        element
            .sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -10.0, -5.0, 10.0, 5.0,
            ));
        element.sprite.position_iface.set_map_position(start);
        element
            .sprite
            .position_iface
            .set_pathfinder_index(crate::position_interface::PathfinderIndex::new(0).unwrap());
        let owner = engine.add_test_entity(Entity::Pc(ActorPc {
            element,
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        }));

        let order_id = engine.orders.allocate_order_id();
        let mut movement = SequenceElement::new_movement(
            1,
            crate::element::Command::MoveOk,
            Some(owner),
            OrderType::WalkingUpright,
        );
        movement.priority = SequencePriority::Normal;
        if let crate::sequence::SequenceElementData::Movement {
            destination: stored,
            ..
        } = &mut movement.data
        {
            *stored = destination;
        }
        movement.orders.push_back(Order::new(
            OrderType::WalkingUpright,
            destination.x,
            destination.y,
            order_id,
        ));
        let sequence = engine.orders.sequence_manager.insert_element(movement);
        engine
            .orders
            .sequence_manager
            .start_sequence_level(sequence);
        engine
            .orders
            .sequence_manager
            .get_element_mut(sequence, 0)
            .unwrap()
            .state = crate::sequence::SequenceState::InProgress;
        engine.orders.sequence_manager.rebuild_indices();

        engine.select_sequence_element(owner, Some((sequence, 0)));

        let move_box = *engine
            .get_entity(owner)
            .expect("test PC exists")
            .position_iface()
            .get_move_box_map();
        assert!(
            !engine.world.fast_grid.is_position_authorized(&move_box, 0),
            "fixture must begin inside the active obstruction"
        );
        let mut expected_box = crate::coordinates::MapBBox::from_coords(
            move_box.x_min() - 0.5,
            move_box.y_min() - 0.5,
            move_box.x_max() + 0.5,
            move_box.y_max() + 0.5,
        );
        assert!(
            engine
                .world
                .fast_grid
                .find_authorized_position(&mut expected_box, 0),
            "expanded Original extraction box must find a valid center"
        );
        let expected = expected_box.center();

        let obstacle = crate::fast_find_grid::GridSector {
            bounding_box: crate::coordinates::MapBBox::from_coords(120.0, 124.0, 140.0, 128.0),
            points: vec![
                MapPoint::new(120.0, 124.0),
                MapPoint::new(140.0, 124.0),
                MapPoint::new(140.0, 128.0),
                MapPoint::new(120.0, 128.0),
            ],
            sector_type: crate::sector::SectorType::MOTION,
            layer: 0,
            sector_number: crate::sector::SectorNumber::new(2),
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
        assert!(obstacle.bounding_box.intersects_bbox(&move_box));
        assert!(!obstacle.bounding_box.intersects_bbox(&expected_box));
        let obstacle_index = engine.world.fast_grid_mut().add_sector(obstacle, 0);
        engine.invalidate_paths_and_kill_crushed(
            &crate::sim_rng::test_context(),
            &LevelAssets::default(),
            0,
            1,
            &[crate::fast_find_grid::SectorIndex::new(obstacle_index).unwrap()],
        );

        let corrected = engine
            .get_entity(owner)
            .expect("test PC survives path invalidation")
            .element_data()
            .position_map();
        assert_eq!(corrected, expected);
        assert_ne!(corrected, start);
        assert!(!engine.get_entity(owner).unwrap().element_data().unreachable);
    }
}
