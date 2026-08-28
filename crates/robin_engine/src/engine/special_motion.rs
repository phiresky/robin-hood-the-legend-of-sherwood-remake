//! Shared finalization for C++-style special motion.
//!
//! Special transitions such as jumps, wall/ladder door passes, falls, and
//! scripted teleports all converge in the original engine through the same
//! sprite position path: set the final position, recompute map/3D coordinates,
//! refresh obstacle/material, then leave display ordering to the normal draw
//! pass. Keeping that ordering in one Rust helper avoids each subsystem
//! inventing a slightly different landing/update sequence.

use super::{EngineInner, LevelAssets};
use crate::coordinates::{MapPoint, WorldPoint3D};
use crate::element::{ActiveDoorPass, EntityId};
use crate::movement::ActiveMovement;
use crate::order::{Order, OrderType};
use crate::sequence::SequenceId;

#[derive(Debug, Clone, Copy)]
pub(super) enum SpecialMovePosition {
    Map(MapPoint),
    World(WorldPoint3D),
}

impl SpecialMovePosition {
    fn map_point(self) -> MapPoint {
        match self {
            Self::Map(point) => point,
            Self::World(point) => point.to_map(),
        }
    }
}

impl EngineInner {
    /// Finalize a nonstandard movement step through the same position stack.
    ///
    /// `obstacle_probe` is intentionally independent from the final actor
    /// map point. C++ jump landing, for example, asks the destination line's
    /// sector for the projection area at the line midpoint rather than at
    /// the exact landing point.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn finalize_special_move_position(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        position: SpecialMovePosition,
        layer: Option<u16>,
        sector: Option<u16>,
        obstacle_probe: Option<MapPoint>,
        context: &'static str,
    ) {
        self.finalize_special_move_position_inner(
            assets,
            entity_id,
            position,
            layer,
            sector,
            None,
            obstacle_probe,
            false,
            context,
        );
    }

    /// Jump landings always install the destination sector's projection
    /// area. Original's synthetic ground projection is represented by no
    /// Rust obstacle, so a missing authored obstacle must clear the previous
    /// plane rather than preserve it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn finalize_special_move_position_with_ground(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        position: SpecialMovePosition,
        layer: Option<u16>,
        sector: Option<u16>,
        obstacle_probe: MapPoint,
        context: &'static str,
    ) {
        self.finalize_special_move_position_inner(
            assets,
            entity_id,
            position,
            layer,
            sector,
            None,
            Some(obstacle_probe),
            true,
            context,
        );
    }

    /// Finalize position and projection-plane state without entering the
    /// projection sector's topology. Door transition action points can select
    /// the far-side obstacle before the later explicit `PassingDoor` order
    /// changes layer/sector.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn finalize_special_move_position_using_projection_sector(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        position: SpecialMovePosition,
        projection_layer: u16,
        projection_sector: u16,
        obstacle_probe: MapPoint,
        context: &'static str,
    ) {
        self.finalize_special_move_position_inner(
            assets,
            entity_id,
            position,
            None,
            None,
            Some((projection_layer, projection_sector)),
            Some(obstacle_probe),
            false,
            context,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn finalize_special_move_position_inner(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        position: SpecialMovePosition,
        layer: Option<u16>,
        sector: Option<u16>,
        projection_topology: Option<(u16, u16)>,
        obstacle_probe: Option<MapPoint>,
        clear_projection_when_missing: bool,
        context: &'static str,
    ) {
        let Some((target_layer, current_sector)) = self.get_entity(entity_id).map(|entity| {
            (
                layer.unwrap_or_else(|| entity.element_data().layer()),
                entity.element_data().sector(),
            )
        }) else {
            tracing::warn!(
                ?entity_id,
                context,
                "special motion finalization missing entity"
            );
            return;
        };
        let exact_sector = |number: u16| {
            current_sector
                .filter(|handle| handle.get() == number)
                .or_else(|| {
                    let number = crate::sector::SectorNumber::new(number as i16);
                    self.world
                        .fast_grid
                        .level
                        .sector_number_map
                        .get(&number)
                        .copied()
                        .and_then(|index| {
                            let index = crate::fast_find_grid::SectorIndex::new(
                                u32::try_from(index).expect("sector arena index exceeds u32"),
                            )
                            .expect("sector arena index collides with the null encoding");
                            crate::position_interface::SectorHandle::new(number.get() as u16)
                                .map(|handle| handle.with_arena_index(index))
                        })
                })
                .unwrap_or_else(|| panic!("{context}: sector {number} has no exact arena identity"))
        };
        let target_sector = sector.map(exact_sector).or(current_sector);
        let (projection_layer, projection_sector) = match projection_topology {
            Some((layer, sector)) => (layer, Some(exact_sector(sector))),
            None => (target_layer, target_sector),
        };

        let obstacle = obstacle_probe.map(|probe| {
            let resolved = if let Some(sector) = projection_sector {
                self.get_projection_area_index(assets, sector, projection_layer, probe)
            } else {
                self.find_plane_obstacle_at(assets, projection_layer, probe)
            };
            if resolved.is_none() {
                tracing::warn!(
                    ?entity_id,
                    context,
                    layer = projection_layer,
                    ?projection_sector,
                    probe_x = probe.x,
                    probe_y = probe.y,
                    "special motion finalization found no projection-area obstacle"
                );
            }
            resolved
        });

        if let Some(entity) = self.get_entity_mut(entity_id) {
            let elem = entity.element_data_mut();
            if let Some(layer) = layer {
                elem.set_layer(layer);
            }
            if sector.is_some() {
                elem.set_sector(target_sector);
            }
            match position {
                SpecialMovePosition::Map(point) => elem.set_position_map(point),
                SpecialMovePosition::World(point) => elem.set_position(point),
            }
            elem.update_grid_cell();
        }

        // SetPositionMap + ComputePositionAll in the Original recomputes 3D
        // from the sprite's currently installed plane. A probe that falls
        // just outside the projection polygon (door midpoints commonly sit
        // on that boundary) therefore does not clear the old obstacle/plane.
        // Only replace projection state when this operation resolves a real
        // successor obstacle.
        if let Some(resolved) = obstacle
            && (resolved.is_some() || clear_projection_when_missing)
        {
            self.set_obstacle_and_material(assets, entity_id, resolved);
            if let Some(entity) = self.get_entity_mut(entity_id) {
                let point = position.map_point();
                entity.element_data_mut().set_position_map(point);
                entity.element_data_mut().update_grid_cell();
            }
        }

        tracing::trace!(
            ?entity_id,
            context,
            map = ?self.get_entity(entity_id).map(|e| e.element_data().position_map()),
            elevation = ?self.get_entity(entity_id).map(|e| e.element_data().position().z),
            plane = ?self.get_entity(entity_id).and_then(|e| e.position_iface().get_plane().copied()),
            "special motion finalization applied"
        );

        self.update_opponents_jump_lines(assets, entity_id);
    }

    /// Install a movement order that was produced by a special-motion
    /// translator such as PassDoor/lift/wall/stairs.
    ///
    /// C++ still executes these as normal actor movement orders after the
    /// translator has selected the exact step. Selecting the successor does
    /// not execute it: posture/action-state changes belong to the successor's
    /// own actor slot and are therefore deliberately not applied here.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn install_special_walk_order(
        &mut self,
        entity_id: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
        destination: MapPoint,
        action: OrderType,
        reverse: bool,
        compute_direction: bool,
        tolerance: f32,
        active_door_pass: Option<ActiveDoorPass>,
        context: &'static str,
    ) {
        let order_id = self.orders.allocate_order_id();
        let mut order = Order::new(action, destination.x, destination.y, order_id);
        order.reverse = reverse;
        order.compute_direction = compute_direction;
        order.tolerance = tolerance;
        self.orders
            .sequence_manager
            .push_order_on(seq_id, elem_idx, order);

        if let Some(entity) = self.world.entities.get_mut(entity_id) {
            if let Some(actor) = entity.actor_data_mut() {
                actor.active_movement = ActiveMovement::new(seq_id, elem_idx);
                if let Some(dp) = active_door_pass {
                    actor.passing_door_directly = dp.position_direct;
                    actor.active_door_pass = Some(dp);
                }
                actor.sequence_element_started = true;
            }
        } else {
            tracing::warn!(
                ?entity_id,
                ?seq_id,
                elem_idx,
                context,
                "special walk order installed for missing entity"
            );
        }

        tracing::debug!(
            entity = ?entity_id,
            ?seq_id,
            elem_idx,
            ?action,
            target_x = destination.x,
            target_y = destination.y,
            context,
            "special walk order installed"
        );
    }
}
