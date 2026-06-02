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
use crate::element::EntityId;

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
        let Some((target_layer, target_sector)) = self.get_entity(entity_id).map(|entity| {
            (
                layer.unwrap_or_else(|| entity.element_data().layer()),
                sector.or_else(|| entity.element_data().sector().map(u16::from)),
            )
        }) else {
            tracing::warn!(
                ?entity_id,
                context,
                "special motion finalization missing entity"
            );
            return;
        };

        let obstacle = obstacle_probe.map(|probe| {
            let resolved = if let Some(sector) = target_sector {
                self.get_projection_area_index(assets, sector, target_layer, probe)
            } else {
                self.find_plane_obstacle_at(assets, target_layer, probe)
            };
            if resolved.is_none() {
                tracing::warn!(
                    ?entity_id,
                    context,
                    layer = target_layer,
                    sector = target_sector,
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
            if let Some(sector) = sector {
                elem.set_sector(crate::position_interface::SectorHandle::new(sector));
            }
            match position {
                SpecialMovePosition::Map(point) => elem.set_position_map(point),
                SpecialMovePosition::World(point) => elem.set_position(point),
            }
            elem.update_grid_cell();
        }

        if let Some(obstacle) = obstacle {
            self.set_obstacle_and_material(assets, entity_id, obstacle);
            if let Some(entity) = self.get_entity_mut(entity_id) {
                let point = position.map_point();
                entity.element_data_mut().set_position_map(point);
                entity.element_data_mut().update_grid_cell();
            }
        }

        self.update_opponents_jump_lines(assets, entity_id);
    }
}
