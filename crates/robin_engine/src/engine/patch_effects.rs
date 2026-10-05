//! Direct patch transitions and their terrain, actor, animation, and render updates.

use super::movement::MovePathOutcome;
use super::*;
use crate::engine::TickCtx;
use crate::order::OrderType;
use crate::sequence::SequenceElementRef;

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
    /// Rebuild persistent patch visuals after adopting a different timeline.
    /// A reverse transition has already removed its applied background.
    pub fn background_patch_blits(&self, assets: &LevelAssets) -> Vec<super::PendingBgBlit> {
        self.script_domains
            .interactables
            .patches
            .iter()
            .enumerate()
            .filter(|(_, patch)| {
                patch.applied && !patch.in_transition && patch.integrate_in_background
            })
            .filter_map(|(index, _)| {
                let handle = assets
                    .entities
                    .patch_animation_entities
                    .get(index)
                    .copied()
                    .flatten()?;
                let Some(entity_id) = self.entity_id_for_actor_handle(handle) else {
                    tracing::warn!(
                        index,
                        handle,
                        "cannot reconstruct patch background: missing FX entity"
                    );
                    return None;
                };
                Some(super::PendingBgBlit {
                    entity_id,
                    restore_only: false,
                    decal: self.snapshot_patch_transition_decal(entity_id),
                })
            })
            .collect()
    }

    pub(crate) fn apply_patch(&mut self, tcx: TickCtx<'_>, index: crate::patch::PatchIndex) {
        let i = usize::from(index);
        let patch = &self.script_domains.interactables.patches[i];
        if patch.animated && patch.in_transition {
            self.execute_deactivate_animation(index);
            self.apply_patch_final(tcx, index, false);
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
            self.apply_patch_final(tcx, index, false);
        }
    }

    pub(crate) fn apply_patch_final(
        &mut self,
        tcx: TickCtx<'_>,
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
                self.execute_swap_objects(tcx, index, false, forced_reset);
            }
        } else {
            self.script_domains.interactables.patches[i].applied = true;
            self.swap_patch_background(index, true);
            self.execute_swap_objects(tcx, index, true, forced_reset);
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

    pub(crate) fn reset_patch(&mut self, tcx: TickCtx<'_>, index: crate::patch::PatchIndex) {
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
        self.execute_swap_objects(tcx, index, false, true);
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
        tcx: TickCtx<'_>,
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
        let mut changes = Vec::with_capacity(1 + patch.additional_motion_changes.len());
        if patch.use_changing_obstacles {
            changes.push(crate::level_data::PatchMotionChange {
                layer: patch.pathfinder_layer,
                sector: patch.pathfinder_sector,
                changing_obstacle: u16::try_from(patch.pathfinder_changing_obstacles)
                    .expect("patch changing-obstacle index exceeds u16"),
            });
        }
        changes.extend_from_slice(&patch.additional_motion_changes);

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
        // Resolve every binding before mutating any area. A duplicated binding
        // would toggle twice, silently undoing part of the transition.
        let mut unique = std::collections::BTreeSet::new();
        let changes: Vec<_> = changes
            .into_iter()
            .map(|change| {
                let crate::level_data::PatchMotionChange {
                    layer,
                    sector,
                    changing_obstacle,
                } = change;
                assert!(
                    changing_obstacle < 16,
                    "patch movement state exceeds the area's 16 bit pairs"
                );
                assert!(
                    unique.insert((layer, sector, changing_obstacle)),
                    "duplicate patch movement binding"
                );
                let area = self
                    .world
                    .pathfinder
                    .try_convert_sector(tcx.assets.navigation.pathfinder_graph.as_ref(), sector)
                    .unwrap_or_else(|| {
                        panic!(
                            "patch_effects: ConvertSector failed — no area mapping \
                         for pathfinder_sector={} (layer={})",
                            sector, layer
                        )
                    });
                assert!(
                    tcx.assets
                        .navigation
                        .pathfinder_graph
                        .static_data
                        .move_layers
                        .get(usize::from(layer))
                        .and_then(|areas| areas.get(usize::from(area)))
                        .is_some(),
                    "patch movement binding references a missing layer/area"
                );
                (change, area)
            })
            .collect();
        let mut appeared_by_area = Vec::with_capacity(changes.len());
        for (change, area) in changes {
            let appeared = self.world.pathfinder.toggle_obstacle_state(
                tcx.assets.navigation.pathfinder_graph.as_ref(),
                std::sync::Arc::make_mut(&mut self.world.fast_grid),
                change.layer as usize,
                area as usize,
                change.changing_obstacle,
            );

            appeared_by_area.push((change, appeared));
        }
        // Replanning must see the complete transition, not a mix of old and new areas.
        if !forced_reset {
            for (change, appeared) in appeared_by_area {
                self.invalidate_paths_and_kill_crushed(tcx, change.layer, change.sector, &appeared);
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
        tcx: TickCtx<'_>,
        layer: u16,
        sector: u16,
        appeared: &[crate::fast_find_grid::SectorIndex],
    ) {
        let actor_slots = self.world.entities.len();
        for slot in 0..actor_slots {
            let Some((id, entity)) = self.world.entities.get_legacy_slot(slot as u32) else {
                continue;
            };
            if entity.actor_data().is_none() {
                continue;
            }
            let own_sector = entity.element_data().sector();
            let same_area = entity.element_data().layer() == layer
                && own_sector == crate::position_interface::SectorHandle::new(sector);
            let physical_owner = own_sector
                .and_then(|owner| tcx.assets.navigation.physical_stairs.get(&owner.get()));
            let physical_collision = physical_owner
                .filter(|stair| same_area || stair.has_landing(layer, sector))
                .map(|stair| (stair, same_area))
                .or_else(|| {
                    let owner = own_sector?;
                    let stair = tcx.assets.navigation.physical_stairs.get(&sector)?;
                    let position = entity.position_iface().get_position();
                    stair
                        .supports_landing_neighbour(
                            entity.element_data().layer(),
                            owner.get(),
                            [position.x, position.y, position.z],
                        )
                        .then_some((stair, true))
                });
            if !same_area && physical_collision.is_none() {
                continue;
            }
            // Read destination and action from the selected movement,
            // clear its orders, then re-run
            // `try_dispatch_move_path` to re-submit the path request.
            let retranslate =
                self.current_sequence_element_for_actor(id)
                    .and_then(|(seq_id, elem_idx)| {
                        // Only paths inside the changed area need retranslation.
                        // Physical routes query adjoining state before each step.
                        if !same_area {
                            return None;
                        }
                        let elem = self.orders.sequence_manager.get_element(seq_id, elem_idx)?;
                        if elem.command != crate::element::Command::MoveOk {
                            return None;
                        }
                        if elem
                            .current_order()
                            .is_some_and(|order| order.physical_stair == Some(sector))
                        {
                            // Physical orders rebuild their route from live state
                            // before every step. Screen-space retranslation would
                            // discard their world destination and stair identity.
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
                if !self.extract_move_instruction_owner(tcx.assets, id) {
                    self.element_impossible(
                        tcx,
                        &mut Vec::new(),
                        SequenceElementRef::new(seq_id, elem_idx),
                    );
                } else {
                    match self.try_dispatch_move_path(
                        tcx,
                        id,
                        SequenceElementRef::new(seq_id, elem_idx),
                        dest,
                        action,
                    ) {
                        MovePathOutcome::Success | MovePathOutcome::Pending => {
                            // Retranslation replaces order storage. Install the
                            // new current order only if this element still owns
                            // the actor after its callbacks.
                            let installed_order = self
                                .orders
                                .sequence_manager
                                .current_order_for_actor(&self.world.entities, id)
                                .filter(|(live_seq, live_idx, _)| {
                                    *live_seq == seq_id && *live_idx == elem_idx
                                })
                                .map(|(sequence_id, element_index, order)| {
                                    crate::element::InstalledActorOrder::new(
                                        crate::sequence::SequenceElementRef::new(
                                            sequence_id,
                                            element_index,
                                        ),
                                        order,
                                    )
                                });
                            self.install_actor_order(id, installed_order);
                        }
                        MovePathOutcome::ActorGone | MovePathOutcome::Refused => {
                            self.element_impossible(
                                tcx,
                                &mut Vec::new(),
                                SequenceElementRef::new(seq_id, elem_idx),
                            );
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
            let physical_position = entity.position_iface().get_position();
            let physical_half = entity.position_iface().get_half_diagonal();
            for sector_index in appeared {
                let obstacle = &self.world.fast_grid.level.sectors[sector_index.get() as usize];
                let intersects = if let Some((stair, obstacle_on_stair)) = physical_collision {
                    let local_obstacle = u16::from(obstacle.sector_number)
                        .checked_sub(sector)
                        .and_then(|offset| offset.checked_sub(1))
                        .expect("physical control references an unrelated motion obstacle");
                    if obstacle_on_stair {
                        stair.obstacle_intersects_actor(
                            local_obstacle,
                            [physical_position.x, physical_position.y],
                            physical_half,
                        )
                    } else {
                        stair.landing_obstacle_intersects_actor(
                            layer,
                            sector,
                            local_obstacle,
                            [physical_position.x, physical_position.y],
                            physical_half,
                        )
                    }
                } else {
                    move_box.is_somewhere()
                        && obstacle.bounding_box.is_somewhere()
                        && obstacle.bounding_box.intersects_bbox(&move_box)
                        && obstacle.intersects_bbox(&move_box)
                };
                if intersects {
                    if let Some(entity) = self.get_entity_mut(id) {
                        entity.element_data_mut().unreachable = true;
                    }
                    self.launch_damage(tcx, id, 1000, 1000);
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
    /// returns its `HostEffects` (see `robin_rs::blit_to_map`).
    pub(crate) fn queue_blit_fx_to_map(&mut self, entity_id: crate::element::EntityId) {
        let decal = self.snapshot_patch_transition_decal(entity_id);
        self.feedback
            .pending_side_effects
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
    mod route_tests {
        use super::*;
        include!("patch_route_tests.rs");
    }
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

    #[test]
    fn preserved_state_contours_keep_fractional_routes_through_apply_and_reset() {
        let (mut engine, assets) = load_compiled_transition(
            include_bytes!("../../tests/fixtures/asset-preserved-state-boundary.level.json"),
            (2000., 2000.),
        );
        let patch = crate::patch::PatchIndex::new(0).unwrap();
        let sim = crate::sim_rng::test_context();
        for (step, applied) in [false, true, false].into_iter().enumerate() {
            if step == 1 {
                engine.apply_patch(TickCtx::new(&sim, &assets), patch);
            } else if step == 2 {
                engine.reset_patch(TickCtx::new(&sim, &assets), patch);
            }
            let grid = &engine.world.fast_grid;
            let end = MapPoint::new(350., 334.5);
            assert!(grid.is_reachable_thin(MapPoint::new(301.1, 300.05), end, 0));
            assert_eq!(
                grid.is_reachable_thin(end, MapPoint::new(350., 333.5), 0),
                applied
            );
            assert!(!grid.is_reachable_thin(end, MapPoint::new(350., 335.5), 0));
        }
    }

    #[test]
    fn unavailable_terrain_control_retains_loadable_initial_barriers() {
        let (engine, _) = load_compiled_transition(
            include_bytes!("../../tests/fixtures/asset-unavailable-terrain-control.level.json"),
            (2000., 2000.),
        );
        assert!(engine.script_domains.interactables.patches.is_empty());
        for layer in [0, 1] {
            assert!(!engine.world.fast_grid.is_reachable_thin(
                MapPoint::new(325., 350.),
                MapPoint::new(375., 350.),
                layer,
            ));
            assert!(engine.world.fast_grid.is_reachable_thin(
                MapPoint::new(425., 350.),
                MapPoint::new(475., 350.),
                layer,
            ));
        }
    }

    #[test]
    fn unavailable_mask_control_keeps_initial_coverage_and_other_switches_independent() {
        let (mut engine, assets) = load_compiled_transition(
            include_bytes!("../../tests/fixtures/asset-unavailable-mask-control.level.json"),
            (2000., 2000.),
        );
        assert_eq!(engine.script_domains.interactables.patches.len(), 1);
        assert_eq!(engine.world.fast_grid.level.masks.len(), 3);
        assert_eq!(engine.world.fast_grid.mask_active, [true, true, false]);
        assert_eq!(engine.world.fast_grid.level.masks[0].height, 40);
        let patch = crate::patch::PatchIndex::new(0).unwrap();
        let sim = crate::sim_rng::test_context();
        assert_eq!(engine.script_domains.interactables.doors.len(), 2);
        assert_eq!(
            engine.script_domains.interactables.doors[0].patch_index,
            None
        );
        assert_eq!(
            engine.script_domains.interactables.doors[1].patch_index,
            Some(patch)
        );
        for index in 0..2 {
            let door = engine.script_domains.interactables.doors[index].clone();
            let actor = engine.add_test_entity(
                crate::engine::test_support::actors::TestActor::pc(
                    crate::element::Posture::Upright,
                )
                .sector(u16::from(door.sector_out))
                .map_position(door.point_out)
                .build(),
            );
            engine
                .get_entity_mut(actor)
                .unwrap()
                .element_data_mut()
                .set_layer(door.layer_out);
            let door_index = crate::gate::DoorIndex::new(index as u32).unwrap();
            engine.execute_pass_door(TickCtx::new(&sim, &assets), actor, door_index, true);
            let element = engine.get_entity(actor).unwrap().element_data();
            assert_eq!(
                element.sector().map(u16::from),
                Some(u16::from(door.sector_in))
            );
            assert_eq!(element.layer(), door.layer_in);
            assert_eq!(
                engine.world.fast_grid.mask_active,
                [true, index == 0, index == 1]
            );
            assert_eq!(
                engine.script_domains.interactables.patches[0].applied,
                index == 1
            );
            engine.execute_pass_door(TickCtx::new(&sim, &assets), actor, door_index, false);
            let element = engine.get_entity(actor).unwrap().element_data();
            assert_eq!(
                element.sector().map(u16::from),
                Some(u16::from(door.sector_out))
            );
            assert_eq!(element.layer(), door.layer_out);
        }
        assert_eq!(engine.world.fast_grid.mask_active, [true, true, false]);
        engine.apply_patch(TickCtx::new(&sim, &assets), patch);
        assert_eq!(engine.world.fast_grid.mask_active, [true, false, true]);
        engine.reset_patch(TickCtx::new(&sim, &assets), patch);
        assert_eq!(engine.world.fast_grid.mask_active, [true, true, false]);
    }

    #[test]
    fn terrain_bound_gate_changes_slope_routes_without_blocking_the_floor_above() {
        let (mut engine, assets) = load_compiled_transition(
            include_bytes!("../../tests/fixtures/asset-terrain-transition.level.json"),
            (2000., 2000.),
        );
        let patch = crate::patch::PatchIndex::new(0).unwrap();
        let sim = crate::sim_rng::test_context();
        for (step, applied) in [false, true, false, true, false].into_iter().enumerate() {
            if step > 0 {
                if applied {
                    engine.apply_patch(TickCtx::new(&sim, &assets), patch);
                } else {
                    engine.reset_patch(TickCtx::new(&sim, &assets), patch);
                }
            }
            let grid = &engine.world.fast_grid;
            assert_eq!(
                grid.is_reachable_thin(MapPoint::new(325., 350.), MapPoint::new(375., 350.), 0),
                applied,
            );
            assert_eq!(
                grid.is_reachable_thin(MapPoint::new(425., 350.), MapPoint::new(475., 350.), 0),
                !applied,
            );
            assert!(grid.is_reachable_thin(
                MapPoint::new(325., 250.),
                MapPoint::new(475., 250.),
                1
            ));
        }
    }

    #[test]
    fn compiled_stair_barriers_update_live_routes_after_placement_and_reset() {
        let fixtures: Vec<serde_json::Value> = serde_json::from_slice(include_bytes!(
            "../../tests/fixtures/asset-changing-lifts.levels.json"
        ))
        .unwrap();
        assert_eq!(fixtures.len(), 4);
        for (rotation, fixture) in fixtures.iter().enumerate() {
            let bytes = serde_json::to_vec(fixture).unwrap();
            let loaded = crate::level_data::LoadedLevel::hackable_from_json(&bytes).unwrap();
            let lift = &loaded.proto.lifts[0];
            let layer = lift.doors[0].layer_in;
            let sector = lift.motion_area_index;
            let [a, b] = [lift.doors[0].point_in, lift.doors[1].point_in]
                .map(|(x, y)| MapPoint::new(f32::from(x), f32::from(y)));
            let (mut engine, assets) = load_compiled_transition(&bytes, (2000., 2000.));
            let sim = crate::sim_rng::test_context();
            let patch = crate::patch::PatchIndex::new(0).unwrap();
            for (step, applied) in [false, true, false, true, false].into_iter().enumerate() {
                if step > 0 {
                    if applied {
                        engine.apply_patch(TickCtx::new(&sim, &assets), patch);
                    } else {
                        engine.reset_patch(TickCtx::new(&sim, &assets), patch);
                    }
                }
                let grid = &engine.world.fast_grid;
                for (direction, (source, goal)) in [(a, b), (b, a)].into_iter().enumerate() {
                    assert_eq!(
                        grid.is_reachable_thin(source, goal, layer),
                        !applied,
                        "stair collision, rotation {rotation}, applied {applied}"
                    );
                    let stair = &assets.navigation.physical_stairs[&sector];
                    let from = stair.definition.doors[direction].inside;
                    let to = stair.definition.doors[1 - direction].inside;
                    let route = stair
                        .route(
                            &engine.world.pathfinder,
                            [from[0], from[1]],
                            [to[0], to[1]],
                            crate::coordinates::MoveBoxHalfDiagonal::new(6., 3.),
                        )
                        .unwrap();
                    assert_eq!(
                        route.is_some(),
                        !applied,
                        "stair route, rotation {rotation}, applied {applied}"
                    );
                }
            }
        }
    }

    #[test]
    fn editor_compiled_movement_transition_changes_live_routes_without_mission_content() {
        check_compiled_transition(
            include_bytes!("../../tests/fixtures/asset-movement-transition.level.json"),
            false,
        );
    }

    #[test]
    fn generated_jump_approaches_remain_walkable_when_nearby_obstacles_switch() {
        let (mut engine, assets) = load_compiled_transition(
            include_bytes!("../../tests/fixtures/asset-jump-changing-approach.level.json"),
            (2000., 2000.),
        );
        let patch = crate::patch::PatchIndex::new(0).unwrap();
        assert_eq!(engine.script_domains.interactables.patches.len(), 1);
        assert_eq!(engine.world.fast_grid.level.jump_lines.len(), 4);
        let original_lines = engine.world.fast_grid.level.jump_lines.clone();
        let sim = crate::sim_rng::test_context();
        for (step, applied) in [false, true, false, true, false].into_iter().enumerate() {
            if step > 0 {
                if applied {
                    engine.apply_patch(TickCtx::new(&sim, &assets), patch);
                } else {
                    engine.reset_patch(TickCtx::new(&sim, &assets), patch);
                }
            }
            let grid = &engine.world.fast_grid;
            // This crosses the excluded middle of the original unsplit ledge.
            assert_eq!(
                grid.is_reachable_thin(MapPoint::new(383., 335.), MapPoint::new(383., 365.), 0),
                !applied
            );
            let footprint = grid.try_move_box_half_diagonal(0).unwrap();
            for (line, original) in grid.level.jump_lines.iter().zip(&original_lines) {
                assert_eq!(line.point_a, original.point_a);
                assert_eq!(line.point_b, original.point_b);
                let dx = line.point_b.x - line.point_a.x;
                let dy = line.point_b.y - line.point_a.y;
                let length = dx.hypot(dy);
                let sector =
                    grid.level.sectors[line.sector_index.unwrap().get() as usize].sector_number;
                for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
                    let edge = MapPoint::new(line.point_a.x + dx * t, line.point_a.y + dy * t);
                    let inside =
                        MapPoint::new(edge.x + dy / length * 6., edge.y - dx / length * 6.);
                    for (start, goal) in [(inside, edge), (edge, inside)] {
                        assert!(grid.is_reachable_thick(start, goal, line.layer, footprint));
                        let route = engine
                            .world
                            .pathfinder
                            .find_path(
                                &assets.navigation.pathfinder_graph,
                                grid,
                                line.layer,
                                i16::from(sector) as u16,
                                0,
                                start,
                                goal,
                                false,
                            )
                            .expect("retained jump approach must work in both obstacle states");
                        assert_eq!(route.last(), Some(&goal));
                    }
                }
            }
        }
    }

    #[test]
    fn editor_compiled_sight_transition_swaps_obstacles_with_navigation() {
        check_compiled_transition(
            include_bytes!("../../tests/fixtures/asset-sight-transition.level.json"),
            true,
        );
    }

    #[test]
    fn editor_joined_asset_switch_updates_both_members_and_resets() {
        let bytes = include_bytes!("../../tests/fixtures/asset-joined-transition.level.json");
        let (mut engine, assets) = load_compiled_transition(bytes, (2000., 2000.));
        let descriptor: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        let transition: crate::level_data::CompiledMovementTransition =
            serde_json::from_value(descriptor["asset_geometry"]["movement_transitions"][0].clone())
                .unwrap();
        assert_eq!(engine.script_domains.interactables.patches.len(), 1);
        assert_eq!(transition.aliases, ["wing/wing/barriers"]);
        assert_eq!(transition.initial_sight.len(), 2);
        assert_eq!(transition.applied_sight.len(), 2);
        assert_eq!(transition.motion_changes.len(), 4);
        let states = engine.world.pathfinder.states.clone();
        let sight = engine.world.static_sight_obstacle_active.clone();
        let grid = engine.world.fast_grid.sector_active.clone();
        let mut expected = states.clone();
        for change in &transition.motion_changes {
            let area = engine
                .world
                .pathfinder
                .try_convert_sector(assets.navigation.pathfinder_graph.as_ref(), change.sector)
                .unwrap();
            let state = &mut expected[change.layer as usize][area as usize];
            let initial = 1u32 << (2 * change.changing_obstacle);
            assert_ne!(*state & initial, 0);
            *state = (*state & !(initial * 3)) | (initial * 2);
        }
        let sim = crate::sim_rng::test_context();
        let index = crate::patch::PatchIndex::new(0).unwrap();
        for _ in 0..2 {
            engine.apply_patch(TickCtx::new(&sim, &assets), index);
            assert_eq!(engine.world.pathfinder.states, expected);
            for &obstacle in &transition.initial_sight {
                assert!(!engine.world.static_sight_obstacle_active[obstacle as usize]);
            }
            for &obstacle in &transition.applied_sight {
                assert!(engine.world.static_sight_obstacle_active[obstacle as usize]);
            }
            assert_ne!(engine.world.fast_grid.sector_active, grid);
            engine.reset_patch(TickCtx::new(&sim, &assets), index);
            assert_eq!(engine.world.pathfinder.states, states);
            assert_eq!(engine.world.static_sight_obstacle_active, sight);
            assert_eq!(engine.world.fast_grid.sector_active, grid);
        }
        for aliases in [
            vec![""],
            vec!["hut-a/hut/barriers"],
            vec!["duplicate", "duplicate"],
        ] {
            let mut invalid = descriptor.clone();
            invalid["asset_geometry"]["movement_transitions"][0]["aliases"] =
                serde_json::json!(aliases);
            assert!(
                crate::level_data::LoadedLevel::hackable_from_json(
                    &serde_json::to_vec(&invalid).unwrap(),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn unavailable_projection_control_keeps_receiving_geometry_without_physical_collision() {
        use crate::coordinates::WorldPoint3D;
        use crate::sight_obstacle::{
            SIGHTOBSTACLE_OPAQUE, SIGHTOBSTACLE_SOLID, is_reachable_impact_3d,
        };
        let (engine, assets) = load_compiled_transition(
            include_bytes!("../../tests/fixtures/asset-unavailable-projection-control.level.json"),
            (2000., 2000.),
        );
        assert!(engine.script_domains.interactables.patches.is_empty());
        assert!(!engine.world.static_sight_obstacle_active[0]);
        let receiver = &assets.environment.static_sight_obstacles[0];
        assert!(receiver.projection_area_ref().is_some());
        let point = MapPoint::new(350., 330.);
        let sector_index =
            engine.world.fast_grid.level.sector_number_map[&crate::sector::SectorNumber::new(1)];
        let sector = crate::position_interface::SectorHandle::new(1)
            .unwrap()
            .with_arena_index(
                crate::fast_find_grid::SectorIndex::new(sector_index as u32).unwrap(),
            );
        assert_eq!(
            engine.get_projection_area_index(&assets, sector, 1, point),
            crate::sight_obstacle::SightObstacleIndex::new(0)
        );
        assert_eq!(
            receiver.compute_top_z_from_projection(point.x, point.y),
            20.
        );
        assert_eq!(
            assets
                .environment
                .material_sectors
                .material_at_with_obstacle(Some(receiver), point),
            crate::element::GameMaterial::from_u32(2)
        );
        for filter in [SIGHTOBSTACLE_SOLID, SIGHTOBSTACLE_OPAQUE] {
            assert!(
                is_reachable_impact_3d(
                    WorldPoint3D::new(350., 350., 100.),
                    WorldPoint3D::new(350., 350., 1.),
                    filter,
                    engine.sight_obstacles(&assets),
                    None,
                    None,
                )
                .is_none()
            );
        }
    }

    #[test]
    fn editor_projection_volume_preserves_physical_state_and_receiving_geometry() {
        use crate::coordinates::WorldPoint3D;
        use crate::sight_obstacle::{
            SIGHTOBSTACLE_OPAQUE, SIGHTOBSTACLE_SOLID, is_reachable_impact_3d,
        };
        let (mut engine, assets) = load_compiled_transition(
            include_bytes!("../../tests/fixtures/asset-projection-volume.level.json"),
            (2000., 2000.),
        );
        let receiver = &assets.environment.static_sight_obstacles[0];
        assert!(receiver.projection_area_ref().is_some());
        assert_eq!(receiver.compute_top_z(350., 350.), 20.);
        let impact = |engine: &EngineInner, filter, upward| {
            is_reachable_impact_3d(
                WorldPoint3D::new(350., 350., if upward { 1. } else { 100. }),
                WorldPoint3D::new(350., 350., if upward { 100. } else { 1. }),
                filter,
                engine.sight_obstacles(&assets),
                None,
                None,
            )
            .map(|hit| hit.impact.z)
        };
        let patch = crate::patch::PatchIndex::new(0).unwrap();
        let sim = crate::sim_rng::test_context();
        for applied in [false, true, false] {
            if applied {
                engine.apply_patch(TickCtx::new(&sim, &assets), patch);
            } else {
                engine.reset_patch(TickCtx::new(&sim, &assets), patch);
            }
            for filter in [SIGHTOBSTACLE_SOLID, SIGHTOBSTACLE_OPAQUE] {
                assert_eq!(impact(&engine, filter, false), applied.then_some(20.));
                assert_eq!(impact(&engine, filter, true), applied.then_some(15.));
            }
        }
    }

    #[test]
    fn compiled_projection_states_toggle_collision_without_changing_elevation_lookup() {
        use crate::coordinates::MapPoint;
        use crate::fast_find_grid::SectorIndex;
        use crate::position_interface::SectorHandle;
        use crate::sector::SectorNumber;
        for swap in [false, true] {
            let mut descriptor: serde_json::Value = serde_json::from_slice(include_bytes!(
                "../../tests/fixtures/asset-projection-material.level.json"
            ))
            .unwrap();
            descriptor["asset_geometry"]["sight_obstacles"][0]["solid"] = true.into();
            let applied = if swap {
                let mut upper = descriptor["asset_geometry"]["sight_obstacles"][0].clone();
                for point in upper["points"].as_array_mut().unwrap() {
                    point["y"] = (point["y"].as_f64().unwrap() + 20.).into();
                    point["z_top"] = 40.into();
                    point["z_bottom"] = 40.into();
                }
                upper["default_material"] = 4.into();
                upper["material_indices"] = serde_json::json!([]);
                descriptor["asset_geometry"]["sight_obstacles"]
                    .as_array_mut()
                    .unwrap()
                    .push(upper);
                2
            } else {
                0
            };
            descriptor["asset_geometry"]["movement_transitions"] = serde_json::json!([{
                "id":"receiver-state", "waypoint":[300,300], "sector":0,"layer":0,
                "active":true,"definitive":false,
                "apply_polygon":{"points":[]},"no_apply_polygon":{"points":[]},
                "motion_changes":[], "initial_sight":if swap {vec![0]} else {vec![]},
                "applied_sight":[applied]
            }]);
            let (mut engine, assets) =
                load_compiled_transition(&serde_json::to_vec(&descriptor).unwrap(), (2000., 2000.));
            let level = engine.world.fast_grid.level.clone();
            let index = level.sector_number_map[&SectorNumber::new(1)];
            let sector = SectorHandle::new(1)
                .unwrap()
                .with_arena_index(SectorIndex::new(index as u32).unwrap());
            let point = MapPoint::new(350., 330.);
            let receive = |engine: &EngineInner| {
                engine
                    .get_projection_area_index(&assets, sector, 1, point)
                    .map(|index| {
                        let obstacle =
                            &assets.environment.static_sight_obstacles[usize::from(index)];
                        (
                            obstacle.compute_top_z_from_projection(point.x, point.y),
                            assets
                                .environment
                                .material_sectors
                                .material_at_with_obstacle(Some(obstacle), point),
                        )
                    })
            };
            let collision = |engine: &EngineInner| {
                use crate::coordinates::WorldPoint3D;
                use crate::sight_obstacle::{SIGHTOBSTACLE_SOLID, is_reachable_impact_3d};
                is_reachable_impact_3d(
                    WorldPoint3D::new(350., 350., 100.),
                    WorldPoint3D::new(350., 350., 1.),
                    SIGHTOBSTACLE_SOLID,
                    engine.sight_obstacles(&assets),
                    None,
                    None,
                )
                .map(|hit| hit.impact.z)
            };
            let receiving = Some((
                if swap { 40. } else { 20. },
                crate::element::GameMaterial::from_u32(if swap { 4 } else { 2 }),
            ));
            assert_eq!(receive(&engine), receiving);
            assert_eq!(collision(&engine), if swap { Some(20.) } else { None });
            let patch = crate::patch::PatchIndex::new(0).unwrap();
            let sim = crate::sim_rng::test_context();
            engine.apply_patch(TickCtx::new(&sim, &assets), patch);
            assert_eq!(receive(&engine), receiving);
            assert_eq!(collision(&engine), Some(if swap { 40. } else { 20. }));
            engine.reset_patch(TickCtx::new(&sim, &assets), patch);
            assert_eq!(receive(&engine), receiving);
            assert_eq!(collision(&engine), if swap { Some(20.) } else { None });
            assert!(std::sync::Arc::ptr_eq(
                &level,
                &engine.world.fast_grid.level
            ));
        }
    }

    #[test]
    fn compiled_mask_only_transition_resolves_layer_indices_and_resets() {
        let mut descriptor: serde_json::Value =
            serde_json::from_slice(include_bytes!("../../tests/fixtures/asset-lift.level.json"))
                .unwrap();
        let mask = serde_json::json!({
            "layer": 0, "mask_type": 1,
            "character_polyline": [[300, 320], [308, 320]],
            "projectile_polyline": null,
            "box_top_left": [300, 300], "box_size": [8, 1],
            "mask_data": [2, 129, 255], "obstacle_indices": []
        });
        let mut upper = mask.clone();
        upper["layer"] = 1.into();
        descriptor["asset_geometry"]["masks"] = serde_json::json!([mask, upper, mask]);
        descriptor["asset_geometry"]["movement_transitions"] = serde_json::json!([{
            "id": "mask-state", "waypoint": [300, 300], "sector": 0, "layer": 0,
            "active": true, "definitive": false,
            "apply_polygon": {"points": []}, "no_apply_polygon": {"points": []},
            "motion_changes": [], "initial_masks": [2], "applied_masks": [0]
        }]);
        let (mut engine, assets) =
            load_compiled_transition(&serde_json::to_vec(&descriptor).unwrap(), (2000., 2000.));
        let patch = crate::patch::PatchIndex::new(0).unwrap();
        let index = |i| crate::mask::MaskIndex::new(i).unwrap();
        let state = |engine: &EngineInner| {
            (0..3)
                .map(|i| engine.world.fast_grid.is_mask_active(index(i)))
                .collect::<Vec<_>>()
        };
        assert_eq!(state(&engine), [false, true, true]);
        let binding = &engine.script_domains.interactables.patches[0];
        assert_eq!(binding.old_mask_indices, [index(2)]);
        assert_eq!(binding.new_mask_indices, [index(0)]);
        let sim = crate::sim_rng::test_context();
        engine.apply_patch(TickCtx::new(&sim, &assets), patch);
        assert_eq!(state(&engine), [true, true, false]);
        engine.reset_patch(TickCtx::new(&sim, &assets), patch);
        assert_eq!(state(&engine), [false, true, true]);

        for (initial, applied) in [(vec![3], vec![]), (vec![0], vec![0]), (vec![2, 2], vec![0])] {
            let mut bad = descriptor.clone();
            bad["asset_geometry"]["movement_transitions"][0]["initial_masks"] =
                serde_json::json!(initial);
            bad["asset_geometry"]["movement_transitions"][0]["applied_masks"] =
                serde_json::json!(applied);
            assert!(
                crate::level_data::LoadedLevel::hackable_from_json(
                    &serde_json::to_vec(&bad).unwrap()
                )
                .is_err()
            );
        }
        let mut duplicate = descriptor.clone();
        let mut second = duplicate["asset_geometry"]["movement_transitions"][0].clone();
        second["id"] = "second-controller".into();
        duplicate["asset_geometry"]["movement_transitions"]
            .as_array_mut()
            .unwrap()
            .push(second);
        assert!(
            crate::level_data::LoadedLevel::hackable_from_json(
                &serde_json::to_vec(&duplicate).unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn editor_appearance_only_transition_toggles_without_geometry_side_effects() {
        let bytes = include_bytes!("../../tests/fixtures/asset-appearance-only.level.json");
        let (mut engine, assets) = load_compiled_transition(bytes, (2000., 2000.));
        let index = crate::patch::PatchIndex::new(0).unwrap();
        assert_eq!(engine.script_domains.interactables.patches.len(), 1);
        let patch = &engine.script_domains.interactables.patches[0];
        assert!(!patch.use_changing_obstacles);
        assert!(patch.additional_motion_changes.is_empty());
        assert!(patch.door_indices.is_empty());
        assert!(patch.old_mask_indices.is_empty() && patch.new_mask_indices.is_empty());
        assert!(
            patch.old_sight_obstacle_indices.is_empty()
                && patch.new_sight_obstacle_indices.is_empty()
        );
        let grid = serde_json::to_value(&engine.world.fast_grid).unwrap();
        let doors = serde_json::to_value(&engine.script_domains.interactables.doors).unwrap();
        let sim = crate::sim_rng::test_context();
        for expected in [true, false, true] {
            engine.apply_patch(TickCtx::new(&sim, &assets), index);
            assert_eq!(
                engine.script_domains.interactables.patches[0].applied,
                expected
            );
            assert_eq!(serde_json::to_value(&engine.world.fast_grid).unwrap(), grid);
            assert_eq!(
                serde_json::to_value(&engine.script_domains.interactables.doors).unwrap(),
                doors
            );
        }
        engine.reset_patch(TickCtx::new(&sim, &assets), index);
        assert!(!engine.script_domains.interactables.patches[0].applied);
        assert_eq!(serde_json::to_value(&engine.world.fast_grid).unwrap(), grid);
        assert_eq!(
            serde_json::to_value(&engine.script_domains.interactables.doors).unwrap(),
            doors
        );
        let mut invalid: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        invalid["asset_geometry"]["movement_transitions"][0]["has_appearance"] = false.into();
        let error = crate::level_data::LoadedLevel::hackable_from_json(
            &serde_json::to_vec(&invalid).unwrap(),
        )
        .err()
        .unwrap();
        assert!(error.contains("invalid compiled movement transition"));
    }

    #[test]
    fn editor_asset_mask_transition_switches_baked_coverage_and_resets() {
        let (mut engine, assets) = load_compiled_transition(
            include_bytes!("../../tests/fixtures/asset-mask.level.json"),
            (2000., 2000.),
        );
        let patch = crate::patch::PatchIndex::new(0).unwrap();
        let coverage = |engine: &EngineInner| {
            engine
                .world
                .fast_grid
                .level
                .masks
                .iter()
                .enumerate()
                .filter(|(index, _)| {
                    engine
                        .world
                        .fast_grid
                        .is_mask_active(crate::mask::MaskIndex::new(*index as u32).unwrap())
                })
                .map(|(_, mask)| {
                    mask.bitmap
                        .iter()
                        .map(|&pixel| usize::from(pixel))
                        .sum::<usize>()
                })
                .sum::<usize>()
        };
        assert_eq!(coverage(&engine), 400);
        let sim = crate::sim_rng::test_context();
        engine.apply_patch(TickCtx::new(&sim, &assets), patch);
        assert_eq!(coverage(&engine), 100);
        engine.reset_patch(TickCtx::new(&sim, &assets), patch);
        assert_eq!(coverage(&engine), 400);
    }

    #[test]
    fn editor_compiled_door_links_wire_both_directions_and_restore_rights() {
        let (mut engine, assets) = load_compiled_transition(
            include_bytes!("../../tests/fixtures/asset-door-transition.level.json"),
            (2000., 2000.),
        );
        let trigger = crate::patch::PatchIndex::new(0).unwrap();
        let swap = crate::patch::PatchIndex::new(1).unwrap();
        let domains = &engine.script_domains.interactables;
        assert_eq!(domains.doors.len(), 3);
        assert_eq!(domains.doors[2].patch_index, Some(trigger));
        assert_eq!(domains.doors[0].patch_index, None);
        assert_eq!(domains.doors[1].patch_index, None);
        assert!(domains.patches[0].door_indices.is_empty());
        assert_eq!(domains.patches[1].door_indices, vec![0, 1]);
        let rights = |engine: &EngineInner| {
            engine
                .script_domains
                .interactables
                .doors
                .iter()
                .map(|door| {
                    [
                        door.locked_pc,
                        door.unlockable,
                        door.locked_npc_villain,
                        door.locked_npc_civilian,
                    ]
                })
                .collect::<Vec<_>>()
        };
        let before = rights(&engine);
        let sim = crate::sim_rng::test_context();
        engine.apply_patch(TickCtx::new(&sim, &assets), swap);
        let after = rights(&engine);
        assert_eq!(after[0], [true, false, true, false]);
        assert_eq!(after[1], [false, false, true, false]);
        assert_eq!(after[2], before[2]);
        engine.reset_patch(TickCtx::new(&sim, &assets), swap);
        assert_eq!(rights(&engine), before);
        let linked = engine.script_domains.interactables.doors[2]
            .patch_index
            .unwrap();
        engine.apply_patch(TickCtx::new(&sim, &assets), linked);
        assert!(engine.script_domains.interactables.patches[0].applied);
        assert_eq!(rights(&engine), before);
        engine.reset_patch(TickCtx::new(&sim, &assets), linked);
        assert!(!engine.script_domains.interactables.patches[0].applied);
        assert_eq!(rights(&engine), before);
    }

    fn load_compiled_transition(
        bytes: &[u8],
        bg_pixel_dims: (f32, f32),
    ) -> (EngineInner, LevelAssets) {
        let loaded = crate::level_data::LoadedLevel::hackable_from_json(bytes).unwrap();
        assert!(loaded.mission.beam_mes.is_empty());
        assert!(loaded.mission.soldiers.is_empty());
        let mut assets = LevelAssets::new();
        let mut profiles = crate::profiles::ProfileManager::new();
        let mut campaign = crate::campaign::Campaign::new();
        let mission = campaign
            .force_next_mission_by_name(&mut profiles, "asset-state", "asset-state", true)
            .unwrap();
        campaign.current_mission_idx = Some(mission);
        assets.profile_manager = std::sync::Arc::new(profiles);
        let engine = crate::engine::Engine::new(crate::engine::EngineArgs {
            campaign,
            level: crate::engine::LevelLoadArgs {
                assets: &mut assets,
                level_directory: "",
                progress: &mut |_| {},
                loaded,
                bg_pixel_dims,
            },
            ground_mark_sprite: None,
            titbit_row_frame_counts: vec![],
            rng_seed: 0,
            original_rng_replay: None,
            sim_config: crate::engine::SimConfig {
                script_enabled: false,
                ..Default::default()
            },
        })
        .expect("load editor movement state fixture without datadir");
        let level_grid = engine.fast_grid().level.clone();
        let mut engine =
            crate::engine::snapshot::decode_native_engine_inner(&engine.encode_native_snapshot())
                .unwrap();
        engine.world.fast_grid_mut().attach_level_grid(level_grid);
        (engine, assets)
    }

    #[test]
    #[ignore = "requires recovered diagnostics via ROBIN_ASSET_MAP_DIAGNOSTICS"]
    fn recovered_lift_passage_callbacks_preserve_sector_and_layer() {
        let directory = std::path::PathBuf::from(
            std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").expect("diagnostic directory"),
        );
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["complete"], true, "incomplete diagnostic batch");
        let sim = crate::sim_rng::test_context();
        let mut checked = 0;
        for result in manifest["results"].as_array().unwrap() {
            assert!(result["error"].is_null(), "failed diagnostic: {result}");
            let file = result["file"].as_str().unwrap();
            let bytes = std::fs::read(directory.join(file)).unwrap();
            let descriptor: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let dims = &descriptor["walkable_polygon"][2];
            let (mut engine, assets) = load_compiled_transition(
                &bytes,
                (
                    dims[0].as_f64().unwrap() as f32 + 1.,
                    dims[1].as_f64().unwrap() as f32 + 1.,
                ),
            );
            let doors: Vec<_> = engine
                .script_domains
                .interactables
                .doors
                .iter()
                .enumerate()
                .filter(|(_, door)| {
                    engine.world.fast_grid.level.sectors[usize::from(door.sector_in_index.unwrap())]
                        .sector_type
                        .is_lift()
                })
                .map(|(index, door)| (index, door.clone()))
                .collect();
            let mut passages = 0;
            for (entry_index, entry) in &doors {
                for (exit_index, exit) in &doors {
                    if entry_index == exit_index || entry.sector_in_index != exit.sector_in_index {
                        continue;
                    }
                    // Exercise each directed endpoint pair through passage callbacks.
                    // Approach routing and climb animation are separate checks.
                    let actor = engine.add_test_entity(
                        crate::engine::test_support::actors::TestActor::pc(Posture::Upright)
                            .sector(u16::from(entry.sector_out))
                            .map_position(entry.point_out)
                            .build(),
                    );
                    engine
                        .get_entity_mut(actor)
                        .unwrap()
                        .element_data_mut()
                        .set_layer(entry.layer_out);
                    engine.execute_pass_door(
                        TickCtx::new(&sim, &assets),
                        actor,
                        crate::gate::DoorIndex::new(*entry_index as u32).unwrap(),
                        true,
                    );
                    let element = engine.get_entity(actor).unwrap().element_data();
                    assert_eq!(
                        element.sector().map(u16::from),
                        Some(u16::from(entry.sector_in)),
                        "{file}"
                    );
                    assert_eq!(element.layer(), entry.layer_in, "{file}");
                    engine.execute_pass_door(
                        TickCtx::new(&sim, &assets),
                        actor,
                        crate::gate::DoorIndex::new(*exit_index as u32).unwrap(),
                        false,
                    );
                    let element = engine.get_entity(actor).unwrap().element_data();
                    assert_eq!(
                        element.sector().map(u16::from),
                        Some(u16::from(exit.sector_out)),
                        "{file}"
                    );
                    assert_eq!(element.layer(), exit.layer_out, "{file}");
                    passages += 1;
                }
            }
            println!("{file}: checked {passages} directed lift passage callbacks");
            checked += passages;
        }
        assert!(checked > 0, "No recovered lift passages were tested");
    }

    #[test]
    #[ignore = "requires recovered diagnostics via ROBIN_ASSET_MAP_DIAGNOSTICS"]
    fn recovered_asset_transitions_apply_and_reset_native_geometry() {
        let directory = std::path::PathBuf::from(
            std::env::var("ROBIN_ASSET_MAP_DIAGNOSTICS").expect("diagnostic directory"),
        );
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(directory.join("diagnostics.json")).unwrap())
                .unwrap();
        assert_eq!(
            manifest["scope"],
            "static-geometry-only-not-gameplay-parity"
        );
        let mut checked = 0;
        assert_ne!(manifest["complete"], false, "incomplete diagnostic batch");
        for result in manifest["results"].as_array().unwrap() {
            assert!(result["error"].is_null(), "failed diagnostic: {result}");
            let file = result["file"].as_str().expect("missing diagnostic file");
            let bytes = std::fs::read(directory.join(file)).unwrap();
            let descriptor: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let transitions: Vec<crate::level_data::CompiledMovementTransition> =
                serde_json::from_value(
                    descriptor["asset_geometry"]
                        .get("movement_transitions")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!([])),
                )
                .unwrap();
            let dims = &descriptor["walkable_polygon"][2];
            let (mut engine, assets) = load_compiled_transition(
                &bytes,
                (
                    dims[0].as_f64().unwrap() as f32 + 1.,
                    dims[1].as_f64().unwrap() as f32 + 1.,
                ),
            );
            assert_eq!(
                engine.script_domains.interactables.patches.len(),
                transitions.len()
            );
            let sim = crate::sim_rng::test_context();
            let rights = |engine: &EngineInner| {
                engine
                    .script_domains
                    .interactables
                    .doors
                    .iter()
                    .map(|door| {
                        [
                            door.locked_pc,
                            door.unlockable,
                            door.locked_npc_villain,
                            door.locked_npc_civilian,
                            door.locked_pc_after_patch,
                            door.unlockable_after_patch,
                            door.locked_npc_villain_after_patch,
                            door.locked_npc_civilian_after_patch,
                        ]
                    })
                    .collect::<Vec<_>>()
            };
            for (index, transition) in transitions.iter().enumerate() {
                let check_routes = |engine: &EngineInner, applied: bool| {
                    for probe in result["transition_probes"].as_array().into_iter().flatten() {
                        if probe["id"].as_str() != Some(transition.id.as_str()) {
                            continue;
                        }
                        let point = |key: &str| {
                            MapPoint::new(
                                probe[key][0].as_f64().unwrap() as f32,
                                probe[key][1].as_f64().unwrap() as f32,
                            )
                        };
                        assert_eq!(
                            engine.world.fast_grid.is_reachable_thin(
                                point("start"),
                                point("end"),
                                probe["layer"].as_u64().unwrap() as u16
                            ),
                            probe[if applied { "applied" } else { "initial" }]
                                .as_bool()
                                .unwrap(),
                            "{file}: transition route {probe}, applied={applied}",
                        );
                    }
                };
                check_routes(&engine, false);
                let before_rights = rights(&engine);
                let mut expected_rights = before_rights.clone();
                let patch = crate::patch::PatchIndex::new(index as u32).unwrap();
                if let Some(links) = &transition.door_links {
                    for &door in &links.indices {
                        match links.mode {
                            crate::level_data::CompiledDoorLinkMode::TriggerTransition => {
                                assert_eq!(
                                    engine.script_domains.interactables.doors[door as usize]
                                        .patch_index,
                                    Some(patch),
                                    "{file}"
                                );
                            }
                            crate::level_data::CompiledDoorLinkMode::SwapRights => {
                                expected_rights[door as usize].rotate_left(4);
                            }
                        }
                    }
                }
                let before_states = engine.world.pathfinder.states.clone();
                let before_sight = engine.world.static_sight_obstacle_active.clone();
                let before_sectors = engine.world.fast_grid.sector_active.clone();
                let mask_count = descriptor["asset_geometry"]["masks"]
                    .as_array()
                    .map_or(0, Vec::len);
                let mask_states = |engine: &EngineInner| {
                    (0..mask_count)
                        .map(|i| {
                            engine
                                .world
                                .fast_grid
                                .is_mask_active(crate::mask::MaskIndex::new(i as u32).unwrap())
                        })
                        .collect::<Vec<_>>()
                };
                let before_masks = mask_states(&engine);
                let mut expected_masks = before_masks.clone();
                for &mask in &transition.initial_masks {
                    assert!(before_masks[mask as usize], "{file}: {}", transition.id);
                    expected_masks[mask as usize] = false;
                }
                for &mask in &transition.applied_masks {
                    assert!(!before_masks[mask as usize], "{file}: {}", transition.id);
                    expected_masks[mask as usize] = true;
                }
                let mut expected_states = before_states.clone();
                for change in &transition.motion_changes {
                    let area = engine
                        .world
                        .pathfinder
                        .try_convert_sector(
                            assets.navigation.pathfinder_graph.as_ref(),
                            change.sector,
                        )
                        .unwrap();
                    let state = &mut expected_states[change.layer as usize][area as usize];
                    let initial = 1u32 << (2 * change.changing_obstacle);
                    assert_ne!(*state & initial, 0, "{file}: {}", transition.id);
                    *state = (*state & !(initial * 3)) | (initial * 2);
                }
                for &sight in &transition.initial_sight {
                    assert!(before_sight[sight as usize]);
                }
                for &sight in &transition.applied_sight {
                    assert!(!before_sight[sight as usize]);
                }
                engine.apply_patch(TickCtx::new(&sim, &assets), patch);
                check_routes(&engine, true);
                assert_eq!(
                    mask_states(&engine),
                    expected_masks,
                    "{file}: {}",
                    transition.id
                );
                assert_eq!(
                    rights(&engine),
                    expected_rights,
                    "{file}: {}",
                    transition.id
                );
                assert_eq!(
                    engine.world.pathfinder.states, expected_states,
                    "{file}: {}",
                    transition.id
                );
                for &sight in &transition.initial_sight {
                    assert!(!engine.world.static_sight_obstacle_active[sight as usize]);
                }
                for &sight in &transition.applied_sight {
                    assert!(engine.world.static_sight_obstacle_active[sight as usize]);
                }
                for (layer, areas) in assets
                    .navigation
                    .pathfinder_graph
                    .static_data
                    .move_layers
                    .iter()
                    .enumerate()
                {
                    for (area, motion) in areas.iter().enumerate() {
                        for obstacle in &motion.motion_obstacles {
                            let active = expected_states[layer][area] & obstacle.state_id
                                == obstacle.state_id;
                            if let Some(sector) = obstacle.grid_sector_index {
                                assert_eq!(
                                    engine.world.fast_grid.sector_active[sector.get() as usize],
                                    active,
                                    "{file}"
                                );
                            }
                        }
                    }
                }
                engine.reset_patch(TickCtx::new(&sim, &assets), patch);
                check_routes(&engine, false);
                assert_eq!(
                    mask_states(&engine),
                    before_masks,
                    "{file}: {}",
                    transition.id
                );
                assert_eq!(rights(&engine), before_rights, "{file}: {}", transition.id);
                assert_eq!(engine.world.pathfinder.states, before_states, "{file}");
                assert_eq!(
                    engine.world.static_sight_obstacle_active, before_sight,
                    "{file}"
                );
                assert_eq!(
                    engine.world.fast_grid.sector_active, before_sectors,
                    "{file}"
                );
                if (!transition.initial_masks.is_empty() || !transition.applied_masks.is_empty())
                    && let Some(links) = &transition.door_links
                    && matches!(
                        links.mode,
                        crate::level_data::CompiledDoorLinkMode::TriggerTransition
                    )
                {
                    for &door_index in &links.indices {
                        // Exercise the passage callback with a test actor; this does
                        // not simulate approach routing or animation playback.
                        let (mut passage, passage_assets) = load_compiled_transition(
                            &bytes,
                            (
                                dims[0].as_f64().unwrap() as f32 + 1.,
                                dims[1].as_f64().unwrap() as f32 + 1.,
                            ),
                        );
                        let door =
                            passage.script_domains.interactables.doors[door_index as usize].clone();
                        let actor = passage.add_test_entity(
                            crate::engine::test_support::actors::TestActor::pc(
                                crate::element::Posture::Upright,
                            )
                            .sector(u16::from(door.sector_out))
                            .map_position(door.point_out)
                            .build(),
                        );
                        passage
                            .get_entity_mut(actor)
                            .unwrap()
                            .element_data_mut()
                            .set_layer(door.layer_out);
                        let door_index =
                            crate::gate::DoorIndex::new(u32::from(door_index)).unwrap();
                        passage.execute_pass_door(
                            TickCtx::new(&sim, &passage_assets),
                            actor,
                            door_index,
                            true,
                        );
                        let element = passage.get_entity(actor).unwrap().element_data();
                        assert_eq!(
                            element.sector().map(u16::from),
                            Some(u16::from(door.sector_in)),
                            "{file}"
                        );
                        assert_eq!(element.layer(), door.layer_in, "{file}");
                        assert_eq!(
                            mask_states(&passage),
                            expected_masks,
                            "{file}: door {door_index}"
                        );
                        assert!(passage.script_domains.interactables.patches[index].applied);
                        for &sight in &transition.initial_sight {
                            assert!(!passage.world.static_sight_obstacle_active[sight as usize]);
                        }
                        for &sight in &transition.applied_sight {
                            assert!(passage.world.static_sight_obstacle_active[sight as usize]);
                        }
                        passage.execute_pass_door(
                            TickCtx::new(&sim, &passage_assets),
                            actor,
                            door_index,
                            false,
                        );
                        let element = passage.get_entity(actor).unwrap().element_data();
                        assert_eq!(
                            element.sector().map(u16::from),
                            Some(u16::from(door.sector_out)),
                            "{file}"
                        );
                        assert_eq!(element.layer(), door.layer_out, "{file}");
                        passage.reset_patch(TickCtx::new(&sim, &passage_assets), patch);
                        assert_eq!(mask_states(&passage), before_masks, "{file}: door reset");
                    }
                }
                checked += 1;
            }
            println!(
                "{file}: applied and reset {} recovered transitions",
                transitions.len()
            );
        }
        assert!(checked > 0, "No recovered transitions were tested");
    }

    fn check_compiled_transition(bytes: &[u8], sight: bool) {
        let (mut engine, assets) = load_compiled_transition(bytes, (2000., 2000.));
        let check_actor_routes = |engine: &mut EngineInner, applied: bool| {
            let grid = &engine.world.fast_grid;
            let half = grid.try_move_box_half_diagonal(0).unwrap();
            assert_eq!((half.x, half.y), (6., 3.));
            for (sector, left, right, open) in [(0, 330., 370., applied), (2, 430., 490., !applied)]
            {
                let left = MapPoint::new(left, 320.);
                let right = MapPoint::new(right, 320.);
                for (source, goal) in [(left, right), (right, left)] {
                    // Reuse the live finder: rebuilding it would hide stale state.
                    let route = engine.world.pathfinder.find_path(
                        &assets.navigation.pathfinder_graph,
                        grid,
                        0,
                        sector,
                        0,
                        source,
                        goal,
                        false,
                    );
                    assert_eq!(route.is_some(), open, "sector {sector}, applied={applied}");
                    if let Some(route) = route {
                        assert_eq!(route.last(), Some(&goal));
                        let mut previous = source;
                        for point in route {
                            assert!(grid.is_reachable_thick(previous, point, 0, half));
                            previous = point;
                        }
                    }
                }
            }
        };
        let western_route = |e: &EngineInner| {
            e.world.fast_grid.is_reachable_thin(
                MapPoint::new(330., 320.),
                MapPoint::new(370., 320.),
                0,
            )
        };
        let eastern_route = |e: &EngineInner| {
            e.world.fast_grid.is_reachable_thin(
                MapPoint::new(430., 320.),
                MapPoint::new(490., 320.),
                0,
            )
        };
        assert!(!western_route(&engine));
        assert!(eastern_route(&engine));
        check_actor_routes(&mut engine, false);
        if sight {
            assert_eq!(engine.world.static_sight_obstacle_active, vec![true, false]);
        }
        let sim = crate::sim_rng::test_context();
        let patch = crate::patch::PatchIndex::new(0).unwrap();
        engine.apply_patch(TickCtx::new(&sim, &assets), patch);
        assert!(western_route(&engine));
        assert!(!eastern_route(&engine));
        check_actor_routes(&mut engine, true);
        if sight {
            assert_eq!(engine.world.static_sight_obstacle_active, vec![false, true]);
        }
        engine.reset_patch(TickCtx::new(&sim, &assets), patch);
        assert!(!western_route(&engine));
        assert!(eastern_route(&engine));
        check_actor_routes(&mut engine, false);
        if sight {
            assert_eq!(engine.world.static_sight_obstacle_active, vec![true, false]);
        }
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
    fn one_asset_transition_updates_and_resets_multiple_navigation_areas() {
        use crate::fast_find_grid::{GridSector, SectorIndex};
        use crate::pathfinder::{MotionArea, MotionObstacle, PathGraph};
        let (mut engine, index) = patch_fixture(false, false);
        let old_line = engine.script_domains.interactables.patches[0].old_line_indices[0];
        let new_line = engine.script_domains.interactables.patches[0].new_line_indices[0];
        let patch = &mut engine.script_domains.interactables.patches[0];
        patch.old_line_indices.clear();
        patch.new_line_indices.clear();
        patch.use_changing_obstacles = true;
        patch.pathfinder_layer = 0;
        patch.pathfinder_sector = 0;
        patch
            .additional_motion_changes
            .push(crate::level_data::PatchMotionChange {
                layer: 0,
                sector: 2,
                changing_obstacle: 0,
            });
        let mut graph = PathGraph::new();
        graph.static_mut().move_layers = vec![vec![
            MotionArea {
                polygon: vec![],
                skeleton: vec![],
                motion_obstacles: vec![MotionObstacle {
                    state_id: 1,
                    active: true,
                    bounding_box: Default::default(),
                    polygon: vec![],
                    grid_sector_index: SectorIndex::new(0),
                    grid_line_indices: vec![old_line],
                }],
            },
            MotionArea {
                polygon: vec![],
                skeleton: vec![],
                motion_obstacles: vec![MotionObstacle {
                    state_id: 2,
                    active: false,
                    bounding_box: Default::default(),
                    polygon: vec![],
                    grid_sector_index: SectorIndex::new(1),
                    grid_line_indices: vec![new_line],
                }],
            },
        ]];
        graph.layers = vec![vec![vec![vec![]], vec![vec![]]]];
        graph.alternative_layers = graph.layers.clone();
        graph.states = vec![vec![0, 0]];
        graph.build_sector_conversion();
        for number in 0..2 {
            engine.world.fast_grid_mut().add_sector(
                GridSector {
                    sector_type: crate::sector::SectorType::MOTION,
                    layer: 0,
                    sector_number: crate::sector::SectorNumber::new(number),
                    ..Default::default()
                },
                0,
            );
        }
        engine.world.pathfinder.initialize_from_graph(
            &graph,
            std::sync::Arc::make_mut(&mut engine.world.fast_grid),
        );
        engine.world.pathfinder.synchronize_motion_obstacle_sectors(
            &graph,
            std::sync::Arc::make_mut(&mut engine.world.fast_grid),
        );
        let mut assets = LevelAssets::default();
        assets.navigation.pathfinder_graph = std::sync::Arc::new(graph);
        let sim = crate::sim_rng::test_context();
        assert_eq!(engine.world.fast_grid.sector_active, [true, false]);
        engine.apply_patch(TickCtx::new(&sim, &assets), index);
        assert_eq!(engine.world.fast_grid.sector_active, [false, true]);
        assert!(!engine.world.fast_grid.is_line_active(old_line));
        assert!(engine.world.fast_grid.is_line_active(new_line));
        let saved = crate::engine::snapshot::encode_native_engine_inner(&engine);
        let mut engine = crate::engine::snapshot::decode_native_engine_inner(&saved)
            .expect("movement bindings survive a native snapshot");
        engine.reset_patch(TickCtx::new(&sim, &assets), index);
        assert_eq!(engine.world.fast_grid.sector_active, [true, false]);
        assert!(engine.world.fast_grid.is_line_active(old_line));
        assert!(!engine.world.fast_grid.is_line_active(new_line));
    }

    #[test]
    fn patch_transitions_update_canonical_terrain_and_door_rights() {
        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::default();
        for animated in [false, true] {
            for definitive in [false, true] {
                let (mut engine, index) = patch_fixture(animated, definitive);
                engine.apply_patch(TickCtx::new(&sim, &assets), index);
                if animated {
                    assert_patch_terrain(&engine, false);
                    assert!(engine.script_domains.interactables.patches[0].in_transition);
                    engine.finish_patch_transition_for(TickCtx::new(&sim, &assets), index);
                }
                assert_patch_terrain(&engine, true);
                assert_eq!(
                    engine.script_domains.interactables.patches[0].active,
                    !definitive
                );
                engine.apply_patch(TickCtx::new(&sim, &assets), index);
                if animated && !definitive {
                    assert_patch_terrain(&engine, true);
                    engine.finish_patch_transition_for(TickCtx::new(&sim, &assets), index);
                }
                assert_patch_terrain(&engine, definitive);
                engine.reset_patch(TickCtx::new(&sim, &assets), index);
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
        engine.apply_patch(TickCtx::new(&sim, &assets), index);
        engine.apply_patch(TickCtx::new(&sim, &assets), index);
        assert_patch_terrain(&engine, true);
        assert!(engine.script_domains.interactables.patches[0].in_transition);
        engine.finish_patch_transition_for(TickCtx::new(&sim, &assets), index);
        assert_patch_terrain(&engine, false);
    }

    #[test]
    fn reset_during_forward_transition_does_not_toggle_doors() {
        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::default();
        let (mut engine, index) = patch_fixture(true, false);
        engine.apply_patch(TickCtx::new(&sim, &assets), index);
        engine.reset_patch(TickCtx::new(&sim, &assets), index);
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
            engine.apply_patch(TickCtx::new(&sim, &assets), index);
            engine.finish_patch_transition_for(TickCtx::new(&sim, &assets), index);
            assert_patch_terrain(&engine, true);
            let patch = &mut engine.script_domains.interactables.patches[0];
            *patch = serde_json::from_str(&serde_json::to_string(patch).unwrap()).unwrap();
            assert_eq!(
                patch.repeat_activation,
                Some((123, "ActivatedBySword".into()))
            );
            engine.apply_patch(TickCtx::new(&sim, &assets), index);
            engine.finish_patch_transition_for(TickCtx::new(&sim, &assets), index);
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
        let mut assets = LevelAssets::default();
        assets.entities.patch_animation_entities = std::sync::Arc::new(vec![Some(
            crate::natives::ScriptHandleCodec::actor_handle_from_index(0),
        )]);
        engine
            .script_domains
            .interactables
            .patches
            .push(crate::patch::Patch {
                integrate_in_background: true,
                ..Default::default()
            });
        let initial = engine.clone();
        let index = crate::patch::PatchIndex::new(0).unwrap();
        engine.apply_patch(
            TickCtx::new(&crate::sim_rng::test_context(), &assets),
            index,
        );
        let applied = engine.clone();
        let blits = applied.background_patch_blits(&assets);
        assert_eq!(blits.len(), 1);
        assert_eq!(blits[0].entity_id, entity_id);
        assert_eq!(blits[0].decal.as_ref().unwrap().bank_id, 22);
        assert!(
            initial.background_patch_blits(&assets).is_empty(),
            "backward adoption removes the patch"
        );
        engine.script_domains.interactables.patches[0].in_transition = true;
        assert!(
            engine.background_patch_blits(&assets).is_empty(),
            "reverse transition has restored the base map"
        );
        engine.reset_patch(
            TickCtx::new(&crate::sim_rng::test_context(), &assets),
            index,
        );
        assert!(engine.background_patch_blits(&assets).is_empty());
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
            highest_door_index: None,
            lowest_door_index: None,
            jump_line_indices: Vec::new(),
            gate_indices: Vec::new(),
            underlying_sector: None,
        };
        assert!(obstacle.bounding_box.intersects_bbox(&move_box));
        assert!(!obstacle.bounding_box.intersects_bbox(&expected_box));
        let obstacle_index = engine.world.fast_grid_mut().add_sector(obstacle, 0);
        engine.invalidate_paths_and_kill_crushed(
            TickCtx::new(&crate::sim_rng::test_context(), &LevelAssets::default()),
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
